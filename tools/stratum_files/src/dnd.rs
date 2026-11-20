//! Dragging items within the window: when a press becomes a drag, where
//! the items may be dropped, and whether a drop moves or copies them.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropAction {
    Move,
    Copy,
}

/// Whether the pointer has gone far enough from where it was pressed for
/// a press to be a drag rather than a click.
pub fn started(from: (f32, f32), to: (f32, f32), threshold: f32) -> bool {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    dx * dx + dy * dy > threshold * threshold
}

/// Whether `dest` can take `sources`: not one of them or inside one (a
/// folder into itself), and not the folder they are all already in.
pub fn valid_target(sources: &[PathBuf], dest: &Path) -> bool {
    !sources.is_empty()
        && !sources.iter().any(|s| dest.starts_with(s))
        && !sources.iter().all(|s| s.parent() == Some(dest))
}

/// What a drop does, as other file managers do it: Ctrl copies, Shift
/// moves, and otherwise items move within a file system and are copied
/// to another. `dev` gives a path's device.
pub fn action(sources: &[PathBuf], dest: &Path, ctrl: bool, shift: bool, dev: impl Fn(&Path) -> Option<u64>) -> DropAction {
    if ctrl {
        return DropAction::Copy;
    }
    if shift {
        return DropAction::Move;
    }
    let target = dev(dest);
    if target.is_some() && sources.iter().all(|s| s.parent().and_then(&dev) == target) {
        DropAction::Move
    } else {
        DropAction::Copy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_starts_past_the_threshold() {
        assert!(!started((10.0, 10.0), (13.0, 14.0), 6.0));
        assert!(started((10.0, 10.0), (17.0, 10.0), 6.0));
    }

    #[test]
    fn targets_exclude_the_items_and_their_folder() {
        let sources = vec![PathBuf::from("/h/docs/a"), PathBuf::from("/h/docs/b")];
        assert!(valid_target(&sources, Path::new("/h/pics")));
        assert!(valid_target(&sources, Path::new("/h")));
        assert!(!valid_target(&sources, Path::new("/h/docs")), "where they already are");
        assert!(!valid_target(&sources, Path::new("/h/docs/a")), "one of them");
        assert!(!valid_target(&sources, Path::new("/h/docs/a/inner")), "inside one of them");
        assert!(!valid_target(&[], Path::new("/h")));
    }

    #[test]
    fn same_drive_moves_another_copies_and_keys_choose() {
        let dev = |p: &Path| Some(if p.starts_with("/media/usb") { 2 } else { 1 });
        let home = vec![PathBuf::from("/h/a")];
        assert_eq!(action(&home, Path::new("/h/pics"), false, false, dev), DropAction::Move);
        assert_eq!(action(&home, Path::new("/media/usb"), false, false, dev), DropAction::Copy);
        assert_eq!(action(&home, Path::new("/h/pics"), true, false, dev), DropAction::Copy);
        assert_eq!(action(&home, Path::new("/media/usb"), false, true, dev), DropAction::Move);
        // Unknown device: copy, which never loses the original.
        assert_eq!(action(&home, Path::new("/gone"), false, false, |_| None), DropAction::Copy);
    }
}
