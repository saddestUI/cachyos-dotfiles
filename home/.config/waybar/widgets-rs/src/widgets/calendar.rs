use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use glib::ControlFlow;
use gtk::prelude::*;

use crate::common::{Align, WidgetPopup};

const DAYS_RU: [&str; 8] = [
    "",
    "Понедельник",
    "Вторник",
    "Среда",
    "Четверг",
    "Пятница",
    "Суббота",
    "Воскресенье",
];

const WEEKDAYS_SHORT_RU: [&str; 7] = ["Пн", "Вт", "Ср", "Чт", "Пт", "Сб", "Вс"];

const MONTHS_GEN_RU: [&str; 13] = [
    "",
    "января",
    "февраля",
    "марта",
    "апреля",
    "мая",
    "июня",
    "июля",
    "августа",
    "сентября",
    "октября",
    "ноября",
    "декабря",
];

const MONTHS_NOM_RU: [&str; 13] = [
    "",
    "Январь",
    "Февраль",
    "Март",
    "Апрель",
    "Май",
    "Июнь",
    "Июль",
    "Август",
    "Сентябрь",
    "Октябрь",
    "Ноябрь",
    "Декабрь",
];

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap(year) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

fn to_jdn(year: i32, month: u32, day: u32) -> i64 {
    let (y, m, d) = (year as i64, month as i64, day as i64);
    let a = (14 - m) / 12;
    let y2 = y + 4800 - a;
    let m2 = m + 12 * a - 3;
    d + (153 * m2 + 2) / 5 + 365 * y2 + y2 / 4 - y2 / 100 + y2 / 400 - 32045
}

/// Returns ISO weekday: 1 = Monday .. 7 = Sunday.
fn weekday_mon1(year: i32, month: u32, day: u32) -> u32 {
    (to_jdn(year, month, day).rem_euclid(7) as u32) + 1
}

fn step_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let total = year * 12 + (month as i32 - 1) + delta;
    let new_year = total.div_euclid(12);
    let new_month = (total.rem_euclid(12) + 1) as u32;
    (new_year, new_month)
}

/// Builds 42 cells (6x7 Monday-first grid) of `(year, month, day, is_other_month)`.
fn build_month_cells(year: i32, month: u32) -> [(i32, u32, u32, bool); 42] {
    let mut cells = [(year, month, 1, false); 42];
    let first_wd = weekday_mon1(year, month, 1) as usize; // 1..=7
    let leading = first_wd - 1; // 0..=6
    let (prev_y, prev_m) = step_month(year, month, -1);
    let prev_days = days_in_month(prev_y, prev_m);
    let cur_days = days_in_month(year, month);
    let (next_y, next_m) = step_month(year, month, 1);

    for (idx, slot) in cells.iter_mut().enumerate() {
        if idx < leading {
            let day = prev_days - (leading - 1 - idx) as u32;
            *slot = (prev_y, prev_m, day, true);
        } else if idx < leading + cur_days as usize {
            let day = (idx - leading) as u32 + 1;
            *slot = (year, month, day, false);
        } else {
            let day = (idx - leading - cur_days as usize) as u32 + 1;
            *slot = (next_y, next_m, day, true);
        }
    }

    cells
}

fn plural_days_ru(n: u64) -> &'static str {
    let rem100 = n % 100;
    let rem10 = n % 10;
    if (11..=14).contains(&rem100) {
        "дней"
    } else if rem10 == 1 {
        "день"
    } else if (2..=4).contains(&rem10) {
        "дня"
    } else {
        "дней"
    }
}

fn format_relative_ru(diff: i64) -> String {
    match diff {
        0 => "сегодня".to_string(),
        1 => "завтра".to_string(),
        -1 => "вчера".to_string(),
        2 => "послезавтра".to_string(),
        -2 => "позавчера".to_string(),
        d if d > 0 => {
            let n = d as u64;
            format!("через {n} {}", plural_days_ru(n))
        }
        d => {
            let n = d.unsigned_abs();
            format!("{n} {} назад", plural_days_ru(n))
        }
    }
}

fn format_full_date_ru(year: i32, month: u32, day: u32) -> String {
    let dow = weekday_mon1(year, month, day).clamp(1, 7) as usize;
    let mon = month.clamp(1, 12) as usize;
    format!("{}, {} {} {}", DAYS_RU[dow], day, MONTHS_GEN_RU[mon], year)
}

struct CalState {
    today: (i32, u32, u32),
    view_year: i32,
    view_month: u32,
    selected: Option<(i32, u32, u32)>,
}

struct CellSlot {
    btn: gtk::Button,
    date: Rc<Cell<(i32, u32, u32, bool)>>,
}

