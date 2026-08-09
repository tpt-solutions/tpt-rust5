//! # tpt-safety-envelope
//!
//! Alarm generation, E-stop logic, and safe-state degradation paths for when sensors fail or
//! power is lost. The E-stop is the highest-priority override path and **always wins** regardless
//! of any other layer.
//!
//! `no_std` + `alloc`: pure logic. Live state is read through `tpt-control-action`'s
//! `LiveStateProvider` trait; the E-stop condition reuses `tpt-safety-interlock`'s `Expr`.

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

use alloc::vec::Vec;
use core::fmt;
use tpt_control_action::live_state::{LiveStateProvider, SensorId};
use tpt_control_action::{ActuatorId, ActuatorState, CommandValue};
use tpt_safety_interlock::{Expr, FailPolicy, RelOp};

/// Alarm severity. Ordered low → high.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Severity {
    Info,
    Warning,
    Critical,
    Emergency,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Severity::Info => "INFO",
            Severity::Warning => "WARN",
            Severity::Critical => "CRIT",
            Severity::Emergency => "EMERG",
        };
        f.write_str(s)
    }
}

/// A single configured alarm threshold on a sensor channel.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AlarmThreshold {
    pub id: u32,
    pub sensor: SensorId,
    pub op: RelOp,
    pub threshold: f64,
    pub severity: Severity,
}

/// An active alarm produced by evaluation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Alarm {
    pub id: u32,
    pub sensor: SensorId,
    pub severity: Severity,
    pub value: f64,
}

impl fmt::Display for Alarm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "alarm {}: sensor {} = {} ({})", self.id, self.sensor, self.value, self.severity)
    }
}

/// Engine that evaluates configured alarm thresholds against live state.
#[derive(Clone, Debug, Default)]
pub struct AlarmEngine {
    thresholds: Vec<AlarmThreshold>,
}

impl AlarmEngine {
    pub fn new(thresholds: Vec<AlarmThreshold>) -> Self {
        AlarmEngine { thresholds }
    }

    /// Active alarms (only thresholds whose condition currently holds). A missing sensor returns
    /// an error so the caller can degrade to a safe state.
    pub fn evaluate(&self, state: &dyn LiveStateProvider) -> Result<Vec<Alarm>, EnvelopeError> {
        let mut out = Vec::new();
        for t in &self.thresholds {
            let r = state.read_sensor(t.sensor).ok_or(EnvelopeError::SensorMissing(t.sensor))?;
            if !r.is_usable() {
                return Err(EnvelopeError::SensorUnusable(t.sensor));
            }
            if t.op.apply(r.value, t.threshold) {
                out.push(Alarm {
                    id: t.id,
                    sensor: t.sensor,
                    severity: t.severity,
                    value: r.value,
                });
            }
        }
        Ok(out)
    }

    /// Highest severity currently active, if any.
    pub fn max_severity(
        &self,
        state: &dyn LiveStateProvider,
    ) -> Result<Option<Severity>, EnvelopeError> {
        let alarms = self.evaluate(state)?;
        Ok(alarms.iter().map(|a| a.severity).max())
    }
}

/// Emergency-stop controller. Once latched it stays active until explicitly cleared (and only
/// clears when the underlying condition is also false).
#[derive(Clone, Debug)]
pub struct Estop {
    condition: Option<Expr>,
    latched: bool,
}

impl Estop {
    /// E-stop with no automatic condition (manual latch only).
    pub fn manual() -> Self {
        Estop { condition: None, latched: false }
    }

    /// E-stop driven by a live condition (e.g. a guard rail opened).
    pub fn with_condition(expr: Expr) -> Self {
        Estop { condition: Some(expr), latched: false }
    }

    /// Manually trigger the E-stop (latches on).
    pub fn trigger(&mut self) {
        self.latched = true;
    }

    /// Clear the latch. Only succeeds when the underlying condition (if any) is also false.
    /// Returns `Ok(true)` if it was actually cleared, `Ok(false)` if it remains active.
    pub fn clear(
        &mut self,
        state: &dyn LiveStateProvider,
        policy: FailPolicy,
    ) -> Result<bool, EnvelopeError> {
        let cond_active = match &self.condition {
            Some(e) => e.evaluate(state, policy).unwrap_or(true), // fail-safe: treat as active
            None => false,
        };
        if self.latched && cond_active {
            Ok(false) // condition still active: cannot clear
        } else if self.latched && !cond_active {
            self.latched = false;
            Ok(true)
        } else {
            Ok(!cond_active) // already unlatched; report whether condition is quiescent
        }
    }

    /// Whether the E-stop is currently active (latched or condition true).
    pub fn is_active(
        &self,
        state: &dyn LiveStateProvider,
        policy: FailPolicy,
    ) -> Result<bool, EnvelopeError> {
        if self.latched {
            return Ok(true);
        }
        match &self.condition {
            Some(e) => Ok(e.evaluate(state, policy).unwrap_or(true)),
            None => Ok(false),
        }
    }
}

/// Predetermined safe command for an actuator used during degradation.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SafeCommand {
    pub actuator: ActuatorId,
    pub command: CommandValue,
}

