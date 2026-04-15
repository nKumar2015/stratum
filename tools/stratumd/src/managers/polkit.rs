use crate::AppState;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tracing::{error, info};
use zbus::zvariant;
use zbus::{dbus_interface, Connection};

fn get_session_id() -> Option<String> {
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        return Some(id);
    }

    let uid = unsafe { libc::getuid() };
    if let Ok(entries) = fs::read_dir("/run/systemd/sessions") {
        for entry in entries.flatten() {
            if let Ok(content) = fs::read_to_string(entry.path()) {
                if content.contains(&format!("UID={}", uid)) && content.contains("CLASS=user") {
                    return Some(entry.file_name().to_str().unwrap_or("").to_string());
                }
            }
        }
    }
    None
}

pub struct PolkitAgent {
    state: Arc<AppState>,
}

#[dbus_interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl PolkitAgent {
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        icon_name: String,
        _details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, zvariant::OwnedValue>)>,
    ) -> zbus::fdo::Result<()> {
        info!(
            "[polkit] BEGIN_AUTHENTICATION: action={}, cookie={}",
            action_id, cookie
        );

        let target_user = identities
            .first()
            .and_then(|(kind, details)| {
                if kind == "unix-user" {
                    details.get("uid").and_then(|v| {
                        v.downcast_ref::<u32>()
                            .copied()
                            .or_else(|| v.downcast_ref::<u16>().map(|&v| v as u32))
                            .or_else(|| v.downcast_ref::<i32>().map(|&v| v as u32))
                    })
                } else {
                    None
                }
            })
            .and_then(|uid| unsafe {
                let pwd = libc::getpwuid(uid);
                if !pwd.is_null() {
                    std::ffi::CStr::from_ptr((*pwd).pw_name)
                        .to_str()
                        .ok()
                        .map(|s| s.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "root".to_string());

        // 1. Create the suspension channel
        let (tx, rx) = tokio::sync::oneshot::channel();
        {
            let mut pending = self.state.pending_auths.lock().unwrap();
            pending.insert(cookie.clone(), crate::AuthSession { 
                tx, 
                attempts: 0 
            });
        }

        let payload = json!({
            "polkit": {
                "action": action_id,
                "message": message,
                "icon": icon_name,
                "cookie": cookie.clone(),
                "active": true,
                "user": target_user
            }
        });

        info!("[polkit] broadcasting UI request for cookie: {}", cookie);
        crate::ipc::broadcast_polkit_update(&self.state.qs_pid_cache, &payload);
        self.state.polkit.update(payload["polkit"].clone());

        // 2. The critical await: This keeps pkexec alive but lets D-Bus stay responsive
        let success = rx.await.unwrap_or(false);

        info!(
            "[polkit] authentication result for cookie {}: {}",
            cookie, success
        );

        let final_payload = json!({
            "polkit": {
                "active": false,
                "success": success,
                "cookie": cookie.clone(),
                "message": if success { 
                    "Authentication successful." 
                } else { 
                    "Authentication failed. Please check your password." 
                }
            }
        });

        self.state.polkit.update(final_payload["polkit"].clone());
        crate::ipc::broadcast_polkit_update(&self.state.qs_pid_cache, &final_payload);

        Ok(())
    }

    async fn cancel_authentication(&self, cookie: String) {
        info!("[polkit] CANCEL_AUTHENTICATION: cookie={}", cookie);
        let payload = json!({ "active": false, "cookie": cookie.clone() });
        self.state.polkit.update(payload);

        // Clean up the pending waker if it exists
        if let Ok(mut pending) = self.state.pending_auths.lock() {
            pending.remove(&cookie);
        }
    }
}

pub async fn register_agent(state: Arc<AppState>) -> zbus::Result<()> {
    let connection = Connection::system().await?;
    let agent = PolkitAgent {
        state: Arc::clone(&state),
    };

    let object_path = "/org/freedesktop/PolicyKit1/AuthenticationAgent";
    connection.object_server().at(object_path, agent).await?;

    let session_id =
        get_session_id().ok_or_else(|| zbus::Error::Failure("No session ID".into()))?;

    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    )
    .await?;

    let mut details = HashMap::new();
    details.insert(
        "session-id".to_string(),
        zvariant::Value::from(session_id.clone()),
    );
    let subject = ("unix-session", details);

    proxy
        .call::<&str, _, ()>(
            "RegisterAuthenticationAgent",
            &(subject, "en_US.UTF-8", object_path),
        )
        .await?;

    info!("[polkit] agent registered for session {}", session_id);
    std::future::pending::<()>().await;
    Ok(())
}

pub async fn verify_password_and_notify(user: &str, password: &str, cookie: &str) -> bool {
    // 1. Sanitize the password and format the magical two-line payload
    let clean_password = password.trim_matches(&['\r', '\n', ' '][..]);
    let payload = format!("{}\n{}\n", cookie, clean_password);

    tracing::info!("[polkit] spawning async helper for user: {}...", user);

    let mut child = Command::new("/run/wrappers/bin/polkit-agent-helper-1")
        .arg(user)
        .stdin(Stdio::piped())
        .stdout(Stdio::null()) // We no longer need to listen for the prompt!
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn polkit-agent-helper-1");

    // 2. Dump the cookie and password directly into the pipe
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes()).await;
        let _ = stdin.flush().await;
        // stdin is automatically dropped here, sending the required EOF
    }

    // 3. Asynchronously wait for the helper to verify with polkitd
    let status = child.wait().await.expect("Failed to wait for helper");

    tracing::info!(
        "[polkit] verification finished. Success: {}",
        status.success()
    );

    status.success()
}
