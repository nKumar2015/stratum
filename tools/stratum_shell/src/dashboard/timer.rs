//! The Focus tab's timer: a countdown, a stopwatch, or Pomodoro rounds.
//!
//! It lives in memory, outside the dashboard's open/closed state, so it keeps
//! running while the dashboard is closed. The event loop asks it how long it
//! may sleep ([`Timer::next_wake`]) and calls [`Timer::tick`], which reports a
//! countdown or a Pomodoro phase reaching zero; the shell turns that into a
//! notification. Every method takes `now`, so the tests need no clock.

use std::time::{Duration, Instant};

/// Pomodoro lengths, from `[dashboard.timer]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pomodoro {
    pub work: Duration,
    pub short_break: Duration,
    pub long_break: Duration,
    /// Work phases before a long break.
    pub rounds: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Work,
    Break,
    LongBreak,
}

impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Phase::Work => "Work",
            Phase::Break => "Break",
            Phase::LongBreak => "Long break",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Countdown,
    Stopwatch,
    /// `round` counts work phases from 1.
    Pomodoro { phase: Phase, round: u32, settings: Pomodoro },
}

/// What reached zero in a [`Timer::tick`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ended {
    /// A countdown of this length finished; the timer now reads zero.
    Countdown(Duration),
    /// A Pomodoro phase finished and the next one has started.
    Phase { finished: Phase, next: Phase, next_length: Duration },
}

