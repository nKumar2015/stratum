//! The shell's IPC socket.
//!
//! External keybinds reach the shell through `stratum-cli`, which talks to a
//! Unix socket at `$XDG_RUNTIME_DIR/stratum-shell.sock` (override with
//! `$STRATUM_SOCKET`), one JSON request line and one JSON response line per
//! connection.
//!
//! A request for a target or function [`functions`] does not list is refused
//! while parsing. A call the shell refuses at run time (a lock it cannot take
//! safely) answers `unsupported`, which the CLI reports.
//!
//! Protocol:
//!
//! ```text
//! -> {"target":"notifications","function":"toggle","args":[]}\n
//! <- {"ok":true}\n
//! <- {"ok":false,"code":"unsupported","error":"..."}\n
//! ```

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use serde_json::{json, Value};
use tracing::{debug, info, warn};

/// Longest request we will read. Every real call is far smaller; the limit
/// exists so a misbehaving client cannot make the shell allocate without bound.
const MAX_REQUEST_BYTES: u64 = 64 * 1024;

/// How long a client may take to send its request and read our answer.
const CLIENT_TIMEOUT: Duration = Duration::from_secs(3);

/// The functions each target exposes. This mirrors `IpcModule.qml` exactly.
pub fn functions(target: &str) -> Option<&'static [&'static str]> {
    match target {
        "dashboard" => Some(&["open", "close", "toggle"]),
        "lockscreen" => Some(&["lock"]),
        "notifications" => Some(&["open", "close", "toggle", "clear", "toggleDnd"]),
        "powermenu" => Some(&["toggle"]),
        "screenshot" => Some(&["start"]),
        // Native only: the QML shell had no settings window to open.
        "settings" => Some(&["open", "close", "toggle"]),
        "theme" => Some(&["open", "close", "toggle", "set"]),
        // Native only: the dashboard's Focus tab.
        "todo" => Some(&["add"]),
        "timer" => Some(&["start", "pause", "toggle", "stop"]),
        _ => None,
    }
}

/// One parsed IPC call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpcCall {
    pub target: String,
    pub function: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcError {
    BadRequest(String),
    UnknownTarget(String),
    UnknownFunction {
        target: String,
        function: String,
        expected: &'static [&'static str],
    },
    BadArgs(String),
    Unsupported,
    Failed(String),
}

impl IpcError {
    pub fn code(&self) -> &'static str {
        match self {
            IpcError::BadRequest(_) => "bad_request",
            IpcError::UnknownTarget(_) => "unknown_target",
            IpcError::UnknownFunction { .. } => "unknown_function",
            IpcError::BadArgs(_) => "bad_args",
            IpcError::Unsupported => "unsupported",
            IpcError::Failed(_) => "failed",
        }
    }

    pub fn message(&self) -> String {
        match self {
            IpcError::BadRequest(message) => message.clone(),
            IpcError::UnknownTarget(target) => format!("unknown target: {target}"),
            IpcError::UnknownFunction {
                target,
                function,
                expected,
            } => format!(
                "unknown {target} function: {function} (expected one of: {})",
                expected.join(", ")
            ),
            IpcError::BadArgs(message) => message.clone(),
            IpcError::Unsupported => "not implemented by this shell".to_string(),
            IpcError::Failed(message) => message.clone(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "ok": false,
            "code": self.code(),
            "error": self.message(),
        })
    }
}

