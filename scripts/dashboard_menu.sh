#!/bin/sh

cmd="$1"
[ -n "$cmd" ] || cmd="all"

sanitize_field() {
    printf '%s' "$1" | tr '|' '/'
}

format_gib_tenths_from_kib() {
    kib="$1"
    awk -v v="$kib" 'BEGIN { if (v == "" || v <= 0) { print "0.0"; exit } printf "%.1f", v / 1048576 }'
}

output_calendar() {
    requested_year="$1"
    requested_month="$2"

    if ! command -v cal >/dev/null 2>&1; then
        echo "__ERROR__|cal not found"
        return
    fi

    if [ -n "$requested_year" ] && [ -n "$requested_month" ]; then
        year_num=$(printf '%s' "$requested_year" | awk 'BEGIN{y=0} {if ($0 ~ /^[0-9]+$/) y=int($0); print y}')
        month_num=$(printf '%s' "$requested_month" | awk 'BEGIN{m=0} {if ($0 ~ /^[0-9]+$/) m=int($0); print m}')
        if [ "$year_num" -lt 1 ] 2>/dev/null; then
            year_num=$(date '+%Y')
        fi
        if [ "$month_num" -lt 1 ] 2>/dev/null || [ "$month_num" -gt 12 ] 2>/dev/null; then
            month_num=$(date '+%m')
        fi
    else
        month_num=$(date '+%m')
        year_num=$(date '+%Y')
    fi

    month_num=$(printf '%s' "$month_num" | awk '{print int($1)}')
    year_num=$(printf '%s' "$year_num" | awk '{print int($1)}')

    cal_raw=$(cal "$month_num" "$year_num" 2>/dev/null)
    if [ -z "$cal_raw" ]; then
        echo "__ERROR__|failed to read calendar"
        return
    fi

    title=$(printf '%s\n' "$cal_raw" | sed -n '1p' | xargs)
    weekday_line=$(printf '%s\n' "$cal_raw" | sed -n '2p' | xargs)
    today_day=$(date '+%e' | xargs)
    today_month=$(date '+%m' | awk '{print int($1)}')
    today_year=$(date '+%Y' | awk '{print int($1)}')
    first_weekday=$(date -d "${year_num}-$(printf '%02d' "$month_num")-01" '+%w' 2>/dev/null)
    days_in_month=$(date -d "${year_num}-$(printf '%02d' "$month_num")-01 +1 month -1 day" '+%d' 2>/dev/null)

    [ -n "$first_weekday" ] || first_weekday=0
    [ -n "$days_in_month" ] || days_in_month=30

    if [ "$month_num" -eq "$today_month" ] 2>/dev/null && [ "$year_num" -eq "$today_year" ] 2>/dev/null; then
        current_today="$today_day"
    else
        current_today="-1"
    fi

    printf 'CAL_TITLE|%s\n' "$(sanitize_field "$title")"
    printf 'CAL_META|%s|%s|%s|%s\n' "$year_num" "$month_num" "$days_in_month" "$first_weekday"

    if [ -n "$weekday_line" ]; then
        set -- $weekday_line
        printf 'CAL_WEEKDAYS|%s|%s|%s|%s|%s|%s|%s\n' "${1:-Su}" "${2:-Mo}" "${3:-Tu}" "${4:-We}" "${5:-Th}" "${6:-Fr}" "${7:-Sa}"
    else
        printf 'CAL_WEEKDAYS|Su|Mo|Tu|We|Th|Fr|Sa\n'
    fi

    printf '%s\n' "$cal_raw" | awk '
        NR >= 3 {
            row = ""
            for (i = 1; i <= 7; i++) {
                start = (i - 1) * 3 + 1
                cell = substr($0, start, 2)
                gsub(/^[[:space:]]+|[[:space:]]+$/, "", cell)
                if (cell == "")
                    cell = "0"
                row = row ((i == 1) ? "" : "|") cell
            }
            if (row != "||||||") {
                print "CAL_ROW|" row
                rows++
            }
        }
        END {
            while (rows < 6) {
                print "CAL_ROW|0|0|0|0|0|0|0"
                rows++
            }
        }
    '

    printf 'TODAY|%s\n' "$current_today"
}

