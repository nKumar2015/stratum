# stratum-shell

The native Stratum desktop shell — the Rust replacement for the Quickshell/QML
implementation. It owns the Wayland connection, renders with wgpu on Vulkan, and
embeds the system managers directly (no daemon socket between UI and state).

## Status

M3: the sidebar is config-driven. It draws real widgets from the configuration
file — launcher, workspaces, vertical app title, quick-panel group, battery, and
clock — using the four palettes the QML shell shipped with.

Not yet ported: menus, hover panels, notifications, OSD, lock screen, polkit
dialog, power menu, dashboard, and the screenshot suite. See
`docs/native-shell-parity.md` at the repository root.

## Configuration

The shell reads one TOML file:

1. `$STRATUM_CONFIG`
2. `$XDG_CONFIG_HOME/stratum/config.toml`
3. `~/.config/stratum/config.toml`

With no file it uses built-in defaults that mirror the old QML shell. A copy of
the full schema with comments is in `config.example.toml`.

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
