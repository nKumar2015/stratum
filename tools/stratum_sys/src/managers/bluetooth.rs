//! Bluetooth through BlueZ's D-Bus API.
//!
//! Talks to `org.bluez` with zbus, which every other manager already uses.
//! This used to go through `bluer`, which brought a second D-Bus stack
//! (libdbus bindings, dbus-crossroads, dbus-tokio) into every binary for a
//! dozen calls.
//!
//! Every call is blocking and runs on a thread of its own with a timeout, so
//! an unresponsive bluetoothd cannot hang the caller. The JSON shapes are the
//! ones the CLI and both shells already read.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};
use zvariant::{OwnedObjectPath, OwnedValue};

const BT_TIMEOUT: Duration = Duration::from_secs(4);
const ACTION_TIMEOUT: Duration = Duration::from_secs(12);

const BLUEZ: &str = "org.bluez";
const ADAPTER: &str = "org.bluez.Adapter1";
const DEVICE: &str = "org.bluez.Device1";

type Properties = HashMap<String, OwnedValue>;
type Objects = HashMap<OwnedObjectPath, HashMap<String, Properties>>;

/// Run `f` on a thread of its own, giving up after `timeout`.
fn with_timeout<T: Send + 'static>(
    timeout: Duration,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err("timeout".to_string()),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err("thread panicked".to_string()),
    }
}

fn system() -> Result<Connection, String> {
    Connection::system().map_err(|err| format!("system bus unavailable: {err}"))
}

/// Every BlueZ object and its interfaces' properties.
fn managed_objects(conn: &Connection) -> Result<Objects, String> {
    let manager = Proxy::new(conn, BLUEZ, "/", "org.freedesktop.DBus.ObjectManager")
        .map_err(|err| err.to_string())?;
    manager
        .call("GetManagedObjects", &())
        .map_err(|err| format!("bluez object query failed: {err}"))
}

