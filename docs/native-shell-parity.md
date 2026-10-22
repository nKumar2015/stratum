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
| App title | panel section | Active window title via Hyprland, app icon from the window class, elided when the title overflows | wip — each monitor shows its own workspace's most recently focused window (seen on Hyprland: HDMI showed kitty, eDP's empty workspace showed Desktop), with the app icon from its desktop entry or class, elided like QML's `ElideRight`. (This row used to say "marquee": the QML title never scrolled; `MarqueeLabel` is used by the device/network detail panels, the audio menu, and the dashboard's music panel.) |
| Tray | panel section | StatusNotifierItem icons, left-click activate, right-click opens the item's DBusMenu, hover-out auto-close | wip — icons render on both monitors, both as watcher owner and as a consumer of Quickshell's watcher. Right-click fetched and showed a real app's DBusMenu (vicinae), and the menu closes 150 ms after the pointer leaves both it and its icon (seen in headless sway). Entry activation and left/middle-click activation not yet exercised against a real app |
| Quick icons (audio / BT / wifi) | panel section | Rounded container, per-icon state colour, hover opens the corresponding hover menu on the focused monitor | wip — container and state colours; resting on an icon opens its menu beside it on that monitor (exercised in headless sway, not yet on Hyprland). Icon hit areas are now per output |
| Battery | panel section | Percentage, charging/low/critical states and icons | todo |
| Clock | panel section | Time, updated each minute, aligned to the minute boundary | todo |

## Menus

| Surface | Kind | Behaviour to preserve | Status |
|---|---|---|---|
| Audio menu | popup overlay | Output/input device lists with selection, per-device volume slider, mute, EQ panel access, optimistic selection guard (pending value wins until confirmed or 5s) | todo |
| Audio hover menu | popup overlay | Compact volume + devices, lightweight fast path | wip — volume slider (0–150%), output and input lists with optimistic selection. Opening and layout seen in headless sway (no PipeWire there, so lists were empty); device clicks not exercised live |
| Equalizer panel | popup section | Parametric EQ response curve, draggable band points (drag changes gain; frequency fixed per band), numeric band fields, per-band enable, add/remove band, preamp, apply, reset-to-flat, preset list with builtin/custom distinction, save/delete preset | wip — the Equalizer page of the settings window, on the current output. Curve with QML's visual model; dragging a point previews and commits gain only (seen in headless sway against the real output). Band fields are sliders with readouts, not typed numbers. Filter type, enable, add/remove, preamp, presets (built-ins marked), Save as (name field; open/type/cancel seen live) and Delete (custom only). Apply, Reset, preset apply, Save and Delete not clicked live: they change the real audio and EQ config |
| Bluetooth menu | popup overlay | Adapter power/scan toggle, paired and discovered device lists, pair/connect/disconnect/forget/trust, device detail panel | wip — the Bluetooth page of the settings window, laid out like the QML menu: adapter switch (scans after turning on), connected-device card with disconnect, refresh/scan/count/loading, every known device including discovered ones (connected first), and the selected device's panel with its scrolling name (`MarqueeLabel`), Pair/Remove, Connect/Disconnect, Trust/Untrust, plus the QML status messages. Real adapter's devices, selection and the panel seen in headless sway; the actions, the switch and Scan were not clicked (they act on real devices). BlueZ is now reached over zbus; its snapshots matched bluer's exactly |
| Bluetooth hover menu | popup overlay | Compact connect/disconnect for known devices | wip — up to six paired devices with connect/disconnect, "+N more". Seen with real devices; connect not clicked in tests (it acts on the real adapter) |
| Wi-Fi menu | popup overlay | Enable/disable radio, known networks with connect/disconnect/forget, scan list, password entry for secured networks, active connection detail | todo |
| Wi-Fi hover menu | popup overlay | Compact SSID/signal, quick connect | wip — active connections with IP, gateway, device, signal and bars, as the QML menu showed (it had no connect action either). Seen with real connections |
| Battery hover menu | popup overlay | Percentage, state, projected time, power-profile selection (low-power/balanced/performance) | wip — state and percentage, screen-on time, projected time or status, rate, Saver/Normal/Performance. Seen with real data; profile clicks not exercised (they write the real platform profile) |
| Dashboard menu | popup overlay | Calendar grid with month navigation and a ±3-month sliding window, music panel, performance panel; `watch`/`unwatch` lifecycle so the daemon only streams the visible month | wip — `dashboard open/close/toggle` over IPC; performance (CPU, GPU + source, RAM, storage, load-coloured bars), calendar («« « Today » »», arrows too, today marked, resets to this month on close), now playing (art, title, artist, album, position, progress, controls; "No active player" without one). Polled every 2 s only while open. The calendar is computed locally, so navigation is instant without the ±3-month cache. Seen in headless sway with real data; media controls not clicked live; the music marquee is not done |
| Theme switcher | fullscreen overlay, exclusive keyboard | Search filter, up/down/enter/escape navigation, applies theme immediately | wip — `theme open/close/toggle` over IPC; search, wrapping arrows, Return, Escape, click outside to close, hover to select, click to apply. All exercised in headless sway (keyboard via wtype, pointer via a virtual pointer); not yet on Hyprland. The list scrolls past `max_rows`, which the QML list did not |

