//! The launcher: a search box over applications, open windows, shell
//! commands and a calculator (`stratum-cli launcher toggle`), and in modes
//! of its own over emoji and the shell's clipboard history
//! (`stratum-cli launcher toggle emoji|clipboard`).
//!
//! It holds nothing while closed: the desktop entries, windows and results
//! are read when it opens and dropped when it closes, and its icons are
//! evicted from the icon cache. Ranking is [`fuzzy`] plus how often an app
//! was launched ([`apps::Usage`]).

pub mod apps;
pub mod calc;
pub mod clipboard;
pub mod emoji;
pub mod fuzzy;
pub mod grid;

use std::collections::HashMap;

use crate::ipc::IpcCall;
use crate::render::display_list::{Color, DisplayList, Point, Rect};
use crate::ui::style::LauncherMetrics;
use crate::ui::text_field::{FieldKey, FieldStyle, TextField};
use crate::ui::{self, PaintCtx};

/// What the launcher searches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Apps, windows, commands and the calculator.
    #[default]
    All,
    Emoji,
    /// The shell's clipboard history.
    Clipboard,
}

impl Mode {
    /// The names `stratum-cli launcher open <mode>` takes.
    pub const NAMES: [&'static str; 3] = ["apps", "emoji", "clipboard"];

    pub fn from_name(name: &str) -> Option<Mode> {
        match name {
            "apps" => Some(Mode::All),
            "emoji" => Some(Mode::Emoji),
            "clipboard" => Some(Mode::Clipboard),
            _ => None,
        }
    }
}

/// The clipboard history, or why there is none.
#[derive(Debug, Default)]
enum Clips {
    #[default]
    Loading,
    Ready(Vec<clipboard::Clip>),
    Failed(String),
}

/// An open window, from Hyprland.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub address: String,
    pub class: String,
    pub title: String,
    pub workspace: String,
}

/// A shell action offered by name.
#[derive(Clone, Debug, PartialEq)]
pub struct Command {
    pub title: String,
    pub glyph: &'static str,
    pub call: IpcCall,
}

#[derive(Clone, Debug, PartialEq)]
enum Kind {
    App(usize),
    Window(usize),
    Command(usize),
    Calc(String),
    Emoji(usize),
    Clip(usize),
}

#[derive(Clone, Debug)]
struct Item {
    kind: Kind,
    score: f32,
}

/// What the shell should do after a key, click or wheel.
#[derive(Debug, PartialEq)]
pub enum LauncherAction {
    None,
    Redraw,
    /// Closed without acting.
    Close,
    /// Run an app (already closed). `terminal` asks for the configured
    /// terminal around it.
    Launch { argv: Vec<String>, terminal: bool },
    /// Put text on the clipboard.
    Copy(String),
    FocusWindow(String),
    Ipc(IpcCall),
    /// Put a clipboard history entry back on the clipboard.
    RestoreClip(clipboard::Clip),
    /// Remove a clipboard history entry, by id. The launcher stays open.
    DeleteClip(u64),
}

#[derive(Default)]
pub struct Launcher {
    target: Option<String>,
    mode: Mode,
    emoji: Vec<emoji::Emoji>,
    /// Emoji drawn in colour, by index, while open.
    emoji_images: HashMap<usize, Option<crate::icons::IconImage>>,
    clips: Clips,
    /// Image entries' thumbnails by id (`None` while decoding or when it
    /// failed), and the ids a paint found missing.
    thumbs: HashMap<u64, Option<crate::icons::IconImage>>,
    thumb_requests: Vec<(u64, std::path::PathBuf)>,
    /// The colour emoji font (`[launcher] emoji_font`).
    emoji_font: Option<std::sync::Arc<str>>,
    /// The emoji grid as last laid out, rebuilt when the results or the
    /// column count change; its scroll offset, whether that follows the
    /// selection (a wheel stops it), and the row height and rows in view.
    grid: grid::Grid,
    grid_stale: bool,
    scroll: f32,
    follow: bool,
    cell_height: f32,
    grid_rows: usize,
    /// The clipboard preview: the selected text entry's start (and whether
    /// it goes on), and the selected image decoded at the pane's size,
    /// with the decode the last paint asked for.
    preview_text: Option<(u64, String, bool)>,
    preview_image: Option<(u64, Option<crate::icons::IconImage>)>,
    preview_request: Option<(u64, std::path::PathBuf, u32, u32)>,
    field: TextField,
    apps: Vec<apps::App>,
    windows: Vec<Window>,
    commands: Vec<Command>,
    usage: apps::Usage,
    results: Vec<Item>,
    selected: usize,
    /// The first result shown.
    first: usize,
    /// Rows as last painted, for clicks and hover.
    rows: Vec<(Rect, usize)>,
}

/// How many results are kept for scrolling.
const MAX_RESULTS: usize = 60;

/// How much of a text entry the preview reads.
const PREVIEW_BYTES: u64 = 16 * 1024;

impl Launcher {
    pub fn is_open(&self) -> bool {
        self.target.is_some()
    }

