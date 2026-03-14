#!/bin/sh

if ! command -v bluetoothctl >/dev/null 2>&1; then
    echo "__ERROR__|bluetoothctl not found"
    exit 0
fi

cmd="$1"
shift || true

case "$cmd" in
    hover-list)
        bluetoothctl devices Paired 2>/dev/null | while read -r _ mac rest; do
            name="$rest"
            connected=$(bluetoothctl info "$mac" 2>/dev/null | awk -F': ' '/Connected:/ {print tolower($2); exit}')
            printf '%s|%s|%s\n' "$mac" "$name" "${connected:-no}"
        done
        ;;
    hover-connect)
        bluetoothctl connect "$1" 2>&1
        ;;
    hover-disconnect)
        bluetoothctl disconnect "$1" 2>&1
        ;;
    state)
        bluetoothctl show 2>/dev/null | awk -F': ' '/Powered:/ {print tolower($2); exit}'
        ;;
    list)
        {
            bluetoothctl devices 2>/dev/null
            bluetoothctl devices Paired 2>/dev/null
        } | awk '!seen[$2]++' | while read -r _ mac name; do
            info=$(bluetoothctl info "$mac" 2>/dev/null)
            connected=$(printf '%s\n' "$info" | awk -F': ' '/Connected:/ {print tolower($2); exit}')
            trusted=$(printf '%s\n' "$info" | awk -F': ' '/Trusted:/ {print tolower($2); exit}')
            paired=$(printf '%s\n' "$info" | awk -F': ' '/Paired:/ {print tolower($2); exit}')
            printf '%s|%s|%s|%s|%s\n' "$mac" "$name" "${connected:-no}" "${trusted:-no}" "${paired:-no}"
        done
        ;;
    connect)
        bluetoothctl connect "$1" 2>&1
        ;;
    disconnect)
        bluetoothctl disconnect "$1" 2>&1
        ;;
    forget)
        bluetoothctl remove "$1" 2>&1
        ;;
    power)
        bluetoothctl --timeout 4 power "$1" 2>&1
        ;;
    scan)
        bluetoothctl --timeout 5 scan on 2>&1
        ;;
    *)
        echo "__ERROR__|unknown bluetooth command"
        ;;
esac
