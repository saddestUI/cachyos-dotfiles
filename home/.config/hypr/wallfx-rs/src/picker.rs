use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::ffi::{c_char, c_void, CString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use gdk::keys::constants as keys;
use glib::ControlFlow;
use glow::HasContext;
use gtk::prelude::*;
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use image::imageops::FilterType;

use crate::paths::{cache_key, cache_root, list_wallpapers, thumbs_dir};
use crate::theme::{extract_theme, load_cached_theme, ThemeColors};

const THUMB_MAX: u32 = 800;
const CARD_W: f64 = 640.0;
const CARD_H: f64 = 360.0;
const CARD_SKEW: f64 = -0.28;
const CARD_IMG_W: u32 = 755;
const CARD_IMG_H: u32 = 360;
const SLOT_W: f64 = 370.0;
const DEBOUNCE_MS: u128 = 350;

#[link(name = "EGL")]
extern "C" {
    fn eglGetProcAddress(procname: *const c_char) -> *const c_void;
}

fn load_gl_proc(gles_handle: *mut c_void, name: &str) -> *const c_void {
    let Ok(cs) = CString::new(name) else {
        return std::ptr::null();
    };
    unsafe {
        if !gles_handle.is_null() {
            let p = libc::dlsym(gles_handle, cs.as_ptr());
            if !p.is_null() {
                return p;
            }
        }
        eglGetProcAddress(cs.as_ptr())
    }
}

fn now_epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn pid_file() -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.join("wallfx-picker.pid")
}

fn is_wallfx_pid(pid: i32) -> bool {
    let cmd_path = format!("/proc/{pid}/cmdline");
    if let Ok(bytes) = fs::read(cmd_path) {
        return String::from_utf8_lossy(&bytes).contains("wallfx");
    }
    false
}

pub fn toggle_pid() -> bool {
    let p = pid_file();
    let now_ms = now_epoch_ms();
    if let Ok(raw) = fs::read_to_string(&p) {
        let mut parts = raw.split_whitespace();
        if let Some(old_pid) = parts.next().and_then(|s| s.parse::<i32>().ok()) {
            let old_ts = parts
                .next()
                .and_then(|s| s.parse::<u128>().ok())
                .unwrap_or(0);
            let my_pid = unsafe { libc::getpid() };
            if old_pid > 0 && old_pid != my_pid && is_wallfx_pid(old_pid) {
                let alive = unsafe { libc::kill(old_pid, 0) } == 0;
                if alive {
                    // Ignore accidental double-release (e.g. both Super_L and Shift_L released together).
                    if now_ms.saturating_sub(old_ts) < DEBOUNCE_MS {
                        return true;
                    }
                    unsafe {
                        libc::kill(old_pid, libc::SIGTERM);
                    }
                    let _ = fs::remove_file(&p);
                    return true;
                }
            }
        }
    }
    let _ = fs::write(&p, format!("{} {}", unsafe { libc::getpid() }, now_ms));
    false
}

pub fn cleanup_pid() {
    let p = pid_file();
    if let Ok(raw) = fs::read_to_string(&p) {
        let first = raw.split_whitespace().next().and_then(|s| s.parse::<i32>().ok());
        if first == Some(unsafe { libc::getpid() }) {
            let _ = fs::remove_file(p);
        }
    }
}

fn save_jpeg_atomic(img: &image::RgbImage, dst: &Path) -> Option<PathBuf> {
    let tmp = dst.with_extension(format!("{}.tmp.jpg", unsafe { libc::getpid() }));
    if img.save(&tmp).is_ok() {
        let _ = fs::rename(&tmp, dst);
        Some(dst.to_path_buf())
    } else {
        let _ = fs::remove_file(&tmp);
        None
    }
}

fn ensure_assets(src: &Path) -> (Option<PathBuf>, Option<PathBuf>) {
    let dir = thumbs_dir();
    let _ = fs::create_dir_all(&dir);
    let key = cache_key(src);
    let thumb_dst = dir.join(format!("{key}.jpg"));
    let bg_dst = dir.join(format!("{key}-bg.jpg"));

    let need_thumb = !thumb_dst.exists();
    let need_bg = !bg_dst.exists();
    if !need_thumb && !need_bg {
        return (Some(thumb_dst), Some(bg_dst));
    }

    let Ok(img) = image::open(src) else {
        return (
            (!need_thumb).then_some(thumb_dst),
            (!need_bg).then_some(bg_dst),
        );
    };

    let thumb_res = if need_thumb {
        let thumb = img.resize(THUMB_MAX, THUMB_MAX, FilterType::Triangle).to_rgb8();
        save_jpeg_atomic(&thumb, &thumb_dst)
    } else {
        Some(thumb_dst)
    };

    let bg_res = if need_bg {
        let small = img.resize(240, 135, FilterType::Triangle).blur(8.0).to_rgb8();
        save_jpeg_atomic(&small, &bg_dst)
    } else {
        Some(bg_dst)
    };

    (thumb_res, bg_res)
}

