# tpt-safety-envelope

Alarm generation, E-stop logic, and safe-state degradation paths for when sensors fail or power is
lost. The **E-stop is the highest-priority override path and always wins** regardless of any other
layer.

`no_std` + `alloc`. Part of [tpt-rust5](https://github.com/tpt-solutions/tpt-rust5).

## What it provides

- `AlarmEngine` + `AlarmThreshold` + `Severity` — threshold-based alarms over live sensors.
- `Estop` — manual latch and/or condition-driven (`Expr` from `tpt-safety-interlock`); once active,
  always wins. `is_active` is fail-safe (treats sensor faults as active).
- `SafeStatePlan` — a per-actuator safe command table; `degrade` drives actuators to safe state on
  sensor/power loss (optionally forcing *all* actuators OFF on power loss).

## Example

```rust
use tpt_control_action::{ActuatorState, CommandValue};
use tpt_safety_envelope::{Estop, SafeCommand, SafeStatePlan};

let mut estop = Estop::manual();
estop.trigger(); // latched — now always active

let plan = SafeStatePlan::new(
    vec![SafeCommand { actuator: 1, command: CommandValue::discrete(ActuatorState::Off) }],
    true,
);
```

Consumed by `tpt-actuation-gate` as the final safety net before live gating.
