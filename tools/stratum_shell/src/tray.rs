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
#[derive(Clone, Debug, Default, PartialEq)]
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
    /// The session connection, stored when `serve` takes the watcher name.
    /// Activation and menu calls use it; requests run on the action worker, so
    /// they are synchronous `block_on` calls over zbus's async API.
    connection: Mutex<Option<Connection>>,
}

impl Tray {
    pub fn new(sink: TraySink) -> Arc<Self> {
        Arc::new(Self {
            items: Mutex::new(Vec::new()),
            sink,
            connection: Mutex::new(None),
        })
    }

    fn connection(&self) -> Result<Connection, String> {
        self.connection
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
            .ok_or_else(|| "tray watcher is not running".to_string())
    }

    /// Left-click activation (`org.kde.StatusNotifierItem.Activate`).
    ///
    /// The position arguments are for items that open a menu at the cursor;
    /// zero is what most hosts send.
    pub fn activate(&self, service: &str, path: &str) -> Result<(), String> {
        let connection = self.connection()?;
        pollster::block_on(async {
            connection
                .call_method(
                    Some(service),
                    path,
                    Some("org.kde.StatusNotifierItem"),
                    "Activate",
                    &(0i32, 0i32),
                )
                .await
                .map_err(|err| err.to_string())
        })
        .map(|_| ())
    }

    /// Secondary activation (middle click), used by some applications to toggle
    /// a window.
    pub fn secondary_activate(&self, service: &str, path: &str) -> Result<(), String> {
        let connection = self.connection()?;
        pollster::block_on(async {
            connection
                .call_method(
                    Some(service),
                    path,
                    Some("org.kde.StatusNotifierItem"),
                    "SecondaryActivate",
                    &(0i32, 0i32),
                )
                .await
                .map_err(|err| err.to_string())
        })
        .map(|_| ())
    }

    /// Fetch and flatten the item's DBusMenu.
    ///
    /// `AboutToShow` is called first, which is what lets applications populate
    /// their menu lazily; without it many menus come back empty.
    pub fn menu(&self, service: &str, menu_path: &str) -> Result<Vec<MenuEntry>, String> {
        let connection = self.connection()?;
        pollster::block_on(async {
            let _ = connection
                .call_method(
                    Some(service),
                    menu_path,
                    Some("com.canonical.dbusmenu"),
                    "AboutToShow",
                    &(0i32,),
                )
                .await;

            let reply = connection
                .call_method(
                    Some(service),
                    menu_path,
                    Some("com.canonical.dbusmenu"),
                    "GetLayout",
                    &(0i32, -1i32, Vec::<String>::new()),
                )
                .await
                .map_err(|err| err.to_string())?;

            let body = reply.body();
            let (_revision, layout): (u32, OwnedValue) =
                body.deserialize().map_err(|err| err.to_string())?;
            let mut entries = Vec::new();
            decode_layout(&layout, &mut entries, 0, "0");
            Ok(entries)
        })
    }

