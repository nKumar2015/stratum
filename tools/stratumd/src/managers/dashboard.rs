use serde_json::{json, Value};
use std::fs;
use std::path::Path;

use crate::managers::common::{command_available, run_capture_optional, run_command_capture};

fn current_date_parts() -> (i32, u32, u32) {
    let output = run_command_capture("date", &["+%Y %m %d"]).unwrap_or_default();
    let mut parts = output.split_whitespace();
    let year = parts
        .next()
        .and_then(|v| v.parse::<i32>().ok())
        .unwrap_or(1970);
    let month = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 12);
    let day = parts
        .next()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1)
        .clamp(1, 31);
    (year, month, day)
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn weekday_sunday_first(year: i32, month: u32, day: u32) -> u32 {
    let offsets = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut y = year;
    if month < 3 {
        y -= 1;
    }

    let w = y + y / 4 - y / 100 + y / 400 + offsets[(month - 1) as usize] + day as i32;
    (((w % 7) + 7) % 7) as u32
}

fn parse_cal_row(line: &str) -> Vec<i32> {
    let bytes = line.as_bytes();
    let mut row = Vec::with_capacity(7);

    for i in 0..7 {
        let start = i * 3;
        if start >= bytes.len() {
            row.push(0);
            continue;
        }

        let end = (start + 2).min(bytes.len());
        let cell = std::str::from_utf8(&bytes[start..end]).unwrap_or("").trim();
        row.push(cell.parse::<i32>().unwrap_or(0));
    }

    row
}

fn parse_weekdays(line: &str) -> Vec<String> {
    let defaults = ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
    let values = line
        .split_whitespace()
        .take(7)
        .map(|v| v.to_string())
        .collect::<Vec<_>>();

    if values.len() == 7 {
        values
    } else {
        defaults.iter().map(|v| (*v).to_string()).collect()
    }
}

fn calendar_payload(year: i32, month: u32) -> Value {
    if !command_available("cal") {
        return json!({
            "error": "cal not found",
        });
    }

    let month_text = month.to_string();
    let year_text = year.to_string();
    let cal_raw = match run_command_capture("cal", &[&month_text, &year_text]) {
        Ok(output) => output,
        Err(_) => {
            return json!({
                "error": "failed to read calendar",
            })
        }
    };

    let lines = cal_raw.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return json!({
            "error": "failed to read calendar",
        });
    }

    let title = lines.first().map(|v| v.trim()).unwrap_or("").to_string();
    let weekday_line = lines.get(1).copied().unwrap_or("");
    let weekdays = parse_weekdays(weekday_line);

    let mut rows = Vec::new();
    for line in lines.iter().skip(2) {
        rows.push(parse_cal_row(line));
    }
    while rows.len() < 6 {
        rows.push(vec![0, 0, 0, 0, 0, 0, 0]);
    }
    if rows.len() > 6 {
        rows.truncate(6);
    }

    let (current_year, current_month, current_day) = current_date_parts();
    let today = if year == current_year && month == current_month {
        current_day as i32
    } else {
        -1
    };

    json!({
        "title": title,
        "year": year,
        "month": month,
        "days_in_month": days_in_month(year, month),
        "first_weekday": weekday_sunday_first(year, month, 1),
        "weekdays": weekdays,
        "rows": rows,
        "today": today,
    })
}

fn parse_first_int(text: &str) -> Option<i64> {
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_digit() {
            current.push(ch);
            continue;
        }

        if !current.is_empty() {
            return current.parse::<i64>().ok();
        }
    }

    if current.is_empty() {
        None
    } else {
        current.parse::<i64>().ok()
    }
}

fn read_cpu_snapshot() -> Option<(u64, u64)> {
    let stat = fs::read_to_string("/proc/stat").ok()?;
    let line = stat.lines().next()?.trim();
    if !line.starts_with("cpu ") {
        return None;
    }

    let values = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .filter_map(|v| v.parse::<u64>().ok())
        .collect::<Vec<_>>();

    if values.len() < 8 {
        return None;
    }

    let total = values.iter().sum::<u64>();
    let idle_all = values[3] + values[4];
    Some((total, idle_all))
}

