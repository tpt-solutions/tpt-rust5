//! # tpt-actuation-gate
//!
//! The final pre-flight check. Validates a proposed TUP command envelope against **every** safety
//! layer in a fixed, documented order before it is handed to `tpt-protocol-daemon`:
//!
//! ```text
//! priority ─▶ interlock ─▶ limiter / ratelimit ─▶ envelope (E-stop always wins)
//! ```
//!
//! Every passed / modified / blocked command carries a reason code emitted through
//! [`tpt_control_audit`]. Optional shadow verification via [`tpt_control_dryrun`] can run before
//! live gating.
//!
//! **Upstream gap:** `tpt-protocol-daemon` has no command-ingest API yet, so the gate emits a
//! well-defined [`ValidatedCommand`] rather than calling a not-yet-existing API.

use std::collections::HashMap;
use std::vec::Vec;

use tpt_control_action::live_state::LiveStateProvider;
use tpt_control_action::{
    ActuatorId, ActuatorState, CommandEnvelope, CommandValue, RequestId, Setpoint,
};
use tpt_control_audit::{AuditSink, DecidingLayer, Decision, DecisionOutcome, ReasonKind};
use tpt_control_dryrun::{ShadowReport, ShadowRunner};
use tpt_control_limiter::{clamp_setpoint, DeadbandLimiter, DiscreteHysteresis, LimitOutcome};
use tpt_control_priority::{ArbitrationEngine, PrioritizedCommand};
use tpt_control_ratelimit::{RateProfile, SlewRateLimiter};
use tpt_safety_envelope::{Alarm, AlarmEngine, Estop, SafeStatePlan, Severity};
use tpt_safety_interlock::{evaluate_all, FailPolicy, Interlock};

/// Gate configuration — the assembled safety layers.
#[derive(Clone, Debug)]
pub struct GateConfig {
    pub interlocks: Vec<Interlock>,
    pub limiter: DeadbandLimiter,
    pub rate_profile: RateProfile,
    pub alarm_engine: AlarmEngine,
    pub estop: Estop,
    pub safe_plan: SafeStatePlan,
    pub fail_policy: FailPolicy,
    /// Time step used for slew-rate limiting (ms).
    pub step_ms: u64,
    /// Confirmation cycles for discrete hysteresis (1 = immediate).
    pub hysteresis_cycles: u32,
    /// If true, a dryrun "would-block" verdict also blocks the live command.
    pub enforce_dryrun: bool,
}

impl GateConfig {
    /// Build a permissive default (no interlocks, no limits, no E-stop) for testing/bring-up.
    pub fn permissive() -> Self {
        GateConfig {
            interlocks: Vec::new(),
            limiter: DeadbandLimiter::new(0.0),
            rate_profile: RateProfile { max_rate_per_ms: f64::INFINITY, ramp_total_ms: None },
            alarm_engine: AlarmEngine::new(Vec::new()),
            estop: Estop::manual(),
            safe_plan: SafeStatePlan::new(Vec::new(), true),
            fail_policy: FailPolicy::FailSafe,
            step_ms: 100,
            hysteresis_cycles: 1,
            enforce_dryrun: false,
        }
    }
}

/// A validated command ready to hand to `tpt-protocol-daemon`.
///
/// `passed == true` means the gate authorizes emission (for an E-stop this is the *safe-state*
/// substitution). `passed == false` means nothing should be emitted for this actuator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ValidatedCommand {
    pub actuator: ActuatorId,
    pub final_value: CommandValue,
    pub request_id: RequestId,
    pub passed: bool,
    pub reason: ReasonKind,
}

/// Result of running the gate over a batch of proposed commands.
#[derive(Clone, Debug, PartialEq)]
pub struct GateResult {
    pub commands: Vec<ValidatedCommand>,
    /// One decision record per proposed command (audit trail).
    pub decisions: Vec<Decision>,
    pub estop_active: bool,
    pub alarms: Vec<Alarm>,
    /// Advisory shadow-run report, if dryrun was configured.
    pub shadow: Option<ShadowReport>,
}

/// The actuation gate.
pub struct ActuationGate {
    config: GateConfig,
    audit: Box<dyn AuditSink>,
    dryrun: Option<ShadowRunner>,
    last_continuous: HashMap<ActuatorId, f64>,
    hysteresis: HashMap<ActuatorId, DiscreteHysteresis>,
    sequence: u64,
}

impl ActuationGate {
    /// Create a gate with an audit sink (e.g. an [`InMemoryAuditLog`]).
    pub fn new(config: GateConfig, audit: Box<dyn AuditSink>) -> Self {
        ActuationGate {
            config,
            audit,
            dryrun: None,
            last_continuous: HashMap::new(),
            hysteresis: HashMap::new(),
            sequence: 0,
        }
    }

