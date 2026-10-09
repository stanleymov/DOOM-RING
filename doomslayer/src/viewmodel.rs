//! 3D first-person viewmodel: the real DOOM Eternal arms + weapons with their real animations,
//! rendered with our own D3D12 pass inside the present hook, right before the HUD (like Doom draws
//! its viewmodel over the world). Data comes from tools/vm/convert_weapon.py (user's own install).
//!
//! The vendored hudhook calls [`pre_render`] with the game's back buffer bound.

use std::{
    collections::HashMap,
    mem::ManuallyDrop,
    path::{Path, PathBuf},
    sync::Mutex,
};

use windows::{
    Win32::Graphics::{
        Direct3D::{D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST, ID3DBlob},
        Direct3D12::*,
        Dxgi::Common::*,
    },
    core::{Interface, PCSTR, s},
};

use crate::config;

/// Weapon slot -> baked folder name under doom_vm/.
pub const FOLDERS: [&str; 8] = [
    "combat_shotgun", "heavy_cannon", "plasma_rifle", "rocket_launcher", "super_shotgun",
    "ballista", "chaingun", "bfg",
];
pub const CHAINSAW_FOLDER: &str = "chainsaw";
pub const CRUCIBLE_FOLDER: &str = "crucible";
/// The Crucible's energy blade is lit (slayer: off until the side fangs have swung down in the
/// draw, off again once they start folding in the put-away - user).
pub static BLADE_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

const MAX_BONES: usize = 96;
const RING: usize = 4;
/// Per ring slot: frame CB (256) + arms palette + gun palette, each 256-aligned.
const BONES_BYTES: usize = MAX_BONES * 48;
/// Muzzle-flash vertices (pos3 uv2 col4 = 36 bytes) after the two palettes.
const FX_BYTES: usize = 96 * 1024;
const SLOT_BYTES: usize = 256 + 2 * ((BONES_BYTES + 255) & !255) + FX_BYTES;
const SRV_CAPACITY: u32 = 4096;
const MAX_TEX: u32 = 2560;

// ------------------------------------------------------------------------- CPU-side data

struct MeshData {
    part: u8, // 0 arms, 1 gun
    material: String,
    vertices: Vec<u8>, // 56-byte vertices
    indices: Vec<u32>,
}

struct ClipData {
    fps: f32,
    frames: usize,
    arms: Vec<[f32; 12]>, // frames * arms_bones
    gun: Vec<[f32; 12]>,  // frames * gun_bones
    /// Frames one loop of it plays: Doom's loops end on their first pose again, and playing that
    /// frame too held the gun for a frame at every loop (user: Heavy Cannon / BFG idle).
    loop_frames: usize,
}

struct ModelData {
    arms_bones: usize,
    gun_bones: usize,
    meshes: Vec<MeshData>,
    clips: HashMap<String, ClipData>,
    /// Per arms bone: -1 skins the left arm mesh, 1 the right, 0 neither (melee_arm_push).
    arm_side: Vec<i8>,
}

fn parse_model(path: &Path) -> Option<ModelData> {
    let b = std::fs::read(path).ok()?;
    let mut o = 0usize;
    let rd = |o: &mut usize, n: usize| -> Option<&[u8]> {
        let s = b.get(*o..*o + n)?;
        *o += n;
        Some(s)
    };
    if rd(&mut o, 4)? != b"DVM1" {
        return None;
    }
    let u32_ = |o: &mut usize| -> Option<u32> { Some(u32::from_le_bytes(rd(o, 4)?.try_into().ok()?)) };
    let u16_ = |o: &mut usize| -> Option<u16> { Some(u16::from_le_bytes(rd(o, 2)?.try_into().ok()?)) };
    let arms_bones = u32_(&mut o)? as usize;
    let gun_bones = u32_(&mut o)? as usize;
    let nm = u32_(&mut o)? as usize;
    let mut meshes = Vec::with_capacity(nm);
    for _ in 0..nm {
        let part = rd(&mut o, 1)?[0];
        let ln = u16_(&mut o)? as usize;
        let material = String::from_utf8_lossy(rd(&mut o, ln)?).into_owned();
        let nv = u32_(&mut o)? as usize;
        let nf = u32_(&mut o)? as usize;
        let vertices = rd(&mut o, nv * 56)?.to_vec();
        let indices = rd(&mut o, nf * 12)?
            .chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
            .collect();
        meshes.push(MeshData { part, material, vertices, indices });
    }
    let nc = u32_(&mut o)? as usize;
    let mut clips = HashMap::new();
    let mats = |s: &[u8]| -> Vec<[f32; 12]> {
        s.chunks_exact(48)
            .map(|c| std::array::from_fn(|i| f32::from_le_bytes(c[i * 4..i * 4 + 4].try_into().unwrap())))
            .collect()
    };
    for _ in 0..nc {
        let ln = u16_(&mut o)? as usize;
        let name = String::from_utf8_lossy(rd(&mut o, ln)?).into_owned();
        let fps = f32::from_le_bytes(rd(&mut o, 4)?.try_into().ok()?);
        let frames = u32_(&mut o)? as usize;
        let arms = mats(rd(&mut o, frames * arms_bones * 48)?);
        let gun = mats(rd(&mut o, frames * gun_bones * 48)?);
        // (closed: the last frame is the first pose, within 0.1 mm / 1e-4 of a matrix entry)
        let same = |v: &[[f32; 12]], n: usize| {
            n > 0 && v[..n].iter().zip(&v[(frames - 1) * n..frames * n]).all(|(a, b)| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4))
        };
        let closed = frames > 2 && same(&arms, arms_bones) && (gun_bones == 0 || same(&gun, gun_bones));
        let loop_frames = if closed { frames - 1 } else { frames };
        clips.insert(name, ClipData { fps, frames, arms, gun, loop_frames });
    }
    let mut closed: Vec<&str> = clips.iter().filter(|(_, c)| c.loop_frames < c.frames).map(|(n, _)| n.as_str()).collect();
    closed.sort();
    log::info!("vm: closed loops (last frame skipped when looping): {}", closed.join(" "));
    // which arm each arms bone belongs to (the two sleeve meshes share no bones)
    let mut arm_side = vec![0i8; arms_bones];
    for m in meshes.iter().filter(|m| m.part == 0) {
        let side = if m.material.ends_with("_left") { -1 } else if m.material.ends_with("_right") { 1 } else { continue };
        for v in m.vertices.chunks_exact(56) {
            for k in 0..4 {
                let bone = u16::from_le_bytes([v[32 + k * 2], v[33 + k * 2]]) as usize;
                let w = f32::from_le_bytes(v[40 + k * 4..44 + k * 4].try_into().unwrap());
                if w > 0.0 && bone < arms_bones {
                    arm_side[bone] = if arm_side[bone] == 0 || arm_side[bone] == side { side } else { 0 };
                }
            }
        }
    }
    Some(ModelData { arms_bones, gun_bones, meshes, clips, arm_side })
}

/// RGBA8 image with a CPU-built mip chain, capped at MAX_TEX.
struct Image {
    w: u32,
    h: u32,
    mips: Vec<Vec<u8>>,
}

/// A texture whose GPU resource and filled upload heap were made off the render thread; only the
/// copy commands are left (finish_texture).
struct Prepared {
    tex: ID3D12Resource,
    up: ID3D12Resource,
    fp: Vec<D3D12_PLACED_SUBRESOURCE_FOOTPRINT>,
}

// D3D12 resources and the device are free-threaded.
unsafe impl Send for Prepared {}
struct SendDevice(ID3D12Device);
unsafe impl Send for SendDevice {}

/// Heavy half of a texture upload (any thread): create the texture and an upload heap and copy the
/// mip chain into it. These are up to 35 MB each at 2560, which hitched the game on the render thread.
unsafe fn prepare_texture(device: &ID3D12Device, img: &Image) -> windows::core::Result<Prepared> {
    unsafe {
        let mips = img.mips.len() as u16;
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: img.w as u64,
            Height: img.h,
            DepthOrArraySize: 1,
            MipLevels: mips,
            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            ..Default::default()
        };
        let mut tex: Option<ID3D12Resource> = None;
        device.CreateCommittedResource(&heap_props(D3D12_HEAP_TYPE_DEFAULT), D3D12_HEAP_FLAG_NONE, &desc, D3D12_RESOURCE_STATE_COPY_DEST, None, &mut tex)?;
        let n = mips as usize;
        let mut fp = vec![D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default(); n];
        let mut rows = vec![0u32; n];
        let mut row_size = vec![0u64; n];
        let mut total = 0u64;
        device.GetCopyableFootprints(&desc, 0, n as u32, 0, Some(fp.as_mut_ptr()), Some(rows.as_mut_ptr()), Some(row_size.as_mut_ptr()), Some(&mut total));
        let mut up: Option<ID3D12Resource> = None;
        device.CreateCommittedResource(&heap_props(D3D12_HEAP_TYPE_UPLOAD), D3D12_HEAP_FLAG_NONE, &buffer_desc(total.max(256)), D3D12_RESOURCE_STATE_GENERIC_READ, None, &mut up)?;
        let up = up.unwrap();
        let mut ptr = std::ptr::null_mut();
        up.Map(0, None, Some(&mut ptr))?;
        let dst = ptr as *mut u8;
        let (mut mw, mut mh) = (img.w, img.h);
        for m in 0..n {
            let src = &img.mips[m];
            let pitch = fp[m].Footprint.RowPitch as usize;
            let row = mw as usize * 4;
            for y in 0..mh as usize {
                std::ptr::copy_nonoverlapping(src.as_ptr().add(y * row), dst.add(fp[m].Offset as usize + y * pitch), row);
            }
            mw = (mw / 2).max(1);
            mh = (mh / 2).max(1);
        }
        up.Unmap(0, None);
        Ok(Prepared { tex: tex.unwrap(), up, fp })
    }
}

/// Textures the workers have prepared, waiting for the render thread to finish them.
static PREPARED: Mutex<Vec<(PathBuf, Prepared)>> = Mutex::new(Vec::new());
/// Weapon folder the renderer is waiting on: the workers do its textures first.
static WANT: Mutex<Option<String>> = Mutex::new(None);
/// Textures the workers still have to prepare (0 = all done).
static WARM_LEFT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(usize::MAX);

/// Every viewmodel texture the renderer can sample in a weapon folder (glass "albedo" is a mask).
fn folder_textures(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|f| f.path())
        .filter(|p| p.extension().is_some_and(|e| e == "png") && !p.file_stem().is_some_and(|s| s.to_string_lossy().ends_with("_glass")))
        .collect()
}

/// Decoded textures (sync fallback path: load_png).
static DECODED: Mutex<Option<HashMap<PathBuf, std::sync::Arc<Image>>>> = Mutex::new(None);

fn load_png(path: &Path) -> Option<std::sync::Arc<Image>> {
    if let Some(img) = DECODED.lock().ok()?.as_ref().and_then(|m| m.get(path).cloned()) {
        return Some(img);
    }
    let img = std::sync::Arc::new(decode_png(path)?);
    DECODED.lock().ok()?.get_or_insert_with(HashMap::new).insert(path.to_path_buf(), img.clone());
    Some(img)
}

fn decode_png(path: &Path) -> Option<Image> {
    let file = std::fs::File::open(path).ok()?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let px = &buf[..info.buffer_size()];
    let (w, h) = (info.width, info.height);
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        _ => return None,
    };
    let mut img = Image { w, h, mips: vec![rgba] };
    while img.w >= MAX_TEX * 2 || img.h >= MAX_TEX * 2 {
        img = downsample(&img);
        img.mips.truncate(1);
    }
    if img.w > MAX_TEX || img.h > MAX_TEX {
        let k = MAX_TEX as f32 / img.w.max(img.h) as f32;
        img = resample(&img, ((img.w as f32 * k).round() as u32).max(1), ((img.h as f32 * k).round() as u32).max(1));
    }
    let (mut cw, mut ch) = (img.w, img.h);
    while cw > 1 || ch > 1 {
        let prev = Image { w: cw, h: ch, mips: vec![img.mips.last().unwrap().clone()] };
        let next = downsample(&prev);
        cw = next.w;
        ch = next.h;
        img.mips.push(next.mips.into_iter().next().unwrap());
    }
    Some(img)
}

/// Area-weighted resize to a smaller size (scale between 0.5 and 1), separable.
fn resample(src: &Image, w: u32, h: u32) -> Image {
    // weights for one axis: for each output texel, the source span it covers
    let taps = |n_in: u32, n_out: u32| -> Vec<Vec<(usize, f32)>> {
        let k = n_in as f32 / n_out as f32;
        (0..n_out)
            .map(|o| {
                let (a, b) = (o as f32 * k, (o + 1) as f32 * k);
                let mut v = Vec::new();
                let mut i = a.floor() as u32;
                while (i as f32) < b && i < n_in {
                    let wgt = (b.min(i as f32 + 1.0) - a.max(i as f32)).max(0.0);
                    if wgt > 0.0 {
                        v.push((i as usize, wgt / k));
                    }
                    i += 1;
                }
                v
            })
            .collect()
    };
    let (tx, ty) = (taps(src.w, w), taps(src.h, h));
    let s = &src.mips[0];
    let sw = src.w as usize;
    let mut mid = vec![0f32; w as usize * src.h as usize * 4];
    for y in 0..src.h as usize {
        for (x, t) in tx.iter().enumerate() {
            for c in 0..4 {
                mid[(y * w as usize + x) * 4 + c] = t.iter().map(|&(i, k)| s[(y * sw + i) * 4 + c] as f32 * k).sum();
            }
        }
    }
    let mut out = vec![0u8; (w * h * 4) as usize];
    for (y, t) in ty.iter().enumerate() {
        for x in 0..w as usize {
            for c in 0..4 {
                let v: f32 = t.iter().map(|&(i, k)| mid[(i * w as usize + x) * 4 + c] * k).sum();
                out[(y * w as usize + x) * 4 + c] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Image { w, h, mips: vec![out] }
}

fn downsample(src: &Image) -> Image {
    let (w, h) = ((src.w / 2).max(1), (src.h / 2).max(1));
    let s = &src.mips[0];
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            for c in 0..4 {
                let mut acc = 0u32;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let sx = (x * 2 + dx).min(src.w - 1);
                    let sy = (y * 2 + dy).min(src.h - 1);
                    acc += s[((sy * src.w + sx) * 4 + c) as usize] as u32;
                }
                out[((y * w + x) * 4 + c) as usize] = (acc / 4) as u8;
            }
        }
    }
    Image { w, h, mips: vec![out] }
}

// ------------------------------------------------------------------------- GPU objects

struct GpuMesh {
    part: u8,
    _vb: ID3D12Resource,
    _ib: ID3D12Resource,
    vbv: D3D12_VERTEX_BUFFER_VIEW,
    ibv: D3D12_INDEX_BUFFER_VIEW,
    count: u32,
    srv_first: u32, // 5 consecutive SRVs: albedo, normal, spec, emissive, gloss
    has_normal: bool,
    has_spec: bool,
    /// The Crucible's energy blade: drawn after the solid parts, additive (vm.hlsl GlowMain).
    glow: bool,
}

struct GpuModel {
    data: ModelData,
    meshes: Vec<GpuMesh>,
    /// Barrel tip: the gun vertex furthest forward in the idle pose (skinned each frame).
    muzzle: Option<([f32; 3], [u16; 4], [f32; 4])>,
    /// Point 10 cm down the barrel from the muzzle tag (bind space): the barrel axis.
    muzzle_dir: Option<[f32; 3]>,
    /// A sample of the gun's vertices (bind space): the walking bob pivots at the nearest one
    /// still on screen, so the visible back of the gun holds still and the barrel swings.
    samples: Vec<([f32; 3], [u16; 4], [f32; 4])>,
    /// Spinning barrels (info.json "spin"): group, bones, pivot and axis in bind space (cm).
    spins: Vec<(usize, Vec<usize>, glam::Vec3, glam::Vec3)>,
    /// Per mesh: 0 always shown, else only in that Pose.mode (Doom's showHideMeshInfo).
    mesh_modes: Vec<u8>,
    /// Doom's md6def tags: name -> (gun bone, bind position in cm).
    tags: HashMap<String, (u16, [f32; 3])>,
}

/// Where the muzzle is on screen right now (pixels) and in view space (m), for the HUD's flashes.
/// Inspect mode (F10): move / turn the gun with the keyboard for a close look.
/// Arrows move, 1/2 turn left/right, 3/4 tilt, 5/6 closer/further, 0 resets.
pub static INSPECT_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
struct Inspect {
    f10: bool,
    off: glam::Vec3,
    yaw: f32,
    pitch: f32,
    last: Option<std::time::Instant>,
}
static INSPECT: Mutex<Inspect> = Mutex::new(Inspect { f10: false, off: glam::Vec3::ZERO, yaw: 0.0, pitch: 0.0, last: None });

