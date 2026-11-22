#!/usr/bin/env bash
set -euo pipefail

if [[ $EUID -eq 0 ]]; then
    echo "Ошибка: не запускайте install.sh от root / через sudo. Скрипт сам запросит sudo при необходимости." >&2
    exit 1
fi

REPO_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

CORE_ONLY=false
SKIP_PACKAGES=false
REBUILD=false
NO_SYSTEM=false
NO_ZAPRET=false
ONLY_FIREFOX=false
DRY_RUN=false

usage() {
    cat <<'EOF'
Использование: ./install.sh [опции]

Универсальный установщик окружения Hyprland + Asura-Glass с автоопределением железа
(Intel / AMD CPU, NVIDIA / AMD / Intel / гибридная графика, ноутбуки и десктопы,
любые батареи, подсветка, Bluetooth и сетевые интерфейсы, CachyOS и Arch Linux).

Опции:
  --core-only       Установить только ядро окружения (без прикладных программ и AUR)
  --skip-packages   Не устанавливать пакеты через pacman и yay
  --rebuild         Пересобрать все Rust-утилиты и druncalc.so из исходников
  --no-system       Не менять /etc и системные systemd-юниты (без sudo для /etc)
  --no-zapret       Не клонировать и не настраивать zapret-discord-youtube-linux
  --only-firefox    Установить только Betterfox user.js в профиль Firefox
  --dry-run         Показать определённый профиль железа, пакеты и изменения без записи
  -h, --help        Показать эту справку
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --core-only)     CORE_ONLY=true ;;
        --skip-packages) SKIP_PACKAGES=true ;;
        --rebuild)       REBUILD=true ;;
        --no-system)     NO_SYSTEM=true ;;
        --no-zapret)     NO_ZAPRET=true ;;
        --only-firefox)  ONLY_FIREFOX=true ;;
        --dry-run)       DRY_RUN=true ;;
        -h|--help)       usage; exit 0 ;;
        *)               echo "Неизвестная опция: $1" >&2; usage; exit 1 ;;
    esac
    shift
done

log()  { printf '\033[1;34m[install]\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m[warn]\033[0m %s\n' "$*" >&2; }

# ---------------------------------------------------------------------------
# Автоопределение железа и окружения
# ---------------------------------------------------------------------------
detect_cpu_vendor() {
    local v
    v=$(awk -F': *' '/^vendor_id/{print $2; exit}' /proc/cpuinfo 2>/dev/null || true)
    case "$v" in
        AuthenticAMD) echo "amd" ;;
        GenuineIntel) echo "intel" ;;
        *)            echo "other" ;;
    esac
}

detect_gpus() {
    HAS_NVIDIA=false
    HAS_AMD_GPU=false
    HAS_INTEL_GPU=false
    NVIDIA_ONLY=false

    if command -v lspci >/dev/null 2>&1; then
        local gpu_lines
        gpu_lines=$(lspci -nn 2>/dev/null | grep -iE 'vga|3d|display' || true)
        if grep -qiE '\[10de:|\bnvidia\b' <<<"$gpu_lines"; then HAS_NVIDIA=true; fi
        if grep -qiE '\[1002:|\b(amd|ati|radeon)\b|advanced micro devices' <<<"$gpu_lines"; then HAS_AMD_GPU=true; fi
        if grep -qiE '\[8086:|\bintel\b' <<<"$gpu_lines"; then HAS_INTEL_GPU=true; fi
    elif [[ -d /sys/class/drm ]]; then
        local vfile vendor
        for vfile in /sys/class/drm/card*/device/vendor; do
            [[ -r "$vfile" ]] || continue
            vendor=$(tr -d '\r\n' <"$vfile" 2>/dev/null || true)
            case "$vendor" in
                0x10de) HAS_NVIDIA=true ;;
                0x1002) HAS_AMD_GPU=true ;;
                0x8086) HAS_INTEL_GPU=true ;;
            esac
        done
    fi

    if $HAS_NVIDIA && ! $HAS_AMD_GPU && ! $HAS_INTEL_GPU; then
        NVIDIA_ONLY=true
    fi
}

