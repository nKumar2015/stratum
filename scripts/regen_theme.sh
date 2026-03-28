#!/bin/sh
set -eu

error() {
    echo "regen-theme: $1" >&2
    exit 1
}

require_tool() {
    if ! command -v "$1" >/dev/null 2>&1; then
        error "$1 not found"
    fi
}

resolve_wallpaper() {
    if [ "$#" -gt 0 ]; then
        printf "%s\n" "$1"
        return 0
    fi

    # Prefer Hyprpaper when running on Hyprland.
    if command -v hyprctl >/dev/null 2>&1; then
        wall="$(hyprctl hyprpaper listactive 2>/dev/null | sed -nE 's/^.*[:=][[:space:]]*(\/.*)$/\1/p' | head -n1)"
        if [ -z "$wall" ]; then
            wall="$(hyprctl hyprpaper listloaded 2>/dev/null | sed -nE 's/^.*[:=][[:space:]]*(\/.*)$/\1/p' | head -n1)"
        fi
        if [ -n "$wall" ]; then
            printf "%s\n" "$wall"
            return 0
        fi
    fi

    if command -v swww >/dev/null 2>&1; then
        wall="$(swww query 2>/dev/null | sed -n 's/.*image: //p' | head -n1)"
        if [ -n "$wall" ]; then
            printf "%s\n" "$wall"
            return 0
        fi
    fi

    return 1
}

require_tool matugen

wallpaper="$(resolve_wallpaper "$@" || true)"
if [ -z "$wallpaper" ]; then
    error "could not detect wallpaper automatically; pass an image path"
fi

if [ ! -f "$wallpaper" ]; then
    error "wallpaper file does not exist: $wallpaper"
fi

# This uses ~/.config/matugen/config.toml and regenerates configured templates.
matugen image "$wallpaper"

# Reload quickshell if managed as a user service.
if command -v systemctl >/dev/null 2>&1; then
    systemctl --user try-restart quickshell.service >/dev/null 2>&1 || true
fi

echo "regen-theme: updated using $wallpaper"
