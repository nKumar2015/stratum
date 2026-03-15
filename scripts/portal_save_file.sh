#!/bin/sh
set -u

error() {
    echo "__ERROR__|$1"
    exit 0
}

escape_gvariant_string() {
    printf "%s" "$1" | sed -e 's/\\/\\\\/g' -e "s/'/\\\\'/g"
}

require_filechooser_portal() {
    introspect_output="$(gdbus introspect \
        --session \
        --dest org.freedesktop.portal.Desktop \
        --object-path /org/freedesktop/portal/desktop 2>&1)" || error "$introspect_output"

    printf "%s\n" "$introspect_output" | grep -Fq "interface org.freedesktop.portal.FileChooser" || \
        error "xdg-desktop-portal FileChooser is unavailable on this session; install or enable a portal backend that provides file choosing"
}

title_raw="${1:-Save File}"
default_name_raw="${2:-untitled}"

command -v gdbus >/dev/null 2>&1 || error "gdbus is required for xdg-portal Save As"
command -v dbus-monitor >/dev/null 2>&1 || error "dbus-monitor is required for xdg-portal Save As"
command -v stdbuf >/dev/null 2>&1 || error "stdbuf is required for reliable xdg-portal Save As monitoring"

require_filechooser_portal

default_name="$(escape_gvariant_string "$default_name_raw")"
handle_token="quickshell$(date +%s)$$"
options="{'handle_token': <'$handle_token'>, 'modal': <true>, 'current_name': <'$default_name'>}"

monitor_log="$(mktemp "${XDG_RUNTIME_DIR:-/tmp}/qs-portal-monitor-XXXXXX.log")" || error "failed to create portal monitor log"
monitor_pid=""

cleanup() {
    [ -n "$monitor_pid" ] && kill "$monitor_pid" 2>/dev/null || true
    rm -f "$monitor_log"
}
trap cleanup EXIT INT TERM

# Start monitoring before SaveFile so quick responses cannot be missed.
stdbuf -oL dbus-monitor --session \
    "type='signal',interface='org.freedesktop.portal.Request',member='Response'" \
    > "$monitor_log" 2>/dev/null &
monitor_pid=$!

# Give the monitor a short moment to register on the bus.
sleep 0.15

# Open the portal file chooser
request_output="$(gdbus call \
    --session \
    --dest org.freedesktop.portal.Desktop \
    --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.portal.FileChooser.SaveFile \
    '' \
    "$title_raw" \
    "$options" 2>&1)" || error "$request_output"

request_path="$(printf "%s\n" "$request_output" | sed -n "s/^(objectpath '\([^']*\)',)$/\1/p")"
[ -n "$request_path" ] || error "failed to read portal request handle"

portal_timeout="${XDG_PORTAL_TIMEOUT:-300}"
result=""
deadline=$(( $(date +%s) + portal_timeout ))

while [ "$(date +%s)" -le "$deadline" ]; do
    result="$(awk -v req_path="$request_path" '
        /^signal / {
            in_response = (index($0, "path=" req_path) > 0 && index($0, "member=Response") > 0)
            if (in_response)
                code = -1
            next
        }
        in_response && /uint32 / && code < 0 {
            val = $0
            sub(/.*uint32 /, "", val)
            sub(/[^0-9].*/, "", val)
            code = int(val)
            if (code == 1 || code == 2) {
                print "cancel"
                exit
            }
            next
        }
        in_response && code == 0 && /string "file:\/\// {
            uri = $0
            sub(/.*string "/, "", uri)
            sub(/".*$/, "", uri)
            print "ok"
            print uri
            exit
        }
    ' "$monitor_log")"

    [ -n "$result" ] && break
    sleep 0.08
done

[ -n "$result" ] || error "xdg-portal file chooser timed out"

case "$result" in
    ok*)
        chosen_uri="$(printf "%s\n" "$result" | sed -n '2p')"
        [ -n "$chosen_uri" ] || error "xdg-portal returned no destination URI"
        printf "ok\n%s\n" "$chosen_uri"
        ;;
    cancel)
        printf "cancel\n"
        ;;
    "")
        error "xdg-portal file chooser timed out"
        ;;
    *)
        error "xdg-portal returned unexpected result"
        ;;
esac