    /// Send a menu item's `clicked` event.
    pub fn menu_event(&self, service: &str, menu_path: &str, id: i32) -> Result<(), String> {
        let connection = self.connection()?;
        pollster::block_on(async {
            let data = OwnedValue::try_from(zbus::zvariant::Value::from(0i32))
                .map_err(|err| err.to_string())?;
            connection
                .call_method(
                    Some(service),
                    menu_path,
                    Some("com.canonical.dbusmenu"),
                    "Event",
                    &(id, "clicked", &data, 0u32),
                )
                .await
                .map_err(|err| err.to_string())
        })
        .map(|_| ())
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

/// One entry in a tray item's context menu.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuEntry {
    /// DBusMenu id, used both as an identifier and (in the QML shell's model)
    /// as the node path. Kept separately so nested items stay distinct.
    pub id: i32,
    /// Node path, e.g. `0/1/2`.
    pub path: String,
    pub label: String,
    pub enabled: bool,
    pub visible: bool,
    pub separator: bool,
    /// Nesting depth, for indentation.
    pub depth: usize,
    /// Whether this entry has visible children.
    pub has_children: bool,
}

impl MenuEntry {
    /// Whether the entry should be drawn and can be clicked.
    pub fn selectable(&self) -> bool {
        self.visible && !self.separator && self.enabled && !self.has_children
    }
}

/// A snapshot for the UI, published through [`crate::sys_state::SystemState`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TrayMenu {
    pub service: String,
    /// Object path of the DBusMenu interface.
    pub path: String,
    pub entries: Vec<MenuEntry>,
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

/// Recursively decode a DBusMenu layout node.
///
/// The wire type is `(ia{sv}av)` for the node, where the `av` holds child
/// nodes in the same shape.
fn decode_layout(value: &OwnedValue, out: &mut Vec<MenuEntry>, depth: usize, path: &str) {
    let Ok(clone) = value.try_clone() else {
        return;
    };
    let Ok((id, properties, children)) =
        <(i32, std::collections::HashMap<String, OwnedValue>, Vec<OwnedValue>)>::try_from(clone)
    else {
        return;
    };

    let text = |key: &str| {
        properties
            .get(key)
            .and_then(|value| <&str>::try_from(value).ok())
            .map(str::to_string)
    };
    let flag = |key: &str| {
        properties
            .get(key)
            .and_then(|value| bool::try_from(value).ok())
    };

    // The root node (id 0) is a container with no label; skip it but keep its
    // children, which is what the QML shell's menu opener shows.
    let is_root = path == "0";
    if !is_root {
        out.push(MenuEntry {
            id,
            path: path.to_string(),
            label: text("label").unwrap_or_default().replace('&', ""),
            enabled: flag("enabled").unwrap_or(true),
            visible: flag("visible").unwrap_or(true),
            separator: text("type").as_deref() == Some("separator"),
            depth,
            has_children: !children.is_empty(),
        });
    }

    for (index, child) in children.iter().enumerate() {
        let child_path = format!("{path}/{index}");
        decode_layout(
            child,
            out,
            if is_root { depth } else { depth + 1 },
            &child_path,
        );
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
    if let Ok(mut slot) = tray.connection.lock() {
        *slot = Some(connection.clone());
    }
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

    /// Build a DBusMenu layout node in the wire shape `(ia{sv}av)`.
    fn node(
        id: i32,
        props: &[(&str, zbus::zvariant::Value<'static>)],
        children: Vec<OwnedValue>,
    ) -> OwnedValue {
        let properties: std::collections::HashMap<String, OwnedValue> = props
            .iter()
            .map(|(key, value)| {
                (
                    key.to_string(),
                    OwnedValue::try_from(value.try_clone().unwrap()).unwrap(),
                )
            })
            .collect();
        let children: Vec<OwnedValue> = children;
        // DBusMenu nodes reach us as a serialized struct; build the same shape
        // here so the decoder is exercised against real wire types.
        let mut builder = zbus::zvariant::StructureBuilder::new();
        builder.push_value(zbus::zvariant::Value::from(id));
        builder.push_value(zbus::zvariant::Value::from(properties));
        builder.push_value(zbus::zvariant::Value::from(children));
        let structure = builder.build().unwrap();
        OwnedValue::try_from(zbus::zvariant::Value::from(structure)).unwrap()
    }

    #[test]
    fn decodes_menu_tree_and_skips_the_root() {
        let child = node(10, &[("label", zbus::zvariant::Value::from("Open"))], vec![]);
        let disabled = node(
            11,
            &[
                ("label", zbus::zvariant::Value::from("Disabled")),
                ("enabled", zbus::zvariant::Value::from(false)),
            ],
            vec![],
        );
        let separator = node(
            12,
            &[("type", zbus::zvariant::Value::from("separator"))],
            vec![],
        );
        let root = node(0, &[], vec![child, disabled, separator]);

        let mut entries = Vec::new();
        decode_layout(&root, &mut entries, 0, "0");

        // The container itself is not shown; its three children are.
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].label, "Open");
        assert!(entries[0].enabled);
        assert_eq!(entries[0].path, "0/0");
        assert!(!entries[1].enabled);
        assert!(entries[2].separator);
        assert!(!entries[2].selectable());
        assert!(!entries[1].selectable(), "disabled entries must not be clickable");
    }

    #[test]
    fn nested_items_get_depth_and_children_flag() {
        let leaf = node(21, &[("label", zbus::zvariant::Value::from("Nested"))], vec![]);
        let parent = node(20, &[("label", zbus::zvariant::Value::from("More"))], vec![leaf]);
        let root = node(0, &[], vec![parent]);

        let mut entries = Vec::new();
        decode_layout(&root, &mut entries, 0, "0");

        assert_eq!(entries.len(), 2);
        assert!(entries[0].has_children);
        assert!(!entries[0].selectable(), "a submenu parent does not activate");
        assert_eq!(entries[1].depth, 1);
        assert_eq!(entries[1].path, "0/0/0");
        assert_eq!(entries[1].label, "Nested");
    }

    /// A fake StatusNotifierItem served on the session bus.
    ///
    /// This exercises the real call paths (`Activate`, `GetLayout`) rather than
    /// only the decoder, which is the part most likely to break against a real
    /// application.
    struct FakeItem {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[zbus::interface(name = "org.kde.StatusNotifierItem")]
    impl FakeItem {
        fn activate(&self, _x: i32, _y: i32) {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push("activate".into());
            }
        }

        fn secondary_activate(&self, _x: i32, _y: i32) {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push("secondary".into());
            }
        }

        #[zbus(property)]
        fn menu(&self) -> OwnedObjectPath {
            OwnedObjectPath::try_from("/Menu").unwrap()
        }
    }

    /// The DBusMenu half. zbus generates one `Interface` impl per annotated
    /// block, so the two interfaces need distinct types.
    struct FakeMenu {
        calls: Arc<Mutex<Vec<String>>>,
        layout: OwnedValue,
    }

    #[zbus::interface(name = "com.canonical.dbusmenu")]
    impl FakeMenu {
        fn about_to_show(&self, _id: i32) -> bool {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push("about_to_show".into());
            }
            true
        }

        fn get_layout(
            &self,
            _parent: i32,
            _depth: i32,
            _properties: Vec<String>,
        ) -> (u32, OwnedValue) {
            (1, self.layout.clone())
        }
    }

