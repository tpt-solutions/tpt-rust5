//! # tpt-safety-interlock
//!
//! Boolean logic engine for permissive and blocking conditions — e.g. "do not start the pump if
//! suction pressure < X". Builds combinatorial AND/OR/NOT expressions over live sensor inputs and
//! evaluates them against a [`LiveStateProvider`].
//!
//! `no_std` + `alloc`: pure logic, no inherent I/O. Consumes live state through the
//! `tpt-control-action` `LiveStateProvider` trait (the upstream `tpt-rust3` `tpt-state-snapshot`
//! will implement it).

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

use core::fmt;
use tpt_control_action::live_state::{LiveStateProvider, SensorId};

/// Relational comparator for a numeric condition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum RelOp {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

impl RelOp {
    pub fn apply(self, a: f64, b: f64) -> bool {
        match self {
            RelOp::Lt => a < b,
            RelOp::Le => a <= b,
            RelOp::Gt => a > b,
            RelOp::Ge => a >= b,
            RelOp::Eq => a == b,
            RelOp::Ne => a != b,
        }
    }
}

impl fmt::Display for RelOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            RelOp::Lt => "<",
            RelOp::Le => "<=",
            RelOp::Gt => ">",
            RelOp::Ge => ">=",
            RelOp::Eq => "==",
            RelOp::Ne => "!=",
        };
        f.write_str(s)
    }
}

/// An operand in a comparison: a live sensor reading or a constant.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Operand {
    Sensor(SensorId),
    Const(f64),
}

/// A boolean expression over live sensor state.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Expr {
    /// A raw boolean sensor/latch (true/false channel).
    BoolSensor(SensorId),
    Const(bool),
    Compare {
        op: RelOp,
        lhs: Operand,
        rhs: Operand,
    },
    And(alloc::boxed::Box<Expr>, alloc::boxed::Box<Expr>),
    Or(alloc::boxed::Box<Expr>, alloc::boxed::Box<Expr>),
    Not(alloc::boxed::Box<Expr>),
}

impl Expr {
    /// Build `lhs op rhs`.
    pub fn compare(op: RelOp, lhs: Operand, rhs: Operand) -> Self {
        Expr::Compare { op, lhs, rhs }
    }
    pub fn and(self, other: Expr) -> Self {
        Expr::And(alloc::boxed::Box::new(self), alloc::boxed::Box::new(other))
    }
    pub fn or(self, other: Expr) -> Self {
        Expr::Or(alloc::boxed::Box::new(self), alloc::boxed::Box::new(other))
    }
    #[allow(clippy::should_implement_trait)]
    pub fn not(self) -> Self {
        Expr::Not(alloc::boxed::Box::new(self))
    }

    /// Evaluate against live state. Returns an error if any referenced sensor is missing or
    /// not usable (stale/failed), per the given [`FailPolicy`].
    pub fn evaluate(
        &self,
        state: &dyn LiveStateProvider,
        policy: FailPolicy,
    ) -> Result<bool, InterlockError> {
        match self {
            Expr::BoolSensor(s) => match state.read_sensor(*s) {
                None => match policy {
                    FailPolicy::FailSafe => Err(InterlockError::SensorMissing(*s)),
                    FailPolicy::FailOpen => Ok(false),
                },
                Some(r) => {
                    if !r.is_usable() {
                        return match policy {
                            FailPolicy::FailSafe => Err(InterlockError::SensorUnusable(*s)),
                            FailPolicy::FailOpen => Ok(r.value != 0.0),
                        };
                    }
                    Ok(r.value != 0.0)
                }
            },
            Expr::Const(b) => Ok(*b),
            Expr::Compare { op, lhs, rhs } => {
                let l = read_operand(*lhs, state, policy)?;
                let r = read_operand(*rhs, state, policy)?;
                Ok(op.apply(l, r))
            }
            Expr::And(a, b) => Ok(a.evaluate(state, policy)? && b.evaluate(state, policy)?),
            Expr::Or(a, b) => Ok(a.evaluate(state, policy)? || b.evaluate(state, policy)?),
            Expr::Not(a) => Ok(!a.evaluate(state, policy)?),
        }
    }
}

fn read_operand(
    op: Operand,
    state: &dyn LiveStateProvider,
    policy: FailPolicy,
) -> Result<f64, InterlockError> {
    match op {
        Operand::Const(v) => Ok(v),
        Operand::Sensor(s) => match state.read_sensor(s) {
            None => match policy {
                FailPolicy::FailSafe => Err(InterlockError::SensorMissing(s)),
                FailPolicy::FailOpen => Ok(0.0),
            },
            Some(r) => {
                if !r.is_usable() {
                    return match policy {
                        FailPolicy::FailSafe => Err(InterlockError::SensorUnusable(s)),
                        FailPolicy::FailOpen => Ok(r.value),
                    };
                }
                Ok(r.value)
            }
        },
    }
}

/// How to treat a missing/stale sensor during evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FailPolicy {
    /// Fail closed (block) — the safe default for safety interlocks.
    #[default]
    FailSafe,
    /// Fail open (allow) — only for non-safety convenience logic.
    FailOpen,
}

/// Errors produced while evaluating interlock expressions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterlockError {
    SensorMissing(SensorId),
    SensorUnusable(SensorId),
}

impl fmt::Display for InterlockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InterlockError::SensorMissing(s) => write!(f, "sensor {s} missing"),
            InterlockError::SensorUnusable(s) => write!(f, "sensor {s} stale/failed"),
        }
    }
}

/// Whether an interlock permits or blocks when its expression holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum InterlockKind {
    /// The expression must be TRUE for the action to be permitted (e.g. "pressure OK").
    Permissive,
    /// The expression being TRUE blocks the action (e.g. "over-temperature").
    Blocking,
}

