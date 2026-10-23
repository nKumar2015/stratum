# CLAUDE.md

Guidance for working in this repository.

## What this project is

Stratum is a NixOS desktop environment built around **Hyprland**. Historically its
shell was written in QML and run on **Quickshell**; `stratumd` is a daemon that
serves that QML shell over a Unix socket, with `stratum-cli` as its client.

We are replacing Quickshell with a **native Rust shell** in
`tools/stratum_shell/`. This is not a fork or a port — it is a from-scratch
implementation with its own renderer, layout, and theming.

### The user's requirements (verbatim intent, do not reinterpret)

- **Full replacement of QML.** No QML, and no existing UI/design toolkit
  (no GTK, no Qt, no slint, no iced). Every visual is ours.
- **The motivation is footprint.** Binary size, memory, CPU, and startup time must
  beat Quickshell. Measured baselines: Quickshell's closure is ~1.1 GiB,
  qtdeclarative alone ~692 MiB. Our stripped release binary was ~7.1 MB early
  on, grew to ~16 MB as the system managers were embedded, and is ~13.1 MB
  as of 2026-10-22. Before adding a dependency, check its default features:
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
  The QML tree and stratumd stay in the repo until a soak period ends.

### Naming and layout

```
tools/stratum_sys/     library: system managers (audio, wifi, bluetooth,
                       battery, music, dashboard, polkit), host-agnostic,
                       publishes snapshots through `Publisher`
tools/stratumd/        the daemon that served the QML shell; retired, to delete
tools/stratum_cli/     CLI; its commands are the contract external keybinds use
                       (audio, battery, bluetooth, net, wifi and dashboard need
                       stratumd and are to delete with it)
tools/stratum_shell/   the native shell
sidebar/ modules/ globals/ components/ themes/ shell.qml
                       the Quickshell implementation — reference only, to delete
module.nix             Home Manager module: shell, CLI, runtime tools, user unit
docs/native-shell-parity.md
                       behavioural acceptance checklist; mark a row `verified`
                       only after exercising it in the running native shell
```

## Current state

Implemented and working: sidebar (workspaces, app title, tray, quick panel,
status icons, battery, clock), control centre as an xdg-shell window with eight
pages (overview, audio, bluetooth, wifi, battery, notifications, display, power),
volume/brightness OSD, notification toasts, notification server, tray watcher,
live theme switching, config hot reload, keyboard input, the power menu overlay,
the IPC socket, the lock screen, and the polkit dialog (both verified only
against fixtures in a nested compositor; see below).

The theme switcher overlay is implemented too (`theme_switcher.rs`).

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
annotated exports. Actions shell out to `stratum-cli screenshot-post` and
`portal-save-file`, the QML contract. Every surface sets a cursor shape on
enter: once one surface sets the crosshair, compositors keep it across
surfaces. Live tests must set `HOME` to a scratch directory, because Save
writes to `~/Pictures/Screenshots`.

The **dashboard** (`dashboard/`) is a fullscreen overlay of tabs, each rows
of panels, from `[[dashboard.tabs]]`; it always opens on the first, or on the
tab `dashboard open|toggle <tab>` names. Its data comes from a sampler thread
(`dashboard/feed.rs`, over `stratum_sys::managers::sysmon`) that keeps a
history for the graphs, recording in the background but publishing into
`SystemState::dashboard` only while open, so a closed dashboard never wakes
the loop. GPUs and processes are sampled only while open: an NVML reading
costs ~20 ms (held open, NVML would keep the dGPU awake), and processes a
file each. The System tab's graphs are `ui/graph.rs`, which spans a fixed
number of readings so the line scrolls rather than stretches. Weather
(`dashboard/weather.rs`, `stratum_sys::managers::weather`) comes from
Open-Meteo through `curl`, only while open and at most every `refresh_min`;
the postal code's coordinates and the last forecast are cached in
`$XDG_STATE_HOME/stratum/weather.json`, so live tests set `XDG_STATE_HOME`. The sampler also refreshes
`SystemState::music` while open. The calendar grid is computed locally. Media
controls run `playerctl` on the action worker. The overhaul has phases still
to come (graphs, weather, calendar sync, to-dos); the plan is in memory.

Notifications persist (`notification_store.rs`) to
`$XDG_STATE_HOME/stratum/notifications.json`, not to Quickshell's
`~/.local/state/quickshell/notifications-snapshot.json`, so the two shells never
overwrite each other; the Quickshell file is imported once when the native one
does not exist. Restored notifications are marked as already toasted.

A theme picked live (switcher, Display page, `theme set`) is saved to
`$XDG_STATE_HOME/stratum/theme.json` with the config's `theme` at the time
(`theme_choice.rs`), since the config is usually a read-only store link. It
overrides the config across reloads and restarts until the config's own
`theme` changes, which wins and removes the file. A dev build shares that
file with the installed shell, so live tests that switch themes set
`XDG_STATE_HOME` to a scratch directory.

