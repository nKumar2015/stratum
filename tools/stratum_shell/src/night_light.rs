//! Night light: warmer screens at night. Pure: when night is, how far into
//! its fade, the colour that makes, and the gamma ramps that show it. The
//! output side is `wayland/state/night_light.rs`.

use chrono::{DateTime, Duration, Local, NaiveDate, NaiveTime, TimeZone};

use crate::config::{NightLightConfig, NightSchedule};

/// Daylight: the screen's own colour, which night light leaves alone.
pub const DAY_KELVIN: u32 = 6500;

/// Sunrise and sunset on `date` at `latitude`/`longitude` (east positive),
/// as Unix times, from the sunrise equation (accurate to a minute or two).
/// `None` in a polar day or night.
pub fn sun_times(date: NaiveDate, latitude: f64, longitude: f64) -> Option<(i64, i64)> {
    use std::f64::consts::PI;
    let rad = PI / 180.0;
    // Julian date at midnight UTC starting `date` (the day count is
    // rounded up from it, so noon would count the next day).
    let midnight = date.and_time(NaiveTime::MIN).and_utc().timestamp() as f64;
    let julian = midnight / 86_400.0 + 2_440_587.5;
    let n = (julian - 2_451_545.0 + 0.0008).ceil();
    let mean_noon = n - longitude / 360.0;
    let anomaly = (357.5291 + 0.985_600_28 * mean_noon).rem_euclid(360.0);
    let m = anomaly * rad;
    let center = 1.9148 * m.sin() + 0.02 * (2.0 * m).sin() + 0.0003 * (3.0 * m).sin();
    let ecliptic = (anomaly + center + 180.0 + 102.9372).rem_euclid(360.0) * rad;
    let transit = 2_451_545.0 + mean_noon + 0.0053 * m.sin() - 0.0069 * (2.0 * ecliptic).sin();
    let declination = (ecliptic.sin() * (23.4397 * rad).sin()).asin();
    let phi = latitude * rad;
    let cos_hour = ((-0.833 * rad).sin() - phi.sin() * declination.sin()) / (phi.cos() * declination.cos());
    if !(-1.0..=1.0).contains(&cos_hour) {
        return None;
    }
    let half_day = cos_hour.acos() / rad / 360.0;
    let unix = |j: f64| ((j - 2_440_587.5) * 86_400.0).round() as i64;
    Some((unix(transit - half_day), unix(transit + half_day)))
}

/// What night light is doing, for the Display page and the bar.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// Warmer than daylight right now.
    pub on: bool,
    pub kelvin: u32,
    /// When that changes next (Unix time), if it will.
    pub until: Option<i64>,
    /// Turned on or off by hand until `until`, over the schedule.
    pub manual: Option<bool>,
    /// The location the sun schedule uses, and whether it is the config's.
    pub location: Option<(Location, bool)>,
    /// Why it cannot work: no gamma control, or another program has it.
    pub problem: Option<String>,
}

/// Where night light takes the sun from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Location {
    pub latitude: f64,
    pub longitude: f64,
}

/// The configured location, else the one the dashboard's weather found.
pub fn location(config: &NightLightConfig) -> Option<Location> {
    if let (Some(latitude), Some(longitude)) = (config.latitude, config.longitude) {
        return Some(Location { latitude, longitude });
    }
    let path = crate::paths::state_file("weather.json")?;
    let cache: serde_json::Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    let found = &cache["location"];
    Some(Location { latitude: found["latitude"].as_f64()?, longitude: found["longitude"].as_f64()? })
}

pub use crate::config::parse_time;

/// Minutes after midnight as `"HH:MM"`.
pub fn format_time(minutes: u32) -> String {
    format!("{:02}:{:02}", minutes / 60 % 24, minutes % 60)
}