    pub fn target(&self) -> Option<&str> {
        self.target.as_deref()
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Open on `output` in `mode`. All searches the apps it reads and the
    /// given windows and commands; the clipboard history is given through
    /// [`Launcher::set_clips`].
    pub fn open_on(&mut self, output: &str, mode: Mode, windows: Vec<Window>, commands: Vec<Command>) {
        *self = Self::default();
        self.mode = mode;
        match mode {
            Mode::All => {
                let started = std::time::Instant::now();
                self.apps = apps::load();
                tracing::debug!("launcher: {} apps in {:?}", self.apps.len(), started.elapsed());
                self.usage = apps::Usage::load();
                self.windows = windows;
                self.commands = commands;
            }
            Mode::Emoji => self.emoji = emoji::all(),
            Mode::Clipboard => self.clips = Clips::Loading,
        }
        self.target = Some(output.to_string());
        self.refilter();
    }

    /// The font emoji are drawn in, set after opening.
    pub fn set_emoji_font(&mut self, family: &str) {
        self.emoji_font = Some(std::sync::Arc::from(family));
    }

    /// The clipboard history, or why there is none. Given again when a
    /// copy is made while the list is open, so the query stays.
    pub fn set_clips(&mut self, clips: Result<Vec<clipboard::Clip>, String>) {
        if self.is_open() && self.mode == Mode::Clipboard {
            self.clips = clips.map_or_else(Clips::Failed, Clips::Ready);
            self.refilter();
        }
    }

    /// A decoded thumbnail for an image entry.
    pub fn set_thumb(&mut self, id: u64, image: Option<crate::icons::IconImage>) {
        if self.is_open() {
            self.thumbs.insert(id, image);
        }
    }

    /// The selected image entry decoded for the preview.
    pub fn set_preview(&mut self, id: u64, image: Option<crate::icons::IconImage>) {
        if self.is_open() {
            self.preview_image = Some((id, image));
        }
    }

    /// The preview decode the last paint asked for: an id, its file, and
    /// the size in pixels to fit.
    pub fn take_preview_request(&mut self) -> Option<(u64, std::path::PathBuf, u32, u32)> {
        self.preview_request.take()
    }

    /// Image entries the last paint showed without a thumbnail, each asked
    /// for once.
    pub fn take_thumb_requests(&mut self) -> Vec<(u64, std::path::PathBuf)> {
        std::mem::take(&mut self.thumb_requests)
    }

    /// Close, dropping everything it read.
    pub fn close(&mut self) {
        *self = Self::default();
    }

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }

    fn refilter(&mut self) {
        let query = self.field.text().trim().to_string();
        let mut results = match self.mode {
            Mode::All => self.search_all(&query),
            Mode::Emoji => self
                .emoji
                .iter()
                .enumerate()
                .filter_map(|(i, e)| {
                    let score = if query.is_empty() {
                        Some(0)
                    } else {
                        // Names are phrases ("face with tears of joy"):
                        // match whole words, not scattered letters.
                        let lower = query.to_lowercase();
                        let named = e.name.to_lowercase().contains(&lower);
                        named.then(|| fuzzy::score(&query, e.name)).flatten().or_else(|| {
                            e.group.to_lowercase().contains(&lower).then_some(-200)
                        })
                    };
                    score.map(|s| Item { kind: Kind::Emoji(i), score: s as f32 })
                })
                .collect(),
            Mode::Clipboard => match &self.clips {
                Clips::Ready(clips) => {
                    let lower = query.to_lowercase();
                    clips
                        .iter()
                        .enumerate()
                        .filter(|(_, c)| lower.is_empty() || c.preview.to_lowercase().contains(&lower))
                        // Newest first, as the store keeps them.
                        .map(|(i, _)| Item { kind: Kind::Clip(i), score: -(i as f32) })
                        .collect()
                }
                _ => Vec::new(),
            },
        };
        // Stable, so equal scores keep their order (apps by name, emoji
        // as Unicode orders them).
        results.sort_by(|a, b| b.score.total_cmp(&a.score));
        if self.mode == Mode::All {
            results.truncate(MAX_RESULTS);
        }
        self.results = results;
        self.selected = 0;
        self.first = 0;
        self.grid_stale = true;
        self.scroll = 0.0;
        self.follow = true;
    }

    /// The emoji results in groups for the grid: Unicode's groups for an
    /// empty query, one ranked group for a search.
    fn layout_grid(&mut self, columns: usize) {
        let groups: Vec<(Option<String>, usize)> = if self.field.text().trim().is_empty() {
            let mut groups: Vec<(Option<String>, usize)> = Vec::new();
            for item in &self.results {
                let Kind::Emoji(i) = item.kind else { continue };
                let name = self.emoji[i].group;
                match groups.last_mut() {
                    Some((Some(last), count)) if last == name => *count += 1,
                    _ => groups.push((Some(name.to_string()), 1)),
                }
            }
            groups
        } else {
            vec![(None, self.results.len())]
        };
        self.grid = grid::Grid::build(&groups, columns);
        self.grid_stale = false;
    }

    fn search_all(&self, query: &str) -> Vec<Item> {
        let query = query.to_string();
        let now = Self::now();
        let mut results = Vec::new();
        if let Some(value) = calc::evaluate(&query) {
            results.push(Item { kind: Kind::Calc(calc::format(value)), score: f32::MAX });
        }
        for (i, app) in self.apps.iter().enumerate() {
            let usage = self.usage.weight(&app.id, now);
            let score = if query.is_empty() {
                Some(usage)
            } else {
                // The name counts most; the generic name, keywords and
                // comment find apps by what they do.
                let fields = std::iter::once((app.name.as_str(), 0))
                    .chain(app.generic.as_deref().map(|g| (g, 40)))
                    .chain(app.keywords.iter().map(|k| (k.as_str(), 60)));
                // A description is a sentence: scattered letters in it match
                // almost anything, so only the words themselves count.
                let comment = app
                    .comment
                    .as_deref()
                    .filter(|c| c.to_lowercase().contains(&query.to_lowercase()))
                    .and_then(|c| fuzzy::score(&query, c))
                    .map(|s| s - 120);
                fields
                    .filter_map(|(text, penalty)| fuzzy::score(&query, text).map(|s| s - penalty))
                    .chain(comment)
                    .max()
                    .map(|s| s as f32 + 20.0 * (1.0 + usage).ln())
            };
            if let Some(score) = score {
                results.push(Item { kind: Kind::App(i), score });
            }
        }
        if !query.is_empty() {
            for (i, window) in self.windows.iter().enumerate() {
                let score = [(&window.title, 0), (&window.class, 20)]
                    .into_iter()
                    .filter_map(|(text, penalty)| fuzzy::score(&query, text).map(|s| s - penalty))
                    .max();
                if let Some(score) = score {
                    results.push(Item { kind: Kind::Window(i), score: score as f32 - 30.0 });
                }
            }
            for (i, command) in self.commands.iter().enumerate() {
                if let Some(score) = fuzzy::score(&query, &command.title) {
                    results.push(Item { kind: Kind::Command(i), score: score as f32 - 20.0 });
                }
            }
        }
        results
    }

