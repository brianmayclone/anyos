//! Shared animation API for anyOS.
//!
//! Provides easing functions, single-value animation interpolation, color
//! blending, and a set-based manager for tracking multiple concurrent
//! animations.  All math is integer-only (no floats) and all timing uses
//! PIT ticks obtained via [`crate::sys::uptime`] / [`crate::sys::tick_hz`].
//!
//! # Quick start
//! ```ignore
//! use anyos_std::anim::{Anim, AnimSet, Easing};
//!
//! let a = Anim::new(0, 1000, 300, Easing::EaseOut); // 0→1000 over 300ms
//! // each frame:
//! let now = anyos_std::sys::uptime();
//! let v = a.value(now); // interpolated value
//! ```

use alloc::vec::Vec;

// ── Easing ──────────────────────────────────────────────────────────────────

/// Easing mode for interpolation.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Easing {
    /// Constant speed.
    Linear,
    /// Slow start, fast end (quadratic).
    EaseIn,
    /// Fast start, slow end (quadratic).
    EaseOut,
}

/// Apply easing to a linear progress value.
///
/// `t` is in the range 0..1000 (fixed-point, 1000 = 100%).
/// Returns a value in the same range with the easing curve applied.
fn apply_easing(t: u32, easing: Easing) -> u32 {
    match easing {
        Easing::Linear => t,
        // Quadratic ease-in: f(t) = t^2
        Easing::EaseIn => t * t / 1000,
        // Quadratic ease-out: f(t) = 1 - (1-t)^2
        Easing::EaseOut => {
            let inv = 1000 - t;
            1000 - inv * inv / 1000
        }
    }
}

// ── Anim ────────────────────────────────────────────────────────────────────

/// A single animation that interpolates an `i32` value from `from` to `to`
/// over a given duration using the specified easing curve.
///
/// Values are in caller-defined units.  For pixel offsets use raw integers;
/// for sub-pixel precision use fixed-point (e.g. multiply by 1000).
#[derive(Clone)]
pub struct Anim {
    /// Start value.
    pub from: i32,
    /// End value.
    pub to: i32,
    /// Easing curve.
    pub easing: Easing,
    /// PIT tick when the animation was created.
    start_tick: u32,
    /// Duration expressed in PIT ticks.
    duration_ticks: u32,
    /// Cached PIT tick rate (Hz).
    tick_hz: u32,
}

impl Anim {
    /// Create a new animation that starts **now**.
    ///
    /// * `from` / `to` — value range (any units)
    /// * `duration_ms` — animation length in milliseconds
    /// * `easing` — interpolation curve
    pub fn new(from: i32, to: i32, duration_ms: u32, easing: Easing) -> Self {
        let hz = crate::sys::tick_hz().max(1);
        let ticks = (duration_ms as u64 * hz as u64 / 1000) as u32;
        Anim {
            from,
            to,
            easing,
            start_tick: crate::sys::uptime(),
            duration_ticks: ticks.max(1),
            tick_hz: hz,
        }
    }

    /// Create an animation with an explicit start tick (for batched starts).
    pub fn new_at(from: i32, to: i32, duration_ms: u32, easing: Easing, start: u32) -> Self {
        let hz = crate::sys::tick_hz().max(1);
        let ticks = (duration_ms as u64 * hz as u64 / 1000) as u32;
        Anim {
            from,
            to,
            easing,
            start_tick: start,
            duration_ticks: ticks.max(1),
            tick_hz: hz,
        }
    }

    /// Linear progress in 0..1000 (fixed-point).
    pub fn progress(&self, now_tick: u32) -> u32 {
        let elapsed = now_tick.wrapping_sub(self.start_tick);
        if elapsed >= self.duration_ticks {
            return 1000;
        }
        // p = elapsed * 1000 / duration  (fits in u64)
        (elapsed as u64 * 1000 / self.duration_ticks as u64) as u32
    }

    /// Current interpolated value.  Returns `to` once done.
    pub fn value(&self, now_tick: u32) -> i32 {
        let p = self.progress(now_tick);
        let t = apply_easing(p, self.easing);
        // lerp: from + (to - from) * t / 1000
        let delta = self.to as i64 - self.from as i64;
        (self.from as i64 + delta * t as i64 / 1000) as i32
    }

