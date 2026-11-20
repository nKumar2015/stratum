//! Undo: what each change did, and how to reverse it. Each window keeps
//! its own history (not saved). Deleting for good is never recorded: it
//! cannot be reversed, and its confirmation says so.
//!
//! A reversal never overwrites: a name that has been taken again since is
//! reported, and that item is left as it is.

use std::path::{Path, PathBuf};

/// How many changes are kept.
pub const DEPTH: usize = 50;

/// One change, as what reverses it needs.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Rename { from: PathBuf, to: PathBuf },
    NewFolder(PathBuf),
    /// Each item's place before and after.
    Move(Vec<(PathBuf, PathBuf)>),
    /// Each item copied, and its copy.
    Copy(Vec<(PathBuf, PathBuf)>),
    /// Each item's place before, and where it is in a trash.
    Trash(Vec<(PathBuf, PathBuf)>),
}

impl Step {
    /// What was done, for "Undid: …".
    pub fn describe(&self) -> String {
        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let items = |n: usize, one: &Path| if n == 1 { name(one) } else { crate::browser::count(n, "item") };
        match self {
            Step::Rename { from, to } => format!("renamed {} to {}", name(from), name(to)),
            Step::NewFolder(path) => format!("made {}", name(path)),
            Step::Move(pairs) => format!("moved {}", items(pairs.len(), &pairs[0].0)),
            Step::Copy(pairs) => format!("copied {}", items(pairs.len(), &pairs[0].0)),
            Step::Trash(pairs) => format!("moved {} to the trash", items(pairs.len(), &pairs[0].0)),
        }
    }
}

/// The steps that can be undone, newest last.
#[derive(Default)]
pub struct History {
    steps: Vec<Step>,
}

impl History {
    /// Record a change. Empty ones (nothing was moved) are not.
    pub fn push(&mut self, step: Step) {
        let empty = match &step {
            Step::Move(pairs) | Step::Copy(pairs) | Step::Trash(pairs) => pairs.is_empty(),
            Step::Rename { .. } | Step::NewFolder(_) => false,
        };
        if empty {
            return;
        }
        if self.steps.len() == DEPTH {
            self.steps.remove(0);
        }
        self.steps.push(step);
    }

    pub fn pop(&mut self) -> Option<Step> {
        self.steps.pop()
    }

    pub fn last(&self) -> Option<&Step> {
        self.steps.last()
    }
}

/// How undoing a step went.
#[derive(Debug, Default, PartialEq)]
pub struct Undone {
    /// Items to move back with a job (a move between drives copies).
    pub move_back: Vec<(PathBuf, PathBuf)>,
    /// What could not be reversed, item by item.
    pub errors: Vec<String>,
}

