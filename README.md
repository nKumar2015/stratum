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

The devshell provides the Vulkan loader; outside it the shell exits with "no
suitable Vulkan adapter" unless `LD_LIBRARY_PATH` points at `vulkan-loader`.

| Path | What it is |
|---|---|
| `tools/stratum_shell/` | the shell: Wayland surfaces, renderer, widgets, pages, overlays |
| `tools/stratum_sys/` | system managers: audio, Bluetooth, Wi-Fi, battery, media, metrics, weather, calendar, polkit |
| `tools/stratum_cli/` | `stratum-cli`, the socket client keybinds use |
| `module.nix` | the Home Manager module |

### Running a development build beside the installed shell

The installed shell owns the notification server
(`org.freedesktop.Notifications`), the tray watcher
(`org.kde.StatusNotifierWatcher`), the polkit agent and the IPC socket, and
only one process can hold each. Give the development build a config that
turns those off and a socket of its own:

```toml
# /tmp/dev-config.toml
[behavior]
tray = false
notifications = false
polkit = false
```

```sh
STRATUM_SOCKET=/run/user/$UID/stratum-dev.sock \
  STRATUM_CONFIG=/tmp/dev-config.toml \
  XDG_STATE_HOME=/tmp/stratum-dev-state \
  ./target/debug/stratum-shell &
STRATUM_SOCKET=/run/user/$UID/stratum-dev.sock stratum-cli dashboard toggle
```

`XDG_STATE_HOME` keeps the build away from your real state: a theme picked
live, notifications, the dashboard's to-dos and note, and the weather and
calendar caches all live there. Set `HOME` to a scratch directory as well
when testing the screenshot viewer's Save, which writes to
`~/Pictures/Screenshots`.

Stop the development build by its PID (`kill $!`). `pkill -x stratum-shell`
stops the installed one too, and systemd does not restart it after a
`SIGTERM`; `systemctl --user start stratum-shell` brings it back.

A devshell build renders on whichever GPU the devshell's Vulkan loader can
use, and it refuses to lock (see below). To try a change in your real session,
build against your system's nixpkgs, stop the service and run that binary:

```sh
nix build .#stratum-shell --no-write-lock-file \
  --override-input nixpkgs path:<the path `nix registry list` shows for system flake:nixpkgs>
systemctl --user stop stratum-shell
./result/bin/stratum-shell
# afterwards
systemctl --user start stratum-shell
```

### Testing notifications and the tray

Run the build on a private session bus, where it can own the notification
server and tray watcher without touching the real ones (leave `notifications`
and `tray` on in its config for this):

```sh
eval $(dbus-launch --sh-syntax)
STRATUM_SOCKET=/run/user/$UID/stratum-dev.sock \
  STRATUM_CONFIG=/tmp/dev-config.toml ./target/debug/stratum-shell &
notify-send -a App "Summary" "Body"
```

### Testing the lock screen

Never lock your real session to test: if the build cannot check your
password, you cannot get back in. Run a headless sway in its own runtime
directory and point the shell at it. Keep the directory short
(`/run/user/$UID/<name>`), since Wayland socket paths are limited to 108
bytes.

PAM needs a service you control. In a debug build, `STRATUM_PAM_CONFDIR`
names a directory to read PAM services from instead of `/etc/pam.d`; put a
`stratum-shell` service there whose `auth` and `account` lines use
`pam_exec` with a script that accepts one fixed test password. Take the
modules from the libpam the binary links (`ldd target/debug/stratum-shell`),
and use only shell builtins in the script, because `pam_exec` runs it with
an empty environment.

Type with `wtype`. Sway drops the first key of each new virtual keyboard, so
start with a delay (`wtype -s 300 …`) and give `-k Return` its own.

The shell refuses to lock, and `stratum-cli lockscreen lock` reports
`unsupported`, whenever PAM cannot work: when the service is missing from
`/etc/pam.d` (PAM would fall back to `other`, which denies everyone on
NixOS), or when one of its modules does not load. A devshell build hits the
second case against the real `/etc/pam.d` whenever the flake's nixpkgs is
older than the system's, because the system's PAM modules need a newer glibc.
That refusal is deliberate; build against the system's nixpkgs for a lock
that works.

### Testing the polkit dialog

polkit allows one agent per session, so leave `polkit = false` beside the
installed shell, and never send wrong passwords to your real account. Two
variables replace the real pieces:

- `STRATUM_POLKIT_FIXTURE_NAME=<name>` (debug builds only) serves the agent
  on the session bus under that name, without polkitd. On a private
  `dbus-launch` bus, call its `BeginAuthentication` (`sssa{ss}sa(sa{sv})`)
  and `CancelAuthentication` with `busctl --address`.
- `STRATUM_POLKIT_HELPER=<script>` replaces `polkit-agent-helper-1`. The
  script reads the cookie and password lines and exits 0 on a fixed test
  password.

`STRATUM_POLKIT_SUBJECT_PID` (debug builds only) registers with the real
polkitd for one process only; trigger it with `pkcheck --process <pid> --action-id … --allow-user-interaction`
(polkit 127's `pkexec` is not setuid).

### Clicking in headless sway

Headless sway has no pointer device, so the shell never gets a pointer and
`swaymsg seat … cursor` clicks go nowhere. Start a persistent
`zwlr_virtual_pointer_v1` client before the shell and drive it from there.
One-shot tools such as `wlrctl` attach a new device per command, which races
the shell binding its pointer.

## License

MIT; see [LICENSE](LICENSE).
