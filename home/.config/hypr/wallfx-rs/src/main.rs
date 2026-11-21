mod maps;
mod paths;
mod picker;
mod shader;
mod theme;

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use maps::{choose_transition, ensure_contagion_slot, generate_outline};
use paths::{cache_root, list_wallpapers, model_path};

fn cmd_warm(dir: Option<&Path>) {
    for k in 0..8 {
        ensure_contagion_slot(k);
    }
    if !model_path().exists() {
        return;
    }
    for wall in list_wallpapers(dir) {
        let _ = generate_outline(&wall);
    }
}

fn cmd_install_model(flavor: &str) {
    let url = match flavor {
        "u2netp" => "https://github.com/danielgatis/rembg/releases/download/v0.0.0/u2netp.onnx",
        "isnet" => "https://github.com/danielgatis/rembg/releases/download/v0.0.0/isnet-general-use.onnx",
        "birefnet" => "https://github.com/danielgatis/rembg/releases/download/v0.0.0/BiRefNet-general-bb_swin_v1_tiny-epoch_232.onnx",
        _ => "https://github.com/danielgatis/rembg/releases/download/v0.0.0/silueta.onnx",
    };

    let dst = model_path();
    if let Some(parent) = dst.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = dst.with_extension("onnx.part");
    eprintln!("Downloading {flavor} model from {url} ...");
    let status = Command::new("curl")
        .args(["-fL", "--progress-bar", "-o"])
        .arg(&tmp)
        .arg(url)
        .status();

    match status {
        Ok(s) if s.success() => {
            if fs::rename(&tmp, &dst).is_ok() {
                eprintln!("Model installed to {}", dst.display());
            } else {
                eprintln!("Failed to move model into place");
                std::process::exit(1);
            }
        }
        _ => {
            let _ = fs::remove_file(&tmp);
            eprintln!("Failed to download model");
            std::process::exit(1);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let Some(cmd) = args.get(1).map(String::as_str) else {
        eprintln!("Usage: wallfx <picker|set <image>|warm [dir]|install-model [silueta|u2netp|isnet|birefnet]>");
        std::process::exit(1);
    };

    let _ = fs::create_dir_all(cache_root());

    match cmd {
        "picker" => {
            if gtk::init().is_err() {
                eprintln!("GTK init failed");
                std::process::exit(1);
            }
            picker::run_picker();
        }
        "set" => {
            let Some(img_arg) = args.get(2) else {
                eprintln!("Usage: wallfx set <image>");
                std::process::exit(1);
            };
            let img_path = fs::canonicalize(img_arg).unwrap_or_else(|_| PathBuf::from(img_arg));
            if !img_path.exists() {
                eprintln!("File not found: {}", img_path.display());
                std::process::exit(1);
            }
            if gtk::init().is_err() {
                eprintln!("GTK init failed");
                std::process::exit(1);
            }
            let kind = choose_transition(&img_path);
            shader::run_transition(&img_path, kind);
        }
        "warm" => {
            let dir = args.get(2).map(PathBuf::from);
            cmd_warm(dir.as_deref());
        }
        "install-model" => {
            let flavor = args.get(2).map(String::as_str).unwrap_or("silueta");
            cmd_install_model(flavor);
        }
        "theme" => {
            let Some(img_arg) = args.get(2) else {
                eprintln!("Usage: wallfx theme <image>");
                std::process::exit(1);
            };
            let img_path = fs::canonicalize(img_arg).unwrap_or_else(|_| PathBuf::from(img_arg));
            if let Ok(img) = image::open(&img_path) {
                let small = img
                    .resize(640, 640, image::imageops::FilterType::Triangle)
                    .to_rgb8();
                theme::apply_theme_sync(&img_path, &small);
            }
        }
        other => {
            eprintln!("Unknown command: {other}");
            std::process::exit(1);
        }
    }
}