fn taken(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

fn name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Reverse `step`. A move comes back as `move_back` for the caller to
/// run as a job; the rest is done here.
pub fn undo(step: Step) -> Undone {
    let mut out = Undone::default();
    match step {
        Step::Rename { from, to } => {
            if taken(&from) {
                out.errors.push(format!("{} exists again", name(&from)));
            } else if let Err(err) = std::fs::rename(&to, &from) {
                out.errors.push(format!("{}: {err}", name(&to)));
            }
        }
        Step::NewFolder(path) => {
            // Only while still empty: anything put in it since is kept.
            if let Err(err) = std::fs::remove_dir(&path) {
                let why = if err.kind() == std::io::ErrorKind::DirectoryNotEmpty { "it is no longer empty".to_string() } else { err.to_string() };
                out.errors.push(format!("{}: {why}", name(&path)));
            }
        }
        Step::Move(pairs) => {
            for (before, after) in pairs {
                if !taken(&after) {
                    out.errors.push(format!("{} is gone", name(&after)));
                } else if taken(&before) {
                    out.errors.push(format!("{} exists again", before.display()));
                } else {
                    out.move_back.push((after, before));
                }
            }
        }
        Step::Copy(pairs) => {
            // To the trash, not deleted: a copy may have been changed
            // since, and the trash keeps it.
            for (_, copy) in pairs {
                if let Err(err) = crate::trash::trash(&copy) {
                    out.errors.push(format!("{}: {err}", name(&copy)));
                }
            }
        }
        Step::Trash(pairs) => {
            for (_, item) in pairs {
                if let Err(err) = crate::trash::restore(&item) {
                    out.errors.push(format!("{}: {err}", name(&item)));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stratum-undo-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_history_is_bounded_and_skips_empty_steps() {
        let mut history = History::default();
        history.push(Step::Move(Vec::new()));
        assert!(history.last().is_none());
        for i in 0..DEPTH + 5 {
            history.push(Step::NewFolder(PathBuf::from(format!("/x/{i}"))));
        }
        assert_eq!(history.steps.len(), DEPTH);
        assert_eq!(history.pop(), Some(Step::NewFolder(PathBuf::from(format!("/x/{}", DEPTH + 4)))));
        assert_eq!(history.steps[0], Step::NewFolder("/x/5".into()));
    }

    #[test]
    fn a_rename_is_reversed_unless_the_name_is_taken_again() {
        let dir = scratch("rename");
        std::fs::write(dir.join("b"), "1").unwrap();
        let step = Step::Rename { from: dir.join("a"), to: dir.join("b") };
        assert_eq!(step.describe(), "renamed a to b");
        assert!(undo(step.clone()).errors.is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("a")).unwrap(), "1");
        // Renamed again, and something new now called `a`.
        std::fs::rename(dir.join("a"), dir.join("b")).unwrap();
        std::fs::write(dir.join("a"), "2").unwrap();
        assert_eq!(undo(step).errors.len(), 1);
        assert_eq!(std::fs::read_to_string(dir.join("a")).unwrap(), "2");
        assert_eq!(std::fs::read_to_string(dir.join("b")).unwrap(), "1");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_new_folder_goes_only_while_empty() {
        let dir = scratch("folder");
        std::fs::create_dir(dir.join("n")).unwrap();
        std::fs::write(dir.join("n/kept"), "").unwrap();
        assert_eq!(undo(Step::NewFolder(dir.join("n"))).errors, ["n: it is no longer empty"]);
        std::fs::remove_file(dir.join("n/kept")).unwrap();
        assert!(undo(Step::NewFolder(dir.join("n"))).errors.is_empty());
        assert!(!dir.join("n").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_move_comes_back_as_a_job_for_what_can_go_back() {
        let dir = scratch("move");
        std::fs::create_dir(dir.join("to")).unwrap();
        for f in ["a", "b", "c"] {
            std::fs::write(dir.join("to").join(f), "").unwrap();
        }
        // `b`'s old place is taken again; `gone` no longer exists.
        std::fs::write(dir.join("b"), "").unwrap();
        let pairs = ["a", "b", "gone"].map(|f| (dir.join(f), dir.join("to").join(f))).to_vec();
        let done = undo(Step::Move(pairs));
        assert_eq!(done.move_back, [(dir.join("to/a"), dir.join("a"))]);
        assert_eq!(done.errors.len(), 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn copies_go_to_the_trash_and_trashed_files_come_back() {
        // Tests get a scratch home trash (`Trash::home`).
        let dir = scratch("trash");
        std::fs::write(dir.join("copy.txt"), "c").unwrap();
        let step = Step::Copy(vec![(dir.join("orig.txt"), dir.join("copy.txt"))]);
        assert_eq!(step.describe(), "copied orig.txt");
        assert!(undo(step).errors.is_empty());
        assert!(!dir.join("copy.txt").exists());
        let trashed = crate::trash::Trash::home().files().join("copy.txt");
        assert!(trashed.exists());
        crate::trash::delete(&trashed).unwrap();

        std::fs::write(dir.join("t.txt"), "t").unwrap();
        let item = crate::trash::trash(&dir.join("t.txt")).unwrap();
        assert!(undo(Step::Trash(vec![(dir.join("t.txt"), item)])).errors.is_empty());
        assert_eq!(std::fs::read_to_string(dir.join("t.txt")).unwrap(), "t");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
