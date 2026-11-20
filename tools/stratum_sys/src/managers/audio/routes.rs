//! Apps sent to an output of their own, as Windows' per-app device
//! preferences: every app plays through an EQ. Apps on Default play into
//! the main one (`effect_input.stratum_eq`), which follows the output
//! chosen on the Audio page. An app sent to a device plays into an EQ of
//! that device's own, made on first use with the device's preset, and stays
//! there when the main output changes. Such an EQ is removed once no app has
//! used it for a while, or when its device goes (its apps return to
//! Default).
//!
//! The EQs run in this process, as the main one does. A stream that asked
//! not to be moved (`node.dont-reconnect`, which pipewire-pulse and
//! WirePlumber both honour) is moved by linking it to the new sink and only
//! then unlinking it from the old one.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use pulsectl::controllers::{AppControl, DeviceControl};
use pulsectl::ControllerError;
use tracing::{info, warn};

use super::{config, engine, native_eq, pulse};

/// What `move_stream` takes for "follow the main output".
pub const DEFAULT_OUTPUT: &str = "@DEFAULT_SINK@";

const INPUT_PREFIX: &str = "effect_input.stratum_eq.";
const OUTPUT_PREFIX: &str = "effect_output.stratum_eq.";

/// How long a new device EQ may stay empty before it is removed: the app
/// is moved into it just after it is made.
const GRACE: Duration = Duration::from_secs(5);

/// How long to wait for PipeWire to show a node or a link.
const SETTLE: Duration = Duration::from_secs(3);

struct Route {
    /// Names its nodes: a retuned EQ is a new one, made before the old one
    /// goes, so its apps are never without a sink.
    generation: u32,
    /// Its curve, as [`curve_key`], to tell whether a new one differs.
    curve: String,
    made: Instant,
}

impl Route {
    fn input(&self) -> String {
        input_name(self.generation)
    }
}

struct Routes {
    next: u32,
    by_device: HashMap<String, Route>,
}

static ROUTES: LazyLock<Mutex<Routes>> = LazyLock::new(|| Mutex::new(Routes { next: 1, by_device: HashMap::new() }));

fn input_name(generation: u32) -> String {
    format!("{INPUT_PREFIX}{generation}")
}

fn output_name(generation: u32) -> String {
    format!("{OUTPUT_PREFIX}{generation}")
}

/// Whether `name` is a node of a device's EQ.
pub(crate) fn is_route_node(name: &str) -> bool {
    name.starts_with(INPUT_PREFIX) || name.starts_with(OUTPUT_PREFIX)
}

/// The sinks of the device EQs.
pub(crate) fn input_sinks() -> Vec<String> {
    ROUTES.lock().map(|r| r.by_device.values().map(Route::input).collect()).unwrap_or_default()
}

/// The device whose EQ has the sink `sink`.
pub(crate) fn device_of_sink(sink: &str) -> Option<String> {
    let routes = ROUTES.lock().ok()?;
    routes.by_device.iter().find(|(_, r)| r.input() == sink).map(|(d, _)| d.clone())
}

/// The output port names of the sink to link a stream's port `port` to:
/// left to left, right to right, anything else (mono, centre) to both.
pub(crate) fn inputs_for(port: &str) -> &'static [&'static str] {
    match port.rsplit('_').next().unwrap_or("") {
        "FL" | "RL" | "SL" => &["playback_FL"],
        "FR" | "RR" | "SR" => &["playback_FR"],
        _ => &["playback_FL", "playback_FR"],
    }
}

/// Move the playing stream `index` to `device`, or to the main EQ for
/// [`DEFAULT_OUTPUT`]. Without the main EQ (the EQ failed to load) it goes
/// to the device itself.
pub(crate) fn move_playback(index: u32, device: &str) -> Result<bool, ControllerError> {
    let eq_running = pulse::with_sink(|ctrl| ctrl.get_device_by_name(config::EQ_VIRTUAL_INPUT_SINK)).is_ok();
    let sink = if device == DEFAULT_OUTPUT {
        if eq_running {
            config::EQ_VIRTUAL_INPUT_SINK.to_string()
        } else {
            pulse::default_sink_name().unwrap_or_default()
        }
    } else if eq_running {
        match ensure(device) {
            Ok(sink) => sink,
            Err(err) => {
                warn!("could not make an EQ for {device}: {err}");
                return Ok(false);
            }
        }
    } else {
        device.to_string()
    };
    let moved = move_into(index, &sink);
    prune();
    // The EQ it left may be unused once its grace runs out.
    prune_soon(GRACE + Duration::from_secs(1));
    moved
}