struct CalendarUi {
    month_btn: gtk::Button,
    date_lbl: gtk::Label,
    today_btn: gtk::Button,
    slots: Vec<CellSlot>,
}

fn toggle_class(ctx: &gtk::StyleContext, class_name: &str, enabled: bool) {
    if enabled {
        ctx.add_class(class_name);
    } else {
        ctx.remove_class(class_name);
    }
}

fn render_calendar(st: &CalState, ui: &CalendarUi) {
    let mon_name = MONTHS_NOM_RU[st.view_month.clamp(1, 12) as usize];
    ui.month_btn
        .set_label(&format!("{mon_name} {}", st.view_year));

    let show_today_btn = (st.view_year, st.view_month) != (st.today.0, st.today.1)
        || matches!(st.selected, Some(sel) if sel != st.today);
    ui.today_btn.set_visible(show_today_btn);

    let date_ctx = ui.date_lbl.style_context();
    match st.selected {
        None => {
            toggle_class(&date_ctx, "widget-subtitle", true);
            toggle_class(&date_ctx, "cal-date-selected", false);
            ui.date_lbl.set_text(&format_full_date_ru(
                st.today.0, st.today.1, st.today.2,
            ));
        }
        Some((sy, sm, sd)) => {
            toggle_class(&date_ctx, "widget-subtitle", false);
            toggle_class(&date_ctx, "cal-date-selected", true);
            let diff = to_jdn(sy, sm, sd) - to_jdn(st.today.0, st.today.1, st.today.2);
            let rel = format_relative_ru(diff);
            ui.date_lbl
                .set_text(&format!("{} · {rel}", format_full_date_ru(sy, sm, sd)));
        }
    }

    let cells = build_month_cells(st.view_year, st.view_month);
    for (idx, slot) in ui.slots.iter().enumerate() {
        let (y, m, d, is_other) = cells[idx];
        slot.date.set((y, m, d, is_other));
        slot.btn.set_label(&d.to_string());

        let ctx = slot.btn.style_context();
        let is_weekend = (idx % 7) >= 5;
        let is_today = !is_other && (y, m, d) == st.today;
        let is_selected = !is_other && st.selected == Some((y, m, d));

        toggle_class(&ctx, "cal-other", is_other);
        toggle_class(&ctx, "cal-weekend", !is_other && is_weekend);
        toggle_class(&ctx, "cal-today", is_today);
        toggle_class(&ctx, "cal-selected", is_selected);
    }
}

