#!/bin/sh

if ! command -v nmcli >/dev/null 2>&1; then
    echo "__ERROR__|nmcli not found"
    exit 0
fi

cmd="$1"
shift || true

case "$cmd" in
    hover-status)
        nmcli -t -f DEVICE,TYPE,STATE,CONNECTION dev 2>/dev/null | while IFS=: read -r dev type state conn; do
            [ "$state" = "connected" ] || continue
            ip=$(nmcli -t -f IP4.ADDRESS dev show "$dev" 2>/dev/null | awk -F: '{print $2; exit}' | sed 's|/.*||')
            gw=$(nmcli -t -f IP4.GATEWAY dev show "$dev" 2>/dev/null | awk -F: '{print $2; exit}')
            if [ "$type" = "ethernet" ] || [ "$type" = "bridge" ]; then
                printf 'ethernet|%s|%s||%s|%s\n' "$dev" "$conn" "${ip}" "${gw}"
            elif [ "$type" = "wifi" ]; then
                sig=$(nmcli -t -f IN-USE,SSID,SIGNAL dev wifi list ifname "$dev" 2>/dev/null | awk -F: '/^\*/ {print $3; exit}')
                printf 'wifi|%s|%s|%s|%s|%s\n' "$dev" "$conn" "${sig}" "${ip}" "${gw}"
            fi
        done
        ;;
    state)
        nmcli -t -f WIFI general status 2>/dev/null
        ;;
    device-status)
        nmcli -t -f DEVICE,TYPE,STATE,CONNECTION dev status 2>/dev/null
        ;;
    known-connections)
        nmcli -t -f NAME,TYPE connection show 2>/dev/null
        ;;
    list)
        nmcli -t -f IN-USE,SSID,SIGNAL,SECURITY dev wifi list --rescan auto 2>/dev/null
        ;;
    active-info)
        nmcli -t -f IP4.ADDRESS,IP4.GATEWAY dev show "$1" 2>/dev/null
        ;;
    connect)
        ssid="$1"
        password="$2"
        if [ -n "$password" ]; then
            nmcli dev wifi connect "$ssid" password "$password" 2>&1
        else
            nmcli dev wifi connect "$ssid" 2>&1
        fi
        ;;
    disconnect)
        nmcli dev disconnect "$1" 2>&1
        ;;
    forget)
        nmcli connection delete id "$1" 2>&1
        ;;
    toggle)
        nmcli radio wifi "$1" 2>&1
        ;;
    *)
        echo "__ERROR__|unknown wifi command"
        ;;
esac
