# Project Directive: Rust Tactical Automation Warfare Game
## Master intake prompt + architecture + milestone execution plan

> **Purpose:** This file is the authoritative implementation brief for an autonomous coding agent.  
> **Target agent:** `qwen3-coder-next-q4km` running through PI, or another coding agent with shell/file access.  
> **Primary language:** Rust.  
> **Primary target:** Windows client + headless dedicated server, with Linux dedicated-server compatibility as an architectural requirement.  
> **Core multiplayer target:** 1–4 player cooperative play.  
> **Future-proofing:** PvP must remain possible without redesigning simulation, networking, sensors, or anti-cheat boundaries.

---

# 0. AGENT OPERATING CONTRACT

You are implementing a real game/engine project, not producing a design mockup.

Read this entire file before changing code.

Your job is to complete the milestones in order. A milestone is complete only when its acceptance criteria pass. Do not skip hard foundational work to make a flashy demo. Do not fake systems that later milestones depend on unless the milestone explicitly calls for a temporary adapter.

## 0.1 Rules you must follow

1. **Inspect before editing.** Read the repository structure, `Cargo.toml`, existing docs, tests, and implementation state. Reuse good existing work instead of rebuilding blindly.
2. **Work milestone-by-milestone.** Determine the first incomplete milestone, break it into the listed subtasks, and finish/validate all subtasks before moving on.
3. **Never claim success without evidence.** At minimum after each milestone run `cargo fmt --all -- --check`, `cargo check --workspace`, `cargo clippy --workspace --all-targets --all-features -- -D warnings` where practical, `cargo test --workspace`, plus milestone-specific tests/benchmarks. If something cannot run, record the exact reason.
4. **Preserve architectural boundaries.** Simulation does not depend on rendering. Server owns authoritative state. Client presentation is replaceable. Networking transports commands/state; it does not define gameplay. Anti-cheat is an adapter around a server-authoritative design, never the authority itself.
5. **Prefer data-oriented systems.** Use typed IDs, compact components, SoA/packed layouts where useful, bitsets/flags, sparse data, event-driven updates, and bounded queues. Avoid giant inheritance-style actor objects.
6. **Keep hot paths allocation-conscious.** Avoid per-frame/per-entity heap churn. Pool/reuse buffers where useful. Do not add a garbage-collected runtime to simulation hot paths.
7. **No global tick-everything-at-60-Hz design.** Use multi-rate simulation and event-driven systems.
8. **No client trust for authoritative outcomes.** Clients request actions; the server validates and applies them.
9. **Do not send hidden enemy state to clients.** Faction knowledge/sensor visibility must gate replication, especially for future PvP.
10. **Do not simulate bulk cargo as colliding physics items.** Logistics uses inventories, queues, reservations, docks, buffers, service rates, routes, and transactional transfers. Visual cargo is presentation only unless a specific gameplay item requires physics.
11. **Do not over-engineer distributed servers.** Initial world simulation runs on one authoritative process. Internally partition the world so future sharding remains possible.
12. **Avoid unsafe Rust unless necessary.** Any `unsafe` requires a concise safety comment and should be isolated behind a small audited boundary. FFI is an acceptable use case.
13. **Version durable/network formats.** Network protocol, save format, replay/event format, and mod/content manifests need explicit versioning.
14. **Do not silently recover from corrupt authoritative data.** Return typed errors, preserve corrupt state for diagnosis where practical, and never silently coerce invalid persistent state into valid defaults.
15. **A public feature is not complete without** implementation, tests, documentation, runnable example/integration path, and performance evidence where relevant.

## 0.2 Required progress files

Create and maintain:

- `docs/PROJECT_ARCHITECTURE.md`
- `docs/MILESTONE_STATUS.md`
- `docs/DECISIONS.md`
- `docs/BENCHMARKS.md`
- `docs/PROTOCOL.md`
- `docs/SAVE_FORMAT.md`
- `docs/SECURITY.md`

`docs/MILESTONE_STATUS.md` must list every milestone as `NOT STARTED`, `IN PROGRESS`, `BLOCKED`, or `COMPLETE`.

For completed milestones record date, key files changed, test commands, benchmark result if applicable, and unresolved non-blocking debt. If blocked, record the blocker, evidence, and the smallest next action.


## 0.3 FRESH REPOSITORY DETECTION — MANDATORY STARTUP STATE MACHINE

A missing `Cargo.toml`, missing `src/`, or missing `docs/` is **not an error** when this project has not been initialized yet.

At the beginning of every run:

1. List the repository root **once** using the agent's native directory-listing/file tool.
   - Prefer the native `list_directory`/filesystem tool.
   - On PowerShell, use `Get-ChildItem -LiteralPath . -Force`.
   - Do **not** use `dir /b` directly in PowerShell; if absolutely necessary use `cmd /c dir /b`.

2. Classify the repository into exactly one state:

### STATE A — FRESH_REPOSITORY

Treat the repository as fresh when:
- `RUST_GAME_AGENT_MASTER_SPEC.md` is present, and
- `Cargo.toml` is absent, and
- there is no existing Rust workspace under `crates/`, and
- there is no existing implementation that must be preserved.

In this state:
- Do **not** repeatedly attempt to read nonexistent `Cargo.toml`, `src/`, or `docs/`.
- Do **not** call project-memory/smart-recall repeatedly.
- If one memory lookup returns "no matching project memory", stop recalling and continue.
- Set Milestone 0 to `IN PROGRESS`.
- Bootstrap the repository immediately according to Milestone 0.

### STATE B — EXISTING_REPOSITORY

Treat the repository as existing when a `Cargo.toml` or implementation directories/files are present.

In this state:
- inspect the workspace,
- read existing architecture/docs/tests,
- preserve valid work,
- determine the first incomplete milestone.

### STATE C — AMBIGUOUS_REPOSITORY

Use this only when implementation-looking files exist but the workspace is incomplete or inconsistent.

In this state:
- inspect only the files that actually exist,
- document what was found,
- do not fabricate missing history,
- repair/bootstrap conservatively without deleting existing work.

3. Never loop on missing-path errors.
   After a path is confirmed absent, record that fact for the current run and move on.

4. Never require a root `src/` directory.
   This project is expected to use a **virtual Cargo workspace** with crates under `crates/`.

### Required Milestone 0 bootstrap for a fresh repository

For `FRESH_REPOSITORY`, create at minimum:

