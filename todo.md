# tpt-rust5 — Build Checklist

**Project:** tpt-rust5 ("The Reflexes & Safety Guardrails") — TPT Solutions
**License:** Dual MIT / Apache-2.0
**Depends on:** [tpt-solutions/tpt-protocol](https://github.com/tpt-solutions/tpt-protocol) (TUP envelope spec, `SPEC-TUP.md`)

Scope: the 10 crates defined in `spec.txt`. See spec.txt for full crate descriptions and the "TUP Integration & Data Flow" section that motivates the phase ordering below.

`no_std` split: `tpt-control-action`, `tpt-control-limiter`, `tpt-control-ratelimit`, `tpt-safety-interlock`, `tpt-safety-envelope`, `tpt-control-priority`, and `tpt-state-machine` are `no_std`+`alloc` at the core — pure logic with no inherent I/O, deployable on constrained/PLC-like controllers. `tpt-control-dryrun`, `tpt-actuation-gate`, and `tpt-control-audit` are `std`-only (or `std`-gated) — they inherently touch live-state queries, the daemon handoff, or persistent logging.

Known upstream gaps (external blockers): (a) `tpt-protocol`'s TUP schema (`SPEC-TUP.md`) is currently telemetry-only (read direction) — no "TUP Command Envelope" (write/setpoint direction) is defined anywhere yet, not in `tpt-protocol`, not in `tpt-rust4` (which is supposed to emit them per its own spec). (b) `tpt-protocol-daemon` has no command-ingest API today — it only streams telemetry outbound over its WebSocket API. Both are called out as explicit blocking tasks below (Phase 1 and Phase 6) rather than assumed away; the internal trait/logic work can proceed in parallel, but concrete wire-level mapping cannot be finalized until these land upstream.

Note: `tpt-control-audit` (reason-code logging for setpoint decisions) is conceptually adjacent to `tpt-rust3`'s `tpt-audit-trail` (cryptographic hash-chained log persistence). Treat them as complementary, not duplicative: `tpt-control-audit` owns the domain-specific "why" (reason codes, decision provenance across arbitration/interlock/gate), and should use `tpt-audit-trail` as its persistence backend rather than reimplementing hash-chaining from scratch.

---

## Phase 0 — Repository & Workspace Bootstrap

- [ ] Cargo workspace root `Cargo.toml` (`resolver = "2"`, edition 2021, 10 members)
- [ ] `LICENSE-MIT` and `LICENSE-APACHE` at workspace root
- [ ] Every crate's `Cargo.toml` set to `license = "MIT OR Apache-2.0"`, authors/org = TPT Solutions
- [ ] Root `README.md` explaining tpt-rust5's role in the wider TUP platform, linking `spec.txt`
- [ ] `.gitignore`, `rustfmt.toml`, workspace-level `clippy` lint config
- [ ] GitHub Actions CI: build, test, `cargo fmt --check`, `cargo clippy`, `no_std` build check scoped to the 7 no_std-split crates
- [ ] Wire up git dependency on `tpt-solutions/tpt-protocol` (for `tpt-protocol-tup` / `tpt-protocol-core`)

## Phase 1 — `tpt-control-action` (foundation)

- [ ] Core trait for discrete actuator state (ON/OFF/FAULT) — `no_std`+`alloc`
- [ ] Core trait for continuous setpoint value (0–100%), with bounds/units metadata
- [ ] Internal setpoint/state model design — independent of any concrete wire envelope, so this can proceed without the blocker below
- [ ] **BLOCKED (external):** coordinate with `tpt-protocol`/`tpt-rust4` maintainers to define the TUP Command Envelope (write-direction) schema — does not exist yet in `SPEC-TUP.md`; nothing to map to until it lands upstream
- [ ] Once unblocked: mapping layer from internal setpoint/state types to/from the TUP Command Envelope
- [ ] Tests — trait conformance, bounds/units validation, `no_std`+`alloc` build

## Phase 2 — Actuator Protection: `tpt-control-limiter` + `tpt-control-ratelimit`

- [ ] `tpt-control-limiter`: saturation clamping against `tpt-control-action` bounds
- [ ] `tpt-control-limiter`: deadband logic (ignore/hold changes within a configurable band)
- [ ] `tpt-control-limiter`: hysteresis logic to prevent rapid on/off cycling of discrete actuators
- [ ] `tpt-control-limiter`: tests — deadband/hysteresis correctness, cycling-prevention scenarios, `no_std`+`alloc` build
- [ ] `tpt-control-ratelimit`: slew-rate limiter (max change per unit time) for continuous setpoints
- [ ] `tpt-control-ratelimit`: ramp-generator for staged transitions (e.g. valve open/close, motor start)
- [ ] `tpt-control-ratelimit`: configurable per-actuator rate profiles (sourced from `tpt-control-action` metadata)
- [ ] `tpt-control-ratelimit`: tests — slew-rate enforcement, ramp correctness, water-hammer/inrush-prevention scenarios

## Phase 3 — Safety Logic: `tpt-safety-interlock` + `tpt-safety-envelope`

- [ ] `tpt-safety-interlock`: boolean permissive/blocking-condition data model (e.g. "suction pressure < X blocks pump start")
- [ ] `tpt-safety-interlock`: evaluation engine — combinatorial logic (AND/OR/NOT chains) over live sensor inputs
- [ ] `tpt-safety-interlock`: integration hook consuming live state (external dependency on `tpt-rust3`'s `tpt-state-snapshot`)
- [ ] `tpt-safety-interlock`: tests — permissive evaluation correctness, edge cases (missing/stale sensor input), `no_std`+`alloc` build
- [ ] `tpt-safety-envelope`: alarm generation (thresholds, severity levels)
- [ ] `tpt-safety-envelope`: E-stop logic — highest-priority override path, always wins regardless of other layers
- [ ] `tpt-safety-envelope`: safe-state degradation paths when sensors fail or power is lost
- [ ] `tpt-safety-envelope`: tests — E-stop precedence, degradation-path correctness under simulated sensor/power loss, `no_std`+`alloc` build

## Phase 4 — Arbitration & Sequencing: `tpt-control-priority` + `tpt-state-machine`

- [ ] `tpt-control-priority`: priority tiers (Safety > Manual Override > Auto-Optimization > Schedule) as a data model
- [ ] `tpt-control-priority`: arbitration engine — resolves competing setpoints from multiple sources into one winner + reason
- [ ] `tpt-control-priority`: tie-break / conflict rules explicitly documented and tested
- [ ] `tpt-control-priority`: tests — arbitration correctness across all tier combinations, `no_std`+`alloc` build
- [ ] `tpt-state-machine`: deterministic FSM core (states, transitions, guards, actions)
- [ ] `tpt-state-machine`: auditability — every transition emits a structured, loggable record
- [ ] `tpt-state-machine`: reference sequence implementation (e.g. boiler purge/start/stop) as a worked example
- [ ] `tpt-state-machine`: tests — transition correctness, invalid-transition rejection, `no_std`+`alloc` build

## Phase 5 — Verification & Audit: `tpt-control-dryrun` + `tpt-control-audit`

- [ ] `tpt-control-dryrun`: shadow execution engine — runs a proposed control policy against current live state without emitting real commands
- [ ] `tpt-control-dryrun`: diff/comparison API — proposed vs. live setpoints, surfaced for operator review or automated gating
- [ ] `tpt-control-dryrun`: integration hook for live state (external dependency on `tpt-rust3`'s `tpt-state-snapshot`)
- [ ] `tpt-control-dryrun`: tests — shadow-run correctness, no-side-effect guarantee (never touches the real daemon path)
- [ ] `tpt-control-audit`: reason-code data model — why a setpoint was chosen, modified, or blocked, tied to the deciding crate (priority/interlock/envelope/gate)
- [ ] `tpt-control-audit`: logging API called from each safety-decision point across the workspace
- [ ] `tpt-control-audit`: persistence — integrate with `tpt-rust3`'s `tpt-audit-trail` (cryptographic hash-chained log) as the backing store rather than reimplementing chaining
- [ ] `tpt-control-audit`: tests — reason-code completeness across all decision points, persistence round-trip

## Phase 6 — `tpt-actuation-gate` (final pre-flight gate)

*(Capstone crate — validates a TUP command envelope against every safety layer above before handing it to `tpt-protocol-daemon`.)*

- [ ] Aggregate validation pipeline: run a proposed command through `tpt-control-priority` → `tpt-safety-interlock` → `tpt-control-limiter`/`tpt-control-ratelimit` → `tpt-safety-envelope` (E-stop always wins) in a fixed, documented order
- [ ] Integration with `tpt-control-dryrun` for optional shadow-verification before live gating
- [ ] Reason-code emission via `tpt-control-audit` for every blocked/modified/passed command
- [ ] **BLOCKED (external):** `tpt-protocol-daemon` has no command-ingest API today (WebSocket API is telemetry-outbound only) — final hand-off wiring cannot be completed until that lands upstream; build the gate's output as a well-defined validated-command type in the meantime
- [ ] Tests — full pipeline ordering correctness, E-stop precedence under all combinations, reason-code completeness

## Phase 7 — Cross-Crate Integration

- [ ] Integration test/example: full Read Path — mock proposed envelope (standing in for `tpt-rust4`'s `tpt-dispatch-solve`) + mock live state (standing in for `tpt-rust3`'s `tpt-state-snapshot`) → arbitration → interlock → limiter/ratelimit → envelope → dryrun → `tpt-actuation-gate`
- [ ] Verify combined `no_std`/`std` feature-flag builds across the whole workspace (per the Phase 1–5 no_std split)
- [ ] E-stop drill: inject E-stop mid-pipeline at every stage, confirm it always wins regardless of arbitration/interlock outcome
- [ ] Documentation pass: top-level data-flow diagram (Read Path + Write Path), per-crate README, explicit "known upstream gaps" doc section (Command Envelope schema, daemon command-ingest API)

## Phase 8 — Publish Readiness (gated — requires explicit approval before publishing)

- [ ] crates.io metadata for all 10 crates (description, keywords, categories, repository links)
- [ ] Confirm dual-license files/fields consistent across every crate
- [ ] Versioning strategy (start at `0.1.0`)
- [ ] docs.rs configuration checks (`doc_cfg` for feature-gated std items)
- [ ] `cargo publish --dry-run` per crate in dependency order
- [ ] **Stop and get explicit approval before running real `cargo publish`**
