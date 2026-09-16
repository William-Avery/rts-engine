# Architectural Decisions

This document records key architectural decisions and their rationales.

## Decision 1: Virtual Workspace

**Decision**: Use a virtual Cargo workspace with no root package.

**Rationale**: This project consists of multiple crates that should be developed together but have distinct responsibilities. A virtual workspace allows independent versioning and testing while sharing the build configuration.

## Decision 2: Rust 2024 Edition

**Decision**: Use Rust 2024 edition.

**Rationale**: The latest edition provides the most modern features and safety guarantees. If the installed toolchain proves incompatible, we will document the reason and adjust.

## Decision 3: Typed IDs

**Decision**: Use newtype wrappers for all IDs (EntityId, PlayerId, etc.).

**Rationale**: Type safety prevents mixing different ID types and makes the code self-documenting. The wrapper can be transparent with `#[repr(transparent)]`.

## Decision 4: No Root src/

**Decision**: Do not create a root src/ directory.

**Rationale**: This is a workspace project. Each crate has its own src/ directory. A root src/ would be redundant and confusing.

## Decision 5: SimTick as u64

**Decision**: Represent simulation ticks as u64.

**Rationale**: u64 provides ample range for simulation duration. The wrapper type prevents mixing with wall-clock time.

## Decision 6: Multi-Rate Scheduler Buckets

**Decision**: Implement a scheduler with buckets (High, Medium, Low, Event).

**Rationale**: Different systems have different update frequency requirements. Combat needs high frequency; strategic AI needs low frequency. This prevents over-processing.

## Decision 7: No Global Tick-Every-Entity

**Decision**: Do not implement a global tick that updates every entity every frame.

**Rationale**: This would be prohibitively expensive for 1000s of entities. Instead, use event-driven and scheduled updates based on region state and importance.

## Decision 8: Server-Authoritative Model

**Decision**: Server owns all authoritative state; clients send commands.

**Rationale**: This is essential for multiplayer fairness and anti-cheat. Single-player uses a local server for consistency.

## Decision 9: Command Envelope

**Decision**: Wrap commands with session_id, sequence, and client_tick.

**Rationale**: This enables replay detection, out-of-order handling, and client-side prediction reconciliation.

## Decision 10: Event Journal

**Decision**: Record events with tick timestamps for replay and debugging.

**Rationale**: This enables deterministic replays and post-mortem debugging without recording all state snapshots.

## Decision 11: Fixed World Region Partitioning with AABB Bounds

**Decision**: Partition world space into fixed regions using 2D axis-aligned bounding boxes (XZ plane) with grid indexing.

**Rationale**: Fixed spatial partitions provide $O(1)$ coordinate-to-region lookups and allow the engine to scale by segmenting simulation workload into independent regions.

## Decision 12: BTreeSet for Deterministic Entity Ownership

**Decision**: Use `BTreeSet<EntityId>` for region entity ownership sets and `BTreeMap` for regional routing tables instead of hash sets/maps.

**Rationale**: Deterministic iteration order is required across all platforms and execution runs for lockstep simulation, replay consistency, and rollback netcode.

## Decision 13: Bounded Cross-Region Message Queues with Backpressure

**Decision**: Decouple inter-region state mutations via explicit message queues bounded by maximum capacity and configurable backpressure policies (`Reject`, `DropOldest`).

**Rationale**: Unbounded queues cause memory bloat and latency spikes under heavy load. Bounded queues provide predictable resource consumption and fail-fast guarantees.

## Decision 14: Scheduled and Event-Driven Wakeups for Cold Regions

**Decision**: Cold regions are dormant and execute zero jobs and zero entity iterations per tick unless an event arrives or a tick in the `ScheduledWakeup` min-heap triggers.

**Rationale**: Ticking thousands of inert entities or distant factory units every frame is wasteful. Wakeup queues provide scalable simulation of large worlds with thousands of entities.

## Decision 15: Headless Synthetic Benchmark Architecture

**Decision**: Implement `sim-bench` as an autonomous, headless command-line tool with dedicated synthetic stress scenarios (10k walls, 1k units, hot vs cold, scheduled wakeups, event queue stress) capable of emitting both human-readable ASCII tables and machine-readable JSON reports.

**Rationale**: Catching performance regressions early requires reproducible benchmarks that run in CI and headless environments without graphic hardware or windowing systems.

## Decision 16: Unified Server Authority via Transport Abstraction

**Decision**: Enforce an identical client/server network state machine for both single-player and multiplayer by abstracting packet transmission behind a `Transport` trait, using `LoopbackTransport` for local play and `UdpTransport` for remote networking.

