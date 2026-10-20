# Native shell parity checklist

The native Rust shell must preserve the **features, data, and interactions** of
the Quickshell implementation. Visual style is intentionally changing
(vectorheart), so this checklist tracks behaviour, not pixels.

Mark a row `verified` only after exercising it against real devices/services in
the running native shell — not after the code is written.

Status: `todo` · `wip` · `verified`

## Sidebar (per monitor)

| Surface | Kind | Behaviour to preserve | Status |
|---|---|---|---|
| Sidebar container | layer-shell panel, left edge, per monitor | Full-height, transparent margins, corner insets, `InvertedCorner` chrome top and bottom | todo |
| Workspaces | panel section | 10-workspace window around the active workspace, active/occupied/muted colours, click and scroll-wheel to switch, animated scroll to the active row | todo |
| App title | panel section | Active window title via Hyprland, app icon from the window class, marquee when the title overflows | todo |
| Tray | panel section | StatusNotifierItem icons, left-click activate, right-click opens the item's DBusMenu, hover-out auto-close | wip |
| Quick icons (audio / BT / wifi) | panel section | Rounded container, per-icon state colour, hover opens the corresponding hover menu on the focused monitor | todo |
| Battery | panel section | Percentage, charging/low/critical states and icons | todo |
| Clock | panel section | Time, updated each minute, aligned to the minute boundary | todo |

## Menus

| Surface | Kind | Behaviour to preserve | Status |
|---|---|---|---|
| Audio menu | popup overlay | Output/input device lists with selection, per-device volume slider, mute, EQ panel access, optimistic selection guard (pending value wins until confirmed or 5s) | todo |
| Audio hover menu | popup overlay | Compact volume + devices, lightweight fast path | todo |
| Equalizer panel | popup section | Parametric EQ response curve, draggable band points (drag changes gain; frequency fixed per band), numeric band fields, per-band enable, add/remove band, preamp, apply, reset-to-flat, preset list with builtin/custom distinction, save/delete preset | todo |
| Bluetooth menu | popup overlay | Adapter power/scan toggle, paired and discovered device lists, pair/connect/disconnect/forget/trust, device detail panel | todo |
| Bluetooth hover menu | popup overlay | Compact connect/disconnect for known devices | todo |
| Wi-Fi menu | popup overlay | Enable/disable radio, known networks with connect/disconnect/forget, scan list, password entry for secured networks, active connection detail | todo |
| Wi-Fi hover menu | popup overlay | Compact SSID/signal, quick connect | todo |
| Battery hover menu | popup overlay | Percentage, state, projected time, power-profile selection (low-power/balanced/performance) | todo |
| Dashboard menu | popup overlay | Calendar grid with month navigation and a ±3-month sliding window, music panel, performance panel; `watch`/`unwatch` lifecycle so the daemon only streams the visible month | todo |
| Theme switcher | fullscreen overlay, exclusive keyboard | Search filter, up/down/enter/escape navigation, applies theme immediately | todo |

## Overlays & notifications

| Surface | Kind | Behaviour to preserve | Status |
|---|---|---|---|
| System OSD | layer-shell overlay | Volume and brightness channels, icon by level, fade in/out, absolute volume baseline at 150% ceiling, poll while visible | todo |
| Notification listener | background service | Own `org.freedesktop.Notifications` server, snapshot persistence across restarts, identity-based replacement of same-app/summary notifications | todo |
| Notification center | popup overlay | Notification list, dismiss individual, clear all, do-not-disturb toggle, inline reply with action buttons | todo |
| Notification toasts | popup overlay | Appear per notification with app icon/name/summary/body, urgency styling, auto-expiry, click to dismiss, inline reply field | todo |

## Interactive / privileged

| Surface | Kind | Behaviour to preserve | Status |
|---|---|---|---|
| Lock screen | fullscreen overlay, exclusive keyboard | PAM authentication, password field, failure message and retry, clock, backdrop blur, only unlocks on success | todo |
| polkit dialog | popup overlay, exclusive keyboard | Shows action/message/icon, password field, submits via the agent, shows remaining attempts, closes on success/failure | todo |
| Power menu | popup overlay | Lock, hibernate, reboot, shutdown, Windows boot entry, firmware setup, logout | wip |

## Screenshots

| Surface | Kind | Behaviour to preserve | Status |
|---|---|---|---|
| Screenshot toolbar | layer-shell overlay, per monitor | Region/window/fullscreen modes, geometry readout, freeze-frame preview | todo |
| Screenshot viewer | normal xdg-shell window | Image load, zoom and pan, pen/colour/size annotation, undo, save, copy, upload, portal "save as", temp-file cleanup | todo |

## IPC & integration

| Surface | Kind | Behaviour to preserve | Status |
|---|---|---|---|
| Shell IPC | Unix socket | Targets `powermenu`, `theme`, `lockscreen`, `screenshot`, `dashboard`, `notifications` with the same function names `stratum-cli` already calls | wip |
| `stratum-cli` | CLI | Every subcommand keeps its output contract; `qs ipc` calls replaced by the shell socket | verified |
| External keybinds | scripts | Anything calling `stratum-cli` keeps working; anything calling `qs ipc` directly is documented as changed | verified |

**Shell IPC** — `$XDG_RUNTIME_DIR/stratum-shell.sock` (`$STRATUM_SOCKET`
overrides), one JSON request/response line per connection. All six targets and
their function names are parsed and validated. Implemented so far:
`notifications` (`open`, `close`, `toggle`, `clear`, `toggleDnd`),
`theme set`, and `powermenu toggle`. The rest answer `{"code":"unsupported"}`,
which `stratum-cli` uses to fall back to Quickshell while both shells run; that
fallback is exercised.

**`stratum-cli`** — verified end-to-end against the native shell: `notifications
open/toggle/clear/toggleDnd` and `theme set` were answered by the native pid,
and `theme open` / `powermenu toggle` transparently reached Quickshell.

**External keybinds** — `SUPER+T` (`theme open`) and `SUPER+N`
(`notifications toggle`) still work: the CLI contract and output JSON are
unchanged.

## Behaviours easy to miss

These are details the QML implements that a feature-level read of the surface
list would not reveal. Each one is a likely regression source.

- **Notification snapshot round-trip** — notifications survive a shell restart
  via `stratum-cli notifications-snapshot`, including their action maps.
- **Audio optimistic selection** — a device the user just picked stays selected
  for 5s even if the daemon's next snapshot disagrees; a matching snapshot
  clears the guard early.
- **Headphone detection heuristic** — output is treated as headphones when the
  sink name or description matches bluez/headphone/headset/buds/airpods/earbud.
- **Volume ceiling of 150%** — both the OSD and the audio menu use this, not 100%.
- **EQ graph drag semantics** — dragging a point changes gain only; frequency
  stays fixed per band. The curve previews the drag before it is committed.
- **Device-list change suppression** — device arrays are compared by
  name+description so an unchanged list does not tear down and rebuild the
  delegates (this caused visible flicker in the QML).
- **Hover-intent timers** — hover menus open after an intent delay and close on
  a short grace period, so the pointer may cross gaps without dismissing them.
- **Popup monitor routing** — menus open on the monitor that was focused when
  the interaction started, not whichever is focused when they render.
- **Calendar ±3-month window** — the dashboard keeps adjacent months warm so
  navigation is instant; only the centre month refreshes metrics on a timer.
- **Radio-busy handling** — Wi-Fi and Bluetooth operations can take seconds;
  the UI shows progress and remains responsive.
- **Screenshot temp files** — annotated exports are written to a temp path and
  cleaned up by the CLI after the chosen action completes.
