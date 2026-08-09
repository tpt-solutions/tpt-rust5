# tpt-rust5 — The Reflexes & Safety Guardrails

> Final safety and control layer for the [TUP platform](https://github.com/tpt-solutions/tpt-protocol).
> Before the Brain's optimal setpoints reach physical hardware, the **Reflexes** layer guarantees
> every command is mechanically safe, logically permitted, and properly prioritized.

See [`spec.txt`](./spec.txt) for the full crate inventory and design rationale, and
[`todo.md`](./todo.md) for the build checklist this workspace was produced from.

## Why this exists

Rust has basic PID crates, but almost no infrastructure for **industrial safety guardrails**:
interlocks, priority arbitration, rate limiting, and safe actuation gating. Without it, automated
optimization can destroy physical hardware. tpt-rust5 fills that gap.

### Core principles

- **Safety Over Optimization** — an E-stop or mechanical interlock always overrides the Brain's
  most profitable mathematical solution.
- **Mechanical Empathy** — valves cause water hammer, motors draw inrush current; rate limiters,
  deadbands, and hysteresis protect the hardware.
- **Absolute Auditability** — every blocked, modified, or executed command carries a reason code
  produced by the deciding layer.

## Crate layout

| Crate | Role | Build profile |
|-------|------|--------------|
| `tpt-control-action` | Core setpoint/state traits + TUP command envelope mapping | `no_std` + `alloc` |
| `tpt-control-limiter` | Saturation, deadband, hysteresis | `no_std` + `alloc` |
| `tpt-control-ratelimit` | Slew-rate limiter, ramp generator | `no_std` + `alloc` |
| `tpt-safety-interlock` | Boolean permissive/blocking evaluation engine | `no_std` + `alloc` |
| `tpt-safety-envelope` | Alarms, E-stop, safe-state degradation | `no_std` + `alloc` |
| `tpt-control-priority` | Priority arbitration (Safety > Manual > Auto > Schedule) | `no_std` + `alloc` |
| `tpt-state-machine` | Deterministic, auditable FSMs | `no_std` + `alloc` |
| `tpt-control-dryrun` | Shadow execution / diff against live state | `std` |
| `tpt-control-audit` | Reason-code logging + persistence backend trait | `no_std` core, `std` feature |
| `tpt-actuation-gate` | Final pre-flight validation pipeline | `std` |

## Data flow

**Read Path** — receives proposed TUP Command Envelopes (from `tpt-dispatch-solve` in tpt-rust4)
and reads live hardware state (from `tpt-state-snapshot` in tpt-rust3).

**Write Path** — passes validated, safe TUP Commands to `tpt-protocol-daemon` for execution.

```
proposed envelope ─┐
                   ▼
            tpt-control-priority  (arbitrate competing sources)
                   ▼
            tpt-safety-interlock  (permissives / blocking conditions)
                   ▼
     tpt-control-limiter ─ tpt-control-ratelimit  (mechanical empathy)
                   ▼
            tpt-safety-envelope  (alarms, E-stop always wins, safe-state)
                   ▼
            tpt-control-dryrun  (optional shadow verification)
                   ▼
            tpt-actuation-gate  ──► validated command ──► tpt-protocol-daemon
                   │
            tpt-control-audit  (reason code per decision point)
```

## Known upstream gaps (external blockers)

These are documented in detail in [`docs/KNOWN_GAPS.md`](./docs/KNOWN_GAPS.md). In short:

1. **TUP Command Envelope (write direction)** — `tpt-protocol`'s `SPEC-TUP.md` is telemetry-only
   today. The write/setpoint envelope does not yet exist upstream. tpt-rust5 defines the internal
   command model (`CommandEnvelope` in `tpt-control-action`) and is ready to map to the upstream
   schema the moment it lands; the mapping layer is the single place that will change.
2. **`tpt-protocol-daemon` command-ingest API** — the daemon currently streams telemetry outbound
   only. The final hand-off in `tpt-actuation-gate` therefore emits a well-defined
   `ValidatedCommand` type rather than calling a not-yet-existing ingest API.

## Live-state integration

`tpt-rust3`'s `tpt-state-snapshot` and `tpt-audit-trail` are consumed through local **traits**
(`LiveStateProvider`, `PersistentAuditStore`) so the workspace builds and tests fully with mock
implementations. The real upstream crates drop in behind those traits once available.

## License

Dual-licensed under [MIT](./LICENSE-MIT) and [Apache-2.0](./LICENSE-APACHE).