    fn select(&mut self, index: usize, rows: usize) {
        if self.results.is_empty() {
            return;
        }
        self.selected = index.min(self.results.len() - 1);
        if self.selected < self.first {
            self.first = self.selected;
        } else if self.selected >= self.first + rows {
            self.first = self.selected + 1 - rows;
        }
    }

    /// A key; `text` is what it types. `rows` is how many results show.
    pub fn key(
        &mut self,
        keysym: smithay_client_toolkit::seat::keyboard::Keysym,
        text: Option<&str>,
        ctrl: bool,
        rows: usize,
    ) -> LauncherAction {
        use smithay_client_toolkit::seat::keyboard::Keysym;
        if !self.is_open() {
            return LauncherAction::None;
        }
        let last = self.results.len().saturating_sub(1);
        let step = |this: &mut Self, delta: isize| {
            let next = (this.selected as isize + delta).clamp(0, last as isize) as usize;
            this.select(next, rows);
            LauncherAction::Redraw
        };
        if self.mode == Mode::Emoji {
            let page = self.grid_rows.max(1) as isize;
            let target = match keysym {
                Keysym::Left => Some(self.selected.saturating_sub(1)),
                Keysym::Right => Some((self.selected + 1).min(last)),
                Keysym::Up => Some(self.grid.vertical(self.selected, -1)),
                Keysym::Down => Some(self.grid.vertical(self.selected, 1)),
                Keysym::p | Keysym::k if ctrl => Some(self.grid.vertical(self.selected, -1)),
                Keysym::n | Keysym::j if ctrl => Some(self.grid.vertical(self.selected, 1)),
                Keysym::Page_Up => Some(self.grid.vertical(self.selected, -page)),
                Keysym::Page_Down => Some(self.grid.vertical(self.selected, page)),
                _ => None,
            };
            if let Some(target) = target {
                self.selected = target;
                self.follow = true;
                return LauncherAction::Redraw;
            }
        }
        match keysym {
            Keysym::Up => return step(self, -1),
            Keysym::Down => return step(self, 1),
            Keysym::p | Keysym::k if ctrl => return step(self, -1),
            Keysym::n | Keysym::j if ctrl => return step(self, 1),
            Keysym::Page_Up => return step(self, -(rows as isize)),
            Keysym::Page_Down => return step(self, rows as isize),
            Keysym::Return | Keysym::KP_Enter => return self.activate(self.selected),
            // The field has no cursor, so Delete is free to remove the
            // selected history entry.
            Keysym::Delete if self.mode == Mode::Clipboard => return self.delete_selected(rows),
            Keysym::Escape => {
                if self.field.text().is_empty() {
                    self.close();
                    return LauncherAction::Close;
                }
                self.field.clear();
                self.refilter();
                return LauncherAction::Redraw;
            }
            _ => {}
        }
        match self.field.key(keysym, text, ctrl) {
            FieldKey::Changed => {
                self.refilter();
                LauncherAction::Redraw
            }
            _ => LauncherAction::None,
        }
    }

    fn delete_selected(&mut self, rows: usize) -> LauncherAction {
        let (Some(Item { kind: Kind::Clip(i), .. }), Clips::Ready(clips)) = (self.results.get(self.selected), &mut self.clips) else {
            return LauncherAction::None;
        };
        let clip = clips.remove(*i);
        let selected = self.selected;
        self.refilter();
        self.select(selected, rows);
        LauncherAction::DeleteClip(clip.id)
    }

    /// Act on result `index`, closing the launcher.
    fn activate(&mut self, index: usize) -> LauncherAction {
        let Some(item) = self.results.get(index) else {
            return LauncherAction::None;
        };
        let action = match &item.kind {
            Kind::App(i) => {
                let app = &self.apps[*i];
                self.usage.record(&app.id, Self::now());
                self.usage.save();
                LauncherAction::Launch { argv: app.argv.clone(), terminal: app.terminal }
            }
            Kind::Window(i) => LauncherAction::FocusWindow(self.windows[*i].address.clone()),
            Kind::Command(i) => LauncherAction::Ipc(self.commands[*i].call.clone()),
            Kind::Calc(value) => LauncherAction::Copy(value.clone()),
            Kind::Emoji(i) => LauncherAction::Copy(self.emoji[*i].glyph.to_string()),
            Kind::Clip(i) => match &self.clips {
                Clips::Ready(clips) => LauncherAction::RestoreClip(clips[*i].clone()),
                _ => LauncherAction::None,
            },
        };
        self.close();
        action
    }

    pub fn on_click(&mut self, point: Point) -> LauncherAction {
        match self.rows.iter().find(|(rect, _)| rect.contains(point)) {
            Some(&(_, index)) => self.activate(index),
            None => LauncherAction::None,
        }
    }

    /// The pointer selects the row under it.
    pub fn on_hover(&mut self, point: Point) -> LauncherAction {
        match self.rows.iter().find(|(rect, _)| rect.contains(point)) {
            Some(&(_, index)) if index != self.selected => {
                self.selected = index;
                LauncherAction::Redraw
            }
            _ => LauncherAction::None,
        }
    }

