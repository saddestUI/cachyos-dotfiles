use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::rc::Rc;

use gtk::pango::EllipsizeMode;
use gtk::prelude::*;

use crate::common::{clear_container, make_section_label, run_bg, run_cmd, Align, WidgetPopup};

#[derive(Clone)]
struct WifiNet {
    ssid: String,
    signal: i32,
    security: String,
    active: bool,
}

#[derive(Clone)]
struct WiredIface {
    device: String,
    kind: String,
    state: String,
    connection: String,
    ip4: String,
    gateway: String,
    dns: String,
    speed_mbps: Option<u32>,
}

fn detect_wifi_device() -> Option<String> {
    let (ok, out, _) = run_cmd(&["nmcli", "-t", "-f", "DEVICE,TYPE", "dev", "status"]);
    if !ok {
        return None;
    }
    for line in out.lines() {
        if let Some((dev, kind)) = line.split_once(':') {
            if kind.trim() == "wifi" && !dev.trim().is_empty() {
                return Some(dev.trim().to_string());
            }
        }
    }
    None
}

fn wifi_enabled() -> bool {
    let (ok, out, _) = run_cmd(&["nmcli", "radio", "wifi"]);
    ok && out.trim() == "enabled"
}

fn active_ip(dev: &str) -> String {
    let (ok, out, _) = run_cmd(&["nmcli", "-g", "IP4.ADDRESS", "dev", "show", dev]);
    if ok {
        if let Some(first) = out.lines().next() {
            return first.trim().to_string();
        }
    }
    String::new()
}

fn active_ethernet_summary() -> Option<String> {
    let (ok, out, _) = run_cmd(&["nmcli", "-t", "-f", "DEVICE,TYPE,STATE", "dev", "status"]);
    if !ok {
        return None;
    }
    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() == 3 && parts[1].trim() == "ethernet" && parts[2].trim().starts_with("connected")
        {
            let dev = parts[0].trim();
            let ip = active_ip(dev);
            return Some(if ip.is_empty() {
                dev.to_string()
            } else {
                format!("{dev}: {ip}")
            });
        }
    }
    None
}

fn read_wired_ifaces() -> Vec<WiredIface> {
    let (ok, out, _) = run_cmd(&[
        "nmcli",
        "-t",
        "-f",
        "DEVICE,TYPE,STATE,CONNECTION",
        "dev",
        "status",
    ]);
    if !ok {
        return Vec::new();
    }
    let mut result = Vec::new();
    for line in out.lines() {
        let parts: Vec<&str> = line.splitn(4, ':').collect();
        if parts.len() != 4 {
            continue;
        }
        let dev = parts[0].trim();
        let kind = parts[1].trim();
        if dev.is_empty() || kind == "loopback" {
            continue;
        }
        let state = parts[2].trim().to_string();
        let connection = parts[3].trim().to_string();

        let (ip4, gateway, dns) = {
            let (ok_d, out_d, _) = run_cmd(&[
                "nmcli",
                "-g",
                "IP4.ADDRESS,IP4.GATEWAY,IP4.DNS",
                "dev",
                "show",
                dev,
            ]);
            if ok_d {
                let mut lines = out_d.lines();
                let ip = lines.next().unwrap_or("").trim().to_string();
                let gw = lines.next().unwrap_or("").trim().to_string();
                let d = lines
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>()
                    .join(", ");
                (ip, gw, d)
            } else {
                (String::new(), String::new(), String::new())
            }
        };

        let speed_mbps = if kind == "ethernet" {
            fs::read_to_string(format!("/sys/class/net/{dev}/speed"))
                .ok()
                .and_then(|s| s.trim().parse::<i32>().ok())
                .filter(|&v| v > 0)
                .map(|v| v as u32)
        } else {
            None
        };

        result.push(WiredIface {
            device: dev.to_string(),
            kind: kind.to_string(),
            state,
            connection,
            ip4,
            gateway,
            dns,
            speed_mbps,
        });
    }
    result
}

