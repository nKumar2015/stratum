//! Notifications that survive a restart.
//!
//! The QML shell saved its notification list, do-not-disturb, and next id to
//! `~/.local/state/quickshell/notifications-snapshot.json` (through
//! `stratum-cli notifications-snapshot`) 700 ms after the last change, and
//! restored it on start, actions included. This does the same in-process, to
//! its own file (`$XDG_STATE_HOME/stratum/notifications.json`), so the two
//! shells never overwrite each other while both run. When that file does not
//! exist yet, the Quickshell snapshot is imported once, so notifications carry
//! over at the switch.
//!
//! Decoded `image-data` pixels are not saved; an `image-path` is.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::notifications::{Notification, INLINE_REPLY};

/// The QML shell kept this many.
pub const MAX_SAVED: usize = 50;
const VERSION: u64 = 1;
/// QML's save timer.
const DEBOUNCE: Duration = Duration::from_millis(700);

/// What is saved and restored.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub do_not_disturb: bool,
    pub next_id: u32,
    pub notifications: Vec<Notification>,
}

fn state_dir() -> Option<PathBuf> {
    match std::env::var("XDG_STATE_HOME") {
        Ok(dir) if !dir.trim().is_empty() => Some(PathBuf::from(dir)),
        _ => std::env::var("HOME").ok().map(|h| PathBuf::from(h).join(".local/state")),
    }
}

pub fn default_path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("stratum/notifications.json"))
}

fn quickshell_path() -> Option<PathBuf> {
    state_dir().map(|d| d.join("quickshell/notifications-snapshot.json"))
}

/// The saved snapshot, else the Quickshell one, else `None`.
pub fn load() -> Option<Snapshot> {
    if let Some(path) = default_path().filter(|p| p.exists()) {
        return read(&path, from_json);
    }
    let imported = quickshell_path().and_then(|p| read(&p, from_quickshell_json));
    if let Some(snapshot) = &imported {
        tracing::info!("imported {} notifications from the Quickshell snapshot", snapshot.notifications.len());
    }
    imported
}

fn read(path: &Path, parse: fn(&Value) -> Option<Snapshot>) -> Option<Snapshot> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text)
        .map_err(|err| tracing::warn!("ignoring unreadable {}: {err}", path.display()))
        .ok()?;
    parse(&value)
}

/// Keep the newest `MAX_SAVED`, and an id above every one kept.
fn normalise(mut snapshot: Snapshot) -> Snapshot {
    let len = snapshot.notifications.len();
    if len > MAX_SAVED {
        // The list is in arrival order, oldest first.
        snapshot.notifications.drain(..len - MAX_SAVED);
    }
    let max_id = snapshot.notifications.iter().map(|n| n.id).max().unwrap_or(0);
    snapshot.next_id = snapshot.next_id.max(max_id.saturating_add(1)).max(1);
    snapshot
}

pub fn to_json(snapshot: &Snapshot) -> Value {
    let snapshot = normalise(snapshot.clone());
    json!({
        "version": VERSION,
        "do_not_disturb": snapshot.do_not_disturb,
        "next_id": snapshot.next_id,
        "notifications": snapshot.notifications.iter().map(|n| json!({
            "id": n.id,
            "app_name": n.app_name,
            "app_icon": n.app_icon,
            "summary": n.summary,
            "body": n.body,
            "actions": n.actions,
            "urgency": n.urgency,
            "expire_timeout": n.expire_timeout,
            "image_path": n.image_path,
            "resident": n.resident,
            "reply_placeholder": n.reply_placeholder,
            "last_reply": n.last_reply,
        })).collect::<Vec<_>>(),
    })
}

