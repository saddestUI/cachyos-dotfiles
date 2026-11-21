#!/usr/bin/env bash
/home/samerim/.config/hypr/sync-lock-theme.sh
pidof hyprlock >/dev/null && exit 0
exec hyprlock "$@"
