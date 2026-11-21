use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use image::imageops::FilterType;
use image::{DynamicImage, GenericImageView, GrayImage, Luma, Rgb, RgbImage};
use ort::environment::Environment;
use ort::session::Session;
use ort::value::Tensor;

use crate::paths::{cache_key, model_path, outline_dir, spread_dir};

const GW: usize = 480;
const GH: usize = 270;
const POOL: usize = 8;
pub const FIELD_MAX: u32 = 1280;
pub const SD_RANGE: f32 = 16.0;
const WIDE: f32 = 48.0;

pub enum TransitionKind {
    Contagion {
        field_path: PathBuf,
        line_path: PathBuf,
    },
    Outline {
        field_path: PathBuf,
        line_path: PathBuf,
    },
}

// ───────────────────────────── Fast RNG ─────────────────────────────

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }

    fn range_f64(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.f64()
    }

    fn range_usize(&mut self, lo: usize, hi: usize) -> usize {
        lo + (self.next_u64() as usize % (hi - lo))
    }
}

fn time_seed() -> u64 {
    let ns = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(42);
    ns ^ (unsafe { libc::getpid() } as u64).wrapping_mul(0x9e3779b97f4a7c15)
}

// ───────────────────────────── Noise & Blur ─────────────────────────────

fn value_noise(rng: &mut Rng, w: usize, h: usize, scale: f64) -> Vec<f64> {
    let nx = (w as f64 / scale) as usize + 3;
    let ny = (h as f64 / scale) as usize + 3;
    let mut lat = vec![0.0_f64; nx * ny];
    for v in &mut lat {
        *v = rng.f64();
    }

    let mut out = vec![0.0_f64; w * h];
    for y in 0..h {
        let sy = y as f64 / scale;
        let iy = sy as usize;
        let fy = sy - iy as f64;
        let fy = fy * fy * (3.0 - 2.0 * fy);
        let row0 = iy * nx;
        let row1 = (iy + 1) * nx;
        for x in 0..w {
            let sx = x as f64 / scale;
            let ix = sx as usize;
            let fx = sx - ix as f64;
            let fx = fx * fx * (3.0 - 2.0 * fx);
            let top = lat[row0 + ix] * (1.0 - fx) + lat[row0 + ix + 1] * fx;
            let bot = lat[row1 + ix] * (1.0 - fx) + lat[row1 + ix + 1] * fx;
            out[y * w + x] = top * (1.0 - fy) + bot * fy;
        }
    }
    out
}

fn gaussian_blur(src: &[f64], w: usize, h: usize, sigma: f64) -> Vec<f64> {
    let r = (3.0 * sigma) as isize + 1;
    let klen = (2 * r + 1) as usize;
    let mut kernel = vec![0.0_f64; klen];
    let mut sum = 0.0_f64;
    for i in -r..=r {
        let val = (-((i * i) as f64) / (2.0 * sigma * sigma)).exp();
        kernel[(i + r) as usize] = val;
        sum += val;
    }
    for k in &mut kernel {
        *k /= sum;
    }

    let mut tmp = vec![0.0_f64; w * h];
    for y in 0..h {
        let row = y * w;
        for x in 0..w {
            let mut acc = 0.0;
            for (ki, &kv) in kernel.iter().enumerate() {
                let ox = (x as isize + ki as isize - r).clamp(0, w as isize - 1) as usize;
                acc += src[row + ox] * kv;
            }
            tmp[row + x] = acc;
        }
    }

    let mut out = vec![0.0_f64; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (ki, &kv) in kernel.iter().enumerate() {
                let oy = (y as isize + ki as isize - r).clamp(0, h as isize - 1) as usize;
                acc += tmp[oy * w + x] * kv;
            }
            out[y * w + x] = acc;
        }
    }
    out
}

