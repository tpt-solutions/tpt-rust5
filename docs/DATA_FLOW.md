# Data Flow — Read Path & Write Path

tpt-rust5 sits between the **Brain** (optimization) and the **Senses** (hardware).

```text
                ┌─────────────────── tpt-rust5 : The Reflexes ───────────────────┐
                │                                                                │
 Read Path ───▶ │  proposed envelope (from tpt-dispatch-solve / tpt-rust4)       │
                │        │                                                       │
                │        ▼                                                       │
                │  tpt-control-priority   (arbitrate competing sources)          │
                │        ▼                                                       │
                │  tpt-safety-interlock  (permissives / blocking conditions)     │
                │        ▼                                                       │
                │  tpt-control-limiter ─ tpt-control-ratelimit (mechanical empathy)│
                │        ▼                                                       │
                │  tpt-safety-envelope   (alarms, E-stop always wins, safe-state)│
                │        ▼                                                       │
                │  tpt-control-dryrun    (optional shadow verification)          │
                │        ▼                                                       │
                │  tpt-actuation-gate    ──▶ validated command                   │
                │        │                     │                                 │
 Write Path ──▶ │        │                     ▼                                 │
                │        │              tpt-protocol-daemon (execution)          │
                │        ▼                                                       │
                │  tpt-control-audit     (reason code per decision point)        │
                └────────────────────────────────────────────────────────────────┘

   Reads live hardware state from tpt-state-snapshot (tpt-rust3) via the LiveStateProvider trait.
   Decision records are persisted via the PersistentAuditStore trait (tpt-audit-trail, tpt-rust3).
```

## Layer ordering (fixed)

1. **Priority** — highest tier wins (`Safety > Manual > Auto > Schedule`).
2. **Interlock** — permissive/blocking evaluation (fail-safe on missing/stale sensors).
3. **Limiter / Ratelimit** — saturation, deadband, hysteresis, slew-rate, ramps.
4. **Envelope** — alarms, E-stop (always wins), safe-state degradation.
5. **Dryrun** (optional) — shadow verification before live gating.
6. **Gate** — emits a `ValidatedCommand` + a reason-code `Decision` for every command.

See [`docs/KNOWN_GAPS.md`](./KNOWN_GAPS.md) for the two external blockers (TUP Command Envelope
schema and `tpt-protocol-daemon` command-ingest API).
