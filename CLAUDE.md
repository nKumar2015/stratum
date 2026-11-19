# CLAUDE.md

Guidance for working in this repository.

## What this project is

Stratum is a NixOS desktop environment built around **Hyprland**. Historically its
shell was written in QML and run on **Quickshell**, served by a daemon,
`stratumd`.

Quickshell has been replaced by a **native Rust shell** in
`tools/stratum_shell/`. This is not a fork or a port — it is a from-scratch
implementation with its own renderer, layout, and theming. The QML tree and
`stratumd` were deleted on 2026-10-23; code comments that cite a `.qml` file
or "QML did" refer to the git history before that removal.

### The user's requirements (verbatim intent, do not reinterpret)

- **Full replacement of QML.** No QML, and no existing UI/design toolkit
  (no GTK, no Qt, no slint, no iced). Every visual is ours.
- **The motivation is footprint.** Binary size, memory, CPU, and startup time must
  beat Quickshell. Measured baselines: Quickshell's closure is ~1.1 GiB,
  qtdeclarative alone ~692 MiB. Our stripped release binary was ~7.1 MB early
  on, grew to ~16 MB as the system managers were embedded, and was ~13.1 MB
  on 2026-10-22; after the dashboard overhaul and a cleanup pass it is
  14.26 MB (2026-10-23; chrono is ~71 KB of that, jiff ~16 KB, and naga,
  zbus and wgpu remain the largest pieces), and 14.58 MB once `image` gained
JPEG decoding for album art (~320 KB). The CLI is 0.41 MB.
  Before adding a dependency, check its default features:
  resvg's defaults alone cost 1.9 MB. To find what is big, relink with
  `CARGO_PROFILE_RELEASE_STRIP=false` and sum `nm -S -C` sizes by crate.
- **Feature parity, not pixel parity.** The QML shell's behaviour and data must be
  preserved; the visual style is deliberately changing to "vectorheart".
- **Long-term goal:** eventually as configurable as Quickshell — arbitrary
  placement, animation, styling — without the footprint.
- **Hard switch at the end.** Quickshell stops being used. `stratumd` may be
  removed or absorbed as long as its features survive.
- **The switch happened on 2026-10-22.** The native shell is installed from
  this flake's Home Manager module and runs as the systemd user unit
  `stratum-shell`; Quickshell and stratumd are no longer installed or started.

### Naming and layout

```
tools/stratum_sys/     library: system managers (audio, wifi, bluetooth,
                       battery, music, sysmon, gpu, weather, calendar,
                       polkit), host-agnostic; queries return JSON
                       snapshots, and the audio monitor and polkit agent
                       publish theirs through `Publisher`
tools/stratum_cli/     CLI, a socket client only; its IPC targets are the
                       contract external keybinds use
tools/stratum_shell/   the native shell
module.nix             Home Manager module: shell, CLI, runtime tools, user unit
```

## Current state

Implemented and working: sidebar (workspaces, app title, tray, quick panel,
status icons, battery, clock), control centre as an xdg-shell window with ten
pages (overview, audio, equalizer, bluetooth, wifi, battery, notifications,
display (monitor layout), theme, power),
volume/brightness OSD, idle (dim, lock, screens off, suspend, and a lock
before every sleep), notification toasts, notification server, tray watcher,
live theme switching, config hot reload, keyboard input, the power menu overlay,
the IPC socket, the lock screen, and the polkit dialog (both verified only
against fixtures in a nested compositor; see below).

The **theme switcher** (`theme_switcher/`) is a band of slanted slices,
one per theme, across the output it opens on, with the desktop visible
behind. A slice shows the theme's linked wallpaper over columns of its
palette; the focused one opens wide and the band slides to centre it
(`layout.rs`, pure and tested). Moving the focus previews the theme on the
whole shell through `preview_theme`, and its wallpaper (or, for a theme
without one, the wallpaper from before) once the focus has rested
`[theme_switcher] wallpaper_delay_ms`. Return keeps it through
`switch_theme`; Escape after clearing the search, a click outside the band,
or `theme close|toggle` reverts. Hover only highlights: the band slides
under the pointer, so a focus that followed it would chase the mouse. Its
tick runs before the frame resolves its theme, since it may preview one.
The wallpapers are decoded large (`preview_px`) on their own thread only
while it is open, published in `SystemState::wallpaper_previews`, and
dropped when it closes.

Settings pages are built on the scaffold in `ui/pages/layout.rs` (header,
sections, cards, rows, badges, pills) and scroll as a whole. The Bluetooth
and Network pages keep the QML menus' behaviour but not their layout: the
pieces in `ui/pages/devices.rs` give them a header with refresh/scan, a power
card whose caption is the `Status` line, and rows that expand in place where
QML had a side panel. A page learns how its own action
ended by sending `SystemAction::Tracked { ticket, .. }`; the worker publishes
the outcome under that ticket in `SystemState::action_result`. Queue actions
from `paint`, not from `on_page_shown`: an action queued there can be cleared
by the frame start.

**Screenshots** (`screenshot.rs`, `screenshot_viewer.rs`,
`wayland/viewer_window.rs`). `screenshot start` freezes every output with
`grim -t ppm -o`, shown as each overlay's `DisplayList::backdrop` (a texture of
its own, far too big for the glyph atlas). Window picking reads
`HyprState::client_rects`, and global coordinates come from the `wl_output`
logical position, so picking works without Hyprland except for windows. A
capture runs `grim` after the overlay is hidden, and the PNG is decoded on that
worker. The viewer is a second xdg toplevel. Its strokes are stored in image
pixels and rasterised on the CPU (`screenshot_viewer::rasterize`) for
annotated exports. Copy runs `wl-copy`; Save and Save to copy the file in
Rust (`screenshot_viewer::post`); Save As asks the desktop portal's
FileChooser over zbus (`portal_save_as`). Every surface sets a cursor shape on
enter: once one surface sets the crosshair, compositors keep it across
surfaces. Live tests must set `HOME` to a scratch directory, because Save
writes to `~/Pictures/Screenshots`.

