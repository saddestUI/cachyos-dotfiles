# CachyOS + Hyprland + Asura-Glass Dotfiles

Полный воспроизводимый бэкап системы на базе **CachyOS Linux** (без паролей, токенов, куки и аккаунтов).

## Стек и компоненты
* **Композитор и окружение:** Hyprland (конфиг на Lua `hyprland.lua`), Waybar (`waybar` + `waybar-widgets`), Asura-launcher (`launcher-rs`), Mako, Kitty, Fish (`pure`), Fastfetch, btop, Thunar.
* **Динамическая тема `wallfx`:** автоматически извлекает палитру из выбранных обоев (`autumn_sunset.png`, `blue_lake.png`, `emerald_city.png`) и обновляет цвета Hyprland, Waybar, Rofi/Asura-launcher, Mako, Kitty, Fish, `hyprlock` и экран входа `ly`. *(Нейросетевая модель `model.onnx` для вырезания силуэтов при первом запуске автоматически скачивается самой утилитой `wallfx`.)*
* **Блокировка и простой:** `hyprlock` (блюр обоев, матовая карточка, русская дата, плеер, батарея `BAT1`, синхронизация с `wallfx`) + `hypridle` (плавное затухание через `smooth-brightness`, блокировка, DPMS, сон, `inhibit_sleep = 3`).
* **Вход в систему (`ly`):** `/etc/ly/config.ini` и `/etc/ly/startup.sh` с динамической подгрузкой палитры `wallfx` в цвета Linux TTY, часами, зарядом батареи и клавишей сна `F3`.
* **Браузер:** конфиг **Betterfox** ([firefox/user.js](firefox/user.js)) с плавной прокруткой Smoothfox и личными переопределениями (стандартная защита, подсказки поиска, автозаполнение, встроенные AI/ML-функции, сохранение куки, отключённый HTTPS-Only).
* **Сеть и софт:** настройки и списки для `zapret-discord-youtube-linux`, настройки `WordCraft` / `PDFCraft` / `craft-fonts`, `dconf`, бинарник `samloader`, правило udev `99-uinput.rules`.

## Структура репозитория
* `packages/` — списки пакетов `pkglist-repo.txt` (официальные репо), `pkglist-aur.txt` (AUR) и включённых сервисов `systemd-enabled.txt`, `systemd-user-enabled.txt`.
* `home/` — пользовательские конфиги (`~/.config/*`, `~/.bashrc`, `~/.nvidia-settings-rc`), исходники на Rust (`wallfx-rs`, `smooth-brightness-rs`, `widgets-rs`, `launcher-rs`), исходник `druncalc.c`, а также готовые бинарники (`~/.local/bin/*`, `waybar-widgets`, `samloader`).
* `firefox/` — `user.js` (Betterfox).
* `wallpapers/` — обои рабочего стола.
* `dconf/` — дамп настроек `dconf`.
* `zapret/` — конфигурация `conf.env`, стратегии, пользовательские списки и systemd-юнит `zapret_discord_youtube.service`.
* `system/etc/` — системные конфиги `ly`, `udev`, `vconsole.conf`.
* `install.sh` — скрипт автоматического восстановления на чистой системе.
* `sync.sh` — скрипт обновления бэкапа из текущей системы.

## Что намеренно НЕ включено
В репозиторий не попадают личные данные, токены, куки, пароли, кэши и артефакты сборки (`target/`): профили/сессии Firefox (кроме `user.js`), Discord, Telegram, Spotify, Throne, Antigravity/Gemini, ключи LocalSend, `~/.ssh`, история оболочек и `wallfx/model.onnx`.

## Восстановление на новой системе
```bash
git clone <URL_репозитория> ~/Проекты/cachyos-dotfiles
cd ~/Проекты/cachyos-dotfiles
./install.sh
```

Дополнительные флаги `install.sh`:
* `--skip-packages` — пропустить установку пакетов через `pacman`/`yay`.
* `--rebuild` — пересобрать все Rust-утилиты из исходников вместо копирования готовых бинарников.
* `--no-system` — не трогать `/etc` и системные `systemd`-юниты (без `sudo`).
* `--only-firefox` — накатить только `firefox/user.js` в профиль Firefox.

## Обновление бэкапа
```bash
cd ~/Проекты/cachyos-dotfiles
./sync.sh
git add -A && git commit -m "Update dotfiles" && git push
```

## Основные горячие клавиши
* `Super + Return` — терминал (`kitty`)
* `Super + Space` — лаунчер приложений (`asura-launcher`)
* `Win + Shift` — выбор обоев (`wallfx picker`, с обновлением всей темы)
* `Super + L` — блокировка экрана (`hyprlock`)
* `Super + Q` / `Super + F` / `Super + M` — закрыть окно / полный экран / выход из Hyprland
