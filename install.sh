#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

SKIP_PACKAGES=false
REBUILD=false
NO_SYSTEM=false
ONLY_FIREFOX=false

usage() {
    cat <<'EOF'
Использование: ./install.sh [опции]

Опции:
  --skip-packages   Не устанавливать пакеты через pacman и yay
  --rebuild         Пересобрать Rust-утилиты (wallfx, waybar-widgets, asura-launcher, smooth-brightness) из исходников
  --no-system       Не менять системные файлы (/etc/ly, /etc/udev, /etc/vconsole.conf, systemd system units)
  --only-firefox    Установить только Betterfox user.js в профиль Firefox
  -h, --help        Показать эту справку
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-packages) SKIP_PACKAGES=true ;;
        --rebuild)       REBUILD=true ;;
        --no-system)     NO_SYSTEM=true ;;
        --only-firefox)  ONLY_FIREFOX=true ;;
        -h|--help)       usage; exit 0 ;;
        *)               echo "Неизвестная опция: $1" >&2; usage; exit 1 ;;
    esac
    shift
done

log()  { printf '\033[1;34m[install]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[warn]\033[0m %s\n' "$*" >&2; }

deploy_firefox() {
    local src="$REPO_DIR/firefox/user.js"
    [[ -f "$src" ]] || { warn "Не найден $src"; return 0; }

    local found=0
    local root profile_dir
    for root in "$HOME/.config/mozilla/firefox" "$HOME/.mozilla/firefox"; do
        [[ -d "$root" ]] || continue
        while IFS= read -r -d '' profile_dir; do
            found=1
            if [[ -f "$profile_dir/prefs.js" && ! -f "$profile_dir/prefs.js.bak.pre-betterfox" ]]; then
                cp -p "$profile_dir/prefs.js" "$profile_dir/prefs.js.bak.pre-betterfox"
            fi
            cp -f "$src" "$profile_dir/user.js"
            log "Betterfox user.js установлен в $profile_dir"
        done < <(find "$root" -maxdepth 1 -mindepth 1 -type d -name '*.default-release' -print0)
    done

    if [[ $found -eq 0 ]]; then
        warn "Профиль Firefox (*.default-release) не найден. Запустите Firefox один раз и повторите: ./install.sh --only-firefox"
    fi
}

adapt_home_in_file() {
    local file="$1"
    [[ -f "$file" && "$HOME" != "/home/samerim" ]] || return 0
    python3 - "$file" "$HOME" <<'PY'
import sys
path, home = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8", errors="surrogateescape") as f:
    text = f.read()
new_text = text.replace("/home/samerim", home)
if new_text != text:
    with open(path, "w", encoding="utf-8", errors="surrogateescape") as f:
        f.write(new_text)
PY
}

if $ONLY_FIREFOX; then
    deploy_firefox
    exit 0
fi

# 1. Пакеты pacman + AUR
if ! $SKIP_PACKAGES; then
    if [[ -f "$REPO_DIR/packages/pkglist-repo.txt" ]]; then
        log "Установка официальных пакетов из pkglist-repo.txt..."
        sudo pacman -S --needed --noconfirm - < <(grep -vE '^\s*(#|$)' "$REPO_DIR/packages/pkglist-repo.txt")
    fi

    if [[ -f "$REPO_DIR/packages/pkglist-aur.txt" ]] && [[ -s "$REPO_DIR/packages/pkglist-aur.txt" ]]; then
        if ! command -v yay >/dev/null 2>&1; then
            log "Установка yay из AUR..."
            sudo pacman -S --needed --noconfirm base-devel git
            tmpdir="$(mktemp -d)"
            git clone https://aur.archlinux.org/yay-bin.git "$tmpdir/yay-bin"
            (cd "$tmpdir/yay-bin" && makepkg -si --noconfirm)
            rm -rf "$tmpdir"
        fi
        log "Установка AUR-пакетов из pkglist-aur.txt..."
        yay -S --needed --noconfirm - < <(grep -vE '^\s*(#|$)' "$REPO_DIR/packages/pkglist-aur.txt")
    fi