The **dashboard** (`dashboard/`) is a fullscreen overlay of tabs, each rows
of panels, from `[[dashboard.tabs]]`; it always opens on the first, or on the
tab `dashboard open|toggle <tab>` names. Its data comes from a sampler thread
(`dashboard/feed.rs`, over `stratum_sys::managers::sysmon`) that keeps a
history for the graphs, recording in the background but publishing into
`SystemState::dashboard` only while open, so a closed dashboard never wakes
the loop. GPUs are sampled only while open: an NVML reading costs ~20 ms
(held open, NVML would keep the dGPU awake). NVML runs in a helper process,
the shell binary started as `--nvml-read <slot>` (`gpu::nvml_helper`): it
loads `libcuda`, which can never be unloaded and kept ~4 MB and a thread in
the shell for the rest of the session. The System tab's graphs are
`ui/graph.rs`, which spans a fixed number of readings so the line scrolls
rather than stretches; they fill their panel in a grid. The whole card,
text included, scales up from `[metrics.dashboard]` to span
`[dashboard] screen_fraction` of the output (`DashboardMetrics::scaled`),
never down. (A process list was built and dropped at the user's request.) Weather
(`dashboard/weather.rs`, `stratum_sys::managers::weather`) comes from
Open-Meteo through `curl`, only while open and at most every `refresh_min`;
the postal code's coordinates and the last forecast are cached in
`$XDG_STATE_HOME/stratum/weather.json`, so live tests set `XDG_STATE_HOME`.
Calendar sync (`dashboard/calendar.rs`, `dashboard/agenda.rs`,
`stratum_sys::managers::calendar`) reads ICS addresses from
`[dashboard.calendar] urls_file` (the user supplies it as a sops secret:
the addresses must never reach the Nix store, the logs, or an error
message), fetches with `curl` only while open, and keeps copies in
`$XDG_STATE_HOME/stratum/calendar/` (mode 700). Time zones come from `jiff`
over the system zoneinfo; series repeat in their own zone's wall time. Test
with fixture feeds as `file://` addresses, never the user's calendar.
Clicking an agenda or day row shows the event's details over the panels
(`dashboard/event.rs`): location, calendar name, organizer, attendees, `URL`
and the description, which may be plain text or HTML (Google's `<a>` links,
Outlook's `X-ALT-DESC` document). Only `http`, `https` and `mailto` links
are clickable, and a click runs `xdg-open` and closes the dashboard; live
tests put a stub `xdg-open` first in `PATH`. `SystemAction::Spawn` waits for
its child on a thread, or each finished one stays a zombie. The sampler also refreshes
`SystemState::music` while open. Views draw album art from the snapshot's
`art_path`, never `art_url`: `music::status_for` percent-decodes `file://`
addresses and downloads `http(s)` ones (web players such as Orchard send
those) with `curl` into `$XDG_CACHE_HOME/stratum/art/`, keeping only the
current cover and not retrying a failed address. The calendar grid is computed locally. Media
controls run `playerctl` on the action worker. The Focus tab
(`dashboard/focus.rs`, `note.rs`, `timer.rs`) keeps to-dos and a note in
`$XDG_STATE_HOME/stratum/focus.json`, loaded at start rather than in
`Dashboard::new` so tests never touch the real file. Its timer lives in
memory and runs while the dashboard is closed: the loop's timeout follows
`Dashboard::timer_wake`, and its end is posted over D-Bus
(`SystemAction::Notify`) so DND and the toasts treat it like any other
notification. `stratum-cli todo add …` and `timer start|pause|toggle|stop`
drive it. Keys go to a focused field first; Tab moves between fields and
Ctrl+Tab still switches tabs. Other keys act only where their panel shows,
and `Dashboard::key_hint` lists them in the tab bar: keep it in step when
adding one.

The **launcher** (`launcher/`, `wayland/state/launcher.rs`,
`stratum-cli launcher open|close|toggle`) searches applications (desktop
entries, `launcher/apps.rs`), Hyprland's windows (`j/clients`, focused with
`hl.dsp.focus({ window = "address:0x…" })`), the shell's own commands (IPC
calls: lock, power menu, settings pages, themes, stay awake…) and a
calculator (`launcher/calc.rs`; Return copies the result). Ranking is
`launcher/fuzzy.rs` plus decaying launch counts in
`$XDG_STATE_HOME/stratum/launcher.json`, so live tests set
`XDG_STATE_HOME`. It holds nothing while closed: entries and windows are
read on open, and its icons are evicted from the icon cache on close
(`IconTheme::forget`). Its surface is only as big as its box, and clicks
outside land on a transparent catcher, a single-pixel layer on the `Top`
layer under it. Closing unmaps the box *before* destroying the catcher;
the other order made sway kill the client (the configure race, under
"Overlays on wlroots"). Apps start under `[launcher] launch_prefix`, by
default `systemd-run --user --scope … setsid --fork`: a child of the shell
would be in its unit's cgroup and die with every `systemctl restart`.