fn rgb16_image(v: &[f64], w: usize, h: usize, extra_r: Option<&[u8]>) -> RgbImage {
    let mut img = RgbImage::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let q = (v[idx].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16;
            let r = extra_r.map(|er| er[idx]).unwrap_or(0);
            img.put_pixel(x as u32, y as u32, Rgb([r, (q >> 8) as u8, (q & 0xff) as u8]));
        }
    }
    img
}

fn save_atomic_rgb(dest: &Path, img: &RgbImage) {
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let tmp = dest.with_extension(format!("{}.tmp.png", unsafe { libc::getpid() }));
    if img.save(&tmp).is_ok() {
        let _ = fs::rename(&tmp, dest);
    } else {
        let _ = fs::remove_file(&tmp);
    }
}

// ───────────────────────────── Contagion ─────────────────────────────

#[derive(Copy, Clone, PartialEq)]
struct HeapItem {
    t: f64,
    idx: usize,
}

impl Eq for HeapItem {}

impl Ord for HeapItem {
    fn cmp(&self, other: &Self) -> Ordering {
        other.t.partial_cmp(&self.t).unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for HeapItem {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn area_rank(values: &[f64]) -> Vec<f64> {
    let n = values.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut ranked = vec![0.0_f64; n];
    let denom = (n.saturating_sub(1)).max(1) as f64;
    for (rank, &idx) in order.iter().enumerate() {
        ranked[idx] = rank as f64 / denom;
    }
    ranked
}

fn generate_contagion_pair(seed: u64) -> (RgbImage, RgbImage) {
    let mut rng = Rng::new(seed);
    let s1 = rng.range_f64(18.0, 65.0);
    let s2 = rng.range_f64(7.0, 20.0);
    let s3 = rng.range_f64(2.0, 6.0);
    let n1 = value_noise(&mut rng, GW, GH, s1);
    let n2 = value_noise(&mut rng, GW, GH, s2);
    let n3 = value_noise(&mut rng, GW, GH, s3);
    let exp_k = rng.range_f64(5.0, 9.0);
    let grain_k = rng.range_f64(0.15, 0.65);

    let mut cost = vec![0.0_f64; GW * GH];
    for i in 0..cost.len() {
        let n = 0.52 * n1[i] + 0.30 * n2[i] + 0.18 * n3[i];
        cost[i] = 0.12 + (exp_k * (n - 0.5)).exp() + grain_k * rng.f64();
    }

    let mut arrival = vec![f64::INFINITY; GW * GH];
    let mut heap = BinaryHeap::new();
    let n_seeds = rng.range_usize(6, 13);
    let max_t0 = ((GW * GH) as f64).sqrt() * 0.12;
    for _ in 0..n_seeds {
        let sx = (rng.range_f64(0.06, 0.94) * GW as f64) as usize;
        let sy = (rng.range_f64(0.08, 0.92) * GH as f64) as usize;
        let idx = sy.min(GH - 1) * GW + sx.min(GW - 1);
        let t0 = rng.range_f64(0.0, max_t0);
        if t0 < arrival[idx] {
            arrival[idx] = t0;
            heap.push(HeapItem { t: t0, idx });
        }
    }

    const STEPS: [(isize, isize, f64); 8] = [
        (-1, 0, 1.0),
        (1, 0, 1.0),
        (0, -1, 1.0),
        (0, 1, 1.0),
        (-1, -1, std::f64::consts::SQRT_2),
        (1, -1, std::f64::consts::SQRT_2),
        (-1, 1, std::f64::consts::SQRT_2),
        (1, 1, std::f64::consts::SQRT_2),
    ];

    while let Some(HeapItem { t, idx }) = heap.pop() {
        if t != arrival[idx] {
            continue;
        }
        let x = (idx % GW) as isize;
        let y = (idx / GW) as isize;
        let ci = cost[idx];
        for &(dx, dy, dist) in &STEPS {
            let nx = x + dx;
            let ny = y + dy;
            if nx >= 0 && nx < GW as isize && ny >= 0 && ny < GH as isize {
                let j = ny as usize * GW + nx as usize;
                let cand = t + (ci + cost[j]) * 0.5 * dist;
                if cand < arrival[j] {
                    arrival[j] = cand;
                    heap.push(HeapItem { t: cand, idx: j });
                }
            }
        }
    }

    let ranked = area_rank(&arrival);
    let mut field_img = RgbImage::new(GW as u32, GH as u32);
    for y in 0..GH {
        for x in 0..GW {
            let q = (ranked[y * GW + x] * 65535.0 + 0.5) as u16;
            field_img.put_pixel(
                x as u32,
                y as u32,
                Rgb([(q >> 8) as u8, (q & 0xff) as u8, 0]),
            );
        }
    }

    let smooth = gaussian_blur(&ranked, GW, GH, 2.5);
    let line_img = rgb16_image(&smooth, GW, GH, None);
    (field_img, line_img)
}

pub fn ensure_contagion_slot(k: usize) -> (PathBuf, PathBuf) {
    let dir = spread_dir();
    let field_path = dir.join(format!("spread-{k}.png"));
    let line_path = dir.join(format!("spread-{k}-liso.png"));
    if !field_path.exists() || !line_path.exists() {
        let (f, l) = generate_contagion_pair(time_seed() ^ (k as u64 + 1).wrapping_mul(7919));
        save_atomic_rgb(&field_path, &f);
        save_atomic_rgb(&line_path, &l);
    }
    (field_path, line_path)
}

pub fn pick_contagion_and_renew() -> (PathBuf, PathBuf) {
    let k = (time_seed() as usize) % POOL;
    let pair = ensure_contagion_slot(k);
    let (fp, lp) = pair.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(3));
        let (f, l) = generate_contagion_pair(time_seed() ^ (k as u64 + 17));
        save_atomic_rgb(&fp, &f);
        save_atomic_rgb(&lp, &l);
    });
    pair
}

