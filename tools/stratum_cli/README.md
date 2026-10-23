# Stratum CLI Command Reference

The binary name is `stratum-cli`. Keybinds use it to reach the native shell.

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

