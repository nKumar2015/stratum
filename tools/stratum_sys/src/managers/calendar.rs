//! Calendar events from ICS (RFC 5545) feeds: parsing, time zones and
//! recurrence.
//!
//! What is understood: `VEVENT`s with `SUMMARY`, `LOCATION`, `DTSTART`,
//! `DTEND` or `DURATION`, `STATUS:CANCELLED`, `EXDATE`, `RECURRENCE-ID`
//! (a moved or cancelled instance of a series) and `RRULE` with `FREQ`
//! daily to yearly, `INTERVAL`, `COUNT`, `UNTIL`, `BYDAY` (with ordinals in
//! monthly and yearly rules), `BYMONTHDAY` and `BYMONTH`. `BYSETPOS`,
//! `BYWEEKNO`, `BYYEARDAY` and sub-daily frequencies are ignored, so such a
//! series shows its first instance or a superset.
//!
//! Times are converted to the system's zone. A `TZID` is looked up in the
//! system's zoneinfo; one it does not know (Outlook's Windows names) is read
//! as local time. Series repeat in their own zone's wall time, so a weekly
//! 9:00 meeting stays at 9:00 across daylight saving changes.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::Duration;

use chrono::{Datelike, Days, Months, NaiveDate, NaiveDateTime, NaiveTime, Timelike, Weekday};
use jiff::tz::TimeZone;

use super::common::run_command_with_timeout;

/// How long one feed may take to download.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Recurrence periods walked at most per series, so a malformed rule cannot
/// spin.
const MAX_PERIODS: usize = 20_000;

/// A start, end or exception as written.
#[derive(Clone, Debug, PartialEq)]
pub enum When {
    /// All day.
    Date(NaiveDate),
    Utc(NaiveDateTime),
    /// Wall time in a named zone.
    Zoned(NaiveDateTime, String),
    /// Wall time wherever the viewer is.
    Floating(NaiveDateTime),
}

impl When {
    fn wall(&self) -> NaiveDateTime {
        match self {
            When::Date(d) => d.and_time(NaiveTime::MIN),
            When::Utc(t) | When::Zoned(t, _) | When::Floating(t) => *t,
        }
    }

    /// The same form, at another wall time.
    fn with_wall(&self, wall: NaiveDateTime) -> When {
        match self {
            When::Date(_) => When::Date(wall.date()),
            When::Utc(_) => When::Utc(wall),
            When::Zoned(_, zone) => When::Zoned(wall, zone.clone()),
            When::Floating(_) => When::Floating(wall),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub freq: Freq,
    pub interval: u32,
    pub count: Option<u32>,
    pub until: Option<When>,
    /// Weekdays, with an ordinal in monthly and yearly rules (`2TU`, `-1FR`).
    pub by_day: Vec<(Option<i32>, Weekday)>,
    pub by_month_day: Vec<i32>,
    pub by_month: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub uid: String,
    pub summary: String,
    pub location: String,
    pub start: When,
    pub end: Option<When>,
    /// Seconds, from `DURATION`.
    pub duration: Option<i64>,
    pub rule: Option<Rule>,
    pub exdates: Vec<When>,
    /// Set on an instance of a series that was moved or cancelled.
    pub recurrence_id: Option<When>,
    pub cancelled: bool,
    /// Which feed it came from, by position.
    pub calendar: usize,
}

/// One instance of an event, in local time.
#[derive(Clone, Debug, PartialEq)]
pub struct Occurrence {
    pub summary: String,
    pub location: String,
    pub start: NaiveDateTime,
    /// Exclusive; for an all-day event, midnight after its last day.
    pub end: NaiveDateTime,
    pub all_day: bool,
    pub calendar: usize,
}

/// Download `url` into `dest`. `webcal://` is fetched as `https://`.
pub fn fetch(url: &str, dest: &Path) -> Result<(), String> {
    let url = match url.strip_prefix("webcal://") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_string(),
    };
    let seconds = TIMEOUT.as_secs().to_string();
    let dest = dest.to_string_lossy();
    // Written to a file: a feed can be megabytes, more than a pipe holds.
    run_command_with_timeout(
        "curl",
        &["-sS", "--fail", "-L", "--max-time", &seconds, "-o", &dest, &url],
        TIMEOUT + Duration::from_secs(1),
    )
    .map(|_| ())
    // The address is a secret: never repeat it, or any part of it, in an
    // error.
    .map_err(|err| format!("download failed: {}", redact(&strip_urls(curl_reason(&err)), &url)))
}

/// `text` with every part of `url` (the whole, without its scheme, its
/// path and query, its host) replaced, longest first.
fn redact(text: &str, url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let (host, path) = rest.split_once('/').map_or((rest, ""), |(host, path)| (host, path));
    let mut parts: Vec<String> = [url.to_string(), rest.to_string(), format!("/{path}"), host.to_string()]
        .into_iter()
        .filter(|p| p.len() > 1)
        .collect();
    parts.sort_by_key(|p| std::cmp::Reverse(p.len()));
    parts.into_iter().fold(text.to_string(), |text, part| text.replace(&part, "<address>"))
}

/// curl's own words from its error, without the runner's and curl's
/// prefixes: "curl failed: curl: (6) Could not resolve host" reads "Could
/// not resolve host".
fn curl_reason(err: &str) -> &str {
    let err = err.trim();
    match err.find("curl: (").and_then(|i| err[i..].find(") ").map(|j| i + j + 2)) {
        Some(start) => &err[start..],
        None => err,
    }
}

fn strip_urls(text: &str) -> String {
    text.split_whitespace()
        .map(|word| if word.contains("://") { "<address>" } else { word })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Lines with RFC 5545 folding undone: a line starting with a space or tab
/// continues the one before.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        match (line.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => lines.push(line.to_string()),
        }
    }
    lines
}

