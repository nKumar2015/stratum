//! Wi-Fi through NetworkManager.
//!
//! These used to shell out to `stratum-cli`, which itself shells out to
//! `nmcli`. Two process hops per query made the UI visibly lag behind a click;
//! talking to `nmcli` directly removes the outer hop and returns in about
//! 20 ms, which is fast enough to refresh on the interaction itself rather than
//! waiting for a poll tick.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value as Variant};

use crate::managers::common::{run_command_capture, with_timeout};

/// Run `nmcli` and split its terse (`-t`) output into fields.
fn nmcli(args: &[&str]) -> Result<String, String> {
    run_command_capture("nmcli", args)
}

/// Split one line of `nmcli -t` output, honouring backslash escapes.
fn split_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == ':' {
            fields.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    fields.push(current);
    fields
}

/// Radio state: `enabled`, `disabled`, or the raw NetworkManager string.
pub fn state() -> Value {
    let output = match nmcli(&["-t", "-f", "WIFI", "general", "status"]) {
        Ok(output) => output,
        Err(err) => {
            return json!({
                "ok": false,
                "error": err,
                "state": "unknown",
            });
        }
    };
    let state = output.lines().next().unwrap_or("").trim().to_string();
    json!({
        "ok": true,
        "state": state,
        "enabled": state == "enabled",
    })
}

/// Wi-Fi access points, strongest first.
///
/// `rescan` asks NetworkManager to refresh the cache first. That costs a few
/// hundred milliseconds, so callers pass `false` for the frequent refreshes and
/// `true` only when the user explicitly asked to scan.
pub fn list(rescan: bool) -> Value {
    let mut args = vec![
        "-t",
        "-f",
        "IN-USE,SSID,SIGNAL,SECURITY",
        "dev",
        "wifi",
        "list",
    ];
    if rescan {
        args.extend_from_slice(&["--rescan", "yes"]);
    }

    let output = match nmcli(&args) {
        Ok(output) => output,
        Err(err) => {
            return json!({ "ok": false, "error": err, "networks": [] });
        }
    };

    let mut networks = Vec::new();
    for line in output.lines() {
        let fields = split_fields(line);
        if fields.len() < 4 {
            continue;
        }
        let ssid = fields[1].trim().to_string();
        if ssid.is_empty() {
            continue;
        }
        let signal = fields[2].trim().parse::<i64>().unwrap_or(0);
        networks.push(json!({
            "in_use": fields[0].trim() == "*",
            "ssid": ssid,
            "signal": signal,
            "security": fields[3].trim(),
        }));
    }
    networks.sort_by_key(|network| {
        std::cmp::Reverse(network["signal"].as_i64().unwrap_or(0))
    });

    json!({
        "ok": true,
        "networks": networks,
    })
}

/// Active connections, one entry per connected device.
fn device_status() -> Value {
    let output = match nmcli(&["-t", "-f", "DEVICE,TYPE,STATE,CONNECTION", "dev", "status"]) {
        Ok(output) => output,
        Err(err) => {
            return json!({ "ok": false, "error": err, "devices": [] });
        }
    };
    let mut devices = Vec::new();
    for line in output.lines() {
        let fields = split_fields(line);
        if fields.len() < 4 {
            continue;
        }
        devices.push(json!({
            "device": fields[0].trim(),
            "type": fields[1].trim(),
            "state": fields[2].trim(),
            "connection": fields[3].trim(),
        }));
    }
    json!({ "ok": true, "devices": devices })
}

fn known_connections() -> Value {
    let output = match nmcli(&["-t", "-f", "NAME,TYPE", "connection", "show"]) {
        Ok(output) => output,
        Err(err) => {
            return json!({ "ok": false, "error": err, "connections": [] });
        }
    };
    let mut connections = Vec::new();
    for line in output.lines() {
        let fields = split_fields(line);
        if fields.len() < 2 {
            continue;
        }
        connections.push(json!({
            "name": fields[0].trim(),
            "type": fields[1].trim(),
        }));
    }
    json!({ "ok": true, "connections": connections })
}