fn prop_bool(props: &Properties, key: &str) -> bool {
    props.get(key).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

fn prop_str(props: &Properties, key: &str) -> Option<String> {
    props.get(key).and_then(|v| <&str>::try_from(v).ok()).map(str::to_string)
}

/// The default adapter: the first by object path (`hci0` before `hci1`), as
/// bluer chose it.
fn default_adapter(objects: &Objects) -> Result<(String, &Properties), String> {
    objects
        .iter()
        .filter_map(|(path, interfaces)| interfaces.get(ADAPTER).map(|props| (path.as_str(), props)))
        .min_by(|a, b| a.0.cmp(b.0))
        .map(|(path, props)| (path.to_string(), props))
        .ok_or_else(|| "no bluetooth adapter".to_string())
}

/// The status snapshot from BlueZ's objects: the default adapter's state and
/// its devices (paired ones only unless `include_unpaired`), connected first,
/// then by name.
fn snapshot(objects: &Objects, include_unpaired: bool) -> Result<Value, String> {
    let (adapter_path, adapter) = default_adapter(objects)?;
    if !prop_bool(adapter, "Powered") {
        return Ok(json!({
            "ok": true,
            "state": "off",
            "scanning": "no",
            "devices": [],
        }));
    }
    let scanning = prop_bool(adapter, "Discovering");

    let mut devices: Vec<(bool, String, Value)> = Vec::new();
    for interfaces in objects.values() {
        let Some(device) = interfaces.get(DEVICE) else {
            continue;
        };
        let on_adapter = device
            .get("Adapter")
            .and_then(|v| v.downcast_ref::<zvariant::ObjectPath>().ok())
            .is_some_and(|p| p.as_str() == adapter_path);
        if !on_adapter {
            continue;
        }
        let paired = prop_bool(device, "Paired");
        if !paired && !include_unpaired {
            continue;
        }
        let Some(mac) = prop_str(device, "Address").map(|m| m.to_uppercase()) else {
            continue;
        };
        let connected = prop_bool(device, "Connected");
        let name = prop_str(device, "Name").filter(|n| !n.is_empty()).unwrap_or_else(|| mac.clone());
        let entry = json!({
            "mac": mac,
            "name": name,
            "connected": if connected { "yes" } else { "no" },
            "paired": if paired { "yes" } else { "no" },
            "trusted": if prop_bool(device, "Trusted") { "yes" } else { "no" },
        });
        devices.push((connected, name, entry));
    }
    devices.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let has_connected = devices.iter().any(|d| d.0);

    Ok(json!({
        "ok": true,
        "state": if has_connected { "connected" } else { "on" },
        "scanning": if scanning { "yes" } else { "no" },
        "devices": devices.into_iter().map(|d| d.2).collect::<Vec<_>>(),
    }))
}

fn query(include_unpaired: bool) -> Result<Value, String> {
    with_timeout(BT_TIMEOUT, move || snapshot(&managed_objects(&system()?)?, include_unpaired))
}

pub fn status() -> Value {
    query(false).unwrap_or_else(|error| {
        json!({ "ok": false, "error": error, "state": "off", "scanning": "no", "devices": [] })
    })
}

/// `status()` plus `all_devices`: every device the adapter knows, discovered
/// ones included (what `list()` returns), from the same single query. For a
/// UI that shows both the paired summary and the full list.
pub fn status_with_all() -> Value {
    let result = with_timeout(BT_TIMEOUT, || {
        let objects = managed_objects(&system()?)?;
        let mut status = snapshot(&objects, false)?;
        status["all_devices"] = snapshot(&objects, true)?["devices"].clone();
        Ok(status)
    });
    result.unwrap_or_else(|error| {
        json!({ "ok": false, "error": error, "state": "off", "scanning": "no", "devices": [], "all_devices": [] })
    })
}

pub fn state() -> Value {
    let st = status();
    if let Some(obj) = st.as_object() {
        let powered = obj.get("state").and_then(Value::as_str).unwrap_or("off") != "off";
        let state_val = obj.get("state").and_then(Value::as_str).unwrap_or("off");
        return json!({
            "ok": true,
            "command": "bluetooth",
            "subcommand": "state",
            "powered": if powered { "yes" } else { "no" },
            "state": state_val,
        });
    }
    json!({"ok": false, "error": "failed to get status"})
}

pub fn list() -> Value {
    match query(true) {
        Ok(snapshot) => json!({
            "ok": true,
            "command": "bluetooth",
            "subcommand": "list",
            "hover": false,
            "devices": snapshot.get("devices").cloned().unwrap_or(json!([])),
        }),
        Err(e) => json!({"ok": false, "error": e}),
    }
}

/// The default adapter's object path.
fn adapter_path(conn: &Connection) -> Result<String, String> {
    default_adapter(&managed_objects(conn)?).map(|(path, _)| path)
}

fn proxy<'a>(conn: &Connection, path: String, interface: &'static str) -> Result<Proxy<'a>, String> {
    Proxy::new(conn, BLUEZ, path, interface).map_err(|e| e.to_string())
}

pub fn power(target: &str) -> Value {
    let on = target == "on";
    let result = with_timeout(BT_TIMEOUT, move || {
        let conn = system()?;
        let adapter = proxy(&conn, adapter_path(&conn)?, ADAPTER)?;
        adapter.set_property("Powered", on).map_err(|e| e.to_string())
    });
    match result {
        Ok(_) => json!({ "ok": true, "command": "bluetooth", "subcommand": "power", "successful": true, "output": format!("Powered {}", target) }),
        Err(e) => json!({ "ok": false, "error": e, "command": "bluetooth", "subcommand": "power", "successful": false }),
    }
}

pub fn scan() -> Value {
    // Discovery runs for BT_TIMEOUT, so allow for the calls around it.
    let result = with_timeout(BT_TIMEOUT * 2, || {
        let conn = system()?;
        let adapter = proxy(&conn, adapter_path(&conn)?, ADAPTER)?;
        adapter.call::<_, _, ()>("StartDiscovery", &()).map_err(|e| e.to_string())?;
        std::thread::sleep(BT_TIMEOUT);
        // BlueZ stops it anyway once this connection closes.
        let _ = adapter.call::<_, _, ()>("StopDiscovery", &());
        Ok(())
    });
    match result {
        Ok(_) => json!({ "ok": true, "command": "bluetooth", "subcommand": "scan", "successful": true, "output": "Discovery finished" }),
        Err(e) => json!({ "ok": false, "error": e, "command": "bluetooth", "subcommand": "scan", "successful": false }),
    }
}