    /// The wheel scrolls a row per notch.
    pub fn on_wheel(&mut self, notches: f32, rows: usize) -> LauncherAction {
        if self.mode == Mode::Emoji {
            // Clamped when painted, which knows the grid's height.
            self.scroll = (self.scroll + notches * self.cell_height).max(0.0);
            self.follow = false;
            return LauncherAction::Redraw;
        }
        let max_first = self.results.len().saturating_sub(rows);
        let first = (self.first as f32 + notches.round()).clamp(0.0, max_first as f32) as usize;
        if first == self.first {
            return LauncherAction::None;
        }
        self.first = first;
        self.selected = self.selected.clamp(first, first + rows - 1);
        LauncherAction::Redraw
    }

    /// The surface size for `rows` results in `mode`: the emoji grid and
    /// the clipboard's list with its preview are wider.
    pub fn size(metrics: &LauncherMetrics, rows: usize, mode: Mode) -> (u32, u32) {
        let height = metrics.padding * 2.0 + metrics.field_height + metrics.gap + metrics.row_height * rows as f32;
        let width = if mode == Mode::All { metrics.width } else { metrics.wide_width };
        (width.round() as u32, height.round() as u32)
    }

    fn label(&self, item: &Item) -> (String, String) {
        match &item.kind {
            Kind::App(i) => {
                let app = &self.apps[*i];
                let detail = app.generic.clone().or_else(|| app.comment.clone()).unwrap_or_default();
                (app.name.clone(), detail)
            }
            Kind::Window(i) => {
                let w = &self.windows[*i];
                (w.title.clone(), format!("{} · workspace {}", w.class, w.workspace))
            }
            Kind::Command(i) => (self.commands[*i].title.clone(), "Stratum".to_string()),
            Kind::Calc(value) => (format!("= {value}"), "Enter copies the result".to_string()),
            Kind::Emoji(i) => (self.emoji[*i].name.to_string(), self.emoji[*i].group.to_string()),
            Kind::Clip(i) => {
                let Clips::Ready(clips) = &self.clips else {
                    return (String::new(), String::new());
                };
                let clip = &clips[*i];
                match &clip.image {
                    Some(clipboard::ImageInfo { format, size: Some((w, h)) }) => ("Image".to_string(), format!("{format} · {w}×{h}")),
                    Some(image) => ("Image".to_string(), image.format.clone()),
                    None => (clip.preview.clone(), String::new()),
                }
            }
        }
    }

    pub fn paint(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, rect: Rect, rows: usize) {
        let m = ctx.metrics.launcher.clone();
        let theme = ctx.theme;
        ui::paint_frame(
            list,
            Rect { radius: ctx.appearance.corner_radius, ..rect },
            ctx.appearance.frame_border(),
            theme.border_active,
            theme.bg_main,
        );
        let inner = Rect {
            x: rect.x + m.padding,
            y: rect.y + m.padding,
            width: rect.width - m.padding * 2.0,
            height: rect.height - m.padding * 2.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };

        // The search field, with a magnifier before it.
        let field = Rect { height: m.field_height, radius: m.row_radius, color: theme.bg_widget, ..inner };
        list.push_rect(field);
        let glyph_box = Rect { width: m.field_height, ..field };
        ui::paint_icon_centered(ctx, list, "\u{f0349}", glyph_box, m.search_icon, theme.text_muted);
        let text_box = Rect { x: field.x + m.field_height, width: field.width - m.field_height, ..field };
        self.field.paint(
            ctx,
            list,
            text_box,
            FieldStyle {
                text: m.field_text,
                padding: 0.0,
                caret_width: m.caret_width,
                masked: false,
                placeholder: Some(match self.mode {
                    Mode::All => "Search apps, windows and commands, or calculate",
                    Mode::Emoji => "Search emoji",
                    Mode::Clipboard => "Search the clipboard",
                }),
            },
            false,
        );

        let body_top = field.y + field.height + m.gap;
        let body = Rect { y: body_top, height: inner.y + inner.height - body_top, ..inner };
        self.rows.clear();
        match self.mode {
            Mode::All => self.paint_list(ctx, list, body, rows, &m),
            Mode::Emoji => self.paint_grid(ctx, list, body, &m),
            Mode::Clipboard => {
                let list_width = (body.width * m.list_fraction).round();
                let left = Rect { width: list_width, ..body };
                self.paint_list(ctx, list, left, rows, &m);
                let pane = Rect {
                    x: body.x + list_width + m.pane_gap,
                    width: body.width - list_width - m.pane_gap,
                    ..body
                };
                self.paint_clip_preview(ctx, list, pane, &m);
            }
        }
    }

    /// Why nothing is listed.
    fn empty_message(&self) -> String {
        match (&self.clips, self.mode) {
            (Clips::Loading, Mode::Clipboard) => "Loading the clipboard history…".to_string(),
            (Clips::Failed(err), Mode::Clipboard) => err.clone(),
            (Clips::Ready(clips), Mode::Clipboard) if clips.is_empty() => "The clipboard history is empty".to_string(),
            _ => "No matches".to_string(),
        }
    }