```text
Cargo.toml
rust-toolchain.toml

crates/
  game-types/
    Cargo.toml
    src/lib.rs
  sim-core/
    Cargo.toml
    src/lib.rs
  dedicated-server/
    Cargo.toml
    src/main.rs
  game-client/
    Cargo.toml
    src/main.rs

docs/
  PROJECT_ARCHITECTURE.md
  MILESTONE_STATUS.md
  DECISIONS.md
  BENCHMARKS.md
  PROTOCOL.md
  SAVE_FORMAT.md
  SECURITY.md
```

The root `Cargo.toml` should be a virtual workspace. Do not create a root package merely because the directory is empty.

Use Rust 2024 edition unless the installed toolchain proves incompatible, in which case document the reason before changing editions.

The initial binaries may be minimal placeholders, but:
- both must compile,
- the dedicated server must have no graphics dependency,
- the client and server must be separate workspace members,
- simulation code must live outside the presentation binary.

Copy/translate `MILESTONE_STATUS_TEMPLATE.md` into `docs/MILESTONE_STATUS.md` if the template exists. Do not treat the template itself as implementation state.

After bootstrap, run the Milestone 0 validation commands before marking it complete.

---

# 1. GAME IDENTITY

Working genre description:

> **Third-person tactical automation warfare:** third-person combat + base building + dynamic tower defense + light RTS command + industrial automation + 1–4 player co-op.

The player exists physically in the battlefield rather than floating permanently above it.

The game should evolve from exposed/manual survival into command of an increasingly autonomous industrial war machine.

## 1.1 Core gameplay loop

**Explore → Secure → Mine → Transport → Refine → Manufacture → Fortify → Automate → Expand → Fight → Research → Repeat at larger scale**

The player should progress from vulnerable individual operator → basic fortifications → factories and power → personal robot escorts → squads and automated defenses → distributed logistics/sensor network → autonomous war economy → endgame siege → final offensive.

## 1.2 Camera modes

All camera modes observe the same simulation.

### Third-person
Used for movement, aiming/shooting, interacting, construction, repairs, exploration, leading squads, and close combat.

### Tactical command view
Raised angled camera used for selecting squads/robots, issuing attack/defend/patrol/escort/repair orders, construction planning, and local battlefield management.

### Strategic network view
Used primarily for power, logistics, production, territory, sensors, threat, transport routes, factory queues, and strategic alerts.

**Important:** tactical/strategic information is limited by the faction knowledge/sensor network. Camera freedom must not equal omniscience.

---

# 2. WORLD AND PROGRESSION THEMES

## 2.1 Defensive material progression

### Mk.1 — Stone
Cheap, available early, basic protection, quick to build.

### Mk.2 — Iron/Steel
Midgame standard, substantially stronger, requires smelting/industry.

### Mk.3 — Tungsten Composite
Late-game fortress material.

Do not allow raw tungsten to become Mk.3 walls directly.

Suggested production chain:

**Tungsten Ore → Refined Tungsten**

Then:

**Refined Tungsten + Hardened Steel + Ceramic Plate → Tungsten Composite**

Tungsten Composite is used for Mk.3 walls, heavy robot armor, bunkers, high-end components, and strategic structures. Advanced refining should be power-hungry.

## 2.2 Research

Research remains understandable and StarCraft-like:
- research facilities unlock units/modules,
- unlock broad technologies,
- distribute percentage improvements as network/software patches.

Examples: +10% damage, +5% fire rate, +10% accuracy, faster mining, faster transport, faster robot fabrication, better power efficiency, reinforcement upgrades.

Avoid hundreds of opaque crafting substeps.

## 2.3 Early vs late automation

Early game should be more manual and exposed. Do **not** give the player a fully autonomous drone empire immediately.

Progress toward factories, land robots, automated logistics, vehicles, air support, combat drones, and potentially orbital systems. Drones are a later progression layer.

---

# 3. ROBOT / UNIT LANGUAGE

Ground combat robots use a cohesive faction design:
- bipedal/digitigrade land robots,
- dark industrial military chassis,
- angular armor,
- practical grounded mechanisms,
- modular loadouts,
- subtle machine accent lighting,
- not cute toy robots.

Initial roster direction:
- Guardsman / escort bot
- Rifle bot
- Anti-armor bot
- Heavy/frontline bot
- AA/support bot
- Recon bot
- Field Engineer / repair bot
- Logistics bot
- Artillery/siege bot
- Command/relay bot later

## 3.1 Guardsman
- Personal escort.
- 1–2 can eventually be assigned to a player.
- Follows/guards/assists.
- Part of player progression.

## 3.2 Anti-armor robot
- Heavy specialized biped.
- Lower mobility.
- Anti-vehicle / anti-heavy / anti-fortification role.
- Heavy cannon/recoilless/launcher role, optionally shoulder missile pods.
- Expensive enough that losing one matters.

## 3.3 Field Engineer / repair robot
Consumes actual materials to repair robots, walls, turrets, vehicles, sensor relays, and infrastructure. It does not generate free healing.

Potential inventory: steel, components, repair kits, advanced composite plates.

Behaviors: follow squad, repair units first, repair structures first, base defense, emergency player recovery.

Later upgrades: faster repair, larger capacity, better material efficiency, multi-target maintenance, rebuild wall segments, revive/stabilize downed players, auto-request resupply.

---

# 4. LOGISTICS AND INDUSTRY RULES

The logistics system is a core game system, not decorative conveyor spam.

## 4.1 No colliding physical cargo for the real economy

Use inventory quantities, queue/reservation docks, bounded buffers, transactional transfer, route capacity, service rate, and scheduled/aggregated distant transport.

Visual pallets/containers are presentation.

## 4.2 Universal container concept

Later mobile logistics can use standardized drop containers.

**Mobile transport → universal container/buffer → fixed facility network**

Containers decouple long-range mobile movement from local factory layout.

## 4.3 Depots

Supply depots should be material buffers, logistics request hubs, resupply points, and potentially powered coverage pylons for the local logistics network.

## 4.4 Reservation system

If three engineers see one damaged turret, only the successful reservation should commit to it unless the job explicitly supports multiple workers.

The same job/reservation concept applies to repair, hauling, construction delivery, factory input requests, and resupply.

---

# 5. RESPONSIVE ENGINE ARCHITECTURE

The target experience must support local high-frame-rate combat while distant industry continues efficiently.

> A player should be able to fight at high client FPS while distant mines, factories, transports, power networks, sensors, AI, repair systems, and enemy operations continue without requiring every entity to tick every frame.

## 5.1 Server authority

The authoritative server owns world state, entities, damage, resources, construction, research, AI, logistics, sensors, power, reinforcement state, and mission/endgame state. Clients send intent/commands.

## 5.2 Multi-rate simulation

