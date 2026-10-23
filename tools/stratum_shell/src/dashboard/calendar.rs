//! The calendar's data: ICS feeds listed in `[dashboard.calendar]
//! urls_file`, fetched on a thread of their own only while the dashboard is
//! open, at most every `refresh_min`, and at once when the list changes.
//!
//! Each feed is kept as downloaded in `$XDG_STATE_HOME/stratum/calendar/`
//! (a directory only the user can read: the events are private), so the
//! agenda shows at once after a restart or offline. A feed that fails keeps
//! its last copy. Errors never include an address: the addresses are
//! secrets.

use std::hash::{Hash, Hasher};
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use stratum_sys::managers::calendar::{self, Event};

use crate::config::CalendarConfig;
use crate::sys_state::SystemState;

/// How soon a failed fetch is tried again while the dashboard stays open.
const RETRY: Duration = Duration::from_secs(60);

/// What the panels paint from.
#[derive(Clone, Debug, Default)]
pub struct CalendarState {
    /// Every feed's events.
    pub events: Arc<Vec<Event>>,
    /// Bumped whenever `events` changes, so painting can cache expansions
    /// and comparing states need not compare every event.
    pub generation: u64,
    /// Whether `urls_file` is set at all.
    pub configured: bool,
    pub loading: bool,
    /// Why the last fetch (or part of it) failed.
    pub error: Option<String>,
}

impl PartialEq for CalendarState {
    fn eq(&self, other: &Self) -> bool {
        (self.generation, self.configured, self.loading, &self.error)
            == (other.generation, other.configured, other.loading, &other.error)
    }
}

fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn cache_dir() -> Option<PathBuf> {
    crate::theme_choice::state_dir().map(|d| d.join("stratum/calendar"))
}

/// The addresses in a URL list: one per line, `#` comments and blank lines
/// skipped.
pub fn parse_urls(text: &str) -> Vec<String> {
    text.lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

fn hash(urls: &[String]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    urls.hash(&mut hasher);
    hasher.finish()
}

/// Parse the cached copies of feeds `0..count`.
fn load_cached(dir: &Path, count: usize) -> Vec<Event> {
    (0..count)
        .filter_map(|i| std::fs::read_to_string(dir.join(format!("{i}.ics"))).ok().map(|text| (i, text)))
        .flat_map(|(i, text)| calendar::parse(&text, i))
        .collect()
}

/// When the cache was last written: the list file is rewritten after every
/// fetch.
fn cached_at(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir.join("list")).ok()?.modified().ok()
}

/// Fetches the feeds when due. Owned by `SystemState`.
#[derive(Default)]
pub struct CalendarFeed {
    inner: Mutex<FetchState>,
}

#[derive(Default)]
struct FetchState {
    /// The URL list's hash when the cache was last loaded or fetched;
    /// `None` until then.
    urls: Option<u64>,
    fetched: Option<SystemTime>,
    in_flight: bool,
    last_failure: Option<Instant>,
}

impl CalendarFeed {
    /// Called with each sample while the dashboard is open.
    pub fn refresh_if_due(&self, state: &Arc<SystemState>, config: &CalendarConfig) {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let Some(list) = config.urls_file.as_deref().and_then(crate::lock::expand_home) else {
            if state.calendar.get().configured {
                state.calendar.set(CalendarState::default());
                state.notify();
            }
            *inner = FetchState::default();
            return;
        };
        if inner.in_flight {
            return;
        }
        // Read every time: a sops secret changes under a running shell. The
        // path is not secret; the contents are.
        let urls = match std::fs::read_to_string(&list) {
            Ok(text) => parse_urls(&text),
            Err(err) => {
                let error = Some(format!("cannot read {}: {err}", list.display()));
                let mut current = state.calendar.get();
                if current.error != error || !current.configured {
                    (current.error, current.configured) = (error, true);
                    state.calendar.set(current);
                    state.notify();
                }
                return;
            }
        };
        let key = hash(&urls);
        let Some(dir) = cache_dir() else { return };
        if inner.urls.is_none() {
            // First use: show what the last run fetched, if it was for this
            // list; a cache for another list is fetched over at once.
            let stored = std::fs::read_to_string(dir.join("list")).ok().and_then(|s| s.trim().parse::<u64>().ok());
            inner.urls = Some(stored.unwrap_or(!key));
            if stored == Some(key) {
                inner.fetched = cached_at(&dir);
                state.calendar.set(CalendarState {
                    events: Arc::new(load_cached(&dir, urls.len())),
                    generation: next_generation(),
                    configured: true,
                    ..state.calendar.get()
                });
                state.notify();
            }
        }
        let changed = inner.urls != Some(key);
        let stale = inner.fetched.is_none_or(|t| t.elapsed().map_or(true, |age| age >= Duration::from_secs(config.refresh_min * 60)));
        let backing_off = inner.last_failure.is_some_and(|t| t.elapsed() < RETRY);
        if !(changed || (stale && !backing_off)) {
            return;
        }
        inner.in_flight = true;
        inner.urls = Some(key);
        drop(inner);

        let mut current = state.calendar.get();
        (current.loading, current.configured) = (true, true);
        state.calendar.set(current);
        state.notify();

        let state = Arc::clone(state);
        let feed = Arc::clone(&state.calendar_feed);
        let spawned = std::thread::Builder::new().name("stratum-calendar".into()).spawn(move || {
            let error = fetch_all(&dir, &urls, changed);
            // Which list the cache now holds (a hash: the addresses are
            // secrets).
            let _ = std::fs::write(dir.join("list"), key.to_string());
            let events = load_cached(&dir, urls.len());
            let failed = error.is_some();
            state.calendar.set(CalendarState {
                events: Arc::new(events),
                generation: next_generation(),
                configured: true,
                loading: false,
                error,
            });
            let mut inner = feed.inner.lock().unwrap_or_else(|p| p.into_inner());
            inner.in_flight = false;
            inner.fetched = Some(SystemTime::now());
            inner.last_failure = failed.then(Instant::now);
            drop(inner);
            state.notify();
        });
        if spawned.is_err() {
            self.inner.lock().unwrap_or_else(|p| p.into_inner()).in_flight = false;
        }
    }
}

