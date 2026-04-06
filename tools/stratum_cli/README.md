# Stratum CLI Command Reference

This document summarizes the Stratum Rust CLI commands and what each command does to the system.

The binary name is `stratum-cli`.

- Hover mode note:
  - `--hover` exists for hover menu UI flows. Hover menus need fast, lightweight responses and avoid full settings-window behavior, so this flag enables hover-oriented output/command behavior without duplicating subcommand names.

## Global Help

- Show root help:
  - `stratum-cli`
  - `stratum-cli help`
  - `stratum-cli --help`
- Show command-specific help:
  - `stratum-cli help <command>`
  - `stratum-cli <command> --help`
  - `stratum-cli <command> -h`
  - `stratum-cli <command> help`

- Show shell IPC target help:
  - `stratum-cli help <target>`
  - `stratum-cli <target> --help`

Command responses are JSON on stdout. Help output is human-readable plain text.

Direct shell IPC syntax:

- `stratum-cli <target> <function> [args...]`
- Internally this resolves the newest running Quickshell instance and executes:
  - `qs ipc --pid <PID> call <target> <function> [args...]`

Supported IPC targets:

- `notifications`
- `powermenu`
- `lockscreen`
- `screenshot`
- `dashboard` (only `open|close|toggle`; data commands remain under dashboard subcommands)

## Command Groups and Effects

## Shell IPC Targets

- `notifications`
  - Functions: `open`, `close`, `toggle`, `clear`, `toggleDnd`
  - Example: `stratum-cli notifications toggle`

- `powermenu`
  - Functions: `toggle`
  - Example: `stratum-cli powermenu toggle`

- `lockscreen`
  - Functions: `lock`
  - Example: `stratum-cli lockscreen lock`

- `screenshot`
  - Functions: `start`
  - Example: `stratum-cli screenshot start`

- `dashboard` (IPC controls)
  - Functions: `open`, `close`, `toggle`
  - Example: `stratum-cli dashboard toggle`

Notes:

- `dashboard` also has non-IPC data subcommands (`all`, `calendar`, `music`, `performance`).
- `stratum-cli dashboard open|close|toggle` routes to shell IPC.
- `stratum-cli dashboard all|calendar|music|performance` keeps existing data behavior.

## `audio`

- Query and control PulseAudio/PipeWire default sink/source state.
- Subcommands:
  - `status [--hover]`: default returns current output volume, mute state, and headphone detection; with `--hover`, returns volume/mute plus default sink/source and available sinks/sources for hover menu UI.
  - `set-output <sink>`: sets the default output sink.
  - `set-input <source>`: sets the default input source.
  - `set-volume <0-150>`: sets default sink volume percentage.
  - `open-control`: launches `pavucontrol`.
- Side effects:
  - Changes default audio routes and volume.
  - Launches a GUI app for `open-control`.

## `battery`

- Read battery/charging status and set platform profile.
- Subcommands:
  - `status`: reads battery percent/state, projected runtime/charge time, uptime-derived screen-on time, charging info, and active platform profile.
  - `set-profile <low-power|balanced|balanced-performance>`: writes profile to `/sys/firmware/acpi/platform_profile`.
- Side effects:
  - `set-profile` changes system power profile.

## `bluetooth`

- Read and control Bluetooth adapter/devices.
- Subcommands:
  - `check`: quick state for sidebar (off/on/connected/none fallback).
  - `state`: adapter powered state from `bluetoothctl show`.
  - `list [--hover]`: all known/paired devices with connected/trusted/paired flags; with `--hover`, returns paired devices with connected state.
  - `connect <mac> [--hover]`: default mode trusts device, enables agent/default-agent, then connects; with `--hover`, performs direct connect flow.
  - `disconnect <mac> [--hover]`: disconnects device; `--hover` marks hover response mode.
  - `pair <mac> [--hover]`: pairs and trusts device; `--hover` marks hover response mode.
  - `forget <mac>`: removes device.
  - `power <on|off>`: toggles adapter power.
  - `scan`: starts a short scan window.
- Side effects:
  - Pairing/trusting/removing devices, power toggle, and scan operations all mutate Bluetooth state.

## `dashboard`

- Provides dashboard data blocks and dashboard panel IPC controls.
- Subcommands:
  - `all [year month]`: emits calendar + media + performance in one payload.
  - `calendar [year month]`: emits calendar metadata and rows.
  - `music`: emits current media metadata from `playerctl`.
  - `performance`: emits CPU/GPU/RAM/storage metrics.
  - `open`: opens the dashboard panel via shell IPC target.
  - `close`: closes dashboard panel via shell IPC target.
  - `toggle`: toggles dashboard panel via shell IPC target.