pub fn from_json(value: &Value) -> Option<Snapshot> {
    let list = value["notifications"].as_array()?;
    let text = |v: &Value| v.as_str().map(str::to_string);
    let notifications = list
        .iter()
        .filter_map(|n| {
            Some(Notification {
                id: u32::try_from(n["id"].as_u64()?).ok().filter(|id| *id > 0)?,
                app_name: text(&n["app_name"]).unwrap_or_default(),
                app_icon: text(&n["app_icon"]).unwrap_or_default(),
                summary: text(&n["summary"]).unwrap_or_default(),
                body: text(&n["body"]).unwrap_or_default(),
                actions: n["actions"]
                    .as_array()
                    .map(|a| a.iter().filter_map(text).collect())
                    .unwrap_or_default(),
                urgency: n["urgency"].as_u64().unwrap_or(1).min(2) as u8,
                expire_timeout: n["expire_timeout"].as_i64().unwrap_or(-1) as i32,
                image_path: text(&n["image_path"]),
                resident: n["resident"].as_bool().unwrap_or(false),
                reply_placeholder: text(&n["reply_placeholder"]),
                last_reply: text(&n["last_reply"]),
                image: None,
            })
        })
        .collect();
    Some(normalise(Snapshot {
        do_not_disturb: value["do_not_disturb"].as_bool().unwrap_or(false),
        next_id: value["next_id"].as_u64().unwrap_or(1) as u32,
        notifications,
    }))
}

/// The QML shell's format (`NotificationState.normalizeNotificationEntry`).
/// Its list was newest first.
pub fn from_quickshell_json(value: &Value) -> Option<Snapshot> {
    let list = value["notifications"].as_array()?;
    let text = |v: &Value| v.as_str().unwrap_or_default().to_string();
    let mut notifications: Vec<Notification> = list
        .iter()
        .filter_map(|n| {
            let id = u32::try_from(n["id"].as_u64()?).ok().filter(|id| *id > 0)?;
            let mut actions = Vec::new();
            let mut reply = None;
            for action in n["actions"].as_array().into_iter().flatten() {
                let key = text(&action["id"]);
                if key.is_empty() {
                    continue;
                }
                if action["inlineReply"].as_bool() == Some(true) {
                    reply = Some(key);
                    continue;
                }
                actions.push(key);
                actions.push(text(&action["label"]));
            }
            let placeholder = Some(text(&n["inlineReplyPlaceholder"])).filter(|p| !p.is_empty());
            let reply_placeholder = reply.map(|_| placeholder.unwrap_or_else(|| "Reply...".into()));
            if reply_placeholder.is_some() {
                actions.push(INLINE_REPLY.into());
                actions.push("Reply".into());
            }
            let image = text(&n["imageUrl"]);
            let image_path = image
                .strip_prefix("file://")
                .map(str::to_string)
                .or_else(|| image.starts_with('/').then(|| image.clone()));
            Some(Notification {
                id,
                app_name: text(&n["appName"]),
                app_icon: text(&n["appIcon"]),
                summary: text(&n["summary"]),
                body: text(&n["body"]),
                actions,
                urgency: n["urgency"].as_u64().unwrap_or(1).min(2) as u8,
                expire_timeout: n["expiryMs"].as_i64().unwrap_or(5000) as i32,
                image_path,
                resident: false,
                reply_placeholder,
                last_reply: Some(text(&n["lastInlineReplyText"])).filter(|r| !r.is_empty()),
                image: None,
            })
        })
        .collect();
    notifications.reverse();
    Some(normalise(Snapshot {
        do_not_disturb: value["doNotDisturb"].as_bool().unwrap_or(false),
        next_id: value["nextNotificationId"].as_u64().unwrap_or(1) as u32,
        notifications,
    }))
}

/// Writes snapshots on a background thread, 700 ms after the last change, as
/// the QML shell did. Dropping it flushes the latest snapshot.
pub struct Saver {
    tx: Option<mpsc::Sender<Snapshot>>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Saver {
    pub fn start(path: PathBuf) -> Self {
        let (tx, rx) = mpsc::channel::<Snapshot>();
        let worker = std::thread::Builder::new()
            .name("notification-store".into())
            .spawn(move || {
                while let Ok(mut latest) = rx.recv() {
                    // Coalesce a burst into one write.
                    loop {
                        match rx.recv_timeout(DEBOUNCE) {
                            Ok(next) => latest = next,
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Err(mpsc::RecvTimeoutError::Disconnected) => {
                                write(&path, &latest);
                                return;
                            }
                        }
                    }
                    write(&path, &latest);
                }
            })
            .ok();
        Self { tx: Some(tx), worker }
    }

    pub fn save(&self, snapshot: Snapshot) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(snapshot);
        }
    }
}

