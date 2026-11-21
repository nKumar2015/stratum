//! Drives and their filesystems, through udisks2 (`org.freedesktop.UDisks2`
//! on the system bus).
//!
//! udisks is what lets a user mount a USB stick without root: its polkit
//! rules allow the active session to mount removable drives, and ask for an
//! administrator for internal ones (our own polkit agent shows that
//! dialog). Mounted filesystems go under `/run/media/$USER`.
//!
//! udisksd stays up once started (~10–15 MB), so nothing here starts it
//! until it is needed: [`wait_for_disk`] sleeps on an inotify watch of
//! `/dev` until a disk that can come and go shows up, unless one is already
//! attached or udisks already runs. The file manager starts it by listing
//! drives.
//!
//! [`parse`] turns `GetManagedObjects` into [`Drive`]s, and is pure.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use zbus::blocking::{Connection, MessageIterator};
use zbus::zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

pub const SERVICE: &str = "org.freedesktop.UDisks2";
const ROOT: &str = "/org/freedesktop/UDisks2";
const BLOCK: &str = "org.freedesktop.UDisks2.Block";
const FILESYSTEM: &str = "org.freedesktop.UDisks2.Filesystem";
const ENCRYPTED: &str = "org.freedesktop.UDisks2.Encrypted";
const DRIVE: &str = "org.freedesktop.UDisks2.Drive";

/// One filesystem a user could open.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Volume {
    /// The block device's object path, which [`Udisks::mount`] takes.
    pub object: String,
    /// `/dev/sdb1`.
    pub device: String,
    /// The filesystem's label, or udisks' name hint; may be empty.
    pub label: String,
    /// `vfat`, `exfat`, `ext4`, …; `crypto_LUKS` while locked.
    pub fs_type: String,
    pub size: u64,
    pub mount_points: Vec<PathBuf>,
    pub read_only: bool,
    /// udisks' `HintSystem`: an internal disk, which only an administrator
    /// may mount.
    pub system: bool,
    /// Encrypted and not unlocked, so it cannot be mounted yet.
    pub locked: bool,
    /// For an unlocked filesystem, the encrypted device under it, locked
    /// again before its drive is powered off.
    pub backing: Option<String>,
}

impl Volume {
    /// Where it is mounted, if it is.
    pub fn mounted(&self) -> Option<&Path> {
        self.mount_points.first().map(PathBuf::as_path)
    }

    /// The name to show: its label, else its size.
    pub fn name(&self) -> String {
        if self.label.trim().is_empty() {
            format!("{} Volume", format_size(self.size))
        } else {
            self.label.trim().to_string()
        }
    }
}

/// A disk (or card, or disc) and the filesystems on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Drive {
    /// The drive's object path, which [`Udisks::eject`] takes.
    pub object: String,
    /// Vendor and model, `SanDisk Cruzer Blade`.
    pub name: String,
    /// The drive or its media can be taken away (USB, FireWire, card
    /// readers, discs): what the bar shows and toasts.
    pub removable: bool,
    pub can_power_off: bool,
    pub ejectable: bool,
    pub optical: bool,
    /// `usb`, `sdio`, `ieee1394`, or empty.
    pub bus: String,
    pub size: u64,
    pub volumes: Vec<Volume>,
}

impl Drive {
    /// Whether any of its filesystems is mounted.
    pub fn mounted(&self) -> bool {
        self.volumes.iter().any(|v| v.mounted().is_some())
    }
}

/// `GetManagedObjects`' reply: object → interface → property → value.
pub type Objects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;
type Props = HashMap<String, OwnedValue>;

