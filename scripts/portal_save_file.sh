#!/bin/sh
set -u

error() {
    echo "__ERROR__|$1"
    exit 0
}

escape_gvariant_string() {
    printf "%s" "$1" | sed -e 's/\\/\\\\/g' -e "s/'/\\\\'/g"
}

title_raw="${1:-Save File}"
default_name_raw="${2:-untitled}"

command -v gdbus >/dev/null 2>&1 || error "gdbus is required for xdg-portal Save As"
command -v dbus-monitor >/dev/null 2>&1 || error "dbus-monitor is required for xdg-portal Save As"
command -v timeout >/dev/null 2>&1 || error "timeout is required for xdg-portal Save As"

default_name="$(escape_gvariant_string "$default_name_raw")"
handle_token="quickshell$(date +%s)$$"
options="{'handle_token': <'$handle_token'>, 'modal': <true>, 'current_name': <'$default_name'>}"

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
result="$(timeout "$portal_timeout" dbus-monitor --session \
    "type='signal',interface='org.freedesktop.portal.Request',member='Response',path='$request_path'" 2>/dev/null | awk '
    /uint32 / && code == "" {
        code_line = $0
        sub(/.*uint32 /, "", code_line)
        sub(/[^0-9].*/, "", code_line)
        code = int(code_line)
        if (code == 1 || code == 2) {
            print "cancel"
            exit
        }
        next
    }
    code == 0 && /string "file:\/\// && uri == "" {
        uri = $0
        sub(/.*string "/, "", uri)
        sub(/".*$/, "", uri)
        print "ok|" uri
        exit
    }
')"

[ -n "$result" ] || error "xdg-portal file chooser timed out"

case "$result" in
    ok\|*)
        printf "%s\n" "$result"
        ;;
    cancel)
        printf "cancel\n"
        ;;
    *)
        error "xdg-portal returned unexpected result"
        ;;
esac