/// A content line: `NAME;PARAM=value;PARAM="quoted":value`.
struct Property {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Property {
    fn param(&self, name: &str) -> Option<&str> {
        self.params.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

fn parse_property(line: &str) -> Option<Property> {
    // The value starts at the first colon outside quotes.
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(i, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        (c == ':' && !quoted).then_some(i)
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.trim().to_ascii_uppercase(), v.trim().trim_matches('"').to_string()))
        })
        .collect();
    Some(Property { name, params, value: value.to_string() })
}

/// A TEXT value with its escapes undone.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push(' '),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

fn parse_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value.get(..8)?, "%Y%m%d").ok()
}

fn parse_datetime(value: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(value.get(..15)?, "%Y%m%dT%H%M%S").ok()
}

/// One date or date-time value, in the form the property's parameters say.
fn parse_when_value(value: &str, tzid: Option<&str>, date_only: bool) -> Option<When> {
    let value = value.trim();
    if date_only || value.len() == 8 {
        return parse_date(value).map(When::Date);
    }
    let time = parse_datetime(value)?;
    Some(if value.ends_with('Z') {
        When::Utc(time)
    } else if let Some(zone) = tzid {
        When::Zoned(time, zone.to_string())
    } else {
        When::Floating(time)
    })
}

fn parse_when(prop: &Property) -> Vec<When> {
    let date_only = prop.param("VALUE") == Some("DATE");
    let tzid = prop.param("TZID");
    prop.value.split(',').filter_map(|v| parse_when_value(v, tzid, date_only)).collect()
}

/// `DURATION` (`P1D`, `PT1H30M`, `-PT15M`, `P2W`) in seconds.
fn parse_duration(value: &str) -> Option<i64> {
    let (sign, rest) = match value.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, value.strip_prefix('+').unwrap_or(value)),
    };
    let rest = rest.strip_prefix('P')?;
    let (mut total, mut number, mut in_time) = (0i64, String::new(), false);
    for c in rest.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => number.push(c),
            unit => {
                let n: i64 = std::mem::take(&mut number).parse().ok()?;
                total += n * match (unit, in_time) {
                    ('W', _) => 7 * 86_400,
                    ('D', _) => 86_400,
                    ('H', true) => 3600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
            }
        }
    }
    Some(sign * total)
}

fn parse_weekday(code: &str) -> Option<Weekday> {
    Some(match code {
        "MO" => Weekday::Mon,
        "TU" => Weekday::Tue,
        "WE" => Weekday::Wed,
        "TH" => Weekday::Thu,
        "FR" => Weekday::Fri,
        "SA" => Weekday::Sat,
        "SU" => Weekday::Sun,
        _ => return None,
    })
}

