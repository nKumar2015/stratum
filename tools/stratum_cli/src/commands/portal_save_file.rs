use dbus::arg::{PropMap, Variant};
use dbus::blocking::{Connection, Proxy};
use dbus::message::MatchRule;
use std::collections::HashMap;
use std::env;
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::common::{debug, emit_help, emit_json, fail, is_help_flag};

fn print_help() {
    emit_help(
        "portal-save-file",
        "stratum-cli portal-save-file [title] [default_name]",
        &["(no subcommands)", "title (optional)", "default_name (optional)"],
    );
}

fn first_uri(results: &PropMap) -> Option<String> {
    let uris_variant = results.get("uris")?;
    let uris = &*uris_variant.0;

    let iter = uris.as_iter()?;
    for uri_arg in iter {
        if let Some(uri) = uri_arg.as_str() {
            return Some(uri.to_string());
        }
    }
    None
}

fn unique_token() -> String {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros())
        .unwrap_or(0);
    format!("quickshell{}{}", micros, std::process::id())
}

fn sender_path_fragment(conn: &Connection) -> String {
    let name = conn.unique_name().to_string();
    name.trim_start_matches(':').replace('.', "_")
}

pub fn handle(args: &[String]) {
    if let Some(first) = args.first() {
        if is_help_flag(first) {
            print_help();
            return;
        }
    }

    let title = args
        .first()
        .cloned()
        .unwrap_or_else(|| "Save File".to_string());
    let default_name = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "untitled".to_string());

    let timeout_secs = env::var("XDG_PORTAL_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|s| *s > 0)
        .unwrap_or(30);

    let conn = Connection::new_session().unwrap_or_else(|e| fail(&format!("failed to open session bus: {}", e)));

    let token = unique_token();
    let sender = sender_path_fragment(&conn);
    let expected_path = format!(
        "/org/freedesktop/portal/desktop/request/{}/{}",
        sender, token
    );

    debug(&format!("savefile.handle_token={}", token));
    debug(&format!("savefile.timeout={}", timeout_secs));
    debug(&format!("savefile.expected_path={}", expected_path));

    let mut options: PropMap = HashMap::new();
    options.insert("handle_token".to_string(), Variant(Box::new(token.clone())));
    options.insert("modal".to_string(), Variant(Box::new(true)));
    options.insert("current_name".to_string(), Variant(Box::new(default_name.clone())));

    let (tx, rx) = mpsc::channel::<(u32, PropMap)>();
    let mut rule = MatchRule::new_signal("org.freedesktop.portal.Request", "Response");
    rule.path = Some(expected_path.clone().into());

    conn.add_match(rule, move |(response, results): (u32, PropMap), _, _| {
        let _ = tx.send((response, results));
        true
    })
    .unwrap_or_else(|e| fail(&format!("failed to subscribe to request response: {}", e)));

    let proxy = Proxy::new(
        "org.freedesktop.portal.Desktop",
        "/org/freedesktop/portal/desktop",
        Duration::from_secs(timeout_secs),
        &conn,
    );

    let (request_path,): (dbus::Path<'static>,) = proxy
        .method_call(
            "org.freedesktop.portal.FileChooser",
            "SaveFile",
            ("", title.as_str(), options),
        )
        .unwrap_or_else(|e| fail(&format!("SaveFile call failed: {}", e)));

    let request_path_string = request_path.to_string();
    debug(&format!("savefile.request_path={}", request_path_string));
    if request_path_string != expected_path {
        debug(&format!(
            "savefile.handle_mismatch token={} returned={}",
            token, request_path_string
        ));
    }

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    loop {
        if Instant::now() >= deadline {
            fail("xdg-portal response timed out");
        }

        if let Ok((response, results)) = rx.try_recv() {
            if response == 1 || response == 2 {
                emit_json(json!({
                    "ok": true,
                    "command": "portal-save-file",
                    "status": "cancel",
                }));
                return;
            }

            if response != 0 {
                fail(&format!("xdg-portal unexpected response code: {}", response));
            }

            if let Some(uri) = first_uri(&results) {
                debug(&format!("savefile.result=ok|{}", uri));
                emit_json(json!({
                    "ok": true,
                    "command": "portal-save-file",
                    "status": "ok",
                    "uri": uri,
                }));
                return;
            }

            fail("xdg-portal response missing uris");
        }

        conn.process(Duration::from_millis(150))
            .unwrap_or_else(|e| fail(&format!("dbus process failed: {}", e)));
    }
}