/// The night that starts on `date`: its start and end as Unix times.
fn night_of(date: NaiveDate, config: &NightLightConfig, location: Option<Location>) -> Option<(i64, i64)> {
    let at = |date: NaiveDate, minutes: u32| -> Option<i64> {
        let naive = date.and_time(NaiveTime::from_hms_opt(minutes / 60, minutes % 60, 0)?);
        // A time a DST change skips falls back to the hour after.
        Local.from_local_datetime(&naive).earliest().or_else(|| Local.from_local_datetime(&(naive + Duration::hours(1))).earliest()).map(|t| t.timestamp())
    };
    let next = date.succ_opt()?;
    match (config.schedule, location) {
        (NightSchedule::Sun, Some(loc)) => {
            let (_, sunset) = sun_times(date, loc.latitude, loc.longitude)?;
            let (sunrise, _) = sun_times(next, loc.latitude, loc.longitude)?;
            Some((sunset, sunrise))
        }
        // Without a location the sun schedule falls back to the fixed times.
        _ => {
            let start = parse_time(&config.start)?;
            let end = parse_time(&config.end)?;
            let end_date = if end <= start { next } else { date };
            Some((at(date, start)?, at(end_date, end)?))
        }
    }
}

/// How far into night it is at `now`, 0 (day) to 1, faded over
/// `fade_min` after night starts and before it ends; and when that changes
/// next (the end of the night it is in, or the start of the next).
pub fn strength(now: DateTime<Local>, config: &NightLightConfig, location: Option<Location>) -> (f32, Option<i64>) {
    let t = now.timestamp();
    let fade = (i64::from(config.fade_min) * 60).max(1);
    let today = now.date_naive();
    let nights: Vec<(i64, i64)> = [today.pred_opt(), Some(today), today.succ_opt()]
        .into_iter()
        .flatten()
        .filter_map(|d| night_of(d, config, location))
        .collect();
    for &(start, end) in &nights {
        if (start..end).contains(&t) {
            let into = (t - start) as f32 / fade as f32;
            let left = (end - t) as f32 / fade as f32;
            return (into.min(left).clamp(0.0, 1.0), Some(end));
        }
    }
    (0.0, nights.iter().map(|n| n.0).filter(|s| *s > t).min())
}

/// The colour temperature at `strength` into night.
pub fn kelvin(strength: f32, night: u32) -> u32 {
    let night = night.min(DAY_KELVIN) as f32;
    (DAY_KELVIN as f32 - (DAY_KELVIN as f32 - night) * strength.clamp(0.0, 1.0)).round() as u32
}

/// Red, green and blue multipliers for a white at `kelvin`, 1 at 6500 K
/// (Tanner Helland's fit to the blackbody curve).
pub fn whitepoint(kelvin: u32) -> [f64; 3] {
    fn raw(kelvin: u32) -> [f64; 3] {
        let t = f64::from(kelvin.clamp(1000, 40_000)) / 100.0;
        let r = if t <= 66.0 { 255.0 } else { 329.698_727_446 * (t - 60.0).powf(-0.133_204_759_2) };
        let g = if t <= 66.0 {
            99.470_802_586_1 * t.ln() - 161.119_568_166_1
        } else {
            288.122_169_528_3 * (t - 60.0).powf(-0.075_514_849_2)
        };
        let b = if t >= 66.0 {
            255.0
        } else if t <= 19.0 {
            0.0
        } else {
            138.517_731_223_1 * (t - 10.0).ln() - 305.044_792_730_7
        };
        [r, g, b].map(|c| c.clamp(0.0, 255.0) / 255.0)
    }
    let day = raw(DAY_KELVIN);
    let at = raw(kelvin);
    [0, 1, 2].map(|i| (at[i] / day[i]).min(1.0))
}

