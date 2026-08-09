# tpt-state-machine

Deterministic, auditable Finite State Machines for complex hardware sequences (e.g. a boiler
purge / start / stop sequence). Every transition — accepted or rejected — emits a structured,
loggable `TransitionRecord`.

`no_std` + `alloc`. Part of [tpt-rust5](https://github.com/tpt-solutions/tpt-rust5).

## What it provides

- `Fsm` — built from an initial state and a transition table.
- `Transition` — `from` / `event` / `to`, optional guard (`tpt-safety-interlock::Expr`, evaluated
  fail-safe against a `LiveStateProvider`) and optional action id.
- `step(event, &dyn LiveStateProvider, timestamp)` — attempts a transition and records the outcome.
- `TransitionRecord` — the audit trail (`from`, `event`, `to`, `timestamp_ms`, `accepted`, `reason`).
- `boiler_fsm()` — a worked example: `Idle → Purging → Lit → Running → Stopping`, with a `Fault`
  escape and guards requiring purge airflow and a proven flame.

## Example

```rust
use tpt_state_machine::boiler_fsm;

let mut fsm = boiler_fsm();
// `state` implements LiveStateProvider
// fsm.step(10, &state, ts)?; // Idle -> Purging
// assert!(fsm.history().last().unwrap().accepted);
```

Guards integrate directly with `tpt-safety-interlock`, so a transition won't fire unless its
permissive condition holds.