**Rationale**: Maintaining separate single-player and multiplayer simulation pipelines creates divergence bugs and desync risks. By running the authoritative server loop even locally over thread-safe memory channels, single-player and multiplayer remain bug-compatible and share identical validation logic.

## Decision 17: Decoupled Client Presentation, Local Prediction, and Server Reconciliation

**Decision**: Implement a decoupled, zero-external-dependency Rust presentation foundation in `crates/game-client` comprising a third-person orbital camera, input mapping, greybox terrain with collision resolution, local client prediction with input history re-simulation upon server divergence, remote entity snapshot interpolation, avatar representation, and a diagnostic telemetry HUD. The headless simulation server retains complete authority over movement validation and speed/boundary clamping.

**Rationale**: Adheres to Master Spec Section 5.6 and Milestone 5 rules:
1. **Simulation/Presentation Decoupling**: Simulation logic and authoritative validation remain strictly independent of graphics/windowing libraries, preserving <0.5s workspace compilation and 100% headless CI reproducibility.
2. **Instant Responsiveness**: Client-side prediction steps kinematic movement immediately upon user input without waiting for network roundtrip latency.
3. **Strict Server Authority**: Illegal client movements (speed hacking, teleportation, wall clipping) are authoritatively clamped by the server. When the authoritative state is received, the client detects divergence beyond an epsilon threshold, snaps to authoritative state, and re-simulates unacknowledged inputs cleanly.

## Decision 18: Authoritative World Interaction, Concurrency Site Reservation, and Compact Wall Grid

**Decision**: Implement world structures in `crates/sim-core/src/structure.rs` managed by `StructureRegistry`, with raycast queries and interactive placement ghosts in `crates/game-client`:
1. **Instant Client Preview**: Interactive placement ghost projects camera ray to the ground plane, snaps to the construction grid, checks distance and obstacle overlaps, and previews valid/invalid states locally with zero server latency.
2. **Strict Server Authority**: The server validates all placement requests (`BuildRequest`) against max reach distance (15m), world boundaries, terrain collision, and structure overlap.
3. **Atomic Concurrency Protection**: Footprint cells are atomically reserved on the authoritative grid. If two clients race to place structures on the same or overlapping site on the same tick, the first acquires the reservation and the second is rejected with `GameError::SiteOccupied`, guaranteeing zero structure duplication or corrupt state.
4. **Compact Wall Grid**: Static wall segments are stored in `CompactWallGrid` using coordinate-indexed cells, keeping 10,000+ wall placeholders under 1 MB of memory with microsecond $O(1)$ spatial queries.

## Decision 19: Universal Container Component, 2-Phase Reservations, and Atomic Transactional Transfers

**Decision**: Implement the game economy foundation in `crates/game-types/src/resource.rs` and `crates/sim-core/src/inventory.rs`:
1. **Resource Archetypes & Checked Quantities**: Define canonical resource catalog (raw minerals, refined alloys, energy, manufactured components) with unit volume ($m^3$), mass ($kg$), stack limits, and compact `ResourceQuantity` wrapper enforcing overflow and underflow checks.
2. **Fixed-Capacity Containers**: Model containers (`Backpack`, `Depot`, `Silo`, `Hopper`) with explicit slot limits and volumetric limits ($L$), with specialized acceptance rules (e.g. Silos restricted to bulk raw minerals).
3. **Two-Phase Reservation Protocol**: Support non-duplicating resource locks via `two_phase_reserve(ReserveRequest, ...)`. Reserved quantities are locked against the source inventory's available balance. Concurrent orders racing for finite supplies fail deterministically with `InsufficientUnreservedBalance`, guaranteeing zero resource duplication.
4. **Zero-Loss Transactional Atomicity**: Both direct transfers and reservation commits validate destination capacity before mutating balances; failures leave both source and destination 100% unchanged. Aborted reservations release locked resources back to available balance without item loss.
5. **Auditing & Invariant Validation**: Every balance mutation logs structured audit events (`ResourceReserved`, `ResourceCommitted`, `ResourceReleased`, `ResourceTransferred`) in `EventJournal`. Runtime invariant validation (`validate_invariants()`) detects any corruption (e.g., reserved > quantity, map mismatch, volume overflow) with typed `GameError::CorruptedState`.

## Decision 20: Data-Driven Wall Archetypes, Material Resistance Model, and Batched Client Rendering

