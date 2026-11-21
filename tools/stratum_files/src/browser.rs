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
use crate::preview::{self, Preview};
use crate::undo::{self, Step};
use crate::{clipboard, dnd, open, ops, thumbs, trash};

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
const PREVIEW: &str = "\u{f10ab}";

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
    Undo,
    /// A place in the sidebar, opened from its menu.
    OpenPlace(PathBuf),
    /// A drive's filesystem by udisks object, with its name for messages
    /// and, to unmount, the folder it is mounted on.
    Mount(String, String),
    /// Ask for an encrypted volume's passphrase.
    Unlock(String, String),
    Unmount(String, String, PathBuf),
    /// A drive by udisks object, and its name.
    Eject(String, String),
    AddBookmark(PathBuf),
    RemoveBookmark(PathBuf),
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

/// The passphrase dialog for an encrypted drive.
struct Unlock {
    volume: String,
    name: String,
    field: TextField,
    /// Waiting for udisks.
    busy: bool,
    error: Option<String>,
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

/// When the preview pane is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreviewMode {
    /// While one previewable item is selected.
    Auto,
    Always,
    Never,
}

impl PreviewMode {
    /// `[files] preview`: "auto", "always" or "never".
    fn from_config(value: &str) -> Self {
        match value {
            "always" => PreviewMode::Always,
            "never" => PreviewMode::Never,
            _ => PreviewMode::Auto,
        }
    }
}

/// Items pressed and perhaps being dragged.
struct Drag {
    /// Where the press was.
    start: (f32, f32),
    paths: Vec<PathBuf>,
    /// Past the threshold: a drag, not a click.
    active: bool,
    /// What the press does if it ends as a click rather than a drag.
    on_click: Option<Click>,
    /// The card's icon, for the drag icon if it leaves the window.
    icon: Option<IconImage>,
}

/// A click's effect on the selection, put off until the press ends: a
/// press on a selected item keeps the selection for dragging.
#[derive(Clone, Copy)]
enum Click {
    /// Pressed on one of several selected: just this one.
    SelectOnly(usize),
    /// Ctrl+pressed on a selected one: it leaves the selection.
    Deselect(usize),
}

/// A running copy or move.
struct Job {
    label: &'static str,
    /// Said when it ends without errors.
    done: String,
    /// What to record for undo when it ends; `None` for an undo's own job.
    record: Option<ops::Kind>,
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
    /// udisks, started with the window; and the drives it last listed.
    drives: Option<crate::drives::Drives>,
    drive_list: Vec<stratum_drives::Drive>,
    menu: Option<Menu>,
    menu_rect: Option<Rect>,
    open_with: Vec<stratum_ui::apps::App>,
    confirm: Option<Vec<PathBuf>>,
    unlock: Option<Unlock>,
    job: Option<Job>,
    message: Option<(String, bool, Instant)>,
    thumbs: HashMap<PathBuf, Option<IconImage>>,
    /// In the Trash view, the folder each item was deleted from.
    origins: HashMap<PathBuf, PathBuf>,
    thumb_tx: Option<std::sync::mpsc::Sender<ThumbJob>>,
    /// The installed thumbnailers, read when the first thumbnail is wanted.
    thumbnailers: Option<Arc<Vec<thumbs::Thumbnailer>>>,
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
    history: undo::History,
    /// The preview pane, its contents, and what is known of the item in it.
    preview_mode: PreviewMode,
    /// What the button turns the pane back on to.
    preview_default: PreviewMode,
    /// In "auto", whether the one selected item opens the pane, kept until
    /// the selection changes.
    auto_preview: Option<(PathBuf, bool)>,
    preview: Preview,
    facts: Option<preview::Facts>,
    /// The sidebar's scroll, its area and its content's height, from the
    /// last paint: a long list of bookmarks scrolls on its own.
    sidebar_scroll: f32,
    sidebar_rect: Rect,
    sidebar_height: f32,
    drag: Option<Drag>,
    /// Items handed to the compositor as a drag to other apps, until it
    /// ends.
    handed_out: Vec<PathBuf>,
    /// Files from outside dragged over the window: where, and the folder a
    /// drop would go to.
    hover_in: Option<(Rect, PathBuf)>,
    /// Paint again: something changed after this frame was painted.
    repaint: bool,
    /// Where dragged items can be dropped, from the last paint.
    drop_zones: Vec<(Rect, PathBuf)>,
}

/// A decoded thumbnail, or `None` for a picture that did not decode.
type Thumbnail = (PathBuf, Option<IconImage>);