/// The drives in `objects` that hold a filesystem worth showing, in udisks'
/// own order (`SortKey`), each with its filesystems in partition order.
///
/// Left out: anything udisks says to ignore (EFI and recovery partitions),
/// block devices on no drive (loop devices), swap and other non-filesystems,
/// and an internal disk's filesystems mounted outside the media directories
/// (`/`, `/boot`, `/nix/store`), which are the system, not a drive.
pub fn parse(objects: &Objects) -> Vec<Drive> {
    let mut drives: HashMap<&str, (String, Drive)> = HashMap::new();
    for (path, interfaces) in objects {
        let Some(drive) = interfaces.get(DRIVE) else {
            continue;
        };
        let name = [text(drive, "Vendor"), text(drive, "Model")]
            .iter()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let bus = text(drive, "ConnectionBus");
        drives.insert(
            path.as_str(),
            (
                text(drive, "SortKey"),
                Drive {
                    object: path.to_string(),
                    name: if name.is_empty() { "Drive".into() } else { name },
                    removable: flag(drive, "Removable") || flag(drive, "MediaRemovable"),
                    can_power_off: flag(drive, "CanPowerOff"),
                    ejectable: flag(drive, "Ejectable"),
                    optical: flag(drive, "Optical"),
                    bus,
                    size: number(drive, "Size"),
                    volumes: Vec::new(),
                },
            ),
        );
    }

    let block_of = |path: &str| objects.iter().find(|(p, _)| p.as_str() == path).and_then(|(_, i)| i.get(BLOCK));
    // Encrypted devices that are unlocked are shown as the filesystem
    // inside them.
    let unlocked: Vec<String> = objects
        .values()
        .filter_map(|i| i.get(BLOCK))
        .map(|b| object(b, "CryptoBackingDevice"))
        .filter(|b| b != "/")
        .collect();

    // Any order: each drive's filesystems are sorted below.
    for (path, interfaces) in objects {
        let Some(block) = interfaces.get(BLOCK) else {
            continue;
        };
        let encrypted = interfaces.contains_key(ENCRYPTED);
        let filesystem = interfaces.get(FILESYSTEM);
        if flag(block, "HintIgnore") || (filesystem.is_none() && !encrypted) {
            continue;
        }
        if encrypted && unlocked.iter().any(|b| b == path.as_str()) {
            continue;
        }
        let backing = Some(object(block, "CryptoBackingDevice")).filter(|b| b != "/");
        let backing_block = backing.as_deref().and_then(block_of);
        let drive_path = match backing_block {
            Some(outer) => object(outer, "Drive"),
            None => object(block, "Drive"),
        };
        let Some((_, drive)) = drives.get_mut(drive_path.as_str()) else {
            continue;
        };
        let mount_points: Vec<PathBuf> = filesystem.map(mount_points).unwrap_or_default();
        let system = flag(block, "HintSystem");
        if system && mount_points.iter().any(|m| !in_media(m)) {
            continue;
        }
        let mut label = [text(block, "HintName"), text(block, "IdLabel")]
            .into_iter()
            .find(|s| !s.trim().is_empty())
            .unwrap_or_default();
        if label.is_empty() {
            if let Some(outer) = backing_block {
                label = text(outer, "IdLabel");
            }
        }
        drive.volumes.push(Volume {
            object: path.to_string(),
            device: bytes_text(block, "PreferredDevice").or_else(|| bytes_text(block, "Device")).unwrap_or_default(),
            label,
            fs_type: text(block, "IdType"),
            size: number(block, "Size"),
            mount_points,
            read_only: flag(block, "ReadOnly"),
            system,
            locked: encrypted,
            backing,
        });
    }

    let mut drives: Vec<(String, Drive)> = drives.into_values().filter(|(_, d)| !d.volumes.is_empty()).collect();
    drives.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.object.cmp(&b.1.object)));
    drives
        .into_iter()
        .map(|(_, mut drive)| {
            drive.volumes.sort_by(|a, b| partition_order(&a.device, &b.device));
            drive
        })
        .collect()
}

/// `sdb2` before `sdb10`.
fn partition_order(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| {
        let digits = s.len() - s.trim_end_matches(|c: char| c.is_ascii_digit()).len();
        let (stem, number) = s.split_at(s.len() - digits);
        (stem.to_string(), number.parse::<u64>().unwrap_or(0))
    };
    split(a).cmp(&split(b))
}

/// Mounted where a desktop mounts drives, rather than as part of the system.
fn in_media(path: &Path) -> bool {
    ["/run/media/", "/media/", "/mnt/"].iter().any(|dir| path.starts_with(dir))
}

fn flag(props: &Props, key: &str) -> bool {
    props.get(key).and_then(|v| bool::try_from(v).ok()).unwrap_or(false)
}