Initial target classes:
- client render: independent, 60–240+ Hz as hardware allows,
- local input sampling: high rate,
- server simulation base: ~30 Hz,
- player/combat-critical movement: 30–60 Hz where needed,
- nearby combat AI: 15–30 Hz,
- squad logic: 5–10 Hz,
- strategic AI: 0.5–2 Hz,
- logistics matching: ~1–2 Hz or event driven,
- factories/research: ~1 Hz or scheduled completion,
- power topology: event driven,
- cold-region economy: event/scheduled aggregation.

These are targets, not magic constants. Benchmark and tune.

## 5.3 World simulation regions

Partition the world into fixed regions/cells with activity levels.

**HOT:** player nearby or active combat; high-fidelity simulation.

**WARM:** relevant activity but no nearby player; reduced AI/physics/update frequency.

**COLD/DORMANT:** stable distant infrastructure; event-driven or aggregate simulation.

Entities moving between regions transfer via explicit messages/commands.

## 5.4 Data orientation

Use compact typed IDs such as `EntityId`, `PlayerId`, `FactionId`, `RegionId`, `ResourceId`, `ItemId`, `StructureId`, `UnitArchetypeId`, `JobId`, and `SessionId`.

Avoid heavyweight per-object allocations for huge counts of walls/resources. Bulk resources are quantities in containers, not one entity per unit.

## 5.5 Suggested Rust workspace boundaries

```text
crates/
  game-types/
  sim-core/
  sim-world/
  sim-combat/
  sim-industry/
  sim-logistics/
  sim-power/
  sim-sensors/
  sim-navigation/
  sim-ai/
  game-protocol/
  net-server/
  net-client/
  persistence/
  anticheat-api/
  anticheat-basic/
  anticheat-eos/       # optional until EAC milestone
  dedicated-server/
  game-client/
tools/
  sim-bench/
  replay-tool/
```

Do not create crates merely for aesthetics. Stable interfaces matter more than maximal crate separation.

## 5.6 Rendering/presentation

Recommended direction:
- Rust-first simulation/server.
- Bevy is acceptable/recommended for client rendering, input, audio, asset integration, animation, UI, and presentation.
- Keep simulation independent enough that presentation could be replaced later.
- Do not put game authority inside Bevy rendering components.

A headless dedicated server must not require graphics/audio assets.

## 5.7 Physics

Use high-fidelity physics only where gameplay needs it.

Nearby: player collision, active projectiles, vehicles if implemented, combat interactions.

Distant: route/state simulation, no unnecessary rigid bodies.

Cosmetic destruction/debris should be client-side unless debris has explicit gameplay significance.

---

# 6. AI AND NAVIGATION

## 6.1 Hierarchical AI

Do not make every robot solve the whole war.

**Strategic director:** where to attack, what infrastructure is threatening, whether to raid power/logistics/sensors, composition and timing.

**Squad/formation AI:** route, formation, local objective, role positioning, fallback/rally.

**Individual robot AI:** local movement, target acquisition, cover/spacing, firing, avoidance, immediate survival.

## 6.2 Navigation hierarchy

Prefer:

**world route graph → regional path/corridor → squad flow field/path → local steering/avoidance**

Do not run expensive global A* separately every frame for every robot. Stable logistics routes should use route graphs and only recalculate when invalidated.

---

# 7. SENSOR / FACTION KNOWLEDGE NETWORK

This is a foundational engine system.

Sensors, command range, fog of war, tactical visibility, and multiplayer replication should derive from one faction-knowledge concept.

**Actual World → Sensor Evaluation → Faction Knowledge → Replication Interest → Client**

Faction knowledge may contain confirmed entities, friendly infrastructure, current sensor coverage, command coverage, last-known enemy contacts, contact confidence, last-known position/heading/time, and classification certainty.

If an enemy leaves sensor coverage, the client can retain a stale contact marker rather than the live entity.

Unknown enemy entities should not be replicated to the client merely as "invisible."

---

# 8. MULTIPLAYER MODEL

Target: **1–4 player co-op** first. Architecture should leave room for future PvP.

## 8.1 Client/server model

Single-player: **client → local authoritative server**

Multiplayer: **client → remote authoritative dedicated server**

Do not maintain separate gameplay logic for single-player and multiplayer.

## 8.2 Prediction model

Use prediction only where responsiveness requires it.

- player movement: client prediction + server reconciliation,
- aiming/input presentation: immediate local feedback,
- weapon visuals: immediate local feedback, authoritative server result,
- robots: server simulation + client interpolation,
- building: immediate placement ghost, server validates actual construction,
- tactical orders: immediate marker/UI, server validates/executes command,
- factories/economy: server only.

## 8.3 Interest management

Never replicate the whole world. Replicate based on local relevance, friendly faction state, sensor knowledge, command-network relevance, and strategic summaries.

## 8.4 Late join

Late join should receive current world snapshot, current faction state, nearby active entities, known sensor contacts, ongoing jobs/production, and subsequent deltas. Do not replay the entire match history to join.

## 8.5 Disconnects

Define safe behavior for player body state, escort ownership, queued commands, reconnection grace, and authority transfer if needed.

---

# 9. ANTI-CHEAT / SECURITY PLUMBING

The authoritative server is the first anti-cheat layer.

Add an anti-cheat provider interface early so Easy Anti-Cheat can be integrated later without contaminating gameplay code.

```rust
pub trait AntiCheatProvider {
    fn initialize(&mut self) -> Result<(), AntiCheatError>;
    fn begin_session(&mut self, player: PlayerId, session: SessionId)
        -> Result<(), AntiCheatError>;
    fn end_session(&mut self, player: PlayerId);
    fn poll(&mut self);
    fn player_status(&self, player: PlayerId) -> AntiCheatStatus;
    fn report_event(&mut self, event: SecurityEvent);
}
```

Implementations:
- `NullAntiCheat`
- `BasicAntiCheat`
- later `EosEasyAntiCheat`

Possible policy:
- local single-player: none,
- private co-op: host policy/basic,
- official co-op: stricter,
- future ranked PvP: EAC required.

Server validations should include impossible movement, fire rate/ammo/reload, invalid damage claims, invalid construction, impossible resource changes, invalid research, repair without material, unauthorized squad commands, attempts to target state the faction does not know, and replayed/malformed command envelopes.

Do not instant-ban solely from weak heuristic evidence. Record telemetry/security events.

Use command envelopes with session ID, monotonic sequence, client tick, and command.

Keep platform identity separate from internal `PlayerId`. Keep debug/admin cheats server-permissioned. Support a future distinction between official unmodified servers and modded/private servers.

---

