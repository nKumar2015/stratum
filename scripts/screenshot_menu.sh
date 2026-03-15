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
        if ! grim "$out_file" >/dev/null 2>&1; then
            rm -f "$out_file"
            error "grim capture failed"
        fi
    else
        if [ -z "$geom" ]; then
            rm -f "$out_file"
            error "selection cancelled"
        fi
        if ! grim -g "$geom" "$out_file" >/dev/null 2>&1; then
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

    if ! grim -g "$geom" "$out_file" >/dev/null 2>&1; then
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
    require_tool grim

    out_file="$(make_temp_output)"
    [ -n "$out_file" ] || error "failed to allocate output file"

    if ! grim "$out_file" >/dev/null 2>&1; then
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

case "${1:-}" in
    capture)
        capture "${2:-window}"
        ;;
    capture-geometry)
        capture_geometry "${2:-}" "${3:-region}"
        ;;
    capture-fullscreen)
        capture_fullscreen "${2:-fullscreen}"
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
