//! The weather panel's data: fetched from Open-Meteo on a thread of its own,
//! only while the dashboard is open and at most every `refresh_min`, and
//! cached in `$XDG_STATE_HOME/stratum/weather.json` so the panel shows the
//! last forecast (with its age) at once, offline too.
//!
//! The postal code's coordinates are cached with it and looked up again only
//! when `zip` or `country` change. A failed fetch keeps the last forecast,
//! reports the error, and is retried after [`RETRY`].

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use stratum_sys::managers::weather::{self, Forecast, Location, Units};

use crate::config::WeatherConfig;
use crate::sys_state::SystemState;

/// How soon a failed fetch is tried again while the dashboard stays open.
const RETRY: Duration = Duration::from_secs(60);

/// What the panel paints.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WeatherState {
    pub report: Option<Report>,
    /// Why the last fetch failed; cleared by the next success.
    pub error: Option<String>,
    pub loading: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// The place as the geocoder named it; empty for configured
    /// coordinates.
    pub place: String,
    pub units: Units,
    pub forecast: Forecast,
    /// Seconds since the Unix epoch.
    pub fetched: u64,
    /// Which coordinates and units it is for; a report for others is stale.
    key: String,
}

impl Report {
    /// Seconds since it was fetched.
    pub fn age(&self) -> u64 {
        now().saturating_sub(self.fetched)
    }
}

/// The file: the resolved postal code, and the last report.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    /// `zip,country` the location was looked up for.
    location_key: String,
    location: Option<Location>,
    report: Option<Report>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn cache_path() -> Option<PathBuf> {
    crate::paths::state_file("weather.json")
}

fn load_cache() -> Cache {
    cache_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn store_cache(cache: &Cache) {
    let Some(path) = cache_path() else { return };
    let Ok(text) = serde_json::to_string(cache) else { return };
    if let Err(err) = crate::paths::write_atomic(&path, text) {
        tracing::warn!("could not save the weather to {}: {err}", path.display());
    }
}

fn location_key(config: &WeatherConfig) -> String {
    format!("{},{}", config.zip.trim(), config.country.trim().to_uppercase())
}

fn report_key(location: &Location, units: Units) -> String {
    format!("{:.4},{:.4},{units:?}", location.latitude, location.longitude)
}

/// The configured coordinates, if both are set.
fn configured_location(config: &WeatherConfig) -> Option<Location> {
    Some(Location { name: String::new(), latitude: config.latitude?, longitude: config.longitude? })
}

/// Whether `report` answers `config`: the same place and units, fresh
/// enough. Without coordinates in the config, the place is the cached
/// lookup of its postal code.
fn is_current(report: &Report, config: &WeatherConfig, cache: &Cache) -> bool {
    let location = configured_location(config)
        .or_else(|| cache.location.clone().filter(|_| cache.location_key == location_key(config)));
    location.is_some_and(|l| report.key == report_key(&l, config.units)) && report.age() < config.refresh_min * 60
}

/// Fetches the weather when it is due. Owned by `SystemState`.
#[derive(Default)]
pub struct WeatherFeed {
    inner: Mutex<FetchState>,
}

#[derive(Default)]
struct FetchState {
    /// The cache file has been read into `SystemState::weather`.
    loaded: bool,
    in_flight: bool,
    last_failure: Option<Instant>,
}

impl WeatherFeed {
    /// Called with each sample while the dashboard is open.
    pub fn refresh_if_due(&self, state: &Arc<SystemState>, config: &WeatherConfig) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if !inner.loaded {
            inner.loaded = true;
            let cache = load_cache();
            if let Some(report) = cache.report {
                let mut weather = state.weather.get();
                weather.report = Some(report);
                state.weather.set(weather);
            }
        }
        if inner.in_flight || inner.last_failure.is_some_and(|t| t.elapsed() < RETRY) {
            return;
        }
        let current = state.weather.get().report.is_some_and(|r| is_current(&r, config, &load_cache()));
        if current {
            return;
        }
        inner.in_flight = true;
        drop(inner);

        let mut weather = state.weather.get();
        weather.loading = true;
        state.weather.set(weather);
        state.notify();

        let (state, config) = (Arc::clone(state), config.clone());
        let feed = Arc::clone(&state.weather_feed);
        let spawned = std::thread::Builder::new().name("stratum-weather".into()).spawn(move || {
            let result = fetch(&config);
            let mut weather = state.weather.get();
            weather.loading = false;
            let failed = result.is_err();
            match result {
                Ok(report) => {
                    weather.report = Some(report);
                    weather.error = None;
                }
                Err(err) => {
                    tracing::warn!("{err}");
                    weather.error = Some(err);
                }
            }
            state.weather.set(weather);
            let mut inner = feed.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.in_flight = false;
            inner.last_failure = failed.then(Instant::now);
            drop(inner);
            state.notify();
        });
        if spawned.is_err() {
            self.inner.lock().unwrap_or_else(|p| p.into_inner()).in_flight = false;
        }
    }
}