The **equalizer** is a settings page (`ui/pages/equalizer.rs`). `EqModel` is
the pure, tested state (QML's editing rules and curve model); the blocking
`stratum_sys` EQ calls run on the action worker (`SystemAction::Eq*`), which
publishes the presets and the outcome into `SystemState::eq`. Note that every
shell start already rewrites `~/.config/stratum/audio-eq-presets.json`: the
audio manager re-applies each output's last preset, which saves it again.

Settings pages find clicks as a pressed edge between paints, so the window
latches a press (`settings_press`) until it has painted once with it; a press
and release arriving together were otherwise never seen. Text fields in a page
get keys only while the settings window has keyboard focus.

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
user icon theme at all. See
`docs/native-shell-parity.md`.

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
`xkbcommon` feature (`KeyboardHandler` in `wayland/state.rs`); modifier state is
tracked for future text entry.

The **IPC socket** exists at `$XDG_RUNTIME_DIR/stratum-shell.sock` (override
with `$STRATUM_SOCKET`) and mirrors the QML `IpcHandler` targets and function
names. `stratum-cli` talks only to it; the `qs ipc` fallback was removed at
the switch. Every QML target and function is served natively
(`ipc::tests::every_qml_function_is_served_natively`). Only a lock the shell
refuses to take answers `unsupported`, and the CLI reports it: there is no
other locker behind it, so a refused lock means no lock. A new function needs
an arm in `App::execute_ipc` and in `ipc::is_supported`.

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
- **The glibc trap.** This repo's `flake.lock` nixpkgs is older than the
  system's, so a devshell build links glibc 2.42 while the system's PAM modules
  need 2.43. Such a build *must* refuse to lock against the real
  `/etc/pam.d` — and it does. A shell that should lock for real has to be built
  against the system's nixpkgs (`inputs.nixpkgs.follows`).
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
`stratum_sys::managers::polkit`, enabled by `[behavior] polkit` (off while
stratumd holds the session's agent — polkit allows one). The password field is
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

**Overlays on wlroots (sway).** Hyprland is lenient where wlroots is not; two
rules found in sway, both handled by `SurfaceEntry`:

- `unmap()` drops keyboard focus in the *same* commit as the null buffer. A
  separate commit first makes wlroots send a configure that the unmap then
  invalidates, and SCTK's late ack is a fatal "wrong configure serial".
- After an unmap, wlroots sends no configure until a fresh bufferless commit.
  Overlay loops call `entry.ready()`, which makes that commit once, instead of
  checking `entry.configured` directly.

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
- **Cutover requires every surface.** Quickshell is removed only when every row
  in `docs/native-shell-parity.md` exists in the native shell, so the switch is
  seamless. No surface is optional, including screenshots and the dashboard.

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

A devshell build cannot replace it for real use: it renders on the NVIDIA GPU
(the devshell's glibc cannot load RADV) and refuses to lock. To try a change
in the session, build against the system nixpkgs
(`nix build .#stratum-shell --override-input nixpkgs path:<system nixpkgs>
--no-write-lock-file`; the path is `nix registry list`'s `system flake:nixpkgs`),
`systemctl --user stop stratum-shell`, and run that binary; start the unit
again afterwards. Never leave the user without a bar or a lock.

### Running a dev build beside the installed shell

The installed shell owns `org.freedesktop.Notifications`,
`org.kde.StatusNotifierWatcher`, the polkit agent and the IPC socket. A second
shell needs `[behavior] tray = false`, `notifications = false`,
`polkit = false` and its own `STRATUM_SOCKET`. To exercise notifications or the
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
ui/animation.rs      Tween, Spring, ColorSpring, Easing
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
format, one draw call split into scissored segments. Notable invariants:

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
- Clear to `Color::TRANSPARENT`, never a theme colour with zero alpha. The
  swapchain is premultiplied, so colour under zero alpha is *added* to what is
  below: the strip beside the rail that holds the inverted corners came out
  solid `bg_main`.

`text.rs` shapes with cosmic-text and rasterizes with swash. Shaping results are
cached by `(text, family, size, weight)`. Widths come from the shaper's
`line_w`; deriving them from the last glyph's pen position omits that glyph's
advance and shifts every centred label to the right.

Each `SurfaceEntry` owns the layer surface and its swapchain. **Field order is
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
- Keep `docs/native-shell-parity.md` honest. `verified` means exercised in the
  native shell against real devices, not "the code exists".
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
- Nerd Font codepoints are used for all icons. The theme's `font` must resolve to
  a Nerd Font or icons render as boxes.