It has two more modes (`launcher open|toggle emoji|clipboard`, or the
Emoji and Clipboard history commands), laid out like vicinae's in a wider
box (`[metrics.launcher] wide_width`, set each time it opens): emoji in a
grid of cells under their group's header (`launcher/grid.rs`, pure and
tested: rows, two-dimensional arrows, scrolling that keeps the selection
and its header in view; one headerless ranked grid while searching), and
the clipboard as a list beside a preview pane (the entry's text, read 16 KB
at most, lines kept as written and only long ones wrapped with their
indent; or the image, decoded at the pane's size on a thread and drawn as
a texture of its own) over an information section. **Emoji** come from a table compiled
in (`launcher/emoji.txt`, ~40 KB, generated from Unicode's
`emoji-test.txt` by `launcher/gen_emoji.py`). Text runs keep only a glyph's
coverage, so each visible emoji is rasterised in colour by
`TextEngine::rasterize_color` and drawn as an image, in `[launcher]
emoji_font` asked for by name: under the theme's font, a monochrome font
shaped the common ones first. Only the rows on screen keep their images.
The **clipboard** mode shows the shell's own history (`clipboard/`,
`wayland/state/clipboard.rs`). The shell watches the selection through
`ext-data-control-v1`; for each copy `clipboard::choose` picks a type (an
image over text; a copy offering `x-kde-passwordManagerHint` is never
kept), the owner writes it into a pipe, and one reader thread records the
copies in order, giving up after 5 s without data or past `[clipboard]
max_entry_mb`. `clipboard::store` keeps one file per entry, named by id,
and an index newest first in `$XDG_STATE_HOME/stratum/clipboard/`
(directory 0700, files 0600); a repeated copy moves up rather than being
stored twice. Only the index is in memory. Return restores an entry with
`wl-copy --type <mime> < <file>`, which the watcher then sees as a new copy
and moves to the top; Delete removes one; `stratum-cli clipboard clear`
forgets them all. Files are named by id, so thumbnails guess the format
from the content. (cliphist was never installed: the session's `wl-paste
--watch cliphist store` failed on every copy.) Live tests copy with the
real `wl-copy` inside the sandbox and set `XDG_STATE_HOME`. Closing trims the heap (`memory::release_freed`); the first emoji
use leaves ~4 MB of font caches, which do not grow after.

**Key repeat** comes from SCTK's calloop timer at the compositor's rate
(`get_keyboard_with_repeat`, and `repeat_key` for compositors that repeat
themselves), and reaches text fields only (`key_repeat.rs`): the lock and
polkit fields, a toast reply, a focused Focus-tab field and the settings
window's fields get every key but Return, Escape and Tab; the theme
switcher's search gets typing and deleting only, since its arrows move the
band; menus and overlays get none, so a held arrow moves once. When testing
it live, rebuild `target/debug/stratum-shell` with `cargo build`: `cargo
test` and `clippy` leave a stale binary.

**Idle** (`idle.rs` policy, pure and tested; `wayland/state/idle.rs`
plumbing; `logind.rs`). Each `[idle]` stage (dim, lock, screens off,
suspend) is its own `ext-idle-notify-v1` notification, so a client holding
an idle inhibitor (a video) holds them all off, and so does the stay-awake
icon (`idle_inhibit` widget, `stratum-cli idle inhibit|uninhibit|toggle`).
The dim is an overlay layer per output holding a single-pixel buffer that
`wp_viewporter` stretches, never a swapchain; its fade commits one cached
buffer per opacity step. Screens go off through
`wlr-output-power-management`, and input turns them back on (Hyprland does
not by itself). Suspend runs `systemctl suspend`, on battery only unless
`suspend_on_ac`, and not while `playerctl` reports `Playing` unless
`suspend_while_playing`. **Lock before sleep**: the shell holds a logind
`sleep` *delay* inhibitor; on `PrepareForSleep(true)` it locks and closes
the descriptor once every lock surface has drawn (or the lock was refused),
with a 4 s give-up under logind's 5 s, and takes a new one on wake. It also
locks on the session's `Lock` signal (`loginctl lock-session`), finding the
user's display session because a user unit is in none. Test it against a
stand-in logind on a private bus (`DBUS_SYSTEM_BUS_ADDRESS`) in headless sway
with a stub `systemctl` first in `PATH`: the suspend stage otherwise
suspends the real machine.

The Battery page's **Idle and sleep** section (`ui/pages/idle_settings.rs`)
sets every `[idle]` key but `dim_color`, and the stay-awake switch.

**Night light** (`night_light.rs` pure and tested: the sunrise equation,
the schedule and its fade, kelvin to RGB, the ramps;
`wayland/state/night_light.rs`) holds a `wlr-gamma-control` per output
only while warmer than daylight, and destroys them in daylight, which gives
the outputs back their own colours. It looks at the schedule every 5 s and
resends the ramps only after a 15 K move. `schedule = "sun"` takes the
location from `[night_light]` or the dashboard weather's `weather.json`,
and without one uses the fixed times. `nightlight on|off|toggle` holds
until the schedule would change anyway; `auto` returns to it. A control
that fails (another program such as gammastep holds the gamma, *or* the
output has none: headless sway's and nested outputs never have it, so the
sandbox can only test the page) is reported on the Display page's card
(`ui/pages/night_light.rs`) and not retried until something changes.

**Settings changed on a page** (idle, night light) are saved by dotted
path in `$XDG_STATE_HOME/stratum/settings.json` (`settings_override.rs`),
with the config's value when each was changed, and laid over the merged
config in `ConfigSource::load`. An override applies only while the config
still has that value: editing the key in the config wins and drops it. A
page sends `SystemAction::SetSetting`, and the shell reloads at once.
Their rows are `ui/pages/settings_card.rs`, whose label column widens to
its longest label. Live tests set `XDG_STATE_HOME`.

Notifications persist (`notification_store.rs`) to
`$XDG_STATE_HOME/stratum/notifications.json`. Restored notifications are
marked as already toasted.

A theme picked live (switcher, Theme page, `theme set`) is saved to
`$XDG_STATE_HOME/stratum/theme.json` with the config's `theme` at the time
(`theme_choice.rs`), since the config is usually a read-only store link. It
overrides the config across reloads and restarts until the config's own
`theme` changes, which wins and removes the file. A dev build shares that
file with the installed shell, so live tests that switch themes set
`XDG_STATE_HOME` to a scratch directory.

**Custom themes** are made on the Theme page (`ui/pages/theme.rs`, editor in
`ui/pages/theme_editor.rs`) and saved one per file as
`$XDG_STATE_HOME/stratum/themes/<name>.toml` (`custom_themes.rs`): a bare
`ThemeConfig`, named by the file. They are merged into `config.themes` at
start and on every reload, before the saved pick is applied; a configured
theme of the same name wins, and an unreadable file is logged and skipped.
The editor sets the fourteen core colours and the font; the Material aliases
stay derived. An edit is shown live through `SystemAction::PreviewTheme`:
`App::theme_preview` holds the config from before it, and every preview is
applied over that, so Discard, leaving the page or closing the window
restores it exactly (`render_settings_window` ends any preview that
outlives the editor). A preview never writes `theme.json`; Save does, via
`switch_theme`.

**Wallpapers** (`wallpapers.rs`) live in `[wallpapers] dir`; the one shown is
`[wallpapers] current`, a symlink hyprpaper and the lock screen read.
Applying one repoints that link (never a real file there) and runs
`hyprctl hyprpaper wallpaper ",<path>"`, on a worker thread that also
decodes thumbnails and 48² colour samples, only once the Theme page asks.
Any theme, built-in or custom, may be linked to a wallpaper
(`$XDG_STATE_HOME/stratum/theme-wallpapers.json`; none by default), which
`switch_theme` applies, so the page, the switcher and `theme set` all bring
it along. The editor's wallpaper is previewed live (`PreviewWallpaper`) and
restored with the theme when the edit ends unsaved; Save links it.
`App::show_wallpaper` compares with the last wallpaper *requested*
(`wallpaper_requested`), not the worker's published `current`: a restore
still queued would otherwise make showing the wallpaper it replaces look
redundant, and the restore would win.
`palette.rs` generates a dark or light theme from a sample: hue bins give
the background tint and three distinct accents, then every colour is moved
in lightness to WCAG contrast (text 7:1, muted 4.5:1, accents 3:1) on
`bg_main`. The lock artwork is keyed on the wallpaper's resolved path as well
as its mtime, since a relinked file may be as old as the last. Live tests use
a scratch `[wallpapers]` directory and, in the nested Hyprland, their own
hyprpaper (`XDG_CONFIG_HOME` holding a `hypr/hyprpaper.conf`).

The **Display page** (`ui/pages/display.rs`) configures monitors in Hyprland:
a map of the enabled monitors at their logical sizes, where a dragged one is
always put against another's edge and never over it (`displays::place`), and
the selected monitor's resolution, refresh rate, scale (only those giving a
whole logical size, from `[displays] scales`), rotation, flip, VRR and on/off.
`display_manager.rs` owns a thread that applies layouts as `hl.monitor`
rules sent with `hyprctl eval` over the socket (`hyprland::eval`). Things
learned against Hyprland 0.57 that the design depends on:

- `eval` answers before the monitors change, and a move raises no event, so
  `send` waits until `j/monitors all` shows the rules before publishing.
- A monitor the config places `auto` is moved whenever another moves, so
  Apply, Revert and re-applying always send a rule for *every* monitor.
- `hyprctl reload` drops every `eval`'d rule; disabling a monitor reports it
  removed, enabling it added. Re-applying is therefore idempotent (nothing is
  sent unless something differs) and skipped while a layout is on trial.

Resolution is a dropdown (`layout::dropdown`/`dropdown_list`: the list is
painted last in its own layer, ends at the window's bottom and scrolls,
with the wheel routed to it through `Pages::scroll`); scale is a
`controls::stepped_slider` over the valid scales. Apply puts a layout on
trial, shown as a modal over the whole settings window on any page
(`display::paint_trial_modal`, painted by `ControlCenter::paint_window`,
which keeps the pointer, the wheel and the nav rail from what is under
it); unless Keep comes within `[displays] confirm_s` the old one is sent back, by the thread's own timeout, so it
works with the window closed. Kept layouts are saved per *set* of connected
monitors (identified by description, so a port change does not matter) in
`$XDG_STATE_HOME/stratum/displays.json`, and applied at start, after
`configreloaded`, and on `monitoradded`/`monitorremoved`.
"Use Hyprland config" forgets the set's layout and reloads.

The **equalizer** is a settings page (`ui/pages/equalizer.rs`). `EqModel` is
the pure, tested state (QML's editing rules and curve model); the blocking
`stratum_sys` EQ calls run on the action worker (`SystemAction::Eq*`), which
publishes the presets and the outcome into `SystemState::eq`. Note that every
shell start already rewrites `~/.config/stratum/audio-eq-presets.json`: the
audio manager re-applies each output's last preset, which saves it again.

Settings pages find clicks as a pressed edge between paints, so the window
latches a press (`settings_press`) until it has painted once with it; a press
and release arriving together were otherwise never seen. Text fields in a page
get keys only while the settings window has keyboard focus. Every pointer
event over the window marks it dirty: without that, hover and drags were
only seen when something else repainted.

**Wi-Fi connect** (`stratum_sys::managers::wifi::connect`) uses NetworkManager's
D-Bus API, never `nmcli … password`: a command line is readable by every local
user. It waits up to 60 s for the activation, so the shell runs it on a thread
of its own, outside the action queue. Test it against a stand-in
NetworkManager (zbus `serve_at` on a `dbus-launch` bus, with
`DBUS_SYSTEM_BUS_ADDRESS` pointing there), never by connecting the real
machine to anything.

**Hover menus** (`ui/hover_menu.rs`) open after resting on a status icon and
close once neither the icon nor the menu has been hovered for a grace period;
the timing is `HoverIntent`, which is pure and tested. Widgets report what
they open through `Widget::hover_target`, keyed by output. The menu is
immediate-mode and painted once per change: the painted height sizes the
surface, and a size change waits for the configure before drawing. Their
actions are real (audio devices, Bluetooth, the platform power profile), so
live tests must not click them.

App icons come from `IconTheme::load_app_icon`, which maps a window class to
its desktop entry's `Icon` (`desktop.rs`) before trying the class as an icon
name. Icon themes and desktop entries are searched in `icons::data_dirs()`,
which always includes the NixOS profile directories: a shell started without
the session's `XDG_DATA_DIRS` (a devshell, a systemd unit) otherwise finds no
user icon theme at all.

**Tray activation** works: left click sends `Activate`, middle click
`SecondaryActivate`, and right click fetches the item's `com.canonical.dbusmenu`
layout (calling `AboutToShow` first, or many menus come back empty) and shows it
in an overlay surface beside the icon. Nested menus are drawn inline rather than
as flyouts. The menu closes `tray_menu.close_ms` after the pointer leaves both
it and its icon, as QML did. The D-Bus calls run
on the action worker through `SystemState::tray_handle`.

The **power menu** is a fullscreen overlay surface per output. Only the output
it opened on is mapped and given `KeyboardInteractivity::Exclusive`, so keys
reach it and nowhere else. It is toggled by `stratum-cli powermenu toggle`; the
shortcut itself stays in the compositor. Keyboard support comes from SCTK's
`xkbcommon` feature (`KeyboardHandler` in `wayland/state/input.rs`); modifier state is
tracked for future text entry.

The **IPC socket** exists at `$XDG_RUNTIME_DIR/stratum-shell.sock` (override
with `$STRATUM_SOCKET`); its targets and function names are the QML
`IpcHandler`s', so keybinds written for Quickshell kept working.
`stratum-cli` talks only to it. Only a lock the shell refuses to take answers
`unsupported`, and the CLI reports it: there is no other locker behind it, so
a refused lock means no lock. A new function needs an entry in
`ipc::functions`, an arm in `App::execute_ipc`, and one in the CLI's
`ipc_functions`.

The **lock screen** (`lock.rs`, `pam.rs`, `wayland/lock_surface.rs`) uses
`ext-session-lock-v1` and checks the password with PAM on a worker thread.
Safety rules it depends on — keep them:

- **Never lock without a way back.** `pam::resolve_service` only picks a
  service that exists in `/etc/pam.d` (PAM silently falls back to `other`,
  which denies everyone on NixOS), and `pam::check_service` dlopens every
  absolute `auth`/`account` module first. If either fails, the shell refuses
  and answers `unsupported`, which the CLI reports. With Quickshell gone that
  means the screen does not lock, so the installed build must link the
  system's glibc (see below) and `/etc/pam.d/stratum-shell` must exist (the
  user's NixOS config defines it).
- **The glibc trap.** A build must link the glibc the system's PAM modules
  (and Vulkan drivers) were built against. One linked against an older glibc
  *must* refuse to lock against the real `/etc/pam.d`, and it does. The
  installed shell follows the system's nixpkgs (`inputs.nixpkgs.follows`);
  the devshell is kept on the same revision through `flake.lock` (below).
- `SessionLock` is released only through `unlock()` after PAM success, or by
  dropping it after the compositor's `finished`. Dropping it while locked is a
  protocol error, which keeps the session locked (fail secure).
- While locked, **no other surface is painted**. The compositor stops releasing
  hidden surfaces' buffers, and presenting to one blocks in the swapchain
  acquire, freezing the loop and the lock screen with it. This happened.

**Testing the lock without locking the real session.** Never lock the user's
Hyprland session to test — you cannot type their password. Run headless sway
in its own runtime dir (it must be short: socket paths are limited to 108
bytes, so use `/run/user/$UID/<name>`, not the scratchpad), point the shell at
it, and in debug builds set `STRATUM_PAM_CONFDIR` to a directory holding a
`stratum-shell` service that uses `pam_exec` with a script accepting a fixed
test password. The modules in that service must come from the libpam the binary
links (`ldd target/debug/stratum-shell`). Drive input with `wtype`: sway drops
the first key of each new virtual keyboard, so start with `wtype -s 300 ...`,
and give `-k Return` its own delay. pam_exec runs its script with an empty
environment, so use shell builtins only.

The **polkit dialog** (`polkit_dialog.rs`) fronts the agent in
`stratum_sys::managers::polkit`, enabled by `[behavior] polkit` (off in a dev
build beside the installed shell — polkit allows one agent). The password field is
shared with the lock screen (`ui/text_field.rs`, masked). The agent's methods run
on **zbus's own executor, not Tokio**: anything Tokio-specific inside them (a
`tokio::time` timer) panics on every request. Password checks are spawned onto
the polkit runtime from the shell instead.

**Testing polkit without a real password.** Do not send wrong passwords to the
user's real account. Debug builds have two hooks: `STRATUM_POLKIT_FIXTURE_NAME`
serves the agent on the session bus under that name without polkitd, and
`STRATUM_POLKIT_HELPER` replaces `polkit-agent-helper-1` with a stand-in
script (reads the cookie and password lines, exits 0 on a fixed test
password). On a private `dbus-launch` bus, call `BeginAuthentication`
(`sssa{ss}sa(sa{sv})`) and `CancelAuthentication` with `busctl --address`.
`STRATUM_POLKIT_SUBJECT_PID` registers with real polkitd for one process only.
polkit 127's `pkexec` is not setuid; use `pkcheck --process` instead.

**Testing against Hyprland without touching the session.** Never apply
monitor rules to the user's Hyprland. Use a nested Hyprland (it opens as a
window in the session; headless sway's xdg-shell is too old to host it) with
fake outputs (`hyprctl output create headless T1`). **It must be isolated
from systemd**: on start Hyprland imports its `WAYLAND_DISPLAY`,
`HYPRLAND_INSTANCE_SIGNATURE`, `PATH` and `XDG_DATA_DIRS` into the user
manager and starts `hyprland-session.target`, and on exit it stops
`graphical-session.target` and unsets them. That restarted the installed
shell inside the nested window, then stopped it; the `HYPRLAND_NO_SD_*`
variables did not prevent the import. What works: give it its own
`XDG_RUNTIME_DIR` (`/run/user/$UID/<short>`) with `WAYLAND_DISPLAY` as the
absolute path of the session's socket, `DBUS_SESSION_BUS_ADDRESS` pointing
nowhere, and stub `systemctl` and `dbus-update-activation-environment` first
in `PATH`; diff `systemctl --user show-environment` before and after, and
end it with `SIGKILL`. Run the dev shell with the same runtime dir and
signature. A virtual pointer's absolute coordinates span the current layout's
bounds, which change as monitors move.

**Overlays on wlroots (sway).** Hyprland is lenient where wlroots is not; two
rules found in sway, both handled by `SurfaceEntry`:

- `unmap()` drops keyboard focus in the *same* commit as the null buffer. A
  separate commit first makes wlroots send a configure that the unmap then
  invalidates, and SCTK's late ack is a fatal "wrong configure serial".
- After an unmap, wlroots sends no configure until a fresh bufferless commit.
  Overlay loops call `entry.ready()`, which makes that commit once, instead of
  checking `entry.configured` directly.
- **The configure race.** A map, unmap or destroy makes wlroots send every
  layer surface a configure. If another surface unmaps after that configure
  was sent but before it is dispatched, SCTK still acks it (it acks before
  our handler runs) and sway kills the client: "wrong configure serial".
  Opening one overlay while closing another in the same frame does it
  (`theme toggle` and `powermenu toggle` together reproduce it). Hyprland
  tolerates the late ack. When one action changes two surfaces, unmap first,
  in the same handler, as the launcher does. After any protocol error
  `wayland::run` exits with failure, so the unit restarts the shell; it used
  to loop with a dead connection and no bar.

The renderer must also release a `Suboptimal` frame before reconfiguring the
surface; reconfiguring while holding it is a fatal wgpu validation error.

Surfaces present with **Mailbox, not Fifo**. Under Fifo a present waits for the
surface's frame callback, which a hidden surface (an overlay unmapped while a
frame was in flight, anything behind the lock) never receives, and the event
loop froze. Measured idle CPU is the same either way.

**Notifications.** Toasts and the page share `ui/notification_card.rs`, whose
`layout` produces every rect that `paint` draws and `hit` resolves. The server
emits `ActionInvoked` and `NotificationReplied` (the `inline-reply`
capability), closing afterwards unless the notification is `resident`. A
focused toast reply field takes exclusive keyboard focus on that output's toast
surface. The QML centre showed only the last reply, never a field, and so does
the page.

**Clicking in headless sway.** It has no pointer device, so the shell never gets
a `wl_pointer`, and `swaymsg seat … cursor` clicks go nowhere. Use a persistent
`zwlr_virtual_pointer_v1` client started before the shell (the scratch tool
used for the notification tests reads `click X Y` lines from a FIFO). One-shot
tools such as `wlrctl` attach and detach the device per command, which races
the shell binding its pointer.

### Decisions made with the user (2026-10-21)

- **Hover menus are kept.** The control centre stays a real window, *and* the
  compact hover menus (audio, Bluetooth, Wi-Fi, battery) are built alongside it,
  with the QML hover-intent delay and close grace period.
- **Cutover requires every surface.** Quickshell was removed only once every
  QML surface existed in the native shell, so the switch was seamless. No
  surface was optional, including screenshots and the dashboard.

## Build and run

Everything goes through the Nix devshell — the Vulkan loader is not on the
default library path, and the shell exits with "no suitable Vulkan adapter"
without it.

```sh
nix develop
cargo build -p stratum-shell
cargo test  -p stratum-shell
cargo clippy -p stratum-shell --all-targets

# Run (from inside `nix develop`):
./target/debug/stratum-shell
```

Outside the devshell, set `LD_LIBRARY_PATH` to a `vulkan-loader` store path.

**Every default lives in `tools/stratum_shell/config.default.toml`; the binary
has none.** Config structs have no `Default` impls and no `#[serde(default)]`
(only `Option` fields and a widget's `children` may be left out), and a
config missing a key is refused. A new option is a struct field *and* a line
in that file, or every test fails. Tests get the defaults from
`Config::test_defaults()`. The binary finds the file at `$STRATUM_DEFAULTS`,
else `<bin>/../share/stratum/` (the package installs it there), else, in debug
builds, the crate directory. The user's file (`$STRATUM_CONFIG`, else
`$XDG_CONFIG_HOME/stratum/config.toml`, else `~/.config/stratum/config.toml`)
is merged over it: tables key by key, lists and values replaced. The Home
Manager module does the same merge with `programs.stratum.settings` and always
writes the whole file. A start whose user file is invalid logs the error and
runs on the defaults alone; a reload that fails keeps the previous config.
Reload watches the user file's link target as well as its mtime, because every
Nix store file has the same mtime.

### The installed shell

The user's session runs the installed shell as `stratum-shell.service`
(`systemctl --user status|restart stratum-shell`, logs in
`journalctl --user -u stratum-shell`). It is built from GitHub `main` through
`~/nixos-config` (host `legion`), whose `stratum` input follows the system's
nixpkgs; without that the shell links an older glibc and the lock refuses.
A change reaches the session by pushing to `main`, `nix flake update stratum`
in `~/nixos-config`, and the user's `nixos-rebuild switch`. Ask before
pushing.

**Keep `flake.lock`'s nixpkgs on the system's revision**
(`nixos-version --json` → `nixpkgsRevision`; copy `rev`, `narHash` and
`lastModified` from `~/nixos-config/flake.lock`'s root nixpkgs node, keeping
`original`). Then a devshell build links the system's glibc, renders on the
real drivers and passes the lock's PAM check, so `cargo build` (seconds
incrementally; dependencies are built at opt-level 2 even in debug) is
enough to test a change. When the system moves ahead, a devshell build stops
rendering ("no suitable Vulkan adapter") and refuses to lock until the lock
is moved too. To try a change in the session,
`systemctl --user stop stratum-shell`, run the build, and start the unit
again afterwards. Never leave the user without a bar or a lock. The Nix
packages skip `checkPhase` (`doCheck = false`): it compiled the whole release
build a second time for `cargo test`, doubling a ~7 min build. That makes
`cargo test --workspace` in the devshell the only test run, so run it before
every commit.