pub fn build() -> gtk::Window {
    let popup = WidgetPopup::new("Календарь", "󰃭", Align::Center, 320);

    let today = glib::DateTime::now_local()
        .map(|now| (now.year(), now.month() as u32, now.day_of_month() as u32))
        .unwrap_or((2026, 1, 1));

    let state = Rc::new(RefCell::new(CalState {
        today,
        view_year: today.0,
        view_month: today.1,
        selected: None,
    }));

    let today_btn = gtk::Button::with_label("Сегодня");
    today_btn.style_context().add_class("cal-today-pill");
    today_btn.set_can_focus(false);
    today_btn.set_relief(gtk::ReliefStyle::None);
    today_btn.set_no_show_all(true);
    today_btn.set_visible(false);
    popup.header_actions.pack_start(&today_btn, false, false, 0);

    let time_card = gtk::Box::new(gtk::Orientation::Vertical, 2);
    time_card.style_context().add_class("card");

    let time_lbl = gtk::Label::new(None);
    time_lbl.style_context().add_class("cal-clock");
    time_lbl.set_xalign(0.5);

    let date_lbl = gtk::Label::new(None);
    date_lbl.set_xalign(0.5);

    time_card.pack_start(&time_lbl, false, false, 0);
    time_card.pack_start(&date_lbl, false, false, 0);
    popup.root_box.pack_start(&time_card, false, false, 0);

    let cal_event_box = gtk::EventBox::new();
    cal_event_box.set_visible_window(false);
    cal_event_box.add_events(gdk::EventMask::SCROLL_MASK);

    let cal_card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    cal_card.style_context().add_class("card");
    cal_event_box.add(&cal_card);
    popup.root_box.pack_start(&cal_event_box, false, false, 0);

    let nav_row = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let prev_btn = gtk::Button::with_label("‹");
    prev_btn.style_context().add_class("cal-nav-btn");
    prev_btn.set_can_focus(false);
    prev_btn.set_relief(gtk::ReliefStyle::None);

    let month_btn = gtk::Button::with_label("");
    month_btn.style_context().add_class("cal-month-btn");
    month_btn.set_can_focus(false);
    month_btn.set_relief(gtk::ReliefStyle::None);

    let next_btn = gtk::Button::with_label("›");
    next_btn.style_context().add_class("cal-nav-btn");
    next_btn.set_can_focus(false);
    next_btn.set_relief(gtk::ReliefStyle::None);

    nav_row.pack_start(&prev_btn, false, false, 0);
    nav_row.pack_start(&month_btn, true, true, 0);
    nav_row.pack_start(&next_btn, false, false, 0);
    cal_card.pack_start(&nav_row, false, false, 0);

    let hdr_grid = gtk::Grid::new();
    hdr_grid.set_column_homogeneous(true);
    hdr_grid.set_column_spacing(4);
    for (col, &name) in WEEKDAYS_SHORT_RU.iter().enumerate() {
        let lbl = gtk::Label::new(Some(name));
        lbl.set_xalign(0.5);
        let ctx = lbl.style_context();
        ctx.add_class("cal-weekday");
        if col >= 5 {
            ctx.add_class("cal-weekend-hdr");
        }
        hdr_grid.attach(&lbl, col as i32, 0, 1, 1);
    }
    cal_card.pack_start(&hdr_grid, false, false, 0);

    let day_grid = gtk::Grid::new();
    day_grid.set_column_homogeneous(true);
    day_grid.set_row_homogeneous(true);
    day_grid.set_column_spacing(4);
    day_grid.set_row_spacing(4);

    let mut slots = Vec::with_capacity(42);
    for idx in 0..42 {
        let btn = gtk::Button::with_label("");
        btn.style_context().add_class("cal-day");
        btn.set_can_focus(false);
        btn.set_relief(gtk::ReliefStyle::None);
        let row = (idx / 7) as i32;
        let col = (idx % 7) as i32;
        day_grid.attach(&btn, col, row, 1, 1);
        slots.push(CellSlot {
            btn,
            date: Rc::new(Cell::new((today.0, today.1, 1, false))),
        });
    }
    cal_card.pack_start(&day_grid, false, false, 0);

    let ui = Rc::new(CalendarUi {
        month_btn: month_btn.clone(),
        date_lbl,
        today_btn: today_btn.clone(),
        slots,
    });

    render_calendar(&state.borrow(), &ui);

    // Connect day cell clicks
    for slot in &ui.slots {
        let st_ref = state.clone();
        let ui_ref = ui.clone();
        let cell_date = slot.date.clone();
        slot.btn.connect_clicked(move |_| {
            let (y, m, d, is_other) = cell_date.get();
            {
                let mut st = st_ref.borrow_mut();
                if is_other {
                    st.view_year = y;
                    st.view_month = m;
                    st.selected = Some((y, m, d));
                } else if st.selected == Some((y, m, d)) {
                    st.selected = None;
                } else {
                    st.selected = Some((y, m, d));
                }
            }
            render_calendar(&st_ref.borrow(), &ui_ref);
        });
    }

    // Navigation buttons
    {
        let st_ref = state.clone();
        let ui_ref = ui.clone();
        prev_btn.connect_clicked(move |_| {
            {
                let mut st = st_ref.borrow_mut();
                let (ny, nm) = step_month(st.view_year, st.view_month, -1);
                st.view_year = ny;
                st.view_month = nm;
            }
            render_calendar(&st_ref.borrow(), &ui_ref);
        });
    }
    {
        let st_ref = state.clone();
        let ui_ref = ui.clone();
        next_btn.connect_clicked(move |_| {
            {
                let mut st = st_ref.borrow_mut();
                let (ny, nm) = step_month(st.view_year, st.view_month, 1);
                st.view_year = ny;
                st.view_month = nm;
            }
            render_calendar(&st_ref.borrow(), &ui_ref);
        });
    }

    // Return to current month / today
    {
        let st_ref = state.clone();
        let ui_ref = ui.clone();
        month_btn.connect_clicked(move |_| {
            {
                let mut st = st_ref.borrow_mut();
                let different_month = (st.view_year, st.view_month) != (st.today.0, st.today.1);
                st.view_year = st.today.0;
                st.view_month = st.today.1;
                if different_month {
                    st.selected = None;
                }
            }
            render_calendar(&st_ref.borrow(), &ui_ref);
        });
    }
    {
        let st_ref = state.clone();
        let ui_ref = ui.clone();
        today_btn.connect_clicked(move |_| {
            {
                let mut st = st_ref.borrow_mut();
                st.view_year = st.today.0;
                st.view_month = st.today.1;
                st.selected = None;
            }
            render_calendar(&st_ref.borrow(), &ui_ref);
        });
    }

    // Scroll-wheel month navigation
    {
        let st_ref = state.clone();
        let ui_ref = ui.clone();
        cal_event_box.connect_scroll_event(move |_, ev| {
            let delta = match ev.direction() {
                gdk::ScrollDirection::Up => Some(-1),
                gdk::ScrollDirection::Down => Some(1),
                _ => None,
            };
            if let Some(d) = delta {
                {
                    let mut st = st_ref.borrow_mut();
                    let (ny, nm) = step_month(st.view_year, st.view_month, d);
                    st.view_year = ny;
                    st.view_month = nm;
                }
                render_calendar(&st_ref.borrow(), &ui_ref);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }

    // 1s live clock + midnight rollover guard
    let update_clock = {
        let time_weak = time_lbl.downgrade();
        let st_ref = state;
        let ui_ref = ui;
        move || -> ControlFlow {
            let Some(lbl) = time_weak.upgrade() else {
                return ControlFlow::Break;
            };
            if let Ok(now) = glib::DateTime::now_local() {
                lbl.set_text(&format!(
                    "{:02}:{:02}:{:02}",
                    now.hour(),
                    now.minute(),
                    now.second()
                ));
                let cur_today = (now.year(), now.month() as u32, now.day_of_month() as u32);
                let changed = {
                    let mut st = st_ref.borrow_mut();
                    if cur_today != st.today {
                        st.today = cur_today;
                        true
                    } else {
                        false
                    }
                };
                if changed {
                    render_calendar(&st_ref.borrow(), &ui_ref);
                }
            }
            ControlFlow::Continue
        }
    };

    update_clock();
    let timer_id = glib::timeout_add_local(Duration::from_secs(1), update_clock);
    let timer_slot = Rc::new(RefCell::new(Some(timer_id)));
    popup.window.connect_destroy(move |_| {
        if let Some(id) = timer_slot.borrow_mut().take() {
            id.remove();
        }
    });

    popup.window
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leap_years_and_february_days() {
        assert!(is_leap(2000));
        assert!(!is_leap(1900));
        assert!(is_leap(2024));
        assert!(!is_leap(2026));

        assert_eq!(days_in_month(2000, 2), 29);
        assert_eq!(days_in_month(1900, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 11), 30);
    }

    #[test]
    fn weekday_alignment() {
        assert_eq!(weekday_mon1(2000, 1, 1), 6); // Saturday
        assert_eq!(weekday_mon1(2026, 11, 1), 7); // Sunday
        assert_eq!(weekday_mon1(2026, 11, 19), 4); // Thursday
    }

    #[test]
    fn month_stepping_wraps_year() {
        assert_eq!(step_month(2026, 1, -1), (2025, 12));
        assert_eq!(step_month(2026, 12, 1), (2027, 1));
        assert_eq!(step_month(2026, 11, -1), (2026, 10));
    }

    #[test]
    fn grid_layout_nov_2026_and_cross_month_isolation() {
        let nov = build_month_cells(2026, 11);
        assert_eq!(nov.len(), 42);
        // Nov 1 2026 is Sunday -> 6 leading Oct days (26..=31)
        assert_eq!(nov[0], (2026, 10, 26, true));
        assert_eq!(nov[5], (2026, 10, 31, true));
        assert_eq!(nov[6], (2026, 11, 1, false));
        assert_eq!(nov[24], (2026, 11, 19, false));
        assert_eq!(nov[35], (2026, 11, 30, false));
        assert_eq!(nov[36], (2026, 12, 1, true));
        assert_eq!(nov[41], (2026, 12, 6, true));

        let selected = Some((2026, 11, 19));
        let nov_hits: Vec<_> = nov
            .iter()
            .filter(|&&(y, m, d, is_other)| !is_other && selected == Some((y, m, d)))
            .collect();
        assert_eq!(nov_hits.len(), 1);

        let dec = build_month_cells(2026, 12);
        let dec_hits: Vec<_> = dec
            .iter()
            .filter(|&&(y, m, d, is_other)| !is_other && selected == Some((y, m, d)))
            .collect();
        assert_eq!(dec_hits.len(), 0);
    }

    #[test]
    fn relative_days_formatting() {
        assert_eq!(format_relative_ru(0), "сегодня");
        assert_eq!(format_relative_ru(1), "завтра");
        assert_eq!(format_relative_ru(-1), "вчера");
        assert_eq!(format_relative_ru(2), "послезавтра");
        assert_eq!(format_relative_ru(-2), "позавчера");
        assert_eq!(format_relative_ru(3), "через 3 дня");
        assert_eq!(format_relative_ru(5), "через 5 дней");
        assert_eq!(format_relative_ru(11), "через 11 дней");
        assert_eq!(format_relative_ru(21), "через 21 день");
        assert_eq!(format_relative_ru(-4), "4 дня назад");
        assert_eq!(format_relative_ru(-14), "14 дней назад");
    }
}
