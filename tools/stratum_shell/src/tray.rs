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
use tracing::{debug, info, warn};
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
            // `GetLayout` returns `(u(ia{sv}av))`: the revision and the layout
            // node itself, which is a structure rather than a variant.
            let (_revision, (id, properties, children)): (u32, LayoutNode) =
                body.deserialize().map_err(|err| err.to_string())?;
            let mut entries = Vec::new();
            decode_node(id, &properties, &children, &mut entries, 0, "0");
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
    async fn register_status_notifier_item(
        &self,
        service: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) {
        // libappindicator and Electron register an object path alone; the item
        // is then at that path on the caller's own connection.
        let service = match (service.starts_with('/'), header.sender()) {
            (true, Some(sender)) => format!("{sender}{service}"),
            _ => service,
        };
        info!("tray item registered: {service}");
        let (bus, path) = split_service(&service);
        let added = match self.inner.items.lock() {
            Ok(mut items) if !items.iter().any(|item| item.service == bus) => {
                // Properties are filled in asynchronously by the shell's
                // poller; register the identity now so the item appears
                // immediately.
                items.push(TrayItem {
                    service: bus.clone(),
                    path,
                    id: bus.clone(),
                    title: bus,
                    status: "Active".into(),
                    ..Default::default()
                });
                true
            }
            _ => false,
        };
        if added {
            self.inner.publish();
            let _ = Self::status_notifier_item_registered(&emitter, service).await;
        }
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
/// One DBusMenu layout node, in its wire shape `(ia{sv}av)`.
type LayoutNode = (
    i32,
    std::collections::HashMap<String, OwnedValue>,
    Vec<OwnedValue>,
);

/// Decode a node that arrived as a variant (a child), then recurse.
fn decode_variant(value: &OwnedValue, out: &mut Vec<MenuEntry>, depth: usize, path: &str) {
    let Ok(clone) = value.try_clone() else {
        return;
    };
    let Ok((id, properties, children)) = <LayoutNode>::try_from(clone) else {
        return;
    };
    decode_node(id, &properties, &children, out, depth, path);
}

/// Recursively decode a DBusMenu layout node.
///
/// The node is `(ia{sv}av)`: id, properties, and child *variants*, each of which
/// wraps another node in the same shape.
fn decode_node(
    id: i32,
    properties: &std::collections::HashMap<String, OwnedValue>,
    children: &[OwnedValue],
    out: &mut Vec<MenuEntry>,
    depth: usize,
    path: &str,
) {
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
        decode_variant(
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

    // Own the watcher when the name is free. During migration another shell
    // (Quickshell) usually owns it, so fall back to *consuming* its item list:
    // the icons and menus are the same objects either way, and both shells can
    // then display them side by side.
    // The object is registered before the name is requested so that no call
    // can arrive at a name with nothing behind it; it is withdrawn again if the
    // name turns out to be taken.
    connection
        .object_server()
        .at("/StatusNotifierWatcher", Watcher { inner: Arc::clone(&tray) })
        .await?;
    let owns_watcher = match connection
        .request_name("org.kde.StatusNotifierWatcher")
        .await
    {
        Ok(()) => {
            info!("tray watcher listening on org.kde.StatusNotifierWatcher");
            true
        }
        Err(_) => {
            connection
                .object_server()
                .remove::<Watcher, _>("/StatusNotifierWatcher")
                .await?;
            info!(
                "org.kde.StatusNotifierWatcher is owned by another shell; \
                 consuming its item list"
            );
            false
        }
    };

    if let Ok(mut slot) = tray.connection.lock() {
        *slot = Some(connection.clone());
    }

    // Populate immediately rather than leaving the tray empty for the first
    // poll interval.
    if owns_watcher {
        tokio::spawn(forget_vanished_items(Arc::clone(&tray), connection.clone()));
    } else {
        adopt_foreign_items(&tray, &connection).await;
        refresh_items(&tray, &connection).await;
    }

    // Refresh item properties periodically; SNI exposes no signal for them.
    // When we do not own the watcher, this is also where the item list itself
    // comes from.
    let refresh_tray = Arc::clone(&tray);
    let refresh_connection = connection.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            if !owns_watcher {
                adopt_foreign_items(&refresh_tray, &refresh_connection).await;
            }
            refresh_items(&refresh_tray, &refresh_connection).await;
        }
    });

    std::future::pending::<()>().await;
    Ok(())
}