**Decision**: Implement the defensive material progression in `crates/sim-core/src/wall.rs`, `crates/sim-core/src/structure.rs`, and `crates/game-client/src/wall_batch.rs`:
1. **Data-Driven Wall Archetypes (No Hardcoded Actor Classes)**: All wall tiers (Mk.1 Stone, Mk.2 Steel, Mk.3 Tungsten Composite) share the universal authoritative `Structure` entity, dynamically parameterized by `WallArchetype` definitions:
   - **Mk.1 Stone**: 1,000 HP, 5 flat armor, 5% damage reduction, cost: 20 Stone, repair: 1 Stone = 50 HP.
   - **Mk.2 Steel**: 3,000 HP, 20 flat armor, 25% damage reduction, cost: 15 Steel, repair: 1 Steel = 100 HP.
   - **Mk.3 Tungsten Composite**: 8,000 HP, 50 flat armor, 50% damage reduction, cost: 10 Tungsten Composite + 5 Steel, repair: 1 Repair Kit = 250 HP.
2. **Authoritative Resistance & Armor Damage Calculation**: Damage is mitigated by both flat armor (reduced by weapon penetration) and multiplicative percentage damage reduction:
   $$\text{post\_armor} = \max(0, \text{raw\_damage} - \max(0, \text{flat\_armor} - \text{armor\_penetration}))$$
   $$\text{effective\_damage} = \text{post\_armor} \times (1.0 - \text{damage\_reduction})$$
   Lethal damage transitions the structure to `Destroyed` and immediately cleans up spatial grid reservations and compact wall grid cells.
3. **Authoritative Economy Validation & Resource-Consuming Repairs**:
   - `StructureRegistry::request_build` validates material balance and authoritatively deducts construction costs from builder or depot containers.
   - `StructureRegistry::request_repair` calculates missing durability and authoritatively consumes corresponding repair units from the builder's inventory, clamping restoration to `max_health`.
4. **Batched Client Rendering Representation**:
   - `BatchedWallRenderer` partitions thousands of mixed-tier walls into at most 3 compact instanced draw calls (one per `WallTier`).
   - Reusing pre-allocated instance buffers across frames eliminates buffer reallocations and GPU memory churn.

## Decision 21: Component-Based Power Graph, Event-Driven Topology Rebuilds, and Tiered Priority Load Shedding

**Decision**: Implement the authoritative electrical power grid in `crates/sim-core/src/power.rs`, `crates/sim-core/src/structure.rs`, and `crates/game-client/src/power_view.rs`:
1. **Producer, Consumer, Relay, and Battery Roles**:
   - Model all power-participating entities with data-driven `PowerSpec` definitions attached to structures:
     - `Generator`: 100 kW generation, 0 demand, 20m connection radius.
     - `Battery`: 5,000 kWh capacity, 50 kW max charge/discharge rate, 15m radius.
     - `Pylon`: 0 kW generation/demand, 25m relay radius.
     - `Fabricator`: 35 kW demand, high priority, 12m radius.
     - `Turret`: 25 kW demand, high priority, 10m radius.
     - `Wall`: Passive (no power required).
2. **Event-Driven Topology Partitioning & Zero-Allocation Steady State**:
   - Nodes are grouped into isolated connected components (`PowerSubnet`) using spatial range connectivity filtered strictly per `FactionId`.
   - The topology graph is flagged `topology_dirty = true` *only* upon structure placement, destruction, dismantle, or operational toggle. Steady-state simulation frames perform numerical accounting only and bypass graph traversals entirely.
3. **Multi-Tiered Priority Load Shedding & Battery Buffers**:
   - When demand exceeds generation within an islanded subnet, battery reserves discharge up to their max rate to satisfy deficits.
   - If energy is still insufficient, load shedding executes authoritatively across priority tiers:
     - `High` (Turrets, Shields) are preserved first.
     - `Normal` (Fabricators, Refineries) are throttled next.
     - `Low` (Logistics Depots, Auxiliaries) are shed first.
   - Structures in unpowered or brownout states have `is_operational() == false`, safely inhibiting operational execution (e.g. `Turret::can_fire()` and `Fabricator::can_operate()`).
4. **Client Power Telemetry & Diagnostic Visualization**:
   - `PowerViewSnapshot` generates visual transmission links between connected nodes and coverage circles for relays.
   - Color coding highlights power status (Green = Powered, Yellow = Brownout, Red = Unpowered, Cyan = Transmission links).
   - Authoritative status events (`PowerBrownoutStarted`, `PowerBlackoutStarted`, `PowerRestored`) replicate through the `EventJournal`.

## Decision 22: Industrial Production Chain, Input Reservation, and Multi-Step Material Progression

**Decision**: Implement the mining, refining, and manufacturing engine in `crates/sim-core/src/production.rs`, `crates/sim-core/src/structure.rs`, and `crates/sim-core/src/inventory.rs`:
1. **World Resource Deposits & Extraction**:
   - Model harvestable raw mineral nodes (`ResourceDeposit`) with `DepositId`, `resource_id: ResourceId`, coordinates, `remaining_quantity`, and `purity` multiplier.
   - `MiningDrill` structures extract mineral batches into output hopper buffers at an extraction interval, automatically detecting exhaustion and emitting `SimEvent::DepositDepleted`.
