# tpt-safety-interlock

Boolean logic engine for permissive and blocking conditions — e.g. *"do not start the pump if suction
pressure < X"*. Builds combinatorial AND/OR/NOT expressions over live sensor inputs and evaluates them
against a `LiveStateProvider`.

`no_std` + `alloc`. Part of [tpt-rust5](https://github.com/tpt-solutions/tpt-rust5).

## What it provides

- `Expr` — a boolean expression tree (`Compare`, `And`, `Or`, `Not`, `BoolSensor`, `Const`) over
  `Operand::Sensor(id)` / `Operand::Const(f64)`.
- `RelOp` — `Lt/Le/Gt/Ge/Eq/Ne`.
- `Interlock` — a named permissive (`Permissive`) or blocking (`Blocking`) condition.
- `InterlockState` — `Allowed` / `Blocked(reason)`.
- `FailPolicy` — `FailSafe` (block on missing/stale sensor, the safe default) or `FailOpen`.
- `evaluate_all` — evaluates a set and returns the first block (or `Allowed`).

## Example

```rust
use tpt_control_action::live_state::LiveStateProvider;
use tpt_safety_interlock::{Expr, FailPolicy, Interlock, InterlockKind, Operand, RelOp};

let il = Interlock {
    id: 1,
    kind: InterlockKind::Permissive,
    expr: Expr::compare(RelOp::Ge, Operand::Sensor(1), Operand::Const(1.0)),
};

// `state` implements LiveStateProvider (e.g. tpt-rust3's tpt-state-snapshot).
// il.evaluate(&state, FailPolicy::FailSafe)
```

See `tpt-safety-envelope` for alarms / E-stop / safe-state degradation.
