#!/bin/sh
set -u

SLURP_BACKGROUND="#00000066"
SLURP_BORDER="#7aa2f7ff"
SLURP_SELECTION="#7aa2f744"
SLURP_BOX="#101520dd"
SLURP_BORDER_WIDTH="2"

error() {
    echo "__ERROR__|$1"
    exit 0
}

# Intentionally do not pass '-c' so the cursor is never included.
grim_capture() {
    grim "$@"
}

require_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        error "$1 not found"
    fi
}

make_temp_output() {
    local runtime_dir
    runtime_dir="${XDG_RUNTIME_DIR:-/tmp}"
    mktemp "$runtime_dir/quickshell-screenshot-XXXXXX.png" 2>/dev/null || mktemp "/tmp/quickshell-screenshot-XXXXXX.png"
}

freeze_output_path() {
    local runtime_dir
    local monitor_key
    runtime_dir="${XDG_RUNTIME_DIR:-/tmp}"
    monitor_key="${1:-default}"
    monitor_key="$(printf "%s" "$monitor_key" | tr -c 'A-Za-z0-9._-' '_' )"
    [ -n "$monitor_key" ] || monitor_key="default"
    printf "%s/quickshell-screenshot-freeze-%s.png\n" "$runtime_dir" "$monitor_key"
}

