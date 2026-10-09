//! Doom Eternal sounds (converted from the user's own install by tools/convert_audio.py) played on
//! our own WASAPI stream through rodio, mixed on top of Elden Ring's audio.
//!
//! `doom_audio/<event>/<n>.wav` next to the DLL; `play("ssg_fire")` picks a random variant.

use std::{
    collections::HashMap,
    io::Cursor,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use rodio::{Decoder, OutputStream, Source};

use crate::config;

struct Request {
    event: &'static str,
    volume: f32,
}

static TX: OnceLock<mpsc::SyncSender<Request>> = OnceLock::new();

/// Queue a sound; never blocks the game thread (drops the request if the queue is full).
pub fn play(event: &'static str) {
    play_vol(event, 1.0);
}

pub fn play_vol(event: &'static str, volume: f32) {
    if let Some(tx) = TX.get() {
        let _ = tx.try_send(Request { event, volume });
    }
}

/// The sound files are converted 9 dB below the files' own level (their peaks go over full scale
/// and were clipped - tools/convert_audio.py HEADROOM_DB) and played at that: about Doom
/// Eternal's own level (it plays them far below full scale too). The +6 dB boost made Doom's own
/// built-in grit in the Flame Belch / guns stand out (user: Doom is quieter and sounds fine).
const HEADROOM_GAIN: f32 = 1.0;

/// A sound, decoded once: stereo f32 at the output device's rate.
type Pcm = Arc<[f32]>;

struct Voice {
    event: &'static str,
    pcm: Pcm,
    /// Next frame.
    pos: usize,
    gain: f32,
    /// Frames left of a quick fade-out (a voice cut for a newer one; no click), None = playing.
    fade: Option<u32>,
}

/// Doom's own mix of all the sound effects, then a peak limiter (user: gunshots still distorted on
/// headphones - stacked rapid-fire shots summed past full scale, and every sound went through the
/// output's cheap per-play rate conversion). Sounds are resampled once at load to the device rate.
struct Mixer {
    voices: Arc<std::sync::Mutex<Vec<Voice>>>,
    rate: u32,
    buf: Vec<f32>,
    at: usize,
    /// Limiter gain (1 = untouched).
    gain: f32,
}

/// Fade-out length when a voice is cut (frames at 48 kHz-ish: ~6 ms).
const CUT_FADE: u32 = 300;

impl Mixer {
    fn fill(&mut self) {
        const FRAMES: usize = 256;
        self.buf.clear();
        self.buf.resize(FRAMES * 2, 0.0);
        if let Ok(mut vs) = self.voices.lock() {
            for v in vs.iter_mut() {
                let frames = v.pcm.len() / 2;
                for f in 0..FRAMES {
                    if v.pos >= frames {
                        break;
                    }
                    let mut g = v.gain;
                    if let Some(left) = v.fade.as_mut() {
                        if *left == 0 {
                            v.pos = frames;
                            break;
                        }
                        g *= *left as f32 / CUT_FADE as f32;
                        *left -= 1;
                    }
                    self.buf[f * 2] += v.pcm[v.pos * 2] * g;
                    self.buf[f * 2 + 1] += v.pcm[v.pos * 2 + 1] * g;
                    v.pos += 1;
                }
            }
            vs.retain(|v| v.pos < v.pcm.len() / 2);
        }
        // Peak limiter: instant attack (never over 0.95), ~80 ms release back to unity. Smooth
        // gain changes instead of flat-topped clipping.
        let release = 1.0 - (-1.0 / (0.08 * self.rate as f32)).exp();
        for f in 0..FRAMES {
            let (l, r) = (self.buf[f * 2], self.buf[f * 2 + 1]);
            let peak = l.abs().max(r.abs());
            if peak * self.gain > 0.95 {
                self.gain = 0.95 / peak;
            } else {
                self.gain += (1.0 - self.gain) * release;
            }
            self.buf[f * 2] = l * self.gain;
            self.buf[f * 2 + 1] = r * self.gain;
        }
        self.at = 0;
    }
}

impl Iterator for Mixer {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.at >= self.buf.len() {
            self.fill();
        }
        let s = self.buf[self.at];
        self.at += 1;
        Some(s)
    }
}

