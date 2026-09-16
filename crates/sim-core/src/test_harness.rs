//! Test-only scaffolding around the authoritative [`crate::world::WorldState`].
//!
//! The production world state used to live here under the name `TestSimState`;
//! it is now `sim_core::world::WorldState`. What remains is genuinely about
//! testing: a harness that owns a world, drives whole ticks through the single
//! [`crate::dispatch::apply_command`] dispatcher, and can be reset and compared.

use crate::command::CommandEnvelope;
use crate::dispatch::{ActorContext, DEFAULT_PLAYER_REGION, apply_command};
use crate::logistics::JobPriority;
use crate::robot::player_for_session;
use crate::world::WorldState;
use game_types::{EntityId, FactionId, GameResult, LogisticsJobId, ResourceId, SimTick};

/// Faction the harness attributes commands to when a session has no lobby entry.
///
/// The harness has no session layer, so it derives both the player identity and
/// the faction from the session id exactly the way the server does, rather than
/// hardcoding `FactionId::new(1)` at each call site the way the three old
/// dispatchers did.
pub const HARNESS_FACTION: FactionId = FactionId::new(1);

/// Test harness for deterministic simulation runs.
pub struct TestHarness {
    state: WorldState,
    initial_state: WorldState,
}

impl TestHarness {
    /// Create a new test harness with default seed.
    pub fn new() -> Self {
        let state = WorldState::default();
        TestHarness {
            state: state.clone(),
            initial_state: state,
        }
    }

    /// Create a new test harness with a specific seed.
    pub fn with_seed(seed: u64) -> Self {
        let state = WorldState::with_seed(seed);
        TestHarness {
            state: state.clone(),
            initial_state: state,
        }
    }

    /// Run the simulation for N ticks.
    pub fn run_for_ticks(&mut self, num_ticks: u64) {
        let target_tick = self.state.tick + num_ticks;
        self.run_until(target_tick);
    }

    /// Resolve the actor identity of a session the same way the server does.
    fn actor_for(&mut self, envelope: &CommandEnvelope) -> ActorContext {
        let player = player_for_session(envelope.session_id);
        ActorContext::new(envelope.session_id, player, HARNESS_FACTION)
            .resolve_avatar(&mut self.state, DEFAULT_PLAYER_REGION)
    }

    /// Execute a single simulation tick.
    pub fn step_tick(&mut self) {
        self.state.tick = self.state.tick.next();

        // Drain and apply queued commands through the one dispatcher the
        // servers use, in deterministic `(session_id, sequence)` order.
        for envelope in self
            .state
            .command_buffer
            .drain_ordered()
            .collect::<Vec<_>>()
        {
            let actor = self.actor_for(&envelope);
            let _ = apply_command(&mut self.state, &actor, &envelope.command);
        }
        // The harness has no session layer, so authorized session directives
        // are simply observed and dropped.
        self.state.pending_session_directives.clear();

        self.state.step_systems();
    }

    /// Run the simulation until a target tick.
    pub fn run_until(&mut self, target_tick: SimTick) {
        while self.state.tick < target_tick {
            self.step_tick();
        }
    }

    /// Get the current simulation state.
    pub fn state(&self) -> &WorldState {
        &self.state
    }

    /// Get mutable access to the simulation state.
    pub fn state_mut(&mut self) -> &mut WorldState {
        &mut self.state
    }

    /// Get the current tick.
    pub fn current_tick(&self) -> SimTick {
        self.state.tick
    }

    /// Create an entity for testing.
    pub fn create_entity(
        &mut self,
        faction_id: FactionId,
        region_id: game_types::RegionId,
    ) -> EntityId {
        self.state.create_entity(faction_id, region_id)
    }

    /// Add a command to the buffer.
    pub fn add_command(&mut self, envelope: CommandEnvelope) {
        self.state.add_command(envelope);
    }

    /// Create a logistics job in the simulation state.
    pub fn create_logistics_job(
        &mut self,
        actor_faction: FactionId,
        source: EntityId,
        destination: EntityId,
        resource_id: ResourceId,
        amount: u32,
        priority: JobPriority,
    ) -> GameResult<LogisticsJobId> {
        self.state.create_logistics_job(
            actor_faction,
            source,
            destination,
            resource_id,
            amount,
            priority,
        )
    }

    /// Atomically claim a logistics job for a worker hauler.
    pub fn claim_logistics_job(
        &mut self,
        actor_faction: FactionId,
        job_id: LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.state
            .claim_logistics_job(actor_faction, job_id, worker_id)
    }

    /// Execute atomic material pickup for a logistics job.
    pub fn execute_logistics_pickup(
        &mut self,
        actor_faction: FactionId,
        job_id: LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.state
            .execute_logistics_pickup(actor_faction, job_id, worker_id)
    }

    /// Execute atomic material dropoff for a logistics job.
    pub fn execute_logistics_dropoff(
        &mut self,
        actor_faction: FactionId,
        job_id: LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.state
            .execute_logistics_dropoff(actor_faction, job_id, worker_id)
    }

    /// Assert that two harnesses with the same seed produce identical state.
    ///
    /// Compares the whole [`WorldState`], not a handful of counters.
    pub fn assert_reproducibility(harness1: &mut TestHarness, harness2: &mut TestHarness) {
        assert_eq!(
            harness1.state.tick, harness2.state.tick,
            "Ticks should be equal"
        );
        assert_eq!(
            harness1.state.rng_state0(),
            harness2.state.rng_state0(),
            "RNG state0 should be equal"
        );
        assert_eq!(
            harness1.state.rng_state1(),
            harness2.state.rng_state1(),
            "RNG state1 should be equal"
        );
        assert!(
            harness1.state == harness2.state,
            "Whole authoritative world state should be identical"
        );
    }

    /// Reset the harness to its initial state.
    pub fn reset(&mut self) {
        self.state = self.initial_state.clone();
    }
}