# 10. PLAYER PERSISTENT PROGRESSION

Persistent progression exists outside individual matches and remains separate from match research/economy.

## 10.1 Persistent loadout

Categories:
- weapons,
- armor,
- accessories/tools,
- doctrine/talent points,
- cosmetics/blueprint access where appropriate.

Prefer sidegrades and specialization over endless raw power.

Example:
- light armor: speed/sensor/stamina,
- medium: balanced,
- heavy: protection/carry capacity with mobility cost.

Possible loadout slots: primary, secondary, tool, accessory, armor.

Avoid rigid permanent classes; loadouts create roles.

## 10.2 Doctrine/service points

Earn through meaningful accomplishments and difficulty: completing higher difficulties, killing armored/heavy enemies, successful defense, repairing large amounts, logistics/service contributions, completing missions without death, and strategic objectives.

Do not make one trivial monster farm the optimal progression path. Server validates awards. Future PvP must not become unwinnable for new players due to persistent stat inflation.

---

# 11. PLAYER DEATH / REINFORCEMENT SYSTEM

Do not use normal permanent death for the base co-op mode.

## 11.1 Downed state

On lethal damage:
- player becomes downed/disabled for a short window,
- teammate or qualified support/engineer can revive,
- if not recovered, player is lost and must reinforce.

## 11.2 Reinforcement charges

Respawn is infrastructure. Command Core owns reinforcement capability.

Team has finite charge capacity, renewable charges, recharge/manufacturing time, and resource/power cost.

Repeated deaths can add bounded temporary reinforcement fatigue: increasingly longer respawn delay, capped, decays after surviving for a while.

## 11.3 Difficulty scaling

Lower difficulty: larger reserve, faster regeneration, shorter delays.

Higher difficulty: lower reserve, slower/expensive recovery, potentially no passive recovery at extreme tiers.

## 11.4 Forward respawn nodes

Research can unlock forward command relays/reinforcement beacons. They require power, network/command connection, and valid faction infrastructure. Destroying/jamming/cutting a node removes that spawn point.

## 11.5 Command Core failure

As long as a valid Command Core exists, recovery is possible.

If the final functioning Command Core is destroyed:
- reinforcement network goes offline,
- surviving players enter **LAST STAND**,
- further deaths are permanent until recovery,
- team can potentially rebuild/recover the core under pressure,
- total team death with no recoverable state = defeat.

---

# 12. DYNAMIC TOWER-DEFENSE PRESSURE

Avoid relying only on arbitrary "Wave 17" timers.

Enemy pressure should respond to player activity.

Threat inputs may include territory, mining volume, power generation, military strength, sensor coverage, strategic construction, and endgame progress.

Responses may include reconnaissance, raids, armor pushes, air attacks, artillery, sabotage/network attacks, and major coordinated assaults.

The game can still use authored encounters, but systemic threat response should create the main tower-defense pressure.

---

# 13. ENDGAME

Endgame should test the entire machine the players built.

## 13.1 Strategic Command Array

Late-game objective requires enormous sustained industry: power, steel, tungsten composite, ceramics, electronics/components, and possibly coolant/specialized resources.

It consumes/depends on resources continuously enough that logistics and power stay relevant.

## 13.2 Activation siege

Array activation triggers escalating coordinated pressure:
1. reconnaissance/raids,
2. armor,
3. air attack,
4. artillery,
5. electronic/network disruption,
6. full assault.

All systems matter: AA, anti-armor, repair bots, walls, logistics, factories, sensors, power, players, escorts.

## 13.3 Final offensive

Successful activation reveals/disables the enemy strategic network and shifts the players from defense to offense. The factories and infrastructure built throughout the match now support the largest offensive of the match.

Do not end with only a scripted HP-sponge boss if the simulation can provide a systemic climax.

---

# 14. PERFORMANCE DESIGN TARGETS

These are architecture targets, not promises.

Initial goals:
- 1–4 players,
- 100–300 high-fidelity agents in a local major battle,
- 500–2,000 active autonomous units match-wide,
- 10,000–100,000 structures/wall segments,
- large aggregate resource quantities,
- ~30 Hz authoritative server base tick,
- render independent from server tick.

Every scaling milestone must benchmark CPU time by system, memory/entity, network bytes/sec/client, snapshot size, active vs cold-region cost, pathfinding jobs/sec, and AI jobs/sec.

---

# 15. MILESTONES

## MILESTONE 0 — Repository Bootstrap and Engineering Guardrails

### Objective
Create a clean Rust workspace and the process the autonomous agent will follow.

### Tasks
- Create workspace skeleton.
- Pin Rust edition/toolchain policy.
- Add formatting/lint/test scripts or documented commands.
- Create required docs files.
- Add CI configuration if repository supports it.
- Add `README.md` with build/run commands.
- Add logging/tracing foundation.
- Add typed error conventions.
- Add feature flags for client/server/dev tooling where useful.
- Create `docs/MILESTONE_STATUS.md` from this plan.
- Record initial architectural decisions.

### Acceptance
- Empty/minimal workspace builds.
- `cargo fmt`, `cargo check`, `cargo test` pass.
- Dedicated-server and game-client placeholder binaries both launch.
- Simulation crates have no rendering dependency.

---

## MILESTONE 1 — Core Types, Time, Commands, Events, and Deterministic Test Harness

### Objective
Establish a small reliable simulation kernel.

### Tasks
- Typed IDs.
- Fixed simulation tick type.
- Monotonic simulation clock.
- Versioned command envelope.
- Internal event type.
- Seeded RNG abstraction for simulation systems.
- Command buffer / commit boundary.
- Minimal entity registry or ECS foundation.
- Simulation test runner that advances N ticks deterministically for deterministic subsystems.
- Property/invariant tests for IDs, ticks, ordering, and command application.

### Acceptance
- Same seed + same command stream produces the same core test state.
- Invalid commands return typed errors.
- No wall-clock time inside authoritative simulation logic.

---

## MILESTONE 2 — World Regions and Multi-Rate Scheduler

### Objective
Make scale a first-class concern before adding gameplay volume.

### Tasks
- Fixed world-region IDs/bounds.
- Entity-to-region ownership.
- HOT/WARM/COLD state.
- Multi-rate scheduling buckets.
- Event/scheduled wakeups for cold systems.
- Region transfer command.
- Cross-region message queue.
- Bounded work queues/backpressure.
- Metrics for tick duration and jobs executed.

### Acceptance
- Test entities move between regions without duplication/loss.
- Cold regions cost measurably less than hot regions.
- A benchmark can simulate thousands of inert entities cheaply.
- No single global "tick every entity every frame" loop.