impl Source for Mixer {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

/// The default output device's rate (the mix runs at it: no conversion while playing).
fn device_rate() -> u32 {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    rodio::cpal::default_host()
        .default_output_device()
        .and_then(|d| d.default_output_config().ok())
        .map_or(48000, |c| c.sample_rate().0)
}

/// Windowed-sinc resampling of interleaved stereo (Blackman window, 32 taps, a table of 1024
/// sub-sample phases; anti-aliased when going down in rate).
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    const HALF: usize = 16;
    const TAPS: usize = HALF * 2;
    const PHASES: usize = 1024;
    let cutoff = (to as f64 / from as f64).min(1.0) * 0.95;
    // weights[p][j]: tap j (source frame i0 - HALF + 1 + j) for a sub-sample offset p / PHASES
    let mut table = vec![[0.0f32; TAPS]; PHASES + 1];
    for (p, row) in table.iter_mut().enumerate() {
        let frac = p as f64 / PHASES as f64;
        let mut sum = 0.0;
        let mut w = [0.0f64; TAPS];
        for (j, wj) in w.iter_mut().enumerate() {
            let x = frac - (j as f64 - (HALF as f64 - 1.0));
            let t = x * cutoff;
            let sinc = if t.abs() < 1e-9 { 1.0 } else { (std::f64::consts::PI * t).sin() / (std::f64::consts::PI * t) };
            let n = (x / HALF as f64 + 1.0) * 0.5;
            let win = if (0.0..=1.0).contains(&n) {
                0.42 - 0.5 * (2.0 * std::f64::consts::PI * n).cos() + 0.08 * (4.0 * std::f64::consts::PI * n).cos()
            } else {
                0.0
            };
            *wj = sinc * win;
            sum += *wj;
        }
        for j in 0..TAPS {
            row[j] = (w[j] / sum) as f32;
        }
    }
    let frames = input.len() / 2;
    let out_frames = (frames as u64 * to as u64 / from as u64) as usize;
    let step = from as f64 / to as f64;
    let mut out = Vec::with_capacity(out_frames * 2);
    for o in 0..out_frames {
        let src = o as f64 * step;
        let i0 = src.floor() as i64;
        let row = &table[((src - i0 as f64) * PHASES as f64).round() as usize];
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for (j, &w) in row.iter().enumerate() {
            let k = i0 - (HALF as i64 - 1) + j as i64;
            if k < 0 || k as usize >= frames {
                continue;
            }
            l += input[k as usize * 2] * w;
            r += input[k as usize * 2 + 1] * w;
        }
        out.push(l);
        out.push(r);
    }
    out
}

/// A wav file as stereo f32 at `rate`.
fn decode(bytes: Vec<u8>, rate: u32) -> Option<Pcm> {
    let dec = Decoder::new(Cursor::new(bytes)).ok()?;
    let (ch, from) = (dec.channels() as usize, dec.sample_rate());
    let raw: Vec<f32> = dec.convert_samples::<f32>().collect();
    let stereo: Vec<f32> = match ch {
        1 => raw.iter().flat_map(|&v| [v, v]).collect(),
        2 => raw,
        n => raw.chunks_exact(n).flat_map(|c| [c[0], c[1]]).collect(),
    };
    Some(Arc::from(resample(&stereo, from, rate)))
}

pub fn init() {
    let (tx, rx) = mpsc::sync_channel::<Request>(64);
    if TX.set(tx).is_err() {
        return;
    }
    std::thread::spawn(move || {
        let rate = device_rate();
        let t0 = std::time::Instant::now();
        let bank = load_bank(rate);
        log::info!(
            "audio: {} events, {} files, decoded to {rate} Hz in {:.1}s",
            bank.len(),
            bank.values().map(Vec::len).sum::<usize>(),
            t0.elapsed().as_secs_f32()
        );
        let (stream, handle) = match OutputStream::try_default() {
            Ok(s) => s,
            Err(e) => {
                log::error!("audio: no output device: {e}");
                return;
            }
        };
        let _keep = stream;
        let voices: Arc<std::sync::Mutex<Vec<Voice>>> = Arc::default();
        let mixer = Mixer { voices: voices.clone(), rate, buf: Vec::new(), at: 0, gain: 1.0 };
        if let Err(e) = handle.play_raw(mixer) {
            log::error!("audio: mixer not started: {e}");
            return;
        }
        while let Ok(req) = rx.recv() {
            let Some(variants) = bank.get(req.event) else {
                log::debug!("audio: no sound for {}", req.event);
                continue;
            };
            let pcm = variants[fastrand::usize(..variants.len())].clone();
            // (live: the settings window's effects slider)
            let cfg = config::get_cached();
            // per-sound level, live: [sound_db] event = dB
            let db = cfg.sound_db.get(req.event).copied().unwrap_or(0.0).clamp(-40.0, 20.0);
            let gain = req.volume * cfg.volume * event_gain(req.event) * HEADROOM_GAIN * 10f32.powf(db / 20.0);
            if let Ok(mut vs) = voices.lock() {
                // Live voices per event: rapid fire (plasma 13/s, chaingun 20/s) stacked dozens
                // of shots; the oldest of this event fade out quickly for the new one.
                let max = max_voices(req.event);
                let live = vs.iter().filter(|v| v.event == req.event && v.fade.is_none()).count();
                let mut cut = (live + 1).saturating_sub(max);
                for v in vs.iter_mut().filter(|v| v.event == req.event && v.fade.is_none()) {
                    if cut == 0 {
                        break;
                    }
                    v.fade = Some(CUT_FADE);
                    cut -= 1;
                }
                vs.push(Voice { event: req.event, pcm, pos: 0, gain, fade: None });
            }
        }
    });
}