fn scan_networks(rescan: bool) -> Vec<WifiNet> {
    if rescan {
        run_cmd(&["nmcli", "dev", "wifi", "rescan"]);
    }
    let (ok, out, _) = run_cmd(&[
        "nmcli",
        "-t",
        "-f",
        "ACTIVE,SIGNAL,SECURITY,SSID",
        "dev",
        "wifi",
        "list",
    ]);
    if !ok {
        return Vec::new();
    }

    let mut seen: HashMap<String, WifiNet> = HashMap::new();
    for line in out.lines() {
        let mut parts = line.splitn(4, ':');
        let (Some(act_s), Some(sig_s), Some(sec_s), Some(ssid_s)) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let ssid = ssid_s.trim().to_string();
        if ssid.is_empty() {
            continue;
        }
        let signal = sig_s.trim().parse::<i32>().unwrap_or(0);
        let active = act_s.trim() == "yes";
        let security = sec_s.trim().to_string();

        let replace = match seen.get(&ssid) {
            None => true,
            Some(prev) => active || (!prev.active && signal > prev.signal),
        };
        if replace {
            seen.insert(
                ssid.clone(),
                WifiNet {
                    ssid,
                    signal,
                    security,
                    active,
                },
            );
        }
    }

    let mut list: Vec<WifiNet> = seen.into_values().collect();
    list.sort_by(|a, b| b.active.cmp(&a.active).then_with(|| b.signal.cmp(&a.signal)));
    list
}

#[derive(Clone)]
struct UiCtx {
    win: gtk::Window,
    wifi_dev: String,
    status_lbl: gtk::Label,
    wifi_switch: gtk::Switch,
    pwd_box: gtk::Box,
    pwd_entry: gtk::Entry,
    list_box: gtk::Box,
    syncing: Rc<Cell<bool>>,
    pending_ssid: Rc<RefCell<Option<String>>>,
}

fn load_list(ctx: &UiCtx, rescan: bool) {
    ctx.status_lbl
        .set_text(if rescan { "Сканирование..." } else { "Обновление..." });
    let ctx_c = ctx.clone();
    let dev = ctx.wifi_dev.clone();
    run_bg(
        move || {
            let enabled = wifi_enabled();
            let nets = if enabled { scan_networks(rescan) } else { Vec::new() };
            let ip = if enabled { active_ip(&dev) } else { String::new() };
            let eth = active_ethernet_summary();
            (enabled, nets, ip, eth)
        },
        move |(enabled, nets, ip, eth)| {
            ctx_c.syncing.set(true);
            ctx_c.wifi_switch.set_active(enabled);
            ctx_c.wifi_switch.set_state(enabled);
            ctx_c.syncing.set(false);
            render_list(&ctx_c, &nets, enabled, &ip, eth.as_deref());
        },
    );
}

fn on_network_click(ctx: &UiCtx, item: WifiNet) {
    let ssid = item.ssid.clone();
    if item.active {
        ctx.status_lbl.set_text(&format!("Отключение от {ssid}..."));
        let ctx_c = ctx.clone();
        run_bg(
            move || run_cmd(&["nmcli", "con", "down", "id", &ssid]),
            move |_| load_list(&ctx_c, false),
        );
        return;
    }

    ctx.status_lbl.set_text(&format!("Подключение к {ssid}..."));
    let ctx_c = ctx.clone();
    let ssid_bg = ssid.clone();
    run_bg(
        move || run_cmd(&["nmcli", "dev", "wifi", "connect", &ssid_bg]),
        move |(ok, _out, err)| {
            if ok {
                load_list(&ctx_c, false);
                return;
            }
            if !item.security.is_empty()
                && (err.contains("Secrets") || err.to_lowercase().contains("password"))
            {
                ctx_c.status_lbl.set_text(&format!("Требуется пароль для {ssid}"));
                *ctx_c.pending_ssid.borrow_mut() = Some(ssid.clone());
                ctx_c.pwd_entry.set_text("");
                ctx_c
                    .pwd_entry
                    .set_placeholder_text(Some(&format!("Пароль для {ssid}...")));
                ctx_c.pwd_box.set_visible(true);
                ctx_c.pwd_entry.grab_focus();
            } else {
                let short: String = err.chars().take(45).collect();
                ctx_c.status_lbl.set_text(&format!("Не удалось: {short}"));
            }
        },
    );
}