---

## MILESTONE 3 — Headless Simulation Benchmark Harness

### Objective
Make regressions visible before content complexity arrives.

### Tasks
- `sim-bench` tool.
- Synthetic scenarios: 10k walls, 1k idle units, hot vs cold regions, scheduled factories, event queue stress.
- Output machine-readable and human-readable results.
- Track baseline in `docs/BENCHMARKS.md`.

### Acceptance
- One command runs baseline scenarios.
- Results include CPU time, memory estimate/measurement, entity counts.
- Bench harness runs without graphics.

---

## MILESTONE 4 — Authoritative Server and Shared Protocol Skeleton

### Objective
Single-player and multiplayer share the same authority model.

### Tasks
- Headless authoritative server.
- Shared protocol crate.
- Client connection/session lifecycle.
- Protocol version handshake.
- Command sequence IDs.
- Snapshot/delta envelope types.
- Local loopback transport option.
- Remote transport abstraction.
- Basic connection timeout/reconnect handling.

### Acceptance
- One client connects to local server and remote server with same gameplay protocol.
- Server rejects wrong protocol version.
- Duplicate/out-of-order commands are handled safely.
- Client cannot directly mutate server world state.

---

## MILESTONE 5 — Client Presentation Foundation and Third-Person Controller

### Objective
Create a responsive playable client without leaking authority into rendering.

### Tasks
- Bevy-based client presentation layer or equivalent Rust presentation stack.
- Camera and input abstraction.
- Local predicted player movement.
- Server reconciliation.
- Client interpolation buffer for remote entities.
- Simple greybox terrain.
- Debug HUD: ping, server tick, region, predicted/authoritative position.
- Simple avatar placeholder.

### Acceptance
- Third-person player moves responsively.
- Artificial latency test demonstrates prediction/reconciliation.
- Server remains authoritative over legal movement.
- Headless server still runs with zero graphics dependencies.

---

## MILESTONE 6 — Interaction, Construction Placement, and World Structures

### Objective
Establish server-authoritative world interaction.

### Tasks
- Interaction ray/query.
- Placement ghost on client.
- Authoritative build request.
- Position/terrain/overlap/permission validation.
- Construction state machine.
- Structure ownership/faction.
- Dismantle/deconstruction command.
- Compact wall-segment representation.

### Acceptance
- Client can preview placement instantly.
- Invalid placements are rejected by server.
- Two clients cannot create conflicting authoritative structures through race conditions.
- 10k+ wall placeholders remain practical in headless benchmark.

---

## MILESTONE 7 — Resources, Inventories, Storage, and Transactions

### Objective
Create the economy primitives used everywhere else.

### Tasks
- Resource IDs and registry.
- Compact quantity type and overflow rules.
- Inventory/container component.
- Transactional reserve/commit/release.
- Capacity limits.
- Warehouse/depot storage.
- Transfer commands.
- Audit events for resource mutation.
- Corruption/invalid-state handling.

### Acceptance
- Resources cannot duplicate through concurrent reservations.
- Failed transaction leaves state unchanged.
- Overflow/underflow tested.
- Resource mutations are attributable in debug journal.

---

## MILESTONE 8 — Wall Tiers and Material Progression

### Objective
Implement first meaningful construction progression.

### Tasks
- Mk.1 Stone wall.
- Mk.2 Steel wall.
- Mk.3 Tungsten Composite wall.
- Armor/material resistance model.
- Damage/repair hooks.
- Construction cost data.
- Data-driven archetype definitions.
- Instanced/batched client rendering path for repeated segments.

### Acceptance
- All three tiers can be constructed through authoritative economy.
- Different durability/resistance is observable.
- Large wall counts remain performant.
- No unique hardcoded actor class per wall tier.

---

## MILESTONE 9 — Power Network

### Objective
Make energy a strategic dependency.

### Tasks
- Producer, consumer, relay, storage concepts.
- Graph topology.
- Event-driven topology rebuild/incremental update.
- Supply/demand accounting.
- Brownout/load-shedding policy.
- Powered/unpowered structure state.
- Debug visualization.

### Acceptance
- Destroying/disconnecting a relay can depower downstream infrastructure.
- Stable graph does not recompute unnecessarily each frame.
- Power state affects at least one real structure.

---

## MILESTONE 10 — Mining, Refining, and Manufacturing

### Objective
Build the first complete production chain.

### Tasks
- Deposit/resource node.
- Extraction job/rate.
- Refinery state machine.
- Factory recipe system.
- Scheduled completion.
- Input reservation.
- Output buffering.
- Simple steel chain.
- Tungsten chain: Tungsten Ore → Refined Tungsten; Refined Tungsten + Hardened Steel + Ceramic Plate → Tungsten Composite.
- Power requirements.

### Acceptance
- Player can produce Stone/Steel/Tungsten Composite through distinct progression.
- Factory cannot fabricate without reserved inputs/power.
- Distant production can run at low/event-driven frequency.
- Production state is serialization-ready.

---

## MILESTONE 11 — Logistics Jobs, Depots, Docks, Buffers, and Reservations

### Objective
Make material movement scalable without physics cargo chaos.

### Tasks
- Supply/demand job model.
- Job priority.
- Reservation/claim.
- Pickup/dropoff transaction.
- Depot buffer.
- Powered logistics coverage concept.
- Dock queue and service rate.
- Route graph.
- Abstract distant transport.
- Universal container/buffer data model for future vehicles/drones.
- Deadlock/starvation detection metrics.

### Acceptance
- Three haulers do not all claim one single-worker job.
- No resource duplication/loss across transfer.
- 1k+ logistics jobs benchmark.
- No bulk cargo collision simulation is required for the economy.

---

## MILESTONE 12 — Basic Biped Robot Framework and Guardsman

### Objective
Introduce the faction's core land-robot identity.

### Tasks
- Data-driven robot chassis/archetype.
- Biped presentation placeholder.
- Server movement state.
- Simple navigation.
- Health/armor.
- Faction.
- Squad membership.
- Guardsman escort assignment.
- Follow/guard/regroup commands.
- 1–2 escort cap as configurable progression rule.

### Acceptance
- Guardsman follows player without blocking movement excessively.
- Ownership/assignment survives server authority.
- Multiple players can each have escorts.
- Robot simulation can run headless.

---

## MILESTONE 13 — Combat, Weapons, Damage, Armor, and Projectiles

### Objective
Create server-authoritative combat.

