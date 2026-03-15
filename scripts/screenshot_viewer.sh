#!/bin/sh
set -u

error() {
    echo "__ERROR__|$1"
    exit 0
}

require_file() {
    if [ -z "${1:-}" ]; then
        error "missing image path"
    fi
    if [ ! -f "$1" ]; then
        error "image file not found"
    fi
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

save_and_copy() {
    image_path="$1"
    target_dir="$HOME/Pictures/Screenshots"
    timestamp="$(date +%Y%m%d-%H%M%S)"
    target_path="$target_dir/Screenshot-$timestamp.png"

    mkdir -p "$target_dir" || error "failed to create screenshot directory"

    if ! cp -- "$image_path" "$target_path"; then
        error "failed to save screenshot"
    fi

    if command -v wl-copy >/dev/null 2>&1; then
        if wl-copy < "$target_path" >/dev/null 2>&1; then
            echo "ok|save-copy|$target_path"
            return 0
        fi
    fi

    if command -v xclip >/dev/null 2>&1; then
        if xclip -selection clipboard -t image/png -i "$target_path" >/dev/null 2>&1; then
            echo "ok|save-copy|$target_path"
            return 0
        fi
    fi

    error "saved but failed to copy (install wl-clipboard or xclip)"
}

action="${1:-}"
image_path="${2:-}"
require_file "$image_path"

case "$action" in
    copy)
        copy_to_clipboard "$image_path"
        ;;
    save-copy)
        save_and_copy "$image_path"
        ;;
    *)
        error "unknown action"
        ;;
esac
