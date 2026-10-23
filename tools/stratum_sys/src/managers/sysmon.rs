//! Resource readings for the dashboard: CPU, memory, the root filesystem,
//! network, temperatures, every GPU and the busiest processes, as typed
//! values.
//!
//! [`Sampler`] keeps the previous counters, so CPU load, network rates and
//! each process's CPU share cover the time since the last sample. The older
//! `dashboard::performance_payload` slept 200 ms between two reads of its own
//! on every call instead, and ran `df` for the disk; this reads `statvfs`.

use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

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
    /// Over physical interfaces since the previous sample; `None` on the
    /// first.
    pub network: Option<Rates>,
    pub temperatures: Temperatures,
    /// Seconds since boot.
    pub uptime: Option<u64>,
    /// One per GPU present, in card order; empty unless asked for.
    pub gpus: Vec<GpuReading>,
    /// The busiest processes, most CPU first; empty unless asked for, and on
    /// the first sample that asks.
    pub processes: Vec<Process>,
}

/// Bytes per second received and sent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rates {
    pub rx: f64,
    pub tx: f64,
}

/// °C, where a sensor exists.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Temperatures {
    /// The package (k10temp's Tctl, coretemp's package).
    pub cpu: Option<f32>,
    /// The hottest NVMe drive.
    pub storage: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Process {
    pub pid: u32,
    /// The program's name from its command line, else the kernel's `comm`
    /// (at most 15 bytes).
    pub name: String,
    /// Share of the whole machine since the previous sample, 0–100, so the
    /// processes add up to the CPU row.
    pub cpu_percent: f32,
    /// Resident memory in bytes.
    pub memory: u64,
}

/// What a sample reads beyond the cheap counters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Detail {
    /// Read the GPUs: an NVIDIA reading initialises and shuts down NVML (held
    /// open, it would keep the card awake), about 20 ms each time.
    pub gpus: bool,
    /// Keep this many processes. Reading them costs a file per process.
    pub processes: usize,
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
    /// Received and sent bytes, and when.
    last_network: Option<(u64, u64, Instant)>,
    /// Each process's CPU ticks, and the machine's total jiffies, when
    /// processes were last read.
    last_processes: HashMap<u32, u64>,
    last_process_total: Option<u64>,
    /// Temperature inputs, found on first use.
    sensors: Option<Sensors>,
}

#[derive(Default)]
struct Sensors {
    cpu: Option<PathBuf>,
    storage: Vec<PathBuf>,
}

