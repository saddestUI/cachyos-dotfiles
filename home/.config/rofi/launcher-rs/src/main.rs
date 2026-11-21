mod apps;
mod calc;
mod theme;

use apps::{filter_and_rank, launch_app, load_apps, load_history, AppEntry, UsageHistory};
use calc::{evaluate_expr, CalcOutput};
use gdk::prelude::*;
use gtk::prelude::*;
use gtk_layer_shell::{KeyboardMode, Layer, LayerShell};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::env;
use std::f64::consts::PI;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::{Duration, Instant};
use theme::{to_hex, to_rgb_u8, Theme};

const WIN_W: i32 = 580;
const ROW_H: f64 = 52.0;
const LIST_VIEW_H: f64 = 382.0;
const ICON_SIZE: i32 = 28;
const FONT_FAMILY: &str = "MesloLGL Nerd Font";

fn pid_file() -> PathBuf {
    let rt = env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(rt).join("asura-launcher.pid")
}

fn is_our_pid(pid: i32) -> bool {
    let cmdline_path = format!("/proc/{pid}/cmdline");
    if let Ok(bytes) = fs::read(cmdline_path) {
        let s = String::from_utf8_lossy(&bytes);
        return s.contains("asura-launcher");
    }
    false
}

fn try_toggle_existing() -> bool {
    let p = pid_file();
    if let Ok(raw) = fs::read_to_string(&p) {
        if let Ok(pid) = raw.trim().parse::<i32>() {
            if pid > 0 && pid != unsafe { libc::getpid() } && is_our_pid(pid) {
                if unsafe { libc::kill(pid, 0) } == 0 {
                    unsafe {
                        libc::kill(pid, libc::SIGTERM);
                    }
                    let _ = fs::remove_file(&p);
                    return true;
                }
            }
        }
    }
    let _ = fs::write(&p, format!("{}", unsafe { libc::getpid() }));
    false
}

fn cleanup_pid() {
    let p = pid_file();
    if let Ok(raw) = fs::read_to_string(&p) {
        if raw.trim().parse::<i32>().ok() == Some(unsafe { libc::getpid() }) {
            let _ = fs::remove_file(p);
        }
    }
}

fn rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w * 0.5).min(h * 0.5).max(0.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI * 0.5, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI * 0.5);
    cr.arc(x + r, y + h - r, r, PI * 0.5, PI);
    cr.arc(x + r, y + r, r, PI, PI * 1.5);
    cr.close_path();
}

fn text_visual_width(ext: &cairo::TextExtents) -> f64 {
    (ext.x_bearing() + ext.width()).max(ext.x_advance())
}

fn truncate_to_width(cr: &cairo::Context, text: &str, max_w: f64) -> String {
    if max_w <= 12.0 || text.is_empty() {
        return String::new();
    }
    if let Ok(ext) = cr.text_extents(text) {
        if text_visual_width(&ext) <= max_w {
            return text.to_string();
        }
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let trimmed: String = chars.iter().collect();
        let cand = format!("{}…", trimmed.trim_end());
        if let Ok(ext) = cr.text_extents(&cand) {
            if text_visual_width(&ext) <= max_w {
                return cand;
            }
        }
    }
    "…".to_string()
}

fn step_selection_wrap(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    ((current as isize + delta).rem_euclid(len as isize)) as usize
}

fn load_app_icon(icon: Option<&gio::Icon>) -> Option<cairo::ImageSurface> {
    let icon = icon?;
    let theme = gtk::IconTheme::default()?;
    let info = theme.lookup_by_gicon(icon, ICON_SIZE, gtk::IconLookupFlags::FORCE_SIZE)?;
    let pb = info.load_icon().ok()?;
    let surf = cairo::ImageSurface::create(cairo::Format::ARgb32, ICON_SIZE, ICON_SIZE).ok()?;
    let cr = cairo::Context::new(&surf).ok()?;
    let pw = pb.width().max(1) as f64;
    let ph = pb.height().max(1) as f64;
    let scale = (ICON_SIZE as f64 / pw).min(ICON_SIZE as f64 / ph);
    cr.translate(
        (ICON_SIZE as f64 - pw * scale) * 0.5,
        (ICON_SIZE as f64 - ph * scale) * 0.5,
    );
    cr.scale(scale, scale);
    cr.set_source_pixbuf(&pb, 0.0, 0.0);
    let _ = cr.paint();
    drop(cr);
    Some(surf)
}

fn copy_to_clipboard(text: &str) {
    if let Ok(mut child) = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
    }
    let _ = Command::new("notify-send")
        .arg("-a")
        .arg("Калькулятор")
        .arg("Результат скопирован")
        .arg(text)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[derive(Clone)]
struct RowVisual {
    y: f64,
    alpha: f64,
    x_offset: f64,
}

struct ExitingRow {
    app_idx: usize,
    y: f64,
    alpha: f64,
    x_offset: f64,
}

#[derive(Clone, PartialEq)]
enum CalcView {
    Inactive,
    Prompt,
    Ok(CalcOutput),
    Err(String),
}

struct LauncherState {
    apps: Vec<AppEntry>,
    history: UsageHistory,
    filtered: Vec<usize>,
    selected: usize,
    query: String,
    theme: Theme,

    // 60fps animation state
    last_tick: Instant,
    open_progress: f64,
    typing_pulse: f64,
    sel_pill_y: f64,
    sel_pill_h: f64,
    scroll_y: f64,
    target_scroll_y: f64,
    scroll_bar_alpha: f64,
    calc_blend: f64,
    result_pop: f64,
    calc_view: CalcView,
    last_calc_expr: String,

    row_visuals: HashMap<usize, RowVisual>,
    exiting_rows: Vec<ExitingRow>,
    icons: HashMap<String, Option<cairo::ImageSurface>>,
    last_pointer: Option<(f64, f64)>,
}