fn cpu_usage_percent() -> i64 {
    let Some((total1, idle1)) = read_cpu_snapshot() else {
        return 0;
    };

    std::thread::sleep(std::time::Duration::from_millis(200));

    let Some((total2, idle2)) = read_cpu_snapshot() else {
        return 0;
    };

    if total2 <= total1 {
        return 0;
    }

    let total_delta = total2 - total1;
    let idle_delta = idle2.saturating_sub(idle1);
    let used = total_delta.saturating_sub(idle_delta);
    ((used * 100) / total_delta) as i64
}

fn read_gpu_usage() -> (String, String) {
    if command_available("nvidia-smi") {
        let output = run_capture_optional(
            "nvidia-smi",
            &[
                "--query-gpu=utilization.gpu",
                "--format=csv,noheader,nounits",
            ],
        );
        if let Some(first_line) = output.lines().next() {
            if let Some(value) = parse_first_int(first_line) {
                return (value.to_string(), "NVIDIA".to_string());
            }
        }
    }

    let drm_root = Path::new("/sys/class/drm");
    if let Ok(entries) = fs::read_dir(drm_root) {
        let mut cards = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.starts_with("card"))
                    .unwrap_or(false)
            })
            .collect::<Vec<_>>();
        cards.sort();

        for card in &cards {
            let path = card.join("device/gpu_busy_percent");
            if let Ok(text) = fs::read_to_string(&path) {
                if let Some(value) = parse_first_int(&text) {
                    return (value.to_string(), "DRM".to_string());
                }
            }
        }

        for card in &cards {
            let path = card.join("gt_busy_percent");
            if let Ok(text) = fs::read_to_string(&path) {
                if let Some(value) = parse_first_int(&text) {
                    return (value.to_string(), "INTEL".to_string());
                }
            }
        }
    }

    ("N/A".to_string(), "N/A".to_string())
}

fn read_meminfo_kib(key: &str) -> i64 {
    let content = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    for line in content.lines() {
        if !line.starts_with(key) {
            continue;
        }

        let mut parts = line.split_whitespace();
        let _label = parts.next();
        if let Some(value) = parts.next() {
            return value.parse::<i64>().unwrap_or(0);
        }
    }
    0
}

fn format_gib_tenths(kib: i64) -> f64 {
    let value = if kib <= 0 {
        0.0
    } else {
        kib as f64 / 1_048_576.0
    };
    (value * 10.0).round() / 10.0
}

fn root_storage_kib() -> (i64, i64, i64) {
    let output = run_capture_optional("df", &["-kP", "/"]);
    let line = output.lines().nth(1).unwrap_or("");
    let cols = line.split_whitespace().collect::<Vec<_>>();
    if cols.len() < 5 {
        return (0, 0, 0);
    }

    let total = cols[1].parse::<i64>().unwrap_or(0);
    let used = cols[2].parse::<i64>().unwrap_or(0);
    let pct = cols[4]
        .trim_end_matches('%')
        .parse::<i64>()
        .unwrap_or(0)
        .clamp(0, 100);

    (total, used, pct)
}

fn performance_payload() -> Value {
    let cpu = cpu_usage_percent().clamp(0, 100);
    let (gpu_text, gpu_source) = read_gpu_usage();
    let gpu_value = parse_first_int(&gpu_text).map(|value| value.clamp(0, 100));

    let mem_total_kib = read_meminfo_kib("MemTotal:");
    let mem_avail_kib = read_meminfo_kib("MemAvailable:");
    let mem_used_kib = (mem_total_kib - mem_avail_kib).max(0);
    let mem_pct = if mem_total_kib > 0 {
        ((mem_used_kib * 100) / mem_total_kib).clamp(0, 100)
    } else {
        0
    };

    let (storage_total_kib, storage_used_kib, storage_pct) = root_storage_kib();

    json!({
        "cpu_percent": cpu,
        "gpu_percent_text": gpu_text,
        "gpu_percent_value": gpu_value,
        "gpu_source": gpu_source,
        "ram_used_gib": format_gib_tenths(mem_used_kib),
        "ram_total_gib": format_gib_tenths(mem_total_kib),
        "ram_percent": mem_pct,
        "storage_used_gib": format_gib_tenths(storage_used_kib),
        "storage_total_gib": format_gib_tenths(storage_total_kib),
        "storage_percent": storage_pct,
    })
}

pub fn status(year: i32, month: i32) -> Value {
    let month_u = (month as u32).clamp(1, 12);
    json!({
        "ok": true,
        "calendar": calendar_payload(year, month_u),
        "music": super::music::status(),
        "performance": performance_payload(),
    })
}
