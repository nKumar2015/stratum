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
#[derive(Clone, Debug, Default, PartialEq)]
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
    /// `resident` hint: the notification stays after an action is invoked.
    pub resident: bool,
    /// Set when the notification accepts an inline reply (an `inline-reply`
    /// action), to the placeholder for the reply field.
    pub reply_placeholder: Option<String>,
    /// The last reply sent, shown on notifications that stay after replying.
    pub last_reply: Option<String>,
    /// The `image-data` hint, decoded: raw pixels many apps (Chromium,
    /// Discord, Signal) send instead of a path.
    pub image: Option<NotificationImage>,
}

/// Decoded `image-data`, premultiplied RGBA8, at most `IMAGE_MAX` pixels on
/// a side so a large picture is not re-uploaded every frame.
#[derive(Clone, Debug, PartialEq)]
pub struct NotificationImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
}

const IMAGE_MAX: u32 = 64;

/// Decode the spec's `(iiibiiay)` image: width, height, rowstride, has
/// alpha, bits per sample (8), channels (3 or 4), data.
fn decode_image_data(value: &OwnedValue) -> Option<NotificationImage> {
    let (width, height, stride, has_alpha, bits, channels, data) =
        <(i32, i32, i32, bool, i32, i32, Vec<u8>)>::try_from(value.try_clone().ok()?).ok()?;
    let (width, height, stride, channels) = (
        u32::try_from(width).ok()?,
        u32::try_from(height).ok()?,
        usize::try_from(stride).ok()?,
        usize::try_from(channels).ok()?,
    );
    if bits != 8 || width == 0 || height == 0 || !(3..=4).contains(&channels) || (channels == 4) != has_alpha {
        return None;
    }
    let mut rgba = image::RgbaImage::new(width, height);
    for y in 0..height as usize {
        for x in 0..width as usize {
            let i = y * stride + x * channels;
            let px = data.get(i..i + channels)?;
            let alpha = if channels == 4 { px[3] } else { 255 };
            rgba.put_pixel(x as u32, y as u32, image::Rgba([px[0], px[1], px[2], alpha]));
        }
    }
    let scaled = if width.max(height) > IMAGE_MAX {
        image::imageops::resize(
            &rgba,
            (width * IMAGE_MAX / width.max(height)).max(1),
            (height * IMAGE_MAX / width.max(height)).max(1),
            image::imageops::FilterType::Triangle,
        )
    } else {
        rgba
    };
    let (width, height) = scaled.dimensions();
    let mut pixels = scaled.into_raw();
    for px in pixels.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for c in &mut px[..3] {
            *c = ((*c as u32 * a) / 255) as u8;
        }
    }
    Some(NotificationImage {
        width,
        height,
        pixels: Arc::from(pixels),
    })
}

/// The action key that marks a notification as accepting an inline reply.
pub const INLINE_REPLY: &str = "inline-reply";

impl Notification {
    /// Action key/label pairs, without the inline-reply pseudo-action.
    pub fn action_pairs(&self) -> Vec<(&str, &str)> {
        self.actions
            .chunks_exact(2)
            .map(|pair| (pair[0].as_str(), pair[1].as_str()))
            .filter(|(key, _)| *key != INLINE_REPLY)
            .collect()
    }

    /// Whether the notification has a `default` action, run by clicking it.
    pub fn has_default_action(&self) -> bool {
        self.action_pairs().iter().any(|(key, _)| *key == "default")
    }