resolve_window_geometry_at() {
    px="$1"
    py="$2"

    if ! command -v hyprctl >/dev/null 2>&1; then
        return 1
    fi
    if ! command -v jq >/dev/null 2>&1; then
        return 1
    fi

    hyprctl -j clients 2>/dev/null | jq -r --argjson px "$px" --argjson py "$py" '
        map(
            select(.mapped == true)
            | select(.hidden != true)
            | select((.size[0] // 0) > 0 and (.size[1] // 0) > 0)
            | . + {
                x: (.at[0] // 0),
                y: (.at[1] // 0),
                w: (.size[0] // 0),
                h: (.size[1] // 0),
                z: (.focusHistoryID // 999999)
            }
            | select($px >= .x and $px < (.x + .w) and $py >= .y and $py < (.y + .h))
        )
        | sort_by(.z)
        | .[0]
        | if . == null then "" else "\(.x),\(.y) \(.w)x\(.h)" end
    '
}

window_boxes_hyprland() {
    if ! command -v hyprctl >/dev/null 2>&1; then
        return 1
    fi
    if ! command -v jq >/dev/null 2>&1; then
        return 1
    fi

    hyprctl -j clients 2>/dev/null | jq -r '
        .[]
        | select(.mapped == true)
        | select(.hidden != true)
        | select((.size[0] // 0) > 0 and (.size[1] // 0) > 0)
        | "\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1]) \((.title // .class // "Window") | gsub("\\n"; " "))"
    '
}

active_monitor_name() {
    if ! command -v hyprctl >/dev/null 2>&1; then
        return 1
    fi
    if ! command -v jq >/dev/null 2>&1; then
        return 1
    fi

    monitor_name="$(hyprctl -j activeworkspace 2>/dev/null | jq -r '.monitor // empty')"
    if [ -n "$monitor_name" ]; then
        printf "%s\n" "$monitor_name"
        return 0
    fi

    monitor_name="$(hyprctl -j monitors 2>/dev/null | jq -r 'map(select(.focused == true)) | .[0].name // empty')"
    if [ -n "$monitor_name" ]; then
        printf "%s\n" "$monitor_name"
        return 0
    fi

    return 1
}

pick_geometry() {
    mode="$1"

    case "$mode" in
        fullscreen)
            echo ""
            return 0
            ;;
        region)
            require_tool slurp
            slurp \
                -b "$SLURP_BACKGROUND" \
                -c "$SLURP_BORDER" \
                -s "$SLURP_SELECTION" \
                -B "$SLURP_BOX" \
                -w "$SLURP_BORDER_WIDTH" \
                -f "%x,%y %wx%h"
            return $?
            ;;
        window)
            require_tool slurp
            if boxes="$(window_boxes_hyprland)" && [ -n "$boxes" ]; then
                printf "%s\n" "$boxes" | slurp \
                    -r \
                    -b "$SLURP_BACKGROUND" \
                    -c "$SLURP_BORDER" \
                    -s "$SLURP_SELECTION" \
                    -B "$SLURP_BOX" \
                    -w "$SLURP_BORDER_WIDTH" \
                    -f "%x,%y %wx%h"
                return $?
            fi
            slurp \
                -b "$SLURP_BACKGROUND" \
                -c "$SLURP_BORDER" \
                -s "$SLURP_SELECTION" \
                -B "$SLURP_BOX" \
                -w "$SLURP_BORDER_WIDTH" \
                -f "%x,%y %wx%h"
            return $?
            ;;
        *)
            error "unknown mode: $mode"
            ;;
    esac
}

capture() {
    mode="${1:-window}"
    require_tool grim

    out_file="$(make_temp_output)"
    [ -n "$out_file" ] || error "failed to allocate output file"

    geom="$(pick_geometry "$mode")"
    geom_status=$?
    if [ "$geom_status" -ne 0 ]; then
        rm -f "$out_file"
        error "selection cancelled"
    fi

    if [ "$mode" = "fullscreen" ]; then
        if ! grim_capture "$out_file" >/dev/null 2>&1; then
            rm -f "$out_file"
            error "grim capture failed"
        fi
    else
        if [ -z "$geom" ]; then
            rm -f "$out_file"
            error "selection cancelled"
        fi
        if ! grim_capture -g "$geom" "$out_file" >/dev/null 2>&1; then
            rm -f "$out_file"
            error "grim capture failed"
        fi
    fi

    if [ ! -s "$out_file" ]; then
        rm -f "$out_file"
        error "capture produced empty file"
    fi

    ts="$(date +%s)"
    echo "ok|$out_file|$mode|$ts"
}

capture_geometry() {
    geom="${1:-}"
    mode="${2:-region}"
    require_tool grim

    [ -n "$geom" ] || error "missing geometry"

    out_file="$(make_temp_output)"
    [ -n "$out_file" ] || error "failed to allocate output file"

    if ! grim_capture -g "$geom" "$out_file" >/dev/null 2>&1; then
        rm -f "$out_file"
        error "grim capture failed"
    fi

    if [ ! -s "$out_file" ]; then
        rm -f "$out_file"
        error "capture produced empty file"
    fi

    ts="$(date +%s)"
    echo "ok|$out_file|$mode|$ts"
}

capture_fullscreen() {
    mode="${1:-fullscreen}"
    geometry="${2:-}"
    output_name="${3:-}"
    require_tool grim

    out_file="$(make_temp_output)"
    [ -n "$out_file" ] || error "failed to allocate output file"

    if [ -n "$output_name" ]; then
        if ! grim_capture -o "$output_name" "$out_file" >/dev/null 2>&1; then
            rm -f "$out_file"
            error "grim capture failed"
        fi
    elif [ -n "$geometry" ]; then
        if ! grim_capture -g "$geometry" "$out_file" >/dev/null 2>&1; then
            rm -f "$out_file"
            error "grim capture failed"
        fi
    elif ! grim_capture "$out_file" >/dev/null 2>&1; then
        rm -f "$out_file"
        error "grim capture failed"
    fi

    if [ ! -s "$out_file" ]; then
        rm -f "$out_file"
        error "capture produced empty file"
    fi

    ts="$(date +%s)"
    echo "ok|$out_file|$mode|$ts"
}

freeze_frame() {
    geometry="${1:-}"
    monitor_key="${2:-default}"
    output_name="${3:-}"
    require_tool grim

    out_file="$(freeze_output_path "$monitor_key")"
    if [ -n "$output_name" ]; then
        if ! grim_capture -o "$output_name" "$out_file" >/dev/null 2>&1; then
            error "failed to freeze screen"
        fi
    elif [ -n "$geometry" ]; then
        if ! grim_capture -g "$geometry" "$out_file" >/dev/null 2>&1; then
            error "failed to freeze screen"
        fi
    elif ! grim_capture "$out_file" >/dev/null 2>&1; then
        error "failed to freeze screen"
    fi

    if [ ! -s "$out_file" ]; then
        error "freeze image is empty"
    fi

    echo "ok|$out_file"
}

case "${1:-}" in
    capture)
        capture "${2:-window}"
        ;;
    capture-geometry)
        capture_geometry "${2:-}" "${3:-region}"
        ;;
    capture-fullscreen)
        capture_fullscreen "${2:-fullscreen}" "${3:-}" "${4:-}"
        ;;
    freeze-frame)
        freeze_frame "${2:-}" "${3:-}" "${4:-}"
        ;;
    active-monitor)
        if monitor_name="$(active_monitor_name)" && [ -n "$monitor_name" ]; then
            echo "ok|$monitor_name"
        else
            echo "none"
        fi
        ;;
    window-at)
        if geom="$(resolve_window_geometry_at "${2:-0}" "${3:-0}")" && [ -n "$geom" ]; then
            echo "ok|$geom"
        else
            echo "none"
        fi
        ;;
    *)
        error "unknown command"
        ;;
esac