// ───────────────────────────── 2D Euclidean Distance Transform ─────────────────────────────

/// Exact Felzenszwalb-Huttenlocher 2D EDT returning (euclidean_dist, nearest_site_index).
fn edt_2d(mask: &[bool], w: usize, h: usize) -> (Vec<f32>, Vec<usize>) {
    const INF: f32 = 1e18;
    let n = w * h;
    let mut f = vec![INF; n];
    let mut site_row = vec![0usize; n];

    // Pass 1: 1D EDT along columns (store nearest y in same column? No, pass rows then cols).
    // Pass 1 along rows:
    let mut row_d2 = vec![INF; n];
    for y in 0..h {
        let row_off = y * w;
        let mut v = Vec::with_capacity(w);
        let mut z = Vec::with_capacity(w + 1);
        for x in 0..w {
            f[x] = if mask[row_off + x] { 0.0 } else { INF };
        }

        let mut has_any = false;
        for q in 0..w {
            if f[q] >= INF * 0.5 {
                continue;
            }
            has_any = true;
            if v.is_empty() {
                v.push(q);
                z.push(f32::NEG_INFINITY);
                z.push(f32::INFINITY);
            } else {
                loop {
                    let r = *v.last().unwrap();
                    let s = ((f[q] + (q * q) as f32) - (f[r] + (r * r) as f32))
                        / (2 * (q as isize - r as isize)) as f32;
                    if s <= *z.last().unwrap() && v.len() > 1 {
                        v.pop();
                        z.pop();
                    } else if s <= z[z.len() - 2] {
                        v.pop();
                        z.pop();
                        if v.is_empty() {
                            v.push(q);
                            z.clear();
                            z.push(f32::NEG_INFINITY);
                            z.push(f32::INFINITY);
                            break;
                        }
                    } else {
                        z.pop();
                        z.push(s);
                        v.push(q);
                        z.push(f32::INFINITY);
                        break;
                    }
                }
            }
        }

        if !has_any {
            continue;
        }
        let mut k = 0;
        for q in 0..w {
            while z[k + 1] < q as f32 {
                k += 1;
            }
            let vk = v[k];
            let dx = (q as isize - vk as isize) as f32;
            row_d2[row_off + q] = dx * dx;
            site_row[row_off + q] = vk;
        }
    }

    // Pass 2 along columns:
    let mut dist = vec![INF; n];
    let mut nearest = vec![0usize; n];
    for x in 0..w {
        let mut v = Vec::with_capacity(h);
        let mut z = Vec::with_capacity(h + 1);
        let mut has_any = false;

        for q in 0..h {
            let fq = row_d2[q * w + x];
            if fq >= INF * 0.5 {
                continue;
            }
            has_any = true;
            if v.is_empty() {
                v.push(q);
                z.push(f32::NEG_INFINITY);
                z.push(f32::INFINITY);
            } else {
                loop {
                    let r = *v.last().unwrap();
                    let fr = row_d2[r * w + x];
                    let s = ((fq + (q * q) as f32) - (fr + (r * r) as f32))
                        / (2 * (q as isize - r as isize)) as f32;
                    if s <= z[z.len() - 2] {
                        v.pop();
                        z.pop();
                        if v.is_empty() {
                            v.push(q);
                            z.clear();
                            z.push(f32::NEG_INFINITY);
                            z.push(f32::INFINITY);
                            break;
                        }
                    } else {
                        z.pop();
                        z.push(s);
                        v.push(q);
                        z.push(f32::INFINITY);
                        break;
                    }
                }
            }
        }

        if !has_any {
            continue;
        }
        let mut k = 0;
        for q in 0..h {
            while z[k + 1] < q as f32 {
                k += 1;
            }
            let vk = v[k];
            let dy = (q as isize - vk as isize) as f32;
            let d2 = dy * dy + row_d2[vk * w + x];
            dist[q * w + x] = d2.sqrt();
            let sx = site_row[vk * w + x];
            nearest[q * w + x] = vk * w + sx;
        }
    }

    (dist, nearest)
}