fn parse_rule(value: &str, tzid: Option<&str>) -> Option<Rule> {
    let mut rule = Rule {
        freq: Freq::Daily,
        interval: 1,
        count: None,
        until: None,
        by_day: Vec::new(),
        by_month_day: Vec::new(),
        by_month: Vec::new(),
    };
    let mut freq = None;
    for part in value.split(';') {
        let Some((key, value)) = part.split_once('=') else { continue };
        let list = || value.split(',').map(str::trim);
        match key.trim().to_ascii_uppercase().as_str() {
            "FREQ" => {
                freq = Some(match value.trim().to_ascii_uppercase().as_str() {
                    "DAILY" => Freq::Daily,
                    "WEEKLY" => Freq::Weekly,
                    "MONTHLY" => Freq::Monthly,
                    "YEARLY" => Freq::Yearly,
                    // Hourly and finer are not shown as series.
                    _ => return None,
                })
            }
            "INTERVAL" => rule.interval = value.trim().parse().ok().filter(|&n| n > 0)?,
            "COUNT" => rule.count = value.trim().parse().ok(),
            // UNTIL is UTC when the start is zoned; read it as written.
            "UNTIL" => rule.until = parse_when_value(value, tzid, false),
            "BYDAY" => {
                rule.by_day = list()
                    .filter_map(|d| {
                        let split = d.len().checked_sub(2)?;
                        let ordinal = d[..split].parse::<i32>().ok();
                        Some((ordinal, parse_weekday(&d[split..].to_ascii_uppercase())?))
                    })
                    .collect()
            }
            "BYMONTHDAY" => rule.by_month_day = list().filter_map(|d| d.parse().ok()).collect(),
            "BYMONTH" => rule.by_month = list().filter_map(|d| d.parse().ok()).collect(),
            _ => {}
        }
    }
    rule.freq = freq?;
    Some(rule)
}

/// Every `VEVENT` in an ICS document, tagged with `calendar`.
pub fn parse(text: &str, calendar: usize) -> Vec<Event> {
    let mut events = Vec::new();
    let mut current: Option<Vec<Property>> = None;
    // VALARMs nest inside events; their properties are not the event's.
    let mut nested = 0usize;
    for line in unfold(text) {
        let upper = line.trim().to_ascii_uppercase();
        if upper == "BEGIN:VEVENT" {
            current = Some(Vec::new());
            nested = 0;
        } else if upper == "END:VEVENT" {
            if let Some(props) = current.take() {
                events.extend(build_event(&props, calendar));
            }
        } else if let Some(props) = current.as_mut() {
            if upper.starts_with("BEGIN:") {
                nested += 1;
            } else if upper.starts_with("END:") {
                nested = nested.saturating_sub(1);
            } else if nested == 0 {
                props.extend(parse_property(&line));
            }
        }
    }
    events
}

fn build_event(props: &[Property], calendar: usize) -> Option<Event> {
    let get = |name: &str| props.iter().find(|p| p.name == name);
    let start_prop = get("DTSTART")?;
    let start = parse_when(start_prop).into_iter().next()?;
    Some(Event {
        uid: get("UID").map(|p| p.value.trim().to_string()).unwrap_or_default(),
        summary: get("SUMMARY").map(|p| unescape(&p.value)).filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "(No title)".into()),
        location: get("LOCATION").map(|p| unescape(&p.value)).unwrap_or_default(),
        end: get("DTEND").and_then(|p| parse_when(p).into_iter().next()),
        duration: get("DURATION").and_then(|p| parse_duration(p.value.trim())),
        rule: get("RRULE").and_then(|p| parse_rule(&p.value, start_prop.param("TZID"))),
        exdates: props.iter().filter(|p| p.name == "EXDATE").flat_map(parse_when).collect(),
        recurrence_id: get("RECURRENCE-ID").and_then(|p| parse_when(p).into_iter().next()),
        cancelled: get("STATUS").is_some_and(|p| p.value.trim().eq_ignore_ascii_case("CANCELLED")),
        start,
        calendar,
    })
}

