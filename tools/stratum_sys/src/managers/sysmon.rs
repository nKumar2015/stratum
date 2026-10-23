//! Resource readings for the dashboard: CPU, memory, the root filesystem and
//! every GPU, as typed values.
//!
//! [`Sampler`] keeps the previous `/proc/stat` totals, so CPU load is the
//! share of time busy since the last sample. The older
//! `dashboard::performance_payload` slept 200 ms between two reads of its own
//! on every call instead, and ran `df` for the disk; this reads `statvfs`.

use std::ffi::CString;
use std::fs;

use super::gpu::{self, GpuReading};

/// One reading of everything the dashboard's system panel shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Busy share of all CPUs since the previous sample, 0–100. `None` on a
    /// sampler's first reading, which has nothing to compare against.
    pub cpu_percent: Option<f32>,
    pub memory: Usage,
    /// The filesystem mounted at `/`.
    pub root: Usage,
    /// One per GPU present, in card order; empty when not asked for.
    pub gpus: Vec<GpuReading>,
}

/// Used and total bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Usage {
    pub used: u64,
    pub total: u64,
}

impl Usage {
    /// Used share, 0–100; 0 when the total is unknown.
    pub fn percent(&self) -> f32 {
        if self.total == 0 {
            0.0
        } else {
            (self.used as f64 * 100.0 / self.total as f64) as f32
        }
    }
}

/// Takes successive [`Sample`]s.
#[derive(Default)]
pub struct Sampler {
    /// `(total, idle)` jiffies at the previous sample.
    last_cpu: Option<(u64, u64)>,
}

impl Sampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read everything, and the GPUs only when `gpus` is set: an NVIDIA
    /// reading initialises and shuts down NVML (held open, it would keep the
    /// card awake), about 20 ms each time, against well under a millisecond
    /// for the rest.
    pub fn sample(&mut self, gpus: bool) -> Sample {
        let cpu = fs::read_to_string("/proc/stat").ok().as_deref().and_then(parse_cpu_times);
        let cpu_percent = match (self.last_cpu, cpu) {
            (Some(before), Some(now)) => cpu_busy(before, now),
            _ => None,
        };
        if cpu.is_some() {
            self.last_cpu = cpu;
        }
        Sample {
            cpu_percent,
            memory: fs::read_to_string("/proc/meminfo").map(|s| parse_meminfo(&s)).unwrap_or_default(),
            root: statvfs("/").unwrap_or_default(),
            gpus: if gpus { gpu::read_all() } else { Vec::new() },
        }
    }
}

/// `(total, idle)` jiffies from the aggregate `cpu` line of `/proc/stat`.
/// Idle counts `iowait` too, as `top` does.
fn parse_cpu_times(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().next()?;
    let mut fields = line.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    // user nice system idle iowait irq softirq steal; guest time is already
    // counted in user.
    let values: Vec<u64> = fields.take(8).filter_map(|v| v.parse().ok()).collect();
    if values.len() < 8 {
        return None;
    }
    Some((values.iter().sum(), values[3] + values[4]))
}

fn cpu_busy((total1, idle1): (u64, u64), (total2, idle2): (u64, u64)) -> Option<f32> {
    let total = total2.checked_sub(total1).filter(|&t| t > 0)?;
    let idle = idle2.saturating_sub(idle1).min(total);
    Some(((total - idle) as f64 * 100.0 / total as f64) as f32)
}

/// Used memory as `MemTotal - MemAvailable`, which is what `free` calls used.
fn parse_meminfo(meminfo: &str) -> Usage {
    let kib = |key: &str| {
        meminfo
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let total = kib("MemTotal:") * 1024;
    let available = kib("MemAvailable:") * 1024;
    Usage { used: total.saturating_sub(available), total }
}

/// Used and total bytes of the filesystem holding `path`. Used is what `df`
/// reports: total less free, including the blocks reserved for root.
fn statvfs(path: &str) -> Option<Usage> {
    let path = CString::new(path).ok()?;
    // SAFETY: a zeroed statvfs is a valid out-parameter, and the path is a
    // NUL-terminated string that outlives the call.
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let block = st.f_frsize as u64;
    let total = st.f_blocks as u64 * block;
    let free = st.f_bfree as u64 * block;
    Some(Usage { used: total.saturating_sub(free), total })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_load_is_the_busy_share_between_samples() {
        let before = parse_cpu_times("cpu  100 0 100 700 100 0 0 0 0 0\ncpu0 1 2 3").unwrap();
        assert_eq!(before, (1000, 800));
        let after = parse_cpu_times("cpu  250 0 150 1300 100 0 0 0 0 0\n").unwrap();
        // 800 jiffies passed, 600 of them idle.
        assert_eq!(cpu_busy(before, after), Some(25.0));
        assert_eq!(cpu_busy(after, after), None, "no time passed");
        assert_eq!(cpu_busy(after, before), None, "counters went backwards");
    }

    #[test]
    fn a_malformed_stat_line_is_ignored() {
        assert_eq!(parse_cpu_times("intr 1 2 3"), None);
        assert_eq!(parse_cpu_times("cpu 1 2 3"), None);
    }

    #[test]
    fn memory_used_excludes_what_is_available() {
        let usage = parse_meminfo("MemTotal:       32000 kB\nMemFree:  1000 kB\nMemAvailable:   24000 kB\n");
        assert_eq!(usage, Usage { used: 8000 * 1024, total: 32000 * 1024 });
        assert_eq!(usage.percent(), 25.0);
        assert_eq!(Usage::default().percent(), 0.0);
    }

    #[test]
    fn the_first_sample_has_no_cpu_load() {
        let mut sampler = Sampler::new();
        assert_eq!(sampler.sample(false).cpu_percent, None);
        let root = statvfs("/").expect("/ has a filesystem");
        assert!(root.total > 0 && root.used <= root.total);
    }
}