### Running a dev build beside the installed shell

The installed shell owns `org.freedesktop.Notifications`,
`org.kde.StatusNotifierWatcher`, the polkit agent, the idle stages and the
logind sleep lock, the clipboard history, and the IPC socket. A second shell needs
`[behavior] tray = false`, `notifications = false`, `polkit = false`,
`[idle] enabled = false`, `[clipboard] enabled = false` and its own
`STRATUM_SOCKET`. To exercise notifications or the
tray in a dev build, use a private bus instead:

```sh
eval $(dbus-launch --sh-syntax)
DBUS_SESSION_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
  STRATUM_SOCKET=/run/user/$UID/stratum-dev.sock \
  STRATUM_CONFIG=/tmp/dev-config.toml ./target/debug/stratum-shell &
notify-send -a App "Summary" "Body"
```

Stop a dev shell by its PID, not `pkill -x stratum-shell`, which also stops
the installed one (systemd does not restart it after `SIGTERM`).

## Architecture of the native shell

### Process and event loop

One process. The Wayland connection, all rendering, and all system managers live
in it; there is no IPC between UI and state.

`wayland/mod.rs` runs a `calloop` loop with an **adaptive timeout**: 16 ms while
anything animates, one second when idle. Worker threads (actions, pollers,
PipeWire, D-Bus) wake it through a `calloop::ping` installed in `App::new` and
exposed as `SystemState::set_waker`.

