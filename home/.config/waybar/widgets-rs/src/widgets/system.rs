use std::cell::RefCell;
use std::fs;
use std::rc::Rc;
use std::time::Duration;

use glib::ControlFlow;
use gtk::prelude::*;

use crate::common::{make_section_label, Align, WidgetPopup};

#[derive(Copy, Clone, Default)]
struct CpuSample {
    total: u64,
    idle: u64,
}

fn read_cpu_sample() -> CpuSample {
    let Ok(raw) = fs::read_to_string("/proc/stat") else {
        return CpuSample::default();
    };
    let Some(line) = raw.lines().next() else {
        return CpuSample::default();
    };
    let nums: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|s| s.parse::<u64>().ok())
        .collect();
    if nums.len() < 4 {
        return CpuSample::default();
    }
    let idle = nums[3] + nums.get(4).copied().unwrap_or(0);
    let total: u64 = nums.iter().sum();
    CpuSample { total, idle }
}

fn read_cpu_temp_c() -> Option<f64> {
    for z in 0..10 {
        let type_p = format!("/sys/class/thermal/thermal_zone{z}/type");
        if let Ok(t) = fs::read_to_string(&type_p) {
            if t.trim() == "x86_pkg_temp" {
                let temp_p = format!("/sys/class/thermal/thermal_zone{z}/temp");
                if let Ok(val) = fs::read_to_string(temp_p) {
                    if let Ok(mc) = val.trim().parse::<f64>() {
                        return Some(mc / 1000.0);
                    }
                }
            }
        }
    }
    None
}

fn read_mem_gb() -> (f64, f64, f64) {
    let Ok(raw) = fs::read_to_string("/proc/meminfo") else {
        return (0.0, 16.0, 0.0);
    };
    let mut total_kb = 0.0_f64;
    let mut avail_kb = 0.0_f64;
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total_kb = rest.split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            avail_kb = rest.split_whitespace().next().and_then(|s| s.parse().ok()).unwrap_or(0.0);
        }
    }
    let used_gb = (total_kb - avail_kb).max(0.0) / (1024.0 * 1024.0);
    let total_gb = (total_kb / (1024.0 * 1024.0)).max(0.1);
    let frac = (used_gb / total_gb).clamp(0.0, 1.0);
    (used_gb, total_gb, frac)
}

fn read_gpu_stats() -> (u32, u32, f64) {
    let read_u32 = |p: &str, d: u32| -> u32 {
        fs::read_to_string(p)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .unwrap_or(d)
    };
    let act = read_u32("/sys/class/drm/card1/gt_act_freq_mhz", 350);
    let min = read_u32("/sys/class/drm/card1/gt_RPn_freq_mhz", 350);
    let max = read_u32("/sys/class/drm/card1/gt_max_freq_mhz", 950).max(min + 1);
    let frac = ((act.saturating_sub(min)) as f64 / (max - min) as f64).clamp(0.0, 1.0);
    (act, max, frac)
}

fn read_uptime_and_load() -> String {
    let uptime_s = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok())
        .unwrap_or(0.0) as u64;
    let h = uptime_s / 3600;
    let m = (uptime_s % 3600) / 60;
    let load = fs::read_to_string("/proc/loadavg")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(str::to_string))
        .unwrap_or_else(|| "0.00".to_string());
    format!("Аптайм: {h}ч {m:02}м  ·  Load: {load}")
}

fn make_stat_card(title: &str) -> (gtk::Box, gtk::Label, gtk::ProgressBar) {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 5);
    card.style_context().add_class("card");

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name_lbl = gtk::Label::new(Some(title));
    name_lbl.set_xalign(0.0);
    let val_lbl = gtk::Label::new(Some("—"));
    val_lbl.style_context().add_class("accent");
    val_lbl.set_xalign(1.0);
    row.pack_start(&name_lbl, true, true, 0);
    row.pack_end(&val_lbl, false, false, 0);

    let bar = gtk::ProgressBar::new();
    card.pack_start(&row, false, false, 0);
    card.pack_start(&bar, false, false, 0);
    (card, val_lbl, bar)
}

pub fn build() -> gtk::Window {
    let popup = WidgetPopup::new("Монитор системы", "󰍛", Align::Right, 310);

    let (cpu_card, cpu_val, cpu_bar) = make_stat_card("󰍛 Процессор (CPU)");
    let (ram_card, ram_val, ram_bar) = make_stat_card("󰘚 Память (RAM)");
    let (gpu_card, gpu_val, gpu_bar) = make_stat_card("󰢮 Графика (Intel HD 530)");

    popup.root_box.pack_start(&cpu_card, false, false, 0);
    popup.root_box.pack_start(&ram_card, false, false, 0);
    popup.root_box.pack_start(&gpu_card, false, false, 0);

    let meta_lbl = make_section_label("");
    meta_lbl.set_xalign(0.5);
    popup.root_box.pack_start(&meta_lbl, false, false, 0);

    let prev_cpu = Rc::new(RefCell::new(read_cpu_sample()));

    let update = move || {
        let cur = read_cpu_sample();
        let prev = *prev_cpu.borrow();
        *prev_cpu.borrow_mut() = cur;
        let dt = cur.total.saturating_sub(prev.total);
        let di = cur.idle.saturating_sub(prev.idle);
        let cpu_frac = if dt > 0 {
            (dt.saturating_sub(di) as f64 / dt as f64).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let cpu_pct = (cpu_frac * 100.0).round() as u32;
        let temp_str = read_cpu_temp_c()
            .map(|t| format!(" · {t:.0}°C"))
            .unwrap_or_default();
        cpu_val.set_text(&format!("{cpu_pct}%{temp_str}"));
        cpu_bar.set_fraction(cpu_frac);

        let (used_gb, total_gb, mem_frac) = read_mem_gb();
        let mem_pct = (mem_frac * 100.0).round() as u32;
        ram_val.set_text(&format!("{used_gb:.1} / {total_gb:.0} ГБ ({mem_pct}%)"));
        ram_bar.set_fraction(mem_frac);

        let (gpu_act, gpu_max, gpu_frac) = read_gpu_stats();
        let gpu_pct = (gpu_frac * 100.0).round() as u32;
        gpu_val.set_text(&format!("{gpu_pct}% · {gpu_act}/{gpu_max} МГц"));
        gpu_bar.set_fraction(gpu_frac);

        meta_lbl.set_text(&read_uptime_and_load());
    };

    update();
    let timer_id = glib::timeout_add_local(Duration::from_secs(1), move || {
        update();
        ControlFlow::Continue
    });
    let timer_cell = Rc::new(RefCell::new(Some(timer_id)));
    popup.window.connect_destroy(move |_| {
        if let Some(id) = timer_cell.borrow_mut().take() {
            id.remove();
        }
    });

    popup.window
}