    /// Create a gate that also runs an advisory (or enforcing) dryrun before live gating.
    pub fn with_dryrun(
        config: GateConfig,
        audit: Box<dyn AuditSink>,
        dryrun: ShadowRunner,
    ) -> Self {
        ActuationGate { dryrun: Some(dryrun), ..Self::new(config, audit) }
    }

    fn next_seq(&mut self) -> u64 {
        self.sequence += 1;
        self.sequence
    }

    fn record(&mut self, decision: Decision) {
        let _ = self.audit.record(&decision);
    }

    /// Run the full validation pipeline over a set of proposed (prioritized) commands.
    pub fn process(
        &mut self,
        commands: &[PrioritizedCommand],
        live: &dyn LiveStateProvider,
    ) -> GateResult {
        let estop_active =
            self.config.estop.is_active(live, self.config.fail_policy).unwrap_or(true);

        let alarms = self.config.alarm_engine.evaluate(live).unwrap_or_default();
        let envelope_blocks =
            alarms.iter().any(|a| a.severity >= Severity::Critical) && !estop_active;

        // Optional advisory dryrun (uses the same safety config).
        let shadow = self.dryrun.as_ref().map(|r| {
            let proposed: Vec<CommandEnvelope> = commands.iter().map(|c| c.envelope).collect();
            r.run(&proposed, live, self.config.fail_policy)
        });

        let winners = ArbitrationEngine::arbitrate_by_actuator(commands);

        let mut out_commands = Vec::new();
        let mut out_decisions = Vec::new();

        for (actuator, outcome) in winners {
            let proposed = outcome.winner.envelope;
            let seq = self.next_seq();
            let timestamp = proposed.timestamp_ms;
            let request_id = proposed.request_id;

            // 1) E-STOP — highest precedence, always wins.
            if estop_active {
                let safe = self
                    .config
                    .safe_plan
                    .safe_for(actuator)
                    .unwrap_or(CommandValue::discrete(ActuatorState::Off));
                out_commands.push(ValidatedCommand {
                    actuator,
                    final_value: safe,
                    request_id,
                    passed: true,
                    reason: ReasonKind::Estop,
                });
                self.update_last(actuator, safe);
                let d = Decision {
                    actuator,
                    request_id,
                    value: Some(proposed.value),
                    outcome: DecisionOutcome::Blocked,
                    reason: ReasonKind::Estop,
                    layer: DecidingLayer::Envelope,
                    timestamp_ms: timestamp,
                    sequence: seq,
                };
                self.record(d);
                out_decisions.push(d);
                continue;
            }

            // 2) Interlock / permissive evaluation.
            match evaluate_all(&self.config.interlocks, live, self.config.fail_policy) {
                Ok(tpt_safety_interlock::InterlockState::Blocked(_)) | Err(_) => {
                    let reason = ReasonKind::InterlockBlocked;
                    let cmd = ValidatedCommand {
                        actuator,
                        final_value: proposed.value,
                        request_id,
                        passed: false,
                        reason,
                    };
                    out_commands.push(cmd);
                    let d = Decision {
                        actuator,
                        request_id,
                        value: Some(proposed.value),
                        outcome: DecisionOutcome::Blocked,
                        reason,
                        layer: DecidingLayer::Interlock,
                        timestamp_ms: timestamp,
                        sequence: seq,
                    };
                    self.record(d);
                    out_decisions.push(d);
                    continue;
                }
                Ok(tpt_safety_interlock::InterlockState::Allowed) => {}
            }

            // Optional dryrun enforcement.
            if self.config.enforce_dryrun {
                if let Some(report) = &shadow {
                    if let Some(diff) = report.diffs.iter().find(|d| d.actuator == actuator) {
                        if let tpt_control_dryrun::Disposition::WouldBlock(rk) = diff.disposition {
                            let cmd = ValidatedCommand {
                                actuator,
                                final_value: proposed.value,
                                request_id,
                                passed: false,
                                reason: rk,
                            };
                            out_commands.push(cmd);
                            let d = Decision {
                                actuator,
                                request_id,
                                value: Some(proposed.value),
                                outcome: DecisionOutcome::Blocked,
                                reason: rk,
                                layer: DecidingLayer::Dryrun,
                                timestamp_ms: timestamp,
                                sequence: seq,
                            };
                            self.record(d);
                            out_decisions.push(d);
                            continue;
                        }
                    }
                }
            }

            // 3) Limiter + ratelimit (mechanical empathy).
            let (final_value, modify_reason) = self.apply_limits(actuator, proposed.value, live);

            // 4) Envelope (alarms / safe-state). Already computed `envelope_blocks`.
            if envelope_blocks {
                let reason = ReasonKind::EnvelopeViolation;
                let cmd = ValidatedCommand {
                    actuator,
                    final_value: proposed.value,
                    request_id,
                    passed: false,
                    reason,
                };
                out_commands.push(cmd);
                let d = Decision {
                    actuator,
                    request_id,
                    value: Some(proposed.value),
                    outcome: DecisionOutcome::Blocked,
                    reason,
                    layer: DecidingLayer::Envelope,
                    timestamp_ms: timestamp,
                    sequence: seq,
                };
                self.record(d);
                out_decisions.push(d);
                continue;
            }

            // Passed (possibly modified by limiter/ratelimit).
            let outcome_kind = if final_value == proposed.value {
                DecisionOutcome::Passed
            } else {
                DecisionOutcome::Modified
            };
            let reason = if outcome_kind == DecisionOutcome::Passed {
                ReasonKind::Accepted
            } else {
                modify_reason.unwrap_or(ReasonKind::Accepted)
            };
            out_commands.push(ValidatedCommand {
                actuator,
                final_value,
                request_id,
                passed: true,
                reason,
            });
            self.update_last(actuator, final_value);
            let d = Decision {
                actuator,
                request_id,
                value: Some(final_value),
                outcome: outcome_kind,
                reason,
                layer: DecidingLayer::Gate,
                timestamp_ms: timestamp,
                sequence: seq,
            };
            self.record(d);
            out_decisions.push(d);
        }

        GateResult {
            commands: out_commands,
            decisions: out_decisions,
            estop_active,
            alarms,
            shadow,
        }
    }

