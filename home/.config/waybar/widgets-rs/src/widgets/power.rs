use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::rc::Rc;

use gtk::prelude::*;

use crate::common::{close_app, make_section_label, run_cmd, Align, WidgetPopup};

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

fn is_system_battery(ps_type: &str, scope: Option<&str>, has_capacity: bool) -> bool {
    has_capacity
        && ps_type.trim() == "Battery"
        && scope.map(str::trim).unwrap_or("System") != "Device"
}

fn detect_laptop_battery() -> Option<String> {
    let entries = fs::read_dir("/sys/class/power_supply").ok()?;
    let mut bats = Vec::new();
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(ps_type) = fs::read_to_string(dir.join("type")) else {
            continue;
        };
        let scope = fs::read_to_string(dir.join("scope")).ok();
        let has_cap = dir.join("capacity").is_file();
        if is_system_battery(&ps_type, scope.as_deref(), has_cap) {
            bats.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    bats.sort();
    bats.into_iter().next()
}

fn read_ac_online() -> bool {
    let Ok(entries) = fs::read_dir("/sys/class/power_supply") else {
        return false;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(ps_type) = fs::read_to_string(dir.join("type")) else {
            continue;
        };
        if ps_type.trim() == "Mains" {
            if let Ok(online) = fs::read_to_string(dir.join("online")) {
                if online.trim() == "1" {
                    return true;
                }
            }
        }
    }
    false
}

fn read_battery(bat_name: &str) -> BatInfo {
    let ac_online = read_ac_online();
    let sysfs_dir = format!("/sys/class/power_supply/{bat_name}");
    let sysfs_pct = fs::read_to_string(format!("{sysfs_dir}/capacity"))
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .map(|p| p.clamp(0, 100))
        .unwrap_or(100);
    let sysfs_state = fs::read_to_string(format!("{sysfs_dir}/status"))
        .ok()
        .map(|s| match s.trim() {
            "Charging" => "Заряжается",
            "Discharging" => "От батареи",
            "Full" | "Not charging" => {
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
                    "От батареи"
                }
            }
        })
        .unwrap_or(if ac_online { "От сети" } else { "От батареи" });

    let mut info = BatInfo {
        pct: sysfs_pct,
        state: sysfs_state,
        time: String::new(),
        watts_val: 0.0,
        voltage_v: 0.0,
        ac_online,
    };

    let upower_path = format!("/org/freedesktop/UPower/devices/battery_{bat_name}");
    let (ok, out, _) = run_cmd(&["upower", "-i", &upower_path]);
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

fn lock_command() -> Vec<String> {
    if let Some(home) = std::env::var_os("HOME") {
        let lock_sh = PathBuf::from(home).join(".config/hypr/lock.sh");
        if lock_sh.is_file() {
            return vec![lock_sh.to_string_lossy().into_owned()];
        }
    }
    vec!["hyprlock".to_string()]
}

pub fn build() -> gtk::Window {
    let popup = WidgetPopup::new("Питание", "󱐋", Align::Right, 310);

    if let Some(bat_name) = detect_laptop_battery() {
        let bat = read_battery(&bat_name);
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
    } else {
        let ac_card = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        ac_card.style_context().add_class("card");
        let ac_lbl = gtk::Label::new(Some("󰚥 Питание от сети · Десктоп"));
        ac_lbl.set_xalign(0.0);
        ac_card.pack_start(&ac_lbl, true, true, 0);
        popup.root_box.pack_start(&ac_card, false, false, 0);
    }

    if run_cmd(&["powerprofilesctl", "list"]).0 {
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
    let actions: [(&'static str, &'static str, &'static str, Vec<String>, bool); 4] = [
        ("lock", "󰌾", "Блок", lock_command(), false),
        (
            "suspend",
            "󰤄",
            "Сон",
            vec!["systemctl".into(), "suspend".into()],
            false,
        ),
        (
            "reboot",
            "󰜉",
            "Ребут",
            vec!["systemctl".into(), "reboot".into()],
            true,
        ),
        (
            "poweroff",
            "󰐥",
            "Выкл",
            vec!["systemctl".into(), "poweroff".into()],
            true,
        ),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_device_batteries_and_mains() {
        assert!(is_system_battery("Battery\n", None, true));
        assert!(is_system_battery("Battery\n", Some("System\n"), true));
        assert!(!is_system_battery("Battery\n", Some("Device\n"), true));
        assert!(!is_system_battery("Mains\n", None, false));
        assert!(!is_system_battery("Battery\n", None, false));
    }
}