fn render_list(ctx: &UiCtx, nets: &[WifiNet], enabled: bool, ip_addr: &str, eth: Option<&str>) {
    clear_container(&ctx.list_box);

    if !enabled {
        if let Some(eth_s) = eth {
            ctx.status_lbl.set_text(&format!("Wi-Fi выключен · ETH {eth_s}"));
        } else {
            ctx.status_lbl.set_text("Wi-Fi выключен");
        }
        let lbl = gtk::Label::new(Some("Включите переключатель сверху"));
        lbl.style_context().add_class("muted-text");
        ctx.list_box.pack_start(&lbl, false, false, 12);
        ctx.win.show_all();
        ctx.pwd_box.set_visible(false);
        return;
    }

    if let Some(active) = nets.iter().find(|n| n.active) {
        let ip_part = if ip_addr.is_empty() {
            String::new()
        } else {
            format!(" · {ip_addr}")
        };
        ctx.status_lbl
            .set_text(&format!("Подключено: {}{}", active.ssid, ip_part));
    } else if let Some(eth_s) = eth {
        ctx.status_lbl.set_text(&format!("ETH: {eth_s}"));
    } else {
        ctx.status_lbl.set_text("Не подключено");
    }

    for item in nets {
        let btn = gtk::Button::new();
        btn.style_context().add_class("row-btn");
        if item.active {
            btn.style_context().add_class("active");
        }

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let prefix = if item.active { "󰄬 " } else { "󰖩 " };
        let name_lbl = gtk::Label::new(Some(&format!("{prefix}{}", item.ssid)));
        name_lbl.set_xalign(0.0);
        name_lbl.set_ellipsize(EllipsizeMode::End);
        name_lbl.set_max_width_chars(28);
        name_lbl.set_width_chars(1);
        name_lbl.set_tooltip_text(Some(&item.ssid));

        let lock = if item.security.is_empty() { "" } else { "󰌾 " };
        let sig_lbl = gtk::Label::new(Some(&format!("{lock}{}%", item.signal)));
        sig_lbl.style_context().add_class("muted-text");

        row.pack_start(&name_lbl, true, true, 0);
        row.pack_end(&sig_lbl, false, false, 0);
        btn.add(&row);

        let ctx_c = ctx.clone();
        let item_c = item.clone();
        btn.connect_clicked(move |_| on_network_click(&ctx_c, item_c.clone()));
        ctx.list_box.pack_start(&btn, false, false, 0);
    }

    ctx.win.show_all();
    ctx.pwd_box.set_visible(false);
}

#[derive(Clone)]
struct WiredCtx {
    win: gtk::Window,
    status_lbl: gtk::Label,
    list_box: gtk::Box,
}

