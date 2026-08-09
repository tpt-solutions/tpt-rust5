//! Cross-crate integration test: the full Read Path through every safety layer.
//!
//! ```text
//! proposed envelope ─▶ priority ─▶ interlock ─▶ limiter/ratelimit ─▶ envelope ─▶ dryrun ─▶ gate
//! ```
//!
//! Uses a mock `LiveStateProvider` in place of `tpt-rust3`'s `tpt-state-snapshot`.

use std::collections::HashMap;

use tpt_control_action::live_state::{LiveStateProvider, SensorReading};
use tpt_control_action::{ActuatorState, CommandEnvelope, CommandValue};
use tpt_control_audit::InMemoryAuditLog;
use tpt_control_dryrun::{ShadowConfig, ShadowRunner};
use tpt_control_priority::{ArbitrationEngine, ControlSource, PriorityTier};
use tpt_safety_envelope::{AlarmEngine, AlarmThreshold, Estop, SafeStatePlan, Severity};
use tpt_safety_interlock::{Interlock, InterlockKind, Operand, RelOp};

use tpt_actuation_gate::{ActuationGate, GateConfig, GateResult};
use tpt_control_limiter::DeadbandLimiter;
use tpt_control_ratelimit::RateProfile;

/// Mock live state: a handful of sensors + actuator states.
struct MockPlant {
    sensors: HashMap<u32, SensorReading>,
    actuators: HashMap<u32, ActuatorState>,
}

impl LiveStateProvider for MockPlant {
    fn read_sensor(&self, sensor: u32) -> Option<SensorReading> {
        self.sensors.get(&sensor).copied()
    }
    fn read_actuator(&self, actuator: u32) -> Option<ActuatorState> {
        self.actuators.get(&actuator).copied()
    }
}

fn pcmd(
    source_id: u32,
    tier: PriorityTier,
    act: u32,
    on: bool,
    ts: u64,
) -> tpt_control_priority::PrioritizedCommand {
    tpt_control_priority::PrioritizedCommand {
        source: ControlSource { id: source_id, tier },
        envelope: CommandEnvelope::new(
            act,
            CommandValue::discrete(if on { ActuatorState::On } else { ActuatorState::Off }),
            source_id as u64,
            ts,
        ),
    }
}

/// Build a representative gate config: permissive on suction pressure, an E-stop condition, modest
/// limiting, and a safe-state plan that drives the pump OFF on degradation.
fn build_config(estop: Estop) -> GateConfig {
    // Permissive: suction pressure (sensor 1) must be >= 1.0 bar to allow pump start.
    let interlocks = vec![Interlock {
        id: 1,
        kind: InterlockKind::Permissive,
        expr: tpt_safety_interlock::Expr::compare(
            RelOp::Ge,
            Operand::Sensor(1),
            Operand::Const(1.0),
        ),
    }];

    let alarm_engine = AlarmEngine::new(Vec::from([AlarmThreshold {
        id: 1,
        sensor: 2,
        op: RelOp::Gt,
        threshold: 120.0,
        severity: Severity::Critical,
    }]));

    let safe_plan = SafeStatePlan::new(
        Vec::from([tpt_safety_envelope::SafeCommand {
            actuator: 1,
            command: CommandValue::discrete(ActuatorState::Off),
        }]),
        true,
    );

    GateConfig {
        interlocks,
        limiter: DeadbandLimiter::new(2.0),
        rate_profile: RateProfile { max_rate_per_ms: f64::INFINITY, ramp_total_ms: None },
        alarm_engine,
        estop,
        safe_plan,
        fail_policy: tpt_safety_interlock::FailPolicy::FailSafe,
        step_ms: 100,
        hysteresis_cycles: 1,
        enforce_dryrun: false,
    }
}

fn build_plant(suction: f64, discharge: f64, pump: ActuatorState) -> MockPlant {
    let mut sensors = HashMap::new();
    sensors.insert(1, SensorReading::healthy(suction, 0));
    sensors.insert(2, SensorReading::healthy(discharge, 0));
    let mut actuators = HashMap::new();
    actuators.insert(1, pump);
    MockPlant { sensors, actuators }
}

