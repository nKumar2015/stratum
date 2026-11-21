//! Privacy indicators: who is using the microphone or the camera, and
//! whether the screen is being shared, for the bar's dots and their menu.
//!
//! - The microphone comes from the audio snapshot's `recording` streams
//!   (pipewire-pulse lists native PipeWire ones too), less any paused one
//!   and any that records a sink's monitor rather than a microphone.
//! - The camera is whoever holds `/dev/video*` open: an inotify watch sees
//!   every open and close, by an app using V4L2 itself or by PipeWire for
//!   an app using it, and a scan of `/proc` then names the holders. Nothing
//!   holds a camera open while it is idle, so this costs nothing until one
//!   is used. When PipeWire holds it, the apps taking video from PipeWire
//!   (the audio monitor's `video` channel) are named instead.
//! - A screen being shared is the desktop portal's screen-cast stream, a
//!   PipeWire video source with no device behind it (same channel).
//!
//! [`Indicators::from_state`] is pure and tested.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::warn;

use crate::sys_state::SystemState;

/// What the indicator shows, each with the apps responsible.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Indicators {
    pub microphone: Vec<String>,
    pub camera: Vec<String>,
    /// Whether a screen or window is being shared.
    pub screen: bool,
}

impl Indicators {
    pub fn any(&self) -> bool {
        !self.microphone.is_empty() || !self.camera.is_empty() || self.screen
    }

    /// From the audio snapshot, the `video` channel and the camera's
    /// holders (process names).
    pub fn from_state(audio: &serde_json::Value, video: &serde_json::Value, holders: &[String]) -> Self {
        let microphone: BTreeSet<String> = audio["recording"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["corked"].as_bool() != Some(true))
            .filter(|s| !s["device"].as_str().unwrap_or_default().ends_with(".monitor"))
            .filter_map(|s| s["app"].as_str().map(str::to_string))
            .collect();
        // PipeWire and its session manager only open the camera for an app.
        let through_pipewire = holders.iter().any(|h| is_pipewire(h));
        let mut camera: BTreeSet<String> = holders.iter().filter(|h| !is_pipewire(h)).cloned().collect();
        if through_pipewire {
            let clients = video["clients"].as_array().into_iter().flatten().filter_map(|c| c.as_str());
            let before = camera.len();
            camera.extend(clients.map(str::to_string));
            if camera.len() == before {
                camera.insert("An app through PipeWire".into());
            }
        }
        Self {
            microphone: microphone.into_iter().collect(),
            camera: camera.into_iter().collect(),
            screen: video["screens"].as_u64().unwrap_or(0) > 0,
        }
    }
}

fn is_pipewire(process: &str) -> bool {
    // `comm` is cut at 15 bytes; a wrapped binary starts with a dot.
    let process = process.trim_start_matches('.');
    process.starts_with("pipewire") || process.starts_with("wireplumber")
}

/// Where camera devices are: `/dev`, or in debug builds
/// `STRATUM_VIDEO_DIR`, so a test opens a stand-in file rather than the
/// real camera.
fn video_dir() -> PathBuf {
    #[cfg(debug_assertions)]
    if let Some(dir) = std::env::var_os("STRATUM_VIDEO_DIR") {
        return dir.into();
    }
    PathBuf::from("/dev")
}

