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
  qtdeclarative alone ~692 MiB. Our release binary is currently ~7.1 MB.
- **Feature parity, not pixel parity.** The QML shell's behaviour and data must be
  preserved; the visual style is deliberately changing to "vectorheart".
- **Long-term goal:** eventually as configurable as Quickshell — arbitrary
  placement, animation, styling — without the footprint.
- **Hard switch at the end.** Quickshell stops being used. `stratumd` may be
  removed or absorbed as long as its features survive.
- **Parallel migration** until parity is reached. Both shells run side by side
  today, which means name collisions on D-Bus (see below).

### Naming and layout

```
tools/stratum_sys/     library: system managers (audio, wifi, bluetooth,
                       battery, music, dashboard, polkit), host-agnostic,
                       publishes snapshots through `Publisher`
tools/stratumd/        transitional daemon serving the QML shell; to be retired
tools/stratum_cli/     CLI; its commands are the contract external keybinds use
tools/stratum_shell/   the native shell
sidebar/ modules/ globals/ components/ themes/ shell.qml
                       the Quickshell implementation — reference only, deleted
                       at cutover
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

Not implemented: hover menus, theme switcher
overlay, audio EQ panel, notification actions/inline reply, screenshots,
dashboard, app icon beside the window title. See
`docs/native-shell-parity.md`.

**Tray activation** works: left click sends `Activate`, middle click
`SecondaryActivate`, and right click fetches the item's `com.canonical.dbusmenu`
layout (calling `AboutToShow` first, or many menus come back empty) and shows it
in an overlay surface beside the icon. Nested menus are drawn inline rather than
as flyouts, and hover-out auto-close is not implemented yet. The D-Bus calls run
on the action worker through `SystemState::tray_handle`.

The **power menu** is a fullscreen overlay surface per output. Only the output
it opened on is mapped and given `KeyboardInteractivity::Exclusive`, so keys
reach it and nowhere else. It is toggled by `stratum-cli powermenu toggle`; the
shortcut itself stays in the compositor. Keyboard support comes from SCTK's
`xkbcommon` feature (`KeyboardHandler` in `wayland/state.rs`); modifier state is
tracked for future text entry.

The **IPC socket** exists at `$XDG_RUNTIME_DIR/stratum-shell.sock` (override
with `$STRATUM_SOCKET`) and mirrors the QML `IpcHandler` targets and function
names. `stratum-cli` tries it first and falls back to `qs ipc` for functions the
native shell reports as `unsupported`, so keybinds work during the parallel
migration. Implemented targets: `notifications` (all functions), `theme set`,
`powermenu toggle`, and `lockscreen lock`. The remaining targets (`screenshot`,
`dashboard`, `theme open/close/toggle`) still fall back to Quickshell; porting
one means adding an arm in `App::execute_ipc` and extending
`ipc::is_supported`.

The **lock screen** (`lock.rs`, `pam.rs`, `wayland/lock_surface.rs`) uses
`ext-session-lock-v1` and checks the password with PAM on a worker thread.
Safety rules it depends on — keep them:

- **Never lock without a way back.** `pam::resolve_service` only picks a
  service that exists in `/etc/pam.d` (PAM silently falls back to `other`,
  which denies everyone on NixOS), and `pam::check_service` dlopens every
  absolute `auth`/`account` module first. If either fails, the shell refuses
  and answers `unsupported`, so the CLI falls back to Quickshell.
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
shared with the lock screen (`ui/password_field.rs`). The agent's methods run
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

Configuration is read from `$STRATUM_CONFIG`, else
`$XDG_CONFIG_HOME/stratum/config.toml`, else `~/.config/stratum/config.toml`.
The full commented schema is `tools/stratum_shell/config.example.toml`; that file
is also the best smoke test (`STRATUM_CONFIG=... ./target/debug/stratum-shell`).

**Stop the shell with `pkill -x stratum-shell`.** Do not leave it running between
sessions — the user starts it themselves when they want to test.

### Testing surfaces that Quickshell owns

Quickshell currently owns `org.freedesktop.Notifications` and
`org.kde.StatusNotifierWatcher`. To exercise those in the native shell without
stopping Quickshell, run on a private bus:

```sh
eval $(dbus-launch --sh-syntax)
DBUS_SESSION_BUS_ADDRESS="$DBUS_SESSION_BUS_ADDRESS" \
  STRATUM_CONFIG=/tmp/notify-config.toml ./target/debug/stratum-shell &
notify-send -a App "Summary" "Body"
```

A minimal config for that has `[behavior] tray = true` and `notifications = true`.

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
instead. This is what lets the user restyle and re-lay-out without touching
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