/// A single named interlock.
#[derive(Clone, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Interlock {
    pub id: u32,
    pub kind: InterlockKind,
    pub expr: Expr,
}

/// Result of evaluating one interlock against live state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterlockState {
    /// Action is permitted by this interlock.
    Allowed,
    /// Action is blocked by this interlock (with reason).
    Blocked(InterlockBlockReason),
}

/// Why an interlock blocked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InterlockBlockReason {
    PermissiveNotMet,
    BlockingCondition,
    SensorFault,
}

impl Interlock {
    /// Evaluate this interlock. A sensor fault returns `Err` under `FailSafe`; the gate treats
    /// any error as a block (fail-safe).
    pub fn evaluate(
        &self,
        state: &dyn LiveStateProvider,
        policy: FailPolicy,
    ) -> Result<InterlockState, InterlockError> {
        let held = self.expr.evaluate(state, policy)?;
        let state = match self.kind {
            InterlockKind::Permissive => {
                if held {
                    InterlockState::Allowed
                } else {
                    InterlockState::Blocked(InterlockBlockReason::PermissiveNotMet)
                }
            }
            InterlockKind::Blocking => {
                if held {
                    InterlockState::Blocked(InterlockBlockReason::BlockingCondition)
                } else {
                    InterlockState::Allowed
                }
            }
        };
        Ok(state)
    }
}

/// Evaluate a set of interlocks; returns the first block encountered (or `Allowed` if all pass).
pub fn evaluate_all(
    interlocks: &[Interlock],
    state: &dyn LiveStateProvider,
    policy: FailPolicy,
) -> Result<InterlockState, InterlockError> {
    for il in interlocks {
        if let InterlockState::Blocked(reason) = il.evaluate(state, policy)? {
            return Ok(InterlockState::Blocked(reason));
        }
    }
    Ok(InterlockState::Allowed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_control_action::live_state::{SensorHealth, SensorReading};

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
    fn permissive_met_allows() {
        // permissive: suction pressure >= 1.0 bar (sensor 1)
        let il = Interlock {
            id: 1,
            kind: InterlockKind::Permissive,
            expr: Expr::compare(RelOp::Ge, Operand::Sensor(1), Operand::Const(1.0)),
        };
        let st = MockState { values: alloc::vec![(1, reading(2.0))] };
        assert_eq!(il.evaluate(&st, FailPolicy::FailSafe).unwrap(), InterlockState::Allowed);
    }

    #[test]
    fn permissive_unmet_blocks() {
        let il = Interlock {
            id: 1,
            kind: InterlockKind::Permissive,
            expr: Expr::compare(RelOp::Ge, Operand::Sensor(1), Operand::Const(1.0)),
        };
        let st = MockState { values: alloc::vec![(1, reading(0.2))] };
        assert!(matches!(
            il.evaluate(&st, FailPolicy::FailSafe).unwrap(),
            InterlockState::Blocked(InterlockBlockReason::PermissiveNotMet)
        ));
    }

    #[test]
    fn blocking_condition_triggers() {
        let il = Interlock {
            id: 2,
            kind: InterlockKind::Blocking,
            expr: Expr::compare(RelOp::Gt, Operand::Sensor(5), Operand::Const(100.0)),
        };
        let st = MockState { values: alloc::vec![(5, reading(150.0))] };
        assert!(matches!(
            il.evaluate(&st, FailPolicy::FailSafe).unwrap(),
            InterlockState::Blocked(InterlockBlockReason::BlockingCondition)
        ));
    }

    #[test]
    fn missing_sensor_fails_safe() {
        let il = Interlock {
            id: 3,
            kind: InterlockKind::Permissive,
            expr: Expr::compare(RelOp::Ge, Operand::Sensor(9), Operand::Const(0.0)),
        };
        let st = MockState { values: alloc::vec![] };
        assert!(il.evaluate(&st, FailPolicy::FailSafe).is_err());
    }

    #[test]
    fn missing_sensor_fail_open() {
        let il = Interlock {
            id: 3,
            kind: InterlockKind::Permissive,
            expr: Expr::compare(RelOp::Ge, Operand::Sensor(9), Operand::Const(0.0)),
        };
        let st = MockState { values: alloc::vec![] };
        assert_eq!(il.evaluate(&st, FailPolicy::FailOpen).unwrap(), InterlockState::Allowed);
    }

    #[test]
    fn stale_sensor_fails_safe() {
        let il = Interlock {
            id: 4,
            kind: InterlockKind::Permissive,
            expr: Expr::compare(RelOp::Ge, Operand::Sensor(1), Operand::Const(0.0)),
        };
        let mut r = reading(5.0);
        r.health = SensorHealth::Stale;
        let st = MockState { values: alloc::vec![(1, r)] };
        assert!(il.evaluate(&st, FailPolicy::FailSafe).is_err());
    }

    #[test]
    fn combinatorial_and_or_not() {
        let e = Expr::compare(RelOp::Gt, Operand::Sensor(1), Operand::Const(0.0))
            .and(Expr::compare(RelOp::Lt, Operand::Sensor(2), Operand::Const(10.0)))
            .or(Expr::compare(RelOp::Eq, Operand::Sensor(3), Operand::Const(1.0)).not());
        let st = MockState {
            values: alloc::vec![(1, reading(5.0)), (2, reading(5.0)), (3, reading(0.0))],
        };
        // (1>0 && 2<10) || !(3==1) => true || true
        assert!(e.evaluate(&st, FailPolicy::FailSafe).unwrap());
    }
}
