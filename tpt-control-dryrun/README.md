# tpt-control-dryrun

Shadow execution engine: runs a proposed control policy against current live state **without emitting
any real commands**. Produces a diff/comparison (proposed vs. live) for operator review or automated
gating.

`std`-only — it inherently touches live-state queries and reporting surfaces. The `run` API is pure
read + comparison, which is the **no-side-effect guarantee** (it never calls the daemon path).

Part of [tpt-rust5](https://github.com/tpt-solutions/tpt-rust5).

## What it provides

- `ShadowConfig` — interlocks + E-stop + alarm engine used for the simulation.
- `ShadowRunner::run(proposed, &dyn LiveStateProvider, policy)` → `ShadowReport`.
- `Diff` — per-actuator proposed vs. live with a `Disposition` (`WouldPass` / `WouldBlock(reason)`)
  and a numeric `delta()` for continuous values.
- `ShadowReport` — diffs, `estop_active`, active `alarms`, and `all_pass`.

## Example

```rust
use tpt_control_dryrun::{ShadowConfig, ShadowRunner, Disposition};

let runner = ShadowRunner::new(ShadowConfig::default());
let report = runner.run(&proposed_envelopes, &live_state, tpt_safety_interlock::FailPolicy::FailSafe);
assert!(report.all_pass);
```

The `tpt-actuation-gate` can run a `ShadowRunner` for optional advisory (or enforcing) verification
before live gating.