impl Default for TestHarness {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventJournal, SimEvent};
    use crate::inventory::{ContainerKind, Inventory};
    use crate::message_queue::{BackpressurePolicy, CrossRegionPayload, CrossRegionRouter};
    use crate::region::{Region, RegionBounds, RegionGrid, RegionState};
    use crate::scheduler::WakeupReason;
    use game_types::{GameError, RegionId, StructureId, TechId};

    #[test]
    fn test_basic_simulation() {
        let mut harness = TestHarness::new();
        harness.run_for_ticks(10);
        assert_eq!(harness.current_tick().value(), 10);
    }

    #[test]
    fn test_entity_creation() {
        let mut harness = TestHarness::new();
        let entity_id = harness.create_entity(FactionId::new(1), RegionId::new(0));
        assert!(!entity_id.is_null());
        assert_eq!(harness.state().entity_registry.count(), 1);
        let entity = harness.state().entity_registry.get(entity_id);
        assert!(entity.is_some());
        assert_eq!(entity.unwrap().id, entity_id);
    }

    #[test]
    fn test_reproducibility() {
        let mut harness1 = TestHarness::with_seed(42);
        let mut harness2 = TestHarness::with_seed(42);

        // Run both for the same number of ticks
        harness1.run_for_ticks(100);
        harness2.run_for_ticks(100);

        // States should be identical
        TestHarness::assert_reproducibility(&mut harness1, &mut harness2);
    }

    #[test]
    fn test_command_buffer() {
        let mut harness = TestHarness::new();
        let envelope = CommandEnvelope::new(
            game_types::SessionId::new(1),
            1,
            SimTick::zero(),
            crate::command::Command::Move {
                position: (0.0, 0.0, 0.0),
                velocity: (1.0, 0.0, 0.0),
            },
        );
        harness.add_command(envelope);
        assert_eq!(harness.state().command_buffer.len(), 1);
        harness.state_mut().clear_commands();
        assert_eq!(harness.state().command_buffer.len(), 0);
    }

    #[test]
    fn test_tick_arithmetic() {
        let tick = SimTick::new(10);
        assert_eq!((tick + 5).value(), 15);
        assert_eq!((tick - 3).value(), 7);
        assert_eq!((tick.next()).value(), 11);
    }

    #[test]
    fn test_rng_state_clone() {
        let mut harness = TestHarness::new();
        let state1 = harness.state().rng_state0();

        harness.run_for_ticks(10);
        harness.state_mut().rng.next_u64();

        let state2 = harness.state().rng_state0();
        assert_ne!(
            state1, state2,
            "RNG state should change after generating numbers"
        );
    }

    #[test]
    fn test_spatial_region_mapping() {
        let grid = RegionGrid::new(0.0, 0.0, 100.0, 100.0, 4, 4).unwrap();
        // (x=50, z=50) -> col 0, row 0 -> RegionId(1)
        assert_eq!(grid.region_at_coords(50.0, 50.0), Some(RegionId::new(1)));
        // (x=150, z=50) -> col 1, row 0 -> RegionId(2)
        assert_eq!(grid.region_at_coords(150.0, 50.0), Some(RegionId::new(2)));
        // (x=250, z=350) -> col 2, row 3 -> RegionId(3 * 4 + 2 + 1) = RegionId(15)
        assert_eq!(grid.region_at_coords(250.0, 350.0), Some(RegionId::new(15)));
        // Outside grid bounds
        assert_eq!(grid.region_at_coords(-10.0, 50.0), None);
        assert_eq!(grid.region_at_coords(450.0, 50.0), None);
    }

    #[test]
    fn test_entity_region_transfer_without_duplication_or_loss() {
        let mut harness = TestHarness::new();
        let reg1 = RegionId::new(1);
        let reg2 = RegionId::new(2);

        harness
            .state_mut()
            .region_map
            .add_region(Region::new(
                reg1,
                RegionBounds::new(0.0, 0.0, 100.0, 100.0).unwrap(),
                RegionState::Hot,
            ))
            .unwrap();

        harness
            .state_mut()
            .region_map
            .add_region(Region::new(
                reg2,
                RegionBounds::new(100.0, 0.0, 200.0, 100.0).unwrap(),
                RegionState::Cold,
            ))
            .unwrap();

        // Create 10 entities in region 1
        let mut entities = Vec::new();
        for _ in 0..10 {
            entities.push(harness.create_entity(FactionId::new(1), reg1));
        }

        // Validate initial ownership
        assert_eq!(
            harness
                .state()
                .region_map
                .get_region(reg1)
                .unwrap()
                .entity_count(),
            10
        );
        assert_eq!(
            harness
                .state()
                .region_map
                .get_region(reg2)
                .unwrap()
                .entity_count(),
            0
        );
        assert_eq!(harness.state().region_map.total_entities(), 10);

        // Transfer all 10 entities to region 2
        for &e in &entities {
            let res = harness
                .state_mut()
                .transfer_entity(FactionId::null(), e, reg2);
            assert!(res.is_ok());
            assert_eq!(res.unwrap(), (reg1, reg2));
        }

        // Validate complete transfer: no duplication, no loss
        assert_eq!(
            harness
                .state()
                .region_map
                .get_region(reg1)
                .unwrap()
                .entity_count(),
            0
        );
        assert_eq!(
            harness
                .state()
                .region_map
                .get_region(reg2)
                .unwrap()
                .entity_count(),
            10
        );
        assert_eq!(harness.state().region_map.total_entities(), 10);

        // Each entity in registry now reports region 2
        for &e in &entities {
            assert_eq!(
                harness.state().entity_registry.get(e).unwrap().region_id,
                reg2
            );
            assert_eq!(harness.state().region_map.region_for_entity(e), Some(reg2));
        }

        // Transfer 4 entities back to region 1
        for &e in &entities[0..4] {
            let res = harness
                .state_mut()
                .transfer_entity(FactionId::null(), e, reg1);
            assert!(res.is_ok());
        }

        assert_eq!(
            harness
                .state()
                .region_map
                .get_region(reg1)
                .unwrap()
                .entity_count(),
            4
        );
        assert_eq!(
            harness
                .state()
                .region_map
                .get_region(reg2)
                .unwrap()
                .entity_count(),
            6
        );
        assert_eq!(harness.state().region_map.total_entities(), 10);

        // Error cases: non-existent entity
        let err_entity =
            harness
                .state_mut()
                .transfer_entity(FactionId::null(), EntityId::new(9999), reg1);
        assert!(matches!(err_entity, Err(GameError::EntityNotFound(_))));

        // Error cases: non-existent region
        let err_reg = harness.state_mut().transfer_entity(
            FactionId::null(),
            entities[0],
            RegionId::new(9999),
        );
        assert!(matches!(err_reg, Err(GameError::RegionNotFound(_))));

        // Invariant holds: total count unaffected by failed transfers
        assert_eq!(harness.state().region_map.total_entities(), 10);
    }

    #[test]
    fn test_command_region_transfer() {
        let mut harness = TestHarness::new();
        let reg1 = RegionId::new(1);
        let reg2 = RegionId::new(2);

        harness
            .state_mut()
            .region_map
            .add_region(Region::new(
                reg1,
                RegionBounds::new(0.0, 0.0, 100.0, 100.0).unwrap(),
                RegionState::Hot,
            ))
            .unwrap();

        harness
            .state_mut()
            .region_map
            .add_region(Region::new(
                reg2,
                RegionBounds::new(100.0, 0.0, 200.0, 100.0).unwrap(),
                RegionState::Cold,
            ))
            .unwrap();

        let e = harness.create_entity(FactionId::new(1), reg1);
        assert_eq!(harness.state().region_map.region_for_entity(e), Some(reg1));

        // Submit transfer command
        harness.add_command(CommandEnvelope::new(
            game_types::SessionId::new(1),
            1,
            SimTick::zero(),
            crate::command::Command::TransferRegion {
                entity_id: e,
                destination_region: reg2,
            },
        ));

        // Run 1 tick to execute command
        harness.run_for_ticks(1);

        assert_eq!(harness.state().region_map.region_for_entity(e), Some(reg2));
        assert_eq!(
            harness.state().entity_registry.get(e).unwrap().region_id,
            reg2
        );
    }

    #[test]
    fn test_cold_regions_cost_measurably_less_than_hot() {
        let mut harness = TestHarness::new();
        let reg_hot = RegionId::new(1);
        let reg_cold = RegionId::new(2);

        harness
            .state_mut()
            .region_map
            .add_region(Region::new(
                reg_hot,
                RegionBounds::new(0.0, 0.0, 100.0, 100.0).unwrap(),
                RegionState::Hot,
            ))
            .unwrap();

        harness
            .state_mut()
            .region_map
            .add_region(Region::new(
                reg_cold,
                RegionBounds::new(100.0, 0.0, 200.0, 100.0).unwrap(),
                RegionState::Cold,
            ))
            .unwrap();

        // 100 entities in Hot region, 100 in Cold region
        for _ in 0..100 {
            harness.create_entity(FactionId::new(1), reg_hot);
            harness.create_entity(FactionId::new(1), reg_cold);
        }

        // Run 10 ticks
        harness.run_for_ticks(10);

        let metrics = harness.state().scheduler.metrics();
        // Hot region runs every tick: 10 ticks * 1 job = 10 jobs, 10 * 100 = 1000 entity ticks
        assert_eq!(metrics.jobs_executed_hot, 10);
        assert_eq!(metrics.entities_ticked_hot, 1000);

        // Cold region has no wakeups and no messages, so it costs exactly 0 jobs and 0 entity ticks!
        assert_eq!(metrics.jobs_executed_cold, 0);
        assert_eq!(metrics.entities_ticked_cold, 0);
    }

    #[test]
    fn test_cold_region_scheduled_wakeup() {
        let mut harness = TestHarness::new();
        let reg_cold = RegionId::new(2);

        harness
            .state_mut()
            .region_map
            .add_region(Region::new(
                reg_cold,
                RegionBounds::new(0.0, 0.0, 100.0, 100.0).unwrap(),
                RegionState::Cold,
            ))
            .unwrap();

        for _ in 0..50 {
            harness.create_entity(FactionId::new(1), reg_cold);
        }

        // Schedule a wakeup at tick 4
        harness
            .state_mut()
            .scheduler
            .schedule_wakeup(SimTick::new(4), reg_cold, WakeupReason::PeriodicTimer)
            .unwrap();

        // Run 10 ticks
        harness.run_for_ticks(10);

        let metrics = harness.state().scheduler.metrics();
        // Cold region should have executed exactly 1 job (on tick 4)
        assert_eq!(metrics.jobs_executed_cold, 1);
        assert_eq!(metrics.entities_ticked_cold, 50);
        assert_eq!(metrics.scheduled_wakeups_processed, 1);
    }

    #[test]
    fn test_cross_region_messaging_and_cold_wakeup() {
        let mut harness = TestHarness::new();
        let reg_hot = RegionId::new(1);
        let reg_cold = RegionId::new(2);

        harness
            .state_mut()
            .region_map
            .add_region(Region::new(
                reg_hot,
                RegionBounds::new(0.0, 0.0, 100.0, 100.0).unwrap(),
                RegionState::Hot,
            ))
            .unwrap();

        harness
            .state_mut()
            .region_map
            .add_region(Region::new(
                reg_cold,
                RegionBounds::new(100.0, 0.0, 200.0, 100.0).unwrap(),
                RegionState::Cold,
            ))
            .unwrap();

        for _ in 0..20 {
            harness.create_entity(FactionId::new(1), reg_cold);
        }

        // Route a message from hot region to cold region
        harness
            .state_mut()
            .router
            .send(
                reg_hot,
                reg_cold,
                SimTick::zero(),
                CrossRegionPayload::Signal {
                    signal_id: 42,
                    data: 100,
                },
            )
            .unwrap();

        // Run 1 tick: the pending message should wake up cold region 2 and be drained
        harness.run_for_ticks(1);

        let metrics = harness.state().scheduler.metrics();
        assert_eq!(metrics.jobs_executed_cold, 1);
        assert_eq!(metrics.entities_ticked_cold, 20);
        assert_eq!(metrics.cross_region_messages_processed, 1);

        // Next tick: with no messages and no wakeups, cold region costs 0 again!
        harness.run_for_ticks(1);
        assert_eq!(harness.state().scheduler.metrics().jobs_executed_cold, 1);
    }

    #[test]
    fn test_bounded_queue_backpressure() {
        let mut router = CrossRegionRouter::new(3, BackpressurePolicy::Reject);
        let reg_from = RegionId::new(1);
        let reg_to = RegionId::new(2);

        // Enqueue 3 messages (should succeed)
        for i in 0..3 {
            let res = router.send(
                reg_from,
                reg_to,
                SimTick::zero(),
                CrossRegionPayload::Signal {
                    signal_id: i,
                    data: 0,
                },
            );
            assert!(res.is_ok());
        }

        // 4th message should be rejected due to backpressure
        let res4 = router.send(
            reg_from,
            reg_to,
            SimTick::zero(),
            CrossRegionPayload::Signal {
                signal_id: 99,
                data: 0,
            },
        );
        assert!(matches!(res4, Err(GameError::QueueFull { .. })));

        // Test DropOldest policy
        let mut router_drop = CrossRegionRouter::new(2, BackpressurePolicy::DropOldest);
        for i in 0..4 {
            let _ = router_drop.send(
                reg_from,
                reg_to,
                SimTick::zero(),
                CrossRegionPayload::Signal {
                    signal_id: i,
                    data: 0,
                },
            );
        }

        // Capacity is 2, so 2 remain and 2 were dropped
        assert_eq!(router_drop.pending_count(reg_to), 2);
        assert_eq!(router_drop.total_dropped(), 2);
        let drained = router_drop.drain_messages_for_region(reg_to);
        assert_eq!(drained.len(), 2);
        // The ones that survived should be messages 2 and 3
        assert_eq!(
            drained[0].payload,
            CrossRegionPayload::Signal {
                signal_id: 2,
                data: 0
            }
        );
        assert_eq!(
            drained[1].payload,
            CrossRegionPayload::Signal {
                signal_id: 3,
                data: 0
            }
        );
    }

    #[test]
    fn test_large_scale_cold_entities_simulation() {
        let mut harness = TestHarness::new();
        let reg_cold = RegionId::new(10);

        harness
            .state_mut()
            .region_map
            .add_region(Region::new(
                reg_cold,
                RegionBounds::new(0.0, 0.0, 1000.0, 1000.0).unwrap(),
                RegionState::Cold,
            ))
            .unwrap();

        // Create 10,000 inert entities in the cold region
        for _ in 0..10_000 {
            harness.create_entity(FactionId::new(1), reg_cold);
        }

        assert_eq!(harness.state().entity_registry.count(), 10_000);
        assert_eq!(
            harness
                .state()
                .region_map
                .get_region(reg_cold)
                .unwrap()
                .entity_count(),
            10_000
        );

        let start = std::time::Instant::now();
        // Simulate 100 ticks
        harness.run_for_ticks(100);
        let duration = start.elapsed();

        // Cold region should cost 0 jobs and 0 entity ticks across 100 ticks
        let metrics = harness.state().scheduler.metrics();
        assert_eq!(metrics.jobs_executed_cold, 0);
        assert_eq!(metrics.entities_ticked_cold, 0);

        // Simulation must complete rapidly (well under 500ms, typically < 1ms)
        assert!(duration.as_millis() < 500, "100 ticks took {:?}", duration);

        // All 10,000 entities remain intact
        assert_eq!(harness.state().entity_registry.count(), 10_000);
        assert_eq!(harness.state().region_map.total_entities(), 10_000);
    }

    #[test]
    fn test_failed_transaction_leaves_state_unchanged() {
        let mut harness = TestHarness::new();
        let src_ent = harness.create_entity(FactionId::new(1), RegionId::new(1));
        let dst_ent = harness.create_entity(FactionId::new(1), RegionId::new(1));

        harness
            .state_mut()
            .create_container(src_ent, ContainerKind::Depot);
        harness
            .state_mut()
            .create_container(dst_ent, ContainerKind::Backpack);

        // Add 100 Steel to source depot
        harness
            .state_mut()
            .inventory_mut(src_ent)
            .unwrap()
            .add(game_types::RES_STEEL, 100)
            .unwrap();

        // 1. Attempt transfer of 150 Steel (exceeds balance of 100)
        let err1 = harness.state_mut().transfer_resources(
            FactionId::null(),
            src_ent,
            dst_ent,
            game_types::RES_STEEL,
            150,
        );
        assert!(matches!(
            err1,
            Err(game_types::GameError::InsufficientUnreservedBalance { .. })
        ));

        // Source is completely unchanged: still 100 Steel
        assert_eq!(
            harness
                .state()
                .inventory(src_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            100
        );
        // Destination is completely unchanged: 0 Steel
        assert_eq!(
            harness
                .state()
                .inventory(dst_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            0
        );

        // 2. Attempt transfer of 90 Steel into Backpack (Backpack max volume 500L; Steel is 3L each -> 90 * 3 = 270L, wait, 200 Steel = 600L > 500L)
        // Let's attempt transfer of 200 Steel, which would exceed both balance and volume
        // But if source had 300 Steel and tried to transfer 200 Steel into Backpack:
        harness
            .state_mut()
            .inventory_mut(src_ent)
            .unwrap()
            .add(game_types::RES_STEEL, 200)
            .unwrap(); // Now source has 300 Steel
        assert_eq!(
            harness
                .state()
                .inventory(src_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            300
        );

        // Attempt transfer of 180 Steel (180 * 3L = 540L > 500L max volume of Backpack)
        let err2 = harness.state_mut().transfer_resources(
            FactionId::null(),
            src_ent,
            dst_ent,
            game_types::RES_STEEL,
            180,
        );
        assert!(matches!(
            err2,
            Err(game_types::GameError::InventoryFull { .. })
        ));

        // Invariant holds: failed transaction leaves state completely unchanged!
        assert_eq!(
            harness
                .state()
                .inventory(src_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            300
        );
        assert_eq!(
            harness
                .state()
                .inventory(dst_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            0
        );
    }

    #[test]
    fn test_resource_audit_events_in_journal() {
        let mut harness = TestHarness::new();
        let src = harness.create_entity(FactionId::new(1), RegionId::new(1));
        let dst = harness.create_entity(FactionId::new(1), RegionId::new(1));

        harness
            .state_mut()
            .create_container(src, ContainerKind::Depot);
        harness
            .state_mut()
            .create_container(dst, ContainerKind::Depot);

        harness
            .state_mut()
            .inventory_mut(src)
            .unwrap()
            .add(game_types::RES_IRON_ORE, 200)
            .unwrap();

        harness.state_mut().advance_to_tick(SimTick::new(10));
        let res_id = harness.state_mut().inventory_registry.next_reservation_id();

        // 1. Reserve 50 Iron Ore
        harness
            .state_mut()
            .reserve_resources(
                FactionId::null(),
                src,
                res_id,
                game_types::RES_IRON_ORE,
                50,
                Some(dst),
            )
            .unwrap();

        // 2. Commit transfer
        harness
            .state_mut()
            .commit_resource_transfer(FactionId::null(), res_id, src, dst)
            .unwrap();

        // 3. Direct transfer of 30 Iron Ore
        harness
            .state_mut()
            .transfer_resources(FactionId::null(), src, dst, game_types::RES_IRON_ORE, 30)
            .unwrap();

        // 4. Reserve and Cancel
        let res_id_2 = harness.state_mut().inventory_registry.next_reservation_id();
        harness
            .state_mut()
            .reserve_resources(
                FactionId::null(),
                src,
                res_id_2,
                game_types::RES_IRON_ORE,
                20,
                None,
            )
            .unwrap();
        harness
            .state_mut()
            .cancel_resource_reservation(FactionId::null(), res_id_2, src)
            .unwrap();

        // Audit check in event journal
        let events = harness.state().event_journal.events_since(SimTick::new(10));
        let mut reserved_count = 0;
        let mut committed_count = 0;
        let mut transferred_count = 0;
        let mut released_count = 0;

        for (_tick, ev) in events {
            match ev {
                SimEvent::ResourceReserved { .. } => reserved_count += 1,
                SimEvent::ResourceCommitted { .. } => committed_count += 1,
                SimEvent::ResourceTransferred { .. } => transferred_count += 1,
                SimEvent::ResourceReleased { .. } => released_count += 1,
                _ => {}
            }
        }

        assert_eq!(reserved_count, 2, "Expected 2 ResourceReserved events");
        assert_eq!(committed_count, 1, "Expected 1 ResourceCommitted event");
        assert_eq!(transferred_count, 1, "Expected 1 ResourceTransferred event");
        assert_eq!(released_count, 1, "Expected 1 ResourceReleased event");
    }

    #[test]
    fn test_stepped_simulation_economy_commands() {
        let mut harness = TestHarness::new();
        let src = harness.create_entity(FactionId::new(1), RegionId::new(1));
        let dst = harness.create_entity(FactionId::new(1), RegionId::new(1));

        harness
            .state_mut()
            .create_container(src, ContainerKind::Depot);
        harness
            .state_mut()
            .create_container(dst, ContainerKind::Depot);

        harness
            .state_mut()
            .inventory_mut(src)
            .unwrap()
            .add(game_types::RES_ENERGY_CELL, 100)
            .unwrap();

        // Queue direct transfer command
        harness.add_command(CommandEnvelope::new(
            game_types::SessionId::new(1),
            1,
            SimTick::zero(),
            crate::command::Command::TransferResource {
                from_entity: src,
                to_entity: dst,
                resource_id: game_types::RES_ENERGY_CELL,
                amount: 40,
            },
        ));

        // Advance simulation 1 tick
        harness.step_tick();

        assert_eq!(
            harness
                .state()
                .inventory(src)
                .unwrap()
                .total_quantity(game_types::RES_ENERGY_CELL),
            60
        );
        assert_eq!(
            harness
                .state()
                .inventory(dst)
                .unwrap()
                .total_quantity(game_types::RES_ENERGY_CELL),
            40
        );
    }

    #[test]
    fn test_silo_bulk_raw_mineral_restriction() {
        let entity = EntityId::new(99);
        let mut silo = Inventory::new(entity, ContainerKind::Silo);
        assert_eq!(silo.max_slots, 8);
        assert_eq!(silo.max_volume_liters, 500_000);

        // Silo accepts raw minerals
        assert!(silo.can_accept(game_types::RES_IRON_ORE, 1000));
        assert!(silo.add(game_types::RES_IRON_ORE, 1000).is_ok());

        assert!(silo.can_accept(game_types::RES_STONE, 2000));
        assert!(silo.add(game_types::RES_STONE, 2000).is_ok());

        // Silo rejects refined materials and manufactured components
        assert!(!silo.can_accept(game_types::RES_STEEL, 100));
        assert!(!silo.can_accept(game_types::RES_AMMO, 500));
        assert!(!silo.can_accept(game_types::RES_BASIC_COMPONENTS, 50));
    }

    #[test]
    fn test_hopper_buffer_operations() {
        let entity = EntityId::new(100);
        let mut hopper = Inventory::new(entity, ContainerKind::Hopper);
        assert_eq!(hopper.max_slots, 4);
        assert_eq!(hopper.max_volume_liters, 10_000);

        // Fast machine buffer accepting basic components
        assert!(hopper.add(game_types::RES_BASIC_COMPONENTS, 500).is_ok());
        assert_eq!(hopper.total_quantity(game_types::RES_BASIC_COMPONENTS), 500);

        // Removing items
        assert!(hopper.remove(game_types::RES_BASIC_COMPONENTS, 200).is_ok());
        assert_eq!(hopper.total_quantity(game_types::RES_BASIC_COMPONENTS), 300);
    }

    #[test]
    fn test_stepped_wall_damage_and_authoritative_repair_command() {
        let mut harness = TestHarness::new();
        let builder = harness.create_entity(FactionId::new(1), RegionId::new(1));
        harness
            .state_mut()
            .create_container(builder, ContainerKind::Depot);

        // Build Mk.2 Steel wall via structure registry
        let wall_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (2.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Wall(crate::wall::WallTier::Mk2Steel),
                    rotation_deg: 0.0,
                    faction_id: FactionId::new(1),
                    region_id: RegionId::new(1),
                    creation_tick: SimTick::new(1),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .complete_construction(wall_id)
            .unwrap();

        // Deal 120 raw damage to Mk.2 Steel wall (20 armor, 25% res):
        // post armor = 100, effective = 75 damage. HP: 3000 - 75 = 2925.
        let dmg_res = harness
            .state_mut()
            .apply_structure_damage(wall_id, crate::wall::DamageSpec::new(120.0))
            .unwrap();
        assert_eq!(dmg_res.effective_damage, 75.0);
        assert_eq!(dmg_res.remaining_hp, 2925);

        // Add 5 Steel to builder container for repairs
        harness
            .state_mut()
            .inventory_mut(builder)
            .unwrap()
            .add(game_types::RES_STEEL, 5)
            .unwrap();

        // Queue RepairStructure command
        harness.add_command(CommandEnvelope::new(
            game_types::SessionId::new(1),
            1,
            SimTick::zero(),
            crate::command::Command::RepairStructure {
                structure_id: wall_id,
                actor_entity: Some(builder),
            },
        ));

        // Advance 1 tick
        harness.step_tick();

        // 75 missing HP repaired using 1 Steel (restores up to 100 HP, clamped at max 3000 HP)
        let structure = harness.state().structure_registry.get(wall_id).unwrap();
        match structure.state {
            crate::structure::StructureState::Constructed { current_hp, max_hp } => {
                assert_eq!(current_hp, 3000);
                assert_eq!(max_hp, 3000);
            }
            other => panic!("Expected Constructed, got {:?}", other),
        }

        // 1 Steel consumed, 4 Steel remain
        assert_eq!(
            harness
                .state()
                .inventory(builder)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            4
        );
    }

    #[test]
    fn test_power_network_relay_disconnect_depowers_downstream() {
        let mut harness = TestHarness::new();
        let faction = FactionId::new(1);
        let region = RegionId::new(1);

        // Build Generator at (0, 0, 0)
        let gen_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Generator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        // Build Pylon 1 at (10, 0, 0)
        let pylon1_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (10.0, 0.0, 0.0),
                    requested_pos: (10.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Pylon,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        // Build Pylon 2 at (25, 0, 0)
        let pylon2_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (25.0, 0.0, 0.0),
                    requested_pos: (25.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Pylon,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        // Build Turret at (30, 0, 0)
        let turret_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (30.0, 0.0, 0.0),
                    requested_pos: (30.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Turret,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        // Complete construction
        harness
            .state_mut()
            .structure_registry
            .complete_construction(gen_id)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(pylon1_id)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(pylon2_id)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(turret_id)
            .unwrap();

        // Tick simulation
        harness.step_tick();

        // Acceptance check: Turret is powered and operational to fire
        assert!(harness.state().is_structure_powered(turret_id));
        assert!(harness.state().can_turret_fire(turret_id));

        // Acceptance test: Destroying relay Pylon 1 depowers downstream infrastructure
        let dmg = crate::wall::DamageSpec {
            raw_damage: 1000.0,
            armor_penetration: 0.0,
            source: None,
        };
        let res = harness
            .state_mut()
            .apply_structure_damage(pylon1_id, dmg)
            .unwrap();
        assert!(res.destroyed);

        // Tick simulation after relay destruction
        harness.step_tick();

        // Turret must now be unpowered and disabled from firing!
        assert!(!harness.state().is_structure_powered(turret_id));
        assert!(!harness.state().can_turret_fire(turret_id));

        // Acceptance test: Rebuilding relay reconnects and restores downstream infrastructure
        let cur_tick = harness.state().tick;
        let rebuilt_pylon_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (10.0, 0.0, 0.0),
                    requested_pos: (10.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Pylon,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: cur_tick,
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(rebuilt_pylon_id)
            .unwrap();

        // Tick simulation after reconnect
        harness.step_tick();

        // Turret power is fully restored!
        assert!(harness.state().is_structure_powered(turret_id));
        assert!(harness.state().can_turret_fire(turret_id));
    }

    #[test]
    fn test_power_network_zero_redundant_graph_recomputations() {
        let mut harness = TestHarness::new();
        let faction = FactionId::new(1);
        let region = RegionId::new(1);

        // Build a grid of structures
        let gen_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Generator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        let pylon_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (5.0, 0.0, 0.0),
                    requested_pos: (5.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Pylon,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .complete_construction(gen_id)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(pylon_id)
            .unwrap();

        // Tick 1: initial topology solve
        harness.step_tick();
        assert_eq!(
            harness
                .state()
                .structure_registry
                .power_network
                .metrics
                .topology_rebuild_count,
            1
        );

        // Run for 100 stable simulation ticks
        harness.run_for_ticks(100);

        // Invariant: Topology rebuild count MUST remain exactly 1!
        assert_eq!(
            harness
                .state()
                .structure_registry
                .power_network
                .metrics
                .topology_rebuild_count,
            1,
            "Stable graph recomputed topology unnecessarily during unchanging ticks!"
        );

        // Add a new structure to dirty topology
        let cur_tick = harness.state().tick;
        let new_pylon_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (20.0, 0.0, 0.0),
                    requested_pos: (20.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Pylon,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: cur_tick,
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(new_pylon_id)
            .unwrap();

        // Advance 1 tick
        harness.step_tick();

        // Invariant: Now topology rebuild count advances by exactly 1 to 2
        assert_eq!(
            harness
                .state()
                .structure_registry
                .power_network
                .metrics
                .topology_rebuild_count,
            2
        );
    }

    #[test]
    fn test_power_network_priority_load_shedding_and_battery_buffer() {
        let mut harness = TestHarness::new();
        let faction = FactionId::new(1);
        let region = RegionId::new(1);

        // Generator produces 100 kW
        let gen_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Generator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        // 4 Turrets: High priority, 20 kW each = 80 kW demand
        let t1 = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (4.0, 0.0, 0.0),
                    requested_pos: (4.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Turret,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        let t2 = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (7.0, 0.0, 0.0),
                    requested_pos: (7.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Turret,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        let t3 = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (-4.0, 0.0, 0.0),
                    requested_pos: (-4.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Turret,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        let t4 = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (-7.0, 0.0, 0.0),
                    requested_pos: (-7.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Turret,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        // 1 Fabricator: Normal priority, 40 kW demand
        // Total demand = 80 + 40 = 120 kW > 100 kW generation
        let f1 = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 8.0),
                    requested_pos: (0.0, 0.0, 8.0),
                    kind: crate::structure::StructureKind::Fabricator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .complete_construction(gen_id)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(t1)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(t2)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(t3)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(t4)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(f1)
            .unwrap();

        harness.step_tick();

        // All High priority turrets receive full power (80 kW allocated out of 100 kW)
        assert!(harness.state().is_structure_powered(t1));
        assert!(harness.state().is_structure_powered(t2));
        assert!(harness.state().is_structure_powered(t3));
        assert!(harness.state().is_structure_powered(t4));
        assert!(harness.state().can_turret_fire(t1));

        // Normal priority fabricator receives remaining 20 kW out of 40 kW (50% brownout)
        let f1_status = harness
            .state()
            .structure_registry
            .get(f1)
            .unwrap()
            .power_status;
        match f1_status {
            crate::power::PowerStatus::Brownout { satisfaction } => {
                assert!((satisfaction - 0.5).abs() < 1e-3);
            }
            other => panic!("Expected Brownout, got {:?}", other),
        }

        // Now add Battery storage and inject stored energy
        let cur_tick = harness.state().tick;
        let bat_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, -5.0),
                    requested_pos: (0.0, 0.0, -5.0),
                    kind: crate::structure::StructureKind::Battery,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: cur_tick,
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(bat_id)
            .unwrap();

        // Inject 500 kWh of stored energy into the battery
        let bat_node = harness
            .state()
            .structure_registry
            .power_network
            .nodes()
            .get(&bat_id)
            .cloned();
        if let Some(mut modified) = bat_node {
            modified.stored_energy_kwh = 500;
            harness
                .state_mut()
                .structure_registry
                .power_network
                .register_node(modified);
        }

        harness.step_tick();

        // Battery discharges 20 kW deficit -> Total available = 120 kW -> Fabricator receives 100% power!
        assert!(harness.state().is_structure_powered(f1));
        assert!(harness.state().can_fabricator_run(f1));

        let subnets = harness.state().structure_registry.power_network.subnets();
        assert_eq!(
            subnets[0].status,
            crate::power::PowerGridStatus::BatterySupported
        );
    }

    #[test]
    fn test_power_network_multi_faction_isolation() {
        let mut harness = TestHarness::new();
        let f1 = FactionId::new(1);
        let f2 = FactionId::new(2);
        let region = RegionId::new(1);

        // Faction 1 Generator
        let gen_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Generator,
                    rotation_deg: 0.0,
                    faction_id: f1,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        // Faction 2 Turret placed 5m away from Faction 1 Generator (within 10m power range, but distinct bounds)
        let enemy_turret_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (5.0, 0.0, 0.0),
                    requested_pos: (5.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Turret,
                    rotation_deg: 0.0,
                    faction_id: f2,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .complete_construction(gen_id)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(enemy_turret_id)
            .unwrap();

        harness.step_tick();

        // Faction 2 Turret must NOT receive power from Faction 1 Generator!
        assert!(!harness.state().is_structure_powered(enemy_turret_id));
        assert!(!harness.state().can_turret_fire(enemy_turret_id));
    }

    #[test]
    fn test_deposit_extraction_and_depletion() {
        let mut harness = TestHarness::new();
        let faction = FactionId::new(1);
        let region = RegionId::new(1);

        // 1. Register a deposit with 10 units of Iron Ore and 1.5 purity
        let deposit = crate::production::ResourceDeposit::new(
            game_types::DepositId::new(1),
            game_types::RES_IRON_ORE,
            (10.0, 0.0, 10.0),
            10,
            1.5,
        );
        harness
            .state_mut()
            .structure_registry
            .register_deposit(deposit);

        // 2. Build Generator and Mining Drill
        let gen_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (10.0, 0.0, 6.0),
                    requested_pos: (10.0, 0.0, 6.0),
                    kind: crate::structure::StructureKind::Generator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        let drill_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (10.0, 0.0, 11.0),
                    requested_pos: (10.0, 0.0, 11.0),
                    kind: crate::structure::StructureKind::MiningDrill,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .complete_construction(gen_id)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(drill_id)
            .unwrap();

        // Assign deposit target to mining drill
        harness
            .state_mut()
            .structure_registry
            .set_extraction_target(FactionId::null(), drill_id, game_types::DepositId::new(1))
            .unwrap();

        // Step 15 ticks for one complete mining cycle
        for _ in 0..15 {
            harness.step_tick();
        }

        // Drill should have extracted: (2 * 1.5).round() = 3 Iron Ore
        let facility = harness
            .state()
            .structure_registry
            .get_facility(drill_id)
            .unwrap();
        assert_eq!(
            facility
                .output_inventory
                .available_quantity(game_types::RES_IRON_ORE),
            3
        );
        let deposit = harness
            .state()
            .structure_registry
            .get_deposit(game_types::DepositId::new(1))
            .unwrap();
        assert_eq!(deposit.remaining_quantity, 7);

        // Step remaining ticks until depletion (remaining 7 ore will take 3 cycles: 3 + 3 + 1)
        for _ in 0..45 {
            harness.step_tick();
        }

        let deposit_exhausted = harness
            .state()
            .structure_registry
            .get_deposit(game_types::DepositId::new(1))
            .unwrap();
        assert!(deposit_exhausted.is_depleted());
        assert_eq!(deposit_exhausted.remaining_quantity, 0);

        let final_facility = harness
            .state()
            .structure_registry
            .get_facility(drill_id)
            .unwrap();
        assert_eq!(
            final_facility
                .output_inventory
                .available_quantity(game_types::RES_IRON_ORE),
            10
        );
        assert_eq!(
            final_facility.mining_state,
            crate::production::MiningDrillState::Idle
        );
    }

    #[test]
    fn test_refinery_state_machine_and_input_reservation() {
        let mut harness = TestHarness::new();
        let faction = FactionId::new(1);
        let region = RegionId::new(1);

        let gen_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Generator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        let ref_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 7.0),
                    requested_pos: (0.0, 0.0, 7.0),
                    kind: crate::structure::StructureKind::Refinery,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .complete_construction(gen_id)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(ref_id)
            .unwrap();

        // Configure steel smelting recipe (2 Iron Ore -> 1 Steel Ingot, 30 ticks)
        harness
            .state_mut()
            .structure_registry
            .set_production_recipe(
                FactionId::null(),
                ref_id,
                crate::production::RECIPE_SMELT_STEEL,
            )
            .unwrap();

        // Add 10 Iron Ore into refinery input hopper
        let facility = harness
            .state_mut()
            .structure_registry
            .get_facility_mut(ref_id)
            .unwrap();
        facility
            .input_inventory
            .add(game_types::RES_IRON_ORE, 10)
            .unwrap();

        // Step 1 tick: craft starts, 2 iron ore reserved
        harness.step_tick();

        let fac = harness
            .state()
            .structure_registry
            .get_facility(ref_id)
            .unwrap();
        assert!(matches!(
            fac.production_state,
            crate::production::ProductionState::Crafting { .. }
        ));
        // Available balance is 8 (2 locked in reservation), total balance is still 10
        assert_eq!(
            fac.input_inventory
                .available_quantity(game_types::RES_IRON_ORE),
            8
        );
        assert_eq!(
            fac.input_inventory.total_quantity(game_types::RES_IRON_ORE),
            10
        );

        // Step remaining 29 ticks to complete craft (duration 30 ticks)
        for _ in 0..29 {
            harness.step_tick();
        }

        let finished_fac = harness
            .state()
            .structure_registry
            .get_facility(ref_id)
            .unwrap();
        // Reservation committed: total balance is 8, 1 Steel Ingot in output hopper!
        assert_eq!(
            finished_fac
                .input_inventory
                .total_quantity(game_types::RES_IRON_ORE),
            8
        );
        assert_eq!(
            finished_fac
                .output_inventory
                .available_quantity(game_types::RES_STEEL),
            1
        );
        assert_eq!(finished_fac.total_cycles_completed, 1);
    }

    #[test]
    fn test_full_tungsten_composite_production_chain() {
        let mut harness = TestHarness::new();
        let faction = FactionId::new(1);
        let region = RegionId::new(1);

        // Generator providing 100 kW power
        let gen_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Generator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(gen_id)
            .unwrap();

        // Refinery 1: Tungsten Smelting (2 Tungsten Ore -> 1 Refined Tungsten, 40 ticks)
        let ref_tungsten = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 7.0),
                    requested_pos: (0.0, 0.0, 7.0),
                    kind: crate::structure::StructureKind::Refinery,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(ref_tungsten)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .set_production_recipe(
                FactionId::null(),
                ref_tungsten,
                crate::production::RECIPE_SMELT_TUNGSTEN,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .get_facility_mut(ref_tungsten)
            .unwrap()
            .input_inventory
            .add(game_types::RES_TUNGSTEN_ORE, 2)
            .unwrap();

        // Step 40 ticks to smelt Refined Tungsten
        for _ in 0..40 {
            harness.step_tick();
        }

        let ref_tungsten_fac = harness
            .state()
            .structure_registry
            .get_facility(ref_tungsten)
            .unwrap();
        assert_eq!(
            ref_tungsten_fac
                .output_inventory
                .available_quantity(game_types::RES_REFINED_TUNGSTEN),
            1
        );

        // Refinery 2: Sinter Ceramic (2 Silicates -> 1 Ceramic Plate, 30 ticks)
        harness
            .state_mut()
            .structure_registry
            .set_production_recipe(
                FactionId::null(),
                ref_tungsten,
                crate::production::RECIPE_SINTER_CERAMIC,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .get_facility_mut(ref_tungsten)
            .unwrap()
            .input_inventory
            .add(game_types::RES_SILICATES, 2)
            .unwrap();

        for _ in 0..30 {
            harness.step_tick();
        }

        let sinter_fac = harness
            .state()
            .structure_registry
            .get_facility(ref_tungsten)
            .unwrap();
        assert_eq!(
            sinter_fac
                .output_inventory
                .available_quantity(game_types::RES_CERAMIC_PLATE),
            1
        );

        // Refinery 3: Harden Steel (2 Steel Ingot -> 1 Hardened Steel, 35 ticks)
        harness
            .state_mut()
            .structure_registry
            .set_production_recipe(
                FactionId::null(),
                ref_tungsten,
                crate::production::RECIPE_HARDEN_STEEL,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .get_facility_mut(ref_tungsten)
            .unwrap()
            .input_inventory
            .add(game_types::RES_STEEL, 2)
            .unwrap();

        for _ in 0..35 {
            harness.step_tick();
        }

        let harden_fac = harness
            .state()
            .structure_registry
            .get_facility(ref_tungsten)
            .unwrap();
        assert_eq!(
            harden_fac
                .output_inventory
                .available_quantity(game_types::RES_HARDENED_STEEL),
            1
        );

        // Now: Fabricator builds Tungsten Composite!
        // (1 Refined Tungsten + 1 Hardened Steel + 1 Ceramic Plate -> 1 Tungsten Composite, 60 ticks)
        let fab_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, -7.0),
                    requested_pos: (0.0, 0.0, -7.0),
                    kind: crate::structure::StructureKind::Fabricator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(fab_id)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .set_production_recipe(
                FactionId::null(),
                fab_id,
                crate::production::RECIPE_SYNTHESIZE_TUNGSTEN_COMPOSITE,
            )
            .unwrap();

        let fab_fac = harness
            .state_mut()
            .structure_registry
            .get_facility_mut(fab_id)
            .unwrap();
        fab_fac
            .input_inventory
            .add(game_types::RES_REFINED_TUNGSTEN, 1)
            .unwrap();
        fab_fac
            .input_inventory
            .add(game_types::RES_HARDENED_STEEL, 1)
            .unwrap();
        fab_fac
            .input_inventory
            .add(game_types::RES_CERAMIC_PLATE, 1)
            .unwrap();

        // Step 60 ticks for composite synthesis
        for _ in 0..60 {
            harness.step_tick();
        }

        let finished_fab = harness
            .state()
            .structure_registry
            .get_facility(fab_id)
            .unwrap();
        assert_eq!(
            finished_fab
                .output_inventory
                .available_quantity(game_types::RES_TUNGSTEN_COMPOSITE),
            1
        );
    }

    #[test]
    fn test_power_loss_pauses_production() {
        let mut harness = TestHarness::new();
        let faction = FactionId::new(1);
        let region = RegionId::new(1);

        let gen_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Generator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();
        let ref_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 7.0),
                    requested_pos: (0.0, 0.0, 7.0),
                    kind: crate::structure::StructureKind::Refinery,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .complete_construction(gen_id)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(ref_id)
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .set_production_recipe(
                FactionId::null(),
                ref_id,
                crate::production::RECIPE_SMELT_STEEL,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .get_facility_mut(ref_id)
            .unwrap()
            .input_inventory
            .add(game_types::RES_IRON_ORE, 2)
            .unwrap();

        // Step 10 ticks (10 / 30 progress)
        for _ in 0..10 {
            harness.step_tick();
        }

        // Destroy generator -> complete blackout
        harness
            .state_mut()
            .structure_registry
            .remove_structure(gen_id);

        // Step 1 tick: power network detects blackout, refinery enters Unpowered
        harness.step_tick();

        let unpowered_fac = harness
            .state()
            .structure_registry
            .get_facility(ref_id)
            .unwrap();
        assert!(matches!(
            unpowered_fac.production_state,
            crate::production::ProductionState::Unpowered { .. }
        ));

        // Step 50 ticks while unpowered
        for _ in 0..50 {
            harness.step_tick();
        }

        // Zero steel produced while unpowered!
        let still_unpowered = harness
            .state()
            .structure_registry
            .get_facility(ref_id)
            .unwrap();
        assert_eq!(
            still_unpowered
                .output_inventory
                .available_quantity(game_types::RES_STEEL),
            0
        );

        // Build new generator to restore power
        let new_gen = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Generator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(new_gen)
            .unwrap();

        // Step 21 ticks: power restored, remaining 20 ticks of craft execute to completion!
        for _ in 0..21 {
            harness.step_tick();
        }

        let finished_fac = harness
            .state()
            .structure_registry
            .get_facility(ref_id)
            .unwrap();
        assert_eq!(
            finished_fac
                .output_inventory
                .available_quantity(game_types::RES_STEEL),
            1
        );
    }

    #[test]
    fn test_distant_cold_region_scheduled_production() {
        let faction = FactionId::new(1);
        let region = RegionId::new(5);

        let mut facility = crate::production::ProductionFacility::new(
            StructureId::new(99),
            faction,
            region,
            crate::production::FacilityKind::Refinery,
        );
        facility
            .set_recipe(crate::production::RECIPE_SMELT_STEEL)
            .unwrap();
        facility
            .input_inventory
            .add(game_types::RES_IRON_ORE, 20)
            .unwrap();

        let mut deposits = std::collections::BTreeMap::new();
        let mut journal = EventJournal::new();

        // Fast forward 150 ticks (5 complete smelting cycles of 30 ticks each)
        facility
            .advance_ticks(
                150,
                crate::power::PowerStatus::Powered { satisfaction: 1.0 },
                SimTick::zero(),
                &mut deposits,
                &mut journal,
                crate::production::ProductionModifiers::neutral(),
            )
            .unwrap();

        // Exactly 5 Steel Ingots produced and 10 Iron Ore consumed without per-frame CPU polling
        assert_eq!(
            facility
                .output_inventory
                .available_quantity(game_types::RES_STEEL),
            5
        );
        assert_eq!(
            facility
                .input_inventory
                .total_quantity(game_types::RES_IRON_ORE),
            10
        );
        assert_eq!(facility.total_cycles_completed, 5);
    }

    #[test]
    fn test_logistics_three_haulers_single_job_exclusivity() {
        let mut harness = TestHarness::new();
        let region = RegionId::new(1);
        let faction = FactionId::new(1);

        let src_ent = harness.create_entity(faction, region);
        let dst_ent = harness.create_entity(faction, region);
        let h1 = harness.create_entity(faction, region);
        let h2 = harness.create_entity(faction, region);
        let h3 = harness.create_entity(faction, region);

        // Stock source depot with 500 Iron Ore
        let mut src_inv =
            crate::inventory::Inventory::new(src_ent, crate::inventory::ContainerKind::Depot);
        src_inv.add(game_types::RES_IRON_ORE, 500).unwrap();
        harness.state_mut().inventory_registry.register(src_inv);
        harness
            .state_mut()
            .inventory_registry
            .register(crate::inventory::Inventory::new(
                dst_ent,
                crate::inventory::ContainerKind::Depot,
            ));
        harness
            .state_mut()
            .inventory_registry
            .register(crate::inventory::Inventory::new(
                h1,
                crate::inventory::ContainerKind::CargoBuffer,
            ));
        harness
            .state_mut()
            .inventory_registry
            .register(crate::inventory::Inventory::new(
                h2,
                crate::inventory::ContainerKind::CargoBuffer,
            ));
        harness
            .state_mut()
            .inventory_registry
            .register(crate::inventory::Inventory::new(
                h3,
                crate::inventory::ContainerKind::CargoBuffer,
            ));

        // Create single-worker logistics job
        let job_id = harness
            .create_logistics_job(
                FactionId::null(),
                src_ent,
                dst_ent,
                game_types::RES_IRON_ORE,
                100,
                crate::logistics::JobPriority::High,
            )
            .unwrap();

        // Hauler 1 claims successfully
        let res1 = harness.claim_logistics_job(FactionId::null(), job_id, h1);
        assert!(
            res1.is_ok(),
            "Hauler 1 claims single worker job: {:?}",
            res1.err()
        );

        // Haulers 2 and 3 race to claim the same job -> rejected with JobAlreadyClaimed
        let res2 = harness.claim_logistics_job(FactionId::null(), job_id, h2);
        assert!(matches!(
            res2,
            Err(game_types::GameError::JobAlreadyClaimed(_))
        ));
        let res3 = harness.claim_logistics_job(FactionId::null(), job_id, h3);
        assert!(matches!(
            res3,
            Err(game_types::GameError::JobAlreadyClaimed(_))
        ));

        // Verify only hauler 1 is recorded
        let job = harness
            .state()
            .structure_registry
            .logistics
            .jobs
            .get(&job_id)
            .unwrap();
        assert_eq!(job.claimed_workers, vec![h1]);
    }

    #[test]
    fn test_logistics_zero_resource_duplication_or_loss_across_transfers() {
        let mut harness = TestHarness::new();
        let region = RegionId::new(1);
        let faction = FactionId::new(1);

        let src_ent = harness.create_entity(faction, region);
        let dst_ent = harness.create_entity(faction, region);
        let hauler_ent = harness.create_entity(faction, region);

        let mut src_inv =
            crate::inventory::Inventory::new(src_ent, crate::inventory::ContainerKind::Depot);
        src_inv.add(game_types::RES_STEEL, 300).unwrap();
        harness.state_mut().inventory_registry.register(src_inv);
        harness
            .state_mut()
            .inventory_registry
            .register(crate::inventory::Inventory::new(
                dst_ent,
                crate::inventory::ContainerKind::Depot,
            ));
        harness
            .state_mut()
            .inventory_registry
            .register(crate::inventory::Inventory::new(
                hauler_ent,
                crate::inventory::ContainerKind::CargoBuffer,
            ));

        let total_initial = harness
            .state()
            .inventory_registry
            .get(src_ent)
            .unwrap()
            .total_quantity(game_types::RES_STEEL)
            + harness
                .state()
                .inventory_registry
                .get(dst_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL)
            + harness
                .state()
                .inventory_registry
                .get(hauler_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL);
        assert_eq!(total_initial, 300);

        // Create job
        let job_id = harness
            .create_logistics_job(
                FactionId::null(),
                src_ent,
                dst_ent,
                game_types::RES_STEEL,
                150,
                crate::logistics::JobPriority::Normal,
            )
            .unwrap();

        // 1. Claim job via command
        harness.add_command(crate::command::CommandEnvelope::new(
            game_types::SessionId::new(1),
            1,
            harness.state().tick,
            crate::command::Command::ClaimLogisticsJob {
                job_id,
                worker_id: hauler_ent,
            },
        ));
        harness.step_tick();

        let total_after_claim = harness
            .state()
            .inventory_registry
            .get(src_ent)
            .unwrap()
            .total_quantity(game_types::RES_STEEL)
            + harness
                .state()
                .inventory_registry
                .get(dst_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL)
            + harness
                .state()
                .inventory_registry
                .get(hauler_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL);
        assert_eq!(total_after_claim, 300, "Zero loss or duplication on claim");

        // 2. Pickup via command
        harness.add_command(crate::command::CommandEnvelope::new(
            game_types::SessionId::new(1),
            2,
            harness.state().tick,
            crate::command::Command::ExecuteLogisticsPickup {
                job_id,
                worker_id: hauler_ent,
            },
        ));
        harness.step_tick();

        let total_in_transit = harness
            .state()
            .inventory_registry
            .get(src_ent)
            .unwrap()
            .total_quantity(game_types::RES_STEEL)
            + harness
                .state()
                .inventory_registry
                .get(dst_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL)
            + harness
                .state()
                .inventory_registry
                .get(hauler_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL);
        assert_eq!(
            total_in_transit, 300,
            "Zero loss or duplication during transit"
        );

        // 3. Dropoff via command
        harness.add_command(crate::command::CommandEnvelope::new(
            game_types::SessionId::new(1),
            3,
            harness.state().tick,
            crate::command::Command::ExecuteLogisticsDropoff {
                job_id,
                worker_id: hauler_ent,
            },
        ));
        harness.step_tick();

        let total_completed = harness
            .state()
            .inventory_registry
            .get(src_ent)
            .unwrap()
            .total_quantity(game_types::RES_STEEL)
            + harness
                .state()
                .inventory_registry
                .get(dst_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL)
            + harness
                .state()
                .inventory_registry
                .get(hauler_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL);
        assert_eq!(
            total_completed, 300,
            "Zero loss or duplication upon completion"
        );

        assert_eq!(
            harness
                .state()
                .inventory_registry
                .get(src_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            150
        );
        assert_eq!(
            harness
                .state()
                .inventory_registry
                .get(dst_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            150
        );
        assert_eq!(
            harness
                .state()
                .inventory_registry
                .get(hauler_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            0
        );
    }

    #[test]
    fn test_logistics_depot_power_coverage_loss_and_recovery() {
        let mut harness = TestHarness::new();
        let region = RegionId::new(1);
        let faction = FactionId::new(1);

        // Place Generator
        let gen_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Generator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-100.0, 100.0, -100.0, 100.0),
                },
                None,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(gen_id)
            .unwrap();

        // Place Depot nearby (10m away)
        let depot_id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (10.0, 0.0, 0.0),
                    kind: crate::structure::StructureKind::Depot,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-100.0, 100.0, -100.0, 100.0),
                },
                None,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(depot_id)
            .unwrap();

        harness.step_tick();

        let depot_ent = EntityId::new(depot_id.value());
        let coverage_powered = harness
            .state()
            .structure_registry
            .logistics
            .depots
            .get(&depot_ent)
            .unwrap()
            .effective_coverage();
        assert_eq!(
            coverage_powered, 40.0,
            "Depot has 40m coverage when powered"
        );

        // Dismantle generator -> depot loses power
        harness
            .state_mut()
            .structure_registry
            .request_dismantle(gen_id, faction)
            .unwrap();
        harness.run_for_ticks(100);

        let coverage_unpowered = harness
            .state()
            .structure_registry
            .logistics
            .depots
            .get(&depot_ent)
            .unwrap()
            .effective_coverage();
        assert_eq!(
            coverage_unpowered, 0.0,
            "Depot coverage drops to 0m when power lost"
        );
    }

    #[test]
    fn test_logistics_dock_rate_limited_service() {
        let mut harness = TestHarness::new();
        let dock_ent = EntityId::new(999);
        let mut dock = crate::logistics::LogisticsDock::new(dock_ent, 2, 20); // 20 units/tick

        let v1 = EntityId::new(101);
        let v2 = EntityId::new(102);
        let v3 = EntityId::new(103);

        dock.enqueue_vessel(v1).unwrap();
        dock.enqueue_vessel(v2).unwrap();
        dock.enqueue_vessel(v3).unwrap();

        harness
            .state_mut()
            .structure_registry
            .logistics
            .register_dock(dock);

        assert_eq!(
            harness
                .state()
                .structure_registry
                .logistics
                .docks
                .get(&dock_ent)
                .unwrap()
                .queue
                .len(),
            3
        );
    }

    #[test]
    fn test_logistics_distant_transport_cold_region_delivery() {
        let mut harness = TestHarness::new();
        let region = RegionId::new(1);
        let faction = FactionId::new(1);

        let src_ent = harness.create_entity(faction, region);
        let dst_ent = harness.create_entity(faction, region);
        let hauler_ent = harness.create_entity(faction, region);

        harness
            .state_mut()
            .inventory_registry
            .register(crate::inventory::Inventory::new(
                src_ent,
                crate::inventory::ContainerKind::Depot,
            ));
        harness
            .state_mut()
            .inventory_registry
            .register(crate::inventory::Inventory::new(
                dst_ent,
                crate::inventory::ContainerKind::Depot,
            ));
        let mut hauler_inv = crate::inventory::Inventory::new(
            hauler_ent,
            crate::inventory::ContainerKind::CargoBuffer,
        );
        hauler_inv.add(game_types::RES_STEEL, 50).unwrap();
        harness.state_mut().inventory_registry.register(hauler_inv);

        let job_id = harness
            .create_logistics_job(
                FactionId::null(),
                src_ent,
                dst_ent,
                game_types::RES_STEEL,
                50,
                crate::logistics::JobPriority::Normal,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .logistics
            .jobs
            .get_mut(&job_id)
            .unwrap()
            .status = crate::logistics::JobStatus::InTransit;
        harness
            .state_mut()
            .structure_registry
            .logistics
            .jobs
            .get_mut(&job_id)
            .unwrap()
            .claimed_workers
            .push(hauler_ent);

        harness
            .state_mut()
            .structure_registry
            .logistics
            .distant_transports
            .push(crate::logistics::DistantTransport {
                hauler_entity: hauler_ent,
                job_id,
                departure_tick: SimTick::zero(),
                arrival_tick: SimTick::new(10),
                resource_id: game_types::RES_STEEL,
                amount: 50,
            });

        // Run for 15 ticks
        harness.run_for_ticks(15);

        assert_eq!(
            harness
                .state()
                .inventory_registry
                .get(dst_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            50
        );
        assert_eq!(
            harness
                .state()
                .inventory_registry
                .get(hauler_ent)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            0
        );
    }

    #[test]
    fn test_logistics_starvation_and_deadlock_detection() {
        let mut harness = TestHarness::new();
        let region = RegionId::new(1);
        let faction = FactionId::new(1);

        let src_ent = harness.create_entity(faction, region);
        let dst_ent = harness.create_entity(faction, region);

        let job_id = harness
            .create_logistics_job(
                FactionId::null(),
                src_ent,
                dst_ent,
                game_types::RES_IRON_ORE,
                20,
                crate::logistics::JobPriority::High,
            )
            .unwrap();

        harness
            .state_mut()
            .structure_registry
            .logistics
            .jobs
            .get_mut(&job_id)
            .unwrap()
            .starvation_threshold_ticks = 30;

        // Run 50 ticks without claiming
        harness.run_for_ticks(50);

        let job = harness
            .state()
            .structure_registry
            .logistics
            .jobs
            .get(&job_id)
            .unwrap();
        assert!(job.is_starved);
        assert_eq!(
            harness
                .state()
                .structure_registry
                .logistics
                .telemetry
                .jobs_starved_count,
            1
        );
    }

    /// ACCEPTANCE: robot simulation runs headless inside the standard tick loop.
    #[test]
    fn test_headless_harness_ticks_escorted_guardsman_through_commands() {
        let mut harness = TestHarness::new();
        let faction = FactionId::new(1);
        let region = RegionId::new(1);
        let session = game_types::SessionId::new(3);
        let player = crate::robot::player_for_session(session);

        harness
            .state_mut()
            .register_player(player, faction, region, (0.0, 0.0, 0.0))
            .unwrap();
        let guardsman = harness
            .state_mut()
            .spawn_robot(
                crate::chassis::RobotChassis::Guardsman,
                faction,
                region,
                (-10.0, 0.0, 0.0),
            )
            .unwrap();

        // Escort assignment arrives as a normal command envelope on the session.
        harness.add_command(CommandEnvelope::new(
            session,
            1,
            SimTick::zero(),
            crate::command::Command::AssignEscort {
                player,
                robot_id: guardsman,
            },
        ));
        harness.run_for_ticks(1);
        assert_eq!(
            harness.state().robot_registry.escorts_for(player),
            &[guardsman]
        );

        // The player walks away; the guardsman closes to its standoff distance.
        for step in 1..=300u64 {
            harness.add_command(CommandEnvelope::new(
                session,
                step + 1,
                harness.current_tick(),
                crate::command::Command::Move {
                    position: (0.2 * step as f32, 0.0, 0.0),
                    velocity: (6.0, 0.0, 0.0),
                },
            ));
            harness.run_for_ticks(1);
        }

        let distance = harness
            .state()
            .robot_registry
            .distance_to_target(guardsman)
            .unwrap();
        let standoff = crate::chassis::RobotChassis::Guardsman
            .archetype()
            .follow_standoff;
        assert!(
            distance <= standoff + 2.0 && distance >= 1.5,
            "guardsman failed to hold station headlessly: {distance}"
        );
    }

    #[test]
    fn test_harness_rejects_robot_commands_from_a_foreign_session() {
        let mut harness = TestHarness::new();
        let faction = FactionId::new(1);
        let region = RegionId::new(1);
        let owner_session = game_types::SessionId::new(1);
        let attacker_session = game_types::SessionId::new(2);
        let owner = crate::robot::player_for_session(owner_session);
        let attacker = crate::robot::player_for_session(attacker_session);

        harness
            .state_mut()
            .register_player(owner, faction, region, (0.0, 0.0, 0.0))
            .unwrap();
        harness
            .state_mut()
            .register_player(attacker, faction, region, (20.0, 0.0, 0.0))
            .unwrap();
        let robot = harness
            .state_mut()
            .spawn_robot(
                crate::chassis::RobotChassis::Guardsman,
                faction,
                region,
                (2.0, 0.0, 0.0),
            )
            .unwrap();
        harness
            .state_mut()
            .assign_escort(owner, owner, robot)
            .unwrap();

        // The attacker's session issues orders for someone else's escort.
        harness.add_command(CommandEnvelope::new(
            attacker_session,
            1,
            SimTick::zero(),
            crate::command::Command::RobotCommand {
                robot_id: robot,
                command_type: crate::command::RobotCommandType::Move {
                    position: (900.0, 0.0, 900.0),
                },
            },
        ));
        harness.add_command(CommandEnvelope::new(
            attacker_session,
            2,
            SimTick::zero(),
            crate::command::Command::ReleaseEscort {
                player: owner,
                robot_id: robot,
            },
        ));
        harness.run_for_ticks(2);

        let state = harness.state();
        assert_eq!(state.robot_registry.get(robot).unwrap().owner, Some(owner));
        assert!(matches!(
            state.robot_registry.get(robot).unwrap().order,
            crate::robot::RobotOrder::Follow { .. }
        ));
        assert_eq!(state.robot_registry.escorts_for(owner), &[robot]);
    }

    // ---------------------------------------------------------------------
    // Milestone 19 — research, unlocks, and network-distributed modifiers
    // ---------------------------------------------------------------------

    /// Build a structure and finish its construction immediately.
    fn m19_build(
        harness: &mut TestHarness,
        kind: crate::structure::StructureKind,
        pos: (f32, f32, f32),
    ) -> StructureId {
        let id = harness
            .state_mut()
            .structure_registry
            .request_build(
                crate::structure::BuildRequest {
                    player_pos: pos,
                    requested_pos: pos,
                    kind,
                    rotation_deg: 0.0,
                    faction_id: FactionId::new(1),
                    region_id: RegionId::new(1),
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();
        harness
            .state_mut()
            .structure_registry
            .complete_construction(id)
            .unwrap();
        id
    }

    /// Grant a technology (and its prerequisites) directly, bypassing the queue.
    fn m19_grant(harness: &mut TestHarness, techs: &[TechId]) {
        let tick = harness.state().tick;
        for tech in techs {
            let mut journal = EventJournal::new();
            harness
                .state_mut()
                .research_manager
                .grant_tech(FactionId::new(1), *tech, tick, &mut journal)
                .unwrap();
        }
        // Publish the resulting patch into the structure network immediately.
        let patch = harness.state().research_manager.modifiers.clone();
        harness
            .state_mut()
            .structure_registry
            .install_modifier_patch(&patch);
    }

    /// End-to-end research through the authoritative command pipeline.
    #[test]
    fn test_stepped_research_command_pipeline_completes_tech() {
        let mut harness = TestHarness::new();
        m19_build(
            &mut harness,
            crate::structure::StructureKind::Generator,
            (0.0, 0.0, 0.0),
        );
        let lab = m19_build(
            &mut harness,
            crate::structure::StructureKind::ResearchFacility,
            (8.0, 0.0, 0.0),
        );

        // One tick to let the registry discover and power the facility.
        harness.step_tick();
        harness
            .state_mut()
            .research_manager
            .facility_mut(lab)
            .unwrap()
            .input_inventory
            .add(game_types::RES_STEEL, 20)
            .unwrap();

        // Client sends intent; the server validates and decides.
        harness.add_command(CommandEnvelope::new(
            game_types::SessionId::new(1),
            1,
            harness.current_tick(),
            crate::command::Command::QueueResearch {
                tech_id: crate::research::TECH_BASIC_METALLURGY,
            },
        ));

        harness.run_for_ticks(120);

        assert!(
            harness
                .state()
                .research_manager
                .is_completed(FactionId::new(1), crate::research::TECH_BASIC_METALLURGY)
        );
        // The completed patch was distributed into the structure network replica.
        assert_eq!(
            harness
                .state()
                .structure_registry
                .modifiers
                .multiplier_milli(
                    FactionId::new(1),
                    crate::modifier::ModifierKind::RefiningSpeed
                ),
            1100
        );
    }

    /// Research cancellation through the command pipeline refunds inputs in full.
    #[test]
    fn test_stepped_research_cancel_command_refunds_inputs() {
        let mut harness = TestHarness::new();
        m19_build(
            &mut harness,
            crate::structure::StructureKind::Generator,
            (0.0, 0.0, 0.0),
        );
        let lab = m19_build(
            &mut harness,
            crate::structure::StructureKind::ResearchFacility,
            (8.0, 0.0, 0.0),
        );
        harness.step_tick();
        harness
            .state_mut()
            .research_manager
            .facility_mut(lab)
            .unwrap()
            .input_inventory
            .add(game_types::RES_STEEL, 20)
            .unwrap();

        let job = harness
            .state_mut()
            .queue_research(FactionId::new(1), crate::research::TECH_BASIC_METALLURGY)
            .unwrap();
        harness.run_for_ticks(20);
        assert_eq!(
            harness
                .state()
                .research_manager
                .facility(lab)
                .unwrap()
                .input_inventory
                .available_quantity(game_types::RES_STEEL),
            0
        );

        harness.add_command(CommandEnvelope::new(
            game_types::SessionId::new(1),
            2,
            harness.current_tick(),
            crate::command::Command::CancelResearch { job_id: job },
        ));
        harness.step_tick();

        assert!(
            harness
                .state()
                .research_manager
                .queue(FactionId::new(1))
                .is_empty()
        );
        assert_eq!(
            harness
                .state()
                .research_manager
                .facility(lab)
                .unwrap()
                .input_inventory
                .available_quantity(game_types::RES_STEEL),
            20
        );
    }

    /// Mining yield research measurably increases ore extracted by the SAME drill
    /// archetype - no new drill class is introduced.
    #[test]
    fn test_research_modifier_boosts_mining_end_to_end() {
        fn ore_after(ticks: u64, with_research: bool) -> u32 {
            let mut harness = TestHarness::new();
            m19_build(
                &mut harness,
                crate::structure::StructureKind::Generator,
                (0.0, 0.0, 0.0),
            );
            let drill = m19_build(
                &mut harness,
                crate::structure::StructureKind::MiningDrill,
                (8.0, 0.0, 0.0),
            );
            harness.state_mut().structure_registry.register_deposit(
                crate::production::ResourceDeposit::new(
                    game_types::DepositId::new(1),
                    game_types::RES_IRON_ORE,
                    (8.0, 0.0, 0.0),
                    100_000,
                    1.0,
                ),
            );
            harness
                .state_mut()
                .structure_registry
                .set_extraction_target(FactionId::null(), drill, game_types::DepositId::new(1))
                .unwrap();

            if with_research {
                m19_grant(
                    &mut harness,
                    &[
                        crate::research::TECH_BASIC_METALLURGY,
                        crate::research::TECH_DRILL_OPTIMIZATION,
                    ],
                );
            }

            harness.run_for_ticks(ticks);
            harness
                .state()
                .structure_registry
                .get_facility(drill)
                .unwrap()
                .output_inventory
                .total_quantity(game_types::RES_IRON_ORE)
        }

        let baseline = ore_after(150, false);
        let upgraded = ore_after(150, true);
        assert!(baseline > 0, "baseline drill produced nothing");
        assert!(
            upgraded > baseline,
            "research did not increase mining output: {upgraded} vs {baseline}"
        );
    }

    /// Power research raises subnet generation and lowers subnet demand.
    #[test]
    fn test_research_modifier_boosts_power_network_end_to_end() {
        let mut harness = TestHarness::new();
        m19_build(
            &mut harness,
            crate::structure::StructureKind::Generator,
            (0.0, 0.0, 0.0),
        );
        m19_build(
            &mut harness,
            crate::structure::StructureKind::Refinery,
            (8.0, 0.0, 0.0),
        );
        harness.step_tick();

        let base_gen =
            harness.state().structure_registry.power_network.subnets()[0].total_generation_kw;
        let base_dem =
            harness.state().structure_registry.power_network.subnets()[0].total_demand_kw;

        m19_grant(&mut harness, &[crate::research::TECH_POWER_REGULATION]);
        harness.step_tick();

        let new_gen =
            harness.state().structure_registry.power_network.subnets()[0].total_generation_kw;
        let new_dem = harness.state().structure_registry.power_network.subnets()[0].total_demand_kw;

        assert!(new_gen > base_gen, "generation patch not applied");
        assert!(new_dem < base_dem, "efficiency patch not applied");
    }

    /// Logistics research raises dock throughput and depot coverage.
    #[test]
    fn test_research_modifier_boosts_logistics_end_to_end() {
        let mut harness = TestHarness::new();
        m19_build(
            &mut harness,
            crate::structure::StructureKind::Generator,
            (0.0, 0.0, 0.0),
        );
        let depot = m19_build(
            &mut harness,
            crate::structure::StructureKind::Depot,
            (8.0, 0.0, 0.0),
        );
        harness.step_tick();

        let depot_ent = EntityId::new(depot.value());
        let base_coverage = harness
            .state()
            .structure_registry
            .logistics
            .depots
            .get(&depot_ent)
            .unwrap()
            .effective_coverage();

        m19_grant(
            &mut harness,
            &[
                crate::research::TECH_POWER_REGULATION,
                crate::research::TECH_LOGISTICS_PROTOCOLS,
            ],
        );

        assert_eq!(
            harness
                .state()
                .structure_registry
                .logistics
                .throughput_multiplier_milli,
            1250
        );
        let new_coverage = harness
            .state()
            .structure_registry
            .logistics
            .depots
            .get(&depot_ent)
            .unwrap()
            .effective_coverage();
        assert!(
            new_coverage > base_coverage,
            "coverage patch not applied: {new_coverage} vs {base_coverage}"
        );
    }

    /// Repair research restores more integrity per unit of the SAME material.
    #[test]
    fn test_research_modifier_boosts_repair_rate_end_to_end() {
        fn repaired_hp(with_research: bool) -> u32 {
            let mut harness = TestHarness::new();
            let wall = m19_build(
                &mut harness,
                crate::structure::StructureKind::DEFAULT_WALL,
                (20.0, 0.0, 20.0),
            );
            let actor = harness.create_entity(FactionId::new(1), RegionId::new(1));
            harness
                .state_mut()
                .create_container(actor, ContainerKind::Backpack);
            harness
                .state_mut()
                .inventory_mut(actor)
                .unwrap()
                .add(game_types::RES_STONE, 1)
                .unwrap();

            if with_research {
                m19_grant(
                    &mut harness,
                    &[
                        crate::research::TECH_BASIC_METALLURGY,
                        crate::research::TECH_ADVANCED_ALLOYS,
                        crate::research::TECH_FIELD_REPAIR_PATCH,
                    ],
                );
            }

            let mut journal = EventJournal::new();
            harness
                .state_mut()
                .structure_registry
                .apply_damage(
                    wall,
                    crate::wall::DamageSpec {
                        raw_damage: 300.0,
                        armor_penetration: 1000.0,
                        source: None,
                    },
                    SimTick::new(1),
                    &mut journal,
                )
                .unwrap();

            harness
                .state_mut()
                .repair_structure(FactionId::null(), wall, actor)
                .unwrap()
                .hp_restored
        }

        let baseline = repaired_hp(false);
        let upgraded = repaired_hp(true);
        assert!(baseline > 0);
        assert!(
            upgraded > baseline,
            "repair patch not applied: {upgraded} vs {baseline}"
        );
    }

    /// Research is faction-scoped: one faction's patches never leak to another.
    #[test]
    fn test_research_modifiers_are_faction_scoped() {
        let mut harness = TestHarness::new();
        m19_grant(&mut harness, &[crate::research::TECH_POWER_REGULATION]);
        let modifiers = &harness.state().structure_registry.modifiers;
        assert_eq!(
            modifiers.multiplier_milli(
                FactionId::new(1),
                crate::modifier::ModifierKind::PowerGeneration
            ),
            1100
        );
        assert_eq!(
            modifiers.multiplier_milli(
                FactionId::new(2),
                crate::modifier::ModifierKind::PowerGeneration
            ),
            1000
        );
    }

    /// Two independently-seeded harnesses running the identical research script
    /// produce bit-identical modifier state.
    #[test]
    fn test_research_is_deterministic_across_identical_runs() {
        fn run_script() -> Vec<(crate::modifier::ModifierKind, i64)> {
            let mut harness = TestHarness::with_seed(99);
            m19_build(
                &mut harness,
                crate::structure::StructureKind::Generator,
                (0.0, 0.0, 0.0),
            );
            let lab = m19_build(
                &mut harness,
                crate::structure::StructureKind::ResearchFacility,
                (8.0, 0.0, 0.0),
            );
            harness.step_tick();
            let facility = harness
                .state_mut()
                .research_manager
                .facility_mut(lab)
                .unwrap();
            facility
                .input_inventory
                .add(game_types::RES_STEEL, 20)
                .unwrap();
            facility
                .input_inventory
                .add(game_types::RES_ENERGY_CELL, 5)
                .unwrap();
            let _ = harness
                .state_mut()
                .queue_research(FactionId::new(1), crate::research::TECH_BASIC_METALLURGY);
            let _ = harness
                .state_mut()
                .queue_research(FactionId::new(1), crate::research::TECH_POWER_REGULATION);
            harness.run_for_ticks(400);
            harness
                .state()
                .research_manager
                .modifiers
                .active_kinds(FactionId::new(1))
        }

        let a = run_script();
        let b = run_script();
        assert_eq!(a, b);
        assert!(!a.is_empty());
    }
}
