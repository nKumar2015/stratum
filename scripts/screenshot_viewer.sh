#!/bin/sh
set -u

error() {
    echo "__ERROR__|$1"
    exit 0
}

normalize_path() {
    value="${1:-}"
    [ -n "$value" ] || {
        echo ""
        return 0
    }

    case "$value" in
        file://*)
            value="${value#file://}"
            value="/${value#/}"
            ;;
    esac

    printf "%s\n" "$value"
}

require_file() {
    target_path="$(normalize_path "${1:-}")"
    if [ -z "$target_path" ]; then
        error "missing image path"
    fi
    if [ ! -f "$target_path" ]; then
        error "image file not found"
    fi
    printf "%s\n" "$target_path"
}

copy_to_clipboard() {
    image_path="$1"

    if command -v wl-copy >/dev/null 2>&1; then
        if wl-copy < "$image_path" >/dev/null 2>&1; then
            echo "ok|copy|$image_path"
            return 0
        fi
    fi

    if command -v xclip >/dev/null 2>&1; then
        if xclip -selection clipboard -t image/png -i "$image_path" >/dev/null 2>&1; then
            echo "ok|copy|$image_path"
            return 0
        fi
    fi

    error "no clipboard tool found (install wl-clipboard or xclip)"
}

save_only() {
    image_path="$1"
    target_dir="$HOME/Pictures/Screenshots"
    timestamp="$(date +%Y%m%d-%H%M%S)"
    target_path="$target_dir/Screenshot-$timestamp.png"

    mkdir -p "$target_dir" || error "failed to create screenshot directory"

    if ! cp -- "$image_path" "$target_path"; then
        error "failed to save screenshot"
    fi

    echo "ok|save|$target_path"
}

save_to_path() {
    image_path="$1"
    target_path_raw="$2"

    target_path="$(normalize_path "$target_path_raw")"
    [ -n "$target_path" ] || error "missing destination path"

    case "$target_path" in
        *.png) ;;
        *) target_path="$target_path.png" ;;
    esac

    target_parent="$(dirname "$target_path")"
    mkdir -p "$target_parent" || error "failed to create destination directory"

    if ! cp -- "$image_path" "$target_path"; then
        error "failed to save screenshot"
    fi

    echo "ok|save-as|$target_path"
}

action="${1:-}"
image_path="$(require_file "${2:-}")"
destination_path="${3:-}"

case "$action" in
    copy)
        copy_to_clipboard "$image_path"
        ;;
    save)
        save_only "$image_path"
        ;;
    save-to)
        save_to_path "$image_path" "$destination_path"
        ;;
    save-as)
        save_to_path "$image_path" "$destination_path"
        ;;
    *)
        error "unknown action"
        ;;
esac