/// Doom's source files are mastered for its own mixer; some are far hotter than the rest
/// (plasma_shot averages -5 dBFS vs ~-16 for the shotguns).
fn event_gain(event: &str) -> f32 {
    match event {
        "plasma_fire" => 0.4,
        // Chaingun now carries Doom's heavy body layer (~4x the old crack's level) and fires fast.
        "chaingun_fire" => 0.65,
        "turret_fire" => 0.65,
        "heavy_cannon_fire" => 0.8,
        _ => 1.0,
    }
}

fn max_voices(event: &str) -> usize {
    match event {
        // turret: 4 at most (user)
        "turret_fire" => 4,
        "plasma_fire" | "chaingun_fire" | "heavy_cannon_fire" => 3,
        _ => 4,
    }
}

fn load_bank(rate: u32) -> HashMap<String, Vec<Pcm>> {
    let mut bank: HashMap<String, Vec<Pcm>> = HashMap::new();
    let root = config::mod_dir().join("doom_audio");
    let Ok(dirs) = std::fs::read_dir(&root) else {
        log::warn!("audio: {} missing - run tools/convert_audio.py", root.display());
        return bank;
    };
    for dir in dirs.flatten() {
        let name = dir.file_name().to_string_lossy().into_owned();
        let Ok(files) = std::fs::read_dir(dir.path()) else { continue };
        for f in files.flatten() {
            if f.path().extension().is_some_and(|e| e == "wav") {
                if let Some(pcm) = std::fs::read(f.path()).ok().and_then(|b| decode(b, rate)) {
                    bank.entry(name.clone()).or_default().push(pcm);
                }
            }
        }
    }
    bank
}

// ------------------------------------------------------------------------------------ music

static COMBAT: AtomicBool = AtomicBool::new(false);

/// Tell the music director whether a fight is on (cheap; call every frame).
pub fn set_combat(on: bool) {
    COMBAT.store(on, Ordering::Relaxed);
}

/// Music test (keys F7 on/off, F6 next, F5 back to the start point, F2 / F4 -5 / +5 s, F3 mark
/// the start point here): plays the playlist without a fight and shows the track and position.
pub static MUSIC_TEST: AtomicBool = AtomicBool::new(false);
/// One test command for the music thread: 1 next track, 2 restart at the start point, 3 back 5 s,
/// 4 forward 5 s, 5 mark the start point at the current position.
pub static MUSIC_CMD: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
static MUSIC_STATUS: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// "BFG_DIVISION  1:02.4 / 8:26  START 1:02.0" while a track plays (test overlay, logs).
pub fn music_status() -> Option<String> {
    MUSIC_STATUS.lock().ok().and_then(|s| s.clone())
}

fn mmss(t: f64) -> String {
    format!("{}:{:04.1}", (t / 60.0) as u32, t % 60.0)
}

/// A playlist entry from `music_tracks` ("name@start seconds"): the file in doom_music_tracks, where
/// it starts (its drop) and its level match.
struct Track {
    name: String,
    path: std::path::PathBuf,
    start: f64,
}

fn playlist(cfg: &config::Config) -> Vec<Track> {
    let dir = config::mod_dir().join("doom_music_tracks");
    cfg.music_tracks
        .iter()
        .filter_map(|e| {
            let (name, start) = e.split_once('@').unwrap_or((e.as_str(), "0"));
            let path = dir.join(format!("{}.ogg", name.trim()));
            path.exists().then(|| Track { name: name.trim().to_string(), path, start: start.trim().parse().unwrap_or(0.0) })
        })
        .collect()
}

