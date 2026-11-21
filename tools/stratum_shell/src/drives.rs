//! Removable drives: the watcher behind the bar's drive icon and menu, the
//! toast when one is plugged in, and mount, unmount and eject.
//!
//! The udisks client is `stratum_drives`, shared with the file manager.
//! Nothing talks to udisks until there is a reason to
//! (`stratum_drives::wait_for_disk`): a disk that can come and go is
//! attached, appears in `/dev`, or udisks already runs. udisksd stays up
//! once started, so starting it at login would cost ~10–15 MB for a drive
//! that may never come.
//!
//! Operations run on threads of their own, never the action worker: a
//! mount may wait for polkit, and so for the user to type a password.
//! Toasts go over the session bus like any other notification, and their
//! buttons come back as `ActionInvoked`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use stratum_drives::{Drive, Udisks, Volume};
use tracing::{info, warn};

use crate::sys_state::SystemState;

/// What the menu and the toasts ask for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DriveOp {
    /// Mount the filesystem if it is not, then open its folder.
    Open(String),
    Unmount(String),
    /// Unmount everything on the drive, then power it off or eject it.
    Eject(String),
}

/// A locked volume waiting for its passphrase: the shell opens the
/// dialog (`PolkitDialog::open_unlock`) on its next frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnlockAsk {
    pub volume: String,
    pub name: String,
    pub drive: String,
    pub open: bool,
}

/// What the watcher and the operations share.
#[derive(Default)]
pub struct Handle {
    udisks: Mutex<Option<Udisks>>,
    /// The folder opener: `[launcher] launch_prefix` then `[drives]
    /// open_command`. Set at start and on every reload.
    open_command: Mutex<Vec<String>>,
    /// Toasts on screen: notification id → (drive, filesystem).
    toasts: Mutex<HashMap<u32, (String, String)>>,
    /// Whether the session-bus listener for toast buttons runs.
    listening: Mutex<bool>,
}

impl Handle {
    pub fn set_open_command(&self, command: Vec<String>) {
        if let Ok(mut slot) = self.open_command.lock() {
            *slot = command;
        }
    }

    /// The watcher's connection, or a new one (udisks is then started by
    /// the first call).
    fn udisks(&self) -> Result<Udisks, String> {
        let mut slot = self.udisks.lock().map_err(|_| "poisoned".to_string())?;
        if let Some(udisks) = slot.as_ref() {
            return Ok(udisks.clone());
        }
        let udisks = Udisks::connect()?;
        *slot = Some(udisks.clone());
        Ok(udisks)
    }
}

/// Start watching on a thread of its own.
pub fn start(state: Arc<SystemState>, notify: bool, automount: bool) {
    let spawned = std::thread::Builder::new().name("stratum-drives".into()).spawn(move || {
        let udisks = match Udisks::connect() {
            Ok(udisks) => udisks,
            Err(err) => {
                warn!("drives: no system bus, so drives will not show: {err}");
                return;
            }
        };
        let mut plugged = true;
        let woke = stratum_drives::wait_for_disk(|| {
            plugged = !(udisks.running() || stratum_drives::hotplug_present());
            !plugged
        });
        if let Err(err) = woke {
            warn!("drives: cannot watch /dev ({err}); asking udisks now");
            plugged = false;
        }
        info!("drives: talking to udisks");
        if let Ok(mut slot) = state.drives_handle.udisks.lock() {
            *slot = Some(udisks.clone());
        }
        let first = match udisks.drives() {
            Ok(drives) => drives,
            Err(err) => {
                warn!("drives: udisks is unavailable ({err}); is services.udisks2 enabled?");
                return;
            }
        };
        // Drives attached before the shell started are not news; the one
        // that woke the watcher is.
        let mut known: HashSet<String> = if plugged { HashSet::new() } else { volume_ids(&first) };
        let mut changed = |drives: Vec<Drive>| {
            for (drive, volume) in arrivals(&drives, &known) {
                if automount && can_mount(volume) {
                    spawn(&state, DriveOp::Open(volume.object.clone()), false);
                }
                if notify {
                    toast_arrival(&state, drive, volume, automount);
                }
            }
            close_departed_toasts(&state, &drives);
            known = volume_ids(&drives);
            if state.drives.set(drives) {
                state.notify();
            }
            true
        };
        changed(first);
        if let Err(err) = udisks.watch(Duration::from_millis(150), changed) {
            warn!("drives: stopped watching: {err}");
        }
    });
    if let Err(err) = spawned {
        warn!("drives: failed to start the watcher: {err}");
    }
}