// ───────────────────────────── Outline / Character Cutout ─────────────────────────────

static ORT_ENV: OnceLock<Option<Environment>> = OnceLock::new();
static ORT_SESS: OnceLock<Mutex<Option<Session>>> = OnceLock::new();

fn get_ort_session() -> Option<&'static Mutex<Option<Session>>> {
    let env = ORT_ENV
        .get_or_init(|| ort::init().build().ok())
        .as_ref()?;
    let cell = ORT_SESS.get_or_init(|| {
        let mp = model_path();
        let sess = if mp.exists() {
            Session::builder(env).ok().and_then(|mut b| b.commit_from_file(&mp).ok())
        } else {
            None
        };
        Mutex::new(sess)
    });
    Some(cell)
}

fn run_onnx_cutout(img: &DynamicImage) -> Option<GrayImage> {
    let sess_lock = get_ort_session()?;
    let mut guard = sess_lock.lock().ok()?;
    let session = guard.as_mut()?;

    let (mw, mh) = session
        .inputs()
        .first()
        .and_then(|inp| inp.dtype().tensor_shape())
        .and_then(|shape| {
            if shape.len() >= 4 {
                let h = if shape[2] > 0 { shape[2] as u32 } else { 320 };
                let w = if shape[3] > 0 { shape[3] as u32 } else { 320 };
                Some((w, h))
            } else {
                None
            }
        })
        .unwrap_or((320, 320));

    let resized = img.resize_exact(mw, mh, FilterType::Triangle).to_rgb8();
    let plane = (mw * mh) as usize;
    let mut input_data = vec![0.0_f32; 3 * plane];
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];

    for y in 0..mh {
        for x in 0..mw {
            let p = resized.get_pixel(x, y);
            let idx = (y * mw + x) as usize;
            input_data[idx] = (f32::from(p[0]) / 255.0 - MEAN[0]) / STD[0];
            input_data[plane + idx] = (f32::from(p[1]) / 255.0 - MEAN[1]) / STD[1];
            input_data[2 * plane + idx] = (f32::from(p[2]) / 255.0 - MEAN[2]) / STD[2];
        }
    }

    let tensor = Tensor::from_array(([1usize, 3, mh as usize, mw as usize], input_data)).ok()?;
    let outputs = session.run(ort::inputs![tensor]).ok()?;
    let (_, slice) = outputs[0].try_extract_tensor::<f32>().ok()?;
    if slice.len() < plane {
        return None;
    }

    let raw = &slice[..plane];
    let min_v = raw.iter().copied().fold(f32::INFINITY, f32::min);
    let max_v = raw.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let is_logits = min_v < -0.05 || max_v > 1.05;

    let mut mask = GrayImage::new(mw, mh);
    for i in 0..plane {
        let mut p = raw[i];
        if is_logits {
            p = 1.0 / (1.0 + (-p).exp());
        } else if max_v > min_v {
            p = (p - min_v) / (max_v - min_v);
        }
        let val = (p.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
        mask.put_pixel((i as u32) % mw, (i as u32) / mw, Luma([val]));
    }
    Some(mask)
}

