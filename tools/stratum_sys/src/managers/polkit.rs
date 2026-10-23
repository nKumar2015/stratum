//! polkit authentication agent.
//!
//! Registers as the session's `AuthenticationAgent` on the system bus and
//! drives prompts for privileged actions. The agent owns its own pending
//! session bookkeeping and reports progress through a caller-supplied
//! [`PolkitSink`] so this module stays independent of any host state or UI.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::oneshot;
use tracing::{info, warn};
use zbus::zvariant;
use zbus::Connection;

/// How a pending request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Authorized,
    Denied,
    /// The user closed the prompt. Reported to polkitd as `Cancelled`, which
    /// pkexec shows as "Request dismissed" rather than a failed password.
    Dismissed,
}

/// A single in-flight authorization request, keyed by polkit cookie.
pub struct AuthSession {
    tx: oneshot::Sender<Outcome>,
    attempts: u32,
}

/// Errors returned to polkitd from `BeginAuthentication`.
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.freedesktop.PolicyKit1.Error")]
enum AgentError {
    #[zbus(error)]
    ZBus(zbus::Error),
    Failed(String),
    Cancelled(String),
}

/// How long the agent waits for a UI answer before treating a request as
/// abandoned. Without this a dismissed or never-shown prompt would leave the
/// originating `pkexec` blocked forever.
const AUTH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);

/// Maximum password attempts before the agent gives up on a cookie.
const MAX_ATTEMPTS: u32 = 3;

/// Result of recording a failed authentication attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureOutcome {
    /// The cookie is still live; the user may try again.
    Retry { remaining: u32 },
    /// Attempts are used up; the session has been closed.
    Exhausted,
    /// No such cookie — already answered, cancelled, or timed out.
    Unknown,
}

/// Callback the agent uses to surface authentication state to its host.
///
/// The payload is the `polkit` object the shell already understands:
/// `{ active, action, message, icon, cookie, user, success? }`.
pub type PolkitSink = Arc<dyn Fn(Value) + Send + Sync>;

/// Everything the agent needs from the embedding host.
#[derive(Clone)]
pub struct PolkitHost {
    publisher: crate::Publisher,
}

impl PolkitHost {
    pub fn new(publisher: crate::Publisher) -> Self {
        Self { publisher }
    }

    fn emit(&self, payload: Value) {
        self.publisher.polkit(payload);
    }
}

fn get_session_id() -> Option<String> {
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        return Some(id);
    }

    let uid = crate::current_uid();
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
    host: PolkitHost,
    /// Shared so the instance served over D-Bus and the handle kept by the host
    /// observe one pending-session map.
    pending: Arc<Mutex<HashMap<String, AuthSession>>>,
}

impl Clone for PolkitAgent {
    fn clone(&self) -> Self {
        Self {
            host: self.host.clone(),
            pending: Arc::clone(&self.pending),
        }
    }
}