    /// Apply saturation/deadband/hysteresis and slew-rate limiting to a proposed value.
    fn apply_limits(
        &mut self,
        actuator: ActuatorId,
        value: CommandValue,
        _live: &dyn LiveStateProvider,
    ) -> (CommandValue, Option<ReasonKind>) {
        match value {
            CommandValue::Continuous(sp) => {
                let previous = self.last_continuous.get(&actuator).copied().unwrap_or(sp.value);
                let clamped = clamp_setpoint(sp);
                let clamped_val = clamped.value;

                // deadband
                let (after_db, db_reason) = match self.config.limiter.apply(previous, clamped) {
                    LimitOutcome::Passed => (clamped_val, None),
                    LimitOutcome::Clamped { to, .. } => {
                        (to, Some(ReasonKind::ClampedToBounds { from: previous, to }))
                    }
                    LimitOutcome::DeadbandHeld { previous, proposed } => {
                        (previous, Some(ReasonKind::DeadbandHeld { previous, proposed }))
                    }
                    LimitOutcome::HysteresisHeld { .. } => (clamped_val, None),
                };

                // slew rate
                let mut slew = SlewRateLimiter::new(self.config.rate_profile, previous);
                let rate = slew.step(previous, after_db, self.config.step_ms);
                let final_val = rate.value();
                let slew_reason = rate.to_reason();

                // pick the dominant modification reason (clamp > slew > deadband)
                let reason = if db_reason
                    .map(|r| matches!(r, ReasonKind::ClampedToBounds { .. }))
                    .unwrap_or(false)
                {
                    db_reason
                } else if slew_reason.is_some() {
                    slew_reason
                } else {
                    db_reason
                };

                (CommandValue::Continuous(Setpoint { value: final_val, ..clamped }), reason)
            }
            CommandValue::Discrete(state) => {
                let initial = _live.read_actuator(actuator).unwrap_or(ActuatorState::Off);
                let h = self.hysteresis.entry(actuator).or_insert_with(|| {
                    DiscreteHysteresis::new(self.config.hysteresis_cycles, initial)
                });
                let (committed, outcome) = h.apply(state);
                let reason = outcome.to_reason();
                (CommandValue::Discrete(committed), reason)
            }
        }
    }

