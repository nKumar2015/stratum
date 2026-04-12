use std::env;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

fn socket_path() -> PathBuf {
    let runtime_dir = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime_dir).join("stratumd.sock")
}

/// Send a JSON-RPC request to the daemon and return the result.
/// Returns `Ok(Value)` with the full JSON-RPC response, or `Err(String)` on connection/timeout.
pub fn daemon_call(method: &str, params: Value) -> Result<Value, String> {
    let path = socket_path();
    let stream = UnixStream::connect(&path)
        .map_err(|err| format!("cannot connect to stratumd socket: {}", err))?;

    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .map_err(|err| format!("failed to set read timeout: {}", err))?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|err| format!("failed to set write timeout: {}", err))?;

    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": method,
        "params": params,
    });

    let payload = format!("{}\n", request);
    let mut writer = stream
        .try_clone()
        .map_err(|err| format!("failed to clone stream: {}", err))?;
    writer
        .write_all(payload.as_bytes())
        .map_err(|err| format!("failed to send request: {}", err))?;

    let reader = BufReader::new(stream);
    let mut response_line = String::new();
    let mut reader = reader;
    reader
        .read_line(&mut response_line)
        .map_err(|err| format!("failed to read daemon response: {}", err))?;

    let response: Value = serde_json::from_str(response_line.trim())
        .map_err(|err| format!("failed to parse daemon response: {}", err))?;

    Ok(response)
}
