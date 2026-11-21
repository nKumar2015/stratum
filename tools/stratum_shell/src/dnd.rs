//! Do Not Disturb on a schedule, as a phone does it: on at the start of
//! the window, off at its end, and a toggle by hand in between holds until
//! the next of those. Only crossings change anything, so a schedule never
//! fights the user.
//!
//! Pure and tested; the shell feeds it the time once per loop iteration.

/// Whether `now` is in the window from `start` to `end` (minutes since
/// midnight). A window may run past midnight; one whose start is its end
/// is empty.
pub fn in_window(now: u32, start: u32, end: u32) -> bool {
    match start.cmp(&end) {
        std::cmp::Ordering::Equal => false,
        std::cmp::Ordering::Less => (start..end).contains(&now),
        std::cmp::Ordering::Greater => now >= start || now < end,
    }
}

#[derive(Debug, Default)]
pub struct Schedule {
    /// Whether the last look was inside the window; `None` before the
    /// first, and while the schedule is off.
    inside: Option<bool>,
}

impl Schedule {
    /// Look at the time. Returns what Do Not Disturb should become, when a
    /// boundary was crossed: starting inside the window (the shell started,
    /// or the schedule was turned on, during it) turns it on, and starting
    /// outside leaves it as it is.
    pub fn step(&mut self, enabled: bool, now: u32, start: u32, end: u32) -> Option<bool> {
        if !enabled {
            self.inside = None;
            return None;
        }
        let inside = in_window(now, start, end);
        let before = self.inside.replace(inside);
        match before {
            None => inside.then_some(true),
            Some(was) if was != inside => Some(inside),
            Some(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn hm(h: u32, m: u32) -> u32 {
        h * 60 + m
    }

    #[test]
    fn windows_may_run_past_midnight() {
        let (night_start, night_end) = (hm(22, 0), hm(7, 0));
        assert!(in_window(hm(23, 30), night_start, night_end));
        assert!(in_window(hm(3, 0), night_start, night_end));
        assert!(!in_window(hm(7, 0), night_start, night_end), "the end is outside");
        assert!(in_window(hm(22, 0), night_start, night_end), "the start is inside");
        assert!(!in_window(hm(12, 0), night_start, night_end));
        assert!(in_window(hm(10, 0), hm(9, 0), hm(17, 0)), "a daytime window");
        assert!(!in_window(hm(10, 0), hm(9, 0), hm(9, 0)), "an empty one");
    }

    #[test]
    fn only_crossings_change_it() {
        let mut schedule = Schedule::default();
        let (start, end) = (hm(22, 0), hm(7, 0));
        assert_eq!(schedule.step(true, hm(12, 0), start, end), None, "starting outside leaves it");
        assert_eq!(schedule.step(true, hm(21, 59), start, end), None);
        assert_eq!(schedule.step(true, hm(22, 0), start, end), Some(true), "on at the start");
        // The user turns it off at 23:00; the schedule does not fight that.
        assert_eq!(schedule.step(true, hm(23, 0), start, end), None);
        assert_eq!(schedule.step(true, hm(7, 0), start, end), Some(false), "off at the end");
    }

    #[test]
    fn starting_inside_turns_it_on_and_turning_it_off_forgets() {
        let mut schedule = Schedule::default();
        let (start, end) = (hm(22, 0), hm(7, 0));
        assert_eq!(schedule.step(true, hm(23, 0), start, end), Some(true));
        assert_eq!(schedule.step(false, hm(23, 1), start, end), None, "off: no say");
        assert_eq!(schedule.step(true, hm(23, 2), start, end), Some(true), "back on inside: on again");
    }
}
