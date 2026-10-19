//! System tray.
//!
//! Implements the `org.kde.StatusNotifierWatcher` service and consumes
//! `org.kde.StatusNotifierItem` objects, which is how tray applications
//! advertise icons. The watcher is what applications register with; the host
//! flag is what makes them believe someone is displaying them.
//!
//! Like the notification server this is opt-in, because only one host can own
//! the watcher name and Quickshell currently does during migration.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tracing::{debug, info};
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

/// A tray item as reported by its D-Bus properties.
#[derive(Clone, Debug, Default)]
pub struct TrayItem {
    /// Bus name and object path used to talk to it.
    pub service: String,
    pub path: String,
    pub id: String,
    pub title: String,
    /// `Passive`, `Active`, or `NeedsAttention`.
    pub status: String,
    pub icon_name: String,
    /// ARGB32 pixmaps, newest preferred first.
    pub icon_pixmaps: Vec<(u32, u32, Vec<u8>)>,
    pub tooltip_title: String,
    pub tooltip_body: String,
    /// Menu object path, when the item exposes a DBusMenu.
    pub menu: Option<String>,
}

/// Callback the tray uses to publish its item list.
pub type TraySink = Arc<dyn Fn(Vec<TrayItem>) + Send + Sync>;

/// Shared tray state.
pub struct Tray {
    items: Mutex<Vec<TrayItem>>,
    sink: TraySink,
}

impl Tray {
    pub fn new(sink: TraySink) -> Arc<Self> {
        Arc::new(Self {
            items: Mutex::new(Vec::new()),
            sink,
        })
    }

    pub fn snapshot(&self) -> Vec<TrayItem> {
        self.items
            .lock()
            .map(|items| items.clone())
            .unwrap_or_default()
    }

    fn publish(&self) {
        (self.sink)(self.snapshot());
    }

    /// Replace the item list, preserving entries that disappeared only
    /// transiently.
    fn set_items(&self, items: Vec<TrayItem>) {
        if let Ok(mut slot) = self.items.lock() {
            *slot = items;
        }
        self.publish();
    }
}

/// The served watcher interface.
struct Watcher {
    inner: Arc<Tray>,
}

#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    /// Applications call this when they want to be shown. `service` is either a
    /// bus name or a bus name plus object path.
    fn register_status_notifier_item(&self, service: String) {
        info!("tray item registered: {service}");
        let (bus, path) = split_service(&service);
        let mut items = self
            .inner
            .items
            .lock()
            .map(|items| items.clone())
            .unwrap_or_default();

        if !items.iter().any(|item| item.service == bus) {
            // Properties are filled in asynchronously by the shell's poller;
            // register the identity now so the item appears immediately.
            items.push(TrayItem {
                service: bus.clone(),
                path,
                id: bus.clone(),
                title: bus,
                status: "Active".into(),
                ..Default::default()
            });
        }
        drop(items);
        // Re-read through the published path so the sink sees the update.
        let current = self
            .inner
            .items
            .lock()
            .map(|items| items.clone())
            .unwrap_or_default();
        self.inner.set_items(current);
    }

    fn register_status_notifier_host(&self, service: String) {
        debug!("status notifier host registered: {service}");
    }

    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        self.inner
            .snapshot()
            .into_iter()
            .map(|item| item.service)
            .collect()
    }

    #[zbus(property)]
    fn is_status_notifier_host_registered(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn protocol_version(&self) -> i32 {
        0
    }

    #[zbus(signal)]
    async fn status_notifier_item_registered(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        service: String,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn status_notifier_item_unregistered(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        service: String,
    ) -> zbus::Result<()>;
}

/// Split `org.kde.StatusNotifierItem-1234-1/StatusNotifierItem` into bus and path.
fn split_service(value: &str) -> (String, String) {
    match value.find('/') {
        Some(index) => (
            value[..index].to_string(),
            value[index..].to_string(),
        ),
        None => (value.to_string(), "/StatusNotifierItem".to_string()),
    }
}

/// Take the watcher name and serve until the process exits.
///
/// Also starts a poller that refreshes item properties, since SNI has no
/// change notification for most of them.
pub async fn serve(tray: Arc<Tray>) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    connection
        .object_server()
        .at("/StatusNotifierWatcher", Watcher { inner: Arc::clone(&tray) })
        .await?;
    connection.request_name("org.kde.StatusNotifierWatcher").await?;
    info!("tray watcher listening on org.kde.StatusNotifierWatcher");

    // Refresh item properties periodically; SNI exposes no signal for them.
    let refresh_tray = Arc::clone(&tray);
    let refresh_connection = connection.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            refresh_items(&refresh_tray, &refresh_connection).await;
        }
    });

    std::future::pending::<()>().await;
    Ok(())
}

use zbus::Connection;

/// Read the properties of every registered item.
async fn refresh_items(tray: &Arc<Tray>, connection: &Connection) {
    let current = tray.snapshot();
    if current.is_empty() {
        return;
    }

    let mut updated = Vec::with_capacity(current.len());
    for mut item in current {
        match read_item(connection, &item.service, &item.path).await {
            Ok(fresh) => {
                item.id = fresh.0;
                item.title = fresh.1;
                item.status = fresh.2;
                item.icon_name = fresh.3;
                item.icon_pixmaps = fresh.4;
                item.tooltip_title = fresh.5;
                item.tooltip_body = fresh.6;
                item.menu = fresh.7;
                updated.push(item);
            }
            Err(err) => {
                debug!("tray item {} unavailable: {err}", item.service);
                // Keep it; a restart may make it reachable again.
                updated.push(item);
            }
        }
    }
    tray.set_items(updated);
}

