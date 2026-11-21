use std::cell::RefCell;
use std::ffi::{c_char, c_void, CStr, CString};
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::time::Instant;

use glib::ControlFlow;
use glow::HasContext;
use gtk::prelude::*;
use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use image::{Rgb, RgbImage};

use crate::maps::{TransitionKind, SD_RANGE};
use crate::paths::extract_palette;
use crate::theme::apply_theme;

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

const VERT_SRC: &str = r#"#version 300 es
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

const FRAG_SRC: &str = r#"#version 300 es
precision highp float;

in vec2 vUv;
out vec4 fragColor;

uniform float u_t;
uniform float u_opacity;
uniform float u_mode;
uniform float u_sw;
uniform float u_sh;
uniform float u_fw;
uniform float u_fh;
uniform float u_lw;
uniform float u_lh;
uniform float u_glow;
uniform float u_lineW;
uniform float u_sdRange;
uniform float u_seed;
uniform vec4 u_cropOld;
uniform vec4 u_cropNew;
uniform vec4 u_ember;
uniform vec4 u_ink;

uniform sampler2D oldTex;
uniform sampler2D newTex;
uniform sampler2D fieldTex;
uniform sampler2D lineTex;

const float PI = 3.14159265;
const float CELL = 4.0;

float escena(float x) {
    x = clamp(x, 0.0, 1.0);
    return x * x * (3.0 - 2.0 * x);
}

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7)) + u_seed * 13.17) * 43758.5453);
}

float luma(vec3 c) {
    return dot(c, vec3(0.299, 0.587, 0.114));
}

vec2 cam(vec2 uv, float z) {
    return 0.5 + (uv - 0.5) / z;
}

vec3 oldAt(vec2 uv) {
    return texture(oldTex, u_cropOld.xy + uv * u_cropOld.zw).rgb;
}

vec2 newUv(vec2 uv, float z) {
    return u_cropNew.xy + cam(uv, z) * u_cropNew.zw;
}

float s16(vec2 texel) {
    texel = clamp(texel, vec2(0.0), vec2(u_lw, u_lh) - 1.0);
    vec4 f = texture(lineTex, (texel + 0.5) / vec2(u_lw, u_lh));
    return (floor(f.g * 255.0 + 0.5) * 256.0 + floor(f.b * 255.0 + 0.5)) / 65535.0;
}

float surface(vec2 tuv) {
    vec2 p = tuv * vec2(u_lw, u_lh) - 0.5;
    vec2 i = floor(p);
    vec2 f = p - i;
    return mix(
        mix(s16(i), s16(i + vec2(1.0, 0.0)), f.x),
        mix(s16(i + vec2(0.0, 1.0)), s16(i + vec2(1.0, 1.0)), f.x),
        f.y
    );
}

float arrivalAt(vec2 c) {
    c = clamp(c, vec2(0.0), vec2(u_fw, u_fh) - 1.0);
    vec4 f = texture(fieldTex, (c + 0.5) / vec2(u_fw, u_fh));
    return (floor(f.r * 255.0 + 0.5) * 256.0 + floor(f.g * 255.0 + 0.5)) / 65535.0;
}

vec3 contagion(vec2 uv) {
    const float LIFE = 0.05;
    float energy = sin(PI * u_t);
    float T = escena(u_t) * (1.0 + LIFE + 0.01) - 0.01;
    float s = surface(uv);
    vec2 cell = floor(uv * vec2(u_fw, u_fh));
    float a = arrivalAt(cell);
    float flick = hash(cell);
    vec3 hot = mix(u_ember.rgb, vec3(1.0), 0.18);
    vec3 c;
    if (a > T) {
        c = oldAt(uv);
        float ahead = a - T;
        if (flick > 0.985 && ahead < 0.02) {
            c = mix(c, hot, (1.0 - ahead / 0.02) * 0.8 * u_glow * step(0.0, T));
        }
    } else {
        c = texture(newTex, newUv(uv, 1.0 + 0.025 * (1.0 - escena(u_t)))).rgb;
        bool edge = max(
            max(arrivalAt(cell + vec2(1.0, 0.0)), arrivalAt(cell - vec2(1.0, 0.0))),
            max(arrivalAt(cell + vec2(0.0, 1.0)), arrivalAt(cell - vec2(0.0, 1.0)))
        ) > T;
        float life = clamp((T - a) / LIFE, 0.0, 1.0);
        float burn = (1.0 - life) * (1.0 - life) * (edge ? 1.0 : 0.55);
        c = mix(c, hot, burn * (0.55 + 0.45 * flick) * 0.85 * u_glow);
    }
    return c + u_ember.rgb * 0.22 * u_glow * energy * exp(-abs(s - T) / 0.006);
}

