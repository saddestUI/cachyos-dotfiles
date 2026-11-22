use std::env;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Theme {
    pub primary: [f64; 3],
    pub secondary: [f64; 3],
    pub bg: [f64; 3],
    pub input_bg: [f64; 3],
    pub surface: [f64; 3],
    pub fg: [f64; 3],
    pub fg_muted: [f64; 3],
    pub pill_end: [f64; 3],
    pub on_accent: [f64; 3],
}

fn mix(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

fn rel_luminance(c: [f64; 3]) -> f64 {
    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

pub fn to_rgb_u8(c: [f64; 3]) -> [u8; 3] {
    [
        (c[0].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[1].clamp(0.0, 1.0) * 255.0).round() as u8,
        (c[2].clamp(0.0, 1.0) * 255.0).round() as u8,
    ]
}

pub fn to_hex(c: [f64; 3]) -> String {
    let [r, g, b] = to_rgb_u8(c);
    format!("#{r:02x}{g:02x}{b:02x}")
}

impl Theme {
    pub fn from_rgb_triplets(p: [u8; 3], s: [u8; 3], inact: [u8; 3]) -> Self {
        let norm = |rgb: [u8; 3]| [rgb[0] as f64 / 255.0, rgb[1] as f64 / 255.0, rgb[2] as f64 / 255.0];
        let primary = norm(p);
        let secondary = norm(s);
        let inactive = norm(inact);

        let bg = mix([0.045, 0.048, 0.068], primary, 0.09);
        let input_bg = mix([0.085, 0.092, 0.125], primary, 0.11);
        let surface = mix([0.072, 0.078, 0.108], secondary, 0.09);
        let fg = mix([0.94, 0.95, 0.97], secondary, 0.16);
        let fg_muted = mix([0.62, 0.65, 0.72], inactive, 0.28);
        let pill_end = mix(primary, secondary, 0.55);

        let pill_mid = mix(primary, pill_end, 0.5);
        let on_accent = if rel_luminance(pill_mid) > 0.44 {
            [0.05, 0.055, 0.075]
        } else {
            [0.98, 0.985, 0.99]
        };

        Self {
            primary,
            secondary,
            bg,
            input_bg,
            surface,
            fg,
            fg_muted,
            pill_end,
            on_accent,
        }
    }

    pub fn load() -> Self {
        let home = PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".into()));
        let path = home.join(".cache/wallfx/theme.txt");
        if let Ok(raw) = fs::read_to_string(&path) {
            let nums: Vec<u8> = raw
                .split_whitespace()
                .filter_map(|tok| tok.parse::<u8>().ok())
                .collect();
            if nums.len() >= 9 {
                return Self::from_rgb_triplets(
                    [nums[0], nums[1], nums[2]],
                    [nums[3], nums[4], nums[5]],
                    [nums[6], nums[7], nums[8]],
                );
            }
        }
        // Fallback: Autumn Sunset warm orange/gold
        Self::from_rgb_triplets([235, 98, 56], [235, 183, 78], [160, 150, 145])
    }
}
