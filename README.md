# CachyOS / Arch Linux + Hyprland + Asura-Glass Dotfiles

Универсальная, аппаратно-независимая сборка окружения **Hyprland (Lua)** для **любого ПК или ноутбука** на базе **CachyOS / Arch Linux** (без паролей, токенов, куки и привязки к конкретному железу).

## Поддержка любого железа (автонастройка в `install.sh`)
Скрипт `install.sh` автоматически определяет текущую машину при установке и адаптирует все конфиги, сервисы и виджеты:
* **CPU (Intel / AMD):** не устанавливает чужой микрокод, а монитор системы (`waybar-widgets system`) считывает температуру через `k10temp`, `zenpower`, `coretemp` и thermal zones.
* **GPU (NVIDIA / AMD / Intel / гибридная графика):**
  * Не устанавливает конфликтующие видеодрайверы (`nvidia-*`, `vulkan-*`, `intel-media-*`) — используются уже установленные драйверы системы.
  * Для систем с одной NVIDIA автоматически генерируется `~/.config/hypr/host.lua` с `LIBVA_DRIVER_NAME=nvidia` и `NVD_BACKEND=direct`, а также включаются `nvidia-{suspend,resume,hibernate}.service`.
  * Монитор GPU в `waybar-widgets system` поддерживает **NVIDIA** (`nvidia-smi`), **AMD** (`amdgpu` sysfs) и **Intel** (`i915` / `xe` sysfs).
* **Десктоп / Ноутбук:**
  * Динамически определяет системную батарею (`/sys/class/power_supply/*`, игнорируя беспроводные мыши/гарнитуры `scope=Device`) и подсветку экрана (`/sys/class/backlight/*`).
  * На десктопе автоматически скрывает `backlight` / `battery` в Waybar и `ly`, убирает ноутбучный dim-listener из `hypridle.conf` и батарею из `fastfetch`, а `smooth-brightness` завершается без ошибки.
  * Виджет раскладки `waybar-widgets language` сканирует любые клавиатурные LED-индикаторы (`/sys/class/leds/*::{capslock,numlock}`).
* **Сеть и Bluetooth:**
  * Скрывает модуль `bluetooth` и не включает `bluetooth.service`, если нет контроллера `/sys/class/bluetooth/hci*`.
  * Виджет сети `waybar-widgets network` показывает Wi-Fi при наличии Wi-Fi адаптера и Ethernet/VPN интерфейсы на проводном ПК.
  * `zapret-discord-youtube-linux` автоматически настраивается на активный физический сетевой интерфейс (`ip route default` / `nmcli`) и `nftables` / `iptables`.
* **Любой пользователь и локаль:** все пути используют `$HOME`, `__USER_HOME__` и `xdg-user-dir PICTURES` (обои ставятся в `~/Изображения/wallpapers` или `~/Pictures/wallpapers`).

## Стек и компоненты
* **Композитор и окружение:** Hyprland (`hyprland.lua` + `host.lua`), Waybar (`waybar` + `waybar-widgets`), Asura-launcher (`launcher-rs`), Mako, Kitty, Fish (`pure`), Fastfetch, btop, Thunar.
* **Динамическая тема `wallfx`:** извлекает палитру из выбранных обоев (`autumn_sunset.png`, `blue_lake.png`, `emerald_city.png`) и обновляет Hyprland, Waybar, Rofi/Asura-launcher, Mako, Kitty, Fish, `hyprlock` и экран входа `ly`. *(Модель `model.onnx` при необходимости скачивается самой утилитой `wallfx`.)*
* **Блокировка и простой:** `hyprlock` + `hypridle` (на ноутбуках плавное затухание через `smooth-brightness`, блокировка, DPMS, сон, `inhibit_sleep = 3`).
* **Вход в систему (`ly`):** `/etc/ly/config.ini` и `/etc/ly/startup.sh` с палитрой `wallfx` для TTY; при установке автоматически отключает предыдущий display manager и активирует `ly@tty2.service`.
* **Браузер:** конфиг **Betterfox** ([firefox/user.js](firefox/user.js)) со Smoothfox.
* **Сеть и софт:** `zapret-discord-youtube-linux`, `WordCraft`, `ArtCraft Launcher` *(пакеты `DeckCraft` / `GridCraft` / `PDFCraft` устанавливаются внутри ArtCraft Launcher)*, `dconf`, `samloader`, правило udev `99-uinput.rules`. При установке автоматически удаляются предустановленные `micro`, `meld` и `alacritty` (включая `cachyos-micro-settings` и `cachyos-alacritty-config`), а временные кэши пакетов (`~/.cache/yay`, `pacman -Sc`) очищаются.

## Установка на чистой системе
```bash
git clone https://github.com/saddestUI/cachyos-dotfiles.git ~/cachyos-dotfiles
cd ~/cachyos-dotfiles
./install.sh --dry-run   # посмотреть профиль железа и список пакетов без изменений
./install.sh
```

Дополнительные флаги `install.sh`:
* `--core-only` — поставить только ядро окружения (без прикладных программ и AUR).
* `--skip-packages` — пропустить установку пакетов через `pacman`/`yay`.
* `--rebuild` — пересобрать все Rust-утилиты (`wallfx`, `smooth-brightness`, `waybar-widgets`, `asura-launcher`) и `druncalc.so` из исходников.
* `--no-system` — не трогать `/etc` и системные `systemd`-юниты.
* `--no-zapret` — не клонировать и не настраивать `zapret-discord-youtube-linux`.
* `--only-firefox` — накатить только `firefox/user.js` в профиль Firefox.
* `--dry-run` — диагностический вывод без изменений в системе.

## Обновление бэкапа
```bash
cd ~/cachyos-dotfiles
./sync.sh
git status
```

## Основные горячие клавиши
* `Super + Return` — терминал (`kitty`)
* `Super + Space` — лаунчер приложений (`asura-launcher`)
* `Win + Shift` — выбор обоев (`wallfx picker`, с обновлением всей темы)
* `Super + L` — блокировка экрана (`hyprlock`)
* `Super + Q` / `Super + F` / `Super + M` — закрыть окно / полный экран / выход из Hyprland
* ПКМ по часам в Waybar — системный монитор CPU / RAM / GPU (`waybar-widgets system`)
