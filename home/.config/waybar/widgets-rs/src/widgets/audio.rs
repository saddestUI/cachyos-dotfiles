use std::cell::Cell;
use std::rc::Rc;

use gtk::pango::EllipsizeMode;
use gtk::prelude::*;
use serde::Deserialize;

use crate::common::{clear_container, make_section_label, run_cmd, Align, WidgetPopup};

#[derive(Deserialize)]
struct PactlSink {
    name: String,
    description: Option<String>,
}

fn get_wpctl_volume(target: &str) -> (i32, bool) {
    let (ok, out, _) = run_cmd(&["wpctl", "get-volume", target]);
    if !ok {
        return (50, false);
    }
    let muted = out.contains("[MUTED]");
    let vol = out
        .split_whitespace()
        .find_map(|tok| tok.parse::<f64>().ok())
        .map(|v| (v * 100.0).round() as i32)
        .unwrap_or(50)
        .clamp(0, 100);
    (vol, muted)
}

fn list_sinks() -> Vec<(String, String, bool)> {
    let (ok_d, def_sink, _) = run_cmd(&["pactl", "get-default-sink"]);
    let (ok_l, out_l, _) = run_cmd(&["pactl", "-f", "json", "list", "sinks"]);
    if !ok_l {
        return Vec::new();
    }
    let Ok(sinks) = serde_json::from_str::<Vec<PactlSink>>(&out_l) else {
        return Vec::new();
    };
    sinks
        .into_iter()
        .map(|s| {
            let desc = s.description.unwrap_or_else(|| s.name.clone());
            let is_def = ok_d && s.name == def_sink;
            (s.name, desc, is_def)
        })
        .collect()
}

fn make_channel_row(
    title: &str,
    target: &'static str,
    icons: (&'static str, &'static str),
) -> (gtk::Box, Rc<dyn Fn()>) {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 4);
    card.style_context().add_class("card");

    let top = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let lbl = gtk::Label::new(Some(title));
    lbl.style_context().add_class("widget-subtitle");
    lbl.set_xalign(0.0);
    let pct_lbl = gtk::Label::new(None);
    pct_lbl.set_xalign(1.0);
    top.pack_start(&lbl, true, true, 0);
    top.pack_end(&pct_lbl, false, false, 0);
    card.pack_start(&top, false, false, 0);

    let ctrl = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let mute_btn = gtk::Button::new();
    mute_btn.style_context().add_class("icon-btn");

    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 100.0, 1.0);
    scale.set_draw_value(false);
    scale.set_hexpand(true);

    ctrl.pack_start(&mute_btn, false, false, 0);
    ctrl.pack_start(&scale, true, true, 0);
    card.pack_start(&ctrl, false, false, 0);

    let syncing = Rc::new(Cell::new(false));

    let sync_ui: Rc<dyn Fn()> = {
        let syncing = syncing.clone();
        let scale = scale.clone();
        let pct_lbl = pct_lbl.clone();
        let mute_btn = mute_btn.clone();
        Rc::new(move || {
            let (vol, muted) = get_wpctl_volume(target);
            syncing.set(true);
            scale.set_value(vol as f64);
            syncing.set(false);
            let pct_text = if muted { "Mute".to_string() } else { format!("{vol}%") };
            pct_lbl.set_text(&pct_text);
            mute_btn.set_label(if muted { icons.1 } else { icons.0 });
            let ctx = mute_btn.style_context();
            if muted {
                ctx.add_class("muted");
            } else {
                ctx.remove_class("muted");
            }
        })
    };

    sync_ui();

    let syncing_s = syncing.clone();
    let pct_lbl_s = pct_lbl.clone();
    scale.connect_value_changed(move |s| {
        if syncing_s.get() {
            return;
        }
        let val = (s.value().round() as i32).clamp(0, 100);
        let arg = format!("{val}%");
        run_cmd(&["wpctl", "set-volume", "-l", "1", target, &arg]);
        pct_lbl_s.set_text(&format!("{val}%"));
    });

    let sync_m = sync_ui.clone();
    mute_btn.connect_clicked(move |_| {
        run_cmd(&["wpctl", "set-mute", target, "toggle"]);
        sync_m();
    });

    (card, sync_ui)
}

fn populate_sinks(sinks_box: &gtk::Box, win: &gtk::Window, sync_out: &Rc<dyn Fn()>) {
    clear_container(sinks_box);
    for (name, desc, is_default) in list_sinks() {
        let btn = gtk::Button::new();
        btn.style_context().add_class("row-btn");
        if is_default {
            btn.style_context().add_class("active");
        }
        let prefix = if is_default { "󰄬 " } else { "󰓃 " };
        let lbl = gtk::Label::new(Some(&format!("{prefix}{desc}")));
        lbl.set_ellipsize(EllipsizeMode::End);
        lbl.set_max_width_chars(30);
        lbl.set_width_chars(1);
        lbl.set_tooltip_text(Some(&desc));
        lbl.set_xalign(0.0);
        btn.add(&lbl);

        let sinks_box_c = sinks_box.clone();
        let win_c = win.clone();
        let sync_c = sync_out.clone();
        btn.connect_clicked(move |_| {
            run_cmd(&["pactl", "set-default-sink", &name]);
            sync_c();
            populate_sinks(&sinks_box_c, &win_c, &sync_c);
        });
        sinks_box.pack_start(&btn, false, false, 0);
    }
    win.show_all();
}

pub fn build() -> gtk::Window {
    let popup = WidgetPopup::new("Звук", "󰕾", Align::Right, 320);

    let (out_row, sync_out) =
        make_channel_row("Выход", "@DEFAULT_AUDIO_SINK@", ("󰕾", "󰖁"));
    let (in_row, _) =
        make_channel_row("Микрофон", "@DEFAULT_AUDIO_SOURCE@", ("󰍬", "󰍭"));
    popup.root_box.pack_start(&out_row, false, false, 0);
    popup.root_box.pack_start(&in_row, false, false, 0);

    popup
        .root_box
        .pack_start(&make_section_label("Устройство вывода"), false, false, 0);
    let scroll = gtk::ScrolledWindow::new(gtk::Adjustment::NONE, gtk::Adjustment::NONE);
    scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroll.set_propagate_natural_height(true);
    scroll.set_max_content_height(220);
    let sinks_box = gtk::Box::new(gtk::Orientation::Vertical, 4);
    scroll.add(&sinks_box);
    popup.root_box.pack_start(&scroll, false, false, 0);

    populate_sinks(&sinks_box, &popup.window, &sync_out);
    popup.window
}