fn query_current_wallpaper() -> Option<PathBuf> {
    let cur_cache = cache_root().join("current");
    if let Ok(raw) = fs::read_to_string(&cur_cache) {
        let p = PathBuf::from(raw.trim());
        if p.exists() {
            return Some(fs::canonicalize(&p).unwrap_or(p));
        }
    }
    let out = Command::new("awww").arg("query").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    for line in s.lines() {
        if let Some(rest) = line.split("currently displaying: image: ").nth(1) {
            let p = PathBuf::from(rest.trim());
            if p.exists() {
                return Some(fs::canonicalize(&p).unwrap_or(p));
            }
        }
    }
    None
}

struct LoadedAsset {
    idx: usize,
    card_w: u32,
    card_h: u32,
    card_rgb: Vec<u8>,
    bg_w: u32,
    bg_h: u32,
    bg_rgb: Vec<u8>,
    colors: ThemeColors,
}

const BG_VERT: &str = r#"#version 300 es
precision highp float;
out vec2 vUv;
void main() {
    const vec2 pos[4] = vec2[](
        vec2(-1.0, -1.0),
        vec2( 1.0, -1.0),
        vec2(-1.0,  1.0),
        vec2( 1.0,  1.0)
    );
    vec2 p = pos[gl_VertexID];
    vUv = vec2(p.x * 0.5 + 0.5, 0.5 - p.y * 0.5);
    gl_Position = vec4(p, 0.0, 1.0);
}
"#;

const BG_FRAG: &str = r#"#version 300 es
precision highp float;
in vec2 vUv;
out vec4 fragColor;

uniform sampler2D bgOld;
uniform sampler2D bgNew;
uniform float u_hasOld;
uniform float u_hasNew;
uniform float u_blend;
uniform float u_open;
uniform vec2 u_view;
uniform vec2 u_oldSize;
uniform vec2 u_newSize;

vec2 coverUv(vec2 uv, vec2 imgSize, vec2 viewSize) {
    float sAr = viewSize.x / max(viewSize.y, 1.0);
    float iAr = imgSize.x / max(imgSize.y, 1.0);
    if (iAr > sAr) {
        float w = sAr / iAr;
        return vec2((1.0 - w) * 0.5 + uv.x * w, uv.y);
    } else {
        float h = iAr / sAr;
        return vec2(uv.x, (1.0 - h) * 0.5 + uv.y * h);
    }
}

void main() {
    vec3 base = vec3(0.055, 0.062, 0.090);
    vec3 cOld = base;
    if (u_hasOld > 0.5) {
        cOld = texture(bgOld, coverUv(vUv, u_oldSize, u_view)).rgb;
    }
    vec3 cNew = base;
    if (u_hasNew > 0.5) {
        cNew = texture(bgNew, coverUv(vUv, u_newSize, u_view)).rgb;
    }
    vec3 bgCol = mix(cOld, cNew, clamp(u_blend, 0.0, 1.0));
    float vignette = smoothstep(1.25, 0.25, length(vUv - 0.5));
    vec3 col = mix(base, bgCol * (0.36 + 0.14 * vignette), 0.72);
    float a = 0.94 * u_open;
    fragColor = vec4(col * a, a);
}
"#;

const CARD_VERT: &str = r#"#version 300 es
precision highp float;

uniform vec2 u_view;
uniform vec2 u_center;
uniform float u_scale;
uniform vec2 u_cardSize;
uniform float u_skew;

out vec2 vP;

void main() {
    const vec2 corners[4] = vec2[](
        vec2(-1.0, -1.0),
        vec2( 1.0, -1.0),
        vec2(-1.0,  1.0),
        vec2( 1.0,  1.0)
    );
    vec2 c = corners[gl_VertexID];
    float hw = u_cardSize.x * 0.5 + abs(u_skew) * u_cardSize.y * 0.5 + 36.0;
    float hh = u_cardSize.y * 0.5 + 36.0;
    vec2 local = vec2(c.x * hw, c.y * hh);
    vP = local;

    vec2 scr = u_center + local * u_scale;
    float ndcX = (scr.x / max(u_view.x, 1.0)) * 2.0 - 1.0;
    float ndcY = 1.0 - (scr.y / max(u_view.y, 1.0)) * 2.0;
    gl_Position = vec4(ndcX, ndcY, 0.0, 1.0);
}
"#;