    /// Results as rows with an icon, a title and a detail line.
    fn paint_list(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, area: Rect, rows: usize, m: &LauncherMetrics) {
        let theme = ctx.theme;
        let shown: Vec<usize> = (self.first..self.results.len()).take(rows).collect();
        if shown.is_empty() {
            let empty = Rect { height: m.row_height, ..area };
            let message = self.empty_message();
            let weight = ctx.metrics.typography.weight_regular;
            ui::paint_text_centered_weighted(ctx, list, &message, empty, m.title_text, weight, theme.text_muted);
            return;
        }
        let mut y = area.y;
        for index in shown {
            let row = Rect { y, height: m.row_height, radius: m.row_radius, ..area };
            self.rows.push((row, index));
            if index == self.selected {
                list.push_rect(Rect { color: theme.bg_hover, ..row });
            }
            let icon_box = Rect { width: m.row_height, ..row };
            self.paint_icon(ctx, list, &self.results[index].clone(), icon_box, m);
            let (title, detail) = self.label(&self.results[index]);
            let text_x = row.x + m.row_height + m.icon_gap;
            let text_width = row.width - (text_x - row.x) - m.icon_gap;
            let regular = ctx.metrics.typography.weight_regular;
            let medium = ctx.metrics.typography.weight_medium;
            let title = ui::elide(ctx, &title, m.title_text, medium, text_width);
            let half = Rect { x: text_x, width: text_width, height: m.row_height * 0.5, ..row };
            if detail.is_empty() {
                ui::paint_text_vcentered(ctx, list, &title, text_x, row, m.title_text, medium, theme.text_main);
            } else {
                let top = Rect { y: row.y + m.row_height * 0.08, ..half };
                ui::paint_text_vcentered(ctx, list, &title, text_x, top, m.title_text, medium, theme.text_main);
                let detail = ui::elide(ctx, &detail, m.detail_text, regular, text_width);
                let bottom = Rect { y: row.y + m.row_height * 0.46, ..half };
                ui::paint_text_vcentered(ctx, list, &detail, text_x, bottom, m.detail_text, regular, theme.text_muted);
            }
            y += m.row_height;
        }
    }

    /// The emoji picker: cells under group headers, and the selection's
    /// name in a footer.
    fn paint_grid(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, area: Rect, m: &LauncherMetrics) {
        let theme = ctx.theme;
        let regular = ctx.metrics.typography.weight_regular;
        let medium = ctx.metrics.typography.weight_medium;
        let view = Rect { height: area.height - m.footer_height, ..area };
        let footer = Rect { y: view.y + view.height, height: m.footer_height, ..area };
        let columns = ((view.width / m.emoji_cell).floor() as usize).max(1);
        if self.grid_stale || self.grid.columns != columns {
            self.layout_grid(columns);
        }
        self.cell_height = m.emoji_cell;
        self.grid_rows = ((view.height / m.emoji_cell).floor() as usize).max(1);
        if self.results.is_empty() {
            let message = self.empty_message();
            let empty = Rect { height: m.row_height, ..view };
            ui::paint_text_centered_weighted(ctx, list, &message, empty, m.title_text, regular, theme.text_muted);
            return;
        }
        if self.follow {
            self.scroll = self.grid.scroll_to(self.selected, self.scroll, view.height, m.section_height, m.emoji_cell);
        }
        let (tops, total) = self.grid.offsets(m.section_height, m.emoji_cell);
        self.scroll = self.scroll.clamp(0.0, (total - view.height).max(0.0));
        let cell_width = view.width / columns as f32;

        list.push_clip(view);
        let mut visible = std::collections::HashSet::new();
        for (line, top) in self.grid.lines.clone().iter().zip(tops) {
            let y = view.y + top - self.scroll;
            match line {
                grid::Line::Header(name) => {
                    if y + m.section_height < view.y || y > view.y + view.height {
                        continue;
                    }
                    let header = Rect { y, height: m.section_height, ..view };
                    ui::paint_text_vcentered(ctx, list, name, view.x + m.icon_gap, header, m.section_text, medium, theme.text_muted);
                }
                grid::Line::Cells { start, end } => {
                    if y + m.emoji_cell < view.y || y > view.y + view.height {
                        continue;
                    }
                    for index in *start..*end {
                        let cell = Rect {
                            x: view.x + (index - start) as f32 * cell_width,
                            y,
                            width: cell_width,
                            height: m.emoji_cell,
                            radius: m.row_radius,
                            color: Color::TRANSPARENT,
                        };
                        self.rows.push((cell, index));
                        if index == self.selected {
                            list.push_rect(Rect { color: theme.bg_hover, ..cell });
                        }
                        let Kind::Emoji(i) = self.results[index].kind else { continue };
                        visible.insert(i);
                        match self.emoji_image(ctx, i, m.emoji_glyph) {
                            Some(image) => list.push_image(crate::render::display_list::Image {
                                width: image.width,
                                height: image.height,
                                pixels: image.pixels.clone(),
                                x: cell.x + (cell.width - m.emoji_glyph) * 0.5,
                                y: cell.y + (cell.height - m.emoji_glyph) * 0.5,
                                width_logical: m.emoji_glyph,
                                height_logical: m.emoji_glyph,
                            }),
                            // No colour glyph: the emoji as text, which the
                            // font may still draw in one colour.
                            None => ui::paint_text_centered_weighted(
                                ctx,
                                list,
                                self.emoji[i].glyph,
                                cell,
                                m.emoji_glyph * 0.8,
                                regular,
                                theme.text_main,
                            ),
                        }
                    }
                }
            }
        }
        list.pop_clip();
        // Only the emoji on screen keep their images: scrolling through all
        // of them would otherwise hold ~30 MB until the picker closes.
        self.emoji_images.retain(|i, _| visible.contains(i));

        // The footer: a rule, the selection's name, and what Return does.
        list.push_rect(Rect { height: ctx.appearance.border_width, color: theme.border_inactive, ..footer });
        if let Some(Kind::Emoji(i)) = self.results.get(self.selected).map(|item| item.kind.clone()) {
            let name = self.emoji[i].name;
            let hint = "Return copies";
            let hint_style = crate::text::TextStyle {
                family: std::sync::Arc::from(theme.font.as_str()),
                size: m.detail_text,
                weight: regular,
                color: theme.text_muted,
            };
            let hint_width = ctx.text.measure(hint, &hint_style).0;
            let name = ui::elide(ctx, name, m.title_text, medium, footer.width - hint_width - m.pane_gap * 2.0);
            ui::paint_text_vcentered(ctx, list, &name, footer.x + m.icon_gap, footer, m.title_text, medium, theme.text_main);
            let x = footer.x + footer.width - hint_width - m.icon_gap;
            ui::paint_text_vcentered(ctx, list, hint, x, footer, m.detail_text, regular, theme.text_muted);
        }
    }

