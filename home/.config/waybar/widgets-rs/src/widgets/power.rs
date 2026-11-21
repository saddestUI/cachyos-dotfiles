use std::cell::RefCell;
use std::fs;
use std::process::Command;
use std::rc::Rc;

use gtk::prelude::*;

use crate::common::{close_app, make_section_label, run_cmd, Align, WidgetPopup};

const BAT_PATH: &str = "/org/freedesktop/UPower/devices/battery_BAT1";

struct BatInfo {
    pct: i32,
    state: &'static str,
    time: String,
    watts_val: f64,
    voltage_v: f64,
    ac_online: bool,
}

fn parse_first_float(val: &str) -> f64 {
    val.split_whitespace()
        .next()
        .map(|s| s.replace(',', "."))
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0)
}

fn read_ac_online() -> bool {
    fs::read_to_string("/sys/class/power_supply/ADP1/online")
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

fn read_battery() -> BatInfo {
    let ac_online = read_ac_online();
    let mut info = BatInfo {
        pct: 100,
        state: if ac_online { "От сети" } else { "От батареи" },
        time: String::new(),
        watts_val: 0.0,
        voltage_v: 0.0,
        ac_online,
    };
    let (ok, out, _) = run_cmd(&["upower", "-i", BAT_PATH]);
    if !ok {
        return info;
    }
    for line in out.lines() {
        let Some((k_raw, v_raw)) = line.split_once(':') else {
            continue;
        };
        let key = k_raw.trim();
        let val = v_raw.trim();
        match key {
            "percentage" => {
                if let Ok(p) = val.trim_end_matches('%').replace(',', ".").parse::<f64>() {
                    info.pct = (p.round() as i32).clamp(0, 100);
                }
            }
            "state" => {
                info.state = match val {
                    "charging" => "Заряжается",
                    "discharging" => "От батареи",
                    "fully-charged" | "pending-charge" => {
                        if ac_online {
                            "От сети (заряжено)"
                        } else {
                            "Заряжено"
                        }
                    }
                    _ => {
                        if ac_online {
                            "От сети"
                        } else {
                            "Питание"
                        }
                    }
                };
            }
            "time to empty" | "time to full" => info.time = val.to_string(),
            "energy-rate" => info.watts_val = parse_first_float(val),
            "voltage" => info.voltage_v = parse_first_float(val),
            _ => {}
        }
    }
    info
}

fn get_profile() -> String {
    let (ok, out, _) = run_cmd(&["powerprofilesctl", "get"]);
    if ok && !out.is_empty() {
        out
    } else {
        "balanced".to_string()
    }
}

pub fn build() -> gtk::Window {
    let popup = WidgetPopup::new("Питание", "󱐋", Align::Right, 310);

    let bat = read_battery();
    let bat_card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    bat_card.style_context().add_class("card");

    let bat_top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let state_lbl = gtk::Label::new(Some(bat.state));
    state_lbl.set_xalign(0.0);
    let pct_lbl = gtk::Label::new(Some(&format!("{}%", bat.pct)));
    pct_lbl.style_context().add_class("accent");
    bat_top.pack_start(&state_lbl, true, true, 0);
    bat_top.pack_end(&pct_lbl, false, false, 0);

    let pbar = gtk::ProgressBar::new();
    pbar.set_fraction((bat.pct as f64 / 100.0).clamp(0.0, 1.0));

    let meta_text = if bat.watts_val > 0.05 {
        let watt_str = format!("{:.1} Вт", bat.watts_val);
        if bat.time.is_empty() {
            watt_str
        } else {
            format!("{} · {}", bat.time, watt_str)
        }
    } else if bat.ac_online {
        if bat.voltage_v > 0.1 {
            format!("Питание от сети · {:.1} В", bat.voltage_v)
        } else {
            "Питание от сети".to_string()
        }
    } else if !bat.time.is_empty() {
        bat.time.clone()
    } else if bat.voltage_v > 0.1 {
        format!("{:.1} В", bat.voltage_v)
    } else {
        "От батареи".to_string()
    };
    let meta_lbl = gtk::Label::new(Some(&meta_text));
    meta_lbl.style_context().add_class("widget-subtitle");
    meta_lbl.set_xalign(0.0);

    bat_card.pack_start(&bat_top, false, false, 0);
    bat_card.pack_start(&pbar, false, false, 0);
    bat_card.pack_start(&meta_lbl, false, false, 0);
    popup.root_box.pack_start(&bat_card, false, false, 0);

    popup
        .root_box
        .pack_start(&make_section_label("Профиль производительности"), false, false, 0);
    let prof_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    popup.root_box.pack_start(&prof_box, false, false, 0);

    let current_prof = get_profile();
    let prof_buttons: Rc<RefCell<Vec<(&'static str, gtk::Button)>>> =
        Rc::new(RefCell::new(Vec::new()));

    let profiles = [
        ("power-saver", "󰌪", "Экономия"),
        ("balanced", "󰾅", "Баланс"),
        ("performance", "󱐋", "Турбо"),
    ];
    for (key, icon, title) in profiles {
        let btn = gtk::Button::with_label(&format!("{icon} {title}"));
        btn.style_context().add_class("pill-btn");
        if key == current_prof {
            btn.style_context().add_class("active");
        }
        prof_box.pack_start(&btn, true, true, 0);
        prof_buttons.borrow_mut().push((key, btn.clone()));

        let buttons_c = prof_buttons.clone();
        btn.connect_clicked(move |_| {
            run_cmd(&["powerprofilesctl", "set", key]);
            for (k, b) in buttons_c.borrow().iter() {
                let ctx = b.style_context();
                if *k == key {
                    ctx.add_class("active");
                } else {
                    ctx.remove_class("active");
                }
            }
        });
    }

    popup
        .root_box
        .pack_start(&make_section_label("Система"), false, false, 0);
    let sys_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    popup.root_box.pack_start(&sys_box, false, false, 0);

    let confirm_hint = gtk::Label::new(Some(""));
    confirm_hint.style_context().add_class("warn-text");
    confirm_hint.set_xalign(0.5);
    popup.root_box.pack_start(&confirm_hint, false, false, 0);

    let armed: Rc<RefCell<Option<&'static str>>> = Rc::new(RefCell::new(None));
    let actions: [(&'static str, &'static str, &'static str, &[&'static str], bool); 4] = [
        ("lock", "󰌾", "Блок", &["hyprlock"], false),
        ("suspend", "󰤄", "Сон", &["systemctl", "suspend"], false),
        ("reboot", "󰜉", "Ребут", &["systemctl", "reboot"], true),
        ("poweroff", "󰐥", "Выкл", &["systemctl", "poweroff"], true),
    ];

    for (key, icon, label, cmd, need_confirm) in actions {
        let btn = gtk::Button::with_label(&format!("{icon} {label}"));
        btn.style_context().add_class("pill-btn");
        if need_confirm {
            btn.style_context().add_class("danger");
        }
        let armed_c = armed.clone();
        let hint_c = confirm_hint.clone();
        btn.connect_clicked(move |_| {
            if need_confirm && *armed_c.borrow() != Some(key) {
                *armed_c.borrow_mut() = Some(key);
                hint_c.set_text(&format!("Нажмите «{label}» ещё раз для подтверждения"));
                return;
            }
            close_app();
            if let Some((prog, rest)) = cmd.split_first() {
                let _ = Command::new(prog).args(rest).spawn();
            }
        });
        sys_box.pack_start(&btn, true, true, 0);
    }

    popup.window
}
