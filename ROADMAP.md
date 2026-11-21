# Roadmap

What is left of the plan made on 2026-11-19 to bring Stratum closer to a
full desktop while staying light. The original plan, with a memory cost
for every item, is `~/.claude/plans/what-other-features-could-flickering-hennessy.md`;
this file is its current state. The file manager's own next round is
`tools/stratum_files/PLAN.md`.

Rules from the user that apply to every item:
- Give the expected memory cost (idle and while open) before building.
- Verify that a tool the shell wraps actually works on this machine first
  (cliphist turned out never to have been installed).
- Never remove an app a feature replaces: both stay installed while the
  user compares them over time.
- Ask before pushing; commit messages say why; no Co-Authored-By trailer.

## Done (in order)

| Item | Commit(s) | Replaces (still installed) |
|---|---|---|
| Idle manager, lock before sleep | early in the round | — |
| Devshell on the system's nixpkgs, `doCheck = false` | early | — |
| Key repeat | early | — |
| Launcher: apps, windows, commands, calculator | `189ab9c` | vicinae |
| Emoji picker (grid), clipboard history (own, list + preview) | `9501b9c` `531d308` `aa67cb9` | vicinae, cliphist (never installed) |
| Night light; idle settings on the Battery page | `57fae80` | — |
| Per-app mixer; wired, VPN, hotspot, traffic on the Network page | `36d2b00` | pavucontrol, blueman |
| `stratum_ui` crate; file manager and FileChooser portal | `5d18075` `79ac476` | thunar |
| File manager's next round: long paths, other-drive trash, undo, preview, thumbnails, drag and drop, Hyprland portal fix, loaf-fm look, bookmarks | `81ab92c` … `b9e29a0` (pushed 2026-11-20) | — |
| Workspace overview (5) and window switcher (6), over `hyprland-toplevel-export-v1` | `6d14f17` (pushed 2026-11-20); empty workspaces hidden from the overview afterwards | — |
| Per-app outputs through an EQ for each device (a follow-up to the mixer) | `9a44aae` | pavucontrol |
| Removable drives (7): udisks2, a bar icon and menu, toasts with Open and Eject, unmounted drives and Unmount/Eject in the file manager, a passphrase prompt for LUKS drives | committed 2026-11-20 | udiskie-style helpers (none installed) |
| Battery menu revamp: stay awake and night light moved into it, the power mode as a draggable tachometer (asked for 2026-11-20) | `fb10c9b` | — |

On the user's side: the file manager is enabled in `~/nixos-config`;
`nix flake update stratum` and a rebuild bring in everything pushed,
after which the FileChooser check (PLAN.md, item 9) can be done.
Removing vicinae, pavucontrol, blueman or thunar is the user's call, after
comparing.

Measured for 5 and 6 (release build, nested Hyprland at 2476x1396, five
windows of about 2400x1350): binary +298 KB (16.30 → 16.59 MB); idle before
first use ~0 (+1 MB RSS, within noise; no VRAM); overview open +22 MB RSS
(the capture buffer, as large as the largest window, and the pictures) and
+77 MB VRAM (the full-screen swapchain), 6 % of a core in the shell and
3.5 % in Hyprland while it retakes each window every second; after closing
+2 MB RSS and +4 MB VRAM, constant over repeated opens (wgpu keeps the
wallpaper's upload buffer; without `[overview] wallpaper` it is 0). The
overview shows only workspaces holding a window, and the one on screen
even when empty (the user's 1–20 are persistent, so most were empty
cards). The keys are bound Windows-style in `~/nixos-config`'s
`modules/core/hyprland.nix`: `ALT + Tab` / `ALT + SHIFT + Tab` →
`stratum-cli switcher next|prev`, `SUPER + Tab` and a four-finger swipe up
→ `overview toggle` (passes Hyprland's `--verify-config`; not yet tried on
the real keyboard and touchpad).

## Left, Tier 2: shell features

Estimated costs from the original plan; measure before committing
(the memory profiling recipe is in Claude's memory).

| # | Feature | Idle | While open / binary |
|---|---|---|---|
| 10 | Privacy indicators: a bar dot while a mic or camera is in use (PipeWire stream state; the mixer's `recording` list already has the mic part) or the screen is shared (portal ScreenCast sessions) | ~0 | ~+10 KB |
| 11 | Do-not-disturb schedules and per-app notification rules (mute, no toast, keep in history), on the Notifications page | ~0 | ~+15 KB |
| 12 | Screen recording: a "record region" mode in the screenshot overlay running `wf-recorder` (or the portal), a stop button and timer in the bar | 0 | wf-recorder 30–60 MB while recording; ~+20 KB |
| 13 | Colour picker: a screenshot-overlay mode with a magnifier, copies hex/rgb | 0 | a backdrop texture per output (~15 MB at 1440p) while open; ~+15 KB |
| 14 | On-screen keyboard, touch gestures: only if the laptop is used as a tablet | — | — |
| 15 | Polish: notifications grouped by app; keyboard-layout and Caps Lock widgets; a calendar popup on the clock (reuse `dashboard/agenda.rs`); a weather widget; per-output bar layouts | 0 | a few MB VRAM per popup; ~+10 KB each |

(5, 6, 7, 8 and 9 of Tier 2, the overview, the switcher, removable
drives, network extras and the mixer, are done.)

Removable drives needed `services.udisks2.enable = true` in
`~/nixos-config` (added 2026-11-20, not yet rebuilt). udisksd is D-Bus
activated and stays up once started (~10–15 MB, system-wide), so the shell
waits for a removable disk in `/dev` before asking it anything; the file
manager starts it when it lists drives. Tested against a stand-in udisks
only: not yet with a real USB stick, polkit or an unlocked LUKS drive.
Not built: `stratum-cli` commands for drives.

Measured for 7 before the passphrase prompt (release builds against `9a44aae`): shell +385 KB
(17.21 → 17.59 MB), file manager +454 KB (12.54 → 12.99 MB), mostly zbus's
blocking calls and zvariant's (de)serialisers, far over the +40 KB
estimated; using plain `call_method` instead of `Proxy` took ~200 KB off
the file manager. Idle, with no removable drive and udisks not running:
RSS unchanged within noise (66.55 vs 66.52 MB), two more sleeping threads.

Also from this round:
- Network locations in the file manager: put off by the user
  (2026-11-19); notes in `tools/stratum_files/PLAN.md`.

## Left, Tier 3: separate apps

Each is its own binary on `stratum_ui::app`, packaged as the file manager
is (an output of the shell's derivation, enabled by a Home Manager
option), since that is the pattern the user chose.

| App | Replaces | Notes |
|---|---|---|
| Image viewer | — | mostly the screenshot viewer already: zoom, pan, the folder's other pictures, "set as wallpaper" through `wallpapers.rs`; the file manager's "Open with" would list it |
| Greeter (`stratum-greet`) | tuigreet | a greetd client (its JSON IPC), the lock screen's look, under `cage` or as Hyprland's first client; runs before login, 0 during the session |
| System monitor / task manager | btop in kitty | the process list the user dropped from the dashboard, with sort and kill; ~35–40 MB while open |
| Text editor / notes | — | low priority; needs selection and IME first |
| Terminal | kitty | not recommended |

## Suggested next

Privacy indicators (10): cheap, and the mixer already sees recording
streams. Then DND schedules and per-app notification rules (11).
