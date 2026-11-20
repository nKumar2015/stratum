//! NetworkManager beyond Wi-Fi scanning: VPN and WireGuard profiles, wired
//! devices, and a Wi-Fi hotspot. Over D-Bus, like `wifi::connect`, so the
//! hotspot's password never appears on a command line.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value as Variant};

use crate::managers::common::with_timeout;
use crate::managers::wifi::{
    bytes, nm_proxy, owned, saved_profiles, wait_active, Settings, DEVICE_TYPE_WIFI, NM, NM_ACTIVE, NM_CONNECTION,
    NM_DEVICE, NM_PATH, NM_WIRELESS,
};

const NM_WIRED: &str = "org.freedesktop.NetworkManager.Device.Wired";
const NM_IP4: &str = "org.freedesktop.NetworkManager.IP4Config";
const DEVICE_TYPE_ETHERNET: u32 = 1;
/// `NM_WIFI_DEVICE_CAP_AP`: the radio can be an access point.
const WIFI_CAP_AP: u32 = 0x40;
const ACTIVATED: u32 = 2;
/// How long a VPN or the hotspot may take to come up.
const ACTIVATE_TIMEOUT: Duration = Duration::from_secs(45);

fn system() -> Result<Connection, String> {
    Connection::system().map_err(|err| format!("system bus unavailable: {err}"))
}

fn text(settings: &Settings, section: &str, key: &str) -> String {
    settings
        .get(section)
        .and_then(|s| s.get(key))
        .and_then(|v| <&str>::try_from(v).ok())
        .unwrap_or_default()
        .to_string()
}

/// What kind of VPN a profile is, for people: its plugin's last word.
pub fn vpn_kind(kind: &str, service: &str) -> String {
    if kind == "wireguard" {
        return "WireGuard".into();
    }
    match service.rsplit('.').next().unwrap_or("") {
        "openvpn" => "OpenVPN".into(),
        "openconnect" => "OpenConnect".into(),
        "strongswan" | "libreswan" => "IPsec".into(),
        "" => "VPN".into(),
        other => {
            let mut chars = other.chars();
            chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default()
        }
    }
}

/// A device state as people say it (`NMDeviceState`).
pub fn device_state(state: u32) -> &'static str {
    match state {
        10 => "Unmanaged",
        20 => "Cable unplugged",
        30 => "Disconnected",
        40..=90 => "Connecting",
        100 => "Connected",
        110 => "Disconnecting",
        120 => "Failed",
        _ => "Unknown",
    }
}

/// An active connection: its profile's UUID, state, and mode when Wi-Fi.
struct Active {
    path: OwnedObjectPath,
    uuid: String,
    state: u32,
}

fn active_connections(conn: &Connection) -> Result<Vec<Active>, String> {
    let nm = nm_proxy(conn, NM_PATH, NM)?;
    let paths: Vec<OwnedObjectPath> = nm.get_property("ActiveConnections").map_err(|e| e.to_string())?;
    Ok(paths
        .into_iter()
        .filter_map(|path| {
            let active = nm_proxy(conn, path.as_str(), NM_ACTIVE).ok()?;
            Some(Active {
                uuid: active.get_property("Uuid").ok()?,
                state: active.get_property("State").unwrap_or(0),
                path,
            })
        })
        .collect())
}

/// The first IPv4 address (with its prefix), the gateway and the DNS
/// servers of an `IP4Config`.
fn ip4(conn: &Connection, path: &OwnedObjectPath) -> (String, String, Vec<String>) {
    if path.as_str() == "/" {
        return Default::default();
    }
    let Ok(config) = nm_proxy(conn, path.as_str(), NM_IP4) else { return Default::default() };
    let field = |entry: &HashMap<String, OwnedValue>, key: &str| entry.get(key).and_then(|v| <&str>::try_from(v).ok()).map(str::to_string);
    let addresses: Vec<HashMap<String, OwnedValue>> = config.get_property("AddressData").unwrap_or_default();
    let address = addresses
        .first()
        .and_then(|a| {
            let prefix = a.get("prefix").and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
            field(a, "address").map(|ip| format!("{ip}/{prefix}"))
        })
        .unwrap_or_default();
    let gateway: String = config.get_property("Gateway").unwrap_or_default();
    let servers: Vec<HashMap<String, OwnedValue>> = config.get_property("NameserverData").unwrap_or_default();
    let dns = servers.iter().filter_map(|s| field(s, "address")).collect();
    (address, gateway, dns)
}

