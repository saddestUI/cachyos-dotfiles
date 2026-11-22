use std::cell::Cell;
use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Duration;

use gdk::keys::constants::Escape;
use glib::ControlFlow;
use gtk::prelude::*;
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde::Deserialize;

const CSS_DATA: &str = include_str!("widget.css");

#[derive(Copy, Clone, Eq, PartialEq)]
pub enum Align {
    Center,
    Right,
}

#[derive(Deserialize)]
struct Monitor {
    name: String,
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    scale: f64,
    #[serde(default)]
    transform: i32,
    focused: bool,
}

#[derive(Deserialize)]
struct LayerItem {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    namespace: String,
}

#[derive(Deserialize)]
struct MonitorLayers {
    levels: HashMap<String, Vec<LayerItem>>,
}

#[derive(Deserialize)]
struct CursorPos {
    x: i32,
    y: i32,
}

pub fn hypr_socket_path(sock_name: &str) -> Option<PathBuf> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|s| !s.is_empty())?;
    let sig = std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").filter(|s| !s.is_empty())?;
    Some(
        PathBuf::from(runtime)
            .join("hypr")
            .join(sig)
            .join(sock_name),
    )
}

/// Query or dispatch over Hyprland's `.socket.sock` without spawning `hyprctl`.
pub fn hypr_ipc(cmd: &str) -> Option<String> {
    let sock = hypr_socket_path(".socket.sock")?;
    let mut stream = UnixStream::connect(sock).ok()?;
    let timeout = Some(Duration::from_millis(400));
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);
    stream.write_all(cmd.as_bytes()).ok()?;
    let _ = stream.shutdown(Shutdown::Write);
    let mut out = String::new();
    stream.read_to_string(&mut out).ok()?;
    Some(out.trim().to_string())
}

pub fn run_cmd(args: &[&str]) -> (bool, String, String) {
    let Some((prog, rest)) = args.split_first() else {
        return (false, String::new(), "empty command".to_string());
    };
    match Command::new(prog)
        .args(rest)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
    {
        Ok(out) => (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ),
        Err(err) => (false, String::new(), err.to_string()),
    }
}

#[allow(deprecated)]
pub fn run_bg<T, F, D>(work: F, done: D)
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
    D: FnOnce(T) + 'static,
{
    let (tx, rx) = glib::MainContext::channel(glib::Priority::DEFAULT);
    std::thread::spawn(move || {
        let res = work();
        let _ = tx.send(res);
    });
    let mut done_opt = Some(done);
    rx.attach(None, move |res| {
        if let Some(cb) = done_opt.take() {
            cb(res);
        }
        ControlFlow::Break
    });
}

fn pid_file_path() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.join("waybar-widget.pid")
}

/// Kill any active widget popup. Returns true if the same widget was toggled off.
pub fn toggle_or_replace(widget_name: &str) -> bool {
    let path = pid_file_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Some((pid_s, old_name)) = raw.trim().split_once(':') {
            if let Ok(old_pid) = pid_s.parse::<i32>() {
                if old_pid > 0 && old_pid != unsafe { libc::getpid() } {
                    unsafe {
                        libc::kill(old_pid, libc::SIGTERM);
                    }
                }
                let _ = fs::remove_file(&path);
                if old_name == widget_name {
                    return true;
                }
            }
        }
    }
    let my_pid = unsafe { libc::getpid() };
    let _ = fs::write(path, format!("{my_pid}:{widget_name}"));
    false
}

pub fn cleanup_pid() {
    let path = pid_file_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Some((pid_s, _)) = raw.trim().split_once(':') {
            if pid_s.parse::<i32>().ok() == Some(unsafe { libc::getpid() }) {
                let _ = fs::remove_file(path);
            }
        }
    }
}