/// Parse and validate one request line.
pub fn parse_request(line: &str) -> Result<IpcCall, IpcError> {
    let line = line.trim();
    if line.is_empty() {
        return Err(IpcError::BadRequest("empty request".into()));
    }
    let value: Value = serde_json::from_str(line)
        .map_err(|err| IpcError::BadRequest(format!("invalid json: {err}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| IpcError::BadRequest("request must be a json object".into()))?;

    let target = object
        .get("target")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| IpcError::BadRequest("missing target".into()))?
        .to_string();
    let function = object
        .get("function")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| IpcError::BadRequest("missing function".into()))?
        .to_string();

    let args = match object.get("args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_string)
                    .ok_or_else(|| IpcError::BadRequest("args must be strings".into()))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(IpcError::BadRequest("args must be an array".into())),
    };

    let expected =
        functions(&target).ok_or_else(|| IpcError::UnknownTarget(target.clone()))?;
    if !expected.contains(&function.as_str()) {
        return Err(IpcError::UnknownFunction {
            target,
            function,
            expected,
        });
    }

    // `theme set` takes exactly one non-empty name, `settings open|toggle`
    // an optional page and `dashboard open|toggle` an optional tab; every
    // other function takes none. Validating here keeps
    // the UI-side match arms simple and gives the CLI a useful error instead of
    // a silently ignored argument.
    if target == "settings" && function != "close" {
        if args.len() > 1 {
            return Err(IpcError::BadArgs(format!(
                "settings {function} expects at most 1 argument, got {}",
                args.len()
            )));
        }
        if let Some(name) = args.first() {
            if crate::ui::control_center::Page::from_name(name).is_none() {
                return Err(IpcError::BadArgs(format!("unknown settings page: {name}")));
            }
        }
    } else if target == "dashboard" && function != "close" {
        // An optional tab; the shell checks the name against its config.
        if args.len() > 1 {
            return Err(IpcError::BadArgs(format!(
                "dashboard {function} expects at most 1 argument, got {}",
                args.len()
            )));
        }
    } else if target == "todo" {
        // The words of an unquoted `todo add buy milk`, joined by the shell.
        if args.iter().all(|a| a.trim().is_empty()) {
            return Err(IpcError::BadArgs("todo add expects the to-do's text".into()));
        }
    } else if target == "timer" && function == "start" {
        // A length, `pomodoro` or `stopwatch`; none resumes. The shell
        // parses the length.
        if args.len() > 1 {
            return Err(IpcError::BadArgs(format!("timer start expects at most 1 argument, got {}", args.len())));
        }
    } else {
        let required = usize::from(function == "set");
        if args.len() != required {
            return Err(IpcError::BadArgs(format!(
                "{target} {function} expects {required} argument(s), got {}",
                args.len()
            )));
        }
        if required == 1 && args[0].trim().is_empty() {
            return Err(IpcError::BadArgs("theme name must not be empty".into()));
        }
    }

    Ok(IpcCall {
        target,
        function,
        args,
    })
}

/// A request handed to the UI thread, with a channel for its answer.
pub struct Request {
    pub call: IpcCall,
    pub reply: Sender<Result<(), IpcError>>,
}

/// The socket path. `$STRATUM_SOCKET` wins, which makes tests easy.
pub fn socket_path() -> PathBuf {
    if let Ok(path) = std::env::var("STRATUM_SOCKET") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(runtime).join("stratum-shell.sock")
}

/// A running listener. Dropping it removes the socket file.
pub struct Server {
    path: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        // Only remove it if it is still ours: a newer instance may have replaced
        // it, and unlinking theirs would silently break the other shell.
        if let Ok(metadata) = std::fs::metadata(&self.path) {
            if metadata.file_type().is_socket() {
                let _ = std::fs::remove_file(&self.path);
            }
        }
    }
}

/// Start the listener thread.
///
/// Returns `None` when another shell already owns the socket; the shell keeps
/// running without IPC rather than stealing it. `$XDG_RUNTIME_DIR` is a
/// per-user directory, so the file never needs to be world-accessible.
pub fn start(tx: calloop::channel::Sender<Request>) -> Option<Server> {
    let path = socket_path();
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            warn!("ipc: cannot create {}: {err}", parent.display());
            return None;
        }
    }

    // Refuse to clobber a live instance. A refused connection means the file is
    // stale (the previous shell crashed or was killed) and safe to replace.
    if path.exists() {
        match UnixStream::connect(&path) {
            Ok(_) => {
                warn!(
                    "ipc: {} is already in use by another shell; running without IPC",
                    path.display()
                );
                return None;
            }
            Err(_) => {
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(err) => {
            warn!("ipc: cannot bind {}: {err}", path.display());
            return None;
        }
    };
    // The runtime directory is already 0700, but be explicit: the socket
    // accepts commands as the user, so nobody else may connect.
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));

    info!("ipc listening on {}", path.display());
    let thread_path = path.clone();
    std::thread::Builder::new()
        .name("stratum-ipc".into())
        .spawn(move || serve(listener, tx))
        .map_err(|err| warn!("ipc: cannot start listener thread: {err}"))
        .ok()?;

    Some(Server {
        path: thread_path,
    })
}

