//! Lightweight animation timeline for UI transitions (GUI_research.md §6.6).
//!
//! A `Tween` advances a normalized `t ∈ [0, 1]` over a fixed duration, applies an
//! easing curve, and interpolates between two endpoints. It is pure CPU logic
//! (no allocation, no GPU) so it is fully unit-testable; widgets drive epoch
//! reactivity by writing the sampled value into a `Signal` each frame and only
//! mark their subtree dirty while the tween is still running.

/// Easing curves. Kept as a small `Copy` enum (no `Box<dyn Fn>`), consistent
/// with the engine's no-dynamic-dispatch-on-hot-paths rule.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum Easing {
    #[default]
    Linear,
    EaseInQuad,
    EaseOutQuad,
    EaseInOutQuad,
}

impl Easing {
    /// Map linear progress `t ∈ [0,1]` through the curve. Endpoints are fixed
    /// points: `apply(0) == 0`, `apply(1) == 1`.
    #[inline]
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            Easing::Linear => t,
            Easing::EaseInQuad => t * t,
            Easing::EaseOutQuad => t * (2.0 - t),
            Easing::EaseInOutQuad => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    let u = 2.0 * t - 1.0;
                    -0.5 * (u * (u - 2.0) - 1.0)
                }
            }
        }
    }
}

/// A finite f32 tween between `from` and `to` over `duration` seconds.
#[derive(Copy, Clone, Debug)]
pub struct Tween {
    from: f32,
    to: f32,
    duration: f32,
    elapsed: f32,
    easing: Easing,
}

impl Tween {
    pub fn new(from: f32, to: f32, duration: f32, easing: Easing) -> Self {
        Self { from, to, duration, elapsed: 0.0, easing }
    }

    /// Advance by `dt` seconds; returns the current sampled value. Once finished
    /// the value stays pinned at `to`.
    #[inline]
    pub fn advance(&mut self, dt: f32) -> f32 {
        self.elapsed = (self.elapsed + dt).min(self.duration.max(0.0));
        self.value()
    }

    /// Current interpolated value without advancing time.
    #[inline]
    pub fn value(&self) -> f32 {
        let t = if self.duration <= 0.0 { 1.0 } else { self.elapsed / self.duration };
        let e = self.easing.apply(t);
        self.from + (self.to - self.from) * e
    }

    /// Normalized, un-eased progress in `[0, 1]`.
    #[inline]
    pub fn progress(&self) -> f32 {
        if self.duration <= 0.0 {
            1.0
        } else {
            (self.elapsed / self.duration).clamp(0.0, 1.0)
        }
    }

    /// Whether the tween has reached its end (drives dirty-subtree gating: a
    /// finished tween stops dirtying its widget).
    #[inline]
    pub fn finished(&self) -> bool {
        self.elapsed >= self.duration
    }

    pub fn reset(&mut self) {
        self.elapsed = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_endpoints_are_fixed() {
        for e in [Easing::Linear, Easing::EaseInQuad, Easing::EaseOutQuad, Easing::EaseInOutQuad] {
            assert!((e.apply(0.0) - 0.0).abs() < 1e-6, "{e:?} at 0");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-6, "{e:?} at 1");
        }
        // Out-of-range input is clamped, not extrapolated.
        assert_eq!(Easing::Linear.apply(-1.0), 0.0);
        assert_eq!(Easing::Linear.apply(2.0), 1.0);
    }

    #[test]
    fn tween_interpolates_and_pins_at_end() {
        let mut t = Tween::new(0.0, 10.0, 1.0, Easing::Linear);
        assert_eq!(t.value(), 0.0);
        assert!((t.advance(0.5) - 5.0).abs() < 1e-6);
        assert!(!t.finished());
        assert!((t.advance(0.5) - 10.0).abs() < 1e-6);
        assert!(t.finished());
        // Overshooting time keeps the value pinned at `to`.
        assert!((t.advance(5.0) - 10.0).abs() < 1e-6);
    }

    #[test]
    fn zero_duration_is_instantly_done() {
        let mut t = Tween::new(2.0, 8.0, 0.0, Easing::EaseInOutQuad);
        assert!(t.finished());
        assert!((t.value() - 8.0).abs() < 1e-6);
        assert_eq!(t.progress(), 1.0);
        assert!((t.advance(0.016) - 8.0).abs() < 1e-6);
    }

    #[test]
    fn ease_in_quad_is_below_linear_midway() {
        // Quadratic ease-in starts slower than linear.
        assert!(Easing::EaseInQuad.apply(0.5) < Easing::Linear.apply(0.5));
        assert!(Easing::EaseOutQuad.apply(0.5) > Easing::Linear.apply(0.5));
    }
}
