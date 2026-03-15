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

choose_save_path() {
    default_path="$1"

    if command -v zenity >/dev/null 2>&1; then
        zenity --file-selection --save --confirm-overwrite --filename="$default_path" 2>/dev/null
        return $?
    fi

    if command -v kdialog >/dev/null 2>&1; then
        kdialog --getsavefilename "$default_path" "*.png" 2>/dev/null
        return $?
    fi

    return 1
}

save_as() {
    image_path="$1"
    target_dir="$HOME/Pictures/Screenshots"
    timestamp="$(date +%Y%m%d-%H%M%S)"
    default_path="$target_dir/Screenshot-$timestamp.png"

    mkdir -p "$target_dir" || error "failed to create screenshot directory"

    chosen_path="$(choose_save_path "$default_path")"
    if [ $? -ne 0 ] || [ -z "$chosen_path" ]; then
        error "save-as cancelled"
    fi

    chosen_path="$(normalize_path "$chosen_path")"
    case "$chosen_path" in
        *.png) ;;
        *) chosen_path="$chosen_path.png" ;;
    esac

    target_parent="$(dirname "$chosen_path")"
    mkdir -p "$target_parent" || error "failed to create destination directory"

    if ! cp -- "$image_path" "$chosen_path"; then
        error "failed to save screenshot"
    fi

    echo "ok|save-as|$chosen_path"
}

save_and_copy() {
    image_path="$1"
    target_path="$(save_only "$image_path" | awk -F'|' '/^ok\|save\|/ {print $3}')"
    if [ -z "$target_path" ]; then
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
image_path="$(require_file "${2:-}")"

case "$action" in
    copy)
        copy_to_clipboard "$image_path"
        ;;
    save)
        save_only "$image_path"
        ;;
    save-as)
        save_as "$image_path"
        ;;
    save-copy)
        save_and_copy "$image_path"
        ;;
    *)
        error "unknown action"
        ;;
esac