    /// The actions drawn as buttons: everything but `default`, which is the
    /// click on the notification itself. An empty label falls back to the key.
    pub fn buttons(&self) -> Vec<(String, String)> {
        self.action_pairs()
            .into_iter()
            .filter(|(key, _)| *key != "default")
            .map(|(key, label)| {
                let label = if label.trim().is_empty() { key } else { label };
                (key.to_string(), label.to_string())
            })
            .collect()
    }
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
    /// The session connection once `serve` has acquired the name. Dismissals
    /// initiated by the shell use it to emit `NotificationClosed`, which is how
    /// clients (and the QML shell during migration) learn a notification is
    /// gone. It is a blocking connection so `close` can be called from the UI
    /// thread.
    connection: Mutex<Option<zbus::blocking::Connection>>,
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
            *slot = Some(zbus::blocking::Connection::from(connection.clone()));
        }
        info!("notification server listening on org.freedesktop.Notifications");
        std::future::pending::<()>().await;
        Ok(())
    }

    /// Put back notifications saved by an earlier run. Called before
    /// `serve`, so no client can race it; new ids continue above `next_id`
    /// and every restored id.
    pub fn restore(&self, notifications: Vec<Notification>, next_id: u32) {
        if let Ok(mut store) = self.store.lock() {
            let max_id = notifications.iter().map(|n| n.id).max().unwrap_or(0);
            store.next_id = next_id.max(max_id.saturating_add(1)).max(1);
            store.by_id = notifications
                .iter()
                .enumerate()
                .map(|(index, n)| (n.id, index))
                .collect();
            store.notifications = notifications;
        }
        self.publish();
    }

    /// The id the next notification will get, for saving.
    pub fn next_id(&self) -> u32 {
        self.store.lock().map(|store| store.next_id).unwrap_or(1)
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
    ///
    /// The signal matters even though we initiated the close: a client that
    /// tracks its own notifications (libnotify, the QML shell during the
    /// migration) only frees its state when it arrives.
    ///
    /// Closing an id that is not live does nothing. Clients commonly call
    /// `CloseNotification` after an action already closed the notification
    /// (notify-send does), and a second `NotificationClosed` for it would be
    /// spurious.
    pub fn close(&self, id: u32, reason: u32) {
        {
            let Ok(mut store) = self.store.lock() else {
                return;
            };
            let Some(index) = store.by_id.remove(&id) else {
                return;
            };
            store.notifications.remove(index);
            // Reindex from the list, collecting first so the map is not
            // mutated while the list is borrowed.
            let ids: Vec<u32> = store.notifications.iter().map(|n| n.id).collect();
            store.by_id.clear();
            for (index, notification_id) in ids.into_iter().enumerate() {
                store.by_id.insert(notification_id, index);
            }
        }
        self.publish();
        self.emit("NotificationClosed", &(id, reason));
    }

    /// Run one of a notification's actions: tell the client, then close the
    /// notification unless it asked to stay (`resident`), as the spec says.
    pub fn invoke(&self, id: u32, key: &str) {
        if !self.knows(id) {
            return;
        }
        self.emit("ActionInvoked", &(id, key));
        if !self.resident(id) {
            // Reason 2 is "dismissed by the user".
            self.close(id, 2);
        }
    }

    /// Send an inline reply (`NotificationReplied`, the extension behind the
    /// `inline-reply` capability), then close like an invoked action.
    pub fn reply(&self, id: u32, text: &str) {
        if !self.knows(id) {
            return;
        }
        self.emit("NotificationReplied", &(id, text));
        if !self.resident(id) {
            self.close(id, 2);
            return;
        }
        if let Ok(mut store) = self.store.lock() {
            if let Some(index) = store.by_id.get(&id).copied() {
                store.notifications[index].last_reply = Some(text.to_string());
            }
        }
        self.publish();
    }

    fn knows(&self, id: u32) -> bool {
        self.store
            .lock()
            .map(|store| store.by_id.contains_key(&id))
            .unwrap_or(false)
    }

    fn resident(&self, id: u32) -> bool {
        self.store
            .lock()
            .ok()
            .and_then(|store| {
                let index = *store.by_id.get(&id)?;
                store.notifications.get(index).map(|n| n.resident)
            })
            .unwrap_or(false)
    }

    /// Emit a signal on the notifications interface, if the name is ours.
    fn emit<B>(&self, name: &str, body: &B)
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let connection = self.connection.lock().ok().and_then(|slot| slot.clone());
        if let Some(connection) = connection {
            if let Err(err) = connection.emit_signal(
                None::<&str>,
                "/org/freedesktop/Notifications",
                "org.freedesktop.Notifications",
                name,
                body,
            ) {
                debug!("failed to emit {name}: {err}");
            }
        }
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
        // Spec 1.2 name first, then the older spellings clients still send.
        let image = ["image-data", "image_data", "icon_data"]
            .iter()
            .find_map(|key| hints.get(*key).and_then(decode_image_data));
        let resident = hints
            .get("resident")
            .and_then(|value| bool::try_from(value).ok())
            .unwrap_or(false);
        // An `inline-reply` action opts in to a reply field; KDE's hints carry
        // the placeholder (the QML shell read the same three names).
        let reply_placeholder = actions
            .chunks_exact(2)
            .any(|pair| pair[0] == INLINE_REPLY)
            .then(|| {
                [
                    "x-kde-reply-placeholder-text",
                    "x-kde-reply-placeholder",
                    "x-kde-inline-reply-placeholder-text",
                ]
                .iter()
                .find_map(|key| hints.get(*key).and_then(|v| <&str>::try_from(v).ok()))
                .filter(|text| !text.trim().is_empty())
                .unwrap_or("Reply...")
                .to_string()
            });

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
            resident,
            reply_placeholder,
            last_reply: None,
            image,
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
            "inline-reply".into(),
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

    #[zbus(signal)]
    async fn notification_replied(
        signal_emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
        text: String,
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

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| s.to_string()).collect()
    }

    /// Deliver a notification through the served interface, as a client would.
    fn notify(server: &Arc<NotificationServer>, actions: &[&str], hints: &[(&str, zbus::zvariant::Value<'static>)]) -> u32 {
        let served = Served {
            inner: Arc::clone(server),
        };
        let hints = hints
            .iter()
            .map(|(k, v)| (k.to_string(), OwnedValue::try_from(v.try_clone().unwrap()).unwrap()))
            .collect();
        served.notify("app".into(), 0, String::new(), "s".into(), "b".into(), strings(actions), hints, -1)
    }

    fn server() -> Arc<NotificationServer> {
        NotificationServer::new(Arc::new(|_: Vec<Notification>| {}))
    }

    #[test]
    fn buttons_leave_out_default_and_reply() {
        let n = Notification {
            actions: strings(&["default", "Open", "inline-reply", "Reply", "mark", "", "archive", "Archive"]),
            ..Default::default()
        };
        assert!(n.has_default_action());
        assert_eq!(
            n.buttons(),
            vec![("mark".to_string(), "mark".to_string()), ("archive".into(), "Archive".into())]
        );
        // A dangling key without a label is ignored rather than misread.
        let odd = Notification {
            actions: strings(&["default"]),
            ..Default::default()
        };
        assert!(!odd.has_default_action());
    }

    #[test]
    fn invoking_closes_unless_resident() {
        let server = server();
        let id = notify(&server, &["default", "Open"], &[]);
        server.invoke(id, "default");
        assert!(!server.knows(id), "an invoked action closes the notification");

        let id = notify(&server, &["default", "Open"], &[("resident", true.into())]);
        server.invoke(id, "default");
        assert!(server.knows(id), "resident notifications stay");
        server.reply(id, "hi");
        assert!(server.knows(id));
        let store = server.store.lock().unwrap();
        assert_eq!(store.notifications[store.by_id[&id]].last_reply.as_deref(), Some("hi"));
        drop(store);

        // Unknown ids are ignored.
        server.invoke(999, "default");
    }

    #[test]
    fn inline_reply_takes_the_kde_placeholder() {
        let server = server();
        let id = notify(
            &server,
            &["inline-reply", "Reply"],
            &[("x-kde-reply-placeholder-text", "Message Ada".into())],
        );
        let id2 = notify(&server, &["inline-reply", ""], &[]);
        let id3 = notify(&server, &["default", "Open"], &[]);
        let store = server.store.lock().unwrap();
        let get = |id: u32| store.notifications[store.by_id[&id]].reply_placeholder.clone();
        assert_eq!(get(id).as_deref(), Some("Message Ada"));
        assert_eq!(get(id2).as_deref(), Some("Reply..."));
        assert_eq!(get(id3), None);
    }

    fn image_value(width: i32, height: i32, channels: i32, data: Vec<u8>) -> OwnedValue {
        let stride = width * channels;
        let value = zbus::zvariant::Value::from(zbus::zvariant::Structure::from((
            width, height, stride, channels == 4, 8i32, channels, data,
        )));
        OwnedValue::try_from(value).unwrap()
    }

    #[test]
    fn image_data_is_decoded_premultiplied() {
        // 2x1: opaque red, then half-transparent white.
        let value = image_value(2, 1, 4, vec![255, 0, 0, 255, 255, 255, 255, 128]);
        let image = decode_image_data(&value).unwrap();
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(&image.pixels[..4], &[255, 0, 0, 255]);
        assert_eq!(&image.pixels[4..], &[128, 128, 128, 128], "premultiplied");

        // RGB without alpha is opaque.
        let rgb = decode_image_data(&image_value(1, 1, 3, vec![10, 20, 30])).unwrap();
        assert_eq!(&rgb.pixels[..], &[10, 20, 30, 255]);
    }

    #[test]
    fn large_images_are_downscaled_and_bad_ones_rejected() {
        let big = decode_image_data(&image_value(256, 128, 3, vec![0; 256 * 128 * 3])).unwrap();
        assert_eq!((big.width, big.height), (IMAGE_MAX, IMAGE_MAX / 2), "aspect kept");
        assert!(decode_image_data(&image_value(4, 4, 4, vec![0; 3])).is_none(), "short data");
    }

    #[test]
    fn restored_notifications_keep_their_ids_and_new_ones_follow() {
        let server = server();
        let restored = vec![
            Notification { id: 4, summary: "old".into(), ..Default::default() },
            Notification { id: 9, summary: "older".into(), ..Default::default() },
        ];
        server.restore(restored, 2);
        assert!(server.knows(9));
        let id = notify(&server, &[], &[]);
        assert_eq!(id, 10, "no collision with a restored id");
        assert_eq!(server.next_id(), 11);
        // Restored notifications can be closed like any other.
        server.close(4, 2);
        assert!(!server.knows(4));
        assert!(server.knows(9) && server.knows(10));
    }

    #[test]
    fn reply_is_advertised() {
        let served = Served { inner: server() };
        assert!(served.get_capabilities().contains(&"inline-reply".to_string()));
    }

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
            ..Default::default()
        });
        store.by_id.insert(id, 0);
        store.notifications[0].summary = "second".into();
        assert_eq!(store.notifications.len(), 1);
        assert_eq!(store.notifications[0].summary, "second");
    }

    #[test]
    fn publish_reports_the_current_list() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let count = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&count);
        let sink: NotificationSink = Arc::new(move |list| {
            seen.store(list.len(), Ordering::SeqCst);
        });
        let server = NotificationServer::new(sink);

        {
            let mut store = server.store.lock().unwrap();
            store.notifications.push(Notification {
                id: 1,
                app_name: "app".into(),
                app_icon: String::new(),
                summary: "one".into(),
                body: String::new(),
                actions: Vec::new(),
                urgency: 1,
                expire_timeout: 0,
                ..Default::default()
            });
            store.by_id.insert(1, 0);
        }
        server.publish();
        assert_eq!(count.load(Ordering::SeqCst), 1);

        server.close(1, 2);
        assert_eq!(count.load(Ordering::SeqCst), 0, "close should republish");
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
            ..Default::default()
        }];
        let json = to_json(&list);
        assert_eq!(json["notifications"][0]["summary"], "hi");
        assert_eq!(json["notifications"][0]["urgency"], 2);
    }
}
