//! # tpt-state-machine
//!
//! Deterministic, auditable Finite State Machines for complex hardware sequences (e.g. a boiler
//! purge/start/stop sequence). Every transition — accepted or rejected — emits a structured,
//! loggable [`TransitionRecord`].
//!
//! `no_std` + `alloc`. Guards are expressed with `tpt-safety-interlock`'s `Expr` and evaluated
//! against a [`LiveStateProvider`], so a transition can require a permissive (e.g. purge airflow
//! proven) before it fires.

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

use alloc::vec::Vec;
use core::fmt;
use tpt_control_action::live_state::LiveStateProvider;
use tpt_safety_interlock::{Expr, FailPolicy};

/// State identifier.
pub type StateId = u32;
/// Event identifier.
pub type EventId = u32;
/// Action identifier (an action executed on a transition).
pub type ActionId = u32;

/// A single FSM transition. Guards are evaluated fail-safe.
#[derive(Clone, Debug)]
pub struct Transition {
    pub from: StateId,
    pub event: EventId,
    pub to: StateId,
    /// Optional guard condition that must hold for the transition to fire.
    pub guard: Option<Expr>,
    /// Optional action executed when the transition fires.
    pub action: Option<ActionId>,
    /// Human-readable description (static).
    pub description: &'static str,
}

impl Transition {
    pub fn new(from: StateId, event: EventId, to: StateId, description: &'static str) -> Self {
        Transition { from, event, to, guard: None, action: None, description }
    }

    /// Add a guard expression.
    pub fn guarded(mut self, guard: Expr) -> Self {
        self.guard = Some(guard);
        self
    }

    /// Attach an action id.
    pub fn with_action(mut self, action: ActionId) -> Self {
        self.action = Some(action);
        self
    }
}

/// Auditable record of a (attempted) transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionRecord {
    pub from: StateId,
    pub event: EventId,
    pub to: StateId,
    pub timestamp_ms: u64,
    /// Whether the transition was accepted.
    pub accepted: bool,
    /// Why it was accepted or rejected.
    pub reason: &'static str,
}

/// Errors returned when a transition cannot be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsmError {
    /// No transition matches the current state + event.
    InvalidTransition,
    /// A guard condition evaluated false (or sensor fault under fail-safe).
    GuardFailed,
}

impl fmt::Display for FsmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FsmError::InvalidTransition => write!(f, "no valid transition for this state/event"),
            FsmError::GuardFailed => write!(f, "transition guard condition not met"),
        }
    }
}

/// A deterministic finite state machine.
#[derive(Clone, Debug)]
pub struct Fsm {
    initial: StateId,
    current: StateId,
    transitions: Vec<Transition>,
    history: Vec<TransitionRecord>,
}

impl Fsm {
    /// Build an FSM from its initial state and transition table.
    pub fn new(initial: StateId, transitions: Vec<Transition>) -> Self {
        Fsm { initial, current: initial, transitions, history: Vec::new() }
    }

    /// Current state.
    pub fn current(&self) -> StateId {
        self.current
    }

    /// Full transition history (audit trail).
    pub fn history(&self) -> &[TransitionRecord] {
        &self.history
    }

    /// Reset to the initial state (history is preserved for audit).
    pub fn reset(&mut self) {
        self.current = self.initial;
    }

    /// Attempt to fire `event` given live `state`. Returns the recorded transition (accepted or
    /// rejected as an `Err`). A sensor fault during guard evaluation fails safe (rejected).
    pub fn step(
        &mut self,
        event: EventId,
        state: &dyn LiveStateProvider,
        timestamp_ms: u64,
    ) -> Result<TransitionRecord, FsmError> {
        let transition =
            self.transitions.iter().find(|t| t.from == self.current && t.event == event);

        let t = match transition {
            Some(t) => t,
            None => {
                let rec = TransitionRecord {
                    from: self.current,
                    event,
                    to: self.current,
                    timestamp_ms,
                    accepted: false,
                    reason: "invalid-transition",
                };
                self.history.push(rec);
                return Err(FsmError::InvalidTransition);
            }
        };

        // Evaluate the guard (fail-safe).
        let guard_ok = match &t.guard {
            Some(expr) => expr.evaluate(state, FailPolicy::FailSafe).unwrap_or(false),
            None => true,
        };

        if !guard_ok {
            let rec = TransitionRecord {
                from: self.current,
                event,
                to: t.to,
                timestamp_ms,
                accepted: false,
                reason: "guard-failed",
            };
            self.history.push(rec);
            return Err(FsmError::GuardFailed);
        }

        let rec = TransitionRecord {
            from: self.current,
            event,
            to: t.to,
            timestamp_ms,
            accepted: true,
            reason: t.description,
        };
        self.current = t.to;
        self.history.push(rec);
        Ok(rec)
    }
}

