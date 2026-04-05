use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use crate::common::{emit_help, emit_json, fail, is_help_flag, run_command_capture};

fn print_help() {
    emit_help(
        "screenshot-post",
        "stratum-cli screenshot-post <copy|save|save-to|save-as|copy-text> [args]",
        &[
            "copy <image_path>",
            "save <image_path>",
            "save-to <image_path> <target_path>",
            "save-as <image_path> <target_path>",
            "copy-text <text>",
        ],
    );
}

fn normalize_path(value: &str) -> String {
    let mut out = value.trim().to_string();
    if out.starts_with("file://") {
        out = out.trim_start_matches("file://").to_string();
        while out.starts_with('/') {
            out.remove(0);
        }
        out.insert(0, '/');
    }

    match urlencoding::decode(&out) {
        Ok(decoded) => decoded.into_owned(),
        Err(_) => out,
    }
}

fn unique_png_path(candidate: &Path) -> PathBuf {
    if !candidate.exists() {
        return candidate.to_path_buf();
    }

    let stem = candidate
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Screenshot")
        .to_string();
    let parent = candidate.parent().unwrap_or_else(|| Path::new("."));

    for idx in 1..10000 {
        let next = parent.join(format!("{}-{}.png", stem, idx));
        if !next.exists() {
            return next;
        }
    }

    candidate.to_path_buf()
}

fn timestamp() -> String {
    if let Ok(ts) = run_command_capture("date", &["+%Y%m%d-%H%M%S"]) {
        if !ts.is_empty() {
            return ts;
        }
    }

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{}", secs)
}

fn copy_text_clipboard(text: &str) -> Result<(), String> {
    if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| format!("wl-copy stdin failed: {}", e))?;
        }
        let status = child.wait().map_err(|e| format!("wl-copy wait failed: {}", e))?;
        if status.success() {
            return Ok(());
        }
    }

    if let Ok(mut child) = Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(text.as_bytes())
                .map_err(|e| format!("xclip stdin failed: {}", e))?;
        }
        let status = child.wait().map_err(|e| format!("xclip wait failed: {}", e))?;
        if status.success() {
            return Ok(());
        }
    }

    Err("no clipboard tool found (install wl-clipboard or xclip)".to_string())
}

fn copy_image_clipboard(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|e| format!("failed to read image file: {}", e))?;

    if let Ok(mut child) = Command::new("wl-copy").stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(&bytes)
                .map_err(|e| format!("wl-copy stdin failed: {}", e))?;
        }
        let status = child.wait().map_err(|e| format!("wl-copy wait failed: {}", e))?;
        if status.success() {
            return Ok(());
        }
    }

    let status = Command::new("xclip")
        .args([
            "-selection",
            "clipboard",
            "-t",
            "image/png",
            "-i",
            path.to_string_lossy().as_ref(),
        ])
        .status();
    if let Ok(ok) = status {
        if ok.success() {
            return Ok(());
        }
    }

    Err("no clipboard tool found (install wl-clipboard or xclip)".to_string())
}

fn require_file(path: &str) -> PathBuf {
    let normalized = normalize_path(path);
    if normalized.is_empty() {
        fail("missing image path");
    }

    let pb = PathBuf::from(&normalized);
    if !pb.is_file() {
        fail("image file not found");
    }
    pb
}

pub fn handle(args: &[String]) {
    if args.is_empty() {
        fail("missing action");
    }

    let action = args[0].as_str();
    if is_help_flag(action) {
        print_help();
        return;
    }

    if action == "copy-text" {
        let text = args.get(1).map(|s| s.as_str()).unwrap_or("");
        if let Err(e) = copy_text_clipboard(text) {
            fail(&e);
        }
        emit_json(json!({
            "ok": true,
            "command": "screenshot-post",
            "action": "copy-text",
            "text": text,
        }));
        return;
    }

    let image_path = require_file(args.get(1).map(|s| s.as_str()).unwrap_or(""));

    match action {
        "copy" => {
            if let Err(e) = copy_image_clipboard(&image_path) {
                fail(&e);
            }
            emit_json(json!({
                "ok": true,
                "command": "screenshot-post",
                "action": "copy",
                "path": image_path.to_string_lossy(),
            }));
        }
        "save" => {
            let home = env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            let target_dir = PathBuf::from(home).join("Pictures/Screenshots");
            if let Err(e) = fs::create_dir_all(&target_dir) {
                fail(&format!("failed to create screenshot directory: {}", e));
            }

            let target = unique_png_path(&target_dir.join(format!("Screenshot-{}.png", timestamp())));
            if let Err(e) = fs::copy(&image_path, &target) {
                fail(&format!("failed to save screenshot: {}", e));
            }
            emit_json(json!({
                "ok": true,
                "command": "screenshot-post",
                "action": "save",
                "path": target.to_string_lossy(),
            }));
        }
        "save-to" | "save-as" => {
            let raw_target = args.get(2).map(|s| s.as_str()).unwrap_or("");
            let target = normalize_path(raw_target);
            if target.is_empty() {
                fail("missing destination path");
            }

            let mut target_path = PathBuf::from(&target);
            let mut generated_name = false;
            if target.ends_with('/') || target_path.is_dir() {
                target_path = PathBuf::from(target.trim_end_matches('/'))
                    .join(format!("Screenshot-{}.png", timestamp()));
                generated_name = true;
            }

            if target_path.extension().and_then(|s| s.to_str()) != Some("png") {
                target_path.set_extension("png");
            }

            if generated_name {
                target_path = unique_png_path(&target_path);
            }

            if let Some(parent) = target_path.parent() {
                if let Err(e) = fs::create_dir_all(parent) {
                    fail(&format!("failed to create destination directory: {}", e));
                }
            }

            if let Err(e) = fs::copy(&image_path, &target_path) {
                fail(&format!("failed to save screenshot: {}", e));
            }
            emit_json(json!({
                "ok": true,
                "command": "screenshot-post",
                "action": "save-as",
                "path": target_path.to_string_lossy(),
            }));
        }
        _ => fail("unknown action"),
    }
}
