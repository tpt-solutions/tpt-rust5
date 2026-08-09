//! # tpt-control-audit
//!
//! Reason-code logging for exactly why a setpoint was chosen, modified, or blocked by the safety
//! guardrails.
//!
//! This crate owns the **domain-specific "why"**: reason codes and decision provenance across
//! arbitration / interlock / limiter / ratelimit / envelope / gate. It is intentionally
//! complementary to [`tpt-rust3`]'s `tpt-audit-trail` (cryptographic hash-chained persistence):
//! `tpt-control-audit` defines the decision records and emits them through a sink, while the
//! durable, tamper-evident storage is provided by a [`PersistentAuditStore`] backend (the
//! `tpt-audit-trail` integration drops in behind that trait).
//!
//! Core is `no_std` + `alloc`. The `std` feature enables the std-backed persistence helpers and
//! the `display`/`Error` impls; the pure data model and the in-memory sink work with no `std`.

#![no_std]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::vec::Vec;
use core::fmt;
use tpt_control_action::{ActuatorId, ActuatorState, CommandValue, RequestId};

/// Which safety layer produced a given decision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DecidingLayer {
    Priority,
    Interlock,
    Limiter,
    Ratelimit,
    Envelope,
    Gate,
    Dryrun,
    Audit,
    Unknown,
}

impl fmt::Display for DecidingLayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DecidingLayer::Priority => "priority",
            DecidingLayer::Interlock => "interlock",
            DecidingLayer::Limiter => "limiter",
            DecidingLayer::Ratelimit => "ratelimit",
            DecidingLayer::Envelope => "envelope",
            DecidingLayer::Gate => "gate",
            DecidingLayer::Dryrun => "dryrun",
            DecidingLayer::Audit => "audit",
            DecidingLayer::Unknown => "unknown",
        };
        f.write_str(s)
    }
}

/// Outcome of a safety decision for one command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DecisionOutcome {
    Passed,
    Modified,
    Blocked,
}

impl fmt::Display for DecisionOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            DecisionOutcome::Passed => "passed",
            DecisionOutcome::Modified => "modified",
            DecisionOutcome::Blocked => "blocked",
        };
        f.write_str(s)
    }
}

/// The "why" — a specific reason code for a decision.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum ReasonKind {
    /// Command accepted unchanged.
    Accepted,
    /// Value clamped to actuator bounds.
    ClampedToBounds { from: f64, to: f64 },
    /// Change limited by slew-rate (mechanical shock protection).
    SlewLimited { requested: f64, allowed: f64 },
    /// Change limited by a staged ramp.
    RampLimited { requested: f64, allowed: f64 },
    /// Change within deadband — held at previous value.
    DeadbandHeld { previous: f64, proposed: f64 },
    /// Change suppressed by hysteresis to avoid cycling.
    HysteresisHeld { previous: ActuatorState, proposed: ActuatorState },
    /// Arbitrated to a higher-priority source.
    ArbitratedToHigherTier,
    /// Arbitrated between equal tiers via tie-break rule.
    ArbitratedTieBreak,
    /// Blocked by a permissive/interlock condition.
    InterlockBlocked,
    /// Emergency stop — highest precedence, always wins.
    Estop,
    /// Outside the safe operating envelope / alarm threshold.
    EnvelopeViolation,
    /// Sensor missing/stale — degraded to safe state.
    SafeStateDegraded,
    /// Actuator faulted — cannot be commanded.
    ActuatorFaulted,
    /// Shadow-only evaluation; no real command emitted.
    ShadowOnly,
}

impl fmt::Display for ReasonKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReasonKind::Accepted => write!(f, "accepted"),
            ReasonKind::ClampedToBounds { from, to } => write!(f, "clamped from {from} to {to}"),
            ReasonKind::SlewLimited { requested, allowed } => {
                write!(f, "slew-limited from {requested} to {allowed}")
            }
            ReasonKind::RampLimited { requested, allowed } => {
                write!(f, "ramp-limited from {requested} to {allowed}")
            }
            ReasonKind::DeadbandHeld { previous, proposed } => {
                write!(f, "deadband held {previous} (proposed {proposed})")
            }
            ReasonKind::HysteresisHeld { previous, proposed } => {
                write!(f, "hysteresis held {previous:?} (proposed {proposed:?})")
            }
            ReasonKind::ArbitratedToHigherTier => write!(f, "arbitrated to higher tier"),
            ReasonKind::ArbitratedTieBreak => write!(f, "arbitrated via tie-break"),
            ReasonKind::InterlockBlocked => write!(f, "interlock blocked"),
            ReasonKind::Estop => write!(f, "E-STOP"),
            ReasonKind::EnvelopeViolation => write!(f, "envelope violation"),
            ReasonKind::SafeStateDegraded => write!(f, "degraded to safe state"),
            ReasonKind::ActuatorFaulted => write!(f, "actuator faulted"),
            ReasonKind::ShadowOnly => write!(f, "shadow-only (no command emitted)"),
        }
    }
}

