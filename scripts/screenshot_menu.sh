#!/bin/sh
set -u

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
            slurp -f "%x,%y %wx%h"
            return $?
            ;;
        window)
            require_tool slurp
            if boxes="$(window_boxes_hyprland)" && [ -n "$boxes" ]; then
                printf "%s\n" "$boxes" | slurp -r -f "%x,%y %wx%h"
                return $?
            fi
            slurp -f "%x,%y %wx%h"
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

case "${1:-}" in
    capture)
        capture "${2:-window}"
        ;;
    *)
        error "unknown command"
        ;;
esac