/// Centre of the gun's bones in view space (pivot for tilting the gun in place).
fn bone_centre(pal_g: &[u8], ng: usize) -> glam::Vec3 {
    let mut c = glam::Vec3::ZERO;
    for b in 0..ng {
        let r = |k: usize| f32::from_le_bytes(pal_g[b * 48 + k * 4..b * 48 + k * 4 + 4].try_into().unwrap());
        c += glam::Vec3::new(r(3), r(7), r(11));
    }
    c / ng.max(1) as f32
}

/// Per-frame inspect update; returns the extra transform (about the gun's centre) when on.
fn inspect_step(pal_g: &[u8], ng: usize) -> Option<glam::Mat4> {
    use std::sync::atomic::Ordering::Relaxed;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    let focus = crate::input::game_has_focus();
    let key = |vk: i32| focus && unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000 != 0;
    let mut st = INSPECT.lock().ok()?;
    // the toggle key: keys.inspect (0 = the debugger is off)
    let vk = crate::config::get_cached().keys.inspect as i32;
    let f10 = vk != 0 && key(vk);
    if f10 && !st.f10 {
        let on = !INSPECT_ON.load(Relaxed);
        INSPECT_ON.store(on, Relaxed);
        log::info!("viewmodel inspect {}", if on { "on" } else { "off" });
    }
    st.f10 = f10;
    let now = std::time::Instant::now();
    let dt = st.last.map_or(0.0, |t| (now - t).as_secs_f32()).min(0.1);
    st.last = Some(now);
    if !INSPECT_ON.load(Relaxed) {
        return None;
    }
    let ax = |p: i32, n: i32| key(p) as i32 as f32 - key(n) as i32 as f32;
    let speed = 0.12 * dt; // metres per second
    st.off.x += ax(0x27, 0x25) * speed; // right / left arrow
    st.off.y += ax(0x26, 0x28) * speed; // up / down arrow
    st.off.z += ax(0x36, 0x35) * speed; // 6 further, 5 closer
    st.yaw += ax(0x32, 0x31) * 60f32.to_radians() * dt; // 2 right, 1 left
    st.pitch += ax(0x34, 0x33) * 45f32.to_radians() * dt; // 4 down, 3 up
    if key(0x30) {
        st.off = glam::Vec3::ZERO;
        st.yaw = 0.0;
        st.pitch = 0.0;
    }
    let c = bone_centre(pal_g, ng);
    Some(glam::Mat4::from_translation(st.off + c)
        * glam::Mat4::from_rotation_y(st.yaw)
        * glam::Mat4::from_rotation_x(st.pitch)
        * glam::Mat4::from_translation(-c))
}

/// Gun parts drawn again over the effects (they sit in front of a no-depth glow): folder, meshes.
const FRONT_MESHES: &[(&str, &[usize])] = &[("rocket_launcher", &[12])];

static VM_DUMPED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub static MUZZLE: Mutex<Option<([f32; 2], glam::Vec3)>> = Mutex::new(None);
/// Muzzle in normalized device coords + aspect, to place world-space flashes at the barrel.
pub static MUZZLE_NDC: Mutex<Option<[f32; 3]>> = Mutex::new(None);

/// Plasma Rifle heat (0..1): the gun's glow shifts from its blue to a hot orange.
pub static HEAT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// 0..1 extra emissive glow (Ballista: the arbalest's core heats up while it is drawn).
pub static GLOW: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn set_glow(g: f32) {
    GLOW.store(g.clamp(0.0, 1.0).to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// -1 = not the Plasma Rifle (no tint).
pub fn set_heat(h: f32) {
    HEAT.store(h.clamp(-1.0, 1.0).to_bits(), std::sync::atomic::Ordering::Relaxed);
}

static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();

/// BFG wind-up: (start, length in s) while the shot charges; drawn as a growing green core at
/// the muzzle.
pub static CHARGE: Mutex<Option<(std::time::Instant, f32)>> = Mutex::new(None);

pub fn bfg_charge(len: Option<f32>) {
    if let Ok(mut c) = CHARGE.lock() {
        *c = len.map(|l| (std::time::Instant::now(), l));
    }
}

/// Last shot (time, weapon slot) for the muzzle flash.
static FLASH: Mutex<Option<(std::time::Instant, usize, u32)>> = Mutex::new(None);
/// Look-dev: keep the last flash on screen (bridge `flashhold`).
pub static FLASH_HOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static FLASH_N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x9e37);

pub fn muzzle_flash(slot: usize) {
    let n = FLASH_N.fetch_add(0x9e37_79b9, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut f) = FLASH.lock() {
        *f = Some((std::time::Instant::now(), slot, n ^ (n >> 15)));
    }
}

/// Per weapon slot: colour ramp, front flash size, side flame length, star size (m, view space),
/// duration (s), and a view-space nudge (m) onto the bore when the furthest-forward vertex isn't it.
struct FlashLook {
    ramp: &'static str,
    front: f32,
    side: f32,
    star: f32,
    dur: f32,
    nudge: [f32; 3],
}

const FLASH_LOOKS: [FlashLook; 8] = [
    FlashLook { ramp: "fire", front: 0.12, side: 0.26, star: 0.0, dur: 0.06, nudge: [0.0, 0.0, 0.0] },   // combat shotgun
    FlashLook { ramp: "fire", front: 0.12, side: 0.24, star: 0.0, dur: 0.05, nudge: [0.0, 0.0, 0.0] },  // heavy cannon
    FlashLook { ramp: "blue", front: 0.13, side: 0.22, star: 0.0, dur: 0.05, nudge: [0.0, 0.0, 0.0] },  // plasma rifle
    FlashLook { ramp: "fire", front: 0.13, side: 0.22, star: 0.0, dur: 0.07, nudge: [0.0, 0.0, 0.0] },   // rocket launcher
    FlashLook { ramp: "fire", front: 0.17, side: 0.34, star: 0.14, dur: 0.075, nudge: [0.0, 0.0, 0.0] }, // super shotgun
    FlashLook { ramp: "fire", front: 0.18, side: 0.42, star: 0.12, dur: 0.08, nudge: [0.0, 0.0, 0.0] },   // ballista (red-orange like Doom)
    FlashLook { ramp: "fire", front: 0.13, side: 0.26, star: 0.0, dur: 0.045, nudge: [0.0, 0.0, 0.0] },   // chaingun
    FlashLook { ramp: "green", front: 0.2, side: 0.26, star: 0.13, dur: 0.09, nudge: [0.0, 0.0, 0.0] },  // BFG
];

fn put_vtx(buf: &mut Vec<u8>, p: glam::Vec3, uv: [f32; 2], a: f32) {
    for v in [p.x, p.y, p.z, uv[0], uv[1], 1.0, 1.0, 1.0, a] {
        buf.extend_from_slice(&v.to_le_bytes());
    }
}

/// Two triangles for a quad given its 4 corners (p00 p10 p11 p01) and uv rect.
fn put_quad(buf: &mut Vec<u8>, c: [glam::Vec3; 4], uv: [f32; 4], a: f32) {
    let [u0, v0, u1, v1] = uv;
    let t = [(c[0], [u0, v1]), (c[1], [u1, v1]), (c[2], [u1, v0]), (c[0], [u0, v1]), (c[2], [u1, v0]), (c[3], [u0, v0])];
    for (p, uv) in t {
        put_vtx(buf, p, uv, a);
    }
}

fn vtx(v: &[u8], i: usize) -> ([f32; 3], [u16; 4], [f32; 4]) {
    let f = |o: usize| f32::from_le_bytes(v[i * 56 + o..i * 56 + o + 4].try_into().unwrap());
    let u = |o: usize| u16::from_le_bytes(v[i * 56 + o..i * 56 + o + 2].try_into().unwrap());
    ([f(0), f(4), f(8)], [u(32), u(34), u(36), u(38)], [f(40), f(44), f(48), f(52)])
}

fn skin(p: [f32; 3], bi: [u16; 4], bw: [f32; 4], pal: impl Fn(usize) -> [f32; 12]) -> glam::Vec3 {
    let mut o = glam::Vec3::ZERO;
    for k in 0..4 {
        if bw[k] <= 0.0 {
            continue;
        }
        let m = pal(bi[k] as usize);
        let x = m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3];
        let y = m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7];
        let z = m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11];
        o += glam::Vec3::new(x, y, z) * bw[k];
    }
    o
}

fn gun_samples(data: &ModelData, modes: &[u8]) -> Vec<([f32; 3], [u16; 4], [f32; 4])> {
    let mut out = Vec::new();
    for (mi, m) in data.meshes.iter().enumerate().filter(|(_, m)| m.part == 1) {
        if modes.get(mi).copied().unwrap_or(0) == 9 {
            continue;
        }
        let n = m.vertices.len() / 56;
        let step = (n / 60).max(1);
        for i in (0..n).step_by(step) {
            out.push(vtx(&m.vertices, i));
        }
    }
    out
}