### Tasks
- Weapon definitions.
- Ammo/reload/cooldown.
- Ray/hitscan path where appropriate.
- Projectile entity path for slower/special weapons.
- Server hit/damage authority.
- Armor/resistance.
- Area damage.
- Friendly-fire rules as configurable policy.
- Client muzzle/impact FX separated from authority.
- Basic lag compensation design for player fire.
- Combat telemetry.

### Acceptance
- Client cannot submit arbitrary damage amount.
- Fire-rate/ammo validation works.
- Armor materially changes outcomes.
- Rifle and anti-armor-style projectile paths both exist.

---

## MILESTONE 14 — Sensors, Faction Knowledge, Fog, and Replication Interest

### Objective
Implement the defining information architecture.

### Tasks
- Sensor emitters.
- Detection queries.
- Faction knowledge store.
- Known/unknown/live/stale contacts.
- Confidence/last-known data.
- Command coverage.
- Replication filter generated from faction knowledge + local relevance.
- Network metrics.
- Test with two factions even if public gameplay is co-op only.

### Acceptance
- Hidden enemy entity is not sent to unauthorized client.
- Leaving sensor range converts live contact to stale knowledge where appropriate.
- Friendly infrastructure remains known according to rules.
- Replication bytes drop when entities are outside relevance/knowledge.

---

## MILESTONE 15 — Tactical and Strategic Camera Modes

### Objective
Expose strategy layers without creating a separate game simulation.

### Tasks
- Smooth third-person ↔ tactical transition.
- Tactical selection.
- Squad order UI.
- Strategic network/map view.
- Sensor coverage overlay.
- Power overlay.
- Logistics overlay.
- Production summary.
- Camera can move over unknown terrain but must not reveal unauthorized state.

### Acceptance
- Same entities continue simulating during camera transition.
- Tactical orders are server commands.
- Unknown enemy state remains unavailable regardless of camera position.

---

## MILESTONE 16 — Hierarchical AI and Scalable Navigation

### Objective
Support large robot groups without each unit planning globally.

### Tasks
- Strategic objective interface.
- Squad goal/state.
- Formation slots.
- Shared route/corridor or flow-field concept.
- Local steering/avoidance.
- Path cache/invalidation.
- Route graph for long travel.
- Stuck detection/recovery.
- AI tick-rate LOD by region/activity.

### Acceptance
- Large squad travels together without every unit running global pathfinding each tick.
- Path invalidation reacts to destroyed/built obstacles.
- Benchmark 100–300 local combat/navigation agents.

---

## MILESTONE 17 — Defensive Structures

### Objective
Deliver the tower-defense toolkit.

### Tasks
- Automated ground turret.
- Sensory tower.
- Anti-air tower.
- Bunker/hardpoint.
- Target selection.
- Power dependency.
- Sensor dependency where appropriate.
- Firing arcs/range.
- Repairability.
- Placement/coverage overlays.

### Acceptance
- Sensory tower meaningfully extends faction knowledge.
- AA ignores invalid ground-only targets according to design.
- Turrets lose capability when required power/network is cut.
- Structures integrate with repair/logistics.

---

## MILESTONE 18 — Specialist Robots

### Objective
Expand robot ecosystem using composition rather than bespoke architecture.

### Tasks
- Rifle Bot.
- Anti-Armor Bot.
- Field Engineer.
- AA/support robot.
- Recon robot.
- Logistics robot.
- Optional Heavy Bot.
- Role-specific targeting/behavior.
- Material/production costs.
- Squad compatibility.

### Field Engineer specifics
- Material inventory.
- Repair job creation/claim.
- Structure/unit repair.
- Material consumption.
- Optional downed-player stabilization hook.

### Anti-Armor specifics
- Heavy weapon.
- Armor-target preference.
- Slower/heavier chassis.
- Bunker/heavy-wall effectiveness.

### Acceptance
- Each specialist is created from common composition primitives.
- Repair consumes materials transactionally.
- Anti-armor role is mechanically distinct from rifle spam.
- No free healing/material generation.

---

## MILESTONE 19 — Research Facilities and Software-Patch Upgrades

### Objective
Implement simple readable technology progression.

### Tasks
- Research facility.
- Research queue.
- Prerequisites.
- Unlocks.
- Network-distributed modifier system.
- Upgrades for damage, fire rate, accuracy, mining, transport, robot fabrication, power, repair, and reinforcement.
- Data-driven tech tree.
- UI/debug tree.

### Acceptance
- Modifier stacking is deterministic and test-covered.
- Unlocks are data-driven.
- Research cannot complete without authoritative resources/time/power.
- Upgrade system does not require new unit class per tier.

---

## MILESTONE 20 — Threat Director and Dynamic Assaults

### Objective
Turn economic growth into systemic defensive pressure.

### Tasks
- Threat score model.
- Inputs from territory/economy/power/military/sensors/objectives.
- Enemy recon.
- Raid generation.
- Composition escalation.
- Attack target selection: mines, power, logistics, sensors, core.
- Cooldown/budget system to avoid unfair spam.
- Difficulty modifiers.
- Deterministic test scenario for director decisions where possible.

### Acceptance
- Expanding industry changes enemy response.
- Director can attack infrastructure, not only nearest player.
- Difficulty changes pressure without simply multiplying enemy HP.

---

## MILESTONE 21 — Downed State, Reinforcements, Forward Relays, and Last Stand

### Objective
Make death meaningful and tied to infrastructure.

### Tasks
- Downed timer.
- Teammate revive.
- Engineer/support revive hook.
- Reinforcement charge pool.
- Charge regeneration/manufacturing.
- Power/resource requirements.
- Temporary reinforcement fatigue with cap/decay.
- Respawn selection.
- Forward reinforcement relay.
- Power/network requirement.
- Difficulty scaling.
- Command Core.
- Core destruction → reinforcement offline.
- Last Stand state.
- Core recovery/rebuild flow.
- Defeat condition.

### Acceptance
- Player cannot respawn when network is offline/no charges according to rules.
- Destroying forward relay removes spawn option.
- Core loss transitions to Last Stand without crashing/disconnecting players.
- Rebuilt core can restore recovery if rules/resources permit.

---

## MILESTONE 22 — Persistent Character Loadouts and Doctrine Progression

### Objective
Add between-match progression without corrupting match economy.

### Tasks
- Persistent profile format.
- Weapons.
- Armor categories.
- Accessories/tools.
- Loadout validation.
- Doctrine/service points.
- Challenge/accomplishment definitions.
- Difficulty-aware rewards.
- Server validation for awards.
- Migration/versioning.
- Separation between persistent profile and match state.
- Hooks for future cosmetics.

