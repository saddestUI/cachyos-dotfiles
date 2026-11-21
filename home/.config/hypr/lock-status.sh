#!/usr/bin/env bash
set -euo pipefail

case "${1:-}" in
    date)
        LC_TIME=ru_RU.UTF-8 date "+%A, %-d %B" | python3 -c '
import sys
s = sys.stdin.read().strip()
print(s[:1].upper() + s[1:] if s else "")
'
        ;;
    media)
        status=$(playerctl status 2>/dev/null || true)
        if [[ "$status" == "Playing" || "$status" == "Paused" ]]; then
            icon="󰝚"
            [[ "$status" == "Paused" ]] && icon="󰏤"
            artist=$(playerctl metadata xesam:artist 2>/dev/null || true)
            title=$(playerctl metadata xesam:title 2>/dev/null || true)
            if [[ -n "$artist" && -n "$title" ]]; then
                text="$artist — $title"
            elif [[ -n "$title" ]]; then
                text="$title"
            elif [[ -n "$artist" ]]; then
                text="$artist"
            else
                exit 0
            fi
            python3 - "$icon" "$text" <<'EOF'
import html, sys
icon, text = sys.argv[1], sys.argv[2].replace("\n", " ").strip()
if len(text) > 52:
    text = text[:51].rstrip() + "…"
print(f"{icon}  {html.escape(text)}")
EOF
        fi
        ;;
    battery)
        bat_dir="/sys/class/power_supply/BAT1"
        [[ -r "$bat_dir/capacity" ]] || exit 0
        pct=$(tr -d '\r\n' < "$bat_dir/capacity")
        status=$(tr -d '\r\n' < "$bat_dir/status" 2>/dev/null || echo "Unknown")
        if [[ "$status" == "Charging" || "$status" == "Full" || "$status" == "Not charging" ]]; then
            icon="󰂄"
        elif (( pct >= 85 )); then
            icon="󰁹"
        elif (( pct >= 60 )); then
            icon="󰂀"
        elif (( pct >= 35 )); then
            icon="󰁾"
        elif (( pct >= 15 )); then
            icon="󰁼"
        else
            icon="󰁺"
        fi
        printf '%s %s%%\n' "$icon" "$pct"
        ;;
    *)
        echo "Usage: $0 {date|media|battery}" >&2
        exit 1
        ;;
esac