/// Download every feed into `dir` as `<index>.ics`, keeping the last copy of
/// any that fails, and drop copies past the end of the list. A `changed`
/// list drops every copy first: an index may now be another address.
/// Returns what went wrong, if anything.
fn fetch_all(dir: &Path, urls: &[String], changed: bool) -> Option<String> {
    if let Err(err) = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir) {
        return Some(format!("cannot create {}: {err}", dir.display()));
    }
    // The directory may predate this code: keep it private either way.
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    if changed {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            if entry.path().extension().is_some_and(|e| e == "ics") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    let mut errors = Vec::new();
    for (i, url) in urls.iter().enumerate() {
        let part = dir.join(format!("{i}.ics.part"));
        let result = calendar::fetch(url, &part).and_then(|()| {
            std::fs::rename(&part, dir.join(format!("{i}.ics"))).map_err(|err| format!("cannot save calendar {}: {err}", i + 1))
        });
        if let Err(err) = result {
            let _ = std::fs::remove_file(&part);
            tracing::warn!("calendar {}: {err}", i + 1);
            errors.push(if urls.len() > 1 { format!("calendar {}: {err}", i + 1) } else { err });
        }
    }
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name();
        let index = name.to_str().and_then(|n| n.strip_suffix(".ics")).and_then(|n| n.parse::<usize>().ok());
        if index.is_some_and(|i| i >= urls.len()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    (!errors.is_empty()).then(|| errors.join("; "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_lists_skip_comments_and_blanks() {
        let text = "# work\nhttps://a.example/x.ics\n\n  webcal://b.example/y.ics  # family\n#https://off.example\n";
        assert_eq!(parse_urls(text), ["https://a.example/x.ics", "webcal://b.example/y.ics"]);
    }

    #[test]
    fn states_compare_by_generation_not_by_events() {
        let a = CalendarState { generation: 1, ..Default::default() };
        let b = CalendarState { generation: 1, events: Arc::new(Vec::new()), ..Default::default() };
        assert_eq!(a, b);
        assert_ne!(a, CalendarState { generation: 2, ..Default::default() });
    }

    #[test]
    fn copies_past_the_list_are_dropped_and_the_directory_is_private() {
        let dir = std::env::temp_dir().join(format!("stratum-calendar-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("0.ics"), "BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:a\nDTSTART:20261023T100000\nEND:VEVENT\nEND:VCALENDAR\n").unwrap();
        std::fs::write(dir.join("1.ics"), "stale").unwrap();
        // No URLs: nothing to fetch, and both copies are past the end.
        assert_eq!(fetch_all(&dir, &[], false), None);
        assert!(!dir.join("0.ics").exists() && !dir.join("1.ics").exists());
        // A changed list drops copies whose fetch then fails.
        std::fs::write(dir.join("0.ics"), "old").unwrap();
        let error = fetch_all(&dir, &[format!("file://{}/missing.ics", dir.display())], true);
        assert!(error.is_some_and(|e| !e.contains("missing.ics")), "the address stays out of the error");
        assert!(!dir.join("0.ics").exists());
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cached_feeds_keep_their_index() {
        let dir = std::env::temp_dir().join(format!("stratum-calendar-cache-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let event = |uid: &str| format!("BEGIN:VCALENDAR\nBEGIN:VEVENT\nUID:{uid}\nDTSTART:20261023T100000\nEND:VEVENT\nEND:VCALENDAR\n");
        std::fs::write(dir.join("0.ics"), event("a")).unwrap();
        std::fs::write(dir.join("1.ics"), event("b")).unwrap();
        let events = load_cached(&dir, 2);
        assert_eq!(events.iter().map(|e| (e.uid.as_str(), e.calendar)).collect::<Vec<_>>(), [("a", 0), ("b", 1)]);
        assert_eq!(load_cached(&dir, 1).len(), 1, "only the listed feeds");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