/// VPN profiles, wired devices and the hotspot, as one snapshot.
pub fn status() -> Value {
    match with_timeout(Duration::from_secs(5), read_status) {
        Ok(value) => value,
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

fn read_status() -> Result<Value, String> {
    let conn = system()?;
    let profiles = saved_profiles(&conn)?;
    let active = active_connections(&conn)?;
    let state_of = |uuid: &str| active.iter().find(|a| a.uuid == uuid).map(|a| a.state);

    let mut vpns = Vec::new();
    let mut hotspot = json!({ "supported": false, "active": false, "ssid": "" });
    let mut hotspot_profile = None;
    for (_, settings) in &profiles {
        let kind = text(settings, "connection", "type");
        let uuid = text(settings, "connection", "uuid");
        let name = text(settings, "connection", "id");
        match kind.as_str() {
            "vpn" | "wireguard" => {
                let state = state_of(&uuid);
                vpns.push(json!({
                    "uuid": uuid,
                    "name": name,
                    "kind": vpn_kind(&kind, &text(settings, "vpn", "service-type")),
                    "active": state == Some(ACTIVATED),
                    "connecting": state.is_some_and(|s| s < ACTIVATED),
                }));
            }
            "802-11-wireless" if text(settings, "802-11-wireless", "mode") == "ap" => {
                let ssid = settings.get("802-11-wireless").and_then(|w| w.get("ssid")).map(bytes).unwrap_or_default();
                let ssid = String::from_utf8_lossy(&ssid).into_owned();
                let on = state_of(&uuid) == Some(ACTIVATED);
                // An active one first, else the first saved.
                if hotspot_profile.is_none() || on {
                    hotspot_profile = Some((uuid, ssid, on));
                }
            }
            _ => {}
        }
    }
    vpns.sort_by_key(|v| (!v["active"].as_bool().unwrap_or(false), v["name"].as_str().unwrap_or("").to_lowercase()));

    let nm = nm_proxy(&conn, NM_PATH, NM)?;
    let devices: Vec<OwnedObjectPath> = nm.call("GetDevices", &()).map_err(|e| e.to_string())?;
    let mut wired = Vec::new();
    let mut ap_capable = false;
    for path in devices {
        let device = nm_proxy(&conn, path.as_str(), NM_DEVICE)?;
        match device.get_property::<u32>("DeviceType").unwrap_or(0) {
            DEVICE_TYPE_ETHERNET => {
                let state: u32 = device.get_property("State").unwrap_or(0);
                // Unmanaged ports (docks, virtual ones) are not the user's.
                if state == 10 {
                    continue;
                }
                let interface: String = device.get_property("Interface").unwrap_or_default();
                let speed = nm_proxy(&conn, path.as_str(), NM_WIRED)
                    .ok()
                    .and_then(|w| w.get_property::<u32>("Speed").ok())
                    .unwrap_or(0);
                let ip4_path: OwnedObjectPath =
                    device.get_property("Ip4Config").unwrap_or_else(|_| OwnedObjectPath::try_from("/").expect("valid"));
                let (address, gateway, dns) = ip4(&conn, &ip4_path);
                let active_path: OwnedObjectPath =
                    device.get_property("ActiveConnection").unwrap_or_else(|_| OwnedObjectPath::try_from("/").expect("valid"));
                let connection = if active_path.as_str() == "/" {
                    String::new()
                } else {
                    nm_proxy(&conn, active_path.as_str(), NM_ACTIVE)
                        .ok()
                        .and_then(|a| a.get_property::<String>("Id").ok())
                        .unwrap_or_default()
                };
                wired.push(json!({
                    "device": interface,
                    "state": device_state(state),
                    "connected": state == 100,
                    "connection": connection,
                    "speed": speed,
                    "mac": device.get_property::<String>("HwAddress").unwrap_or_default(),
                    "ip": address,
                    "gateway": gateway,
                    "dns": dns,
                }));
            }
            DEVICE_TYPE_WIFI => {
                let caps = nm_proxy(&conn, path.as_str(), NM_WIRELESS)
                    .ok()
                    .and_then(|w| w.get_property::<u32>("WirelessCapabilities").ok())
                    .unwrap_or(0);
                ap_capable |= caps & WIFI_CAP_AP != 0;
            }
            _ => {}
        }
    }
    hotspot["supported"] = json!(ap_capable);
    if let Some((uuid, ssid, on)) = hotspot_profile {
        hotspot["uuid"] = json!(uuid);
        hotspot["ssid"] = json!(ssid);
        hotspot["active"] = json!(on);
    }
    Ok(json!({ "ok": true, "vpns": vpns, "wired": wired, "hotspot": hotspot }))
}

fn profile_path(conn: &Connection, uuid: &str) -> Result<OwnedObjectPath, String> {
    saved_profiles(conn)?
        .into_iter()
        .find(|(_, s)| text(s, "connection", "uuid") == uuid)
        .map(|(path, _)| path)
        .ok_or_else(|| "That connection no longer exists.".to_string())
}

/// Bring the profile with `uuid` up (and wait for it), or down.
pub fn set_active(uuid: &str, on: bool) -> Value {
    let uuid = uuid.to_string();
    let result = with_timeout(ACTIVATE_TIMEOUT, move || {
        let conn = system()?;
        let nm = nm_proxy(&conn, NM_PATH, NM)?;
        if on {
            let path = profile_path(&conn, &uuid)?;
            let root = OwnedObjectPath::try_from("/").expect("valid");
            let active: OwnedObjectPath = nm
                .call("ActivateConnection", &(&path, &root, &root))
                .map_err(|err| format!("Could not connect: {err}"))?;
            wait_active(&conn, active)
        } else {
            let active = active_connections(&conn)?
                .into_iter()
                .find(|a| a.uuid == uuid)
                .ok_or_else(|| "It is not connected.".to_string())?;
            nm.call::<_, _, ()>("DeactivateConnection", &(&active.path,))
                .map_err(|err| format!("Could not disconnect: {err}"))
        }
    });
    match result {
        Ok(()) => json!({ "ok": true }),
        Err(err) if err == "timeout" => json!({ "ok": false, "error": "Timed out." }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

/// A password for a new hotspot: 12 characters people can read out
/// (no 0/O, 1/l/I).
pub fn new_password(random: &[u8]) -> String {
    const ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    random.iter().take(12).map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect()
}

/// Turn the hotspot on, making its profile the first time (named after the
/// host, with a new password), or off. On takes the Wi-Fi radio from its
/// connection.
pub fn set_hotspot(on: bool) -> Value {
    let result = with_timeout(ACTIVATE_TIMEOUT, move || {
        let conn = system()?;
        let status = read_status()?;
        let hotspot = &status["hotspot"];
        if let Some(uuid) = hotspot["uuid"].as_str() {
            if !on {
                let nm = nm_proxy(&conn, NM_PATH, NM)?;
                return match active_connections(&conn)?.into_iter().find(|a| a.uuid == uuid) {
                    Some(active) => nm
                        .call::<_, _, ()>("DeactivateConnection", &(&active.path,))
                        .map_err(|err| format!("Could not stop the hotspot: {err}")),
                    None => Ok(()),
                };
            }
            let path = profile_path(&conn, uuid)?;
            let nm = nm_proxy(&conn, NM_PATH, NM)?;
            let root = OwnedObjectPath::try_from("/").expect("valid");
            let active: OwnedObjectPath = nm
                .call("ActivateConnection", &(&path, &root, &root))
                .map_err(|err| format!("Could not start the hotspot: {err}"))?;
            return wait_active(&conn, active);
        }
        if !on {
            return Ok(());
        }
        if !hotspot["supported"].as_bool().unwrap_or(false) {
            return Err("This Wi-Fi card cannot be a hotspot.".into());
        }
        create_hotspot(&conn)
    });
    match result {
        Ok(()) => json!({ "ok": true }),
        Err(err) if err == "timeout" => json!({ "ok": false, "error": "Timed out starting the hotspot." }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

fn create_hotspot(conn: &Connection) -> Result<(), String> {
    let nm = nm_proxy(conn, NM_PATH, NM)?;
    let devices: Vec<OwnedObjectPath> = nm.call("GetDevices", &()).map_err(|e| e.to_string())?;
    let device = devices
        .into_iter()
        .find(|d| nm_proxy(conn, d.as_str(), NM_DEVICE).ok().and_then(|p| p.get_property::<u32>("DeviceType").ok()) == Some(DEVICE_TYPE_WIFI))
        .ok_or_else(|| "No Wi-Fi device.".to_string())?;
    let host = std::fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default();
    let ssid = format!("{} hotspot", host.trim()).trim().to_string();
    let mut random = [0u8; 12];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut random))
        .map_err(|err| format!("No randomness for a password: {err}"))?;
    let password = new_password(&random);

    let section = |pairs: Vec<(&str, Variant<'_>)>| -> HashMap<String, OwnedValue> {
        pairs.into_iter().map(|(k, v)| (k.to_string(), owned(v))).collect()
    };
    let mut settings = Settings::new();
    settings.insert(
        "connection".into(),
        section(vec![
            ("id", Variant::from("Hotspot")),
            ("type", Variant::from("802-11-wireless")),
            ("autoconnect", Variant::from(false)),
        ]),
    );
    settings.insert(
        "802-11-wireless".into(),
        section(vec![("ssid", Variant::from(ssid.as_bytes().to_vec())), ("mode", Variant::from("ap"))]),
    );
    settings.insert(
        "802-11-wireless-security".into(),
        section(vec![
            ("key-mgmt", Variant::from("wpa-psk")),
            ("psk", Variant::from(password.as_str())),
            ("psk-flags", Variant::from(0u32)),
            ("proto", Variant::from(vec!["rsn"])),
            ("pairwise", Variant::from(vec!["ccmp"])),
            ("group", Variant::from(vec!["ccmp"])),
        ]),
    );
    settings.insert("ipv4".into(), section(vec![("method", Variant::from("shared"))]));
    settings.insert("ipv6".into(), section(vec![("method", Variant::from("ignore"))]));
    let root = OwnedObjectPath::try_from("/").expect("valid");
    let (_, active): (OwnedObjectPath, OwnedObjectPath) = nm
        .call("AddAndActivateConnection", &(settings, &device, &root))
        .map_err(|err| format!("Could not start the hotspot: {err}"))?;
    wait_active(conn, active)
}

/// The hotspot's password, read from NetworkManager when asked for (it may
/// ask polkit). Never kept.
pub fn hotspot_password(uuid: &str) -> Value {
    let uuid = uuid.to_string();
    let result = with_timeout(Duration::from_secs(30), move || {
        let conn = system()?;
        let path = profile_path(&conn, &uuid)?;
        let profile = nm_proxy(&conn, path.as_str(), NM_CONNECTION)?;
        let secrets: Settings = profile
            .call("GetSecrets", &("802-11-wireless-security",))
            .map_err(|err| format!("Could not read the password: {err}"))?;
        Ok(text(&secrets, "802-11-wireless-security", "psk"))
    });
    match result {
        Ok(password) => json!({ "ok": true, "password": password }),
        Err(err) => json!({ "ok": false, "error": err }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vpn_kinds_read_naturally() {
        assert_eq!(vpn_kind("wireguard", ""), "WireGuard");
        assert_eq!(vpn_kind("vpn", "org.freedesktop.NetworkManager.openvpn"), "OpenVPN");
        assert_eq!(vpn_kind("vpn", "org.freedesktop.NetworkManager.fortisslvpn"), "Fortisslvpn");
        assert_eq!(vpn_kind("vpn", ""), "VPN");
    }

    #[test]
    fn hotspot_passwords_avoid_lookalikes() {
        let password = new_password(&[0, 8, 14, 31, 32, 255, 100, 7, 3, 9, 11, 21]);
        assert_eq!(password.len(), 12);
        assert!(!password.contains(['0', 'o', '1', 'l', 'I']));
    }

    /// Reads the real NetworkManager; changes nothing. Run by hand with
    /// `cargo test -p stratum_sys print_status -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn print_status() {
        println!("{:#}", status());
    }

    #[test]
    fn device_states() {
        assert_eq!(device_state(100), "Connected");
        assert_eq!(device_state(20), "Cable unplugged");
        assert_eq!(device_state(70), "Connecting");
    }
}