2. **Canonical Multi-Tier Recipe Progression**:
   - **Simple Steel Chain**:
     - Iron Ore $\rightarrow$ Steel Ingot (Refinery, 30 ticks).
     - Steel Ingot $\rightarrow$ Basic Component (Fabricator, 40 ticks).
   - **Tungsten Composite Chain**:
     - Tungsten Ore $\rightarrow$ Refined Tungsten (Refinery, 40 ticks).
     - Silicates $\rightarrow$ Ceramic Plate (Refinery, 30 ticks).
     - Steel Ingot $\rightarrow$ Hardened Steel (Refinery, 35 ticks).
     - Refined Tungsten + Hardened Steel + Ceramic Plate $\rightarrow$ Tungsten Composite (Fabricator, 60 ticks).
     - Tungsten Composite supplies downstream Mk.3 defensive fortifications (Milestone 8).
3. **Two-Phase Input Reservation & Output Buffer Backpressure**:
   - Facilities lock input ingredients prior to crafting using `Inventory::reserve`. Locked items cannot be double-spent or transferred.
   - Upon completion, inputs are permanently committed (`commit_reservation`) and outputs are added to output hoppers.
   - If output buffers are full, facilities enter `OutputBlocked` without losing manufactured goods.
4. **Authoritative Power Gating & Event-Driven Cold Production**:
   - Facilities require operational electrical power (`can_operate()`). Blackouts or brownouts transition active crafts to `Unpowered`, freezing timers and resuming seamlessly on restoration.
   - Distant facilities support `advance_ticks` fast-forwarding, enabling cold regions to schedule wakeups on job completion with zero per-tick polling overhead.

## Decision 23: Logistics Jobs, Depots, Docks, Buffers, and Reservations

**Decision**: Implement the logistics routing, depots, docks, universal buffers, and transport reservation engine in `crates/sim-core/src/logistics.rs`, `crates/game-client/src/logistics_view.rs`, and `crates/sim-core/src/command.rs`:
1. **Abstract Bulk Transport Model**:
   - Explicitly avoid simulating bulk cargo as individual colliding physics items. Haulers, depots, and structures exchange discrete resource quantities mediated by atomic reservation and transfer invariants.
2. **Deterministic Route Graph & Topology Search**:
   - `RouteGraph` maintains nodes (waypoints, docks, depots) and directed edges with lane capacity and distance metrics.
   - Route pathfinding executes Dijkstra shortest path routing deterministically with zero runtime heap churn.
3. **Atomic Two-Phase Logistics Reservations**:
   - Jobs lock source cargo upon claim via `two_phase_reserve`. Claimed jobs cannot be double-claimed or over-allocated.
   - Resource transfer commits atomically upon dropoff. Aborted dropoffs roll back safely to hauler cargo buffers with zero resource duplication or loss.
4. **Dock Queuing & Berth Rate Limiting**:
   - Depots and factories expose `LogisticsDock` with limited concurrent berths and max transfer rate per tick, preventing instant unloading and simulating physical logistics flow.
5. **Powered Depot Coverage & Cold-Region Transport**:
   - Depots provide coverage (40m) only when receiving electrical power from their power subnet. Unpowered depots drop coverage to 0m, preventing new job dispatch.
   - Distant transports across cold regions simulate scheduled arrival without per-tick hot physics evaluation.

## Decision 24: Multithreaded Authoritative Dedicated Server Architecture

**Decision**: Implement a decoupled, multi-threaded authoritative dedicated server architecture in `crates/game-protocol/src/threaded.rs`, `crates/game-protocol/src/transport.rs`, and `crates/dedicated-server/src/main.rs`:
1. **Decoupled Thread Topology**:
   - **Network Ingress Thread (`server-net-rx`)**: Dedicated thread polling transport receiver non-blocking and queuing inbound packets into a thread-safe MPSC channel.
   - **Network Egress Thread (`server-net-tx`)**: Dedicated asynchronous packet and snapshot broadcaster, ensuring network transmission does not stall simulation ticks.
   - **Authoritative Simulation Loop (`server-sim-tick`)**: High-precision deterministic tick runner (30 Hz) owning simulation state, draining commands, advancing ticks, and packaging snapshots.
   - **Background Worker Pool (`server-worker-N`)**: Configurable pool of background worker threads for parallel jobs (snapshot compression, pathfinding, batch calculations).
2. **Split Transport Abstraction**:
   - Define `TransportSend: Send + 'static` and `TransportRecv: Send + 'static` traits, enabling safe splitting of `UdpTransport` (via cloned socket handles) and `LoopbackTransport` across independent thread contexts.
