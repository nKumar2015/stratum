#!/bin/sh

cmd="$1"

clamp_percent() {
    value="$1"
    if [ -z "$value" ]; then
        echo 0
        return
    fi

    if [ "$value" -lt 0 ] 2>/dev/null; then
        echo 0
        return
    fi

    if [ "$value" -gt 100 ] 2>/dev/null; then
        echo 100
        return
    fi

    echo "$value"
}

get_brightness_percent() {
    if command -v brightnessctl >/dev/null 2>&1; then
        current=$(brightnessctl get 2>/dev/null)
        max=$(brightnessctl max 2>/dev/null)
        if [ -n "$current" ] && [ -n "$max" ] && [ "$max" -gt 0 ] 2>/dev/null; then
            pct=$(awk -v c="$current" -v m="$max" 'BEGIN { printf "%d", (c * 100 / m) + 0.5 }')
            clamp_percent "$pct"
            return
        fi
    fi

    if command -v light >/dev/null 2>&1; then
        value=$(light -G 2>/dev/null | awk '{ printf "%d", $1 + 0.5 }')
        if [ -n "$value" ]; then
            clamp_percent "$value"
            return
        fi
    fi

    if command -v brillo >/dev/null 2>&1; then
        value=$(brillo -G 2>/dev/null | awk '{ printf "%d", $1 + 0.5 }')
        if [ -n "$value" ]; then
            clamp_percent "$value"
            return
        fi
    fi

    echo "__ERROR__|no brightness backend"
}

case "$cmd" in
    volume)
        if ! command -v pactl >/dev/null 2>&1; then
            echo "__ERROR__|pactl not found"
            exit 0
        fi

        volume=$(pactl get-sink-volume @DEFAULT_SINK@ 2>/dev/null | awk 'NR == 1 { if (match($0, /[0-9]+%/)) print substr($0, RSTART, RLENGTH); exit }')
        mute=$(pactl get-sink-mute @DEFAULT_SINK@ 2>/dev/null | awk '{print tolower($2); exit}')

        [ -n "$volume" ] || volume="0%"
        [ -n "$mute" ] || mute="yes"

        value=$(printf '%s' "$volume" | tr -d '%' | awk '{print int($1)}')
        if [ -z "$value" ]; then
            value=0
        fi

        if [ "$value" -lt 0 ] 2>/dev/null; then
            value=0
        elif [ "$value" -gt 150 ] 2>/dev/null; then
            value=150
        fi

        printf 'VOLUME|%s|%s\n' "$value" "$mute"
        ;;
    brightness)
        result=$(get_brightness_percent)
        if printf '%s' "$result" | grep -q '^__ERROR__'; then
            echo "$result"
            exit 0
        fi
        printf 'BRIGHTNESS|%s\n' "$result"
        ;;
    *)
        echo "__ERROR__|unknown osd command"
        ;;
esac