/// Zones by `TZID`, looked up once each.
struct Zones {
    local: TimeZone,
    known: HashMap<String, Option<TimeZone>>,
}

impl Zones {
    fn new() -> Self {
        Self { local: TimeZone::system(), known: HashMap::new() }
    }

    /// A zone by name. Some feeds prefix the IANA name
    /// (`/mozilla.org/20050126_1/America/New_York`), so trailing parts are
    /// tried too.
    fn get(&mut self, name: &str) -> Option<TimeZone> {
        self.known
            .entry(name.to_string())
            .or_insert_with(|| {
                let parts: Vec<&str> = name.split('/').filter(|p| !p.is_empty()).collect();
                (0..parts.len()).find_map(|i| TimeZone::get(&parts[i..].join("/")).ok())
            })
            .clone()
    }

    /// `when` as local wall time.
    fn local(&mut self, when: &When) -> NaiveDateTime {
        let (wall, zone) = match when {
            When::Date(_) | When::Floating(_) => return when.wall(),
            When::Utc(t) => (*t, Some(TimeZone::UTC)),
            When::Zoned(t, name) => (*t, self.get(name)),
        };
        let Some(zone) = zone else { return wall };
        let civil = jiff::civil::DateTime::new(
            wall.year() as i16,
            wall.month() as i8,
            wall.day() as i8,
            wall.hour() as i8,
            wall.minute() as i8,
            wall.second() as i8,
            0,
        );
        let Ok(zoned) = civil.and_then(|c| c.to_zoned(zone)) else { return wall };
        let here = self.local.to_datetime(zoned.timestamp());
        NaiveDate::from_ymd_opt(i32::from(here.year()), here.month() as u32, here.day() as u32)
            .and_then(|d| d.and_hms_opt(here.hour() as u32, here.minute() as u32, here.second() as u32))
            .unwrap_or(wall)
    }
}

/// The days of `year`/`month` a monthly or yearly rule picks, in order.
fn month_days(rule: &Rule, year: i32, month: u32, start_day: u32) -> Vec<NaiveDate> {
    let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else { return Vec::new() };
    let last = first.checked_add_months(Months::new(1)).and_then(|d| d.pred_opt()).map_or(28, |d| d.day());
    let mut days: Vec<u32> = if !rule.by_month_day.is_empty() {
        rule.by_month_day
            .iter()
            .filter_map(|&d| match d {
                1..=31 => Some(d as u32),
                -31..=-1 => (last as i32 + 1 + d).try_into().ok(),
                _ => None,
            })
            .filter(|&d| d <= last)
            .collect()
    } else if !rule.by_day.is_empty() {
        rule.by_day
            .iter()
            .flat_map(|&(ordinal, weekday)| {
                let matching: Vec<u32> =
                    (1..=last).filter(|&d| first.with_day(d).is_some_and(|date| date.weekday() == weekday)).collect();
                match ordinal {
                    None => matching,
                    Some(n) if n > 0 => matching.get(n as usize - 1).copied().into_iter().collect(),
                    Some(n) => matching.len().checked_sub(n.unsigned_abs() as usize).and_then(|i| matching.get(i)).copied().into_iter().collect(),
                }
            })
            .collect()
    } else {
        // A month without the start's day (the 31st) is skipped, per RFC.
        vec![start_day].into_iter().filter(|&d| d <= last).collect()
    };
    days.sort_unstable();
    days.dedup();
    days.into_iter().filter_map(|d| first.with_day(d)).collect()
}