3. **Thread-Safe Supervision & Metrics**:
   - Expose `ServerHandle` with atomic tick tracking, concurrent telemetry counters (`ThreadedServerMetrics`), and graceful shutdown guarantees.

## Decision 25: Data-Driven Biped Robot Chassis, Server-Authoritative Escort Ownership, and the Swappable Navigation Boundary

**Decision**: Implement the biped robot framework and the Guardsman escort in `crates/sim-core/src/chassis.rs`, `crates/sim-core/src/robot.rs`, `crates/sim-core/src/navigation.rs`, `crates/game-protocol/src/server.rs`, and `crates/game-client/src/robot_view.rs`:

1. **Data-Driven Chassis Table**:
   - The chassis table lives in its own module (`chassis.rs`) separate from the simulation logic (`robot.rs`) and the navigation strategy (`navigation.rs`).
   - `RobotChassis` maps to a `&'static RobotArchetype` exactly as `WallTier` maps to `WallArchetype` and `StructureKind` maps to its static data. Mass, move speed, acceleration, turn rate, max health, armor class, sensor radius, body radius, follow standoff, arrival tolerance, separation radius, build cost, build power draw, and build time are all table data.
   - No simulation behaviour branches on the chassis: adding the Milestone 18 specialist roster is one enum variant plus one static per robot, not a new code path.
2. **One Armor Model, Not Two**:
   - `wall.rs` gains `ArmorProfile` and `calculate_damage`; `calculate_wall_damage` now delegates to it. Robot `ArmorClass` (Light/Medium/Heavy) produces an `ArmorProfile` fed into the same formula, so flat armor, penetration, and percentage resistance behave identically for walls, structures, and units.
3. **Server-Authoritative Escort Ownership**:
   - The acting player identity is derived from the session the server issued (`player_for_session`), never read from the command payload. `Command::AssignEscort`/`ReleaseEscort` carry a `player` field purely as a stated intent; a mismatch against the session identity is rejected with `PermissionDenied`.
   - An assigned escort answers only to its owner; an unassigned robot answers to any player of its own faction. A robot escorting another player can never be stolen, re-ordered, or released by anyone else.
   - Ownership lives in the registry keyed by `PlayerId`, not in region state, so it survives ticks, region transfers, and any number of simultaneous players.
   - Client-reported player positions are clamped to a physically reachable step before they can drag an escort anywhere, mirroring the movement-clamping discipline from Decision 17.
4. **Escort Cap as Configurable Progression**:
   - `RobotConfig::base_escort_cap` (1) and `max_escort_cap` (2) plus per-player `escort_cap_overrides` express the spec's "1-2 escorts" rule as data. Research and doctrine call `grant_escort_cap`, which clamps to the configured ceiling; raising the ceiling itself is a config change, not a code change.
5. **Squads as First-Class Deterministic Entities**:
   - `SquadId` (new in `game-types/src/ids.rs`), an explicit leader, and an **ordered** member roster. Roster index determines the formation slot, so `SquadRegroup` produces the same formation every run. Removing a leader promotes the next member in roster order. Regroup authorizes every member up front, so a partially applied regroup is impossible.
6. **Navigation Behind a Deliberately Small Interface**:
   - `NavigationProvider` is the entire navigation surface: goal in, desired planar velocity out. Milestone 12 ships `DirectSteering` (direct seek, arrival easing, per-tick overshoot clamping, local separation) with no global search, no path caching, and no flow fields.
   - Movement integration (acceleration clamp, chassis speed clamp, turn-rate limiting, fixed-step position update) lives in the registry, not the provider, so Milestone 16's hierarchical navigation swaps in at one call site and changes nothing else.
   - Neighbour queries rebuild a uniform grid per step instead of an O(n²) sweep; iteration order is `BTreeMap`-deterministic throughout.
7. **Presentation Holds Zero Authority**:
   - `BipedRobotRenderer` reads the authoritative registry and emits one instanced draw batch per chassis with LOD, culling, gait selection, and distance-driven walk phase. It never writes simulation state; escort markers are a read of `Robot::owner`.

## Decision 26: Anti-Cheat Provider Boundary, Internal Heuristic Layer, and EOS/EAC Adapter Seam

**Decision**: Implement the security layer as a standalone `crates/anti-cheat` crate consumed only through a trait, in `crates/anti-cheat/src/*`, `crates/game-protocol/src/server.rs`, `crates/game-protocol/src/threaded.rs`, `crates/sim-core/src/command.rs`, and `crates/dedicated-server/src/main.rs`:

1. **One-Way Dependency Edge and a Read-Only World Lens**:
   - The graph is `game-types <- sim-core <- anti-cheat <- game-protocol <- dedicated-server`. Gameplay crates never depend on `anti-cheat`, and therefore can never depend transitively on an anti-cheat SDK (`cargo tree -p sim-core --all-features` proves it).
   - `anti-cheat` reads simulation state exclusively through its own `WorldView` trait, implemented **inside the anti-cheat crate** for `TestSimState`. `sim-core` is unaware the trait exists. A provider receives `&dyn WorldView` and can only answer with a `Verdict`; it can never mutate simulation state.

2. **Security Telemetry Deliberately Outside the Simulation Journal**:
   - `SecurityEvent` is a separate typed event with severity, session, player, tick and a structured detail enum, appended to a bounded append-only `SecurityLog` — **not** a `sim_core::event::SimEvent`.
   - This is what makes "anti-cheat can be disabled without changing gameplay" a structural guarantee rather than a hope: installing, removing or changing a provider cannot perturb simulation state, event-journal length or replay hashes.

3. **Graduated Verdicts, Not Binary Accept/Reject**:
   - `Allow` / `Observe(reason)` / `Reject(reason)` / `Kick(reason)`. `Allow` and `Observe` both hand the command on to the server's own authoritative validation, so anti-cheat is strictly additive.
   - Weak evidence (`Low`/`Medium`) only records telemetry; `High` drops the command; only conclusive (`Critical`) evidence or a sustained accumulation kicks. This implements the spec's "do not instant-ban solely from weak heuristic evidence" directly in the severity table.
   - The provider never disconnects anyone: it queues `drain_pending_kicks()` and the **server** performs enforcement, keeping enforcement in one place.

4. **Deterministic Count-Based Trust State Machine**:
   - `Untrusted -> Probationary -> Trusted` driven by counted clean commands; suspicion moves a session sideways to `Flagged`; `Banned` is terminal. Thresholds are counts and scores, never wall-clock time, so transitions are reproducible from a command sequence alone.
   - Suspicion decays on clean commands, so honest players who hit a latency-induced false positive recover.

5. **Inert-by-Default Milestone Hooks Instead of Guessing**:
   - Detectors that need later systems call a `WorldView` method whose default is deliberately inert — `faction_knows_entity` returns `KnowledgeQuery::Unavailable` (M14), `weapon_cooldown_ticks` and `loaded_ammo` return `None` (M13), `InspectionContext::avatar_entity` is `None` (M12).
   - A detector that cannot answer honestly reports nothing rather than guessing, so a missing subsystem can never produce a false accusation. Each hook is doc-commented with the milestone that must override it.

6. **No Fabricated SDK Bindings**:
   - The EOS/EAC SDK is proprietary and is not licensed or vendored in this environment, so per the milestone's own condition **no FFI crate was created**. Instead `crates/anti-cheat/src/eos.rs` is a compile-safe, feature-gated (`eos-eac`, default off) adapter interface containing zero SDK calls, exercised by `cargo clippy --all-features` so it cannot bit-rot.
   - `EosEacAdapter::initialize` returns an explicit, actionable `AntiCheatError::SdkUnavailable` naming the env var, the integration steps and the `--anti-cheat basic` fallback. `player_status` returns `Unknown`, never `Clean`: an unlinked adapter never vouches for a player it could not check.

7. **Manifests as an Accident Boundary, Enforced by the Server Not the Provider**:
   - `BuildManifest` (build id + protocol version + content hash + official flag) and `ContentManifest` (order-independent `BTreeMap` rollup, FNV-1a 64). Chosen over a cryptographic digest deliberately: a client controlling its own process can report any manifest, so this is a casual-tamper and version-skew boundary, not an integrity guarantee.
   - `ServerPolicy::{LocalDev, PrivateCustom, Official}`. Official requires an exact manifest match and an unmodified official build; private servers may opt into a pinned custom manifest. The **server** validates the policy independently of the provider, so an official server with anti-cheat disabled still refuses a mismatched manifest.

8. **Admin Cheats Are Server-Permissioned in One Choke Point**:
   - `AdminRole` forms a strict containment ladder (`Player < Moderator < Host < ServerOwner`); every session starts as `Player`. `required_admin_permission(&Command)` enumerates the privileged command set in exactly one place and `AdminRegistry::authorize` runs before the command is buffered, independently of the anti-cheat provider.

## Decision 27: Data-Driven Tech Tree and the Network-Distributed Fixed-Point Modifier Store

**Decision**: Implement research as two cooperating modules, `crates/sim-core/src/research.rs` (facilities, tech graph, queue, unlocks) and `crates/sim-core/src/modifier.rs` (the faction-wide modifier network), wired into every already-existing system that an upgrade can touch:

1. **Fixed-point, order-independent modifier stacking**:
   - All modifier arithmetic is integer fixed-point in thousandths (`MODIFIER_SCALE = 1000`; `+100` == `+10%`). No floating point is ever accumulated.
   - Stacking rule: **contributions add within a `ModifierGroup`, and groups multiply across each other.** Within-group sums are plain integer addition (commutative and associative). Across groups the store builds the exact rational factor `prod(1000 + sum_g) / 1000^n` in `i128` and rounds exactly once, half-away-from-zero, at query time.
   - Because both the summation and the product are exact integer operations, the result is provably independent of the order in which technologies completed. `test_acceptance_modifier_stacking_is_order_independent` asserts bit-identical results (including `f32::to_bits`) across every rotation of the insertion order, alongside pinned exact expected values.
   - The `ModifierGroup` set is small and fixed (SoftwarePatch, Efficiency, Overclock, Doctrine, FieldAura, Emergency) and each bucket factor is clamped to `[0, 10.0x]`, which bounds the `i128` product so evaluation can never overflow or panic on malformed content.

2. **Modifiers as a replicated software patch, not a recomputed stat**:
   - `ResearchManager` owns the authoritative `ModifierStore`. `StructureRegistry` holds a replica updated through `install_modifier_patch`, matching the "software patch distributed over the faction network" fiction and keeping every structure-side system (production, power, logistics, repair) able to read modifiers without reaching across registries.
   - Nothing caches a pre-multiplied stat. Systems call `value_for` / `value_for_u32` / `duration_ticks_for` against their unchanged base archetype values, so **an upgraded turret is the same `StructureKind::Turret`, never a new class per tier**.
   - Patch distribution costs one tick of latency (research completes, the patch is installed, the next tick runs under it). This is deterministic and explicitly documented.

3. **Data-driven, versioned, validated tech tree**:
   - Technologies are `TechDef` rows in a `&'static [TechDef]` table (`STATIC_TECH_CATALOG`, `TECH_TREE_VERSION`), mirroring the existing `Recipe` catalog style. Prerequisites, costs, durations, unlocks and modifiers are all data.
   - `TechTree::load` validates at load time and returns an actionable `GameError::TechTreeInvalid` (never a panic) for null/duplicate ids, zero-duration research, zero-amount costs, self-prerequisites, dangling prerequisites, and prerequisite cycles. Cycle detection is a deterministic Kahn topological sort over `BTreeMap`/`BTreeSet`, and the error names the technologies in the cycle.
   - `TechTree::builtin()` degrades to an empty tree carrying a `load_error` string rather than panicking if the shipped catalog is ever broken.

4. **Unlocks gate by data, not by code**:
   - `UnlockTarget` covers structure kinds, recipes, robot chassis (`ChassisId`) and loadout upgrades (`ItemId`). A target is gated **because some technology mentions it**; targets nobody mentions stay ungated, which keeps all pre-research content buildable without special cases and makes new gating a pure content edit.

5. **Three independent authority gates on research progress**:
   - Authoritative **materials** must be present in the research facility hopper and are locked under two-phase reservations for the whole job (the same pattern as `production.rs`), authoritative **power** must reach the facility through the power network, and the required **ticks** must elapse. Each gate has its own negative test.
   - **Cancellation refund policy**: because inputs stay reserved and are only committed at completion, cancelling releases every outstanding reservation back to available balance — a full 100% material refund with elapsed research time forfeited. No resources are created and none are destroyed.

6. **Superseding the placeholder research command**:
   - The Milestone 1 placeholder `Command::Research { tech_id: ItemId }` (codec discriminant 6) is removed rather than kept alongside the real implementation, so there is exactly one path to queue research. Discriminant 6 is permanently retired and decodes to an actionable error pointing at `QueueResearch`.
   - New commands use the reserved Milestone 19 range: `QueueResearch` (100), `CancelResearch` (101), `ReorderResearchQueue` (102). All three are validated server-side in `AuthoritativeServer::step_tick` and `ThreadedAuthoritativeServer::apply_commands`; the client sends intent only and can never assert that a technology is researched.

## Decision 28: One Authoritative Dispatcher, Session Capability Binding, and Per-Command Entitlement

**Context**: The command path had three independent implementations
(`game-protocol::server`, `game-protocol::threaded`, `sim-core::test_harness`)
that had already diverged, all ending in `_ => {}`. `Session` carried no player
or faction, `session_id` was an unauthenticated field in the packet body, and
no command checked whether its sender was entitled to affect its target.

**Decision**:

1. **One dispatcher, exhaustive match.**
   `sim_core::dispatch::apply_command(&mut WorldState, &ActorContext, &Command)
   -> GameResult<()>` is the only implementation. Its match has **no
   catch-all**, so a new `Command` variant fails to compile until the server
   decides what to do with it. `AuthoritativeServer` and
   `ThreadedAuthoritativeServer` are thin wrappers over one shared
   `apply_buffered_commands`; the test harness drives the same function.
   Every refusal is a typed `GameError` returned to the caller and counted.

