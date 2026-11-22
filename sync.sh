#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

log() { printf '\033[1;34m[sync]\033[0m %s\n' "$*"; }

# 1. Обновление переносимого списка AUR (без машинно-зависимых/несуществующих пакетов)
if command -v yay >/dev/null 2>&1; then
    log "Обновление packages/pkglist-aur.txt..."
    raw_aur=$(pacman -Qqem | sort || true)
    if [[ -f "$REPO_DIR/packages/hardware-exclude.regex" ]]; then
        regex_clean=$(grep -Ev '^\s*(#|$)' "$REPO_DIR/packages/hardware-exclude.regex" || true)
        if [[ -n "$regex_clean" ]]; then
            raw_aur=$(grep -Evf <(printf '%s\n' "$regex_clean") <<<"$raw_aur" || true)
        fi
    fi
    valid_aur=()
    while IFS= read -r pkg; do
        [[ -n "$pkg" ]] || continue
        if yay -Si "$pkg" >/dev/null 2>&1; then
            valid_aur+=("$pkg")
        fi
    done <<<"$raw_aur"
    printf '%s\n' "${valid_aur[@]}" >"$REPO_DIR/packages/pkglist-aur.txt"
fi

log "Обновление packages/systemd-user-enabled.txt..."
systemctl --user list-unit-files --state=enabled --no-legend --no-pager \
    | awk '{print $1}' \
    | grep -v '^hyprpolkitagent\.service$' \
    | sort >"$REPO_DIR/packages/systemd-user-enabled.txt"

# 2. Домашние конфиги, исходники и бинарники (без хостовых host.lua, .nvidia-settings-rc, QtProject.conf, user-dirs.*)
log "Обновление переносимых домашних конфигов, исходников и бинарников..."
for f in .bashrc .bash_profile .bash_logout; do
    [[ -f "$HOME/$f" ]] && cp -a "$HOME/$f" "$REPO_DIR/home/$f"
done

[[ -f "$HOME/.config/btop/btop.conf" ]] && cp -a "$HOME/.config/btop/btop.conf" "$REPO_DIR/home/.config/btop/"
if [[ -d "$HOME/.config/btop/themes" ]] && [[ -n "$(ls -A "$HOME/.config/btop/themes" 2>/dev/null)" ]]; then
    cp -a "$HOME/.config/btop/themes" "$REPO_DIR/home/.config/btop/"
fi

rsync -a --delete "$HOME/.config/fastfetch/" "$REPO_DIR/home/.config/fastfetch/"
python3 - "$REPO_DIR/home/.config/fastfetch/config.jsonc" "$HOME" <<'PY'
import sys
path, home = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8") as f:
    s = f.read()
s = s.replace(home, "__USER_HOME__")
with open(path, "w", encoding="utf-8") as f:
    f.write(s)
PY

rsync -a --delete "$HOME/.config/fish/" "$REPO_DIR/home/.config/fish/"
if [[ -f "$REPO_DIR/home/.config/fish/fish_variables" ]]; then
    grep -v '^SETUVAR fish_user_paths:' "$REPO_DIR/home/.config/fish/fish_variables" >"$REPO_DIR/home/.config/fish/fish_variables.tmp" \
        && mv -f "$REPO_DIR/home/.config/fish/fish_variables.tmp" "$REPO_DIR/home/.config/fish/fish_variables"
fi

mkdir -p "$REPO_DIR/home/.config/fontconfig/conf.d"
[[ -f "$HOME/.config/fontconfig/conf.d/99-doc-fonts.conf" ]] && \
    cp -a "$HOME/.config/fontconfig/conf.d/99-doc-fonts.conf" "$REPO_DIR/home/.config/fontconfig/conf.d/"

rsync -a --delete \
    --exclude='target/' --exclude='*.bak*' --exclude='*.log' --exclude='*.staged' \
    --exclude='host.lua' --exclude='.git/' \
    "$HOME/.config/hypr/" "$REPO_DIR/home/.config/hypr/"

if [[ -f "$REPO_DIR/home/.config/hypr/hyprlock-theme.conf" ]]; then
    sed -i -E 's|^\$wallpaper = .*|$wallpaper = screenshot|' "$REPO_DIR/home/.config/hypr/hyprlock-theme.conf"
fi

rsync -a --delete "$HOME/.config/kitty/" "$REPO_DIR/home/.config/kitty/"
[[ -f "$HOME/.config/mako/config" ]] && cp -a "$HOME/.config/mako/config" "$REPO_DIR/home/.config/mako/"

for f in mimeapps.list pavucontrol.ini; do
    [[ -f "$HOME/.config/$f" ]] && cp -a "$HOME/.config/$f" "$REPO_DIR/home/.config/$f"
done

if [[ -f "$HOME/.config/pdfcraft/app.ron" ]]; then
    python3 - "$HOME/.config/pdfcraft/app.ron" "$REPO_DIR/home/.config/pdfcraft/app.ron" <<'PY'
import json, re, sys
src, dst = sys.argv[1:]
with open(src, "r", encoding="utf-8") as f:
    s = f.read()
m = re.search(r'("pdfcraft":\s*)"((?:\\.|[^"\\])*)"', s)
if m:
    data = json.loads(json.loads('"' + m.group(2) + '"'))
    data["author"] = ""
    data["recent"] = []
    data["last_views"] = []
    encoded = json.dumps(json.dumps(data, ensure_ascii=False, separators=(",", ":")), ensure_ascii=False)
    s = s[:m.start(2) - 1] + encoded + s[m.end(2) + 1:]
with open(dst, "w", encoding="utf-8") as f:
    f.write(s)
PY
fi

