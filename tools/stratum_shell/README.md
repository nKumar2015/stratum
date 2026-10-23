# stratum-shell

The native Stratum desktop shell — the Rust replacement for the Quickshell/QML
implementation. It owns the Wayland connection, renders with wgpu on Vulkan, and
embeds the system managers directly (no daemon socket between UI and state).

## Status

Every surface of the QML shell is ported, and the native shell replaced it on
2026-10-22. See `docs/native-shell-parity.md` at the repository root.

## IPC

External tools talk to the shell over a Unix socket at
`$XDG_RUNTIME_DIR/stratum-shell.sock` (override with `$STRATUM_SOCKET`). It
mirrors the QML `IpcHandler` contract, so `stratum-cli` and existing keybinds
work unchanged:

```sh
stratum-cli notifications toggle
stratum-cli notifications toggleDnd
stratum-cli theme set gruvbox
```

A function the shell will not serve (only a lock it cannot safely take)
answers `unsupported`, and the CLI reports it.

## Configuration

Every default is in `config.default.toml`, which documents every option and is
installed as `share/stratum/config.default.toml`; the binary has no defaults of
its own. Your file is merged over it (tables key by key, lists and values
replaced), so it only needs what you change:

1. `$STRATUM_CONFIG`
2. `$XDG_CONFIG_HOME/stratum/config.toml`
3. `~/.config/stratum/config.toml`

The Home Manager module always writes that file in full: the defaults with
`programs.stratum.settings` merged in. The shell reloads it when it changes,
including when a rebuild re-points its link into the Nix store.

```toml
theme = "gruvbox"

[sidebar]
position = "left"      # or "right"
width = 40
padding = 12
spacing = 16
inverted_corners = true

[[sidebar.widgets]]
kind = "app_title"     # flexible: absorbs the leftover space

[[sidebar.widgets]]
kind = "quick_panel"
framed = true
children = [
  { kind = "audio" },
  { kind = "bluetooth" },
  { kind = "wifi" },
]

[themes.gruvbox]
bg_main = "#1d2021"
primary = "#83a598"
# ... see the example for the full palette
```

Notes:

- Widgets render in list order, top to bottom. Removing an entry hides it.
- `app_title` is flexible (like QML's `Layout.fillHeight`): it consumes the
  remaining vertical space, which pushes the widgets after it to the bottom.
- Colours accept `#rgb`, `#rrggbb`, or `#rrggbbaa`.
- Colours live in `[themes.*]`; everything else visual (font sizes, row heights,
  paddings, gaps, radii, animation timings) lives in `[metrics.*]`. Neither has
  literals left in widget code, so restyling means editing the config.
- **Unknown keys are a hard error**, so a typo surfaces instead of being ignored.
- Editing the file reloads it live (`behavior.reload_on_change`), including theme
  switches. A parse error keeps the previous config and logs the problem, so a
  typo cannot blank a running shell. Changing the sidebar width or side needs a
  restart, since the layer surface anchorage is set once.

## Layout

- `config.rs` — TOML schema, colour parsing, the four built-in themes, and the
  live-reload path.
- `ui/style.rs` — every size, padding, gap, radius, and duration, grouped by
  surface. Colours stay in `config.rs`; nothing draws with a literal.
- `ui/widgets/` — one module per bar widget (workspaces, app title, tray,
  quick panel, icons, battery, clock, launcher, spacer).
- `ui/pages/` — one module per control-centre page.
- `ui/control_center.rs` — window chrome only: navigation rail and page frame.
- `ui/controls.rs` — shared immediate-mode controls (slider, toggle, button,
  selectable row) that pages compose.
- `sys_state.rs` — in-process state cells fed by `stratum_sys` publishers.
- `render/` — display list, wgpu renderer, glyph atlas, tessellation.
- `text.rs` / `icons.rs` — shaping with Nerd Font fallback, icon-theme lookup.
- `wayland/` — SCTK plumbing, layer surfaces, per-output scaling.
- `glyph_probe.rs` — offscreen render test for the text pipeline.

## Build

```sh
nix develop
cargo run -p stratum-shell
```

The Vulkan loader lives in the devshell, so run through `nix develop` (or put
its `lib` on `LD_LIBRARY_PATH`) if the shell reports no adapter.
