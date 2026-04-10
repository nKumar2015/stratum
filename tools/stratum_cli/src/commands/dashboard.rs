use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

use crate::common::{command_available, emit_help, emit_json, fail, is_help_flag, run_command_capture};

fn print_help() {
    emit_help(
        "dashboard",
        "stratum-cli dashboard [all|calendar|music|performance] [args]",
        &[
            "all [year month]",
            "calendar [year month]",
            "music",
            "performance",
        ],
    );
}

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

fn parse_requested_year_month(args: &[String]) -> (i32, u32) {
    let (current_year, current_month, _current_day) = current_date_parts();

    let year = args
        .first()
        .and_then(|v| v.parse::<i32>().ok())
        .filter(|v| *v >= 1)
        .unwrap_or(current_year);

    let month = args
        .get(1)
        .and_then(|v| v.parse::<u32>().ok())
        .filter(|v| (1..=12).contains(v))
        .unwrap_or(current_month);

    (year, month)
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
        fail("cal not found");
    }

    let month_text = month.to_string();
    let year_text = year.to_string();
    let cal_raw = run_command_capture("cal", &[&month_text, &year_text])
        .unwrap_or_else(|_| fail("failed to read calendar"));

    let lines = cal_raw.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        fail("failed to read calendar");
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

fn run_capture_optional(program: &str, args: &[&str]) -> String {
    let output = match Command::new(program).args(args).output() {
        Ok(output) => output,
        Err(_) => return String::new(),
    };

    if !output.status.success() {
        return String::new();
    }

    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn normalize_player_title(raw: &str) -> String {
    let mut text = raw.to_string();

    if let Some(idx) = text.rfind("MediaPlayer2.") {
        let start = idx + "MediaPlayer2.".len();
        text = text[start..].to_string();
    }

    if let Some(idx) = text.find(".instance") {
        text = text[..idx].to_string();
    }

    text = text
        .chars()
        .map(|ch| if ch == '.' || ch == '_' || ch == '-' { ' ' } else { ch })
        .collect();

    let titled = text
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            let Some(first) = chars.next() else {
                return String::new();
            };
            let mut out = String::new();
            out.push(first.to_ascii_uppercase());
            out.push_str(&chars.as_str().to_ascii_lowercase());
            out
        })
        .collect::<Vec<_>>()
        .join(" ");

    if titled.is_empty() {
        raw.to_string()
    } else {
        titled
    }
}

fn parse_int_from_text(text: &str) -> i64 {
    text.trim()
        .parse::<i64>()
        .ok()
        .or_else(|| {
            text.trim()
                .parse::<f64>()
                .ok()
                .map(|value| value.floor() as i64)
        })
        .unwrap_or(0)
}

fn format_mmss(total_seconds: i64) -> String {
    if total_seconds < 0 {
        return "00:00".to_string();
    }

    let mm = total_seconds / 60;
    let ss = total_seconds % 60;
    format!("{mm:02}:{ss:02}")
}

fn music_payload() -> Value {
    if !command_available("playerctl") {
        return json!({
            "status": "Unavailable",
            "player": "N/A",
            "title": "playerctl not installed",
            "artist": "N/A",
            "album": "N/A",
            "position": "00:00",
            "length": "00:00",
            "art_url": "",
            "player_title": "N/A",
        });
    }

    let mut selected_player = String::new();
    let mut selected_status = String::new();

    let players_raw = run_capture_optional("playerctl", &["-l"]);
    let mut seen = HashSet::new();
    let players = players_raw
        .lines()
        .filter_map(|line| {
            let name = line.trim();
            if name.is_empty() {
                return None;
            }
            if seen.insert(name.to_string()) {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    for player in players {
        let status = run_capture_optional("playerctl", &["-p", &player, "status"]);
        if status.is_empty() {
            continue;
        }

        if status == "Playing" {
            selected_player = player;
            selected_status = status;
            break;
        }

        if selected_player.is_empty() {
            selected_player = player;
            selected_status = status;
        }
    }

    if selected_player.is_empty() {
        return json!({
            "status": "Stopped",
            "player": "None",
            "title": "Nothing playing",
            "artist": "N/A",
            "album": "N/A",
            "position": "00:00",
            "length": "00:00",
            "art_url": "",
            "player_title": "None",
        });
    }

    let mut title = run_capture_optional("playerctl", &["-p", &selected_player, "metadata", "xesam:title"]);
    let artist_raw = run_capture_optional("playerctl", &["-p", &selected_player, "metadata", "xesam:artist"]);
    let mut artist = artist_raw
        .lines()
        .next()
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    let mut album = run_capture_optional("playerctl", &["-p", &selected_player, "metadata", "xesam:album"]);
    let art_url = run_capture_optional("playerctl", &["-p", &selected_player, "metadata", "mpris:artUrl"]);
    let mut player_title = run_capture_optional(
        "playerctl",
        &["-p", &selected_player, "metadata", "--format", "{{mpris:identity}}"],
    );

    if title.is_empty() {
        title = "Unknown Title".to_string();
    }
    if artist.is_empty() {
        artist = "Unknown Artist".to_string();
    }
    if album.is_empty() {
        album = "Unknown Album".to_string();
    }
    if player_title.is_empty() {
        player_title = normalize_player_title(&selected_player);
    }
    if player_title.is_empty() {
        player_title = selected_player.clone();
    }

    let pos_raw = run_capture_optional("playerctl", &["-p", &selected_player, "position"]);
    let pos_seconds = parse_int_from_text(&pos_raw);

    let len_raw = run_capture_optional(
        "playerctl",
        &["-p", &selected_player, "metadata", "mpris:length"],
    );
    let len_micro = parse_int_from_text(&len_raw);
    let len_seconds = if len_micro > 0 { len_micro / 1_000_000 } else { 0 };

    json!({
        "status": selected_status,
        "player": selected_player,
        "title": title,
        "artist": artist,
        "album": album,
        "position": format_mmss(pos_seconds),
        "position_sec": pos_seconds,
        "length": format_mmss(len_seconds),
        "length_sec": len_seconds,
        "art_url": art_url,
        "player_title": player_title,
    })
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

    thread::sleep(Duration::from_millis(200));

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

pub fn handle(args: &[String]) {
    let subcommand = args.first().map(String::as_str).unwrap_or("all");
    if is_help_flag(subcommand) {
        print_help();
        return;
    }

    match subcommand {
        "calendar" => {
            let year_month_args = args.get(1..).unwrap_or(&[]);
            let (year, month) = parse_requested_year_month(year_month_args);
            emit_json(json!({
                "ok": true,
                "command": "dashboard",
                "subcommand": "calendar",
                "calendar": calendar_payload(year, month),
            }));
        }
        "music" => {
            emit_json(json!({
                "ok": true,
                "command": "dashboard",
                "subcommand": "music",
                "music": music_payload(),
            }));
        }
        "performance" => {
            emit_json(json!({
                "ok": true,
                "command": "dashboard",
                "subcommand": "performance",
                "performance": performance_payload(),
            }));
        }
        "all" => {
            let year_month_args = args.get(1..).unwrap_or(&[]);
            let (year, month) = parse_requested_year_month(year_month_args);
            emit_json(json!({
                "ok": true,
                "command": "dashboard",
                "subcommand": "all",
                "calendar": calendar_payload(year, month),
                "music": music_payload(),
                "performance": performance_payload(),
            }));
        }
        _ => fail("unknown dashboard command"),
    }
}