const CARD_FRAG: &str = r#"#version 300 es
precision highp float;

in vec2 vP;
out vec4 fragColor;

uniform sampler2D cardTex;
uniform float u_hasTex;
uniform float u_texFade;
uniform vec2 u_cardSize;
uniform vec2 u_imgSize;
uniform float u_skew;
uniform float u_invSlant;
uniform float u_scale;
uniform float u_alpha;
uniform float u_isActive;
uniform vec3 u_primary;
uniform vec3 u_secondary;

float skewBoxSdf(vec2 p, vec2 halfSz) {
    float xr = p.x - u_skew * p.y;
    vec2 q = vec2((abs(xr) - halfSz.x) * u_invSlant, abs(p.y) - halfSz.y);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0);
}

void main() {
    vec2 halfSz = u_cardSize * 0.5;
    float d = skewBoxSdf(vP, halfSz);
    float aa = 1.1 / max(u_scale, 0.15);

    // Drop shadow
    float sd = skewBoxSdf(vP - vec2(6.0, 12.0), halfSz);
    float sh_a = 0.46 * (1.0 - smoothstep(-2.0, 22.0, sd)) * u_alpha;
    vec4 col = vec4(0.0, 0.0, 0.0, sh_a);

    float xr = vP.x - u_skew * vP.y;
    float gt = clamp((xr / u_cardSize.x + 0.5) * 0.6 + (vP.y / u_cardSize.y + 0.5) * 0.4, 0.0, 1.0);
    vec3 bcol = mix(u_primary, u_secondary, gt);

    // Subtle colored outer rim glow on active card
    if (u_isActive > 0.5 && d > -1.0) {
        float glow_a = 0.22 * (1.0 - smoothstep(0.0, 10.0, abs(d))) * u_alpha;
        vec4 glow_c = vec4(bcol * glow_a, glow_a);
        col = glow_c + col * (1.0 - glow_c.a);
    }

    // Card interior (straight un-skewed texture)
    float fill_mask = 1.0 - smoothstep(-aa * 0.65, aa * 0.65, d);
    if (fill_mask > 0.001) {
        vec3 base = vec3(0.08, 0.09, 0.13);
        vec3 inner = base;
        if (u_hasTex > 0.5) {
            vec2 uv = clamp((vP + u_imgSize * 0.5) / u_imgSize, 0.0, 1.0);
            vec3 texCol = texture(cardTex, uv).rgb;
            inner = mix(base, texCol, clamp(u_texFade, 0.0, 1.0));
        }
        float body_a = fill_mask * u_alpha;
        vec4 body = vec4(inner * body_a, body_a);
        col = body + col * (1.0 - body.a);
    }

    // Per-wallpaper colored border
    float bw = (u_isActive > 0.5) ? 2.8 : 1.6;
    float border_mask = 1.0 - smoothstep(bw * 0.5 - aa * 0.7, bw * 0.5 + aa * 0.7, abs(d));
    if (border_mask > 0.001) {
        vec3 final_bcol = (u_isActive > 0.5) ? bcol : mix(bcol, vec3(1.0), 0.12);
        float bop = (u_isActive > 0.5) ? 0.98 : 0.50;
        float brd_a = border_mask * bop * u_alpha;
        vec4 brd = vec4(final_bcol * brd_a, brd_a);
        col = brd + col * (1.0 - brd.a);
    }

    if (col.a <= 0.001) {
        discard;
    }
    fragColor = col;
}
"#;

fn compile_prog(gl: &glow::Context, vert_src: &str, frag_src: &str) -> glow::Program {
    unsafe {
        let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
        gl.shader_source(vs, vert_src);
        gl.compile_shader(vs);
        if !gl.get_shader_compile_status(vs) {
            panic!("VS error: {}", gl.get_shader_info_log(vs));
        }

        let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
        gl.shader_source(fs, frag_src);
        gl.compile_shader(fs);
        if !gl.get_shader_compile_status(fs) {
            panic!("FS error: {}", gl.get_shader_info_log(fs));
        }

        let prog = gl.create_program().unwrap();
        gl.attach_shader(prog, vs);
        gl.attach_shader(prog, fs);
        gl.link_program(prog);
        if !gl.get_program_link_status(prog) {
            panic!("Link error: {}", gl.get_program_info_log(prog));
        }
        gl.delete_shader(vs);
        gl.delete_shader(fs);
        prog
    }
}