rsync -a --delete --exclude='target/' --exclude='*.bak*' --exclude='*.log' --exclude='.git/' \
    "$HOME/.config/rofi/" "$REPO_DIR/home/.config/rofi/"
[[ -f "$HOME/.config/Thunar/accels.scm" ]] && cp -a "$HOME/.config/Thunar/accels.scm" "$REPO_DIR/home/.config/Thunar/"
[[ -f "$HOME/.config/Thunar/uca.xml" ]] && cp -a "$HOME/.config/Thunar/uca.xml" "$REPO_DIR/home/.config/Thunar/"

rsync -a --delete --exclude='target/' --exclude='*.bak*' --exclude='*.log' --exclude='.git/' \
    "$HOME/.config/waybar/" "$REPO_DIR/home/.config/waybar/"
if [[ -f "$REPO_DIR/home/.config/waybar/config.jsonc" ]]; then
    python3 - "$REPO_DIR/home/.config/waybar/config.jsonc" <<'PY'
import json, sys
path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    cfg = json.load(f)
if isinstance(cfg.get("backlight"), dict):
    cfg["backlight"].pop("device", None)
if isinstance(cfg.get("battery"), dict):
    cfg["battery"].pop("bat", None)
    cfg["battery"].pop("adapter", None)
with open(path, "w", encoding="utf-8") as f:
    json.dump(cfg, f, indent=2, ensure_ascii=False)
    f.write("\n")
PY
fi

[[ -f "$HOME/.config/xfce4/xfconf/xfce-perchannel-xml/thunar.xml" ]] && \
    cp -a "$HOME/.config/xfce4/xfconf/xfce-perchannel-xml/thunar.xml" \
    "$REPO_DIR/home/.config/xfce4/xfconf/xfce-perchannel-xml/"

if [[ -f "$HOME/.config/wordcraft/ui.json" ]]; then
    python3 - "$HOME/.config/wordcraft/ui.json" "$REPO_DIR/home/.config/wordcraft/ui.json" <<'PY'
import json, sys
with open(sys.argv[1], "r", encoding="utf-8") as f:
    data = json.load(f)
data["recent"] = []
with open(sys.argv[2], "w", encoding="utf-8", newline="\n") as f:
    json.dump(data, f, indent=2, ensure_ascii=False)
    f.write("\n")
PY
fi

for b in wallfx smooth-brightness asura-launcher; do
    [[ -f "$HOME/.local/bin/$b" ]] && cp -a "$HOME/.local/bin/$b" "$REPO_DIR/home/.local/bin/$b"
done
cp -a "$HOME/.local/share/applications/"*.desktop "$REPO_DIR/home/.local/share/applications/" 2>/dev/null || true
cp -a "$HOME/.local/share/craft-fonts/settings.json" "$HOME/.local/share/craft-fonts/language-packs.json" \
    "$REPO_DIR/home/.local/share/craft-fonts/" 2>/dev/null || true
[[ -f "$HOME/.cargo/bin/samloader" ]] && cp -a "$HOME/.cargo/bin/samloader" "$REPO_DIR/home/.cargo/bin/samloader"

# Betterfox user.js
for p in "$HOME/.config/mozilla/firefox/"*.default-release/user.js "$HOME/.mozilla/firefox/"*.default-release/user.js; do
    if [[ -f "$p" ]]; then
        cp -a "$p" "$REPO_DIR/firefox/user.js"
        break
    fi
done

# Обои и dconf
PICTURES_DIR=""
if command -v xdg-user-dir >/dev/null 2>&1; then
    PICTURES_DIR=$(xdg-user-dir PICTURES 2>/dev/null || true)
fi
[[ -n "$PICTURES_DIR" && -d "$PICTURES_DIR/wallpapers" ]] || PICTURES_DIR="$HOME/Изображения"
if [[ -d "$PICTURES_DIR/wallpapers" ]]; then
    rsync -a --delete --include='*.png' --exclude='*' "$PICTURES_DIR/wallpapers/" "$REPO_DIR/wallpapers/"
fi

if command -v dconf >/dev/null 2>&1; then
    dconf dump / >"$REPO_DIR/dconf/dconf-settings.ini"
fi

# Zapret: синхронизируем только стратегии и списки, НЕ затирая шаблонный conf.env с interface=auto
if [[ -d "$HOME/zapret-discord-youtube-linux" ]]; then
    rsync -a --delete "$HOME/zapret-discord-youtube-linux/custom-strategies/" "$REPO_DIR/zapret/custom-strategies/"
    rsync -a --delete "$HOME/zapret-discord-youtube-linux/user-lists/" "$REPO_DIR/zapret/user-lists/"
fi

# Системные шаблоны: санитизируем $HOME -> __USER_HOME__ и сохраняем только переносимые поля
if [[ -r /etc/ly/startup.sh ]]; then
    sed "s|$HOME|__USER_HOME__|g" /etc/ly/startup.sh >"$REPO_DIR/system/etc/ly/startup.sh"
    chmod +x "$REPO_DIR/system/etc/ly/startup.sh"
fi
if [[ -r /etc/systemd/system/zapret_discord_youtube.service ]]; then
    sed "s|$HOME|__USER_HOME__|g" /etc/systemd/system/zapret_discord_youtube.service >"$REPO_DIR/zapret/zapret_discord_youtube.service"
fi
[[ -r /etc/udev/rules.d/99-uinput.rules ]] && cp -a /etc/udev/rules.d/99-uinput.rules "$REPO_DIR/system/etc/udev/rules.d/99-uinput.rules"

log "Текущие изменения в репозитории:"
git -C "$REPO_DIR" status -s