fn filter_small_components(fg: &mut [bool], w: usize, h: usize) {
    let min_area = (0.002 * (w * h) as f64) as usize;
    let mut visited = vec![false; w * h];
    let mut queue = VecDeque::new();
    let mut comp = Vec::new();

    for start in 0..(w * h) {
        if !fg[start] || visited[start] {
            continue;
        }
        visited[start] = true;
        queue.push_back(start);
        comp.clear();

        while let Some(idx) = queue.pop_front() {
            comp.push(idx);
            let x = (idx % w) as isize;
            let y = (idx / w) as isize;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let nx = x + dx;
                    let ny = y + dy;
                    if nx >= 0 && nx < w as isize && ny >= 0 && ny < h as isize {
                        let j = ny as usize * w + nx as usize;
                        if fg[j] && !visited[j] {
                            visited[j] = true;
                            queue.push_back(j);
                        }
                    }
                }
            }
        }

        if comp.len() < min_area {
            for &idx in &comp {
                fg[idx] = false;
            }
        }
    }
}

/// Trace ordered boundary loops using Moore-neighbor walking.
fn trace_contours(fg: &[bool], w: usize, h: usize) -> Vec<Vec<(usize, usize)>> {
    let mut is_bound = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            if !fg[idx] {
                continue;
            }
            let edge = x == 0
                || y == 0
                || x + 1 == w
                || y + 1 == h
                || !fg[idx - 1]
                || !fg[idx + 1]
                || !fg[idx - w]
                || !fg[idx + w];
            if edge {
                is_bound[idx] = true;
            }
        }
    }

    // Clockwise 8-neighbor offsets starting from N.
    const NB: [(isize, isize); 8] = [
        (0, -1),
        (1, -1),
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
        (-1, 0),
        (-1, -1),
    ];

    let mut used = vec![false; w * h];
    let mut contours = Vec::new();

    for sy in 0..h {
        for sx in 0..w {
            let s_idx = sy * w + sx;
            if !is_bound[s_idx] || used[s_idx] {
                continue;
            }

            let mut loop_pts = Vec::new();
            let mut cx = sx;
            let mut cy = sy;
            let mut dir = 0usize; // start looking from North

            for _ in 0..(w * h) {
                let c_idx = cy * w + cx;
                if !used[c_idx] {
                    used[c_idx] = true;
                    loop_pts.push((cx, cy));
                }

                let mut found = None;
                for step in 0..8 {
                    let d = (dir + step) % 8;
                    let nx = cx as isize + NB[d].0;
                    let ny = cy as isize + NB[d].1;
                    if nx >= 0 && nx < w as isize && ny >= 0 && ny < h as isize {
                        let n_idx = ny as usize * w + nx as usize;
                        if is_bound[n_idx] && (!used[n_idx] || (nx as usize == sx && ny as usize == sy && loop_pts.len() > 2)) {
                            found = Some((nx as usize, ny as usize, (d + 5) % 8));
                            break;
                        }
                    }
                }

                match found {
                    Some((nx, ny, next_dir)) => {
                        if nx == sx && ny == sy {
                            break;
                        }
                        cx = nx;
                        cy = ny;
                        dir = next_dir;
                    }
                    None => break,
                }
            }

            if loop_pts.len() >= 40 {
                contours.push(loop_pts);
            }
        }
    }

    contours
}

