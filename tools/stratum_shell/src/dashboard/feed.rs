//! The dashboard's data: resource samples taken on a thread of their own,
//! and the history its graphs draw.
//!
//! The sampler records every `sample_ms`, also while the dashboard is closed
//! when `background_history` is on, so the graphs are full when it opens.
//! It publishes into `SystemState::dashboard` only while the dashboard is
//! open: a closed one costs a few file reads per sample and never wakes the
//! event loop. GPUs are read only while it is open (a reading costs ~20 ms
//! through NVML), so their history covers that time alone. Opening samples
//! at once instead of waiting for the next interval.
//!
//! Now-playing data is refreshed with each published sample, as the QML
//! dashboard's `watch` did, because `music::status` runs `playerctl`.

use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use stratum_sys::managers::gpu::Load;
use stratum_sys::managers::sysmon::{Sample, Sampler};

use crate::sys_state::SystemState;

/// What the dashboard paints from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// The latest sample; `None` until the first.
    pub sample: Option<Sample>,
    pub history: History,
}

/// Recent readings, oldest first: loads as percentages, network in bytes
/// per second.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    pub cpu: VecDeque<f32>,
    pub memory: VecDeque<f32>,
    /// One series per GPU, in the sample's order.
    pub gpus: Vec<VecDeque<f32>>,
    pub rx: VecDeque<f32>,
    pub tx: VecDeque<f32>,
}

impl History {
    /// Append `sample`, keeping at most `len` readings per series.
    pub fn push(&mut self, sample: &Sample, len: usize) {
        fn push(series: &mut VecDeque<f32>, value: f32, len: usize) {
            series.push_back(value);
            while series.len() > len {
                series.pop_front();
            }
        }
        // The first sample has no CPU load yet; leave a gap rather than a
        // false zero.
        if let Some(cpu) = sample.cpu_percent {
            push(&mut self.cpu, cpu, len);
        }
        push(&mut self.memory, sample.memory.percent(), len);
        if let Some(network) = sample.network {
            push(&mut self.rx, network.rx as f32, len);
            push(&mut self.tx, network.tx as f32, len);
        }
        // GPUs are sampled only while the dashboard is open, so a closed
        // sample (none) ends their series and the next open starts afresh.
        // A GPU appearing or going away renumbers them too.
        if self.gpus.len() != sample.gpus.len() {
            self.gpus = vec![VecDeque::new(); sample.gpus.len()];
        }
        for (series, gpu) in self.gpus.iter_mut().zip(&sample.gpus) {
            let value = match gpu.load {
                Load::Percent(p) => p.clamp(0, 100) as f32,
                Load::Asleep | Load::Unknown => 0.0,
            };
            push(series, value, len);
        }
    }
}

/// How the sampler runs, from `[dashboard]`.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub sample_ms: u64,
    pub history: usize,
    pub background: bool,
    /// Fetched while open, when due.
    pub weather: crate::config::WeatherConfig,
    pub calendar: crate::config::CalendarConfig,
}

impl Settings {
    pub fn from_config(config: &crate::config::DashboardConfig) -> Self {
        Self {
            sample_ms: config.sample_ms,
            history: config.history,
            background: config.background_history,
            weather: config.weather.clone(),
            calendar: config.calendar.clone(),
        }
    }
}

/// The UI thread's handle on the sampler: whether the dashboard is open, and
/// how to sample.
#[derive(Default)]
pub struct Feed {
    state: Mutex<FeedState>,
    wake: Condvar,
}

#[derive(Default)]
struct FeedState {
    open: bool,
    /// Set when the dashboard opens, until the sampler has published.
    opened: bool,
    /// `None` until the shell has read its config: the sampler waits.
    settings: Option<Settings>,
    /// The player picked on the dashboard; `None` follows whichever plays.
    player: Option<String>,
}

impl Feed {
    fn lock(&self) -> MutexGuard<'_, FeedState> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Called every frame; only a change wakes the sampler.
    pub fn set_open(&self, open: bool) {
        let mut state = self.lock();
        if state.open != open {
            state.open = open;
            state.opened |= open;
            self.wake.notify_all();
        }
    }

    /// Called every frame with the dashboard's picked player.
    pub fn set_player(&self, player: Option<&str>) {
        let mut state = self.lock();
        if state.player.as_deref() != player {
            state.player = player.map(str::to_string);
        }
    }

    /// Called every frame, and so on every config reload.
    pub fn configure(&self, settings: Settings) {
        let mut state = self.lock();
        if state.settings.as_ref() != Some(&settings) {
            state.settings = Some(settings);
            self.wake.notify_all();
        }
    }