impl Sampler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read the counters, and what `detail` asks for. Without detail a
    /// sample is a handful of file reads, well under a millisecond.
    pub fn sample(&mut self, detail: Detail) -> Sample {
        let cpu = fs::read_to_string("/proc/stat").ok().as_deref().and_then(parse_cpu_times);
        let cpu_percent = match (self.last_cpu, cpu) {
            (Some(before), Some(now)) => cpu_busy(before, now),
            _ => None,
        };
        if cpu.is_some() {
            self.last_cpu = cpu;
        }
        let processes = if detail.processes > 0 {
            self.processes(cpu.map(|(total, _)| total), detail.processes)
        } else {
            // A later read compares against these; stale ones would count
            // all the time in between.
            self.last_processes.clear();
            self.last_process_total = None;
            Vec::new()
        };
        Sample {
            cpu_percent,
            memory: fs::read_to_string("/proc/meminfo").map(|s| parse_meminfo(&s)).unwrap_or_default(),
            root: statvfs("/").unwrap_or_default(),
            network: self.network(),
            temperatures: self.temperatures(),
            uptime: fs::read_to_string("/proc/uptime")
                .ok()
                .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
                .map(|s| s as u64),
            gpus: if detail.gpus { gpu::read_all() } else { Vec::new() },
            processes,
        }
    }

    fn network(&mut self) -> Option<Rates> {
        let (rx, tx) = fs::read_to_string("/proc/net/dev").ok().map(|dev| parse_net_dev(&dev, is_physical))?;
        let now = Instant::now();
        let rates = self.last_network.and_then(|(rx0, tx0, then)| {
            let seconds = now.duration_since(then).as_secs_f64();
            (seconds > 0.0).then(|| Rates {
                rx: rx.saturating_sub(rx0) as f64 / seconds,
                tx: tx.saturating_sub(tx0) as f64 / seconds,
            })
        });
        self.last_network = Some((rx, tx, now));
        rates
    }

    fn temperatures(&mut self) -> Temperatures {
        let sensors = self.sensors.get_or_insert_with(find_sensors);
        let read = |path: &PathBuf| {
            fs::read_to_string(path).ok()?.trim().parse::<f32>().ok().map(|milli| milli / 1000.0)
        };
        Temperatures {
            cpu: sensors.cpu.as_ref().and_then(read),
            storage: sensors.storage.iter().filter_map(read).reduce(f32::max),
        }
    }

    /// The `limit` busiest processes since the last call. The first call
    /// only records, and returns none.
    fn processes(&mut self, total: Option<u64>, limit: usize) -> Vec<Process> {
        let page = page_size();
        let mut ticks = HashMap::new();
        let mut found = Vec::new();
        for entry in fs::read_dir("/proc").into_iter().flatten().flatten() {
            let Some(pid) = entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) else {
                continue;
            };
            // A process can exit between the listing and the read.
            let Some(stat) = fs::read_to_string(entry.path().join("stat")).ok() else {
                continue;
            };
            let Some((name, cpu, rss)) = parse_pid_stat(&stat) else {
                continue;
            };
            ticks.insert(pid, cpu);
            found.push((pid, name, cpu, rss * page));
        }
        let elapsed = match (self.last_process_total, total) {
            (Some(before), Some(now)) if now > before => Some(now - before),
            _ => None,
        };
        let mut processes: Vec<Process> = match elapsed {
            Some(elapsed) => found
                .into_iter()
                .map(|(pid, name, cpu, memory)| {
                    // A pid seen for the first time started since the last
                    // read: all its ticks are new.
                    let used = cpu.saturating_sub(self.last_processes.get(&pid).copied().unwrap_or(0));
                    Process { pid, name, cpu_percent: (used as f64 * 100.0 / elapsed as f64) as f32, memory }
                })
                .collect(),
            None => Vec::new(),
        };
        processes.sort_by(|a, b| b.cpu_percent.total_cmp(&a.cpu_percent).then(b.memory.cmp(&a.memory)));
        processes.truncate(limit);
        // Only for the ones shown: a file each.
        for process in &mut processes {
            let cmdline = fs::read(format!("/proc/{}/cmdline", process.pid)).unwrap_or_default();
            if let Some(name) = program_name(&cmdline) {
                process.name = name;
            }
        }
        self.last_processes = ticks;
        self.last_process_total = total;
        processes
    }
}

/// The CPU package and NVMe temperature inputs among the hwmon devices.
fn find_sensors() -> Sensors {
    let mut sensors = Sensors::default();
    for hwmon in fs::read_dir("/sys/class/hwmon").into_iter().flatten().flatten() {
        let path = hwmon.path();
        let name = fs::read_to_string(path.join("name")).unwrap_or_default();
        let input = path.join("temp1_input");
        if !input.exists() {
            continue;
        }
        match name.trim() {
            // AMD's Tctl, Intel's package, and the out-of-tree zenpower.
            "k10temp" | "coretemp" | "zenpower" if sensors.cpu.is_none() => sensors.cpu = Some(input),
            "nvme" => sensors.storage.push(input),
            _ => {}
        }
    }
    sensors
}

/// Whether `interface` is backed by a device: loopback, bridges, veths and
/// tunnels are left out, since traffic through them is counted again on
/// the physical link.
fn is_physical(interface: &str) -> bool {
    std::path::Path::new("/sys/class/net").join(interface).join("device").exists()
}

/// Received and sent bytes summed over the interfaces `keep` accepts.
fn parse_net_dev(dev: &str, keep: impl Fn(&str) -> bool) -> (u64, u64) {
    dev.lines()
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| keep(name.trim()))
        .filter_map(|(_, counters)| {
            let fields: Vec<u64> = counters.split_whitespace().filter_map(|v| v.parse().ok()).collect();
            // 8 receive counters, then transmit; bytes come first in each.
            Some((*fields.first()?, *fields.get(8)?))
        })
        .fold((0, 0), |(rx, tx), (r, t)| (rx + r, tx + t))
}