pub fn generate_outline(img_path: &Path) -> Option<(PathBuf, PathBuf)> {
    let dir = outline_dir();
    let _ = fs::create_dir_all(&dir);
    let key = cache_key(img_path);
    let campo_path = dir.join(format!("{key}-campo.png"));
    let linea_path = dir.join(format!("{key}-linea.png"));
    let meta_path = dir.join(format!("{key}.ok"));

    if meta_path.exists() {
        let flag = fs::read_to_string(&meta_path).unwrap_or_default();
        if flag.trim() == "1" && campo_path.exists() && linea_path.exists() {
            return Some((campo_path, linea_path));
        }
        if flag.trim() == "0" {
            return None;
        }
    }

    let orig = image::open(img_path).ok()?;
    let mask_raw = run_onnx_cutout(&orig)?;

    let thumb = orig.resize(FIELD_MAX, FIELD_MAX, FilterType::Triangle);
    let (w, h) = thumb.dimensions();
    let (wu, hu) = (w as usize, h as usize);
    let mask = DynamicImage::ImageLuma8(mask_raw)
        .resize_exact(w, h, FilterType::Triangle)
        .to_luma8();

    let n = wu * hu;
    let mut fg = vec![false; n];
    let mut unsure_cnt = 0usize;
    for y in 0..hu {
        for x in 0..wu {
            let v = f32::from(mask.get_pixel(x as u32, y as u32)[0]) / 255.0;
            if v > 0.1 && v < 0.9 {
                unsure_cnt += 1;
            }
            fg[y * wu + x] = v > 0.5;
        }
    }

    filter_small_components(&mut fg, wu, hu);
    let fg_cnt = fg.iter().filter(|&&b| b).count();
    let cover = fg_cnt as f64 / n as f64;
    let unsure = unsure_cnt as f64 / n as f64;

    if !(0.008..=0.72).contains(&cover) || unsure > 0.14 {
        let _ = fs::write(&meta_path, "0");
        return None;
    }

    let bg: Vec<bool> = fg.iter().map(|&b| !b).collect();
    let (din, _) = edt_2d(&bg, wu, hu);
    let (dout, _) = edt_2d(&fg, wu, hu);

    let mut din_fg: Vec<f32> = din
        .iter()
        .zip(fg.iter())
        .filter_map(|(&d, &is_fg)| if is_fg { Some(d) } else { None })
        .collect();
    din_fg.sort_by(|a, b| a.total_cmp(b));
    let p99_idx = ((din_fg.len() as f64 * 0.99) as usize).min(din_fg.len().saturating_sub(1));
    let din_norm = din_fg.get(p99_idx).copied().unwrap_or(1.0).max(1.0);

    // Outward noise-warped arrival order.
    let mut rng = Rng::new(u64::from_str_radix(&key[..8], 16).unwrap_or(1));
    let n_coarse = value_noise(&mut rng, wu, hu, (wu as f64 / 9.0).max(2.0));
    let n_fine = value_noise(&mut rng, wu, hu, (wu as f64 / 28.0).max(2.0));
    let scale_wh = wu.max(hu) as f64;

    let mut out_indices = Vec::new();
    let mut out_warped = Vec::new();
    for i in 0..n {
        if !fg[i] {
            let noise = 0.6 * n_coarse[i] + 0.4 * n_fine[i];
            let val = f64::from(dout[i]) * (0.75 + 0.5 * noise) + noise * 0.08 * scale_wh;
            out_indices.push(i);
            out_warped.push(val);
        }
    }
    let ranked_out = area_rank(&out_warped);
    let mut order = vec![0.0_f64; n];
    for (&idx, &rval) in out_indices.iter().zip(ranked_out.iter()) {
        order[idx] = rval;
    }
    let smooth_out = gaussian_blur(&order, wu, hu, 4.0);

    // Contour pen-stroke parameterization.
    let contours = trace_contours(&fg, wu, hu);
    let mut edge_mask = vec![false; n];
    let mut param_at = vec![-1.0_f32; n];

    for c in contours {
        let len = c.len();
        let mut seg = vec![0.0_f32; len];
        let mut s = vec![0.0_f32; len];
        let mut top_idx = 0usize;
        let mut min_y = usize::MAX;
        let mut total = 0.0_f32;

        for i in 0..len {
            let (x0, y0) = c[i];
            let (x1, y1) = c[(i + 1) % len];
            if y0 < min_y {
                min_y = y0;
                top_idx = i;
            }
            let d = (((x1 as f32 - x0 as f32).powi(2)) + ((y1 as f32 - y0 as f32).powi(2))).sqrt();
            seg[i] = d;
            total += d;
        }
        if total < 1.0 {
            continue;
        }

        let mut acc = 0.0_f32;
        for step in 0..len {
            let idx = (top_idx + step) % len;
            s[idx] = acc;
            acc += seg[idx];
        }

        let half = (total * 0.5).max(1.0);
        for i in 0..len {
            let (x, y) = c[i];
            let idx = y * wu + x;
            edge_mask[idx] = true;
            if x <= 1 || x + 2 >= wu || y <= 1 || y + 2 >= hu {
                param_at[idx] = -1.0;
            } else {
                let sv = s[i];
                param_at[idx] = sv.min(total - sv) / half;
            }
        }
    }

    let mut line_r = vec![0u8; n];
    if edge_mask.iter().any(|&b| b) {
        let (_, nearest_edge) = edt_2d(&edge_mask, wu, hu);
        for i in 0..n {
            let sd = dout[i] - din[i];
            if sd.abs() < SD_RANGE {
                let p = param_at[nearest_edge[i]];
                if p >= 0.0 {
                    line_r[i] = ((p * 255.0).clamp(1.0, 255.0)) as u8;
                }
            }
        }
    }

    let mut campo_img = RgbImage::new(w, h);
    for y in 0..hu {
        for x in 0..wu {
            let i = y * wu + x;
            let sd = dout[i] - din[i];
            let r = ((dout[i] / WIDE).clamp(0.0, 1.0) * 255.0) as u8;
            let g = ((din[i] / din_norm).clamp(0.0, 1.0) * 255.0) as u8;
            let b = (((sd + SD_RANGE) / (2.0 * SD_RANGE)).clamp(0.0, 1.0) * 255.0) as u8;
            campo_img.put_pixel(x as u32, y as u32, Rgb([r, g, b]));
        }
    }

    let linea_img = rgb16_image(&smooth_out, wu, hu, Some(&line_r));
    save_atomic_rgb(&campo_path, &campo_img);
    save_atomic_rgb(&linea_path, &linea_img);
    let _ = fs::write(&meta_path, "1");

    Some((campo_path, linea_path))
}

pub fn choose_transition(new_img: &Path) -> TransitionKind {
    if let Some((field_path, line_path)) = generate_outline(new_img) {
        return TransitionKind::Outline {
            field_path,
            line_path,
        };
    }
    let (field_path, line_path) = pick_contagion_and_renew();
    TransitionKind::Contagion {
        field_path,
        line_path,
    }
}

#[allow(dead_code)]
pub fn has_outline_cached(img: &Path) -> bool {
    let key = cache_key(img);
    let dir = outline_dir();
    let meta = dir.join(format!("{key}.ok"));
    if let Ok(s) = fs::read_to_string(meta) {
        if s.trim() == "1" {
            return dir.join(format!("{key}-campo.png")).exists()
                && dir.join(format!("{key}-linea.png")).exists();
        }
    }
    false
}