/// What the thumbnail thread is asked to make.
struct ThumbJob {
    path: PathBuf,
    mime: String,
    modified: Option<SystemTime>,
    bytes: u64,
    px: u32,
    max_bytes: u64,
    external: bool,
}

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
            drives: None,
            drive_list: Vec::new(),
            menu: None,
            menu_rect: None,
            open_with: Vec::new(),
            confirm: None,
            unlock: None,
            job: None,
            message: None,
            thumbs: HashMap::new(),
            origins: HashMap::new(),
            thumb_tx: None,
            thumbnailers: None,
            thumb_done: Arc::default(),
            waker: None,
            hits: Vec::new(),
            viewport: rect(0.0, 0.0, 0.0, 0.0),
            content_height: 0.0,
            grid_columns: 1,
            chooser: None,
            quit: false,
            history: undo::History::default(),
            preview_mode: PreviewMode::from_config(&config.preview),
            preview_default: match PreviewMode::from_config(&config.preview) {
                PreviewMode::Never => PreviewMode::Auto,
                mode => mode,
            },
            auto_preview: None,
            preview: Preview::default(),
            facts: None,
            sidebar_scroll: 0.0,
            sidebar_rect: rect(0.0, 0.0, 0.0, 0.0),
            sidebar_height: 0.0,
            drag: None,
            handed_out: Vec::new(),
            hover_in: None,
            repaint: false,
            drop_zones: Vec::new(),
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
        self.dir_stamp = self.stamp();
        self.origins.clear();
        self.auto_preview = None;
        let listed = if trash::is_trash(&self.dir) { Ok(self.list_trash()) } else { entries::list(&self.dir, &self.db) };
        match listed {
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

    /// When the folder last changed; for the Trash, the latest of every
    /// trash's.
    fn stamp(&self) -> Option<SystemTime> {
        let modified = |dir: &Path| std::fs::metadata(dir).and_then(|m| m.modified()).ok();
        if trash::is_trash(&self.dir) {
            trash::all().iter().filter_map(|t| modified(&t.files())).max()
        } else {
            modified(&self.dir)
        }
    }

    /// Every trash's items, each under the name it had and with the
    /// folder it came from in `origins`.
    fn list_trash(&mut self) -> Vec<Entry> {
        let mut out = Vec::new();
        for trash in trash::all() {
            let Ok(items) = entries::list(&trash.files(), &self.db) else { continue };
            for mut entry in items {
                if let Some(from) = trash::original(&entry.path) {
                    if let Some(name) = from.file_name() {
                        // Its name in the trash may have a `.2` added.
                        entry.name = name.to_string_lossy().into_owned();
                        if let Some(mime) = self.db.guess(&entry.name).filter(|_| !entry.is_dir) {
                            entry.mime = mime.to_string();
                        }
                    }
                    if let Some(folder) = from.parent() {
                        self.origins.insert(entry.path.clone(), folder.to_path_buf());
                    }
                }
                out.push(entry);
            }
        }
        out
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
                if new_folder {
                    self.history.push(Step::NewFolder(target.clone()));
                } else if target != path {
                    self.history.push(Step::Rename { from: path, to: target.clone() });
                }
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
        self.trash_paths(paths);
    }

    /// Move `paths` to the trash, recording it for undo.
    fn trash_paths(&mut self, paths: Vec<PathBuf>) {
        let mut failed = Vec::new();
        let mut trashed = Vec::new();
        for path in &paths {
            match trash::trash(path) {
                Ok(item) => trashed.push((path.clone(), item)),
                Err(err) => failed.push((path.clone(), err)),
            }
        }
        self.history.push(Step::Trash(trashed));
        self.reload();
        match failed.as_slice() {
            [] => self.say(format!("Moved {} to the trash", count(paths.len(), "item")), false),
            [(path, trash::TrashError::OtherDevice), ..] => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.say(format!("{name} is on a drive with no trash; Shift+Delete deletes it for good"), true);
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
                (true, Some(_)) => trash::delete(path),
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
            let name = self.entries.iter().find(|e| e.path == path).map(|e| e.name.clone()).unwrap_or_default();
            match trash::restore(&path) {
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
        let items = sources.into_iter().map(|s| (s, ops::Target::Into(dest.clone()))).collect();
        let (label, done) = if kind == ops::Kind::Copy { ("Copying", "Copying done") } else { ("Moving", "Moving done") };
        self.spawn_job(kind, items, label, done.into(), Some(kind));
    }

    fn spawn_job(&mut self, kind: ops::Kind, items: Vec<(PathBuf, ops::Target)>, label: &'static str, done: String, record: Option<ops::Kind>) {
        let progress = Arc::new(Mutex::new(ops::Progress::default()));
        let cancel = Arc::new(AtomicBool::new(false));
        let report = Arc::clone(&progress);
        let waker = self.waker.clone();
        let last = Mutex::new(Instant::now());
        ops::start_to(kind, items, Arc::clone(&cancel), move |p| {
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
        self.job = Some(Job { label, done, record, cancel, progress });
    }

    /// Reverse the last change.
    fn undo(&mut self) {
        if self.job.is_some() {
            return self.say("Wait for the copy or move to finish, or cancel it".into(), true);
        }
        let Some(step) = self.history.pop() else {
            return self.say("Nothing to undo".into(), false);
        };
        let what = step.describe();
        let undone = undo::undo(step);
        self.reload();
        if !undone.move_back.is_empty() {
            let items = undone.move_back.into_iter().map(|(from, to)| (from, ops::Target::Exactly(to))).collect();
            self.spawn_job(ops::Kind::Move, items, "Moving back", format!("Undid: {what}"), None);
        }
        match undone.errors.as_slice() {
            [] if self.job.is_none() => self.say(format!("Undid: {what}"), false),
            [] => {}
            [one] => self.say(format!("Could not undo all of it: {one}"), true),
            many => self.say(format!("Could not undo {}: {}", count(many.len(), "item"), many[0]), true),
        }
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
            Command::Undo => self.undo(),
            Command::OpenPlace(path) => {
                if trash::is_trash(&path) {
                    let _ = std::fs::create_dir_all(&path);
                }
                self.go(path);
            }
            Command::Mount(volume, name) => self.mount(volume, name),
            Command::Unlock(volume, name) => self.ask_unlock(volume, name),
            Command::Unmount(volume, name, at) => {
                if let Some(drives) = &self.drives {
                    drives.unmount(volume, name, at);
                }
            }
            Command::Eject(drive, name) => {
                if let Some(drives) = &self.drives {
                    drives.eject(drive, name);
                }
            }
            Command::AddBookmark(path) => self.set_bookmark(&path, true),
            Command::RemoveBookmark(path) => self.set_bookmark(&path, false),
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
                // One folder: in or out of the bookmarks.
                if let [one] = self.selected_paths().as_slice() {
                    if one.is_dir() {
                        items.push(self.bookmark_item(one));
                    }
                }
                items.extend([("Copy path", Command::CopyPath), ("Move to the trash", Command::Trash), ("Delete for good", Command::Delete)]);
            }
        } else if in_trash {
            items.push(("Empty the trash", Command::EmptyTrash));
        } else {
            items.extend([("New folder", Command::NewFolder), ("Paste", Command::Paste), ("Select all", Command::SelectAll)]);
            items.extend([("Open a terminal here", Command::TerminalHere), ("Copy path", Command::CopyPath)]);
            let dir = self.dir.clone();
            items.push(self.bookmark_item(&dir));
            items.push((if self.show_hidden { "Hide hidden files" } else { "Show hidden files" }, Command::ToggleHidden));
        }
        // The last change, first, on the folder's own menu.
        let undo = self.history.last().filter(|_| !on_entry && self.job.is_none()).map(|step| (format!("Undo: {}", step.describe()), Command::Undo));
        self.menu = Some(Menu {
            anchor: rect(at.0, at.1, width, 0.0),
            items: undo.into_iter().chain(items.into_iter().map(|(l, c)| (l.to_string(), c))).collect(),
            scroll: None,
            fresh: true,
        });
    }

    fn bookmarked(&self, path: &Path) -> bool {
        self.places.iter().any(|(s, p)| *s == Section::Bookmarks && p.path == path)
    }

    /// "Add to bookmarks" or "Remove from bookmarks" for a folder.
    fn bookmark_item(&self, path: &Path) -> (&'static str, Command) {
        if self.bookmarked(path) {
            ("Remove from bookmarks", Command::RemoveBookmark(path.to_path_buf()))
        } else {
            ("Add to bookmarks", Command::AddBookmark(path.to_path_buf()))
        }
    }

    /// A sidebar row's menu: open it, and keep or drop it as a bookmark.
    fn place_menu(&mut self, at: (f32, f32), section: Section, place: Place, width: f32) {
        let path = place.path.clone();
        let mut items: Vec<(String, Command)> = Vec::new();
        if let Some(drive) = &place.drive {
            if drive.locked {
                items.push(("Unlock and open".into(), Command::Unlock(drive.volume.clone(), place.name.clone())));
            } else if drive.mounted {
                items.push(("Open".into(), Command::OpenPlace(path.clone())));
                items.push(("Unmount".into(), Command::Unmount(drive.volume.clone(), place.name.clone(), path.clone())));
            } else {
                items.push(("Mount and open".into(), Command::Mount(drive.volume.clone(), place.name.clone())));
            }
            if drive.removable {
                items.push((format!("Eject {}", drive.drive_name), Command::Eject(drive.drive.clone(), drive.drive_name.clone())));
            }
            self.menu = Some(Menu { anchor: rect(at.0, at.1, width, 0.0), items, scroll: None, fresh: true });
            return;
        }
        items.push(("Open".into(), Command::OpenPlace(path.clone())));
        if trash::is_trash(&path) {
            items.push(("Empty the trash".into(), Command::EmptyTrash));
        } else if section == Section::Bookmarks || path.is_dir() {
            let (label, command) = self.bookmark_item(&path);
            items.push((label.into(), command));
        }
        self.menu = Some(Menu { anchor: rect(at.0, at.1, width, 0.0), items, scroll: None, fresh: true });
    }

    /// The places read again, with the drives udisks last listed.
    fn reload_places(&mut self) {
        self.places = places::with_drives(places::load(), &self.drive_list);
    }

    fn mount(&mut self, volume: String, name: String) {
        match &self.drives {
            Some(drives) => {
                drives.mount(volume, name.clone());
                self.say(format!("Mounting {name}\u{2026}"), false);
            }
            None => self.say("udisks is not running".into(), true),
        }
        self.repaint = true;
    }

    fn ask_unlock(&mut self, volume: String, name: String) {
        self.unlock = Some(Unlock { volume, name, field: TextField::new(), busy: false, error: None });
        self.repaint = true;
    }

    fn submit_unlock(&mut self, passphrase: stratum_ui::secret::Secret) {
        let (Some(unlock), Some(drives)) = (&mut self.unlock, &self.drives) else {
            return;
        };
        unlock.busy = true;
        unlock.error = None;
        drives.unlock(unlock.volume.clone(), passphrase, unlock.name.clone());
        self.repaint = true;
    }

    /// The passphrase dialog: a masked field, Cancel and Unlock. It stays
    /// open on a wrong passphrase.
    fn paint_unlock(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, input: &InputState, clicked: bool) {
        let Some(unlock) = &self.unlock else { return };
        let (name, busy, error) = (unlock.name.clone(), unlock.busy, unlock.error.clone());
        let m = ui.config.metrics.files.clone();
        let page = ui.config.metrics.page.clone();
        let theme = ui.theme.clone();
        list.push_layer();
        ui::paint_scrim(list, area, 0.4);
        let h = page.card_padding * 4.0 + page.badge * 3.0;
        let card = rect(area.x + (area.width - m.dialog_width) * 0.5, area.y + (area.height - h) * 0.4, m.dialog_width, h);
        layout::card(ui, list, card, Card::Plain);
        let inner_w = card.width - page.card_padding * 2.0;
        let line = rect(card.x + page.card_padding, card.y + page.card_padding, inner_w, page.badge);
        let detail = match &error {
            Some(error) => ui::elide(ui, error, m.detail_text, 400, inner_w),
            None => "Enter its passphrase.".to_string(),
        };
        layout::two_lines(ui, list, line, &format!("Unlock {name}"), &detail, if error.is_some() { theme.error } else { theme.text_muted });
        let field = rect(line.x, line.y + page.badge + page.card_padding, inner_w, page.badge);
        ui::paint_frame(list, Rect { radius: field.height * 0.5, ..field }, 1.5, if busy { theme.surface_high } else { theme.primary }, theme.surface_high);
        let style = FieldStyle { text: m.text, padding: m.gap, caret_width: 1.5, masked: true, placeholder: Some("Passphrase") };
        if let Some(unlock) = &self.unlock {
            unlock.field.paint(ui, list, field, style, busy);
        }
        let y = card.y + card.height - page.card_padding - page.badge;
        let labels: [&str; 2] = ["Cancel", if busy { "Unlocking\u{2026}" } else { "Unlock" }];
        let (_, hit) = layout::text_buttons(ui, list, card.x + card.width - page.card_padding, y, &labels, true, input, clicked && !busy);
        match hit {
            Some(1) => {
                if let Some(passphrase) = self.unlock.as_mut().and_then(|u| u.field.take()) {
                    self.submit_unlock(passphrase);
                }
            }
            Some(_) => self.unlock = None,
            None if clicked && !busy && !layout::hovered(input, card) => self.unlock = None,
            None => {}
        }
    }

    /// A new drive list from udisks, and how operations ended.
    fn take_drives(&mut self) -> bool {
        let Some(drives) = &self.drives else {
            return false;
        };
        let (list, events) = drives.take();
        let changed = list.is_some() || !events.is_empty();
        if let Some(list) = list {
            self.drive_list = list;
            self.reload_places();
        }
        for event in events {
            match event {
                crate::drives::Event::UnlockFailed(why) => {
                    if let Some(unlock) = &mut self.unlock {
                        unlock.busy = false;
                        unlock.error = Some(why);
                    }
                }
                crate::drives::Event::Mounted(path) => {
                    self.unlock = None;
                    self.reload_places();
                    self.message = None;
                    self.go(path);
                }
                crate::drives::Event::Unmounted(at) => {
                    if self.dir.starts_with(&at) {
                        self.go(places::home_dir());
                    }
                    self.say(format!("Unmounted {}", at.file_name().map_or_else(|| at.display().to_string(), |n| n.to_string_lossy().into_owned())), false);
                }
                crate::drives::Event::Ejected(name) => {
                    let gone: Vec<PathBuf> = self
                        .places
                        .iter()
                        .filter(|(_, p)| p.drive.as_ref().is_some_and(|d| d.drive_name == name) && !p.path.as_os_str().is_empty())
                        .map(|(_, p)| p.path.clone())
                        .collect();
                    if gone.iter().any(|g| self.dir.starts_with(g)) {
                        self.go(places::home_dir());
                    }
                    self.say(format!("{name} can be removed"), false);
                }
                crate::drives::Event::Failed(message) => {
                    self.unlock = None;
                    self.say(message, true);
                }
            }
        }
        if changed {
            self.repaint = true;
        }
        changed
    }

    /// Add `path` to the bookmarks, or take it out, and show the sidebar
    /// as it now is.
    fn set_bookmark(&mut self, path: &Path, keep: bool) {
        let name = path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned());
        let result = if keep { places::add_bookmark(path) } else { places::remove_bookmark(path) };
        match result {
            Ok(()) => {
                self.reload_places();
                self.say(if keep { format!("Added {name} to the bookmarks") } else { format!("Removed {name} from the bookmarks") }, false);
            }
            Err(err) => self.say(format!("The bookmarks were not changed: {err}"), true),
        }
        self.repaint = true;
    }

    // ---- Thumbnails ----------------------------------------------------

    fn thumbnailers(&mut self) -> Arc<Vec<thumbs::Thumbnailer>> {
        Arc::clone(self.thumbnailers.get_or_insert_with(|| Arc::new(thumbs::load_thumbnailers())))
    }

    fn want_thumbnail(&mut self, entry: &Entry, size: u32, config: &stratum_ui::config::FilesConfig) -> Option<IconImage> {
        let max_bytes = config.thumbnail_max_mb * 1024 * 1024;
        if !config.thumbnails || entry.is_dir {
            return None;
        }
        let thumbnailers = self.thumbnailers();
        if !thumbs::can(&entry.mime, entry.size, max_bytes, config.thumbnail_external, &thumbnailers) {
            return None;
        }
        if let Some(found) = self.thumbs.get(&entry.path) {
            return found.clone();
        }
        let tx = self.thumb_tx.get_or_insert_with(|| {
            let (tx, rx) = std::sync::mpsc::channel::<ThumbJob>();
            let done = Arc::clone(&self.thumb_done);
            let waker = self.waker.clone();
            std::thread::Builder::new()
                .name("stratum-thumbs".into())
                .spawn(move || {
                    for job in rx {
                        let request = thumbs::Request {
                            path: &job.path,
                            mime: &job.mime,
                            modified: job.modified,
                            bytes: job.bytes,
                            px: job.px,
                            max_bytes: job.max_bytes,
                            external: job.external,
                        };
                        let image = thumbs::make(&request, &thumbnailers).map(thumbs::premultiplied);
                        if let Ok(mut done) = done.lock() {
                            done.push((job.path, image));
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
        let _ = tx.send(ThumbJob {
            path: entry.path.clone(),
            mime: entry.mime.clone(),
            modified: entry.modified,
            bytes: entry.size,
            px: size,
            max_bytes,
            external: config.thumbnail_external,
        });
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
        // Pills on the ground: back, forward and up together; the path; the
        // search; the views together.
        let inset = (bar.height - m.button) * 0.5;
        let pill = |list: &mut DisplayList, r: Rect, focused: bool| {
            let r = Rect { radius: r.height * 0.5, ..r };
            list.push_rect(Rect { color: theme.surface, ..r });
            list.push_outline(Rect { color: if focused { theme.primary } else { theme.outline }, ..r }, if focused { 1.5 } else { 1.0 });
        };
        let y = bar.y + inset;
        let nav_w = m.button * 3.0 + m.gap * 0.5 * 2.0 + inset * 2.0;
        pill(list, rect(bar.x, bar.y, nav_w, bar.height), false);
        let mut x = bar.x + inset;
        for (glyph, enabled, action) in [(BACK, !self.back.is_empty(), 0), (FORWARD, !self.forward.is_empty(), 1), (UP, self.dir.parent().is_some(), 2)] {
            if layout::round_button(ui, list, rect(x, y, m.button, m.button), glyph, false, enabled, theme.primary, input, clicked) {
                match action {
                    0 => self.go_back(),
                    1 => self.go_forward(),
                    _ => self.go_up(),
                }
            }
            x += m.button + m.gap * 0.5;
        }
        let x = bar.x + nav_w + m.gap;

        // At the right: the views, then search.
        let buttons = [(PREVIEW, self.preview_mode != PreviewMode::Never, 3), (GRID, self.view == View::Grid, 0), (LIST, self.view == View::List, 1), (HIDDEN, self.show_hidden, 2)];
        let views_w = m.button * buttons.len() as f32 + m.gap * 0.5 * (buttons.len() - 1) as f32 + inset * 2.0;
        let views = rect(bar.x + bar.width - views_w, bar.y, views_w, bar.height);
        pill(list, views, false);
        let mut right = views.x + views.width - inset;
        for (glyph, on, action) in buttons {
            right -= m.button;
            if layout::round_button(ui, list, rect(right, y, m.button, m.button), glyph, on, true, theme.primary, input, clicked) {
                match action {
                    0 => self.view = View::Grid,
                    1 => self.view = View::List,
                    2 => self.show_hidden = !self.show_hidden,
                    _ => self.toggle_preview(),
                }
            }
            right -= m.gap * 0.5;
        }
        let search = rect(views.x - m.gap - m.search_width, bar.y, m.search_width, bar.height);
        let searching = matches!(self.editing, Editing::Search);
        pill(list, search, searching);
        let glyph = rect(search.x + inset, search.y, m.button, search.height);
        ui::paint_icon_centered(ui, list, SEARCH, glyph, m.text, theme.text_muted);
        let field = rect(glyph.x + m.button, search.y, search.width - m.button - inset * 2.0, search.height);
        let style = FieldStyle { text: m.text, padding: 0.0, caret_width: 1.5, masked: false, placeholder: Some("Search this folder") };
        self.search.paint(ui, list, field, style, !searching);
        if clicked && layout::hovered(input, search) {
            self.editing = Editing::Search;
        }

        let editing_path = matches!(self.editing, Editing::Path(_));
        let path_pill = rect(x, bar.y, (search.x - m.gap - x).max(0.0), bar.height);
        pill(list, path_pill, editing_path);
        let path_area = rect(path_pill.x + inset, y, (path_pill.width - inset * 2.0).max(0.0), m.button);
        // The path between them: breadcrumbs, or a field while editing.
        match &mut self.editing {
            Editing::Path(field) => {
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
    fn paint_breadcrumbs(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, input: &InputState, clicked: bool) -> Option<PathBuf> {
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
            if !last {
                self.drop_zones.push((cell, path.clone()));
            }
            // Each crumb a chip: the folder shown tinted, the others lit
            // under the pointer.
            let chip = Rect { radius: cell.height * 0.5, ..cell };
            if last {
                list.push_rect(Rect { color: layout::mix(theme.surface_high, theme.primary, 0.25), ..chip });
            } else if layout::hovered(input, cell) {
                list.push_rect(Rect { color: theme.surface_high, ..chip });
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

    fn paint_sidebar(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, input: &InputState, clicked: bool, secondary: bool) {
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        // Clipped to its card, and scrolled when the places do not fit.
        // Inside the card's rounded ends, so a row cut short never sits on
        // its corners.
        let area = rect(area.x, area.y + m.padding * 0.5, area.width, area.height - m.padding);
        self.sidebar_rect = area;
        self.sidebar_scroll = self.sidebar_scroll.clamp(0.0, (self.sidebar_height - area.height).max(0.0));
        list.push_clip(area);
        let top = area.y + m.padding - self.sidebar_scroll;
        let mut y = top;
        let mut section = None;
        let mut go = None;
        let mut mount = None;
        let mut unlock = None;
        let mut menu = None;
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
            // The current place an accent pill; others lit under the pointer.
            if current || layout::hovered(input, row) {
                let color = if current { layout::mix(theme.surface, theme.primary, 0.25) } else { theme.surface_high };
                list.push_rect(Rect { radius: row.height * 0.5, color, ..row });
            }
            let icon_box = rect(row.x + m.gap, row.y + (row.height - m.sidebar_icon) * 0.5, m.sidebar_icon, m.sidebar_icon);
            match ui.icons.load_icon(place.icon, m.sidebar_icon as u32, 2).or_else(|| ui.icons.load_icon("folder", m.sidebar_icon as u32, 2)) {
                Some(image) => push_image(list, image, icon_box),
                None => ui::paint_icon_centered(ui, list, FOLDER, icon_box, m.sidebar_icon, theme.text_muted),
            }
            let text_x = icon_box.x + icon_box.width + m.gap;
            let name = ui::elide(ui, &place.name, m.text, 400, row.x + row.width - text_x - m.gap);
            let weight = if current { ui.config.metrics.typography.weight_bold } else { 400 };
            ui::paint_text_vcentered(ui, list, &name, text_x, row, m.text, weight, if current { theme.primary } else { theme.text_main });
            let visible = row.y + row.height > area.y && row.y < area.y + area.height;
            let under = visible && layout::hovered(input, row) && layout::hovered(input, area);
            if clicked && under {
                match &place.drive {
                    Some(drive) if drive.locked => unlock = Some((drive.volume.clone(), place.name.clone())),
                    Some(drive) if !drive.mounted => mount = Some((drive.volume.clone(), place.name.clone())),
                    _ => go = Some(place.path.clone()),
                }
            }
            if secondary && under {
                menu = Some((s, place.clone()));
            }
            if visible && !place.path.as_os_str().is_empty() {
                // Only what shows, and only its shown part, takes a drop.
                let y0 = row.y.max(area.y);
                let y1 = (row.y + row.height).min(area.y + area.height);
                self.drop_zones.push((Rect { y: y0, height: y1 - y0, ..row }, place.path.clone()));
            }
            y += m.sidebar_row;
        }
        list.pop_clip();
        self.sidebar_height = y - top + m.padding;
        if let (Some((section, place)), Some(at)) = (menu, input.pointer) {
            self.place_menu(at, section, place, m.menu_width);
        }
        if let Some((volume, name)) = mount {
            self.mount(volume, name);
        }
        if let Some((volume, name)) = unlock {
            self.ask_unlock(volume, name);
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
            if entry.is_dir {
                self.drop_zones.push((Rect { x: cell.x + 2.0, y: cell.y + 1.0, width: cell.width - 4.0, height: cell.height - 2.0, ..cell }, entry.path.clone()));
            }
            let selected = self.selected.contains(&entry.path);
            let hot = layout::hovered(input, cell) && layout::hovered(input, viewport);
            let inset = Rect { x: cell.x + 2.0, y: cell.y + 1.0, width: cell.width - 4.0, height: cell.height - 2.0, radius: m.row_radius, color: Color::TRANSPARENT };
            if selected {
                // A tinted fill with an accent edge.
                list.push_rect(Rect { color: layout::mix(theme.surface, theme.primary, 0.2), ..inset });
                list.push_outline(Rect { color: theme.primary.with_alpha(theme.primary.a * 0.6), ..inset }, 1.5);
            } else if hot {
                list.push_rect(Rect { color: theme.surface_high, ..inset });
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
                    } else if let Some(folder) = self.origins.get(&entry.path) {
                        // In the Trash, the folder it came from after the
                        // name, muted, sharing the column.
                        let name = ui::elide(ui, &entry.name, m.text, 400, name_w * 0.5);
                        ui::paint_text_vcentered(ui, list, &name, name_x, cell, m.text, 400, text_color);
                        let from_x = name_x + ui.measure(&name, m.text) + m.gap * 2.0;
                        let from = ui::elide(ui, &tilde(folder), m.detail_text, 400, name_x + name_w - from_x);
                        ui::paint_text_vcentered(ui, list, &from, from_x, cell, m.detail_text, 400, theme.text_muted);
                    } else {
                        // Folders a little heavier, as in the sidebar.
                        let weight = if entry.is_dir { ui.config.metrics.typography.weight_medium } else { 400 };
                        let name = ui::elide(ui, &entry.name, m.text, weight, name_w);
                        ui::paint_text_vcentered(ui, list, &name, name_x, cell, m.text, weight, text_color);
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
                // Added now, so it can be dragged with the rest; taken out
                // only if this ends as a click.
                let selected = self.selected.contains(&visible[i].path);
                if !selected {
                    self.toggle(i);
                }
                self.begin_drag(input, selected.then_some(Click::Deselect(i)));
            } else if pointer.shift {
                self.select_range(i);
            } else if pointer.double {
                self.select_only(i);
                self.open_selected();
            } else {
                // A press on one of several selected keeps them all, in
                // case it is a drag; a click then selects just this one.
                let keep = self.selected.len() > 1 && self.selected.contains(&visible[i].path);
                if !keep {
                    self.select_only(i);
                }
                self.begin_drag(input, keep.then_some(Click::SelectOnly(i)));
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
                list.push_rect(Rect { radius: m.row_radius, color: theme.surface_high, ..cell });
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
        list.push_rect(Rect { y: area.y + area.height - 1.0, height: 1.0, color: theme.outline, ..area });
    }

    fn paint_rename(&self, ui: &mut AppUi, list: &mut DisplayList, area: Rect) {
        let Editing::Rename { field, .. } = &self.editing else { return };
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        ui::paint_frame(list, Rect { radius: m.radius * 0.6, ..area }, 1.5, theme.primary, theme.surface_high);
        let style = FieldStyle { text: m.text, padding: m.gap * 0.5, caret_width: 1.5, masked: false, placeholder: None };
        field.paint(ui, list, area, style, false);
    }

    /// Where files dragged in from outside would go at `point`: a folder
    /// under it, else the folder shown (not the Trash, nor in a chooser).
    /// Our own items, handed out and come back, keep the in-window rules.
    fn drop_target(&self, point: (f32, f32)) -> Option<(Rect, PathBuf)> {
        let (x, y) = point;
        let inside = |r: &Rect| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
        let ok = |dest: &Path| self.handed_out.is_empty() || dnd::valid_target(&self.handed_out, dest);
        if let Some(zone) = self.drop_zones.iter().rev().find(|(r, p)| inside(r) && ok(p)) {
            return Some(zone.clone());
        }
        let here = self.chooser.is_none() && !trash::is_trash(&self.dir) && inside(&self.viewport) && ok(&self.dir);
        here.then(|| (self.viewport, self.dir.clone()))
    }

    /// A press on the selection, which may become a drag of it. Items in
    /// the Trash, or in a chooser, are not dragged.
    fn begin_drag(&mut self, input: &InputState, on_click: Option<Click>) {
        if self.chooser.is_some() || trash::is_trash(&self.dir) {
            return;
        }
        if let Some(at) = input.pointer {
            self.drag = Some(Drag { start: at, paths: self.selected_paths(), active: false, on_click, icon: None });
        }
    }

    /// A press becoming a drag, the target under it, and the drop.
    fn paint_drag(&mut self, ui: &mut AppUi, list: &mut DisplayList, pointer: &Pointer) {
        let Some(drag) = &mut self.drag else { return };
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        let at = pointer.input.pointer;
        if let Some(at) = at.filter(|_| pointer.input.pressed && !drag.active) {
            drag.active = dnd::started(drag.start, at, m.drag_threshold);
        }
        let (active, paths) = (drag.active, drag.paths.clone());
        let target = at.filter(|_| active).and_then(|(x, y)| {
            let inside = |r: &Rect| x >= r.x && x < r.x + r.width && y >= r.y && y < r.y + r.height;
            // The last painted is on top.
            self.drop_zones.iter().rev().find(|(r, p)| inside(r) && dnd::valid_target(&paths, p)).cloned()
        });
        let dev = |p: &Path| std::fs::metadata(p).ok().map(|m| std::os::unix::fs::MetadataExt::dev(&m));
        // What a drop here would do, as "{verb} {what}{after}".
        let action = target.as_ref().map(|(_, dest)| {
            if trash::is_trash(dest) {
                ("Move", " to the trash")
            } else {
                match dnd::action(&paths, dest, pointer.ctrl, pointer.shift, dev) {
                    dnd::DropAction::Move => ("Move", ""),
                    dnd::DropAction::Copy => ("Copy", ""),
                }
            }
        });

        if !pointer.input.pressed || pointer.released {
            let drag = self.drag.take().expect("checked above");
            // This runs after the rest of the window painted: what it
            // changes shows in another frame.
            self.repaint = true;
            match (drag.active, target) {
                (true, Some((_, dest))) if trash::is_trash(&dest) => self.trash_paths(drag.paths),
                (true, Some((_, dest))) => {
                    let kind = match dnd::action(&drag.paths, &dest, pointer.ctrl, pointer.shift, dev) {
                        dnd::DropAction::Move => ops::Kind::Move,
                        dnd::DropAction::Copy => ops::Kind::Copy,
                    };
                    if self.job.is_some() {
                        self.say("Wait for the copy or move to finish, or cancel it".into(), true);
                    } else {
                        self.start_job(kind, drag.paths, dest);
                    }
                }
                (true, None) => {}
                (false, _) => match drag.on_click {
                    Some(Click::SelectOnly(i)) => self.select_only(i),
                    Some(Click::Deselect(i)) => self.toggle(i),
                    None => {}
                },
            }
            return;
        }
        if !active {
            return;
        }
        let Some((x, y)) = at else { return };
        list.push_layer();
        if let Some((zone, _)) = &target {
            list.push_outline(Rect { radius: m.row_radius, color: theme.primary, ..*zone }, 2.0);
        }
        // A card under the pointer: the first item's icon, what is dragged,
        // and what a drop here would do.
        let first = self.entries.iter().find(|e| Some(&e.path) == paths.first()).cloned();
        let what = match (paths.as_slice(), &first) {
            ([_], Some(entry)) => entry.name.clone(),
            _ => count(paths.len(), "item"),
        };
        let label = match action {
            Some((verb, after)) => format!("{verb} {what}{after}"),
            None => what,
        };
        let text_w = ui.measure(&label, m.detail_text);
        let card = rect(x + m.gap * 1.5, y + m.gap * 1.5, m.padding * 2.0 + m.row_icon + m.gap + text_w, m.row_height);
        list.push_rect(Rect { radius: card.height * 0.5, color: theme.surface_highest, ..card });
        list.push_outline(Rect { radius: card.height * 0.5, color: theme.outline, ..card }, 1.0);
        let icon_box = rect(card.x + m.padding, card.y + (card.height - m.row_icon) * 0.5, m.row_icon, m.row_icon);
        let icon = first.and_then(|entry| self.icon(ui, &entry, m.row_icon, &ui.config.files.clone()));
        if let Some(drag) = &mut self.drag {
            drag.icon = icon.clone();
        }
        match icon {
            Some(image) => {
                let (image, at) = fit(image, icon_box);
                push_image(list, image, at);
            }
            None => ui::paint_icon_centered(ui, list, FILE, icon_box, m.row_icon, theme.text_muted),
        }
        ui::paint_text_vcentered(ui, list, &label, icon_box.x + icon_box.width + m.gap, card, m.detail_text, 400, theme.text_main);
    }

    /// The toolbar's button and F9: the pane off, or back on as the
    /// config has it ("auto" when that is "never", so the button always
    /// does something).
    fn toggle_preview(&mut self) {
        self.preview_mode = match self.preview_mode {
            PreviewMode::Never => self.preview_default,
            _ => PreviewMode::Never,
        };
    }

    /// What the pane shows: the selection, else the folder itself (but
    /// not the Trash, which is many folders).
    fn preview_targets(&self) -> Vec<PathBuf> {
        let selected = self.selected_paths();
        if selected.is_empty() && !trash::is_trash(&self.dir) {
            vec![self.dir.clone()]
        } else {
            selected
        }
    }

    /// Start showing `targets`: what is known now, and the rest loading.
    /// How one item would be previewed, and its type.
    fn preview_kind(&mut self, one: &Path, config: &stratum_ui::config::FilesConfig) -> (preview::Kind, String) {
        let entry = self.entries.iter().find(|e| e.path == one).cloned();
        let entry = entry.as_ref();
        let is_dir = entry.map_or_else(|| one.is_dir(), |e| e.is_dir);
        let mime = entry.map_or_else(|| self.db.of(one, is_dir), |e| e.mime.clone());
        let size = entry.map_or(0, |e| e.size);
        let max_bytes = config.thumbnail_max_mb * 1024 * 1024;
        let thumbnailers = self.thumbnailers();
        let thumbnail = config.thumbnails && thumbs::can(&mime, size, max_bytes, config.thumbnail_external, &thumbnailers);
        let kind = if is_dir {
                preview::Kind::Folder
            } else if thumbnail && thumbs::in_process(&mime) && mime != "image/svg+xml" {
                preview::Kind::Picture
            } else if thumbnail {
                preview::Kind::Thumbnail(preview::Thumb {
                    path: one.to_path_buf(),
                    mime: mime.clone(),
                    modified: entry.and_then(|e| e.modified),
                    bytes: size,
                    max_bytes,
                    external: config.thumbnail_external,
                    thumbnailers,
                })
            } else if self.db.ancestors(&mime).iter().any(|m| m == "text/plain") {
                preview::Kind::Text
            } else {
                preview::Kind::Icon
            };
        (kind, mime)
    }

    /// Whether the pane is open: in "auto", while one picture, thumbnail
    /// or text file is selected.
    fn preview_open(&mut self, config: &stratum_ui::config::FilesConfig) -> bool {
        match self.preview_mode {
            PreviewMode::Never => false,
            PreviewMode::Always => true,
            PreviewMode::Auto => {
                let selected = self.selected_paths();
                let [one] = selected.as_slice() else { return false };
                if let Some((path, open)) = &self.auto_preview {
                    if path == one {
                        return *open;
                    }
                }
                let (kind, _) = self.preview_kind(one, config);
                let open = matches!(kind, preview::Kind::Picture | preview::Kind::Thumbnail(_) | preview::Kind::Text);
                self.auto_preview = Some((one.clone(), open));
                open
            }
        }
    }

    fn show_preview(&mut self, targets: Vec<PathBuf>, px: u32, config: &stratum_ui::config::FilesConfig) {
        self.facts = None;
        let mut kind = preview::Kind::Icon;
        if let [one] = targets.as_slice() {
            let (k, mime) = self.preview_kind(one, config);
            kind = k;
            self.facts = Some(preview::Facts::read(one, &mime));
        }
        self.preview.show(targets, kind, px, self.waker.as_ref());
    }

    fn paint_preview(&mut self, ui: &mut AppUi, list: &mut DisplayList, pane: Rect, config: &stratum_ui::config::FilesConfig) {
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        let inner = rect(pane.x + m.padding, pane.y + m.padding, pane.width - m.padding * 2.0, pane.height - m.padding * 2.0);
        let side = inner.width.min(inner.height * 0.5);
        let visual = rect(inner.x, inner.y, inner.width, side);
        // Twice the logical size, for scaled outputs.
        let px = (side * 2.0) as u32;

        let targets = self.preview_targets();
        if targets != self.preview.showing {
            self.show_preview(targets, px, config);
        }
        let targets = self.preview.showing.clone();
        let entry = |path: &Path| self.entries.iter().find(|e| e.path == path).cloned();

        // The top: the picture, the text, or an icon.
        match self.preview.visual.clone() {
            preview::Visual::Picture { image, .. } => {
                let fit = (visual.width / image.width as f32).min(visual.height / image.height as f32).min(1.0);
                let (w, h) = (image.width as f32 * fit, image.height as f32 * fit);
                let (x, y) = (visual.x + (visual.width - w) * 0.5, visual.y + (visual.height - h) * 0.5);
                use stratum_ui::render::display_list::{Point, TexturedQuad};
                list.push_texture(TexturedQuad {
                    // A new key for each new picture.
                    key: 0x5052_4556_0000_0000 ^ self.preview.generation(),
                    width: image.width,
                    height: image.height,
                    pixels: image.pixels.clone(),
                    corners: [Point::new(x, y), Point::new(x + w, y), Point::new(x + w, y + h), Point::new(x, y + h)],
                    uv: [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
                    alpha: 1.0,
                });
            }
            preview::Visual::Text { text, more } => {
                list.push_rect(Rect { radius: m.radius, color: theme.surface_high, ..visual });
                let view = rect(visual.x + m.gap, visual.y + m.gap, visual.width - m.gap * 2.0, visual.height - m.gap * 2.0);
                ui::paint_text_preview(ui, list, &text, more, view, m.preview_text);
            }
            preview::Visual::Pending => {}
            preview::Visual::Icon => {
                let icon_box = rect(visual.x + (visual.width - m.preview_icon) * 0.5, visual.y + (visual.height - m.preview_icon) * 0.5, m.preview_icon, m.preview_icon);
                let one = match targets.as_slice() {
                    [one] => Some(entry(one).unwrap_or_else(|| folder_entry(one))),
                    _ => None,
                };
                let names = match &one {
                    Some(e) if e.is_dir => folder_icons(&e.path),
                    Some(e) => self.db.icon_names(&e.mime),
                    None => Vec::new(),
                };
                match names.iter().find_map(|n| ui.icons.load_icon(n, m.preview_icon as u32, 2)) {
                    Some(image) => {
                        let (image, at) = fit(image, icon_box);
                        push_image(list, image, at);
                    }
                    None => {
                        let glyph = if one.as_ref().is_some_and(|e| e.is_dir) { FOLDER } else { FILE };
                        ui::paint_icon_centered(ui, list, glyph, icon_box, m.preview_icon * 0.8, theme.text_muted);
                    }
                }
            }
        }

        // Under it: the name, then the information.
        let mut y = visual.y + visual.height + m.gap;
        let title = match targets.as_slice() {
            [one] => entry(one).map(|e| e.name).unwrap_or_else(|| one.file_name().map_or_else(|| one.display().to_string(), |n| n.to_string_lossy().into_owned())),
            [] => "Nothing selected".into(),
            many => format!("{} selected", count(many.len(), "item")),
        };
        let title = if trash::is_trash(&self.dir) && targets.is_empty() { "Trash".to_string() } else { title };
        let weight = ui.config.metrics.typography.weight_medium;
        for line in ui::wrap_text(ui, &title, m.text, weight, inner.width, 2) {
            let line = ui::elide(ui, &line, m.text, weight, inner.width);
            ui::paint_text_centered_weighted(ui, list, &line, rect(inner.x, y, inner.width, m.preview_row), m.text, weight, theme.text_main);
            y += m.preview_row;
        }
        y += m.gap;

        let pixels = match &self.preview.visual {
            preview::Visual::Picture { size, .. } => *size,
            _ => None,
        };
        let mut lines: Vec<(&str, String)> = match (&self.facts, targets.as_slice()) {
            (Some(facts), [_]) => preview::info_lines(facts, self.preview.folder, pixels, chrono::Local::now()),
            (_, []) => vec![("Contains", count(self.entries.len(), "item"))],
            (_, many) => {
                let picked: Vec<Entry> = many.iter().filter_map(|p| entry(p)).collect();
                let folders = picked.iter().filter(|e| e.is_dir).count();
                let bytes: u64 = picked.iter().filter(|e| !e.is_dir).map(|e| e.size).sum();
                let mut lines = vec![("Files", (picked.len() - folders).to_string()), ("Size", entries::format_size(bytes))];
                if folders > 0 {
                    lines.insert(1, ("Folders", folders.to_string()));
                }
                lines
            }
        };
        // In the Trash, where an item came from.
        if let [one] = targets.as_slice() {
            if let Some(folder) = self.origins.get(one) {
                lines.insert(1, ("From", tilde(folder)));
            }
        }
        list.push_clip(inner);
        let value_x = inner.x + m.preview_label;
        for (label, value) in lines {
            if y + m.preview_row > inner.y + inner.height {
                break;
            }
            let row = rect(inner.x, y, inner.width, m.preview_row);
            ui::paint_text_vcentered(ui, list, label, inner.x, row, m.detail_text, 400, theme.text_muted);
            let value = ui::elide(ui, &value, m.detail_text, 400, inner.x + inner.width - value_x);
            ui::paint_text_vcentered(ui, list, &value, value_x, row, m.detail_text, 400, theme.text_main);
            y += m.preview_row;
        }
        list.pop_clip();
    }

    /// The folder's name, as its title shows it.
    fn folder_name(&self) -> String {
        if trash::is_trash(&self.dir) {
            return "Trash".into();
        }
        self.dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "/".into())
    }

    /// Over the list: the folder's name, pills with its counts, and the
    /// Trash's Empty button.
    fn paint_title(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, input: &InputState, clicked: bool) {
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        let bold = ui.config.metrics.typography.weight_bold;
        let mut right = area.x + area.width;
        if trash::is_trash(&self.dir) && !self.entries.is_empty() {
            let label = "Empty the trash";
            let w = layout::text_button_width(ui, label);
            let h = ui.config.metrics.page.badge;
            right -= w;
            if layout::text_button(ui, list, right, area.y + (area.height - h) * 0.5, label, true, input, clicked) {
                self.run(Command::EmptyTrash, &ui.config.files.clone());
            }
            right -= m.gap;
        }
        let title = self.folder_name();
        let mut pills = vec![count(self.visible().len(), "item")];
        let selected = self.selected_paths();
        if !selected.is_empty() {
            let bytes: u64 = self.entries.iter().filter(|e| self.selected.contains(&e.path) && !e.is_dir).map(|e| e.size).sum();
            pills.push(if bytes > 0 { format!("{} selected \u{b7} {}", selected.len(), entries::format_size(bytes)) } else { format!("{} selected", selected.len()) });
        }
        let pill_w: f32 = pills.iter().map(|p| ui.measure(p, m.detail_text) + m.padding * 2.0 + m.gap).sum();
        let title = ui::elide(ui, &title, m.title_text, bold, (right - area.x - pill_w - m.gap).max(0.0));
        ui::paint_text_vcentered(ui, list, &title, area.x + m.gap * 0.5, area, m.title_text, bold, theme.text_main);
        let mut x = area.x + m.gap * 0.5 + ui.measure_weighted(&title, m.title_text, bold) + m.gap * 1.5;
        for (i, pill) in pills.iter().enumerate() {
            let w = ui.measure(pill, m.detail_text) + m.padding * 2.0;
            let r = rect(x, area.y + (area.height - m.pill_height) * 0.5, w, m.pill_height);
            let fill = if i == 0 { theme.surface_high } else { layout::mix(theme.surface_high, theme.primary, 0.25) };
            list.push_rect(Rect { radius: m.pill_height * 0.5, color: fill, ..r });
            ui::paint_text_centered(ui, list, pill, r, m.detail_text, if i == 0 { theme.text_muted } else { theme.text_main });
            x += w + m.gap;
        }
    }

    /// Pills floating over the bottom of the folder: a running copy or move
    /// at the right, with its progress and Cancel; a message in the middle.
    fn paint_floating(&mut self, ui: &mut AppUi, list: &mut DisplayList, card: Rect, input: &InputState, clicked: bool) {
        let m = ui.config.metrics.files.clone();
        let theme = ui.theme.clone();
        let bottom = card.y + card.height - m.padding - m.status_height;
        list.push_layer();
        if let Some(job) = &self.job {
            let p = job.progress.lock().map(|p| p.clone()).unwrap_or_default();
            let fraction = if p.total > 0 { p.done as f32 / p.total as f32 } else { 0.0 };
            let text = format!("{} {} \u{b7} {}% of {}", job.label, p.current, (fraction * 100.0) as u32, entries::format_size(p.total));
            let text = ui::elide(ui, &text, m.detail_text, 400, card.width * 0.5);
            let label = "Cancel";
            let button_w = layout::text_button_width(ui, label);
            let h = ui.config.metrics.page.badge;
            let w = ui.measure(&text, m.detail_text) + m.padding * 3.0 + button_w;
            let pill = rect(card.x + card.width - m.padding - w, bottom + m.status_height - h - m.gap * 0.5, w, h + m.gap);
            list.push_rect(Rect { radius: pill.height * 0.5, color: theme.surface_highest, ..pill });
            // The progress fills the pill from the left.
            list.push_rect(Rect { radius: pill.height * 0.5, color: layout::mix(theme.surface_highest, theme.primary, 0.3), width: pill.width * fraction, ..pill });
            list.push_outline(Rect { radius: pill.height * 0.5, color: theme.outline, ..pill }, 1.0);
            ui::paint_text_vcentered(ui, list, &text, pill.x + m.padding, pill, m.detail_text, 400, theme.text_main);
            if layout::text_button(ui, list, pill.x + pill.width - button_w - m.gap * 0.5, pill.y + (pill.height - h) * 0.5, label, true, input, clicked) {
                job.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let warning = self.chooser.as_ref().and_then(Chooser::warning);
        let message = match (&warning, &self.message) {
            (Some(text), _) => Some((text.clone(), theme.warning)),
            (None, Some((text, error, at))) if at.elapsed().as_secs_f32() < MESSAGE_SECONDS => Some((text.clone(), if *error { theme.error } else { theme.text_main })),
            _ => None,
        };
        if let Some((text, color)) = message {
            let text = ui::elide(ui, &text, m.detail_text, 400, card.width * 0.6);
            let w = ui.measure(&text, m.detail_text) + m.padding * 2.0;
            let pill = rect(card.x + (card.width - w) * 0.5, bottom, w, m.status_height);
            list.push_rect(Rect { radius: pill.height * 0.5, color: theme.surface_highest, ..pill });
            list.push_outline(Rect { radius: pill.height * 0.5, color: theme.outline, ..pill }, 1.0);
            ui::paint_text_centered(ui, list, &text, pill, m.detail_text, color);
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
        let Some(job) = self.job.take() else { return false };
        match job.record {
            Some(ops::Kind::Move) => self.history.push(Step::Move(p.made.clone())),
            Some(ops::Kind::Copy) => self.history.push(Step::Copy(p.made.clone())),
            None => {}
        }
        self.reload();
        match p.errors.as_slice() {
            [] => self.say(job.done, false),
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
        let stamp = self.stamp();
        if stamp != self.dir_stamp {
            self.reload();
            // What is shown may have changed too.
            self.preview.forget();
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

/// A floating card: the theme's surface, rounded, with a hairline edge.
fn paint_card(ui: &mut AppUi, list: &mut DisplayList, area: Rect) {
    let radius = ui.config.metrics.files.card_radius;
    list.push_rect(Rect { radius, color: ui.theme.surface, ..area });
    list.push_outline(Rect { radius, color: ui.theme.outline, ..area }, 1.0);
}

/// An entry for a folder that is not in the list (the one shown).
fn folder_entry(path: &Path) -> Entry {
    Entry {
        name: path.file_name().map_or_else(|| "/".into(), |n| n.to_string_lossy().into_owned()),
        path: path.to_path_buf(),
        is_dir: true,
        is_link: false,
        size: 0,
        modified: None,
        mime: crate::mime::DIRECTORY.into(),
    }
}

/// `path` with the home folder written `~`.
fn tilde(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

pub fn count(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

impl App for Browser {
    fn start(&mut self, waker: Waker) {
        self.drives = Some(crate::drives::Drives::start(waker.clone()));
        self.waker = Some(waker);
    }

    fn title(&self) -> String {
        if let Some(chooser) = &self.chooser {
            return chooser.title.clone();
        }
        self.folder_name()
    }

    fn paint(&mut self, ui: &mut AppUi, list: &mut DisplayList, area: Rect, pointer: &Pointer) {
        let m: FilesMetrics = ui.config.metrics.files.clone();
        let config = ui.config.files.clone();
        // An open menu or dialog takes the pointer from what it covers.
        // A dialog opened during this frame (the status bar's Empty the
        // trash) must not take the click that opened it as a click
        // outside, which closed it at once.
        let confirming = self.confirm.is_some() || self.unlock.is_some();
        let covered = confirming || self.menu_rect.is_some_and(|r| layout::hovered(&pointer.input, r));
        let masked = InputState { pointer: if covered { None } else { pointer.input.pointer }, pressed: pointer.input.pressed };
        let input = &masked;
        let clicked = pointer.clicked && !covered;
        let pointer_in = Pointer { input: masked.clone(), clicked, secondary: pointer.secondary && !covered, ..pointer.clone() };

        // Floating cards on the ground: the places, the folder and its
        // preview, under a row of pills, with the chooser's bar below.
        list.push_rect(Rect { color: ui.theme.surface_lowest, ..area });
        let (margin, gap) = (m.window_margin, m.pane_gap);
        let chooser_h = if self.chooser.is_some() { m.chooser_height + gap } else { 0.0 };
        let toolbar = rect(area.x + margin, area.y + margin, area.width - margin * 2.0, m.toolbar_height);
        let body_y = toolbar.y + toolbar.height + gap;
        let body_h = (area.y + area.height - margin - chooser_h - body_y).max(0.0);
        let sidebar = rect(area.x + margin, body_y, m.sidebar_width.min(area.width * 0.4), body_h);
        let main_x = sidebar.x + sidebar.width + gap;
        let main_w = area.x + area.width - margin - main_x;
        // The preview takes the right while the folder keeps room for its
        // names and columns; a narrower window hides it.
        let room = main_w - m.preview_width - gap >= m.preview_width + m.size_column + m.modified_column;
        let pane_w = if room && self.preview_open(&config) { m.preview_width } else { 0.0 };
        let main = rect(main_x, body_y, main_w - if pane_w > 0.0 { pane_w + gap } else { 0.0 }, body_h);

        self.drop_zones.clear();
        paint_card(ui, list, sidebar);
        self.paint_sidebar(ui, list, sidebar, input, clicked, pointer_in.secondary);
        paint_card(ui, list, main);
        let inner = rect(main.x + m.padding, main.y + m.padding * 0.5, main.width - m.padding * 2.0, main.height - m.padding);
        let title = rect(inner.x, inner.y, inner.width, m.title_height);
        self.paint_title(ui, list, title, input, clicked);
        let listing = rect(inner.x, title.y + title.height, inner.width, inner.y + inner.height - title.y - title.height);
        self.paint_entries(ui, list, listing, input, &pointer_in, clicked, &config);
        if pane_w > 0.0 {
            let pane = rect(main.x + main.width + gap, body_y, pane_w, body_h);
            paint_card(ui, list, pane);
            self.paint_preview(ui, list, pane, &config);
        } else if !self.preview.showing.is_empty() {
            // Hidden: its picture is not kept.
            self.preview.clear();
            self.facts = None;
        }
        self.paint_floating(ui, list, main, input, clicked);
        if let Some(mut chooser) = self.chooser.take() {
            let bar = rect(area.x + margin, body_y + body_h + gap, area.width - margin * 2.0, m.chooser_height);
            let selected = self.selected_paths();
            chooser.paint(ui, list, bar, input, clicked, &self.dir, &selected);
            self.chooser = Some(chooser);
        }
        self.paint_toolbar(ui, list, toolbar, input, clicked);
        self.paint_drag(ui, list, pointer);
        if let Some((zone, _)) = &self.hover_in {
            list.push_layer();
            list.push_outline(Rect { radius: m.row_radius, color: ui.theme.primary, ..*zone }, 2.0);
        }
        self.paint_confirm(ui, list, area, &pointer.input, pointer.clicked && confirming);
        self.paint_unlock(ui, list, area, &pointer.input, pointer.clicked && confirming);

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
        if key.keysym == Keysym::Escape && self.drag.as_ref().is_some_and(|d| d.active) {
            self.drag = None;
            return Flow::Continue;
        }
        let config = ui.config.files.clone();
        if let Some(unlock) = &mut self.unlock {
            if unlock.busy {
                return Flow::Continue;
            }
            match unlock.field.key(key.keysym, key.text.as_deref(), key.ctrl) {
                FieldKey::Submit(passphrase) => self.submit_unlock(passphrase),
                FieldKey::Escape => self.unlock = None,
                FieldKey::Changed => unlock.error = None,
                FieldKey::Ignored => {}
            }
            self.repaint = true;
            return Flow::Continue;
        }
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
            Keysym::F9 => self.toggle_preview(),
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
            Keysym::z | Keysym::Z if key.ctrl => self.undo(),
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
        let r = self.sidebar_rect;
        if point.0 >= r.x && point.0 < r.x + r.width && point.1 >= r.y && point.1 < r.y + r.height {
            self.sidebar_scroll = (self.sidebar_scroll + delta).clamp(0.0, (self.sidebar_height - r.height).max(0.0));
            return;
        }
        self.scroll = (self.scroll + delta).max(0.0);
    }

    fn tick(&mut self, _dt: f32) -> bool {
        // A running job's bar moves; the folder is looked at once a second.
        self.check_dir();
        // A message that ran out is taken down (the loop looks at least
        // once a second).
        let expired = self.message.as_ref().is_some_and(|(_, _, at)| at.elapsed().as_secs_f32() >= MESSAGE_SECONDS);
        if expired {
            self.message = None;
        }
        self.job.is_some() || expired || std::mem::take(&mut self.repaint)
    }

    fn poll(&mut self) -> bool {
        let thumbs = self.take_thumbnails();
        let job = self.take_job();
        let preview = self.preview.take();
        let drives = self.take_drives();
        thumbs || job || preview || drives
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

    fn drag_out(&mut self) -> Option<stratum_ui::app::DragOut> {
        let drag = self.drag.take().filter(|d| d.active)?;
        self.handed_out = drag.paths.clone();
        self.repaint = true;
        Some(stratum_ui::app::DragOut { paths: drag.paths, icon: drag.icon })
    }

    fn drag_out_ended(&mut self, _taken: bool) {
        // Another app may have moved them: the folder is read again.
        self.handed_out.clear();
        self.hover_in = None;
        self.reload();
        self.repaint = true;
    }

    fn drop_hover(&mut self, point: Option<(f32, f32)>) -> Option<stratum_ui::app::DropAction> {
        self.hover_in = point.and_then(|p| self.drop_target(p));
        let (_, dest) = self.hover_in.as_ref()?;
        if trash::is_trash(dest) {
            return Some(stratum_ui::app::DropAction::Move);
        }
        // Another app's files are copied: what they are is known only at
        // the drop, and a copy never loses them. Our own follow the
        // in-window rule.
        if self.handed_out.is_empty() {
            return Some(stratum_ui::app::DropAction::Copy);
        }
        let dev = |p: &Path| std::fs::metadata(p).ok().map(|m| std::os::unix::fs::MetadataExt::dev(&m));
        Some(match dnd::action(&self.handed_out, dest, false, false, dev) {
            dnd::DropAction::Move => stratum_ui::app::DropAction::Move,
            dnd::DropAction::Copy => stratum_ui::app::DropAction::Copy,
        })
    }

    fn drop_files(&mut self, point: (f32, f32), paths: Vec<PathBuf>, action: stratum_ui::app::DropAction) {
        self.hover_in = None;
        self.repaint = true;
        let Some((_, dest)) = self.drop_target(point) else { return };
        let paths: Vec<PathBuf> = paths.into_iter().filter(|p| dnd::valid_target(std::slice::from_ref(p), &dest)).collect();
        if paths.is_empty() {
            return;
        }
        if trash::is_trash(&dest) {
            return self.trash_paths(paths);
        }
        if self.job.is_some() {
            return self.say("Wait for the copy or move to finish, or cancel it".into(), true);
        }
        let kind = match action {
            stratum_ui::app::DropAction::Move => ops::Kind::Move,
            stratum_ui::app::DropAction::Copy => ops::Kind::Copy,
        };
        self.start_job(kind, paths, dest);
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