    /// Emoji `i` in colour at `size` logical pixels, kept while on screen.
    fn emoji_image(&mut self, ctx: &mut PaintCtx<'_>, i: usize, size: f32) -> Option<crate::icons::IconImage> {
        let glyph = self.emoji[i].glyph;
        let family = self.emoji_font.clone().unwrap_or_else(|| std::sync::Arc::from(ctx.theme.font.as_str()));
        self.emoji_images
            .entry(i)
            .or_insert_with(|| ctx.text.rasterize_color(glyph, &family, size as u32 * 2))
            .clone()
    }

    /// The selected clipboard entry, large, over what is known about it.
    fn paint_clip_preview(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, pane: Rect, m: &LauncherMetrics) {
        let theme = ctx.theme;
        let regular = ctx.metrics.typography.weight_regular;
        let medium = ctx.metrics.typography.weight_medium;
        list.push_rect(Rect { radius: m.row_radius, color: theme.bg_widget, ..pane });
        let inner = Rect {
            x: pane.x + m.pane_padding,
            y: pane.y + m.pane_padding,
            width: pane.width - m.pane_padding * 2.0,
            height: pane.height - m.pane_padding * 2.0,
            radius: 0.0,
            color: Color::TRANSPARENT,
        };
        let clip = match (&self.clips, self.results.get(self.selected).map(|i| &i.kind)) {
            (Clips::Ready(clips), Some(Kind::Clip(i))) => clips[*i].clone(),
            _ => return,
        };

        // The information section, at the bottom.
        let now = Self::now();
        let mut info: Vec<(&str, String)> = Vec::new();
        match &clip.image {
            Some(image) => {
                info.push(("Type", format!("Image ({})", image.format.to_uppercase())));
                if let Some((w, h)) = image.size {
                    info.push(("Dimensions", format!("{w}×{h}")));
                }
            }
            None => info.push(("Type", "Text".to_string())),
        }
        if clip.image.is_none() {
            if self.preview_text.as_ref().map(|p| p.0) != Some(clip.id) {
                let (text, more) = clipboard::read_text(&clip.path, PREVIEW_BYTES);
                self.preview_text = Some((clip.id, text, more));
            }
            if let Some((_, text, more)) = &self.preview_text {
                let plus = if *more { "+" } else { "" };
                info.push(("Characters", format!("{}{plus}", text.chars().count())));
                info.push(("Lines", format!("{}{plus}", text.lines().count().max(1))));
            }
        }
        info.push(("Size", clipboard::bytes(clip.len)));
        info.push(("Copied", clipboard::ago(now, clip.created)));
        let info_height = m.info_row * (info.len() + 1) as f32;
        let info_top = inner.y + inner.height - info_height;
        list.push_rect(Rect {
            y: info_top - m.pane_gap * 0.5,
            height: ctx.appearance.border_width,
            color: theme.border_inactive,
            ..inner
        });
        let mut y = info_top;
        let heading = Rect { y, height: m.info_row, ..inner };
        ui::paint_text_vcentered(ctx, list, "Information", inner.x, heading, m.detail_text, medium, theme.text_muted);
        for (label, value) in &info {
            y += m.info_row;
            let line = Rect { y, height: m.info_row, ..inner };
            ui::paint_text_vcentered(ctx, list, label, inner.x, line, m.detail_text, regular, theme.text_muted);
            let style = crate::text::TextStyle {
                family: std::sync::Arc::from(theme.font.as_str()),
                size: m.detail_text,
                weight: regular,
                color: theme.text_main,
            };
            let width = ctx.text.measure(value, &style).0;
            ui::paint_text_vcentered(ctx, list, value, inner.x + inner.width - width, line, m.detail_text, regular, theme.text_main);
        }

        // The content above it.
        let view = Rect { height: (info_top - m.pane_gap - inner.y).max(0.0), ..inner };
        if clip.image.is_some() {
            self.paint_preview_image(list, &clip, view);
        } else if let Some((_, text, more)) = self.preview_text.clone() {
            self.paint_preview_text(ctx, list, &text, more, view, m);
        }
    }