vec3 outline(vec2 uv) {
    float zCh = 1.0 + 0.03 * (1.0 - escena(u_t));
    float zBg = 1.0 + 0.015 * (1.0 - escena(u_t));
    vec4 f = texture(fieldTex, newUv(uv, zCh));
    float sdPx = (f.b * 2.0 - 1.0) * u_sdRange * u_sw * zCh / (u_fw * u_cropNew.z);
    float inside = clamp(0.5 - sdPx / max(fwidth(sdPx), 1e-3), 0.0, 1.0);

    float back = escena(u_t / 0.18);
    vec3 st = oldAt(uv);
    st = mix(st, vec3(luma(st)), 0.25 * back) * (1.0 - 0.2 * back);

    vec2 grid = vec2(u_sw, u_sh) / CELL;
    vec2 cuv = (floor(uv * grid) + 0.5) / grid;

    float Ti = escena((u_t - 0.32) / 0.32) * 1.02 - 0.01;
    vec3 cIn = texture(fieldTex, newUv(cuv, zCh)).g <= Ti
        ? texture(newTex, newUv(uv, zCh)).rgb
        : st;

    float To = escena((u_t - 0.45) / 0.50) * 1.02 - 0.01;
    vec3 cOut = surface(newUv(cuv, zBg)) <= To
        ? texture(newTex, newUv(uv, zBg)).rgb
        : st;

    vec3 c = mix(cOut, cIn, inside);

    float drawP = escena((u_t - 0.02) / 0.33);
    float lp = texture(lineTex, newUv(uv, zCh)).r;
    float fade = 1.0 - escena((u_t - 0.45) / 0.25);
    if (lp > 0.0 && lp <= drawP && fade > 0.0) {
        float d = abs(sdPx) - u_lineW * 0.5;
        float core = clamp(0.5 - d / max(fwidth(sdPx), 1e-3), 0.0, 1.0);
        c = mix(c, u_ink.rgb, core * 0.75 * fade);
    }
    return c;
}

void main() {
    vec3 c = u_mode < 0.5 ? contagion(vUv) : outline(vUv);
    fragColor = vec4(clamp(c, 0.0, 1.0), 1.0) * u_opacity;
}
"#;

enum AwwwCurrent {
    Image(String),
    Color([u8; 3]),
}

fn query_awww_current() -> AwwwCurrent {
    if let Ok(out) = Command::new("awww").arg("query").output() {
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            if let Some(rest) = line.split("currently displaying: image: ").nth(1) {
                let p = rest.trim().to_string();
                if Path::new(&p).exists() {
                    return AwwwCurrent::Image(p);
                }
            }
            if let Some(rest) = line.split("currently displaying: color: ").nth(1) {
                let hex = rest.trim().trim_start_matches('#');
                if hex.len() >= 6 {
                    if let (Ok(r), Ok(g), Ok(b)) = (
                        u8::from_str_radix(&hex[0..2], 16),
                        u8::from_str_radix(&hex[2..4], 16),
                        u8::from_str_radix(&hex[4..6], 16),
                    ) {
                        return AwwwCurrent::Color([r, g, b]);
                    }
                }
            }
        }
    }
    AwwwCurrent::Color([16, 18, 24])
}

