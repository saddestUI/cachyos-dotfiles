use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::Rc;
use std::time::Duration;

use glib::ControlFlow;
use gtk::pango::EllipsizeMode;
use gtk::prelude::*;

use crate::common::{make_section_label, run_cmd, Align, WidgetPopup};

#[derive(Copy, Clone, Default)]
struct CpuSample {
    total: u64,
    idle: u64,
}

#[derive(Debug, PartialEq)]
struct GpuStats {
    name: String,
    detail: String,
    frac: f64,
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

fn read_hwmon_temp_by_names(targets: &[&str]) -> Option<f64> {
    let entries: Vec<_> = fs::read_dir("/sys/class/hwmon").ok()?.flatten().collect();
    for target in targets {
        for entry in &entries {
            let dir = entry.path();
            if let Ok(name) = fs::read_to_string(dir.join("name")) {
                if name.trim() == *target {
                    if let Ok(val) = fs::read_to_string(dir.join("temp1_input")) {
                        if let Ok(mc) = val.trim().parse::<f64>() {
                            return Some(mc / 1000.0);
                        }
                    }
                }
            }
        }
    }
    None
}

fn read_cpu_temp_c() -> Option<f64> {
    if let Some(t) = read_hwmon_temp_by_names(&["k10temp", "zenpower", "coretemp"]) {
        return Some(t);
    }
    if let Some(t) = read_hwmon_temp_by_names(&["thinkpad", "dell_smm", "acpitz"]) {
        return Some(t);
    }
    for zone_type in ["x86_pkg_temp", "TCPU", "acpitz"] {
        for z in 0..16 {
            let type_p = format!("/sys/class/thermal/thermal_zone{z}/type");
            if let Ok(t) = fs::read_to_string(&type_p) {
                if t.trim() == zone_type {
                    let temp_p = format!("/sys/class/thermal/thermal_zone{z}/temp");
                    if let Ok(val) = fs::read_to_string(temp_p) {
                        if let Ok(mc) = val.trim().parse::<f64>() {
                            return Some(mc / 1000.0);
                        }
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
            total_kb = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            avail_kb = rest
                .split_whitespace()
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
        }
    }
    let used_gb = (total_kb - avail_kb).max(0.0) / (1024.0 * 1024.0);
    let total_gb = (total_kb / (1024.0 * 1024.0)).max(0.1);
    let frac = (used_gb / total_gb).clamp(0.0, 1.0);
    (used_gb, total_gb, frac)
}

fn parse_nvidia_csv(out: &str) -> Option<GpuStats> {
    let line = out.lines().next()?;
    let parts: Vec<&str> = line.split(',').map(str::trim).collect();
    if parts.len() != 5 {
        return None;
    }
    let util = parts[1].parse::<f64>().ok()?;
    let clk = parts[2].parse::<u32>().ok()?;
    let max_clk = parts[3].parse::<u32>().ok()?;
    let temp = parts[4].parse::<f64>().ok()?;

    let short_name = parts[0]
        .strip_prefix("NVIDIA GeForce ")
        .or_else(|| parts[0].strip_prefix("NVIDIA "))
        .unwrap_or(parts[0])
        .to_string();
    let frac = (util / 100.0).clamp(0.0, 1.0);
    let pct = util.round().clamp(0.0, 100.0) as u32;
    Some(GpuStats {
        name: short_name,
        detail: format!("{pct}% · {clk}/{max_clk} МГц · {temp:.0}°C"),
        frac,
    })
}

fn parse_amd_sclk_mhz(sclk_raw: &str) -> Option<u32> {
    for line in sclk_raw.lines() {
        let trimmed = line.trim();
        if trimmed.ends_with('*') {
            for tok in trimmed.split_whitespace() {
                let cleaned = tok.trim_end_matches("Mhz").trim_end_matches("MHz");
                if cleaned.len() != tok.len() {
                    if let Ok(mhz) = cleaned.parse::<u32>() {
                        return Some(mhz);
                    }
                }
            }
        }
    }
    None
}

fn read_u32_file(path: &Path) -> Option<u32> {
    fs::read_to_string(path).ok()?.trim().parse::<u32>().ok()
}

fn read_amd_temp(dev_dir: &Path) -> Option<f64> {
    let entries = fs::read_dir(dev_dir.join("hwmon")).ok()?;
    for entry in entries.flatten() {
        if let Ok(val) = fs::read_to_string(entry.path().join("temp1_input")) {
            if let Ok(mc) = val.trim().parse::<f64>() {
                return Some(mc / 1000.0);
            }
        }
    }
    None
}

fn read_gpu_stats() -> Option<GpuStats> {
    let (ok, out, _) = run_cmd(&[
        "nvidia-smi",
        "--query-gpu=name,utilization.gpu,clocks.gr,clocks.max.gr,temperature.gpu",
        "--format=csv,noheader,nounits",
    ]);
    if ok {
        if let Some(stats) = parse_nvidia_csv(&out) {
            return Some(stats);
        }
    }

    let mut cards: Vec<_> = fs::read_dir("/sys/class/drm")
        .ok()?
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            let s = n.to_string_lossy();
            s.starts_with("card") && !s.contains('-')
        })
        .collect();
    cards.sort_by_key(|e| e.file_name());

    // AMD (amdgpu)
    for card in &cards {
        let dev_dir = card.path().join("device");
        if let Some(busy) = read_u32_file(&dev_dir.join("gpu_busy_percent")) {
            let pct = busy.min(100);
            let frac = (pct as f64 / 100.0).clamp(0.0, 1.0);
            let mut detail = format!("{pct}%");
            if let Ok(sclk) = fs::read_to_string(dev_dir.join("pp_dpm_sclk")) {
                if let Some(mhz) = parse_amd_sclk_mhz(&sclk) {
                    detail.push_str(&format!(" · {mhz} МГц"));
                }
            }
            if let Some(temp) = read_amd_temp(&dev_dir) {
                detail.push_str(&format!(" · {temp:.0}°C"));
            }
            return Some(GpuStats {
                name: "AMD Radeon".to_string(),
                detail,
                frac,
            });
        }
    }

    // Intel (i915 or xe)
    for card in &cards {
        let p = card.path();
        if let Some(act) = read_u32_file(&p.join("gt_act_freq_mhz")) {
            let min = read_u32_file(&p.join("gt_RPn_freq_mhz")).unwrap_or(0);
            let max = read_u32_file(&p.join("gt_max_freq_mhz"))
                .unwrap_or(act.max(1000))
                .max(min + 1);
            let frac = ((act.saturating_sub(min)) as f64 / (max - min) as f64).clamp(0.0, 1.0);
            let pct = (frac * 100.0).round() as u32;
            return Some(GpuStats {
                name: "Intel GPU".to_string(),
                detail: format!("{pct}% · {act}/{max} МГц"),
                frac,
            });
        }
        let xe_freq = p.join("device/tile0/gt0/freq0");
        if let Some(act) = read_u32_file(&xe_freq.join("act_freq")) {
            let min = read_u32_file(&xe_freq.join("min_freq")).unwrap_or(0);
            let max = read_u32_file(&xe_freq.join("max_freq"))
                .unwrap_or(act.max(1000))
                .max(min + 1);
            let frac = ((act.saturating_sub(min)) as f64 / (max - min) as f64).clamp(0.0, 1.0);
            let pct = (frac * 100.0).round() as u32;
            return Some(GpuStats {
                name: "Intel GPU".to_string(),
                detail: format!("{pct}% · {act}/{max} МГц"),
                frac,
            });
        }
    }

    None
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
    name_lbl.set_ellipsize(EllipsizeMode::End);
    name_lbl.set_max_width_chars(22);
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

    let init_gpu = read_gpu_stats();
    let (cpu_card, cpu_val, cpu_bar) = make_stat_card("󰍛 Процессор (CPU)");
    let (ram_card, ram_val, ram_bar) = make_stat_card("󰘚 Память (RAM)");

    popup.root_box.pack_start(&cpu_card, false, false, 0);
    popup.root_box.pack_start(&ram_card, false, false, 0);

    let gpu_widgets = init_gpu.map(|g| {
        let (card, val, bar) = make_stat_card(&format!("󰢮 Графика ({})", g.name));
        popup.root_box.pack_start(&card, false, false, 0);
        (val, bar)
    });

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

        if let Some((ref gpu_val, ref gpu_bar)) = gpu_widgets {
            if let Some(gpu) = read_gpu_stats() {
                gpu_val.set_text(&gpu.detail);
                gpu_bar.set_fraction(gpu.frac);
            }
        }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nvidia_smi_csv() {
        let g = parse_nvidia_csv("NVIDIA GeForce RTX 3050, 10, 210, 2145, 39\n").unwrap();
        assert_eq!(g.name, "RTX 3050");
        assert_eq!(g.detail, "10% · 210/2145 МГц · 39°C");
        assert!((g.frac - 0.1).abs() < 1e-6);
    }

    #[test]
    fn parses_amd_pp_dpm_sclk() {
        let sample = "0: 300Mhz \n1: 1400Mhz *\n2: 2400Mhz \n";
        assert_eq!(parse_amd_sclk_mhz(sample), Some(1400));
        assert_eq!(parse_amd_sclk_mhz("0: 300Mhz \n"), None);
    }
}