fn serve(listener: UnixListener, tx: calloop::channel::Sender<Request>) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle_client(stream, &tx),
            Err(err) => {
                warn!("ipc accept failed: {err}");
                // A persistent accept error should not spin the CPU.
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

fn handle_client(stream: UnixStream, tx: &calloop::channel::Sender<Request>) {
    let _ = stream.set_read_timeout(Some(CLIENT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CLIENT_TIMEOUT));

    let mut line = String::new();
    let mut reader = BufReader::new((&stream).take(MAX_REQUEST_BYTES));
    match reader.read_line(&mut line) {
        Ok(0) => return,
        Ok(_) => {}
        Err(err) => {
            debug!("ipc read failed: {err}");
            return;
        }
    }

    let response = match parse_request(&line) {
        Ok(call) => {
            let (reply_tx, reply_rx) = mpsc::channel();
            if tx.send(Request { call, reply: reply_tx }).is_err() {
                IpcError::Failed("shell is shutting down".into()).to_json()
            } else {
                match reply_rx.recv_timeout(CLIENT_TIMEOUT) {
                    Ok(Ok(())) => json!({ "ok": true }),
                    Ok(Err(err)) => err.to_json(),
                    Err(_) => IpcError::Failed("timed out waiting for the shell".into()).to_json(),
                }
            }
        }
        Err(err) => err.to_json(),
    };

    let mut stream = stream;
    let _ = writeln!(stream, "{response}");
    let _ = stream.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(value: Value) -> Result<IpcCall, IpcError> {
        parse_request(&value.to_string())
    }

    #[test]
    fn parses_a_call() {
        let call = parse(json!({
            "target": "notifications",
            "function": "toggle",
            "args": []
        }))
        .unwrap();
        assert_eq!(call.target, "notifications");
        assert_eq!(call.function, "toggle");
        assert!(call.args.is_empty());
    }

    #[test]
    fn parses_an_argument() {
        let call = parse(json!({ "target": "theme", "function": "set", "args": ["gruvbox"] }))
            .unwrap();
        assert_eq!(call.args, vec!["gruvbox".to_string()]);
    }

    #[test]
    fn rejects_unknown_target() {
        let err = parse(json!({ "target": "nope", "function": "toggle" })).unwrap_err();
        assert_eq!(err.code(), "unknown_target");
    }

    #[test]
    fn rejects_unknown_function() {
        let err = parse(json!({ "target": "powermenu", "function": "open" })).unwrap_err();
        assert_eq!(err.code(), "unknown_function");
    }

    #[test]
    fn rejects_arity_mismatches() {
        let err = parse(json!({ "target": "theme", "function": "set", "args": [] })).unwrap_err();
        assert_eq!(err.code(), "bad_args");
        let err = parse(json!({
            "target": "notifications", "function": "toggle", "args": ["x"]
        }))
        .unwrap_err();
        assert_eq!(err.code(), "bad_args");
        let err = parse(json!({
            "target": "theme", "function": "set", "args": ["  "]
        }))
        .unwrap_err();
        assert_eq!(err.code(), "bad_args");
    }

    #[test]
    fn rejects_malformed_requests() {
        assert_eq!(parse_request("").unwrap_err().code(), "bad_request");
        assert_eq!(parse_request("{").unwrap_err().code(), "bad_request");
        assert_eq!(
            parse_request("[]").unwrap_err().code(),
            "bad_request",
            "a non-object must be rejected"
        );
        assert_eq!(
            parse(json!({ "target": "theme", "function": "set", "args": [1] }))
                .unwrap_err()
                .code(),
            "bad_request",
            "non-string args are a malformed request, not an arity error"
        );
    }

    #[test]
    fn settings_takes_an_optional_page() {
        let call = |function: &str, args: Value| {
            parse(json!({ "target": "settings", "function": function, "args": args }))
        };
        assert!(call("open", json!([])).is_ok());
        assert_eq!(call("toggle", json!(["Wifi"])).unwrap().args, ["Wifi"]);
        assert!(call("open", json!(["network"])).is_ok(), "the rail's title works too");
        assert_eq!(call("open", json!(["nope"])).unwrap_err().code(), "bad_args");
        assert_eq!(call("open", json!(["audio", "x"])).unwrap_err().code(), "bad_args");
        assert_eq!(call("close", json!(["audio"])).unwrap_err().code(), "bad_args");
    }

    #[test]
    fn dashboard_takes_an_optional_tab() {
        let call = |function: &str, args: Value| {
            parse(json!({ "target": "dashboard", "function": function, "args": args }))
        };
        assert!(call("toggle", json!([])).is_ok());
        // Any name parses: the tabs are config, checked by the shell.
        assert_eq!(call("open", json!(["system"])).unwrap().args, ["system"]);
        assert_eq!(call("open", json!(["a", "b"])).unwrap_err().code(), "bad_args");
        assert_eq!(call("close", json!(["system"])).unwrap_err().code(), "bad_args");
    }

    #[test]
    fn todo_and_timer_arguments() {
        let call = |target: &str, function: &str, args: Value| {
            parse(json!({ "target": target, "function": function, "args": args }))
        };
        assert_eq!(call("todo", "add", json!(["buy", "milk"])).unwrap().args, ["buy", "milk"]);
        assert_eq!(call("todo", "add", json!([])).unwrap_err().code(), "bad_args");
        assert_eq!(call("todo", "add", json!([" "])).unwrap_err().code(), "bad_args");
        assert!(call("timer", "start", json!(["25m"])).is_ok());
        assert!(call("timer", "start", json!([])).is_ok(), "resumes");
        assert_eq!(call("timer", "start", json!(["1", "2"])).unwrap_err().code(), "bad_args");
        assert_eq!(call("timer", "stop", json!(["x"])).unwrap_err().code(), "bad_args");
    }

    #[test]
    fn error_json_keeps_the_cli_contract() {
        let value = IpcError::Unsupported.to_json();
        assert_eq!(value["ok"], false);
        assert_eq!(value["code"], "unsupported");
        assert!(value["error"].is_string());
    }

    /// Exchange one request with a listener started on a temporary path.
    ///
    /// `STRATUM_SOCKET` is process-global, so the two socket tests live in one
    /// test to avoid racing each other.
    #[test]
    fn listener_answers_the_wire_protocol() {
        let dir = std::env::temp_dir().join(format!("stratum-ipc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.sock");
        std::env::set_var("STRATUM_SOCKET", &path);

        let (tx, rx) = calloop::channel::channel::<Request>();
        let server = start(tx).expect("listener starts on a free path");
        assert!(path.exists(), "socket file is created");

        // Requests are handled on the UI thread in the real shell; here a
        // helper thread plays that role and acknowledges every request.
        let responder = std::thread::spawn(move || {
            while let Ok(request) = rx.recv() {
                let ok = request.call.target == "notifications";
                let _ = request.reply.send(if ok {
                    Ok(())
                } else {
                    Err(IpcError::Failed("rejected".into()))
                });
            }
        });

        let ask = |body: &str| -> Value {
            let mut stream = UnixStream::connect(&path).unwrap();
            stream.write_all(format!("{body}\n").as_bytes()).unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            serde_json::from_str(line.trim()).unwrap()
        };

        let response = ask(r#"{"target":"notifications","function":"open"}"#);
        assert_eq!(response["ok"], true);

        // A validated-but-rejected call carries the responder's error.
        let response = ask(r#"{"target":"theme","function":"set","args":["carbon"]}"#);
        assert_eq!(response["ok"], false);
        assert_eq!(response["code"], "failed");

        let response = ask(r#"{"target":"screenshot","function":"stop"}"#);
        assert_eq!(response["code"], "unknown_function");

        let response = ask(r#"{"target":"nope","function":"toggle"}"#);
        assert_eq!(response["code"], "unknown_target");

        // A second shell must not steal a live socket.
        let (tx2, _rx2) = calloop::channel::channel::<Request>();
        assert!(
            start(tx2).is_none(),
            "a second shell must not steal a live socket"
        );

        drop(server);
        assert!(!path.exists(), "dropping the listener unlinks the socket");

        // ...but a stale file left by a crashed shell is replaced.
        let (tx3, _rx3) = calloop::channel::channel::<Request>();
        assert!(
            start(tx3).is_some(),
            "a stale socket file is replaced after the owner drops"
        );

        // The listener thread outlives the `Server` handle by design — the
        // shell runs until it exits — so its channel never closes. Detach the
        // responder with the listener rather than joining it.
        drop(responder);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
