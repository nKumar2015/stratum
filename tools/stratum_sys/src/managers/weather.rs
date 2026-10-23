//! Weather from Open-Meteo (no key, no account), fetched with `curl` so the
//! shell carries no HTTP or TLS stack of its own.
//!
//! A postal code becomes coordinates through Open-Meteo's geocoder, which
//! matches postal codes as well as place names; the caller keeps the result,
//! so it is looked up once per code. Times stay as the API's local ISO
//! strings (`2026-10-23T14:00`, `2026-10-23`) in the location's own zone.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::common::run_command_with_timeout;

/// How long one request may take.
const TIMEOUT: Duration = Duration::from_secs(15);

/// A place to forecast.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Location {
    /// As the geocoder names it ("Washington D.C."); empty for coordinates
    /// given directly.
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    /// °F and mph.
    Imperial,
    /// °C and km/h.
    Metric,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Forecast {
    pub current: Current,
    /// From the current hour on.
    pub hours: Vec<Hour>,
    /// From today on.
    pub days: Vec<Day>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Current {
    pub temperature: f32,
    pub feels_like: f32,
    /// WMO weather code; see [`describe`].
    pub code: u8,
    pub wind: f32,
    pub humidity: f32,
    pub is_day: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hour {
    pub time: String,
    pub temperature: f32,
    pub code: u8,
    /// Chance of precipitation, 0–100.
    pub precipitation: u8,
    pub is_day: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Day {
    pub date: String,
    pub code: u8,
    pub high: f32,
    pub low: f32,
    pub precipitation: u8,
}

/// A WMO weather code in words.
pub fn describe(code: u8) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51..=55 => "Drizzle",
        56 | 57 => "Freezing drizzle",
        61..=65 => "Rain",
        66 | 67 => "Freezing rain",
        71..=75 => "Snow",
        77 => "Snow grains",
        80..=82 => "Showers",
        85 | 86 => "Snow showers",
        95 => "Thunderstorm",
        96 | 99 => "Thunderstorm, hail",
        _ => "Unknown",
    }
}

/// Coordinates for a postal code in a country (ISO 3166 alpha-2, which
/// matters: postal codes repeat across countries).
pub fn geocode(postal_code: &str, country: &str) -> Result<Location, String> {
    let url = format!(
        "https://geocoding-api.open-meteo.com/v1/search?name={}&countryCode={}&count=1&language=en&format=json",
        encode(postal_code),
        encode(country)
    );
    parse_geocode(&fetch(&url)?).ok_or_else(|| format!("no place found for postal code {postal_code} in {country}"))
}

/// The forecast for `location`: now, the next `hours` hours, and `days`
/// days from today.
pub fn forecast(location: &Location, units: Units, hours: usize, days: usize) -> Result<Forecast, String> {
    let (temperature, wind) = match units {
        Units::Imperial => ("fahrenheit", "mph"),
        Units::Metric => ("celsius", "kmh"),
    };
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}\
         &current=temperature_2m,apparent_temperature,weather_code,wind_speed_10m,relative_humidity_2m,is_day\
         &hourly=temperature_2m,weather_code,precipitation_probability,is_day\
         &daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max\
         &temperature_unit={temperature}&wind_speed_unit={wind}&timezone=auto\
         &forecast_hours={hours}&forecast_days={days}",
        location.latitude, location.longitude
    );
    parse_forecast(&fetch(&url)?)
}

fn fetch(url: &str) -> Result<String, String> {
    let seconds = TIMEOUT.as_secs().to_string();
    // --fail turns an HTTP error into a failed run instead of an error page.
    run_command_with_timeout("curl", &["-sS", "--fail", "--max-time", &seconds, url], TIMEOUT + Duration::from_secs(1))
        .map_err(|err| format!("weather request failed: {}", err.trim()))
}

/// Percent-encode everything but unreserved characters.
fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn parse_geocode(body: &str) -> Option<Location> {
    #[derive(Deserialize)]
    struct Response {
        #[serde(default)]
        results: Vec<Place>,
    }
    #[derive(Deserialize)]
    struct Place {
        name: String,
        latitude: f64,
        longitude: f64,
    }
    let place = serde_json::from_str::<Response>(body).ok()?.results.into_iter().next()?;
    Some(Location { name: place.name, latitude: place.latitude, longitude: place.longitude })
}