/// A plan mapping actuators to their safe-state commands. When sensors fail or power is lost, the
/// actuators are driven to these values rather than left to the (possibly unsafe) last command.
#[derive(Clone, Debug, Default)]
pub struct SafeStatePlan {
    entries: Vec<SafeCommand>,
    /// When true, a power-loss event forces *every* actuator to its safe command, including those
    /// not explicitly listed (defaulting to OFF).
    force_all_off_on_power_loss: bool,
}

impl SafeStatePlan {
    pub fn new(entries: Vec<SafeCommand>, force_all_off_on_power_loss: bool) -> Self {
        SafeStatePlan { entries, force_all_off_on_power_loss }
    }

    /// Safe command for a specific actuator, if defined.
    pub fn safe_for(&self, actuator: ActuatorId) -> Option<CommandValue> {
        self.entries.iter().find(|e| e.actuator == actuator).map(|e| e.command)
    }

    /// Produce the full degraded command set given which actuators exist and the failure context.
    pub fn degrade(&self, all_actuators: &[ActuatorId], power_lost: bool) -> Vec<SafeCommand> {
        if power_lost && self.force_all_off_on_power_loss {
            return all_actuators
                .iter()
                .map(|&a| SafeCommand {
                    actuator: a,
                    command: CommandValue::discrete(ActuatorState::Off),
                })
                .collect();
        }
        self.entries.clone()
    }
}

/// Errors produced by the envelope layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvelopeError {
    SensorMissing(SensorId),
    SensorUnusable(SensorId),
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EnvelopeError::SensorMissing(s) => write!(f, "sensor {s} missing"),
            EnvelopeError::SensorUnusable(s) => write!(f, "sensor {s} stale/failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_control_action::live_state::SensorReading;
    use tpt_control_action::CommandValue;
    use tpt_safety_interlock::Operand;

    struct MockState {
        values: alloc::vec::Vec<(SensorId, SensorReading)>,
    }
    impl LiveStateProvider for MockState {
        fn read_sensor(&self, sensor: SensorId) -> Option<SensorReading> {
            self.values.iter().find(|(s, _)| *s == sensor).map(|(_, r)| *r)
        }
        fn read_actuator(
            &self,
            _actuator: tpt_control_action::ActuatorId,
        ) -> Option<tpt_control_action::ActuatorState> {
            None
        }
    }
    fn reading(v: f64) -> SensorReading {
        SensorReading::healthy(v, 0)
    }

    #[test]
    fn alarm_generated_when_threshold_exceeded() {
        let engine = AlarmEngine::new(alloc::vec![AlarmThreshold {
            id: 1,
            sensor: 1,
            op: RelOp::Gt,
            threshold: 100.0,
            severity: Severity::Critical,
        }]);
        let st = MockState { values: alloc::vec![(1, reading(150.0))] };
        let alarms = engine.evaluate(&st).unwrap();
        assert_eq!(alarms.len(), 1);
        assert_eq!(alarms[0].severity, Severity::Critical);
    }

    #[test]
    fn no_alarm_below_threshold() {
        let engine = AlarmEngine::new(alloc::vec![AlarmThreshold {
            id: 1,
            sensor: 1,
            op: RelOp::Gt,
            threshold: 100.0,
            severity: Severity::Critical,
        }]);
        let st = MockState { values: alloc::vec![(1, reading(50.0))] };
        assert!(engine.evaluate(&st).unwrap().is_empty());
    }

    #[test]
    fn estop_latched_always_active() {
        let mut estop = Estop::manual();
        estop.trigger();
        let st = MockState { values: alloc::vec::Vec::new() };
        // While latched, the E-stop is always active regardless of state.
        assert!(estop.is_active(&st, FailPolicy::FailSafe).unwrap());
        // A manual latch resets (no automatic condition to keep it active).
        assert!(estop.clear(&st, FailPolicy::FailSafe).unwrap());
        assert!(!estop.is_active(&st, FailPolicy::FailSafe).unwrap());
    }

    #[test]
    fn estop_condition_drives_active() {
        let expr = Expr::compare(RelOp::Eq, Operand::Sensor(7), Operand::Const(1.0));
        let estop = Estop::with_condition(expr);
        let st_open = MockState { values: alloc::vec![(7, reading(1.0))] };
        let st_closed = MockState { values: alloc::vec![(7, reading(0.0))] };
        assert!(estop.is_active(&st_open, FailPolicy::FailSafe).unwrap());
        assert!(!estop.is_active(&st_closed, FailPolicy::FailSafe).unwrap());
    }

    #[test]
    fn safe_state_degradation_on_power_loss() {
        let plan = SafeStatePlan::new(
            alloc::vec![
                SafeCommand { actuator: 1, command: CommandValue::discrete(ActuatorState::Off) },
                SafeCommand { actuator: 2, command: CommandValue::continuous(0.0) },
            ],
            true,
        );
        let cmds = plan.degrade(&[1, 2, 3], true);
        assert_eq!(cmds.len(), 3);
        // actuator 3 wasn't listed but power-loss forces OFF
        assert_eq!(cmds[2].command, CommandValue::discrete(ActuatorState::Off));
    }

    #[test]
    fn safe_state_partial_when_no_power_loss() {
        let plan = SafeStatePlan::new(
            alloc::vec![SafeCommand {
                actuator: 1,
                command: CommandValue::discrete(ActuatorState::Off),
            }],
            true,
        );
        let cmds = plan.degrade(&[1, 2], false);
        assert_eq!(cmds.len(), 1);
        assert_eq!(cmds[0].actuator, 1);
    }
}
