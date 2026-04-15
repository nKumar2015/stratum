use std::process::Command;
use tracing::{debug, error};

pub fn newest_quickshell_pid() -> Result<u32, String> {
    let output = Command::new("qs")
        .args(["list", "--all", "--json"])
        .output()
        .map_err(|e| format!("failed to run qs list: {}", e))?;

    if !output.status.success() {
        return Err("qs list failed".to_string());
    }

    let list: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("failed to parse qs list: {}", e))?;

    let newest = list.as_array()
        .and_then(|a| a.iter().max_by_key(|i| i["launch_time"].as_str().unwrap_or("")))
        .and_then(|i| i["pid"].as_u64());

    match newest {
        Some(pid) => Ok(pid as u32),
        None => Err("no quickshell instances found".to_string()),
    }
}

pub fn send_shell_ipc_with_pid(pid: u32, source: &str, method: &str, args: &[String]) -> Result<(), String> {
    let mut cmd = Command::new("qs");
    cmd.arg("ipc").arg("--pid").arg(pid.to_string()).arg("call");
    cmd.arg(source).arg(method);
    for arg in args {
        cmd.arg(arg);
    }

    debug!("Running IPC command: {:?}", cmd);
    let output = cmd.output().map_err(|e| format!("failed to run qs ipc: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        error!("IPC call failed: {}", err);
        return Err(err.to_string());
    }

    Ok(())
}

pub fn broadcast_polkit_update(pid_cache: &std::sync::Mutex<Option<u32>>, payload: &serde_json::Value) {
    let pid = {
        let cached = pid_cache.lock().ok().and_then(|g| *g);
        match cached {
            Some(pid) => pid,
            None => {
                if let Ok(pid) = newest_quickshell_pid() {
                    if let Ok(mut cache) = pid_cache.lock() {
                        *cache = Some(pid);
                    }
                    pid
                } else {
                    return;
                }
            }
        }
    };

    let payload_text = payload.to_string();
    let _ = send_shell_ipc_with_pid(pid, "daemon", "polkit", &[payload_text]);
}