fn active_info(device: &str) -> Value {
    let output = match nmcli(&["-t", "-f", "IP4.ADDRESS,IP4.GATEWAY", "dev", "show", device]) {
        Ok(output) => output,
        Err(err) => {
            return json!({ "ok": false, "error": err });
        }
    };
    let mut address = String::new();
    let mut gateway = String::new();
    for line in output.lines() {
        let fields = split_fields(line);
        if fields.len() < 2 {
            continue;
        }
        match fields[0].trim() {
            "IP4.ADDRESS[1]" => address = fields[1].trim().to_string(),
            "IP4.GATEWAY" => gateway = fields[1].trim().to_string(),
            _ => {}
        }
    }
    json!({ "ok": true, "ip4_address": address, "ip4_gateway": gateway })
}

/// The connected Wi-Fi device from `device_status()`: `(device, connection,
/// state)`. Only a state that starts with "connected" counts; the QML menu
/// matched "connected" anywhere, which "disconnected" also contains.
fn pick_active(devices: &Value) -> Option<(String, String, String)> {
    devices["devices"].as_array()?.iter().find_map(|row| {
        let field = |k: &str| row[k].as_str().unwrap_or("").trim().to_string();
        let state = field("state");
        (field("type").eq_ignore_ascii_case("wifi") && state.to_ascii_lowercase().starts_with("connected"))
            .then(|| (field("device"), field("connection"), state))
    })
}