fn number(props: &Props, key: &str) -> u64 {
    props.get(key).and_then(|v| u64::try_from(v).ok()).unwrap_or(0)
}

fn text(props: &Props, key: &str) -> String {
    props.get(key).and_then(|v| <&str>::try_from(v).ok()).unwrap_or_default().to_string()
}

fn object(props: &Props, key: &str) -> String {
    props
        .get(key)
        .and_then(|v| <&ObjectPath>::try_from(v).ok())
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| "/".into())
}

/// udisks writes paths as NUL-terminated byte arrays.
fn c_text(bytes: Vec<u8>) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

fn bytes_text(props: &Props, key: &str) -> Option<String> {
    let value = props.get(key)?.try_clone().ok()?;
    Vec::<u8>::try_from(value).ok().map(c_text).filter(|s| !s.is_empty())
}

fn mount_points(filesystem: &Props) -> Vec<PathBuf> {
    filesystem
        .get("MountPoints")
        .and_then(|v| v.try_clone().ok())
        .and_then(|v| Vec::<Vec<u8>>::try_from(v).ok())
        .unwrap_or_default()
        .into_iter()
        .map(c_text)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// `15.5 GB`, in powers of 1000 as drives are sold.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["kB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in UNITS {
        if value < 1000.0 {
            break;
        }
        value /= 1000.0;
        unit = next;
    }
    if value < 10.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.0} {unit}")
    }
}

/// The text of a udisks error, which is already a sentence
/// (`Not authorized to perform operation`).
fn message(err: zbus::Error) -> String {
    match err {
        zbus::Error::MethodError(name, text, _) => text.unwrap_or_else(|| name.to_string()),
        other => other.to_string(),
    }
}

/// The innermost reason of a udisks error, which chains them with ": "
/// (`Error unlocking /dev/sdb3: Failed to activate device: Incorrect
/// passphrase.` is "Incorrect passphrase."): the part worth showing where
/// there is room for one line.
fn last_reason(text: &str) -> &str {
    text.rsplit(": ").next().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(text)
}

fn is_error(err: &zbus::Error, suffix: &str) -> bool {
    matches!(err, zbus::Error::MethodError(name, _, _) if name.as_str().ends_with(suffix))
}

/// A connection to udisks on the system bus.
#[derive(Clone)]
pub struct Udisks {
    conn: Connection,
}

impl Udisks {
    /// Connect to the system bus. udisks itself starts on the first call.
    pub fn connect() -> Result<Self, String> {
        Connection::system().map(|conn| Self { conn }).map_err(message)
    }

    /// Whether udisks is already running, asked without starting it.
    pub fn running(&self) -> bool {
        self.conn
            .call_method(Some("org.freedesktop.DBus"), "/org/freedesktop/DBus", Some("org.freedesktop.DBus"), "NameHasOwner", &(SERVICE,))
            .and_then(|reply| reply.body().deserialize::<bool>())
            .unwrap_or(false)
    }

    /// Call a udisks method. Plain method calls only: a `Proxy` would bring
    /// zbus's property cache and signal streams into every binary, which
    /// cost the file manager ~200 KB.
    fn call<B, R>(&self, path: &str, interface: &str, method: &str, body: &B) -> Result<R, zbus::Error>
    where
        B: serde::Serialize + zbus::zvariant::DynamicType,
        R: for<'d> serde::Deserialize<'d> + zbus::zvariant::Type,
    {
        self.conn.call_method(Some(SERVICE), path, Some(interface), method, body)?.body().deserialize()
    }

    /// A method taking only udisks' options dictionary, left empty.
    fn call_with_options(&self, path: &str, interface: &str, method: &str) -> Result<(), zbus::Error> {
        let options: HashMap<&str, Value> = HashMap::new();
        self.conn.call_method(Some(SERVICE), path, Some(interface), method, &(options,)).map(drop)
    }

    /// Every drive worth showing ([`parse`]).
    pub fn drives(&self) -> Result<Vec<Drive>, String> {
        let objects: Objects = self.call(ROOT, "org.freedesktop.DBus.ObjectManager", "GetManagedObjects", &()).map_err(message)?;
        Ok(parse(&objects))
    }