fn upload_rgb_tex(gl: &glow::Context, w: u32, h: u32, rgb: &[u8]) -> glow::Texture {
    unsafe {
        let tex = gl.create_texture().expect("create tex");
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGB8 as i32,
            w as i32,
            h as i32,
            0,
            glow::RGB,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(rgb)),
        );
        tex
    }
}

struct GlRes {
    gl: glow::Context,
    vao: glow::VertexArray,
    bg_prog: glow::Program,
    card_prog: glow::Program,
    card_tex: HashMap<usize, (glow::Texture, u32, u32)>,
    bg_tex: HashMap<usize, (glow::Texture, u32, u32)>,
}

struct PickerState {
    walls: Vec<PathBuf>,
    target: usize,
    pos: f64,
    open_alpha: f64,
    prev_bg_idx: Option<usize>,
    cur_bg_idx: usize,
    bg_blend: f64,
    requested: HashSet<usize>,
    pending_uploads: Vec<LoadedAsset>,
    card_fade: HashMap<usize, f64>,
    card_colors: HashMap<usize, ThemeColors>,
    theme: ThemeColors,
    scroll_accum: f64,
    last_scroll: Instant,
}

fn apply_selected(state: &Rc<RefCell<PickerState>>, window: &gtk::Window) {
    let path = {
        let st = state.borrow();
        st.walls.get(st.target).cloned()
    };
    window.hide();
    cleanup_pid();
    if gtk::main_level() > 0 {
        gtk::main_quit();
    }
    if let Some(p) = path {
        if let Ok(exe) = std::env::current_exe() {
            let _ = Command::new(exe)
                .args(["set", &p.to_string_lossy()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
}

#[allow(deprecated)]
pub fn run_picker() {
    if toggle_pid() {
        return;
    }

    glib::unix_signal_add_local(libc::SIGTERM, || {
        cleanup_pid();
        if gtk::main_level() > 0 {
            gtk::main_quit();
        }
        ControlFlow::Break
    });

    let walls = list_wallpapers(None);
    let initial_idx = query_current_wallpaper()
        .and_then(|cur| walls.iter().position(|w| w == &cur))
        .unwrap_or(0);

    let theme = load_cached_theme();

    let state = Rc::new(RefCell::new(PickerState {
        walls,
        target: initial_idx,
        pos: initial_idx as f64,
        open_alpha: 0.0,
        prev_bg_idx: None,
        cur_bg_idx: initial_idx,
        bg_blend: 1.0,
        requested: HashSet::new(),
        pending_uploads: Vec::new(),
        card_fade: HashMap::new(),
        card_colors: HashMap::new(),
        theme,
        scroll_accum: 0.0,
        last_scroll: Instant::now(),
    }));

    // Single background decode worker -> zero main-thread disk/image decoding.
    let (req_tx, req_rx) = mpsc::channel::<(usize, PathBuf)>();
    let (res_tx, res_rx) = glib::MainContext::channel::<LoadedAsset>(glib::Priority::DEFAULT);

    std::thread::spawn(move || {
        while let Ok((idx, path)) = req_rx.recv() {
            let (thumb_p, bg_p) = ensure_assets(&path);
            let card_img = thumb_p
                .and_then(|p| image::open(&p).ok())
                .map(|im| im.resize_to_fill(CARD_IMG_W, CARD_IMG_H, FilterType::Triangle).to_rgb8());
            let bg_img = bg_p
                .and_then(|p| image::open(&p).ok())
                .map(|im| im.to_rgb8());

            if let (Some(c), Some(b)) = (card_img, bg_img) {
                let colors = extract_theme(&c);
                let _ = res_tx.send(LoadedAsset {
                    idx,
                    card_w: c.width(),
                    card_h: c.height(),
                    card_rgb: c.into_raw(),
                    bg_w: b.width(),
                    bg_h: b.height(),
                    bg_rgb: b.into_raw(),
                    colors,
                });
            }
        }
    });

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_app_paintable(true);
    if let Some(screen) = gtk::prelude::WidgetExt::screen(&window) {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
    }

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_exclusive_zone(-1);
    window.set_namespace("wallfx-picker");
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Bottom, true);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, true);

    let gl_area = gtk::GLArea::new();
    gl_area.set_has_alpha(true);
    gl_area.set_required_version(3, 0);
    gl_area.set_use_es(true);
    gl_area.add_events(
        gdk::EventMask::BUTTON_PRESS_MASK
            | gdk::EventMask::SCROLL_MASK
            | gdk::EventMask::SMOOTH_SCROLL_MASK,
    );

    if state.borrow().walls.is_empty() {
        let overlay = gtk::Overlay::new();
        overlay.add(&gl_area);
        let empty_lbl = gtk::Label::new(Some("Обои не найдены"));
        empty_lbl.set_halign(gtk::Align::Center);
        empty_lbl.set_valign(gtk::Align::Center);
        overlay.add_overlay(&empty_lbl);
        overlay.set_overlay_pass_through(&empty_lbl, true);
        window.add(&overlay);
    } else {
        window.add(&gl_area);
    }

    let queue_neighbors = {
        let state = state.clone();
        let req_tx = req_tx.clone();
        move |center: usize| {
            let mut st = state.borrow_mut();
            let len = st.walls.len();
            if len == 0 {
                return;
            }
            for offset in [0_isize, -1, 1, -2, 2, -3, 3, -4, 4] {
                let idx = (center as isize + offset).clamp(0, len as isize - 1) as usize;
                if st.requested.insert(idx) {
                    let path = st.walls[idx].clone();
                    let _ = req_tx.send((idx, path));
                }
            }
        }
    };
    queue_neighbors(initial_idx);

    let gl_res: Rc<RefCell<Option<GlRes>>> = Rc::new(RefCell::new(None));

    // On-demand synchronized tick callback (stops ticking completely once settled)
    let tick_active = Rc::new(Cell::new(false));
    let ensure_tick: Rc<dyn Fn()> = {
        let state = state.clone();
        let gl_area = gl_area.clone();
        let active_flag = tick_active;
        Rc::new(move || {
            if active_flag.get() {
                return;
            }
            active_flag.set(true);
            let st_t = state.clone();
            let flag_t = active_flag.clone();
            let last_us = Cell::new(None::<i64>);
            gl_area.add_tick_callback(move |area, clock| {
                let now = clock.frame_time();
                let dt = match last_us.get() {
                    Some(prev) => {
                        ((now - prev) as f64 / 1_000_000.0).clamp(1.0 / 240.0, 1.0 / 20.0)
                    }
                    None => 1.0 / 60.0,
                };
                last_us.set(Some(now));

                let mut st = st_t.borrow_mut();
                let mut dirty = !st.pending_uploads.is_empty();

                if st.open_alpha < 1.0 {
                    st.open_alpha = (st.open_alpha + dt * 8.0).min(1.0);
                    dirty = true;
                }
                if st.bg_blend < 1.0 {
                    st.bg_blend = (st.bg_blend + dt * 4.5).min(1.0);
                    dirty = true;
                }
                for v in st.card_fade.values_mut() {
                    if *v < 1.0 {
                        *v = (*v + dt * 7.5).min(1.0);
                        dirty = true;
                    }
                }
                let diff = st.target as f64 - st.pos;
                if diff.abs() > 0.001 {
                    st.pos += diff * (1.0 - (-16.0 * dt).exp());
                    dirty = true;
                } else if (st.pos - st.target as f64).abs() > 0.0 {
                    st.pos = st.target as f64;
                    dirty = true;
                }

                if dirty {
                    area.queue_draw();
                    ControlFlow::Continue
                } else {
                    flag_t.set(false);
                    ControlFlow::Break
                }
            });
        })
    };

    // Push decoded assets from worker thread into pending GPU uploads.
    {
        let state = state.clone();
        let gl_area_c = gl_area.clone();
        let kick = ensure_tick.clone();
        res_rx.attach(None, move |asset| {
            let mut st = state.borrow_mut();
            st.card_fade.entry(asset.idx).or_insert(0.0);
            st.card_colors.insert(asset.idx, asset.colors);
            st.pending_uploads.push(asset);
            drop(st);
            gl_area_c.queue_draw();
            kick();
            ControlFlow::Continue
        });
    }

    // GL setup
    {
        let gl_res_c = gl_res.clone();
        gl_area.connect_realize(move |area| {
            area.make_current();
            if area.error().is_some() {
                return;
            }
            let gles_handle = unsafe {
                libc::dlopen(
                    c"libGLESv2.so.2".as_ptr(),
                    libc::RTLD_LAZY | libc::RTLD_LOCAL,
                )
            };
            let gl = unsafe {
                glow::Context::from_loader_function(|s| load_gl_proc(gles_handle, s))
            };
            let vao = unsafe { gl.create_vertex_array().expect("vao") };
            let bg_prog = compile_prog(&gl, BG_VERT, BG_FRAG);
            let card_prog = compile_prog(&gl, CARD_VERT, CARD_FRAG);

            *gl_res_c.borrow_mut() = Some(GlRes {
                gl,
                vao,
                bg_prog,
                card_prog,
                card_tex: HashMap::new(),
                bg_tex: HashMap::new(),
            });
        });
    }

    // GL cleanup
    {
        let gl_res_c = gl_res.clone();
        gl_area.connect_unrealize(move |area| {
            area.make_current();
            if let Some(res) = gl_res_c.borrow_mut().take() {
                unsafe {
                    for (_, (t, _, _)) in res.card_tex {
                        res.gl.delete_texture(t);
                    }
                    for (_, (t, _, _)) in res.bg_tex {
                        res.gl.delete_texture(t);
                    }
                    res.gl.delete_program(res.bg_prog);
                    res.gl.delete_program(res.card_prog);
                    res.gl.delete_vertex_array(res.vao);
                }
            }
        });
    }

    // GPU Render (60 FPS, zero Cairo allocations)
    {
        let state = state.clone();
        let gl_res_c = gl_res.clone();
        gl_area.connect_render(move |area, _| {
            let mut guard = gl_res_c.borrow_mut();
            let Some(res) = guard.as_mut() else {
                return glib::Propagation::Proceed;
            };

            let mut st = state.borrow_mut();
            if !st.pending_uploads.is_empty() {
                for a in st.pending_uploads.drain(..) {
                    if let Some((old_t, _, _)) = res.card_tex.remove(&a.idx) {
                        unsafe { res.gl.delete_texture(old_t); }
                    }
                    let c_tex = upload_rgb_tex(&res.gl, a.card_w, a.card_h, &a.card_rgb);
                    res.card_tex.insert(a.idx, (c_tex, a.card_w, a.card_h));

                    if let Some((old_b, _, _)) = res.bg_tex.remove(&a.idx) {
                        unsafe { res.gl.delete_texture(old_b); }
                    }
                    let b_tex = upload_rgb_tex(&res.gl, a.bg_w, a.bg_h, &a.bg_rgb);
                    res.bg_tex.insert(a.idx, (b_tex, a.bg_w, a.bg_h));
                }
            }

            let alloc = area.allocation();
            let scale = area.scale_factor().max(1);
            let sw_log = alloc.width().max(1) as f32;
            let sh_log = alloc.height().max(1) as f32;
            let sw_fb = alloc.width().max(1) * scale;
            let sh_fb = alloc.height().max(1) * scale;

            let gl = &res.gl;
            unsafe {
                gl.viewport(0, 0, sw_fb, sh_fb);
                gl.clear_color(0.0, 0.0, 0.0, 0.0);
                gl.clear(glow::COLOR_BUFFER_BIT);
                gl.enable(glow::BLEND);
                gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                gl.bind_vertex_array(Some(res.vao));

                // 1. Blurred wallpaper backdrop with smooth crossfade
                gl.use_program(Some(res.bg_prog));
                let bu = |n: &str| gl.get_uniform_location(res.bg_prog, n);
                let old_entry = st.prev_bg_idx.and_then(|i| res.bg_tex.get(&i).copied());
                let new_entry = res.bg_tex.get(&st.cur_bg_idx).copied();

                if let Some((t, w, h)) = old_entry {
                    gl.active_texture(glow::TEXTURE0);
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                    gl.uniform_1_f32(bu("u_hasOld").as_ref(), 1.0);
                    gl.uniform_2_f32(bu("u_oldSize").as_ref(), w as f32, h as f32);
                } else {
                    gl.uniform_1_f32(bu("u_hasOld").as_ref(), 0.0);
                    gl.uniform_2_f32(bu("u_oldSize").as_ref(), 240.0, 135.0);
                }
                if let Some((t, w, h)) = new_entry {
                    gl.active_texture(glow::TEXTURE1);
                    gl.bind_texture(glow::TEXTURE_2D, Some(t));
                    gl.uniform_1_f32(bu("u_hasNew").as_ref(), 1.0);
                    gl.uniform_2_f32(bu("u_newSize").as_ref(), w as f32, h as f32);
                } else {
                    gl.uniform_1_f32(bu("u_hasNew").as_ref(), 0.0);
                    gl.uniform_2_f32(bu("u_newSize").as_ref(), 240.0, 135.0);
                }
                gl.uniform_1_i32(bu("bgOld").as_ref(), 0);
                gl.uniform_1_i32(bu("bgNew").as_ref(), 1);
                gl.uniform_1_f32(bu("u_blend").as_ref(), st.bg_blend as f32);
                gl.uniform_1_f32(bu("u_open").as_ref(), st.open_alpha as f32);
                gl.uniform_2_f32(bu("u_view").as_ref(), sw_log, sh_log);
                gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);

                // 2. Coverflow parallelogram cards (drawn back-to-front)
                if !st.walls.is_empty() {
                    gl.use_program(Some(res.card_prog));
                    let cu = |n: &str| gl.get_uniform_location(res.card_prog, n);
                    let inv_slant = 1.0 / (1.0 + CARD_SKEW * CARD_SKEW).sqrt();

                    gl.uniform_2_f32(cu("u_view").as_ref(), sw_log, sh_log);
                    gl.uniform_2_f32(cu("u_cardSize").as_ref(), CARD_W as f32, CARD_H as f32);
                    gl.uniform_2_f32(cu("u_imgSize").as_ref(), CARD_IMG_W as f32, CARD_IMG_H as f32);
                    gl.uniform_1_f32(cu("u_skew").as_ref(), CARD_SKEW as f32);
                    gl.uniform_1_f32(cu("u_invSlant").as_ref(), inv_slant as f32);
                    gl.uniform_1_i32(cu("cardTex").as_ref(), 0);

                    let mut visible: Vec<(usize, f64)> = (0..st.walls.len())
                        .map(|i| (i, (i as f64 - st.pos).abs()))
                        .filter(|&(_, ad)| ad <= 3.6)
                        .collect();
                    visible.sort_by(|a, b| b.1.total_cmp(&a.1));

                    let cy = sh_log * 0.47;
                    for (i, ad) in visible {
                        let d = i as f64 - st.pos;
                        let sc = if ad <= 1.0 {
                            0.56 + 0.44 * (1.0 - ad).powf(1.35)
                        } else {
                            0.56 * 0.86_f64.powf(ad - 1.0)
                        };
                        let alpha = (1.15 - 0.34 * ad).clamp(0.0, 1.0) * st.open_alpha;
                        let cx = (sw_log as f64) * 0.5 + d * SLOT_W * (0.94 - 0.05 * ad.min(2.0));

                        let col = st.card_colors.get(&i).copied().unwrap_or(st.theme);
                        gl.uniform_3_f32(
                            cu("u_primary").as_ref(),
                            f32::from(col.primary[0]) / 255.0,
                            f32::from(col.primary[1]) / 255.0,
                            f32::from(col.primary[2]) / 255.0,
                        );
                        gl.uniform_3_f32(
                            cu("u_secondary").as_ref(),
                            f32::from(col.secondary[0]) / 255.0,
                            f32::from(col.secondary[1]) / 255.0,
                            f32::from(col.secondary[2]) / 255.0,
                        );

                        if let Some((tex, _, _)) = res.card_tex.get(&i).copied() {
                            gl.active_texture(glow::TEXTURE0);
                            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                            let fade = st.card_fade.get(&i).copied().unwrap_or(1.0);
                            gl.uniform_1_f32(cu("u_hasTex").as_ref(), 1.0);
                            gl.uniform_1_f32(cu("u_texFade").as_ref(), fade as f32);
                        } else {
                            gl.uniform_1_f32(cu("u_hasTex").as_ref(), 0.0);
                            gl.uniform_1_f32(cu("u_texFade").as_ref(), 0.0);
                        }

                        gl.uniform_2_f32(cu("u_center").as_ref(), cx as f32, cy);
                        gl.uniform_1_f32(cu("u_scale").as_ref(), sc as f32);
                        gl.uniform_1_f32(cu("u_alpha").as_ref(), alpha as f32);
                        gl.uniform_1_f32(cu("u_isActive").as_ref(), if ad < 0.45 { 1.0 } else { 0.0 });
                        gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
                    }
                }
            }

            glib::Propagation::Stop
        });
    }

    ensure_tick();

    let shift_target = {
        let state = state.clone();
        let gl_area = gl_area.clone();
        let queue_neighbors = queue_neighbors.clone();
        let kick = ensure_tick;
        move |new_t: isize| {
            let (changed, clamped) = {
                let mut st = state.borrow_mut();
                if st.walls.is_empty() {
                    return;
                }
                let c = new_t.clamp(0, st.walls.len() as isize - 1) as usize;
                let changed = c != st.target;
                if changed {
                    st.prev_bg_idx = Some(st.cur_bg_idx);
                    st.cur_bg_idx = c;
                    st.bg_blend = 0.0;
                    st.target = c;
                }
                (changed, c)
            };
            if changed {
                queue_neighbors(clamped);
                gl_area.queue_draw();
                kick();
            }
        }
    };

    // Keyboard controls
    {
        let state = state.clone();
        let window_c = window.clone();
        let shift_c = shift_target.clone();
        window.connect_key_press_event(move |_, ev| {
            let kv = ev.keyval();
            let (cur, len) = {
                let st = state.borrow();
                (st.target as isize, st.walls.len() as isize)
            };
            if kv == keys::Escape || kv == keys::q {
                window_c.hide();
                cleanup_pid();
                if gtk::main_level() > 0 {
                    gtk::main_quit();
                }
                return glib::Propagation::Stop;
            } else if kv == keys::Left || kv == keys::h || kv == keys::a {
                shift_c(cur - 1);
                return glib::Propagation::Stop;
            } else if kv == keys::Right || kv == keys::l || kv == keys::d {
                shift_c(cur + 1);
                return glib::Propagation::Stop;
            } else if kv == keys::Home {
                shift_c(0);
                return glib::Propagation::Stop;
            } else if kv == keys::End {
                shift_c(len - 1);
                return glib::Propagation::Stop;
            } else if kv == keys::Return || kv == keys::KP_Enter || kv == keys::space {
                apply_selected(&state, &window_c);
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }

    // Smooth accumulated scroll wheel & touchpad
    {
        let state = state.clone();
        let shift_c = shift_target.clone();
        gl_area.connect_scroll_event(move |_, ev| {
            match ev.direction() {
                gdk::ScrollDirection::Up | gdk::ScrollDirection::Left => {
                    let cur = state.borrow().target as isize;
                    shift_c(cur - 1);
                }
                gdk::ScrollDirection::Down | gdk::ScrollDirection::Right => {
                    let cur = state.borrow().target as isize;
                    shift_c(cur + 1);
                }
                gdk::ScrollDirection::Smooth => {
                    let (dx, dy) = ev.delta();
                    let delta = dx + dy;
                    let steps = {
                        let mut st = state.borrow_mut();
                        if st.last_scroll.elapsed().as_millis() > 180 {
                            st.scroll_accum = 0.0;
                        }
                        st.last_scroll = Instant::now();
                        st.scroll_accum += delta;
                        let mut s = 0_isize;
                        while st.scroll_accum >= 0.75 {
                            st.scroll_accum -= 0.75;
                            s += 1;
                        }
                        while st.scroll_accum <= -0.75 {
                            st.scroll_accum += 0.75;
                            s -= 1;
                        }
                        s
                    };
                    if steps != 0 {
                        let cur = state.borrow().target as isize;
                        shift_c(cur + steps);
                    }
                }
                _ => {}
            }
            glib::Propagation::Stop
        });
    }

    // Click card or outside card band
    {
        let state = state.clone();
        let window_c = window.clone();
        let shift_c = shift_target.clone();
        gl_area.connect_button_press_event(move |widget, ev| {
            if ev.button() != 1 {
                return glib::Propagation::Proceed;
            }
            let alloc = widget.allocation();
            let sw = alloc.width() as f64;
            let sh = alloc.height() as f64;
            let (mx, my) = ev.position();

            if my < sh * 0.47 - 210.0 || my > sh * 0.47 + 210.0 {
                window_c.hide();
                cleanup_pid();
                if gtk::main_level() > 0 {
                    gtk::main_quit();
                }
                return glib::Propagation::Stop;
            }

            let (cur, pos, len) = {
                let st = state.borrow();
                (st.target, st.pos, st.walls.len())
            };
            if len == 0 {
                return glib::Propagation::Stop;
            }

            let dx = mx - sw * 0.5;
            if dx.abs() <= 270.0 {
                apply_selected(&state, &window_c);
            } else {
                let rel_slot = (dx / 350.0).round() as isize;
                let clicked = (pos.round() as isize + rel_slot).clamp(0, len as isize - 1);
                if clicked == cur as isize {
                    apply_selected(&state, &window_c);
                } else {
                    shift_c(clicked);
                }
            }
            glib::Propagation::Stop
        });
    }

    window.connect_destroy(|_| {
        cleanup_pid();
        if gtk::main_level() > 0 {
            gtk::main_quit();
        }
    });

    window.show_all();
    window.present();
    gtk::main();
    cleanup_pid();
}