impl LauncherState {
    fn new() -> Self {
        let apps = load_apps();
        let history = load_history();
        let filtered = filter_and_rank(&apps, "", &history);
        let mut row_visuals = HashMap::new();
        for (rank, &app_idx) in filtered.iter().enumerate() {
            row_visuals.insert(
                app_idx,
                RowVisual {
                    y: rank as f64 * ROW_H,
                    alpha: 1.0,
                    x_offset: 0.0,
                },
            );
        }

        Self {
            apps,
            history,
            filtered,
            selected: 0,
            query: String::new(),
            theme: Theme::load(),
            last_tick: Instant::now(),
            open_progress: 0.0,
            typing_pulse: 0.0,
            sel_pill_y: 0.0,
            sel_pill_h: if ROW_H > 0.0 { 1.0 } else { 0.0 },
            scroll_y: 0.0,
            target_scroll_y: 0.0,
            scroll_bar_alpha: 0.0,
            calc_blend: 0.0,
            result_pop: 0.0,
            calc_view: CalcView::Inactive,
            last_calc_expr: String::new(),
            row_visuals,
            exiting_rows: Vec::new(),
            icons: HashMap::new(),
            last_pointer: None,
        }
    }

    fn max_scroll(&self) -> f64 {
        let content_h = self.filtered.len() as f64 * ROW_H;
        (content_h - LIST_VIEW_H).max(0.0)
    }

    fn ensure_selected_visible(&mut self) {
        if self.filtered.is_empty() {
            self.target_scroll_y = 0.0;
            return;
        }
        let max_s = self.max_scroll();
        let item_top = self.selected as f64 * ROW_H;
        let item_bot = item_top + ROW_H;
        let margin = 10.0;

        if item_top - margin < self.target_scroll_y {
            self.target_scroll_y = (item_top - margin).clamp(0.0, max_s);
        } else if item_bot + margin > self.target_scroll_y + LIST_VIEW_H {
            self.target_scroll_y = (item_bot + margin - LIST_VIEW_H).clamp(0.0, max_s);
        }
        self.scroll_bar_alpha = 1.0;
    }

    fn set_query(&mut self, new_query: String) {
        self.typing_pulse = 1.0;
        self.query = new_query;

        let trimmed = self.query.trim_start();
        if let Some(rest) = trimmed.strip_prefix('=') {
            let expr = rest.trim();
            self.last_calc_expr = expr.to_string();
            let prev_view = self.calc_view.clone();
            if expr.is_empty() {
                self.calc_view = CalcView::Prompt;
            } else {
                match evaluate_expr(expr) {
                    Ok(out) => {
                        if !matches!(&prev_view, CalcView::Ok(p) if p.formatted == out.formatted) {
                            self.result_pop = 1.0;
                        }
                        self.calc_view = CalcView::Ok(out);
                    }
                    Err(msg) => {
                        self.calc_view = CalcView::Err(msg);
                    }
                }
            }
            return;
        }

        self.calc_view = CalcView::Inactive;

        let prev_filtered: Vec<usize> = std::mem::take(&mut self.filtered);
        let new_filtered = filter_and_rank(&self.apps, &self.query, &self.history);

        let mut new_set: HashMap<usize, usize> = HashMap::with_capacity(new_filtered.len());
        for (rank, &idx) in new_filtered.iter().enumerate() {
            new_set.insert(idx, rank);
        }

        // Spawn smooth exit trails for items leaving visible window
        for &old_idx in &prev_filtered {
            if !new_set.contains_key(&old_idx) {
                if let Some(rv) = self.row_visuals.remove(&old_idx) {
                    let screen_y = rv.y - self.scroll_y;
                    if rv.alpha > 0.05 && screen_y > -ROW_H && screen_y < LIST_VIEW_H + ROW_H {
                        self.exiting_rows.push(ExitingRow {
                            app_idx: old_idx,
                            y: rv.y,
                            alpha: rv.alpha,
                            x_offset: rv.x_offset,
                        });
                    }
                }
            }
        }
        if self.exiting_rows.len() > 24 {
            self.exiting_rows.drain(0..self.exiting_rows.len() - 24);
        }

        for (rank, &idx) in new_filtered.iter().enumerate() {
            self.row_visuals.entry(idx).or_insert_with(|| RowVisual {
                y: (rank as f64) * ROW_H + 10.0,
                alpha: 0.0,
                x_offset: 14.0,
            });
        }

        self.filtered = new_filtered;
        self.selected = 0;
        self.target_scroll_y = 0.0;
        if self.scroll_y > self.max_scroll() {
            self.target_scroll_y = self.max_scroll();
        }
    }

    fn step_animations(&mut self) -> bool {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64().clamp(0.001, 0.05);
        self.last_tick = now;

        let damp = |speed: f64| 1.0 - (-speed * dt).exp();

        let mut active = false;

        if self.open_progress < 0.999 {
            self.open_progress += (1.0 - self.open_progress) * damp(14.0);
            if (1.0 - self.open_progress).abs() < 0.002 {
                self.open_progress = 1.0;
            }
            active = true;
        }

        if self.typing_pulse > 0.003 {
            self.typing_pulse *= (-8.5 * dt).exp();
            if self.typing_pulse < 0.004 {
                self.typing_pulse = 0.0;
            }
            active = true;
        }

        if self.result_pop > 0.003 {
            self.result_pop *= (-10.0 * dt).exp();
            if self.result_pop < 0.004 {
                self.result_pop = 0.0;
            }
            active = true;
        }

        let calc_target = if matches!(self.calc_view, CalcView::Inactive) {
            0.0
        } else {
            1.0
        };
        if (self.calc_blend - calc_target).abs() > 0.002 {
            self.calc_blend += (calc_target - self.calc_blend) * damp(18.0);
            if (self.calc_blend - calc_target).abs() < 0.003 {
                self.calc_blend = calc_target;
            }
            active = true;
        }

        let target_pill_y = self.selected as f64 * ROW_H;
        if (self.sel_pill_y - target_pill_y).abs() > 0.15 {
            self.sel_pill_y += (target_pill_y - self.sel_pill_y) * damp(24.0);
            active = true;
        } else {
            self.sel_pill_y = target_pill_y;
        }

        let target_pill_h = if self.filtered.is_empty() { 0.0 } else { 1.0 };
        if (self.sel_pill_h - target_pill_h).abs() > 0.005 {
            self.sel_pill_h += (target_pill_h - self.sel_pill_h) * damp(20.0);
            active = true;
        }

        if (self.scroll_y - self.target_scroll_y).abs() > 0.15 {
            self.scroll_y += (self.target_scroll_y - self.scroll_y) * damp(18.0);
            self.scroll_bar_alpha = (self.scroll_bar_alpha + dt * 6.0).min(1.0);
            active = true;
        } else {
            self.scroll_y = self.target_scroll_y;
            if self.scroll_bar_alpha > 0.01 {
                self.scroll_bar_alpha *= (-2.5 * dt).exp();
                if self.scroll_bar_alpha < 0.02 {
                    self.scroll_bar_alpha = 0.0;
                }
                active = true;
            }
        }

        // Update active list row positions & fades
        let k_row = damp(20.0);
        for (rank, &app_idx) in self.filtered.iter().enumerate() {
            if let Some(rv) = self.row_visuals.get_mut(&app_idx) {
                let target_y = rank as f64 * ROW_H;
                if (rv.y - target_y).abs() > 0.15 {
                    rv.y += (target_y - rv.y) * k_row;
                    active = true;
                } else {
                    rv.y = target_y;
                }
                if (1.0 - rv.alpha).abs() > 0.005 {
                    rv.alpha += (1.0 - rv.alpha) * k_row;
                    active = true;
                } else {
                    rv.alpha = 1.0;
                }
                if rv.x_offset.abs() > 0.15 {
                    rv.x_offset += (0.0 - rv.x_offset) * k_row;
                    active = true;
                } else {
                    rv.x_offset = 0.0;
                }
            }
        }

        // Fade out removed rows
        let k_exit = damp(22.0);
        for ex in &mut self.exiting_rows {
            ex.alpha *= (-14.0 * dt).exp();
            ex.x_offset += (-10.0 - ex.x_offset) * k_exit;
            active = true;
        }
        self.exiting_rows.retain(|ex| ex.alpha > 0.025);

        active
    }