/// The track from `at` seconds (decoded past in this thread, not in the audio callback), level
/// matched (doom_music_tracks/gains.json) - and its length in seconds.
fn open_track(t: &Track, at: f64) -> Option<(Box<dyn Source<Item = f32> + Send>, f64)> {
    let file = std::fs::File::open(&t.path).ok()?;
    let mut dec = Decoder::new(std::io::BufReader::new(file)).ok()?;
    let (rate, ch) = (dec.sample_rate() as f64, dec.channels() as f64);
    let len = dec.total_duration().map_or(0.0, |d| d.as_secs_f64());
    let skip = (at.max(0.0) * rate * ch) as usize;
    if skip > 0 {
        dec.by_ref().take(skip).for_each(drop);
    }
    let db = std::fs::read_to_string(t.path.with_file_name("gains.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<HashMap<String, f32>>(&s).ok())
        .and_then(|m| m.get(&format!("{}.ogg", t.name)).copied())
        .unwrap_or(0.0);
    let g = 10f32.powf(db.clamp(-12.0, 12.0) / 20.0);
    Some((Box::new(dec.convert_samples::<f32>().amplify(g)), len))
}

/// The keys line under the music test readout (per music mode).
pub fn music_keys_hint() -> &'static str {
    if config::get_cached().music_mode == "tracks" {
        "F6 NEXT   F5 BACK TO START   F2 -5 S   F4 +5 S   F3 MARK START HERE   F7 OFF"
    } else {
        "F6 NEXT SUITE   F5 RESTART SUITE   F4 NEXT PIECE   F3 SKIP THIS PIECE FOR GOOD   F7 OFF"
    }
}

/// Fight music. `music_mode = "doom"` (default): DOOM Eternal's own combat music, played the way its
/// Wwise bank plays it (tools/convert_music.py, docs/doom_music_system.md); `"tracks"`: full
/// jukebox tracks from a start point.
pub fn init_music() {
    if config::get_cached().music_mode == "tracks" {
        init_music_tracks();
    } else {
        init_music_doom();
    }
}

// ----------------------------------------------------------------- music: Doom's playlists

/// A playlist node of a suite (suite.json "tree"): a piece, or a group of nodes.
enum Kind {
    Piece(String),
    /// continuous sequence: every child in order
    Seq,
    /// continuous random: every child, shuffled
    Shuffle,
    /// step sequence: the next child each pass
    StepSeq,
    /// step random: one random child each pass (not one of the last `avoid` picks)
    StepRandom,
}

struct Node {
    kind: Kind,
    /// passes of this node per visit; 0 = forever
    loops: u32,
    avoid: usize,
    kids: Vec<usize>,
}

/// Walks a suite's playlist tree like Wwise: groups in order or at random, loop counts, avoid
/// repeat; a loop-forever node is where the playlist goes on from once reached.
struct Walker {
    nodes: Vec<Node>,
    /// The node one more pass of continues the playlist (the root, then the innermost forever loop).
    forever: usize,
    steps: HashMap<usize, usize>,
    last: HashMap<usize, Vec<usize>>,
    queue: std::collections::VecDeque<String>,
}

impl Walker {
    /// `skip`: piece names left out (groups left empty go too).
    fn new(tree: &serde_json::Value, skip: &[String], intro: bool) -> Option<Walker> {
        fn add(v: &serde_json::Value, skip: &[String], nodes: &mut Vec<Node>) -> Option<usize> {
            let loops = v["loop"].as_u64().unwrap_or(1) as u32;
            let kind = if let Some(p) = v["piece"].as_str() {
                if skip.iter().any(|s| s == p) {
                    return None;
                }
                Kind::Piece(p.to_string())
            } else {
                match v["type"].as_str().unwrap_or("") {
                    "continuous random" => Kind::Shuffle,
                    "step sequence" => Kind::StepSeq,
                    "step random" => Kind::StepRandom,
                    _ => Kind::Seq,
                }
            };
            let kids: Vec<usize> = v["kids"].as_array().map_or(vec![], |a| a.iter().filter_map(|k| add(k, skip, nodes)).collect());
            if !matches!(kind, Kind::Piece(_)) && kids.is_empty() {
                return None;
            }
            nodes.push(Node { kind, loops, avoid: v["avoid"].as_u64().unwrap_or(1) as usize, kids });
            Some(nodes.len() - 1)
        }
        let mut nodes = Vec::new();
        let root = add(tree, skip, &mut nodes)?;
        // a play-once [intro, looping body]: start at the body (music_intro = false)
        let r = &nodes[root];
        let start = if !intro && matches!(r.kind, Kind::Seq) && r.loops == 1 && r.kids.len() == 2 && nodes[r.kids[1]].loops == 0 {
            r.kids[1]
        } else {
            root
        };
        Some(Walker { nodes, forever: start, steps: HashMap::new(), last: HashMap::new(), queue: Default::default() })
    }

    fn next(&mut self) -> Option<String> {
        if self.queue.is_empty() {
            let mut out = Vec::new();
            self.pass(self.forever, &mut out);
            self.queue.extend(out);
        }
        self.queue.pop_front()
    }

    /// One pass of node `n`; true when it reached a loop-forever node (nothing after it plays).
    fn pass(&mut self, n: usize, out: &mut Vec<String>) -> bool {
        let kids = self.nodes[n].kids.clone();
        match &self.nodes[n].kind {
            Kind::Piece(p) => {
                out.push(p.clone());
                false
            }
            Kind::Seq | Kind::Shuffle => {
                let mut order = kids;
                if matches!(self.nodes[n].kind, Kind::Shuffle) {
                    fastrand::shuffle(&mut order);
                }
                order.into_iter().any(|k| self.run(k, out))
            }
            Kind::StepSeq => {
                let c = self.steps.entry(n).or_insert(0);
                let k = kids[*c % kids.len()];
                *c += 1;
                self.run(k, out)
            }
            Kind::StepRandom => {
                let avoid = self.nodes[n].avoid.min(kids.len() - 1);
                let last = self.last.entry(n).or_default();
                let free: Vec<usize> = kids.iter().copied().filter(|k| !last.contains(k)).collect();
                let k = free[fastrand::usize(..free.len())];
                last.push(k);
                while last.len() > avoid {
                    last.remove(0);
                }
                self.run(k, out)
            }
        }
    }

    /// Node `k` its loop count of passes (forever: one pass, and the playlist goes on from it).
    fn run(&mut self, k: usize, out: &mut Vec<String>) -> bool {
        if self.nodes[k].loops == 0 {
            self.forever = k;
            self.pass(k, out);
            return true;
        }
        (0..self.nodes[k].loops).any(|_| self.pass(k, out))
    }
}

/// A piece on the music timeline: plays from frame `start` of the music clock.
struct Placed {
    name: String,
    pcm: Pcm,
    start: u64,
    gain: f32,
    /// a quick fade-out (cut by a test key): frames left, of, None = playing
    fade: Option<(u32, u32)>,
}

#[derive(Default)]
struct MusicState {
    voices: Vec<Placed>,
    /// music clock: frames played; stands still while paused
    clock: u64,
    paused: bool,
    /// volume now / wanted (ramped per block, no zipper noise)
    vol: f32,
    target: f32,
}

/// The music's own output: its pieces mixed on the clock, then a peak limiter (overlapping tails).
struct MusicMixer {
    state: Arc<std::sync::Mutex<MusicState>>,
    rate: u32,
    buf: Vec<f32>,
    at: usize,
    lim: f32,
}

impl MusicMixer {
    fn fill(&mut self) {
        const FRAMES: usize = 512;
        self.buf.clear();
        self.buf.resize(FRAMES * 2, 0.0);
        self.at = 0;
        let Ok(mut st) = self.state.lock() else { return };
        if st.paused {
            return;
        }
        let clock = st.clock;
        let (v0, v1) = (st.vol, st.target);
        for v in st.voices.iter_mut() {
            let frames = (v.pcm.len() / 2) as u64;
            for f in 0..FRAMES {
                let t = clock + f as u64;
                if t < v.start {
                    continue;
                }
                let i = (t - v.start) as usize;
                if i as u64 >= frames {
                    break;
                }
                let mut g = v.gain * (v0 + (v1 - v0) * f as f32 / FRAMES as f32);
                if let Some((left, of)) = v.fade.as_mut() {
                    if *left == 0 {
                        // faded out: dropped below
                        v.pcm = Arc::from(Vec::new());
                        break;
                    }
                    g *= *left as f32 / *of as f32;
                    *left -= 1;
                }
                self.buf[f * 2] += v.pcm[i * 2] * g;
                self.buf[f * 2 + 1] += v.pcm[i * 2 + 1] * g;
            }
        }
        st.vol = v1;
        st.clock += FRAMES as u64;
        let end = st.clock;
        st.voices.retain(|v| v.start + (v.pcm.len() / 2) as u64 > end);
        drop(st);
        let release = 1.0 - (-1.0 / (0.08 * self.rate as f32)).exp();
        for f in 0..FRAMES {
            let (l, r) = (self.buf[f * 2], self.buf[f * 2 + 1]);
            let peak = l.abs().max(r.abs());
            if peak * self.lim > 0.95 {
                self.lim = 0.95 / peak;
            } else {
                self.lim += (1.0 - self.lim) * release;
            }
            self.buf[f * 2] = l * self.lim;
            self.buf[f * 2 + 1] = r * self.lim;
        }
    }
}

impl Iterator for MusicMixer {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.at >= self.buf.len() {
            self.fill();
        }
        let s = self.buf[self.at];
        self.at += 1;
        Some(s)
    }
}

