//! GPU load for the dashboard, one reading per GPU that is present.
//!
//! A GPU switched off in firmware (integrated-only mode) has no DRM card, so
//! it gets no row. A card that is present but runtime-suspended is reported
//! as asleep without being touched: querying it would wake it, and the
//! dashboard polls every couple of seconds.
//!
//! AMD reports its load in sysfs (`gpu_busy_percent`), older Intel in
//! `gt_busy_percent`. NVIDIA reports it only through NVML, which is loaded
//! with `dlopen`: `nvidia-smi` is NVML's own client, but it has crashed in
//! the dynamic loader before `main` on NixOS (595.104.02 with glibc 2.44),
//! so the shell does not depend on it.

use std::collections::HashMap;
use std::ffi::{c_char, c_int, c_uint, c_void, CString};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use lazy_static::lazy_static;

use crate::managers::common::run_capture_optional;

lazy_static! {
    /// GPU names by DRM card; a card's name never changes.
    static ref GPU_NAMES: Mutex<HashMap<PathBuf, Option<String>>> = Mutex::new(HashMap::new());
}

/// One GPU's load.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuReading {
    pub name: String,
    pub load: Load,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Load {
    Percent(i64),
    /// Runtime-suspended: idle, and left alone.
    Asleep,
    /// Present, but its driver exposes no load we can read.
    Unknown,
}

/// The fields of one `lspci -mm` line: quoted strings and bare words.
fn lspci_fields(line: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut rest = line.trim();
    while !rest.is_empty() {
        if let Some(quoted) = rest.strip_prefix('"') {
            let end = quoted.find('"').unwrap_or(quoted.len());
            fields.push(&quoted[..end]);
            rest = quoted.get(end + 1..).unwrap_or("").trim_start();
        } else {
            let end = rest.find(' ').unwrap_or(rest.len());
            fields.push(&rest[..end]);
            rest = rest[end..].trim_start();
        }
    }
    fields
}

/// The marketing name in a PCI device name: "Rembrandt [Radeon 680M]" is
/// "Radeon 680M". A name without brackets is kept whole. Of alternatives
/// ("GeForce RTX 4070 Max-Q / Mobile"), the first is kept, so it fits a
/// caption.
fn short_pci_name(device: &str) -> String {
    let name = match (device.rfind('['), device.rfind(']')) {
        (Some(open), Some(close)) if open < close => &device[open + 1..close],
        _ => device,
    };
    name.split(" / ").next().unwrap_or(name).trim().to_string()
}

/// The PCI address of `card`'s device, e.g. `0000:06:00.0`.
fn pci_slot(card: &Path) -> Option<String> {
    let device = fs::canonicalize(card.join("device")).ok()?;
    Some(device.file_name()?.to_str()?.to_string())
}

/// The name of the GPU behind `card`, from lspci's PCI ID database. The
/// kernel exposes only numeric ids.
fn gpu_name(card: &Path) -> Option<String> {
    let mut names = GPU_NAMES.lock().ok()?;
    names
        .entry(card.to_path_buf())
        .or_insert_with(|| {
            let slot = pci_slot(card)?;
            let line = run_capture_optional("lspci", &["-mm", "-s", &slot]);
            // Slot, class, vendor, device.
            let device_name = *lspci_fields(line.lines().next()?).get(3)?;
            Some(short_pci_name(device_name)).filter(|name| !name.is_empty())
        })
        .clone()
}

/// DRM cards backed by a PCI GPU, in card order. Connectors
/// (`card1-HDMI-A-1`) and firmware framebuffers (`simpledrm`, no PCI vendor)
/// are left out.
fn gpu_cards() -> Vec<PathBuf> {
    let mut cards = fs::read_dir("/sys/class/drm")
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("card") && !name.contains('-'))
                        && path.join("device/vendor").exists()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    cards.sort();
    cards
}

fn read_percent(path: &Path) -> Option<i64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn is_suspended(card: &Path) -> bool {
    fs::read_to_string(card.join("device/power/runtime_status")).is_ok_and(|s| s.trim() == "suspended")
}

const NVIDIA: &str = "0x10de";

/// The load of every GPU present.
pub fn read_all() -> Vec<GpuReading> {
    gpu_cards()
        .iter()
        .map(|card| {
            let vendor = fs::read_to_string(card.join("device/vendor")).unwrap_or_default();
            let vendor = vendor.trim();
            let load = if is_suspended(card) {
                Load::Asleep
            } else if let Some(percent) = read_percent(&card.join("device/gpu_busy_percent"))
                .or_else(|| read_percent(&card.join("gt_busy_percent")))
            {
                Load::Percent(percent)
            } else if vendor == NVIDIA {
                pci_slot(card).and_then(|slot| nvml_load(&slot)).map_or(Load::Unknown, Load::Percent)
            } else {
                Load::Unknown
            };
            let fallback = match vendor {
                NVIDIA => "NVIDIA",
                "0x1002" => "AMD",
                "0x8086" => "Intel",
                _ => "GPU",
            };
            GpuReading { name: gpu_name(card).unwrap_or_else(|| fallback.to_string()), load }
        })
        .collect()
}