fn volume_ids(drives: &[Drive]) -> HashSet<String> {
    drives.iter().flat_map(|d| d.volumes.iter().map(|v| v.object.clone())).collect()
}

/// The removable drives' filesystems in `drives` that are not in `known`.
/// One just unlocked is not news: it was asked for.
fn arrivals<'a>(drives: &'a [Drive], known: &HashSet<String>) -> Vec<(&'a Drive, &'a Volume)> {
    drives
        .iter()
        .filter(|d| d.removable)
        .flat_map(|d| d.volumes.iter().map(move |v| (d, v)))
        .filter(|(_, v)| !known.contains(&v.object) && v.backing.is_none())
        .collect()
}

fn can_mount(volume: &Volume) -> bool {
    !volume.locked && volume.mounted().is_none()
}

/// Run `op` on a thread of its own. `open` says whether `Open` opens the
/// folder (an automount only mounts).
pub fn spawn(state: &Arc<SystemState>, op: DriveOp, open: bool) {
    let state = Arc::clone(state);
    let spawned = std::thread::Builder::new()
        .name("stratum-drive-op".into())
        .stack_size(256 * 1024)
        .spawn(move || run(&state, op, open));
    if let Err(err) = spawned {
        warn!("drives: failed to start an operation: {err}");
    }
}

fn run(state: &Arc<SystemState>, op: DriveOp, open: bool) {
    let target = match &op {
        DriveOp::Open(id) | DriveOp::Unmount(id) | DriveOp::Eject(id) => id.clone(),
    };
    // A locked volume needs its passphrase first, which the shell asks for.
    if let DriveOp::Open(volume) = &op {
        let drives = state.drives.get();
        let locked = drives.iter().find_map(|d| d.volumes.iter().find(|v| &v.object == volume && v.locked).map(|v| (d, v)));
        if let Some((drive, locked)) = locked {
            let ask = UnlockAsk { volume: volume.clone(), name: locked.name(), drive: drive.name.clone(), open };
            if state.drives_unlock.set(Some(ask)) {
                state.notify();
            }
            return;
        }
    }
    set_busy(state, &target, true);
    let drives = state.drives.get();
    let result = state.drives_handle.udisks().and_then(|udisks| match &op {
        DriveOp::Open(volume) => {
            let path = udisks.mount(volume)?;
            if open {
                open_folder(state, &path)?;
            }
            Ok(())
        }
        DriveOp::Unmount(volume) => udisks.unmount(volume),
        DriveOp::Eject(drive) => {
            udisks.eject(drive)?;
            if let Some(found) = drives.iter().find(|d| &d.object == drive).filter(|d| !d.can_power_off) {
                post(&format!("{} can be removed", found.name), "", &[], 0);
            }
            Ok(())
        }
    });
    set_busy(state, &target, false);
    if let Err(err) = result {
        let name = describe(&drives, &target);
        let verb = match op {
            DriveOp::Open(_) if open => "open",
            DriveOp::Open(_) => "mount",
            DriveOp::Unmount(_) => "unmount",
            DriveOp::Eject(_) => "eject",
        };
        warn!("drives: could not {verb} {name}: {err}");
        post(&format!("Could not {verb} {name}"), &err, &[], 1);
    }
}

/// Unlock `volume` with `passphrase` on a thread of its own, answer the
/// dialog through `reply`, then mount what was inside (and open it when
/// `open`). Only the answer to the passphrase goes to the dialog: a mount
/// that fails afterwards is a notification, like any other.
pub fn unlock(
    state: &Arc<SystemState>,
    volume: String,
    passphrase: crate::pam::Secret,
    open: bool,
    reply: impl FnOnce(crate::polkit_dialog::Verdict) + Send + 'static,
) {
    use crate::polkit_dialog::Verdict;
    let state = Arc::clone(state);
    let spawned = std::thread::Builder::new()
        .name("stratum-drive-unlock".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            set_busy(&state, &volume, true);
            let unlocked = state.drives_handle.udisks().and_then(|udisks| udisks.unlock(&volume, passphrase.expose()));
            drop(passphrase);
            set_busy(&state, &volume, false);
            match unlocked {
                Ok(inner) => {
                    reply(Verdict::Accepted);
                    run(&state, DriveOp::Open(inner), open);
                }
                Err(err) => {
                    warn!("drives: could not unlock {volume}: {err}");
                    reply(Verdict::Failed(err));
                }
            }
        });
    if let Err(err) = spawned {
        warn!("drives: failed to start an unlock: {err}");
    }
}