/// The wall-clock starts of a series, in order, from its first until
/// `limit` (wall time), its `COUNT` or its `UNTIL`.
fn expand(rule: &Rule, start: NaiveDateTime, limit: NaiveDateTime, until: Option<NaiveDateTime>) -> Vec<NaiveDateTime> {
    let time = start.time();
    let interval = rule.interval.max(1);
    let mut out = Vec::new();
    let mut emitted = 0u32;
    for period in 0..MAX_PERIODS {
        let n = period as u32 * interval;
        let candidates: Vec<NaiveDate> = match rule.freq {
            Freq::Daily => start.date().checked_add_days(Days::new(n.into())).into_iter().collect(),
            Freq::Weekly => {
                // Weeks from Monday (WKST=MO, the default).
                let monday = start.date() - Days::new(start.date().weekday().num_days_from_monday().into());
                let Some(week) = monday.checked_add_days(Days::new(u64::from(n) * 7)) else { break };
                let mut days: Vec<Weekday> = rule.by_day.iter().map(|&(_, d)| d).collect();
                if days.is_empty() {
                    days.push(start.date().weekday());
                }
                let mut dates: Vec<NaiveDate> =
                    days.iter().map(|d| week + Days::new(d.num_days_from_monday().into())).collect();
                dates.sort_unstable();
                dates.dedup();
                dates
            }
            Freq::Monthly => {
                let Some(month) = start.date().with_day(1).and_then(|d| d.checked_add_months(Months::new(n))) else { break };
                month_days(rule, month.year(), month.month(), start.day())
            }
            Freq::Yearly => {
                let year = start.year() + n as i32;
                let months = if rule.by_month.is_empty() { vec![start.month()] } else { rule.by_month.clone() };
                let mut dates = Vec::new();
                for month in months {
                    let picks_days = !rule.by_month_day.is_empty() || !rule.by_day.is_empty();
                    if picks_days {
                        dates.extend(month_days(rule, year, month, start.day()));
                    } else {
                        dates.extend(NaiveDate::from_ymd_opt(year, month, start.day()));
                    }
                }
                dates.sort_unstable();
                dates
            }
        };
        let mut past_limit = false;
        for date in candidates {
            if !rule.by_month.is_empty() && !rule.by_month.contains(&date.month()) {
                continue;
            }
            // A daily rule with BYDAY keeps only those weekdays.
            if rule.freq == Freq::Daily && !rule.by_day.is_empty() && !rule.by_day.iter().any(|&(_, d)| d == date.weekday()) {
                continue;
            }
            let at = date.and_time(time);
            if at < start {
                continue;
            }
            if until.is_some_and(|u| at > u) || rule.count.is_some_and(|c| emitted >= c) {
                return out;
            }
            if at > limit {
                past_limit = true;
                break;
            }
            emitted += 1;
            out.push(at);
        }
        if past_limit {
            break;
        }
    }
    out
}

