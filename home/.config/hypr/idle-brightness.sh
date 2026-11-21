#!/usr/bin/env bash
set -euo pipefail

STATE_FILE="${XDG_RUNTIME_DIR:-/tmp}/hypridle-brightness"
SMOOTH_BIN="/home/samerim/.local/bin/smooth-brightness"

case "${1:-}" in
    dim)
        cur=$(brightnessctl -m 2>/dev/null | cut -d, -f4 | tr -d '%' || true)
        if [[ "$cur" =~ ^[0-9]+$ ]] && (( cur > 15 )); then
            printf '%s\n' "$cur" > "$STATE_FILE"
            brightnessctl -s >/dev/null 2>&1 || true
            if [[ -x "$SMOOTH_BIN" ]]; then
                "$SMOOTH_BIN" set 15 >/dev/null 2>&1 || brightnessctl set 15% >/dev/null 2>&1 || true
            else
                brightnessctl set 15% >/dev/null 2>&1 || true
            fi
        fi
        ;;
    restore)
        if [[ -f "$STATE_FILE" ]]; then
            saved=$(tr -d '\r\n' < "$STATE_FILE" || true)
            rm -f "$STATE_FILE"
            if [[ "$saved" =~ ^[0-9]+$ ]]; then
                if [[ -x "$SMOOTH_BIN" ]]; then
                    "$SMOOTH_BIN" set "$saved" >/dev/null 2>&1 || brightnessctl -r >/dev/null 2>&1 || true
                else
                    brightnessctl set "${saved}%" >/dev/null 2>&1 || brightnessctl -r >/dev/null 2>&1 || true
                fi
            else
                brightnessctl -r >/dev/null 2>&1 || true
            fi
        fi
        ;;
    *)
        echo "Usage: $0 {dim|restore}" >&2
        exit 1
        ;;
esac