/// Drop the items of every application that leaves the bus, as the watcher's
/// owner must. Without this a closed app's icon stays in the tray.
async fn forget_vanished_items(tray: Arc<Tray>, connection: Connection) {
    use futures_lite::StreamExt;

    let changes = match zbus::fdo::DBusProxy::new(&connection).await {
        Ok(dbus) => dbus.receive_name_owner_changed().await,
        Err(err) => Err(err),
    };
    let mut changes = match changes {
        Ok(changes) => changes,
        Err(err) => {
            warn!("tray: cannot follow bus names, closed apps will keep their icons: {err}");
            return;
        }
    };
    while let Some(change) = changes.next().await {
        let Ok(args) = change.args() else { continue };
        if args.new_owner().is_some() {
            continue;
        }
        let name = args.name().as_str();
        let removed: Vec<TrayItem> = match tray.items.lock() {
            Ok(mut items) => {
                let (gone, kept) = items.drain(..).partition(|item| item.service == name);
                *items = kept;
                gone
            }
            Err(_) => continue,
        };
        if removed.is_empty() {
            continue;
        }
        info!("tray item gone: {name}");
        tray.publish();
        if let Ok(watcher) = connection
            .object_server()
            .interface::<_, Watcher>("/StatusNotifierWatcher")
            .await
        {
            for item in removed {
                let service = format!("{}{}", item.service, item.path);
                let _ = Watcher::status_notifier_item_unregistered(watcher.signal_emitter(), service)
                    .await;
            }
        }
    }
}