`App` and its fields are in `wayland/state.rs`; its behaviour is split by
concern into `wayland/state/`: `render.rs` (painting each surface),
`input.rs`, `outputs.rs` (surfaces per output), `lock.rs`, `screenshot.rs`,
`overlays.rs` (polkit, toasts, dashboard, themes, hover menus), `ipc.rs`,
`monitors.rs` (managers and config reload) and `settings_window.rs`.
`paint_ctx!` is defined before those modules are declared, which is what
makes it visible in them.

### State

`sys_state.rs` holds `Cell<T>` per domain (audio, net, wifi, wifi_networks,
bluetooth, battery, music, dashboard, polkit, hypr, tray, brightness,
notifications, dnd). Managers in `stratum_sys` publish into them through
`SystemState::publisher`, which **wakes the loop only when a snapshot actually
changed** — PipeWire republishes on many events that carry identical state, and
treating those as changes was the largest source of idle CPU.

Widgets read state while painting. They never subscribe; `render_pending` redraws
when `take_dirty` reports a change.

### UI layer

```
ui/mod.rs            PaintCtx, InputState, ClickCtx, Action, Widget trait,
                     Column, shared drawing helpers
ui/style.rs          every size, padding, gap, radius, duration  <-- [metrics]
config.rs            TOML schema, colours, themes                <-- [themes]
ui/widgets/          one module per bar widget
ui/pages/            one module per control-centre page
ui/control_center.rs window chrome only (nav rail + page frame)
ui/controls.rs       shared immediate-mode primitives
ui/animation.rs      Tween, Easing
```

