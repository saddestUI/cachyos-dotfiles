use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use zbus::blocking::Connection;

const PREFERRED_BACKLIGHTS: &[&str] = &[
    "intel_backlight",
    "amdgpu_bl0",
    "amdgpu_bl1",
    "nvidia_0",
    "nvidia_wmi_ec_backlight",
];

#[derive(Debug, PartialEq)]
enum Action {
    Up(f64),
    Down(f64),
    Set { pct: f64, instant: bool },
}

fn parse_action(args: &[String]) -> Result<Action, String> {
    let cmd = args
        .get(1)
        .ok_or_else(|| "Usage: smooth-brightness <up [pct]|down [pct]|set <pct>|instant <pct>>".to_string())?;
    match cmd.as_str() {
        "up" => {
            let pct = args
                .get(2)
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|&v| v > 0.0)
                .unwrap_or(5.0);
            Ok(Action::Up(pct))
        }
        "down" => {
            let pct = args
                .get(2)
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|&v| v > 0.0)
                .unwrap_or(5.0);
            Ok(Action::Down(pct))
        }
        "set" | "instant" => {
            let raw = args
                .get(2)
                .ok_or_else(|| format!("Usage: smooth-brightness {cmd} <pct>"))?;
            let pct = raw
                .trim_end_matches('%')
                .parse::<f64>()
                .map_err(|_| format!("Invalid percentage: {raw}"))?;
            Ok(Action::Set {
                pct: pct.clamp(2.0, 100.0),
                instant: cmd == "instant",
            })
        }
        other => Err(format!("Unknown subcommand: {other}")),
    }
}

fn find_backlight() -> Option<(String, PathBuf)> {
    let root = Path::new("/sys/class/backlight");
    for pref in PREFERRED_BACKLIGHTS {
        let dir = root.join(pref);
        if dir.join("brightness").exists() && dir.join("max_brightness").exists() {
            return Some(((*pref).to_string(), dir));
        }
    }
    let entries = fs::read_dir(root).ok()?;
    let mut candidates: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|entry| {
            let p = entry.path();
            if p.join("brightness").exists() && p.join("max_brightness").exists() {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| (name.to_string(), p))
            } else {
                None
            }
        })
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates.into_iter().next()
}

fn read_i64(path: &Path) -> Option<i64> {
    fs::read_to_string(path).ok()?.trim().parse::<i64>().ok()
}

fn state_paths() -> (PathBuf, PathBuf) {
    let dir = env::var_os("XDG_RUNTIME_DIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    (
        dir.join("smooth-brightness.pid"),
        dir.join("smooth-brightness.target"),
    )
}

fn min_brightness_for(max_b: i64) -> i64 {
    ((max_b * 2) / 100).max(4).min(max_b)
}

fn compute_target(action: &Action, base: i64, max_b: i64) -> i64 {
    let min_b = min_brightness_for(max_b);
    let raw = match *action {
        Action::Up(pct) => {
            let step = ((pct / 100.0) * max_b as f64).round().max(1.0) as i64;
            base + step
        }
        Action::Down(pct) => {
            let step = ((pct / 100.0) * max_b as f64).round().max(1.0) as i64;
            base - step
        }
        Action::Set { pct, .. } => ((pct / 100.0) * max_b as f64).round() as i64,
    };
    raw.clamp(min_b, max_b)
}

fn ease_out_cubic(t: f64) -> f64 {
    let c = t.clamp(0.0, 1.0);
    1.0 - (1.0 - c).powi(3)
}

struct BrightnessWriter {
    conn: Option<Connection>,
    dev_name: String,
    sysfs_brightness: PathBuf,
}

impl BrightnessWriter {
    fn new(dev_name: String, sysfs_brightness: PathBuf) -> Self {
        let conn = Connection::system().ok();
        Self {
            conn,
            dev_name,
            sysfs_brightness,
        }
    }