/// Mirror the item list published by another host's watcher.
///
/// Only the identity is read here; `refresh_items` fills in the properties on
/// the same cadence as when we own the watcher.
async fn adopt_foreign_items(tray: &Arc<Tray>, connection: &Connection) {
    let proxy = match zbus::Proxy::new(
        connection,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    )
    .await
    {
        Ok(proxy) => proxy,
        Err(err) => {
            debug!("foreign tray watcher unavailable: {err}");
            return;
        }
    };
    let registered = match proxy
        .get_property::<Vec<String>>("RegisteredStatusNotifierItems")
        .await
    {
        Ok(registered) => registered,
        Err(err) => {
            debug!("could not read the foreign tray item list: {err}");
            return;
        }
    };
    debug!("foreign tray items: {registered:?}");

    let current = tray.snapshot();
    let mut items = Vec::with_capacity(registered.len());
    for service in registered {
        let (bus, path) = split_service(&service);
        // Keep properties already read for items that are still registered.
        if let Some(existing) = current.iter().find(|item| item.service == bus) {
            items.push(existing.clone());
        } else {
            items.push(TrayItem {
                service: bus.clone(),
                path,
                id: bus.clone(),
                title: bus,
                status: "Active".into(),
                ..Default::default()
            });
        }
    }
    tray.set_items(items);
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

    /// Build a node's fields in the wire shape `(ia{sv}av)`.
    fn node_fields(
        id: i32,
        props: &[(&str, zbus::zvariant::Value<'static>)],
        children: Vec<OwnedValue>,
    ) -> LayoutNode {
        let properties: std::collections::HashMap<String, OwnedValue> = props
            .iter()
            .map(|(key, value)| {
                (
                    key.to_string(),
                    OwnedValue::try_from(value.try_clone().unwrap()).unwrap(),
                )
            })
            .collect();
        (id, properties, children)
    }

    /// Wrap a node in a variant, which is how `GetLayout` carries children.
    fn node_variant(fields: LayoutNode) -> OwnedValue {
        let mut builder = zbus::zvariant::StructureBuilder::new();
        for value in [
            zbus::zvariant::Value::from(fields.0),
            zbus::zvariant::Value::from(fields.1),
            zbus::zvariant::Value::from(fields.2),
        ] {
            builder.push_value(value);
        }
        OwnedValue::try_from(zbus::zvariant::Value::from(builder.build().unwrap())).unwrap()
    }

    /// A child node, which travels inside the parent's `av` as a variant.
    fn node(
        id: i32,
        props: &[(&str, zbus::zvariant::Value<'static>)],
        children: Vec<OwnedValue>,
    ) -> OwnedValue {
        node_variant(node_fields(id, props, children))
    }

    /// Decode a root node, the way `GetLayout`'s reply is handled.
    fn decode_root(root: &LayoutNode) -> Vec<MenuEntry> {
        let mut entries = Vec::new();
        decode_node(root.0, &root.1, &root.2, &mut entries, 0, "0");
        entries
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
        let root = node_fields(0, &[], vec![child, disabled, separator]);

        let entries = decode_root(&root);

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
        let root = node_fields(0, &[], vec![parent]);

        let entries = decode_root(&root);

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
        layout: LayoutNode,
    }

    #[zbus::interface(name = "com.canonical.dbusmenu")]
    impl FakeMenu {
        fn about_to_show(&self, _id: i32) -> bool {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push("about_to_show".into());
            }
            true
        }

        /// Must match the real signature `(u(ia{sv}av))`. Returning a variant
        /// here is what let the earlier version pass while the real call failed
        /// with a signature mismatch.
        fn get_layout(
            &self,
            _parent: i32,
            _depth: i32,
            _properties: Vec<String>,
        ) -> (u32, LayoutNode) {
            (
                1,
                (
                    self.layout.0,
                    self.layout.1.clone(),
                    self.layout.2.clone(),
                ),
            )
        }
    }

    #[test]
    fn activation_and_menu_work_over_dbus() {
        let name = format!("org.stratum.FakeTray{}", std::process::id());
        let calls = Arc::new(Mutex::new(Vec::new()));
        let layout = node_fields(
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

    /// Wait up to two seconds for `done` to hold.
    fn eventually(done: impl Fn() -> bool) -> bool {
        for _ in 0..40 {
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        done()
    }

    #[test]
    fn registered_items_stay_until_their_app_leaves() {
        let tray = Tray::new(Arc::new(|_| {}));
        // A name of its own, so the test never takes the session's watcher.
        let name = format!("org.stratum.TestWatcher{}", std::process::id());

        let Some(watcher) = pollster::block_on(async {
            let connection = zbus::Connection::session().await.ok()?;
            connection
                .object_server()
                .at("/StatusNotifierWatcher", Watcher { inner: Arc::clone(&tray) })
                .await
                .ok()?;
            connection.request_name(name.as_str()).await.ok()?;
            Some(connection)
        }) else {
            eprintln!("skipping: no session bus");
            return;
        };
        let forget_tray = Arc::clone(&tray);
        let forget_connection = watcher.clone();
        std::thread::spawn(move || {
            pollster::block_on(forget_vanished_items(forget_tray, forget_connection))
        });

        // Registered by path alone, as libappindicator does: the item is on
        // the caller's own connection. Registering twice keeps one item.
        let app = pollster::block_on(async {
            let app = zbus::Connection::session().await.unwrap();
            for _ in 0..2 {
                app.call_method(
                    Some(name.as_str()),
                    "/StatusNotifierWatcher",
                    Some("org.kde.StatusNotifierWatcher"),
                    "RegisterStatusNotifierItem",
                    &("/org/ayatana/NotificationItem/test",),
                )
                .await
                .unwrap();
            }
            app
        });
        let sender = app.unique_name().unwrap().to_string();
        let items = tray.snapshot();
        assert_eq!(items.len(), 1, "one registration was kept: {items:?}");
        assert_eq!(items[0].service, sender);
        assert_eq!(items[0].path, "/org/ayatana/NotificationItem/test");

        pollster::block_on(app.close()).unwrap();
        assert!(
            eventually(|| tray.snapshot().is_empty()),
            "an app that left the bus keeps its icon"
        );
    }
}