    #[test]
    fn activation_and_menu_work_over_dbus() {
        let name = format!("org.stratum.FakeTray{}", std::process::id());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let layout = node(
            0,
            &[],
            vec![node(
                1,
                &[("label", zbus::zvariant::Value::from("Open"))],
                vec![],
            )],
        );
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();

        // Serve the fake item on a dedicated thread. Incoming method calls need
        // an executor to dispatch them; `block_on` provides one, and the client
        // below uses `pollster::block_on` on a *different* thread, which must not
        // inherit this one's async context.
        let calls_for_server = Arc::clone(&calls);
        let name_for_server = name.clone();
        let server = std::thread::spawn(move || {
            pollster::block_on(async move {
                let Ok(connection) = zbus::Connection::session().await else {
                    return;
                };
                if connection.request_name(name_for_server.as_str()).await.is_err() {
                    return;
                }
                if connection
                    .object_server()
                    .at(
                        "/StatusNotifierItem",
                        FakeItem {
                            calls: Arc::clone(&calls_for_server),
                        },
                    )
                    .await
                    .is_err()
                {
                    return;
                }
                if connection
                    .object_server()
                    .at(
                        "/Menu",
                        FakeMenu {
                            calls: Arc::clone(&calls_for_server),
                            layout,
                        },
                    )
                    .await
                    .is_err()
                {
                    return;
                }
                let _ = ready_tx.send(());
                // Keep dispatching until the test process exits.
                std::future::pending::<()>().await;
            });
        });

        if ready_rx.recv_timeout(std::time::Duration::from_secs(5)).is_err() {
            // No session bus or the objects could not be served; nothing to
            // assert (the decoder tests still run).
            eprintln!("skipping: could not serve the fake tray item");
            return;
        }

        let Ok(client) = pollster::block_on(zbus::Connection::session()) else {
            return;
        };
        let tray = Tray::new(Arc::new(|_| {}));
        *tray.connection.lock().unwrap() = Some(client);

        tray.activate(&name, "/StatusNotifierItem")
            .expect("Activate should succeed");
        tray.secondary_activate(&name, "/StatusNotifierItem")
            .expect("SecondaryActivate should succeed");

        let entries = tray.menu(&name, "/Menu").expect("GetLayout should succeed");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "Open");

        let calls = calls.lock().unwrap().clone();
        assert!(calls.contains(&"activate".to_string()));
        assert!(calls.contains(&"secondary".to_string()));
        assert!(
            calls.contains(&"about_to_show".to_string()),
            "menus must be asked to populate before the layout is read"
        );
        drop(server);
    }
}