impl Ended {
    /// The notification's summary and body.
    pub fn message(&self) -> (String, String) {
        match self {
            Ended::Countdown(length) => ("Timer done".into(), format!("The {} timer is up.", describe(*length))),
            Ended::Phase { finished, next, next_length } => (
                format!("{} done", finished.label()),
                format!("{} for {}.", next.label(), describe(*next_length)),
            ),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Timer {
    mode: Option<Mode>,
    /// The countdown's or phase's length; `None` for a stopwatch.
    total: Option<Duration>,
    /// Time counted before the current run.
    banked: Duration,
    /// When the current run began; `None` while paused or idle.
    since: Option<Instant>,
}

impl Timer {
    pub fn mode(&self) -> Option<Mode> {
        self.mode
    }

    pub fn running(&self) -> bool {
        self.since.is_some()
    }

    /// Set, whether running or paused.
    pub fn active(&self) -> bool {
        self.mode.is_some()
    }

    /// A countdown that has reached zero and waits for a reset.
    pub fn finished(&self, now: Instant) -> bool {
        self.mode == Some(Mode::Countdown) && !self.running() && self.remaining(now) == Some(Duration::ZERO)
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        self.banked + self.since.map_or(Duration::ZERO, |since| now.saturating_duration_since(since))
    }

    /// What is left, for a countdown or a Pomodoro phase.
    pub fn remaining(&self, now: Instant) -> Option<Duration> {
        self.total.map(|total| total.saturating_sub(self.elapsed(now)))
    }

    pub fn total(&self) -> Option<Duration> {
        self.total
    }

    /// The share of a countdown or phase gone by, 0 to 1.
    pub fn progress(&self, now: Instant) -> Option<f32> {
        let total = self.total.filter(|t| !t.is_zero())?;
        Some((self.elapsed(now).as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0))
    }

    pub fn start_countdown(&mut self, length: Duration, now: Instant) {
        *self = Self { mode: Some(Mode::Countdown), total: Some(length), banked: Duration::ZERO, since: Some(now) };
    }

    pub fn start_stopwatch(&mut self, now: Instant) {
        *self = Self { mode: Some(Mode::Stopwatch), total: None, banked: Duration::ZERO, since: Some(now) };
    }

    pub fn start_pomodoro(&mut self, settings: Pomodoro, now: Instant) {
        *self = Self {
            mode: Some(Mode::Pomodoro { phase: Phase::Work, round: 1, settings }),
            total: Some(settings.work),
            banked: Duration::ZERO,
            since: Some(now),
        };
    }

    pub fn pause(&mut self, now: Instant) {
        if let Some(since) = self.since.take() {
            self.banked += now.saturating_duration_since(since);
        }
    }

    /// Continue a paused timer. A finished countdown stays finished.
    pub fn resume(&mut self, now: Instant) {
        if self.active() && !self.running() && !self.finished(now) {
            self.since = Some(now);
        }
    }

    /// Pause a running timer, or resume a paused one.
    pub fn toggle(&mut self, now: Instant) {
        if self.running() {
            self.pause(now);
        } else {
            self.resume(now);
        }
    }

    pub fn stop(&mut self) {
        *self = Self::default();
    }

    /// Back to the start of the countdown or phase, paused; a stopwatch to
    /// zero.
    pub fn reset(&mut self) {
        self.banked = Duration::ZERO;
        self.since = None;
    }

    /// Lengthen a countdown or the current phase. A finished countdown
    /// starts again with what was added.
    pub fn add(&mut self, extra: Duration, now: Instant) {
        let finished = self.finished(now);
        if let Some(total) = &mut self.total {
            *total += extra;
            if finished {
                self.since = Some(now);
            }
        }
    }

    /// Whether a countdown or phase reached zero since the last tick. A
    /// countdown stops at zero; Pomodoro moves to the next phase, starting
    /// it from the moment the last one ended.
    pub fn tick(&mut self, now: Instant) -> Option<Ended> {
        let since = self.since?;
        let total = self.total?;
        if self.elapsed(now) < total {
            return None;
        }
        let ended_at = since + total.saturating_sub(self.banked);
        match self.mode? {
            Mode::Stopwatch => None,
            Mode::Countdown => {
                self.since = None;
                self.banked = total;
                Some(Ended::Countdown(total))
            }
            Mode::Pomodoro { phase, round, settings } => {
                let (next, round) = match phase {
                    Phase::Work if round >= settings.rounds => (Phase::LongBreak, round),
                    Phase::Work => (Phase::Break, round),
                    Phase::Break => (Phase::Work, round + 1),
                    Phase::LongBreak => (Phase::Work, 1),
                };
                let length = match next {
                    Phase::Work => settings.work,
                    Phase::Break => settings.short_break,
                    Phase::LongBreak => settings.long_break,
                };
                self.mode = Some(Mode::Pomodoro { phase: next, round, settings });
                self.total = Some(length);
                self.banked = Duration::ZERO;
                self.since = Some(ended_at.min(now));
                Some(Ended::Phase { finished: phase, next, next_length: length })
            }
        }
    }

    /// How long the loop may sleep: until the shown second changes when
    /// `shown`, else until a countdown or phase ends. `None` when stopped.
    pub fn next_wake(&self, now: Instant, shown: bool) -> Option<Duration> {
        self.since?;
        let to_end = self.remaining(now);
        if !shown {
            return to_end;
        }
        // A countdown shows whole seconds rounded up, a stopwatch rounded
        // down; either way the label changes on the next whole second.
        let part = match to_end {
            Some(left) => left.subsec_nanos(),
            None => 1_000_000_000 - self.elapsed(now).subsec_nanos(),
        };
        let part = Duration::from_nanos(u64::from(part));
        Some(if part.is_zero() { Duration::from_secs(1) } else { part })
    }

    /// The face: what is left of a countdown or phase (rounded up, so it
    /// reads 25:00 at the start and 0:01 until the end), or the stopwatch's
    /// time.
    pub fn face(&self, now: Instant) -> String {
        let seconds = match self.remaining(now) {
            Some(left) => left.as_secs() + u64::from(left.subsec_nanos() > 0),
            None => self.elapsed(now).as_secs(),
        };
        clock(seconds)
    }
}

/// `m:ss`, or `h:mm:ss` from an hour.
pub fn clock(seconds: u64) -> String {
    let (h, m, s) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// "25 min", "1 h 30 min", "90 s".
pub fn describe(length: Duration) -> String {
    let seconds = length.as_secs();
    let (h, m, s) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    let mut parts = Vec::new();
    if h > 0 {
        parts.push(format!("{h} h"));
    }
    if m > 0 {
        parts.push(format!("{m} min"));
    }
    if s > 0 || parts.is_empty() {
        parts.push(format!("{s} s"));
    }
    parts.join(" ")
}

/// A length as `stratum-cli timer start` takes it: `25m`, `90s`, `1h30m`,
/// or a bare number of minutes. Zero is refused.
pub fn parse_duration(text: &str) -> Option<Duration> {
    let text = text.trim().to_ascii_lowercase();
    if let Ok(minutes) = text.parse::<u64>() {
        return minutes.checked_mul(60).filter(|s| *s > 0).map(Duration::from_secs);
    }
    let mut total = 0u64;
    let mut number = String::new();
    for ch in text.chars() {
        match ch {
            '0'..='9' => number.push(ch),
            'h' | 'm' | 's' => {
                let value: u64 = number.parse().ok()?;
                let unit = match ch {
                    'h' => 3600,
                    'm' => 60,
                    _ => 1,
                };
                total = total.checked_add(value.checked_mul(unit)?)?;
                number.clear();
            }
            ' ' => {}
            _ => return None,
        }
    }
    (number.is_empty() && total > 0).then(|| Duration::from_secs(total))
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_secs(1);

    fn pomodoro() -> Pomodoro {
        Pomodoro { work: 25 * 60 * S, short_break: 5 * 60 * S, long_break: 15 * 60 * S, rounds: 2 }
    }

    #[test]
    fn durations_parse_like_the_cli_documents() {
        assert_eq!(parse_duration("25"), Some(25 * 60 * S));
        assert_eq!(parse_duration("25m"), Some(25 * 60 * S));
        assert_eq!(parse_duration("90s"), Some(90 * S));
        assert_eq!(parse_duration("1h30m"), Some(90 * 60 * S));
        assert_eq!(parse_duration("1H 5S"), Some(3605 * S));
        for bad in ["", "0", "0m", "m", "5x", "5m3", "-5", "1.5m"] {
            assert_eq!(parse_duration(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn faces_read_as_clocks() {
        assert_eq!(clock(0), "0:00");
        assert_eq!(clock(25 * 60), "25:00");
        assert_eq!(clock(3600 + 61), "1:01:01");
        assert_eq!(describe(25 * 60 * S), "25 min");
        assert_eq!(describe(5430 * S), "1 h 30 min 30 s");
        assert_eq!(describe(Duration::ZERO), "0 s");
    }

    #[test]
    fn a_countdown_rounds_up_pauses_and_ends_at_zero() {
        let t0 = Instant::now();
        let mut timer = Timer::default();
        timer.start_countdown(60 * S, t0);
        assert_eq!(timer.face(t0), "1:00");
        assert_eq!(timer.face(t0 + Duration::from_millis(500)), "1:00", "rounded up");
        assert_eq!(timer.face(t0 + 59 * S + Duration::from_millis(900)), "0:01");
        timer.pause(t0 + 10 * S);
        assert_eq!(timer.remaining(t0 + 1000 * S), Some(50 * S), "paused time does not count");
        timer.resume(t0 + 20 * S);
        assert_eq!(timer.tick(t0 + 69 * S), None);
        assert_eq!(timer.tick(t0 + 70 * S), Some(Ended::Countdown(60 * S)));
        assert!(timer.finished(t0 + 71 * S) && !timer.running());
        assert_eq!(timer.tick(t0 + 80 * S), None, "ends once");
        timer.resume(t0 + 80 * S);
        assert!(!timer.running(), "a finished countdown does not resume");
        timer.add(60 * S, t0 + 90 * S);
        assert_eq!((timer.running(), timer.face(t0 + 90 * S)), (true, "1:00".into()), "+1 min restarts it");
    }

    #[test]
    fn a_stopwatch_counts_up_and_never_ends() {
        let t0 = Instant::now();
        let mut timer = Timer::default();
        timer.start_stopwatch(t0);
        assert_eq!(timer.face(t0 + 61 * S + Duration::from_millis(999)), "1:01");
        assert_eq!(timer.tick(t0 + 100_000 * S), None);
        assert_eq!(timer.next_wake(t0 + Duration::from_millis(300), true), Some(Duration::from_millis(700)));
        assert_eq!(timer.next_wake(t0, false), None, "nothing to wake for when not shown");
        timer.reset();
        assert_eq!((timer.face(t0 + 5 * S), timer.running()), ("0:00".into(), false));
    }

    #[test]
    fn pomodoro_cycles_through_breaks_without_losing_time() {
        let t0 = Instant::now();
        let mut timer = Timer::default();
        timer.start_pomodoro(pomodoro(), t0);
        // The loop woke two seconds late: the break is two seconds in.
        let late = t0 + 25 * 60 * S + 2 * S;
        let ended = timer.tick(late).unwrap();
        assert_eq!(ended, Ended::Phase { finished: Phase::Work, next: Phase::Break, next_length: 5 * 60 * S });
        assert_eq!(timer.remaining(late), Some(5 * 60 * S - 2 * S));
        assert_eq!(ended.message().0, "Work done");
        let t1 = t0 + 30 * 60 * S;
        assert!(matches!(timer.tick(t1), Some(Ended::Phase { next: Phase::Work, .. })));
        assert!(matches!(timer.mode(), Some(Mode::Pomodoro { round: 2, .. })));
        // The second work round is the last before the long break.
        let t2 = t1 + 25 * 60 * S;
        assert!(matches!(timer.tick(t2), Some(Ended::Phase { next: Phase::LongBreak, .. })));
        assert!(matches!(timer.tick(t2 + 15 * 60 * S), Some(Ended::Phase { next: Phase::Work, .. })));
        assert!(matches!(timer.mode(), Some(Mode::Pomodoro { round: 1, phase: Phase::Work, .. })));
    }

    #[test]
    fn closed_it_sleeps_until_the_end_and_shown_until_the_next_second() {
        let t0 = Instant::now();
        let mut timer = Timer::default();
        assert_eq!(timer.next_wake(t0, true), None);
        timer.start_countdown(10 * S, t0);
        let now = t0 + Duration::from_millis(2_250);
        assert_eq!(timer.next_wake(now, false), Some(Duration::from_millis(7_750)));
        assert_eq!(timer.next_wake(now, true), Some(Duration::from_millis(750)));
        timer.pause(now);
        assert_eq!(timer.next_wake(now, true), None, "paused, nothing changes");
    }

    #[test]
    fn progress_and_stop() {
        let t0 = Instant::now();
        let mut timer = Timer::default();
        timer.start_countdown(100 * S, t0);
        assert_eq!(timer.progress(t0 + 25 * S), Some(0.25));
        timer.stop();
        assert!(!timer.active());
        assert_eq!(timer.progress(t0), None);
    }
}