type ItemProperties = (
    String,
    String,
    String,
    String,
    Vec<(u32, u32, Vec<u8>)>,
    String,
    String,
    Option<String>,
);

/// Read the StatusNotifierItem properties for one item.
async fn read_item(
    connection: &Connection,
    service: &str,
    path: &str,
) -> Result<ItemProperties, String> {
    let proxy = zbus::Proxy::new(
        connection,
        service,
        path,
        "org.kde.StatusNotifierItem",
    )
    .await
    .map_err(|e| e.to_string())?;

    let get = |name: &'static str| {
        let proxy = &proxy;
        async move {
            proxy
                .get_property::<OwnedValue>(name)
                .await
                .map_err(|e| e.to_string())
        }
    };

    let id = str_prop(get("Id").await).unwrap_or_default();
    let title = str_prop(get("Title").await).unwrap_or_default();
    let status = str_prop(get("Status").await).unwrap_or_else(|| "Active".into());
    let icon_name = str_prop(get("IconName").await).unwrap_or_default();

    let icon_pixmaps = match get("IconPixmap").await {
        Ok(value) => decode_pixmaps(&value),
        Err(_) => Vec::new(),
    };

    let (tooltip_title, tooltip_body) = match get("ToolTip").await {
        Ok(value) => decode_tooltip(&value),
        Err(_) => (String::new(), String::new()),
    };

    let menu = match get("Menu").await {
        Ok(value) => <OwnedObjectPath>::try_from(value)
            .ok()
            .map(|path| path.to_string()),
        Err(_) => None,
    };

    Ok((id, title, status, icon_name, icon_pixmaps, tooltip_title, tooltip_body, menu))
}

fn str_prop(value: Result<OwnedValue, String>) -> Option<String> {
    let value = value.ok()?;
    <&str>::try_from(&value).ok().map(|s| s.to_string())
}

/// SNI `IconPixmap` is `a(iiay)` of ARGB32 in network byte order.
fn decode_pixmaps(value: &OwnedValue) -> Vec<(u32, u32, Vec<u8>)> {
    let mut out = Vec::new();
    let Ok(array) = value.try_clone() else {
        return out;
    };
    let Ok(entries) = Vec::<(i32, i32, Vec<u8>)>::try_from(array) else {
        return out;
    };
    for (width, height, data) in entries {
        if width <= 0 || height <= 0 {
            continue;
        }
        // ARGB in network order -> RGBA premultiplied for our atlas.
        let mut rgba = Vec::with_capacity(data.len());
        for px in data.chunks_exact(4) {
            let a = px[0];
            let r = ((px[1] as u32 * a as u32) / 255) as u8;
            let g = ((px[2] as u32 * a as u32) / 255) as u8;
            let b = ((px[3] as u32 * a as u32) / 255) as u8;
            rgba.extend_from_slice(&[r, g, b, a]);
        }
        out.push((width as u32, height as u32, rgba));
    }
    out
}

/// SNI `ToolTip` is `(sa(iiay)ss)`: icon, title, body. Only text is used.
fn decode_tooltip(value: &OwnedValue) -> (String, String) {
    let Ok(clone) = value.try_clone() else {
        return (String::new(), String::new());
    };
    // ToolTip is (s a(iiay) s s): icon name, icon pixmaps, title, body.
    let Ok((_icon, _pixmaps, title, body)) =
        <(String, OwnedValue, String, String)>::try_from(clone)
    else {
        return (String::new(), String::new());
    };
    (title, body)
}

/// Snapshot for UI state.
pub fn to_json(items: &[TrayItem]) -> Value {
    json!({
        "items": items.iter().map(|item| json!({
            "id": item.id,
            "title": item.title,
            "status": item.status,
            "icon": item.icon_name,
            "menu": item.menu,
        })).collect::<Vec<_>>()
    })
}

/// Format a tray item list for logging.
#[allow(dead_code)]
fn summary(items: &HashMap<String, TrayItem>) -> String {
    items.values().map(|i| i.id.as_str()).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_service_and_path() {
        let (bus, path) = split_service("org.kde.StatusNotifierItem-1234-1/StatusNotifierItem");
        assert_eq!(bus, "org.kde.StatusNotifierItem-1234-1");
        assert_eq!(path, "/StatusNotifierItem");
    }

    #[test]
    fn defaults_path_when_absent() {
        let (bus, path) = split_service("org.example.App");
        assert_eq!(bus, "org.example.App");
        assert_eq!(path, "/StatusNotifierItem");
    }

    #[test]
    fn json_snapshot_lists_items() {
        let items = vec![TrayItem {
            id: "app".into(),
            title: "App".into(),
            status: "Active".into(),
            icon_name: "app-icon".into(),
            ..Default::default()
        }];
        let json = to_json(&items);
        assert_eq!(json["items"][0]["id"], "app");
        assert_eq!(json["items"][0]["icon"], "app-icon");
    }
}