else
    log "Пропуск установки пакетов (--skip-packages)."
fi

# 2. Домашние конфиги, бинарники и обои
log "Копирование пользовательских конфигов и утилит в $HOME..."
mkdir -p "$HOME/.config" "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/craft-fonts" "$HOME/.cargo/bin" "$HOME/Изображения/wallpapers"

rsync -a "$REPO_DIR/home/" "$HOME/"
cp -a "$REPO_DIR/wallpapers/"*.png "$HOME/Изображения/wallpapers/"

chmod +x \
    "$HOME/.local/bin/wallfx" \
    "$HOME/.local/bin/smooth-brightness" \
    "$HOME/.local/bin/asura-launcher" \
    "$HOME/.config/waybar/waybar-widgets" \
    "$HOME/.cargo/bin/samloader" \
    "$HOME/.config/hypr/"*.sh 2>/dev/null || true

if [[ "$HOME" != "/home/samerim" ]]; then
    log "Адаптация путей /home/samerim -> $HOME..."
    for f in \
        "$HOME/.config/hypr/hyprlock.conf" \
        "$HOME/.config/hypr/hyprlock-theme.conf" \
        "$HOME/.config/hypr/hypridle.conf" \
        "$HOME/.config/hypr/hyprland.lua" \
        "$HOME/.config/hypr/sync-lock-theme.sh" \
        "$HOME/.config/hypr/lock.sh" \
        "$HOME/.config/hypr/idle-brightness.sh" \
        "$HOME/.config/fastfetch/config.jsonc" \
        "$HOME/.config/rofi/config.rasi" \
        "$HOME/.config/rofi/theme.rasi" \
        "$HOME/.config/fish/fish_variables"; do
        adapt_home_in_file "$f"
    done
fi

# 3. Пересборка из исходников при --rebuild
if $REBUILD; then
    command -v cargo >/dev/null 2>&1 || { warn "cargo не найден; пересборка невозможна"; exit 1; }
    rebuild_rust() {
        local crate_dir="$1"
        local bin_name="$2"
        local dest_path="$3"
        log "Сборка $bin_name из $crate_dir..."
        (
            cd "$crate_dir"
            cargo build --release
            install -Dm755 "target/release/$bin_name" "$dest_path"
            cargo clean
        )
    }
    rebuild_rust "$HOME/.config/hypr/wallfx-rs"            "wallfx"            "$HOME/.local/bin/wallfx"
    rebuild_rust "$HOME/.config/hypr/smooth-brightness-rs" "smooth-brightness" "$HOME/.local/bin/smooth-brightness"
    rebuild_rust "$HOME/.config/waybar/widgets-rs"         "waybar-widgets"    "$HOME/.config/waybar/waybar-widgets"
    rebuild_rust "$HOME/.config/rofi/launcher-rs"          "asura-launcher"    "$HOME/.local/bin/asura-launcher"

    if command -v gcc >/dev/null 2>&1 && pkg-config --exists rofi glib-2.0 2>/dev/null; then
        log "Сборка rofi/druncalc.so..."
        gcc -shared -fPIC -O2 "$HOME/.config/rofi/druncalc/druncalc.c" \
            $(pkg-config --cflags --libs rofi glib-2.0) -lm \
            -o "$HOME/.config/rofi/druncalc/druncalc.so"
    else
        log "Заголовки rofi для gcc не найдены — оставлен готовый druncalc.so."
    fi
fi

# 4. Betterfox
deploy_firefox

# 5. dconf и кэш шрифтов
if command -v dconf >/dev/null 2>&1 && [[ -f "$REPO_DIR/dconf/dconf-settings.ini" ]]; then
    log "Восстановление dconf..."
    dconf load / < "$REPO_DIR/dconf/dconf-settings.ini"
fi
if command -v fc-cache >/dev/null 2>&1; then
    fc-cache -f >/dev/null 2>&1 || true