    /// Returns `true` when the animation has finished.
    pub fn done(&self, now_tick: u32) -> bool {
        now_tick.wrapping_sub(self.start_tick) >= self.duration_ticks
    }
}

// ── Color Blend ─────────────────────────────────────────────────────────────

/// Blend two ARGB colours by progress `t` (0..1000).
///
/// `t = 0` → pure `c1`, `t = 1000` → pure `c2`.
pub fn color_blend(c1: u32, c2: u32, t: u32) -> u32 {
    let t = t.min(1000);
    let inv = 1000 - t;
    let a1 = (c1 >> 24) & 0xFF;
    let r1 = (c1 >> 16) & 0xFF;
    let g1 = (c1 >> 8) & 0xFF;
    let b1 = c1 & 0xFF;
    let a2 = (c2 >> 24) & 0xFF;
    let r2 = (c2 >> 16) & 0xFF;
    let g2 = (c2 >> 8) & 0xFF;
    let b2 = c2 & 0xFF;
    let a = (a1 * inv + a2 * t) / 1000;
    let r = (r1 * inv + r2 * t) / 1000;
    let g = (g1 * inv + g2 * t) / 1000;
    let b = (b1 * inv + b2 * t) / 1000;
    (a << 24) | (r << 16) | (g << 8) | b
}

// ── AnimSet ─────────────────────────────────────────────────────────────────

/// Manages multiple concurrent animations keyed by a `u32` identifier.
///
/// Starting an animation with an existing ID replaces the previous one.
pub struct AnimSet {
    anims: Vec<(u32, Anim)>,
}

impl AnimSet {
    /// Create an empty animation set.
    pub fn new() -> Self {
        AnimSet {
            anims: Vec::with_capacity(8),
        }
    }

    /// Start (or replace) an animation for the given `id`.
    pub fn start(&mut self, id: u32, from: i32, to: i32, duration_ms: u32, easing: Easing) {
        let anim = Anim::new(from, to, duration_ms, easing);
        if let Some(entry) = self.anims.iter_mut().find(|(k, _)| *k == id) {
            entry.1 = anim;
        } else {
            self.anims.push((id, anim));
        }
    }

    /// Start (or replace) with an explicit start tick.
    pub fn start_at(
        &mut self,
        id: u32,
        from: i32,
        to: i32,
        duration_ms: u32,
        easing: Easing,
        start: u32,
    ) {
        let anim = Anim::new_at(from, to, duration_ms, easing, start);
        if let Some(entry) = self.anims.iter_mut().find(|(k, _)| *k == id) {
            entry.1 = anim;
        } else {
            self.anims.push((id, anim));
        }
    }

    /// Get the current interpolated value for an animation, or `None` if no
    /// animation with that ID exists.
    pub fn value(&self, id: u32, now: u32) -> Option<i32> {
        self.anims
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, a)| a.value(now))
    }

    /// Returns the current value, or `default` if the animation doesn't exist
    /// or has completed.
    pub fn value_or(&self, id: u32, now: u32, default: i32) -> i32 {
        match self.anims.iter().find(|(k, _)| *k == id) {
            Some((_, a)) => a.value(now),
            None => default,
        }
    }

    /// Check whether a specific animation is still running.
    pub fn is_active(&self, id: u32, now: u32) -> bool {
        self.anims
            .iter()
            .any(|(k, a)| *k == id && !a.done(now))
    }

    /// Returns `true` if **any** animation in the set is still running.
    pub fn has_active(&self, now: u32) -> bool {
        self.anims.iter().any(|(_, a)| !a.done(now))
    }

    /// Remove all finished animations (garbage collection).
    pub fn remove_done(&mut self, now: u32) {
        self.anims.retain(|(_, a)| !a.done(now));
    }

    /// Remove a specific animation by ID.
    pub fn remove(&mut self, id: u32) {
        self.anims.retain(|(k, _)| *k != id);
    }

    /// Number of tracked animations (including finished ones).
    pub fn len(&self) -> usize {
        self.anims.len()
    }
}
