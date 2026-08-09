//! # tpt-control-limiter
//!
//! Saturation clamping, deadband, and hysteresis logic for physical actuators — protecting
//! hardware from mechanical wear and rapid cycling.
//!
//! `no_std` + `alloc`: pure logic, no inherent I/O.

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

use core::fmt;
use tpt_control_action::{ActuatorState, Setpoint};
use tpt_control_audit::ReasonKind;

/// Result of applying a limiting stage to a proposed value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LimitOutcome {
    /// Value accepted unchanged.
    Passed,
    /// Continuous value clamped to actuator bounds.
    Clamped { from: f64, to: f64 },
    /// Continuous change ignored because it fell inside the deadband (held previous).
    DeadbandHeld { previous: f64, proposed: f64 },
    /// Discrete change suppressed by hysteresis/confirmation (held previous).
    HysteresisHeld { previous: ActuatorState, proposed: ActuatorState },
}

impl LimitOutcome {
    /// Map to an audit [`ReasonKind`], if the value was modified.
    pub fn to_reason(self) -> Option<ReasonKind> {
        match self {
            LimitOutcome::Passed => None,
            LimitOutcome::Clamped { from, to } => Some(ReasonKind::ClampedToBounds { from, to }),
            LimitOutcome::DeadbandHeld { previous, proposed } => {
                Some(ReasonKind::DeadbandHeld { previous, proposed })
            }
            LimitOutcome::HysteresisHeld { previous, proposed } => {
                Some(ReasonKind::HysteresisHeld { previous, proposed })
            }
        }
    }
}

impl fmt::Display for LimitOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LimitOutcome::Passed => write!(f, "passed"),
            LimitOutcome::Clamped { from, to } => write!(f, "clamped {from}->{}", to),
            LimitOutcome::DeadbandHeld { previous, proposed } => {
                write!(f, "deadband held {previous} (proposed {proposed})")
            }
            LimitOutcome::HysteresisHeld { previous, proposed } => {
                write!(f, "hysteresis held {previous:?} (proposed {proposed:?})")
            }
        }
    }
}

/// Saturation clamping against actuator bounds. Always safe to apply.
pub fn clamp_setpoint(sp: Setpoint) -> Setpoint {
    sp.clamped()
}

/// Deadband limiter — ignore/hold changes that fall within a configurable band around the
/// previous value. This dampens sensor/command noise that would otherwise cause chatter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DeadbandLimiter {
    /// Absolute deadband magnitude (in setpoint units).
    pub band: f64,
}

impl DeadbandLimiter {
    pub fn new(band: f64) -> Self {
        DeadbandLimiter { band: band.abs() }
    }

    /// Returns `LimitOutcome` for a proposed continuous setpoint given the `previous` value.
    ///
    /// Within the band the previous value is held; outside it the proposed value (clamped to
    /// bounds) is accepted.
    pub fn apply(&self, previous: f64, proposed: Setpoint) -> LimitOutcome {
        let proposed = proposed.clamped().value;
        if (proposed - previous).abs() <= self.band {
            return LimitOutcome::DeadbandHeld { previous, proposed };
        }
        if proposed == previous {
            LimitOutcome::Passed
        } else {
            LimitOutcome::Clamped { from: previous, to: proposed }
        }
    }
}

/// Discrete hysteresis / anti-cycling debouncer.
///
/// A desired change of a discrete actuator is only committed after it has been requested
/// consistently for `confirm_cycles` consecutive evaluations. A single (or short) spurious
/// command is held at the previously committed state, preventing rapid ON/OFF chatter.
#[derive(Clone, Copy, Debug)]
pub struct DiscreteHysteresis {
    confirm_cycles: u32,
    last_committed: ActuatorState,
    last_desired: ActuatorState,
    pending_count: u32,
}

impl DiscreteHysteresis {
    pub fn new(confirm_cycles: u32, initial: ActuatorState) -> Self {
        DiscreteHysteresis {
            confirm_cycles: confirm_cycles.max(1),
            last_committed: initial,
            last_desired: initial,
            pending_count: 0,
        }
    }

