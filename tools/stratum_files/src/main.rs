//! Stratum's file manager, `stratum-files`.
//!
//! - `stratum-files [FOLDER]` opens a window on the folder (home by
//!   default).
//! - `stratum-files --choose` is the file chooser: the request as JSON on
//!   stdin, the answer as JSON on stdout (`chooser.rs`).
//! - `stratum-files --portal` serves the desktop portal's FileChooser on
//!   the session bus, starting a chooser per request (`portal.rs`).

mod browser;
mod chooser;
mod clipboard;
mod dnd;
mod entries;
mod mime;
mod open;
mod ops;
mod places;
mod portal;
mod preview;
mod thumbs;
mod trash;
mod undo;

use std::io::Read;
use std::path::PathBuf;

use stratum_ui::app::{self, Options};

fn main() {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("--choose") => choose(),
        Some("--portal") => portal::serve(),
        Some("--help") | Some("-h") => {
            println!("usage: stratum-files [FOLDER] | --choose | --portal");
            Ok(())
        }
        other => browse(other.map(browser::expand_home)),
    };
    if let Err(err) = result {
        eprintln!("stratum-files: {err}");
        std::process::exit(1);
    }
}

fn options(title: &str) -> Result<Options, String> {
    let source = stratum_ui::config::ConfigSource::locate().map_err(|e| e.to_string())?;
    let config = app::load_config(&source)?;
    let m = &config.metrics.files;
    Ok(Options { app_id: "org.stratum.Files".into(), title: title.into(), size: (m.width, m.height), min_size: (m.min_width, m.min_height) })
}

fn files_config() -> Result<stratum_ui::config::FilesConfig, String> {
    let source = stratum_ui::config::ConfigSource::locate().map_err(|e| e.to_string())?;
    Ok(app::load_config(&source)?.files)
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/"))
}

fn browse(dir: Option<PathBuf>) -> Result<(), String> {
    // A file opens its folder with it selected; a URI is a path too.
    let dir = dir.map(|d| d.to_string_lossy().strip_prefix("file://").and_then(|_| stratum_ui::uri::to_path(&d.to_string_lossy())).unwrap_or(d));
    let (dir, select) = match dir {
        Some(path) if path.is_file() => (path.parent().map(PathBuf::from).unwrap_or_else(home), Some(path)),
        Some(path) => (path, None),
        None => (home(), None),
    };
    let mut browser = browser::Browser::new(dir, &files_config()?);
    if let Some(path) = select {
        browser.selected.insert(path);
    }
    app::run(browser, options("Files")?)
}

fn choose() -> Result<(), String> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text).map_err(|e| e.to_string())?;
    let request: chooser::Request = if text.trim().is_empty() { Default::default() } else { serde_json::from_str(&text).map_err(|e| e.to_string())? };
    let start = request.current_folder.clone().filter(|d| d.is_dir()).unwrap_or_else(home);
    let mut browser = browser::Browser::new(start, &files_config()?);
    let chooser = chooser::Chooser::new(&request);
    let title = chooser.title.clone();
    browser.chooser = Some(chooser);
    // Listed again now the chooser's filter applies.
    browser.refresh();
    let answer = std::sync::Arc::new(std::sync::Mutex::new(None));
    let slot = std::sync::Arc::clone(&answer);
    let result = app::run(Answering { browser, slot }, options(&title)?);
    let answer = answer.lock().ok().and_then(|a| a.clone()).unwrap_or(chooser::Answer { cancelled: true, ..Default::default() });
    println!("{}", serde_json::to_string(&answer).map_err(|e| e.to_string())?);
    result
}

/// The browser, keeping the chooser's answer where `choose` reads it once
/// the window has closed.
struct Answering {
    browser: browser::Browser,
    slot: std::sync::Arc<std::sync::Mutex<Option<chooser::Answer>>>,
}

impl app::App for Answering {
    fn start(&mut self, waker: app::Waker) {
        self.browser.start(waker);
    }
    fn title(&self) -> String {
        self.browser.title()
    }
    fn paint(&mut self, ui: &mut app::AppUi, list: &mut stratum_ui::render::display_list::DisplayList, area: stratum_ui::render::display_list::Rect, pointer: &app::Pointer) {
        let filter = self.browser.chooser.as_ref().and_then(|c| c.filter());
        self.browser.paint(ui, list, area, pointer);
        if self.browser.chooser.as_ref().and_then(|c| c.filter()) != filter {
            self.browser.refresh();
        }
        self.keep();
    }
    fn key(&mut self, ui: &mut app::AppUi, key: &app::Key) -> app::Flow {
        let flow = self.browser.key(ui, key);
        self.keep();
        flow
    }
    fn scroll(&mut self, point: (f32, f32), delta: f32) {
        self.browser.scroll(point, delta);
    }
    fn tick(&mut self, dt: f32) -> bool {
        self.browser.tick(dt)
    }
    fn poll(&mut self) -> bool {
        self.browser.poll()
    }
    fn done(&self) -> bool {
        self.browser.done()
    }
    fn close_requested(&mut self) -> bool {
        let quit = self.browser.close_requested();
        self.keep();
        quit
    }
}

impl Answering {
    fn keep(&mut self) {
        if let Some(answer) = self.browser.chooser.as_ref().and_then(|c| c.answer.clone()) {
            if let Ok(mut slot) = self.slot.lock() {
                *slot = Some(answer);
            }
        }
    }
}