**Colours and sizing are separated on purpose.** Colours live in
`[themes.*]`; every dimension lives in `[metrics.*]`. Widget code must not
contain a literal colour or a literal size — add a field to `ui/style.rs`
and its value to `config.default.toml` instead. This is what lets the user restyle and re-lay-out without touching
code, and they have asked for it explicitly.

`Widget` has `height(width, metrics)`, `width(height, metrics)`, `paint`, `click`,
`tick`, `animating`. Flexible widgets return `None` from `height`/`width` and
absorb slack (the QML `Layout.fillHeight` idiom).

Clicks are routed with `ClickCtx`, which carries the **output name** the surface
belongs to. The widget tree is shared by every monitor, so per-monitor state
(workspace pages) must be keyed by that name — this was a real bug.

Pages keep an `Optimistic<T>` for values the user just changed, so a toggle or
slider reflects the input on the next frame and clears when reality agrees.

### Rendering

`render/display_list.rs` is the backend-agnostic vocabulary: `Color`, `Rect`
(SDF rounded box), `Path` (lyon tessellation), `Image`, `TextRun`, plus
`push_clip`/`pop_clip` for scissor clipping. It has no Wayland or wgpu types.

`render/wgpu_renderer.rs` implements `SurfaceRenderer`: one pipeline, one vertex
format, one draw call split into scissored segments. The shader, layouts,
sampler, a pipeline per surface format, and the 1024² atlas are built once
in `Gpu::shared` (`Shared`) and used by every surface. The atlas is
repacked by each surface's frame, which is safe because wgpu orders a
`write_texture` between the submits around it; it has no CPU copy. The
device asks for `MemoryHints::MemoryUsage`: the default reserves 192 MiB
(128 device + 64 host) up front, which on the iGPU is over a third of its
512 MiB carve-out. Notable invariants:

- **No padding fields in `Vertex`.** `vertex_attr_array!` assigns offsets
  sequentially; an extra field silently corrupts the UVs and makes all text draw
  as solid blocks. This has happened.
- The surface must be **`Bgra8Unorm`/`Rgba8Unorm`**, not sRGB, or the palette
  washes out.
- Overlays need a **premultiplied alpha** composite mode, not `alpha_modes[0]`,
  or a transparent clear composites as opaque black.
- The **path cache key must include the paint**. Tessellation bakes colour into
  the vertices, so two paths with the same geometry and different colours have
  different vertices. Keying on geometry alone made the power menu's selection
  outline stay on whichever tile was highlighted first.
- `textureSampleLevel`, not `textureSample`, because of divergent control flow.
- **Antialiasing has no MSAA**; coverage comes from the shader. A rect's quad
  reaches half a pixel past it so the SDF can fade its edge. Outlines
  (`DisplayList::push_outline`, which `ui::stroke_rounded` uses) are a band of
  that same SDF, inside the edge. Butt-capped path strokes are tessellated a
  pixel wider, with each vertex's signed distance from the centreline in
  `local.x` (`radius` = `RADIUS_STROKE`). Fills and round-capped strokes are
  still hard-edged: lyon puts same-side vertices next to each other in round
  caps, which that distance cannot describe.