    /// Current committed state.
    pub fn committed(&self) -> ActuatorState {
        self.last_committed
    }

    /// Feed a desired state; returns the (possibly unchanged) committed state and whether it
    /// changed this evaluation.
    pub fn apply(&mut self, desired: ActuatorState) -> (ActuatorState, LimitOutcome) {
        if desired == self.last_committed {
            self.pending_count = 0;
            self.last_desired = desired;
            return (self.last_committed, LimitOutcome::Passed);
        }
        if desired == self.last_desired {
            self.pending_count += 1;
        } else {
            self.last_desired = desired;
            self.pending_count = 1;
        }
        if self.pending_count >= self.confirm_cycles {
            self.last_committed = desired;
            self.pending_count = 0;
            (desired, LimitOutcome::Passed)
        } else {
            (
                self.last_committed,
                LimitOutcome::HysteresisHeld { previous: self.last_committed, proposed: desired },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_control_action::Setpoint;

    #[test]
    fn clamp_setpoint_bounds() {
        let sp = Setpoint::percent(140.0);
        assert_eq!(clamp_setpoint(sp).value, 100.0);
        let sp2 = Setpoint::percent(-10.0);
        assert_eq!(clamp_setpoint(sp2).value, 0.0);
    }

    #[test]
    fn deadband_holds_and_passes() {
        let db = DeadbandLimiter::new(2.0);
        // proposed within band of previous => held
        match db.apply(50.0, Setpoint::percent(51.0)) {
            LimitOutcome::DeadbandHeld { previous, proposed } => {
                assert_eq!(previous, 50.0);
                assert_eq!(proposed, 51.0);
            }
            other => panic!("expected deadband held, got {other:?}"),
        }
        // proposed outside band => accepted
        match db.apply(50.0, Setpoint::percent(60.0)) {
            LimitOutcome::Clamped { from, to } => {
                assert_eq!(from, 50.0);
                assert_eq!(to, 60.0);
            }
            other => panic!("expected clamped, got {other:?}"),
        }
    }

    #[test]
    fn deadband_clamps_out_of_bounds() {
        let db = DeadbandLimiter::new(2.0);
        match db.apply(50.0, Setpoint::percent(200.0)) {
            LimitOutcome::Clamped { to, .. } => assert_eq!(to, 100.0),
            other => panic!("expected clamped, got {other:?}"),
        }
    }

    #[test]
    fn hysteresis_requires_confirmation() {
        let mut h = DiscreteHysteresis::new(3, ActuatorState::Off);
        // single ON request => held OFF
        let (s1, o1) = h.apply(ActuatorState::On);
        assert_eq!(s1, ActuatorState::Off);
        assert!(matches!(o1, LimitOutcome::HysteresisHeld { .. }));
        let (s2, _) = h.apply(ActuatorState::On);
        assert_eq!(s2, ActuatorState::Off);
        // third consecutive ON => commits
        let (s3, o3) = h.apply(ActuatorState::On);
        assert_eq!(s3, ActuatorState::On);
        assert_eq!(o3, LimitOutcome::Passed);
    }

    #[test]
    fn hysteresis_resets_on_change() {
        let mut h = DiscreteHysteresis::new(3, ActuatorState::Off);
        h.apply(ActuatorState::On);
        h.apply(ActuatorState::Off); // changed mind -> resets counter
        let (s, _) = h.apply(ActuatorState::On);
        assert_eq!(s, ActuatorState::Off);
    }

    #[test]
    fn outcome_to_reason() {
        let o = LimitOutcome::Clamped { from: 1.0, to: 0.5 };
        assert_eq!(o.to_reason(), Some(ReasonKind::ClampedToBounds { from: 1.0, to: 0.5 }));
        assert_eq!(LimitOutcome::Passed.to_reason(), None);
    }
}
