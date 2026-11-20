//! The file chooser: the browser with a bar for the file's name (when
//! saving), the type filter, and Cancel and Open (or Save). It is what the
//! desktop portal shows for every app's Open and Save dialogs
//! (`portal.rs`), started as `stratum-files --choose`, with the request as
//! JSON on stdin and the answer as JSON on stdout.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use stratum_ui::app::{AppUi, Key, Keysym};
use stratum_ui::render::display_list::{DisplayList, Rect};
use stratum_ui::widgets::draw as ui;
use stratum_ui::widgets::layout::{self, rect};
use stratum_ui::widgets::text_field::{FieldKey, FieldStyle, TextField};
use stratum_ui::widgets::{InputState, Ui};

use crate::entries::Entry;

/// What an app asked for, as the portal passes it on.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(default)]
pub struct Request {
    pub save: bool,
    pub title: String,
    pub multiple: bool,
    pub directory: bool,
    pub accept_label: Option<String>,
    pub current_folder: Option<PathBuf>,
    pub current_name: Option<String>,
    /// `(name, [(0 for a glob or 1 for a MIME type, pattern)])`.
    pub filters: Vec<(String, Vec<(u32, String)>)>,
    pub current_filter: Option<usize>,
}

/// The answer printed on stdout.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct Answer {
    pub uris: Vec<String>,
    pub current_filter: Option<usize>,
    pub cancelled: bool,
}

pub struct Chooser {
    pub title: String,
    pub save: bool,
    pub multiple: bool,
    pub directory: bool,
    accept_label: String,
    name: TextField,
    filters: Vec<(String, Vec<(u32, String)>)>,
    filter: Option<usize>,
    /// The file to save over exists: the next accept replaces it.
    replace: Option<PathBuf>,
    last_selected: Vec<PathBuf>,
    pub answer: Option<Answer>,
}

impl Chooser {
    pub fn new(request: &Request) -> Self {
        let default_label = match (request.save, request.directory) {
            (true, _) => "Save",
            (false, true) => "Select",
            (false, false) => "Open",
        };
        Self {
            title: if request.title.is_empty() { default_label.to_string() } else { request.title.clone() },
            save: request.save,
            multiple: request.multiple,
            directory: request.directory,
            accept_label: request.accept_label.clone().filter(|l| !l.is_empty()).unwrap_or_else(|| default_label.into()).replace('_', ""),
            name: TextField::with_text(request.current_name.as_deref().unwrap_or("")),
            filters: request.filters.clone(),
            filter: request.current_filter.or(if request.filters.is_empty() { None } else { Some(0) }),
            replace: None,
            last_selected: Vec::new(),
            answer: None,
        }
    }

    pub fn answered(&self) -> bool {
        self.answer.is_some()
    }

    pub fn cancel(&mut self) {
        self.answer = Some(Answer { cancelled: true, ..Answer::default() });
    }

    /// Whether the browser lists `entry`: folders always, files when they
    /// pass the filter (and not at all when choosing a folder).
    pub fn shows(&self, entry: &Entry) -> bool {
        if entry.is_dir {
            return true;
        }
        if self.directory {
            return false;
        }
        let Some((_, rules)) = self.filter.and_then(|i| self.filters.get(i)) else { return true };
        let name = entry.name.to_lowercase();
        rules.iter().any(|(kind, pattern)| match kind {
            0 => crate::mime::glob_match(&pattern.to_lowercase(), &name),
            _ => crate::mime::glob_match(pattern, &entry.mime),
        })
    }

    /// Answer with what is chosen, if it is enough.
    pub fn accept(&mut self, dir: &Path, selected: &[PathBuf]) {
        let uris = |paths: &[PathBuf]| paths.iter().map(|p| stratum_ui::uri::from_path(p)).collect::<Vec<_>>();
        let answer = |uris: Vec<String>, filter| Answer { uris, current_filter: filter, cancelled: false };
        if self.save {
            let name = self.name.text().trim().to_string();
            if name.is_empty() || name.contains('/') {
                return;
            }
            let path = dir.join(&name);
            if path.is_dir() {
                return;
            }
            if path.exists() && self.replace.as_ref() != Some(&path) {
                // Asked once: the button now says Replace.
                self.replace = Some(path);
                return;
            }
            self.answer = Some(answer(uris(&[path]), self.filter));
        } else if self.directory {
            let dirs: Vec<PathBuf> = selected.iter().filter(|p| p.is_dir()).cloned().collect();
            let chosen = if dirs.is_empty() { vec![dir.to_path_buf()] } else { dirs };
            let chosen = if self.multiple { chosen } else { chosen.into_iter().take(1).collect() };
            self.answer = Some(answer(uris(&chosen), None));
        } else {
            let files: Vec<PathBuf> = selected.iter().filter(|p| !p.is_dir()).cloned().collect();
            if files.is_empty() {
                return;
            }
            let files = if self.multiple { files } else { files.into_iter().take(1).collect() };
            self.answer = Some(answer(uris(&files), self.filter));
        }
    }

