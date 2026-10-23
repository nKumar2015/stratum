# Stratum CLI Command Reference

The binary name is `stratum-cli`. Keybinds use it to reach the native shell,
and the shell itself runs its screenshot helpers through it.

## Help

- `stratum-cli`, `stratum-cli help`, `stratum-cli --help`: the command list.
- `stratum-cli help <command>`, `stratum-cli <command> --help`: one command.

Command responses are JSON on stdout. Help output is plain text.

## Shell IPC targets

`stratum-cli <target> <function> [args...]` sends one request to the shell's
socket, `$XDG_RUNTIME_DIR/stratum-shell.sock` (override with
`$STRATUM_SOCKET`). A function the shell will not serve answers `unsupported`
(only a lock it cannot safely take), and the CLI reports it.

| Target | Functions |
|---|---|
| `dashboard` | `open [tab]`, `close`, `toggle [tab]` |
| `lockscreen` | `lock` |
| `notifications` | `open`, `close`, `toggle`, `clear`, `toggleDnd` |
| `powermenu` | `toggle` |
| `screenshot` | `start` |
| `settings` | `open [page]`, `close`, `toggle [page]` |
| `theme` | `open`, `close`, `toggle`, `set <themeName>` |
| `todo` | `add <text>` |
| `timer` | `start [25m\|90s\|1h30m\|pomodoro\|stopwatch]`, `pause`, `toggle`, `stop` |

`stratum-cli theme list` also prints the built-in theme names without asking
the shell.

## `portal-save-file`

- Opens the XDG desktop portal's Save File dialog and returns the chosen URI.
- Invocation: `portal-save-file [title] [default_name]`

## `screenshot-post`

- Post-capture actions for the screenshot viewer.
- Subcommands:
  - `copy <image_path>`: copy the image to the clipboard.
  - `save <image_path>`: save to `~/Pictures/Screenshots` with a generated name.
  - `save-to <image_path> <target_path>`: copy to a target path.
  - `save-as <image_path> <target_path>`: copy to a target path.
  - `copy-text <text>`: copy text to the clipboard.
- Side effects: writes files for the save actions.