fn parse_forecast(body: &str) -> Result<Forecast, String> {
    #[derive(Deserialize)]
    struct Response {
        current: CurrentRaw,
        hourly: HourlyRaw,
        daily: DailyRaw,
    }
    #[derive(Deserialize)]
    struct CurrentRaw {
        temperature_2m: f32,
        apparent_temperature: f32,
        weather_code: u8,
        wind_speed_10m: f32,
        relative_humidity_2m: f32,
        is_day: u8,
    }
    // Open-Meteo sends null for a value it lacks, hence the Options.
    #[derive(Deserialize)]
    struct HourlyRaw {
        time: Vec<String>,
        temperature_2m: Vec<Option<f32>>,
        weather_code: Vec<Option<u8>>,
        precipitation_probability: Vec<Option<u8>>,
        is_day: Vec<Option<u8>>,
    }
    #[derive(Deserialize)]
    struct DailyRaw {
        time: Vec<String>,
        weather_code: Vec<Option<u8>>,
        temperature_2m_max: Vec<Option<f32>>,
        temperature_2m_min: Vec<Option<f32>>,
        precipitation_probability_max: Vec<Option<u8>>,
    }
    let raw: Response = serde_json::from_str(body).map_err(|err| format!("unexpected weather response: {err}"))?;
    let at = |values: &[Option<u8>], i: usize| values.get(i).copied().flatten().unwrap_or(0);
    let hours = raw
        .hourly
        .time
        .iter()
        .enumerate()
        .filter_map(|(i, time)| {
            Some(Hour {
                time: time.clone(),
                temperature: raw.hourly.temperature_2m.get(i).copied().flatten()?,
                code: at(&raw.hourly.weather_code, i),
                precipitation: at(&raw.hourly.precipitation_probability, i),
                is_day: at(&raw.hourly.is_day, i) == 1,
            })
        })
        .collect();
    let days = raw
        .daily
        .time
        .iter()
        .enumerate()
        .filter_map(|(i, date)| {
            Some(Day {
                date: date.clone(),
                code: at(&raw.daily.weather_code, i),
                high: raw.daily.temperature_2m_max.get(i).copied().flatten()?,
                low: raw.daily.temperature_2m_min.get(i).copied().flatten()?,
                precipitation: at(&raw.daily.precipitation_probability_max, i),
            })
        })
        .collect();
    let c = raw.current;
    Ok(Forecast {
        current: Current {
            temperature: c.temperature_2m,
            feels_like: c.apparent_temperature,
            code: c.weather_code,
            wind: c.wind_speed_10m,
            humidity: c.relative_humidity_2m,
            is_day: c.is_day == 1,
        },
        hours,
        days,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_postal_code_resolves_to_its_first_place() {
        let body = r#"{"results":[{"id":4140963,"name":"Washington D.C.","latitude":38.89511,"longitude":-77.03637,"postcodes":["20001"]}],"generationtime_ms":0.5}"#;
        let place = parse_geocode(body).unwrap();
        assert_eq!(place, Location { name: "Washington D.C.".into(), latitude: 38.89511, longitude: -77.03637 });
        // The geocoder leaves `results` out when nothing matches.
        assert_eq!(parse_geocode(r#"{"generationtime_ms":0.3}"#), None);
    }

    #[test]
    fn a_forecast_parses_and_skips_missing_values() {
        let body = r#"{
            "current":{"time":"2026-10-23T02:00","temperature_2m":47.3,"apparent_temperature":41.4,"weather_code":0,"wind_speed_10m":5.2,"relative_humidity_2m":58,"is_day":0},
            "hourly":{"time":["2026-10-23T02:00","2026-10-23T03:00"],"temperature_2m":[47.3,null],"weather_code":[0,0],"precipitation_probability":[null,10],"is_day":[0,0]},
            "daily":{"time":["2026-10-23"],"weather_code":[3],"temperature_2m_max":[63.2],"temperature_2m_min":[44.7],"precipitation_probability_max":[20]}
        }"#;
        let forecast = parse_forecast(body).unwrap();
        assert_eq!(forecast.current.code, 0);
        assert!(!forecast.current.is_day);
        assert_eq!(forecast.hours.len(), 1, "an hour without a temperature is dropped");
        assert_eq!(forecast.hours[0].precipitation, 0, "a null chance reads as none");
        assert_eq!(forecast.days[0], Day { date: "2026-10-23".into(), code: 3, high: 63.2, low: 44.7, precipitation: 20 });
    }

    #[test]
    fn an_error_page_is_an_error() {
        assert!(parse_forecast(r#"{"error":true,"reason":"Latitude must be in range"}"#).is_err());
    }

    #[test]
    fn query_values_are_encoded() {
        assert_eq!(encode("SW1A 1AA"), "SW1A%201AA");
        assert_eq!(encode("20001"), "20001");
    }

    #[test]
    fn codes_have_words() {
        assert_eq!(describe(0), "Clear");
        assert_eq!(describe(63), "Rain");
        assert_eq!(describe(200), "Unknown");
    }
}