pub fn load_css() {
    let colors_path = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/waybar/colors.css");
    let dynamic_colors = fs::read_to_string(colors_path).unwrap_or_default();
    let combined = format!("{CSS_DATA}\n{dynamic_colors}");

    let provider = gtk::CssProvider::new();
    let _ = provider.load_from_data(combined.as_bytes());
    if let Some(screen) = gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

struct PopupPlacement {
    top_margin: i32,
    mon_w: i32,
    bar_x: i32,
    bar_w: i32,
    cursor_cx: Option<i32>,
    fallback_align: Align,
}

impl PopupPlacement {
    fn left_for_width(&self, w: i32) -> i32 {
        let max_left = (self.mon_w - w - 8).max(8);
        let raw = if let Some(cx) = self.cursor_cx {
            cx - w / 2
        } else {
            match self.fallback_align {
                Align::Center => self.bar_x + (self.bar_w - w) / 2,
                Align::Right => self.bar_x + self.bar_w - w,
            }
        };
        raw.clamp(8, max_left)
    }
}

fn get_popup_placement(fallback_align: Align) -> PopupPlacement {
    let mut top_margin = 42;
    let mut mon_x = 0;
    let mut mon_y = 0;
    let mut mon_w = 1920;
    let (mut bar_x, mut bar_y, mut bar_w, mut bar_h) = (530, 8, 860, 28);

    let out_l = hypr_ipc("j/layers").or_else(|| {
        let (ok, out, _) = run_cmd(&["hyprctl", "layers", "-j"]);
        ok.then_some(out)
    });
    let out_m = hypr_ipc("j/monitors").or_else(|| {
        let (ok, out, _) = run_cmd(&["hyprctl", "monitors", "-j"]);
        ok.then_some(out)
    });
    let out_c = hypr_ipc("j/cursorpos").or_else(|| {
        let (ok, out, _) = run_cmd(&["hyprctl", "cursorpos", "-j"]);
        ok.then_some(out)
    });

    if let (Some(out_l), Some(out_m)) = (out_l, out_m) {
        if let (Ok(layers_map), Ok(monitors)) = (
            serde_json::from_str::<HashMap<String, MonitorLayers>>(&out_l),
            serde_json::from_str::<Vec<Monitor>>(&out_m),
        ) {
            if let Some(mon) = monitors.iter().find(|m| m.focused).or_else(|| monitors.first()) {
                mon_x = mon.x;
                mon_y = mon.y;
                let raw_w = if mon.transform % 2 == 1 {
                    mon.height
                } else {
                    mon.width
                };
                if mon.scale > 0.0 {
                    mon_w = ((raw_w as f64) / mon.scale).round() as i32;
                }
                if let Some(ml) = layers_map.get(&mon.name) {
                    'outer: for items in ml.levels.values() {
                        for item in items {
                            if item.namespace == "waybar" {
                                bar_x = item.x;
                                bar_y = item.y;
                                bar_w = item.w;
                                bar_h = item.h;
                                top_margin = bar_y + bar_h + 6;
                                break 'outer;
                            }
                        }
                    }
                }
            }
        }
    }

    let mut cursor_cx = None;
    if let Some(out_c) = out_c {
        if let Ok(pos) = serde_json::from_str::<CursorPos>(&out_c) {
            let cx = pos.x - mon_x;
            let cy = pos.y - mon_y;
            let on_bar_x = cx >= (bar_x - 8) && cx <= (bar_x + bar_w + 8);
            let on_bar_y = cy >= (bar_y - 6) && cy <= (bar_y + bar_h + 10);
            if on_bar_x && on_bar_y {
                cursor_cx = Some(cx);
            }
        }
    }

    PopupPlacement {
        top_margin,
        mon_w,
        bar_x,
        bar_w,
        cursor_cx,
        fallback_align,
    }
}

pub struct WidgetPopup {
    pub window: gtk::Window,
    pub root_box: gtk::Box,
    pub header_actions: gtk::Box,
}