fi

# 6. Zapret
ZAPRET_DIR="$HOME/zapret-discord-youtube-linux"
if [[ ! -d "$ZAPRET_DIR" ]]; then
    log "Клонирование zapret-discord-youtube-linux..."
    git clone https://github.com/Sergeydigl3/zapret-discord-youtube-linux.git "$ZAPRET_DIR" || warn "Не удалось клонировать zapret"
fi
if [[ -d "$ZAPRET_DIR" ]]; then
    cp -f "$REPO_DIR/zapret/conf.env" "$ZAPRET_DIR/conf.env"
    mkdir -p "$ZAPRET_DIR/custom-strategies" "$ZAPRET_DIR/user-lists"
    cp -a "$REPO_DIR/zapret/custom-strategies/"* "$ZAPRET_DIR/custom-strategies/" 2>/dev/null || true
    cp -a "$REPO_DIR/zapret/user-lists/"* "$ZAPRET_DIR/user-lists/" 2>/dev/null || true
fi

# 7. Системные настройки (/etc) и сервисы
if ! $NO_SYSTEM; then
    log "Установка системных конфигов /etc (потребуется sudo)..."
    sudo mkdir -p /etc/ly /etc/udev/rules.d

    if [[ -f /etc/ly/config.ini && ! -f /etc/ly/config.ini.bak.pre-restore ]]; then
        sudo cp -p /etc/ly/config.ini /etc/ly/config.ini.bak.pre-restore
    fi
    if [[ -f /etc/ly/startup.sh && ! -f /etc/ly/startup.sh.bak.pre-restore ]]; then
        sudo cp -p /etc/ly/startup.sh /etc/ly/startup.sh.bak.pre-restore
    fi

    sudo install -Dm644 "$REPO_DIR/system/etc/ly/config.ini" /etc/ly/config.ini
    tmp_ly_startup="$(mktemp)"
    cp "$REPO_DIR/system/etc/ly/startup.sh" "$tmp_ly_startup"
    adapt_home_in_file "$tmp_ly_startup"
    sudo install -Dm755 "$tmp_ly_startup" /etc/ly/startup.sh
    rm -f "$tmp_ly_startup"

    sudo install -Dm644 "$REPO_DIR/system/etc/udev/rules.d/99-uinput.rules" /etc/udev/rules.d/99-uinput.rules
    sudo install -Dm644 "$REPO_DIR/system/etc/vconsole.conf" /etc/vconsole.conf
    sudo udevadm control --reload-rules && sudo udevadm trigger || true

    if [[ -f "$REPO_DIR/zapret/zapret_discord_youtube.service" && -d "$ZAPRET_DIR" ]]; then
        tmp_zapret_unit="$(mktemp)"
        cp "$REPO_DIR/zapret/zapret_discord_youtube.service" "$tmp_zapret_unit"
        adapt_home_in_file "$tmp_zapret_unit"
        sudo install -Dm644 "$tmp_zapret_unit" /etc/systemd/system/zapret_discord_youtube.service
        rm -f "$tmp_zapret_unit"
        sudo systemctl daemon-reload
        sudo systemctl enable zapret_discord_youtube.service || true
    fi

    if systemctl list-unit-files 'ly@.service' >/dev/null 2>&1; then
        sudo systemctl disable getty@tty2.service >/dev/null 2>&1 || true
        sudo systemctl enable ly@tty2.service || true
    fi
else
    log "Пропуск системных файлов /etc (--no-system)."
fi

# 8. Пользовательские сервисы и синхронизация темы блокировки
systemctl --user enable --now hypridle.service 2>/dev/null || systemctl --user enable hypridle.service 2>/dev/null || true
if [[ -x "$HOME/.config/hypr/sync-lock-theme.sh" ]]; then
    "$HOME/.config/hypr/sync-lock-theme.sh" || true
fi

log "Готово! Перезайдите в сессию или перезагрузите ПК, чтобы все настройки (включая ly и systemd) вступили в силу."
