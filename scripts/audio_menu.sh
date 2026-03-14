#!/bin/sh

if ! command -v pactl >/dev/null 2>&1; then
    echo "__ERROR__|pactl not found"
    exit 0
fi

sanitize_field() {
    printf '%s' "$1" | tr '|' '/'
}

print_sink_rows() {
    pactl list sinks 2>/dev/null | awk '
        /^Sink #/ {
            if (name != "")
                printf "%s|%s\n", name, desc;
            name = "";
            desc = "";
            next;
        }
        /^[[:space:]]*Name: / {
            sub(/^[[:space:]]*Name: /, "");
            name = $0;
            next;
        }
        /^[[:space:]]*Description: / {
            sub(/^[[:space:]]*Description: /, "");
            desc = $0;
            next;
        }
        END {
            if (name != "")
                printf "%s|%s\n", name, desc;
        }
    '
}

print_source_rows() {
    pactl list sources 2>/dev/null | awk '
        /^Source #/ {
            if (name != "" && name !~ /\.monitor$/)
                printf "%s|%s\n", name, desc;
            name = "";
            desc = "";
            next;
        }
        /^[[:space:]]*Name: / {
            sub(/^[[:space:]]*Name: /, "");
            name = $0;
            next;
        }
        /^[[:space:]]*Description: / {
            sub(/^[[:space:]]*Description: /, "");
            desc = $0;
            next;
        }
        END {
            if (name != "" && name !~ /\.monitor$/)
                printf "%s|%s\n", name, desc;
        }
    '
}

cmd="$1"
shift || true

case "$cmd" in
    status)
        volume=$(pactl get-sink-volume @DEFAULT_SINK@ 2>/dev/null | awk 'NR == 1 { if (match($0, /[0-9]+%/)) print substr($0, RSTART, RLENGTH); exit }')
        mute=$(pactl get-sink-mute @DEFAULT_SINK@ 2>/dev/null | awk '{print tolower($2); exit}')
        [ -n "$volume" ] || volume="0%"
        [ -n "$mute" ] || mute="yes"
        printf '%s|%s\n' "$volume" "$mute"
        ;;
    hover-status)
        volume=$(pactl get-sink-volume @DEFAULT_SINK@ 2>/dev/null | awk 'NR == 1 { if (match($0, /[0-9]+%/)) print substr($0, RSTART, RLENGTH); exit }')
        mute=$(pactl get-sink-mute @DEFAULT_SINK@ 2>/dev/null | awk '{print tolower($2); exit}')
        [ -n "$volume" ] || volume="0%"
        [ -n "$mute" ] || mute="yes"

        def_sink=$(sanitize_field "$(pactl get-default-sink 2>/dev/null)")
        def_source=$(sanitize_field "$(pactl get-default-source 2>/dev/null)")
        printf 'STATUS|%s|%s\n' "$volume" "$mute"
        printf 'DEFAULT|%s|%s\n' "$def_sink" "$def_source"

        print_sink_rows | while IFS='|' read -r sink desc; do
            sink_safe=$(sanitize_field "$sink")
            desc_safe=$(sanitize_field "$desc")
            printf 'SINK|%s|%s\n' "$sink_safe" "$desc_safe"
        done

        print_source_rows | while IFS='|' read -r source desc; do
            source_safe=$(sanitize_field "$source")
            desc_safe=$(sanitize_field "$desc")
            printf 'SOURCE|%s|%s\n' "$source_safe" "$desc_safe"
        done
        ;;
    set-output)
        pactl set-default-sink "$1" 2>&1
        ;;
    set-input)
        pactl set-default-source "$1" 2>&1
        ;;
    set-volume)
        volume="$1"
        case "$volume" in
            ''|*[!0-9]*)
                echo "__ERROR__|invalid volume"
                exit 0
                ;;
        esac
        if [ "$volume" -lt 0 ]; then
            volume=0
        elif [ "$volume" -gt 150 ]; then
            volume=150
        fi
        pactl set-sink-volume @DEFAULT_SINK@ "${volume}%" 2>&1
        ;;
    open-control)
        if ! command -v pavucontrol >/dev/null 2>&1; then
            echo "__ERROR__|pavucontrol not found"
            exit 0
        fi
        nohup pavucontrol >/dev/null 2>&1 &
        echo "ok"
        ;;
    *)
        echo "__ERROR__|unknown audio command"
        ;;
esac