impl WidgetPopup {
    pub fn new(title: &str, icon: &str, align: Align, width: i32) -> Self {
        let placement = get_popup_placement(align);
        let eff_width = width.min((placement.mon_w - 16).max(200));

        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_resizable(false);
        window.set_default_size(eff_width, -1);

        window.init_layer_shell();
        window.set_layer(Layer::Overlay);
        window.set_exclusive_zone(-1);
        window.set_namespace("waybar-widget");
        window.set_keyboard_mode(KeyboardMode::OnDemand);

        let init_left = placement.left_for_width(eff_width);
        window.set_anchor(Edge::Top, true);
        window.set_anchor(Edge::Left, true);
        window.set_layer_shell_margin(Edge::Top, placement.top_margin);
        window.set_layer_shell_margin(Edge::Left, init_left);

        let last_left = Cell::new(init_left);
        window.connect_size_allocate(move |win, alloc| {
            let actual_w = alloc.width();
            if actual_w > 0 {
                let target_left = placement.left_for_width(actual_w);
                if target_left != last_left.get() {
                    last_left.set(target_left);
                    win.set_layer_shell_margin(Edge::Left, target_left);
                }
            }
        });

        let has_focused = Rc::new(Cell::new(false));
        let focus_flag = has_focused.clone();
        window.connect_focus_in_event(move |_, _| {
            focus_flag.set(true);
            glib::Propagation::Proceed
        });

        let focus_out_flag = has_focused;
        window.connect_focus_out_event(move |win, _| {
            if focus_out_flag.get() {
                let win_weak = win.downgrade();
                glib::timeout_add_local(Duration::from_millis(80), move || {
                    if let Some(w) = win_weak.upgrade() {
                        if !w.is_active() {
                            close_app();
                        }
                    }
                    ControlFlow::Break
                });
            }
            glib::Propagation::Proceed
        });

        window.connect_key_press_event(|_, ev| {
            if ev.keyval() == Escape {
                close_app();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });

        window.connect_destroy(|_| close_app());

        let root_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        root_box.style_context().add_class("widget-root");
        root_box.set_size_request(eff_width, -1);
        window.add(&root_box);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.style_context().add_class("widget-header");

        let title_lbl = gtk::Label::new(Some(&format!("{icon}  {title}")));
        title_lbl.style_context().add_class("widget-title");
        title_lbl.set_xalign(0.0);
        header.pack_start(&title_lbl, true, true, 0);

        let header_actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        header.pack_end(&header_actions, false, false, 0);

        let close_btn = gtk::Button::with_label("✕");
        close_btn.style_context().add_class("icon-btn");
        close_btn.connect_clicked(|_| close_app());
        header_actions.pack_end(&close_btn, false, false, 0);

        root_box.pack_start(&header, false, false, 0);

        Self {
            window,
            root_box,
            header_actions,
        }
    }
}

pub fn close_app() {
    cleanup_pid();
    if gtk::main_level() > 0 {
        gtk::main_quit();
    }
}

pub fn make_section_label(text: &str) -> gtk::Label {
    let lbl = gtk::Label::new(Some(text));
    lbl.style_context().add_class("widget-subtitle");
    lbl.set_xalign(0.0);
    lbl
}

pub fn clear_container(container: &gtk::Box) {
    for child in container.children() {
        container.remove(&child);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_popup_left_for_1920_1280_and_rotated() {
        let p_1920 = PopupPlacement {
            top_margin: 48,
            mon_w: 1920,
            bar_x: 180,
            bar_w: 1560,
            cursor_cx: Some(1650),
            fallback_align: Align::Right,
        };
        assert_eq!(p_1920.left_for_width(320), 1490);
        // Even if actual allocated width balloons to 600px, right edge stays within 1920 - 8.
        assert_eq!(p_1920.left_for_width(600), 1312);

        let p_1280_fallback = PopupPlacement {
            top_margin: 44,
            mon_w: 1280,
            bar_x: 80,
            bar_w: 1120,
            cursor_cx: None,
            fallback_align: Align::Right,
        };
        assert_eq!(p_1280_fallback.left_for_width(320), 880);

        let p_1280_cursor = PopupPlacement {
            cursor_cx: Some(1200),
            ..p_1280_fallback
        };
        assert_eq!(p_1280_cursor.left_for_width(320), 952);

        let p_rotated = PopupPlacement {
            top_margin: 44,
            mon_w: 1080,
            bar_x: 100,
            bar_w: 880,
            cursor_cx: Some(950),
            fallback_align: Align::Right,
        };
        assert_eq!(p_rotated.left_for_width(320), 752);
    }
}
