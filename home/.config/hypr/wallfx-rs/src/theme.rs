use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use image::RgbImage;

use crate::paths::{cache_root, home_dir};

#[derive(Copy, Clone, Debug)]
pub struct ThemeColors {
    pub primary: [u8; 3],
    pub secondary: [u8; 3],
    pub inactive: [u8; 3],
}

impl Default for ThemeColors {
    fn default() -> Self {
        Self {
            primary: [0x33, 0xcc, 0xff],
            secondary: [0x00, 0xff, 0x99],
            inactive: [0x4b, 0x55, 0x63],
        }
    }
}

fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let s = if max > 1e-4 { d / max } else { 0.0 };
    let h = if d <= 1e-4 {
        0.0
    } else if (max - r).abs() < 1e-5 {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if (max - g).abs() < 1e-5 {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, s, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h = h.rem_euclid(360.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0).rem_euclid(2.0) - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = if h < 60.0 {
        (c, x, 0.0)
    } else if h < 120.0 {
        (x, c, 0.0)
    } else if h < 180.0 {
        (0.0, c, x)
    } else if h < 240.0 {
        (0.0, x, c)
    } else if h < 300.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    [
        ((r1 + m).clamp(0.0, 1.0) * 255.0).round() as u8,
        ((g1 + m).clamp(0.0, 1.0) * 255.0).round() as u8,
        ((b1 + m).clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

fn hue_dist(a: f32, b: f32) -> f32 {
    let d = (a - b).abs();
    d.min(360.0 - d)
}

pub fn extract_theme(img: &RgbImage) -> ThemeColors {
    const BINS: usize = 36;
    let mut weights = [0.0_f64; BINS];
    let mut sum_s = [0.0_f64; BINS];
    let mut sum_v = [0.0_f64; BINS];
    let mut sum_h_sin = [0.0_f64; BINS];
    let mut sum_h_cos = [0.0_f64; BINS];

    let step_x = (img.width() / 96).max(1) as usize;
    let step_y = (img.height() / 96).max(1) as usize;

    for y in (0..img.height() as usize).step_by(step_y) {
        for x in (0..img.width() as usize).step_by(step_x) {
            let p = img.get_pixel(x as u32, y as u32);
            let r = f32::from(p[0]) / 255.0;
            let g = f32::from(p[1]) / 255.0;
            let b = f32::from(p[2]) / 255.0;
            let (h, s, v) = rgb_to_hsv(r, g, b);
            let l = v * (1.0 - s * 0.5);
            if v < 0.14 || l > 0.92 || s < 0.18 {
                continue;
            }
            let w = f64::from(s * s * (0.4 + 0.6 * v));
            let bin = ((h / 10.0) as usize) % BINS;
            weights[bin] += w;
            sum_s[bin] += f64::from(s) * w;
            sum_v[bin] += f64::from(v) * w;
            let rad = f64::from(h).to_radians();
            sum_h_sin[bin] += rad.sin() * w;
            sum_h_cos[bin] += rad.cos() * w;
        }
    }

    let mut smooth = [0.0_f64; BINS];
    for i in 0..BINS {
        let prev = (i + BINS - 1) % BINS;
        let next = (i + 1) % BINS;
        smooth[i] = 0.25 * weights[prev] + 0.5 * weights[i] + 0.25 * weights[next];
    }

    let (best_bin, &best_w) = smooth
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap_or((0, &0.0));

    if best_w <= 1e-6 {
        return ThemeColors {
            primary: [0x7a, 0xa2, 0xf7],
            secondary: [0x7d, 0xcf, 0xff],
            inactive: [0x4b, 0x55, 0x63],
        };
    }

    let bin_hue = |b: usize| -> (f32, f32, f32) {
        let w = weights[b].max(1e-6);
        let h = sum_h_sin[b]
            .atan2(sum_h_cos[b])
            .to_degrees()
            .rem_euclid(360.0) as f32;
        let s = (sum_s[b] / w) as f32;
        let v = (sum_v[b] / w) as f32;
        (h, s, v)
    };

    let (ph_raw, ps_raw, pv_raw) = if weights[best_bin] > 1e-6 {
        bin_hue(best_bin)
    } else {
        ((best_bin as f32) * 10.0 + 5.0, 0.72, 0.90)
    };

    let ps = ps_raw.clamp(0.55, 0.92);
    let pv = pv_raw.clamp(0.80, 0.98);
    let primary = hsv_to_rgb(ph_raw, ps, pv);

    // Look for a distinct secondary hue peak (>= 25 deg away and >= 18% weight).
    let mut second_bin: Option<(usize, f64)> = None;
    for (b, &sw) in smooth.iter().enumerate() {
        if weights[b] <= 1e-6 || sw < best_w * 0.18 {
            continue;
        }
        let (h, _, _) = bin_hue(b);
        if hue_dist(ph_raw, h) >= 25.0 {
            if second_bin.map(|(_, w)| sw > w).unwrap_or(true) {
                second_bin = Some((b, sw));
            }
        }
    }

    let secondary = if let Some((sb, _)) = second_bin {
        let (sh, ss, sv) = bin_hue(sb);
        hsv_to_rgb(sh, ss.clamp(0.50, 0.88), sv.clamp(0.82, 0.98))
    } else {
        hsv_to_rgb((ph_raw + 26.0).rem_euclid(360.0), (ps * 0.88).max(0.45), (pv + 0.08).min(1.0))
    };

    let inactive = hsv_to_rgb(ph_raw, 0.18, 0.36);

    ThemeColors {
        primary,
        secondary,
        inactive,
    }
}

fn hex_rgb(c: [u8; 3]) -> String {
    format!("{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn rel_luminance(c: [u8; 3]) -> f32 {
    let r = f32::from(c[0]) / 255.0;
    let g = f32::from(c[1]) / 255.0;
    let b = f32::from(c[2]) / 255.0;
    0.2126 * r.powf(2.2) + 0.7152 * g.powf(2.2) + 0.0722 * b.powf(2.2)
}

fn harmonize_hue(base: f32, target: f32, max_shift: f32) -> f32 {
    let diff = ((target - base + 540.0).rem_euclid(360.0)) - 180.0;
    (base + (diff * 0.16).clamp(-max_shift, max_shift)).rem_euclid(360.0)
}

fn readable_accent(rgb: [u8; 3]) -> [u8; 3] {
    let (h, s, v) = rgb_to_hsv(
        f32::from(rgb[0]) / 255.0,
        f32::from(rgb[1]) / 255.0,
        f32::from(rgb[2]) / 255.0,
    );
    let initial = hsv_to_rgb(h, s, v.max(0.92));
    if rel_luminance(initial) < 0.22 {
        hsv_to_rgb(h, s.clamp(0.38, 0.62), 0.96)
    } else {
        initial
    }
}

pub fn load_cached_theme() -> ThemeColors {
    let p = cache_root().join("theme.txt");
    if let Ok(s) = fs::read_to_string(p) {
        let nums: Vec<u8> = s
            .split_whitespace()
            .filter_map(|tok| tok.parse::<u8>().ok())
            .collect();
        if nums.len() >= 9 {
            return ThemeColors {
                primary: [nums[0], nums[1], nums[2]],
                secondary: [nums[3], nums[4], nums[5]],
                inactive: [nums[6], nums[7], nums[8]],
            };
        }
    }
    ThemeColors::default()
}

pub fn apply_theme(wallpaper_path: &Path, img: &RgbImage) {
    apply_theme_inner(wallpaper_path, img, false);
}

pub fn apply_theme_sync(wallpaper_path: &Path, img: &RgbImage) {
    apply_theme_inner(wallpaper_path, img, true);
}

fn apply_theme_inner(wallpaper_path: &Path, img: &RgbImage, wait_reload: bool) {
    let t = extract_theme(img);
    let p_hex = hex_rgb(t.primary);
    let s_hex = hex_rgb(t.secondary);
    let i_hex = hex_rgb(t.inactive);

    let _ = fs::create_dir_all(cache_root());
    let _ = fs::write(
        cache_root().join("current"),
        wallpaper_path.to_string_lossy().as_bytes(),
    );
    let _ = fs::write(
        cache_root().join("theme.txt"),
        format!(
            "{} {} {} {} {} {} {} {} {}",
            t.primary[0],
            t.primary[1],
            t.primary[2],
            t.secondary[0],
            t.secondary[1],
            t.secondary[2],
            t.inactive[0],
            t.inactive[1],
            t.inactive[2]
        ),
    );

    let home = home_dir();

    // 1. Hyprland Lua theme file
    let hypr_theme = home.join(".config/hypr/theme.lua");
    let hypr_lua = format!(
        "-- Auto-generated by wallfx from wallpaper\n\
         hl.config({{\n    \
             general = {{\n        \
                 col = {{\n            \
                     active_border   = {{ colors = {{\"rgba({p_hex}ee)\", \"rgba({s_hex}ee)\"}}, angle = 45 }},\n            \
                     inactive_border = \"rgba({i_hex}aa)\",\n        \
                 }},\n    \
             }},\n\
         }})\n"
    );
    let _ = fs::write(&hypr_theme, hypr_lua);

    // Shared derived palette for Waybar, Rofi, Kitty & Fish
    let (ph, ps, _) = rgb_to_hsv(
        f32::from(t.primary[0]) / 255.0,
        f32::from(t.primary[1]) / 255.0,
        f32::from(t.primary[2]) / 255.0,
    );
    let (_sh, _ss, _) = rgb_to_hsv(
        f32::from(t.secondary[0]) / 255.0,
        f32::from(t.secondary[1]) / 255.0,
        f32::from(t.secondary[2]) / 255.0,
    );

    let bg = hsv_to_rgb(ph, (ps * 0.42).clamp(0.16, 0.36), 0.085);
    let bg_bar = hsv_to_rgb(ph, (ps * 0.36).clamp(0.14, 0.30), 0.06);
    let bg_alt = hsv_to_rgb(ph, (ps * 0.34).clamp(0.14, 0.30), 0.15);
    let fg_tint = hsv_to_rgb(ph, (ps * 0.36).clamp(0.18, 0.30), 0.98);
    let fg_bar_muted = hsv_to_rgb(ph, (ps * 0.28).clamp(0.14, 0.24), 0.72);
    let ap = readable_accent(t.primary);
    let as_col = readable_accent(t.secondary);

    let lum = rel_luminance(t.primary);
    let on_primary = if lum > 0.34 { "#0d0f14" } else { "#f4f7ff" };

    let fg_tint_hex = hex_rgb(fg_tint);
    let fg_bar_muted_hex = hex_rgb(fg_bar_muted);
    let ap_hex = hex_rgb(ap);
    let as_hex = hex_rgb(as_col);

    // 2. Waybar + Widgets CSS colors
    let waybar_colors = home.join(".config/waybar/colors.css");
    let css = format!(
        "/* Auto-generated by wallfx */\n\
         @define-color primary #{p_hex};\n\
         @define-color secondary #{s_hex};\n\
         @define-color fg_tint #{fg_tint_hex};\n\
         @define-color fg_accent #{ap_hex};\n\
         @define-color fg_secondary #{as_hex};\n\
         @define-color fg_muted #{fg_bar_muted_hex};\n\
         @define-color on_primary {on_primary};\n\
         @define-color bar_bg rgba({}, {}, {}, 0.84);\n\
         @define-color bar_tooltip_bg rgba({}, {}, {}, 0.94);\n\
         @define-color bar_hover rgba({}, {}, {}, 0.14);\n\
         @define-color primary_soft rgba({}, {}, {}, 0.48);\n\
         @define-color primary_hover rgba({}, {}, {}, 0.75);\n\
         @define-color primary_border rgba({}, {}, {}, 0.22);\n\
         @define-color primary_border_strong rgba({}, {}, {}, 0.42);\n\
         @define-color primary_bg rgba({}, {}, {}, 0.14);\n",
        bg[0],
        bg[1],
        bg[2],
        bg[0],
        bg[1],
        bg[2],
        fg_tint[0],
        fg_tint[1],
        fg_tint[2],
        t.primary[0],
        t.primary[1],
        t.primary[2],
        t.primary[0],
        t.primary[1],
        t.primary[2],
        t.primary[0],
        t.primary[1],
        t.primary[2],
        t.primary[0],
        t.primary[1],
        t.primary[2],
        t.primary[0],
        t.primary[1],
        t.primary[2],
    );
    let _ = fs::write(&waybar_colors, css);

    // 3. Mako notifications config
    let mako_cfg = home.join(".config/mako/config");
    let mako_body = format!(
        "# Asura-style notification cards (synced to wallpaper)\n\n\
         font=Noto Sans 10\n\
         width=360\n\
         height=110\n\
         anchor=top-right\n\
         outer-margin=16,20,0,0\n\
         margin=8\n\
         padding=14\n\n\
         border-size=1\n\
         border-radius=16\n\n\
         background-color=#12151dd9\n\
         text-color=#e6edf3\n\
         border-color=#{p_hex}66\n\n\
         icons=1\n\
         icon-location=left\n\
         max-icon-size=42\n\
         icon-border-radius=21\n\n\
         markup=1\n\
         format=<b><span foreground=\"#{p_hex}\" size=\"small\">%a</span></b>\\n<b>%s</b>\\n<span foreground=\"#b0bac5\" size=\"small\">%b</span>\n\n\
         default-timeout=4500\n\
         ignore-timeout=0\n\
         layer=overlay\n\n\
         [urgency=low]\n\
         border-color=#{p_hex}33\n\
         default-timeout=3000\n\n\
         [urgency=high]\n\
         border-color=#f7768ecc\n\
         default-timeout=0\n"
    );
    let _ = fs::write(&mako_cfg, mako_body);

    // 4. Rofi dynamic colors
    let rofi_dir = home.join(".config/rofi");
    let _ = fs::create_dir_all(&rofi_dir);
    let rofi_colors = rofi_dir.join("colors.rasi");
    let rofi_rasi = format!(
        "/* Auto-generated by wallfx */\n\
         * {{\n    \
             bg:               rgba({}, {}, {}, 0.88);\n    \
             bg-surface:       rgba({}, {}, {}, 0.76);\n    \
             bg-hover:         rgba({}, {}, {}, 0.92);\n    \
             fg:               #{fg_tint_hex};\n    \
             fg-muted:         rgba({}, {}, {}, 0.58);\n    \
             primary:          #{p_hex};\n    \
             primary-bright:   #{ap_hex};\n    \
             secondary:        #{as_hex};\n    \
             on-primary:       {on_primary};\n    \
             primary-border:   rgba({}, {}, {}, 0.45);\n    \
             primary-focus:    rgba({}, {}, {}, 0.75);\n    \
             primary-soft:     rgba({}, {}, {}, 0.22);\n\
         }}\n",
        bg[0],
        bg[1],
        bg[2],
        bg_alt[0],
        bg_alt[1],
        bg_alt[2],
        bg_alt[0],
        bg_alt[1],
        bg_alt[2],
        fg_tint[0],
        fg_tint[1],
        fg_tint[2],
        t.primary[0],
        t.primary[1],
        t.primary[2],
        t.primary[0],
        t.primary[1],
        t.primary[2],
        t.primary[0],
        t.primary[1],
        t.primary[2],
    );
    let _ = fs::write(&rofi_colors, rofi_rasi);

    // 5. Kitty + Fish/Pure dynamic colors
    let fg = hsv_to_rgb(ph, 0.07, 0.95);
    let fg_muted = hsv_to_rgb(ph, 0.18, 0.52);
    let fg_soft = hsv_to_rgb(ph, 0.09, 0.82);
    let fg_bright = hsv_to_rgb(ph, 0.04, 0.98);
    let sel_bg = hsv_to_rgb(ph, (ps * 0.55).clamp(0.28, 0.50), 0.35);
    let sel_fg = fg_bright;

    let h_r = harmonize_hue(354.0, ph, 10.0);
    let h_g = harmonize_hue(142.0, ph, 12.0);
    let h_y = harmonize_hue(40.0, ph, 10.0);
    let h_b = harmonize_hue(214.0, ph, 12.0);
    let h_m = harmonize_hue(288.0, ph, 12.0);
    let h_c = harmonize_hue(184.0, ph, 12.0);

    let c1 = hsv_to_rgb(h_r, 0.58, 0.92);
    let c9 = hsv_to_rgb(h_r, 0.52, 0.98);
    let c2 = hsv_to_rgb(h_g, 0.52, 0.88);
    let c10 = hsv_to_rgb(h_g, 0.46, 0.96);
    let c3 = hsv_to_rgb(h_y, 0.56, 0.94);
    let c11 = hsv_to_rgb(h_y, 0.48, 0.99);
    let c4 = hsv_to_rgb(h_b, 0.56, 0.94);
    let c12 = hsv_to_rgb(h_b, 0.48, 0.99);
    let c5 = hsv_to_rgb(h_m, 0.50, 0.92);
    let c13 = hsv_to_rgb(h_m, 0.44, 0.98);
    let c6 = hsv_to_rgb(h_c, 0.54, 0.90);
    let c14 = hsv_to_rgb(h_c, 0.46, 0.97);

    let bg_hex = hex_rgb(bg);
    let bg_bar_hex = hex_rgb(bg_bar);
    let bg_alt_hex = hex_rgb(bg_alt);
    let fg_hex = hex_rgb(fg);
    let fg_muted_hex = hex_rgb(fg_muted);
    let fg_soft_hex = hex_rgb(fg_soft);
    let fg_bright_hex = hex_rgb(fg_bright);
    let sel_bg_hex = hex_rgb(sel_bg);
    let sel_fg_hex = hex_rgb(sel_fg);
    let c1_hex = hex_rgb(c1);
    let c2_hex = hex_rgb(c2);
    let c3_hex = hex_rgb(c3);
    let c4_hex = hex_rgb(c4);
    let c5_hex = hex_rgb(c5);
    let c6_hex = hex_rgb(c6);
    let c9_hex = hex_rgb(c9);
    let c10_hex = hex_rgb(c10);
    let c11_hex = hex_rgb(c11);
    let c12_hex = hex_rgb(c12);
    let c13_hex = hex_rgb(c13);
    let c14_hex = hex_rgb(c14);

    let kitty_dir = home.join(".config/kitty");
    let _ = fs::create_dir_all(&kitty_dir);
    let kitty_theme = format!(
        "# Auto-generated by wallfx from wallpaper\n\
         background            #{bg_hex}\n\
         foreground            #{fg_hex}\n\
         cursor                #{ap_hex}\n\
         cursor_text_color     #{bg_hex}\n\
         selection_background  #{sel_bg_hex}\n\
         selection_foreground  #{sel_fg_hex}\n\
         url_color             #{as_hex}\n\n\
         active_tab_background   #{p_hex}\n\
         active_tab_foreground   {on_primary}\n\
         inactive_tab_background #{bg_alt_hex}\n\
         inactive_tab_foreground #{fg_soft_hex}\n\
         tab_bar_background      #{bg_bar_hex}\n\n\
         active_border_color   #{ap_hex}\n\
         inactive_border_color #{i_hex}\n\n\
         color0  #{bg_alt_hex}\n\
         color8  #{fg_muted_hex}\n\
         color1  #{c1_hex}\n\
         color9  #{c9_hex}\n\
         color2  #{c2_hex}\n\
         color10 #{c10_hex}\n\
         color3  #{c3_hex}\n\
         color11 #{c11_hex}\n\
         color4  #{c4_hex}\n\
         color12 #{c12_hex}\n\
         color5  #{c5_hex}\n\
         color13 #{c13_hex}\n\
         color6  #{c6_hex}\n\
         color14 #{c14_hex}\n\
         color7  #{fg_soft_hex}\n\
         color15 #{fg_bright_hex}\n\
         color250 #{ap_hex}\n\
         color251 #{as_hex}\n\
         color252 #{fg_tint_hex}\n"
    );
    let _ = fs::write(kitty_dir.join("theme.conf"), kitty_theme);

    // 6. Fastfetch dynamic pagoda logo & system info colors
    let fastfetch_dir = home.join(".config/fastfetch");
    let _ = fs::create_dir_all(&fastfetch_dir);
    let pagoda_path = fastfetch_dir.join("pagoda.txt");
    let fastfetch_cfg = format!(
        "{{\n  \
         \"$schema\": \"https://github.com/fastfetch-cli/fastfetch/raw/dev/doc/json_schema.json\",\n  \
         \"logo\": {{\n    \
           \"type\": \"file\",\n    \
           \"source\": \"{}\",\n    \
           \"padding\": {{\n      \
             \"top\": 1,\n      \
             \"left\": 2,\n      \
             \"right\": 3\n    \
           }},\n    \
           \"color\": {{\n      \
             \"1\": \"38;5;250\",\n      \
             \"2\": \"38;5;251\",\n      \
             \"3\": \"38;5;252\"\n    \
           }}\n  \
         }},\n  \
         \"display\": {{\n    \
           \"separator\": \"  \",\n    \
           \"color\": {{\n      \
             \"keys\": \"38;5;250\",\n      \
             \"title\": \"38;5;251\",\n      \
             \"output\": \"38;5;252\"\n    \
           }}\n  \
         }},\n  \
         \"modules\": [\n    \
           \"title\",\n    \
           \"separator\",\n    \
           \"os\",\n    \
           \"host\",\n    \
           \"kernel\",\n    \
           \"uptime\",\n    \
           \"packages\",\n    \
           \"shell\",\n    \
           \"display\",\n    \
           \"wm\",\n    \
           \"terminal\",\n    \
           \"terminalfont\",\n    \
           \"cpu\",\n    \
           \"gpu\",\n    \
           \"memory\",\n    \
           \"disk\",\n    \
           \"battery\",\n    \
           \"locale\",\n    \
           \"break\",\n    \
           \"colors\"\n  \
         ]\n\
         }}\n",
        pagoda_path.to_string_lossy(),
    );
    let _ = fs::write(fastfetch_dir.join("config.jsonc"), fastfetch_cfg);

    let fish_cmd = format!(
        "set -U pure_color_primary {ap_hex}; \
         set -U pure_color_success {as_hex}; \
         set -U pure_color_info {ap_hex}; \
         set -U fish_color_command {ap_hex}; \
         set -U fish_color_keyword {as_hex}; \
         set -U fish_color_operator {as_hex}; \
         set -U fish_color_end {as_hex}; \
         set -U fish_color_quote {c3_hex}; \
         set -U fish_color_param {fg_hex}; \
         set -U fish_color_autosuggestion {fg_muted_hex}; \
         set -U fish_color_comment {fg_muted_hex}; \
         set -U fish_color_error {c1_hex}"
    );

    // Apply live reloads asynchronously so transition framerate never stutters.
    let handle = std::thread::spawn(move || {
        let _ = Command::new("hyprctl")
            .arg("reload")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("makoctl")
            .arg("reload")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("pkill")
            .args(["-USR1", "-x", "kitty"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("fish")
            .args(["-c", &fish_cmd])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
    if wait_reload {
        let _ = handle.join();
    }
}
