use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

use glib::ControlFlow;
use gtk::prelude::*;

use crate::common::{make_section_label, run_cmd, Align, WidgetPopup};

const DEVICE: &str = "intel_backlight";
const BRIGHTNESS_ICONS: [&str; 7] = ["󰃚", "󰃛", "󰃜", "󰃝", "󰃞", "󰃟", "󰃠"];

fn brightness_icon(pct: i32) -> &'static str {
    let norm = (pct.clamp(5, 100) - 5) as f64 / 95.0;
    let idx = (norm * (BRIGHTNESS_ICONS.len() - 1) as f64).round() as usize;
    BRIGHTNESS_ICONS[idx.min(BRIGHTNESS_ICONS.len() - 1)]
}

fn read_sysfs_i64(file: &str) -> Option<i64> {
    let path = format!("/sys/class/backlight/{DEVICE}/{file}");
    fs::read_to_string(path).ok()?.trim().parse::<i64>().ok()
}

fn get_brightness() -> i32 {
    if let (Some(cur), Some(max_b)) = (read_sysfs_i64("brightness"), read_sysfs_i64("max_brightness"))
    {
        if max_b > 0 {
            let pct = ((cur as f64 / max_b as f64) * 100.0).round() as i32;
            return pct.clamp(5, 100);
        }
    }
    let (ok, out, _) = run_cmd(&["brightnessctl", "-d", DEVICE, "-m"]);
    if ok {
        if let Some(pct_field) = out.split(',').nth(3) {
            if let Ok(val) = pct_field.trim_end_matches('%').parse::<i32>() {
                return val.clamp(5, 100);
            }
        }
    }
    50
}

fn smooth_brightness_bin() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/samerim"));
    let bin = home.join(".local/bin/smooth-brightness");
    if bin.exists() {
        bin
    } else {
        PathBuf::from("smooth-brightness")
    }
}

fn spawn_brightness_helper(mode: &str, pct: i32) {
    let bin = smooth_brightness_bin();
    let mode_s = mode.to_string();
    let arg = pct.to_string();
    // Wait/reap on a short background thread so the popup never blocks or leaves zombie children.
    std::thread::spawn(move || {
        let _ = Command::new(bin)
            .args([&mode_s, &arg])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}

pub fn build() -> gtk::Window {
    let popup = WidgetPopup::new("Яркость экрана", "󰃠", Align::Right, 290);

    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.style_context().add_class("card");

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon_lbl = gtk::Label::new(Some("󰃟"));
    icon_lbl.style_context().add_class("accent");

    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 5.0, 100.0, 1.0);
    scale.set_draw_value(false);
    scale.set_hexpand(true);

    let pct_lbl = gtk::Label::new(None);
    pct_lbl.set_width_chars(4);
    pct_lbl.set_xalign(1.0);

    row.pack_start(&icon_lbl, false, false, 0);
    row.pack_start(&scale, true, true, 0);
    row.pack_start(&pct_lbl, false, false, 0);
    card.pack_start(&row, false, false, 0);
    popup.root_box.pack_start(&card, false, false, 0);

    popup
        .root_box
        .pack_start(&make_section_label("Быстрые уровни"), false, false, 0);
    let presets_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    popup.root_box.pack_start(&presets_box, false, false, 0);

    let syncing = Rc::new(Cell::new(false));
    let anim_gen = Rc::new(Cell::new(0u64));
    let debounce_timer: Rc<RefCell<Option<glib::SourceId>>> = Rc::new(RefCell::new(None));

    let current = get_brightness();
    syncing.set(true);
    scale.set_value(current as f64);
    syncing.set(false);
    pct_lbl.set_text(&format!("{current}%"));
    icon_lbl.set_text(brightness_icon(current));

    let syncing_s = syncing.clone();
    let anim_gen_s = anim_gen.clone();
    let pct_lbl_s = pct_lbl.clone();
    let icon_lbl_s = icon_lbl.clone();
    let debounce_s = debounce_timer.clone();
    scale.connect_value_changed(move |s| {
        if syncing_s.get() {
            return;
        }
        anim_gen_s.set(anim_gen_s.get().wrapping_add(1));
        let val = (s.value().round() as i32).clamp(5, 100);
        pct_lbl_s.set_text(&format!("{val}%"));
        icon_lbl_s.set_text(brightness_icon(val));

        if let Some(prev) = debounce_s.borrow_mut().take() {
            prev.remove();
        }
        let deb_ref = debounce_s.clone();
        let id = glib::timeout_add_local(Duration::from_millis(25), move || {
            *deb_ref.borrow_mut() = None;
            spawn_brightness_helper("instant", val);
            ControlFlow::Break
        });
        *debounce_s.borrow_mut() = Some(id);
    });

    for level in [25, 50, 75, 100] {
        let btn = gtk::Button::with_label(&format!("{level}%"));
        btn.style_context().add_class("pill-btn");
        let scale_c = scale.clone();
        let pct_c = pct_lbl.clone();
        let icon_c = icon_lbl.clone();
        let syncing_c = syncing.clone();
        let anim_gen_c = anim_gen.clone();
        let debounce_c = debounce_timer.clone();
        btn.connect_clicked(move |_| {
            let my_gen = anim_gen_c.get().wrapping_add(1);
            anim_gen_c.set(my_gen);
            if let Some(prev) = debounce_c.borrow_mut().take() {
                prev.remove();
            }

            // Drive hardware fade in one non-blocking D-Bus helper call (0 brightnessctl spawns).
            spawn_brightness_helper("set", level);

            let start_val = scale_c.value();
            let target_val = level as f64;
            let total_steps = 12u32;
            let step = Rc::new(Cell::new(0u32));

            let scale_t = scale_c.clone();
            let pct_t = pct_c.clone();
            let icon_t = icon_c.clone();
            let syncing_t = syncing_c.clone();
            let anim_gen_t = anim_gen_c.clone();

            glib::timeout_add_local(Duration::from_millis(12), move || {
                if anim_gen_t.get() != my_gen {
                    return ControlFlow::Break;
                }
                let s = step.get() + 1;
                step.set(s);
                let t = (s as f64 / total_steps as f64).min(1.0);
                let eased = 1.0 - (1.0 - t).powi(3);
                let cur_f = start_val + (target_val - start_val) * eased;
                let cur_i = (cur_f.round() as i32).clamp(5, 100);

                syncing_t.set(true);
                scale_t.set_value(cur_f);
                syncing_t.set(false);
                pct_t.set_text(&format!("{cur_i}%"));
                icon_t.set_text(brightness_icon(cur_i));

                if s >= total_steps {
                    ControlFlow::Break
                } else {
                    ControlFlow::Continue
                }
            });
        });
        presets_box.pack_start(&btn, true, true, 0);
    }

    popup.window
}