    fn get_icon(&mut self, app_idx: usize) -> Option<&cairo::ImageSurface> {
        let key = self.apps[app_idx].id.clone();
        if !self.icons.contains_key(&key) {
            let loaded = load_app_icon(self.apps[app_idx].icon.as_ref());
            self.icons.insert(key.clone(), loaded);
        }
        self.icons.get(&key).and_then(|opt| opt.as_ref())
    }
}

fn draw_app_row(
    cr: &cairo::Context,
    app: &AppEntry,
    icon_surf: Option<&cairo::ImageSurface>,
    theme: &Theme,
    w: f64,
    y: f64,
    x_offset: f64,
    alpha: f64,
    selected_strength: f64,
) {
    if alpha <= 0.01 || y + ROW_H < -10.0 || y > LIST_VIEW_H + 10.0 {
        return;
    }

    let pad_x = 12.0 + x_offset;
    let icon_x = pad_x;
    let icon_y = y + (ROW_H - ICON_SIZE as f64) * 0.5;

    cr.save().ok();

    if let Some(surf) = icon_surf {
        let _ = cr.set_source_surface(surf, icon_x, icon_y);
        let _ = cr.paint_with_alpha(alpha);
    } else {
        // Crisp rounded geometric glyph fallback when app has no theme icon
        rounded_rect(cr, icon_x, icon_y, ICON_SIZE as f64, ICON_SIZE as f64, 7.0);
        let [r, g, b] = theme.primary;
        cr.set_source_rgba(r, g, b, 0.25 * alpha);
        let _ = cr.fill_preserve();
        cr.set_source_rgba(r, g, b, 0.55 * alpha);
        cr.set_line_width(1.0);
        let _ = cr.stroke();
        cr.select_font_face(FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Bold);
        cr.set_font_size(14.0);
        let first = app.name.chars().next().unwrap_or('A').to_uppercase().to_string();
        if let Ok(ext) = cr.text_extents(&first) {
            let [fr, fg, fb] = theme.fg;
            cr.set_source_rgba(fr, fg, fb, 0.9 * alpha);
            cr.move_to(
                icon_x + (ICON_SIZE as f64 - ext.width()) * 0.5 - ext.x_bearing(),
                icon_y + (ICON_SIZE as f64 - ext.height()) * 0.5 - ext.y_bearing(),
            );
            let _ = cr.show_text(&first);
        }
    }

    let text_x = pad_x + ICON_SIZE as f64 + 13.0;
    let tty_layout = if app.terminal {
        cr.select_font_face(FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Bold);
        cr.set_font_size(10.0);
        cr.text_extents("TTY").ok().map(|ext| {
            let pill_w = ext.width() + 12.0;
            let pill_x = w - pill_w - 18.0 + x_offset;
            let pill_y = y + (ROW_H - 18.0) * 0.5;
            let text_bx = pill_x + (pill_w - ext.width()) * 0.5 - ext.x_bearing();
            let text_by = pill_y + (18.0 - ext.height()) * 0.5 - ext.y_bearing();
            (pill_x, pill_w, pill_y, text_bx, text_by)
        })
    } else {
        None
    };

    let right_limit = match tty_layout {
        Some((pill_x, ..)) => pill_x - 14.0,
        None => w - 22.0 + x_offset,
    };
    let max_text_w = (right_limit - text_x).max(40.0);

    let title_rgb = [
        theme.fg[0] + (theme.on_accent[0] - theme.fg[0]) * selected_strength,
        theme.fg[1] + (theme.on_accent[1] - theme.fg[1]) * selected_strength,
        theme.fg[2] + (theme.on_accent[2] - theme.fg[2]) * selected_strength,
    ];
    let sub_rgb = [
        theme.fg_muted[0] + (theme.on_accent[0] - theme.fg_muted[0]) * selected_strength * 0.85,
        theme.fg_muted[1] + (theme.on_accent[1] - theme.fg_muted[1]) * selected_strength * 0.85,
        theme.fg_muted[2] + (theme.on_accent[2] - theme.fg_muted[2]) * selected_strength * 0.85,
    ];

    cr.select_font_face(FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(14.0);
    let title = truncate_to_width(cr, &app.name, max_text_w);
    cr.set_source_rgba(title_rgb[0], title_rgb[1], title_rgb[2], 0.96 * alpha);
    if app.subtitle.is_empty() {
        cr.move_to(text_x, y + ROW_H * 0.5 + 5.0);
        let _ = cr.show_text(&title);
    } else {
        cr.move_to(text_x, y + 22.0);
        let _ = cr.show_text(&title);

        cr.select_font_face(FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
        cr.set_font_size(11.5);
        let sub = truncate_to_width(cr, &app.subtitle, max_text_w);
        cr.set_source_rgba(
            sub_rgb[0],
            sub_rgb[1],
            sub_rgb[2],
            (0.82 + 0.10 * selected_strength) * alpha,
        );
        cr.move_to(text_x, y + 39.0);
        let _ = cr.show_text(&sub);
    }

    if let Some((pill_x, pill_w, pill_y, text_bx, text_by)) = tty_layout {
        cr.select_font_face(FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Bold);
        cr.set_font_size(10.0);
        rounded_rect(cr, pill_x, pill_y, pill_w, 18.0, 6.0);
        cr.set_source_rgba(title_rgb[0], title_rgb[1], title_rgb[2], 0.14 * alpha);
        let _ = cr.fill();
        cr.set_source_rgba(title_rgb[0], title_rgb[1], title_rgb[2], 0.72 * alpha);
        cr.move_to(text_bx, text_by);
        let _ = cr.show_text("TTY");
    }

    cr.restore().ok();
}

fn draw_calc_card(cr: &cairo::Context, st: &LauncherState, w: f64, h: f64, alpha: f64) {
    if alpha <= 0.01 {
        return;
    }
    let theme = &st.theme;
    let card_h = 200.0;
    let card_y = ((h - card_h) * 0.36).max(16.0) + (1.0 - alpha) * 14.0;
    let card_x = 4.0;
    let card_w = w - 8.0;

    cr.save().ok();

    // Elevated calc surface card
    rounded_rect(cr, card_x, card_y, card_w, card_h, 16.0);
    let [sr, sg, sb] = theme.surface;
    cr.set_source_rgba(sr, sg, sb, 0.92 * alpha);
    let _ = cr.fill_preserve();

    let grad = cairo::LinearGradient::new(card_x, card_y, card_x + card_w, card_y + card_h);
    let [pr, pg, pb] = theme.primary;
    let [qr, qg, qb] = theme.pill_end;
    grad.add_color_stop_rgba(0.0, pr, pg, pb, (0.45 + 0.35 * st.typing_pulse) * alpha);
    grad.add_color_stop_rgba(1.0, qr, qg, qb, (0.28 + 0.35 * st.typing_pulse) * alpha);
    let _ = cr.set_source(&grad);
    cr.set_line_width(1.5);
    let _ = cr.stroke();

    // Header badge
    cr.select_font_face(FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(11.5);
    let expr_preview = if st.last_calc_expr.is_empty() {
        "КАЛЬКУЛЯТОР ASURA".to_string()
    } else {
        truncate_to_width(cr, &format!("= {}", st.last_calc_expr), card_w - 48.0)
    };
    cr.set_source_rgba(pr, pg, pb, 0.92 * alpha);
    cr.move_to(card_x + 24.0, card_y + 34.0);
    let _ = cr.show_text(&expr_preview);

    // Thin divider
    cr.set_source_rgba(theme.fg[0], theme.fg[1], theme.fg[2], 0.08 * alpha);
    cr.set_line_width(1.0);
    cr.move_to(card_x + 24.0, card_y + 48.0);
    cr.line_to(card_x + card_w - 24.0, card_y + 48.0);
    let _ = cr.stroke();

    match &st.calc_view {
        CalcView::Prompt => {
            cr.select_font_face(FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
            cr.set_font_size(15.0);
            cr.set_source_rgba(theme.fg_muted[0], theme.fg_muted[1], theme.fg_muted[2], 0.85 * alpha);
            cr.move_to(card_x + 24.0, card_y + 114.0);
            let _ = cr.show_text("Выражение…");
        }
        CalcView::Ok(out) => {
            let pop_scale = 1.0 + st.result_pop * 0.035;
            cr.save().ok();
            let cx = card_x + 24.0;
            let cy = card_y + 126.0;
            cr.translate(cx, cy);
            cr.scale(pop_scale, pop_scale);

            let mut font_sz = 40.0;
            cr.select_font_face(FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            while font_sz > 18.0 {
                cr.set_font_size(font_sz);
                if let Ok(ext) = cr.text_extents(&out.formatted) {
                    if ext.width() <= card_w - 52.0 {
                        break;
                    }
                }
                font_sz -= 3.0;
            }
            let txt = truncate_to_width(cr, &out.formatted, card_w - 52.0);
            cr.set_source_rgba(theme.fg[0], theme.fg[1], theme.fg[2], 0.98 * alpha);
            cr.move_to(0.0, 0.0);
            let _ = cr.show_text(&txt);
            cr.restore().ok();
        }
        CalcView::Err(msg) => {
            cr.select_font_face(FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
            cr.set_font_size(16.0);
            cr.set_source_rgba(theme.secondary[0], theme.secondary[1], theme.secondary[2], 0.90 * alpha);
            let txt = truncate_to_width(cr, msg, card_w - 48.0);
            cr.move_to(card_x + 24.0, card_y + 120.0);
            let _ = cr.show_text(&txt);
        }
        CalcView::Inactive => {}
    }

    cr.restore().ok();
}

fn install_css(theme: &Theme) {
    let fg_hex = to_hex(theme.fg);
    let [pr, pg, pb] = to_rgb_u8(theme.primary);
    let [cr_r, cr_g, cr_b] = to_rgb_u8(theme.secondary);
    let [on_r, on_g, on_b] = to_rgb_u8(theme.on_accent);

    let css = format!(
        r#"
window.asura-window {{
    background-color: transparent;
}}
entry.asura-entry {{
    background-color: transparent;
    background-image: none;
    border: none;
    box-shadow: none;
    outline: none;
    color: {fg_hex};
    caret-color: rgb({pr}, {pg}, {pb});
    font-family: "{FONT_FAMILY}", "Noto Sans", sans-serif;
    font-size: 16px;
    font-weight: 500;
    padding: 6px 2px;
    min-height: 28px;
}}
entry.asura-entry selection {{
    background-color: rgba({pr}, {pg}, {pb}, 0.82);
    color: rgb({on_r}, {on_g}, {on_b});
}}
label.asura-counter {{
    color: rgba({cr_r}, {cr_g}, {cr_b}, 0.85);
    font-family: "{FONT_FAMILY}", sans-serif;
    font-size: 11px;
    font-weight: 700;
}}
"#,
    );

    let provider = gtk::CssProvider::new();
    let _ = provider.load_from_data(css.as_bytes());
    if let Some(screen) = gdk::Screen::default() {
        gtk::StyleContext::add_provider_for_screen(
            &screen,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn activate_current(st: &Rc<RefCell<LauncherState>>, entry: &gtk::Entry) {
    let st_b = st.borrow();
    match &st_b.calc_view {
        CalcView::Ok(out) => {
            let formatted = out.formatted.clone();
            drop(st_b);
            copy_to_clipboard(&formatted);
            cleanup_pid();
            gtk::main_quit();
        }
        CalcView::Prompt | CalcView::Err(_) => {
            entry.grab_focus_without_selecting();
        }
        CalcView::Inactive => {
            if let Some(&app_idx) = st_b.filtered.get(st_b.selected) {
                let app = st_b.apps[app_idx].clone();
                drop(st_b);
                cleanup_pid();
                launch_app(&app);
                gtk::main_quit();
            }
        }
    }
}

fn run_check() -> i32 {
    let apps = load_apps();
    let theme = Theme::load();
    let calc_ok = evaluate_expr("2+2*5")
        .map(|o| o.formatted == "12")
        .unwrap_or(false);
    println!(
        "asura-launcher ok | apps: {} | primary: {} | secondary: {} | calc 2+2*5={}",
        apps.len(),
        to_hex(theme.primary),
        to_hex(theme.secondary),
        if calc_ok { "12" } else { "err" }
    );
    if apps.len() > 10 && calc_ok {
        0
    } else {
        1
    }
}

fn main() {
    if env::args().any(|a| a == "--check") {
        std::process::exit(run_check());
    }

    if try_toggle_existing() {
        return;
    }

    gtk::init().expect("Не удалось инициализировать GTK3");

    let state = Rc::new(RefCell::new(LauncherState::new()));
    install_css(&state.borrow().theme);

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.style_context().add_class("asura-window");
    window.set_app_paintable(true);
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_default_size(WIN_W, -1);

    if let Some(screen) = gtk::prelude::WidgetExt::screen(&window) {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
    }

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_exclusive_zone(-1);
    window.set_namespace("asura-launcher");
    window.set_keyboard_mode(KeyboardMode::Exclusive);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 10);
    root.set_margin_start(16);
    root.set_margin_end(16);
    root.set_margin_top(16);
    root.set_margin_bottom(12);
    window.add(&root);

    // Search bar row: dynamic badge + native GtkEntry + match count label
    let search_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    search_row.set_margin_start(10);
    search_row.set_margin_end(12);
    search_row.set_margin_top(4);
    search_row.set_margin_bottom(4);
    root.pack_start(&search_row, false, false, 0);

    let badge = gtk::DrawingArea::new();
    badge.set_size_request(28, 28);
    badge.set_can_focus(false);
    search_row.pack_start(&badge, false, false, 0);

    let entry = gtk::Entry::new();
    entry.style_context().add_class("asura-entry");
    entry.set_has_frame(false);
    entry.set_hexpand(true);
    entry.set_placeholder_text(Some("Поиск…"));
    search_row.pack_start(&entry, true, true, 0);

    let counter = gtk::Label::new(Some(""));
    counter.style_context().add_class("asura-counter");
    search_row.pack_end(&counter, false, false, 0);

    let update_counter = {
        let state = state.clone();
        let counter = counter.clone();
        move || {
            let st = state.borrow();
            match &st.calc_view {
                CalcView::Inactive => {
                    counter.set_text(&format!("{} / {}", st.filtered.len(), st.apps.len()));
                }
                CalcView::Ok(_) => counter.set_text("="),
                CalcView::Prompt | CalcView::Err(_) => counter.set_text("КАЛЬКУЛЯТОР"),
            }
        }
    };
    update_counter();

    // 60 FPS list + calculator DrawingArea
    let list_area = gtk::DrawingArea::new();
    list_area.set_size_request(WIN_W - 32, LIST_VIEW_H as i32);
    list_area.set_can_focus(false);
    list_area.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::POINTER_MOTION_MASK
            | gdk::EventMask::SCROLL_MASK
            | gdk::EventMask::SMOOTH_SCROLL_MASK,
    );
    root.pack_start(&list_area, false, false, 0);

    // Draw search bar card & glass panel background
    let state_win_draw = state.clone();
    let search_row_ref = search_row.clone();
    window.connect_draw(move |win, cr| {
        let alloc = win.allocation();
        let w = alloc.width() as f64;
        let h = alloc.height() as f64;
        let st = state_win_draw.borrow();
        let t = &st.theme;

        cr.set_operator(cairo::Operator::Source);
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
        let _ = cr.paint();
        cr.set_operator(cairo::Operator::Over);

        // Main rounded translucent frosted shell
        rounded_rect(cr, 0.5, 0.5, w - 1.0, h - 1.0, 18.0);
        cr.set_source_rgba(t.bg[0], t.bg[1], t.bg[2], 0.90);
        let _ = cr.fill_preserve();

        // Soft top ambient radial glow synced to wallpaper accent
        let rad = cairo::RadialGradient::new(w * 0.25, 0.0, 10.0, w * 0.25, 0.0, w * 0.75);
        rad.add_color_stop_rgba(0.0, t.primary[0], t.primary[1], t.primary[2], 0.14);
        rad.add_color_stop_rgba(0.6, t.secondary[0], t.secondary[1], t.secondary[2], 0.05);
        rad.add_color_stop_rgba(1.0, t.bg[0], t.bg[1], t.bg[2], 0.0);
        let _ = cr.set_source(&rad);
        let _ = cr.fill_preserve();

        let border_grad = cairo::LinearGradient::new(0.0, 0.0, w, h);
        border_grad.add_color_stop_rgba(0.0, t.primary[0], t.primary[1], t.primary[2], 0.52);
        border_grad.add_color_stop_rgba(0.5, t.secondary[0], t.secondary[1], t.secondary[2], 0.30);
        border_grad.add_color_stop_rgba(1.0, t.primary[0], t.primary[1], t.primary[2], 0.42);
        let _ = cr.set_source(&border_grad);
        cr.set_line_width(1.4);
        let _ = cr.stroke();

        // Search input box card positioned around search_row allocation
        let s_alloc = search_row_ref.allocation();
        let sx = (s_alloc.x() - 4) as f64;
        let sy = (s_alloc.y() - 3) as f64;
        let sw = (s_alloc.width() + 8) as f64;
        let sh = (s_alloc.height() + 6) as f64;
        if sw > 20.0 && sh > 10.0 {
            if st.typing_pulse > 0.01 {
                rounded_rect(cr, sx - 2.0, sy - 2.0, sw + 4.0, sh + 4.0, 15.0);
                cr.set_source_rgba(
                    t.primary[0],
                    t.primary[1],
                    t.primary[2],
                    0.22 * st.typing_pulse,
                );
                cr.set_line_width(3.0);
                let _ = cr.stroke();
            }

            rounded_rect(cr, sx, sy, sw, sh, 13.0);
            cr.set_source_rgba(t.input_bg[0], t.input_bg[1], t.input_bg[2], 0.88);
            let _ = cr.fill_preserve();

            let glow = 0.28 + 0.55 * st.typing_pulse;
            cr.set_source_rgba(t.primary[0], t.primary[1], t.primary[2], glow);
            cr.set_line_width(1.25);
            let _ = cr.stroke();
        }

        glib::Propagation::Proceed
    });

    // Draw animated search / "=" mode badge
    let state_badge = state.clone();
    badge.connect_draw(move |widget, cr| {
        let alloc = widget.allocation();
        let w = alloc.width() as f64;
        let h = alloc.height() as f64;
        let st = state_badge.borrow();
        let t = &st.theme;

        let cx = w * 0.5;
        let cy = h * 0.5;
        let pulse = 1.0 + st.typing_pulse * 0.10;

        cr.save().ok();
        cr.translate(cx, cy);
        cr.scale(pulse, pulse);

        let calc_a = st.calc_blend;
        let search_a = 1.0 - calc_a;

        if search_a > 0.02 {
            cr.set_source_rgba(t.primary[0], t.primary[1], t.primary[2], 0.92 * search_a);
            cr.set_line_width(2.0);
            cr.arc(-1.5, -1.5, 6.2, 0.0, 2.0 * PI);
            let _ = cr.stroke();
            cr.move_to(3.0, 3.0);
            cr.line_to(7.5, 7.5);
            cr.set_line_cap(cairo::LineCap::Round);
            let _ = cr.stroke();
        }
        if calc_a > 0.02 {
            rounded_rect(cr, -11.0, -11.0, 22.0, 22.0, 7.0);
            cr.set_source_rgba(t.primary[0], t.primary[1], t.primary[2], 0.25 * calc_a);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(t.primary[0], t.primary[1], t.primary[2], 0.85 * calc_a);
            cr.set_line_width(1.2);
            let _ = cr.stroke();

            cr.select_font_face(FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            cr.set_font_size(15.0);
            cr.set_source_rgba(t.secondary[0], t.secondary[1], t.secondary[2], 0.98 * calc_a);
            cr.move_to(-4.5, 5.5);
            let _ = cr.show_text("=");
        }
        cr.restore().ok();

        glib::Propagation::Stop
    });

    // Draw 60fps list rows, gliding selection pill, scrollbar, or smooth calc card
    let state_draw = state.clone();
    list_area.connect_draw(move |widget, cr| {
        let alloc = widget.allocation();
        let w = alloc.width() as f64;
        let h = alloc.height() as f64;

        let mut st = state_draw.borrow_mut();
        let theme = st.theme.clone();
        let calc_a = st.calc_blend;
        let list_a = 1.0 - calc_a;
        let scroll_y = st.scroll_y;
        let open_p = st.open_progress;

        cr.rectangle(0.0, 0.0, w, h);
        cr.clip();

        if list_a > 0.01 {
            cr.save().ok();

            // 1. Exiting ghost items
            let exiting: Vec<(usize, f64, f64, f64)> = st
                .exiting_rows
                .iter()
                .map(|e| (e.app_idx, e.y, e.x_offset, e.alpha))
                .collect();
            for (app_idx, y, x_off, a) in exiting {
                let app = st.apps[app_idx].clone();
                let screen_y = y - scroll_y;
                if screen_y > -ROW_H && screen_y < h + ROW_H {
                    let icon = st.get_icon(app_idx).cloned();
                    draw_app_row(
                        cr,
                        &app,
                        icon.as_ref(),
                        &theme,
                        w,
                        screen_y,
                        x_off,
                        a * list_a,
                        0.0,
                    );
                }
            }

            // 2. 60fps gliding selection pill
            let pill_screen_y = st.sel_pill_y - scroll_y;
            if st.sel_pill_h > 0.01 && !st.filtered.is_empty() && pill_screen_y > -ROW_H && pill_screen_y < h {
                let pad_v = 3.0;
                let px = 4.0;
                let py = pill_screen_y + pad_v;
                let pw = w - 12.0;
                let ph = ROW_H - pad_v * 2.0;
                let pill_alpha = list_a * st.sel_pill_h * open_p;

                rounded_rect(cr, px, py, pw, ph, 12.0);
                let pill_grad = cairo::LinearGradient::new(px, py, px + pw, py + ph);
                pill_grad.add_color_stop_rgba(
                    0.0,
                    theme.primary[0],
                    theme.primary[1],
                    theme.primary[2],
                    0.90 * pill_alpha,
                );
                pill_grad.add_color_stop_rgba(
                    1.0,
                    theme.pill_end[0],
                    theme.pill_end[1],
                    theme.pill_end[2],
                    0.82 * pill_alpha,
                );
                let _ = cr.set_source(&pill_grad);
                let _ = cr.fill_preserve();

                // Crisp top sheen
                cr.set_source_rgba(1.0, 1.0, 1.0, 0.22 * pill_alpha);
                cr.set_line_width(1.0);
                let _ = cr.stroke();
            }

            // 3. Visible active rows with smooth stagger reveal & position lerp
            if st.filtered.is_empty() {
                cr.select_font_face(FONT_FAMILY, cairo::FontSlant::Normal, cairo::FontWeight::Normal);
                cr.set_font_size(14.0);
                cr.set_source_rgba(
                    theme.fg_muted[0],
                    theme.fg_muted[1],
                    theme.fg_muted[2],
                    0.85 * list_a,
                );
                let msg = "Ничего не найдено";
                if let Ok(ext) = cr.text_extents(msg) {
                    cr.move_to((w - ext.width()) * 0.5, h * 0.42);
                    let _ = cr.show_text(msg);
                }
            } else {
                let filtered = st.filtered.clone();
                let selected = st.selected;
                let pill_y = st.sel_pill_y;
                for (rank, app_idx) in filtered.into_iter().enumerate() {
                    let rv = st.row_visuals.get(&app_idx).cloned().unwrap_or(RowVisual {
                        y: rank as f64 * ROW_H,
                        alpha: 1.0,
                        x_offset: 0.0,
                    });
                    let stagger = ((open_p * 1.4) - (rank.min(8) as f64) * 0.045).clamp(0.0, 1.0);
                    let entrance_y = (1.0 - stagger) * 14.0;
                    let screen_y = rv.y - scroll_y + entrance_y;

                    if screen_y + ROW_H < -16.0 || screen_y > h + 16.0 {
                        continue;
                    }
                    let dist = ((rv.y - pill_y) / ROW_H).abs();
                    let sel_strength = if rank == selected {
                        (1.0 - dist.min(1.0) * 0.55) * list_a
                    } else {
                        (1.0 - dist).clamp(0.0, 1.0) * 0.5 * list_a
                    };
                    let app = st.apps[app_idx].clone();
                    let icon = st.get_icon(app_idx).cloned();
                    draw_app_row(
                        cr,
                        &app,
                        icon.as_ref(),
                        &theme,
                        w,
                        screen_y,
                        rv.x_offset,
                        rv.alpha * stagger * list_a,
                        sel_strength,
                    );
                }
            }

            // 4. Smooth fading pill scrollbar
            let total_h = st.filtered.len() as f64 * ROW_H;
            if total_h > h + 2.0 {
                let sb_alpha = (0.18 + 0.65 * st.scroll_bar_alpha) * list_a;
                let ratio = (h / total_h).clamp(0.08, 1.0);
                let thumb_h = (h * ratio).max(28.0);
                let track_h = h - thumb_h;
                let frac = (scroll_y / (total_h - h)).clamp(0.0, 1.0);
                let thumb_y = frac * track_h;

                rounded_rect(cr, w - 5.0, thumb_y + 2.0, 3.5, thumb_h - 4.0, 2.0);
                cr.set_source_rgba(theme.primary[0], theme.primary[1], theme.primary[2], sb_alpha);
                let _ = cr.fill();
            }

            cr.restore().ok();
        }

        // 5. Smooth calculator card crossfade
        if calc_a > 0.01 {
            draw_calc_card(cr, &st, w, h, calc_a * open_p);
        }

        glib::Propagation::Stop
    });

    // On-demand 60 FPS animation scheduler (0% CPU / zero wakeups when settled)
    let anim_active = Rc::new(Cell::new(false));
    let ensure_animating: Rc<dyn Fn()> = {
        let state = state.clone();
        let list_ref = list_area.clone();
        let badge_ref = badge.clone();
        let win_ref = window.clone();
        let active_flag = anim_active;
        Rc::new(move || {
            if active_flag.get() {
                return;
            }
            active_flag.set(true);
            state.borrow_mut().last_tick = Instant::now();
            let st_t = state.clone();
            let list_t = list_ref.clone();
            let badge_t = badge_ref.clone();
            let win_t = win_ref.clone();
            let flag_t = active_flag.clone();
            glib::timeout_add_local(Duration::from_millis(16), move || {
                let running = st_t.borrow_mut().step_animations();
                if running {
                    list_t.queue_draw();
                    badge_t.queue_draw();
                    win_t.queue_draw();
                    glib::ControlFlow::Continue
                } else {
                    flag_t.set(false);
                    glib::ControlFlow::Break
                }
            });
        })
    };

    // React to typing cleanly (no manual selection blocking -> native Ctrl+A works seamlessly)
    {
        let state = state.clone();
        let entry_ref = entry.clone();
        let list_ref = list_area.clone();
        let win_ref = window.clone();
        let badge_ref = badge.clone();
        let update_counter = update_counter.clone();
        let kick = ensure_animating.clone();
        entry.connect_changed(move |_| {
            let text = entry_ref.text().to_string();
            state.borrow_mut().set_query(text);
            update_counter();
            list_ref.queue_draw();
            badge_ref.queue_draw();
            win_ref.queue_draw();
            kick();
        });
    }

    // Keyboard navigation & Ctrl+A (EN/RU)
    {
        let state = state.clone();
        let entry_ref = entry.clone();
        let list_ref = list_area.clone();
        let update_counter = update_counter.clone();
        let kick = ensure_animating.clone();
        window.connect_key_press_event(move |_, ev| {
            let keyval = ev.keyval();
            let mods = ev.state();
            let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
            let alt_super = mods.intersects(gdk::ModifierType::MOD1_MASK | gdk::ModifierType::SUPER_MASK);
            let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);

            if keyval == gdk::keys::constants::Escape {
                cleanup_pid();
                gtk::main_quit();
                return glib::Propagation::Stop;
            }

            if ctrl && !alt_super && !shift {
                match keyval.to_unicode() {
                    Some('a' | 'A' | 'ф' | 'Ф') => {
                        entry_ref.select_region(0, -1);
                        return glib::Propagation::Stop;
                    }
                    Some('j' | 'J' | 'n' | 'N' | 'о' | 'О' | 'т' | 'Т') => {
                        let mut st = state.borrow_mut();
                        if !st.filtered.is_empty() {
                            st.selected = step_selection_wrap(st.selected, st.filtered.len(), 1);
                            st.ensure_selected_visible();
                        }
                        drop(st);
                        list_ref.queue_draw();
                        kick();
                        return glib::Propagation::Stop;
                    }
                    Some('k' | 'K' | 'p' | 'P' | 'л' | 'Л' | 'з' | 'З') => {
                        let mut st = state.borrow_mut();
                        if !st.filtered.is_empty() {
                            st.selected = step_selection_wrap(st.selected, st.filtered.len(), -1);
                            st.ensure_selected_visible();
                        }
                        drop(st);
                        list_ref.queue_draw();
                        kick();
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }

            match keyval {
                gdk::keys::constants::Down => {
                    let mut st = state.borrow_mut();
                    if matches!(st.calc_view, CalcView::Inactive) && !st.filtered.is_empty() {
                        st.selected = step_selection_wrap(st.selected, st.filtered.len(), 1);
                        st.ensure_selected_visible();
                        drop(st);
                        list_ref.queue_draw();
                        kick();
                    }
                    glib::Propagation::Stop
                }
                gdk::keys::constants::Up => {
                    let mut st = state.borrow_mut();
                    if matches!(st.calc_view, CalcView::Inactive) && !st.filtered.is_empty() {
                        st.selected = step_selection_wrap(st.selected, st.filtered.len(), -1);
                        st.ensure_selected_visible();
                        drop(st);
                        list_ref.queue_draw();
                        kick();
                    }
                    glib::Propagation::Stop
                }
                gdk::keys::constants::Page_Down => {
                    let mut st = state.borrow_mut();
                    if matches!(st.calc_view, CalcView::Inactive) && !st.filtered.is_empty() {
                        st.selected = (st.selected + 6).min(st.filtered.len() - 1);
                        st.ensure_selected_visible();
                        drop(st);
                        list_ref.queue_draw();
                        kick();
                    }
                    glib::Propagation::Stop
                }
                gdk::keys::constants::Page_Up => {
                    let mut st = state.borrow_mut();
                    if matches!(st.calc_view, CalcView::Inactive) {
                        st.selected = st.selected.saturating_sub(6);
                        st.ensure_selected_visible();
                        drop(st);
                        list_ref.queue_draw();
                        kick();
                    }
                    glib::Propagation::Stop
                }
                gdk::keys::constants::Return | gdk::keys::constants::KP_Enter => {
                    activate_current(&state, &entry_ref);
                    glib::Propagation::Stop
                }
                gdk::keys::constants::Tab => {
                    let maybe_val = {
                        let st = state.borrow();
                        if let CalcView::Ok(out) = &st.calc_view {
                            Some(format!("= {}", out.formatted))
                        } else {
                            None
                        }
                    };
                    if let Some(new_txt) = maybe_val {
                        entry_ref.set_text(&new_txt);
                        entry_ref.set_position(-1);
                        update_counter();
                        kick();
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
    }

    // Smooth wheel / touchpad scrolling
    {
        let state = state.clone();
        let list_ref = list_area.clone();
        let kick = ensure_animating.clone();
        list_area.connect_scroll_event(move |_, ev| {
            let mut st = state.borrow_mut();
            if !matches!(st.calc_view, CalcView::Inactive) {
                return glib::Propagation::Proceed;
            }
            let delta = match ev.direction() {
                gdk::ScrollDirection::Up => -ROW_H * 1.4,
                gdk::ScrollDirection::Down => ROW_H * 1.4,
                gdk::ScrollDirection::Smooth => ev.delta().1 * 36.0,
                _ => 0.0,
            };
            if delta.abs() > 0.01 {
                let max_s = st.max_scroll();
                st.target_scroll_y = (st.target_scroll_y + delta).clamp(0.0, max_s);
                st.scroll_bar_alpha = 1.0;
                drop(st);
                list_ref.queue_draw();
                kick();
            }
            glib::Propagation::Stop
        });
    }

    // Mouse hover ONLY when pointer coordinates actually move (never fights keyboard)
    {
        let state = state.clone();
        let list_ref = list_area.clone();
        let kick = ensure_animating.clone();
        list_area.connect_motion_notify_event(move |_, ev| {
            let (px, py) = ev.position();
            let mut st = state.borrow_mut();
            if !matches!(st.calc_view, CalcView::Inactive) || st.filtered.is_empty() {
                return glib::Propagation::Proceed;
            }
            if let Some((lx, ly)) = st.last_pointer {
                if (px - lx).hypot(py - ly) < 1.5 {
                    return glib::Propagation::Proceed;
                }
            }
            st.last_pointer = Some((px, py));
            let row = ((py + st.scroll_y) / ROW_H).floor() as isize;
            if row >= 0 && (row as usize) < st.filtered.len() && st.selected != row as usize {
                st.selected = row as usize;
                drop(st);
                list_ref.queue_draw();
                kick();
            }
            glib::Propagation::Proceed
        });
    }

    // Mouse click launches clicked app
    {
        let state = state.clone();
        let entry_ref = entry.clone();
        list_area.connect_button_press_event(move |_, ev| {
            if ev.button() == 1 {
                let (_, py) = ev.position();
                let mut st = state.borrow_mut();
                if matches!(st.calc_view, CalcView::Inactive) {
                    let row = ((py + st.scroll_y) / ROW_H).floor() as isize;
                    if row >= 0 && (row as usize) < st.filtered.len() {
                        st.selected = row as usize;
                        drop(st);
                        activate_current(&state, &entry_ref);
                        return glib::Propagation::Stop;
                    }
                }
                entry_ref.grab_focus_without_selecting();
            }
            glib::Propagation::Proceed
        });
    }

    ensure_animating();

    glib::unix_signal_add_local_once(libc::SIGTERM, || {
        cleanup_pid();
        gtk::main_quit();
    });
    glib::unix_signal_add_local_once(libc::SIGINT, || {
        cleanup_pid();
        gtk::main_quit();
    });

    window.show_all();
    entry.grab_focus();
    gtk::main();
    cleanup_pid();
}

#[cfg(test)]
mod tests {
    use super::step_selection_wrap;

    #[test]
    fn selection_wraps_infinitely_at_bounds() {
        assert_eq!(step_selection_wrap(0, 10, -1), 9);
        assert_eq!(step_selection_wrap(9, 10, 1), 0);
        assert_eq!(step_selection_wrap(3, 10, 1), 4);
        assert_eq!(step_selection_wrap(3, 10, -1), 2);
        assert_eq!(step_selection_wrap(0, 1, -1), 0);
        assert_eq!(step_selection_wrap(0, 1, 1), 0);
        assert_eq!(step_selection_wrap(0, 0, -1), 0);
        assert_eq!(step_selection_wrap(0, 0, 1), 0);
    }
}
