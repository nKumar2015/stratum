//! The file manager's window: a toolbar (back, forward, up, the path or a
//! path field, search, view and hidden-file switches), the places sidebar,
//! the folder as a list or a grid, and a status bar. Right click opens a
//! context menu; copying, moving and deleting run on threads and report in
//! the status bar. The same view, with a bar for the name and the answer
//! buttons, is the file chooser (`chooser.rs`).

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime};

use stratum_ui::app::{App, AppUi, Flow, Key, Keysym, Pointer, Waker};
use stratum_ui::icons::IconImage;
use stratum_ui::render::display_list::{Color, DisplayList, Image, Rect};
use stratum_ui::style::FilesMetrics;
use stratum_ui::widgets::draw as ui;
use stratum_ui::widgets::layout::{self, rect, Card};
use stratum_ui::widgets::text_field::{FieldKey, FieldStyle, TextField};
use stratum_ui::widgets::{InputState, Ui};

use crate::chooser::Chooser;
use crate::entries::{self, Entry, SortKey};
use crate::mime::MimeDb;
use crate::places::{self, Place, Section};
use crate::{clipboard, open, ops, trash};

const BACK: &str = "\u{f004d}";
const FORWARD: &str = "\u{f0054}";
const UP: &str = "\u{f005d}";
const SEARCH: &str = "\u{f0349}";
const LIST: &str = "\u{f0279}";
const GRID: &str = "\u{f0570}";
const HIDDEN: &str = "\u{f0208}";
const FOLDER: &str = "\u{f024b}";
const FILE: &str = "\u{f0214}";
const SORT_UP: &str = "\u{f0360}";
const SORT_DOWN: &str = "\u{f035d}";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    List,
    Grid,
}

/// What a context menu item does.
#[derive(Clone, Debug, PartialEq)]
enum Command {
    Open,
    OpenWith,
    OpenApp(usize),
    Cut,
    Copy,
    Paste,
    Rename,
    Trash,
    Delete,
    Restore,
    EmptyTrash,
    NewFolder,
    CopyPath,
    TerminalHere,
    ToggleHidden,
    SelectAll,
}

struct Menu {
    anchor: Rect,
    items: Vec<(String, Command)>,
    scroll: Option<f32>,
    /// Opened by this frame's click, which is not a click outside it.
    fresh: bool,
}

/// What is being typed into.
enum Editing {
    None,
    /// The path, replacing the breadcrumbs.
    Path(TextField),
    Search,
    /// A new name for an entry (or for a folder being made).
    Rename { path: PathBuf, field: TextField, new_folder: bool },
}

/// A running copy or move.
struct Job {
    label: &'static str,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<ops::Progress>>,
}

pub struct Browser {
    db: MimeDb,
    pub dir: PathBuf,
    entries: Vec<Entry>,
    error: Option<String>,
    dir_stamp: Option<SystemTime>,
    last_check: Instant,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    /// Selected entries, by path.
    pub selected: BTreeSet<PathBuf>,
    /// Where keyboard movement and Shift ranges start (an index into the
    /// visible entries).
    cursor: Option<usize>,
    anchor: Option<usize>,
    scroll: f32,
    follow: bool,
    pub view: View,
    pub show_hidden: bool,
    sort: SortKey,
    descending: bool,
    search: TextField,
    editing: Editing,
    places: Vec<(Section, Place)>,
    menu: Option<Menu>,
    menu_rect: Option<Rect>,
    open_with: Vec<stratum_ui::apps::App>,
    confirm: Option<Vec<PathBuf>>,
    job: Option<Job>,
    message: Option<(String, bool, Instant)>,
    thumbs: HashMap<PathBuf, Option<IconImage>>,
    thumb_tx: Option<std::sync::mpsc::Sender<(PathBuf, u32)>>,
    thumb_done: Arc<Mutex<Vec<Thumbnail>>>,
    waker: Option<Waker>,
    /// Hit areas from the last paint: entries by visible index, and the
    /// list's viewport and content height.
    hits: Vec<(Rect, usize)>,
    viewport: Rect,
    content_height: f32,
    grid_columns: usize,
    pub chooser: Option<Chooser>,
    pub quit: bool,
}

/// A decoded thumbnail, or `None` for a picture that did not decode.
type Thumbnail = (PathBuf, Option<IconImage>);

/// Seconds a status message stays.
const MESSAGE_SECONDS: f32 = 4.0;

/// Room in the typed-path and search fields: Linux's `PATH_MAX`. A field
/// holds 256 bytes by default, a name's limit, which a deep path passes.
const LONG_FIELD: usize = libc::PATH_MAX as usize;

impl Browser {
    pub fn new(dir: PathBuf, config: &stratum_ui::config::FilesConfig) -> Self {
        let mut browser = Self {
            db: MimeDb::load(),
            dir: dir.clone(),
            entries: Vec::new(),
            error: None,
            dir_stamp: None,
            last_check: Instant::now(),
            back: Vec::new(),
            forward: Vec::new(),
            selected: BTreeSet::new(),
            cursor: None,
            anchor: None,
            scroll: 0.0,
            follow: false,
            view: if config.view == "grid" { View::Grid } else { View::List },
            show_hidden: config.show_hidden,
            sort: match config.sort.as_str() {
                "size" => SortKey::Size,
                "modified" => SortKey::Modified,
                "type" => SortKey::Type,
                _ => SortKey::Name,
            },
            descending: config.sort_descending,
            search: TextField::with_capacity(LONG_FIELD),
            editing: Editing::None,
            places: places::load(),
            menu: None,
            menu_rect: None,
            open_with: Vec::new(),
            confirm: None,
            job: None,
            message: None,
            thumbs: HashMap::new(),
            thumb_tx: None,
            thumb_done: Arc::default(),
            waker: None,
            hits: Vec::new(),
            viewport: rect(0.0, 0.0, 0.0, 0.0),
            content_height: 0.0,
            grid_columns: 1,
            chooser: None,
            quit: false,
        };
        browser.load(dir);
        browser
    }

    // ---- The folder -------------------------------------------------

    fn load(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.reload();
        self.selected.clear();
        self.cursor = None;
        self.anchor = None;
        self.scroll = 0.0;
        self.thumbs.clear();
        self.search.clear();
        if matches!(self.editing, Editing::Search | Editing::Rename { .. }) {
            self.editing = Editing::None;
        }
    }

    /// Read the folder again, keeping the selection where it still exists.
    fn reload(&mut self) {
        self.dir_stamp = std::fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
        match entries::list(&self.dir, &self.db) {
            Ok(mut list) => {
                entries::sort(&mut list, self.sort, self.descending);
                if let Some(chooser) = &self.chooser {
                    list.retain(|e| chooser.shows(e));
                }
                self.selected.retain(|p| list.iter().any(|e| &e.path == p));
                self.entries = list;
                self.error = None;
            }
            Err(err) => {
                self.entries.clear();
                self.error = Some(err.to_string());
            }
        }
    }

    /// Go to `dir`, remembering where we were.
    pub fn go(&mut self, dir: PathBuf) {
        if dir == self.dir {
            return;
        }
        if !dir.is_dir() {
            self.say(format!("{} is not a folder", dir.display()), true);
            return;
        }
        self.back.push(self.dir.clone());
        self.forward.clear();
        self.load(dir);
    }

    fn go_back(&mut self) {
        if let Some(dir) = self.back.pop() {
            self.forward.push(self.dir.clone());
            self.load(dir);
        }
    }

    fn go_forward(&mut self) {
        if let Some(dir) = self.forward.pop() {
            self.back.push(self.dir.clone());
            self.load(dir);
        }
    }

