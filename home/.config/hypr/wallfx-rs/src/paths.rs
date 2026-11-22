use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use image::RgbImage;

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn cache_root() -> PathBuf {
    home_dir().join(".cache/wallfx")
}

pub fn spread_dir() -> PathBuf {
    cache_root().join("spread")
}

pub fn outline_dir() -> PathBuf {
    cache_root().join("outline")
}

pub fn thumbs_dir() -> PathBuf {
    home_dir().join(".cache/wallpaper-thumbs")
}

pub fn model_path() -> PathBuf {
    cache_root().join("model.onnx")
}

pub fn wallpaper_dirs() -> Vec<PathBuf> {
    let home = home_dir();
    vec![
        home.join("Изображения/wallpapers"),
        home.join("Изображения"),
        home.join("Pictures/wallpapers"),
        home.join("Pictures"),
    ]
}

pub fn is_image_file(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "webp"
    )
}

pub fn list_wallpapers(custom_dir: Option<&Path>) -> Vec<PathBuf> {
    let dirs: Vec<PathBuf> = match custom_dir {
        Some(d) => vec![d.to_path_buf()],
        None => wallpaper_dirs(),
    };

    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for dir in dirs {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut local = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && is_image_file(&path) {
                let canon = fs::canonicalize(&path).unwrap_or(path);
                if seen.insert(canon.clone()) {
                    local.push(canon);
                }
            }
        }
        local.sort();
        result.extend(local);
    }
    result
}

pub fn cache_key(path: &Path) -> String {
    let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let (size, mtime) = fs::metadata(&canon)
        .map(|m| {
            let mt = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (m.len(), mt)
        })
        .unwrap_or((0, 0));

    // FNV-1a 64-bit hash -> 16 hex chars.
    let mut h: u64 = 0xcbf29ce484222325;
    let payload = format!("{}:{size}:{mtime}", canon.to_string_lossy());
    for b in payload.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

/// Extract (ember, ink) colors from the target wallpaper without external palette tools.
pub fn extract_palette(img: &RgbImage) -> ([f32; 3], [f32; 3]) {
    let mut best_ember = [0.20_f32, 0.80, 1.0];
    let mut best_ember_score = -1.0_f32;
    let mut best_ink = [0.85_f32, 0.94, 0.98];
    let mut best_ink_score = -1.0_f32;

    let step_x = (img.width() / 64).max(1);
    let step_y = (img.height() / 64).max(1);

    for y in (0..img.height()).step_by(step_y as usize) {
        for x in (0..img.width()).step_by(step_x as usize) {
            let p = img.get_pixel(x, y);
            let r = f32::from(p[0]) / 255.0;
            let g = f32::from(p[1]) / 255.0;
            let b = f32::from(p[2]) / 255.0;
            let maxc = r.max(g).max(b);
            let minc = r.min(g).min(b);
            let sat = if maxc > 1e-4 { (maxc - minc) / maxc } else { 0.0 };
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;

            let ember_score = sat * (0.4 + 0.6 * maxc) * (1.0 - (luma - 0.58).abs() * 0.5);
            if ember_score > best_ember_score {
                best_ember_score = ember_score;
                best_ember = [r, g, b];
            }

            let ink_score = luma * (1.0 - sat * 0.55);
            if ink_score > best_ink_score {
                best_ink_score = ink_score;
                best_ink = [
                    (r * 0.7 + 0.3).clamp(0.0, 1.0),
                    (g * 0.7 + 0.3).clamp(0.0, 1.0),
                    (b * 0.7 + 0.3).clamp(0.0, 1.0),
                ];
            }
        }
    }

    (best_ember, best_ink)
}