/// `AA:BB:CC:DD:EE:FF`, upper-cased; `None` if it is not a MAC address.
fn parse_mac(mac: &str) -> Option<String> {
    let parts: Vec<&str> = mac.split(':').collect();
    let valid = parts.len() == 6 && parts.iter().all(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_hexdigit()));
    valid.then(|| mac.to_uppercase())
}

/// The device's object path under `adapter`, as BlueZ names it.
fn device_path(adapter: &str, mac: &str) -> String {
    format!("{adapter}/dev_{}", mac.replace(':', "_"))
}

/// Run `f` on the device `mac` of the default adapter.
fn with_device(
    mac: &str,
    f: impl FnOnce(&Proxy<'_>) -> zbus::Result<()> + Send + 'static,
) -> Result<(), String> {
    let mac = parse_mac(mac).ok_or_else(|| format!("Invalid MAC address: {}", mac))?;
    with_timeout(ACTION_TIMEOUT, move || {
        let conn = system()?;
        let device = proxy(&conn, device_path(&adapter_path(&conn)?, &mac), DEVICE)?;
        f(&device).map_err(|e| e.to_string())
    })
}

fn device_action(
    subcommand: &str,
    mac: &str,
    f: impl FnOnce(&Proxy<'_>) -> zbus::Result<()> + Send + 'static,
) -> Value {
    match with_device(mac, f) {
        Ok(_) => json!({ "ok": true, "command": "bluetooth", "subcommand": subcommand, "successful": true, "output": format!("{} succeeded", subcommand) }),
        Err(e) => json!({ "ok": false, "error": e, "command": "bluetooth", "subcommand": subcommand, "successful": false }),
    }
}

pub fn pair(mac: &str) -> Value {
    let result = with_device(mac, |device| {
        // Already paired is fine; trusting is what makes it reconnect.
        let _ = device.call::<_, _, ()>("Pair", &());
        device.set_property("Trusted", true).map_err(zbus::Error::from)
    });
    match result {
        Ok(_) => json!({ "ok": true, "command": "bluetooth", "subcommand": "pair", "successful": true, "output": "paired" }),
        Err(e) => json!({ "ok": false, "error": e, "command": "bluetooth", "subcommand": "pair", "successful": false }),
    }
}

pub fn connect(mac: &str) -> Value {
    device_action("connect", mac, |device| device.call("Connect", &()))
}

pub fn disconnect(mac: &str) -> Value {
    device_action("disconnect", mac, |device| device.call("Disconnect", &()))
}

pub fn forget(mac: &str) -> Value {
    let result = parse_mac(mac)
        .ok_or_else(|| format!("Invalid MAC address: {}", mac))
        .and_then(|mac| {
            with_timeout(BT_TIMEOUT, move || {
                let conn = system()?;
                let adapter_path = adapter_path(&conn)?;
                let device = zvariant::ObjectPath::try_from(device_path(&adapter_path, &mac))
                    .map_err(|e| e.to_string())?;
                let adapter = proxy(&conn, adapter_path, ADAPTER)?;
                adapter.call::<_, _, ()>("RemoveDevice", &(device,)).map_err(|e| e.to_string())
            })
        });
    match result {
        Ok(_) => json!({ "ok": true, "command": "bluetooth", "subcommand": "forget", "successful": true, "output": "forgotten" }),
        Err(e) => json!({ "ok": false, "error": e, "command": "bluetooth", "subcommand": "forget", "successful": false }),
    }
}

pub fn trust(mac: &str) -> Value {
    device_action("trust", mac, |device| device.set_property("Trusted", true).map_err(zbus::Error::from))
}

pub fn untrust(mac: &str) -> Value {
    device_action("untrust", mac, |device| device.set_property("Trusted", false).map_err(zbus::Error::from))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props(entries: Vec<(&str, OwnedValue)>) -> Properties {
        entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn path(p: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(p).unwrap()
    }

    fn device(adapter: &str, mac: &str, name: &str, paired: bool, connected: bool) -> HashMap<String, Properties> {
        let adapter = OwnedValue::from(zvariant::ObjectPath::try_from(adapter).unwrap());
        [(
            DEVICE.to_string(),
            props(vec![
                ("Adapter", adapter),
                ("Address", OwnedValue::from(zvariant::Str::from(mac.to_string()))),
                ("Name", OwnedValue::from(zvariant::Str::from(name.to_string()))),
                ("Paired", OwnedValue::from(paired)),
                ("Connected", OwnedValue::from(connected)),
                ("Trusted", OwnedValue::from(paired)),
            ]),
        )]
        .into_iter()
        .collect()
    }

    fn objects(powered: bool) -> Objects {
        let adapter = |powered: bool| -> HashMap<String, Properties> {
            [(ADAPTER.to_string(), props(vec![("Powered", OwnedValue::from(powered)), ("Discovering", OwnedValue::from(false))]))]
                .into_iter()
                .collect()
        };
        [
            (path("/org/bluez/hci1"), adapter(false)),
            (path("/org/bluez/hci0"), adapter(powered)),
            (path("/org/bluez/hci0/dev_AA"), device("/org/bluez/hci0", "aa:00:00:00:00:01", "Zeta", true, false)),
            (path("/org/bluez/hci0/dev_BB"), device("/org/bluez/hci0", "BB:00:00:00:00:02", "Alpha", true, false)),
            (path("/org/bluez/hci0/dev_CC"), device("/org/bluez/hci0", "CC:00:00:00:00:03", "Mouse", true, true)),
            (path("/org/bluez/hci0/dev_DD"), device("/org/bluez/hci0", "DD:00:00:00:00:04", "", false, false)),
            (path("/org/bluez/hci1/dev_EE"), device("/org/bluez/hci1", "EE:00:00:00:00:05", "Other", true, true)),
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn the_snapshot_lists_the_default_adapters_paired_devices_connected_first() {
        let snap = snapshot(&objects(true), false).unwrap();
        assert_eq!(snap["state"], "connected");
        let names: Vec<&str> = snap["devices"].as_array().unwrap().iter().map(|d| d["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["Mouse", "Alpha", "Zeta"], "hci1's device is not listed");
        assert_eq!(snap["devices"][2]["mac"], "AA:00:00:00:00:01", "upper-cased");
        assert_eq!(snap["devices"][0]["connected"], "yes");
    }

    #[test]
    fn unpaired_devices_are_listed_on_request_named_by_address() {
        let snap = snapshot(&objects(true), true).unwrap();
        let devices = snap["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 4);
        assert!(devices.iter().any(|d| d["name"] == "DD:00:00:00:00:04" && d["paired"] == "no"));
    }

    #[test]
    fn a_powered_off_adapter_reports_off_with_no_devices() {
        let snap = snapshot(&objects(false), false).unwrap();
        assert_eq!((snap["state"].as_str(), snap["devices"].as_array().unwrap().len()), (Some("off"), 0));
        assert!(snapshot(&Objects::new(), false).is_err(), "no adapter");
    }

    #[test]
    fn macs_are_validated_and_map_to_bluez_paths() {
        assert_eq!(parse_mac("aa:bb:cc:dd:ee:0f").as_deref(), Some("AA:BB:CC:DD:EE:0F"));
        assert_eq!(parse_mac("aa:bb:cc:dd:ee"), None);
        assert_eq!(parse_mac("zz:bb:cc:dd:ee:ff"), None);
        assert_eq!(device_path("/org/bluez/hci0", "AA:BB:CC:DD:EE:0F"), "/org/bluez/hci0/dev_AA_BB_CC_DD_EE_0F");
    }
}