/// [`prune`] on a thread of its own after `delay`, when there is any device
/// EQ.
pub(crate) fn prune_soon(delay: Duration) {
    if ROUTES.lock().map(|r| r.by_device.is_empty()).unwrap_or(true) {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        prune();
    });
}

/// The sink of `device`'s EQ, made if there is none.
fn ensure(device: &str) -> Result<String, String> {
    let mut routes = ROUTES.lock().map_err(|_| "routes lock poisoned".to_string())?;
    if let Some(route) = routes.by_device.get(device) {
        return Ok(route.input());
    }
    let (bands, preamp) = super::preset_curve(device);
    let generation = routes.next;
    routes.next += 1;
    build(device, generation, &bands, preamp)?;
    let curve = curve_key(&bands, preamp);
    routes.by_device.insert(device.to_string(), Route { generation, curve, made: Instant::now() });
    Ok(input_name(generation))
}

/// The curve's filters, the same for the same curve whatever the device.
fn curve_key(bands: &[config::EqBand], preamp: f64) -> String {
    engine::filter_graph(bands, preamp)
}

/// Load an EQ for `device` with this curve under `generation`'s names and
/// link it to the device.
fn build(device: &str, generation: u32, bands: &[config::EqBand], preamp: f64) -> Result<(), String> {
    let description = format!("Stratum EQ ({})", describe(device));
    // Lowest priority, so WirePlumber never picks it as the default output
    // while the main EQ is being rebuilt.
    let args = engine::eq_module_args(
        bands,
        preamp,
        &input_name(generation),
        &output_name(generation),
        &description,
        device,
        "priority.session = 1 priority.driver = 1",
    );
    engine::load_eq_module(&args)?;
    let output = output_name(generation);
    let start = Instant::now();
    let node = loop {
        if let Some(node) = engine::find_node_id_by_name(&output) {
            if native_eq::output_ports(node).len() >= 2 || start.elapsed() > SETTLE {
                break node;
            }
        }
        if start.elapsed() > SETTLE {
            engine::destroy_node_named(&input_name(generation));
            return Err("the EQ did not appear".to_string());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    for side in ["FL", "FR"] {
        if let Err(err) = native_eq::create_link(&output, &format!("output_{side}"), device, &format!("playback_{side}")) {
            warn!("could not link {output} to {device}: {err}");
        }
    }
    if !wait_for(|| !native_eq::links_from_node(node).is_empty()) {
        warn!("{output} is not linked to {device}");
    }
    info!("made an EQ for {device} ({output})");
    Ok(())
}

/// The device's description, for the EQ's own.
fn describe(device: &str) -> String {
    pulse::with_sink(|ctrl| ctrl.get_device_by_name(device))
        .ok()
        .and_then(|d| d.description)
        .unwrap_or_else(|| device.to_string())
}

/// Move stream `index` into `sink`: through Pulse, or, for a stream that
/// asked not to be moved, by linking it there and then unlinking it from
/// where it was.
fn move_into(index: u32, sink: &str) -> Result<bool, ControllerError> {
    if pulse::with_sink(|ctrl| ctrl.move_app_by_name(index, sink)).unwrap_or(false) {
        return Ok(true);
    }
    let app = pulse::with_sink(|ctrl| ctrl.get_app_by_index(index))?;
    let Some(node) = app.proplist.get_str("object.id").and_then(|id| id.parse::<u32>().ok()) else {
        return Ok(false);
    };
    Ok(relink(node, sink))
}

fn relink(node: u32, sink: &str) -> bool {
    let Some(sink_node) = engine::find_node_id_by_name(sink) else {
        return false;
    };
    let ports = native_eq::output_ports(node);
    if ports.is_empty() {
        return false;
    }
    let old = native_eq::links_from_node(node);
    let (node_id, sink_id) = (node.to_string(), sink_node.to_string());
    let wanted: usize = ports.iter().map(|p| inputs_for(p).len()).sum();
    for port in &ports {
        for input in inputs_for(port) {
            if let Err(err) = native_eq::create_link(&node_id, port, &sink_id, input) {
                warn!("could not link stream {node} to {sink}: {err}");
            }
        }
    }
    let linked = wait_for(|| native_eq::links_from_node(node).iter().filter(|l| l.input_node == sink_id).count() >= wanted);
    if !linked {
        warn!("stream {node} was not linked to {sink}; leaving it where it was");
        return false;
    }
    for link in old.iter().filter(|l| l.input_node != sink_id) {
        let _ = native_eq::destroy_global(link.id);
    }
    info!("relinked stream {node}, which asks not to be moved, to {sink}");
    true
}

fn wait_for(mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    while start.elapsed() < SETTLE {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    done()
}

/// Remove the device EQs no app has used since their grace ran out, and
/// those whose device is gone, sending their apps back to Default.
pub(crate) fn prune() {
    let Ok(mut routes) = ROUTES.lock() else { return };
    if routes.by_device.is_empty() {
        return;
    }
    let Ok((sinks, apps)) = pulse::with_sink(|ctrl| Ok((ctrl.list_devices()?, ctrl.list_applications()?))) else {
        return;
    };
    let index_of = |name: &str| sinks.iter().find(|d| d.name.as_deref() == Some(name)).map(|d| d.index);
    let mut gone = Vec::new();
    for (device, route) in &routes.by_device {
        let input = route.input();
        let Some(sink) = index_of(&input) else {
            gone.push(device.clone());
            continue;
        };
        let users: Vec<u32> = apps.iter().filter(|a| a.connection_id == sink).map(|a| a.index).collect();
        if index_of(device).is_none() {
            info!("{device} is gone: its apps go back to Default");
            for app in users {
                let _ = move_into(app, config::EQ_VIRTUAL_INPUT_SINK);
            }
        } else if !users.is_empty() || route.made.elapsed() < GRACE {
            continue;
        }
        engine::destroy_node_named(&input);
        gone.push(device.clone());
    }
    for device in gone {
        routes.by_device.remove(&device);
    }
}

/// Give `device`'s EQ, if it has one, a new curve: a new EQ is made, its
/// apps moved into it, and the old one removed.
pub(crate) fn retune(device: &str, bands: &[config::EqBand], preamp: f64) {
    let Ok(mut routes) = ROUTES.lock() else { return };
    let Some(old) = routes.by_device.get(device) else { return };
    let curve = curve_key(bands, preamp);
    if old.curve == curve {
        return;
    }
    let old_input = old.input();
    let generation = routes.next;
    routes.next += 1;
    if let Err(err) = build(device, generation, bands, preamp) {
        warn!("could not retune the EQ for {device}: {err}");
        return;
    }
    let new_input = input_name(generation);
    let users: Vec<u32> = pulse::with_sink(|ctrl| {
        let sink = ctrl.get_device_by_name(&old_input)?.index;
        Ok(ctrl.list_applications()?.into_iter().filter(|a| a.connection_id == sink).map(|a| a.index).collect())
    })
    .unwrap_or_default();
    for app in users {
        let _ = move_into(app, &new_input);
    }
    engine::destroy_node_named(&old_input);
    routes.by_device.insert(device.to_string(), Route { generation, curve, made: Instant::now() });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channels_go_to_their_side_and_mono_to_both() {
        assert_eq!(inputs_for("output_FL"), ["playback_FL"]);
        assert_eq!(inputs_for("output_FR"), ["playback_FR"]);
        assert_eq!(inputs_for("output_RL"), ["playback_FL"]);
        assert_eq!(inputs_for("output_MONO"), ["playback_FL", "playback_FR"]);
        assert_eq!(inputs_for("output_FC"), ["playback_FL", "playback_FR"]);
    }

    #[test]
    fn device_eq_nodes_are_told_apart_from_devices() {
        assert!(is_route_node(&input_name(3)));
        assert!(is_route_node(&output_name(3)));
        assert!(!is_route_node("effect_input.stratum_eq"));
        assert!(!is_route_node("bluez_output.80_C3_BA_87_05_7C.1"));
    }
}

/// Against a private PipeWire only: it loads EQs and moves streams. Start
/// pipewire, wireplumber (hardware monitors off) and pipewire-pulse in their
/// own `XDG_RUNTIME_DIR`, add null sinks `sinkA` (the default) and `sinkB`,
/// set `HOME` to a scratch directory (the EQ presets live there), and run
/// with `STRATUM_AUDIO_SANDBOX=1 cargo test -p stratum_sys live_routing --
/// --ignored --nocapture`.
#[cfg(test)]
mod live {
    use super::*;
    use crate::managers::audio::{self, Stream};
    use std::process::{Child, Command, Stdio};

    fn sleep(ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }

    /// The playing apps as (name, id, device, follows default).
    fn playing() -> Vec<(String, u32, String, bool)> {
        let status = audio::refresh_device_cache();
        status["playback"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| {
                (
                    s["app"].as_str().unwrap().to_string(),
                    s["id"].as_u64().unwrap() as u32,
                    s["device"].as_str().unwrap().to_string(),
                    s["follows_default"].as_bool().unwrap(),
                )
            })
            .collect()
    }

    fn find(name: &str) -> (String, u32, String, bool) {
        playing().into_iter().find(|s| s.0 == name).unwrap_or_else(|| panic!("{name} is not playing: {:?}", playing()))
    }

    /// Ends the players however the test ends.
    struct Players(Vec<Child>);

    impl Drop for Players {
        fn drop(&mut self) {
            for child in &mut self.0 {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// The state pactl reports for sink `name` (RUNNING, IDLE, SUSPENDED).
    fn sink_state(name: &str) -> String {
        let out = String::from_utf8(Command::new("pactl").args(["list", "short", "sinks"]).output().unwrap().stdout).unwrap();
        out.lines().find(|l| l.split('\t').nth(1) == Some(name)).and_then(|l| l.split('\t').next_back()).unwrap_or("").to_string()
    }

    /// This process's resident memory in KB: the EQs run in it.
    fn rss_kb() -> u64 {
        let status = std::fs::read_to_string("/proc/self/status").unwrap();
        let line = status.lines().find(|l| l.starts_with("VmRSS:")).unwrap();
        line.split_whitespace().nth(1).unwrap().parse().unwrap()
    }

    /// The loudness (RMS of s16 samples) on sink `name`, recorded off its
    /// monitor for most of a second.
    fn loudness(name: &str) -> f64 {
        let mut rec = Command::new("parec")
            .args([&format!("--device={name}.monitor"), "--latency-msec=20", "--raw", "--format=s16le", "--channels=2", "--rate=48000"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        sleep(800);
        let _ = rec.kill();
        let out = rec.wait_with_output().unwrap().stdout;
        // Skip the first tenth of a second: the recording starts unsettled.
        let samples: Vec<f64> = out.chunks_exact(2).skip(9600).map(|b| i16::from_le_bytes([b[0], b[1]]) as f64).collect();
        (samples.iter().map(|v| v * v).sum::<f64>() / samples.len().max(1) as f64).sqrt()
    }

    /// A 440 Hz tone, a few seconds of it, as raw s16 stereo.
    fn tone() -> std::path::PathBuf {
        let path = std::env::temp_dir().join("stratum-tone.raw");
        let mut bytes = Vec::new();
        for i in 0..48000 * 30 {
            let v = ((i as f64 * 440.0 * std::f64::consts::TAU / 48000.0).sin() * 8000.0) as i16;
            bytes.extend_from_slice(&v.to_le_bytes());
            bytes.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn play(name: &str, locked: bool) -> Child {
        let mut command = if locked {
            let mut c = Command::new("pw-cat");
            c.args(["-p", "--raw", "--target", config::EQ_VIRTUAL_INPUT_SINK, "--format", "s16", "--rate", "48000", "--channels", "2"]);
            c.args(["-P", &format!("{{ node.name = {name} application.name = {name} node.dont-reconnect = true }}"), "-"]);
            c
        } else {
            // "normal" plays a tone, to hear it on the device it is sent to.
            let source = if name == "normal" { tone() } else { "/dev/zero".into() };
            let mut c = Command::new("paplay");
            c.args(["--raw", &format!("--client-name={name}"), source.to_str().unwrap()]);
            c
        };
        command.stdin(std::fs::File::open("/dev/zero").unwrap()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap()
    }

    #[test]
    #[ignore]
    fn live_routing() {
        let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_default();
        let uid = unsafe { libc_getuid() };
        assert!(
            std::env::var("STRATUM_AUDIO_SANDBOX").as_deref() == Ok("1") && runtime != format!("/run/user/{uid}"),
            "refusing: this moves streams, so it runs only against a private PipeWire"
        );
        audio::initialize();
        sleep(1500);

        // The volume and mute of the main sink, a loopback now, still act.
        let tone = Command::new("paplay")
            .args(["--raw", "--client-name=tone", tone().to_str().unwrap()])
            .spawn()
            .unwrap();
        let _tone = Players(vec![tone]);
        sleep(1000);
        if std::env::var("STRATUM_AUDIO_PAUSE").is_ok() {
            sleep(25_000);
        }
        audio::set_volume(100);
        let full = loudness("sinkA");
        audio::set_volume(25);
        let quarter = loudness("sinkA");
        audio::set_mute(true);
        let muted = loudness("sinkA");
        audio::set_mute(false);
        audio::set_volume(100);
        println!("loudness: {full:.0} at 100 %, {quarter:.0} at 25 %, {muted:.0} muted");
        assert!(full > 1000.0, "the tone reaches the device");
        assert!(quarter < full * 0.2, "the volume lowers it");
        assert!(muted < 10.0, "mute silences it");
        drop(_tone);
        sleep(500);
        let mut players = Players(vec![play("follower", false), play("normal", false), play("stuck", true)]);
        sleep(2000);
        println!("start: {:?}", playing());
        assert!(playing().iter().all(|s| s.2 == "sinkA" && s.3), "all on Default through the main EQ");
        assert_eq!(sink_state("sinkA"), "RUNNING", "sound reaches the device through the sink and the stage");
        let before = rss_kb();

        // Pinned to B, both ways of moving.
        for name in ["normal", "stuck"] {
            let id = find(name).1;
            assert_eq!(audio::move_stream(Stream::Playback(id), "sinkB")["ok"], true, "{name}");
        }
        sleep(500);
        println!("pinned: {:?}", playing());
        assert_eq!(find("normal").2, "sinkB");
        assert!(!find("normal").3);
        assert_eq!(find("stuck").2, "sinkB", "the locked stream was relinked");
        assert_eq!(input_sinks().len(), 1, "one EQ for B, shared");
        assert_eq!(sink_state("sinkB"), "RUNNING", "sound reaches B through its EQ");
        let on_b = loudness("sinkB");
        println!("loudness on B through its EQ: {on_b:.0}");
        assert!(on_b > 1000.0, "the pinned tone is heard on B");
        println!("memory: {} KB before B's EQ, {} KB with it", before, rss_kb());

        // The main output changes: Default follows, the pinned stay.
        assert_eq!(audio::set_output("sinkB")["ok"], true);
        sleep(1500);
        assert_eq!(audio::set_output("sinkA")["ok"], true);
        sleep(1500);
        println!("after switching the output: {:?}", playing());
        assert_eq!(find("follower").2, "sinkA");
        assert!(find("follower").3);
        assert_eq!(find("normal").2, "sinkB", "pinned apps stay");
        assert_eq!(find("stuck").2, "sinkB");

        // Back to Default, the locked one by relinking.
        let id = find("stuck").1;
        assert_eq!(audio::move_stream(Stream::Playback(id), DEFAULT_OUTPUT)["ok"], true);
        sleep(500);
        assert!(find("stuck").3, "{:?}", playing());

        // B's device goes: its app returns to Default and the EQ is removed.
        let module = String::from_utf8(Command::new("pactl").args(["list", "short", "modules"]).output().unwrap().stdout).unwrap();
        let line = module.lines().find(|l| l.contains("sink_name=sinkB")).expect("sinkB's module");
        let index = line.split_whitespace().next().unwrap();
        Command::new("pactl").args(["unload-module", index]).status().unwrap();
        sleep(1000);
        prune();
        sleep(1000);
        println!("after B went: {:?}", playing());
        assert_eq!(find("normal").2, "sinkA");
        assert!(find("normal").3);
        assert!(input_sinks().is_empty(), "B's EQ is gone");

        // Nothing playing: the passive links let the device suspend.
        drop(std::mem::take(&mut players.0).into_iter().map(|mut c| c.kill()).collect::<Vec<_>>());
        sleep(7000);
        assert_eq!(sink_state("sinkA"), "SUSPENDED", "the EQ does not keep the device awake");
    }

    extern "C" {
        #[link_name = "getuid"]
        fn libc_getuid() -> u32;
    }
}
