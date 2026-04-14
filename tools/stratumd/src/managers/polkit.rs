use std::sync::Arc;
use zbus::{dbus_interface, Connection};
use serde_json::json;
use crate::AppState;
use std::collections::HashMap;

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
        _identities: Vec<zvariant::OwnedValue>,
    ) {
        // Extract basic user info from identities if possible
        let payload = json!({
            "action": action_id,
            "message": message,
            "icon": icon_name,
            "cookie": cookie,
            "active": true
        });
        self.state.polkit.update(payload);
    }

    async fn cancel_authentication(&self, cookie: String) {
        let payload = json!({
            "active": false,
            "cookie": cookie
        });
        self.state.polkit.update(payload);
    }
}

pub async fn register_agent(state: Arc<AppState>) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    let agent = PolkitAgent { state: Arc::clone(&state) };
    
    connection
        .object_server()
        .at("/org/stratum/PolkitAgent", agent)
        .await?;
        
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    ).await?;
    
    let mut details = HashMap::new();
    let kind = if let Ok(session_id) = std::env::var("XDG_SESSION_ID") {
        details.insert("session-id".to_string(), zvariant::Value::from(session_id));
        "unix-session"
    } else {
        // Fallback to current process
        details.insert("pid".to_string(), zvariant::Value::from(std::process::id()));
        // Note: start-time is technically required for unix-process but often skipped by newer Polkit versions 
        // if only one agent is registered.
        "unix-process"
    };

    let subject = (kind, details);
    
    let _ : () = proxy.call(
        "RegisterAuthenticationAgent",
        &(subject, "en_US.UTF-8", "/org/stratum/PolkitAgent")
    ).await?;

    println!("Polkit: Agent registered successfully for {}", kind);
    
    Ok(())
}

pub fn verify_password(user: &str, password: &str) -> bool {
    let mut authenticator = match pam_auth::Authenticator::with_password("stratum-polkit") {
        Ok(a) => a,
        Err(_) => return false,
    };
    
    authenticator.set_credentials(user, password);
    authenticator.authenticate().is_ok()
}

pub async fn notify_response(cookie: String, success: bool) -> zbus::Result<()> {
    if !success { return Ok(()); }
    
    let connection = Connection::session().await?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    ).await?;
    
    // Construct the identity (unix-user for current user)
    let mut details = HashMap::new();
    let uid = unsafe { libc::getuid() };
    details.insert("uid".to_string(), zvariant::Value::from(uid));
    let identity = ("unix-user", details);
    
    let _ : () = proxy.call(
        "AuthenticationAgentResponse",
        &(cookie, identity)
    ).await?;
    
    Ok(())
}
