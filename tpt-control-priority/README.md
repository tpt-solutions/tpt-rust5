# tpt-control-priority

Priority arbitration for competing control sources:

**Safety > Manual Override > Auto-Optimization > Schedule**

Resolves a set of competing setpoints for an actuator into a single winning command plus an explicit,
auditable reason.

`no_std` + `alloc`. Part of [tpt-rust5](https://github.com/tpt-solutions/tpt-rust5).

## What it provides

- `PriorityTier` — `Schedule < AutoOptimization < ManualOverride < Safety` (ordered by `Ord`).
- `ControlSource` — `(id, tier)`.
- `PrioritizedCommand` — a `CommandEnvelope` tagged with its source.
- `ArbitrationEngine` — `arbitrate` (single actuator) and `arbitrate_by_actuator`.

## Arbitration rules (deterministic)

1. Highest `PriorityTier` wins.
2. On a tier tie, the **most recent** command (highest `timestamp_ms`) wins.
3. On a full tie (same tier *and* timestamp), the **lowest `source.id`** wins.

Tie-breaks emit `ReasonKind::ArbitratedTieBreak`; otherwise `ArbitratedToHigherTier`.

## Example

```rust
use tpt_control_priority::{ArbitrationEngine, ControlSource, PriorityTier};
// engine.arbitrate_by_actuator(&commands) -> Vec<(ActuatorId, ArbitrationOutcome)>
```

Feeds the `tpt-actuation-gate` pipeline.
