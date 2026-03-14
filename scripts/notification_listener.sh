#!/usr/bin/env bash
set -u

STATE_DIR="${XDG_STATE_HOME:-$HOME/.local/state}/quickshell"
SNAPSHOT_FILE="$STATE_DIR/notifications-snapshot.json"

ensure_state_dir() {
    mkdir -p "$STATE_DIR"
    touch "$SNAPSHOT_FILE"
}

decode_urlencoded() {
    local encoded="$1"
    encoded="${encoded//+/ }"
    printf '%b' "${encoded//%/\\x}"
}

save_snapshot() {
    local encoded="$1"
    ensure_state_dir
    if [ -z "$encoded" ]; then
        exit 0
    fi

    local decoded
    decoded="$(decode_urlencoded "$encoded")"
    if [ -z "$decoded" ]; then
        exit 0
    fi

    printf '%s\n' "$decoded" > "$SNAPSHOT_FILE"
}

load_snapshot() {
    ensure_state_dir
    if [ ! -s "$SNAPSHOT_FILE" ]; then
        exit 0
    fi

    cat "$SNAPSHOT_FILE"
}

cmd="${1:-}" 

case "$cmd" in
    snapshot-save)
        save_snapshot "${2:-}"
        ;;
    snapshot-load)
        load_snapshot
        ;;
    *)
        cat >&2 <<'USAGE'
Usage:
  notification_listener.sh snapshot-save '<urlencoded-json>'
  notification_listener.sh snapshot-load
USAGE
        exit 1
        ;;
esac