impl Source for MusicMixer {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        self.rate
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

/// A suite (doom_music/<name>/suite.json): its tree, pieces' cues (ms) and level match.
struct Suite {
    name: String,
    dir: std::path::PathBuf,
    json: serde_json::Value,
}

fn load_suites() -> Vec<Suite> {
    let root = config::mod_dir().join("doom_music");
    let mut v: Vec<Suite> = std::fs::read_dir(&root)
        .map(|d| {
            d.flatten()
                .filter_map(|e| {
                    let json = serde_json::from_str(&std::fs::read_to_string(e.path().join("suite.json")).ok()?).ok()?;
                    Some(Suite { name: e.file_name().to_string_lossy().into_owned(), dir: e.path(), json })
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

/// The playing suite: its walker and where the next piece's ENTRY cue lands (music clock frame).
struct Run {
    suite: usize,
    walker: Walker,
    cue: Option<u64>,
    gain: f32,
    /// the next piece, decoded ahead: name, audio, entry / exit (frames)
    next: Option<(String, Pcm, u64, u64)>,
}

fn decode_piece(dir: &std::path::Path, name: &str, rate: u32) -> Option<Pcm> {
    decode(std::fs::read(dir.join(format!("{name}.ogg"))).ok()?, rate)
}

/// Fight music, Doom mode (user, 2026-10-08: heavy only): a random suite's heavy playlist from the
/// top of its loop (the intro piece only with `music_intro`), each piece's ENTRY cue on the last
/// one's EXIT cue and its tail ringing over. The fight's end fades it out and pauses it in place;
/// the next fight goes on from there - or, after `music_new_suite` s without a fight, starts
/// another suite. Live: `music_suites` (empty = all), `music_skip` ("suite/piece").
fn init_music_doom() {
    std::thread::spawn(|| {
        let suites = load_suites();
        if suites.is_empty() {
            log::warn!("music: {} missing - run tools/convert_music.py", config::mod_dir().join("doom_music").display());
            return;
        }
        log::info!("music: Doom mode, {} suites", suites.len());
        let rate = device_rate();
        let Ok((_stream, handle)) = OutputStream::try_default() else { return };
        let state: Arc<std::sync::Mutex<MusicState>> = Arc::default();
        state.lock().unwrap().paused = true;
        if let Err(e) = handle.play_raw(MusicMixer { state: state.clone(), rate, buf: Vec::new(), at: 0, lim: 1.0 }) {
            log::error!("music: not started: {e}");
            return;
        }
        let ms = |v: &serde_json::Value| (v.as_f64().unwrap_or(0.0) / 1000.0 * rate as f64) as u64;
        let mut run: Option<Run> = None;
        let mut last_suite = usize::MAX;
        let mut vol = 0.0f32;
        let mut quiet_since = std::time::Instant::now();
        let set_status = |s: Option<String>| {
            if let Ok(mut m) = MUSIC_STATUS.lock() {
                *m = s;
            }
        };
        loop {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let cfg = config::get_cached();
            let test = MUSIC_TEST.load(Ordering::Relaxed);
            let combat = (COMBAT.load(Ordering::Relaxed) && cfg.music) || test;
            let enabled: Vec<usize> = (0..suites.len())
                .filter(|&i| cfg.music_suites.is_empty() || cfg.music_suites.iter().any(|s| s == &suites[i].name))
                .collect();
            let skip_of = |s: &Suite| -> Vec<String> {
                cfg.music_skip.iter().filter_map(|e| e.split_once('/').filter(|(a, _)| *a == s.name).map(|(_, p)| p.to_string())).collect()
            };
            let start_suite = |i: usize| -> Option<Run> {
                let s = &suites[i];
                let walker = Walker::new(&s.json["tree"], &skip_of(s), cfg.music_intro)?;
                log::info!("music: suite {}", s.name);
                let db = s.json["gain_db"].as_f64().unwrap_or(0.0) as f32;
                Some(Run { suite: i, walker, cue: None, gain: 10f32.powf(db.clamp(-12.0, 12.0) / 20.0), next: None })
            };
            let cut_all = |st: &mut MusicState| {
                let now = st.clock;
                st.voices.retain(|v| v.start <= now);
                for v in st.voices.iter_mut() {
                    v.fade.get_or_insert((rate / 5, rate / 5));
                }
            };
            // test keys
            match MUSIC_CMD.swap(0, Ordering::Relaxed) {
                c @ (1 | 2) if !enabled.is_empty() => {
                    // F6 the next suite / F5 this suite from the top
                    let cur = run.as_ref().map_or(enabled[0], |r| r.suite);
                    let pos = enabled.iter().position(|&i| i == cur).unwrap_or(0);
                    let i = if c == 1 && run.is_some() { enabled[(pos + 1) % enabled.len()] } else { cur };
                    cut_all(&mut state.lock().unwrap());
                    run = start_suite(i);
                    last_suite = i;
                }
                c @ (4 | 5) => {
                    // F4 the next piece now / F3 skip the playing piece for good, then the next
                    let mut st = state.lock().unwrap();
                    if let Some(r) = run.as_mut() {
                        let now = st.clock;
                        if c == 5 {
                            if let Some(v) = st.voices.iter().filter(|v| v.start <= now && v.fade.is_none()).max_by_key(|v| v.start) {
                                let entry = format!("{}/{}", suites[r.suite].name, v.name);
                                if !cfg.music_skip.contains(&entry) {
                                    let mut list = cfg.music_skip.clone();
                                    list.push(entry.clone());
                                    let items: Vec<String> = list.iter().map(|e| format!("\"{e}\"")).collect();
                                    config::save_values(&[("", "music_skip", format!("[{}]", items.join(", ")))]);
                                    log::info!("music: skipping {entry} from now on");
                                }
                                if r.next.as_ref().is_some_and(|n| n.0 == v.name) {
                                    r.next = None;
                                }
                            }
                        }
                        cut_all(&mut st);
                        r.cue = None;
                    }
                }
                _ => {}
            }
            if combat {
                // a new suite: at the start, or after a long quiet spell
                let long = cfg.music_new_suite > 0.0 && quiet_since.elapsed().as_secs_f32() > cfg.music_new_suite;
                if (run.is_none() || (long && vol <= 0.0)) && !enabled.is_empty() {
                    let mut i = enabled[fastrand::usize(..enabled.len())];
                    if enabled.len() > 1 && i == last_suite {
                        i = enabled[(enabled.iter().position(|&e| e == i).unwrap() + 1) % enabled.len()];
                    }
                    let mut st = state.lock().unwrap();
                    st.voices.clear();
                    drop(st);
                    run = start_suite(i);
                    last_suite = i;
                }
                quiet_since = std::time::Instant::now();
            }
            // schedule: the next piece once its lead-in is due within 2 s
            if let Some(r) = run.as_mut().filter(|_| combat) {
                let s = &suites[r.suite];
                if r.next.is_none() {
                    // (a piece skipped since the suite started is left out here)
                    let skip = skip_of(s);
                    for _ in 0..64 {
                        let Some(name) = r.walker.next() else { break };
                        if skip.contains(&name) {
                            continue;
                        }
                        let p = &s.json["pieces"][&name];
                        match decode_piece(&s.dir, &name, rate) {
                            Some(pcm) => r.next = Some((name, pcm, ms(&p["entry"]), ms(&p["exit"]))),
                            None => log::warn!("music: bad piece {}/{name}", s.name),
                        }
                        break;
                    }
                }
                let mut st = state.lock().unwrap();
                let now = st.clock;
                // its ENTRY cue on the last piece's EXIT cue (the first one: right away)
                let start = r.next.as_ref().map(|n| r.cue.map_or(now, |c| c.saturating_sub(n.2).max(now)));
                if let Some(start) = start.filter(|&t| t <= now + 2 * rate as u64) {
                    let (name, pcm, _, exit) = r.next.take().unwrap();
                    log::info!("music: {}/{name}", s.name);
                    r.cue = Some(start + exit);
                    st.voices.push(Placed { name, pcm, start, gain: r.gain, fade: None });
                }
            }
            // fade in over ~1.5 s, out over ~4 s, then paused in place
            let target = if combat { cfg.music_volume } else { 0.0 };
            let step = if target > vol { 0.02 / 1.5 } else { 0.02 / 4.0 } * cfg.music_volume.max(0.05);
            vol = if target > vol { (vol + step).min(target) } else { (vol - step).max(target) };
            {
                let mut st = state.lock().unwrap();
                st.target = vol;
                if combat {
                    st.paused = false;
                } else if vol <= 0.0 {
                    st.paused = true;
                }
                set_status(run.as_ref().map(|r| {
                    let now = st.clock;
                    let playing = st.voices.iter().filter(|v| v.start <= now && v.fade.is_none()).max_by_key(|v| v.start);
                    match playing {
                        Some(v) => format!(
                            "{}  {}  {} / {}",
                            suites[r.suite].name.to_uppercase(),
                            v.name.to_uppercase(),
                            mmss((now - v.start) as f64 / rate as f64),
                            mmss((v.pcm.len() / 2) as f64 / rate as f64)
                        ),
                        None => suites[r.suite].name.to_uppercase(),
                    }
                }));
            }
        }
    });
}

/// Fight music, track mode (user, 2026-10-08): one full track per fight from its start point
/// (the drop), on to the next track when it ends; the fight's end fades it out and pauses it in
/// place, the next fight fades back in from there.
fn init_music_tracks() {
    std::thread::spawn(|| {
        let Ok((_stream, handle)) = OutputStream::try_default() else { return };
        let Ok(sink) = rodio::Sink::try_new(&handle) else { return };
        sink.pause();
        log::info!("music: track mode, {} tracks in the playlist", playlist(&config::get_cached()).len());
        // the playing track: playlist index, where the source playing started (s), its length
        let mut cur: Option<(usize, f64, f64)> = None;
        let mut queued = 0usize;
        let mut vol = 0.0f32;
        let set_status = |s: Option<String>| {
            if let Ok(mut m) = MUSIC_STATUS.lock() {
                *m = s;
            }
        };
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            let cfg = config::get_cached();
            let list = playlist(&cfg);
            if list.is_empty() {
                continue;
            }
            let test = MUSIC_TEST.load(Ordering::Relaxed);
            let combat = (COMBAT.load(Ordering::Relaxed) && cfg.music) || test;
            // start (or restart) a track at a position
            let mut play_at = |i: usize, at: f64, sink: &rodio::Sink, cur: &mut Option<(usize, f64, f64)>, queued: &mut usize| {
                let i = i % list.len();
                if let Some((src, len)) = open_track(&list[i], at) {
                    sink.clear();
                    sink.append(src);
                    sink.play();
                    *cur = Some((i, at, len));
                    *queued = 1;
                    log::info!("music: {} from {}", list[i].name, mmss(at));
                }
            };
            let pos = cur.map(|(_, base, _)| base + sink.get_pos().as_secs_f64());
            // test keys
            match MUSIC_CMD.swap(0, Ordering::Relaxed) {
                1 => {
                    let n = cur.map_or(0, |c| (c.0 + 1) % list.len());
                    play_at(n, list[n].start, &sink, &mut cur, &mut queued);
                }
                2 => {
                    if let Some((i, ..)) = cur {
                        play_at(i, list[i].start, &sink, &mut cur, &mut queued);
                    }
                }
                c @ (3 | 4) => {
                    if let (Some((i, ..)), Some(p)) = (cur, pos) {
                        let at = (p + if c == 3 { -5.0 } else { 5.0 }).max(0.0);
                        play_at(i, at, &sink, &mut cur, &mut queued);
                    }
                }
                5 => {
                    if let (Some((i, ..)), Some(p)) = (cur, pos) {
                    // the start point: where it was marked, a quarter second back for the reaction
                    let at = (p - 0.25).max(0.0);
                    let entries: Vec<String> = list
                        .iter()
                        .enumerate()
                        .map(|(k, t)| format!("\"{}@{:.1}\"", t.name, if k == i { at } else { t.start }))
                        .collect();
                    config::save_values(&[("", "music_tracks", format!("[{}]", entries.join(", ")))]);
                    log::info!("music: {} start point set to {}", list[i].name, mmss(at));
                    }
                }
                _ => {}
            }
            if combat {
                match cur {
                    None => {
                        let i = fastrand::usize(..list.len());
                        play_at(i, list[i].start, &sink, &mut cur, &mut queued);
                    }
                    Some((i, _, _)) if sink.empty() => {
                        // the track ended: the next one, from its start point
                        let n = (i + 1) % list.len();
                        play_at(n, list[n].start, &sink, &mut cur, &mut queued);
                    }
                    _ => {}
                }
                if sink.is_paused() {
                    sink.play();
                }
            }
            let _ = queued;
            // fade: in over ~1.5 s, out over ~4 s, then paused in place
            let target = if combat { cfg.music_volume } else { 0.0 };
            let step = if target > vol { 0.05 / 1.5 } else { 0.05 / 4.0 } * cfg.music_volume.max(0.05);
            vol = if target > vol { (vol + step).min(target) } else { (vol - step).max(target) };
            sink.set_volume(vol);
            if !combat && vol <= 0.0 && !sink.is_paused() {
                sink.pause();
            }
            set_status(cur.map(|(i, base, len)| {
                let p = base + sink.get_pos().as_secs_f64();
                format!("{}  {} / {}  START {}", list.get(i).map_or("?", |t| t.name.as_str()).to_uppercase(), mmss(p), mmss(len), mmss(list.get(i).map_or(0.0, |t| t.start)))
            }));
        }
    });
}


#[cfg(test)]
mod tests {
    #[test]
    fn walk_suites() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../dist/natives/doom_music");
        for name in ["metal_hell", "doom_hunter", "mars_core_phobos"] {
            let json: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(root.join(name).join("suite.json")).unwrap()).unwrap();
            for intro in [false, true] {
                let mut w = super::Walker::new(&json["tree"], &["heavy_9".to_string()], intro).unwrap();
                let seq: Vec<String> = (0..16).filter_map(|_| w.next()).collect();
                assert_eq!(seq.len(), 16);
                assert!(!seq.contains(&"heavy_9".to_string()));
                println!("{name} intro {intro}: {}", seq.join(" "));
            }
        }
    }
}
