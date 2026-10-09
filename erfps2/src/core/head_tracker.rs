use fromsoftware_shared::F32ModelMatrix;
use std::sync::atomic::{AtomicU32, Ordering};

use glam::{Mat4, Quat, Vec3};

use crate::{
    core::{
        BehaviorState, CoreLogicContext, frame_cached::FrameCache, stabilizer::CameraStabilizer,
        world::World,
    },
    player::PlayerExt,
};

#[derive(Default)]
pub struct HeadTracker {
    last: Option<Quat>,
    rotation: Quat,
    rotation_target: Quat,
    stabilizer: CameraStabilizer,
    output: Option<Output>,
}

pub struct Args {
    pub model_matrix: F32ModelMatrix,
    pub head_matrix: F32ModelMatrix,
    pub stabilizer_factor: f32,
    pub use_stabilizer: bool,
    pub is_tracked: bool,
}

pub struct Output {
    pub tracking_rotation: Quat,
    pub stabilized_head_position: Vec3,
    pub head_matrix: F32ModelMatrix,
}

impl HeadTracker {
    pub fn set_stabilizer_window(&mut self, window: f32) {
        self.stabilizer.set_window(window);
    }

    fn rotate_towards_target(&mut self, frame_time: f32) {
        let distance = self.rotation.angle_between(self.rotation_target);
        let step = rip(distance, 0.0, 1.0, frame_time);

        self.rotation = self.rotation.rotate_towards(self.rotation_target, step);
    }
}

impl FrameCache for HeadTracker {
    type Input = Args;
    type Output<'a> = &'a Output;

    fn update(&mut self, frame_time: f32, args: Self::Input) -> Self::Output<'_> {
        let mut head_position = args.head_matrix.translation();

        if args.use_stabilizer || true {
            let player_matrix = Mat4::from(args.model_matrix);

            let mut local_head_pos = player_matrix.inverse().project_point3(head_position);
            HEAD_LOCAL_Y.store(local_head_pos.y.to_bits(), Ordering::Relaxed);
            HEAD_LOCAL_X.store(local_head_pos.x.to_bits(), Ordering::Relaxed);
            HEAD_LOCAL_Z.store(local_head_pos.z.to_bits(), Ordering::Relaxed);

            if args.use_stabilizer {
                let stabilized = self.stabilizer.update(frame_time, local_head_pos);
                let delta = stabilized - local_head_pos;

                local_head_pos += delta.clamp_length_max(args.stabilizer_factor * 0.1);
            }


            head_position = player_matrix.project_point3(local_head_pos);
        }

        let input = Quat::from_mat3a(&args.head_matrix.rotation());

        if args.is_tracked
            && let Some(last) = self.last
        {
            self.rotation_target *= last.inverse() * input;
            self.rotation_target = self.rotation_target.normalize();
        } else {
            self.rotation_target = Quat::IDENTITY;
        }

        self.last = Some(input);
        self.rotate_towards_target(frame_time);

        self.output.insert(Output {
            tracking_rotation: self.rotation,
            stabilized_head_position: head_position,
            head_matrix: args.head_matrix,
        })
    }

    fn get_cached(&mut self, _frame_time: f32, _input: Self::Input) -> Self::Output<'_> {
        self.output.as_ref().expect("FrameCache logic error")
    }

    fn reset(&mut self) {
        self.stabilizer.reset();
        self.last = None;
    }
}

impl From<&CoreLogicContext<'_, World<'_>>> for Args {
    fn from(context: &CoreLogicContext<'_, World<'_>>) -> Self {
        let head_matrix = context.player.head_matrix();
        let model_matrix = context.player.model_matrix();

        let is_tracked = context.player.is_in_throw()
            || (context.config.track_damage && context.has_state(BehaviorState::Damage))
            || (context.config.track_dodges && context.has_state(BehaviorState::Evasion));

        Self {
            head_matrix,
            model_matrix,
            stabilizer_factor: context.config.stabilizer_factor,
            use_stabilizer: context.config.use_stabilizer,
            is_tracked,
        }
    }
}

