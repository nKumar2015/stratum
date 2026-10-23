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

/// Separates fields in playerctl's `--format` output: titles may hold any
/// printable character.
const FIELD: char = '\u{1f}';

/// Every player and its status, in playerctl's order, from one run.
fn players_with_status() -> Vec<(String, String)> {
    let format = format!("{{{{playerInstance}}}}{FIELD}{{{{status}}}}");
    let out = run_capture_optional("playerctl", &["-a", "metadata", "--format", &format]);
    let mut seen = HashSet::new();
    out.lines()
        .filter_map(|line| {
            let (name, status) = line.split_once(FIELD)?;
            let name = name.trim();
            (!name.is_empty() && seen.insert(name.to_string())).then(|| (name.to_string(), status.trim().to_string()))
        })
        .collect()
}

/// The player to show: `preferred` while it is there, else the first one
/// playing, else the first.
fn pick_player(players: &[(String, String)], preferred: Option<&str>) -> Option<(String, String)> {
    preferred
        .and_then(|p| players.iter().find(|(name, _)| name == p))
        .or_else(|| players.iter().find(|(_, status)| status == "Playing"))
        .or_else(|| players.first())
        .cloned()
}

pub fn status() -> Value {
    status_for(None)
}

/// [`status`] for `preferred` while it is there, with every player listed
/// (`players`: `[{id, name}]`) and its `shuffle` (`On`, `Off`) and `loop`
/// (`None`, `Track`, `Playlist`); both are empty for a player without them.
pub fn status_for(preferred: Option<&str>) -> Value {
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

    let players = players_with_status();
    let listed: Vec<Value> =
        players.iter().map(|(id, _)| json!({ "id": id, "name": normalize_player_title(id) })).collect();
    let Some((selected_player, selected_status)) = pick_player(&players, preferred.filter(|p| !p.is_empty())) else {
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
            "players": listed,
            "shuffle": "",
            "loop": "",
        });
    };

    // Everything about the player in one run, not one per field.
    let fields = [
        "{{xesam:title}}",
        "{{xesam:artist}}",
        "{{xesam:album}}",
        "{{mpris:artUrl}}",
        "{{mpris:length}}",
        "{{position}}",
        "{{shuffle}}",
        "{{loop}}",
    ];
    let format = fields.join(&FIELD.to_string());
    let out = run_capture_optional("playerctl", &["-p", &selected_player, "metadata", "--format", &format]);
    let values: Vec<&str> = out.split(FIELD).map(str::trim).collect();
    let value = |i: usize| values.get(i).copied().unwrap_or_default();
    let or = |text: &str, fallback: &str| if text.is_empty() { fallback.to_string() } else { text.to_string() };

    let title = or(value(0), "Unknown Title");
    let artist = or(value(1).lines().next().unwrap_or_default().trim(), "Unknown Artist");
    let album = or(value(2), "Unknown Album");
    let art_url = value(3).to_string();
    let len_micro = parse_int_from_text(value(4));
    let len_seconds = if len_micro > 0 { len_micro / 1_000_000 } else { 0 };
    let pos_seconds = parse_int_from_text(value(5)) / 1_000_000;
    let shuffle = match value(6) {
        "true" => "On",
        "false" => "Off",
        _ => "",
    };
    let player_title = or(&normalize_player_title(&selected_player), &selected_player);

    json!({
        "players": listed,
        "shuffle": shuffle,
        "loop": value(7),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn players() -> Vec<(String, String)> {
        [("spotify", "Paused"), ("mpv.instance-1", "Playing"), ("firefox.instance-2", "Stopped")]
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect()
    }

    #[test]
    fn the_chosen_player_wins_while_it_is_there() {
        let pick = |preferred| pick_player(&players(), preferred).map(|(name, _)| name);
        assert_eq!(pick(Some("spotify")).as_deref(), Some("spotify"));
        assert_eq!(pick(None).as_deref(), Some("mpv.instance-1"), "else the one playing");
        assert_eq!(pick(Some("gone")).as_deref(), Some("mpv.instance-1"), "a player that left is not kept");
        let idle = vec![("a".to_string(), "Paused".to_string())];
        assert_eq!(pick_player(&idle, None).map(|(n, _)| n).as_deref(), Some("a"), "else the first");
        assert_eq!(pick_player(&[], None), None);
    }

    #[test]
    fn instance_names_read_as_player_names() {
        assert_eq!(normalize_player_title("mpv.instance-ZxzCQqRK"), "Mpv");
        assert_eq!(normalize_player_title("org.mpris.MediaPlayer2.spotify"), "Spotify");
        assert_eq!(normalize_player_title("firefox.instance_2_45"), "Firefox");
    }
}
