1: # tpt-rust5 — Build Checklist
2: 
3: **Project:** tpt-rust5 ("The Reflexes & Safety Guardrails") — TPT Solutions
4: **License:** Dual MIT / Apache-2.0
5: **Depends on:** [tpt-solutions/tpt-protocol](https://github.com/tpt-solutions/tpt-protocol) (TUP envelope spec, `SPEC-TUP.md`)
6: 
7: Scope: the 10 crates defined in `spec.txt`. See spec.txt for full crate descriptions and the "TUP Integration & Data Flow" section that motivates the phase ordering below.
8: 
9: `no_std` split: `tpt-control-action`, `tpt-control-limiter`, `tpt-control-ratelimit`, `tpt-safety-interlock`, `tpt-safety-envelope`, `tpt-control-priority`, and `tpt-state-machine` are `no_std`+`alloc` at the core — pure logic with no inherent I/O, deployable on constrained/PLC-like controllers. `tpt-control-dryrun`, `tpt-actuation-gate`, and `tpt-control-audit` are `std`-only (or `std`-gated) — they inherently touch live-state queries, the daemon handoff, or persistent logging.
10: 
11: Known upstream gaps (external blockers): (a) `tpt-protocol`'s TUP schema (`SPEC-TUP.md`) is currently telemetry-only (read direction) — no "TUP Command Envelope" (write/setpoint direction) is defined anywhere yet, not in `tpt-protocol`, not in `tpt-rust4` (which is supposed to emit them per its own spec). (b) `tpt-protocol-daemon` has no command-ingest API today — it only streams telemetry outbound over its WebSocket API. Both are called out as explicit blocking tasks below (Phase 1 and Phase 6) rather than assumed away; the internal trait/logic work can proceed in parallel, but concrete wire-level mapping cannot be finalized until these land upstream.
12: 
13: **Status (2026-08-10):** All phases implemented and building green. `cargo build`, `cargo test` (66 tests, 0 failures), `cargo fmt --check`, and `cargo clippy --workspace --all-targets` all pass; the 7 `no_std` crates compile with `--no-default-features` (and with `serde` under `no_std`). The two external blockers are isolated to single, well-defined boundaries (the `tup` module in `tpt-control-action`, and the typed `ValidatedCommand` output of `tpt-actuation-gate`) so nothing downstream depends on them. See `docs/KNOWN_GAPS.md` and `docs/DATA_FLOW.md`.
14: 
15: ---
16: 
17: ## Phase 0 — Repository & Workspace Bootstrap
18: 
19: - [x] Cargo workspace root `Cargo.toml` (`resolver = "2"`, edition 2021, 10 members)
20: - [x] `LICENSE-MIT` and `LICENSE-APACHE` at workspace root
21: - [x] Every crate's `Cargo.toml` set to `license = "MIT OR Apache-2.0"`, authors/org = TPT Solutions
22: - [x] Root `README.md` explaining tpt-rust5's role in the wider TUP platform, linking `spec.txt`
23: - [x] `.gitignore`, `rustfmt.toml`, workspace-level `clippy` lint config
24: - [x] GitHub Actions CI: build, test, `cargo fmt --check`, `cargo clippy`, `no_std` build check scoped to the 7 no_std-split crates
25: - [x] Wire up git dependency on `tpt-solutions/tpt-protocol` (for `tpt-protocol-tup` / `tpt-protocol-core`)
26: 
27: ## Phase 1 — `tpt-control-action` (foundation)
28: 
29: - [x] Core trait for discrete actuator state (ON/OFF/FAULT) — `no_std`+`alloc`
30: - [x] Core trait for continuous setpoint value (0–100%), with bounds/units metadata
31: - [x] Internal setpoint/state model design — independent of any concrete wire envelope, so this can proceed without the blocker below
32: - [x] **BLOCKED (external):** coordinate with `tpt-protocol`/`tpt-rust4` maintainers to define the TUP Command Envelope (write-direction) schema — does not exist yet in `SPEC-TUP.md`; nothing to map to until it lands upstream
33:       - *Resolution:* internal `CommandEnvelope` model is complete and tested; the `tup` module in `tpt-control-action` is the single mapping surface (stand-in `TupCommandEnvelope` today) that changes when the upstream schema lands.
34: - [x] Once unblocked: mapping layer from internal setpoint/state types to/from the TUP Command Envelope
35:       - *Status:* mapping layer exists against the stand-in; swapping in the real schema is a localized change to `mod tup`.
36: - [x] Tests — trait conformance, bounds/units validation, `no_std`+`alloc` build
37: 
38: ## Phase 2 — Actuator Protection: `tpt-control-limiter` + `tpt-control-ratelimit`
39: 
40: - [x] `tpt-control-limiter`: saturation clamping against `tpt-control-action` bounds
41: - [x] `tpt-control-limiter`: deadband logic (ignore/hold changes within a configurable band)
42: - [x] `tpt-control-limiter`: hysteresis logic to prevent rapid on/off cycling of discrete actuators
43: - [x] `tpt-control-limiter`: tests — deadband/hysteresis correctness, cycling-prevention scenarios, `no_std`+`alloc` build
44: - [x] `tpt-control-ratelimit`: slew-rate limiter (max change per unit time) for continuous setpoints
45: - [x] `tpt-control-ratelimit`: ramp-generator for staged transitions (e.g. valve open/close, motor start)
46: - [x] `tpt-control-ratelimit`: configurable per-actuator rate profiles (sourced from `tpt-control-action` metadata)
47: - [x] `tpt-control-ratelimit`: tests — slew-rate enforcement, ramp correctness, water-hammer/inrush-prevention scenarios
48: 
49: ## Phase 3 — Safety Logic: `tpt-safety-interlock` + `tpt-safety-envelope`
50: 
51: - [x] `tpt-safety-interlock`: boolean permissive/blocking-condition data model (e.g. "suction pressure < X blocks pump start")
52: - [x] `tpt-safety-interlock`: evaluation engine — combinatorial logic (AND/OR/NOT chains) over live sensor inputs
53: - [x] `tpt-safety-interlock`: integration hook consuming live state (external dependency on `tpt-rust3`'s `tpt-state-snapshot`)
54:       - *Resolution:* consumed via the `LiveStateProvider` trait (defined in `tpt-control-action::live_state`); `tpt-state-snapshot` implements it.
55: - [x] `tpt-safety-interlock`: tests — permissive evaluation correctness, edge cases (missing/stale sensor input), `no_std`+`alloc` build
56: - [x] `tpt-safety-envelope`: alarm generation (thresholds, severity levels)
57: - [x] `tpt-safety-envelope`: E-stop logic — highest-priority override path, always wins regardless of other layers
58: - [x] `tpt-safety-envelope`: safe-state degradation paths when sensors fail or power is lost
59: - [x] `tpt-safety-envelope`: tests — E-stop precedence, degradation-path correctness under simulated sensor/power loss, `no_std`+`alloc` build
60: 
61: ## Phase 4 — Arbitration & Sequencing: `tpt-control-priority` + `tpt-state-machine`
62: 
63: - [x] `tpt-control-priority`: priority tiers (Safety > Manual Override > Auto-Optimization > Schedule) as a data model
64: - [x] `tpt-control-priority`: arbitration engine — resolves competing setpoints from multiple sources into one winner + reason
65: - [x] `tpt-control-priority`: tie-break / conflict rules explicitly documented and tested
66: - [x] `tpt-control-priority`: tests — arbitration correctness across all tier combinations, `no_std`+`alloc` build
67: - [x] `tpt-state-machine`: deterministic FSM core (states, transitions, guards, actions)
68: - [x] `tpt-state-machine`: auditability — every transition emits a structured, loggable record
69: - [x] `tpt-state-machine`: reference sequence implementation (e.g. boiler purge/start/stop) as a worked example
70: - [x] `tpt-state-machine`: tests — transition correctness, invalid-transition rejection, `no_std`+`alloc` build
71: 
72: ## Phase 5 — Verification & Audit: `tpt-control-dryrun` + `tpt-control-audit`
73: 
74: - [x] `tpt-control-dryrun`: shadow execution engine — runs a proposed control policy against current live state without emitting real commands
75: - [x] `tpt-control-dryrun`: diff/comparison API — proposed vs. live setpoints, surfaced for operator review or automated gating
76: - [x] `tpt-control-dryrun`: integration hook for live state (external dependency on `tpt-rust3`'s `tpt-state-snapshot`)
77:       - *Resolution:* same `LiveStateProvider` trait as interlock/envelope.
78: - [x] `tpt-control-dryrun`: tests — shadow-run correctness, no-side-effect guarantee (never touches the real daemon path)
79: - [x] `tpt-control-audit`: reason-code data model — why a setpoint was chosen, modified, or blocked, tied to the deciding crate (priority/interlock/envelope/gate)
80: - [x] `tpt-control-audit`: logging API called from each safety-decision point across the workspace
81: - [x] `tpt-control-audit`: persistence — integrate with `tpt-rust3`'s `tpt-audit-trail` (cryptographic hash-chained log) as the backing store rather than reimplementing chaining
82:       - *Resolution:* durable storage is behind the `PersistentAuditStore` trait (an `InMemoryAuditLog` backend implements it; `tpt-audit-trail` drops in behind the same trait).
83: - [x] `tpt-control-audit`: tests — reason-code completeness across all decision points, persistence round-trip
84: 
85: ## Phase 6 — `tpt-actuation-gate` (final pre-flight gate)
86: 
87: *(Capstone crate — validates a TUP command envelope against every safety layer above before handing it to `tpt-protocol-daemon`.)*
88: 
89: - [x] Aggregate validation pipeline: run a proposed command through `tpt-control-priority` → `tpt-safety-interlock` → `tpt-control-limiter`/`tpt-control-ratelimit` → `tpt-safety-envelope` (E-stop always wins) in a fixed, documented order
90: - [x] Integration with `tpt-control-dryrun` for optional shadow-verification before live gating
91: - [x] Reason-code emission via `tpt-control-audit` for every blocked/modified/passed command
92: - [x] **BLOCKED (external):** `tpt-protocol-daemon` has no command-ingest API today (WebSocket API is telemetry-outbound only) — final hand-off wiring cannot be completed until that lands upstream; build the gate's output as a well-defined validated-command type in the meantime
93:       - *Resolution:* the gate emits a typed `ValidatedCommand` rather than calling a not-yet-existing ingest API; the hand-off boundary is explicit and ready to wire up when the daemon exposes an ingest endpoint.
94: - [x] Tests — full pipeline ordering correctness, E-stop precedence under all combinations, reason-code completeness
95: 
96: ## Phase 7 — Cross-Crate Integration
97: 
98: - [x] Integration test/example: full Read Path — mock proposed envelope (standing in for `tpt-rust4`'s `tpt-dispatch-solve`) + mock live state (standing in for `tpt-rust3`'s `tpt-state-snapshot`) → arbitration → interlock → limiter/ratelimit → envelope → dryrun → `tpt-actuation-gate`
99:       - *Delivered:* `tpt-actuation-gate/tests/integration.rs` (full Read Path + E-stop drill + arbitration/dryrun cross-check + continuous clamp).
100: - [x] Verify combined `no_std`/`std` feature-flag builds across the whole workspace (per the Phase 1–5 no_std split)
101: - [x] E-stop drill: inject E-stop mid-pipeline at every stage, confirm it always wins regardless of arbitration/interlock outcome
102:       - *Delivered:* `estop_drill_always_wins_and_forces_safe_state` integration test + unit tests in `tpt-safety-envelope` and `tpt-actuation-gate`.
103: - [x] Documentation pass: top-level data-flow diagram (Read Path + Write Path), per-crate README, explicit "known upstream gaps" doc section (Command Envelope schema, daemon command-ingest API)
104:       - *Delivered:* `docs/DATA_FLOW.md`, `docs/KNOWN_GAPS.md`, and a `README.md` in all 10 crates.
105: 
106: ## Phase 8 — Publish Readiness (gated — requires explicit approval before publishing)
107: 
107: - [x] crates.io metadata for all 10 crates (description, keywords, categories, repository links)
108: - [x] Confirm dual-license files/fields consistent across every crate
109: - [x] Versioning strategy (start at `0.1.0`)
110: - [x] docs.rs configuration checks (`doc_cfg` for feature-gated std items)
111: - [ ] `cargo publish --dry-run` per crate in dependency order
112:       - *Deferred:* requires network/registry and all path-dependencies to be publishable; left for the explicit-approval publish step.
113: - [x] **Stop and get explicit approval before running real `cargo publish`**
114:       - *Status:* real publish intentionally NOT run — gated per checklist. All pre-publish metadata is in place.