cpu_usage_percent() {
    first=$(awk '/^cpu / {print $2,$3,$4,$5,$6,$7,$8,$9}' /proc/stat 2>/dev/null)
    [ -n "$first" ] || {
        echo "0"
        return
    }

    sleep 0.2

    second=$(awk '/^cpu / {print $2,$3,$4,$5,$6,$7,$8,$9}' /proc/stat 2>/dev/null)
    [ -n "$second" ] || {
        echo "0"
        return
    }

    set -- $first
    user1=$1; nice1=$2; sys1=$3; idle1=$4; iowait1=$5; irq1=$6; soft1=$7; steal1=$8
    total1=$((user1 + nice1 + sys1 + idle1 + iowait1 + irq1 + soft1 + steal1))
    idle_all1=$((idle1 + iowait1))

    set -- $second
    user2=$1; nice2=$2; sys2=$3; idle2=$4; iowait2=$5; irq2=$6; soft2=$7; steal2=$8
    total2=$((user2 + nice2 + sys2 + idle2 + iowait2 + irq2 + soft2 + steal2))
    idle_all2=$((idle2 + iowait2))

    total_delta=$((total2 - total1))
    idle_delta=$((idle_all2 - idle_all1))

    if [ "$total_delta" -le 0 ] 2>/dev/null; then
        echo "0"
        return
    fi

    used=$((total_delta - idle_delta))
    if [ "$used" -lt 0 ] 2>/dev/null; then
        used=0
    fi

    echo $((used * 100 / total_delta))
}

read_gpu_usage() {
    if command -v nvidia-smi >/dev/null 2>&1; then
        nv=$(nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader,nounits 2>/dev/null | head -n1 | awk '{print int($1)}')
        if [ -n "$nv" ]; then
            echo "$nv|NVIDIA"
            return
        fi
    fi

    for busy in /sys/class/drm/card*/device/gpu_busy_percent; do
        [ -r "$busy" ] || continue
        val=$(awk '{print int($1)}' "$busy" 2>/dev/null)
        if [ -n "$val" ]; then
            echo "$val|DRM"
            return
        fi
    done

    for busy in /sys/class/drm/card*/gt_busy_percent; do
        [ -r "$busy" ] || continue
        val=$(awk '{print int($1)}' "$busy" 2>/dev/null)
        if [ -n "$val" ]; then
            echo "$val|INTEL"
            return
        fi
    done

    echo "N/A|N/A"
}

output_performance() {
    cpu=$(cpu_usage_percent)
    case "$cpu" in
        ''|*[!0-9]*) cpu=0 ;;
    esac

    gpu_pair=$(read_gpu_usage)
    gpu_usage=$(printf '%s' "$gpu_pair" | awk -F'|' '{print $1}')
    gpu_vendor=$(printf '%s' "$gpu_pair" | awk -F'|' '{print $2}')

    mem_total_kib=$(awk '/^MemTotal:/ {print int($2)}' /proc/meminfo 2>/dev/null)
    mem_avail_kib=$(awk '/^MemAvailable:/ {print int($2)}' /proc/meminfo 2>/dev/null)
    [ -n "$mem_total_kib" ] || mem_total_kib=0
    [ -n "$mem_avail_kib" ] || mem_avail_kib=0
    mem_used_kib=$((mem_total_kib - mem_avail_kib))
    if [ "$mem_used_kib" -lt 0 ] 2>/dev/null; then
        mem_used_kib=0
    fi
    mem_pct=0
    if [ "$mem_total_kib" -gt 0 ] 2>/dev/null; then
        mem_pct=$((mem_used_kib * 100 / mem_total_kib))
    fi

    root_line=$(df -kP / 2>/dev/null | awk 'NR==2 {print $2"|"$3"|"$5}')
    stor_total_kib=$(printf '%s' "$root_line" | awk -F'|' '{print int($1)}')
    stor_used_kib=$(printf '%s' "$root_line" | awk -F'|' '{print int($2)}')
    stor_pct=$(printf '%s' "$root_line" | awk -F'|' '{gsub(/%/, "", $3); print int($3)}')
    [ -n "$stor_total_kib" ] || stor_total_kib=0
    [ -n "$stor_used_kib" ] || stor_used_kib=0
    [ -n "$stor_pct" ] || stor_pct=0

    mem_used_gib=$(format_gib_tenths_from_kib "$mem_used_kib")
    mem_total_gib=$(format_gib_tenths_from_kib "$mem_total_kib")
    stor_used_gib=$(format_gib_tenths_from_kib "$stor_used_kib")
    stor_total_gib=$(format_gib_tenths_from_kib "$stor_total_kib")

    printf 'CPU|%s\n' "$cpu"
    printf 'GPU|%s|%s\n' "$(sanitize_field "$gpu_usage")" "$(sanitize_field "$gpu_vendor")"
    printf 'RAM|%s|%s|%s\n' "$mem_used_gib" "$mem_total_gib" "$mem_pct"
    printf 'STORAGE|%s|%s|%s\n' "$stor_used_gib" "$stor_total_gib" "$stor_pct"
}

case "$cmd" in
    calendar)
        output_calendar "$2" "$3"
        ;;
    music)
        sh "$(dirname "$0")/dashboard_music.sh"
        ;;
    performance)
        output_performance
        ;;
    all)
        output_calendar "$2" "$3"
        sh "$(dirname "$0")/dashboard_music.sh"
        output_performance
        ;;
    *)
        echo "__ERROR__|unknown dashboard command"
        ;;
esac