#[test]
fn full_read_path_allows_safe_pump_start() {
    let cfg = build_config(Estop::manual());
    let mut gate = ActuationGate::new(cfg, Box::new(InMemoryAuditLog::new(0)));
    let plant = build_plant(2.0, 80.0, ActuatorState::Off); // good suction, OK discharge

    // Safety wants the pump OFF, Auto wants it ON. Safety must win (priority).
    let cmds = [
        pcmd(1, PriorityTier::AutoOptimization, 1, true, 100),
        pcmd(2, PriorityTier::Safety, 1, false, 200),
    ];
    let result: GateResult = gate.process(&cmds, &plant);

    assert!(!result.estop_active);
    assert_eq!(result.commands.len(), 1);
    assert!(result.commands[0].passed);
    // Safety tier won → pump OFF
    assert_eq!(result.commands[0].final_value, CommandValue::discrete(ActuatorState::Off));
    // An audit decision was recorded for the proposed command.
    assert_eq!(result.decisions.len(), 1);
}

#[test]
fn interlock_blocks_pump_start_with_low_suction() {
    let cfg = build_config(Estop::manual());
    let mut gate = ActuationGate::new(cfg, Box::new(InMemoryAuditLog::new(0)));
    // suction pressure below permissive threshold
    let plant = build_plant(0.2, 80.0, ActuatorState::Off);

    let cmds = [pcmd(1, PriorityTier::AutoOptimization, 1, true, 100)];
    let result = gate.process(&cmds, &plant);

    assert!(!result.commands[0].passed);
    assert_eq!(result.decisions[0].reason, tpt_control_audit::ReasonKind::InterlockBlocked);
}

#[test]
fn estop_drill_always_wins_and_forces_safe_state() {
    let mut estop = Estop::manual();
    estop.trigger();
    let cfg = build_config(estop);
    let mut gate = ActuationGate::new(cfg, Box::new(InMemoryAuditLog::new(0)));
    let plant = build_plant(2.0, 80.0, ActuatorState::On); // even if running fine

    // Even a Safety-tier ON command must be overridden to the safe state.
    let cmds = [pcmd(9, PriorityTier::Safety, 1, true, 100)];
    let result = gate.process(&cmds, &plant);

    assert!(result.estop_active);
    assert!(result.commands[0].passed);
    assert_eq!(result.commands[0].final_value, CommandValue::discrete(ActuatorState::Off));
    assert_eq!(result.decisions[0].reason, tpt_control_audit::ReasonKind::Estop);
}

#[test]
fn arbitration_then_dryrun_advisory_matches() {
    // Independent arbitration check + advisory dryrun should agree on the safe winner.
    let plant = build_plant(2.0, 80.0, ActuatorState::Off);
    let cmds = [
        pcmd(1, PriorityTier::AutoOptimization, 1, true, 100),
        pcmd(2, PriorityTier::Safety, 1, false, 200),
    ];

    let winners = ArbitrationEngine::arbitrate_by_actuator(&cmds);
    assert_eq!(winners[0].1.winner.source.tier, PriorityTier::Safety);

    let runner = ShadowRunner::new(ShadowConfig::default());
    let proposed: Vec<CommandEnvelope> = cmds.iter().map(|c| c.envelope).collect();
    let report = runner.run(&proposed, &plant, tpt_safety_interlock::FailPolicy::FailSafe);
    assert!(report.all_pass); // nothing blocks in this healthy scenario
}

#[test]
fn continuous_clamp_in_pipeline() {
    let cfg = build_config(Estop::manual());
    let mut gate = ActuationGate::new(cfg, Box::new(InMemoryAuditLog::new(0)));
    let plant = build_plant(2.0, 80.0, ActuatorState::Off);

    // 150% is out of the 0..=100 percent bounds → clamped to 100.
    let cmd = tpt_control_priority::PrioritizedCommand {
        source: ControlSource { id: 1, tier: PriorityTier::AutoOptimization },
        envelope: CommandEnvelope::new(1, CommandValue::continuous(150.0), 1, 100),
    };
    let result = gate.process(&[cmd], &plant);
    match result.commands[0].final_value {
        CommandValue::Continuous(sp) => assert_eq!(sp.value, 100.0),
        _ => panic!("expected continuous"),
    }
}
