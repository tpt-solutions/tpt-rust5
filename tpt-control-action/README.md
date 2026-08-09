# tpt-control-action

Foundation crate for **tpt-rust5** ("The Reflexes & Safety Guardrails"). Defines the core traits and
data model for physical actuators and the internal command envelope that every other safety crate
operates on.

`no_std` + `alloc` — pure logic, deployable on constrained / PLC-like controllers.

## What it provides

- `ActuatorState` — discrete `On` / `Off` / `Fault`.
- `CommandValue` — a discrete state or a continuous `Setpoint` (0–100% with `Bounds` + `Units`).
- `CommandEnvelope` — the common currency of the workspace: `actuator`, `value`, `request_id`,
  `timestamp_ms`.
- Core traits `DiscreteActuator` / `ContinuousActuator` for actuator metadata & validation.
- `validate_envelope` for bounds/units and fault checks.
- `live_state` module: the `LiveStateProvider` trait (consumed by the safety layers) and
  `SensorReading` / `SensorHealth`.
- `tup` module: a stand-in **TUP Command Envelope** (write direction) and `From`/`TryFrom` mappings
  — the single surface to change when [tpt-protocol]'s real command schema lands.

## Example

```rust
use tpt_control_action::{CommandEnvelope, CommandValue};

let env = CommandEnvelope::new(7, CommandValue::continuous(55.0), 1, 1_700_000_000_000);
let wire = tpt_control_action::tup::TupCommandEnvelope::from(env); // map to wire shape
let back: CommandEnvelope = wire.try_into().unwrap();               // and back
assert_eq!(env, back);
```

## Upstream gap

The upstream TUP schema is telemetry-only. `CommandEnvelope` is the internal boundary; only the
`tup` module needs to change once the write-direction schema exists.

[tpt-protocol]: https://github.com/tpt-solutions/tpt-protocol