- Side effects:
  - Data subcommands are read-only system probing.
  - `open|close|toggle` mutate UI visibility via shell IPC.

## `net`

- Quick network check for sidebar state.
- Subcommands:
  - `check`: returns `ethernet`, `wifi` (with signal percent), or `none`.
- Side effects:
  - Read-only network state probing.

## `notifications-snapshot`

- Stores/loads the notification snapshot used by the notification listener flow.
- Subcommands:
  - `snapshot-save <urlencoded_json>`: saves snapshot text.
  - `snapshot-load`: returns saved snapshot text (raw JSON payload if present).
- Storage path:
  - `${XDG_STATE_HOME:-$HOME/.local/state}/quickshell/notifications-snapshot.json`
- Side effects:
  - Writes snapshot file on save.

## `osd`

- OSD polling data for volume and brightness.
- Subcommands:
  - `volume`: returns current volume and mute state.
  - `brightness`: returns brightness percent using `brightnessctl`, `light`, or `brillo`.
- Side effects:
  - Read-only polling.

## `wifi`

- Query and control Wi-Fi via `nmcli`.
- Subcommands:
  - `state [--hover]`: default returns radio enabled/disabled state; with `--hover`, returns connected interfaces with IP/gateway and signal for hover menu UI.
  - `device-status`: `nmcli device status` style data.
  - `known-connections`: known saved connections.
  - `list`: scanned AP list with signal/security.
  - `active-info <device>`: active IPv4 details.
  - `connect <ssid> [password]`: connect to AP.
  - `disconnect <device>`: disconnect device.
  - `forget <ssid>`: delete connection profile.
  - `toggle <on|off>`: set Wi-Fi radio state.
- Side effects:
  - Connect/disconnect/forget/toggle mutate network state.

## `portal-save-file`

- Opens XDG desktop portal Save File flow and returns selected URI.
- Invocation:
  - `portal-save-file [title] [default_name]`
- Side effects:
  - Triggers user-facing portal dialog.

## `screenshot-menu`

- Capture/freeze/geometry helper for screenshot toolbar.
- Subcommands:
  - `capture [window|region|fullscreen]`: interactive or fullscreen capture to temp PNG.
  - `capture-geometry <x,y wxh> [mode]`: capture fixed geometry.
  - `capture-fullscreen [mode] [geometry] [output_name]`: monitor/geometry/fullscreen capture.
  - `freeze-frame [geometry] [monitor_key] [output_name]`: writes freeze image for overlay session.
  - `active-monitor`: returns active monitor name if known.
  - `window-at <x> <y>`: returns geometry of window under cursor point.
- Side effects:
  - Creates PNG files in runtime temp paths.
  - Calls compositor tools (`grim`, `slurp`, `hyprctl`).

## `screenshot-post`

- Post-capture actions for viewer workflows.
- Subcommands:
  - `copy <image_path>`: copy image to clipboard.
  - `save <image_path>`: save to `~/Pictures/Screenshots` with generated name.
  - `save-to <image_path> <target_path>`: save/copy to target path.
  - `save-as <image_path> <target_path>`: save/copy to target path.
  - `copy-text <text>`: copy OCR or selected text to clipboard.
- Side effects:
  - Writes files for save actions.
  - Writes clipboard contents for copy actions.

## JSON Contract Notes

- Success payloads include `"ok": true` and command-specific fields.
- Failures return `{"ok": false, "error": "..."}`.
- Help output is plain text for readability and does not use the JSON payload format.

## Runtime Invocation

Quickshell components invoke `stratum-cli` directly.

Examples:

- `stratum-cli audio ...`
- `stratum-cli notifications toggle`
- `stratum-cli powermenu toggle`
- `stratum-cli lockscreen lock`
- `stratum-cli screenshot start`
- `stratum-cli wifi ...`
- `stratum-cli bluetooth ...`
- `stratum-cli net check`
- `stratum-cli dashboard ...`
- `stratum-cli battery ...`
- `stratum-cli osd ...`
- `stratum-cli screenshot-menu ...`
- `stratum-cli screenshot-post ...`
- `stratum-cli portal-save-file ...`
- `stratum-cli notifications-snapshot ...`