fn cover_crop(img_w: u32, img_h: u32, scr_w: i32, scr_h: i32) -> [f32; 4] {
    let iw = img_w.max(1) as f32;
    let ih = img_h.max(1) as f32;
    let sw = scr_w.max(1) as f32;
    let sh = scr_h.max(1) as f32;

    let img_ar = iw / ih;
    let scr_ar = sw / sh;
    if img_ar > scr_ar {
        let zw = scr_ar / img_ar;
        let x0 = (1.0 - zw) * 0.5;
        [x0, 0.0, zw, 1.0]
    } else {
        let zh = img_ar / scr_ar;
        let y0 = (1.0 - zh) * 0.5;
        [0.0, y0, 1.0, zh]
    }
}

fn load_rgb_capped(path: &Path, max_dim: u32) -> Option<RgbImage> {
    let img = image::open(path).ok()?;
    let img = if img.width() > max_dim || img.height() > max_dim {
        img.resize(max_dim, max_dim, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    Some(img.to_rgb8())
}

fn upload_rgb_tex(gl: &glow::Context, unit: u32, img: &RgbImage, linear: bool) -> glow::Texture {
    unsafe {
        let tex = gl.create_texture().expect("create texture");
        gl.active_texture(glow::TEXTURE0 + unit);
        gl.bind_texture(glow::TEXTURE_2D, Some(tex));
        gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
        let filter = if linear {
            glow::LINEAR as i32
        } else {
            glow::NEAREST as i32
        };
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, filter);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, filter);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
        gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
        gl.tex_image_2d(
            glow::TEXTURE_2D,
            0,
            glow::RGB8 as i32,
            img.width() as i32,
            img.height() as i32,
            0,
            glow::RGB,
            glow::UNSIGNED_BYTE,
            glow::PixelUnpackData::Slice(Some(img.as_raw())),
        );
        tex
    }
}

fn compile_program(gl: &glow::Context) -> glow::Program {
    unsafe {
        let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
        gl.shader_source(vs, VERT_SRC);
        gl.compile_shader(vs);
        if !gl.get_shader_compile_status(vs) {
            panic!("VS error: {}", gl.get_shader_info_log(vs));
        }

        let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
        gl.shader_source(fs, FRAG_SRC);
        gl.compile_shader(fs);
        if !gl.get_shader_compile_status(fs) {
            panic!("FS error: {}", gl.get_shader_info_log(fs));
        }

        let prog = gl.create_program().unwrap();
        gl.attach_shader(prog, vs);
        gl.attach_shader(prog, fs);
        gl.link_program(prog);
        if !gl.get_program_link_status(prog) {
            panic!("Program link error: {}", gl.get_program_info_log(prog));
        }
        gl.delete_shader(vs);
        gl.delete_shader(fs);
        prog
    }
}

struct GlState {
    gl: glow::Context,
    prog: glow::Program,
    vao: glow::VertexArray,
    old_tex: glow::Texture,
    new_tex: glow::Texture,
    field_tex: glow::Texture,
    line_tex: glow::Texture,
    mode: f32,
    fw: f32,
    fh: f32,
    lw: f32,
    lh: f32,
    crop_old: [f32; 4],
    crop_new: [f32; 4],
    ember: [f32; 3],
    ink: [f32; 3],
    seed: f32,
    start: Instant,
    duration_s: f32,
    awww_switched: bool,
}

