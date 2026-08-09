//! # tpt-control-ratelimit
//!
//! Slew-rate limiters and ramp generators that protect actuators from mechanical shock —
//! e.g. preventing valve water hammer or motor inrush currents by limiting how fast a setpoint
//! may move.
//!
//! `no_std` + `alloc`: pure logic, no inherent I/O.

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

use core::fmt;
use tpt_control_action::{Bounds, Setpoint};
use tpt_control_audit::ReasonKind;

/// A per-actuator rate profile, derived from actuator metadata.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RateProfile {
    /// Maximum rate of change, in setpoint units per millisecond.
    pub max_rate_per_ms: f64,
    /// Optional total ramp duration for a full staged transition (ms).
    pub ramp_total_ms: Option<u64>,
}

impl RateProfile {
    /// Build a profile from actuator bounds and the time to traverse full scale.
    pub fn from_bounds(bounds: Bounds, full_traverse_ms: u64) -> Self {
        let span = (bounds.max - bounds.min).abs();
        let rate =
            if full_traverse_ms == 0 { f64::INFINITY } else { span / full_traverse_ms as f64 };
        RateProfile { max_rate_per_ms: rate, ramp_total_ms: None }
    }
}

/// Slew-rate limiter for a continuous setpoint.
///
/// Given the current value and a proposed value, it returns the furthest value reachable within
/// `max_rate_per_ms * dt_ms`, i.e. the mechanical-shock-safe next step.
#[derive(Clone, Copy, Debug)]
pub struct SlewRateLimiter {
    max_rate_per_ms: f64,
    last: f64,
}

impl SlewRateLimiter {
    pub fn new(profile: RateProfile, initial: f64) -> Self {
        SlewRateLimiter { max_rate_per_ms: profile.max_rate_per_ms, last: initial }
    }

    /// Largest allowed step magnitude per millisecond.
    pub fn max_rate_per_ms(&self) -> f64 {
        self.max_rate_per_ms
    }

    /// Compute the safe next value when moving from `current` toward `proposed` over `dt_ms`.
    ///
    /// Returns the (possibly unchanged) stepped value and whether it was rate-limited.
    pub fn step(&mut self, current: f64, proposed: f64, dt_ms: u64) -> RateOutcome {
        let max_step = self.max_rate_per_ms * dt_ms as f64;
        let delta = proposed - current;
        self.last = current;
        if max_step.is_infinite() || delta.abs() <= max_step {
            RateOutcome::Passed(proposed)
        } else {
            let stepped = current + delta.signum() * max_step;
            RateOutcome::SlewLimited { requested: proposed, allowed: stepped }
        }
    }
}

/// Ramp generator producing staged transitions between two setpoints over a fixed duration.
///
/// Used for valve open/close or motor start sequences where the setpoint must glide rather than
/// snap (water-hammer / inrush protection).
#[derive(Clone, Copy, Debug)]
pub struct RampGenerator {
    from: f64,
    to: f64,
    total_ms: u64,
    elapsed_ms: u64,
}

impl RampGenerator {
    pub fn new(from: f64, to: f64, total_ms: u64) -> Self {
        RampGenerator { from, to, total_ms: total_ms.max(1), elapsed_ms: 0 }
    }

    /// Advance the ramp by `dt_ms` and return the current target value.
    pub fn advance(&mut self, dt_ms: u64) -> f64 {
        self.elapsed_ms = self.elapsed_ms.saturating_add(dt_ms);
        self.current()
    }

    /// Current interpolated target.
    pub fn current(&self) -> f64 {
        let frac = (self.elapsed_ms as f64 / self.total_ms as f64).clamp(0.0, 1.0);
        self.from + (self.to - self.from) * frac
    }

    /// Whether the ramp has reached its target.
    pub fn is_complete(&self) -> bool {
        self.elapsed_ms >= self.total_ms
    }