/// The three gamma ramps (red, green, blue, `size` entries each) for a white
/// at `kelvin`, as the gamma-control protocol takes them.
pub fn ramps(size: usize, kelvin: u32) -> Vec<u16> {
    let white = whitepoint(kelvin);
    let last = size.saturating_sub(1).max(1) as f64;
    white
        .iter()
        .flat_map(|m| (0..size).map(move |i| (i as f64 / last * m * 65535.0).round() as u16))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(schedule: NightSchedule) -> NightLightConfig {
        NightLightConfig { schedule, ..crate::config::Config::test_defaults().night_light }
    }

    #[test]
    fn sunrise_and_sunset_in_new_york_at_the_equinox() {
        // About 07:01 EDT (11:01Z) and 19:12 EDT (23:12Z) on 2024-03-20. The
        // equation is good to a minute or two, which is plenty for a fade
        // that takes half an hour; the bound allows for that and the
        // reference's rounding.
        let date = NaiveDate::from_ymd_opt(2024, 3, 20).unwrap();
        let (rise, set) = sun_times(date, 40.7128, -74.0060).unwrap();
        let utc = |h: u32, m: u32| date.and_hms_opt(h, m, 0).unwrap().and_utc().timestamp();
        assert!((rise - utc(11, 1)).abs() < 300, "sunrise off by {} s", rise - utc(11, 1));
        assert!((set - utc(23, 12)).abs() < 300, "sunset off by {} s", set - utc(23, 12));
        assert!((set - rise - (12 * 3600 + 11 * 60)).abs() < 300, "an equinox day is about 12h11m");
        assert_eq!(sun_times(NaiveDate::from_ymd_opt(2024, 6, 21).unwrap(), 80.0, 0.0), None, "polar day");
    }

    #[test]
    fn fixed_nights_cross_midnight_and_fade() {
        let mut c = config(NightSchedule::Fixed);
        c.start = "21:00".into();
        c.end = "07:00".into();
        c.fade_min = 60;
        let at = |h: u32, m: u32| Local.from_local_datetime(&NaiveDate::from_ymd_opt(2026, 1, 15).unwrap().and_hms_opt(h, m, 0).unwrap()).unwrap();
        assert_eq!(strength(at(20, 0), &c, None).0, 0.0, "evening");
        assert!((strength(at(21, 30), &c, None).0 - 0.5).abs() < 0.01, "half way into the fade");
        assert_eq!(strength(at(23, 0), &c, None).0, 1.0);
        assert_eq!(strength(at(3, 0), &c, None).0, 1.0, "after midnight, last night's");
        assert!((strength(at(6, 30), &c, None).0 - 0.5).abs() < 0.01, "fading out before the end");
        let (s, next) = strength(at(12, 0), &c, None);
        assert_eq!((s, next), (0.0, Some(at(21, 0).timestamp())), "day: the next night starts at 21:00");
    }

    #[test]
    fn the_sun_schedule_needs_a_location_or_uses_the_fixed_times() {
        let c = config(NightSchedule::Sun);
        let noon = Local.from_local_datetime(&NaiveDate::from_ymd_opt(2026, 6, 1).unwrap().and_hms_opt(12, 0, 0).unwrap()).unwrap();
        let ny = Location { latitude: 40.7, longitude: -74.0 };
        assert_eq!(strength(noon, &c, Some(ny)).0, 0.0, "noon is day");
        assert!(strength(noon, &c, Some(ny)).1.is_some(), "with tonight's sunset next");
        assert_eq!(strength(noon, &c, None).0, 0.0);
    }

    #[test]
    fn colour_temperature() {
        assert_eq!(whitepoint(6500), [1.0, 1.0, 1.0]);
        let warm = whitepoint(3400);
        assert_eq!(warm[0], 1.0, "red stays");
        assert!(warm[1] < 0.9 && warm[2] < warm[1], "green dims, blue more: {warm:?}");
        assert_eq!(kelvin(0.0, 3400), 6500);
        assert_eq!(kelvin(1.0, 3400), 3400);
        assert_eq!(kelvin(0.5, 3500), 5000);
        let r = ramps(4, 6500);
        assert_eq!(r, [0, 21845, 43690, 65535, 0, 21845, 43690, 65535, 0, 21845, 43690, 65535]);
    }

    #[test]
    fn times() {
        assert_eq!(parse_time("07:30"), Some(450));
        assert_eq!(parse_time("24:00"), None);
        assert_eq!(format_time(1290), "21:30");
    }
}
