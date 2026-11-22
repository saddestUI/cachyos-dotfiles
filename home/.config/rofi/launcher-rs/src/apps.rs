use gio::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct AppEntry {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub exec_cmd: String,
    pub working_dir: Option<String>,
    pub terminal: bool,
    pub icon: Option<gio::Icon>,
    pub desktop_info: Option<gio::DesktopAppInfo>,
    pub name_lower: String,
    pub subtitle_lower: String,
    pub keywords_lower: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageEntry {
    pub count: u32,
    pub last_ts: u64,
}

pub type UsageHistory = HashMap<String, UsageEntry>;

fn history_path() -> PathBuf {
    let home = PathBuf::from(env::var("HOME").unwrap_or_else(|_| ".".into()));
    home.join(".cache/asura-launcher/history.json")
}

pub fn load_history() -> UsageHistory {
    let path = history_path();
    let Ok(raw) = fs::read_to_string(&path) else {
        return HashMap::new();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn record_launch(id: &str) {
    let path = history_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut hist = load_history();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = hist.entry(id.to_string()).or_default();
    entry.count = entry.count.saturating_add(1);
    entry.last_ts = now;
    if let Ok(json) = serde_json::to_string_pretty(&hist) {
        let _ = fs::write(&path, json);
    }
}

pub fn clean_desktop_exec(raw: &str) -> String {
    // Strip Desktop Entry Exec field codes (%f, %F, %u, %U, %d, %D, %n, %N, %i, %c, %k, %v, %m),
    // handling standalone placeholders and quoted placeholders like "%U", while preserving %% -> %.
    let mut out = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '"'
            && i + 3 < chars.len()
            && chars[i + 1] == '%'
            && matches!(
                chars[i + 2],
                'f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'i' | 'c' | 'k' | 'v' | 'm'
            )
            && chars[i + 3] == '"'
        {
            i += 4;
            continue;
        }
        if chars[i] == '%' && i + 1 < chars.len() {
            let next = chars[i + 1];
            if next == '%' {
                out.push('%');
                i += 2;
                continue;
            }
            if matches!(
                next,
                'f' | 'F' | 'u' | 'U' | 'd' | 'D' | 'n' | 'N' | 'i' | 'c' | 'k' | 'v' | 'm'
            ) {
                i += 2;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn load_apps() -> Vec<AppEntry> {
    let mut apps = Vec::new();
    let mut seen_ids = HashSet::new();

    for info in gio::AppInfo::all() {
        if !info.should_show() {
            continue;
        }
        let dapp = info.clone().downcast::<gio::DesktopAppInfo>().ok();
        let name = info.name().to_string();
        if name.trim().is_empty() {
            continue;
        }
        let id = info
            .id()
            .map(|s| s.to_string())
            .unwrap_or_else(|| name.clone());
        if !seen_ids.insert(id.clone()) {
            continue;
        }

        let generic = dapp
            .as_ref()
            .and_then(|d| d.generic_name())
            .map(|s| s.to_string())
            .unwrap_or_default();
        let desc = info
            .description()
            .map(|s| s.to_string())
            .unwrap_or_default();

        let subtitle = if !generic.is_empty() && !generic.eq_ignore_ascii_case(&name) {
            generic
        } else if !desc.is_empty() && !desc.eq_ignore_ascii_case(&name) {
            desc
        } else {
            String::new()
        };

        let keywords = dapp
            .as_ref()
            .map(|d| {
                d.keywords()
                    .into_iter()
                    .map(|k| k.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();

        let raw_exec = dapp
            .as_ref()
            .and_then(|d| d.string("Exec"))
            .map(|s| s.to_string())
            .or_else(|| {
                info.commandline()
                    .map(|p| p.to_string_lossy().into_owned())
            })
            .unwrap_or_default();

        let exec_cmd = clean_desktop_exec(&raw_exec);
        if exec_cmd.is_empty() {
            continue;
        }

        let working_dir = dapp
            .as_ref()
            .and_then(|d| d.string("Path"))
            .map(|s| s.to_string())
            .filter(|s| !s.trim().is_empty());

        let terminal = dapp
            .as_ref()
            .map(|d| d.boolean("Terminal"))
            .unwrap_or(false);

        let name_lower = name.to_lowercase();
        let subtitle_lower = subtitle.to_lowercase();
        let keywords_lower = keywords.to_lowercase();

        apps.push(AppEntry {
            id,
            name,
            subtitle,
            exec_cmd,
            working_dir,
            terminal,
            icon: info.icon(),
            desktop_info: dapp,
            name_lower,
            subtitle_lower,
            keywords_lower,
        });
    }

    apps
}

fn fuzzy_subseq_score(query: &str, target: &str) -> Option<i64> {
    if query.is_empty() {
        return Some(0);
    }
    let q: Vec<char> = query.chars().collect();
    let t: Vec<char> = target.chars().collect();
    if q.len() > t.len() {
        return None;
    }

    let mut qi = 0;
    let mut score: i64 = 0;
    let mut prev_matched: Option<usize> = None;

    for (ti, &tc) in t.iter().enumerate() {
        if tc == q[qi] {
            let mut bonus = 12;
            let is_boundary =
                ti == 0 || !t[ti - 1].is_alphanumeric();
            if is_boundary {
                bonus += 24;
            }
            if prev_matched == Some(ti.saturating_sub(1)) {
                bonus += 32;
            } else if let Some(prev) = prev_matched {
                bonus -= ((ti - prev).min(20)) as i64;
            }
            if ti == 0 {
                bonus += 40;
            }
            score += bonus;
            prev_matched = Some(ti);
            qi += 1;
            if qi == q.len() {
                // Slight preference for shorter exactish targets
                return Some(score - (t.len() as i64));
            }
        }
    }
    None
}

pub fn score_text_field(term: &str, text: &str, exact_prefix_bonus: i64, sub_bonus: i64, weight: i64) -> Option<i64> {
    if text.is_empty() {
        return None;
    }
    if text == term {
        return Some(exact_prefix_bonus + 500);
    }
    if text.starts_with(term) {
        return Some(exact_prefix_bonus - (text.len() as i64));
    }
    if let Some(pos) = text.find(term) {
        let byte_before = text[..pos].chars().last();
        let word_start = byte_before.map(|c| !c.is_alphanumeric()).unwrap_or(true);
        let boundary = if word_start { 350 } else { 0 };
        return Some(sub_bonus + boundary - (pos as i64) * 3 - (text.len() as i64));
    }
    fuzzy_subseq_score(term, text).map(|s| s * weight)
}

pub fn filter_and_rank(apps: &[AppEntry], query: &str, history: &UsageHistory) -> Vec<usize> {
    let trimmed = query.trim().to_lowercase();
    if trimmed.is_empty() {
        let mut indices: Vec<usize> = (0..apps.len()).collect();
        indices.sort_by(|&a, &b| {
            let ha = history.get(&apps[a].id);
            let hb = history.get(&apps[b].id);
            let ca = ha.map(|h| h.count).unwrap_or(0);
            let cb = hb.map(|h| h.count).unwrap_or(0);
            cb.cmp(&ca)
                .then_with(|| {
                    let ta = ha.map(|h| h.last_ts).unwrap_or(0);
                    let tb = hb.map(|h| h.last_ts).unwrap_or(0);
                    tb.cmp(&ta)
                })
                .then_with(|| apps[a].name_lower.cmp(&apps[b].name_lower))
        });
        return indices;
    }

    let terms: Vec<&str> = trimmed.split_whitespace().collect();
    let mut scored: Vec<(i64, usize)> = Vec::new();

    for (idx, app) in apps.iter().enumerate() {
        let mut total_score = 0_i64;
        let mut matched_all = true;

        for &term in &terms {
            let s_name = score_text_field(term, &app.name_lower, 2000, 900, 3);
            let s_sub = score_text_field(term, &app.subtitle_lower, 600, 320, 1);
            let s_kw = score_text_field(term, &app.keywords_lower, 450, 240, 1);
            let best = [s_name, s_sub, s_kw].into_iter().flatten().max();
            if let Some(s) = best {
                total_score += s;
            } else {
                matched_all = false;
                break;
            }
        }

        if matched_all {
            let freq_boost = history
                .get(&app.id)
                .map(|h| (h.count.min(40) as i64) * 18)
                .unwrap_or(0);
            scored.push((total_score + freq_boost, idx));
        }
    }

    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| apps[a.1].name_lower.cmp(&apps[b.1].name_lower))
    });
    scored.into_iter().map(|(_, idx)| idx).collect()
}

pub fn launch_app(app: &AppEntry) {
    record_launch(&app.id);

    // Build shell snippet preserving working directory and terminal flag when needed.
    let shell_cmd = if app.terminal {
        let escaped = app.exec_cmd.replace('\'', "'\\''");
        format!("kitty -e sh -c '{escaped}'")
    } else {
        app.exec_cmd.clone()
    };

    let full_cmd = if let Some(ref wd) = app.working_dir {
        let wd_esc = wd.replace('\'', "'\\''");
        format!("cd '{wd_esc}' && exec {shell_cmd}")
    } else {
        format!("exec {shell_cmd}")
    };

    // Prefer Hyprland dispatcher so the launched client is parented cleanly in the compositor session.
    let hypr_ok = Command::new("hyprctl")
        .arg("dispatch")
        .arg("exec")
        .arg("--")
        .arg("sh")
        .arg("-c")
        .arg(&full_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    if hypr_ok {
        return;
    }

    if let Some(ref dapp) = app.desktop_info {
        if dapp.launch(&[], gio::AppLaunchContext::NONE).is_ok() {
            return;
        }
    }

    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&full_cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let _ = cmd.spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_app(id: &str, name: &str, subtitle: &str, keywords: &str) -> AppEntry {
        AppEntry {
            id: id.to_string(),
            name: name.to_string(),
            subtitle: subtitle.to_string(),
            exec_cmd: id.to_string(),
            working_dir: None,
            terminal: false,
            icon: None,
            desktop_info: None,
            name_lower: name.to_lowercase(),
            subtitle_lower: subtitle.to_lowercase(),
            keywords_lower: keywords.to_lowercase(),
        }
    }

    #[test]
    fn cleans_exec_placeholders() {
        assert_eq!(clean_desktop_exec("firefox %u"), "firefox");
        assert_eq!(clean_desktop_exec("kitty --single-instance %F"), "kitty --single-instance");
        assert_eq!(clean_desktop_exec("thunar \"%U\""), "thunar");
        assert_eq!(clean_desktop_exec("env FOO=100%% bar %f"), "env FOO=100% bar");
    }

    #[test]
    fn ranks_prefix_before_fuzzy_and_supports_mru() {
        let apps = vec![
            dummy_app("vlc.desktop", "VLC media player", "Video", "movie"),
            dummy_app("kitty.desktop", "Kitty", "Terminal emulator", "shell"),
            dummy_app("kwrite.desktop", "KWrite", "Text editor", "notes"),
        ];
        let mut hist = HashMap::new();
        hist.insert("vlc.desktop".into(), UsageEntry { count: 5, last_ts: 100 });

        // Empty query puts MRU app first
        let ranked_empty = filter_and_rank(&apps, "", &hist);
        assert_eq!(ranked_empty[0], 0);

        // Search "kit" ranks Kitty first, ahead of distant fuzzy match KWrite ([k]wr[it]e)
        let ranked_kit = filter_and_rank(&apps, "kit", &hist);
        assert_eq!(ranked_kit[0], 1);

        // Multi-term Cyrillic/English keyword search
        let ru_apps = vec![
            dummy_app("t.desktop", "Терминал", "Командная строка", "kitty console"),
            dummy_app("f.desktop", "Файлы", "Менеджер файлов", "thunar"),
        ];
        assert_eq!(filter_and_rank(&ru_apps, "терм", &hist), vec![0]);
        assert_eq!(filter_and_rank(&ru_apps, "менедж фа", &hist), vec![1]);
    }
}