/// Names of saved Wi-Fi connections from `known_connections()`.
fn known_wifi(connections: &Value) -> Vec<String> {
    connections["connections"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|r| matches!(r["type"].as_str(), Some("802-11-wireless" | "wifi")))
                .filter_map(|r| r["name"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// What the Network page shows beyond the access-point list: the active
/// connection (device, SSID, state, IPv4 address and gateway) and the saved
/// Wi-Fi networks, which connect without asking for a password.
pub fn details() -> Value {
    let active = pick_active(&device_status()).map(|(device, ssid, state)| {
        let info = active_info(&device);
        json!({
            "device": device,
            "ssid": ssid,
            "state": state,
            "ip": info["ip4_address"].as_str().unwrap_or(""),
            "gateway": info["ip4_gateway"].as_str().unwrap_or(""),
        })
    });
    json!({
        "ok": true,
        "active": active,
        "known": known_wifi(&known_connections()),
    })
}

/// Connect to `ssid`, saving `password` with the network when one is given.
///
/// This talks to NetworkManager over D-Bus rather than running
/// `nmcli dev wifi connect SSID password …`: a password on a command line is
/// readable by every local user in `/proc/<pid>/cmdline` while it runs. It
/// also waits for the activation to finish, which the five-second `nmcli`
/// timeout usually cut short.
pub fn connect(ssid: &str, password: Option<&str>) -> Value {
    let owned_ssid = ssid.to_string();
    let password = password.map(str::trim).filter(|p| !p.is_empty()).map(str::to_string);
    match with_timeout(CONNECT_TIMEOUT, move || nm_connect(&owned_ssid, password.as_deref())) {
        Ok(message) => json!({ "ok": true, "ssid": ssid, "message": message }),
        Err(err) if err == "timeout" => json!({ "ok": false, "error": format!("Timed out connecting to {ssid}.") }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

const NM: &str = "org.freedesktop.NetworkManager";
const NM_PATH: &str = "/org/freedesktop/NetworkManager";
const SETTINGS_PATH: &str = "/org/freedesktop/NetworkManager/Settings";
const NM_DEVICE: &str = "org.freedesktop.NetworkManager.Device";
const NM_WIRELESS: &str = "org.freedesktop.NetworkManager.Device.Wireless";
const NM_ACCESS_POINT: &str = "org.freedesktop.NetworkManager.AccessPoint";
const NM_ACTIVE: &str = "org.freedesktop.NetworkManager.Connection.Active";
const NM_SETTINGS: &str = "org.freedesktop.NetworkManager.Settings";
const NM_CONNECTION: &str = "org.freedesktop.NetworkManager.Settings.Connection";

const DEVICE_TYPE_WIFI: u32 = 2;
const ACTIVATED: u32 = 2;
const DEACTIVATED: u32 = 4;
/// `NM_802_11_AP_FLAGS_PRIVACY`, and the key-management bits of the WPA and
/// RSN flags.
const AP_PRIVACY: u32 = 0x1;
const KEY_MGMT_PSK: u32 = 0x100;
const KEY_MGMT_802_1X: u32 = 0x200;
const KEY_MGMT_SAE: u32 = 0x400;
const WIRELESS_SECURITY: &str = "802-11-wireless-security";

/// How long an activation may take, DHCP included.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(60);

/// A connection's settings, as `GetSettings` returns them.
type Settings = HashMap<String, HashMap<String, OwnedValue>>;

/// An access point, with what choosing one and its security needs.
#[derive(Clone, Debug)]
struct AccessPoint {
    device: OwnedObjectPath,
    path: OwnedObjectPath,
    ssid: Vec<u8>,
    strength: u8,
    flags: u32,
    wpa_flags: u32,
    rsn_flags: u32,
}

fn owned(value: Variant<'_>) -> OwnedValue {
    OwnedValue::try_from(value).expect("plain values have no file descriptors")
}

fn bytes(value: &OwnedValue) -> Vec<u8> {
    match &**value {
        Variant::Array(array) => array.iter().filter_map(|b| u8::try_from(b).ok()).collect(),
        _ => Vec::new(),
    }
}

/// The strongest access point broadcasting `ssid`.
fn strongest<'a>(aps: &'a [AccessPoint], ssid: &str) -> Option<&'a AccessPoint> {
    aps.iter().filter(|ap| ap.ssid == ssid.as_bytes()).max_by_key(|ap| ap.strength)
}

/// The saved Wi-Fi profile for `ssid`, matched on the SSID itself rather than
/// the profile's name, which the user may have changed.
fn find_profile<'a>(profiles: &'a [(OwnedObjectPath, Settings)], ssid: &str) -> Option<&'a (OwnedObjectPath, Settings)> {
    profiles.iter().find(|(_, settings)| {
        settings.get("802-11-wireless").and_then(|w| w.get("ssid")).is_some_and(|s| bytes(s) == ssid.as_bytes())
    })
}

/// The security section for `password` on `ap`: WPA/WPA2 personal, WPA3
/// personal (SAE) or WEP. The secret is stored by NetworkManager (flags 0),
/// as `nmcli` stored it.
fn security_for(ap: &AccessPoint, password: &str) -> Result<HashMap<String, OwnedValue>, String> {
    let mgmt = ap.wpa_flags | ap.rsn_flags;
    let mut section = HashMap::new();
    let mut set = |key: &str, value: Variant<'_>| {
        section.insert(key.to_string(), owned(value));
    };
    if mgmt & KEY_MGMT_PSK != 0 {
        set("key-mgmt", Variant::from("wpa-psk"));
        set("psk", Variant::from(password));
        set("psk-flags", Variant::from(0u32));
    } else if mgmt & KEY_MGMT_SAE != 0 {
        set("key-mgmt", Variant::from("sae"));
        set("psk", Variant::from(password));
        set("psk-flags", Variant::from(0u32));
    } else if mgmt & KEY_MGMT_802_1X != 0 {
        return Err("This network needs an enterprise (802.1X) login, which this menu cannot set up.".into());
    } else if ap.flags & AP_PRIVACY != 0 {
        // A 5/13-character or 10/26-digit hex key is the key itself (type
        // 1); anything else is a passphrase (type 2).
        let hex = password.chars().all(|c| c.is_ascii_hexdigit());
        let key = matches!(password.len(), 5 | 13) || (hex && matches!(password.len(), 10 | 26));
        set("key-mgmt", Variant::from("none"));
        set("wep-key0", Variant::from(password));
        set("wep-key-flags", Variant::from(0u32));
        set("wep-key-type", Variant::from(if key { 1u32 } else { 2u32 }));
    } else {
        return Err("This network is open; it needs no password.".into());
    }
    Ok(section)
}

/// Put `password` into a saved security section whose access point is out of
/// range, going by the section's own key management.
fn set_secret(security: &mut HashMap<String, OwnedValue>, password: &str) {
    let wep = security.get("key-mgmt").and_then(|v| <&str>::try_from(v).ok()) == Some("none");
    let (secret, flags) = if wep { ("wep-key0", "wep-key-flags") } else { ("psk", "psk-flags") };
    security.insert(secret.to_string(), owned(Variant::from(password)));
    security.insert(flags.to_string(), owned(Variant::from(0u32)));
    security.entry("key-mgmt".to_string()).or_insert_with(|| owned(Variant::from("wpa-psk")));
}

/// Why an activation failed, from `NMActiveConnectionStateReason`.
fn failure(reason: u32) -> String {
    let why = match reason {
        3 => "The device was disconnected.".to_string(),
        5 => "IP configuration could not be reserved (no available address, timeout, etc.).".to_string(),
        6 => "The connection attempt timed out.".to_string(),
        9 | 10 => "Secrets were required, but not provided.".to_string(),
        11 => "The connection was removed.".to_string(),
        14 => "The device was removed.".to_string(),
        other => format!("reason {other}."),
    };
    format!("Connection activation failed: {why}")
}

fn nm_proxy<'a>(conn: &Connection, path: impl Into<String>, interface: &'static str) -> Result<Proxy<'a>, String> {
    // Read on demand: caching would fetch and watch every property of every
    // access point for a single lookup.
    zbus::blocking::proxy::Builder::<Proxy<'a>>::new(conn)
        .destination(NM)
        .and_then(|b| b.path(path.into()))
        .and_then(|b| b.interface(interface))
        .map(|b| b.cache_properties(zbus::proxy::CacheProperties::No))
        .and_then(|b| b.build())
        .map_err(|err| format!("NetworkManager unavailable: {err}"))
}

