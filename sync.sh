#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

log() { printf '\033[1;34m[sync]\033[0m %s\n' "$*"; }

log "Обновление списков пакетов и сервисов..."
pacman -Qqen | sort > "$REPO_DIR/packages/pkglist-repo.txt"
pacman -Qqem | sort > "$REPO_DIR/packages/pkglist-aur.txt"
systemctl list-unit-files --state=enabled --no-legend --no-pager | awk '{print $1}' | sort > "$REPO_DIR/packages/systemd-enabled.txt"
systemctl --user list-unit-files --state=enabled --no-legend --no-pager | awk '{print $1}' | sort > "$REPO_DIR/packages/systemd-user-enabled.txt"

log "Обновление домашних конфигов, исходников и бинарников..."
for f in .bashrc .bash_profile .bash_logout .nvidia-settings-rc; do
    [[ -f "$HOME/$f" ]] && cp -a "$HOME/$f" "$REPO_DIR/home/$f"
done

cp -a "$HOME/.config/btop/btop.conf" "$REPO_DIR/home/.config/btop/"
if [[ -d "$HOME/.config/btop/themes" ]] && [[ -n "$(ls -A "$HOME/.config/btop/themes" 2>/dev/null)" ]]; then
    cp -a "$HOME/.config/btop/themes" "$REPO_DIR/home/.config/btop/"
fi

rsync -a --delete "$HOME/.config/fastfetch/" "$REPO_DIR/home/.config/fastfetch/"
rsync -a --delete "$HOME/.config/fish/" "$REPO_DIR/home/.config/fish/"
cp -a "$HOME/.config/fontconfig/conf.d/99-doc-fonts.conf" "$REPO_DIR/home/.config/fontconfig/conf.d/"

rsync -a --delete --exclude='target/' --exclude='*.bak*' --exclude='*.log' --exclude='.git/' \
    "$HOME/.config/hypr/" "$REPO_DIR/home/.config/hypr/"
rsync -a --delete "$HOME/.config/kitty/" "$REPO_DIR/home/.config/kitty/"
cp -a "$HOME/.config/mako/config" "$REPO_DIR/home/.config/mako/"

for f in mimeapps.list pavucontrol.ini QtProject.conf user-dirs.dirs user-dirs.locale; do
    [[ -f "$HOME/.config/$f" ]] && cp -a "$HOME/.config/$f" "$REPO_DIR/home/.config/$f"
done

cp -a "$HOME/.config/pdfcraft/app.ron" "$REPO_DIR/home/.config/pdfcraft/"
rsync -a --delete --exclude='target/' --exclude='*.bak*' --exclude='*.log' --exclude='.git/' \
    "$HOME/.config/rofi/" "$REPO_DIR/home/.config/rofi/"
cp -a "$HOME/.config/Thunar/accels.scm" "$HOME/.config/Thunar/uca.xml" "$REPO_DIR/home/.config/Thunar/"
rsync -a --delete --exclude='target/' --exclude='*.bak*' --exclude='*.log' --exclude='.git/' \
    "$HOME/.config/waybar/" "$REPO_DIR/home/.config/waybar/"
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

# Wallpapers, dconf, zapret, /etc
rsync -a --delete --include='*.png' --exclude='*' "$HOME/Изображения/wallpapers/" "$REPO_DIR/wallpapers/"
if command -v dconf >/dev/null 2>&1; then
    dconf dump / > "$REPO_DIR/dconf/dconf-settings.ini"
fi

if [[ -d "$HOME/zapret-discord-youtube-linux" ]]; then
    cp -a "$HOME/zapret-discord-youtube-linux/conf.env" "$REPO_DIR/zapret/conf.env"
    rsync -a --delete "$HOME/zapret-discord-youtube-linux/custom-strategies/" "$REPO_DIR/zapret/custom-strategies/"
    rsync -a --delete "$HOME/zapret-discord-youtube-linux/user-lists/" "$REPO_DIR/zapret/user-lists/"
fi
[[ -r /etc/systemd/system/zapret_discord_youtube.service ]] && cp -a /etc/systemd/system/zapret_discord_youtube.service "$REPO_DIR/zapret/zapret_discord_youtube.service"

[[ -r /etc/ly/config.ini ]] && cp -a /etc/ly/config.ini "$REPO_DIR/system/etc/ly/config.ini"
[[ -r /etc/ly/startup.sh ]] && cp -a /etc/ly/startup.sh "$REPO_DIR/system/etc/ly/startup.sh"
[[ -r /etc/udev/rules.d/99-uinput.rules ]] && cp -a /etc/udev/rules.d/99-uinput.rules "$REPO_DIR/system/etc/udev/rules.d/99-uinput.rules"
[[ -r /etc/vconsole.conf ]] && cp -a /etc/vconsole.conf "$REPO_DIR/system/etc/vconsole.conf"

log "Текущие изменения в репозитории:"
git -C "$REPO_DIR" status -s
