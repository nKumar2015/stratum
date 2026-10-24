# Stratum

A desktop shell for [Hyprland](https://hyprland.org), written from scratch in
Rust. It draws its own bar, control centre, notifications, lock screen,
dashboard and screenshot tools with its own renderer and keeps the 
footprint small: one 14 MB binary, where the Quickshell setup it
replaced pulled in a closure of about 1.1 GiB.

## What it does

- **Bar** — a rail on any screen edge with workspaces, the focused window's
  title and icon, the system tray, quick status icons (audio, Bluetooth,
  Wi-Fi), battery and clock. Resting on a status icon opens a small menu.
- **Control centre** — a settings window with pages for an overview, audio
  (devices, volumes, now playing, a parametric equalizer), Bluetooth, Wi-Fi,
  battery and power profile, notifications, display and themes, and power.
- **Notifications** — the session's notification server: toasts with actions
  and inline replies, do-not-disturb, and a history that survives restarts.
- **On-screen display** for volume and brightness.
- **Lock screen** using `ext-session-lock-v1`, checking the password through
  PAM. It refuses to lock rather than lock you out when PAM cannot work.
- **polkit agent** with its own password dialog.
- **Dashboard** — a fullscreen overlay with tabs: today (weather, agenda, now
  playing), a calendar fed from ICS addresses, system graphs (CPU, memory,
  disks, GPUs), and a focus tab with to-dos, a note and a timer.
- **Screenshots** — region, window or screen, then a viewer to draw on the
  capture and copy, save, or save as.
- **Power menu** and **theme switcher** overlays.
- **Live configuration** — edit the config file and the shell reloads it;
  themes switch without a restart.

## Install

Stratum is a Nix flake with a Home Manager module for the shell and a NixOS
module for what needs the system.

```nix
# flake.nix
inputs.stratum = {
  url = "github:nKumar2015/stratum";
  # Needed: the lock screen loads the system's PAM modules, which must
  # match the glibc the shell is built against.
  inputs.nixpkgs.follows = "nixpkgs";
};
```

```nix
# NixOS configuration: the PAM service the lock screen and polkit dialog use
imports = [ inputs.stratum.nixosModules.default ];

# Home Manager configuration
imports = [ inputs.stratum.homeManagerModules.default ];
programs.stratum = {
  enable = true;
  installPortal = true; # the GTK file chooser, for "Save as" in the viewer
  settings = {
    theme = "gruvbox";
    sidebar.position = "right";
  };
};
```

This installs `stratum-shell` and `stratum-cli`, the programs the shell runs
(grim, wl-clipboard, playerctl, brightnessctl, upower, NetworkManager's nmcli,
curl, pciutils, PipeWire tools), and a `stratum-shell` user service started
with the graphical session:

```sh
systemctl --user status stratum-shell
journalctl --user -u stratum-shell
```

You also need a Vulkan driver, a [Nerd Font](https://www.nerdfonts.com) for
the icons, and polkit's `polkit-agent-helper-1` setuid wrapper (NixOS
provides it with `security.polkit.enable`).

## Keybinds

The shell takes no global shortcuts itself; bind keys in Hyprland to
`stratum-cli`, which talks to the running shell over a Unix socket:

```sh
stratum-cli powermenu toggle
stratum-cli lockscreen lock
stratum-cli notifications toggle      # the notification page
stratum-cli notifications toggleDnd
stratum-cli screenshot start
stratum-cli dashboard toggle [tab]
stratum-cli settings toggle [page]
stratum-cli theme open                # the theme switcher
stratum-cli theme set gruvbox
stratum-cli todo add "buy milk"
stratum-cli timer start 25m
```

`stratum-cli help` lists every target and function.

## Configure

Every option, with its default and a comment, is in
[`tools/stratum_shell/config.default.toml`](tools/stratum_shell/config.default.toml).
Your file only needs what you change; it is merged over the defaults. With
Home Manager, set `programs.stratum.settings`; without it, write
`~/.config/stratum/config.toml` (or point `$STRATUM_CONFIG` at a file).

- `[themes.*]` holds colours and `[metrics.*]` every size, gap, radius and
  timing, so restyling never means editing code. Four themes are built in:
  carbon, gruvbox, rosepine and tokyonight.
- `[[sidebar.widgets]]` lists the bar's widgets in order.
- `[[dashboard.tabs]]` lays out the dashboard's panels.
- Unknown keys are an error, so a typo is reported instead of ignored. A
  config that fails to load leaves the running shell on its last good one.

## Develop

```sh
nix develop
cargo build -p stratum-shell
cargo test --workspace
cargo clippy --workspace --all-targets
./target/debug/stratum-shell
```

A development build can run beside the installed shell; see
[CLAUDE.md](CLAUDE.md) for how, and for testing the lock screen, polkit and
notifications without touching your real session.

| Path | What it is |
|---|---|
| `tools/stratum_shell/` | the shell: Wayland surfaces, renderer, widgets, pages, overlays |
| `tools/stratum_sys/` | system managers: audio, Bluetooth, Wi-Fi, battery, media, metrics, weather, calendar, polkit |
| `tools/stratum_cli/` | `stratum-cli`, the socket client keybinds use |
| `module.nix` | the Home Manager module |
| `docs/native-shell-parity.md` | the behaviour checklist kept from the move off Quickshell |

## License

MIT; see [LICENSE](LICENSE).
