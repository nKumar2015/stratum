//! Download and upload rates for the Network page's graph.
//!
//! A sampler thread reads the physical interfaces' byte counters once a
//! second, keeps a minute of rates, and refreshes the page's VPN, wired and
//! hotspot snapshot every few seconds. It runs only while the page is
//! painted: each paint marks the watch, and the thread stops a few seconds
//! after the last, so a closed page costs nothing.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::sys_state::SystemState;

/// Readings kept: a minute at one a second.
pub const CAPACITY: usize = 60;
const EVERY: Duration = Duration::from_secs(1);
/// How long the sampler outlives the page's last paint.
const LINGER: Duration = Duration::from_secs(3);
/// The connections snapshot is read every this many samples.
const CONNECTIONS_EVERY: u32 = 5;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NetRates {
    /// Bytes per second, oldest first.
    pub down: VecDeque<f32>,
    pub up: VecDeque<f32>,
}

/// Whether the sampler runs, and when the page last painted.
#[derive(Debug, Default)]
pub struct Watch {
    running: AtomicBool,
    seen: Mutex<Option<Instant>>,
}

impl Watch {
    /// The page painted: keep sampling. Returns whether the sampler runs.
    pub fn touch(&self) -> bool {
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some(Instant::now());
        }
        self.running.load(Ordering::Acquire)
    }

    fn wanted(&self) -> bool {
        self.seen.lock().ok().and_then(|s| *s).is_some_and(|t| t.elapsed() < LINGER)
    }
}

/// Start the sampler unless it runs.
pub fn start(state: &Arc<SystemState>) {
    if state.net_rates_watch.running.swap(true, Ordering::AcqRel) {
        return;
    }
    let state = Arc::clone(state);
    let spawned = std::thread::Builder::new().name("stratum-netrates".into()).spawn(move || {
        run(&state);
        state.net_rates_watch.running.store(false, Ordering::Release);
    });
    if spawned.is_err() {
        // Not running after all; the next paint asks again.
        tracing::warn!("could not start the network rate sampler");
    }
}

fn run(state: &SystemState) {
    let mut rates = NetRates::default();
    let mut last: Option<(Instant, u64, u64)> = None;
    let mut count = 0u32;
    while state.net_rates_watch.wanted() {
        if count.is_multiple_of(CONNECTIONS_EVERY) {
            state.connections.set(stratum_sys::managers::connections::status());
        }
        count += 1;
        let devices = interfaces();
        let (down, up) = devices.iter().map(|d| counters(d)).fold((0, 0), |(a, b), (c, d)| (a + c, b + d));
        let now = Instant::now();
        if let Some((then, last_down, last_up)) = last {
            let dt = now.duration_since(then).as_secs_f32();
            push(&mut rates.down, rate(last_down, down, dt));
            push(&mut rates.up, rate(last_up, up, dt));
            state.net_rates.set(rates.clone());
        }
        last = Some((now, down, up));
        std::thread::sleep(EVERY);
    }
    // A later showing starts afresh rather than with a gap in the line.
    state.net_rates.set(NetRates::default());
}

/// The physical interfaces that are up: wired and Wi-Fi cards. Virtual
/// ones (a VPN's tunnel, bridges, loopback) carry traffic that also passes
/// through a card, or none of the machine's, so they are left out.
fn interfaces() -> Vec<String> {
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.path().join("device").exists())
        .filter(|e| std::fs::read_to_string(e.path().join("operstate")).is_ok_and(|s| s.trim() == "up"))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect()
}

/// An interface's received and sent byte counts.
fn counters(device: &str) -> (u64, u64) {
    // An interface name from NetworkManager never holds a slash, but a path
    // is built from it: refuse one that would leave the directory.
    if device.contains('/') || device.starts_with('.') {
        return (0, 0);
    }
    let read = |name: &str| {
        std::fs::read_to_string(format!("/sys/class/net/{device}/statistics/{name}"))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };
    (read("rx_bytes"), read("tx_bytes"))
}

/// Bytes per second between two counter readings `dt` seconds apart. A
/// counter that went back (an interface that came and went) reads as 0.
pub fn rate(before: u64, after: u64, dt: f32) -> f32 {
    if dt <= 0.0 {
        return 0.0;
    }
    after.saturating_sub(before) as f32 / dt
}

fn push(history: &mut VecDeque<f32>, value: f32) {
    history.push_back(value);
    while history.len() > CAPACITY {
        history.pop_front();
    }
}

/// A rate for people: `0 B/s`, `12.3 KB/s`, `4.5 MB/s`.
pub fn format_rate(bytes_per_s: f32) -> String {
    match bytes_per_s {
        b if b < 1024.0 => format!("{} B/s", b.round() as u64),
        b if b < 1024.0 * 1024.0 => format!("{:.1} KB/s", b / 1024.0),
        b => format!("{:.1} MB/s", b / (1024.0 * 1024.0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_come_from_counter_differences() {
        assert_eq!(rate(1000, 3000, 2.0), 1000.0);
        assert_eq!(rate(5000, 10, 1.0), 0.0, "a reset counter");
        assert_eq!(rate(0, 10, 0.0), 0.0);
    }

    #[test]
    fn history_keeps_a_minute() {
        let mut h = VecDeque::new();
        for i in 0..100 {
            push(&mut h, i as f32);
        }
        assert_eq!(h.len(), CAPACITY);
        assert_eq!(h.front(), Some(&40.0));
    }

    #[test]
    fn rates_read_naturally() {
        assert_eq!(format_rate(512.0), "512 B/s");
        assert_eq!(format_rate(1536.0), "1.5 KB/s");
        assert_eq!(format_rate(3.0 * 1024.0 * 1024.0), "3.0 MB/s");
    }

    #[test]
    fn counters_refuse_paths() {
        assert_eq!(counters("../etc"), (0, 0));
    }
}