    fn go_up(&mut self) {
        if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
            let from = self.dir.clone();
            self.go(parent);
            // Land on the folder we came out of.
            if let Some(i) = self.visible().iter().position(|e| e.path == from) {
                self.select_only(i);
            }
        }
    }

    /// List the folder again (the chooser's filter changed).
    pub fn refresh(&mut self) {
        self.reload();
    }

    pub fn resort(&mut self) {
        entries::sort(&mut self.entries, self.sort, self.descending);
    }

    /// The entries shown, in order.
    pub fn visible(&self) -> Vec<&Entry> {
        let query = if matches!(self.editing, Editing::Search) || !self.search.is_empty() { self.search.text() } else { "" };
        entries::visible(&self.entries, self.show_hidden, query)
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        // In the shown order.
        self.visible().iter().filter(|e| self.selected.contains(&e.path)).map(|e| e.path.clone()).collect()
    }

    fn say(&mut self, text: String, error: bool) {
        self.message = Some((text, error, Instant::now()));
    }

    // ---- Selection ---------------------------------------------------

    fn select_only(&mut self, i: usize) {
        let Some(path) = self.visible().get(i).map(|e| e.path.clone()) else { return };
        self.selected.clear();
        self.selected.insert(path);
        self.cursor = Some(i);
        self.anchor = Some(i);
        self.follow = true;
    }

    fn select_range(&mut self, to: usize) {
        let from = self.anchor.unwrap_or(to);
        let (a, b) = if from <= to { (from, to) } else { (to, from) };
        let paths: Vec<PathBuf> = self.visible()[a..=b].iter().map(|e| e.path.clone()).collect();
        self.selected = paths.into_iter().collect();
        self.cursor = Some(to);
        self.follow = true;
    }

    fn toggle(&mut self, i: usize) {
        let Some(path) = self.visible().get(i).map(|e| e.path.clone()) else { return };
        if !self.selected.remove(&path) {
            self.selected.insert(path);
        }
        self.cursor = Some(i);
        self.anchor = Some(i);
    }

    fn move_cursor(&mut self, by: isize, shift: bool) {
        let count = self.visible().len();
        if count == 0 {
            return;
        }
        let next = match self.cursor {
            None => 0,
            Some(c) => (c as isize + by).clamp(0, count as isize - 1) as usize,
        };
        if shift {
            if self.anchor.is_none() {
                self.anchor = self.cursor.or(Some(next));
            }
            self.select_range(next);
        } else {
            self.select_only(next);
        }
    }

    // ---- Doing things --------------------------------------------------

    /// Open the selection: a folder is entered, files open in their apps.
    fn open_selected(&mut self) {
        let paths = self.selected_paths();
        if let Some(chooser) = &mut self.chooser {
            if let [dir] = paths.as_slice() {
                if dir.is_dir() && !chooser.directory {
                    let dir = dir.clone();
                    self.go(dir);
                    return;
                }
            }
            chooser.accept(&self.dir, &paths);
            return;
        }
        match paths.as_slice() {
            [] => {}
            [one] if one.is_dir() => {
                let one = one.clone();
                self.go(one);
            }
            _ => {
                let files: Vec<PathBuf> = paths.into_iter().filter(|p| !p.is_dir()).collect();
                self.open_files(&files);
            }
        }
    }

    fn open_files(&mut self, files: &[PathBuf]) {
        let (apps, lists) = open::load();
        let terminal = stratum_ui::app::load_config(&match stratum_ui::config::ConfigSource::locate() {
            Ok(s) => s,
            Err(_) => return,
        })
        .map(|c| c.launcher.terminal)
        .unwrap_or_default();
        // Grouped by type, so each opens in its own default app.
        let mut by_app: Vec<(stratum_ui::apps::App, Vec<PathBuf>)> = Vec::new();
        let mut fallback = Vec::new();
        for file in files {
            let mime = self.db.of(file, false);
            match open::apps_for(&mime, &self.db, &apps, &lists).into_iter().next() {
                Some(app) => match by_app.iter_mut().find(|(a, _)| a.id == app.id) {
                    Some((_, list)) => list.push(file.clone()),
                    None => by_app.push((app, vec![file.clone()])),
                },
                None => fallback.push(file.clone()),
            }
        }
        for (app, files) in by_app {
            if let Err(err) = open::open_with(&app, &files, &terminal) {
                self.say(format!("Could not open: {err}"), true);
            }
        }
        for file in fallback {
            if let Err(err) = open::spawn(&["xdg-open".into(), file.to_string_lossy().into_owned()]) {
                self.say(format!("Nothing opens {}: {err}", file.display()), true);
            }
        }
    }

    fn start_rename(&mut self) {
        let paths = self.selected_paths();
        if let [path] = paths.as_slice() {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            self.editing = Editing::Rename { path: path.clone(), field: TextField::with_text(&name), new_folder: false };
        }
    }

    fn finish_rename(&mut self, name: String) {
        let Editing::Rename { path, new_folder, .. } = std::mem::replace(&mut self.editing, Editing::None) else { return };
        let name = name.trim().to_string();
        if name.is_empty() || name.contains('/') || name == "." || name == ".." {
            self.say("That name cannot be used".into(), true);
            return;
        }
        let target = self.dir.join(&name);
        let result = if new_folder {
            std::fs::create_dir(&target)
        } else if target == path {
            Ok(())
        } else if target.exists() {
            Err(std::io::Error::other(format!("{name} exists")))
        } else {
            std::fs::rename(&path, &target)
        };
        match result {
            Ok(()) => {
                self.reload();
                if let Some(i) = self.visible().iter().position(|e| e.path == target) {
                    self.select_only(i);
                }
            }
            Err(err) => self.say(format!("Could not name it {name}: {err}"), true),
        }
    }

    fn new_folder(&mut self) {
        let name = ops::free_name(&self.dir, "New folder");
        self.editing = Editing::Rename { path: self.dir.join(&name), field: TextField::with_text(&name), new_folder: true };
    }

    fn trash_selected(&mut self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        if trash::is_trash(&self.dir) {
            self.ask_delete(paths);
            return;
        }
        let mut failed = Vec::new();
        for path in &paths {
            if let Err(err) = trash::trash(path) {
                failed.push((path.clone(), err));
            }
        }
        self.reload();
        match failed.as_slice() {
            [] => self.say(format!("Moved {} to the trash", count(paths.len(), "item")), false),
            [(path, trash::TrashError::OtherDevice), ..] => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.say(format!("{name} is on another drive; Shift+Delete deletes it for good"), true);
            }
            [(_, err), ..] => self.say(format!("Could not move to the trash: {err}"), true),
        }
    }

    fn ask_delete(&mut self, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        self.confirm = Some(paths);
    }

    fn delete(&mut self, paths: Vec<PathBuf>) {
        let in_trash = trash::is_trash(&self.dir);
        let mut errors = 0;
        for path in &paths {
            let result = match (in_trash, path.file_name()) {
                (true, Some(name)) => trash::delete(&name.to_string_lossy()),
                _ => ops::remove(path),
            };
            if result.is_err() {
                errors += 1;
            }
        }
        self.reload();
        if errors > 0 {
            self.say(format!("{} could not be deleted", count(errors, "item")), true);
        } else {
            self.say(format!("Deleted {}", count(paths.len(), "item")), false);
        }
    }

    fn restore_selected(&mut self) {
        let mut restored = 0;
        for path in self.selected_paths() {
            let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else { continue };
            match trash::restore(&name) {
                Ok(_) => restored += 1,
                Err(err) => self.say(format!("Could not restore {name}: {err}"), true),
            }
        }
        self.reload();
        if restored > 0 {
            self.say(format!("Restored {}", count(restored, "item")), false);
        }
    }

    fn copy_selected(&mut self, cut: bool) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        match clipboard::put(&paths, cut) {
            Ok(()) => self.say(format!("{} {}", if cut { "Cut" } else { "Copied" }, count(paths.len(), "item")), false),
            Err(err) => self.say(format!("Could not copy: {err}"), true),
        }
    }

    fn paste(&mut self) {
        let (paths, cut) = match clipboard::get() {
            Ok(found) => found,
            Err(err) => return self.say(err, true),
        };
        if paths.is_empty() {
            return self.say("Nothing to paste".into(), true);
        }
        let kind = if cut { ops::Kind::Move } else { ops::Kind::Copy };
        self.start_job(kind, paths, self.dir.clone());
        if cut {
            clipboard::forget_cut();
        }
    }

    fn start_job(&mut self, kind: ops::Kind, sources: Vec<PathBuf>, dest: PathBuf) {
        let progress = Arc::new(Mutex::new(ops::Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let report = Arc::clone(&progress);
        let waker = self.waker.clone();
        let last = Mutex::new(Instant::now());
        ops::start(kind, sources, dest, Arc::clone(&cancel), move |p| {
            if let Ok(mut slot) = report.lock() {
                *slot = p.clone();
            }
            // Wake at most ten times a second, and at the end.
            let mut last = last.lock().unwrap_or_else(|e| e.into_inner());
            if p.finished || last.elapsed().as_millis() > 100 {
                *last = Instant::now();
                if let Some(waker) = &waker {
                    waker.wake();
                }
            }
        });
        self.job = Some(Job { label: if kind == ops::Kind::Copy { "Copying" } else { "Moving" }, cancel, progress });
    }

    fn run(&mut self, command: Command, config: &stratum_ui::config::FilesConfig) {
        match command {
            Command::Open => self.open_selected(),
            Command::OpenWith => {
                // A second menu, at the same place, listing the apps.
                let paths = self.selected_paths();
                let Some(first) = paths.first() else { return };
                let mime = self.db.of(first, first.is_dir());
                let (apps, lists) = open::load();
                self.open_with = open::apps_for(&mime, &self.db, &apps, &lists);
                let anchor = self.menu.as_ref().map(|m| m.anchor).unwrap_or(rect(0.0, 0.0, 0.0, 0.0));
                let items: Vec<(String, Command)> =
                    self.open_with.iter().enumerate().map(|(i, a)| (a.name.clone(), Command::OpenApp(i))).collect();
                if items.is_empty() {
                    self.say(format!("No app opens {mime}"), true);
                } else {
                    self.menu = Some(Menu { anchor, items, scroll: None, fresh: true });
                }
                return;
            }
            Command::OpenApp(i) => {
                let files = self.selected_paths();
                if let Some(app) = self.open_with.get(i).cloned() {
                    let terminal = stratum_ui::config::ConfigSource::locate()
                        .ok()
                        .and_then(|s| stratum_ui::app::load_config(&s).ok())
                        .map(|c| c.launcher.terminal)
                        .unwrap_or_default();
                    if let Err(err) = open::open_with(&app, &files, &terminal) {
                        self.say(format!("Could not open: {err}"), true);
                    }
                }
            }
            Command::Cut => self.copy_selected(true),
            Command::Copy => self.copy_selected(false),
            Command::Paste => self.paste(),
            Command::Rename => self.start_rename(),
            Command::Trash => self.trash_selected(),
            Command::Delete => {
                let paths = self.selected_paths();
                if config.confirm_delete {
                    self.ask_delete(paths);
                } else {
                    self.delete(paths);
                }
            }
            Command::Restore => self.restore_selected(),
            Command::EmptyTrash => {
                let all: Vec<PathBuf> = self.entries.iter().map(|e| e.path.clone()).collect();
                self.ask_delete(all);
            }
            Command::NewFolder => self.new_folder(),
            Command::CopyPath => {
                let text: Vec<String> = self.selected_paths().iter().map(|p| p.display().to_string()).collect();
                let text = if text.is_empty() { self.dir.display().to_string() } else { text.join("\n") };
                if let Err(err) = clipboard::put_text(&text) {
                    self.say(format!("Could not copy: {err}"), true);
                }
            }
            Command::TerminalHere => {
                let mut argv = config.terminal_here.clone();
                argv.push(self.dir.display().to_string());
                if let Err(err) = open::spawn(&argv) {
                    self.say(format!("Could not open a terminal: {err}"), true);
                }
            }
            Command::ToggleHidden => self.show_hidden = !self.show_hidden,
            Command::SelectAll => self.selected = self.visible().iter().map(|e| e.path.clone()).collect(),
        }
        self.menu = None;
    }

    fn context_menu(&mut self, at: (f32, f32), on_entry: bool, width: f32) {
        let in_trash = trash::is_trash(&self.dir);
        let mut items: Vec<(&str, Command)> = Vec::new();
        if on_entry {
            if in_trash {
                items.extend([("Restore", Command::Restore), ("Delete for good", Command::Delete)]);
            } else {
                items.extend([("Open", Command::Open), ("Open with…", Command::OpenWith)]);
                items.extend([("Cut", Command::Cut), ("Copy", Command::Copy)]);
                if self.selected.len() == 1 {
                    items.push(("Rename", Command::Rename));
                }
                items.extend([("Copy path", Command::CopyPath), ("Move to the trash", Command::Trash), ("Delete for good", Command::Delete)]);
            }
        } else if in_trash {
            items.push(("Empty the trash", Command::EmptyTrash));
        } else {
            items.extend([("New folder", Command::NewFolder), ("Paste", Command::Paste), ("Select all", Command::SelectAll)]);
            items.extend([("Open a terminal here", Command::TerminalHere), ("Copy path", Command::CopyPath)]);
            items.push((if self.show_hidden { "Hide hidden files" } else { "Show hidden files" }, Command::ToggleHidden));
        }
        self.menu = Some(Menu {
            anchor: rect(at.0, at.1, width, 0.0),
            items: items.into_iter().map(|(l, c)| (l.to_string(), c)).collect(),
            scroll: None,
            fresh: true,
        });
    }

    // ---- Thumbnails ----------------------------------------------------

    fn want_thumbnail(&mut self, entry: &Entry, size: u32, config: &stratum_ui::config::FilesConfig) -> Option<IconImage> {
        if !config.thumbnails || !matches!(entry.mime.as_str(), "image/png" | "image/jpeg") || entry.size > config.thumbnail_max_mb * 1024 * 1024 {
            return None;
        }
        if let Some(found) = self.thumbs.get(&entry.path) {
            return found.clone();
        }
        let tx = self.thumb_tx.get_or_insert_with(|| {
            let (tx, rx) = std::sync::mpsc::channel::<(PathBuf, u32)>();
            let done = Arc::clone(&self.thumb_done);
            let waker = self.waker.clone();
            std::thread::Builder::new()
                .name("stratum-thumbs".into())
                .spawn(move || {
                    for (path, size) in rx {
                        let image = thumbnail(&path, size);
                        if let Ok(mut done) = done.lock() {
                            done.push((path, image));
                        }
                        if let Some(w) = &waker {
                            w.wake();
                        }
                    }
                })
                .ok();
            tx
        });
        // Asked once: `None` marks it pending until the thread answers.
        self.thumbs.insert(entry.path.clone(), None);
        let _ = tx.send((entry.path.clone(), size));
        None
    }

    // ---- Painting --------------------------------------------------------

    fn icon(&mut self, ui: &mut AppUi, entry: &Entry, size: f32, config: &stratum_ui::config::FilesConfig) -> Option<IconImage> {
        let physical = (size * 2.0) as u32;
        if let Some(thumb) = self.want_thumbnail(entry, physical, config) {
            return Some(thumb);
        }
        let names = if entry.is_dir { folder_icons(&entry.path) } else { self.db.icon_names(&entry.mime) };
        names.iter().find_map(|n| ui.icons.load_icon(n, size as u32, 2))
    }

    fn paint_toolbar(&mut self, ui: &mut AppUi, list: &mut DisplayList, bar: Rect, input: &InputState, clicked: bool) {
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        let y = bar.y + (bar.height - m.button) * 0.5;
        let mut x = bar.x + m.padding;
        for (glyph, enabled, action) in [(BACK, !self.back.is_empty(), 0), (FORWARD, !self.forward.is_empty(), 1), (UP, self.dir.parent().is_some(), 2)] {
            let hit = layout::icon_button(ui, list, x, y, glyph, false, theme.primary, input, clicked && enabled);
            if !enabled {
                // Dim the disabled ones.
                list.push_rect(Rect { color: theme.bg_main.with_alpha(0.5), ..rect(x, y, m.button, m.button) });
            }
            if hit {
                match action {
                    0 => self.go_back(),
                    1 => self.go_forward(),
                    _ => self.go_up(),
                }
            }
            x += m.button + m.gap * 0.5;
        }

        // At the right: view, hidden files, search.
        let mut right = bar.x + bar.width - m.padding;
        for (glyph, on, action) in [(GRID, self.view == View::Grid, 0), (LIST, self.view == View::List, 1), (HIDDEN, self.show_hidden, 2)] {
            right -= m.button;
            if layout::icon_button(ui, list, right, y, glyph, on, theme.primary, input, clicked) {
                match action {
                    0 => self.view = View::Grid,
                    1 => self.view = View::List,
                    _ => self.show_hidden = !self.show_hidden,
                }
            }
            right -= m.gap * 0.5;
        }
        right -= m.gap;
        let search = rect(right - m.search_width, y, m.search_width, m.button);
        let searching = matches!(self.editing, Editing::Search);
        ui::paint_frame(list, Rect { radius: m.radius, ..search }, if searching { 1.5 } else { 0.0 }, theme.primary, theme.surface_high);
        let glyph = rect(search.x, search.y, m.button, m.button);
        ui::paint_icon_centered(ui, list, SEARCH, glyph, m.text, theme.text_muted);
        let field = rect(search.x + m.button, search.y, search.width - m.button, search.height);
        let style = FieldStyle { text: m.text, padding: 0.0, caret_width: 1.5, masked: false, placeholder: Some("Search this folder") };
        self.search.paint(ui, list, field, style, !searching);
        if clicked && layout::hovered(input, search) {
            self.editing = Editing::Search;
        }

        // The path between them: breadcrumbs, or a field while editing.
        let path_area = rect(x + m.gap, y, (search.x - m.gap * 2.0 - x).max(0.0), m.button);
        match &mut self.editing {
            Editing::Path(field) => {
                ui::paint_frame(list, Rect { radius: m.radius, ..path_area }, 1.5, theme.primary, theme.surface_high);
                let style = FieldStyle { text: m.text, padding: m.gap, caret_width: 1.5, masked: false, placeholder: None };
                field.paint(ui, list, path_area, style, false);
            }
            _ => {
                if let Some(dir) = self.paint_breadcrumbs(ui, list, path_area, input, clicked) {
                    self.go(dir);
                } else if pointer_double(input, path_area, clicked) {
                    self.editing = Editing::Path(TextField::with_text_capacity(&self.dir.display().to_string(), LONG_FIELD));
                }
            }
        }
    }

    /// The path as buttons, the last ones when it does not fit. Returns a
    /// folder clicked.
    fn paint_breadcrumbs(&self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, input: &InputState, clicked: bool) -> Option<PathBuf> {
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        let mut parts: Vec<(String, PathBuf)> = Vec::new();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut acc = PathBuf::new();
        for comp in self.dir.components() {
            acc.push(comp);
            let name = match comp {
                std::path::Component::RootDir => "/".to_string(),
                other => other.as_os_str().to_string_lossy().into_owned(),
            };
            parts.push((name, acc.clone()));
        }
        // Home stands for everything above it.
        if let Some(home) = &home {
            if let Some(i) = parts.iter().position(|(_, p)| p == home) {
                parts.drain(..i);
                parts[0].0 = "Home".into();
            }
        }
        // The trash is a place, not a path.
        if trash::is_trash(&self.dir) {
            parts = vec![("Trash".to_string(), self.dir.clone())];
        }
        let sep = "\u{203a}";
        let widths: Vec<f32> = parts.iter().map(|(n, _)| ui.measure(n, m.text) + m.gap * 1.5).collect();
        let sep_w = ui.measure(sep, m.text) + m.gap;
        // Drop from the left until it fits.
        let mut first = 0;
        while first + 1 < parts.len() && widths[first..].iter().sum::<f32>() + sep_w * (parts.len() - first) as f32 > area.width {
            first += 1;
        }
        let mut x = area.x;
        let mut hit = None;
        for (i, ((name, path), w)) in parts.iter().zip(&widths).enumerate().skip(first) {
            if i > first || first > 0 {
                ui::paint_text_vcentered(ui, list, if i == first { "\u{2026}" } else { sep }, x, area, m.text, 400, theme.text_muted);
                x += sep_w;
            }
            let cell = rect(x, area.y, *w, area.height);
            let last = i + 1 == parts.len();
            if layout::hovered(input, cell) && !last {
                list.push_rect(Rect { radius: m.radius, color: theme.bg_hover, ..cell });
                if clicked {
                    hit = Some(path.clone());
                }
            }
            let weight = if last { ui.config.metrics.typography.weight_bold } else { 400 };
            ui::paint_text_vcentered(ui, list, name, x + m.gap * 0.75, area, m.text, weight, if last { theme.text_main } else { theme.text_muted });
            x += w;
        }
        hit
    }

    fn paint_sidebar(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, input: &InputState, clicked: bool) {
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        list.push_rect(Rect { color: theme.bg_widget, ..area });
        let mut y = area.y + m.padding;
        let mut section = None;
        let mut go = None;
        for (s, place) in self.places.clone() {
            if section != Some(s) {
                let label = match s {
                    Section::Places => "Places",
                    Section::Bookmarks => "Bookmarks",
                    Section::Drives => "Drives",
                };
                if section.is_some() {
                    y += m.gap;
                }
                let row = rect(area.x + m.padding, y, area.width - m.padding * 2.0, m.section_height);
                ui::paint_text_vcentered(ui, list, label, row.x + m.gap, row, m.detail_text, 400, theme.text_muted);
                y += m.section_height;
                section = Some(s);
            }
            let row = rect(area.x + m.padding, y, area.width - m.padding * 2.0, m.sidebar_row);
            let current = place.path == self.dir;
            if current || layout::hovered(input, row) {
                let color = if current { layout::mix(theme.bg_widget, theme.primary, 0.25) } else { theme.bg_hover };
                list.push_rect(Rect { radius: m.radius, color, ..row });
            }
            let icon_box = rect(row.x + m.gap, row.y + (row.height - m.sidebar_icon) * 0.5, m.sidebar_icon, m.sidebar_icon);
            match ui.icons.load_icon(place.icon, m.sidebar_icon as u32, 2).or_else(|| ui.icons.load_icon("folder", m.sidebar_icon as u32, 2)) {
                Some(image) => push_image(list, image, icon_box),
                None => ui::paint_icon_centered(ui, list, FOLDER, icon_box, m.sidebar_icon, theme.text_muted),
            }
            let text_x = icon_box.x + icon_box.width + m.gap;
            let name = ui::elide(ui, &place.name, m.text, 400, row.x + row.width - text_x - m.gap);
            ui::paint_text_vcentered(ui, list, &name, text_x, row, m.text, 400, if current { theme.primary } else { theme.text_main });
            if clicked && layout::hovered(input, row) {
                go = Some(place.path.clone());
            }
            y += m.sidebar_row;
        }
        if let Some(dir) = go {
            if trash::is_trash(&dir) {
                let _ = std::fs::create_dir_all(&dir);
            }
            self.go(dir);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn paint_entries(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, input: &InputState, pointer: &Pointer, clicked: bool, config: &stratum_ui::config::FilesConfig) {
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        let visible: Vec<Entry> = self.visible().into_iter().cloned().collect();
        let header_h = if self.view == View::List { m.header_height } else { 0.0 };
        if self.view == View::List {
            self.paint_header(ui, list, rect(area.x, area.y, area.width, header_h), input, clicked);
        }
        let viewport = rect(area.x, area.y + header_h, area.width, area.height - header_h);
        self.viewport = viewport;
        self.hits.clear();

        if let Some(error) = &self.error {
            let text = format!("Cannot open this folder: {error}");
            ui::paint_text_centered(ui, list, &text, rect(viewport.x, viewport.y, viewport.width, m.row_height * 2.0), m.text, theme.error);
            return;
        }
        if visible.is_empty() {
            let text = if self.search.is_empty() { "This folder is empty" } else { "Nothing matches" };
            ui::paint_text_centered(ui, list, text, rect(viewport.x, viewport.y, viewport.width, m.row_height * 2.0), m.text, theme.text_muted);
        }

        // Positions, then the scroll that keeps the cursor in view.
        let (columns, cell_w, cell_h) = match self.view {
            View::List => (1, viewport.width, m.row_height),
            View::Grid => {
                let columns = ((viewport.width - m.padding) / m.cell_width).floor().max(1.0) as usize;
                (columns, (viewport.width - m.padding) / columns as f32, m.cell_height)
            }
        };
        self.grid_columns = columns;
        let rows = visible.len().div_ceil(columns);
        self.content_height = rows as f32 * cell_h + m.padding;
        if self.follow {
            if let Some(c) = self.cursor {
                let top = (c / columns) as f32 * cell_h;
                if top < self.scroll {
                    self.scroll = top;
                } else if top + cell_h > self.scroll + viewport.height {
                    self.scroll = top + cell_h - viewport.height;
                }
            }
            self.follow = false;
        }
        self.scroll = self.scroll.clamp(0.0, (self.content_height - viewport.height).max(0.0));

        list.push_clip(viewport);
        let first_row = (self.scroll / cell_h).floor() as usize;
        let last_row = ((self.scroll + viewport.height) / cell_h).ceil() as usize;
        let renaming = match &self.editing {
            Editing::Rename { path, new_folder: false, .. } => Some(path.clone()),
            _ => None,
        };
        let mut clicked_entry = None;
        for (i, entry) in visible.iter().enumerate().skip(first_row * columns).take((last_row - first_row + 1) * columns) {
            let (row, col) = (i / columns, i % columns);
            let cell = match self.view {
                View::List => rect(viewport.x, viewport.y + row as f32 * cell_h - self.scroll, cell_w, cell_h),
                View::Grid => rect(viewport.x + m.padding * 0.5 + col as f32 * cell_w, viewport.y + m.padding * 0.5 + row as f32 * cell_h - self.scroll, cell_w, cell_h),
            };
            self.hits.push((cell, i));
            let selected = self.selected.contains(&entry.path);
            let hot = layout::hovered(input, cell) && layout::hovered(input, viewport);
            let inset = Rect { x: cell.x + 2.0, y: cell.y + 1.0, width: cell.width - 4.0, height: cell.height - 2.0, radius: m.radius, color: Color::TRANSPARENT };
            if selected {
                list.push_rect(Rect { color: layout::mix(theme.bg_main, theme.primary, 0.3), ..inset });
            } else if hot {
                list.push_rect(Rect { color: theme.bg_hover, ..inset });
            }
            let dim = if entry.hidden() { 0.6 } else { 1.0 };
            let text_color = theme.text_main.with_alpha(theme.text_main.a * dim);
            match self.view {
                View::List => {
                    let icon_box = rect(cell.x + m.padding, cell.y + (cell.height - m.row_icon) * 0.5, m.row_icon, m.row_icon);
                    match self.icon(ui, entry, m.row_icon, config) {
                        Some(image) => {
                            let (image, at) = fit(image, icon_box);
                            push_image(list, image, at);
                        }
                        None => ui::paint_icon_centered(ui, list, if entry.is_dir { FOLDER } else { FILE }, icon_box, m.row_icon, theme.text_muted),
                    }
                    let name_x = icon_box.x + icon_box.width + m.gap;
                    let size_x = cell.x + cell.width - m.modified_column - m.size_column;
                    let name_w = size_x - name_x - m.gap;
                    if renaming.as_ref() == Some(&entry.path) {
                        self.paint_rename(ui, list, rect(name_x - m.gap * 0.5, cell.y + 3.0, name_w, cell.height - 6.0));
                    } else {
                        let name = ui::elide(ui, &entry.name, m.text, 400, name_w);
                        ui::paint_text_vcentered(ui, list, &name, name_x, cell, m.text, 400, text_color);
                    }
                    let size = if entry.is_dir { String::new() } else { entries::format_size(entry.size) };
                    let size_w = ui.measure(&size, m.detail_text);
                    ui::paint_text_vcentered(ui, list, &size, size_x + m.size_column - m.gap - size_w, cell, m.detail_text, 400, theme.text_muted);
                    let when = entry.modified.map(|t| entries::format_time(t, chrono::Local::now())).unwrap_or_default();
                    ui::paint_text_vcentered(ui, list, &when, size_x + m.size_column + m.gap, cell, m.detail_text, 400, theme.text_muted);
                }
                View::Grid => {
                    let icon_box = rect(cell.x + (cell.width - m.cell_icon) * 0.5, cell.y + m.gap, m.cell_icon, m.cell_icon);
                    match self.icon(ui, entry, m.cell_icon, config) {
                        Some(image) => {
                            let (image, at) = fit(image, icon_box);
                            push_image(list, image, at);
                        }
                        None => ui::paint_icon_centered(ui, list, if entry.is_dir { FOLDER } else { FILE }, icon_box, m.cell_icon * 0.8, theme.text_muted),
                    }
                    let text_area = rect(cell.x + m.gap * 0.5, icon_box.y + icon_box.height + m.gap * 0.5, cell.width - m.gap, cell.y + cell.height - icon_box.y - icon_box.height - m.gap);
                    if renaming.as_ref() == Some(&entry.path) {
                        self.paint_rename(ui, list, rect(text_area.x, text_area.y, text_area.width, m.row_height * 0.8));
                    } else {
                        let lines = ui::wrap_text(ui, &entry.name, m.detail_text, 400, text_area.width, 2);
                        let line_h = m.detail_text * 1.35;
                        for (n, line) in lines.iter().enumerate() {
                            let line = ui::elide(ui, line, m.detail_text, 400, text_area.width);
                            let row = rect(text_area.x, text_area.y + n as f32 * line_h, text_area.width, line_h);
                            ui::paint_text_centered(ui, list, &line, row, m.detail_text, text_color);
                        }
                    }
                }
            }
            if hot && (clicked || pointer.secondary) {
                clicked_entry = Some(i);
            }
        }
        // A folder being made shows as a row of its own at the end.
        if let Editing::Rename { new_folder: true, .. } = &self.editing {
            let y = viewport.y + rows as f32 * cell_h - self.scroll;
            let row = rect(viewport.x + m.padding + m.row_icon + m.gap * 0.5, y + 3.0, (viewport.width * 0.5).max(160.0), m.row_height - 6.0);
            ui::paint_icon_centered(ui, list, FOLDER, rect(viewport.x + m.padding, y, m.row_icon, m.row_height), m.row_icon, theme.primary);
            self.paint_rename(ui, list, row);
        }
        list.pop_clip();

        // Clicks.
        let over = layout::hovered(input, viewport);
        if let Some(i) = clicked_entry {
            if pointer.secondary {
                if !self.selected.contains(&visible[i].path) {
                    self.select_only(i);
                }
                if let Some(p) = input.pointer {
                    self.context_menu(p, true, m.menu_width);
                }
            } else if pointer.ctrl {
                self.toggle(i);
            } else if pointer.shift {
                self.select_range(i);
            } else if pointer.double {
                self.select_only(i);
                self.open_selected();
            } else {
                self.select_only(i);
            }
            if matches!(self.editing, Editing::Rename { .. } | Editing::Path(_)) {
                self.editing = Editing::None;
            }
        } else if over && (clicked || pointer.secondary) {
            if !pointer.ctrl {
                self.selected.clear();
                self.cursor = None;
            }
            if pointer.secondary {
                if let Some(p) = input.pointer {
                    self.context_menu(p, false, m.menu_width);
                }
            }
            if matches!(self.editing, Editing::Rename { new_folder: false, .. } | Editing::Path(_)) {
                self.editing = Editing::None;
            }
        }
    }

    fn paint_header(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, input: &InputState, clicked: bool) {
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        let size_x = area.x + area.width - m.modified_column - m.size_column;
        let columns = [
            ("Name", SortKey::Name, rect(area.x, area.y, size_x - area.x, area.height), area.x + m.padding + m.row_icon + m.gap),
            ("Size", SortKey::Size, rect(size_x, area.y, m.size_column, area.height), size_x + m.gap),
            ("Modified", SortKey::Modified, rect(size_x + m.size_column, area.y, m.modified_column, area.height), size_x + m.size_column + m.gap),
        ];
        for (label, key, cell, text_x) in columns {
            if layout::hovered(input, cell) {
                list.push_rect(Rect { color: theme.bg_hover, ..cell });
                if clicked {
                    if self.sort == key {
                        self.descending = !self.descending;
                    } else {
                        self.sort = key;
                        self.descending = false;
                    }
                    self.resort();
                }
            }
            ui::paint_text_vcentered(ui, list, label, text_x, cell, m.detail_text, 400, theme.text_muted);
            if self.sort == key {
                let w = ui.measure(label, m.detail_text);
                let glyph = if self.descending { SORT_DOWN } else { SORT_UP };
                ui::paint_icon_centered(ui, list, glyph, rect(text_x + w + 2.0, cell.y, m.detail_text * 1.4, cell.height), m.detail_text, theme.text_muted);
            }
        }
        list.push_rect(Rect { y: area.y + area.height - 1.0, height: 1.0, color: theme.border_inactive, ..area });
    }

    fn paint_rename(&self, ui: &mut AppUi, list: &mut DisplayList, area: Rect) {
        let Editing::Rename { field, .. } = &self.editing else { return };
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        ui::paint_frame(list, Rect { radius: m.radius * 0.6, ..area }, 1.5, theme.primary, theme.surface_high);
        let style = FieldStyle { text: m.text, padding: m.gap * 0.5, caret_width: 1.5, masked: false, placeholder: None };
        field.paint(ui, list, area, style, false);
    }

    fn paint_status(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, input: &InputState, clicked: bool) {
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        list.push_rect(Rect { height: 1.0, color: theme.border_inactive, ..area });
        let shown = self.visible().len();
        let mut text = count(shown, "item");
        let selected = self.selected_paths();
        if !selected.is_empty() {
            let bytes: u64 = self.entries.iter().filter(|e| self.selected.contains(&e.path) && !e.is_dir).map(|e| e.size).sum();
            text.push_str(&format!(" \u{b7} {} selected", selected.len()));
            if bytes > 0 {
                text.push_str(&format!(" ({})", entries::format_size(bytes)));
            }
        }
        let mut color = theme.text_muted;
        if let Some(job) = &self.job {
            let p = job.progress.lock().map(|p| p.clone()).unwrap_or_default();
            let pct = if p.total > 0 { p.done * 100 / p.total } else { 0 };
            text = format!("{} {} \u{b7} {pct}% of {}", job.label, p.current, entries::format_size(p.total));
            // A thin bar along the top of the status line.
            let bar = rect(area.x, area.y, area.width * (pct as f32 / 100.0), 2.0);
            list.push_rect(Rect { color: theme.primary, ..bar });
        } else if let Some((message, error, at)) = &self.message {
            if at.elapsed().as_secs_f32() < MESSAGE_SECONDS {
                text = message.clone();
                if *error {
                    color = theme.error;
                }
            }
        }
        let text = ui::elide(ui, &text, m.detail_text, 400, area.width - m.padding * 2.0 - m.menu_width);
        ui::paint_text_vcentered(ui, list, &text, area.x + m.padding, area, m.detail_text, 400, color);
        // Cancel a job, or empty the trash, at the right.
        let label = if self.job.is_some() {
            Some("Cancel")
        } else if trash::is_trash(&self.dir) && !self.entries.is_empty() {
            Some("Empty the trash")
        } else {
            None
        };
        if let Some(label) = label {
            let w = layout::text_button_width(ui, label);
            let h = ui.config.metrics.page.badge;
            if layout::text_button(ui, list, area.x + area.width - m.padding - w, area.y + (area.height - h) * 0.5, label, true, input, clicked) {
                match &self.job {
                    Some(job) => job.cancel.store(true, std::sync::atomic::Ordering::Relaxed),
                    None => self.run(Command::EmptyTrash, &ui.config.files.clone()),
                }
            }
        }
    }

    fn paint_confirm(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, input: &InputState, clicked: bool) {
        let Some(paths) = self.confirm.clone() else { return };
        let m = ui.config.metrics.files.clone();
        let page = ui.config.metrics.page.clone();
        let theme = ui.theme.clone();
        list.push_layer();
        ui::paint_scrim(list, area, 0.4);
        let h = page.card_padding * 3.0 + page.badge * 2.0;
        let card = rect(area.x + (area.width - m.dialog_width) * 0.5, area.y + (area.height - h) * 0.4, m.dialog_width, h);
        layout::card(ui, list, card, Card::Plain);
        let title = match paths.as_slice() {
            [one] => format!("Delete {} for good?", one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()),
            many => format!("Delete {} for good?", count(many.len(), "item")),
        };
        let line = rect(card.x + page.card_padding, card.y + page.card_padding, card.width - page.card_padding * 2.0, page.badge);
        layout::two_lines(ui, list, line, &title, "This cannot be undone.", theme.text_muted);
        let y = card.y + card.height - page.card_padding - page.badge;
        let (_, hit) = layout::text_buttons(ui, list, card.x + card.width - page.card_padding, y, &["Cancel", "Delete"], true, input, clicked);
        match hit {
            Some(1) => {
                self.confirm = None;
                self.delete(paths);
            }
            Some(_) => self.confirm = None,
            None if clicked && !layout::hovered(input, card) => self.confirm = None,
            None => {}
        }
    }

    fn take_thumbnails(&mut self) -> bool {
        let done: Vec<_> = self.thumb_done.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default();
        let any = !done.is_empty();
        for (path, image) in done {
            // Only for this folder's files.
            if path.parent() == Some(self.dir.as_path()) {
                self.thumbs.insert(path, image);
            }
        }
        any
    }

    /// A finished copy or move, and a folder changed on disk.
    fn take_job(&mut self) -> bool {
        let finished = self.job.as_ref().and_then(|j| j.progress.lock().ok().filter(|p| p.finished).map(|p| p.clone()));
        let Some(p) = finished else { return self.job.is_some() };
        let label = self.job.take().map(|j| j.label).unwrap_or("");
        self.reload();
        match p.errors.as_slice() {
            [] => self.say(format!("{} done", label), false),
            [one] => self.say(one.clone(), true),
            many => self.say(format!("{} problems: {}", many.len(), many[0]), true),
        }
        true
    }

    fn check_dir(&mut self) -> bool {
        if self.last_check.elapsed().as_millis() < 1000 {
            return false;
        }
        self.last_check = Instant::now();
        let stamp = std::fs::metadata(&self.dir).and_then(|m| m.modified()).ok();
        if stamp != self.dir_stamp {
            self.reload();
            return true;
        }
        false
    }
}

/// The icons a folder has: special folders their own, else a plain one.
fn folder_icons(path: &Path) -> Vec<String> {
    let name = path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let special = match name.as_str() {
        "desktop" => Some("user-desktop"),
        "documents" => Some("folder-documents"),
        "downloads" => Some("folder-download"),
        "music" => Some("folder-music"),
        "pictures" => Some("folder-pictures"),
        "videos" => Some("folder-videos"),
        "templates" => Some("folder-templates"),
        "public" => Some("folder-publicshare"),
        _ => None,
    };
    special.into_iter().chain(["folder"]).map(str::to_string).collect()
}

/// A picture scaled down to fit `size` pixels, for a thumbnail.
fn thumbnail(path: &Path, size: u32) -> Option<IconImage> {
    let image = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?.decode().ok()?;
    let image = image.thumbnail(size, size).to_rgba8();
    let (width, height) = image.dimensions();
    let mut pixels = image.into_raw();
    for px in pixels.chunks_exact_mut(4) {
        let a = u16::from(px[3]);
        for c in &mut px[..3] {
            *c = (u16::from(*c) * a / 255) as u8;
        }
    }
    Some(IconImage { width, height, pixels: pixels.into() })
}

/// `image` placed in `area` with its aspect ratio kept.
fn fit(image: IconImage, area: Rect) -> (IconImage, Rect) {
    let (w, h) = (image.width as f32, image.height as f32);
    let s = (area.width / w).min(area.height / h);
    let (fw, fh) = (w * s, h * s);
    (image, rect(area.x + (area.width - fw) * 0.5, area.y + (area.height - fh) * 0.5, fw, fh))
}

fn push_image(list: &mut DisplayList, image: IconImage, at: Rect) {
    list.push_image(Image {
        width: image.width,
        height: image.height,
        pixels: image.pixels,
        x: at.x,
        y: at.y,
        width_logical: at.width,
        height_logical: at.height,
    });
}

fn pointer_double(input: &InputState, area: Rect, clicked: bool) -> bool {
    // The breadcrumbs' empty space: a click there edits the path.
    clicked && layout::hovered(input, area)
}

pub fn count(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

impl App for Browser {
    fn start(&mut self, waker: Waker) {
        self.waker = Some(waker);
    }

    fn title(&self) -> String {
        if let Some(chooser) = &self.chooser {
            return chooser.title.clone();
        }
        let name = self.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into());
        if trash::is_trash(&self.dir) { "Trash".into() } else { name }
    }

    fn paint(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, pointer: &Pointer) {
        let m: FilesMetrics = ui.config.metrics.files.clone();
        let config = ui.config.files.clone();
        // An open menu or dialog takes the pointer from what it covers.
        let covered = self.confirm.is_some() || self.menu_rect.is_some_and(|r| layout::hovered(&pointer.input, r));
        let masked = InputState { pointer: if covered { None } else { pointer.input.pointer }, pressed: pointer.input.pressed };
        let input = &masked;
        let clicked = pointer.clicked && !covered;
        let inner = Pointer { input: masked.clone(), clicked, secondary: pointer.secondary && !covered, ..pointer.clone() };

        let chooser_h = if self.chooser.is_some() { m.chooser_height } else { 0.0 };
        let toolbar = rect(area.x, area.y, area.width, m.toolbar_height);
        let status = rect(area.x, area.y + area.height - m.status_height - chooser_h, area.width, m.status_height);
        let body_y = toolbar.y + toolbar.height;
        let body_h = status.y - body_y;
        let sidebar = rect(area.x, body_y, m.sidebar_width.min(area.width * 0.4), body_h);
        let main = rect(sidebar.x + sidebar.width, body_y, area.width - sidebar.width, body_h);

        self.paint_sidebar(ui, list, sidebar, input, clicked);
        self.paint_entries(ui, list, main, input, &inner, clicked, &config);
        self.paint_status(ui, list, status, input, clicked);
        if let Some(mut chooser) = self.chooser.take() {
            let bar = rect(area.x, status.y + status.height, area.width, chooser_h);
            let selected = self.selected_paths();
            chooser.paint(ui, list, bar, input, clicked, &self.dir, &selected);
            self.chooser = Some(chooser);
        }
        list.push_rect(Rect { y: toolbar.y + toolbar.height - 1.0, height: 1.0, color: ui.theme.border_inactive, ..toolbar });
        self.paint_toolbar(ui, list, toolbar, input, clicked);
        self.paint_confirm(ui, list, area, &pointer.input, pointer.clicked);

        // The menu, over everything.
        self.menu_rect = None;
        if let Some(mut menu) = self.menu.take() {
            list.push_layer();
            let labels: Vec<String> = menu.items.iter().map(|(l, _)| l.clone()).collect();
            // Open upwards or leftwards where it would leave the window.
            let height = labels.len() as f32 * ui.config.metrics.page.editor_row_height + 8.0;
            let mut anchor = menu.anchor;
            if anchor.y + height > area.y + area.height {
                anchor.y = (anchor.y - height).max(area.y);
            }
            if anchor.x + anchor.width > area.x + area.width {
                anchor.x = area.x + area.width - anchor.width;
            }
            list.push_clip(area);
            let (card, hit) = layout::dropdown_list(ui, list, anchor, &labels, None, &mut menu.scroll, &pointer.input, pointer.clicked);
            list.pop_clip();
            self.menu_rect = Some(card);
            let command = hit.map(|i| menu.items[i].1.clone());
            let outside = !std::mem::take(&mut menu.fresh) && (pointer.clicked || pointer.secondary) && !layout::hovered(&pointer.input, card);
            if !outside {
                self.menu = Some(menu);
            }
            if let Some(command) = command {
                self.menu = None;
                self.run(command, &config);
            }
        }
    }

    fn key(&mut self, ui: &mut AppUi, key: &Key) -> Flow {
        let config = ui.config.files.clone();
        if self.confirm.is_some() {
            match key.keysym {
                Keysym::Return | Keysym::KP_Enter => {
                    if let Some(paths) = self.confirm.take() {
                        self.delete(paths);
                    }
                }
                Keysym::Escape => self.confirm = None,
                _ => {}
            }
            return Flow::Continue;
        }
        if self.menu.is_some() && key.keysym == Keysym::Escape {
            self.menu = None;
            return Flow::Continue;
        }
        // The chooser's name field takes typing first.
        if let Some(chooser) = &mut self.chooser {
            if let Some(done) = chooser.key(key) {
                if done {
                    let selected = self.selected_paths();
                    let dir = self.dir.clone();
                    if let Some(chooser) = &mut self.chooser {
                        chooser.accept(&dir, &selected);
                    }
                }
                return Flow::Continue;
            }
        }
        match &mut self.editing {
            Editing::Rename { field, .. } => {
                let typed = field.text().to_string();
                match field.key(key.keysym, key.text.as_deref(), key.ctrl) {
                    FieldKey::Submit(_) => self.finish_rename(typed),
                    FieldKey::Escape => self.editing = Editing::None,
                    _ => {}
                }
                return Flow::Continue;
            }
            Editing::Path(field) => {
                let typed = field.text().to_string();
                match field.key(key.keysym, key.text.as_deref(), key.ctrl) {
                    FieldKey::Submit(_) => {
                        self.editing = Editing::None;
                        let path = expand_home(typed.trim());
                        self.go(path);
                    }
                    FieldKey::Escape => self.editing = Editing::None,
                    _ => {}
                }
                return Flow::Continue;
            }
            Editing::Search => {
                let moves = matches!(key.keysym, Keysym::Up | Keysym::Down | Keysym::Page_Up | Keysym::Page_Down | Keysym::Return | Keysym::KP_Enter);
                if !moves && !key.ctrl {
                    match self.search.key(key.keysym, key.text.as_deref(), key.ctrl) {
                        FieldKey::Escape => {
                            if self.search.is_empty() {
                                self.editing = Editing::None;
                            } else {
                                self.search.clear();
                            }
                            return Flow::Continue;
                        }
                        FieldKey::Changed => {
                            // The first match is where Return goes.
                            if !self.visible().is_empty() {
                                self.select_only(0);
                            }
                            return Flow::Continue;
                        }
                        // Keys the field does not use (Delete, F2) act on
                        // the selection as usual.
                        _ => {}
                    }
                }
            }
            Editing::None => {}
        }

        let columns = if self.view == View::Grid { self.grid_columns as isize } else { 1 };
        let page = ((self.viewport.height / if self.view == View::Grid { ui.config.metrics.files.cell_height } else { ui.config.metrics.files.row_height }) as isize).max(1) * columns;
        match key.keysym {
            Keysym::Down => self.move_cursor(columns, key.shift),
            Keysym::Up if key.alt => self.go_up(),
            Keysym::Up => self.move_cursor(-columns, key.shift),
            Keysym::Left if key.alt => self.go_back(),
            Keysym::Right if key.alt => self.go_forward(),
            Keysym::Right if self.view == View::Grid => self.move_cursor(1, key.shift),
            Keysym::Left if self.view == View::Grid => self.move_cursor(-1, key.shift),
            Keysym::Page_Down => self.move_cursor(page, key.shift),
            Keysym::Page_Up => self.move_cursor(-page, key.shift),
            Keysym::Home => self.move_cursor(-(self.visible().len() as isize), key.shift),
            Keysym::End => self.move_cursor(self.visible().len() as isize, key.shift),
            Keysym::Return | Keysym::KP_Enter => self.open_selected(),
            Keysym::BackSpace => self.go_up(),
            Keysym::Delete if key.shift => {
                let paths = self.selected_paths();
                if config.confirm_delete { self.ask_delete(paths) } else { self.delete(paths) }
            }
            Keysym::Delete => self.trash_selected(),
            Keysym::F2 => self.start_rename(),
            Keysym::F5 => self.reload(),
            Keysym::Escape => {
                if self.chooser.is_some() {
                    if let Some(chooser) = &mut self.chooser {
                        chooser.cancel();
                    }
                } else if !self.selected.is_empty() {
                    self.selected.clear();
                } else if !self.search.is_empty() {
                    self.search.clear();
                }
            }
            Keysym::l | Keysym::L if key.ctrl => self.editing = Editing::Path(TextField::with_text_capacity(&self.dir.display().to_string(), LONG_FIELD)),
            Keysym::f | Keysym::F if key.ctrl => self.editing = Editing::Search,
            Keysym::h | Keysym::H if key.ctrl => self.show_hidden = !self.show_hidden,
            Keysym::a | Keysym::A if key.ctrl => self.run(Command::SelectAll, &config),
            Keysym::c | Keysym::C if key.ctrl && self.chooser.is_none() => self.copy_selected(false),
            Keysym::x | Keysym::X if key.ctrl && self.chooser.is_none() => self.copy_selected(true),
            Keysym::v | Keysym::V if key.ctrl && self.chooser.is_none() => self.paste(),
            Keysym::n | Keysym::N if key.ctrl && key.shift => self.new_folder(),
            Keysym::_1 if key.ctrl => self.view = View::List,
            Keysym::_2 if key.ctrl => self.view = View::Grid,
            Keysym::w | Keysym::W | Keysym::q | Keysym::Q if key.ctrl => {
                if let Some(chooser) = &mut self.chooser {
                    chooser.cancel();
                } else {
                    return Flow::Quit;
                }
            }
            _ => {
                // Typing searches.
                if let Some(text) = key.text.as_deref().filter(|t| !key.ctrl && !key.alt && !t.trim().is_empty()) {
                    self.editing = Editing::Search;
                    self.search.key(key.keysym, Some(text), false);
                    if !self.visible().is_empty() {
                        self.select_only(0);
                    }
                }
            }
        }
        Flow::Continue
    }

    fn scroll(&mut self, point: (f32, f32), delta: f32) {
        if let Some(menu) = &mut self.menu {
            if self.menu_rect.is_some_and(|r| r.contains(stratum_ui::render::display_list::Point::new(point.0, point.1))) {
                layout::scroll_list(&mut menu.scroll, delta);
                return;
            }
        }
        self.scroll = (self.scroll + delta).max(0.0);
    }

    fn tick(&mut self, _dt: f32) -> bool {
        // A running job's bar moves; the folder is looked at once a second.
        self.check_dir();
        self.job.is_some()
    }

    fn poll(&mut self) -> bool {
        let thumbs = self.take_thumbnails();
        let job = self.take_job();
        thumbs || job
    }

    fn done(&self) -> bool {
        self.quit || self.chooser.as_ref().is_some_and(|c| c.answered())
    }

    fn close_requested(&mut self) -> bool {
        if let Some(chooser) = &mut self.chooser {
            chooser.cancel();
        }
        true
    }
}

/// `~` and `~/x` as the home folder.
pub fn expand_home(path: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
    match path.strip_prefix('~') {
        Some("") => home,
        Some(rest) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_home() {
        assert_eq!(count(1, "item"), "1 item");
        assert_eq!(count(3, "item"), "3 items");
        std::env::set_var("HOME", "/home/me");
        assert_eq!(expand_home("~/x"), PathBuf::from("/home/me/x"));
        assert_eq!(expand_home("~"), PathBuf::from("/home/me"));
        assert_eq!(expand_home("/etc"), PathBuf::from("/etc"));
    }

    #[test]
    fn special_folders_have_icons() {
        assert_eq!(folder_icons(Path::new("/home/me/Downloads")), ["folder-download", "folder"]);
        assert_eq!(folder_icons(Path::new("/srv/x")), ["folder"]);
    }
}