fn render_wired(ctx: &WiredCtx, ifaces: &[WiredIface]) {
    clear_container(&ctx.list_box);

    if let Some(eth) = ifaces
        .iter()
        .find(|i| i.kind == "ethernet" && !i.ip4.is_empty())
        .or_else(|| ifaces.iter().find(|i| !i.ip4.is_empty()))
    {
        ctx.status_lbl
            .set_text(&format!("Подключено: {} · {}", eth.device, eth.ip4));
    } else if ifaces.is_empty() {
        ctx.status_lbl.set_text("Сетевые интерфейсы не найдены");
    } else {
        ctx.status_lbl.set_text("Нет активного подключения");
    }

    for iface in ifaces {
        let card = gtk::Box::new(gtk::Orientation::Vertical, 5);
        card.style_context().add_class("card");

        let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let icon = if iface.kind == "ethernet" {
            "󰈀"
        } else if iface.kind == "tun" || iface.kind == "wireguard" {
            "󰦝"
        } else {
            "󰛳"
        };
        let title = if iface.connection.is_empty() || iface.connection == "--" {
            format!("{icon}  {}", iface.device)
        } else {
            format!("{icon}  {} · {}", iface.device, iface.connection)
        };
        let title_lbl = gtk::Label::new(Some(&title));
        title_lbl.set_xalign(0.0);
        title_lbl.set_ellipsize(EllipsizeMode::End);
        title_lbl.set_max_width_chars(28);
        title_lbl.set_width_chars(1);
        title_lbl.set_tooltip_text(Some(&title));
        top.pack_start(&title_lbl, true, true, 0);

        let connected = iface.state.starts_with("connected");
        let state_lbl = gtk::Label::new(Some(if connected { "Активно" } else { &iface.state }));
        state_lbl
            .style_context()
            .add_class(if connected { "accent" } else { "muted-text" });
        top.pack_end(&state_lbl, false, false, 0);
        card.pack_start(&top, false, false, 0);

        let mut details = Vec::new();
        if !iface.ip4.is_empty() {
            details.push(format!("IPv4: {}", iface.ip4));
        }
        if !iface.gateway.is_empty() && iface.gateway != "--" {
            details.push(format!("Шлюз: {}", iface.gateway));
        }
        if !iface.dns.is_empty() {
            details.push(format!("DNS: {}", iface.dns));
        }
        if let Some(spd) = iface.speed_mbps {
            details.push(format!("Линк: {spd} Мбит/с"));
        }
        if !details.is_empty() {
            let det_lbl = gtk::Label::new(Some(&details.join("\n")));
            det_lbl.style_context().add_class("widget-subtitle");
            det_lbl.set_xalign(0.0);
            det_lbl.set_line_wrap(true);
            det_lbl.set_max_width_chars(34);
            card.pack_start(&det_lbl, false, false, 0);
        }

        if iface.kind == "ethernet" {
            let btn_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let action_lbl = if connected {
                "Отключить"
            } else {
                "Подключить"
            };
            let btn = gtk::Button::with_label(action_lbl);
            btn.style_context().add_class("pill-btn");
            if connected {
                btn.style_context().add_class("danger");
            }
            let dev_name = iface.device.clone();
            let ctx_c = ctx.clone();
            btn.connect_clicked(move |_| {
                ctx_c.status_lbl.set_text(if connected {
                    "Отключение..."
                } else {
                    "Подключение..."
                });
                let dev_bg = dev_name.clone();
                let ctx_done = ctx_c.clone();
                run_bg(
                    move || {
                        let op = if connected { "disconnect" } else { "connect" };
                        run_cmd(&["nmcli", "dev", op, &dev_bg])
                    },
                    move |_| load_wired(&ctx_done),
                );
            });
            btn_row.pack_end(&btn, false, false, 0);
            card.pack_start(&btn_row, false, false, 0);
        }

        ctx.list_box.pack_start(&card, false, false, 0);
    }

    ctx.win.show_all();
}

fn load_wired(ctx: &WiredCtx) {
    ctx.status_lbl.set_text("Обновление...");
    let ctx_c = ctx.clone();
    run_bg(read_wired_ifaces, move |ifaces| render_wired(&ctx_c, &ifaces));
}

fn build_wired() -> gtk::Window {
    let popup = WidgetPopup::new("Сеть", "󰈀", Align::Right, 330);

    let refresh_btn = gtk::Button::with_label("󰑓");
    refresh_btn.style_context().add_class("icon-btn");
    popup.header_actions.pack_start(&refresh_btn, false, false, 0);

    let status_lbl = gtk::Label::new(None);
    status_lbl.style_context().add_class("widget-subtitle");
    status_lbl.set_xalign(0.0);
    status_lbl.set_ellipsize(EllipsizeMode::End);
    status_lbl.set_max_width_chars(32);
    popup.root_box.pack_start(&status_lbl, false, false, 0);

    popup
        .root_box
        .pack_start(&make_section_label("Интерфейсы"), false, false, 0);

    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_min_content_height(140);
    scroll.set_max_content_height(260);
    let list_box = gtk::Box::new(gtk::Orientation::Vertical, 6);
    scroll.add(&list_box);
    popup.root_box.pack_start(&scroll, true, true, 0);

    let ctx = WiredCtx {
        win: popup.window.clone(),
        status_lbl,
        list_box,
    };

    let ctx_ref = ctx.clone();
    refresh_btn.connect_clicked(move |_| load_wired(&ctx_ref));

    load_wired(&ctx);
    popup.window
}