/// A single, fully attributable safety decision.
#[derive(Clone, Copy, Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Decision {
    pub actuator: ActuatorId,
    pub request_id: RequestId,
    pub value: Option<CommandValue>,
    pub outcome: DecisionOutcome,
    pub reason: ReasonKind,
    pub layer: DecidingLayer,
    pub timestamp_ms: u64,
    pub sequence: u64,
}

impl Decision {
    /// Convenience constructor for a passed decision.
    pub fn accepted(
        actuator: ActuatorId,
        request_id: RequestId,
        value: CommandValue,
        layer: DecidingLayer,
        timestamp_ms: u64,
        sequence: u64,
    ) -> Self {
        Decision {
            actuator,
            request_id,
            value: Some(value),
            outcome: DecisionOutcome::Passed,
            reason: ReasonKind::Accepted,
            layer,
            timestamp_ms,
            sequence,
        }
    }

    /// Convenience constructor for a blocked decision.
    pub fn blocked(
        actuator: ActuatorId,
        request_id: RequestId,
        reason: ReasonKind,
        layer: DecidingLayer,
        timestamp_ms: u64,
        sequence: u64,
    ) -> Self {
        Decision {
            actuator,
            request_id,
            value: None,
            outcome: DecisionOutcome::Blocked,
            reason,
            layer,
            timestamp_ms,
            sequence,
        }
    }
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#{} actuator={} layer={} outcome={} reason={}",
            self.sequence, self.actuator, self.layer, self.outcome, self.reason
        )
    }
}

/// Errors produced by the audit layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuditError {
    /// The sink/store rejected a record (e.g. capacity or backend failure).
    SinkFull,
    Backend(&'static str),
}

impl fmt::Display for AuditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuditError::SinkFull => write!(f, "audit sink is full"),
            AuditError::Backend(m) => write!(f, "audit backend error: {m}"),
        }
    }
}

/// A sink that accepts decision records. Implemented by in-memory logs and by durable backends.
pub trait AuditSink {
    /// Record a single decision.
    fn record(&mut self, decision: &Decision) -> Result<(), AuditError>;

    /// Number of records currently held (best effort).
    fn len(&self) -> usize;

    /// Whether the sink currently holds zero records.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// In-memory audit sink — `no_std` + `alloc`, used for tests and embedded deployments.
#[derive(Clone, Debug, Default)]
pub struct InMemoryAuditLog {
    records: Vec<Decision>,
    capacity: usize,
}

impl InMemoryAuditLog {
    /// New log with the given capacity (0 = unbounded).
    pub fn new(capacity: usize) -> Self {
        InMemoryAuditLog { records: Vec::new(), capacity }
    }

    /// All recorded decisions (in insertion order).
    pub fn records(&self) -> &[Decision] {
        &self.records
    }
}

impl AuditSink for InMemoryAuditLog {
    fn record(&mut self, decision: &Decision) -> Result<(), AuditError> {
        if self.capacity != 0 && self.records.len() >= self.capacity {
            return Err(AuditError::SinkFull);
        }
        self.records.push(*decision);
        Ok(())
    }

    fn len(&self) -> usize {
        self.records.len()
    }
}

/// Durable, hash-chained persistence backend (e.g. `tpt-rust3`'s `tpt-audit-trail`).
///
/// The in-memory log above satisfies this trait for non-durable use; the production
/// cryptographic backend implements the same interface without re-implementing chaining.
pub trait PersistentAuditStore: AuditSink {
    /// Flush any buffered records to durable storage.
    fn flush(&mut self) -> Result<(), AuditError> {
        Ok(())
    }
}

impl PersistentAuditStore for InMemoryAuditLog {}

#[cfg(feature = "std")]
impl std::error::Error for AuditError {}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_control_action::{ActuatorState, CommandValue};

    #[test]
    fn in_memory_sink_records() {
        let mut log = InMemoryAuditLog::new(0);
        let d = Decision::accepted(
            1,
            1,
            CommandValue::discrete(ActuatorState::On),
            DecidingLayer::Gate,
            0,
            0,
        );
        assert!(log.record(&d).is_ok());
        assert_eq!(log.len(), 1);
        assert_eq!(log.records()[0], d);
    }

    #[test]
    fn capacity_enforced() {
        let mut log = InMemoryAuditLog::new(1);
        let d = Decision::accepted(
            1,
            1,
            CommandValue::discrete(ActuatorState::On),
            DecidingLayer::Gate,
            0,
            0,
        );
        assert!(log.record(&d).is_ok());
        assert_eq!(log.record(&d), Err(AuditError::SinkFull));
    }

    #[test]
    fn blocked_decision_display() {
        let d = Decision::blocked(2, 1, ReasonKind::Estop, DecidingLayer::Envelope, 0, 5);
        assert_eq!(d.outcome, DecisionOutcome::Blocked);
        let s = alloc::format!("{d}");
        assert!(s.contains("E-STOP"));
    }

    #[test]
    fn persistent_trait_available() {
        let mut log = InMemoryAuditLog::new(0);
        let d =
            Decision::accepted(1, 1, CommandValue::continuous(10.0), DecidingLayer::Limiter, 0, 0);
        let store: &mut dyn PersistentAuditStore = &mut log;
        assert!(store.record(&d).is_ok());
        assert!(store.flush().is_ok());
    }
}
