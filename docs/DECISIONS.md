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

## Future Decisions

These decisions are deferred to later milestones:

- Optional Bevy/wgpu graphical presentation adapter backend
- Physics engine integration
- Compression strategies for network and save files
- Anti-cheat provider interface implementation