fn find_muzzle(data: &ModelData) -> Option<([f32; 3], [u16; 4], [f32; 4])> {
    let clip = data.clips.get("idle").or_else(|| data.clips.values().next())?;
    let pal = |b: usize| clip.gun.get(b).copied().unwrap_or([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    let mut best: Option<(f32, ([f32; 3], [u16; 4], [f32; 4]))> = None;
    for m in data.meshes.iter().filter(|m| m.part == 1) {
        for i in 0..m.vertices.len() / 56 {
            let v = vtx(&m.vertices, i);
            let z = skin(v.0, v.1, v.2, pal).z;
            if best.is_none_or(|(bz, _)| z > bz) {
                best = Some((z, v));
            }
        }
    }
    best.map(|(_, v)| v)
}

struct Gpu {
    device: ID3D12Device,
    root: ID3D12RootSignature,
    psos: HashMap<i32, ID3D12PipelineState>,
    srv_heap: ID3D12DescriptorHeap,
    srv_next: u32,
    srv_inc: u32,
    dsv_heap: ID3D12DescriptorHeap,
    depth: Option<(ID3D12Resource, u64, u32)>,
    /// Copy of the game's frame (texture, width, height, format, in PSR state?).
    scene: Option<(ID3D12Resource, u64, u32, i32)>,
    scene_psr: bool,
    ring: ID3D12Resource,
    ring_ptr: *mut u8,
    frame: usize,
    models: HashMap<String, Option<GpuModel>>,
    /// albedo, normal, spec, emissive, gloss defaults + the glass marker albedo
    fallback: Option<[u32; 6]>,
    /// Set once the background texture workers are running (see warm_textures).
    warm: Option<Vec<PathBuf>>,
    /// Weapon folders waiting on background textures, and since when.
    waiting: HashMap<String, std::time::Instant>,
    /// Upload buffers kept alive until their copies have certainly executed.
    keepalive: Vec<(usize, ID3D12Resource)>,
    texture_cache: HashMap<PathBuf, u32>,
    /// Muzzle-flash pipeline per back-buffer format, and flash texture tables (5-SRV, t0 = flash).
    fx_psos: HashMap<i32, ID3D12PipelineState>,
    fx_tex: HashMap<String, Option<u32>>,
    /// Pose blending (Doom crossfades every clip change instead of snapping).
    blend: Blend,
    // ---- MSAA (cfg.vm_msaa > 1). To take it out: set vm_msaa = 0 (or default 0 in config.rs);
    // the single-sample path below is the original one and is untouched.
    msaa: Option<Msaa>,
    msaa_heaps: Option<(ID3D12DescriptorHeap, ID3D12DescriptorHeap)>,
    blit_psos: HashMap<i32, ID3D12PipelineState>,
    // ---- FXAA (cfg.vm_fxaa). SRV slot 1 = gun depth, slot 2 = frame after the gun pass.
    post: Option<(ID3D12Resource, u64, u32, i32)>,
    post_psr: bool,
    fxaa_psos: HashMap<i32, ID3D12PipelineState>,
    // ---- smoothed light probe: P (written, read by the gun pass) and Q (last frame's copy),
    // 2x1 RGBA16F; its RTV heap; pipeline.
    probe: Option<(ID3D12Resource, ID3D12Resource, ID3D12DescriptorHeap)>,
    probe_pso: Option<ID3D12PipelineState>,
}

/// Multisampled colour + depth for the gun pass (resolved into the back buffer afterwards).
struct Msaa {
    rt: ID3D12Resource,
    depth: ID3D12Resource,
    w: u64,
    h: u32,
    format: i32,
    samples: u32,
}

#[derive(Default)]
struct Blend {
    key: (String, String),
    last_time: f32,
    last_a: Vec<u8>,
    last_g: Vec<u8>,
    from_a: Vec<u8>,
    from_g: Vec<u8>,
    start: Option<std::time::Instant>,
    /// Length of the blend in progress (s).
    dur: f32,
}

const BLEND_SECS: f32 = 0.12;

fn mix_pal(from: &[u8], to: &mut [u8], k: f32) {
    let n = from.len().min(to.len()) / 4;
    for i in 0..n {
        let a = f32::from_le_bytes(from[i * 4..i * 4 + 4].try_into().unwrap());
        let b = f32::from_le_bytes(to[i * 4..i * 4 + 4].try_into().unwrap());
        to[i * 4..i * 4 + 4].copy_from_slice(&(a + (b - a) * k).to_le_bytes());
    }
}

unsafe impl Send for Gpu {}

static GPU: Mutex<Option<Gpu>> = Mutex::new(None);

fn heap_props(t: D3D12_HEAP_TYPE) -> D3D12_HEAP_PROPERTIES {
    D3D12_HEAP_PROPERTIES { Type: t, ..Default::default() }
}

fn buffer_desc(size: u64) -> D3D12_RESOURCE_DESC {
    D3D12_RESOURCE_DESC {
        Dimension: D3D12_RESOURCE_DIMENSION_BUFFER,
        Width: size,
        Height: 1,
        DepthOrArraySize: 1,
        MipLevels: 1,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Layout: D3D12_TEXTURE_LAYOUT_ROW_MAJOR,
        ..Default::default()
    }
}

unsafe fn upload_buffer(device: &ID3D12Device, data: &[u8]) -> windows::core::Result<ID3D12Resource> {
    let mut res: Option<ID3D12Resource> = None;
    unsafe {
        device.CreateCommittedResource(
            &heap_props(D3D12_HEAP_TYPE_UPLOAD),
            D3D12_HEAP_FLAG_NONE,
            &buffer_desc(data.len().max(256) as u64),
            D3D12_RESOURCE_STATE_GENERIC_READ,
            None,
            &mut res,
        )?;
        let res = res.unwrap();
        let mut ptr = std::ptr::null_mut();
        res.Map(0, None, Some(&mut ptr))?;
        std::ptr::copy_nonoverlapping(data.as_ptr(), ptr as *mut u8, data.len());
        res.Unmap(0, None);
        Ok(res)
    }
}

fn transition(res: &ID3D12Resource, before: D3D12_RESOURCE_STATES, after: D3D12_RESOURCE_STATES) -> D3D12_RESOURCE_BARRIER {
    D3D12_RESOURCE_BARRIER {
        Type: D3D12_RESOURCE_BARRIER_TYPE_TRANSITION,
        Flags: D3D12_RESOURCE_BARRIER_FLAG_NONE,
        Anonymous: D3D12_RESOURCE_BARRIER_0 {
            Transition: ManuallyDrop::new(D3D12_RESOURCE_TRANSITION_BARRIER {
                pResource: unsafe { std::mem::transmute_copy(res) },
                Subresource: D3D12_RESOURCE_BARRIER_ALL_SUBRESOURCES,
                StateBefore: before,
                StateAfter: after,
            }),
        },
    }
}

impl Gpu {
    unsafe fn new(device: &ID3D12Device) -> windows::core::Result<Self> {
        unsafe {
            // Root signature: b0 frame CB, b1 bone palette, t0-t3 table, static sampler.
            let range = D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                NumDescriptors: 5,
                BaseShaderRegister: 0,
                RegisterSpace: 0,
                OffsetInDescriptorsFromTableStart: 0,
            };
            let scene_range = D3D12_DESCRIPTOR_RANGE {
                RangeType: D3D12_DESCRIPTOR_RANGE_TYPE_SRV,
                NumDescriptors: 2, // t5 scene, t6 light probe
                BaseShaderRegister: 5,
                RegisterSpace: 0,
                OffsetInDescriptorsFromTableStart: 0,
            };
            let params = [
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_CBV,
                    Anonymous: D3D12_ROOT_PARAMETER_0 {
                        Descriptor: D3D12_ROOT_DESCRIPTOR { ShaderRegister: 0, RegisterSpace: 0 },
                    },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_ALL,
                },
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_CBV,
                    Anonymous: D3D12_ROOT_PARAMETER_0 {
                        Descriptor: D3D12_ROOT_DESCRIPTOR { ShaderRegister: 1, RegisterSpace: 0 },
                    },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_VERTEX,
                },
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                    Anonymous: D3D12_ROOT_PARAMETER_0 {
                        DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                            NumDescriptorRanges: 1,
                            pDescriptorRanges: &range,
                        },
                    },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                },
                D3D12_ROOT_PARAMETER {
                    ParameterType: D3D12_ROOT_PARAMETER_TYPE_DESCRIPTOR_TABLE,
                    Anonymous: D3D12_ROOT_PARAMETER_0 {
                        DescriptorTable: D3D12_ROOT_DESCRIPTOR_TABLE {
                            NumDescriptorRanges: 1,
                            pDescriptorRanges: &scene_range,
                        },
                    },
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                },
            ];
            let sampler = D3D12_STATIC_SAMPLER_DESC {
                Filter: D3D12_FILTER_ANISOTROPIC,
                AddressU: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                AddressV: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                AddressW: D3D12_TEXTURE_ADDRESS_MODE_WRAP,
                MaxAnisotropy: 8,
                ComparisonFunc: D3D12_COMPARISON_FUNC_ALWAYS,
                MaxLOD: f32::MAX,
                ShaderRegister: 0,
                ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                ..Default::default()
            };
            let samplers = [
                sampler,
                D3D12_STATIC_SAMPLER_DESC {
                    Filter: D3D12_FILTER_MIN_MAG_MIP_LINEAR,
                    AddressU: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                    AddressV: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                    AddressW: D3D12_TEXTURE_ADDRESS_MODE_CLAMP,
                    ComparisonFunc: D3D12_COMPARISON_FUNC_ALWAYS,
                    MaxLOD: f32::MAX,
                    ShaderRegister: 1,
                    ShaderVisibility: D3D12_SHADER_VISIBILITY_PIXEL,
                    ..Default::default()
                },
            ];
            let desc = D3D12_ROOT_SIGNATURE_DESC {
                NumParameters: params.len() as u32,
                pParameters: params.as_ptr(),
                NumStaticSamplers: samplers.len() as u32,
                pStaticSamplers: samplers.as_ptr(),
                Flags: D3D12_ROOT_SIGNATURE_FLAG_ALLOW_INPUT_ASSEMBLER_INPUT_LAYOUT,
            };
            let mut blob: Option<ID3DBlob> = None;
            let mut err: Option<ID3DBlob> = None;
            D3D12SerializeRootSignature(&desc, D3D_ROOT_SIGNATURE_VERSION_1, &mut blob, Some(&mut err))?;
            let blob = blob.unwrap();
            let root: ID3D12RootSignature = device.CreateRootSignature(
                0,
                std::slice::from_raw_parts(blob.GetBufferPointer() as *const u8, blob.GetBufferSize()),
            )?;

            let srv_heap: ID3D12DescriptorHeap = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV,
                NumDescriptors: SRV_CAPACITY,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_SHADER_VISIBLE,
                NodeMask: 0,
            })?;
            let dsv_heap: ID3D12DescriptorHeap = device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                Type: D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
                NumDescriptors: 1,
                Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                NodeMask: 0,
            })?;
            let mut ring: Option<ID3D12Resource> = None;
            device.CreateCommittedResource(
                &heap_props(D3D12_HEAP_TYPE_UPLOAD),
                D3D12_HEAP_FLAG_NONE,
                &buffer_desc((SLOT_BYTES * RING) as u64),
                D3D12_RESOURCE_STATE_GENERIC_READ,
                None,
                &mut ring,
            )?;
            let ring = ring.unwrap();
            let mut ptr = std::ptr::null_mut();
            ring.Map(0, None, Some(&mut ptr))?;
            Ok(Self {
                device: device.clone(),
                root,
                psos: HashMap::new(),
                srv_inc: device.GetDescriptorHandleIncrementSize(D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV),
                srv_heap,
                // slots: 0 scene, 1 light probe (t6 of the gun pass), 2 frame after the gun (FXAA),
                // 3 spare, 4 gun depth (FXAA), 5 scene again + 6 last probe (t5/t6 of the probe pass)
                srv_next: 7,
                dsv_heap,
                depth: None,
                scene: None,
                scene_psr: false,
                ring,
                ring_ptr: ptr as *mut u8,
                frame: 0,
                models: HashMap::new(),
                fallback: None,
                warm: None,
                waiting: HashMap::new(),
                keepalive: Vec::new(),
                texture_cache: HashMap::new(),
                blend: Blend::default(),
                fx_psos: HashMap::new(),
                fx_tex: HashMap::new(),
                msaa: None,
                msaa_heaps: None,
                blit_psos: HashMap::new(),
                post: None,
                post_psr: false,
                fxaa_psos: HashMap::new(),
                probe: None,
                probe_pso: None,
            })
        }
    }

    /// mode 0 = the gun (outside faces only), 1 = the arms (both sides: a punch swings the open
    /// end of the sleeve into view - its inside must look solid, not see-through), 2 = the gun
    /// again on top of the effects (equal depth passes: parts in front of a glow).
    unsafe fn pso(&mut self, format: DXGI_FORMAT, samples: u32, mode: i32) -> windows::core::Result<ID3D12PipelineState> {
        let key = format.0 * 16 + samples as i32 + mode * 1_000_000;
        if let Some(p) = self.psos.get(&key) {
            return Ok(p.clone());
        }
        static VS: &[u8] = include_bytes!("../shaders/vm_vs.cso");
        static PS_SOLID: &[u8] = include_bytes!("../shaders/vm_ps.cso");
        // mode 3: the Crucible blade - its own pixel shader, additive, no depth write, both sides
        static PS_GLOW: &[u8] = include_bytes!("../shaders/vm_glow_ps.cso");
        let PS: &[u8] = if mode == 3 { PS_GLOW } else { PS_SOLID };
        let el = |name: PCSTR, fmt: DXGI_FORMAT, off: u32| D3D12_INPUT_ELEMENT_DESC {
            SemanticName: name,
            SemanticIndex: 0,
            Format: fmt,
            InputSlot: 0,
            AlignedByteOffset: off,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        };
        let layout = [
            el(s!("POSITION"), DXGI_FORMAT_R32G32B32_FLOAT, 0),
            el(s!("NORMAL"), DXGI_FORMAT_R32G32B32_FLOAT, 12),
            el(s!("TEXCOORD"), DXGI_FORMAT_R32G32_FLOAT, 24),
            el(s!("BLENDINDICES"), DXGI_FORMAT_R16G16B16A16_UINT, 32),
            el(s!("BLENDWEIGHT"), DXGI_FORMAT_R32G32B32A32_FLOAT, 40),
        ];
        let mut rtv_formats = [DXGI_FORMAT_UNKNOWN; 8];
        rtv_formats[0] = format;
        let mut blend = D3D12_BLEND_DESC::default();
        blend.RenderTarget[0] = D3D12_RENDER_TARGET_BLEND_DESC {
            BlendEnable: (mode == 3).into(),
            LogicOpEnable: false.into(),
            SrcBlend: D3D12_BLEND_ONE,
            DestBlend: if mode == 3 { D3D12_BLEND_ONE } else { D3D12_BLEND_ZERO },
            BlendOp: D3D12_BLEND_OP_ADD,
            SrcBlendAlpha: D3D12_BLEND_ONE,
            DestBlendAlpha: D3D12_BLEND_ZERO,
            BlendOpAlpha: D3D12_BLEND_OP_ADD,
            LogicOp: D3D12_LOGIC_OP_NOOP,
            RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            pRootSignature: unsafe { std::mem::transmute_copy(&self.root) },
            VS: D3D12_SHADER_BYTECODE { pShaderBytecode: VS.as_ptr() as _, BytecodeLength: VS.len() },
            PS: D3D12_SHADER_BYTECODE { pShaderBytecode: PS.as_ptr() as _, BytecodeLength: PS.len() },
            BlendState: blend,
            SampleMask: u32::MAX,
            RasterizerState: D3D12_RASTERIZER_DESC {
                FillMode: D3D12_FILL_MODE_SOLID,
                // The bake mirrors the model (Doom -> view axes, det -1), so its outside faces are
                // wound the other way: cull D3D's "front" faces. Without culling the inner faces of
                // thin panels fought with the outer ones (black seams, sawtooth, see-through spots).
                CullMode: if mode == 1 || mode == 3 { D3D12_CULL_MODE_NONE } else { D3D12_CULL_MODE_FRONT },
                DepthClipEnable: true.into(),
                ..Default::default()
            },
            DepthStencilState: D3D12_DEPTH_STENCIL_DESC {
                DepthEnable: true.into(),
                DepthWriteMask: if mode == 3 { D3D12_DEPTH_WRITE_MASK_ZERO } else { D3D12_DEPTH_WRITE_MASK_ALL },
                DepthFunc: if mode == 2 || mode == 3 { D3D12_COMPARISON_FUNC_LESS_EQUAL } else { D3D12_COMPARISON_FUNC_LESS },
                ..Default::default()
            },
            InputLayout: D3D12_INPUT_LAYOUT_DESC {
                pInputElementDescs: layout.as_ptr(),
                NumElements: layout.len() as u32,
            },
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            NumRenderTargets: 1,
            RTVFormats: rtv_formats,
            DSVFormat: DXGI_FORMAT_D32_FLOAT,
            SampleDesc: DXGI_SAMPLE_DESC { Count: samples.max(1), Quality: 0 },
            ..Default::default()
        };
        let pso: ID3D12PipelineState = unsafe { self.device.CreateGraphicsPipelineState(&desc)? };
        log::info!("viewmodel PSO created for format {} x{samples} mode {mode}", format.0);
        self.psos.insert(key, pso.clone());
        Ok(pso)
    }

    unsafe fn fx_pso(&mut self, format: DXGI_FORMAT, depth: bool, samples: u32) -> windows::core::Result<ID3D12PipelineState> {
        let key = (format.0 * 2 + depth as i32) * 16 + samples as i32;
        if let Some(p) = self.fx_psos.get(&key) {
            return Ok(p.clone());
        }
        static VS: &[u8] = include_bytes!("../shaders/fx_vs.cso");
        static PS: &[u8] = include_bytes!("../shaders/fx_ps.cso");
        let el = |name: PCSTR, fmt: DXGI_FORMAT, off: u32| D3D12_INPUT_ELEMENT_DESC {
            SemanticName: name,
            SemanticIndex: 0,
            Format: fmt,
            InputSlot: 0,
            AlignedByteOffset: off,
            InputSlotClass: D3D12_INPUT_CLASSIFICATION_PER_VERTEX_DATA,
            InstanceDataStepRate: 0,
        };
        let layout = [
            el(s!("POSITION"), DXGI_FORMAT_R32G32B32_FLOAT, 0),
            el(s!("TEXCOORD"), DXGI_FORMAT_R32G32_FLOAT, 12),
            el(s!("COLOR"), DXGI_FORMAT_R32G32B32A32_FLOAT, 20),
        ];
        let mut rtv_formats = [DXGI_FORMAT_UNKNOWN; 8];
        rtv_formats[0] = format;
        let mut blend = D3D12_BLEND_DESC::default();
        // Additive (premultiplied flash colour).
        blend.RenderTarget[0] = D3D12_RENDER_TARGET_BLEND_DESC {
            BlendEnable: true.into(),
            LogicOpEnable: false.into(),
            SrcBlend: D3D12_BLEND_ONE,
            DestBlend: D3D12_BLEND_ONE,
            BlendOp: D3D12_BLEND_OP_ADD,
            SrcBlendAlpha: D3D12_BLEND_ZERO,
            DestBlendAlpha: D3D12_BLEND_ONE,
            BlendOpAlpha: D3D12_BLEND_OP_ADD,
            LogicOp: D3D12_LOGIC_OP_NOOP,
            RenderTargetWriteMask: D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8,
        };
        let desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            pRootSignature: unsafe { std::mem::transmute_copy(&self.root) },
            VS: D3D12_SHADER_BYTECODE { pShaderBytecode: VS.as_ptr() as _, BytecodeLength: VS.len() },
            PS: D3D12_SHADER_BYTECODE { pShaderBytecode: PS.as_ptr() as _, BytecodeLength: PS.len() },
            BlendState: blend,
            SampleMask: u32::MAX,
            RasterizerState: D3D12_RASTERIZER_DESC {
                FillMode: D3D12_FILL_MODE_SOLID,
                CullMode: D3D12_CULL_MODE_NONE,
                DepthClipEnable: true.into(),
                ..Default::default()
            },
            // World visuals are depth-tested (behind the gun); the muzzle flash, anchored at the
            // bore, is drawn over the gun's tip like Doom's.
            DepthStencilState: D3D12_DEPTH_STENCIL_DESC {
                DepthEnable: depth.into(),
                DepthWriteMask: D3D12_DEPTH_WRITE_MASK_ZERO,
                DepthFunc: D3D12_COMPARISON_FUNC_LESS_EQUAL,
                ..Default::default()
            },
            InputLayout: D3D12_INPUT_LAYOUT_DESC { pInputElementDescs: layout.as_ptr(), NumElements: layout.len() as u32 },
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            NumRenderTargets: 1,
            RTVFormats: rtv_formats,
            DSVFormat: DXGI_FORMAT_D32_FLOAT,
            SampleDesc: DXGI_SAMPLE_DESC { Count: samples.max(1), Quality: 0 },
            ..Default::default()
        };
        let pso: ID3D12PipelineState = unsafe { self.device.CreateGraphicsPipelineState(&desc)? };
        log::info!("muzzle flash PSO created for format {} x{samples}", format.0);
        self.fx_psos.insert(key, pso.clone());
        Ok(pso)
    }

    /// 5-SRV table whose t0 is a doom_fx texture (the rest are fallbacks).
    unsafe fn fx_texture(&mut self, cl: &ID3D12GraphicsCommandList, name: &str) -> Option<u32> {
        if let Some(t) = self.fx_tex.get(name) {
            return *t;
        }
        let path = config::mod_dir().join("doom_fx").join(format!("{name}.png"));
        let out = (|| {
            let img = load_png(&path)?;
            let fb = unsafe { self.fallbacks(cl).ok()? };
            if self.srv_next + 6 > SRV_CAPACITY {
                return None;
            }
            let tex = self.srv_next;
            self.srv_next += 1;
            unsafe { self.texture(cl, &img, tex).ok()? };
            let first = self.srv_next;
            self.srv_next += 5;
            self.copy_srv(tex, first);
            for k in 1..5 {
                self.copy_srv(fb[k], first + k as u32);
            }
            Some(first)
        })();
        if out.is_none() {
            log::warn!("muzzle flash texture {} missing", path.display());
        }
        self.fx_tex.insert(name.to_string(), out);
        out
    }

    /// Copy the back buffer (game frame, already in RENDER_TARGET) into our scene texture.
    /// Highest supported MSAA count <= `want` for this format (1 = none).
    fn msaa_samples(&self, format: DXGI_FORMAT, want: u32) -> u32 {
        let mut n = want.clamp(1, 8);
        while n > 1 {
            let mut q = D3D12_FEATURE_DATA_MULTISAMPLE_QUALITY_LEVELS { Format: format, SampleCount: n, ..Default::default() };
            let ok = unsafe {
                self.device.CheckFeatureSupport(
                    D3D12_FEATURE_MULTISAMPLE_QUALITY_LEVELS,
                    &mut q as *mut _ as *mut _,
                    std::mem::size_of_val(&q) as u32,
                )
            };
            if ok.is_ok() && q.NumQualityLevels > 0 {
                return n;
            }
            n /= 2;
        }
        1
    }

    /// (Re)create the multisampled colour / depth targets for this back buffer.
    unsafe fn ensure_msaa(&mut self, w: u64, h: u32, format: DXGI_FORMAT, samples: u32) -> windows::core::Result<()> {
        if self.msaa.as_ref().is_some_and(|m| m.w == w && m.h == h && m.format == format.0 && m.samples == samples) {
            return Ok(());
        }
        unsafe {
            if self.msaa_heaps.is_none() {
                let rtv: ID3D12DescriptorHeap = self.device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                    NumDescriptors: 1,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                    NodeMask: 0,
                })?;
                let dsv: ID3D12DescriptorHeap = self.device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_DSV,
                    NumDescriptors: 1,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                    NodeMask: 0,
                })?;
                self.msaa_heaps = Some((rtv, dsv));
            }
            let mk = |fmt: DXGI_FORMAT, flags: D3D12_RESOURCE_FLAGS, state: D3D12_RESOURCE_STATES, clear: Option<D3D12_CLEAR_VALUE>| -> windows::core::Result<ID3D12Resource> {
                let desc = D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                    Width: w,
                    Height: h,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: fmt,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: samples, Quality: 0 },
                    Flags: flags,
                    ..Default::default()
                };
                let mut res: Option<ID3D12Resource> = None;
                self.device.CreateCommittedResource(&heap_props(D3D12_HEAP_TYPE_DEFAULT), D3D12_HEAP_FLAG_NONE, &desc, state, clear.as_ref().map(|c| c as *const _), &mut res)?;
                Ok(res.unwrap())
            };
            let rt = mk(format, D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET, D3D12_RESOURCE_STATE_RESOLVE_SOURCE, None)?;
            let depth = mk(
                DXGI_FORMAT_D32_FLOAT,
                D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL,
                D3D12_RESOURCE_STATE_DEPTH_WRITE,
                Some(D3D12_CLEAR_VALUE {
                    Format: DXGI_FORMAT_D32_FLOAT,
                    Anonymous: D3D12_CLEAR_VALUE_0 { DepthStencil: D3D12_DEPTH_STENCIL_VALUE { Depth: 1.0, Stencil: 0 } },
                }),
            )?;
            let (rtv_heap, dsv_heap) = self.msaa_heaps.as_ref().unwrap();
            self.device.CreateRenderTargetView(&rt, None, rtv_heap.GetCPUDescriptorHandleForHeapStart());
            self.device.CreateDepthStencilView(&depth, None, dsv_heap.GetCPUDescriptorHandleForHeapStart());
            if let Some(old) = self.msaa.take() {
                self.keepalive.push((self.frame, old.rt));
                self.keepalive.push((self.frame, old.depth));
            }
            log::info!("viewmodel MSAA x{samples} targets {w}x{h}");
            self.msaa = Some(Msaa { rt, depth, w, h, format: format.0, samples });
        }
        Ok(())
    }

    /// Full-screen copy of the scene texture (t5) into the bound multisampled target.
    unsafe fn blit_pso(&mut self, format: DXGI_FORMAT, samples: u32) -> windows::core::Result<ID3D12PipelineState> {
        let key = format.0 * 16 + samples as i32;
        if let Some(p) = self.blit_psos.get(&key) {
            return Ok(p.clone());
        }
        static VS: &[u8] = include_bytes!("../shaders/blit_vs.cso");
        static PS: &[u8] = include_bytes!("../shaders/blit_ps.cso");
        let mut rtv_formats = [DXGI_FORMAT_UNKNOWN; 8];
        rtv_formats[0] = format;
        let mut blend = D3D12_BLEND_DESC::default();
        blend.RenderTarget[0].RenderTargetWriteMask = D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8;
        let desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            pRootSignature: unsafe { std::mem::transmute_copy(&self.root) },
            VS: D3D12_SHADER_BYTECODE { pShaderBytecode: VS.as_ptr() as _, BytecodeLength: VS.len() },
            PS: D3D12_SHADER_BYTECODE { pShaderBytecode: PS.as_ptr() as _, BytecodeLength: PS.len() },
            BlendState: blend,
            SampleMask: u32::MAX,
            RasterizerState: D3D12_RASTERIZER_DESC {
                FillMode: D3D12_FILL_MODE_SOLID,
                CullMode: D3D12_CULL_MODE_NONE,
                DepthClipEnable: true.into(),
                ..Default::default()
            },
            DepthStencilState: D3D12_DEPTH_STENCIL_DESC { DepthEnable: false.into(), ..Default::default() },
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            NumRenderTargets: 1,
            RTVFormats: rtv_formats,
            DSVFormat: DXGI_FORMAT_D32_FLOAT,
            SampleDesc: DXGI_SAMPLE_DESC { Count: samples, Quality: 0 },
            ..Default::default()
        };
        let pso: ID3D12PipelineState = unsafe { self.device.CreateGraphicsPipelineState(&desc)? };
        self.blit_psos.insert(key, pso.clone());
        Ok(pso)
    }

    /// FXAA pipeline (full-screen, no depth, no blend; discards where no gun was drawn).
    unsafe fn fxaa_pso(&mut self, format: DXGI_FORMAT) -> windows::core::Result<ID3D12PipelineState> {
        if let Some(p) = self.fxaa_psos.get(&format.0) {
            return Ok(p.clone());
        }
        static VS: &[u8] = include_bytes!("../shaders/fxaa_vs.cso");
        static PS: &[u8] = include_bytes!("../shaders/fxaa_ps.cso");
        let mut rtv_formats = [DXGI_FORMAT_UNKNOWN; 8];
        rtv_formats[0] = format;
        let mut blend = D3D12_BLEND_DESC::default();
        blend.RenderTarget[0].RenderTargetWriteMask = D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8;
        let desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
            pRootSignature: unsafe { std::mem::transmute_copy(&self.root) },
            VS: D3D12_SHADER_BYTECODE { pShaderBytecode: VS.as_ptr() as _, BytecodeLength: VS.len() },
            PS: D3D12_SHADER_BYTECODE { pShaderBytecode: PS.as_ptr() as _, BytecodeLength: PS.len() },
            BlendState: blend,
            SampleMask: u32::MAX,
            RasterizerState: D3D12_RASTERIZER_DESC {
                FillMode: D3D12_FILL_MODE_SOLID,
                CullMode: D3D12_CULL_MODE_NONE,
                DepthClipEnable: true.into(),
                ..Default::default()
            },
            DepthStencilState: D3D12_DEPTH_STENCIL_DESC { DepthEnable: false.into(), ..Default::default() },
            PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
            NumRenderTargets: 1,
            RTVFormats: rtv_formats,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            ..Default::default()
        };
        let pso: ID3D12PipelineState = unsafe { self.device.CreateGraphicsPipelineState(&desc)? };
        log::info!("viewmodel FXAA PSO created for format {}", format.0);
        self.fxaa_psos.insert(format.0, pso.clone());
        Ok(pso)
    }

    /// Smoothed scene light (probe.hlsl): copy last frame's P into Q, then write P from the scene
    /// copy and Q. Leaves P readable as t6 (slot 1) for the gun pass.
    unsafe fn update_probe(&mut self, cl: &ID3D12GraphicsCommandList) -> windows::core::Result<()> {
        unsafe {
            if self.probe.is_none() {
                let desc = D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                    Width: 2,
                    Height: 1,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Flags: D3D12_RESOURCE_FLAG_ALLOW_RENDER_TARGET,
                    ..Default::default()
                };
                let clear = D3D12_CLEAR_VALUE {
                    Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
                    Anonymous: D3D12_CLEAR_VALUE_0 { Color: [0.0; 4] },
                };
                let mk = |state| -> windows::core::Result<ID3D12Resource> {
                    let mut r: Option<ID3D12Resource> = None;
                    self.device.CreateCommittedResource(&heap_props(D3D12_HEAP_TYPE_DEFAULT), D3D12_HEAP_FLAG_NONE, &desc, state, Some(&clear), &mut r)?;
                    Ok(r.unwrap())
                };
                let pr = mk(D3D12_RESOURCE_STATE_RENDER_TARGET)?;
                let q = mk(D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE)?;
                let rtv: ID3D12DescriptorHeap = self.device.CreateDescriptorHeap(&D3D12_DESCRIPTOR_HEAP_DESC {
                    Type: D3D12_DESCRIPTOR_HEAP_TYPE_RTV,
                    NumDescriptors: 1,
                    Flags: D3D12_DESCRIPTOR_HEAP_FLAG_NONE,
                    NodeMask: 0,
                })?;
                self.device.CreateRenderTargetView(&pr, None, rtv.GetCPUDescriptorHandleForHeapStart());
                // clear once so the first frame reads alpha 0 (= take the measurement as is)
                cl.ClearRenderTargetView(rtv.GetCPUDescriptorHandleForHeapStart(), &[0.0; 4], None);
                cl.ResourceBarrier(&[transition(&pr, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE)]);
                let srv = D3D12_SHADER_RESOURCE_VIEW_DESC {
                    Format: DXGI_FORMAT_R16G16B16A16_FLOAT,
                    ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                    Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                    Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 { Texture2D: D3D12_TEX2D_SRV { MipLevels: 1, ..Default::default() } },
                };
                self.device.CreateShaderResourceView(&pr, Some(&srv), self.srv_cpu(1));
                self.device.CreateShaderResourceView(&q, Some(&srv), self.srv_cpu(6));
                self.probe = Some((pr, q, rtv));
            }
            if self.probe_pso.is_none() {
                static VS: &[u8] = include_bytes!("../shaders/probe_vs.cso");
                static PS: &[u8] = include_bytes!("../shaders/probe_ps.cso");
                let mut rtv_formats = [DXGI_FORMAT_UNKNOWN; 8];
                rtv_formats[0] = DXGI_FORMAT_R16G16B16A16_FLOAT;
                let mut blend = D3D12_BLEND_DESC::default();
                blend.RenderTarget[0].RenderTargetWriteMask = D3D12_COLOR_WRITE_ENABLE_ALL.0 as u8;
                let desc = D3D12_GRAPHICS_PIPELINE_STATE_DESC {
                    pRootSignature: std::mem::transmute_copy(&self.root),
                    VS: D3D12_SHADER_BYTECODE { pShaderBytecode: VS.as_ptr() as _, BytecodeLength: VS.len() },
                    PS: D3D12_SHADER_BYTECODE { pShaderBytecode: PS.as_ptr() as _, BytecodeLength: PS.len() },
                    BlendState: blend,
                    SampleMask: u32::MAX,
                    RasterizerState: D3D12_RASTERIZER_DESC {
                        FillMode: D3D12_FILL_MODE_SOLID,
                        CullMode: D3D12_CULL_MODE_NONE,
                        DepthClipEnable: true.into(),
                        ..Default::default()
                    },
                    DepthStencilState: D3D12_DEPTH_STENCIL_DESC { DepthEnable: false.into(), ..Default::default() },
                    PrimitiveTopologyType: D3D12_PRIMITIVE_TOPOLOGY_TYPE_TRIANGLE,
                    NumRenderTargets: 1,
                    RTVFormats: rtv_formats,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    ..Default::default()
                };
                self.probe_pso = Some(self.device.CreateGraphicsPipelineState(&desc)?);
                log::info!("viewmodel light probe ready");
            }
            let (pr, q, rtv) = self.probe.as_ref().unwrap();
            cl.ResourceBarrier(&[
                transition(pr, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_COPY_SOURCE),
                transition(q, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_COPY_DEST),
            ]);
            cl.CopyResource(q, pr);
            cl.ResourceBarrier(&[
                transition(pr, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET),
                transition(q, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE),
            ]);
            let h = rtv.GetCPUDescriptorHandleForHeapStart();
            cl.OMSetRenderTargets(1, Some(&h), false, None);
            cl.RSSetViewports(&[D3D12_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: 2.0, Height: 1.0, MinDepth: 0.0, MaxDepth: 1.0 }]);
            cl.RSSetScissorRects(&[windows::Win32::Foundation::RECT { left: 0, top: 0, right: 2, bottom: 1 }]);
            cl.SetGraphicsRootSignature(&self.root);
            cl.SetDescriptorHeaps(&[Some(self.srv_heap.clone())]);
            cl.SetPipelineState(self.probe_pso.as_ref().unwrap());
            cl.SetGraphicsRootDescriptorTable(3, self.srv_gpu(5));
            cl.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            cl.DrawInstanced(3, 1, 0, 0);
            let (pr, _, _) = self.probe.as_ref().unwrap();
            cl.ResourceBarrier(&[transition(pr, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE)]);
            Ok(())
        }
    }

    /// Copy of the back buffer after the gun pass (FXAA reads it, SRV slot 2).
    unsafe fn copy_post(&mut self, cl: &ID3D12GraphicsCommandList, target: &ID3D12Resource) -> windows::core::Result<()> {
        unsafe {
            let d = target.GetDesc();
            if !self.post.as_ref().is_some_and(|s| s.1 == d.Width && s.2 == d.Height && s.3 == d.Format.0) {
                let desc = D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                    Width: d.Width,
                    Height: d.Height,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: d.Format,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    ..Default::default()
                };
                let mut res: Option<ID3D12Resource> = None;
                self.device.CreateCommittedResource(&heap_props(D3D12_HEAP_TYPE_DEFAULT), D3D12_HEAP_FLAG_NONE, &desc, D3D12_RESOURCE_STATE_COPY_DEST, None, &mut res)?;
                let res = res.unwrap();
                let srv = D3D12_SHADER_RESOURCE_VIEW_DESC {
                    Format: d.Format,
                    ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                    Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                    Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 { Texture2D: D3D12_TEX2D_SRV { MipLevels: 1, ..Default::default() } },
                };
                self.device.CreateShaderResourceView(&res, Some(&srv), self.srv_cpu(2));
                if let Some(old) = self.post.take() {
                    self.keepalive.push((self.frame, old.0));
                }
                self.post = Some((res, d.Width, d.Height, d.Format.0));
                self.post_psr = false;
            }
            let post = self.post.as_ref().unwrap().0.clone();
            let mut pre = vec![transition(target, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_COPY_SOURCE)];
            if self.post_psr {
                pre.push(transition(&post, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_COPY_DEST));
            }
            cl.ResourceBarrier(&pre);
            cl.CopyResource(&post, target);
            cl.ResourceBarrier(&[
                transition(target, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET),
                transition(&post, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE),
            ]);
            self.post_psr = true;
            Ok(())
        }
    }

    unsafe fn copy_scene(&mut self, cl: &ID3D12GraphicsCommandList, target: &ID3D12Resource) -> windows::core::Result<()> {
        unsafe {
            let d = target.GetDesc();
            if !self.scene.as_ref().is_some_and(|s| s.1 == d.Width && s.2 == d.Height && s.3 == d.Format.0) {
                let desc = D3D12_RESOURCE_DESC {
                    Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
                    Width: d.Width,
                    Height: d.Height,
                    DepthOrArraySize: 1,
                    MipLevels: 1,
                    Format: d.Format,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    ..Default::default()
                };
                let mut res: Option<ID3D12Resource> = None;
                self.device.CreateCommittedResource(
                    &heap_props(D3D12_HEAP_TYPE_DEFAULT),
                    D3D12_HEAP_FLAG_NONE,
                    &desc,
                    D3D12_RESOURCE_STATE_COPY_DEST,
                    None,
                    &mut res,
                )?;
                let res = res.unwrap();
                let srv = D3D12_SHADER_RESOURCE_VIEW_DESC {
                    Format: d.Format,
                    ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                    Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                    Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                        Texture2D: D3D12_TEX2D_SRV { MipLevels: 1, ..Default::default() },
                    },
                };
                self.device.CreateShaderResourceView(&res, Some(&srv), self.srv_cpu(0));
                self.device.CreateShaderResourceView(&res, Some(&srv), self.srv_cpu(5));
                if let Some(old) = self.scene.take() {
                    self.keepalive.push((self.frame, old.0));
                }
                self.scene = Some((res, d.Width, d.Height, d.Format.0));
                self.scene_psr = false;
            }
            let scene = self.scene.as_ref().unwrap().0.clone();
            let mut pre = vec![transition(target, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_COPY_SOURCE)];
            if self.scene_psr {
                pre.push(transition(&scene, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_COPY_DEST));
            }
            cl.ResourceBarrier(&pre);
            cl.CopyResource(&scene, target);
            cl.ResourceBarrier(&[
                transition(target, D3D12_RESOURCE_STATE_COPY_SOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET),
                transition(&scene, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE),
            ]);
            self.scene_psr = true;
            Ok(())
        }
    }

    unsafe fn ensure_depth(&mut self, w: u64, h: u32) -> windows::core::Result<()> {
        if self.depth.as_ref().is_some_and(|d| d.1 == w && d.2 == h) {
            return Ok(());
        }
        let desc = D3D12_RESOURCE_DESC {
            Dimension: D3D12_RESOURCE_DIMENSION_TEXTURE2D,
            Width: w,
            Height: h,
            DepthOrArraySize: 1,
            MipLevels: 1,
            // typeless: D32 depth view for drawing, R32 shader view for the FXAA mask
            Format: DXGI_FORMAT_R32_TYPELESS,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Flags: D3D12_RESOURCE_FLAG_ALLOW_DEPTH_STENCIL,
            ..Default::default()
        };
        let clear = D3D12_CLEAR_VALUE {
            Format: DXGI_FORMAT_D32_FLOAT,
            Anonymous: D3D12_CLEAR_VALUE_0 {
                DepthStencil: D3D12_DEPTH_STENCIL_VALUE { Depth: 1.0, Stencil: 0 },
            },
        };
        let mut res: Option<ID3D12Resource> = None;
        unsafe {
            self.device.CreateCommittedResource(
                &heap_props(D3D12_HEAP_TYPE_DEFAULT),
                D3D12_HEAP_FLAG_NONE,
                &desc,
                D3D12_RESOURCE_STATE_DEPTH_WRITE,
                Some(&clear),
                &mut res,
            )?;
            let res = res.unwrap();
            let dsv = D3D12_DEPTH_STENCIL_VIEW_DESC {
                Format: DXGI_FORMAT_D32_FLOAT,
                ViewDimension: D3D12_DSV_DIMENSION_TEXTURE2D,
                ..Default::default()
            };
            self.device.CreateDepthStencilView(&res, Some(&dsv), self.dsv_heap.GetCPUDescriptorHandleForHeapStart());
            let srv = D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT_R32_FLOAT,
                ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 { Texture2D: D3D12_TEX2D_SRV { MipLevels: 1, ..Default::default() } },
            };
            self.device.CreateShaderResourceView(&res, Some(&srv), self.srv_cpu(4));
            if let Some(old) = self.depth.take() {
                self.keepalive.push((self.frame, old.0));
            }
            self.depth = Some((res, w, h));
        }
        Ok(())
    }

    fn srv_cpu(&self, i: u32) -> D3D12_CPU_DESCRIPTOR_HANDLE {
        let mut h = unsafe { self.srv_heap.GetCPUDescriptorHandleForHeapStart() };
        h.ptr += (i * self.srv_inc) as usize;
        h
    }

    fn srv_gpu(&self, i: u32) -> D3D12_GPU_DESCRIPTOR_HANDLE {
        let mut h = unsafe { self.srv_heap.GetGPUDescriptorHandleForHeapStart() };
        h.ptr += (i * self.srv_inc) as u64;
        h
    }

    /// Create a texture + SRV at `slot`, recording the upload into `cl`.
    unsafe fn texture(&mut self, cl: &ID3D12GraphicsCommandList, img: &Image, slot: u32) -> windows::core::Result<()> {
        let p = unsafe { prepare_texture(&self.device, img)? };
        unsafe { self.finish_texture(cl, p, slot) };
        Ok(())
    }

    /// Render-thread half of a texture upload: record the copies, make it shader-readable, SRV.
    unsafe fn finish_texture(&mut self, cl: &ID3D12GraphicsCommandList, p: Prepared, slot: u32) {
        unsafe {
            for (m, fp) in p.fp.iter().enumerate() {
                let dst = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::transmute_copy(&p.tex),
                    Type: D3D12_TEXTURE_COPY_TYPE_SUBRESOURCE_INDEX,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { SubresourceIndex: m as u32 },
                };
                let src = D3D12_TEXTURE_COPY_LOCATION {
                    pResource: std::mem::transmute_copy(&p.up),
                    Type: D3D12_TEXTURE_COPY_TYPE_PLACED_FOOTPRINT,
                    Anonymous: D3D12_TEXTURE_COPY_LOCATION_0 { PlacedFootprint: *fp },
                };
                cl.CopyTextureRegion(&dst, 0, 0, 0, &src, None);
            }
            cl.ResourceBarrier(&[transition(&p.tex, D3D12_RESOURCE_STATE_COPY_DEST, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE)]);
            let srv = D3D12_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ViewDimension: D3D12_SRV_DIMENSION_TEXTURE2D,
                Shader4ComponentMapping: D3D12_DEFAULT_SHADER_4_COMPONENT_MAPPING,
                Anonymous: D3D12_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D12_TEX2D_SRV { MipLevels: p.fp.len() as u32, ..Default::default() },
                },
            };
            self.device.CreateShaderResourceView(&p.tex, Some(&srv), self.srv_cpu(slot));
            self.keepalive.push((self.frame, p.up));
            self.keepalive.push((usize::MAX, p.tex));
        }
    }

    unsafe fn fallbacks(&mut self, cl: &ID3D12GraphicsCommandList) -> windows::core::Result<[u32; 6]> {
        if let Some(f) = self.fallback {
            return Ok(f);
        }
        let solid = |r, g, b| Image { w: 1, h: 1, mips: vec![vec![r, g, b, 255]] };
        let mut out = [0u32; 6];
        // The last one marks glass (vm.hlsl GLASS_KEY): Doom's glass has no albedo map.
        for (i, img) in [solid(200, 200, 200), solid(128, 128, 255), solid(40, 40, 40), solid(0, 0, 0), solid(110, 110, 110), solid(12, 34, 56)].iter().enumerate() {
            let slot = self.srv_next;
            self.srv_next += 1;
            unsafe { self.texture(cl, img, slot)? };
            out[i] = slot;
        }
        self.fallback = Some(out);
        Ok(out)
    }

    /// Copy an existing SRV into a new slot (for the per-mesh 4-SRV tables).
    fn copy_srv(&mut self, from: u32, to: u32) {
        unsafe {
            self.device.CopyDescriptorsSimple(1, self.srv_cpu(to), self.srv_cpu(from), D3D12_DESCRIPTOR_HEAP_TYPE_CBV_SRV_UAV);
        }
    }

    /// Background texture pipeline: three workers decode and prepare every viewmodel texture (the
    /// folder the renderer is waiting on first); each frame the render thread only records the
    /// copies for what is ready.
    unsafe fn warm_textures(&mut self, cl: &ID3D12GraphicsCommandList) {
        if self.warm.is_none() {
            let root = config::mod_dir().join("doom_vm");
            let all: Vec<PathBuf> = std::fs::read_dir(&root).into_iter().flatten().flatten().flat_map(|d| folder_textures(&d.path())).collect();
            WARM_LEFT.store(all.len(), std::sync::atomic::Ordering::Relaxed);
            let queue = std::sync::Arc::new(Mutex::new(all.clone()));
            let t0 = std::time::Instant::now();
            for _ in 0..3 {
                let queue = queue.clone();
                let dev = SendDevice(self.device.clone());
                std::thread::spawn(move || {
                    let dev = dev;
                    loop {
                        let next = {
                            let Ok(mut q) = queue.lock() else { return };
                            let want = WANT.lock().ok().and_then(|w| w.clone());
                            let i = want
                                .and_then(|w| q.iter().position(|p| p.parent().and_then(|d| d.file_name()).is_some_and(|n| n.to_string_lossy() == w)))
                                .or_else(|| q.len().checked_sub(1));
                            i.map(|i| q.swap_remove(i))
                        };
                        let Some(path) = next else { break };
                        // keep the prepared backlog (upload heaps) bounded
                        while PREPARED.lock().map(|p| p.len() >= 8).unwrap_or(false) {
                            std::thread::sleep(std::time::Duration::from_millis(4));
                        }
                        let img = DECODED.lock().ok().and_then(|mut g| g.as_mut().and_then(|m| m.remove(&path))).or_else(|| decode_png(&path).map(std::sync::Arc::new));
                        if let Some(img) = img {
                            if let Ok(p) = unsafe { prepare_texture(&dev.0, &img) } {
                                if let Ok(mut g) = PREPARED.lock() {
                                    g.push((path, p));
                                }
                            }
                        }
                        let left = WARM_LEFT.fetch_sub(1, std::sync::atomic::Ordering::Relaxed) - 1;
                        if left == 0 {
                            log::info!("viewmodel textures prepared in {:?}", t0.elapsed());
                        }
                    }
                });
            }
            self.warm = Some(Vec::new());
        }
        let ready: Vec<(PathBuf, Prepared)> = PREPARED.lock().map(|mut g| std::mem::take(&mut *g)).unwrap_or_default();
        for (path, p) in ready {
            if self.texture_cache.contains_key(&path) || self.srv_next + 64 > SRV_CAPACITY {
                continue;
            }
            let slot = self.srv_next;
            self.srv_next += 1;
            unsafe { self.finish_texture(cl, p, slot) };
            self.texture_cache.insert(path, slot);
        }
    }

    unsafe fn load_model(&mut self, cl: &ID3D12GraphicsCommandList, folder: &str) -> Option<()> {
        if self.models.contains_key(folder) {
            return Some(());
        }
        let dir = config::mod_dir().join("doom_vm").join(folder);
        // Textures still being prepared in the background: draw this weapon a moment later rather
        // than uploading them all on this frame (the hitch when a gun appeared). After 6 s the
        // old synchronous path takes over.
        if WARM_LEFT.load(std::sync::atomic::Ordering::Relaxed) > 0 {
            let cache = &self.texture_cache;
            if folder_textures(&dir).iter().any(|p| !cache.contains_key(p)) {
                let since = *self.waiting.entry(folder.to_string()).or_insert_with(std::time::Instant::now);
                if since.elapsed().as_secs_f32() < 6.0 {
                    if let Ok(mut w) = WANT.lock() {
                        *w = Some(folder.to_string());
                    }
                    return None;
                }
            }
        }
        let Some(data) = parse_model(&dir.join("model.bin")) else {
            self.models.insert(folder.to_string(), None);
            return None;
        };
        let fb = unsafe { self.fallbacks(cl).ok()? };
        let t0 = std::time::Instant::now();
        let mut meshes = Vec::new();
        for m in &data.meshes {
            unsafe {
                let vb = upload_buffer(&self.device, &m.vertices).ok()?;
                let ib_bytes: Vec<u8> = m.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
                let ib = upload_buffer(&self.device, &ib_bytes).ok()?;
                let vbv = D3D12_VERTEX_BUFFER_VIEW {
                    BufferLocation: vb.GetGPUVirtualAddress(),
                    SizeInBytes: m.vertices.len() as u32,
                    StrideInBytes: 56,
                };
                let ibv = D3D12_INDEX_BUFFER_VIEW {
                    BufferLocation: ib.GetGPUVirtualAddress(),
                    SizeInBytes: ib_bytes.len() as u32,
                    Format: DXGI_FORMAT_R32_UINT,
                };
                // 5 consecutive descriptors for this mesh.
                if self.srv_next + 5 > SRV_CAPACITY {
                    log::warn!("viewmodel: out of SRV slots");
                    return None;
                }
                let first = self.srv_next;
                self.srv_next += 5;
                let mut found = [false; 5];
                for (k, suffix) in ["", "_n", "_s", "_e", "_g"].iter().enumerate() {
                    let mut path = dir.join(format!("{}{}.png", m.material, suffix));
                    // Characters store smoothness as _pm instead of _g.
                    if k == 4 && !path.exists() {
                        path = dir.join(format!("{}_pm.png", m.material));
                    }
                    // Doom's glass (glass_refraction_gun) ships a transparency mask where the
                    // albedo would be: never paint it on, use the glass marker instead.
                    let glass = m.material.ends_with("_glass");
                    let cached = self.texture_cache.get(&path).copied();
                    let src_slot = if k == 0 && glass { None } else { match cached {
                        Some(s) => Some(s),
                        None => match load_png(&path) {
                            Some(img) => {
                                let s = self.srv_next;
                                self.srv_next += 1;
                                self.texture(cl, &img, s).ok()?;
                                self.texture_cache.insert(path.clone(), s);
                                // Uploaded: free the CPU copy.
                                if let Some(m) = DECODED.lock().ok().as_mut().and_then(|g| g.as_mut()) {
                                    m.remove(&path);
                                }
                                Some(s)
                            }
                            None => None,
                        },
                    } };
                    found[k] = src_slot.is_some();
                    let fallback = if k == 0 && glass { fb[5] } else { fb[k] };
                    self.copy_srv(src_slot.unwrap_or(fallback), first + k as u32);
                }
                meshes.push(GpuMesh {
                    part: m.part,
                    _vb: vb,
                    _ib: ib,
                    vbv,
                    ibv,
                    count: m.indices.len() as u32,
                    srv_first: first,
                    has_normal: found[1],
                    has_spec: found[2],
                    glow: m.material.starts_with("crucible_blade"),
                });
            }
        }
        log::info!(
            "viewmodel '{folder}' loaded in {:?}: {} meshes, clips {:?}",
            t0.elapsed(),
            meshes.len(),
            data.clips.keys().collect::<Vec<_>>()
        );
        // Doom's own muzzle tag (tools/vm/convert_weapon.py -> info.json); vertex guess as fallback.
        let tag = std::fs::read_to_string(dir.join("info.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|j| {
                let m = j.get("muzzle")?;
                let v3 = |k: &str| -> Option<[f32; 3]> {
                    let a = m.get(k)?.as_array()?;
                    Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32])
                };
                Some((m.get("bone")?.as_u64()? as u16, v3("pos")?, v3("dir")?))
            });
        let spins = std::fs::read_to_string(dir.join("info.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|j| j.get("spin").and_then(|v| v.as_array()).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|g| {
                let v3 = |k: &str| -> Option<glam::Vec3> {
                    let a = g.get(k)?.as_array()?;
                    Some(glam::Vec3::new(a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32))
                };
                let bones = g.get("bones")?.as_array()?.iter().filter_map(|b| b.as_u64().map(|b| b as usize)).collect();
                Some((g.get("group")?.as_u64()? as usize, bones, v3("pivot")?, v3("axis")?.normalize_or(glam::Vec3::X)))
            })
            .collect::<Vec<_>>();
        let info_json = std::fs::read_to_string(dir.join("info.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
        let mesh_modes: Vec<u8> = info_json
            .as_ref()
            .and_then(|j| j.get("mesh_modes")?.as_array().cloned())
            .map(|a| a.iter().map(|v| v.as_u64().unwrap_or(0) as u8).collect())
            .unwrap_or_default();
        let tags: HashMap<String, (u16, [f32; 3])> = info_json
            .as_ref()
            .and_then(|j| j.get("tags")?.as_object().cloned())
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| {
                        let p = v.get("pos")?.as_array()?;
                        Some((k.clone(), (v.get("bone")?.as_u64()? as u16, [p.first()?.as_f64()? as f32, p.get(1)?.as_f64()? as f32, p.get(2)?.as_f64()? as f32])))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let (muzzle, muzzle_dir) = match tag {
            Some((bone, pos, dir)) => (Some((pos, [bone, 0, 0, 0], [1.0, 0.0, 0.0, 0.0])), Some(dir)),
            None => (find_muzzle(&data), None),
        };
        self.models.insert(folder.to_string(), Some(GpuModel { samples: gun_samples(&data, &mesh_modes), data, meshes, muzzle, muzzle_dir, spins, mesh_modes, tags }));
        if let Ok(mut l) = LOADED.lock() {
            l.push(folder.to_string());
        }
        Some(())
    }
}

// ------------------------------------------------------------------------- per-frame state

/// What the game thread wants drawn this frame.
#[derive(Clone, Debug)]
pub struct Pose {
    pub folder: &'static str,
    pub clip: String,
    pub time: f32,
    pub looping: bool,
    /// View-space offset (m) and extra pitch (rad): bob, sway, swap.
    pub offset: [f32; 3],
    pub pitch: f32,
    pub yaw: f32,
    pub flash: f32,
    pub visible: bool,
    /// Doom's per-weapon handsFovScale: the weapon view FOV is vm_fov times this (Chaingun 0.654).
    pub fov_scale: f32,
    /// Which mode's parts to show (mesh_modes): 1 normal, 2 Chaingun turret.
    pub mode: u8,
    /// Muzzle flash at this md6def tag instead of the model's default muzzle (turret barrels).
    pub muzzle_tag: Option<&'static str>,
    /// Glows anchored to md6def tags: (tag, texture ramp, size m, alpha) - the rocket in the chamber.
    pub glows: Vec<(&'static str, &'static str, f32, f32)>,
    /// Additive layer on top of the playing clip: (clip, time). The renderer applies
    /// clip x inverse(clip + "_base") per bone, so the layer rides on whatever plays (idle).
    pub overlay: Option<(&'static str, f32)>,
    /// Barrel spin angles (rad) per spin group (Chaingun: 0 = rotary cluster, 1 = turret barrels).
    pub spin: [f32; 2],
    /// Weapon sway tilt (rad, + = top of the gun to the right), about the gun's own centre.
    pub roll: f32,
    /// Walking bob of the tip: (yaw, pitch) in rad, + = muzzle right / up, pivoting near the hands.
    pub tip: [f32; 2],
}

static POSE: Mutex<Option<Pose>> = Mutex::new(None);

/// The gun was hidden (death, loading, menus): the next pose starts clean - no crossfade from
/// the last pose drawn before (after a respawn the gun flashed up, cut, then drew - user).
static BLEND_RESET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A frozen viewmodel pose (bridge `freeze`): drawn instead of the game's until unfrozen.
static FROZEN: Mutex<Option<Pose>> = Mutex::new(None);

/// Freeze what's on screen now (`clip` / `time` given: that clip of the current gun at that time,
/// not looping), or unfreeze (`off`). Returns what's frozen.
pub fn freeze(off: bool, folder: Option<&'static str>, clip: Option<&str>, time: Option<f32>) -> String {
    let mut f = FROZEN.lock().unwrap_or_else(|e| e.into_inner());
    if off {
        *f = None;
        return "unfrozen".into();
    }
    let base = f.clone().or_else(|| POSE.lock().ok().and_then(|p| p.clone()));
    let Some(mut p) = base else { return "nothing on screen".into() };
    if let Some(fd) = folder {
        p.folder = fd;
        p.visible = true;
    }
    if let Some(c) = clip {
        p.clip = c.to_string();
    }
    if let Some(t) = time {
        p.time = t;
        p.looping = false;
    }
    let out = format!("frozen: {} {} at {:.3} s", p.folder, p.clip, p.time);
    *POSE.lock().unwrap_or_else(|e| e.into_inner()) = Some(p.clone());
    *f = Some(p);
    out
}

pub fn set_pose(p: Option<Pose>) {
    if FROZEN.lock().is_ok_and(|f| f.is_some()) {
        return;
    }
    if p.is_none() {
        BLEND_RESET.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    *POSE.lock().unwrap_or_else(|e| e.into_inner()) = p;
}

pub fn has_model(folder: &str) -> bool {
    config::mod_dir().join("doom_vm").join(folder).join("model.bin").exists()
}

/// Length in seconds of a clip if the model is loaded (game thread uses this to chain clips).
/// The weapon's model is built and drawable (its textures may still be loading in the background).
/// (Own small list: the game thread asks every frame and must never wait on the render lock.)
pub fn is_loaded(folder: &str) -> bool {
    LOADED.lock().is_ok_and(|l| l.iter().any(|f| f == folder))
}

static LOADED: Mutex<Vec<String>> = Mutex::new(Vec::new());

pub fn clip_len(folder: &str, clip: &str) -> Option<f32> {
    let g = GPU.lock().ok()?;
    let m = g.as_ref()?.models.get(folder)?.as_ref()?;
    let c = m.data.clips.get(clip)?;
    Some(c.frames as f32 / c.fps.max(1.0))
}

/// MSAA on unless vm_msaa is 0/1 (checked per frame, so it can be flipped live in the toml).
fn cfg_msaa_on() -> bool {
    config::get_cached().vm_msaa > 1
}

pub fn install() {
    let _ = hudhook::DX12_PRE_RENDER.set(pre_render);
}

fn lerp_pal(a: &[[f32; 12]], b: &[[f32; 12]], t: f32, out: &mut [u8]) {
    for (i, (ma, mb)) in a.iter().zip(b).enumerate() {
        for k in 0..12 {
            let v = ma[k] + (mb[k] - ma[k]) * t;
            out[i * 48 + k * 4..i * 48 + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
}

/// Pre-multiply every palette matrix by the view adjust (offset + pitch/yaw) in place.
fn adjust(pal: &mut [u8], n: usize, adj: &glam::Mat4) {
    for i in 0..n {
        let r = |k: usize| f32::from_le_bytes(pal[i * 48 + k * 4..i * 48 + k * 4 + 4].try_into().unwrap());
        let m = glam::Mat4::from_cols_array(&[
            r(0), r(4), r(8), 0.0, r(1), r(5), r(9), 0.0, r(2), r(6), r(10), 0.0, r(3), r(7), r(11), 1.0,
        ]);
        let o = *adj * m;
        let c = o.to_cols_array();
        let rows = [c[0], c[4], c[8], c[12], c[1], c[5], c[9], c[13], c[2], c[6], c[10], c[14]];
        for (k, v) in rows.iter().enumerate() {
            pal[i * 48 + k * 4..i * 48 + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
    }
}

fn pre_render(
    device: &ID3D12Device,
    cl: &ID3D12GraphicsCommandList,
    target: &ID3D12Resource,
    rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
) {
    let Some(pose) = POSE.lock().ok().and_then(|p| p.clone()) else { return };
    if !pose.visible {
        return;
    }
    let mut guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        match unsafe { Gpu::new(device) } {
            Ok(g) => *guard = Some(g),
            Err(e) => {
                log::error!("viewmodel GPU init failed: {e:?}");
                return;
            }
        }
    }
    let gpu = guard.as_mut().unwrap();
    if BLEND_RESET.swap(false, std::sync::atomic::Ordering::Relaxed) {
        gpu.blend = Blend::default();
    }
    if let Err(e) = unsafe { draw(gpu, cl, target, rtv, &pose) } {
        log::error!("viewmodel draw failed: {e:?}");
    }
}

unsafe fn draw(
    gpu: &mut Gpu,
    cl: &ID3D12GraphicsCommandList,
    target: &ID3D12Resource,
    rtv: D3D12_CPU_DESCRIPTOR_HANDLE,
    pose: &Pose,
) -> windows::core::Result<()> {
    unsafe {
        gpu.frame += 1;
        let frame = gpu.frame;
        gpu.keepalive.retain(|(f, _)| *f == usize::MAX || frame - *f < 8);

        let desc = target.GetDesc();
        let (w, h) = (desc.Width, desc.Height);
        // MSAA for the gun pass (cfg.vm_msaa); 1 = the original single-sample path.
        let samples = if cfg_msaa_on() { gpu.msaa_samples(desc.Format, config::get_cached().vm_msaa) } else { 1 };
        if samples > 1 {
            gpu.ensure_msaa(w, h, desc.Format, samples)?;
        }
        let pso = gpu.pso(desc.Format, samples, 0)?;
        let pso_arms = gpu.pso(desc.Format, samples, 1)?;
        let pso_front = gpu.pso(desc.Format, samples, 2)?;
        let pso_glow = gpu.pso(desc.Format, samples, 3)?;
        // (cached; building them on the first shot froze the overlay for a moment - user)
        let _ = gpu.fx_pso(desc.Format, true, samples)?;
        let _ = gpu.fx_pso(desc.Format, false, samples)?;
        gpu.ensure_depth(w, h)?;
        gpu.warm_textures(cl);
        if gpu.load_model(cl, pose.folder).is_none() {
            return Ok(());
        }
        // Melee / chainsaw models are needed instantly on a key press: build them early.
        if let Some(f) = ["fists", CHAINSAW_FOLDER, CRUCIBLE_FOLDER].into_iter().find(|f| !gpu.models.contains_key(*f)) {
            let _ = gpu.load_model(cl, f);
        }
        let cfg = config::get_cached();
        let model = gpu.models.get(pose.folder).unwrap().as_ref().unwrap();
        let data = &model.data;
        let Some(clip) = data.clips.get(&pose.clip).or_else(|| data.clips.get("idle")) else {
            return Ok(());
        };

        // Sample the clip (lerp between baked frames).
        let f = pose.time * clip.fps;
        let loop_frames = clip.loop_frames;
        let f = if pose.looping { f.rem_euclid(loop_frames as f32) } else { f.clamp(0.0, (clip.frames - 1) as f32) };
        let f0 = f.floor() as usize % clip.frames;
        let f1 = if pose.looping { (f0 + 1) % loop_frames } else { (f0 + 1).min(clip.frames - 1) };
        let t = f.fract();

        let slot = frame % RING;
        let base = gpu.ring_ptr.add(slot * SLOT_BYTES);
        let arms_off = 256;
        let gun_off = 256 + ((BONES_BYTES + 255) & !255);
        let pal_a = std::slice::from_raw_parts_mut(base.add(arms_off), BONES_BYTES);
        let pal_g = std::slice::from_raw_parts_mut(base.add(gun_off), BONES_BYTES);
        let (na, ng) = (data.arms_bones.min(MAX_BONES), data.gun_bones.min(MAX_BONES));
        lerp_pal(&clip.arms[f0 * data.arms_bones..f0 * data.arms_bones + na], &clip.arms[f1 * data.arms_bones..f1 * data.arms_bones + na], t, pal_a);
        lerp_pal(&clip.gun[f0 * data.gun_bones..f0 * data.gun_bones + ng], &clip.gun[f1 * data.gun_bones..f1 * data.gun_bones + ng], t, pal_g);
        // Crossfade from the last drawn pose when the clip changes or restarts (same model only).
        {
            let b = &mut gpu.blend;
            let key = (pose.folder.to_string(), pose.clip.clone());
            let restarted = key == b.key && pose.time + 0.05 < b.last_time;
            // Heavy Cannon shots restart the fire clip every round: only a very short crossfade
            // (the normal one softened the front piece's snap, none made the rhythm too fast).
            // BFG: a longer blend from the shot back to idle.
            let hc_refire = restarted && pose.folder == "heavy_cannon" && pose.clip.ends_with("fire");
            let new_dur = if hc_refire {
                0.09
            } else if b.key.0 == "bfg" && b.key.1 == "fire" && key.1 != "fire" {
                0.3
            } else {
                BLEND_SECS
            };
            if key != b.key || restarted {
                if key.0 == b.key.0 && b.last_a.len() == na * 48 && b.last_g.len() == ng * 48 {
                    b.from_a = b.last_a.clone();
                    b.from_g = b.last_g.clone();
                    b.start = Some(std::time::Instant::now());
                    b.dur = new_dur;
                } else {
                    b.start = None;
                }
                b.key = key;
            }
            b.last_time = pose.time;
            if let Some(st) = b.start {
                let k = st.elapsed().as_secs_f32() / b.dur.max(0.01);
                if k >= 1.0 {
                    b.start = None;
                } else {
                    let k = k * k * (3.0 - 2.0 * k);
                    mix_pal(&b.from_a, &mut pal_a[..na * 48], k);
                    mix_pal(&b.from_g, &mut pal_g[..ng * 48], k);
                }
            }
            b.last_a.clear();
            b.last_a.extend_from_slice(&pal_a[..na * 48]);
            b.last_g.clear();
            b.last_g.extend_from_slice(&pal_g[..ng * 48]);
        }
        // Melee: both arms pushed out sideways, each the same distance (melee_arm_push cm, live):
        // a right punch showed the open end of the arm at the shoulder (user).
        if pose.folder == "fists" && cfg.melee_arm_push != 0.0 {
            let push = cfg.melee_arm_push / 100.0;
            for (i, &side) in data.arm_side.iter().enumerate().take(na) {
                if side != 0 {
                    let o = i * 48 + 3 * 4; // row 0, translation x (view space: + = right)
                    let x = f32::from_le_bytes(pal_a[o..o + 4].try_into().unwrap()) + push * side as f32;
                    pal_a[o..o + 4].copy_from_slice(&x.to_le_bytes());
                }
            }
        }
        // Additive overlay (Super Shotgun no-target hook): D = clip x inverse(base) per bone, in
        // view space, applied on top of the current pose.
        if let Some((oc, ot)) = pose.overlay {
            if let (Some(o), Some(ob)) = (data.clips.get(oc), data.clips.get(&format!("{oc}_base"))) {
                let f = (ot * o.fps).clamp(0.0, (o.frames - 1) as f32);
                let (f0, k) = (f.floor() as usize, f.fract());
                let f1 = (f0 + 1).min(o.frames - 1);
                let m4 = |r: &[f32; 12]| glam::Mat4::from_cols_array(&[r[0], r[4], r[8], 0.0, r[1], r[5], r[9], 0.0, r[2], r[6], r[10], 0.0, r[3], r[7], r[11], 1.0]);
                let lerp = |a: &[f32; 12], b: &[f32; 12]| -> [f32; 12] { std::array::from_fn(|j| a[j] + (b[j] - a[j]) * k) };
                for (pal, n, nb, oa, ba) in [(&mut *pal_a, na, data.arms_bones, &o.arms, &ob.arms), (&mut *pal_g, ng, data.gun_bones, &o.gun, &ob.gun)] {
                    for i in 0..n {
                        let mo = m4(&lerp(&oa[f0 * nb + i], &oa[f1 * nb + i]));
                        let mb = m4(&lerp(&ba[f0 * nb + i], &ba[f1 * nb + i]));
                        let o48 = i * 48;
                        let r = |j: usize| f32::from_le_bytes(pal[o48 + j * 4..o48 + j * 4 + 4].try_into().unwrap());
                        let cur = glam::Mat4::from_cols_array(&[r(0), r(4), r(8), 0.0, r(1), r(5), r(9), 0.0, r(2), r(6), r(10), 0.0, r(3), r(7), r(11), 1.0]);
                        let c = (mo * mb.inverse() * cur).to_cols_array();
                        for (j, v) in [c[0], c[4], c[8], c[12], c[1], c[5], c[9], c[13], c[2], c[6], c[10], c[14]].iter().enumerate() {
                            pal[o48 + j * 4..o48 + j * 4 + 4].copy_from_slice(&v.to_le_bytes());
                        }
                    }
                }
            }
        }
        // Spinning barrels: turn each group's bones about its axis in bind space (pal * M).
        for (group, bones, pivot, axis) in &model.spins {
            let ang = pose.spin.get(*group).copied().unwrap_or(0.0);
            if ang == 0.0 {
                continue;
            }
            let m = glam::Mat4::from_translation(*pivot) * glam::Mat4::from_axis_angle(*axis, ang) * glam::Mat4::from_translation(-*pivot);
            for &b in bones.iter().filter(|b| **b < ng) {
                let o = b * 48;
                let r = |k: usize| f32::from_le_bytes(pal_g[o + k * 4..o + k * 4 + 4].try_into().unwrap());
                let p = glam::Mat4::from_cols_array(&[
                    r(0), r(4), r(8), 0.0, r(1), r(5), r(9), 0.0, r(2), r(6), r(10), 0.0, r(3), r(7), r(11), 1.0,
                ]) * m;
                let c = p.to_cols_array();
                for (k, v) in [c[0], c[4], c[8], c[12], c[1], c[5], c[9], c[13], c[2], c[6], c[10], c[14]].iter().enumerate() {
                    pal_g[o + k * 4..o + k * 4 + 4].copy_from_slice(&v.to_le_bytes());
                }
            }
        }
        let adj = glam::Mat4::from_translation(glam::Vec3::from(pose.offset)
            + glam::Vec3::new(cfg.vm_offset[0], cfg.vm_offset[1], cfg.vm_offset[2]))
            * glam::Mat4::from_rotation_y(pose.yaw)
            * glam::Mat4::from_rotation_x(-(pose.pitch + cfg.vm_pitch.to_radians()));
        adjust(pal_a, na, &adj);
        adjust(pal_g, ng, &adj);
        if pose.tip != [0.0; 2] {
            // pivot = the stock (rearmost point of the gun): it stays put, the muzzle swings
            let rd = |b: usize| -> [f32; 12] {
                if b >= ng {
                    return [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0];
                }
                let mut m = [0.0f32; 12];
                for k in 0..12 {
                    let o = b * 48 + k * 4;
                    m[k] = f32::from_le_bytes(pal_g[o..o + 4].try_into().unwrap());
                }
                m
            };
            // pivot = the nearest bit of the gun still on screen (its visible back end), or the
            // camera for Doom's bob with doom_bob[3] = 0
            let at_camera = cfg.bob_mode == 1 && cfg.doom_bob[3] < 0.5;
            let ys = 1.0 / (cfg.vm_fov.to_radians() * pose.fov_scale.clamp(0.2, 1.5) * 0.5).tan();
            let xs = ys / (w as f32 / h as f32);
            let c = model
                .samples
                .iter()
                .map(|&(p, bi, bw)| skin(p, bi, bw, rd))
                .filter(|v| v.z > 0.03 && (v.x * xs / v.z).abs() < 1.0 && (v.y * ys / v.z).abs() < 1.0)
                .min_by(|a, b| a.z.total_cmp(&b.z))
                .unwrap_or_else(|| bone_centre(pal_g, ng));
            let c = if at_camera { glam::Vec3::ZERO } else { c };
            let r = glam::Mat4::from_translation(c)
                * glam::Mat4::from_rotation_y(pose.tip[0])
                * glam::Mat4::from_rotation_x(-pose.tip[1])
                * glam::Mat4::from_translation(-c);
            adjust(pal_a, na, &r);
            adjust(pal_g, ng, &r);
        }
        if pose.roll != 0.0 {
            let c = bone_centre(pal_g, ng);
            let r = glam::Mat4::from_translation(c) * glam::Mat4::from_rotation_z(-pose.roll) * glam::Mat4::from_translation(-c);
            adjust(pal_a, na, &r);
            adjust(pal_g, ng, &r);
        }
        if let Some(ins) = inspect_step(pal_g, ng) {
            adjust(pal_a, na, &ins);
            adjust(pal_g, ng, &ins);
        }
        // Gun/arms marker for the pixel shader (palette slot 95 is never a real bone).
        pal_g[95 * 48..95 * 48 + 4].copy_from_slice(&7777.0f32.to_le_bytes());
        pal_a[95 * 48..95 * 48 + 4].copy_from_slice(&0.0f32.to_le_bytes());

        // Frame constants.
        let aspect = w as f32 / h as f32;
        // Weapon-view FOV for this frame (the turret's zoom narrows it); world visuals use the same.
        let vm_tan = (cfg.vm_fov.to_radians() * pose.fov_scale.clamp(0.2, 1.5) * 0.5).tan();
        let ys = 1.0 / vm_tan;
        let xs = ys / aspect;
        // Track the barrel tip through the animation (muzzle flash light + HUD flash sprites).
        let mut flash_at = glam::Vec3::new(0.15, -0.12, 0.7);
        let mut flash_axis = glam::Vec3::Z;
        let tag_muzzle = pose.muzzle_tag.and_then(|t| model.tags.get(t)).map(|&(b, p)| (p, [b, 0, 0, 0], [1.0f32, 0.0, 0.0, 0.0]));
        let muzzle_dir = match tag_muzzle {
            Some((p, _, _)) => Some([p[0] + 10.0, p[1], p[2]]),
            None => model.muzzle_dir,
        };
        if let Some((p, bi, bw)) = tag_muzzle.or(model.muzzle) {
            let rd = |b: usize| -> [f32; 12] {
                let mut m = [0.0f32; 12];
                for k in 0..12 {
                    let o = b * 48 + k * 4;
                    m[k] = f32::from_le_bytes(pal_g[o..o + 4].try_into().unwrap());
                }
                m
            };
            let v = skin(p, bi, bw, |b| if b < ng { rd(b) } else { [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0] });
            if let Some(d) = muzzle_dir {
                let vd = skin(d, bi, bw, |b| if b < ng { rd(b) } else { [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0] });
                flash_axis = (vd - v).normalize_or(glam::Vec3::Z);
            }
            if v.z > 0.05 {
                flash_at = v;
                let sx = (v.x * xs / v.z + 1.0) * 0.5 * w as f32;
                let sy = (1.0 - v.y * ys / v.z) * 0.5 * h as f32;
                if let Ok(mut g) = MUZZLE.lock() {
                    *g = Some(([sx, sy], v));
                }
                if let Ok(mut g) = MUZZLE_NDC.lock() {
                    *g = Some([v.x * xs / v.z, v.y * ys / v.z, aspect]);
                }
            }
        }
        // vm_debug 99: dump this frame's gun palette + projection (offline model checks).
        if cfg.vm_debug == 99 && !VM_DUMPED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            let pal: Vec<f32> = (0..ng * 12).map(|k| f32::from_le_bytes(pal_g[k * 4..k * 4 + 4].try_into().unwrap())).collect();
            let j = serde_json::json!({ "weapon": pose.folder, "w": w, "h": h, "xs": xs, "ys": ys, "ng": ng, "pal_g": pal });
            let _ = std::fs::write(crate::config::mod_dir().join("vm_dump.json"), j.to_string());
        } else if cfg.vm_debug != 99 {
            VM_DUMPED.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        let (zn, zf) = (0.01f32, 20.0f32);
        let proj = [
            xs, 0.0, 0.0, 0.0,
            0.0, ys, 0.0, 0.0,
            0.0, 0.0, zf / (zf - zn), -zn * zf / (zf - zn),
            0.0, 0.0, 1.0, 0.0,
        ];
        let l = glam::Vec3::new(-0.35, 0.75, -0.55).normalize();
        let cb: [f32; 36] = [
            proj[0], proj[1], proj[2], proj[3], proj[4], proj[5], proj[6], proj[7],
            proj[8], proj[9], proj[10], proj[11], proj[12], proj[13], proj[14], proj[15],
            l.x, l.y, l.z, 0.0,
            cfg.vm_light * 1.0, cfg.vm_light * 0.94, cfg.vm_light * 0.85, cfg.vm_env,
            0.18, 0.2, 0.24, cfg.vm_debug as f32,
            cfg.vm_ambient, cfg.vm_ambient * 1.02, cfg.vm_ambient * 1.08,
            2.0 * (1.0 + 5.0 * f32::from_bits(GLOW.load(std::sync::atomic::Ordering::Relaxed))),
            pose.flash * 3.0, 2.2, 1.0, 1.0,
        ];
        let cb_bytes: Vec<u8> = cb.iter().flat_map(|v| v.to_le_bytes()).chain([flash_at.x, flash_at.y, flash_at.z, f32::from_bits(HEAT.load(std::sync::atomic::Ordering::Relaxed))].iter().flat_map(|v| v.to_le_bytes())).collect();
        std::ptr::copy_nonoverlapping(cb_bytes.as_ptr(), base, cb_bytes.len());
        let gpu_base = gpu.ring.GetGPUVirtualAddress() + (slot * SLOT_BYTES) as u64;

        // Record: grab the game's frame for scene-matched lighting, then draw.
        gpu.copy_scene(cl, target)?;
        gpu.update_probe(cl)?;
        let (draw_rtv, dsv) = if samples > 1 {
            let m = gpu.msaa.as_ref().unwrap();
            cl.ResourceBarrier(&[transition(&m.rt, D3D12_RESOURCE_STATE_RESOLVE_SOURCE, D3D12_RESOURCE_STATE_RENDER_TARGET)]);
            let (rh, dh) = gpu.msaa_heaps.as_ref().unwrap();
            (rh.GetCPUDescriptorHandleForHeapStart(), dh.GetCPUDescriptorHandleForHeapStart())
        } else {
            (rtv, gpu.dsv_heap.GetCPUDescriptorHandleForHeapStart())
        };
        cl.OMSetRenderTargets(1, Some(&draw_rtv), false, Some(&dsv));
        cl.ClearDepthStencilView(dsv, D3D12_CLEAR_FLAG_DEPTH, 1.0, 0, None);
        cl.RSSetViewports(&[D3D12_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: w as f32, Height: h as f32, MinDepth: 0.0, MaxDepth: 1.0 }]);
        cl.RSSetScissorRects(&[windows::Win32::Foundation::RECT { left: 0, top: 0, right: w as i32, bottom: h as i32 }]);
        cl.SetGraphicsRootSignature(&gpu.root);
        cl.SetPipelineState(&pso);
        cl.SetDescriptorHeaps(&[Some(gpu.srv_heap.clone())]);
        cl.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        cl.SetGraphicsRootConstantBufferView(0, gpu_base);
        cl.SetGraphicsRootDescriptorTable(3, gpu.srv_gpu(0));
        if samples > 1 {
            // The game's frame first, so everything we draw blends over it exactly as before.
            let blit = gpu.blit_pso(desc.Format, samples)?;
            cl.SetPipelineState(&blit);
            cl.DrawInstanced(3, 1, 0, 0);
            cl.SetPipelineState(&pso);
        }
        // Muzzle flash (at Doom's muzzle tag, along the barrel) + world-space Doom weapon visuals,
        // built now as view-space quads grouped by texture; drawn after the gun.
        let mut by_tex: Vec<(String, Vec<u8>)> = Vec::new();
        let mut add = |tex: String, f: &mut dyn FnMut(&mut Vec<u8>)| {
            let i = match by_tex.iter().position(|(t, _)| *t == tex) {
                Some(i) => i,
                None => {
                    by_tex.push((tex, Vec::new()));
                    by_tex.len() - 1
                }
            };
            f(&mut by_tex[i].1);
        };
        let flash = FLASH.lock().ok().and_then(|f| *f);
        let has_muzzle = gpu.models.get(pose.folder).and_then(|m| m.as_ref()).is_some_and(|m| m.muzzle.is_some());
        if let Some((t0, slot, seed)) = flash.filter(|_| has_muzzle && pose.folder != "fists" && pose.folder != CHAINSAW_FOLDER && pose.folder != CRUCIBLE_FOLDER) {
            let look = &FLASH_LOOKS[slot.min(7)];
            let age = if FLASH_HOLD.load(std::sync::atomic::Ordering::Relaxed) { 0.0 } else { t0.elapsed().as_secs_f32() };
            if age < look.dur {
                let a = (1.0 - age / look.dur).sqrt() * 1.5;
                let frame = (seed >> 3) % 4;
                let rot = ((seed >> 7) % 628) as f32 / 100.0;
                let axis = flash_axis;
                let muzzle = flash_at + axis * 0.01 + glam::Vec3::from(look.nudge);
                let to_cam = (-muzzle).normalize();
                let fu = (frame % 2) as f32 * 0.5;
                let fv = (frame / 2) as f32 * 0.5;
                // Side flame along the barrel, billboarded around its axis.
                let side = axis.cross(to_cam).normalize_or_zero() * look.side * 0.22;
                let p0 = muzzle;
                let p1 = muzzle + axis * look.side;
                add("F:".to_string() + &format!("{}_side", look.ramp), &mut |buf| put_quad(buf, [p0 - side, p1 - side, p1 + side, p0 + side], [fu, fv, fu + 0.5, fv + 0.5], a));
                // Front burst facing the camera, randomly rotated, just ahead of the bore.
                let quad = |size: f32, r: f32| {
                    let (sn, cs) = r.sin_cos();
                    let rx = glam::Vec3::new(cs, sn, 0.0) * size * 0.5;
                    let ry = glam::Vec3::new(-sn, cs, 0.0) * size * 0.5;
                    let ctr = muzzle + axis * size * 0.55;
                    [ctr - rx - ry, ctr + rx - ry, ctr + rx + ry, ctr - rx + ry]
                };
                add("F:".to_string() + &format!("{}_front", look.ramp), &mut |buf| put_quad(buf, quad(look.front, rot), [fu, fv, fu + 0.5, fv + 0.5], a));
                if look.star > 0.0 {
                    add("F:".to_string() + &format!("{}_star", look.ramp), &mut |buf| put_quad(buf, quad(look.star, rot * 1.7 + 1.0), [0.0, 0.0, 1.0, 1.0], a * 0.8));
                }
            }
        }
        // BFG wind-up: a green core growing at the muzzle until the shot leaves.
        let charge = CHARGE.lock().ok().and_then(|c| *c);
        if let Some((t0, len)) = charge.filter(|_| pose.folder == "bfg") {
            let k = (t0.elapsed().as_secs_f32() / len.max(0.05)).min(1.0);
            let at = flash_at + flash_axis * 0.06;
            let t = t0.elapsed().as_secs_f32();
            let quad = |size: f32, r: f32| {
                let (sn, cs) = r.sin_cos();
                let rx = glam::Vec3::new(cs, sn, 0.0) * size * 0.5;
                let ry = glam::Vec3::new(-sn, cs, 0.0) * size * 0.5;
                [at - rx - ry, at + rx - ry, at + rx + ry, at - rx + ry]
            };
            let flick = 0.8 + 0.2 * (t * 41.0).sin();
            add("F:green_star".to_string(), &mut |buf| put_quad(buf, quad((0.12 + 0.68 * k) * flick, t * 5.0), [0.0, 0.0, 1.0, 1.0], 0.6 + 0.8 * k));
            add("F:green_front".to_string(), &mut |buf| put_quad(buf, quad(0.08 + 0.36 * k, -t * 7.0), [0.0, 0.0, 0.5, 0.5], 0.8 + 0.8 * k));
            add("F:green_front".to_string(), &mut |buf| put_quad(buf, quad((0.06 + 0.26 * k) * flick, t * 11.0 + 1.0), [0.5, 0.5, 1.0, 1.0], 1.2));
        }
        // Tag-anchored glows (skinned like a vertex, depth-tested so the gun hides them).
        for (tag, ramp, size, alpha) in &pose.glows {
            let Some(&(b, p)) = gpu.models.get(pose.folder).and_then(|m| m.as_ref()).and_then(|m| m.tags.get(*tag)) else { continue };
            let b = b as usize;
            if b >= ng {
                continue;
            }
            let mut m = [0.0f32; 12];
            for k in 0..12 {
                let o = b * 48 + k * 4;
                m[k] = f32::from_le_bytes(pal_g[o..o + 4].try_into().unwrap());
            }
            let at = skin(p, [b as u16, 0, 0, 0], [1.0, 0.0, 0.0, 0.0], |_| m);
            // Pulled 6 cm toward the camera along the line of sight (same spot and size on
            // screen): the walking bob tilted the gun through the flat glow and it clipped.
            let pull = ((at.length() - 0.06).max(0.02) / at.length().max(1e-3)).min(1.0);
            let at = at * pull;
            let size = &(*size * pull);
            let t = std::time::Instant::now().duration_since(*START.get_or_init(std::time::Instant::now)).as_secs_f32();
            // Gentle flicker and slow turning (user: it pulsed too much and spun too fast).
            let flick = 0.95 + 0.05 * (t * 9.0).sin() * (t * 5.0).cos();
            let quad = |sz: f32, r: f32| {
                let (sn, cs) = r.sin_cos();
                let rx = glam::Vec3::new(cs, sn, 0.0) * sz * 0.5;
                let ry = glam::Vec3::new(-sn, cs, 0.0) * sz * 0.5;
                [at - rx - ry, at + rx - ry, at + rx + ry, at - rx + ry]
            };
            let (sz, a) = (*size * flick, *alpha);
            // Outer glow at full size, drawn without the depth test ("N:"): the gun's sides cut
            // straight lines through it (user) - as light it just blooms over the metal.
            add(format!("N:{ramp}_star"), &mut |buf| put_quad(buf, quad(sz * 1.8, t * 0.4), [0.0, 0.0, 1.0, 1.0], a * 0.5));
            add(format!("{ramp}_front"), &mut |buf| put_quad(buf, quad(sz, t * 1.2), [0.0, 0.0, 0.5, 0.5], a));
            add(format!("{ramp}_front"), &mut |buf| put_quad(buf, quad(sz * 0.7, -t * 1.7 + 1.0), [0.5, 0.5, 1.0, 1.0], a));
        }
        // World visuals: shots travel from the barrel point to where they hit.
        if let Some(cam) = crate::game::camera_full() {
            let kk = vm_tan / (cam.fov * 0.5).tan().max(1e-3);
            // World -> our view space (same screen position as the game camera); far points are
            // pulled in along their ray (our depth range ends at 20 m) with sizes scaled to match.
            let proj = |p: glam::Vec3| -> Option<(glam::Vec3, f32)> {
                let c = p - cam.pos;
                let z = c.dot(cam.fwd);
                if z < 0.3 {
                    return None;
                }
                let v = glam::Vec3::new(c.dot(cam.right) * kk, c.dot(cam.up) * kk, z);
                let zc = z.min(15.0);
                Some((v * (zc / z), kk * (zc / z)))
            };
            let mut shots = crate::fx::SHOTS.lock().map(|g| g.clone()).unwrap_or_default();
            shots.retain(|s| s.t0.elapsed().as_secs_f32() < crate::fx::life(s, &crate::fx::LOOKS[s.slot]));
            if let Ok(mut g) = crate::fx::SHOTS.lock() {
                let now = std::time::Instant::now();
                g.retain(|s| (now - s.t0).as_secs_f32() < crate::fx::life(s, &crate::fx::LOOKS[s.slot]));
            }
            use crate::fx::Style;
            for s in &shots {
                let look = &crate::fx::LOOKS[s.slot];
                let t = s.t0.elapsed().as_secs_f32();
                let frame = (s.seed >> 5) % 4;
                let (fu, fv) = ((frame % 2) as f32 * 0.5, (frame / 2) as f32 * 0.5);
                let atlas = [fu, fv, fu + 0.5, fv + 0.5];
                let n_vis = if s.style == Style::Pellet { 5 } else { s.ends.len() };
                for (i, (end, _chr)) in s.ends.iter().enumerate().take(n_vis) {
                    let d = *end - s.origin;
                    let dist = d.length().max(0.01);
                    let dir = d / dist;
                    let travel = dist / look.speed;
                    let rot = crate::fx::hash(s.seed.wrapping_add(i as u32 * 7)) * 6.28;
                    // streak between two world points, billboarded around its axis
                    let streak = |a: glam::Vec3, b: glam::Vec3, w: f32| -> Option<[glam::Vec3; 4]> {
                        let ((va, sa), (vb, sb)) = (proj(a)?, proj(b)?);
                        let axis = vb - va;
                        let to_cam = -(va + vb).normalize_or_zero();
                        let side = axis.cross(to_cam).normalize_or_zero();
                        let (wa, wb) = (side * w * sa * 0.5, side * w * sb * 0.5);
                        Some([va - wa, vb - wb, vb + wb, va + wa])
                    };
                    let sprite = |p: glam::Vec3, size: f32, r: f32| -> Option<[glam::Vec3; 4]> {
                        let (v, sc) = proj(p)?;
                        let (sn, cs) = r.sin_cos();
                        let h = size * sc * 0.5;
                        let rx = glam::Vec3::new(cs, sn, 0.0) * h;
                        let ry = glam::Vec3::new(-sn, cs, 0.0) * h;
                        Some([v - rx - ry, v + rx - ry, v + rx + ry, v - rx + ry])
                    };
                    let head_d = (look.speed * t).min(dist);
                    let head = s.origin + dir * head_d;
                    match s.style {
                        Style::Pellet | Style::Tracer => {
                            if t < travel + 0.02 {
                                let len = if s.style == Style::Tracer { 4.0 } else { 2.5 };
                                let tail = s.origin + dir * (head_d - len).max(0.0);
                                if let Some(c) = streak(tail, head, look.size) { add(format!("{}_side", look.ramp), &mut |buf| put_quad(buf, c, atlas, 1.2)); }
                            }
                        }
                        Style::Bolt | Style::Rocket | Style::Ball => {
                            if t < travel {
                                let tail = s.origin + dir * (head_d - look.size * 4.0).max(0.0);
                                if let Some(c) = streak(tail, head, look.size * 0.6) { add(format!("{}_side", look.ramp), &mut |buf| put_quad(buf, c, atlas, 1.0)); }
                                if let Some(c) = sprite(head, look.size, rot + t * 6.0) { add(format!("{}_front", look.ramp), &mut |buf| put_quad(buf, c, atlas, 1.3)); }
                                if s.style == Style::Ball {
                                    if let Some(c) = sprite(head, look.size * 1.3, -rot - t * 3.0) { add(format!("{}_star", look.ramp), &mut |buf| put_quad(buf, c, [0.0, 0.0, 1.0, 1.0], 1.0)); }
                                }
                                if s.style == Style::Rocket && s.slot == 3 {
                                    // Rocket in flight: a flickering fireball with a soft glow around
                                    // it, and flame puffs shrinking down the trail.
                                    let flick = 0.85 + 0.15 * (t * 37.0 + rot).sin();
                                    if let Some(c) = sprite(head, look.size * 3.2 * flick, rot) { add(format!("{}_star", look.ramp), &mut |buf| put_quad(buf, c, [0.0, 0.0, 1.0, 1.0], 0.55)); }
                                    if let Some(c) = sprite(head, look.size * 1.6 * flick, -rot - t * 9.0) { add(format!("{}_front", look.ramp), &mut |buf| put_quad(buf, c, atlas, 1.5)); }
                                    if let Some(c) = sprite(head, look.size * 1.1, rot * 2.0 + t * 13.0) { add(format!("{}_front", look.ramp), &mut |buf| put_quad(buf, c, [0.5 - fu, 0.5 - fv, 1.0 - fu, 1.0 - fv], 1.6)); }
                                    for k in 1..6 {
                                        let back = k as f32 * 0.35;
                                        if head_d - back <= 0.2 {
                                            break;
                                        }
                                        let f = 1.0 - k as f32 / 6.0;
                                        let p = s.origin + dir * (head_d - back);
                                        if let Some(c) = sprite(p, look.size * (0.6 + 0.6 * f), rot + k as f32 * 1.9 + t * 5.0) { add(format!("{}_front", look.ramp), &mut |buf| put_quad(buf, c, atlas, 0.9 * f)); }
                                    }
                                }
                            }
                        }
                        Style::Beam => {
                            let k = (t / 0.3).min(1.0);
                            if k < 1.0 {
                                if let Some(c) = streak(s.origin, *end, look.size * (1.0 - k)) { add(format!("{}_side", look.ramp), &mut |buf| put_quad(buf, c, atlas, 1.5 * (1.0 - k))); }
                            }
                        }
                    }
                    // Impact burst when the shot arrives.
                    let dur = if matches!(s.style, Style::Rocket | Style::Ball) { 0.45 } else { 0.18 };
                    let arrive = if s.style == Style::Beam { 0.0 } else { travel };
                    let k = (t - arrive) / dur;
                    if (0.0..1.0).contains(&k) && (s.style != Style::Pellet || i % 2 == 0) {
                        let at = *end - dir * 0.15;
                        let a = (1.0 - k) * 1.4;
                        let size = look.impact * (0.45 + 0.55 * k);
                        if let Some(c) = sprite(at, size, rot) { add(format!("{}_star", look.ramp), &mut |buf| put_quad(buf, c, [0.0, 0.0, 1.0, 1.0], a)); }
                        if let Some(c) = sprite(at, size * 0.8, rot + 1.3) { add(format!("{}_front", look.ramp), &mut |buf| put_quad(buf, c, atlas, a)); }
                    }
                }
            }
        }
        // Stuck sticky bombs / arbalest bolts: small blinking glows.
        if let Some(cam) = crate::game::camera_full() {
            let kk = vm_tan / (cam.fov * 0.5).tan().max(1e-3);
            let stuck = crate::fx::STUCK.lock().map(|g| g.clone()).unwrap_or_default();
            for (p, ramp, blink) in stuck {
                let c = p - cam.pos;
                let z = c.dot(cam.fwd);
                if z < 0.3 {
                    continue;
                }
                let v = glam::Vec3::new(c.dot(cam.right) * kk, c.dot(cam.up) * kk, z);
                let zc = z.min(15.0);
                let (v, sc) = (v * (zc / z), kk * (zc / z));
                let h = if blink { 0.32 } else { 0.18 } * sc * 0.5;
                let (rx, ry) = (glam::Vec3::X * h, glam::Vec3::Y * h);
                add(format!("{ramp}_front"), &mut |buf| put_quad(buf, [v - rx - ry, v + rx - ry, v + rx + ry, v - rx + ry], [0.0, 0.0, 0.5, 0.5], 1.4));
            }
        }
        // Meathook: a burning chain from under the Super Shotgun's barrel to the hooked demon.
        let hook = crate::fx::HOOK.lock().ok().and_then(|g| *g);
        if let (Some((target, t0)), Some(cam)) = (hook.filter(|_| pose.folder == "super_shotgun"), crate::game::camera_full()) {
            let kk = vm_tan / (cam.fov * 0.5).tan().max(1e-3);
            let c = target - cam.pos;
            let z = c.dot(cam.fwd);
            if z > 0.3 {
                let zc = z.min(15.0);
                let vt = glam::Vec3::new(c.dot(cam.right) * kk, c.dot(cam.up) * kk, z) * (zc / z);
                let st = kk * (zc / z);
                let start = flash_at + glam::Vec3::new(0.0, -0.035, 0.0);
                // The chain shoots out in 0.1 s, then stays taut.
                let reach = (t0.elapsed().as_secs_f32() / 0.1).min(1.0);
                let tip = start.lerp(vt, reach);
                let size_at = |f: f32| 1.0 + (st - 1.0) * f * reach;
                let axis = tip - start;
                let to_cam = -(start + tip).normalize_or_zero();
                let side = axis.cross(to_cam).normalize_or_zero();
                let (wa, wb) = (side * 0.012, side * 0.012 * size_at(1.0) * 1.6);
                add("F:fire_side".to_string(), &mut |buf| put_quad(buf, [start - wa, tip - wb, tip + wb, start + wa], [0.0, 0.0, 0.5, 0.5], 1.2));
                let links = ((z * reach) / 0.3).clamp(4.0, 60.0) as usize;
                for i in 0..=links {
                    let f = i as f32 / links as f32;
                    let p = start.lerp(tip, f);
                    let h = 0.022 * size_at(f);
                    let r = if i % 2 == 0 { 0.6 } else { 2.2 };
                    let (sn, cs) = (r as f32).sin_cos();
                    let rx = glam::Vec3::new(cs, sn, 0.0) * h;
                    let ry = glam::Vec3::new(-sn, cs, 0.0) * h * 0.6;
                    add("F:fire_front".to_string(), &mut |buf| put_quad(buf, [p - rx - ry, p + rx - ry, p + rx + ry, p - rx + ry], [0.5, 0.0, 1.0, 0.5], 1.1));
                }
                // Hook head where it bit.
                let h = 0.25 * size_at(1.0) * 0.5;
                let (rx, ry) = (glam::Vec3::X * h, glam::Vec3::Y * h);
                add("F:fire_star".to_string(), &mut |buf| put_quad(buf, [tip - rx - ry, tip + rx - ry, tip + rx + ry, tip - rx + ry], [0.0, 0.0, 1.0, 1.0], 1.3));
            }
        }
        let mut fx_bytes: Vec<u8> = Vec::new();
        let mut flash_draws: Vec<(String, u32)> = Vec::new();
        for (tex, bytes) in &by_tex {
            if bytes.is_empty() || fx_bytes.len() + bytes.len() > FX_BYTES {
                continue;
            }
            fx_bytes.extend_from_slice(bytes);
            flash_draws.push((tex.clone(), (bytes.len() / 36) as u32));
        }
        let fx_off = 256 + 2 * ((BONES_BYTES + 255) & !255);
        if !fx_bytes.is_empty() && fx_bytes.len() <= FX_BYTES {
            std::ptr::copy_nonoverlapping(fx_bytes.as_ptr(), base.add(fx_off), fx_bytes.len());
        } else {
            flash_draws.clear();
        }
        let model = gpu.models.get(pose.folder).unwrap().as_ref().unwrap();
        let mut glow_meshes: Vec<usize> = Vec::new();
        for (mi, m) in model.meshes.iter().enumerate() {
            if m.part == 0 && !cfg.vm_arms {
                continue;
            }
            let mm = model.mesh_modes.get(mi).copied().unwrap_or(0);
            // vm_debug 40 + k: normal view plus hidden mesh k - find which hidden part fills a gap.
            let show_k = (40..80).contains(&cfg.vm_debug) && mi as u32 == cfg.vm_debug - 40;
            if mm != 0 && mm != pose.mode && cfg.vm_debug != 5 && !show_k && !(20..40).contains(&cfg.vm_debug) {
                continue;
            }
            // vm_debug 20 + k: only mesh k (untextured) - find which part makes an artifact.
            if (20..40).contains(&cfg.vm_debug) && mi as u32 != cfg.vm_debug - 20 && m.part != 0 {
                continue;
            }
            if m.glow {
                glow_meshes.push(mi);
                continue;
            }
            let pal = if m.part == 0 { arms_off } else { gun_off };
            cl.SetPipelineState(if m.part == 0 { &pso_arms } else { &pso });
            cl.SetGraphicsRootConstantBufferView(1, gpu_base + pal as u64);
            cl.SetGraphicsRootDescriptorTable(2, gpu.srv_gpu(m.srv_first));
            cl.IASetVertexBuffers(0, Some(&[m.vbv]));
            cl.IASetIndexBuffer(Some(&m.ibv));
            let _ = (m.has_normal, m.has_spec);
            cl.DrawIndexedInstanced(m.count, 1, 0, 0, 0);
        }
        // The Crucible blade: additive light over the solid parts (behind the hand where the hand
        // is in front - depth tested, not written).
        let blade_on = BLADE_ON.load(std::sync::atomic::Ordering::Relaxed);
        for &mi in glow_meshes.iter().filter(|_| blade_on) {
            let m = &model.meshes[mi];
            cl.SetPipelineState(&pso_glow);
            cl.SetGraphicsRootConstantBufferView(1, gpu_base + gun_off as u64);
            cl.SetGraphicsRootDescriptorTable(2, gpu.srv_gpu(m.srv_first));
            cl.IASetVertexBuffers(0, Some(&[m.vbv]));
            cl.IASetIndexBuffer(Some(&m.ibv));
            cl.DrawIndexedInstanced(m.count, 1, 0, 0, 0);
        }
        if !flash_draws.is_empty() {
            let pso_world = gpu.fx_pso(desc.Format, true, samples)?;
            let pso_flash = gpu.fx_pso(desc.Format, false, samples)?;
            cl.IASetVertexBuffers(0, Some(&[D3D12_VERTEX_BUFFER_VIEW {
                BufferLocation: gpu_base + fx_off as u64,
                SizeInBytes: fx_bytes.len() as u32,
                StrideInBytes: 36,
            }]));
            let mut first = 0u32;
            for (tex, count) in &flash_draws {
                let (flash_pass, tex) = match tex.strip_prefix("F:") {
                    Some(t) => (true, t),
                    None => (false, tex.as_str()),
                };
                // The flash also sits *behind* the gun (user: never drawn over the model).
                let _ = flash_pass;
                // "N:" = no depth test (glows that must not be cut by the gun's surfaces).
                let (no_depth, tex) = match tex.strip_prefix("N:") {
                    Some(t) => (true, t),
                    None => (false, tex),
                };
                cl.SetPipelineState(if no_depth { &pso_flash } else { &pso_world });
                if let Some(t) = gpu.fx_texture(cl, tex) {
                    cl.SetGraphicsRootDescriptorTable(2, gpu.srv_gpu(t));
                    cl.DrawInstanced(*count, 1, first, 0);
                }
                first += count;
            }
            // Parts that sit in front of a glow drawn without depth (the rocket launcher's slide in
            // front of the chamber's outer fire ring): drawn again on top, equal depth passes.
            let front: &[usize] = FRONT_MESHES.iter().find(|(f, _)| *f == pose.folder).map(|(_, m)| *m).unwrap_or(&[]);
            if !front.is_empty() {
                cl.SetPipelineState(&pso_front);
                let model = gpu.models.get(pose.folder).unwrap().as_ref().unwrap();
                for &mi in front {
                    let Some(m) = model.meshes.get(mi) else { continue };
                    cl.SetGraphicsRootConstantBufferView(1, gpu_base + gun_off as u64);
                    cl.SetGraphicsRootDescriptorTable(2, gpu.srv_gpu(m.srv_first));
                    cl.IASetVertexBuffers(0, Some(&[m.vbv]));
                    cl.IASetIndexBuffer(Some(&m.ibv));
                    cl.DrawIndexedInstanced(m.count, 1, 0, 0, 0);
                }
            }
        }
        if samples == 1 && config::get_cached().vm_fxaa {
            // FXAA over the gun: copy the frame, then smooth the gun's pixels (mask = its depth).
            let fxaa = gpu.fxaa_pso(desc.Format)?;
            let depth = gpu.depth.as_ref().unwrap().0.clone();
            gpu.copy_post(cl, target)?;
            cl.ResourceBarrier(&[transition(&depth, D3D12_RESOURCE_STATE_DEPTH_WRITE, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE)]);
            cl.OMSetRenderTargets(1, Some(&rtv), false, None);
            cl.SetPipelineState(&fxaa);
            cl.SetGraphicsRootDescriptorTable(2, gpu.srv_gpu(4));
            cl.SetGraphicsRootDescriptorTable(3, gpu.srv_gpu(2));
            cl.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            cl.DrawInstanced(3, 1, 0, 0);
            cl.ResourceBarrier(&[transition(&depth, D3D12_RESOURCE_STATE_PIXEL_SHADER_RESOURCE, D3D12_RESOURCE_STATE_DEPTH_WRITE)]);
        }
        if samples > 1 {
            // Resolve the multisampled frame (game frame + gun + effects) into the back buffer.
            let m = gpu.msaa.as_ref().unwrap();
            cl.ResourceBarrier(&[
                transition(&m.rt, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_RESOLVE_SOURCE),
                transition(target, D3D12_RESOURCE_STATE_RENDER_TARGET, D3D12_RESOURCE_STATE_RESOLVE_DEST),
            ]);
            cl.ResolveSubresource(target, 0, &m.rt, 0, desc.Format);
            cl.ResourceBarrier(&[transition(target, D3D12_RESOURCE_STATE_RESOLVE_DEST, D3D12_RESOURCE_STATE_RENDER_TARGET)]);
            // hand the back buffer back to the HUD pass bound as it was
            cl.OMSetRenderTargets(1, Some(&rtv), false, None);
        }
        Ok(())
    }
}
