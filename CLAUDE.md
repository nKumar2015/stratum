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
live theme switching, config hot reload.

Not implemented: lock screen, polkit dialog, power menu overlay, theme switcher
overlay, audio EQ panel, notification actions/inline reply, tray activation and
DBusMenu, screenshots, dashboard. See `docs/native-shell-parity.md`.

Recommended order and the open questions about cutover criteria are recorded at
the end of the last session's notes; the highest-value next item is the **IPC
socket**, because external keybinds and `stratum-cli` currently reach Quickshell
via `qs ipc` and have no native equivalent yet.

### Decisions still open with the user

Ask before building these; they change how much work remains.

- **Hover menus.** QML opens compact hover menus on icon hover. The user asked
  for the control centre to be a real window instead. Unclear whether hover
  menus are still wanted alongside it, or whether hover should do nothing.
- **Cutover criteria.** Which surfaces must exist before Quickshell is removed.
  Lock screen and polkit are safety-critical; screenshots and dashboard are
  conveniences.

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
notifications). Managers in `stratum_sys` publish into them through
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