type NvmlCall = unsafe extern "C" fn() -> c_int;
type HandleByPci = unsafe extern "C" fn(*const c_char, *mut *mut c_void) -> c_int;
type Utilization = unsafe extern "C" fn(*mut c_void, *mut NvmlUtilization) -> c_int;

/// The NVML entry points used here.
struct Nvml {
    init: NvmlCall,
    shutdown: NvmlCall,
    handle_by_pci: HandleByPci,
    utilization: Utilization,
}

#[repr(C)]
#[derive(Default)]
struct NvmlUtilization {
    gpu: c_uint,
    memory: c_uint,
}

const NVML_SUCCESS: c_int = 0;

/// NVML, loaded once and never unloaded. `None` without the NVIDIA driver.
fn nvml() -> Option<&'static Nvml> {
    static NVML: OnceLock<Option<Nvml>> = OnceLock::new();
    NVML.get_or_init(|| {
        // The NixOS driver directory is not on the default search path of a
        // process started without the session's LD_LIBRARY_PATH.
        let handle = ["libnvidia-ml.so.1", "/run/opengl-driver/lib/libnvidia-ml.so.1"]
            .iter()
            .find_map(|name| {
                let name = CString::new(*name).ok()?;
                // SAFETY: dlopen on a NUL-terminated name; the handle is kept
                // for the life of the process.
                let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
                (!handle.is_null()).then_some(handle)
            })?;
        let symbol = |name: &str| -> Option<*mut c_void> {
            let name = CString::new(name).ok()?;
            // SAFETY: dlsym on a live handle and a NUL-terminated name.
            let ptr = unsafe { libc::dlsym(handle, name.as_ptr()) };
            (!ptr.is_null()).then_some(ptr)
        };
        // SAFETY: each pointer is NVML's exported function of that name,
        // whose C signature the field's type matches (nvml.h).
        unsafe {
            Some(Nvml {
                init: std::mem::transmute::<*mut c_void, NvmlCall>(symbol("nvmlInit_v2")?),
                shutdown: std::mem::transmute::<*mut c_void, NvmlCall>(symbol("nvmlShutdown")?),
                handle_by_pci: std::mem::transmute::<*mut c_void, HandleByPci>(symbol(
                    "nvmlDeviceGetHandleByPciBusId_v2",
                )?),
                utilization: std::mem::transmute::<*mut c_void, Utilization>(symbol(
                    "nvmlDeviceGetUtilizationRates",
                )?),
            })
        }
    })
    .as_ref()
}

/// The NVIDIA GPU at PCI address `slot`'s load. NVML is initialised for the
/// one query only, so the shell holds no handle that could keep the GPU from
/// suspending.
fn nvml_load(slot: &str) -> Option<i64> {
    let nvml = nvml()?;
    let slot = CString::new(slot).ok()?;
    // SAFETY: init and shutdown are paired (NVML counts them); the device
    // handle is used only between them, and both out-pointers are valid.
    unsafe {
        if (nvml.init)() != NVML_SUCCESS {
            return None;
        }
        let mut device = std::ptr::null_mut();
        let mut usage = NvmlUtilization::default();
        let ok = (nvml.handle_by_pci)(slot.as_ptr(), &mut device) == NVML_SUCCESS
            && (nvml.utilization)(device, &mut usage) == NVML_SUCCESS;
        (nvml.shutdown)();
        ok.then_some(i64::from(usage.gpu))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lspci_fields_split_quoted_and_bare_words() {
        let line = r#"06:00.0 "VGA compatible controller" "Advanced Micro Devices, Inc. [AMD/ATI]" "Rembrandt [Radeon 680M]" -r0a -p00 "Lenovo" "Device 3d2f""#;
        let fields = lspci_fields(line);
        assert_eq!(fields[0], "06:00.0");
        assert_eq!(fields[3], "Rembrandt [Radeon 680M]");
        assert_eq!(fields[4], "-r0a");
        assert_eq!(fields[7], "Device 3d2f");
    }

    #[test]
    fn short_pci_name_keeps_the_bracketed_name() {
        assert_eq!(short_pci_name("Rembrandt [Radeon 680M]"), "Radeon 680M");
        assert_eq!(short_pci_name("AD106M [GeForce RTX 4070 Max-Q / Mobile]"), "GeForce RTX 4070 Max-Q");
        assert_eq!(short_pci_name("Device 2860"), "Device 2860");
    }

    #[test]
    fn gpu_cards_are_pci_cards_only() {
        for card in gpu_cards() {
            assert!(!card.to_string_lossy().contains('-'), "{card:?} is a connector");
            assert!(card.join("device/vendor").exists());
        }
    }
}