fn access_points(conn: &Connection) -> Result<Vec<AccessPoint>, String> {
    let nm = nm_proxy(conn, NM_PATH, NM)?;
    let devices: Vec<OwnedObjectPath> = nm.call("GetDevices", &()).map_err(|err| err.to_string())?;
    let mut aps = Vec::new();
    for device in devices {
        if nm_proxy(conn, device.as_str(), NM_DEVICE)?.get_property::<u32>("DeviceType").ok() != Some(DEVICE_TYPE_WIFI) {
            continue;
        }
        let wireless = nm_proxy(conn, device.as_str(), NM_WIRELESS)?;
        let paths: Vec<OwnedObjectPath> = wireless.call("GetAllAccessPoints", &()).map_err(|err| err.to_string())?;
        for path in paths {
            let ap = nm_proxy(conn, path.as_str(), NM_ACCESS_POINT)?;
            let flag = |name: &str| ap.get_property::<u32>(name).unwrap_or(0);
            let Ok(ssid) = ap.get_property::<Vec<u8>>("Ssid") else { continue };
            aps.push(AccessPoint {
                device: device.clone(),
                strength: ap.get_property::<u8>("Strength").unwrap_or(0),
                flags: flag("Flags"),
                wpa_flags: flag("WpaFlags"),
                rsn_flags: flag("RsnFlags"),
                ssid,
                path,
            });
        }
    }
    Ok(aps)
}

fn saved_profiles(conn: &Connection) -> Result<Vec<(OwnedObjectPath, Settings)>, String> {
    let settings = nm_proxy(conn, SETTINGS_PATH, NM_SETTINGS)?;
    let paths: Vec<OwnedObjectPath> = settings.call("ListConnections", &()).map_err(|err| err.to_string())?;
    Ok(paths
        .into_iter()
        .filter_map(|path| {
            let profile = nm_proxy(conn, path.as_str(), NM_CONNECTION).ok()?;
            let settings: Settings = profile.call("GetSettings", &()).ok()?;
            Some((path, settings))
        })
        .collect())
}

/// Wait for an activation to finish.
fn wait_active(conn: &Connection, active: OwnedObjectPath) -> Result<(), String> {
    let proxy = nm_proxy(conn, active.as_str(), NM_ACTIVE)?;
    // Subscribe before reading the state, so no change falls between them.
    let changes = proxy.receive_signal("StateChanged").map_err(|err| err.to_string())?;
    match proxy.get_property::<u32>("State") {
        Ok(ACTIVATED) => return Ok(()),
        // Gone already: it failed before we looked, and the reason went with it.
        Ok(DEACTIVATED) | Err(_) => return Err(failure(0)),
        Ok(_) => {}
    }
    for message in changes {
        let (state, reason): (u32, u32) = message.body().deserialize().map_err(|err| err.to_string())?;
        match state {
            ACTIVATED => return Ok(()),
            DEACTIVATED => return Err(failure(reason)),
            _ => {}
        }
    }
    Err("NetworkManager stopped reporting the connection.".into())
}

