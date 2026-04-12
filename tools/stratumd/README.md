# stratumd

`stratumd` is the long-running user daemon for Stratum.

It has two responsibilities:

- Serve request/response JSON-RPC over a Unix socket at `$XDG_RUNTIME_DIR/stratumd.sock`.
- Push state snapshots into the running Quickshell instance via `qs ipc`.

## Runtime model

The shell now uses a push-first model:

- `audio`, `wifi`, `bluetooth`, and `music` are broadcast from daemon snapshot threads.
- `dashboard` uses an explicit watch lifecycle:
  - `dashboard.watch { year, month }` enables dashboard broadcasts for the selected month.
  - `dashboard.unwatch` disables dashboard broadcasts.

UI modules still keep one-shot bootstrap and fallback calls for recovery, but periodic timer polling is no longer the normal update path.

## JSON-RPC examples

Health check:

```sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"health.ping","params":{}}' \
  | nc -U -w 2 "$XDG_RUNTIME_DIR/stratumd.sock"
```

Watch dashboard month:

```sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"dashboard.watch","params":{"year":2026,"month":4}}' \
  | nc -U -w 2 "$XDG_RUNTIME_DIR/stratumd.sock"
```

Unwatch dashboard:

```sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"dashboard.unwatch","params":{}}' \
  | nc -U -w 2 "$XDG_RUNTIME_DIR/stratumd.sock"
```
