#!/bin/sh

rfkill_bluetooth_power() {
    found=0
    any_unblocked=0

    for dev in /sys/class/rfkill/rfkill*; do
        [ -d "$dev" ] || continue
        [ -r "$dev/type" ] || continue

        type=$(cat "$dev/type" 2>/dev/null)
        [ "$type" = "bluetooth" ] || continue
        found=1

        soft=$(cat "$dev/soft" 2>/dev/null)
        hard=$(cat "$dev/hard" 2>/dev/null)

        if [ "$soft" = "0" ] && [ "$hard" = "0" ]; then
            any_unblocked=1
            break
        fi
    done

    if [ "$found" -eq 0 ]; then
        return 1
    fi

    if [ "$any_unblocked" -eq 1 ]; then
        echo "on"
    else
        echo "off"
    fi
    return 0
}

if command -v bluetoothctl >/dev/null 2>&1; then
    status=$(bluetoothctl show 2>/dev/null | awk -F': ' '/Powered:/ {print tolower($2); exit}')

    if [ "$status" != "yes" ]; then
        echo "off"
        exit 0
    fi

    if bluetoothctl devices Connected 2>/dev/null | grep -q .; then
        echo "connected"
    else
        echo "on"
    fi
    exit 0
fi

# Fallback without bluetoothctl: /sys rfkill can tell power state,
# but not active device connection state.
if state=$(rfkill_bluetooth_power); then
    echo "$state"
else
    echo "none"
fi
