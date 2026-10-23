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
use std::sync::{LazyLock, Mutex};

use crate::managers::common::run_capture_optional;

/// GPU names by DRM card; a card's name never changes.
static GPU_NAMES: LazyLock<Mutex<HashMap<PathBuf, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// One GPU's load, temperature and memory.
#[derive(Clone, Debug, PartialEq)]
pub struct GpuReading {
    pub name: String,
    pub load: Load,
    /// °C, when the driver reports it.
    pub temperature: Option<f32>,
    /// Used and total VRAM in bytes.
    pub vram: Option<(u64, u64)>,
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

/// The load, temperature and VRAM of every GPU present. A suspended card is
/// left alone: all three are unknown.
pub(crate) fn read_all() -> Vec<GpuReading> {
    gpu_cards()
        .iter()
        .map(|card| {
            let vendor = fs::read_to_string(card.join("device/vendor")).unwrap_or_default();
            let vendor = vendor.trim();
            let mut reading = NvmlReading::default();
            let load = if is_suspended(card) {
                Load::Asleep
            } else if let Some(percent) = read_percent(&card.join("device/gpu_busy_percent"))
                .or_else(|| read_percent(&card.join("gt_busy_percent")))
            {
                reading.temperature = hwmon_temperature(card);
                reading.vram = sysfs_vram(card);
                Load::Percent(percent)
            } else if vendor == NVIDIA {
                match pci_slot(card).and_then(|slot| nvml_read(&slot)) {
                    Some(nvml) => {
                        reading = nvml;
                        nvml.load.map_or(Load::Unknown, Load::Percent)
                    }
                    None => Load::Unknown,
                }
            } else {
                Load::Unknown
            };
            let fallback = match vendor {
                NVIDIA => "NVIDIA",
                "0x1002" => "AMD",
                "0x8086" => "Intel",
                _ => "GPU",
            };
            GpuReading {
                name: gpu_name(card).unwrap_or_else(|| fallback.to_string()),
                load,
                temperature: reading.temperature,
                vram: reading.vram,
            }
        })
        .collect()
}

/// The card's own temperature sensor (amdgpu's `edge`), in °C.
fn hwmon_temperature(card: &Path) -> Option<f32> {
    fs::read_dir(card.join("device/hwmon"))
        .ok()?
        .flatten()
        .find_map(|hwmon| fs::read_to_string(hwmon.path().join("temp1_input")).ok())
        .and_then(|milli| milli.trim().parse::<f32>().ok())
        .map(|milli| milli / 1000.0)
}

/// Used and total VRAM in bytes, as amdgpu reports it. An APU's "VRAM" is
/// the carve-out from system memory.
fn sysfs_vram(card: &Path) -> Option<(u64, u64)> {
    let read = |name: &str| fs::read_to_string(card.join("device").join(name)).ok()?.trim().parse::<u64>().ok();
    Some((read("mem_info_vram_used")?, read("mem_info_vram_total")?)).filter(|&(_, total)| total > 0)
}

type NvmlCall = unsafe extern "C" fn() -> c_int;
type HandleByPci = unsafe extern "C" fn(*const c_char, *mut *mut c_void) -> c_int;
type Utilization = unsafe extern "C" fn(*mut c_void, *mut NvmlUtilization) -> c_int;
type Temperature = unsafe extern "C" fn(*mut c_void, c_int, *mut c_uint) -> c_int;
type MemoryInfo = unsafe extern "C" fn(*mut c_void, *mut NvmlMemory) -> c_int;

/// The NVML entry points used here. Temperature and memory are optional, so
/// a driver without them still reports load.
struct Nvml {
    /// The `dlopen` handle, closed by [`release`].
    handle: *mut c_void,
    init: NvmlCall,
    shutdown: NvmlCall,
    handle_by_pci: HandleByPci,
    utilization: Utilization,
    temperature: Option<Temperature>,
    memory: Option<MemoryInfo>,
}

#[repr(C)]
#[derive(Default)]
struct NvmlUtilization {
    gpu: c_uint,
    memory: c_uint,
}

#[repr(C)]
#[derive(Default)]
struct NvmlMemory {
    total: u64,
    free: u64,
    used: u64,
}

const NVML_SUCCESS: c_int = 0;
/// `NVML_TEMPERATURE_GPU`: the die.
const NVML_TEMPERATURE_GPU: c_int = 0;

/// What one NVML session (or amdgpu's sysfs) reports for a card.
#[derive(Clone, Copy, Debug, Default)]
struct NvmlReading {
    load: Option<i64>,
    temperature: Option<f32>,
    vram: Option<(u64, u64)>,
}

// SAFETY: the handle and function pointers are process-wide and NVML's
// calls are thread-safe; the mutex below serialises every use anyway.
unsafe impl Send for Nvml {}

/// NVML while it is loaded: from the first NVIDIA reading until
/// [`release`]. Loaded, it keeps ~15 MB of zeroed statics resident.
static NVML: Mutex<Option<Nvml>> = Mutex::new(None);

/// Unload NVML until the next reading, when nothing samples GPUs (the
/// dashboard closed). Its CUDA helper thread stays, with `libcuda`: that is
/// ~1.5 MB, and unloading a library a thread runs in is not safe.
pub fn release() {
    let mut nvml = NVML.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(nvml) = nvml.take() {
        // SAFETY: every NVML session was shut down before its call returned,
        // and the mutex keeps any new one from starting during the close.
        unsafe {
            libc::dlclose(nvml.handle);
        }
    }
}

/// Load NVML. `None` without the NVIDIA driver.
fn load_nvml() -> Option<Nvml> {
    // The NixOS driver directory is not on the default search path of a
    // process started without the session's LD_LIBRARY_PATH.
    let handle = ["libnvidia-ml.so.1", "/run/opengl-driver/lib/libnvidia-ml.so.1"].iter().find_map(|name| {
        let name = CString::new(*name).ok()?;
        // SAFETY: dlopen on a NUL-terminated name; the handle is kept until
        // `release`.
        let handle = unsafe { libc::dlopen(name.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        (!handle.is_null()).then_some(handle)
    })?;
    // SAFETY: a live handle from dlopen above.
    let nvml = unsafe { nvml_symbols(handle) };
    if nvml.is_none() {
        // SAFETY: a handle this function opened and nothing else uses.
        unsafe {
            libc::dlclose(handle);
        }
    }
    nvml
}

/// NVML's entry points in `handle`; `None` when a required one is missing.
///
/// # Safety
/// `handle` must be a live `dlopen` handle on libnvidia-ml.
unsafe fn nvml_symbols(handle: *mut c_void) -> Option<Nvml> {
    let symbol = |name: &str| -> Option<*mut c_void> {
        let name = CString::new(name).ok()?;
        // SAFETY: dlsym on a live handle and a NUL-terminated name.
        let ptr = unsafe { libc::dlsym(handle, name.as_ptr()) };
        (!ptr.is_null()).then_some(ptr)
    };
    // SAFETY: each pointer is NVML's exported function of that name, whose C
    // signature the field's type matches (nvml.h).
    unsafe {
        Some(Nvml {
            handle,
            init: std::mem::transmute::<*mut c_void, NvmlCall>(symbol("nvmlInit_v2")?),
            shutdown: std::mem::transmute::<*mut c_void, NvmlCall>(symbol("nvmlShutdown")?),
            handle_by_pci: std::mem::transmute::<*mut c_void, HandleByPci>(symbol("nvmlDeviceGetHandleByPciBusId_v2")?),
            utilization: std::mem::transmute::<*mut c_void, Utilization>(symbol("nvmlDeviceGetUtilizationRates")?),
            temperature: symbol("nvmlDeviceGetTemperature").map(|p| std::mem::transmute::<*mut c_void, Temperature>(p)),
            memory: symbol("nvmlDeviceGetMemoryInfo").map(|p| std::mem::transmute::<*mut c_void, MemoryInfo>(p)),
        })
    }
}

/// The NVIDIA GPU at PCI address `slot`: load, temperature and memory. NVML
/// is initialised for the one query only, so the shell holds no handle that
/// could keep the GPU from suspending.
fn nvml_read(slot: &str) -> Option<NvmlReading> {
    let mut loaded = NVML.lock().unwrap_or_else(|p| p.into_inner());
    if loaded.is_none() {
        *loaded = load_nvml();
    }
    let nvml = loaded.as_ref()?;
    let slot = CString::new(slot).ok()?;
    // SAFETY: init and shutdown are paired (NVML counts them); the device
    // handle is used only between them, and every out-pointer is valid.
    unsafe {
        if (nvml.init)() != NVML_SUCCESS {
            return None;
        }
        let mut device = std::ptr::null_mut();
        let mut reading = NvmlReading::default();
        if (nvml.handle_by_pci)(slot.as_ptr(), &mut device) == NVML_SUCCESS {
            let mut usage = NvmlUtilization::default();
            if (nvml.utilization)(device, &mut usage) == NVML_SUCCESS {
                reading.load = Some(i64::from(usage.gpu));
            }
            let mut celsius: c_uint = 0;
            if nvml.temperature.is_some_and(|f| f(device, NVML_TEMPERATURE_GPU, &mut celsius) == NVML_SUCCESS) {
                reading.temperature = Some(celsius as f32);
            }
            let mut memory = NvmlMemory::default();
            if nvml.memory.is_some_and(|f| f(device, &mut memory) == NVML_SUCCESS) && memory.total > 0 {
                reading.vram = Some((memory.used, memory.total));
            }
        }
        (nvml.shutdown)();
        Some(reading)
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
