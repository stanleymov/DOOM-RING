use std::{
    hint::assert_unchecked,
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use portable_atomic::AtomicU128;

pub struct AtomicDuration(SignedAtomicDuration);

pub struct AtomicInstant(SignedAtomicDuration, Instant);

struct SignedAtomicDuration(AtomicU128);

#[derive(Clone, Copy, Debug, Default)]
struct SignedDuration {
    secs: u64,
    nanos: i32,
}

#[derive(Clone, Copy, Debug)]
enum Sign {
    Neg,
    Pos,
}

impl AtomicDuration {
    pub fn new(dur: Duration) -> Self {
        Self(SignedAtomicDuration::new(dur.into()))
    }

    pub fn load(&self, order: Ordering) -> Duration {
        self.0.load(order).into_duration_and_sign().0
    }

    pub fn store(&self, dur: Duration, order: Ordering) {
        self.0.store(SignedDuration::from(dur), order);
    }
}

impl AtomicInstant {
    pub fn new(instant: Instant) -> Self {
        Self(SignedAtomicDuration::new(Default::default()), instant)
    }

    pub fn load(&self, order: Ordering) -> Instant {
        // SAFETY: always safe.
        // This reverses `SignedDuration::from_instant_difference(instant, self.1)`:
        // `self.1 + (instant - self.1)` -> `instant`.
        unsafe { self.0.load(order).add_to_instant_unchecked(self.1) }
    }

    pub fn store(&self, instant: Instant, order: Ordering) {
        let signed_duration = SignedDuration::from_instant_difference(instant, self.1);
        self.0.store(signed_duration, order)
    }
}

impl SignedAtomicDuration {
    fn new(signed_duration: SignedDuration) -> Self {
        Self(AtomicU128::new(signed_duration.into()))
    }

    fn load(&self, order: Ordering) -> SignedDuration {
        // SAFETY: `signed_duration.into()` calls `SignedDuration::into_u128`,
        // the only valid output for `SignedDuration::from_u128`.
        unsafe { SignedDuration::from_u128(self.0.load(order)) }
    }

    fn store(&self, signed_duration: SignedDuration, order: Ordering) {
        self.0.store(signed_duration.into(), order);
    }
}

impl SignedDuration {
    fn from_duration_and_sign(dur: Duration, sign: Sign) -> Self {
        let secs = dur.as_secs();
        let nanos = dur.subsec_nanos() as i32;
        match sign {
            Sign::Pos => Self { secs, nanos },
            Sign::Neg => Self {
                secs,
                nanos: -nanos,
            },
        }
    }

    fn from_instant_difference(lhs: Instant, rhs: Instant) -> Self {
        // SAFETY: never `None` because the lesser (or equal) instant is subtracted from
        // the greater (or equal) instant.
        if lhs < rhs {
            let dur = unsafe { rhs.checked_duration_since(lhs).unwrap_unchecked() };
            SignedDuration::from_duration_and_sign(dur, Sign::Neg)
        } else {
            let dur = unsafe { lhs.checked_duration_since(rhs).unwrap_unchecked() };
            SignedDuration::from_duration_and_sign(dur, Sign::Pos)
        }
    }

    fn into_duration_and_sign(self) -> (Duration, Sign) {
        let secs = self.secs;
        let nanos = self.nanos.unsigned_abs();

        // SAFETY: because `Self::from_u128` is unsafe, we can assume this `SignedDuration`
        // was derived from a valid `Duration`.
        unsafe {
            const NANOS_PER_SEC: u32 = 1_000_000_000;
            assert_unchecked(nanos < NANOS_PER_SEC);
        }

        let dur = Duration::new(secs, nanos);
        let sign = if self.nanos < 0 { Sign::Neg } else { Sign::Pos };

        (dur, sign)
    }

    fn into_u128(self) -> u128 {
        self.secs as u128 | (self.nanos.cast_unsigned() as u128) << 64
    }

    /// # Safety
    ///
    /// Can only use the result of [`Self::into_u128`] as input.
    unsafe fn from_u128(value: u128) -> Self {
        let secs = value as u64;
        let nanos = ((value >> 64) as u32).cast_signed();
        Self { secs, nanos }
    }

    /// # Safety
    ///
    /// The arithmetic operation *must not overflow*.
    unsafe fn add_to_instant_unchecked(self, lhs: Instant) -> Instant {
        match self.into_duration_and_sign() {
            (dur, Sign::Neg) => unsafe { lhs.checked_sub(dur).unwrap_unchecked() },
            (dur, Sign::Pos) => unsafe { lhs.checked_add(dur).unwrap_unchecked() },
        }
    }
}

impl From<Duration> for SignedDuration {
    fn from(dur: Duration) -> Self {
        SignedDuration::from_duration_and_sign(dur, Sign::Pos)
    }
}

impl From<SignedDuration> for u128 {
    fn from(value: SignedDuration) -> Self {
        value.into_u128()
    }
}