/// Worked example: a boiler purge / start / stop sequence.
///
/// States: `Idle(0)`, `Purging(1)`, `Lit(2)`, `Running(3)`, `Stopping(4)`, `Fault(5)`.
/// Events: `Start(10)`, `PurgeOk(11)`, `Ignite(12)`, `Stop(13)`, `FaultDetect(14)`, `Reset(15)`.
///
/// Guards require purge airflow (sensor 1) before `Lit`, and a proven flame (sensor 2) before
/// `Running`. A fault from any running-ish state drops to `Fault`.
pub fn boiler_fsm() -> Fsm {
    use tpt_safety_interlock::{Operand, RelOp};
    let purge_ok = Expr::compare(RelOp::Ge, Operand::Sensor(1), Operand::Const(1.0));
    let flame_ok = Expr::compare(RelOp::Eq, Operand::Sensor(2), Operand::Const(1.0));

    Fsm::new(
        0,
        Vec::from([
            Transition::new(0, 10, 1, "idle->purging"),
            Transition::new(1, 11, 2, "purging->lit").guarded(purge_ok),
            Transition::new(2, 12, 3, "lit->running").guarded(flame_ok).with_action(100),
            Transition::new(3, 13, 4, "running->stopping").with_action(101),
            Transition::new(4, 15, 0, "stopping->idle"),
            Transition::new(1, 14, 5, "purging->fault"),
            Transition::new(2, 14, 5, "lit->fault"),
            Transition::new(3, 14, 5, "running->fault"),
            Transition::new(5, 15, 0, "fault->idle"),
        ]),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_control_action::live_state::{SensorHealth, SensorReading};

    struct MockState {
        values: alloc::vec::Vec<(u32, SensorReading)>,
    }
    impl LiveStateProvider for MockState {
        fn read_sensor(&self, sensor: u32) -> Option<SensorReading> {
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
    fn boiler_sequence_runs() {
        let mut fsm = boiler_fsm();
        let good = MockState { values: alloc::vec![(1, reading(1.0)), (2, reading(1.0))] };
        assert_eq!(fsm.current(), 0);
        fsm.step(10, &good, 1).unwrap();
        assert_eq!(fsm.current(), 1);
        // purge ok guard must pass
        fsm.step(11, &good, 2).unwrap();
        assert_eq!(fsm.current(), 2);
        fsm.step(12, &good, 3).unwrap();
        assert_eq!(fsm.current(), 3);
        fsm.step(13, &good, 4).unwrap();
        assert_eq!(fsm.current(), 4);
        fsm.step(15, &good, 5).unwrap();
        assert_eq!(fsm.current(), 0);
        // history captured every step
        assert_eq!(fsm.history().len(), 5);
        assert!(fsm.history().iter().all(|r| r.accepted));
    }

    #[test]
    fn guard_failure_rejects() {
        let mut fsm = boiler_fsm();
        let no_purge = MockState { values: alloc::vec![(1, reading(0.0)), (2, reading(1.0))] };
        fsm.step(10, &no_purge, 1).unwrap();
        // purge airflow not proven => guard fails
        assert_eq!(fsm.step(11, &no_purge, 2), Err(FsmError::GuardFailed));
        assert_eq!(fsm.current(), 1); // unchanged
        assert!(!fsm.history().last().unwrap().accepted);
    }

    #[test]
    fn invalid_transition_rejected() {
        let mut fsm = boiler_fsm();
        // event 12 (ignite) from Idle is not a valid transition
        assert_eq!(
            fsm.step(12, &MockState { values: alloc::vec![] }, 1),
            Err(FsmError::InvalidTransition)
        );
    }

    #[test]
    fn fault_drops_to_fault_state() {
        let mut fsm = boiler_fsm();
        let good = MockState { values: alloc::vec![(1, reading(1.0)), (2, reading(1.0))] };
        fsm.step(10, &good, 1).unwrap();
        fsm.step(11, &good, 2).unwrap();
        fsm.step(14, &good, 3).unwrap(); // fault detect
        assert_eq!(fsm.current(), 5);
        fsm.step(15, &good, 4).unwrap(); // reset
        assert_eq!(fsm.current(), 0);
    }

    #[test]
    fn sensor_fault_fails_guard_safe() {
        let mut fsm = boiler_fsm();
        let bad = MockState {
            values: alloc::vec![(1, {
                let mut r = reading(1.0);
                r.health = SensorHealth::Failed;
                r
            })],
        };
        fsm.step(10, &bad, 1).unwrap();
        // guard sensor 1 failed => treat as not proven
        assert_eq!(fsm.step(11, &bad, 2), Err(FsmError::GuardFailed));
    }
}
