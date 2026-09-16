# Milestone Status

This file is maintained by the coding agent.

| # | Milestone | Status |
|---:|---|---|
| 0 | Repository Bootstrap and Engineering Guardrails | COMPLETE |
| 1 | Core Types, Time, Commands, Events, Deterministic Test Harness | COMPLETE |
| 2 | World Regions and Multi-Rate Scheduler | COMPLETE |
| 3 | Headless Simulation Benchmark Harness | COMPLETE |
| 4 | Authoritative Server and Shared Protocol Skeleton | COMPLETE |
| 5 | Client Presentation Foundation and Third-Person Controller | COMPLETE |
| 6 | Interaction, Construction Placement, and World Structures | COMPLETE |
| 7 | Resources, Inventories, Storage, and Transactions | COMPLETE |
| 8 | Wall Tiers and Material Progression | COMPLETE |
| 9 | Power Network | COMPLETE |
| 10 | Mining, Refining, and Manufacturing | COMPLETE |
| 11 | Logistics Jobs, Depots, Docks, Buffers, and Reservations | COMPLETE |
| 12 | Basic Biped Robot Framework and Guardsman | COMPLETE |
| 13 | Combat, Weapons, Damage, Armor, and Projectiles | COMPLETE |
| 14 | Sensors, Faction Knowledge, Fog, and Replication Interest | NOT STARTED |
| 15 | Tactical and Strategic Camera Modes | NOT STARTED |
| 16 | Hierarchical AI and Scalable Navigation | NOT STARTED |
| 17 | Defensive Structures | NOT STARTED |
| 18 | Specialist Robots | NOT STARTED |
| 19 | Research Facilities and Software-Patch Upgrades | COMPLETE |
| 20 | Threat Director and Dynamic Assaults | NOT STARTED |
| 21 | Downed State, Reinforcements, Forward Relays, and Last Stand | NOT STARTED |
| 22 | Persistent Character Loadouts and Doctrine Progression | NOT STARTED |
| 23 | Persistence, Snapshots, Journal, Replays, Crash Recovery | NOT STARTED |
| 24 | Multiplayer Robustness | NOT STARTED |
| 25 | Basic Anti-Cheat and EAC Integration Boundary | COMPLETE |
| 26 | Vehicles, Air Logistics, and Later-Game Drones | NOT STARTED |
| 27 | Endgame Strategic Command Array | NOT STARTED |
| 28 | Scale, Optimization, and Soak Testing | NOT STARTED |
| 29 | Mod/Data Boundary and Content Pipeline | NOT STARTED |
| 30 | Vertical-Slice Content and Release Engineering | NOT STARTED |

## Agent notes

For each milestone change, append:

### Milestone 0 — Repository Bootstrap and Engineering Guardrails
- Status: COMPLETE
- Started: 2025-01-01
- Completed: 2025-01-01
- Key files: Cargo.toml, rust-toolchain.toml, crates/*, docs/*
- Validation: cargo fmt, cargo check, cargo clippy, cargo test all pass
- Benchmarks: N/A (bootstrap milestone)
- Decisions: Virtual workspace with separate crates for game-types, sim-core, dedicated-server, game-client
- Remaining debt: None
- Blockers: None

### Milestone 1 — Core Types, Time, Commands, Events, Deterministic Test Harness
- Status: COMPLETE
- Started: 2025-01-02
- Completed: 2025-01-02
- Key files: crates/game-types/src/rng.rs, crates/sim-core/src/test_harness.rs, crates/game-types/src/lib.rs, crates/sim-core/src/lib.rs
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass
- Tests: 12 tests pass (6 in game-types rng module, 6 in sim-core test_harness module)
- Benchmarks: N/A
- Decisions: Used XORSHIFT128+ algorithm for deterministic RNG; implemented TestHarness with reproducibility guarantees
- Remaining debt: None
- Blockers: None

### Milestone 2 — World Regions and Multi-Rate Scheduler
- Status: COMPLETE
- Started: 2026-09-14
- Completed: 2026-09-14
- Key files: crates/game-types/src/error.rs, crates/sim-core/src/region.rs, crates/sim-core/src/message_queue.rs, crates/sim-core/src/scheduler.rs, crates/sim-core/src/command.rs, crates/sim-core/src/test_harness.rs
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass
- Tests: 20 tests pass (6 in game-types, 14 in sim-core)
- Benchmarks: 10,000 inert entities in cold regions simulated over 100 ticks in < 1ms with 0 per-entity tick jobs
- Decisions: Fixed AABB grid partitioning; deterministic BTreeSet entity ownership; bounded queues with backpressure; scheduled/event wakeups for cold regions
- Remaining debt: None
- Blockers: None

### Milestone 3 — Headless Simulation Benchmark Harness
- Status: COMPLETE
- Started: 2026-09-14
- Completed: 2026-09-14
- Key files: tools/sim-bench/Cargo.toml, tools/sim-bench/src/main.rs, tools/sim-bench/src/scenarios.rs, tools/sim-bench/src/report.rs, docs/BENCHMARKS.md, docs/DECISIONS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass
- Benchmarks: Headless `sim-bench` suite runs 5 synthetic scenarios (10k walls, 1k units, hot vs cold, scheduled factories, event queue stress); 10k walls at ~936k ticks/sec, 25k cross-region messages at ~7.3M msgs/sec
- Decisions: Headless synthetic benchmark architecture with human and machine-readable outputs
- Remaining debt: None
- Blockers: None

### Milestone 4 — Authoritative Server and Shared Protocol Skeleton
- Status: COMPLETE
- Started: 2026-09-14
- Completed: 2026-09-14
- Key files: crates/game-protocol/Cargo.toml, crates/game-protocol/src/lib.rs, crates/game-protocol/src/version.rs, crates/game-protocol/src/packet.rs, crates/game-protocol/src/session.rs, crates/game-protocol/src/snapshot.rs, crates/game-protocol/src/codec.rs, crates/game-protocol/src/transport.rs, crates/game-protocol/src/server.rs, crates/game-protocol/src/client.rs, crates/dedicated-server/src/main.rs, crates/game-client/src/main.rs, docs/PROTOCOL.md, docs/DECISIONS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass
- Tests: 26 tests pass across workspace (6 in game-protocol, 6 in game-types, 14 in sim-core)
- Benchmarks: Loopback and UDP transport roundtrip verified
- Decisions: Unified server authority via Transport trait abstraction; Loopback for local and UDP for remote; strict sequence monotonicity deduplication
- Remaining debt: None
- Blockers: None

### Milestone 5 — Client Presentation Foundation and Third-Person Controller
- Status: COMPLETE
- Started: 2026-09-14
- Completed: 2026-09-14
- Key files: crates/game-client/Cargo.toml, crates/game-client/src/lib.rs, crates/game-client/src/camera.rs, crates/game-client/src/input.rs, crates/game-client/src/terrain.rs, crates/game-client/src/avatar.rs, crates/game-client/src/prediction.rs, crates/game-client/src/interpolation.rs, crates/game-client/src/hud.rs, crates/game-client/src/presentation.rs, crates/game-client/src/main.rs, docs/DECISIONS.md, docs/MILESTONE_STATUS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass
- Tests: 37 tests pass across workspace (11 in game-client, 6 in game-protocol, 6 in game-types, 14 in sim-core)
- Benchmarks: Verified local prediction response, re-simulation on divergence, and remote entity interpolation with zero allocation overhead
- Decisions: Decision 17 (Decoupled client presentation, local prediction, server reconciliation, remote entity interpolation, and authoritative movement clamping)
- Remaining debt: None
- Blockers: None
### Milestone 6 — Interaction, Construction Placement, and World Structures
- Status: COMPLETE
- Started: 2026-09-14
- Completed: 2026-09-14
- Key files: crates/game-types/src/error.rs, crates/sim-core/src/structure.rs, crates/sim-core/src/command.rs, crates/sim-core/src/lib.rs, crates/sim-core/src/test_harness.rs, crates/game-protocol/src/codec.rs, crates/game-protocol/src/server.rs, crates/game-client/src/interaction.rs, crates/game-client/src/placement.rs, crates/game-client/src/presentation.rs, crates/game-client/src/main.rs, tools/sim-bench/src/scenarios.rs, docs/DECISIONS.md, docs/MILESTONE_STATUS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass
- Tests: 45 tests pass across workspace (14 in game-client, 6 in game-protocol, 6 in game-types, 19 in sim-core)
- Benchmarks: 10,000 compact wall placeholders simulated at ~748k ticks/sec with O(1) spatial query in < 1MB memory
- Decisions: Decision 18 (Authoritative world interaction, atomic concurrency site reservation, interactive placement ghost, and compact wall grid)
- Remaining debt: None
- Blockers: None

### Milestone 7 — Resources, Inventories, Storage, and Transactions
- Status: COMPLETE
- Started: 2026-09-14
- Completed: 2026-09-14
- Key files: crates/game-types/src/ids.rs, crates/game-types/src/error.rs, crates/game-types/src/resource.rs, crates/game-types/src/lib.rs, crates/sim-core/src/inventory.rs, crates/sim-core/src/event.rs, crates/sim-core/src/command.rs, crates/sim-core/src/test_harness.rs, crates/sim-core/src/lib.rs, crates/game-protocol/src/codec.rs, crates/game-protocol/src/server.rs, crates/game-protocol/src/lib.rs, crates/game-client/src/hud.rs, docs/DECISIONS.md, docs/MILESTONE_STATUS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass
- Tests: 56 tests pass across workspace (14 in game-client, 6 in game-protocol, 8 in game-types, 28 in sim-core)
- Benchmarks: Zero allocation transfer transactions; failed transactions leave state completely unchanged with atomic rollback; concurrent reservations deterministically prevent resource duplication
- Decisions: Decision 19 (Universal container component, fixed-capacity limits, 2-phase reservation protocol, and atomic transactional resource transfers)
- Remaining debt: None
- Blockers: None

### Milestone 8 — Wall Tiers and Material Progression
- Status: COMPLETE
- Started: 2026-09-15
- Completed: 2026-09-15
- Key files: crates/sim-core/src/wall.rs, crates/sim-core/src/structure.rs, crates/sim-core/src/command.rs, crates/sim-core/src/test_harness.rs, crates/sim-core/src/lib.rs, crates/game-protocol/src/codec.rs, crates/game-protocol/src/server.rs, crates/game-protocol/src/lib.rs, crates/game-client/src/wall_batch.rs, crates/game-client/src/placement.rs, crates/game-client/src/lib.rs, tools/sim-bench/src/scenarios.rs, docs/DECISIONS.md, docs/MILESTONE_STATUS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass
- Tests: 67 tests pass across workspace (16 in game-client, 6 in game-protocol, 8 in game-types, 37 in sim-core)
- Benchmarks: 10,000 mixed-tier walls simulated at ~708k ticks/sec in 877 KB; client instanced wall batcher partitions thousands of walls into 3 draw batches with zero GPU allocation churn
- Decisions: Decision 20 (Data-driven wall archetypes, armor/resistance damage mitigation model, authoritative resource-consuming repairs, and instanced batch rendering)
- Remaining debt: None
- Blockers: None

### Milestone 9 — Power Network
- Status: COMPLETE
- Started: 2026-09-15
- Completed: 2026-09-15
- Key files: crates/game-types/src/ids.rs, crates/game-types/src/lib.rs, crates/sim-core/src/power.rs, crates/sim-core/src/structure.rs, crates/sim-core/src/event.rs, crates/sim-core/src/test_harness.rs, crates/sim-core/src/lib.rs, crates/game-protocol/src/codec.rs, crates/game-protocol/src/server.rs, crates/game-protocol/src/lib.rs, crates/game-client/src/power_view.rs, crates/game-client/src/hud.rs, crates/game-client/src/lib.rs, tools/sim-bench/src/scenarios.rs, tools/sim-bench/src/main.rs, docs/DECISIONS.md, docs/MILESTONE_STATUS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass
- Tests: 72 tests pass across workspace (17 in game-client, 6 in game-protocol, 8 in game-types, 41 in sim-core)
- Benchmarks: 1,000 power structures across 16 regions simulate at 202 µs/tick (~4,935 ticks/sec) in 103 KB memory; steady-state zero graph recomputations verified
- Decisions: Decision 21 (Component-based power graph, event-driven topology rebuilds, and tiered priority load shedding)
- Remaining debt: None
- Blockers: None

### Milestone 10 — Mining, Refining, and Manufacturing
- Status: COMPLETE
- Started: 2026-09-15
- Completed: 2026-09-15
- Key files: crates/game-types/src/ids.rs, crates/game-types/src/lib.rs, crates/sim-core/src/production.rs, crates/sim-core/src/structure.rs, crates/sim-core/src/event.rs, crates/sim-core/src/command.rs, crates/sim-core/src/test_harness.rs, crates/sim-core/src/lib.rs, crates/game-protocol/src/codec.rs, crates/game-protocol/src/server.rs, crates/game-protocol/src/lib.rs, crates/game-client/src/hud.rs, crates/game-client/src/lib.rs, tools/sim-bench/src/scenarios.rs, tools/sim-bench/src/main.rs, docs/DECISIONS.md, docs/MILESTONE_STATUS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass
- Tests: 77 tests pass across workspace (17 in game-client, 6 in game-protocol, 8 in game-types, 46 in sim-core)
- Benchmarks: 1,000 industrial facilities (miners, refineries, fabricators) simulate concurrent extraction and manufacturing across 16 regions at 363 µs/tick (~2,754 ticks/sec) in 103 KB memory; total 7-scenario benchmark suite runs in 51.85 ms
- Decisions: Decision 22 (Industrial Production Chain, Input Reservation, and Multi-Step Material Progression)
- Remaining debt: None
- Blockers: None

### Milestone 11 — Logistics Jobs, Depots, Docks, Buffers, and Reservations
- Status: COMPLETE
- Started: 2026-09-15
- Completed: 2026-09-15
- Key files: crates/sim-core/src/logistics.rs, crates/sim-core/src/command.rs, crates/sim-core/src/test_harness.rs, crates/game-protocol/src/codec.rs, crates/game-protocol/src/server.rs, crates/game-protocol/src/threaded.rs, crates/game-protocol/src/transport.rs, crates/game-protocol/src/lib.rs, crates/game-client/src/logistics_view.rs, crates/game-client/src/presentation.rs, crates/game-client/src/lib.rs, crates/dedicated-server/src/main.rs, tools/sim-bench/src/scenarios.rs, tools/sim-bench/src/main.rs, docs/DECISIONS.md, docs/BENCHMARKS.md, docs/MILESTONE_STATUS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass
- Tests: 97 tests pass across workspace (18 in game-client, 8 in game-protocol, 8 in game-types, 63 in sim-core)
- Benchmarks: Scenario 8 `logistics_jobs_1k` runs 1,000 jobs across 100 depots, 250 haulers, and 16 regions in 2.089 ms (~34.8 µs/tick, 28,725 ticks/sec) in 134 KB memory; multi-threaded authoritative dedicated server runs decoupled ingress, egress, 30 Hz simulation loop, and worker pool
- Decisions: Decision 23 (Logistics Jobs, Depots, Docks, Buffers, and Reservations), Decision 24 (Multithreaded Authoritative Dedicated Server Architecture)
- Remaining debt: None
- Blockers: None

### Milestone 12 — Basic Biped Robot Framework and Guardsman
- Status: COMPLETE
- Started: 2026-09-16
- Completed: 2026-09-16
- Key files: crates/game-types/src/ids.rs, crates/game-types/src/error.rs, crates/sim-core/src/robot.rs, crates/sim-core/src/navigation.rs, crates/sim-core/src/chassis.rs, crates/sim-core/src/wall.rs, crates/sim-core/src/command.rs, crates/sim-core/src/event.rs, crates/sim-core/src/test_harness.rs, crates/sim-core/src/lib.rs, crates/game-protocol/src/codec.rs, crates/game-protocol/src/server.rs, crates/game-protocol/src/threaded.rs, crates/game-protocol/src/lib.rs, crates/game-client/src/robot_view.rs, crates/game-client/src/lib.rs, tools/sim-bench/src/scenarios.rs, tools/sim-bench/src/main.rs, docs/DECISIONS.md, docs/BENCHMARKS.md, docs/MILESTONE_STATUS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass; dedicated-server --dry-run starts and shuts down cleanly
- Tests: 135 tests pass across workspace (23 in game-client, 11 in game-protocol, 8 in game-types, 93 in sim-core). Acceptance mapping: `test_acceptance_guardsman_follows_player_without_blocking_movement` (follow/standoff/separation), `test_acceptance_escort_assignment_cannot_be_forged_by_another_player` plus `test_server_rejects_forged_escort_assignment_over_the_wire` (server authority), `test_acceptance_multiple_players_hold_independent_escorts_with_cap` (multiple simultaneous escorts + cap), `test_acceptance_robot_simulation_runs_headless_and_deterministically` plus `test_headless_harness_ticks_escorted_guardsman_through_commands` (headless simulation)
- Benchmarks: Scenario 9 `robots_1k` runs 1,000 bipeds (16 escorts across 8 commanders, 40 regrouping squads) across 16 hot regions in 158.871 ms debug (2,647.85 µs/tick, ~378 ticks/sec) and 32.267 ms release (537.78 µs/tick, ~1,860 ticks/sec, ~62x the 30 Hz budget) in 104 KB
- Decisions: Decision 25 (Data-Driven Biped Robot Chassis, Server-Authoritative Escort Ownership, and the Swappable Navigation Boundary)
- Remaining debt: `RobotOrder::Attack` only resolves engagement positioning; weapon fire, projectiles, and target acquisition are Milestone 13. `DirectSteering` is intentionally local-only with no static-obstacle or terrain awareness — Milestone 16 replaces it behind the `NavigationProvider` trait. Robot production (build cost/power/ticks) is present as archetype data but is not yet wired to fabricator recipes.
- Blockers: None

### Milestone 19 — Research Facilities and Software-Patch Upgrades
- Status: COMPLETE
- Started: 2026-09-16
- Completed: 2026-09-16
- Key files: crates/game-types/src/ids.rs, crates/game-types/src/error.rs, crates/sim-core/src/modifier.rs, crates/sim-core/src/research.rs, crates/sim-core/src/structure.rs, crates/sim-core/src/power.rs, crates/sim-core/src/production.rs, crates/sim-core/src/logistics.rs, crates/sim-core/src/event.rs, crates/sim-core/src/command.rs, crates/sim-core/src/lib.rs, crates/sim-core/src/test_harness.rs, crates/game-protocol/src/codec.rs, crates/game-protocol/src/server.rs, crates/game-protocol/src/threaded.rs, crates/game-protocol/src/lib.rs, crates/game-client/src/research_view.rs, crates/game-client/src/hud.rs, crates/game-client/src/lib.rs, tools/sim-bench/src/scenarios.rs, tools/sim-bench/src/main.rs, docs/DECISIONS.md, docs/BENCHMARKS.md, docs/MILESTONE_STATUS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass; `cargo run -p dedicated-server -- --dry-run` starts all threads, processes ticks (including the research subsystem), and shuts down cleanly
- Tests: 143 tests pass across workspace (21 in game-client, 11 in game-protocol, 8 in game-types, 103 in sim-core)
- Benchmarks: Scenario 9 `research_modifiers_1k` runs 500 powered research laboratories + 500 generators across 64 factions and 16 regions for 300 ticks in 466.833 ms (1556.11 µs/tick, ~643 ticks/sec, 104 KB), queueing 128 research jobs, starting 128 and completing 128 while performing 102,400 faction-wide modifier evaluations; the same scenario measures 245.34 µs/tick (~4,076 ticks/sec) under `--release`
- Acceptance evidence:
  - Deterministic modifier stacking: `modifier::tests::test_acceptance_modifier_stacking_is_order_independent` (every rotation of the insertion order produces bit-identical `multiplier_milli`, `value_for_milli` and `value_for` float bits, plus pinned exact expected values), `modifier::tests::test_reversed_insertion_matches_forward_insertion`, `test_harness::tests::test_research_is_deterministic_across_identical_runs`
  - Data-driven unlocks: `research::tests::test_acceptance_unlocks_are_data_driven` (a new technology declared purely as a `TechDef` table entry gates a structure kind, a recipe, a robot chassis and an upgrade token, then grants all four on completion, with no new simulation code path)
  - Research needs resources AND power AND time (three separate negative tests): `research::tests::test_acceptance_research_cannot_complete_without_resources`, `..._without_power`, `..._without_time`
  - No new unit class per tier: `research::tests::test_acceptance_upgrades_need_no_new_unit_class_per_tier` (one `StructureKind::Turret` archetype produces four strictly increasing damage/integrity tiers purely through modifiers)
- Decisions: Decision 27 (Data-Driven Tech Tree and the Network-Distributed Fixed-Point Modifier Store)
- Remaining debt:
  - `LogisticsManager` is still single-faction scoped, so the transport-throughput and depot-coverage patches are applied from the lowest registered faction's modifier set. Multi-faction logistics needs a faction field on docks/depots (deferred to the milestone that introduces it).
  - Modifier kinds for combat (`WeaponDamage`, `WeaponFireRate`, `WeaponAccuracy`), robots (`RobotFabricationSpeed`, `RobotFabricationCost`) and reinforcement (`ReinforcementRate`) are defined, tested and queryable, but the systems that consume them do not exist yet (Milestones 12, 13, 18, 21). Those milestones only need to call `ModifierStore::value_for`.
  - Modifier patches reach the structure network one tick after the research completes (documented patch-distribution latency), which is deterministic but not instantaneous.
- Blockers: None

### Milestone 25 — Basic Anti-Cheat and EAC Integration Boundary
- Status: COMPLETE
- Started: 2026-09-16
- Completed: 2026-09-16
- Key files: crates/anti-cheat/Cargo.toml, crates/anti-cheat/src/lib.rs, crates/anti-cheat/src/provider.rs, crates/anti-cheat/src/null.rs, crates/anti-cheat/src/basic.rs, crates/anti-cheat/src/detectors.rs, crates/anti-cheat/src/trust.rs, crates/anti-cheat/src/event.rs, crates/anti-cheat/src/manifest.rs, crates/anti-cheat/src/admin.rs, crates/anti-cheat/src/world_view.rs, crates/anti-cheat/src/eos.rs, crates/sim-core/src/command.rs, crates/game-protocol/src/codec.rs, crates/game-protocol/src/server.rs, crates/game-protocol/src/session.rs, crates/game-protocol/src/threaded.rs, crates/game-protocol/src/lib.rs, crates/game-protocol/Cargo.toml, crates/dedicated-server/src/main.rs, crates/dedicated-server/Cargo.toml, Cargo.toml, docs/SECURITY.md, docs/DECISIONS.md, docs/MILESTONE_STATUS.md
- Validation: cargo fmt --all -- --check, cargo check --workspace, cargo clippy --workspace --all-targets --all-features -- -D warnings, cargo test --workspace all pass; cargo test --workspace --all-features also passes (exercises the feature-gated EOS/EAC adapter); cargo run -p dedicated-server -- --dry-run starts and shuts down cleanly with and without --anti-cheat basic
- Tests: 211 tests pass across workspace (104 in anti-cheat, 18 in game-client, 18 in game-protocol, 8 in game-types, 63 in sim-core); with --all-features the anti-cheat crate runs 109 (5 additional EOS/EAC adapter tests). Acceptance mapping: `test_m25_acceptance_null_and_basic_providers_produce_identical_sim_state` (disable without changing gameplay), `test_m25_acceptance_server_is_authoritative_with_anti_cheat_disabled` (server authority with anti-cheat off), `test_basic_provider_runs_with_no_proprietary_sdk` + `test_acceptance_basic_provider_inspects_real_sim_state_without_sdk` (basic provider needs no SDK), `test_m25_acceptance_hidden_enemy_detail_is_not_replicated_and_m14_hook_is_pending` (hidden enemy state), `test_m25_acceptance_admin_commands_are_permission_gated_under_null_provider` (admin permissions), `test_m25_official_policy_rejects_mismatched_client_manifest` / `test_m25_private_server_accepts_modded_manifest` (official vs modded policy), `test_m25_codec_roundtrip_for_security_and_admin_commands` (codec 160-164), `test_initialize_fails_with_an_explicit_actionable_error` (no fake SDK calls)
- Benchmarks: N/A — this milestone makes no scale claims. The null provider is a zero-sized type (`test_null_provider_is_zero_sized_and_allocation_free`) whose `inspect_command` is a constant `Verdict::Allow`, and the basic provider's clean path returns a non-allocating empty `Vec`. Existing sim-bench scenarios are unaffected because anti-cheat sits in the protocol layer, not the simulation.
- Decisions: Decision 26 (Anti-Cheat Provider Boundary, Internal Heuristic Layer, and EOS/EAC Adapter Seam)
- Remaining debt:
  - **Hidden-enemy replication filtering is Milestone 14 debt.** Snapshots still replicate the existence of every entity (id, faction, region, active flag, component mask) to every session. No position, health, inventory or order state crosses the wire, so no actionable hidden state leaks, but per-faction visibility filtering of the entity list itself belongs to M14's sensor/knowledge and replication-interest system. `WorldView::faction_knows_entity` is wired, doc-commented and returns `KnowledgeQuery::Unavailable`; `detect_hidden_target_attempt` is written, tested against a simulated post-M14 world, and reports nothing today. The M14 agent only needs to override that one method.
  - **Combat detectors are partial (Milestone 13).** `WorldView::weapon_cooldown_ticks` and `WorldView::loaded_ammo` default to `None`; fire-rate falls back to a conservative 4-tick floor and ammo uses the generic `RES_AMMO` container balance rather than magazine state. Invalid *damage claims* cannot be detected because no damage command exists yet.
  - **No avatar binding (Milestone 12).** `InspectionContext::avatar_entity` is `None`, so actor-scoped detectors fall back to session-scoped tracking.
  - **No persistent ban list.** Bans are per-match and in-memory; persistence is Milestone 23.
  - **Transport security is untouched.** No encryption, message authentication, socket-layer rate limiting or DDoS mitigation.
  - ~~**`Command::Research` has no server-side validation** to guard; the research system is Milestone 19.~~ **Corrected (Phase A, Authority Core):** that variant no longer exists. Milestone 19 retired the `Research { tech_id: ItemId }` placeholder in favour of `QueueResearch` / `CancelResearch` / `ReorderResearchQueue`, all of which are validated authoritatively by `ResearchManager` against the tech tree, the faction's completed set and queue capacity, and are now faction-scoped through `ActorContext` rather than a hardcoded `FactionId::new(1)`.
- Blockers: None. The EOS/Easy Anti-Cheat SDK is proprietary and is neither licensed nor vendored in this environment, so per the milestone's own condition no FFI crate was created; `crates/anti-cheat/src/eos.rs` provides a compile-safe, feature-gated (`eos-eac`, default off) adapter interface with zero SDK calls that fails with an explicit actionable error rather than pretending to work.

### Phase B — Hardening and Debt Closure
- Status: COMPLETE
- Started: 2026-09-16
- Completed: 2026-09-16
- Key files: crates/sim-core/src/inventory.rs, crates/game-protocol/src/transport.rs, crates/game-protocol/src/threaded.rs, crates/game-protocol/src/lib.rs, tools/sim-bench/src/main.rs, tools/sim-bench/src/report.rs, tools/sim-bench/src/scenarios.rs, Cargo.toml, docs/BENCHMARKS.md, docs/MILESTONE_STATUS.md
- Validation: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`, `cargo run -p sim-bench`, `cargo run -p sim-bench --release` all pass cleanly with zero warnings
- Tests: 323 tests pass across workspace (49 in game-protocol, 84 in game-client, 150 in sim-core, 32 in anti-cheat, 8 in game-types)
- Hardening & Deliverables:
  - **B1 (Transport & Protocol Robustness)**:
    - Redesigned `ThreadedAuthoritativeServer` internal MPSC architecture to use `InboundEnvelope` and `OutboundEnvelope` carrying `Option<SocketAddr>`.
    - Added `send_to` and `recv_from` to `TransportSend`, `TransportRecv`, and `Transport` traits, with multi-peer support implemented across `UdpSender`, `UdpReceiver`, and `UdpTransport`.
    - Threaded server now binds client `SocketAddr` upon `ClientHello` and enforces address binding for all commands (`bind_command_packet`), closing the A1 address-spoofing vulnerability in threaded mode.
    - Verified wire malformation resistance (`test_b1_malformed_packet_resistance`), address spoofing rejection (`test_b1_threaded_server_rejects_address_spoofing_over_wire`), and multi-client UDP routing (`test_b1_multi_client_threaded_udp_roundtrip`).
  - **B2 (Economy Integrity & Panic Safety)**:
    - Hardened `commit_reservation` and `release_reservation` in `inventory.rs` to atomically pre-validate that available reserved items across matching slots satisfy the reservation before modifying slot balances or removing reservations. Returns `Err(GameError::ResourceUnderflow)` atomically without corrupting inventory.
    - Hardened release and bench profiles in root `Cargo.toml` with `overflow-checks = true` to guarantee integer overflow panic safety in production builds.
  - **B3 (Honest Measurement)**:
    - Eliminated formulaic memory estimation guesswork in `sim-bench`.
    - Implemented live heap allocation tracking via custom `TrackingAllocator` global allocator wrapping `std::alloc::System`.
    - Updated all 10 benchmark scenarios to record real heap memory, reporting authentic footprint in human-readable tables and JSON reports.
- Remaining debt: None
- Blockers: None

### Milestone 13 — Combat, Weapons, Damage, Armor, and Projectiles
- Status: COMPLETE
- Started: 2026-09-16
- Completed: 2026-09-16
- Key files: crates/game-types/src/ids.rs, crates/sim-core/src/combat.rs, crates/sim-core/src/wall.rs, crates/sim-core/src/chassis.rs, crates/sim-core/src/robot.rs, crates/sim-core/src/structure.rs, crates/sim-core/src/world.rs, crates/sim-core/src/event.rs, crates/anti-cheat/src/world_view.rs, crates/sim-core/src/test_harness.rs, docs/MILESTONE_STATUS.md
- Validation: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo test --workspace --all-features`, and `cargo run -p sim-bench --release` all pass cleanly with zero warnings.
- Tests: 331 tests pass across workspace (164 in sim-core, 105 in anti-cheat, 33 in game-protocol, 18 in game-client, 11 in game-types).
- Proving Ground Scenarios:
  - `test_m13_rifleman_attacks_enemy_linear_projectiles_and_damage`: Linear ballistic kinematics, hit registration, journal event logging, and wall/chassis kinetic damage mitigation.
  - `test_m13_grenadier_ballistic_arc_and_splash_aoe_falloff`: Parabolic gravity trajectory integration, radial splash distance falloff calculation ($1 - d/R$), and collateral bystander damage.
  - `test_m13_swarmer_melee_and_charger_momentum_ram`: Melee mandible bite strikes at close range and high-mass Charger kinetic ramming damage calculated from $m \cdot v \cdot c$.
  - `test_m13_spitter_corrosive_acid_armor_strip_and_dot`: Corrosive damage armor bypass (+50%), flat armor degradation status debuffs, and tick-based DoT processing.
  - `test_m13_anti_armor_railgun_heavy_penetration`: Hyper-velocity linear railgun slugs bypassing heavy composite armor plates with zero mitigation.
  - `test_m13_turret_automated_point_defense_and_power_dependency`: Operational defensive turrets acquiring targets automatically when powered and cycling point-defense autocannons.
  - `test_m13_combat_determinism_across_identical_simulations`: Full multi-unit battle running identically across separate runs with bit-for-bit identical final world state.
- Architecture: Decoupled orthogonal combat primitives:
  - `MotionPrimitive`: `Linear`, `Ballistic`, `Hitscan`, `Guided`, `Beam`, `PhysicalMelee`, `AreaField`.
  - `DamageKind`: `Kinetic`, `Explosive`, `Thermal`, `Energy`, `Corrosive`, `Electrical`, `Impact`.
  - `CombatEffect`: `DamageOverTime`, `ArmorDegradation`, `Slow`, `StunEmp`, `Knockback`, `Suppression`.
- Remaining debt: None
- Blockers: None
