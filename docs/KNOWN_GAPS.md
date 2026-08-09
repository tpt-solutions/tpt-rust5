# Known Upstream Gaps

These are documented as **explicit dependencies** rather than assumed away. They are the two external
blockers called out in `todo.md` (Phases 1 and 6). The internal trait/logic work in tpt-rust5 proceeds
in parallel; only the concrete wire-level mapping is deferred until the upstream pieces land.

## 1. TUP Command Envelope (write direction)

**Status:** does not exist upstream yet.
**Where:** `tpt-protocol`'s `SPEC-TUP.md` is currently **telemetry-only** (read direction). There is no
"TUP Command Envelope" (write/setpoint direction) defined in `tpt-protocol`, and `tpt-rust4`
(per its own spec) is supposed to emit them but the schema is absent everywhere.

**Impact on tpt-rust5:** the internal command model — `CommandEnvelope` in `tpt-control-action` — is
fully implemented and tested. The **single** mapping surface is the `tup` module in
`tpt-control-action`, which today uses a stand-in `TupCommandEnvelope` wire shape. When the upstream
command schema lands, only `tpt-control-action/src/lib.rs` (`mod tup`) needs to change; every
downstream crate is unaffected.

**Unblock condition:** `tpt-protocol` publishes a write-direction "TUP Command Envelope" schema.

## 2. `tpt-protocol-daemon` command-ingest API

**Status:** does not exist yet.
**Where:** `tpt-protocol-daemon` streams telemetry **outbound** over its WebSocket API only. There is
no API to *ingest* a command envelope for physical execution.

**Impact on tpt-rust5:** `tpt-actuation-gate` (Phase 6, the capstone) validates a command through
every safety layer and emits a well-defined, typed `ValidatedCommand` rather than calling a
not-yet-existing ingest API. The hand-off boundary is therefore explicit and ready to wire up the
moment the daemon exposes an ingest endpoint.

**Unblock condition:** `tpt-protocol-daemon` adds a command-ingest API.

## 3. Live-state & audit-trail backends (consumed via traits)

These are **available upstream** (`tpt-rust3`'s `tpt-state-snapshot` and `tpt-audit-trail`) but are
consumed through local traits so tpt-rust5 builds and tests fully without them:

- `LiveStateProvider` (`tpt-control-action::live_state`) — `tpt-state-snapshot` implements it.
- `PersistentAuditStore` (`tpt-control-audit`) — `tpt-audit-trail` implements it (hash-chained
  durable log). `tpt-control-audit` already provides an in-memory backend behind the same trait.

No code change is blocked by these; they are drop-in behind the existing traits.