    /// A key, if the chooser takes it: `Some(true)` to accept, `Some(false)`
    /// for one it handled.
    pub fn key(&mut self, key: &Key) -> Option<bool> {
        match key.keysym {
            Keysym::Escape => {
                self.cancel();
                return Some(false);
            }
            Keysym::Return | Keysym::KP_Enter if self.save => return Some(true),
            _ => {}
        }
        if !self.save || key.ctrl || matches!(key.keysym, Keysym::Up | Keysym::Down | Keysym::Page_Up | Keysym::Page_Down) {
            return None;
        }
        match self.name.key(key.keysym, key.text.as_deref(), false) {
            FieldKey::Changed => {
                self.replace = None;
                Some(false)
            }
            _ => None,
        }
    }

    /// The bar: name field (saving), filter chips, Cancel and accept.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, ui: &mut AppUi, list: &mut DisplayList, bar: Rect, input: &InputState, clicked: bool, dir: &Path, selected: &[PathBuf]) {
        let m = ui.config.metrics.files.clone();
        let page = ui.config.metrics.page.clone();
        let theme = ui.theme.clone();
        list.push_rect(Rect { radius: m.card_radius, color: theme.surface, ..bar });
        list.push_outline(Rect { radius: m.card_radius, color: theme.outline, ..bar }, 1.0);
        // Picking a file while saving takes its name.
        if self.save && selected != self.last_selected.as_slice() {
            if let [one] = selected {
                if !one.is_dir() {
                    if let Some(name) = one.file_name() {
                        self.name = TextField::with_text(&name.to_string_lossy());
                        self.replace = None;
                    }
                }
            }
        }
        self.last_selected = selected.to_vec();

        let y = bar.y + (bar.height - page.badge) * 0.5;
        let label = if self.replace.is_some() { "Replace".to_string() } else { self.accept_label.clone() };
        let (left, hit) = layout::text_buttons(ui, list, bar.x + bar.width - m.padding, y, &["Cancel", &label], true, input, clicked);
        match hit {
            Some(0) => self.cancel(),
            Some(_) => self.accept(dir, selected),
            None => {}
        }
        let mut x = bar.x + m.padding;
        if self.save {
            ui::paint_text_vcentered(ui, list, "Name", x, bar, m.text, 400, theme.text_muted);
            x += ui.measure("Name", m.text) + m.gap;
            let field_w = (left - m.gap * 2.0 - x).min(m.search_width * 1.6).max(0.0);
            let field = rect(x, y, field_w, page.badge);
            ui::paint_frame(list, Rect { radius: field.height * 0.5, ..field }, 1.5, theme.primary, theme.surface_high);
            let style = FieldStyle { text: m.text, padding: m.gap, caret_width: 1.5, masked: false, placeholder: Some("File name") };
            self.name.paint(ui, list, field, style, false);
            x += field_w + m.gap * 2.0;
        }
        if !self.filters.is_empty() && x < left {
            let labels: Vec<String> = self.filters.iter().map(|(n, _)| n.clone()).collect();
            let (_, hit) = layout::chip_flow(ui, list, x, y, (left - m.gap - x).max(0.0), &labels, self.filter, input, clicked);
            if let Some(i) = hit {
                self.filter = Some(i);
            }
        }
    }

    /// Asked before replacing a file: shown over the folder.
    pub fn warning(&self) -> Option<String> {
        let path = self.replace.as_ref()?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        Some(format!("{name} exists: replace it?"))
    }

    /// Whether the filter changed since `before`, so the folder is listed again.
    pub fn filter(&self) -> Option<usize> {
        self.filter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, mime: &str, is_dir: bool) -> Entry {
        Entry { name: name.into(), path: PathBuf::from("/x").join(name), is_dir, is_link: false, size: 0, modified: None, mime: mime.into() }
    }

    #[test]
    fn filters_pick_files_by_glob_or_type() {
        let request = Request {
            filters: vec![("Images".into(), vec![(1, "image/*".into())]), ("Text".into(), vec![(0, "*.TXT".into())])],
            ..Request::default()
        };
        let mut chooser = Chooser::new(&request);
        assert!(chooser.shows(&entry("a.png", "image/png", false)));
        assert!(!chooser.shows(&entry("a.txt", "text/plain", false)));
        assert!(chooser.shows(&entry("dir", "inode/directory", true)));
        chooser.filter = Some(1);
        assert!(chooser.shows(&entry("notes.txt", "text/plain", false)), "globs ignore case");
    }

    #[test]
    fn saving_asks_before_replacing() {
        let dir = std::env::temp_dir().join(format!("stratum-chooser-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("taken.txt"), "").unwrap();
        let mut chooser = Chooser::new(&Request { save: true, current_name: Some("taken.txt".into()), ..Request::default() });
        chooser.accept(&dir, &[]);
        assert!(chooser.answer.is_none(), "asked first");
        chooser.accept(&dir, &[]);
        assert_eq!(chooser.answer.as_ref().unwrap().uris, [stratum_ui::uri::from_path(&dir.join("taken.txt"))]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn choosing_a_folder_takes_the_current_one() {
        let mut chooser = Chooser::new(&Request { directory: true, ..Request::default() });
        chooser.accept(Path::new("/srv"), &[]);
        assert_eq!(chooser.answer.unwrap().uris, ["file:///srv"]);
    }
}
