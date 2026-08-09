//! # tpt-control-dryrun
//!
//! Shadow execution engine: runs a proposed control policy against current live state **without**
//! emitting any real commands. It produces a diff/comparison (proposed vs. live) for operator
//! review or automated gating.
//!
//! This crate is `std`-only: it inherently touches live-state queries and the comparison/reporting
//! surfaces. It never writes to the daemon path — the `run` API is pure read + comparison, which
//! is the *no-side-effect guarantee* tested below.

use std::vec::Vec;

use tpt_control_action::live_state::LiveStateProvider;
use tpt_control_action::{ActuatorId, CommandEnvelope, CommandValue};
use tpt_control_audit::ReasonKind;
use tpt_safety_envelope::{Alarm, AlarmEngine, Estop, Severity};
use tpt_safety_interlock::{evaluate_all, FailPolicy, Interlock};

/// Safety configuration used by the shadow runner to simulate a policy.
#[derive(Clone, Debug)]
pub struct ShadowConfig {
    pub interlocks: Vec<Interlock>,
    pub estop: Estop,
    pub alarm_engine: AlarmEngine,
}

impl Default for ShadowConfig {
    fn default() -> Self {
        ShadowConfig {
            interlocks: Vec::new(),
            estop: Estop::manual(),
            alarm_engine: AlarmEngine::new(Vec::new()),
        }
    }
}

/// Disposition of a single proposed command under simulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Disposition {
    /// Would be accepted (perhaps after modification).
    WouldPass,
    /// Would be blocked, with the reason.
    WouldBlock(ReasonKind),
}

/// A per-actuator diff between a proposed command and the live state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Diff {
    pub actuator: ActuatorId,
    pub proposed: CommandValue,
    pub live: Option<CommandValue>,
    pub disposition: Disposition,
}

impl Diff {
    /// Numeric delta for continuous commands (proposed − live), if both are continuous.
    pub fn delta(&self) -> Option<f64> {
        match (self.proposed, self.live) {
            (CommandValue::Continuous(a), Some(CommandValue::Continuous(b))) => {
                Some(a.value - b.value)
            }
            _ => None,
        }
    }
}

/// Aggregate report from a shadow run.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadowReport {
    pub diffs: Vec<Diff>,
    pub estop_active: bool,
    pub alarms: Vec<Alarm>,
    /// True only if every proposed command would pass.
    pub all_pass: bool,
}

/// The shadow execution engine.
#[derive(Clone, Debug)]
pub struct ShadowRunner {
    config: ShadowConfig,
}

impl ShadowRunner {
    pub fn new(config: ShadowConfig) -> Self {
        ShadowRunner { config }
    }