/// Resolve the place (once per postal code) and fetch its forecast, saving
/// both.
fn fetch(config: &WeatherConfig) -> Result<Report, String> {
    let mut cache = load_cache();
    let location = match configured_location(config) {
        Some(location) => location,
        None => match cache.location.clone().filter(|_| cache.location_key == location_key(config)) {
            Some(location) => location,
            None => {
                let location = weather::geocode(config.zip.trim(), config.country.trim())?;
                cache.location_key = location_key(config);
                cache.location = Some(location.clone());
                location
            }
        },
    };
    let forecast = weather::forecast(&location, config.units, config.hours, config.days)?;
    let report = Report {
        place: location.name.clone(),
        units: config.units,
        forecast,
        fetched: now(),
        key: report_key(&location, config.units),
    };
    cache.report = Some(report.clone());
    store_cache(&cache);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use stratum_sys::managers::weather::Current;

    fn config() -> WeatherConfig {
        crate::config::Config::test_defaults().dashboard.weather
    }

    fn report(location: &Location, units: Units, fetched: u64) -> Report {
        let current = Current { temperature: 50.0, feels_like: 48.0, code: 0, wind: 3.0, humidity: 40.0, is_day: true };
        Report {
            place: location.name.clone(),
            units,
            forecast: Forecast { current, hours: Vec::new(), days: Vec::new() },
            fetched,
            key: report_key(location, units),
        }
    }

    #[test]
    fn a_report_is_current_for_its_place_units_and_age() {
        let config = config();
        let dc = Location { name: "Washington D.C.".into(), latitude: 38.89511, longitude: -77.03637 };
        let cache = Cache { location_key: location_key(&config), location: Some(dc.clone()), report: None };
        assert!(is_current(&report(&dc, config.units, now()), &config, &cache));
        let stale = now() - config.refresh_min * 60;
        assert!(!is_current(&report(&dc, config.units, stale), &config, &cache), "too old");
        assert!(!is_current(&report(&dc, Units::Metric, now()), &config, &cache), "other units");

        // A new postal code makes the cached place, and so the report, stale.
        let moved = WeatherConfig { zip: "10001".into(), ..config.clone() };
        assert!(!is_current(&report(&dc, config.units, now()), &moved, &cache));

        // Configured coordinates need no lookup.
        let fixed = WeatherConfig { latitude: Some(dc.latitude), longitude: Some(dc.longitude), ..config };
        assert!(is_current(&report(&dc, fixed.units, now()), &fixed, &Cache::default()));
    }

    #[test]
    fn the_cache_round_trips() {
        let dc = Location { name: "Washington D.C.".into(), latitude: 38.9, longitude: -77.0 };
        let cache = Cache { location_key: "20001,US".into(), location: Some(dc.clone()), report: Some(report(&dc, Units::Imperial, 5)) };
        let back: Cache = serde_json::from_str(&serde_json::to_string(&cache).unwrap()).unwrap();
        assert_eq!(back.report, cache.report);
        assert_eq!(back.location, Some(dc));
    }
}