    /// The selected image, fitted into `view`; decoded on a thread the first
    /// time it is shown.
    fn paint_preview_image(&mut self, list: &mut DisplayList, clip: &clipboard::Clip, view: Rect) {
        let image = match &self.preview_image {
            Some((id, image)) if *id == clip.id => image.clone(),
            _ => {
                // Twice the logical size, for scaled outputs.
                let (w, h) = ((view.width * 2.0) as u32, (view.height * 2.0) as u32);
                if self.preview_request.as_ref().map(|r| r.0) != Some(clip.id) {
                    self.preview_request = Some((clip.id, clip.path.clone(), w.max(1), h.max(1)));
                }
                None
            }
        };
        let Some(image) = image else { return };
        // Pixels as logical pixels, shrunk to fit: a large image (decoded at
        // twice the view) fills it sharply, a small one keeps its size.
        let fit = (view.width / image.width as f32).min(view.height / image.height as f32).min(1.0);
        let (w, h) = (image.width as f32 * fit, image.height as f32 * fit);
        let (x, y) = (view.x + (view.width - w) * 0.5, view.y + (view.height - h) * 0.5);
        use crate::render::display_list::{Point, TexturedQuad};
        list.push_texture(TexturedQuad {
            // Its own texture, apart from other surfaces' keys.
            key: 0x434c_4950_0000_0000 ^ clip.id,
            width: image.width,
            height: image.height,
            pixels: image.pixels.clone(),
            corners: [Point::new(x, y), Point::new(x + w, y), Point::new(x + w, y + h), Point::new(x, y + h)],
            uv: [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
            alpha: 1.0,
        });
    }

    /// The selected text: each line as written, tabs expanded, wrapped only
    /// when it does not fit.
    fn paint_preview_text(&self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, text: &str, more: bool, view: Rect, m: &LauncherMetrics) {
        let theme = ctx.theme;
        let regular = ctx.metrics.typography.weight_regular;
        let style = crate::text::TextStyle {
            family: std::sync::Arc::from(theme.font.as_str()),
            size: m.preview_text,
            weight: regular,
            color: theme.text_main,
        };
        let line_height = ctx.text.metrics(&style).1;
        let max_lines = ((view.height / line_height).floor() as usize).max(1);
        let mut lines: Vec<String> = Vec::new();
        'source: for source in text.lines() {
            let source = source.replace('\t', "    ");
            if ctx.text.measure(&source, &style).0 <= view.width {
                lines.push(source);
            } else {
                // Wrapping collapses whitespace, so the indent is put back in
                // front of every part.
                let body = source.trim_start();
                let indent = &source[..source.len() - body.len()];
                let indent_width = ctx.text.measure(indent, &style).0;
                let width = (view.width - indent_width).max(view.width * 0.5);
                for wrapped in ui::wrap_text(ctx, body, m.preview_text, regular, width, max_lines) {
                    let line = format!("{indent}{wrapped}");
                    lines.push(ui::elide(ctx, &line, m.preview_text, regular, view.width));
                    if lines.len() >= max_lines {
                        break 'source;
                    }
                }
            }
            if lines.len() >= max_lines {
                break;
            }
        }
        let cut = lines.len() >= max_lines || more;
        lines.truncate(max_lines);
        if cut {
            if let Some(last) = lines.last_mut() {
                *last = ui::elide(ctx, &format!("{last} …"), m.preview_text, regular, view.width);
            }
        }
        list.push_clip(view);
        for (n, line) in lines.iter().enumerate() {
            let row = Rect { y: view.y + n as f32 * line_height, height: line_height, ..view };
            ui::paint_text_vcentered(ctx, list, line, view.x, row, m.preview_text, regular, theme.text_main);
        }
        list.pop_clip();
    }

