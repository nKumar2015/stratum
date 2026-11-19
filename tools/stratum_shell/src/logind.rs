//! logind: lock before the machine sleeps, lock on `loginctl lock-session`,
//! and suspend when idle.
//!
//! The sleep lock is a logind *delay* inhibitor. When anything asks to sleep
//! (the lid, the power menu, `systemctl suspend`), logind announces
//! `PrepareForSleep(true)` and waits, up to its `InhibitDelayMaxSec` (5 s by
//! default), for every delay inhibitor to close. The shell locks, and closes
//! its inhibitor once the lock screen has been drawn
//! ([`Logind::release_sleep`]), so the machine never wakes to an unlocked
//! desktop. On resume (`PrepareForSleep(false)`) it takes a new one.
//!
//! Both watchers block on zbus signal iterators on threads of their own and
//! report through a calloop channel; nothing here runs on the event loop.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use tracing::{info, warn};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{OwnedFd, OwnedObjectPath};

const DESTINATION: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";

/// How long to hold the sleep back for a lock that has not shown. Under
/// logind's own 5 s default, so it is the shell that gives up, and says so.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(4);

/// What logind asks of the event loop.
#[derive(Debug)]
pub enum Event {
    /// The machine is about to sleep: lock, then call
    /// [`Logind::release_sleep`].
    PrepareForSleep,
    /// `loginctl lock-session` (or anything else) asked the session to lock.
    LockRequested,
}

/// The running watchers. Dropping it lets the sleep watcher exit at its next
/// signal.
pub struct Logind {
    release: Sender<()>,
}

impl Logind {
    /// Start watching on threads of their own. The system bus is connected
    /// there too, so a slow bus never delays the shell's start.
    pub fn start(sink: calloop::channel::Sender<Event>, lock_before_sleep: bool, lock_on_loginctl: bool) -> Self {
        let (release, release_rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("stratum-logind".into()).spawn(move || {
            let bus = match Connection::system() {
                Ok(bus) => bus,
                Err(err) => {
                    warn!("logind unavailable, so the screen will not lock before sleep: {err}");
                    return;
                }
            };
            if lock_on_loginctl {
                let bus = bus.clone();
                let sink = sink.clone();
                let spawned = std::thread::Builder::new()
                    .name("stratum-logind-lock".into())
                    .spawn(move || {
                        if let Err(err) = watch_lock_requests(&bus, &sink) {
                            warn!("not listening for `loginctl lock-session`: {err}");
                        }
                    });
                if let Err(err) = spawned {
                    warn!("failed to start the session lock watcher: {err}");
                }
            }
            if lock_before_sleep {
                if let Err(err) = watch_sleep(&bus, &sink, &release_rx) {
                    warn!("the screen will not lock before sleep: {err}");
                }
            }
        });
        if let Err(err) = spawned {
            warn!("failed to start the logind watcher: {err}");
        }
        Self { release }
    }

    /// The lock screen is up (or will not come): let the sleep go ahead.
    pub fn release_sleep(&self) {
        let _ = self.release.send(());
    }
}

fn manager(bus: &Connection) -> zbus::Result<Proxy<'static>> {
    Proxy::new(bus, DESTINATION, MANAGER_PATH, MANAGER)
}

fn inhibit(manager: &Proxy<'_>) -> zbus::Result<OwnedFd> {
    manager.call("Inhibit", &("sleep", "Stratum", "Lock the screen before sleeping", "delay"))
}

fn watch_sleep(
    bus: &Connection,
    sink: &calloop::channel::Sender<Event>,
    release: &Receiver<()>,
) -> zbus::Result<()> {
    let manager = manager(bus)?;
    // Subscribe before inhibiting, so no announcement falls between the two.
    let signals = manager.receive_signal("PrepareForSleep")?;
    let mut inhibitor = Some(inhibit(&manager)?);
    info!("holding a logind sleep delay lock");
    for message in signals {
        let starting: bool = message.body().deserialize()?;
        if !starting {
            // Awake again: hold the next sleep back too.
            inhibitor = inhibit(&manager)
                .inspect_err(|err| warn!("could not take a new sleep delay lock: {err}"))
                .ok();
            continue;
        }
        // A release left over from a lock the shell confirmed on its own.
        while release.try_recv().is_ok() {}
        if sink.send(Event::PrepareForSleep).is_err() {
            break;
        }
        match release.recv_timeout(RELEASE_TIMEOUT) {
            Ok(()) => {}
            Err(RecvTimeoutError::Timeout) => {
                warn!("the lock screen did not show within {RELEASE_TIMEOUT:?}; letting the sleep go ahead");
            }
            Err(RecvTimeoutError::Disconnected) => break,
        }
        // Closing the descriptor is what lets logind continue.
        drop(inhibitor.take());
    }
    Ok(())
}

/// The session `loginctl lock-session` addresses. A systemd user unit is in
/// no session of its own, so without `XDG_SESSION_ID` it is the user's
/// display session.
fn session_path(bus: &Connection, manager: &Proxy<'_>) -> zbus::Result<OwnedObjectPath> {
    if let Ok(id) = std::env::var("XDG_SESSION_ID") {
        if let Ok(path) = manager.call("GetSession", &(id.as_str(),)) {
            return Ok(path);
        }
    }
    // SAFETY: getuid cannot fail.
    let uid = unsafe { libc::getuid() };
    let user: OwnedObjectPath = manager.call("GetUser", &(uid,))?;
    let user = Proxy::new(bus, DESTINATION, user, "org.freedesktop.login1.User")?;
    let (_, display): (String, OwnedObjectPath) = user.get_property("Display")?;
    if display.as_str() == "/" {
        return Err(zbus::Error::Failure("the user has no display session".into()));
    }
    Ok(display)
}

fn watch_lock_requests(bus: &Connection, sink: &calloop::channel::Sender<Event>) -> zbus::Result<()> {
    let manager = manager(bus)?;
    let path = session_path(bus, &manager)?;
    info!("locking on `loginctl lock-session` for {}", path.as_str());
    let session = Proxy::new(bus, DESTINATION, path, "org.freedesktop.login1.Session")?;
    for _ in session.receive_signal("Lock")? {
        if sink.send(Event::LockRequested).is_err() {
            break;
        }
    }
    Ok(())
}

/// Suspend, on a thread of its own. Unless `even_while_playing`, a media
/// player that is playing holds it off, as an inhibiting video would.
pub fn suspend(even_while_playing: bool) {
    let spawned = std::thread::Builder::new().name("stratum-suspend".into()).spawn(move || {
        if !even_while_playing && media_playing() {
            info!("idle: not suspending while media plays");
            return;
        }
        info!("idle: suspending");
        // Through logind, like every other suspend, so the sleep lock applies.
        match std::process::Command::new("systemctl").arg("suspend").status() {
            Ok(status) if status.success() => {}
            Ok(status) => warn!("`systemctl suspend` failed: {status}"),
            Err(err) => warn!("could not run `systemctl suspend`: {err}"),
        }
    });
    if let Err(err) = spawned {
        warn!("failed to start the suspend thread: {err}");
    }
}

/// Whether any MPRIS player reports `Playing`.
fn media_playing() -> bool {
    std::process::Command::new("playerctl")
        .args(["--all-players", "status"])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).lines().any(|line| line.trim() == "Playing"))
        .unwrap_or(false)
}