    fn update_last(&mut self, actuator: ActuatorId, value: CommandValue) {
        match value {
            CommandValue::Continuous(sp) => {
                self.last_continuous.insert(actuator, sp.value);
            }
            CommandValue::Discrete(s) => {
                self.hysteresis
                    .entry(actuator)
                    .or_insert_with(|| DiscreteHysteresis::new(self.config.hysteresis_cycles, s))
                    .apply(s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tpt_control_action::live_state::SensorReading;
    use tpt_control_audit::InMemoryAuditLog;
    use tpt_control_dryrun::{Disposition, ShadowConfig};
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

    fn pcmd(
        id: u32,
        tier: tpt_control_priority::PriorityTier,
        act: u32,
        on: bool,
        ts: u64,
    ) -> PrioritizedCommand {
        PrioritizedCommand {
            source: tpt_control_priority::ControlSource { id, tier },
            envelope: CommandEnvelope::new(
                act,
                CommandValue::discrete(if on { ActuatorState::On } else { ActuatorState::Off }),
                id as u64,
                ts,
            ),
        }
    }

    #[test]
    fn passes_when_permissive() {
        let cfg = GateConfig::permissive();
        let mut gate = ActuationGate::new(cfg, Box::new(InMemoryAuditLog::new(0)));
        let st = MockState { values: Vec::new(), actuators: std::vec![(1, ActuatorState::Off)] };
        let r = gate.process(
            &[pcmd(1, tpt_control_priority::PriorityTier::AutoOptimization, 1, true, 0)],
            &st,
        );
        assert!(!r.estop_active);
        assert_eq!(r.commands.len(), 1);
        assert!(r.commands[0].passed);
        assert_eq!(r.commands[0].final_value, CommandValue::discrete(ActuatorState::On));
    }

    #[test]
    fn estop_always_wins() {
        let mut cfg = GateConfig::permissive();
        cfg.estop.trigger();
        let mut gate = ActuationGate::new(cfg, Box::new(InMemoryAuditLog::new(0)));
        let st = MockState { values: Vec::new(), actuators: std::vec![(1, ActuatorState::On)] };
        let r =
            gate.process(&[pcmd(1, tpt_control_priority::PriorityTier::Safety, 1, true, 0)], &st);
        assert!(r.estop_active);
        // the authorized command is the safe state (OFF), not the proposed ON
        assert_eq!(r.commands[0].final_value, CommandValue::discrete(ActuatorState::Off));
        assert!(r.commands[0].passed);
        assert_eq!(r.decisions[0].reason, ReasonKind::Estop);
    }

    #[test]
    fn interlock_blocks() {
        let mut cfg = GateConfig::permissive();
        // permissive: sensor 1 >= 1.0; absent => fail-safe block
        cfg.interlocks.push(Interlock {
            id: 1,
            kind: InterlockKind::Permissive,
            expr: tpt_safety_interlock::Expr::compare(
                RelOp::Ge,
                Operand::Sensor(1),
                Operand::Const(1.0),
            ),
        });
        let mut gate = ActuationGate::new(cfg, Box::new(InMemoryAuditLog::new(0)));
        let st = MockState { values: Vec::new(), actuators: std::vec![(1, ActuatorState::Off)] };
        let r = gate.process(
            &[pcmd(1, tpt_control_priority::PriorityTier::AutoOptimization, 1, true, 0)],
            &st,
        );
        assert!(!r.commands[0].passed);
        assert_eq!(r.decisions[0].reason, ReasonKind::InterlockBlocked);
    }

    #[test]
    fn limiter_clamps_out_of_bounds_continuous() {
        let cfg = GateConfig::permissive();
        let mut gate = ActuationGate::new(cfg, Box::new(InMemoryAuditLog::new(0)));
        let st = MockState { values: Vec::new(), actuators: std::vec![(1, ActuatorState::Off)] };
        // continuous 150% should be clamped to 100%
        let cmd = PrioritizedCommand {
            source: tpt_control_priority::ControlSource {
                id: 1,
                tier: tpt_control_priority::PriorityTier::AutoOptimization,
            },
            envelope: CommandEnvelope::new(1, CommandValue::continuous(150.0), 1, 0),
        };
        let r = gate.process(&[cmd], &st);
        match r.commands[0].final_value {
            CommandValue::Continuous(sp) => assert_eq!(sp.value, 100.0),
            _ => panic!("expected continuous"),
        }
        assert_eq!(r.decisions[0].reason, ReasonKind::ClampedToBounds { from: 150.0, to: 100.0 });
    }

    #[test]
    fn dryrun_advisory_included() {
        let cfg = GateConfig::permissive();
        let dryrun = ShadowRunner::new(ShadowConfig::default());
        let mut gate = ActuationGate::with_dryrun(cfg, Box::new(InMemoryAuditLog::new(0)), dryrun);
        let st = MockState { values: Vec::new(), actuators: std::vec![(1, ActuatorState::Off)] };
        let r = gate.process(
            &[pcmd(1, tpt_control_priority::PriorityTier::AutoOptimization, 1, true, 0)],
            &st,
        );
        assert!(r.shadow.is_some());
        assert_eq!(r.shadow.unwrap().diffs[0].disposition, Disposition::WouldPass);
    }

    #[test]
    fn full_pipeline_orders_estop_over_arbitration() {
        // Even a Safety-tier ON command is overridden to safe OFF when E-stop is active.
        let mut cfg = GateConfig::permissive();
        cfg.estop.trigger();
        let mut gate = ActuationGate::new(cfg, Box::new(InMemoryAuditLog::new(0)));
        let st = MockState { values: Vec::new(), actuators: std::vec![(1, ActuatorState::On)] };
        let r =
            gate.process(&[pcmd(9, tpt_control_priority::PriorityTier::Safety, 1, true, 0)], &st);
        assert_eq!(r.commands[0].final_value, CommandValue::discrete(ActuatorState::Off));
    }
}
