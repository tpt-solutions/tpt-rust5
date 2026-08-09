# tpt-actuation-gate

The final pre-flight check. Validates a proposed TUP command envelope against **every** safety layer
in a fixed, documented order before handing it to `tpt-protocol-daemon`:

```text
priority ─▶ interlock ─▶ limiter / ratelimit ─▶ envelope (E-stop always wins)
```

Every passed / modified / blocked command carries a reason code emitted through `tpt-control-audit`.
Optional shadow verification via `tpt-control-dryrun` can run before live gating.

`std`-only. Part of [tpt-rust5](https://github.com/tpt-solutions/tpt-rust5).

## What it provides

- `GateConfig` — assemble the safety layers (interlocks, limiter, rate profile, alarms, E-stop,
  safe-state plan, fail policy, step time, hysteresis cycles, dryrun enforcement).
- `ActuationGate::process(commands, &dyn LiveStateProvider)` → `GateResult` with validated
  `ValidatedCommand`s, a full `decision_log`, `estop_active`, active alarms, and an optional
  `shadow` report.
- `ValidatedCommand` — the well-defined output handed to `tpt-protocol-daemon`.

## Upstream gap

`tpt-protocol-daemon` has no command-ingest API yet (telemetry-outbound only). The gate therefore
emits a typed `ValidatedCommand` rather than calling a not-yet-existing API.

## Example

```rust
use tpt_actuation_gate::{ActuationGate, GateConfig};

let cfg = GateConfig::permissive();
let mut gate = ActuationGate::new(cfg, Box::new(tpt_control_audit::InMemoryAuditLog::new(0)));
// let result = gate.process(&prioritized_commands, &live_state);
```

See `tests/integration.rs` for the full Read Path and an E-stop drill.