    fn set_raw(&mut self, raw: i64) {
        let val = raw.max(0) as u32;
        if let Some(ref conn) = self.conn {
            if conn
                .call_method(
                    Some("org.freedesktop.login1"),
                    "/org/freedesktop/login1/session/auto",
                    Some("org.freedesktop.login1.Session"),
                    "SetBrightness",
                    &("backlight", self.dev_name.as_str(), val),
                )
                .is_ok()
            {
                return;
            }
        }
        if fs::write(&self.sysfs_brightness, format!("{val}\n")).is_ok() {
            return;
        }
        let _ = Command::new("brightnessctl")
            .args(["-d", &self.dev_name, "-q", "set", &val.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let action = match parse_action(&args) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("{msg}");
            std::process::exit(1);
        }
    };

    let Some((dev_name, dev_path)) = find_backlight() else {
        // Desktop PCs without a built-in backlight should exit cleanly.
        return;
    };

    let brightness_path = dev_path.join("brightness");
    let max_path = dev_path.join("max_brightness");
    let (Some(current), Some(max_b)) = (read_i64(&brightness_path), read_i64(&max_path)) else {
        eprintln!("Failed to read backlight sysfs");
        std::process::exit(1);
    };
    if max_b <= 0 {
        eprintln!("Invalid max_brightness: {max_b}");
        std::process::exit(1);
    }

    let min_b = min_brightness_for(max_b);
    let (pid_path, target_path) = state_paths();
    let self_pid = std::process::id() as i32;

    let mut base = current;
    if let Some(old_pid_val) = read_i64(&pid_path) {
        let old_pid = old_pid_val as i32;
        if old_pid > 1 && old_pid != self_pid && unsafe { libc::kill(old_pid, 0) } == 0 {
            if let Some(prev_target) = read_i64(&target_path) {
                if (min_b..=max_b).contains(&prev_target) {
                    base = prev_target;
                }
            }
            unsafe {
                libc::kill(old_pid, libc::SIGTERM);
            }
            thread::sleep(Duration::from_millis(2));
        }
    }

    let target = compute_target(&action, base, max_b);
    let _ = fs::write(&target_path, format!("{target}\n"));
    let _ = fs::write(&pid_path, format!("{self_pid}\n"));

    let mut writer = BrightnessWriter::new(dev_name, brightness_path);
    let instant = matches!(action, Action::Set { instant: true, .. });
    let diff = (target - current).abs();

    if instant || diff == 0 {
        writer.set_raw(target);
    } else {
        let frames = (diff as usize).clamp(8, 12);
        let frame_sleep = Duration::from_millis(10);
        let mut last_written = current;

        for i in 1..=frames {
            let t = i as f64 / frames as f64;
            let eased = ease_out_cubic(t);
            let val = current + ((target - current) as f64 * eased).round() as i64;
            if val != last_written {
                writer.set_raw(val);
                last_written = val;
            }
            if i < frames {
                thread::sleep(frame_sleep);
            }
        }
        if last_written != target {
            writer.set_raw(target);
        }
    }

    if read_i64(&pid_path) == Some(self_pid as i64) {
        let _ = fs::remove_file(&pid_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(arr: &[&str]) -> Vec<String> {
        arr.iter().map(|v| (*v).to_string()).collect()
    }

    #[test]
    fn parses_subcommands_and_clamps_pct() {
        assert_eq!(parse_action(&s(&["sb", "up"])).unwrap(), Action::Up(5.0));
        assert_eq!(
            parse_action(&s(&["sb", "down", "7.5"])).unwrap(),
            Action::Down(7.5)
        );
        assert_eq!(
            parse_action(&s(&["sb", "set", "150%"])).unwrap(),
            Action::Set {
                pct: 100.0,
                instant: false
            }
        );
        assert_eq!(
            parse_action(&s(&["sb", "instant", "0"])).unwrap(),
            Action::Set {
                pct: 2.0,
                instant: true
            }
        );
        assert!(parse_action(&s(&["sb", "bad"])).is_err());
    }

    #[test]
    fn computes_clamped_targets_and_easing_bounds() {
        let max_b = 1000;
        assert_eq!(compute_target(&Action::Up(10.0), 950, max_b), 1000);
        assert_eq!(compute_target(&Action::Down(10.0), 50, max_b), 20);
        assert_eq!(
            compute_target(
                &Action::Set {
                    pct: 50.0,
                    instant: true
                },
                100,
                max_b
            ),
            500
        );
        assert!((ease_out_cubic(0.0) - 0.0).abs() < 1e-9);
        assert!((ease_out_cubic(1.0) - 1.0).abs() < 1e-9);
        assert!(ease_out_cubic(0.5) > 0.5);
    }
}