fn is_video(name: &str) -> bool {
    name.strip_prefix("video").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Watch the cameras on a thread of its own and publish their holders.
pub fn start(state: Arc<SystemState>) {
    let spawned = std::thread::Builder::new()
        .name("stratum-camera".into())
        .stack_size(128 * 1024)
        .spawn(move || {
            if let Err(err) = watch(&state, &video_dir()) {
                warn!("privacy: not watching the camera: {err}");
            }
        });
    if let Err(err) = spawned {
        warn!("privacy: failed to start the camera watch: {err}");
    }
}

fn watch(state: &SystemState, dir: &Path) -> std::io::Result<()> {
    use std::os::unix::ffi::OsStrExt;
    // SAFETY: plain syscalls; the descriptor is closed by `Fd`'s drop.
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    struct Fd(i32);
    impl Drop for Fd {
        fn drop(&mut self) {
            // SAFETY: closing our own descriptor once.
            unsafe { libc::close(self.0) };
        }
    }
    let fd = Fd(fd);
    let add = |path: &Path, mask: u32| -> i32 {
        let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return -1;
        };
        // SAFETY: a NUL-terminated path.
        unsafe { libc::inotify_add_watch(fd.0, c.as_ptr(), mask) }
    };
    const USE: u32 = libc::IN_OPEN | libc::IN_CLOSE_WRITE | libc::IN_CLOSE_NOWRITE;
    // The directory, for cameras plugged in later.
    let dir_watch = add(dir, libc::IN_CREATE);
    for entry in std::fs::read_dir(dir)?.flatten() {
        if is_video(&entry.file_name().to_string_lossy()) {
            add(&entry.path(), USE);
        }
    }
    publish(state, dir);
    let mut buffer = [0u8; 4096];
    loop {
        // SAFETY: reading into our own buffer.
        let read = unsafe { libc::read(fd.0, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        for (wd, name) in events(&buffer[..read as usize]) {
            if wd == dir_watch && is_video(&name) {
                add(&dir.join(&name), USE);
            }
        }
        publish(state, dir);
    }
}

/// `(watch, name)` for each `inotify_event` in a buffer.
fn events(mut buffer: &[u8]) -> Vec<(i32, String)> {
    const HEADER: usize = 16; // wd, mask, cookie, len
    let mut out = Vec::new();
    while buffer.len() >= HEADER {
        let wd = i32::from_ne_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]);
        let len = u32::from_ne_bytes([buffer[12], buffer[13], buffer[14], buffer[15]]) as usize;
        let Some(name) = buffer.get(HEADER..HEADER + len) else {
            break;
        };
        let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        out.push((wd, String::from_utf8_lossy(&name[..end]).into_owned()));
        buffer = &buffer[HEADER + len..];
    }
    out
}

fn publish(state: &SystemState, dir: &Path) {
    if state.camera.set(holders(dir)) {
        state.notify();
    }
}

/// The names of the processes holding a camera in `dir` open, from their
/// file descriptors. Only this user's processes can be read, which are the
/// ones a desktop's camera users are.
fn holders(dir: &Path) -> Vec<String> {
    let me = std::process::id();
    let mut names = BTreeSet::new();
    let Ok(procs) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    for process in procs.flatten() {
        let Some(pid) = process.file_name().to_str().and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        if pid == me {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(process.path().join("fd")) else {
            continue;
        };
        let holds = fds.flatten().any(|fd| {
            std::fs::read_link(fd.path()).is_ok_and(|target| {
                target.parent() == Some(dir) && target.file_name().is_some_and(|n| is_video(&n.to_string_lossy()))
            })
        });
        if holds {
            if let Ok(comm) = std::fs::read_to_string(process.path().join("comm")) {
                names.insert(comm.trim().to_string());
            }
        }
    }
    names.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn microphones_are_recording_apps_but_not_paused_or_monitors() {
        let audio = json!({"recording": [
            {"app": "Discord", "device": "alsa_input.usb-fifine", "corked": false},
            {"app": "Discord", "device": "alsa_input.usb-fifine", "corked": false},
            {"app": "OBS", "device": "alsa_output.pci.monitor", "corked": false},
            {"app": "Firefox", "device": "alsa_input.usb-fifine", "corked": true},
        ]});
        let shown = Indicators::from_state(&audio, &json!(null), &[]);
        assert_eq!(shown.microphone, ["Discord"]);
        assert!(shown.camera.is_empty() && !shown.screen);
        assert!(shown.any());
    }

    #[test]
    fn the_camera_names_apps_behind_pipewire() {
        let video = json!({"screens": 0, "clients": ["Firefox"]});
        let direct = Indicators::from_state(&json!({}), &video, &["zoom".into()]);
        assert_eq!(direct.camera, ["zoom"]);
        let piped = Indicators::from_state(&json!({}), &video, &["pipewire".into()]);
        assert_eq!(piped.camera, ["Firefox"]);
        let unnamed = Indicators::from_state(&json!({}), &json!({}), &["pipewire".into()]);
        assert_eq!(unnamed.camera, ["An app through PipeWire"]);
    }

    #[test]
    fn a_screen_cast_is_a_shared_screen() {
        let shown = Indicators::from_state(&json!({}), &json!({"screens": 1, "clients": []}), &[]);
        assert!(shown.screen && shown.any());
        assert!(!Indicators::default().any());
    }

    #[test]
    fn video_devices_by_name() {
        assert!(is_video("video0") && is_video("video12"));
        assert!(!is_video("video") && !is_video("video0p") && !is_video("vhci"));
    }
}