    /// Run a proposed policy against live state, producing a diff and feasibility report.
    ///
    /// **No commands are emitted.** This reads live state and the proposed envelopes only.
    pub fn run(
        &self,
        proposed: &[CommandEnvelope],
        live: &dyn LiveStateProvider,
        policy: FailPolicy,
    ) -> ShadowReport {
        let estop_active = self.config.estop.is_active(live, policy).unwrap_or(true); // fail-safe

        let alarms = self.config.alarm_engine.evaluate(live).unwrap_or_default();
        let alarms_block = alarms.iter().any(|a| a.severity >= Severity::Critical);

        let interlock_state = evaluate_all(&self.config.interlocks, live, policy);

        let mut diffs = Vec::new();
        for env in proposed {
            let live_value = match env.value {
                CommandValue::Discrete(_) => {
                    live.read_actuator(env.actuator).map(CommandValue::Discrete)
                }
                CommandValue::Continuous(_) => None, // live continuous setpoint not modeled here
            };

            let disposition = if estop_active {
                Disposition::WouldBlock(ReasonKind::Estop)
            } else if matches!(
                interlock_state,
                Ok(tpt_safety_interlock::InterlockState::Blocked(_)) | Err(_)
            ) {
                // fail-safe: a sensor fault (Err) also blocks
                Disposition::WouldBlock(ReasonKind::InterlockBlocked)
            } else if alarms_block {
                Disposition::WouldBlock(ReasonKind::EnvelopeViolation)
            } else {
                Disposition::WouldPass
            };

            diffs.push(Diff {
                actuator: env.actuator,
                proposed: env.value,
                live: live_value,
                disposition,
            });
        }

        let all_pass = diffs.iter().all(|d| d.disposition == Disposition::WouldPass);
        ShadowReport { diffs, estop_active, alarms, all_pass }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_control_action::live_state::SensorReading;
    use tpt_control_action::ActuatorState;
    use tpt_safety_interlock::{InterlockKind, Operand, RelOp};

    struct MockState {
        values: std::vec::Vec<(u32, SensorReading)>,
        actuators: std::vec::Vec<(u32, ActuatorState)>,
    }
    impl LiveStateProvider for MockState {
        fn read_sensor(&self, sensor: u32) -> Option<SensorReading> {
            self.values.iter().find(|(s, _)| *s == sensor).map(|(_, r)| *r)
        }
        fn read_actuator(&self, actuator: u32) -> Option<ActuatorState> {
            self.actuators.iter().find(|(a, _)| *a == actuator).map(|(_, s)| *s)
        }
    }
    fn proposed(act: u32, on: bool) -> CommandEnvelope {
        CommandEnvelope::new(
            act,
            CommandValue::discrete(if on { ActuatorState::On } else { ActuatorState::Off }),
            1u64,
            0,
        )
    }

    #[test]
    fn shadow_passes_when_safe() {
        let cfg = ShadowConfig::default();
        let runner = ShadowRunner::new(cfg);
        let st = MockState {
            values: std::vec::Vec::new(),
            actuators: std::vec![(1, ActuatorState::Off)],
        };
        let report = runner.run(&[proposed(1, true)], &st, FailPolicy::FailSafe);
        assert!(report.all_pass);
        assert!(report.diffs[0].disposition == Disposition::WouldPass);
    }

    #[test]
    fn shadow_blocks_on_estop() {
        let mut cfg = ShadowConfig::default();
        cfg.estop.trigger();
        let runner = ShadowRunner::new(cfg);
        let st = MockState {
            values: std::vec::Vec::new(),
            actuators: std::vec![(1, ActuatorState::Off)],
        };
        let report = runner.run(&[proposed(1, true)], &st, FailPolicy::FailSafe);
        assert!(report.estop_active);
        assert!(report.diffs[0].disposition == Disposition::WouldBlock(ReasonKind::Estop));
        assert!(!report.all_pass);
    }

    #[test]
    fn shadow_blocks_on_interlock() {
        let mut cfg = ShadowConfig::default();
        // permissive: sensor 1 >= 1.0 (absent => fails safe => blocked)
        cfg.interlocks.push(Interlock {
            id: 1,
            kind: InterlockKind::Permissive,
            expr: expr_compare(RelOp::Ge, Operand::Sensor(1), Operand::Const(1.0)),
        });
        let runner = ShadowRunner::new(cfg);
        let st = MockState { values: Vec::new(), actuators: std::vec![(1, ActuatorState::Off)] };
        let report = runner.run(&[proposed(1, true)], &st, FailPolicy::FailSafe);
        assert!(
            report.diffs[0].disposition == Disposition::WouldBlock(ReasonKind::InterlockBlocked)
        );
    }

    #[test]
    fn no_side_effects_when_state_is_borrowed() {
        // The runner only borrows live state immutably; repeated runs are pure.
        let runner = ShadowRunner::new(ShadowConfig::default());
        let st = MockState { values: Vec::new(), actuators: std::vec![(1, ActuatorState::Off)] };
        let a = runner.run(&[proposed(1, true)], &st, FailPolicy::FailSafe);
        let b = runner.run(&[proposed(1, true)], &st, FailPolicy::FailSafe);
        assert_eq!(a, b);
    }

    #[test]
    fn diff_delta_for_continuous() {
        let d = Diff {
            actuator: 1,
            proposed: CommandValue::continuous(80.0),
            live: Some(CommandValue::continuous(50.0)),
            disposition: Disposition::WouldPass,
        };
        assert_eq!(d.delta(), Some(30.0));
    }

    // helper mirroring interlock's Expr::compare
    fn expr_compare(op: RelOp, lhs: Operand, rhs: Operand) -> tpt_safety_interlock::Expr {
        tpt_safety_interlock::Expr::compare(op, lhs, rhs)
    }
}
