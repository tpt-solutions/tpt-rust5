# tpt-control-audit

Reason-code logging for exactly **why** a setpoint was chosen, modified, or blocked by the safety
guardrails.

This crate owns the domain-specific *"why"*: reason codes and decision provenance across
arbitration / interlock / limiter / ratelimit / envelope / gate. It is intentionally complementary to
[`tpt-rust3`]'s `tpt-audit-trail` (cryptographic hash-chained persistence): `tpt-control-audit`
defines the decision records and emits them through a sink, while durable, tamper-evident storage is
provided by a `PersistentAuditStore` backend (the `tpt-audit-trail` integration drops in behind that
trait).

Core is `no_std` + `alloc`; the `std` feature enables the std-backed helpers. Used by every `no_std`
safety crate (with `default-features = false`), so it must not pull in `std`.

## What it provides

- `ReasonKind` — `Accepted`, `ClampedToBounds`, `SlewLimited`, `RampLimited`, `DeadbandHeld`,
  `HysteresisHeld`, `ArbitratedToHigherTier`, `ArbitratedTieBreak`, `InterlockBlocked`, `Estop`,
  `EnvelopeViolation`, `SafeStateDegraded`, `ActuatorFaulted`, `ShadowOnly`.
- `DecisionOutcome` — `Passed` / `Modified` / `Blocked`.
- `DecidingLayer` — which layer produced the decision.
- `Decision` — a fully attributable safety decision.
- `AuditSink` / `InMemoryAuditLog` — record decisions; `PersistentAuditStore` for durable backends.

## Example

```rust
use tpt_control_audit::{InMemoryAuditLog, AuditSink, DecisionOutcome, ReasonKind, DecidingLayer};

let mut log = InMemoryAuditLog::new(0);
let d = Decision::accepted(1, 1, tpt_control_action::CommandValue::discrete(
    tpt_control_action::ActuatorState::On), DecidingLayer::Gate, 0, 0);
log.record(&d).unwrap();
assert_eq!(log.len(), 1);
let _ = DecisionOutcome::Passed;
let _ = ReasonKind::Accepted;
```

[`tpt-rust3`]: https://github.com/tpt-solutions/tpt-rust3
