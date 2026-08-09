# tpt-control-ratelimit

Slew-rate limiters and ramp generators that protect actuators from mechanical shock — preventing
valve water hammer and motor inrush currents by limiting how fast a setpoint may move.

`no_std` + `alloc`. Part of [tpt-rust5](https://github.com/tpt-solutions/tpt-rust5).

## What it provides

- `SlewRateLimiter` — limits the per-step change to `max_rate_per_ms * dt_ms`, returning the
  mechanically-safe next value (`RateOutcome::SlewLimited` when it had to slow down).
- `RampGenerator` — staged glide between two setpoints over a fixed duration (valve open/close,
  motor start sequences).
- `RateProfile` — per-actuator profile, derivable from actuator bounds and a full-traverse time.
- `rate_limited_setpoint` — re-clamp a rate-limited value into bounds.

## Example

```rust
use tpt_control_action::Setpoint;
use tpt_control_ratelimit::{RateProfile, SlewRateLimiter, RateOutcome};

let profile = RateProfile { max_rate_per_ms: 1.0, ramp_total_ms: None };
let mut limiter = SlewRateLimiter::new(profile, 0.0);
// requested 100% in 10ms → only 10% allowed per step
if let RateOutcome::SlewLimited { requested, allowed } = limiter.step(0.0, 100.0, 10) {
    assert_eq!(allowed, 10.0);
    let _ = requested;
}
let _ = Setpoint::percent(0.0);
```

Pair this with `tpt-control-limiter` (deadband / hysteresis) before handing a command to the
`tpt-actuation-gate`.
