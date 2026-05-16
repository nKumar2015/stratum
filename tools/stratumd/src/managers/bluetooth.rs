use serde_json::{json, Value};
use std::time::Duration;
use bluer::{Address, Session};

const BT_TIMEOUT: Duration = Duration::from_secs(4);

async fn status_from_bluer_async(include_unpaired: bool) -> Result<Value, String> {
    let session = Session::new()
        .await
        .map_err(|err| format!("bluer session failed: {}", err))?;
    let adapter = session
        .default_adapter()
        .await
        .map_err(|err| format!("bluer default adapter failed: {}", err))?;

    let powered = adapter
        .is_powered()
        .await
        .map_err(|err| format!("bluer adapter power query failed: {}", err))?;
    if !powered {
        return Ok(json!({
            "ok": true,
            "state": "off",
            "scanning": "no",
            "devices": [],
        }));
    }

    let scanning = adapter.is_discovering().await.unwrap_or(false);
    let addresses = adapter
        .device_addresses()
        .await
        .map_err(|err| format!("bluer device list query failed: {}", err))?;

    let mut devices: Vec<Value> = Vec::new();
    let mut has_connected = false;

    for address in addresses {
        let device = match adapter.device(address) {
            Ok(device) => device,
            Err(_) => continue,
        };
        let paired = device.is_paired().await.unwrap_or(false);
        if !paired && !include_unpaired {
            continue;
        }

        let connected = device.is_connected().await.unwrap_or(false);
        if connected {
            has_connected = true;
        }

        let mac = address.to_string().to_uppercase();
        let name = device
            .name()
            .await
            .ok()
            .flatten()
            .unwrap_or_else(|| mac.clone());

        devices.push(json!({
            "mac": mac,
            "name": if name.is_empty() { mac.clone() } else { name },
            "connected": if connected { "yes" } else { "no" },
            "paired": if paired { "yes" } else { "no" },
            "trusted": if device.is_trusted().await.unwrap_or(false) { "yes" } else { "no" },
        }));
    }

    devices.sort_by(|a, b| {
        let a_connected = a
            .get("connected")
            .and_then(Value::as_str)
            .map(|v| v == "yes")
            .unwrap_or(false);
        let b_connected = b
            .get("connected")
            .and_then(Value::as_str)
            .map(|v| v == "yes")
            .unwrap_or(false);

        if a_connected != b_connected {
            return b_connected.cmp(&a_connected);
        }

        let a_name = a.get("name").and_then(Value::as_str).unwrap_or_default();
        let b_name = b.get("name").and_then(Value::as_str).unwrap_or_default();
        a_name.cmp(b_name)
    });

    Ok(json!({
        "ok": true,
        "state": if has_connected { "connected" } else { "on" },
        "scanning": if scanning { "yes" } else { "no" },
        "devices": devices,
    }))
}

pub fn status() -> Value {
    std::thread::spawn(|| {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(e) => return json!({ "ok": false, "error": e.to_string(), "state": "off", "scanning": "no", "devices": [] }),
        };

        let result = runtime.block_on(async {
            tokio::time::timeout(BT_TIMEOUT, status_from_bluer_async(false)).await
        });

        match result {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(error)) => json!({ "ok": false, "error": error, "state": "off", "scanning": "no", "devices": [] }),
            Err(elapsed) => json!({ "ok": false, "error": elapsed.to_string(), "state": "off", "scanning": "no", "devices": [] }),
        }
    })
    .join()
    .unwrap_or_else(|_| json!({ "ok": false, "error": "thread panicked", "state": "off", "scanning": "no", "devices": [] }))
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
    std::thread::spawn(|| {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(runtime) => runtime,
            Err(e) => return json!({ "ok": false, "error": e.to_string() }),
        };

        let result = runtime.block_on(async {
            tokio::time::timeout(BT_TIMEOUT, status_from_bluer_async(true)).await
        });

        match result {
            Ok(Ok(mut snapshot)) => {
                if let Some(obj) = snapshot.as_object_mut() {
                    let devices = obj.get("devices").cloned().unwrap_or(json!([]));
                    return json!({
                        "ok": true,
                        "command": "bluetooth",
                        "subcommand": "list",
                        "hover": false,
                        "devices": devices,
                    });
                }
                json!({"ok": false, "error": "malformed snapshot"})
            }
            Ok(Err(e)) => json!({"ok": false, "error": e}),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        }
    })
    .join()
    .unwrap_or_else(|_| json!({"ok": false, "error": "thread panicked"}))
}

pub fn power(target: &str) -> Value {
    let target_bool = target == "on";
    let target_str = target.to_string();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => return json!({ "ok": false, "error": e.to_string() }),
        };

        let result = runtime.block_on(async {
            let session = Session::new().await.map_err(|e| e.to_string())?;
            let adapter = session.default_adapter().await.map_err(|e| e.to_string())?;
            adapter.set_powered(target_bool).await.map_err(|e| e.to_string())?;
            Ok::<(), String>(())
        });

        match result {
            Ok(_) => json!({ "ok": true, "command": "bluetooth", "subcommand": "power", "successful": true, "output": format!("Powered {}", target_str) }),
            Err(e) => json!({ "ok": false, "error": e, "command": "bluetooth", "subcommand": "power", "successful": false }),
        }
    })
    .join()
    .unwrap_or_else(|_| json!({ "ok": false, "error": "thread panicked", "command": "bluetooth", "subcommand": "power", "successful": false }))
}