fn apply_awww_instant(path: &Path) {
    let p = path.to_string_lossy().to_string();
    std::thread::spawn(move || {
        let _ = Command::new("awww")
            .args(["img", &p, "--transition-type", "none"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    });
}

pub fn run_transition(new_path: &Path, kind: TransitionKind) {
    let new_img = match load_rgb_capped(new_path, 2560) {
        Some(im) => im,
        None => {
            let _ = Command::new("awww")
                .args(["img", &new_path.to_string_lossy()])
                .status();
            return;
        }
    };

    let (ember, ink) = extract_palette(&new_img);
    apply_theme(new_path, &new_img);

    let old_img = match query_awww_current() {
        AwwwCurrent::Image(p) => {
            load_rgb_capped(Path::new(&p), 2560).unwrap_or_else(|| RgbImage::from_pixel(16, 16, Rgb([16, 18, 24])))
        }
        AwwwCurrent::Color(rgb) => RgbImage::from_pixel(16, 16, Rgb(rgb)),
    };

    let (mode, field_path, line_path, duration_s) = match kind {
        TransitionKind::Contagion {
            field_path,
            line_path,
        } => (0.0_f32, field_path, line_path, 1.80_f32),
        TransitionKind::Outline {
            field_path,
            line_path,
        } => (1.0_f32, field_path, line_path, 2.00_f32),
    };

    let field_img = image::open(&field_path)
        .map(|i| i.to_rgb8())
        .unwrap_or_else(|_| RgbImage::new(480, 270));
    let line_img = image::open(&line_path)
        .map(|i| i.to_rgb8())
        .unwrap_or_else(|_| RgbImage::new(480, 270));

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_app_paintable(true);
    if let Some(screen) = gtk::prelude::WidgetExt::screen(&window) {
        if let Some(visual) = screen.rgba_visual() {
            window.set_visual(Some(&visual));
        }
    }

    window.init_layer_shell();
    window.set_layer(Layer::Bottom);
    window.set_exclusive_zone(-1);
    window.set_namespace("wallfx");
    window.set_keyboard_mode(KeyboardMode::None);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Bottom, true);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, true);

    let gl_area = gtk::GLArea::new();
    gl_area.set_has_alpha(true);
    gl_area.set_required_version(3, 0);
    gl_area.set_use_es(true);
    window.add(&gl_area);

    let state_cell: Rc<RefCell<Option<GlState>>> = Rc::new(RefCell::new(None));
    let new_path_buf = new_path.to_path_buf();

    let state_init = state_cell.clone();
    let old_img_c = old_img;
    let new_img_c = new_img;
    let field_img_c = field_img;
    let line_img_c = line_img;

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

        let prog = compile_program(&gl);
        let vao = unsafe { gl.create_vertex_array().unwrap() };

        let old_tex = upload_rgb_tex(&gl, 0, &old_img_c, true);
        let new_tex = upload_rgb_tex(&gl, 1, &new_img_c, true);
        let field_tex = upload_rgb_tex(&gl, 2, &field_img_c, false);
        let line_tex = upload_rgb_tex(&gl, 3, &line_img_c, true);

        let alloc = area.allocation();
        let scale = area.scale_factor().max(1);
        let sw = alloc.width().max(1920) * scale;
        let sh = alloc.height().max(1080) * scale;

        let crop_old = cover_crop(old_img_c.width(), old_img_c.height(), sw, sh);
        let crop_new = cover_crop(new_img_c.width(), new_img_c.height(), sw, sh);

        let seed = ((std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(1)
            % 10000) as f32)
            / 100.0;

        *state_init.borrow_mut() = Some(GlState {
            gl,
            prog,
            vao,
            old_tex,
            new_tex,
            field_tex,
            line_tex,
            mode,
            fw: field_img_c.width() as f32,
            fh: field_img_c.height() as f32,
            lw: line_img_c.width() as f32,
            lh: line_img_c.height() as f32,
            crop_old,
            crop_new,
            ember,
            ink,
            seed,
            start: Instant::now(),
            duration_s,
            awww_switched: false,
        });
    });

    let state_render = state_cell.clone();
    let new_path_switch = new_path_buf.clone();
    gl_area.connect_render(move |area, _ctx| {
        let mut guard = state_render.borrow_mut();
        let Some(st) = guard.as_mut() else {
            return glib::Propagation::Proceed;
        };

        let elapsed = st.start.elapsed().as_secs_f32();
        let fade_s = 0.13_f32;
        let t = (elapsed / st.duration_s).clamp(0.0, 1.0);

        let opacity = if elapsed < fade_s {
            (elapsed / fade_s).clamp(0.0, 1.0)
        } else if elapsed > st.duration_s {
            (1.0 - (elapsed - st.duration_s) / fade_s).clamp(0.0, 1.0)
        } else {
            1.0
        };

        if !st.awww_switched && elapsed >= fade_s {
            st.awww_switched = true;
            apply_awww_instant(&new_path_switch);
        }

        let alloc = area.allocation();
        let scale = area.scale_factor().max(1);
        let sw = (alloc.width() * scale).max(1) as f32;
        let sh = (alloc.height() * scale).max(1) as f32;

        unsafe {
            let gl = &st.gl;
            gl.viewport(0, 0, sw as i32, sh as i32);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear(glow::COLOR_BUFFER_BIT);
            gl.enable(glow::BLEND);
            gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);

            gl.use_program(Some(st.prog));
            gl.bind_vertex_array(Some(st.vao));

            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(st.old_tex));
            gl.active_texture(glow::TEXTURE1);
            gl.bind_texture(glow::TEXTURE_2D, Some(st.new_tex));
            gl.active_texture(glow::TEXTURE2);
            gl.bind_texture(glow::TEXTURE_2D, Some(st.field_tex));
            gl.active_texture(glow::TEXTURE3);
            gl.bind_texture(glow::TEXTURE_2D, Some(st.line_tex));

            let u = |name: &str| gl.get_uniform_location(st.prog, name);
            gl.uniform_1_i32(u("oldTex").as_ref(), 0);
            gl.uniform_1_i32(u("newTex").as_ref(), 1);
            gl.uniform_1_i32(u("fieldTex").as_ref(), 2);
            gl.uniform_1_i32(u("lineTex").as_ref(), 3);

            gl.uniform_1_f32(u("u_t").as_ref(), t);
            gl.uniform_1_f32(u("u_opacity").as_ref(), opacity);
            gl.uniform_1_f32(u("u_mode").as_ref(), st.mode);
            gl.uniform_1_f32(u("u_sw").as_ref(), sw);
            gl.uniform_1_f32(u("u_sh").as_ref(), sh);
            gl.uniform_1_f32(u("u_fw").as_ref(), st.fw);
            gl.uniform_1_f32(u("u_fh").as_ref(), st.fh);
            gl.uniform_1_f32(u("u_lw").as_ref(), st.lw);
            gl.uniform_1_f32(u("u_lh").as_ref(), st.lh);
            gl.uniform_1_f32(u("u_glow").as_ref(), 0.9);
            gl.uniform_1_f32(u("u_lineW").as_ref(), 2.2);
            gl.uniform_1_f32(u("u_sdRange").as_ref(), SD_RANGE);
            gl.uniform_1_f32(u("u_seed").as_ref(), st.seed);
            gl.uniform_4_f32_slice(u("u_cropOld").as_ref(), &st.crop_old);
            gl.uniform_4_f32_slice(u("u_cropNew").as_ref(), &st.crop_new);
            gl.uniform_4_f32(u("u_ember").as_ref(), st.ember[0], st.ember[1], st.ember[2], 1.0);
            gl.uniform_4_f32(u("u_ink").as_ref(), st.ink[0], st.ink[1], st.ink[2], 1.0);

            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }

        if elapsed >= st.duration_s + fade_s {
            if !st.awww_switched {
                st.awww_switched = true;
                apply_awww_instant(&new_path_switch);
            }
            glib::idle_add_local_once(|| {
                if gtk::main_level() > 0 {
                    gtk::main_quit();
                }
            });
        }

        glib::Propagation::Stop
    });

    gl_area.add_tick_callback(|area, _| {
        area.queue_draw();
        ControlFlow::Continue
    });

    window.connect_destroy(|_| {
        if gtk::main_level() > 0 {
            gtk::main_quit();
        }
    });

    // Unused CStr import suppression if needed:
    let _ = fs::metadata(new_path);
    let _ = CStr::from_bytes_with_nul(b"\0");

    window.show_all();
    gtk::main();
}