### Acceptance
- New profile can play.
- Old version migrates or fails with explicit error.
- Match research does not silently alter persistent profile.
- Persistent progression does not grant raw server-authoritative resources at match start unless explicitly designed.

---

## MILESTONE 23 — Persistence, Snapshots, Journal, Replays, Crash Recovery

### Objective
Make long-running matches robust.

### Tasks
- Versioned snapshot.
- Append-only command/event/security journal as appropriate.
- Async save path.
- Checksum/integrity metadata.
- Crash-safe temp/atomic publish strategy.
- Replay tool for supported deterministic/event-driven state.
- Save/load tests.
- Snapshot + subsequent delta recovery.
- Corruption errors.

### Acceptance
- Save/load reproduces supported world state.
- Save does not block simulation for unacceptable durations.
- Truncated/corrupt save fails explicitly.
- Server can recover from latest valid snapshot/journal boundary.

---

## MILESTONE 24 — Multiplayer Robustness

### Objective
Make 1–4 player co-op resilient.

### Tasks
- 4-player soak test.
- Late join.
- Reconnect.
- Disconnect ownership policy.
- Host/admin permissions.
- Latency/loss simulation.
- Snapshot bandwidth tuning.
- Delta compression where justified.
- Entity interpolation.
- Server backpressure.
- Load tests with active AI/industry.
- Metrics per client.

### Acceptance
- Four clients can join and play the same authoritative simulation.
- Late join obtains coherent current state.
- Packet loss/latency does not duplicate resources or commands.
- Disconnect/reconnect does not orphan critical authority.

---

## MILESTONE 25 — Basic Anti-Cheat and EAC Integration Boundary

### Objective
Ship a useful internal security layer and make future Easy Anti-Cheat integration straightforward.

### Tasks
- `AntiCheatProvider`.
- Null provider.
- Basic provider.
- Client trust/session state.
- Security events.
- Anomaly telemetry: movement, fire rate, ammo, invalid placement, impossible inventory/economy, unauthorized orders, hidden-target attempts.
- Build/protocol/content manifests.
- Official vs modded server policy.
- Admin command permissions.
- Document EOS/EAC adapter boundary.
- Create isolated FFI crate skeleton only if SDK is actually available/licensed in environment; otherwise provide compile-safe feature-gated adapter interface without fake SDK calls.

### Acceptance
- Basic provider runs with no proprietary SDK.
- Anti-cheat can be disabled for local dev without changing gameplay code.
- Gameplay crates do not depend directly on EAC/EOS.
- Server remains authoritative with anti-cheat disabled.
- Hidden enemy state still is not replicated.

---

## MILESTONE 26 — Vehicles, Air Logistics, and Later-Game Drones

### Objective
Add later progression without undermining early manual play.

### Tasks
- Generic vehicle/mover interface.
- Transport vehicle.
- Dock/service interaction.
- Universal container drop/pickup.
- Later-game logistics drone.
- Combat/support drone hooks.
- AA targeting integration.
- Fuel/power/ammo rules if used.
- Distant movement abstraction.
- Client presentation LOD.

### Acceptance
- Mobile logistics uses transactional container/buffer handoff.
- Drones are progression-gated.
- Bulk cargo still does not become colliding physics spam.
- AA system can distinguish air targets.

---

## MILESTONE 27 — Endgame Strategic Command Array

### Objective
Create a systemic climax that uses the whole economy.

### Tasks
- Endgame unlock prerequisites.
- Huge construction project.
- Continuous power/material demand.
- Activation state.
- Director escalation stages: recon, armor, air, artillery, network disruption, full assault.
- Failure/recovery conditions.
- Success transition.
- Final offensive objective.
- Victory state.

### Acceptance
- Endgame cannot be completed while ignoring logistics/power/defense.
- Endgame remains valid with 1–4 players.
- Array and assault survive save/load.
- Final offensive uses normal simulation systems rather than a completely separate scripted combat engine.

---

## MILESTONE 28 — Scale, Optimization, and Soak Testing

### Objective
Prove the architecture under representative load.

### Required scenarios
- 100k wall/structure representations.
- 2k match-wide autonomous units in mixed hot/warm/cold states where feasible.
- 100–300 local battle agents.
- Multiple active factories/refineries/mines.
- Active logistics job graph.
- Power network changes.
- Sensor replication for four clients.
- Endgame assault.
- Save during load.
- Client join during load.

### Optimize only from profiles
Use tracing, allocation profiling, flamegraphs/platform profilers, network byte accounting, and custom per-system metrics.

### Acceptance
- Publish measured results in `docs/BENCHMARKS.md`.
- Identify top bottlenecks.
- Fix architecture-level regressions before content polish.
- No blanket unsafe/hand-written allocator changes without measured justification.

---

## MILESTONE 29 — Mod/Data Boundary and Content Pipeline

### Objective
Allow content growth without recompiling core logic for every unit/recipe.

### Tasks
- Versioned data definitions for units, structures, recipes, weapons, resources, research, difficulty.
- Validation.
- Content manifest hashing.
- Official vs modded policy.
- Safe server-side loading.
- Editor/debug export format.
- Hot reload only in development if safe/practical.

### Acceptance
- Add a new simple unit/recipe/research item mostly through data.
- Invalid content gives actionable errors.
- Official server can require exact manifest.
- Private server can opt into custom manifest.

---

## MILESTONE 30 — Vertical-Slice Content and Release Engineering

### Objective
Turn the engine into a coherent playable slice.

### Minimum slice
- one map/biome,
- third-person combat,
- stone → steel → tungsten progression,
- power,
- mining/refining/manufacturing,
- logistics,
- guardsman,
- rifle bot,
- anti-armor bot,
- repair bot,
- turret,
- sensory tower,
- AA,
- tactical camera,
- dynamic raids,
- reinforcement/core system,
- 1–4 co-op,
- basic persistence,
- basic anti-cheat provider,
- one endgame Command Array victory path.

### Release engineering
- Windows client packaging.
- Windows dedicated server.
- Linux dedicated server build.
- Config files.
- Server logs.
- Crash diagnostics.
- Reproducible release build instructions.
- Version display.
- Save/protocol compatibility policy.

### Acceptance
A new developer/operator can:
1. build the project from documented instructions,
2. start a dedicated server,
3. join with 1–4 clients,
4. play the vertical slice from early gathering to endgame victory,
5. save/restart the world,
6. inspect meaningful logs/metrics.

---

# 16. IMPLEMENTATION ORDER INSIDE EACH MILESTONE

For every milestone, use this loop:

