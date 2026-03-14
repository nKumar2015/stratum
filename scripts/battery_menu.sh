#!/bin/sh

cmd="$1"
shift || true

format_duration() {
    total="$1"
    if [ -z "$total" ] || [ "$total" -le 0 ] 2>/dev/null; then
        echo "Unknown"
        return
    fi

    h=$((total / 3600))
    m=$(((total % 3600) / 60))

    if [ "$h" -gt 0 ]; then
        printf '%dh %02dm' "$h" "$m"
    else
        printf '%dm' "$m"
    fi
}

read_battery_device() {
    upower -e 2>/dev/null | awk '/battery|BAT/ { print; exit }'
}

read_battery_percentage() {
    dev="$1"
    pct=""

    if [ -n "$dev" ]; then
        pct=$(upower -i "$dev" 2>/dev/null | awk -F: '/percentage/ { gsub(/[%[:space:]]/, "", $2); print $2; exit }')
    fi

    if [ -z "$pct" ]; then
        for cap in /sys/class/power_supply/BAT*/capacity; do
            [ -f "$cap" ] || continue
            pct=$(awk '{print int($1)}' "$cap" 2>/dev/null)
            [ -n "$pct" ] && break
        done
    fi

    [ -n "$pct" ] || pct=0
    printf '%s' "$pct"
}

read_battery_state() {
    dev="$1"
    state=""

    if [ -n "$dev" ]; then
        state=$(upower -i "$dev" 2>/dev/null | awk -F: '/state/ { gsub(/^[[:space:]]+/, "", $2); print tolower($2); exit }')
    fi

    if [ -z "$state" ]; then
        for st in /sys/class/power_supply/BAT*/status; do
            [ -f "$st" ] || continue
            state=$(awk '{print tolower($1)}' "$st" 2>/dev/null)
            [ -n "$state" ] && break
        done
    fi

    # Normalise sysfs "not charging" (two words) to the upower token
    case "$state" in
    "not charging") state="pending-charge" ;;
    esac

    [ -n "$state" ] || state="unknown"
    printf '%s' "$state"
}

read_projected_seconds() {
    dev="$1"
    state="$2"

    if [ -z "$dev" ]; then
        echo 0
        return
    fi

    case "$state" in
    charging)
        upower -i "$dev" 2>/dev/null | awk -F: '/time to full/ { gsub(/^[[:space:]]+/, "", $2); print $2; exit }' | awk '
                {
                    if ($1 ~ /^[0-9]+(\.[0-9]+)?$/) {
                        value = $1 + 0;
                        unit = $2;
                        if (unit ~ /hour/) print int(value * 3600);
                        else if (unit ~ /minute/) print int(value * 60);
                        else if (unit ~ /second/) print int(value);
                        else print 0;
                    } else {
                        print 0;
                    }
                }
            '
        ;;
    discharging)
        upower -i "$dev" 2>/dev/null | awk -F: '/time to empty/ { gsub(/^[[:space:]]+/, "", $2); print $2; exit }' | awk '
                {
                    if ($1 ~ /^[0-9]+(\.[0-9]+)?$/) {
                        value = $1 + 0;
                        unit = $2;
                        if (unit ~ /hour/) print int(value * 3600);
                        else if (unit ~ /minute/) print int(value * 60);
                        else if (unit ~ /second/) print int(value);
                        else print 0;
                    } else {
                        print 0;
                    }
                }
            '
        ;;
    *)
        echo 0
        ;;
    esac
}

read_screen_on_time() {
    if [ -f /proc/uptime ]; then
        up=$(awk '{print int($1)}' /proc/uptime 2>/dev/null)
        format_duration "$up"
        return
    fi
    echo "Unknown"
}

read_active_profile() {
    local sysfs="/sys/firmware/acpi/platform_profile"
    if [ -f "$sysfs" ]; then
        cat "$sysfs" 2>/dev/null || echo "unknown"
    else
        echo "unknown"
    fi
}

read_charging_info() {
    dev="$1"
    state="$2"

    # pending-charge: plugged in but not actively charging and not full
    if [ "$state" = "pending-charge" ]; then
        echo "Plugged in, not charging"
        return
    fi

    if [ "$state" != "charging" ] || [ -z "$dev" ]; then
        echo ""
        return
    fi

    rate=$(upower -i "$dev" 2>/dev/null | awk -F: '/energy-rate/ { gsub(/^[[:space:]]+/, "", $2); print $2; exit }')
    [ -n "$rate" ] && {
        printf '%s' "$rate"
        return
    }

    # Fallback for systems that don't expose energy-rate.
    echo "Connected to charger"
}

case "$cmd" in
hover-status)
    dev=$(read_battery_device)
    pct=$(read_battery_percentage "$dev")
    state=$(read_battery_state "$dev")
    projected_sec=$(read_projected_seconds "$dev" "$state")
    projected_text=$(format_duration "$projected_sec")
    screen_on=$(read_screen_on_time)
    profile=$(read_active_profile)
    charging_info=$(read_charging_info "$dev" "$state")

    printf 'BATTERY|%s|%s|%s|%s\n' "$pct" "$state" "$projected_text" "$screen_on"
    printf 'CHARGING|%s\n' "$charging_info"
    printf 'PROFILE|%s\n' "$profile"
    ;;
set-profile)
    profile="$1"
    case "$profile" in
    low-power | balanced | balanced-performance) ;;
    *)
        echo "__ERROR__|invalid profile"
        exit 0
        ;;
    esac

    sysfs="/sys/firmware/acpi/platform_profile"
    if [ ! -f "$sysfs" ]; then
        echo "__ERROR__|platform_profile not available"
        exit 0
    fi

    printf '%s' "$profile" | tee "$sysfs" >/dev/null 2>&1
    if [ $? -ne 0 ]; then
        echo "__ERROR__|failed to set profile"
    else
        echo "ok"
    fi
    ;;
*)
    echo "__ERROR__|unknown battery command"
    ;;
esac