fn nm_connect(ssid: &str, password: Option<&str>) -> Result<String, String> {
    let conn = Connection::system().map_err(|err| format!("system bus unavailable: {err}"))?;
    let aps = access_points(&conn)?;
    let ap = strongest(&aps, ssid);
    let profiles = saved_profiles(&conn)?;
    let profile = find_profile(&profiles, ssid);
    let nm = nm_proxy(&conn, NM_PATH, NM)?;
    let root = || OwnedObjectPath::try_from("/").expect("valid path");
    let (device, specific) = match (ap, profile) {
        (Some(ap), _) => (ap.device.clone(), ap.path.clone()),
        // Out of range but saved: NetworkManager picks the device.
        (None, Some(_)) => (root(), root()),
        (None, None) => return Err(format!("No network with SSID '{ssid}' found.")),
    };

    let (active, added) = match (profile, ap) {
        (Some((path, settings)), _) => {
            if let Some(password) = password {
                // A saved network whose password failed: replace the secret
                // in place rather than adding a second profile beside it.
                let mut settings = settings.clone();
                let security = settings.entry(WIRELESS_SECURITY.to_string()).or_default();
                match ap {
                    Some(ap) => security.extend(security_for(ap, password)?),
                    None => set_secret(security, password),
                }
                nm_proxy(&conn, path.as_str(), NM_CONNECTION)?
                    .call::<_, _, ()>("Update", &(settings,))
                    .map_err(|err| format!("Could not save the password: {err}"))?;
            }
            let active: OwnedObjectPath = nm
                .call("ActivateConnection", &(path, &device, &specific))
                .map_err(|err| format!("Connection activation failed: {err}"))?;
            (active, None)
        }
        (None, Some(ap)) => {
            // NetworkManager completes the rest (SSID, mode, name) from the
            // access point.
            let mut partial = Settings::new();
            if let Some(password) = password {
                partial.insert(WIRELESS_SECURITY.to_string(), security_for(ap, password)?);
            }
            let (profile, active): (OwnedObjectPath, OwnedObjectPath) = nm
                .call("AddAndActivateConnection", &(partial, &device, &specific))
                .map_err(|err| format!("Connection activation failed: {err}"))?;
            (active, Some(profile))
        }
        (None, None) => unreachable!("returned above"),
    };

    match wait_active(&conn, active) {
        Ok(()) => Ok(format!("Connected to {ssid}.")),
        Err(err) => {
            // As nmcli did: a profile made for this attempt goes with it, so a
            // mistyped password does not leave a saved network behind.
            if let Some(profile) = added {
                if let Ok(proxy) = nm_proxy(&conn, profile.as_str(), NM_CONNECTION) {
                    let _ = proxy.call::<_, _, ()>("Delete", &());
                }
            }
            Err(err)
        }
    }
}

