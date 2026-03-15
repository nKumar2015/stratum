#!/bin/sh

sanitize_field() {
    printf '%s' "$1" | tr '|' '/'
}

normalize_player_title() {
    raw="$1"
    raw=${raw##*MediaPlayer2.}
    raw=${raw%%.instance*}
    raw=$(printf '%s' "$raw" | tr '._-' ' ')

    pretty=$(printf '%s' "$raw" | awk '{
        for (i = 1; i <= NF; i++) {
            $i = toupper(substr($i, 1, 1)) tolower(substr($i, 2));
        }
        print;
    }')

    printf '%s' "$pretty"
}

format_mmss() {
    total="$1"
    case "$total" in
        ''|*[!0-9]*)
            printf '00:00'
            return
            ;;
    esac

    mm=$((total / 60))
    ss=$((total % 60))
    printf '%02d:%02d' "$mm" "$ss"
}

if ! command -v playerctl >/dev/null 2>&1; then
    echo "MUSIC|Unavailable|N/A|playerctl not installed|N/A|N/A|00:00|00:00||N/A"
    exit 0
fi

selected_player=""
selected_status=""

players=$(playerctl -l 2>/dev/null | awk '!seen[$0]++')
if [ -n "$players" ]; then
    for p in $players; do
        status=$(playerctl -p "$p" status 2>/dev/null)
        [ -n "$status" ] || continue
        if [ "$status" = "Playing" ]; then
            selected_player="$p"
            selected_status="$status"
            break
        fi
        if [ -z "$selected_player" ]; then
            selected_player="$p"
            selected_status="$status"
        fi
    done
fi

if [ -z "$selected_player" ]; then
    echo "MUSIC|Stopped|None|Nothing playing|N/A|N/A|00:00|00:00||None"
    exit 0
fi

title=$(playerctl -p "$selected_player" metadata xesam:title 2>/dev/null)
artist=$(playerctl -p "$selected_player" metadata xesam:artist 2>/dev/null | head -n1)
album=$(playerctl -p "$selected_player" metadata xesam:album 2>/dev/null)
art_url=$(playerctl -p "$selected_player" metadata mpris:artUrl 2>/dev/null)
player_title=$(playerctl -p "$selected_player" metadata --format '{{mpris:identity}}' 2>/dev/null)

[ -n "$title" ] || title="Unknown Title"
[ -n "$artist" ] || artist="Unknown Artist"
[ -n "$album" ] || album="Unknown Album"
[ -n "$art_url" ] || art_url=""
[ -n "$player_title" ] || player_title=$(normalize_player_title "$selected_player")
[ -n "$player_title" ] || player_title="$selected_player"

pos_raw=$(playerctl -p "$selected_player" position 2>/dev/null | awk '{print int($1)}')
len_micro=$(playerctl -p "$selected_player" metadata mpris:length 2>/dev/null | awk '{print int($1)}')
len_sec=0
if [ -n "$len_micro" ] && [ "$len_micro" -gt 0 ] 2>/dev/null; then
    len_sec=$((len_micro / 1000000))
fi

pos_fmt=$(format_mmss "${pos_raw:-0}")
len_fmt=$(format_mmss "$len_sec")

printf 'MUSIC|%s|%s|%s|%s|%s|%s|%s|%s|%s\n' \
    "$(sanitize_field "$selected_status")" \
    "$(sanitize_field "$selected_player")" \
    "$(sanitize_field "$title")" \
    "$(sanitize_field "$artist")" \
    "$(sanitize_field "$album")" \
    "$pos_fmt" \
    "$len_fmt" \
    "$(sanitize_field "$art_url")" \
    "$(sanitize_field "$player_title")"