- Within a layer, all text draws after all geometry. Something that must
  cover text (a dropdown's open list, a modal) calls
  `DisplayList::push_layer` first: a later layer draws entirely over the
  earlier ones. `pop_clip` inside another clip re-sets the parent, since
  the renderer's `PopClip` clears the scissor.
- Pictures too large for the 1024² atlas (the switcher's wallpapers) are
  `DisplayList::push_texture`: a texture of their own per key, drawn in
  order into any quadrilateral, freed once a frame no longer draws them
  and on `release_backdrop`. Batch segments carry the texture they bind.
  Their edges are hard; stroke a path over them.
- Clear to `Color::TRANSPARENT`, never a theme colour with zero alpha. The
  swapchain is premultiplied, so colour under zero alpha is *added* to what is
  below: the strip beside the rail that holds the inverted corners came out
  solid `bg_main`.

`text.rs` shapes with cosmic-text and rasterizes with swash. Shaping results are
cached by `(text, family, size, weight)`. Widths come from the shaper's
`line_w`; deriving them from the last glyph's pen position omits that glyph's
advance and shifts every centred label to the right.

Each `SurfaceEntry` owns the layer surface and its swapchain. `unmap` drops
the renderer after committing the null buffer, so a hidden overlay holds no
swapchain (four output-sized images: 60 MB at 2560x1440, 150 MB at
4000x2400), and the next show builds one again. **Field order is
load-bearing**: the renderer must be declared before the Wayland object it
presents into, because Rust drops fields in declaration order and destroying the
`wl_surface` first is a use-after-free inside the driver. The same applies to
`run()` dropping `app` before `conn`.

### Wayland specifics

- `wlr-layer-shell` for the bar (exclusive zone), OSD and toasts (overlay layer,
  no exclusive zone).
- xdg-shell for the control centre, so it behaves like a normal window.
- `set_buffer_scale` is set on every surface; without it the compositor assumes
  scale 1 and halves everything.
- Outputs' names and scale arrive **asynchronously**. Surfaces are created only
  once `OutputState::info()` has the name, and retried from `update_output`;
  creating them earlier keys every surface to an empty string and breaks
  per-monitor state.
- Toast surfaces are full-height so their stack can grow, so they install an
  input region covering only the cards. Without it the strip swallows clicks.

## Working practices

- **Never add a UI toolkit.** If something seems to need one, the answer is more
  code in `render/` and `ui/`.
- **Verify against the running shell**, not just the compiler. Several bugs in
  this project were only visible on screen (text at the bottom of boxes, icons as
  solid blocks, per-monitor spacing).
- **Trust the user's screenshot over your own cropping.** The stitched capture is
  6656 px wide across two monitors with different scales; positional crops have
  repeatedly shown the wrong monitor. Ask which monitor, or use
  `grim -o <output>` and get the layer positions from `hyprctl -j layers`.
- Prefer measuring over guessing: `hyprctl -j layers` gives exact layer
  geometry, `grim -o` captures one output, `magick` can report pixel values and
  ink bounding boxes.
- Commit messages should say **why**. The user has asked that no
  `Co-Authored-By` trailer be added.
- Tests exist for parsing, animation, layout maths, and text metrics; run
  `cargo test --workspace` before committing.

## Environment notes

- Hyprland 0.56 routes dispatches through Lua:
  `hyprctl dispatch 'hl.dsp.focus({ workspace = "3" })'`. The old
  `workspace 3` shorthand errors out.
- The machine has both an AMD iGPU and an NVIDIA dGPU; some ICDs in the system
  Vulkan directory fail to load (GLIBC mismatch) and log noise at startup. That
  is expected and not fatal.
- Never run `systemctl --user import-environment` or
  `dbus-update-activation-environment --all` inside `nix develop`: it copies
  the devshell's environment (`CC`, `LD_LIBRARY_PATH`, a `TMPDIR` under
  `/tmp/nix-shell-*` that is deleted on exit) into every user unit,
  including the installed shell. A dead `TMPDIR` made `wl-copy` exit 1
  silently; the shell now unsets a temp variable naming a missing directory
  at start (`drop_missing_temp_dirs`).
- Nerd Font codepoints are used for all icons. The theme's `font` must resolve to
  a Nerd Font or icons render as boxes.