fn set_busy(state: &SystemState, target: &str, busy: bool) {
    let mut list = state.drives_busy.get();
    list.retain(|t| t != target);
    if busy {
        list.push(target.to_string());
    }
    if state.drives_busy.set(list) {
        state.notify();
    }
}

/// A drive's or filesystem's name, for messages.
fn describe(drives: &[Drive], target: &str) -> String {
    drives
        .iter()
        .find_map(|d| {
            if d.object == target {
                return Some(d.name.clone());
            }
            d.volumes.iter().find(|v| v.object == target).map(Volume::name)
        })
        .unwrap_or_else(|| "the drive".into())
}

fn open_folder(state: &SystemState, path: &std::path::Path) -> Result<(), String> {
    let mut argv = state.drives_handle.open_command.lock().map(|c| c.clone()).unwrap_or_default();
    argv.push(path.to_string_lossy().into_owned());
    let Some((program, args)) = argv.split_first() else {
        return Err("no [drives] open_command".into());
    };
    let mut child = std::process::Command::new(program)
        .args(args)
        .spawn()
        .map_err(|err| format!("cannot run {program}: {err}"))?;
    // Reaped here, or it stays a zombie of the shell.
    std::thread::Builder::new()
        .name("stratum-reap".into())
        .stack_size(64 * 1024)
        .spawn(move || child.wait())
        .map(drop)
        .map_err(|err| err.to_string())
}

/// A toast's body: the drive, then the filesystem's size and type
/// (`SanDisk Cruzer · 16 GB · exfat`). Its title is the filesystem's name,
/// which differs between a drive's partitions: the server replaces a
/// notification with the same app and title.
pub fn summary(drive: &Drive, volume: &Volume) -> String {
    let mut parts = vec![drive.name.clone()];
    // An unlabelled one is named by its size already.
    if !volume.label.trim().is_empty() {
        parts.push(stratum_drives::format_size(volume.size));
    }
    if volume.locked {
        parts.push("encrypted".into());
    } else if !volume.fs_type.is_empty() {
        parts.push(volume.fs_type.clone());
    }
    parts.join(" \u{b7} ")
}

fn toast_arrival(state: &Arc<SystemState>, drive: &Drive, volume: &Volume, automount: bool) {
    let mut actions: Vec<(&str, &str)> = Vec::new();
    let open = if volume.locked { "Unlock" } else { "Open" };
    actions.extend([("default", open), ("open", open)]);
    actions.push(("eject", "Eject"));
    let title = if automount && can_mount(volume) { format!("{} mounted", volume.name()) } else { volume.name() };
    if let Some(id) = post(&title, &summary(drive, volume), &actions, 1) {
        if let Ok(mut toasts) = state.drives_handle.toasts.lock() {
            toasts.insert(id, (drive.object.clone(), volume.object.clone()));
        }
        listen_for_buttons(state);
    }
}

/// A drive taken out takes its toast with it.
fn close_departed_toasts(state: &SystemState, drives: &[Drive]) {
    let present = volume_ids(drives);
    let gone: Vec<u32> = match state.drives_handle.toasts.lock() {
        Ok(mut toasts) => {
            let gone = toasts.iter().filter(|(_, (_, v))| !present.contains(v)).map(|(id, _)| *id).collect::<Vec<_>>();
            toasts.retain(|id, _| !gone.contains(id));
            gone
        }
        Err(_) => return,
    };
    if gone.is_empty() {
        return;
    }
    let Ok(bus) = zbus::blocking::Connection::session() else {
        return;
    };
    for id in gone {
        let _ = bus.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "CloseNotification",
            &(id,),
        );
    }
}

/// Post a notification over the session bus; its id, if it was taken.
fn post(summary: &str, body: &str, actions: &[(&str, &str)], urgency: u8) -> Option<u32> {
    let actions: Vec<&str> = actions.iter().flat_map(|(key, label)| [*key, *label]).collect();
    crate::actions::notify("Drives", "drive-removable-media", urgency, summary, body, &actions)
        .inspect_err(|err| warn!("drives: {err}"))
        .ok()
}