    /// Remaining time in ms.
    pub fn remaining_ms(&self) -> u64 {
        self.total_ms.saturating_sub(self.elapsed_ms)
    }
}

/// Outcome of a rate-limiting step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RateOutcome {
    /// Proposed value accepted unchanged.
    Passed(f64),
    /// Value limited by slew rate.
    SlewLimited { requested: f64, allowed: f64 },
}

impl RateOutcome {
    pub fn value(&self) -> f64 {
        match *self {
            RateOutcome::Passed(v) | RateOutcome::SlewLimited { allowed: v, .. } => v,
        }
    }

    pub fn to_reason(self) -> Option<ReasonKind> {
        match self {
            RateOutcome::Passed(_) => None,
            RateOutcome::SlewLimited { requested, allowed } => {
                Some(ReasonKind::SlewLimited { requested, allowed })
            }
        }
    }
}

impl fmt::Display for RateOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RateOutcome::Passed(v) => write!(f, "passed {v}"),
            RateOutcome::SlewLimited { requested, allowed } => {
                write!(f, "slew-limited {requested}->{allowed}")
            }
        }
    }
}

/// Convenience: build a setpoint from a rate-limited value, clamped to bounds.
pub fn rate_limited_setpoint(sp: Setpoint, value: f64) -> Setpoint {
    Setpoint { value: sp.bounds.clamp(value), bounds: sp.bounds, units: sp.units }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_control_action::Setpoint;

    #[test]
    fn slew_limits_over_fast_change() {
        let profile = RateProfile { max_rate_per_ms: 1.0, ramp_total_ms: None };
        let mut limiter = SlewRateLimiter::new(profile, 0.0);
        // requested 100 in 10ms => allowed step 10
        match limiter.step(0.0, 100.0, 10) {
            RateOutcome::SlewLimited { requested, allowed } => {
                assert_eq!(requested, 100.0);
                assert_eq!(allowed, 10.0);
            }
            other => panic!("expected slew limited, got {other:?}"),
        }
    }

    #[test]
    fn slew_allows_slow_change() {
        let profile = RateProfile { max_rate_per_ms: 1.0, ramp_total_ms: None };
        let mut limiter = SlewRateLimiter::new(profile, 0.0);
        match limiter.step(0.0, 5.0, 10) {
            RateOutcome::Passed(v) => assert_eq!(v, 5.0),
            other => panic!("expected passed, got {other:?}"),
        }
    }

    #[test]
    fn ramp_interpolates_and_completes() {
        let mut ramp = RampGenerator::new(0.0, 100.0, 100);
        assert_eq!(ramp.advance(50), 50.0); // halfway
        assert!(!ramp.is_complete());
        assert_eq!(ramp.advance(50), 100.0); // done
        assert!(ramp.is_complete());
        // overshoot clamps at target
        assert_eq!(ramp.advance(100), 100.0);
    }

    #[test]
    fn water_hammer_scenario() {
        // Valve must not snap 0->100 in one tick; limit to 5%/tick.
        let profile = RateProfile { max_rate_per_ms: 0.5, ramp_total_ms: None };
        let mut limiter = SlewRateLimiter::new(profile, 0.0);
        let mut value = 0.0;
        let mut steps = 0;
        while value < 100.0 {
            let out = limiter.step(value, 100.0, 10); // 10ms => max 5% step
            value = out.value();
            steps += 1;
            assert!(value - (value - out.value()).abs() >= 0.0); // no-op sanity
        }
        assert!(value <= 100.0);
        assert!(steps >= 20); // took many small steps, not one
    }

    #[test]
    fn profile_from_bounds() {
        let p = RateProfile::from_bounds(Bounds::PERCENT, 1000);
        assert!((p.max_rate_per_ms - 0.1).abs() < 1e-9);
    }

    #[test]
    fn rate_limited_setpoint_clamps() {
        let sp = Setpoint::percent(0.0);
        assert_eq!(rate_limited_setpoint(sp, 150.0).value, 100.0);
    }
}
