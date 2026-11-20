# stratum-files: the next round

Written 2026-11-19 at the end of the session that built the file manager
(commits `5d18075`, `79ac476`, pushed). Read `CLAUDE.md` first, especially
"The shared crate", "The file manager" and the testing sections: every
rule there still holds (defaults in `config.default.toml`, sizes in
`[metrics.files]`, live tests in headless sway with a scratch `HOME`, never
in the user's real home or session).

The user's list, in their numbering (there is no 5):

1. Drag and drop
2. Network locations
3. A preview of the selection on the right
4. More thumbnails
6. Trashing files on another drive
7. Typed paths of any length
8. Undo
9. Use as a file and folder picker

**Progress (2026-11-19):** 7 `81ab92c`, 6 `238be8d`, 8 `5fcdde1`, 3 `176e1bd`,
4 `2874642`, 1 `4a751a5` (in the window) and `f523b36` (between apps):
all built and committed, not pushed. Left: 9's check in the real session
(below), which needs the user's rebuild, and 2 (put off).

Suggested order, smallest and most self-contained first, so each lands as
its own commit with tests: **7, 6, 8, 3, 4, 1**, then the check for 9.
**2 is put off** (the user's call, 2026-11-19). Run
`cargo test --workspace` and `cargo clippy --workspace --all-targets` in
`nix develop` before each commit, and give each commit message the binary
size change (`cargo build --release -p stratum-files`, then `stat`).

---

## 7. Typed paths of any length

**Problem.** `TextField` stores its text in `stratum_ui::secret::Secret`,
which reserves 256 bytes and refuses to grow (so a password never
reallocates and leaves copies behind). File names fit (255 bytes max);
typed paths (Ctrl+L) and search text can exceed it.

**Plan.**
- `Secret::with_capacity(n)`; `Secret::new()` stays 256.
- `TextField::with_capacity(n)` / `TextField::plain_with_text(text, n)`.
  The path field uses `libc::PATH_MAX` (4096); the search field 4096.
  Rename fields keep 256 (NAME_MAX + 1).
- Test: typing 1000 characters into a 4096 field keeps them all; the
  256-byte field still refuses past its capacity (the lock screen relies
  on this).

**Cost.** Nothing measurable.

---

## 6. Trashing files on another drive

**Problem.** `trash::trash` renames into `$XDG_DATA_HOME/Trash`, which
fails with `EXDEV` across file systems; today the user is told to
Shift+Delete instead.

**Plan (freedesktop trash spec, "top directories").**
- Find the file's mount point: walk up from the file while `st_dev` stays
  the same (`std::os::unix::fs::MetadataExt::dev`).
- Use `$topdir/.Trash/$uid` if `$topdir/.Trash` exists, is a directory, is
  not a symlink and has the sticky bit; otherwise `$topdir/.Trash-$uid`
  (create it 0700). If neither can be made (read-only drive), fall back to
  today's message.
- In top-directory trashes the `.trashinfo` `Path=` is **relative to
  `$topdir`**; `restore` joins it back.
- The Trash view becomes a merged list: the home trash plus the
  top-directory trash of every mounted drive (`places::parse_mounts` gives
  them; also check `/` and other real mounts from `/proc/self/mounts`
  that hold a `.Trash-$uid`). `entries::Entry::path` stays the real path
  inside each trash's `files/`, so restore and delete work per item; make
  `trash::is_trash` recognise any trash `files/` directory, and give the
  view a pseudo-location (`Location::Trash`) rather than one real folder.
  Show the original folder as a column or caption in the Trash view.
- "Empty the trash" empties all of them.
- Tests (pure): mount-point detection with a fake `dev` lookup, top-dir
  info paths relative and back, choice between `.Trash/$uid` and
  `.Trash-$uid`. Live: a loop-mounted or tmpfs scratch file system in the
  sandbox (`unshare -rm` lets an unprivileged test mount a tmpfs), never
  the user's drives.

---

## 8. Undo

**Plan.** An undo stack in `Browser` (Ctrl+Z; Ctrl+Shift+Z redo is
optional). Each entry records what reverses it:

| Operation | Undo |
|---|---|
| Rename `a → b` | rename `b → a` (if `a` is free) |
| New folder | remove it if still empty |
| Move (cut/paste, drag) | move each target back to its source folder |
| Copy (paste, drag with Ctrl) | trash the copies made |
| Trash | `trash::restore` each, by its trash name |
| Delete for good | not undoable; say so in the confirm dialog (it already says "cannot be undone") |

- `ops::start` must report what it created: add `made: Vec<(PathBuf
  source, PathBuf target)>` to `ops::Progress`, filled as each top-level
  item finishes (a cancelled or failed item is not recorded).
- `trash::trash` returns the trash name it used.
- Undo runs through the same machinery (a move back is an `ops::start`
  move), and reports in the status bar ("Undid: renamed note2.md").
- Keep the last 50 entries; the stack is per window and not persisted.
- Tests: each reversal on a scratch directory (rename, move, copy, trash,
  new folder), and that a conflicting target is refused rather than
  overwritten.

---

## 3. A preview of the selection on the right

**Plan.** A collapsible pane at the right of the folder view (F9 and a
toolbar button; `[files] preview = true` for new windows; its width
`[metrics.files] preview_width`).
- One selected item:
  - a picture: the thumbnail at the pane's width, decoded on the
    thumbnail thread at that size; drawn with `DisplayList::push_texture`
    when it is larger than the atlas allows (as the launcher's clipboard
    preview does, `launcher/mod.rs::paint_preview_image`);
  - text (by MIME ancestry, `text/plain`): the first 16 KB, lines kept, as
    the launcher's clipboard preview (`paint_preview_text`; consider
    moving that to `stratum_ui::widgets` so both use it);
  - a folder: its item count and size (size computed on a thread, capped);
  - anything else: the big type icon.
- Under it, information: name, type (the MIME comment from
  shared-mime-info's `mime/packages/*.xml` would be nicer than the raw
  type; a small `<comment>` parser is enough), size, modified, permissions
  (`rwxr-xr-x`), owner, the link target for a symlink, the image's pixel
  size.
- Several selected: a count and total size.
- The pane is also shown in the chooser, where it helps most.
- Tests: the info lines for a file, folder, link (pure, from metadata);
  permissions formatting.

**Cost.** One decoded picture at pane size (about 0.5 MB at 320×400 @2x),
dropped when the selection changes.

---

## 4. More thumbnails

Today: PNG and JPEG, decoded on a thread, held in memory per folder.

**Plan.**
- **The freedesktop thumbnail cache**
  (`$XDG_CACHE_HOME/thumbnails/{normal,large}/<md5 of the URI>.png`, with
  `Thumb::URI` and `Thumb::MTime` text chunks): read it first, so what
  other apps made is reused; write what we make, so it is not made twice.
  Needs MD5 (write a small one with tests, or check whether a crate
  already in `Cargo.lock` provides it) and PNG text chunks (the `png`
  crate, already a dependency of `image`, writes and reads them).
  Skip the cache for files in the cache itself and under `/tmp`.
- **More formats in-process**: `image` features `webp`, `gif`, `bmp`
  (check each one's size cost first: CLAUDE.md asks for that before any
  dependency or feature); SVG through `resvg`, already in `stratum_ui`
  (`icons.rs` rasterises SVG icons; reuse it).
- **Other formats through the thumbnailers installed on the system**:
  `share/thumbnailers/*.thumbnailer` files (the freedesktop thumbnailer
  spec: `TryExec`, `Exec` with `%i %o %s %u`, `MimeType`). That covers
  video (ffmpegthumbnailer), PDF (evince/papers, poppler), fonts, and
  anything else the user installs, with no new dependency. Run them on the
  thumbnail thread with a timeout. None are installed now
  (`ffmpegthumbnailer`, `pdftoppm` are absent), so: a Home Manager option
  `fileManager.thumbnailers` (a list of packages, e.g.
  `ffmpegthumbnailer`, `poppler-utils`) or document adding them.
- Keep `thumbnail_max_mb`; add `[files] thumbnail_external = true`.
- Tests: MD5 of a known URI; the cache file name; parsing a
  `.thumbnailer` file and expanding its `Exec`; a fake thumbnailer script
  in the sandbox.

---

## 9. Use as a file and folder picker: built; check it for real

**Settled with the user (2026-11-19):** they meant the D-Bus dialog
handler, the desktop portal's FileChooser that the screenshot viewer's
Save As calls (`portal_save_as`, over zbus). That is `portal.rs`, already
built and tested on a private bus (SaveFile, OpenFile, Request.Close).

What is left:
- Check it in the real session once the user has rebuilt with
  `programs.stratum.fileManager.enable = true`: Save As in the screenshot
  viewer, an Open dialog in a GTK app and in Firefox/Zen (which use the
  portal when `widget.use-xdg-desktop-portal.file-picker = 1`), and a
  folder pick (OpenFile with `directory`). `busctl --user tree
  org.freedesktop.impl.portal.desktop.stratum` shows whether it is
  running; `journalctl --user -u xdg-desktop-portal` shows which backend
  answered.
- Hyprland may tile the chooser window; a window rule to float it
  (`class org.stratum.Files`, title not "Files") belongs in the user's
  config, so suggest it rather than change `~/nixos-config`.
- Optional, not asked for: pass the requesting app's name into the title
  ("Save: Screenshot Viewer").

---

## 1. Drag and drop

Two parts; do the in-window part first.

**In the window.**
- Press on an entry and move past a few pixels: a drag starts, carrying
  the selection (or the pressed entry, if unselected). Draw a small
  floating card under the pointer (icon and count).
- Drop targets: a folder row or cell, a sidebar place, the breadcrumbs
  (a parent folder). Highlight the target under the pointer.
- Move by default within one file system, copy across file systems or with
  Ctrl held (as other file managers do); Shift forces a move. Runs through
  `ops::start` and records undo (item 8).
- Escape cancels.

**Between apps** (dragging a file into a browser upload, a terminal, or
out of another app into a folder here) needs `wl_data_device` in the
runner (`stratum_ui::app`): SCTK 0.21 has `data_device_manager`
(`DataDeviceManagerState`, `DragSource`, `DataOffer`). Add to `App`:
- `fn drag_data(&mut self) -> Option<Vec<PathBuf>>` and a runner call
  `start_drag(serial)` when the app's in-window drag leaves the window
  (or always start a real Wayland drag and handle our own window as one
  more target; simpler and how GTK does it). Offer `text/uri-list` (and
  `x-special/gnome-copied-files` if a test app needs it).
- Incoming: `enter/motion/leave/drop` from the data device become
  `App::drop_motion(point, mime_types)` and `App::drop(point, paths,
  action)`; read the offer's `text/uri-list` through a pipe on a thread.
- The drag needs the serial of the pointer press that started it; the
  runner already sees press events, so keep the last one.
- Test between two `stratum-files` windows in headless sway, and with
  `wl-paste`'s companion tools if a DnD test client is needed (none is
  installed; a small SCTK test client in the scratchpad, like the virtual
  pointer tool, can act as the other app). The virtual pointer tool
  (`scratchpad/vptr`) needs a `press X Y`, `move X Y` and `release` split
  to drag in steps.