pub fn build() -> gtk::Window {
    let Some(wifi_dev) = detect_wifi_device() else {
        return build_wired();
    };

    let popup = WidgetPopup::new("Сеть Wi-Fi", "󰖩", Align::Right, 330);

    let refresh_btn = gtk::Button::with_label("󰑓");
    refresh_btn.style_context().add_class("icon-btn");
    popup.header_actions.pack_start(&refresh_btn, false, false, 0);

    let wifi_switch = gtk::Switch::new();
    wifi_switch.set_valign(gtk::Align::Center);
    popup.header_actions.pack_start(&wifi_switch, false, false, 0);

    let status_lbl = gtk::Label::new(None);
    status_lbl.style_context().add_class("widget-subtitle");
    status_lbl.set_xalign(0.0);
    status_lbl.set_ellipsize(EllipsizeMode::End);
    status_lbl.set_max_width_chars(32);
    popup.root_box.pack_start(&status_lbl, false, false, 0);

    let pwd_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let pwd_entry = gtk::Entry::new();
    pwd_entry.set_visibility(false);
    pwd_entry.set_placeholder_text(Some("Пароль Wi-Fi..."));
    let pwd_ok = gtk::Button::with_label("Ок");
    pwd_ok.style_context().add_class("pill-btn");
    pwd_box.pack_start(&pwd_entry, true, true, 0);
    pwd_box.pack_start(&pwd_ok, false, false, 0);
    popup.root_box.pack_start(&pwd_box, false, false, 0);

    popup
        .root_box
        .pack_start(&make_section_label("Доступные сети"), false, false, 0);

    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_min_content_height(180);
    scroll.set_max_content_height(240);
    let list_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    scroll.add(&list_box);
    popup.root_box.pack_start(&scroll, true, true, 0);

    let ctx = UiCtx {
        win: popup.window.clone(),
        wifi_dev,
        status_lbl,
        wifi_switch: wifi_switch.clone(),
        pwd_box,
        pwd_entry: pwd_entry.clone(),
        list_box,
        syncing: Rc::new(Cell::new(false)),
        pending_ssid: Rc::new(RefCell::new(None)),
    };

    let submit_pwd = {
        let ctx_c = ctx.clone();
        Rc::new(move || {
            let Some(ssid) = ctx_c.pending_ssid.borrow().clone() else {
                return;
            };
            let password = ctx_c.pwd_entry.text().to_string();
            if password.is_empty() {
                return;
            }
            ctx_c.pwd_box.set_visible(false);
            ctx_c.status_lbl.set_text(&format!("Подключение к {ssid}..."));
            let ctx_done = ctx_c.clone();
            run_bg(
                move || run_cmd(&["nmcli", "dev", "wifi", "connect", &ssid, "password", &password]),
                move |(ok, _out, err)| {
                    if !ok {
                        let short: String = err.chars().take(45).collect();
                        ctx_done.status_lbl.set_text(&format!("Ошибка: {short}"));
                    }
                    load_list(&ctx_done, false);
                },
            );
        })
    };

    let sub1 = submit_pwd.clone();
    pwd_ok.connect_clicked(move |_| sub1());
    pwd_entry.connect_activate(move |_| submit_pwd());

    let ctx_sw = ctx.clone();
    wifi_switch.connect_state_set(move |_, active| {
        if ctx_sw.syncing.get() {
            return glib::Propagation::Proceed;
        }
        let ctx_c = ctx_sw.clone();
        run_bg(
            move || run_cmd(&["nmcli", "radio", "wifi", if active { "on" } else { "off" }]),
            move |_| load_list(&ctx_c, active),
        );
        glib::Propagation::Stop
    });

    let ctx_ref = ctx.clone();
    refresh_btn.connect_clicked(move |_| load_list(&ctx_ref, true));

    load_list(&ctx, false);
    popup.window
}