/// The instances of `events` overlapping `from`..`to` (local dates, `to`
/// exclusive), all-day ones first on each day, then by start.
pub fn occurrences(events: &[Event], from: NaiveDate, to: NaiveDate) -> Vec<Occurrence> {
    let mut zones = Zones::new();
    let window_start = from.and_time(NaiveTime::MIN);
    let window_end = to.and_time(NaiveTime::MIN);
    // Instances that were moved or cancelled, by series and original start.
    let mut overridden: HashSet<(usize, String, NaiveDateTime)> = HashSet::new();
    for event in events {
        if let Some(id) = &event.recurrence_id {
            overridden.insert((event.calendar, event.uid.clone(), zones.local(id)));
        }
    }
    let mut out = Vec::new();
    for event in events.iter().filter(|e| !e.cancelled) {
        let all_day = matches!(event.start, When::Date(_));
        // Length in the event's own wall time.
        let length = match (&event.end, event.duration) {
            (Some(end), _) => end.wall() - event.start.wall(),
            (None, Some(seconds)) => chrono::Duration::seconds(seconds),
            (None, None) if all_day => chrono::Duration::days(1),
            (None, None) => chrono::Duration::zero(),
        }
        .max(chrono::Duration::zero());
        let starts = match (&event.rule, &event.recurrence_id) {
            (Some(rule), None) => {
                // Zone offsets are under a day: two days of slack each side
                // covers any difference between wall and local time.
                let limit = window_end + chrono::Duration::days(2);
                let until = rule.until.as_ref().map(|u| match (u, &event.start) {
                    // A zoned series' UTC UNTIL, compared in its wall time.
                    (When::Utc(_), When::Zoned(_, _)) | (When::Utc(_), When::Floating(_)) => zones.local(u),
                    (When::Date(d), _) => d.and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap_or(NaiveTime::MIN)),
                    (other, _) => other.wall(),
                });
                expand(rule, event.start.wall(), limit, until)
            }
            _ => vec![event.start.wall()],
        };
        let exdates: HashSet<NaiveDateTime> = event.exdates.iter().map(|e| zones.local(e)).collect();
        for wall in starts {
            let instance = event.start.with_wall(wall);
            let start = zones.local(&instance);
            let series = event.rule.is_some() && event.recurrence_id.is_none();
            if series && (exdates.contains(&start) || overridden.contains(&(event.calendar, event.uid.clone(), start))) {
                continue;
            }
            let end = zones.local(&instance.with_wall(wall + length));
            let end = if all_day { end.max(start + chrono::Duration::days(1)) } else { end.max(start) };
            // Overlapping the window; a zero-length event counts at its start.
            if start < window_end && (end > window_start || (end == start && start >= window_start)) {
                out.push(Occurrence {
                    summary: event.summary.clone(),
                    location: event.location.clone(),
                    start,
                    end,
                    all_day,
                    calendar: event.calendar,
                });
            }
        }
    }
    out.sort_by(|a, b| a.start.date().cmp(&b.start.date()).then(b.all_day.cmp(&a.all_day)).then(a.start.cmp(&b.start)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> NaiveDateTime {
        date(y, m, d).and_hms_opt(h, min, 0).unwrap()
    }

    fn ics(events: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nVERSION:2.0\r\n{events}END:VCALENDAR\r\n")
    }

    fn starts(text: &str, from: NaiveDate, to: NaiveDate) -> Vec<NaiveDateTime> {
        occurrences(&parse(&ics(text), 0), from, to).into_iter().map(|o| o.start).collect()
    }

    #[test]
    fn lines_unfold_and_text_unescapes() {
        let text = ics("BEGIN:VEVENT\r\nUID:1\r\nSUMMARY:Lunch\\, with\r\n  friends\\nand more\r\nDTSTART:20261023T120000\r\nEND:VEVENT\r\n");
        let events = parse(&text, 3);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].summary, "Lunch, with friends and more");
        assert_eq!(events[0].calendar, 3);
    }

    #[test]
    fn alarms_inside_an_event_do_not_override_it() {
        let text = ics("BEGIN:VEVENT\r\nUID:1\r\nSUMMARY:Real\r\nDTSTART:20261023T120000\r\nBEGIN:VALARM\r\nSUMMARY:Alarm\r\nEND:VALARM\r\nEND:VEVENT\r\n");
        assert_eq!(parse(&text, 0)[0].summary, "Real");
    }

    #[test]
    fn parameters_may_quote_colons() {
        let p = parse_property(r#"DTSTART;TZID="America/New_York":20261023T090000"#).unwrap();
        assert_eq!((p.name.as_str(), p.param("TZID"), p.value.as_str()), ("DTSTART", Some("America/New_York"), "20261023T090000"));
    }

    #[test]
    fn durations_read_every_unit() {
        assert_eq!(parse_duration("PT1H30M"), Some(5400));
        assert_eq!(parse_duration("P1D"), Some(86_400));
        assert_eq!(parse_duration("-PT15M"), Some(-900));
        assert_eq!(parse_duration("P2W"), Some(14 * 86_400));
        assert_eq!(parse_duration("1H"), None);
    }

    #[test]
    fn all_day_events_cover_their_days() {
        let text = "BEGIN:VEVENT\r\nUID:a\r\nSUMMARY:Trip\r\nDTSTART;VALUE=DATE:20261023\r\nDTEND;VALUE=DATE:20261025\r\nEND:VEVENT\r\n";
        let found = occurrences(&parse(&ics(text), 0), date(2026, 10, 24), date(2026, 10, 25));
        assert_eq!(found.len(), 1, "the second day of a two-day event");
        assert!(found[0].all_day);
        assert_eq!(found[0].end, at(2026, 10, 25, 0, 0));
        assert!(occurrences(&parse(&ics(text), 0), date(2026, 10, 25), date(2026, 10, 26)).is_empty(), "DTEND is exclusive");
    }

    #[test]
    fn a_weekly_series_skips_exceptions_and_moved_instances() {
        let text = "BEGIN:VEVENT\r\nUID:s\r\nSUMMARY:Standup\r\nDTSTART:20261005T090000\r\nDTEND:20261005T091500\r\n\
                    RRULE:FREQ=WEEKLY;BYDAY=MO,WE\r\nEXDATE:20261007T090000\r\nEND:VEVENT\r\n\
                    BEGIN:VEVENT\r\nUID:s\r\nSUMMARY:Standup (moved)\r\nRECURRENCE-ID:20261012T090000\r\nDTSTART:20261012T140000\r\nDTEND:20261012T141500\r\nEND:VEVENT\r\n";
        let found = occurrences(&parse(&ics(text), 0), date(2026, 10, 5), date(2026, 10, 15));
        let summary: Vec<(NaiveDateTime, &str)> = found.iter().map(|o| (o.start, o.summary.as_str())).collect();
        assert_eq!(
            summary,
            [
                (at(2026, 10, 5, 9, 0), "Standup"),
                // The 7th is an exception; the 12th moved to the afternoon.
                (at(2026, 10, 12, 14, 0), "Standup (moved)"),
                (at(2026, 10, 14, 9, 0), "Standup"),
            ]
        );
        assert_eq!(found[0].end, at(2026, 10, 5, 9, 15));
    }

    #[test]
    fn count_and_until_end_a_series() {
        let counted = "BEGIN:VEVENT\r\nUID:c\r\nDTSTART:20261001T100000\r\nRRULE:FREQ=DAILY;INTERVAL=2;COUNT=3\r\nEND:VEVENT\r\n";
        assert_eq!(starts(counted, date(2026, 9, 1), date(2026, 12, 1)), [at(2026, 10, 1, 10, 0), at(2026, 10, 3, 10, 0), at(2026, 10, 5, 10, 0)]);
        let until = "BEGIN:VEVENT\r\nUID:u\r\nDTSTART;VALUE=DATE:20261001\r\nRRULE:FREQ=WEEKLY;UNTIL=20261015\r\nEND:VEVENT\r\n";
        assert_eq!(starts(until, date(2026, 9, 1), date(2026, 12, 1)).len(), 3, "1st, 8th and 15th");
        // COUNT counts from the start, even before the window.
        assert_eq!(starts(counted, date(2026, 10, 4), date(2026, 12, 1)), [at(2026, 10, 5, 10, 0)]);
    }

    #[test]
    fn monthly_rules_pick_ordinal_weekdays_and_month_days() {
        let second_tuesday = "BEGIN:VEVENT\r\nUID:m\r\nDTSTART:20261013T180000\r\nRRULE:FREQ=MONTHLY;BYDAY=2TU\r\nEND:VEVENT\r\n";
        assert_eq!(starts(second_tuesday, date(2026, 10, 1), date(2027, 1, 1)), [at(2026, 10, 13, 18, 0), at(2026, 11, 10, 18, 0), at(2026, 12, 8, 18, 0)]);
        let last_friday = "BEGIN:VEVENT\r\nUID:l\r\nDTSTART:20261030T170000\r\nRRULE:FREQ=MONTHLY;BYDAY=-1FR\r\nEND:VEVENT\r\n";
        assert_eq!(starts(last_friday, date(2026, 11, 1), date(2026, 12, 1)), [at(2026, 11, 27, 17, 0)]);
        // The 31st does not exist in November: that month is skipped.
        let the_31st = "BEGIN:VEVENT\r\nUID:t\r\nDTSTART;VALUE=DATE:20261031\r\nRRULE:FREQ=MONTHLY\r\nEND:VEVENT\r\n";
        assert_eq!(starts(the_31st, date(2026, 10, 1), date(2027, 1, 1)).len(), 2);
        let last_day = "BEGIN:VEVENT\r\nUID:d\r\nDTSTART;VALUE=DATE:20261031\r\nRRULE:FREQ=MONTHLY;BYMONTHDAY=-1\r\nEND:VEVENT\r\n";
        assert_eq!(starts(last_day, date(2026, 11, 1), date(2026, 12, 1)), [at(2026, 11, 30, 0, 0)]);
    }

    #[test]
    fn yearly_birthdays_repeat() {
        let birthday = "BEGIN:VEVENT\r\nUID:b\r\nSUMMARY:Birthday\r\nDTSTART;VALUE=DATE:19900314\r\nRRULE:FREQ=YEARLY\r\nEND:VEVENT\r\n";
        assert_eq!(starts(birthday, date(2027, 1, 1), date(2028, 1, 1)), [at(2027, 3, 14, 0, 0)]);
    }

    #[test]
    fn cancelled_events_and_instances_are_left_out() {
        let text = "BEGIN:VEVENT\r\nUID:x\r\nDTSTART:20261023T100000\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n\
                    BEGIN:VEVENT\r\nUID:y\r\nDTSTART:20261020T100000\r\nRRULE:FREQ=DAILY;COUNT=5\r\nEND:VEVENT\r\n\
                    BEGIN:VEVENT\r\nUID:y\r\nRECURRENCE-ID:20261021T100000\r\nDTSTART:20261021T100000\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n";
        let found = starts(text, date(2026, 10, 1), date(2026, 11, 1));
        assert_eq!(found.len(), 4, "the 21st was cancelled, and so was x");
        assert!(!found.contains(&at(2026, 10, 21, 10, 0)));
    }

    #[test]
    fn zoned_and_utc_times_reach_local_time() {
        let mut zones = Zones::new();
        // Whatever this machine's zone, a zoned time and its UTC instant agree.
        let zoned = zones.local(&When::Zoned(at(2026, 7, 1, 9, 0), "America/New_York".into()));
        let utc = zones.local(&When::Utc(at(2026, 7, 1, 13, 0)));
        assert_eq!(zoned, utc, "9:00 EDT is 13:00 UTC");
        let prefixed = zones.local(&When::Zoned(at(2026, 7, 1, 9, 0), "/mozilla.org/20050126_1/America/New_York".into()));
        assert_eq!(prefixed, zoned);
        // The zones come from the system's zoneinfo, not just the local one:
        // 9:00 in Berlin and 9:00 in New York are six hours apart.
        assert!(zones.get("Europe/Berlin").is_some() && zones.get("Eastern Standard Time").is_none());
        let berlin = zones.local(&When::Zoned(at(2026, 7, 1, 9, 0), "Europe/Berlin".into()));
        assert_eq!(zoned - berlin, chrono::Duration::hours(6));
        // An unknown zone reads as local wall time.
        assert_eq!(zones.local(&When::Zoned(at(2026, 7, 1, 9, 0), "Eastern Standard Time".into())), at(2026, 7, 1, 9, 0));
    }

    #[test]
    fn a_series_keeps_its_wall_time_across_daylight_saving() {
        // 9:00 New York, weekly across the November change: the UTC instant
        // moves by an hour, the wall time does not.
        let mut zones = Zones::new();
        let rule = parse_rule("FREQ=WEEKLY;COUNT=2", Some("America/New_York")).unwrap();
        let walls = expand(&rule, at(2026, 10, 29, 9, 0), at(2027, 1, 1, 0, 0), None);
        assert_eq!(walls, [at(2026, 10, 29, 9, 0), at(2026, 11, 5, 9, 0)]);
        let mut utc = |w: NaiveDateTime| {
            let local = zones.local(&When::Zoned(w, "America/New_York".into()));
            let offset = zones.local(&When::Utc(w)) - w;
            local - offset
        };
        assert_eq!(utc(walls[0]), at(2026, 10, 29, 13, 0));
        assert_eq!(utc(walls[1]), at(2026, 11, 5, 14, 0));
    }

    #[test]
    fn errors_never_repeat_the_address() {
        assert_eq!(strip_urls("curl: (6) Could not resolve https://secret.example/abc.ics"), "curl: (6) Could not resolve <address>");
        assert_eq!(curl_reason("curl failed: curl: (22) The requested URL returned error: 404"), "The requested URL returned error: 404");
        assert_eq!(curl_reason("failed to spawn curl: not found"), "failed to spawn curl: not found");
        let url = "https://calendar.example/private-abc123/basic.ics";
        assert_eq!(redact("Could not resolve host: calendar.example", url), "Could not resolve host: <address>");
        assert_eq!(redact("Couldn't open file /private-abc123/basic.ics", url), "Couldn't open file <address>");
    }
}