    /// Run the sampler on its own thread for the life of the shell.
    pub fn spawn(self: &Arc<Self>, state: Arc<SystemState>) {
        let feed = Arc::clone(self);
        std::thread::Builder::new()
            .name("stratum-dashboard".into())
            .spawn(move || feed.run(&state))
            .ok();
    }

    fn run(&self, state: &Arc<SystemState>) {
        let mut sampler = Sampler::new();
        let mut history = History::default();
        let mut last: Option<Sample> = None;
        let mut next = Instant::now();
        let mut guard = self.lock();
        loop {
            // Sleep until a sample is due or the dashboard opens.
            let settings = loop {
                let active = guard.settings.as_ref().is_some_and(|s| guard.open || s.background);
                let now = Instant::now();
                if let Some(settings) = guard.settings.as_ref().filter(|_| guard.opened || (active && now >= next)) {
                    break settings.clone();
                }
                guard = if active {
                    self.wake.wait_timeout(guard, next - now).map_or_else(|p| p.into_inner().0, |r| r.0)
                } else {
                    self.wake.wait(guard).unwrap_or_else(|p| p.into_inner())
                };
            };
            let opened = std::mem::take(&mut guard.opened);
            let open = guard.open;
            let player = guard.player.clone();
            drop(guard);

            // Without background sampling, what was recorded before the
            // dashboard last closed is stale: start the graphs afresh.
            if opened && !settings.background {
                (sampler, history, last) = (Sampler::new(), History::default(), None);
            }
            // Opening samples at once: background samples carry no GPUs.
            let now = Instant::now();
            if now >= next || opened || last.is_none() {
                let mut sample = sampler.sample(open);
                history.push(&sample, settings.history);
                // Sampled just after a background sample, CPU time may not
                // have moved yet; show the last load rather than none.
                if sample.cpu_percent.is_none() {
                    sample.cpu_percent = last.and_then(|s| s.cpu_percent);
                }
                last = Some(sample);
                next = now + Duration::from_millis(settings.sample_ms);
            }
            if open {
                state.weather_feed.refresh_if_due(state, &settings.weather);
                state.calendar_feed.refresh_if_due(state, &settings.calendar);
                state.music.set(stratum_sys::managers::music::status_for(player.as_deref()));
                state.dashboard.set(Snapshot { sample: last.clone(), history: history.clone() });
                state.notify();
            }
            guard = self.lock();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stratum_sys::managers::gpu::GpuReading;
    use stratum_sys::managers::sysmon::{Rates, Usage};

    fn sample(cpu: Option<f32>, gpus: usize) -> Sample {
        Sample {
            cpu_percent: cpu,
            memory: Usage { used: 1, total: 4 },
            root: Usage::default(),
            network: cpu.map(|_| Rates { rx: 1000.0, tx: 10.0 }),
            temperatures: Default::default(),
            uptime: None,
            gpus: (0..gpus)
                .map(|_| GpuReading { name: "GPU".into(), load: Load::Percent(50), temperature: None, vram: None })
                .collect(),
        }
    }

    #[test]
    fn network_rates_join_once_there_are_two_readings() {
        let mut history = History::default();
        history.push(&sample(None, 0), 3);
        assert!(history.rx.is_empty());
        history.push(&sample(Some(1.0), 0), 3);
        assert_eq!((history.rx[0], history.tx[0]), (1000.0, 10.0));
    }

    #[test]
    fn history_keeps_the_newest_readings() {
        let mut history = History::default();
        for cpu in [10.0, 20.0, 30.0, 40.0] {
            history.push(&sample(Some(cpu), 1), 3);
        }
        assert_eq!(history.cpu, [20.0, 30.0, 40.0]);
        assert_eq!(history.memory, [25.0, 25.0, 25.0]);
        assert_eq!(history.gpus, vec![VecDeque::from([50.0, 50.0, 50.0])]);
    }

    #[test]
    fn a_first_sample_without_cpu_load_leaves_no_point() {
        let mut history = History::default();
        history.push(&sample(None, 0), 3);
        assert!(history.cpu.is_empty());
        assert_eq!(history.memory.len(), 1);
    }

    #[test]
    fn a_change_in_gpus_restarts_their_series() {
        let mut history = History::default();
        history.push(&sample(Some(1.0), 1), 3);
        history.push(&sample(Some(1.0), 2), 3);
        assert_eq!(history.gpus.len(), 2);
        assert!(history.gpus.iter().all(|s| s.len() == 1));
    }

    #[test]
    fn opening_wakes_the_sampler_once() {
        let feed = Feed::default();
        feed.set_open(true);
        assert!(feed.lock().opened);
        feed.lock().opened = false;
        feed.set_open(true);
        assert!(!feed.lock().opened, "already open: nothing to publish early");
        feed.set_open(false);
        assert!(!feed.lock().open);
    }
}