    fn paint_icon(&mut self, ctx: &mut PaintCtx<'_>, list: &mut DisplayList, item: &Item, area: Rect, m: &LauncherMetrics) {
        let image = match &item.kind {
            Kind::Emoji(i) => self.emoji_image(ctx, *i, m.icon),
            Kind::Clip(i) => match &self.clips {
                Clips::Ready(clips) if clips[*i].image.is_some() => {
                    let clip = &clips[*i];
                    match self.thumbs.get(&clip.id) {
                        Some(image) => image.clone(),
                        None => {
                            self.thumbs.insert(clip.id, None);
                            self.thumb_requests.push((clip.id, clip.path.clone()));
                            None
                        }
                    }
                }
                _ => None,
            },
            Kind::App(i) => self.apps[*i].icon.as_deref().and_then(|name| {
                // An absolute path is allowed in `Icon`; the theme lookup
                // only takes names, so fall back to the generic icon.
                ctx.icons.load_icon(name, m.icon as u32, 2)
            }),
            Kind::Window(i) => ctx.icons.load_app_icon(&self.windows[*i].class, m.icon as u32, 2),
            _ => None,
        };
        if let Some(image) = image {
            list.push_image(crate::render::display_list::Image {
                width: image.width,
                height: image.height,
                pixels: image.pixels.clone(),
                x: area.x + (area.width - m.icon) * 0.5,
                y: area.y + (area.height - m.icon) * 0.5,
                width_logical: m.icon,
                height_logical: m.icon,
            });
            return;
        }
        let glyph = match &item.kind {
            Kind::Command(i) => self.commands[*i].glyph,
            Kind::Calc(_) => "\u{f00ec}",
            // nf-fa-image and nf-fa-clipboard.
            Kind::Clip(i) if matches!(&self.clips, Clips::Ready(c) if c[*i].image.is_some()) => "\u{f03e}",
            Kind::Clip(_) => "\u{f0ea}",
            // nf-md-application for an app without an icon.
            _ => "\u{f08c6}",
        };
        ui::paint_icon_centered(ctx, list, glyph, area, m.glyph, ctx.theme.primary);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay_client_toolkit::seat::keyboard::Keysym;

    fn app(id: &str, name: &str) -> apps::App {
        apps::App {
            id: id.into(),
            name: name.into(),
            generic: None,
            comment: None,
            keywords: Vec::new(),
            icon: None,
            argv: vec![name.to_lowercase()],
            terminal: false,
            exec: vec![name.to_lowercase()],
            mime_types: Vec::new(),
            hidden: false,
        }
    }

    fn clip(id: u64, text: &str) -> clipboard::Clip {
        clipboard::Clip {
            id,
            mime: crate::clipboard::TEXT.into(),
            preview: text.into(),
            image: None,
            path: format!("/clips/{id}").into(),
            len: text.len() as u64,
            created: 0,
        }
    }

    fn image_clip(id: u64) -> clipboard::Clip {
        clipboard::Clip {
            image: Some(clipboard::ImageInfo { format: "png".into(), size: Some((10, 10)) }),
            mime: "image/png".into(),
            preview: String::new(),
            ..clip(id, "")
        }
    }

    fn launcher(names: &[&str]) -> Launcher {
        let mut l = Launcher {
            apps: names.iter().map(|n| app(&format!("{n}.desktop"), n)).collect(),
            target: Some("OUT".into()),
            ..Launcher::default()
        };
        l.commands = vec![Command {
            title: "Lock screen".into(),
            glyph: "",
            call: IpcCall { target: "lockscreen".into(), function: "lock".into(), args: Vec::new() },
        }];
        l.refilter();
        l
    }

    fn type_text(l: &mut Launcher, text: &str) {
        for ch in text.chars() {
            let s = ch.to_string();
            l.key(Keysym::from_char(ch), Some(&s), false, 8);
        }
    }

    #[test]
    fn typing_filters_and_return_launches_the_best_match() {
        let mut l = launcher(&["Files", "Firefox", "Kitty"]);
        assert_eq!(l.results.len(), 3, "an empty query lists every app");
        type_text(&mut l, "fire");
        assert_eq!(l.results.len(), 1);
        let action = l.key(Keysym::Return, None, false, 8);
        assert_eq!(action, LauncherAction::Launch { argv: vec!["firefox".into()], terminal: false });
        assert!(!l.is_open(), "launching closes it");
    }

    #[test]
    fn arrows_move_and_scroll_the_selection() {
        let mut l = launcher(&["A1", "A2", "A3", "A4", "A5"]);
        for _ in 0..3 {
            l.key(Keysym::Down, None, false, 2);
        }
        assert_eq!((l.selected, l.first), (3, 2), "the selection stays in view");
        for _ in 0..10 {
            l.key(Keysym::Down, None, false, 2);
        }
        assert_eq!(l.selected, 4, "stops at the last");
        l.key(Keysym::Up, None, false, 2);
        l.key(Keysym::Up, None, false, 2);
        l.key(Keysym::Up, None, false, 2);
        assert_eq!((l.selected, l.first), (1, 1));
    }

    #[test]
    fn commands_and_the_calculator_show_only_for_a_query() {
        let mut l = launcher(&["Files"]);
        assert!(l.results.iter().all(|r| matches!(r.kind, Kind::App(_))));
        type_text(&mut l, "lock");
        assert!(matches!(l.results[0].kind, Kind::Command(0)));
        let action = l.key(Keysym::Return, None, false, 8);
        assert!(matches!(action, LauncherAction::Ipc(ref call) if call.target == "lockscreen"));

        let mut l = launcher(&["Files"]);
        type_text(&mut l, "6*7");
        assert_eq!(l.results[0].kind, Kind::Calc("42".into()));
        assert_eq!(l.key(Keysym::Return, None, false, 8), LauncherAction::Copy("42".into()));
    }

    #[test]
    fn escape_clears_then_closes() {
        let mut l = launcher(&["Files"]);
        type_text(&mut l, "x");
        assert_eq!(l.key(Keysym::Escape, None, false, 8), LauncherAction::Redraw);
        assert!(l.is_open());
        assert_eq!(l.key(Keysym::Escape, None, false, 8), LauncherAction::Close);
        assert!(!l.is_open());
    }

    #[test]
    fn the_emoji_mode_finds_by_name_and_copies() {
        let mut l = Launcher::default();
        l.open_on("OUT", Mode::Emoji, Vec::new(), Vec::new());
        assert!(l.results.len() > 1800, "an empty query lists them all");
        type_text(&mut l, "face with tears of joy");
        assert!(matches!(l.results[0].kind, Kind::Emoji(i) if l.emoji[i].name == "face with tears of joy"));
        assert_eq!(l.key(Keysym::Return, None, false, 8), LauncherAction::Copy("😂".into()));

        // Names have capitals ("flag: Japan"); the query's case is ignored.
        let mut l = Launcher::default();
        l.open_on("OUT", Mode::Emoji, Vec::new(), Vec::new());
        type_text(&mut l, "flag: jap");
        assert!(matches!(l.results[0].kind, Kind::Emoji(i) if l.emoji[i].name == "flag: Japan"));
    }

    #[test]
    fn the_clipboard_mode_filters_restores_and_deletes() {
        let mut l = Launcher::default();
        l.open_on("OUT", Mode::Clipboard, Vec::new(), Vec::new());
        assert!(l.results.is_empty(), "nothing until the history arrives");
        l.set_clips(Ok(vec![clip(3, "ssh key fingerprint"), image_clip(2), clip(1, "nix build")]));
        assert_eq!(l.results.len(), 3);
        assert!(matches!(l.results[0].kind, Kind::Clip(0)), "newest first");
        type_text(&mut l, "NIX");
        assert_eq!(l.results.len(), 1);
        assert_eq!(l.key(Keysym::Delete, None, false, 8), LauncherAction::DeleteClip(1));
        assert!(l.is_open() && l.results.is_empty(), "deleting keeps the launcher open");
        l.key(Keysym::Escape, None, false, 8);
        assert_eq!(l.results.len(), 2);
        assert_eq!(l.key(Keysym::Return, None, false, 8), LauncherAction::RestoreClip(clip(3, "ssh key fingerprint")));
    }

    #[test]
    fn arrows_move_through_the_emoji_grid() {
        let mut l = Launcher::default();
        l.open_on("OUT", Mode::Emoji, Vec::new(), Vec::new());
        l.layout_grid(8);
        l.key(Keysym::Right, None, false, 8);
        l.key(Keysym::Right, None, false, 8);
        assert_eq!(l.selected, 2);
        l.key(Keysym::Down, None, false, 8);
        assert_eq!(l.selected, 10, "a row down, same column");
        l.key(Keysym::Up, None, false, 8);
        l.key(Keysym::Left, None, false, 8);
        assert_eq!(l.selected, 1);
        // Searching makes one ranked grid without group headers.
        type_text(&mut l, "cat");
        l.layout_grid(8);
        assert!(l.grid.lines.iter().all(|line| matches!(line, grid::Line::Cells { .. })));
    }

    #[test]
    fn a_late_history_is_ignored_in_another_mode() {
        let mut l = Launcher::default();
        l.open_on("OUT", Mode::Emoji, Vec::new(), Vec::new());
        let before = l.results.len();
        l.set_clips(Ok(vec![clip(1, "x")]));
        assert_eq!(l.results.len(), before);
    }

    #[test]
    fn usage_ranks_an_empty_query() {
        let mut l = launcher(&["Alpha", "Beta"]);
        l.usage.record("Beta.desktop", Launcher::now());
        l.refilter();
        assert!(matches!(l.results[0].kind, Kind::App(1)), "the launched app comes first");
    }
}