## Overlays & notifications

| Surface | Kind | Behaviour to preserve | Status |
|---|---|---|---|
| System OSD | layer-shell overlay | Volume and brightness channels, icon by level, fade in/out, absolute volume baseline at 150% ceiling, poll while visible | todo |
| Notification listener | background service | Own `org.freedesktop.Notifications` server, snapshot persistence across restarts, identity-based replacement of same-app/summary notifications | wip — server, actions, inline reply. Persistence: list (actions included), do-not-disturb, and next id saved to `$XDG_STATE_HOME/stratum/notifications.json` 700 ms after a change and restored on start without re-toasting; the Quickshell snapshot is imported once when there is no native one. Restart round trip exercised in headless sway. Replacement: `replaces_id` (or the `replaces-id`/`x-kde-replaces-id` hint) updates in place; otherwise the same app and summary supersedes the earlier notification with a new id (so it toasts again) and closes the old one. Replacement covered by tests, not exercised live |
| Notification center | popup overlay | Notification list, dismiss individual, clear all, do-not-disturb toggle, inline reply with action buttons | wip — the Notifications page of the settings window: list, dismiss, clear all, DND over IPC, action buttons, default action on click, and the last reply (as the QML centre showed, no field). Page actions implemented but not yet clicked in a running shell |
| Notification toasts | popup overlay | Appear per notification with app icon/name/summary/body, urgency styling, auto-expiry, click to dismiss, inline reply field | wip — name/summary/body (markup flattened), urgency stripe, expiry paused while hovered or replying, action buttons, default action on click (dismiss when there is none), inline reply. Exercised in headless sway with notify-send and gdbus: `ActionInvoked`, `NotificationReplied`, one `NotificationClosed` each. Picture beside the header (`image-data`, then `image-path`, then the app icon), urgency glyph without one; app icon seen live, `image-data` covered by tests only. PNG and SVG images only (no JPEG decoder) |

## Interactive / privileged

| Surface | Kind | Behaviour to preserve | Status |
|---|---|---|---|
| Lock screen | fullscreen overlay, exclusive keyboard | PAM authentication, password field, failure message and retry, clock, backdrop blur, only unlocks on success | wip — ext-session-lock + PAM implemented; lock, wrong-password retry, unlock, and relock exercised end to end in headless sway against a test PAM service. Not yet exercised on the real session: a devshell build cannot load the system's PAM modules (glibc mismatch) and correctly refuses to lock |
| polkit dialog | popup overlay, exclusive keyboard | Shows action/message/icon, password field, submits via the agent, shows remaining attempts, closes on success/failure | wip — behind `[behavior] polkit` (stratumd holds the session agent). Exercised against a fixture: agent on a private bus, requests sent with busctl, a stand-in checker. Wrong password + retry, success, Escape (`Cancelled`), three failures, and polkitd's cancel all pass. Not yet exercised with real polkitd and the real helper |
| Power menu | popup overlay | Lock, hibernate, reboot, shutdown, Windows boot entry, firmware setup, logout | wip |

## Screenshots

| Surface | Kind | Behaviour to preserve | Status |
|---|---|---|---|
| Screenshot toolbar | layer-shell overlay, per monitor | Region/window/fullscreen modes, geometry readout, freeze-frame preview | wip — `screenshot start` over IPC covers every output with its frozen frame (`grim -t ppm`, straight into a texture). Hover tints the window under the pointer (from the shell's Hyprland state, most recently focused on overlap) or the whole output; click captures it (window + 5 px) or the output; a drag past 8 px captures the region. Crosshair, crosshair cursor, size readout, Escape. Capture hides the overlay, waits 120 ms, runs `grim`; failures notify and return to picking. Region, output click and Escape exercised in headless sway (captures matched the live screen, no overlay in them); window picking needs Hyprland and is not yet seen |
| Screenshot viewer | normal xdg-shell window | Image load, zoom and pan, pen/colour/size annotation, undo, save, copy, portal "save as", temp-file cleanup | wip — opens with each capture: mode, clear/copy/save/save-as/close (busy glyphs, disabled while running), pen colours (`[screenshot] pen_colors`) and size, zoom slider and Ctrl+wheel about the pointer, middle or Ctrl drag to pan, Ctrl+Z, Escape. Strokes are composed at full resolution into a temp file for the action and removed after; the capture itself is removed on close (QML left it in the runtime dir). Draw, save (annotated PNG, full size), undo, zoom and Escape exercised in headless sway; copy and portal Save As not yet run live. (This row used to list "upload": the QML viewer had none.) |

## IPC & integration

| Surface | Kind | Behaviour to preserve | Status |
|---|---|---|---|
| Shell IPC | Unix socket | Targets `powermenu`, `theme`, `lockscreen`, `screenshot`, `dashboard`, `notifications` with the same function names `stratum-cli` already calls | wip — every QML target and function is served natively; only a lock the shell cannot safely take still answers `unsupported` |
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
