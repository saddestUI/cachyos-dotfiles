#!/bin/sh
# This file is executed when starting Ly (before the TTY is taken control of)
# Dynamically loads the wallfx wallpaper palette from kitty/theme.conf into Linux TTY colors.

if [ "$TERM" = "linux" ]; then
    THEME_FILE="__USER_HOME__/.config/kitty/theme.conf"

    get_hex() {
        key="$1"
        fallback="$2"
        val=""
        if [ -r "$THEME_FILE" ]; then
            val=$(awk -v k="$key" '
                $1 == k {
                    v = $2
                    sub(/^#/, "", v)
                    if (v ~ /^[0-9a-fA-F]{6}$/) {
                        print tolower(v)
                        exit
                    }
                }
            ' "$THEME_FILE")
        fi
        if [ -n "$val" ]; then
            printf '%s' "$val"
        else
            printf '%s' "$fallback"
        fi
    }

    C0=$(get_hex "background" "0f1116")
    C1=$(get_hex "color1" "eb6387")
    C2=$(get_hex "color2" "6ce0ae")
    C3=$(get_hex "color3" "f0ad69")
    C4=$(get_hex "active_tab_background" "3b62cc")
    C5=$(get_hex "color5" "bf75eb")
    C6=$(get_hex "color250" "5d86f5")
    C7=$(get_hex "color252" "bacbfa")
    C8=$(get_hex "inactive_border_color" "4b505c")
    C9=$(get_hex "color9" "fa789b")
    CA=$(get_hex "color10" "84f5c4")
    CB=$(get_hex "color11" "fcc083")
    CC=$(get_hex "color250" "5d86f5")
    CD=$(get_hex "color251" "775df5")
    CE=$(get_hex "color14" "86e4f7")
    CF=$(get_hex "color15" "f0f3fa")

    i=0
    for hex in "$C0" "$C1" "$C2" "$C3" "$C4" "$C5" "$C6" "$C7" "$C8" "$C9" "$CA" "$CB" "$CC" "$CD" "$CE" "$CF"; do
        printf "\033]P%x%s" "$i" "$hex"
        i=$((i + 1))
    done

    clear
fi