**Cost.** A drag icon surface while dragging (a small swapchain,
~1 MB), nothing at rest.

---

## 2. Network locations (put off)

**Put off by the user on 2026-11-19.** Kept here for when it comes back.

**Finding.** No gvfs on this machine (`/run/user/1000/gvfs` does not
exist; no `gvfsd`), so `gio mount` cannot reach SMB or SFTP; `sshfs` is
not installed either.

**Options; ask the user which before building.**
- **gvfs** (recommended if they are fine with it): NixOS
  `services.gvfs.enable = true` (Thunar uses it too, so it may already be
  wanted). Then "Connect to server" (`smb://host/share`,
  `sftp://user@host/path`, `dav(s)://`, `ftp://`) runs `gio mount <uri>`
  (it prompts for credentials on stdin: answer from a password field,
  never a command line), and the location is browsed at its FUSE path,
  `/run/user/$UID/gvfs/<mount>`. Mounted ones appear under Drives;
  `gio mount -u` unmounts. Network browsing (`network://`, Avahi) can come
  later. Cost: gvfs daemons run only while something is mounted, plus
  their closure.
- **sshfs / rclone, mounted by us**: no GNOME pieces; SFTP via `sshfs`,
  SMB via `mount.cifs` (needs root) or `rclone mount` (FUSE, many
  protocols). The file manager keeps a list of saved servers in
  `$XDG_STATE_HOME/stratum/servers.json` (no passwords: keys or the
  user's agent for SFTP; rclone's own encrypted config for the rest).