1. **Read** current code, related tests, architecture docs, milestone status.
2. **Plan** exact files/modules/interfaces, risks, and existing code to preserve.
3. **Implement smallest complete slice.** Avoid giant speculative abstractions, but create stable interfaces where later milestones clearly depend on them.
4. **Test** unit/integration/error/concurrency paths.
5. **Benchmark** when the milestone touches scale-sensitive systems.
6. **Document** architecture, protocol/save version changes, and important decisions.
7. **Validate workspace** with fmt/check/clippy/test.
8. **Update milestone status** only after acceptance criteria pass.
9. **Continue** to the next incomplete milestone unless genuinely blocked.

---

# 17. CROSS-CUTTING INVARIANTS

## Authority
- Client never authoritatively creates resources, applies damage, grants research, or decides hidden information.

## Simulation
- Rendering is not required for simulation.
- Wall/factory/resource scale does not imply per-frame heavyweight actors.
- Cold systems can be represented by scheduled state rather than continuous ticks.

## Economy
- Reservations are transactional.
- Failed transactions do not partially mutate state.
- Resources cannot underflow/overflow silently.

## Networking
- Protocol is versioned.
- Duplicate commands are safe.
- Hidden entities are not sent to unauthorized clients.
- Late join uses current state, not replay of all history.

## AI
- Strategic, squad, and individual responsibilities remain separated.
- Global pathfinding is not run independently for every unit every frame.

## Security
- Basic server validation works even with EAC disabled.
- EAC/EOS integration stays behind adapter/FFI boundaries.
- Admin/debug authority is server-permissioned.

## Persistence
- Corrupt saves do not silently become valid defaults.
- Save formats are versioned.
- Published snapshots are immutable once committed.

## Progression
- Persistent operator progression is distinct from match industry/research.
- Early game remains more manual than late game.
- Drones/advanced automation are progression rewards.

---

# 18. TEST SCENARIOS THE AGENT SHOULD BUILD OVER TIME

Maintain reusable integration scenarios.

### Scenario A — Economy integrity
Mine ore → reserve transport → deliver → refine → fabricate → cancel one job halfway → verify no duplication/loss.

### Scenario B — Power failure
Power factory + sensors + turret through relay → destroy relay → verify downstream behavior → restore relay.

### Scenario C — Sensor secrecy
Two factions → hidden enemy outside coverage → verify no replication → detect it → lose contact → verify stale contact semantics.

### Scenario D — Repair logistics
Damage Mk.3 wall → create tungsten-composite repair demand → engineer reserves material → logistics resupplies → repair completes → verify exact quantities.

### Scenario E — Multiplayer race
Two players attempt same limited resource/build slot/job → only valid authoritative outcome commits.

### Scenario F — Reinforcement failure
Player dies → consumes charge → destroy forward relay → destroy core → enter Last Stand → rebuild core → restore network.

### Scenario G — Dynamic threat
Grow industry/power → verify threat rises → verify director chooses infrastructure attack → scale difficulty.

### Scenario H — Endgame
Construct Command Array → sustain logistics → survive escalations → transition to final offensive → win → save/reload during activation.

---

# 19. METRICS TO EXPOSE FROM EARLY BUILDS

Server:
- tick duration p50/p95/p99,
- time per simulation system,
- hot/warm/cold region count,
- entity counts by archetype/component,
- AI jobs queued/completed,
- path jobs queued/completed,
- logistics jobs queued/reserved/starved,
- power graph update time,
- sensor query time,
- replication bytes/client,
- snapshot size/time,
- save duration,
- command rejection reasons,
- anti-cheat anomaly counters.

Client:
- frame time,
- interpolation delay,
- prediction correction magnitude,
- entities rendered,
- draw/instance counts,
- network RTT/jitter/loss,
- bytes/sec.

---

# 20. NON-GOALS FOR EARLY MILESTONES

Do not derail foundations by prematurely building:
- photoreal final art,
- orbital warfare,
- giant public MMO infrastructure,
- elaborate monetization,
- huge PvP ranking systems,
- kernel anti-cheat,
- fully simulated individual cargo pieces,
- custom renderer from scratch unless the existing presentation stack proves insufficient,
- hundreds of unit types,
- procedural everything.

Build the systemic spine first.

---

# 21. DEFINITION OF THE FIRST TRUE PLAYABLE BUILD

The first meaningful vertical slice should feel like this:

1. Player joins locally or through dedicated server.
2. Starts exposed with basic gear.
3. Gathers/mines early resources.
4. Builds Mk.1 stone defenses and power.
5. Establishes steel processing.
6. Builds Mk.2 defensive infrastructure.
7. Unlocks first biped robots and a Guardsman.
8. Establishes logistics/depot network.
9. Enemy pressure responds to expansion.
10. Builds sensors and uses tactical view.
11. Produces specialist Anti-Armor and Field Engineer robots.
12. Researches broad upgrades/software patches.
13. Builds tungsten industry.
14. Constructs Mk.3 Tungsten Composite fortification around critical systems.
15. Players can be downed/revived/reinforced.
16. Losing the Core creates a Last Stand.
17. Team builds/activates Strategic Command Array.
18. Survives systemic endgame siege.
19. Launches final offensive.
20. Wins through the same simulation systems used all game.

If a feature does not help this loop or its scalability, question whether it belongs before the vertical slice.

---

# 22. INITIAL AGENT START COMMAND

After reading this file, begin with the following behavior:

> Read this file completely, then list the repository root once and classify it as `FRESH_REPOSITORY`, `EXISTING_REPOSITORY`, or `AMBIGUOUS_REPOSITORY` using Section 0.3. If `Cargo.toml` is absent and only the specification/template files exist, this is a valid fresh repository: do not loop on missing files, do not repeatedly call smart/project memory, and begin Milestone 0 immediately by creating the virtual Rust workspace and required docs. If implementation already exists, inspect only paths that actually exist and determine the first genuinely incomplete milestone. Create or update `docs/MILESTONE_STATUS.md`, implement every subtask for the active milestone, run its validation commands, fix failures before marking it complete, record decisions/benchmarks, and continue in order until genuinely blocked or the assigned run ends. Never skip acceptance criteria, silently weaken tests, or replace server authority with client trust for convenience.

---

# 23. ARCHITECTURAL NORTH STAR

When uncertain between two implementations, prefer the one that preserves these properties:

> **Fast local action, scalable distant simulation, authoritative multiplayer, sensor-limited information, transactional industry/logistics, modular robot composition, event-driven infrastructure, and a clean Rust simulation core that remains independent of presentation and proprietary anti-cheat SDKs.**

The game should eventually make the player feel that they have progressed from a vulnerable operator into the architect of a functioning autonomous military-industrial network.

That is the project.