    /// Mount a filesystem and return where. One already mounted is not an
    /// error. udisks may ask polkit first, which can wait for a password:
    /// call this off the UI thread.
    pub fn mount(&self, volume: &str) -> Result<PathBuf, String> {
        let options: HashMap<&str, Value> = HashMap::new();
        match self.call::<_, String>(volume, FILESYSTEM, "Mount", &(options,)) {
            Ok(path) => Ok(PathBuf::from(path)),
            Err(err) if is_error(&err, ".AlreadyMounted") => {
                let points: OwnedValue = self
                    .call(volume, "org.freedesktop.DBus.Properties", "Get", &(FILESYSTEM, "MountPoints"))
                    .map_err(message)?;
                let points = Vec::<Vec<u8>>::try_from(points).unwrap_or_default();
                points.into_iter().map(c_text).map(PathBuf::from).next().ok_or_else(|| message(err))
            }
            Err(err) => Err(message(err)),
        }
    }

    /// Unlock an encrypted volume with `passphrase`, returning the object of
    /// the filesystem inside, which [`Udisks::mount`] then takes. The
    /// passphrase goes to udisksd over the system bus and nowhere else.
    pub fn unlock(&self, volume: &str, passphrase: &str) -> Result<String, String> {
        let options: HashMap<&str, Value> = HashMap::new();
        self.call::<_, OwnedObjectPath>(volume, ENCRYPTED, "Unlock", &(passphrase, options))
            .map(|path| path.to_string())
            .map_err(|err| last_reason(&message(err)).to_string())
    }

    /// Unmount a filesystem; one not mounted is not an error.
    pub fn unmount(&self, volume: &str) -> Result<(), String> {
        match self.call_with_options(volume, FILESYSTEM, "Unmount") {
            Err(err) if !is_error(&err, ".NotMounted") => Err(message(err)),
            _ => Ok(()),
        }
    }

    /// Make a drive safe to take out: unmount its filesystems, lock what
    /// was unlocked, then power it off or eject its media where it can.
    pub fn eject(&self, drive: &str) -> Result<(), String> {
        let drives = self.drives()?;
        let Some(found) = drives.iter().find(|d| d.object == drive) else {
            return Err("The drive is gone".into());
        };
        for volume in &found.volumes {
            if volume.mounted().is_some() {
                self.unmount(&volume.object)?;
            }
            if let Some(backing) = &volume.backing {
                self.call_with_options(backing, ENCRYPTED, "Lock").map_err(message)?;
            }
        }
        if found.can_power_off {
            self.call_with_options(drive, DRIVE, "PowerOff").map_err(message)?;
        } else if found.ejectable {
            self.call_with_options(drive, DRIVE, "Eject").map_err(message)?;
        }
        Ok(())
    }

    /// Call `changed` with the drives each time they change, until it
    /// returns false. udisks sends a burst of signals for one plug, so a
    /// change is read once they have been quiet for `settle`.
    pub fn watch(&self, settle: Duration, mut changed: impl FnMut(Vec<Drive>) -> bool) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        let rules = [
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender(SERVICE)
                .and_then(|r| r.path_namespace(ROOT))
                .map(|r| r.build())
                .map_err(message)?,
            // udisks restarting: everything may have changed.
            zbus::MatchRule::builder()
                .msg_type(zbus::message::Type::Signal)
                .sender("org.freedesktop.DBus")
                .and_then(|r| r.member("NameOwnerChanged"))
                .and_then(|r| r.add_arg(SERVICE))
                .map(|r| r.build())
                .map_err(message)?,
        ];
        for rule in rules {
            let messages = MessageIterator::for_match_rule(rule, &self.conn, Some(256)).map_err(message)?;
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("stratum-udisks".into())
                .stack_size(64 * 1024)
                .spawn(move || {
                    for _ in messages {
                        if tx.send(()).is_err() {
                            break;
                        }
                    }
                })
                .map_err(|err| err.to_string())?;
        }
        drop(tx);
        while rx.recv().is_ok() {
            while rx.recv_timeout(settle).is_ok() {}
            match self.drives() {
                Ok(drives) => {
                    if !changed(drives) {
                        break;
                    }
                }
                Err(err) => tracing::warn!("udisks: {err}"),
            }
        }
        Ok(())
    }
}