2. **Faction comes from the actor, never the payload.**
   `ActorContext { session_id, player_id, faction_id, avatar_entity,
   admin_role }` is built entirely from the session the server issued. The three
   spellings of "faction 1" (`DEFAULT_PLAYER_FACTION`,
   `DEFAULT_SESSION_FACTION`, the `session_faction` field) collapse into the
   single `DEFAULT_SESSION_FACTION` constant that seeds `Session::faction_id`.
   Ownership is threaded down into the simulation APIs that lacked it
   (`request_repair`, `set_production_recipe`, `set_extraction_target`,
   `create_job`, `claim_job`, `execute_pickup`/`execute_dropoff`, `cancel_job`,
   `transfer_entity`, and the `InventoryRegistry` transfer/reserve/commit/cancel
   operations), with `FactionId::null()` reserved for server/internal authority
   and refused outright at the network-facing entry point.

3. **Session capability binding.**
   `session_id` is a routing field and proves nothing. The server issues a
   random 64-bit token at handshake (`TokenIssuer`, deliberately **not** the
   simulation `SimRng`, so replay stays a pure function of the seed and tokens
   stay unguessable from a journal), returns it only in `ServerHello`, and
   accepts a command packet only when the token **and** the datagram source
   address match the session. The check runs before
   `validate_and_advance_sequence`, which is what closes the
   "one datagram with `sequence = u64::MAX` mutes a real player" hole.
   Mismatches are reported through M25's existing `SecurityLog` as
   `session_binding_mismatch` (`Critical`); no second security path was built.
   Loopback and in-process transports report no address, so single-player and
   local testing need no configuration.

4. **Session-layer effects travel as directives, not as a second dispatcher.**
   `AdminKickSession`, `AdminSetTrustLevel` and `AdminSetSessionRole` change
   network sessions, not world state, yet they must still be a *single*
   authorised decision. `apply_command` validates them and appends a
   `SessionDirective` to `WorldState::pending_session_directives`; each server
   drains that queue once per tick and executes it. This keeps the signature and
   the exhaustive match intact without re-introducing a per-crate copy of the
   privileged-command logic.

5. **The collision world belongs to the simulation.**
   `GreyboxTerrain`, `MovementConfig` and `validate_authoritative_movement` move
   from `game-client` into `sim_core::terrain`. `Command::Move` now clamps a
   reported position to a physically reachable step and resolves it against the
   authoritative collision world; `game-client` re-exports the same items so
   prediction and authority run identical arithmetic from one source.

6. **`TestSimState` becomes `WorldState`** in `sim-core/src/world.rs` and derives
   `PartialEq`, so determinism can be asserted against whole state rather than a
   handful of counters.

**Alternatives considered**:

- *Keep the per-crate dispatchers and add a lint.* Rejected: the divergence
  already happened silently, and a lint cannot make the compiler refuse an
  unhandled variant.
- *Put `AdminRole` itself in `ActorContext`.* Rejected: `sim-core` must not
  depend on `anti-cheat`. The already-authorized role crosses the boundary as
  `AdminRoleCode`, its stable wire code, via one `From` impl in `anti-cheat`, so
  `AdminRegistry` remains the single authorization choke point.
- *Draw session tokens from `SimRng`.* Rejected: it would make replay depend on
  network history and make live tokens recoverable from a journal.
- *Encrypt or MAC the whole datagram.* Correct eventually, but that is transport
  security and belongs with the Phase B transport work; capability binding is
  the part that had to land with the authority core.

**Consequences**:

- Construction now costs resources and enforces the 15 m reach check, because
  the builder and its position are resolved from `ActorContext` instead of being
  faked from the requested position with no inventory. Content and tests that
  built for free through the command path must now fund the builder.
- `Command::Build` is deleted (it duplicated `BuildStructure`); wire
  discriminant 3 is refused rather than reused, so a stale client fails loudly.
- `Command::Action` and `Command::RequestResource` are explicitly refused with
  `GameError::InvalidCommand` until Milestone 13 and a requisition system exist.
  Silently accepting them would be the client asserting an outcome.
- `CommandBuffer` drains FIFO sorted by `(session_id, sequence)` instead of
  `Vec::pop`, so a contested transfer is decided by the command stream rather
  than by packet jitter, and a journal replay reproduces the match.

## Future Decisions

These decisions are deferred to later milestones:

- Optional Bevy/wgpu graphical presentation adapter backend
- Physics engine integration
- Compression strategies for network and save files
- EOS/Easy Anti-Cheat SDK integration behind the Decision 26 adapter boundary (blocked on an SDK licence)



