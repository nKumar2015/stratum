#!/bin/sh

# 1. Check for active Ethernet
for dev in /sys/class/net/e* /sys/class/net/en*; do
    [ -e "$dev" ] || continue
    if [ "$(cat "$dev/operstate" 2>/dev/null)" = "up" ]; then
        echo "ethernet"
        exit 0
    fi
done

# 2. Check for active Wi-Fi and calculate signal strength
for dev in /sys/class/net/w* /sys/class/net/wl*; do
    [ -e "$dev" ] || continue
    if [ "$(cat "$dev/operstate" 2>/dev/null)" = "up" ]; then
        dev_name=${dev##*/}
        # Read link quality from /proc/net/wireless (usually a scale of 0-70)
        qual=$(awk -v d="$dev_name:" '$1==d {print int($3)}' /proc/net/wireless | tr -d .)

        if [ -n "$qual" ]; then
            # Convert to a 0-100 percentage
            pct=$((qual * 100 / 70))
            echo "wifi:$pct"
            exit 0
        fi
    fi
done

echo "none"