/// Block device names that are never a drive.
fn virtual_block(name: &str) -> bool {
    ["loop", "ram", "zram", "dm-", "md", "nbd"].iter().any(|p| name.starts_with(p))
}

/// Whether a block device's sysfs path (`/sys/devices/…/block/sdb`) is on
/// a bus a drive can be unplugged from.
fn hotplug_path(path: &str) -> bool {
    ["/usb", "/mmc_host/", "/memstick", "/firewire", "/thunderbolt"].iter().any(|bus| path.contains(bus))
}

/// Whether the block device `name` could be a removable drive.
fn hotplug_block(name: &str) -> bool {
    if virtual_block(name) {
        return false;
    }
    let sys = Path::new("/sys/class/block").join(name);
    let removable = std::fs::read_to_string(sys.join("removable")).is_ok_and(|s| s.trim() == "1");
    removable || std::fs::canonicalize(&sys).is_ok_and(|p| hotplug_path(&p.to_string_lossy()))
}

/// Whether a disk that can come and go is attached now.
pub fn hotplug_present() -> bool {
    std::fs::read_dir("/sys/block")
        .map(|dir| dir.flatten().any(|e| hotplug_block(&e.file_name().to_string_lossy())))
        .unwrap_or(false)
}

/// Return once there is reason to talk to udisks: at once if `already`
/// says so, else when a removable disk appears in `/dev`. The watch is set
/// up before `already` is asked, so a disk plugged in between is not
/// missed. On an error the caller should go ahead rather than never start.
pub fn wait_for_disk(already: impl FnOnce() -> bool) -> std::io::Result<()> {
    // SAFETY: plain syscalls; the descriptor is closed by `Fd`'s drop.
    let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    struct Fd(i32);
    impl Drop for Fd {
        fn drop(&mut self) {
            // SAFETY: closing our own descriptor once.
            unsafe { libc::close(self.0) };
        }
    }
    let fd = Fd(fd);
    // SAFETY: a NUL-terminated path.
    if unsafe { libc::inotify_add_watch(fd.0, c"/dev".as_ptr(), libc::IN_CREATE) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if already() {
        return Ok(());
    }
    let mut buffer = [0u8; 4096];
    loop {
        // SAFETY: reading into our own buffer.
        let read = unsafe { libc::read(fd.0, buffer.as_mut_ptr().cast(), buffer.len()) };
        if read < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        if event_names(&buffer[..read as usize]).iter().any(|name| hotplug_block(name)) {
            return Ok(());
        }
    }
}

/// The names in a buffer of `inotify_event`s.
fn event_names(mut buffer: &[u8]) -> Vec<String> {
    const HEADER: usize = 16; // wd, mask, cookie, len
    let mut names = Vec::new();
    while buffer.len() >= HEADER {
        let len = u32::from_ne_bytes([buffer[12], buffer[13], buffer[14], buffer[15]]) as usize;
        let Some(name) = buffer.get(HEADER..HEADER + len) else {
            break;
        };
        names.push(c_text(name.to_vec()));
        buffer = &buffer[HEADER + len..];
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn val<'a>(v: impl Into<Value<'a>>) -> OwnedValue {
        OwnedValue::try_from(v.into()).unwrap()
    }

    fn path(p: &'static str) -> OwnedValue {
        val(ObjectPath::try_from(p).unwrap())
    }

    fn cstr(s: &str) -> Vec<u8> {
        let mut bytes = s.as_bytes().to_vec();
        bytes.push(0);
        bytes
    }

    fn props(entries: Vec<(&str, OwnedValue)>) -> Props {
        entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    }

    fn block(drive: &'static str, device: &str, label: &str, fs: &str, system: bool) -> Props {
        props(vec![
            ("Drive", path(drive)),
            ("Device", val(cstr(device))),
            ("IdLabel", val(label)),
            ("IdType", val(fs)),
            ("Size", val(16_000_000_000u64)),
            ("HintSystem", val(system)),
            ("CryptoBackingDevice", path("/")),
        ])
    }

    fn filesystem(mounts: &[&str]) -> Props {
        props(vec![("MountPoints", val(mounts.iter().map(|m| cstr(m)).collect::<Vec<_>>()))])
    }

    fn drive(model: &str, removable: bool, sort: &str) -> Props {
        props(vec![
            ("Vendor", val("SanDisk ")),
            ("Model", val(model)),
            ("Removable", val(removable)),
            ("CanPowerOff", val(removable)),
            ("ConnectionBus", val(if removable { "usb" } else { "" })),
            ("SortKey", val(sort)),
        ])
    }

    fn objects(list: Vec<(&'static str, Vec<(&str, Props)>)>) -> Objects {
        list.into_iter()
            .map(|(p, ifaces)| {
                (
                    OwnedObjectPath::try_from(p).unwrap(),
                    ifaces.into_iter().map(|(i, props)| (i.to_string(), props)).collect(),
                )
            })
            .collect()
    }

    const D: &str = "/org/freedesktop/UDisks2/drives/";
    const B: &str = "/org/freedesktop/UDisks2/block_devices/";

    #[test]
    fn a_usb_stick_and_an_internal_disk() {
        let objs = objects(vec![
            ("/org/freedesktop/UDisks2/drives/stick", vec![(DRIVE, drive("Cruzer", true, "01hotplug"))]),
            ("/org/freedesktop/UDisks2/drives/nvme", vec![(DRIVE, drive("SSD", false, "00coldplug"))]),
            (
                "/org/freedesktop/UDisks2/block_devices/sdb10",
                vec![(BLOCK, block("/org/freedesktop/UDisks2/drives/stick", "/dev/sdb10", "B", "vfat", false)), (FILESYSTEM, filesystem(&[]))],
            ),
            (
                "/org/freedesktop/UDisks2/block_devices/sdb2",
                vec![
                    (BLOCK, block("/org/freedesktop/UDisks2/drives/stick", "/dev/sdb2", "STICK", "exfat", false)),
                    (FILESYSTEM, filesystem(&["/run/media/me/STICK"])),
                ],
            ),
            // The system's own filesystems are not drives.
            (
                "/org/freedesktop/UDisks2/block_devices/nvme0n1p2",
                vec![(BLOCK, block("/org/freedesktop/UDisks2/drives/nvme", "/dev/nvme0n1p2", "", "btrfs", true)), (FILESYSTEM, filesystem(&["/", "/nix/store"]))],
            ),
            // An unmounted Windows partition is.
            (
                "/org/freedesktop/UDisks2/block_devices/nvme0n1p3",
                vec![(BLOCK, block("/org/freedesktop/UDisks2/drives/nvme", "/dev/nvme0n1p3", "Windows", "ntfs", true)), (FILESYSTEM, filesystem(&[]))],
            ),
            // Swap has no Filesystem interface; a loop device has no drive.
            ("/org/freedesktop/UDisks2/block_devices/nvme0n1p4", vec![(BLOCK, block("/org/freedesktop/UDisks2/drives/nvme", "/dev/nvme0n1p4", "", "swap", true))]),
            ("/org/freedesktop/UDisks2/block_devices/loop0", vec![(BLOCK, block("/", "/dev/loop0", "", "squashfs", false)), (FILESYSTEM, filesystem(&[]))]),
        ]);
        let drives = parse(&objs);
        assert_eq!(drives.len(), 2);
        assert_eq!(drives[0].name, "SanDisk SSD", "udisks' sort key orders the drives");
        assert!(!drives[0].removable);
        assert_eq!(drives[0].volumes.len(), 1);
        assert_eq!(drives[0].volumes[0].label, "Windows");
        assert!(drives[0].volumes[0].system);

        let stick = &drives[1];
        assert_eq!(stick.object, format!("{D}stick"));
        assert!(stick.removable && stick.can_power_off && stick.mounted());
        let devices: Vec<_> = stick.volumes.iter().map(|v| v.device.as_str()).collect();
        assert_eq!(devices, ["/dev/sdb2", "/dev/sdb10"], "partitions in number order");
        assert_eq!(stick.volumes[0].mounted(), Some(Path::new("/run/media/me/STICK")));
        assert_eq!(stick.volumes[0].object, format!("{B}sdb2"));
        assert_eq!(stick.volumes[1].name(), "B");
    }

    #[test]
    fn ignored_partitions_and_unlocked_encryption() {
        let stick = "/org/freedesktop/UDisks2/drives/stick";
        let mut efi = block(stick, "/dev/sdb1", "EFI", "vfat", false);
        efi.insert("HintIgnore".into(), val(true));
        let mut outer = block(stick, "/dev/sdb2", "Secret", "crypto_LUKS", false);
        outer.insert("IdUsage".into(), val("crypto"));
        let mut inner = block("/", "/dev/dm-0", "", "ext4", false);
        inner.insert("CryptoBackingDevice".into(), path("/org/freedesktop/UDisks2/block_devices/sdb2"));
        let locked = block(stick, "/dev/sdb3", "", "crypto_LUKS", false);
        let objs = objects(vec![
            (stick, vec![(DRIVE, drive("Cruzer", true, "a"))]),
            ("/org/freedesktop/UDisks2/block_devices/sdb1", vec![(BLOCK, efi), (FILESYSTEM, filesystem(&[]))]),
            ("/org/freedesktop/UDisks2/block_devices/sdb2", vec![(BLOCK, outer), (ENCRYPTED, Props::new())]),
            ("/org/freedesktop/UDisks2/block_devices/dm_2d0", vec![(BLOCK, inner), (FILESYSTEM, filesystem(&["/run/media/me/x"]))]),
            ("/org/freedesktop/UDisks2/block_devices/sdb3", vec![(BLOCK, locked), (ENCRYPTED, Props::new())]),
        ]);
        let drives = parse(&objs);
        let volumes = &drives[0].volumes;
        assert_eq!(volumes.len(), 2, "{volumes:#?}");
        let open = volumes.iter().find(|v| v.device == "/dev/dm-0").unwrap();
        assert!(!open.locked);
        assert_eq!(open.label, "Secret", "named after the encrypted partition");
        assert_eq!(open.backing.as_deref(), Some("/org/freedesktop/UDisks2/block_devices/sdb2"));
        let shut = volumes.iter().find(|v| v.device == "/dev/sdb3").unwrap();
        assert!(shut.locked && shut.backing.is_none());
        assert_eq!(shut.name(), "16 GB Volume");
    }

    #[test]
    fn sizes_read_as_drives_are_sold() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(15_518_924_800), "16 GB");
        assert_eq!(format_size(7_751_073_792), "7.8 GB");
        assert_eq!(format_size(2_000_398_934_016), "2.0 TB");
    }

    #[test]
    fn errors_keep_their_innermost_reason() {
        assert_eq!(last_reason("Error unlocking /dev/sdb3: Failed to activate device: Incorrect passphrase."), "Incorrect passphrase.");
        assert_eq!(last_reason("Not authorized"), "Not authorized");
        assert_eq!(last_reason("trailing: "), "trailing: ");
    }

    #[test]
    fn hotplug_buses() {
        assert!(hotplug_path("/sys/devices/pci0000:00/0000:00:14.0/usb2/2-1/2-1:1.0/host0/target0:0:0/0:0:0:0/block/sdb"));
        assert!(hotplug_path("/sys/devices/pci0000:00/0000:00:1c.0/0000:02:00.0/rtsx_pci_sdmmc.0/mmc_host/mmc0/mmc0:aaaa/block/mmcblk0"));
        assert!(!hotplug_path("/sys/devices/pci0000:00/0000:00:01.2/0000:02:00.0/nvme/nvme0/nvme0n1"));
        assert!(virtual_block("loop7") && virtual_block("dm-0") && virtual_block("zram0"));
        assert!(!virtual_block("sdb"));
    }

    #[test]
    fn inotify_names() {
        let mut buffer = Vec::new();
        for name in ["sdb", "sdb1"] {
            buffer.extend_from_slice(&1i32.to_ne_bytes());
            buffer.extend_from_slice(&libc::IN_CREATE.to_ne_bytes());
            buffer.extend_from_slice(&0u32.to_ne_bytes());
            buffer.extend_from_slice(&8u32.to_ne_bytes());
            let mut padded = name.as_bytes().to_vec();
            padded.resize(8, 0);
            buffer.extend_from_slice(&padded);
        }
        assert_eq!(event_names(&buffer), ["sdb", "sdb1"]);
    }
}