- Either way: a "Network" section in the sidebar with saved servers,
  "Connect to server…" in it, unmount buttons, and errors in the status
  bar. Test against a local SFTP server in the sandbox (`sshd` on a high
  port with a scratch host key, or `rclone serve sftp` on a scratch
  directory), never the user's servers.

---

## Things to keep in mind

- The file manager is its own process and binary; nothing here touches
  the shell, except moving shared pieces into `stratum_ui` (the text
  preview, maybe a picture-fitting helper).
- New options: a struct field in `stratum_ui/src/config.rs` or
  `style.rs` **and** a line in `tools/stratum_shell/config.default.toml`,
  or every test fails.
- New files must be `git add`ed before `nix build`: the flake sees only
  tracked files (this bit us twice).
- `cargo build -p stratum-files` before a live test; the sandbox scripts
  are in the old session's scratchpad and will be gone: rebuild them from
  CLAUDE.md's testing notes (headless sway, the virtual pointer client,
  `wtype -s 300 -k Shift_L ...`, a scratch `HOME` with `XDG_DATA_HOME`,
  `XDG_CONFIG_HOME`, `XDG_STATE_HOME`, `XDG_CACHE_HOME`).
- Nothing in `~/nixos-config` was changed for the file manager; enabling
  it is `programs.stratum.fileManager.enable = true;` after
  `nix flake update stratum`, which is the user's call.
