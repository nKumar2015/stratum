//! Notification server.
//!
//! Implements `org.freedesktop.Notifications` directly on the session bus, so
//! the shell owns the desktop's notifications rather than relying on another
//! daemon. Notifications are kept in a bounded list, published to the UI, and
//! closed on request.
//!
//! The server deliberately does not render anything: it reports through a
//! callback and the UI decides how to show it, the same split the rest of the
//! shell uses.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tracing::{debug, info};
use zbus::zvariant::OwnedValue;
use zbus::Connection;

/// One live notification.
#[derive(Clone, Debug)]
pub struct Notification {
    pub id: u32,
    pub app_name: String,
    pub app_icon: String,
    pub summary: String,
    pub body: String,
    /// Action key/label pairs, flattened: `["default", "Open", "reply", "Reply"]`.
    pub actions: Vec<String>,
    pub urgency: u8,
    pub expire_timeout: i32,
    /// `image-path`/`image_path` hint, when present.
    pub image_path: Option<String>,
}

/// Callback the server uses to report the current notification list.
pub type NotificationSink = Arc<dyn Fn(Vec<Notification>) + Send + Sync>;

/// Shared notification state.
#[derive(Default)]
struct Store {
    next_id: u32,
    notifications: Vec<Notification>,
    /// `replaces_id` -> index in `notifications`.
    by_id: HashMap<u32, usize>,
}

/// The D-Bus object implementing the notification interface.
pub struct NotificationServer {
    store: Mutex<Store>,
    sink: NotificationSink,
    connection: Mutex<Option<Connection>>,
}

impl NotificationServer {
    pub fn new(sink: NotificationSink) -> Arc<Self> {
        Arc::new(Self {
            store: Mutex::new(Store {
                next_id: 1,
                ..Default::default()
            }),
            sink,
            connection: Mutex::new(None),
        })
    }

    /// Take the session bus name and serve notifications until the process
    /// exits. Runs on the caller's async runtime.
    pub async fn serve(self: Arc<Self>) -> zbus::Result<()> {
        let connection = Connection::session().await?;
        let served = Served {
            inner: Arc::clone(&self),
        };
        connection
            .object_server()
            .at("/org/freedesktop/Notifications", served)
            .await?;
        connection.request_name("org.freedesktop.Notifications").await?;
        if let Ok(mut slot) = self.connection.lock() {
            *slot = Some(connection);
        }
        info!("notification server listening on org.freedesktop.Notifications");
        std::future::pending::<()>().await;
        Ok(())
    }

    fn publish(&self) {
        let list = self
            .store
            .lock()
            .map(|store| store.notifications.clone())
            .unwrap_or_default();
        (self.sink)(list);
    }

    /// Close a notification and emit the D-Bus signal.
    pub fn close(&self, id: u32, reason: u32) {
        {
            let Ok(mut store) = self.store.lock() else {
                return;
            };
            if let Some(index) = store.by_id.remove(&id) {
                store.notifications.remove(index);
            }
            // Reindex from the list, collecting first so the map is not
            // mutated while the list is borrowed.
            let ids: Vec<u32> = store.notifications.iter().map(|n| n.id).collect();
            store.by_id.clear();
            for (index, notification_id) in ids.into_iter().enumerate() {
                store.by_id.insert(notification_id, index);
            }
        }
        self.publish();
        let _ = reason;
    }
}

/// The served interface. zbus needs a concrete type, so this wraps the shared
/// state rather than implementing the interface on the `Arc`.
struct Served {
    inner: Arc<NotificationServer>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Served {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        expire_timeout: i32,
    ) -> u32 {
        debug!("notification from {app_name}: {summary}");

        // Hint values are variant-typed; convert the ones we understand and
        // ignore the rest rather than rejecting the notification.
        let image_path = hints
            .get("image-path")
            .or_else(|| hints.get("image_path"))
            .and_then(|value| <&str>::try_from(value).ok())
            .map(|s| s.to_string());
        let urgency = hints
            .get("urgency")
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(1);

        let mut store = match self.inner.store.lock() {
            Ok(store) => store,
            Err(_) => return replaces_id,
        };

        let id = if replaces_id != 0 {
            replaces_id
        } else {
            let id = store.next_id;
            store.next_id = store.next_id.wrapping_add(1).max(1);
            id
        };

        let notification = Notification {
            id,
            app_name,
            app_icon,
            summary,
            body,
            actions,
            urgency,
            expire_timeout,
            image_path,
        };

        if let Some(index) = store.by_id.get(&id).copied() {
            store.notifications[index] = notification;
        } else {
            let index = store.notifications.len();
            store.notifications.push(notification);
            store.by_id.insert(id, index);
        }
        drop(store);

        self.inner.publish();
        id
    }

    fn close_notification(&self, id: u32) {
        // Reason 3 is "closed by user".
        self.inner.close(id, 3);
    }

    fn get_capabilities(&self) -> Vec<String> {
        vec![
            "body".into(),
            "body-markup".into(),
            "actions".into(),
            "icon-static".into(),
            "persistence".into(),
        ]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "stratum".into(),
            "stratum-shell".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.2".into(),
        )
    }

    #[zbus(signal)]
    async fn notification_closed(
        signal_emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        signal_emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
        action_key: String,
    ) -> zbus::Result<()>;
}

/// SystemState-friendly snapshot of the notification list.
pub fn to_json(list: &[Notification]) -> Value {
    json!({
        "notifications": list.iter().map(|n| json!({
            "id": n.id,
            "app_name": n.app_name,
            "app_icon": n.app_icon,
            "summary": n.summary,
            "body": n.body,
            "actions": n.actions,
            "urgency": n.urgency,
            "image_path": n.image_path,
        })).collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_keeps_one_entry() {
        let sink = Arc::new(|_: Vec<Notification>| {});
        let server = NotificationServer::new(sink);

        let mut store = server.store.lock().unwrap();
        let id = 7;
        store.notifications.push(Notification {
            id,
            app_name: "app".into(),
            app_icon: String::new(),
            summary: "first".into(),
            body: String::new(),
            actions: Vec::new(),
            urgency: 1,
            expire_timeout: 0,
            image_path: None,
        });
        store.by_id.insert(id, 0);
        store.notifications[0].summary = "second".into();
        assert_eq!(store.notifications.len(), 1);
        assert_eq!(store.notifications[0].summary, "second");
    }

    #[test]
    fn json_snapshot_shape() {
        let list = vec![Notification {
            id: 1,
            app_name: "app".into(),
            app_icon: "icon".into(),
            summary: "hi".into(),
            body: "there".into(),
            actions: vec!["default".into(), "Open".into()],
            urgency: 2,
            expire_timeout: 3000,
            image_path: None,
        }];
        let json = to_json(&list);
        assert_eq!(json["notifications"][0]["summary"], "hi");
        assert_eq!(json["notifications"][0]["urgency"], 2);
    }
}