/// `(comm, utime + stime, rss pages)` from `/proc/<pid>/stat`. The name is
/// in parentheses and may itself hold spaces or parentheses, so the fields
/// are counted from the last `)`.
fn parse_pid_stat(stat: &str) -> Option<(String, u64, u64)> {
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?.to_string();
    let fields: Vec<&str> = stat.get(close + 1..)?.split_whitespace().collect();
    // After the name: state is field 3, utime 14, stime 15, rss 24.
    let field = |n: usize| fields.get(n - 3)?.parse::<u64>().ok();
    Some((name, field(14)? + field(15)?, field(24)?))
}

/// The program's name from its command line: the file name of the first
/// argument, without Nix's wrapper form (`.kitty-wrapped` is kitty). Some
/// daemons rewrite their command line into a status line
/// (`avahi-daemon: running [legion.local]`, `sshd: user@pts/0`), which is
/// cut at the first space and loses its colon. `None` for kernel threads,
/// whose command line is empty.
fn program_name(cmdline: &[u8]) -> Option<String> {
    let first = cmdline.split(|&b| b == 0).next()?;
    let first = String::from_utf8_lossy(first);
    let first = first.split(' ').next()?.trim_end_matches(':');
    let name = first.rsplit('/').next()?;
    let name = name.strip_prefix('.').and_then(|n| n.strip_suffix("-wrapped")).unwrap_or(name);
    (!name.is_empty()).then(|| name.to_string())
}

fn page_size() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(size).unwrap_or(4096)
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
    fn network_counts_only_kept_interfaces() {
        let dev = "Inter-|   Receive  |  Transmit\n face |bytes packets errs drop fifo frame compressed multicast|bytes ...\n    lo: 500 5 0 0 0 0 0 0 500 5 0 0 0 0 0 0\nwlan0: 1000 10 0 0 0 0 0 0 200 2 0 0 0 0 0 0\n  eth0: 30 1 0 0 0 0 0 0 4 1 0 0 0 0 0 0\n";
        assert_eq!(parse_net_dev(dev, |name| name != "lo"), (1030, 204));
    }

    #[test]
    fn pid_stat_names_may_hold_spaces_and_parentheses() {
        let stat = "4242 (Web Content (x)) S 1 2 3 4 5 6 7 8 9 10 150 50 0 0 20 0 30 0 100 2000000 777 18446744073709551615";
        assert_eq!(parse_pid_stat(stat), Some(("Web Content (x)".to_string(), 200, 777)));
        assert_eq!(parse_pid_stat("1 (init) S 0"), None);
    }

    #[test]
    fn program_names_come_from_the_command_line() {
        let name = |cmdline: &[u8]| program_name(cmdline);
        assert_eq!(name(b"/nix/store/abc-kitty/bin/.kitty-wrapped\0--single-instance\0"), Some("kitty".into()));
        assert_eq!(name(b"python3\0script.py\0"), Some("python3".into()));
        // Daemons that turn their command line into a status line.
        assert_eq!(name(b"avahi-daemon: running [legion.local]\0"), Some("avahi-daemon".into()));
        assert_eq!(name(b"sshd: nakul@pts/0\0"), Some("sshd".into()));
        assert_eq!(name(b".hidden\0"), Some(".hidden".into()), "only the full wrapper form is unwrapped");
        assert_eq!(name(b""), None, "a kernel thread keeps its comm");
    }

    #[test]
    fn processes_need_two_samples() {
        let mut sampler = Sampler::new();
        let detail = Detail { gpus: false, processes: 5 };
        assert!(sampler.sample(detail).processes.is_empty(), "nothing to compare yet");
        // Burn some CPU so time passes for at least one process.
        let start = Instant::now();
        while start.elapsed().as_millis() < 30 {
            std::hint::black_box(start.elapsed());
        }
        let processes = sampler.sample(detail).processes;
        assert!(!processes.is_empty() && processes.len() <= 5);
        assert!(processes.windows(2).all(|w| w[0].cpu_percent >= w[1].cpu_percent));
        // Without processes, the record is dropped.
        sampler.sample(Detail::default());
        assert!(sampler.sample(detail).processes.is_empty());
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
        assert_eq!(sampler.sample(Detail::default()).cpu_percent, None);
        let root = statvfs("/").expect("/ has a filesystem");
        assert!(root.total > 0 && root.used <= root.total);
    }
}