impl PolkitAgent {
    pub fn new(host: PolkitHost) -> Self {
        Self {
            host,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Complete a pending request with the result of a password check.
    ///
    /// Returns `true` when a live session consumed the answer. Called by the
    /// host in response to its own `polkit.respond` request.
    pub fn resolve(&self, cookie: &str, success: bool) -> bool {
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        let Some(session) = pending.remove(cookie) else {
            return false;
        };
        let outcome = if success {
            Outcome::Authorized
        } else {
            Outcome::Denied
        };
        session.tx.send(outcome).is_ok()
    }

    /// End a pending request because the user closed the prompt.
    ///
    /// Without this a dismissed dialog would leave the requesting program
    /// blocked until the agent's timeout.
    pub fn dismiss(&self, cookie: &str) -> bool {
        let Ok(mut pending) = self.pending.lock() else {
            return false;
        };
        let Some(session) = pending.remove(cookie) else {
            return false;
        };
        session.tx.send(Outcome::Dismissed).is_ok()
    }

    /// Number of attempts already spent on a cookie, if it is still pending.
    pub fn attempts(&self, cookie: &str) -> Option<u32> {
        self.pending
            .lock()
            .ok()
            .and_then(|pending| pending.get(cookie).map(|s| s.attempts))
    }

    /// Record a failed password attempt for `cookie`.
    pub fn record_failure(&self, cookie: &str) -> FailureOutcome {
        let Ok(mut pending) = self.pending.lock() else {
            return FailureOutcome::Unknown;
        };
        let Some(session) = pending.get_mut(cookie) else {
            return FailureOutcome::Unknown;
        };
        session.attempts += 1;
        let remaining = MAX_ATTEMPTS.saturating_sub(session.attempts);
        if remaining == 0 {
            // Signal failure so the waiting `begin_authentication` unblocks.
            if let Some(session) = pending.remove(cookie) {
                let _ = session.tx.send(Outcome::Denied);
            }
            return FailureOutcome::Exhausted;
        }
        FailureOutcome::Retry { remaining }
    }

    fn emit(&self, payload: Value) {
        self.host.emit(payload);
    }
}

#[zbus::interface(name = "org.freedesktop.PolicyKit1.AuthenticationAgent")]
impl PolkitAgent {
    async fn begin_authentication(
        &self,
        action_id: String,
        message: String,
        icon_name: String,
        _details: HashMap<String, String>,
        cookie: String,
        identities: Vec<(String, HashMap<String, zvariant::OwnedValue>)>,
    ) -> Result<(), AgentError> {
        info!(
            "BEGIN_AUTHENTICATION: action={}, cookie={}",
            action_id, cookie
        );

        let target_user = identities
            .first()
            .and_then(|(kind, details)| {
                if kind == "unix-user" {
                    details.get("uid").and_then(|v| {
                        v.downcast_ref::<u32>()
                            .ok()
                            .or_else(|| v.downcast_ref::<u16>().ok().map(|v| v as u32))
                            .or_else(|| v.downcast_ref::<i32>().ok().map(|v| v as u32))
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

        // Register the session before telling the UI, so a fast reply cannot
        // race ahead of the bookkeeping.
        let (tx, rx) = oneshot::channel();
        {
            let Ok(mut pending) = self.pending.lock() else {
                return Err(AgentError::Failed("polkit state poisoned".into()));
            };
            pending.insert(cookie.clone(), AuthSession { tx, attempts: 0 });
        }

        info!("broadcasting UI request for cookie: {}", cookie);
        self.emit(json!({
            "action": action_id,
            "message": message,
            "icon": icon_name,
            "cookie": cookie.clone(),
            "active": true,
            "user": target_user
        }));

        // Hold the D-Bus call open while the user answers, but never forever:
        // a prompt that is dismissed or never rendered must not wedge pkexec.
        //
        // zbus runs this method on its own executor, not inside Tokio, so a
        // Tokio timer here panics ("no reactor running") on every request. A
        // plain thread expires the session instead, and the oneshot receiver
        // works on any executor.
        expire_after(Arc::clone(&self.pending), cookie.clone(), AUTH_TIMEOUT);
        let outcome = rx.await.unwrap_or(Outcome::Denied);
        let success = outcome == Outcome::Authorized;

        info!("authentication result for cookie {}: {:?}", cookie, outcome);

        self.emit(json!({
            "active": false,
            "success": success,
            "cookie": cookie.clone(),
            "message": match outcome {
                Outcome::Authorized => "Authentication successful.",
                Outcome::Denied => "Authentication failed. Please check your password.",
                Outcome::Dismissed => "Authentication dismissed.",
            }
        }));

        match outcome {
            Outcome::Dismissed => Err(AgentError::Cancelled("Dismissed by the user".into())),
            _ => Ok(()),
        }
    }

    async fn cancel_authentication(&self, cookie: String) {
        info!("CANCEL_AUTHENTICATION: cookie={}", cookie);
        self.emit(json!({ "active": false, "cookie": cookie.clone() }));

        if let Ok(mut pending) = self.pending.lock() {
            pending.remove(&cookie);
        }
    }
}

/// Fail `cookie` with `Denied` once `after` has passed, unless it was answered
/// first. Cookies are unique per request, so a later request cannot be hit.
fn expire_after(
    pending: Arc<Mutex<HashMap<String, AuthSession>>>,
    cookie: String,
    after: std::time::Duration,
) {
    std::thread::Builder::new()
        .name("polkit-timeout".into())
        .spawn(move || {
            std::thread::sleep(after);
            let session = pending.lock().ok().and_then(|mut p| p.remove(&cookie));
            if let Some(session) = session {
                warn!("authentication for cookie {} timed out", cookie);
                let _ = session.tx.send(Outcome::Denied);
            }
        })
        .ok();
}

/// Which requests the agent answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSubject {
    /// Every request from the login session. Only one agent may hold this.
    Session,
    /// Only requests made by one process, the way `pkttyagent --process`
    /// works. Lets a second agent be tested while another owns the session.
    Process(u32),
}

/// Register as an authentication agent for `subject` and serve requests until
/// the process exits.
pub async fn register_agent_for(agent: Arc<PolkitAgent>, subject: AgentSubject) -> zbus::Result<()> {
    let connection = Connection::system().await?;

    let object_path = "/org/freedesktop/PolicyKit1/AuthenticationAgent";

    // Serve a clone: `pending` is behind an `Arc`, so the instance receiving
    // D-Bus calls and the handle the host keeps share one session map.
    connection
        .object_server()
        .at(object_path, (*agent).clone())
        .await?;

    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.PolicyKit1",
        "/org/freedesktop/PolicyKit1/Authority",
        "org.freedesktop.PolicyKit1.Authority",
    )
    .await?;

    let mut details: HashMap<&str, zvariant::Value> = HashMap::new();
    let kind = match subject {
        AgentSubject::Session => {
            let session_id =
                get_session_id().ok_or_else(|| zbus::Error::Failure("No session ID".into()))?;
            details.insert("session-id", zvariant::Value::from(session_id));
            "unix-session"
        }
        AgentSubject::Process(pid) => {
            let start_time = process_start_time(pid)
                .ok_or_else(|| zbus::Error::Failure(format!("no such process {pid}")))?;
            details.insert("pid", zvariant::Value::from(pid));
            details.insert("start-time", zvariant::Value::from(start_time));
            "unix-process"
        }
    };

    proxy
        .call::<&str, _, ()>(
            "RegisterAuthenticationAgent",
            &((kind, details), "en_US.UTF-8", object_path),
        )
        .await?;

    info!("agent registered for {:?}", subject);
    std::future::pending::<()>().await;
    Ok(())
}

/// Serve the agent on the session bus under `name` without registering with
/// polkitd, so a test can call `BeginAuthentication` itself. With
/// `STRATUM_POLKIT_HELPER` pointing at a stand-in checker, the whole prompt can
/// be exercised without a real account or password.
pub async fn serve_fixture(agent: Arc<PolkitAgent>, name: &str) -> zbus::Result<()> {
    let connection = Connection::session().await?;
    connection
        .object_server()
        .at("/org/freedesktop/PolicyKit1/AuthenticationAgent", (*agent).clone())
        .await?;
    connection.request_name(name).await?;
    info!("polkit fixture agent serving as {name}");
    std::future::pending::<()>().await;
    Ok(())
}

/// A process's start time in clock ticks since boot, which polkit uses with the
/// pid to identify a process (field 22 of `/proc/<pid>/stat`).
fn process_start_time(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name (field 2) may contain spaces; fields resume after ')'.
    let rest = stat.rsplit_once(')')?.1;
    rest.split_whitespace().nth(19)?.parse().ok()
}

/// Verify a password against polkit's helper, which performs the actual
/// authentication against polkitd.
pub async fn verify_password_and_notify(user: &str, password: &str, cookie: &str) -> bool {
    // The helper reads the cookie and password as two newline-terminated
    // lines, so a line break cannot be sent; everything else, including
    // leading and trailing spaces, is part of the password. (Trimming spaces
    // here used to make such passwords impossible to enter.)
    if password.contains(['\r', '\n']) || cookie.contains(['\r', '\n']) {
        warn!("refusing to send a password or cookie containing a line break");
        return false;
    }
    let payload = format!("{}\n{}\n", cookie, password);

    tracing::info!("spawning async helper for user: {}...", user);

    let helper = std::env::var("STRATUM_POLKIT_HELPER")
        .unwrap_or_else(|_| "/run/wrappers/bin/polkit-agent-helper-1".to_string());

    let mut child = match Command::new(&helper)
        .arg(user)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(err) => {
            warn!("failed to spawn polkit helper at {}: {}", helper, err);
            return false;
        }
    };

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes()).await;
        let _ = stdin.flush().await;
        // Dropping stdin sends the EOF the helper waits for.
    }

    match child.wait().await {
        Ok(status) => {
            tracing::info!("verification finished. Success: {}", status.success());
            status.success()
        }
        Err(err) => {
            warn!("failed to wait for polkit helper: {}", err);
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_agent() -> (Arc<PolkitAgent>, Arc<AtomicUsize>) {
        let emissions = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&emissions);
        let publisher = crate::Publisher::new(move |_channel, _payload| {
            counter.fetch_add(1, Ordering::Relaxed);
        });
        let agent = Arc::new(PolkitAgent::new(PolkitHost::new(publisher)));
        (agent, emissions)
    }

    /// A clone served over D-Bus must share one pending-session map, otherwise
    /// `resolve` would never find the session the interface registered.
    #[test]
    fn clone_shares_pending_sessions() {
        let (agent, _) = test_agent();
        let served = (*agent).clone();

        let (tx, mut rx) = oneshot::channel();
        served
            .pending
            .lock()
            .unwrap()
            .insert("cookie-1".into(), AuthSession { tx, attempts: 0 });

        assert!(agent.resolve("cookie-1", true), "handle should resolve a session registered on the served clone");
        assert_eq!(rx.try_recv(), Ok(Outcome::Authorized));
    }

    #[test]
    fn dismissing_ends_the_request_as_cancelled() {
        let (agent, _) = test_agent();
        let (tx, mut rx) = oneshot::channel();
        agent
            .pending
            .lock()
            .unwrap()
            .insert("c".into(), AuthSession { tx, attempts: 0 });
        assert!(agent.dismiss("c"));
        assert_eq!(rx.try_recv(), Ok(Outcome::Dismissed));
        assert!(!agent.dismiss("c"), "a cookie is answered once");
    }

    /// The timeout must work without a Tokio runtime: zbus calls the agent
    /// from its own executor.
    #[test]
    fn unanswered_requests_expire_without_tokio() {
        let (agent, _) = test_agent();
        let (tx, rx) = oneshot::channel();
        agent
            .pending
            .lock()
            .unwrap()
            .insert("c".into(), AuthSession { tx, attempts: 0 });
        expire_after(
            Arc::clone(&agent.pending),
            "c".into(),
            std::time::Duration::from_millis(20),
        );
        let outcome = block_on_without_runtime(rx);
        assert_eq!(outcome, Ok(Outcome::Denied));
        assert!(!agent.resolve("c", true), "expired sessions are gone");
    }

    #[test]
    fn answered_requests_are_not_expired() {
        let (agent, _) = test_agent();
        let (tx, mut rx) = oneshot::channel();
        agent
            .pending
            .lock()
            .unwrap()
            .insert("c".into(), AuthSession { tx, attempts: 0 });
        expire_after(
            Arc::clone(&agent.pending),
            "c".into(),
            std::time::Duration::from_millis(20),
        );
        assert!(agent.resolve("c", true));
        std::thread::sleep(std::time::Duration::from_millis(60));
        assert_eq!(rx.try_recv(), Ok(Outcome::Authorized));
    }

    /// Poll a future to completion on this thread, with no runtime at all.
    fn block_on_without_runtime<F: std::future::Future>(future: F) -> F::Output {
        use std::task::{Context, Poll, Wake, Waker};
        struct Unpark(std::thread::Thread);
        impl Wake for Unpark {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            std::thread::park();
        }
    }

    /// polkitd reports "dismissed" only for this exact error name.
    #[test]
    fn dismissal_uses_polkits_cancelled_error() {
        use zbus::DBusError;
        let err = AgentError::Cancelled("x".into());
        assert_eq!(err.name().as_str(), "org.freedesktop.PolicyKit1.Error.Cancelled");
    }

    #[test]
    fn start_time_parses_this_process() {
        let start = process_start_time(std::process::id()).expect("own start time");
        assert!(start > 0);
        assert_eq!(process_start_time(u32::MAX), None);
    }

    #[test]
    fn resolve_unknown_cookie_reports_false() {
        let (agent, _) = test_agent();
        assert!(!agent.resolve("nope", true));
    }

    #[test]
    fn failure_retries_then_exhausts() {
        let (agent, _) = test_agent();
        let (tx, mut rx) = oneshot::channel();
        agent
            .pending
            .lock()
            .unwrap()
            .insert("c".into(), AuthSession { tx, attempts: 0 });

        assert_eq!(
            agent.record_failure("c"),
            FailureOutcome::Retry { remaining: 2 }
        );
        assert_eq!(
            agent.record_failure("c"),
            FailureOutcome::Retry { remaining: 1 }
        );
        assert_eq!(agent.record_failure("c"), FailureOutcome::Exhausted);

        // Exhaustion signals the waiting caller and drops the session.
        assert_eq!(rx.try_recv(), Ok(Outcome::Denied));
        assert_eq!(agent.record_failure("c"), FailureOutcome::Unknown);
    }

    #[test]
    fn emissions_use_the_polkit_channel() {
        let (agent, emissions) = test_agent();
        agent.emit(json!({ "active": true }));
        assert_eq!(emissions.load(Ordering::Relaxed), 1);
    }
}
