use std::cell::RefCell;
use std::fs;
use std::io::{BufRead, BufReader, ErrorKind};
use std::os::unix::net::UnixStream;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use glib::ControlFlow;
use gtk::prelude::*;
use serde::Deserialize;

use crate::common::{
    clear_container, hypr_ipc, hypr_socket_path, make_section_label, run_cmd, Align, WidgetPopup,
};

const FALLBACK_KEYBOARD: &str = "at-translated-set-2-keyboard";

#[derive(Deserialize)]
struct DevicesOut {
    keyboards: Vec<KeyboardDev>,
}

#[derive(Deserialize)]
struct KeyboardDev {
    name: String,
    layout: String,
    active_layout_index: usize,
    #[serde(rename = "capsLock")]
    caps_lock: bool,
    #[serde(rename = "numLock")]
    num_lock: bool,
    main: bool,
}

#[derive(Clone, PartialEq, Eq)]
struct KbState {
    active_idx: usize,
    layouts: Vec<String>,
    caps: bool,
    num: bool,
}

fn read_led(name: &str) -> bool {
    let path = format!("/sys/class/leds/input4::{name}/brightness");
    fs::read_to_string(path)
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

fn pick_active_keyboard(keyboards: &[KeyboardDev]) -> Option<&KeyboardDev> {
    keyboards
        .iter()
        .find(|k| k.main)
        .or_else(|| keyboards.iter().find(|k| k.name == FALLBACK_KEYBOARD))
        .or_else(|| keyboards.first())
}

fn read_keyboard_state() -> KbState {
    let out = hypr_ipc("j/devices").or_else(|| {
        let (ok, s, _) = run_cmd(&["hyprctl", "devices", "-j"]);
        ok.then_some(s)
    });
    if let Some(out) = out {
        if let Ok(devs) = serde_json::from_str::<DevicesOut>(&out) {
            if let Some(kb) = pick_active_keyboard(&devs.keyboards) {
                let layouts = kb
                    .layout
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                return KbState {
                    active_idx: kb.active_layout_index,
                    layouts,
                    caps: kb.caps_lock || read_led("capslock"),
                    num: kb.num_lock || read_led("numlock"),
                };
            }
        }
    }
    KbState {
        active_idx: 0,
        layouts: vec!["us".into(), "ru".into()],
        caps: read_led("capslock"),
        num: read_led("numlock"),
    }
}

fn layout_display_name(code: &str) -> String {
    match code {
        "us" => "English (US)".to_string(),
        "ru" => "Русский".to_string(),
        other => other.to_uppercase(),
    }
}

#[derive(Clone)]
struct LangCtx {
    win: gtk::Window,
    layout_box: gtk::Box,
    status_lbl: gtk::Label,
    last_state: Rc<RefCell<Option<KbState>>>,
}

fn render_status_label(ctx: &LangCtx, caps: bool, num: bool) {
    let caps_s = if caps { "вкл" } else { "выкл" };
    let num_s = if num { "вкл" } else { "выкл" };
    ctx.status_lbl
        .set_text(&format!("Caps Lock: {caps_s}  ·  Num Lock: {num_s}"));
}

fn render_state(ctx: &LangCtx, st: &KbState) {
    if ctx.last_state.borrow().as_ref() == Some(st) {
        return;
    }
    *ctx.last_state.borrow_mut() = Some(st.clone());

    clear_container(&ctx.layout_box);

    for (idx, code) in st.layouts.iter().enumerate() {
        let btn = gtk::Button::with_label(&layout_display_name(code));
        btn.style_context().add_class("pill-btn");
        if idx == st.active_idx {
            btn.style_context().add_class("active");
        }
        let ctx_c = ctx.clone();
        btn.connect_clicked(move |_| {
            let cmd = format!("switchxkblayout all {idx}");
            if hypr_ipc(&cmd).is_none() {
                let idx_str = idx.to_string();
                run_cmd(&["hyprctl", "switchxkblayout", "all", &idx_str]);
            }
            render_state(&ctx_c, &read_keyboard_state());
        });
        ctx.layout_box.pack_start(&btn, true, true, 0);
    }

    render_status_label(ctx, st.caps, st.num);
    ctx.win.show_all();
}

#[allow(deprecated)]
pub fn build() -> gtk::Window {
    let popup = WidgetPopup::new("Раскладка", "󰌌", Align::Right, 260);

    popup
        .root_box
        .pack_start(&make_section_label("Активный язык"), false, false, 0);
    let layout_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    popup.root_box.pack_start(&layout_box, false, false, 0);

    let status_lbl = gtk::Label::new(None);
    status_lbl.style_context().add_class("widget-subtitle");
    status_lbl.set_xalign(0.5);
    popup.root_box.pack_start(&status_lbl, false, false, 0);

    let ctx = LangCtx {
        win: popup.window.clone(),
        layout_box,
        status_lbl,
        last_state: Rc::new(RefCell::new(None)),
    };

    render_state(&ctx, &read_keyboard_state());

    // Event-driven listener on Hyprland `.socket2.sock` for instant layout switches (0 polling).
    let stop_flag = Arc::new(AtomicBool::new(false));
    if let Some(sock2) = hypr_socket_path(".socket2.sock") {
        let (tx, rx) = glib::MainContext::channel::<()>(glib::Priority::DEFAULT);
        let stop_bg = stop_flag.clone();
        std::thread::spawn(move || {
            if let Ok(stream) = UnixStream::connect(sock2) {
                let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                while !stop_bg.load(Ordering::Relaxed) {
                    line.clear();
                    match reader.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {
                            if line.starts_with("activelayout>>") && tx.send(()).is_err() {
                                break;
                            }
                        }
                        Err(ref e)
                            if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) =>
                        {
                            continue;
                        }
                        Err(_) => break,
                    }
                }
            }
        });
        let ctx_ev = ctx.clone();
        rx.attach(None, move |_| {
            render_state(&ctx_ev, &read_keyboard_state());
            ControlFlow::Continue
        });
    }

    // Slow 800ms pure-sysfs LED check for Caps/Num Lock (reads two 2-byte files, zero IPC/forks).
    let ctx_led = ctx;
    let led_timer = glib::timeout_add_local(Duration::from_millis(800), move || {
        let caps = read_led("capslock");
        let num = read_led("numlock");
        let mut last = ctx_led.last_state.borrow_mut();
        if let Some(ref mut prev) = *last {
            if prev.caps != caps || prev.num != num {
                prev.caps = caps;
                prev.num = num;
                drop(last);
                render_status_label(&ctx_led, caps, num);
            }
        }
        ControlFlow::Continue
    });

    let led_cell = Rc::new(RefCell::new(Some(led_timer)));
    popup.window.connect_destroy(move |_| {
        stop_flag.store(true, Ordering::Relaxed);
        if let Some(id) = led_cell.borrow_mut().take() {
            id.remove();
        }
    });

    popup.window
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_main_keyboard_over_laptop_builtin() {
        let json = r#"{
            "keyboards": [
                {
                    "name": "at-translated-set-2-keyboard",
                    "layout": "us, ru",
                    "active_layout_index": 0,
                    "capsLock": false,
                    "numLock": false,
                    "main": false
                },
                {
                    "name": "logitech-k850",
                    "layout": "us, ru",
                    "active_layout_index": 1,
                    "capsLock": false,
                    "numLock": false,
                    "main": true
                }
            ]
        }"#;
        let devs: DevicesOut = serde_json::from_str(json).unwrap();
        let kb = pick_active_keyboard(&devs.keyboards).unwrap();
        assert_eq!(kb.name, "logitech-k850");
        assert_eq!(kb.active_layout_index, 1);
    }
}
