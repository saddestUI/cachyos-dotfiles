#!/usr/bin/env bash
"$HOME/.config/hypr/sync-lock-theme.sh"
pidof hyprlock >/dev/null && exit 0
exec hyprlock "$@"