pub fn disconnect(device: &str) -> Value {
    match nmcli(&["dev", "disconnect", device]) {
        Ok(message) => json!({ "ok": true, "device": device, "message": message }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

pub fn forget(ssid: &str) -> Value {
    match nmcli(&["connection", "delete", "id", ssid]) {
        Ok(message) => json!({ "ok": true, "ssid": ssid, "message": message }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

pub fn toggle(target: &str) -> Value {
    match nmcli(&["radio", "wifi", target]) {
        Ok(message) => json!({ "ok": true, "target": target, "message": message }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_escaped_fields() {
        assert_eq!(split_fields("a:b:c"), vec!["a", "b", "c"]);
        // `nmcli` escapes a literal colon as `\:`.
        assert_eq!(split_fields("a:b\\:c:d"), vec!["a", "b:c", "d"]);
        assert_eq!(split_fields("only"), vec!["only"]);
    }

    #[test]
    fn the_active_device_is_a_connected_wifi_one() {
        let devices = json!({ "devices": [
            { "device": "wlan1", "type": "wifi", "state": "disconnected", "connection": "--" },
            { "device": "eth0", "type": "ethernet", "state": "connected", "connection": "Wired" },
            { "device": "wlan0", "type": "wifi", "state": "connected", "connection": "Home" },
        ]});
        assert_eq!(pick_active(&devices), Some(("wlan0".into(), "Home".into(), "connected".into())));
        let none = json!({ "devices": [{ "device": "wlan1", "type": "wifi", "state": "disconnected", "connection": "--" }]});
        assert_eq!(pick_active(&none), None, "\"disconnected\" contains \"connected\"");
    }

    fn ap(ssid: &str, strength: u8, flags: u32, wpa_flags: u32, rsn_flags: u32) -> AccessPoint {
        let path = |p: &str| OwnedObjectPath::try_from(p).unwrap();
        AccessPoint {
            device: path("/org/freedesktop/NetworkManager/Devices/3"),
            path: path(&format!("/org/freedesktop/NetworkManager/AccessPoint/{strength}")),
            ssid: ssid.as_bytes().to_vec(),
            strength,
            flags,
            wpa_flags,
            rsn_flags,
        }
    }

    fn text(section: &HashMap<String, OwnedValue>, key: &str) -> Option<String> {
        section.get(key).and_then(|v| <&str>::try_from(v).ok()).map(str::to_string)
    }

    #[test]
    fn the_strongest_access_point_for_the_ssid_is_chosen() {
        let aps = [ap("Home", 40, 1, 0, KEY_MGMT_PSK), ap("Home", 70, 1, 0, KEY_MGMT_PSK), ap("Cafe", 90, 0, 0, 0)];
        assert_eq!(strongest(&aps, "Home").map(|a| a.strength), Some(70));
        assert!(strongest(&aps, "Office").is_none());
    }

    #[test]
    fn saved_profiles_are_found_by_ssid_not_name() {
        let profile = |name: &str, ssid: &str| {
            let mut settings = Settings::new();
            settings.insert("connection".into(), HashMap::from([("id".into(), owned(Variant::from(name)))]));
            settings.insert(
                "802-11-wireless".into(),
                HashMap::from([("ssid".into(), owned(Variant::from(ssid.as_bytes().to_vec())))]),
            );
            (OwnedObjectPath::try_from(format!("/s/{name}")).unwrap(), settings)
        };
        let profiles = [profile("Home", "HomeNet"), profile("Work", "Work")];
        assert_eq!(find_profile(&profiles, "HomeNet").map(|p| p.0.as_str()), Some("/s/Home"));
        assert!(find_profile(&profiles, "Home").is_none());
    }

    #[test]
    fn the_security_section_follows_the_access_point() {
        let wpa2 = security_for(&ap("A", 50, 1, 0, KEY_MGMT_PSK), "hunter22").unwrap();
        assert_eq!(text(&wpa2, "key-mgmt").as_deref(), Some("wpa-psk"));
        assert_eq!(text(&wpa2, "psk").as_deref(), Some("hunter22"));
        // A WPA2/WPA3 transition network takes WPA2, which every card speaks.
        let mixed = security_for(&ap("A", 50, 1, 0, KEY_MGMT_PSK | KEY_MGMT_SAE), "x").unwrap();
        assert_eq!(text(&mixed, "key-mgmt").as_deref(), Some("wpa-psk"));
        let wpa3 = security_for(&ap("A", 50, 1, 0, KEY_MGMT_SAE), "x").unwrap();
        assert_eq!(text(&wpa3, "key-mgmt").as_deref(), Some("sae"));
        let wep = security_for(&ap("A", 50, AP_PRIVACY, 0, 0), "abcde").unwrap();
        assert_eq!(text(&wep, "key-mgmt").as_deref(), Some("none"));
        assert_eq!(wep.get("wep-key-type").and_then(|v| u32::try_from(v).ok()), Some(1));
        let phrase = security_for(&ap("A", 50, AP_PRIVACY, 0, 0), "a long passphrase").unwrap();
        assert_eq!(phrase.get("wep-key-type").and_then(|v| u32::try_from(v).ok()), Some(2));
        assert!(security_for(&ap("A", 50, 1, 0, KEY_MGMT_802_1X), "x").is_err(), "enterprise");
        assert!(security_for(&ap("A", 50, 0, 0, 0), "x").is_err(), "open");
    }

    #[test]
    fn an_out_of_range_profile_keeps_its_key_management() {
        let mut wep = HashMap::from([("key-mgmt".to_string(), owned(Variant::from("none")))]);
        set_secret(&mut wep, "abcde");
        assert_eq!(text(&wep, "wep-key0").as_deref(), Some("abcde"));
        assert!(!wep.contains_key("psk"));
        let mut empty = HashMap::new();
        set_secret(&mut empty, "hunter22");
        assert_eq!(text(&empty, "key-mgmt").as_deref(), Some("wpa-psk"));
        assert_eq!(text(&empty, "psk").as_deref(), Some("hunter22"));
    }

    #[test]
    fn a_wrong_password_reads_as_missing_secrets() {
        assert!(failure(9).ends_with("Secrets were required, but not provided."));
        assert!(failure(42).contains("reason 42"));
    }

    #[test]
    fn known_networks_are_saved_wifi_connections() {
        let connections = json!({ "connections": [
            { "name": "Home", "type": "802-11-wireless" },
            { "name": "Wired", "type": "802-3-ethernet" },
        ]});
        assert_eq!(known_wifi(&connections), vec!["Home".to_string()]);
    }
}