/**
    Computes a signed distance step that moves `distance` toward 0 over the next `timedelta`.

      Curve: d(t) = (t * b)^6 - a
    Inverse: t(d) = (d + a)^(1/6) / b
       Step:        d(t) - d(t-Δt)

    Method:
    - Interpret `distance` as the remaining distance to zero, offset by `curve_offset`.
    - Convert remaining distance -> remaining time using t(d), scaled by `curve_scale`.
    - Advance time by `timedelta` and map back using d(t) to get the new remaining distance.
    - Return step = distance - distance_new, clamped to \[0, distance\].
*/
fn rip(distance: f32, curve_offset: f32, curve_scale: f32, timedelta: f32) -> f32 {
    let sign = distance.signum();
    let distance = distance.abs();

    let time_remaining = (distance + curve_offset).powf(1.0 / 6.0) / curve_scale;
    let time_new = (time_remaining - timedelta).max(0.0);

    let distance_new = (time_new * curve_scale).powi(6) - curve_offset;

    let step = (distance - distance_new).max(0.0).min(distance);

    step * sign
}

/// Head position relative to the model this frame (before any eye lock), for DOOM RING to read.
static HEAD_LOCAL_Y: AtomicU32 = AtomicU32::new(0);
static HEAD_LOCAL_X: AtomicU32 = AtomicU32::new(0);
static HEAD_LOCAL_Z: AtomicU32 = AtomicU32::new(0);
/// Locked eye x/z (model-local); NaN = keep the head's.
static EYE_X: AtomicU32 = AtomicU32::new(0x7fc0_0000);
static EYE_Z: AtomicU32 = AtomicU32::new(0x7fc0_0000);
/// Eye height DOOM RING wants (model-local metres); NaN = follow the head bone.
static EYE_LOCK: AtomicU32 = AtomicU32::new(0x7fc0_0000);
/// Blend weight of the lock (eases in/out over ~0.2 s so ladders etc. don't pop).
static EYE_WEIGHT: AtomicU32 = AtomicU32::new(0);
static EYE_LAST: AtomicU32 = AtomicU32::new(0x7fc0_0000);

/// DOOM RING eye lock: the eye position on the model (model-local) and its blend weight (eases
/// in/out over ~0.1 s, time-based so the two camera calls per frame don't double-step it).
pub fn doom_eye() -> Option<(Vec3, f32)> {
    static LAST_T: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    let now = std::time::Instant::now();
    let dt = {
        let mut g = LAST_T.lock().unwrap_or_else(|e| e.into_inner());
        let dt = g.map_or(0.0, |t| (now - t).as_secs_f32()).min(0.1);
        *g = Some(now);
        dt
    };
    let want = f32::from_bits(EYE_LOCK.load(Ordering::Relaxed));
    if want.is_finite() {
        EYE_LAST.store(want.to_bits(), Ordering::Relaxed);
    }
    let target = if want.is_finite() { 1.0 } else { 0.0 };
    let w = f32::from_bits(EYE_WEIGHT.load(Ordering::Relaxed));
    let step = dt * 10.0;
    let w = if target > w { (w + step).min(1.0) } else { (w - step).max(0.0) };
    EYE_WEIGHT.store(w.to_bits(), Ordering::Relaxed);
    let y = f32::from_bits(EYE_LAST.load(Ordering::Relaxed));
    let (x, z) = (f32::from_bits(EYE_X.load(Ordering::Relaxed)), f32::from_bits(EYE_Z.load(Ordering::Relaxed)));
    let (x, z) = if x.is_finite() && z.is_finite() { (x, z) } else { (0.0, 0.06) };
    (w > 0.0 && y.is_finite()).then_some((Vec3::new(x, y, z), w))
}

/// # Safety
/// Plain value store; callable from any thread. NaN releases the lock.
#[unsafe(no_mangle)]
pub extern "C" fn erfps2_set_eye_height(height: f32) {
    EYE_LOCK.store(height.to_bits(), Ordering::Relaxed);
}

/// # Safety
/// Plain value load; callable from any thread.
#[unsafe(no_mangle)]
pub extern "C" fn erfps2_head_local_y() -> f32 {
    f32::from_bits(HEAD_LOCAL_Y.load(Ordering::Relaxed))
}

/// # Safety
/// Plain value store; callable from any thread. NaN keeps the head's own x/z.
#[unsafe(no_mangle)]
pub extern "C" fn erfps2_set_eye_xz(x: f32, z: f32) {
    EYE_X.store(x.to_bits(), Ordering::Relaxed);
    EYE_Z.store(z.to_bits(), Ordering::Relaxed);
}

/// # Safety
/// `out` must point to 3 writable f32s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn erfps2_head_local_xyz(out: *mut f32) {
    if out.is_null() {
        return;
    }
    let v = [
        f32::from_bits(HEAD_LOCAL_X.load(Ordering::Relaxed)),
        f32::from_bits(HEAD_LOCAL_Y.load(Ordering::Relaxed)),
        f32::from_bits(HEAD_LOCAL_Z.load(Ordering::Relaxed)),
    ];
    unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), out, 3) };
}
