use serde_json::{json, Value};
use std::collections::HashSet;

use crate::managers::common::{command_available, run_capture_optional};

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
        .map(|ch| {
            if ch == '.' || ch == '_' || ch == '-' {
                ' '
            } else {
                ch
            }
        })
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

fn get_active_player() -> (String, String) {
    if !command_available("playerctl") {
        return (String::new(), String::new());
    }

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

    for player in &players {
        let status = run_capture_optional("playerctl", &["-p", player, "status"]);
        if status == "Playing" {
            return (player.clone(), status);
        }
    }

    if let Some(player) = players.first() {
        let status = run_capture_optional("playerctl", &["-p", player, "status"]);
        return (player.clone(), status);
    }

    (String::new(), String::new())
}

pub fn status() -> Value {
    if !command_available("playerctl") {
        return json!({
            "ok": true,
            "status": "Unavailable",
            "player": "N/A",
            "title": "playerctl not installed",
            "artist": "N/A",
            "album": "N/A",
            "position": "00:00",
            "length": "00:00",
            "position_sec": 0,
            "length_sec": 0,
            "art_url": "",
            "player_title": "N/A",
        });
    }

    let (selected_player, selected_status) = get_active_player();

    if selected_player.is_empty() {
        return json!({
            "ok": true,
            "status": "Stopped",
            "player": "None",
            "title": "Nothing playing",
            "artist": "N/A",
            "album": "N/A",
            "position": "00:00",
            "length": "00:00",
            "position_sec": 0,
            "length_sec": 0,
            "art_url": "",
            "player_title": "None",
        });
    }

    let mut title = run_capture_optional(
        "playerctl",
        &["-p", &selected_player, "metadata", "xesam:title"],
    );
    let artist_raw = run_capture_optional(
        "playerctl",
        &["-p", &selected_player, "metadata", "xesam:artist"],
    );
    let mut artist = artist_raw
        .lines()
        .next()
        .map(|v| v.trim().to_string())
        .unwrap_or_default();
    let mut album = run_capture_optional(
        "playerctl",
        &["-p", &selected_player, "metadata", "xesam:album"],
    );
    let art_url = run_capture_optional(
        "playerctl",
        &["-p", &selected_player, "metadata", "mpris:artUrl"],
    );
    let mut player_title = run_capture_optional(
        "playerctl",
        &[
            "-p",
            &selected_player,
            "metadata",
            "--format",
            "{{mpris:identity}}",
        ],
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
    let len_seconds = if len_micro > 0 {
        len_micro / 1_000_000
    } else {
        0
    };

    json!({
        "ok": true,
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