/// Start, once, the thread that runs a toast's buttons.
fn listen_for_buttons(state: &Arc<SystemState>) {
    {
        let Ok(mut listening) = state.drives_handle.listening.lock() else {
            return;
        };
        if *listening {
            return;
        }
        *listening = true;
    }
    let state = Arc::clone(state);
    let spawned = std::thread::Builder::new()
        .name("stratum-drive-toasts".into())
        .stack_size(256 * 1024)
        .spawn(move || {
            if let Err(err) = watch_buttons(&state) {
                warn!("drives: toast buttons will do nothing: {err}");
            }
            if let Ok(mut listening) = state.drives_handle.listening.lock() {
                *listening = false;
            }
        });
    if let Err(err) = spawned {
        warn!("drives: failed to start the toast listener: {err}");
    }
}

fn watch_buttons(state: &Arc<SystemState>) -> zbus::Result<()> {
    let bus = zbus::blocking::Connection::session()?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.Notifications")?
        .path("/org/freedesktop/Notifications")?
        .build();
    for message in zbus::blocking::MessageIterator::for_match_rule(rule, &bus, Some(64))? {
        let Ok(message) = message else {
            continue;
        };
        let header = message.header();
        match header.member().map(|m| m.as_str()) {
            Some("ActionInvoked") => {
                let Ok((id, key)) = message.body().deserialize::<(u32, String)>() else {
                    continue;
                };
                let Some((drive, volume)) = state.drives_handle.toasts.lock().ok().and_then(|t| t.get(&id).cloned()) else {
                    continue;
                };
                match key.as_str() {
                    "default" | "open" => spawn(state, DriveOp::Open(volume), true),
                    "eject" => spawn(state, DriveOp::Eject(drive), false),
                    _ => {}
                }
            }
            Some("NotificationClosed") => {
                if let Ok((id, _)) = message.body().deserialize::<(u32, u32)>() {
                    if let Ok(mut toasts) = state.drives_handle.toasts.lock() {
                        toasts.remove(&id);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn volume(object: &str, label: &str) -> Volume {
        Volume {
            object: object.into(),
            device: format!("/dev/{object}"),
            label: label.into(),
            fs_type: "exfat".into(),
            size: 16_000_000_000,
            mount_points: Vec::new(),
            read_only: false,
            system: false,
            locked: false,
            backing: None,
        }
    }

    fn drive(object: &str, removable: bool, volumes: Vec<Volume>) -> Drive {
        Drive {
            object: object.into(),
            name: "SanDisk Cruzer".into(),
            removable,
            can_power_off: true,
            ejectable: false,
            optical: false,
            bus: "usb".into(),
            size: 16_000_000_000,
            volumes,
        }
    }

    #[test]
    fn only_new_filesystems_on_removable_drives_arrive() {
        let drives = vec![
            drive("nvme", false, vec![volume("nvme0n1p3", "Windows")]),
            drive("stick", true, vec![volume("sdb1", "OLD"), volume("sdb2", "NEW")]),
        ];
        let known: HashSet<String> = ["sdb1".to_string()].into();
        let arrived: Vec<_> = arrivals(&drives, &known).into_iter().map(|(_, v)| v.object.as_str()).collect();
        assert_eq!(arrived, ["sdb2"]);
        assert!(arrivals(&drives, &volume_ids(&drives)).is_empty());
    }

    #[test]
    fn toast_text() {
        let d = drive("stick", true, Vec::new());
        let mut v = volume("sdb1", "STICK");
        assert_eq!(summary(&d, &v), "SanDisk Cruzer \u{b7} 16 GB \u{b7} exfat");
        v.label.clear();
        v.locked = true;
        v.fs_type = "crypto_LUKS".into();
        assert_eq!(v.name(), "16 GB Volume");
        assert_eq!(summary(&d, &v), "SanDisk Cruzer \u{b7} encrypted");
        assert!(!can_mount(&v));
    }

    #[test]
    fn messages_name_what_failed() {
        let drives = vec![drive("stick", true, vec![volume("sdb1", "STICK")])];
        assert_eq!(describe(&drives, "stick"), "SanDisk Cruzer");
        assert_eq!(describe(&drives, "sdb1"), "STICK");
        assert_eq!(describe(&drives, "gone"), "the drive");
    }
}