detect_battery() {
    local d t s name
    local -a bats=()
    for d in /sys/class/power_supply/*; do
        [[ -d "$d" && -r "$d/type" && -r "$d/capacity" ]] || continue
        t=$(tr -d '\r\n' <"$d/type" 2>/dev/null || true)
        [[ "$t" == "Battery" ]] || continue
        if [[ -r "$d/scope" ]]; then
            s=$(tr -d '\r\n' <"$d/scope" 2>/dev/null || true)
            [[ "$s" == "Device" ]] && continue
        fi
        bats+=("$(basename "$d")")
    done
    if [[ ${#bats[@]} -gt 0 ]]; then
        IFS=$'\n' read -r -d '' -a bats < <(printf '%s\n' "${bats[@]}" | sort && printf '\0') || true
        name="${bats[0]}"
        echo "$name"
    fi
}

detect_ac() {
    local d t
    local -a mains=()
    for d in /sys/class/power_supply/*; do
        [[ -d "$d" && -r "$d/type" ]] || continue
        t=$(tr -d '\r\n' <"$d/type" 2>/dev/null || true)
        [[ "$t" == "Mains" ]] || continue
        mains+=("$(basename "$d")")
    done
    if [[ ${#mains[@]} -gt 0 ]]; then
        IFS=$'\n' read -r -d '' -a mains < <(printf '%s\n' "${mains[@]}" | sort && printf '\0') || true
        echo "${mains[0]}"
    fi
}

detect_backlight() {
    local root="/sys/class/backlight"
    [[ -d "$root" ]] || return 0
    local pref
    for pref in intel_backlight amdgpu_bl0 amdgpu_bl1 nvidia_0 nvidia_wmi_ec_backlight; do
        if [[ -f "$root/$pref/brightness" && -f "$root/$pref/max_brightness" ]]; then
            echo "$pref"
            return 0
        fi
    done
    local d
    local -a devs=()
    for d in "$root"/*; do
        [[ -d "$d" && -f "$d/brightness" && -f "$d/max_brightness" ]] || continue
        devs+=("$(basename "$d")")
    done
    if [[ ${#devs[@]} -gt 0 ]]; then
        IFS=$'\n' read -r -d '' -a devs < <(printf '%s\n' "${devs[@]}" | sort && printf '\0') || true
        echo "${devs[0]}"
    fi
}

detect_bluetooth() {
    shopt -s nullglob
    local -a hcis=(/sys/class/bluetooth/hci*)
    shopt -u nullglob
    if [[ ${#hcis[@]} -gt 0 ]]; then
        echo "true"
    else
        echo "false"
    fi
}

is_virtual_iface() {
    case "$1" in
        lo|tun*|tap*|wg*|tailscale*|docker*|br-*|veth*|virbr*) return 0 ;;
        *) return 1 ;;
    esac
}

detect_net_iface() {
    local iface=""
    if command -v ip >/dev/null 2>&1; then
        while IFS= read -r cand; do
            [[ -n "$cand" ]] || continue
            if ! is_virtual_iface "$cand"; then
                iface="$cand"
                break
            fi
        done < <(ip -o route show default 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="dev"){print $(i+1); break}}' || true)
    fi

    if [[ -z "$iface" ]] && command -v nmcli >/dev/null 2>&1; then
        iface=$(nmcli -t -f DEVICE,TYPE,STATE dev status 2>/dev/null \
            | awk -F: '($2=="ethernet" || $2=="wifi") && $3 ~ /^connected/ {print $1; exit}' || true)
        if [[ -z "$iface" ]]; then
            iface=$(nmcli -t -f DEVICE,TYPE dev status 2>/dev/null \
                | awk -F: '($2=="ethernet" || $2=="wifi") {print $1; exit}' || true)
        fi
    fi

    if [[ -z "$iface" ]] && [[ -d /sys/class/net ]]; then
        local d b
        for d in /sys/class/net/*; do
            b=$(basename "$d")
            is_virtual_iface "$b" && continue
            iface="$b"
            break
        done
    fi

    echo "${iface:-auto}"
}

detect_fw_backend() {
    if command -v nft >/dev/null 2>&1; then
        echo "nftables"
    elif command -v iptables >/dev/null 2>&1; then
        echo "iptables"
    else
        echo "nftables"
    fi
}

detect_pictures_dir() {
    local dir=""
    if command -v xdg-user-dir >/dev/null 2>&1; then
        dir=$(xdg-user-dir PICTURES 2>/dev/null || true)
    fi
    if [[ -z "$dir" || "$dir" == "$HOME" ]]; then
        if [[ -d "$HOME/Изображения" ]]; then
            dir="$HOME/Изображения"
        else
            dir="$HOME/Pictures"
        fi
    fi
    echo "$dir"
}

# ---------------------------------------------------------------------------
# Помощники для пакетов и деплоя
# ---------------------------------------------------------------------------
collect_repo_packages() {
    local -a lists=("$REPO_DIR/packages/pkglist-core.txt")
    if ! $CORE_ONLY && [[ -f "$REPO_DIR/packages/pkglist-apps.txt" ]]; then
        lists+=("$REPO_DIR/packages/pkglist-apps.txt")
    fi

    local raw_list exclude_file sync_avail
    raw_list=$(grep -hEv '^\s*(#|$)' "${lists[@]}" 2>/dev/null | sort -u || true)

    exclude_file="$REPO_DIR/packages/hardware-exclude.regex"
    if [[ -f "$exclude_file" ]]; then
        local regex_clean
        regex_clean=$(grep -Ev '^\s*(#|$)' "$exclude_file" || true)
        if [[ -n "$regex_clean" ]]; then
            raw_list=$(grep -Evf <(printf '%s\n' "$regex_clean") <<<"$raw_list" || true)
        fi
    fi

    if command -v pacman >/dev/null 2>&1; then
        sync_avail=$(pacman -Ssq 2>/dev/null | sort -u || true)
        if [[ -n "$sync_avail" ]]; then
            comm -12 <(printf '%s\n' "$raw_list" | sort -u) <(printf '%s\n' "$sync_avail")
            return 0
        fi
    fi
    printf '%s\n' "$raw_list"
}

collect_remove_packages() {
    local remove_file="$REPO_DIR/packages/pkglist-remove.txt"
    [[ -f "$remove_file" ]] || return 0
    command -v pacman >/dev/null 2>&1 || return 0
    local pkg
    while IFS= read -r pkg; do
        [[ -n "$pkg" ]] || continue
        if pacman -Q "$pkg" >/dev/null 2>&1; then
            echo "$pkg"
        fi
    done < <(grep -Ev '^\s*(#|$)' "$remove_file" | sort -u || true)
}

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

replace_user_home_placeholder() {
    local file="$1"
    [[ -f "$file" ]] || return 0
    python3 - "$file" "$HOME" <<'PY'
import sys
path, home = sys.argv[1], sys.argv[2]
with open(path, "r", encoding="utf-8", errors="surrogateescape") as f:
    text = f.read()
new_text = text.replace("__USER_HOME__", home)
if new_text != text:
    with open(path, "w", encoding="utf-8", errors="surrogateescape") as f:
        f.write(new_text)
PY
}

if $ONLY_FIREFOX; then
    deploy_firefox
    exit 0
fi

# ---------------------------------------------------------------------------
# Сбор профиля железа
# ---------------------------------------------------------------------------
CPU_VENDOR=$(detect_cpu_vendor)
detect_gpus
BAT_DEV=$(detect_battery || true)
AC_DEV=$(detect_ac || true)
BACKLIGHT_DEV=$(detect_backlight || true)
HAS_BT=$(detect_bluetooth)
NET_IFACE=$(detect_net_iface)
FW_BACKEND=$(detect_fw_backend)
PICTURES_DIR=$(detect_pictures_dir)

FORM_FACTOR="Десктоп"
if [[ -n "$BAT_DEV" || -n "$BACKLIGHT_DEV" ]]; then
    FORM_FACTOR="Ноутбук"
fi

log "Определённый профиль системы:"
echo "  Форм-фактор:      $FORM_FACTOR"
echo "  CPU:              $CPU_VENDOR"
echo "  GPU:              nvidia=$HAS_NVIDIA amd=$HAS_AMD_GPU intel=$HAS_INTEL_GPU (nvidia_only=$NVIDIA_ONLY)"
echo "  Батарея / AC:     ${BAT_DEV:-нет} / ${AC_DEV:-нет}"
echo "  Подсветка:        ${BACKLIGHT_DEV:-нет}"
echo "  Bluetooth:        $HAS_BT"
echo "  Сеть / Firewall:  $NET_IFACE / $FW_BACKEND"
echo "  Изображения:      $PICTURES_DIR"

if $DRY_RUN; then
    echo ""
    if ! $SKIP_PACKAGES; then
        mapfile -t DRY_REPO_PKGS < <(collect_repo_packages)
        echo "Пакеты pacman к установке (${#DRY_REPO_PKGS[@]}):"
        printf '  %s\n' "${DRY_REPO_PKGS[@]}"
        if ! $CORE_ONLY && [[ -f "$REPO_DIR/packages/pkglist-aur.txt" ]]; then
            echo "AUR-пакеты из pkglist-aur.txt:"
            grep -Ev '^\s*(#|$)' "$REPO_DIR/packages/pkglist-aur.txt" | sed 's/^/  /' || true
        fi
        mapfile -t DRY_REMOVE_PKGS < <(collect_remove_packages)
        if [[ ${#DRY_REMOVE_PKGS[@]} -gt 0 ]]; then
            echo "Пакеты к удалению (${#DRY_REMOVE_PKGS[@]}):"
            printf '  %s\n' "${DRY_REMOVE_PKGS[@]}"
        fi
    else
        echo "Пакеты: пропущено (--skip-packages)"
    fi
    echo ""
    echo "Планируемые адаптации:"
    echo "  waybar/config.jsonc:  убрать отключённые модули (backlight=${BACKLIGHT_DEV:-off}, battery=${BAT_DEV:-off}, bluetooth=$HAS_BT)"
    echo "  hypr/host.lua:        nvidia_only=$NVIDIA_ONLY"
    echo "  hypr/hypridle.conf:   backlight_listener=${BACKLIGHT_DEV:-off}"
    echo "  fastfetch:            battery_module=${BAT_DEV:-off}"
    if ! $NO_ZAPRET; then
        echo "  zapret/conf.env:      interface=$NET_IFACE, firewall_backend=$FW_BACKEND"
    fi
    if ! $NO_SYSTEM; then
        echo "  /etc/ly/config.ini:   battery_id=${BAT_DEV:-null}, brightness_keys=$([[ -n "$BACKLIGHT_DEV" ]] && echo F5/F6 || echo null)"
        echo "  systemd services:     ly@tty2.service, nvidia-power=$HAS_NVIDIA, bluetooth=$HAS_BT, zapret=$(! $NO_ZAPRET && echo true || echo false)"
    fi
    exit 0
fi

# ---------------------------------------------------------------------------
# Запрос sudo один раз (если требуются системные действия)
# ---------------------------------------------------------------------------
SUDO_KEEPALIVE_PID=""
cleanup_sudo() {
    if [[ -n "$SUDO_KEEPALIVE_PID" ]]; then
        kill "$SUDO_KEEPALIVE_PID" 2>/dev/null || true
    fi
}
trap cleanup_sudo EXIT

if ! $SKIP_PACKAGES || ! $NO_SYSTEM; then
    log "Запрос прав sudo..."
    sudo -v
    (
        while true; do
            sleep 50
            sudo -n true 2>/dev/null || exit 0
        done
    ) &
    SUDO_KEEPALIVE_PID=$!
fi

# ---------------------------------------------------------------------------
# 1. Пакеты pacman + AUR
# ---------------------------------------------------------------------------
if ! $SKIP_PACKAGES; then
    mapfile -t REMOVE_PKGS < <(collect_remove_packages)
    if [[ ${#REMOVE_PKGS[@]} -gt 0 ]]; then
        log "Удаление лишних пакетов (${REMOVE_PKGS[*]})..."
        if ! sudo pacman -Rns --noconfirm "${REMOVE_PKGS[@]}" 2>/dev/null; then
            for pkg in "${REMOVE_PKGS[@]}"; do
                if pacman -Q "$pkg" >/dev/null 2>&1; then
                    sudo pacman -Rns --noconfirm "$pkg" || warn "Не удалось удалить пакет '$pkg' (пропуск)."
                fi
            done
        fi
    fi
    rm -rf "$HOME/.config/micro" "$HOME/.config/meld" "$HOME/.config/alacritty"

    mapfile -t REPO_PKGS < <(collect_repo_packages)
    if [[ ${#REPO_PKGS[@]} -gt 0 ]]; then
        log "Установка официальных пакетов (${#REPO_PKGS[@]} шт.)..."
        sudo pacman -S --needed --noconfirm "${REPO_PKGS[@]}"
    fi

    if ! $CORE_ONLY && [[ -f "$REPO_DIR/packages/pkglist-aur.txt" ]] && [[ -s "$REPO_DIR/packages/pkglist-aur.txt" ]]; then
        if ! command -v yay >/dev/null 2>&1; then
            log "Установка yay из AUR..."
            sudo pacman -S --needed --noconfirm base-devel git
            tmpdir="$(mktemp -d)"
            git clone https://aur.archlinux.org/yay-bin.git "$tmpdir/yay-bin"
            (cd "$tmpdir/yay-bin" && makepkg -si --noconfirm)
            rm -rf "$tmpdir"
        fi

        mapfile -t RAW_AUR < <(grep -Ev '^\s*(#|$)' "$REPO_DIR/packages/pkglist-aur.txt" || true)
        AUR_PKGS=()
        for pkg in "${RAW_AUR[@]}"; do
            if yay -Si "$pkg" >/dev/null 2>&1; then
                AUR_PKGS+=("$pkg")
            else
                warn "AUR-пакет '$pkg' не найден — пропуск."
            fi
        done
        if [[ ${#AUR_PKGS[@]} -gt 0 ]]; then
            log "Установка AUR-пакетов (${AUR_PKGS[*]})..."
            yay -S --needed --noconfirm "${AUR_PKGS[@]}"
        fi
    fi
else
    log "Пропуск установки пакетов (--skip-packages)."
fi

# ---------------------------------------------------------------------------
# 2. Домашние конфиги, бинарники, обои и адаптация под железо
# ---------------------------------------------------------------------------
log "Копирование пользовательских конфигов и утилит в $HOME..."
mkdir -p "$HOME/.config" "$HOME/.local/bin" "$HOME/.local/share/applications" "$HOME/.local/share/craft-fonts" "$HOME/.cargo/bin" "$PICTURES_DIR/wallpapers"

if [[ -f "$HOME/.config/hypr/hyprland.lua" && ! -f "$HOME/.config/hypr/hyprland.lua.bak.pre-dotfiles" ]]; then
    cp -p "$HOME/.config/hypr/hyprland.lua" "$HOME/.config/hypr/hyprland.lua.bak.pre-dotfiles"
fi

rsync -a "$REPO_DIR/home/" "$HOME/"
cp -a "$REPO_DIR/wallpapers/"*.png "$PICTURES_DIR/wallpapers/"

chmod +x \
    "$HOME/.local/bin/wallfx" \
    "$HOME/.local/bin/smooth-brightness" \
    "$HOME/.local/bin/asura-launcher" \
    "$HOME/.config/waybar/waybar-widgets" \
    "$HOME/.cargo/bin/samloader" \
    "$HOME/.config/hypr/"*.sh 2>/dev/null || true

# host.lua для Hyprland (NVIDIA VA-API только на single-NVIDIA системах с libva-nvidia-driver)
HOST_LUA="$HOME/.config/hypr/host.lua"
if $NVIDIA_ONLY && pacman -Q libva-nvidia-driver >/dev/null 2>&1; then
    cat >"$HOST_LUA" <<'EOF'
-- Auto-generated by install.sh for single-NVIDIA hosts
hl.env("LIBVA_DRIVER_NAME", "nvidia")
hl.env("NVD_BACKEND", "direct")
EOF
else
    cat >"$HOST_LUA" <<'EOF'
-- Auto-generated by install.sh (no host-specific GPU overrides required)
EOF
fi

# Адаптация waybar/config.jsonc
python3 - "$HOME/.config/waybar/config.jsonc" "$BACKLIGHT_DEV" "$BAT_DEV" "$AC_DEV" "$HAS_BT" <<'PY'
import json, sys

path, bl_dev, bat_dev, ac_dev, has_bt_s = sys.argv[1:]
has_bt = (has_bt_s == "true")

with open(path, "r", encoding="utf-8") as f:
    cfg = json.load(f)

mods = cfg.get("modules-right", [])
drop = set()
if not bl_dev:
    drop.add("backlight")
if not bat_dev:
    drop.add("battery")
if not has_bt:
    drop.add("bluetooth")
cfg["modules-right"] = [m for m in mods if m not in drop]

if bl_dev and isinstance(cfg.get("backlight"), dict):
    cfg["backlight"]["device"] = bl_dev
if bat_dev and isinstance(cfg.get("battery"), dict):
    cfg["battery"]["bat"] = bat_dev
    if ac_dev:
        cfg["battery"]["adapter"] = ac_dev

with open(path, "w", encoding="utf-8") as f:
    json.dump(cfg, f, indent=2, ensure_ascii=False)
    f.write("\n")
PY

# Адаптация hypridle.conf (убрать затемнение подсветки, если нет /sys/class/backlight)
if [[ -z "$BACKLIGHT_DEV" && -f "$HOME/.config/hypr/hypridle.conf" ]]; then
    python3 - "$HOME/.config/hypr/hypridle.conf" <<'PY'
import re, sys
path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    text = f.read()
text = re.sub(r'# @backlight-start.*?# @backlight-end\n*', '', text, flags=re.S)
text = text.replace(' && $HOME/.config/hypr/idle-brightness.sh restore', '')
with open(path, "w", encoding="utf-8") as f:
    f.write(text)
PY
fi

# Адаптация fastfetch/config.jsonc
replace_user_home_placeholder "$HOME/.config/fastfetch/config.jsonc"
if [[ -z "$BAT_DEV" && -f "$HOME/.config/fastfetch/config.jsonc" ]]; then
    python3 - "$HOME/.config/fastfetch/config.jsonc" <<'PY'
import json, sys
path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    cfg = json.load(f)
if isinstance(cfg.get("modules"), list):
    cfg["modules"] = [m for m in cfg["modules"] if m != "battery"]
with open(path, "w", encoding="utf-8") as f:
    json.dump(cfg, f, indent=2, ensure_ascii=False)
    f.write("\n")
PY
fi

# ---------------------------------------------------------------------------
# 3. Пересборка из исходников при --rebuild
# ---------------------------------------------------------------------------
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

# ---------------------------------------------------------------------------
# 4. Betterfox, dconf и кэш шрифтов
# ---------------------------------------------------------------------------
deploy_firefox

if command -v dconf >/dev/null 2>&1 && [[ -f "$REPO_DIR/dconf/dconf-settings.ini" ]]; then
    log "Восстановление dconf..."
    dconf load / <"$REPO_DIR/dconf/dconf-settings.ini"
fi
if command -v fc-cache >/dev/null 2>&1; then
    fc-cache -f >/dev/null 2>&1 || true
fi

# ---------------------------------------------------------------------------
# 5. Zapret
# ---------------------------------------------------------------------------
ZAPRET_DIR="$HOME/zapret-discord-youtube-linux"
if ! $NO_ZAPRET; then
    if [[ ! -d "$ZAPRET_DIR/.git" ]]; then
        log "Клонирование zapret-discord-youtube-linux..."
        git clone https://github.com/Sergeydigl3/zapret-discord-youtube-linux.git "$ZAPRET_DIR" || warn "Не удалось клонировать zapret"
    fi
    if [[ -d "$ZAPRET_DIR" ]]; then
        if [[ ! -x "$ZAPRET_DIR/nfqws" || ! -d "$ZAPRET_DIR/zapret-latest" ]]; then
            log "Загрузка зависимостей zapret (nfqws и стратегии)..."
            (cd "$ZAPRET_DIR" && bash ./service.sh download-deps --default) || warn "Не удалось скачать зависимости zapret"
        fi
        mkdir -p "$ZAPRET_DIR/custom-strategies" "$ZAPRET_DIR/user-lists"
        cp -f "$REPO_DIR/zapret/custom-strategies/"* "$ZAPRET_DIR/custom-strategies/" 2>/dev/null || true
        cp -f "$REPO_DIR/zapret/user-lists/"* "$ZAPRET_DIR/user-lists/" 2>/dev/null || true
        chmod 644 "$ZAPRET_DIR/user-lists/"* 2>/dev/null || true
        if [[ -d "$ZAPRET_DIR/zapret-latest/lists" ]]; then
            for f in "$ZAPRET_DIR/user-lists/"*; do
                [[ -f "$f" ]] && ln -f "$f" "$ZAPRET_DIR/zapret-latest/lists/" 2>/dev/null || true
            done
        fi
        cat >"$ZAPRET_DIR/conf.env" <<EOF
interface=$NET_IFACE
gamefiltertcp=true
gamefilterudp=true
strategy=general_alt11.bat
firewall_backend=$FW_BACKEND
EOF
    fi
else
    log "Пропуск zapret (--no-zapret)."
fi

# ---------------------------------------------------------------------------
# 6. Системные настройки (/etc) и systemd
# ---------------------------------------------------------------------------
if ! $NO_SYSTEM; then
    log "Установка системных конфигов /etc..."
    sudo mkdir -p /etc/ly /etc/udev/rules.d

    backup_etc() {
        local dst="$1"
        if [[ -f "$dst" && ! -f "${dst}.bak.pre-restore" ]]; then
            sudo cp -p "$dst" "${dst}.bak.pre-restore"
        fi
    }

    backup_etc /etc/ly/config.ini
    backup_etc /etc/ly/startup.sh
    backup_etc /etc/udev/rules.d/99-uinput.rules

    tmp_ly_cfg="$(mktemp)"
    cp "$REPO_DIR/system/etc/ly/config.ini" "$tmp_ly_cfg"
    python3 - "$tmp_ly_cfg" "$BAT_DEV" "$BACKLIGHT_DEV" <<'PY'
import re, sys
path, bat_dev, bl_dev = sys.argv[1:]
with open(path, "r", encoding="utf-8") as f:
    text = f.read()
text = re.sub(r'^battery_id\s*=.*$', f'battery_id = {bat_dev or "null"}', text, flags=re.M)
b_down = "F5" if bl_dev else "null"
b_up = "F6" if bl_dev else "null"
text = re.sub(r'^brightness_down_key\s*=.*$', f'brightness_down_key = {b_down}', text, flags=re.M)
text = re.sub(r'^brightness_up_key\s*=.*$', f'brightness_up_key = {b_up}', text, flags=re.M)
with open(path, "w", encoding="utf-8") as f:
    f.write(text)
PY
    sudo install -Dm644 "$tmp_ly_cfg" /etc/ly/config.ini
    rm -f "$tmp_ly_cfg"

    tmp_ly_startup="$(mktemp)"
    cp "$REPO_DIR/system/etc/ly/startup.sh" "$tmp_ly_startup"
    replace_user_home_placeholder "$tmp_ly_startup"
    sudo install -Dm755 "$tmp_ly_startup" /etc/ly/startup.sh
    rm -f "$tmp_ly_startup"

    sudo install -Dm644 "$REPO_DIR/system/etc/udev/rules.d/99-uinput.rules" /etc/udev/rules.d/99-uinput.rules

    if [[ ! -f /etc/vconsole.conf ]] || ! cmp -s "$REPO_DIR/system/etc/vconsole.conf" /etc/vconsole.conf; then
        backup_etc /etc/vconsole.conf
        sudo install -Dm644 "$REPO_DIR/system/etc/vconsole.conf" /etc/vconsole.conf
    fi

    sudo udevadm control --reload-rules && sudo udevadm trigger || true

    if ! $NO_ZAPRET && [[ -f "$REPO_DIR/zapret/zapret_discord_youtube.service" && -d "$ZAPRET_DIR" ]]; then
        tmp_zapret_unit="$(mktemp)"
        cp "$REPO_DIR/zapret/zapret_discord_youtube.service" "$tmp_zapret_unit"
        replace_user_home_placeholder "$tmp_zapret_unit"
        sudo install -Dm644 "$tmp_zapret_unit" /etc/systemd/system/zapret_discord_youtube.service
        rm -f "$tmp_zapret_unit"
        sudo systemctl daemon-reload
        sudo systemctl enable zapret_discord_youtube.service || true
    fi

    if systemctl list-unit-files 'ly@.service' >/dev/null 2>&1; then
        # Отключить конфликтующий DM (sddm/gdm/lightdm/greetd и т.д.), если был включён
        if cur_dm=$(systemctl read-link /etc/systemd/system/display-manager.service 2>/dev/null); then
            cur_dm_base=$(basename "$cur_dm")
            if [[ "$cur_dm_base" != ly* ]]; then
                log "Отключение предыдущего display manager ($cur_dm_base)..."
                sudo systemctl disable "$cur_dm_base" >/dev/null 2>&1 || true
            fi
        fi
        sudo systemctl disable getty@tty2.service >/dev/null 2>&1 || true
        sudo systemctl enable ly@tty2.service || true
    fi

    if $HAS_NVIDIA; then
        for u in nvidia-suspend.service nvidia-resume.service nvidia-hibernate.service; do
            if systemctl list-unit-files "$u" >/dev/null 2>&1; then
                sudo systemctl enable "$u" || true
            fi
        done
    fi

    if [[ "$HAS_BT" == "true" ]] && systemctl list-unit-files bluetooth.service >/dev/null 2>&1; then
        sudo systemctl enable bluetooth.service || true
    fi

    for u in NetworkManager.service power-profiles-daemon.service; do
        if systemctl list-unit-files "$u" >/dev/null 2>&1; then
            sudo systemctl enable "$u" || true
        fi
    done
else
    log "Пропуск системных файлов /etc (--no-system)."
fi

# ---------------------------------------------------------------------------
# 7. Пользовательские сервисы и живое применение темы/окружения
# ---------------------------------------------------------------------------
if systemctl --user status >/dev/null 2>&1; then
    systemctl --user disable --now hyprpolkitagent.service >/dev/null 2>&1 || true
    systemctl --user enable --now hypridle.service >/dev/null 2>&1 || true
else
    systemctl --user disable hyprpolkitagent.service >/dev/null 2>&1 || true
    systemctl --user enable hypridle.service >/dev/null 2>&1 || true
fi

if [[ -x "$HOME/.config/hypr/sync-lock-theme.sh" ]]; then
    "$HOME/.config/hypr/sync-lock-theme.sh" || true
fi

if [[ -n "${HYPRLAND_INSTANCE_SIGNATURE:-}" ]]; then
    log "Обновление активной сессии Hyprland..."
    pkill -x waybar mako awww-daemon polkit-gnome-authentication-agent-1 2>/dev/null || true
    [[ -x /usr/lib/polkit-gnome/polkit-gnome-authentication-agent-1 ]] && \
        setsid -f /usr/lib/polkit-gnome/polkit-gnome-authentication-agent-1 >/dev/null 2>&1 </dev/null
    if command -v awww-daemon >/dev/null 2>&1; then
        setsid -f awww-daemon >/dev/null 2>&1 </dev/null
        sleep 0.6
        if [[ -f "$PICTURES_DIR/wallpapers/blue_lake.png" ]]; then
            awww img "$PICTURES_DIR/wallpapers/blue_lake.png" >/dev/null 2>&1 || true
            "$HOME/.local/bin/wallfx" theme "$PICTURES_DIR/wallpapers/blue_lake.png" >/dev/null 2>&1 || true
            "$HOME/.config/hypr/sync-lock-theme.sh" || true
        fi
    fi
    command -v mako >/dev/null 2>&1 && setsid -f mako >/dev/null 2>&1 </dev/null
    command -v waybar >/dev/null 2>&1 && setsid -f waybar >/dev/null 2>&1 </dev/null
    pgrep -f 'wl-paste --type text --watch cliphist' >/dev/null || \
        setsid -f wl-paste --type text --watch cliphist store >/dev/null 2>&1 </dev/null
    pgrep -f 'wl-paste --type image --watch cliphist' >/dev/null || \
        setsid -f wl-paste --type image --watch cliphist store >/dev/null 2>&1 </dev/null
    hyprctl reload >/dev/null 2>&1 || true
    pkill -USR1 -x kitty 2>/dev/null || true
fi

log "Готово! Перезайдите в сессию или перезагрузите ПК, чтобы все системные настройки (включая ly и systemd) вступили в силу."