impl Drop for Saver {
    fn drop(&mut self) {
        // Closing the channel makes the worker write what it holds and stop.
        drop(self.tx.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Write atomically: a crash mid-write must not leave a truncated file that
/// loses every notification on the next start.
fn write(path: &Path, snapshot: &Snapshot) {
    let result = (|| -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, format!("{}\n", to_json(snapshot)))?;
        std::fs::rename(&tmp, path)
    })();
    if let Err(err) = result {
        tracing::warn!("failed to save notifications to {}: {err}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(id: u32) -> Notification {
        Notification {
            id,
            app_name: "Chat".into(),
            summary: format!("s{id}"),
            body: "b".into(),
            actions: vec!["default".into(), "Open".into(), INLINE_REPLY.into(), "Reply".into()],
            urgency: 1,
            expire_timeout: -1,
            reply_placeholder: Some("Message Ada".into()),
            last_reply: Some("on my way".into()),
            ..Default::default()
        }
    }

    #[test]
    fn round_trips_with_actions() {
        let snapshot = Snapshot {
            do_not_disturb: true,
            next_id: 9,
            notifications: vec![note(3), note(7)],
        };
        let back = from_json(&to_json(&snapshot)).unwrap();
        assert_eq!(back, snapshot);
    }

    #[test]
    fn keeps_the_newest_fifty_and_ids_above_them() {
        let snapshot = Snapshot {
            do_not_disturb: false,
            next_id: 1,
            notifications: (1..=60).map(note).collect(),
        };
        let back = from_json(&to_json(&snapshot)).unwrap();
        assert_eq!(back.notifications.len(), MAX_SAVED);
        assert_eq!(back.notifications.first().unwrap().id, 11, "the oldest are dropped");
        assert_eq!(back.next_id, 61, "new ids cannot collide with restored ones");
    }

    #[test]
    fn imports_the_quickshell_snapshot() {
        let qml = json!({
            "version": 1,
            "doNotDisturb": true,
            "nextNotificationId": 12,
            "notifications": [
                {"id": 11, "appName": "Chat", "summary": "Ada", "body": "Lunch?", "urgency": 2,
                 "actions": [{"id": "default", "label": "Open", "inlineReply": false},
                             {"id": "__inline_reply__", "label": "Reply", "inlineReply": true}],
                 "inlineReplyPlaceholder": "Message Ada", "imageUrl": "file:///tmp/a.png",
                 "lastInlineReplyText": "on my way", "expiryMs": 0},
                {"id": 4, "appName": "Mail", "summary": "Build", "body": "", "actions": []},
            ],
        });
        let snapshot = from_quickshell_json(&qml).unwrap();
        assert!(snapshot.do_not_disturb);
        assert_eq!(snapshot.next_id, 12);
        let ids: Vec<u32> = snapshot.notifications.iter().map(|n| n.id).collect();
        assert_eq!(ids, vec![4, 11], "QML kept newest first; this list is oldest first");
        let chat = &snapshot.notifications[1];
        assert_eq!(chat.actions, vec!["default", "Open", INLINE_REPLY, "Reply"]);
        assert_eq!(chat.reply_placeholder.as_deref(), Some("Message Ada"));
        assert_eq!(chat.image_path.as_deref(), Some("/tmp/a.png"));
        assert_eq!(chat.urgency, 2);
        assert_eq!(chat.expire_timeout, 0);
        assert_eq!(chat.last_reply.as_deref(), Some("on my way"));
    }

    #[test]
    fn an_empty_quickshell_snapshot_is_nothing() {
        assert!(from_quickshell_json(&json!({"version": 1})).is_none());
    }

    #[test]
    fn the_saver_coalesces_and_flushes_on_drop() {
        let dir = std::env::temp_dir().join(format!("stratum-notes-{}", std::process::id()));
        let path = dir.join("notifications.json");
        {
            let saver = Saver::start(path.clone());
            for id in 1..=5 {
                saver.save(Snapshot {
                    notifications: vec![note(id)],
                    ..Default::default()
                });
            }
        } // drop: flush
        let saved = read(&path, from_json).unwrap();
        assert_eq!(saved.notifications.len(), 1);
        assert_eq!(saved.notifications[0].id, 5, "the latest snapshot wins");
        assert!(!path.with_extension("json.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