pub fn scan() -> Value {
    std::thread::spawn(|| {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => return json!({ "ok": false, "error": e.to_string(), "command": "bluetooth", "subcommand": "scan", "successful": false }),
        };

        let result = runtime.block_on(async {
            let session = Session::new().await.map_err(|e| e.to_string())?;
            let adapter = session.default_adapter().await.map_err(|e| e.to_string())?;

            let _discover = adapter.discover_devices().await.map_err(|e| e.to_string())?;
            tokio::time::sleep(BT_TIMEOUT).await;

            Ok::<(), String>(())
        });

        match result {
            Ok(_) => json!({ "ok": true, "command": "bluetooth", "subcommand": "scan", "successful": true, "output": "Discovery finished" }),
            Err(e) => json!({ "ok": false, "error": e, "command": "bluetooth", "subcommand": "scan", "successful": false }),
        }
    })
    .join()
    .unwrap_or_else(|_| json!({ "ok": false, "error": "thread panicked", "command": "bluetooth", "subcommand": "scan", "successful": false }))
}

async fn with_device<F, Fut>(mac: &str, f: F) -> Result<(), String>
where
    F: FnOnce(bluer::Device) -> Fut,
    Fut: std::future::Future<Output = Result<(), bluer::Error>>,
{
    let addr: Address = mac.parse().map_err(|_| format!("Invalid MAC address: {}", mac))?;
    let session = Session::new().await.map_err(|e| e.to_string())?;
    let adapter = session.default_adapter().await.map_err(|e| e.to_string())?;
    let device = adapter.device(addr).map_err(|e| e.to_string())?;
    f(device).await.map_err(|e| e.to_string())
}

fn execute_device_action<F, Fut>(subcommand: &str, mac: &str, f: F) -> Value
where
    F: FnOnce(bluer::Device) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), bluer::Error>> + Send + 'static,
{
    let mac_str = mac.to_string();
    let subcmd = subcommand.to_string();
    
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => return json!({ "ok": false, "error": e.to_string() }),
        };

        let result = runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(12), with_device(&mac_str, f)).await
        });

        match result {
            Ok(Ok(_)) => json!({ "ok": true, "command": "bluetooth", "subcommand": subcmd, "successful": true, "output": format!("{} succeeded", subcmd) }),
            Ok(Err(e)) => json!({ "ok": false, "error": e, "command": "bluetooth", "subcommand": subcmd, "successful": false }),
            Err(_) => json!({ "ok": false, "error": "timeout", "command": "bluetooth", "subcommand": subcmd, "successful": false }),
        }
    })
    .join()
    .unwrap_or_else(move |_| json!({ "ok": false, "error": "thread panicked", "command": "bluetooth", "subcommand": subcommand, "successful": false }))
}

pub fn pair(mac: &str) -> Value {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => return json!({ "ok": false, "error": e.to_string() }),
    };

    let result = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(12), with_device(mac, |dev| async move {
            let _ = dev.pair().await;
            dev.set_trusted(true).await
        })).await
    });

    match result {
        Ok(Ok(_)) => json!({ "ok": true, "command": "bluetooth", "subcommand": "pair", "successful": true, "output": "paired" }),
        Ok(Err(e)) => json!({ "ok": false, "error": e, "command": "bluetooth", "subcommand": "pair", "successful": false }),
        Err(_) => json!({ "ok": false, "error": "timeout", "command": "bluetooth", "subcommand": "pair", "successful": false }),
    }
}

pub fn connect(mac: &str) -> Value {
    execute_device_action("connect", mac, |dev| async move { dev.connect().await })
}

pub fn disconnect(mac: &str) -> Value {
    execute_device_action("disconnect", mac, |dev| async move { dev.disconnect().await })
}

pub fn forget(mac: &str) -> Value {
    let mac_str = mac.to_string();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt,
            Err(e) => return json!({ "ok": false, "error": e.to_string() }),
        };

        let result = runtime.block_on(async {
            let addr: Address = mac_str.parse().map_err(|_| format!("Invalid MAC address: {}", mac_str))?;
            let session = Session::new().await.map_err(|e| e.to_string())?;
            let adapter = session.default_adapter().await.map_err(|e| e.to_string())?;
            adapter.remove_device(addr).await.map_err(|e| e.to_string())
        });

        match result {
            Ok(_) => json!({ "ok": true, "command": "bluetooth", "subcommand": "forget", "successful": true, "output": "forgotten" }),
            Err(e) => json!({ "ok": false, "error": e, "command": "bluetooth", "subcommand": "forget", "successful": false }),
        }
    })
    .join()
    .unwrap_or_else(|_| json!({ "ok": false, "error": "thread panicked", "command": "bluetooth", "subcommand": "forget", "successful": false }))
}

pub fn trust(mac: &str) -> Value {
    execute_device_action("trust", mac, |dev| async move { dev.set_trusted(true).await })
}

pub fn untrust(mac: &str) -> Value {
    execute_device_action("untrust", mac, |dev| async move { dev.set_trusted(false).await })
}
