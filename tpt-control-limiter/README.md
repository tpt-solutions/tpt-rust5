# tpt-control-limiter

Saturation clamping, deadband, and hysteresis logic for physical actuators — protecting hardware
from mechanical wear and rapid ON/OFF cycling.

`no_std` + `alloc`. Part of [tpt-rust5](https://github.com/tpt-solutions/tpt-rust5).

## What it provides

- `clamp_setpoint` — saturation to actuator `Bounds`.
- `DeadbandLimiter` — ignore/hold changes within a configurable band (damps sensor/command noise).
- `DiscreteHysteresis` — anti-cycling debounce: a discrete change is only committed after it is
  requested consistently for `confirm_cycles` evaluations, preventing chatter.
- `LimitOutcome` — `Passed` / `Clamped` / `DeadbandHeld` / `HysteresisHeld`, convertible to a
  `tpt-control-audit` `ReasonKind`.

## Example

```rust
use tpt_control_action::{Setpoint, ActuatorState};
use tpt_control_limiter::{DeadbandLimiter, DiscreteHysteresis, LimitOutcome};

let db = DeadbandLimiter::new(2.0);
assert!(matches!(
    db.apply(50.0, Setpoint::percent(51.0)),
    LimitOutcome::DeadbandHeld { .. }
));

let mut h = DiscreteHysteresis::new(3, ActuatorState::Off);
h.apply(ActuatorState::On); // held until confirmed 3× consecutively
```

See `tpt-control-ratelimit` for slew-rate / ramp limiting.
