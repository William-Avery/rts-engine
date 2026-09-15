use crate::command::{CommandBuffer, CommandEnvelope};
use crate::entity::EntityRegistry;
use crate::event::{EventJournal, SimEvent};
use crate::inventory::{ContainerKind, Inventory, InventoryRegistry};
use crate::message_queue::CrossRegionRouter;
use crate::region::{Region, RegionBounds, RegionMap, RegionState};
use crate::scheduler::MultiRateScheduler;
use crate::structure::StructureRegistry;
use game_types::{
    EntityId, FactionId, GameError, GameResult, RegionId, SimRng, SimTick, StructureId,
};

/// Simulation state for testing.
#[derive(Clone)]
pub struct TestSimState {
    pub tick: SimTick,
    pub rng: SimRng,
    pub entity_registry: EntityRegistry,
    pub region_map: RegionMap,
    pub router: CrossRegionRouter,
    pub scheduler: MultiRateScheduler,
    pub structure_registry: StructureRegistry,
    pub inventory_registry: InventoryRegistry,
    pub command_buffer: CommandBuffer,
    pub event_journal: EventJournal,
}

impl Default for TestSimState {
    fn default() -> Self {
        Self::new()
    }
}

impl TestSimState {
    /// Create a new test simulation state at tick 0.
    pub fn new() -> Self {
        TestSimState {
            tick: SimTick::zero(),
            rng: SimRng::with_default_seed(),
            entity_registry: EntityRegistry::new(),
            region_map: RegionMap::new(),
            router: CrossRegionRouter::default(),
            scheduler: MultiRateScheduler::default(),
            structure_registry: StructureRegistry::default(),
            inventory_registry: InventoryRegistry::default(),
            command_buffer: CommandBuffer::new(),
            event_journal: EventJournal::new(),
        }
    }

    /// Create a new test simulation state with a specific seed.
    pub fn with_seed(seed: u64) -> Self {
        TestSimState {
            tick: SimTick::zero(),
            rng: SimRng::new(seed),
            entity_registry: EntityRegistry::new(),
            region_map: RegionMap::new(),
            router: CrossRegionRouter::default(),
            scheduler: MultiRateScheduler::default(),
            structure_registry: StructureRegistry::default(),
            inventory_registry: InventoryRegistry::default(),
            command_buffer: CommandBuffer::new(),
            event_journal: EventJournal::new(),
        }
    }

    /// Advance simulation to a target tick directly without running systems.
    pub fn advance_to_tick(&mut self, target: SimTick) {
        self.tick = target;
    }

    /// Create a test entity in the simulation and assign to a region.
    pub fn create_entity(&mut self, faction_id: FactionId, region_id: RegionId) -> EntityId {
        let entity_id = self.entity_registry.create(faction_id, region_id);

        if !region_id.is_null() {
            // Auto-register region if not yet present in map
            if self.region_map.get_region(region_id).is_none() {
                let bounds = RegionBounds::new(0.0, 0.0, 100.0, 100.0).unwrap_or(RegionBounds {
                    min_x: 0.0,
                    min_z: 0.0,
                    max_x: 100.0,
                    max_z: 100.0,
                });
                let _ =
                    self.region_map
                        .add_region(Region::new(region_id, bounds, RegionState::Hot));
            }
            let _ = self.region_map.assign_entity(entity_id, region_id);
        }

        self.event_journal.record(
            self.tick,
            SimEvent::EntityCreated {
                entity: entity_id,
                faction_id,
                region_id,
            },
        );
        entity_id
    }

    /// Transfer an entity to a new region atomically without duplication or loss.
    pub fn transfer_entity(
        &mut self,
        entity_id: EntityId,
        destination_region: RegionId,
    ) -> GameResult<(RegionId, RegionId)> {
        let (old_region, new_region) = self
            .region_map
            .transfer_entity(entity_id, destination_region)?;
        self.entity_registry
            .update_region(entity_id, destination_region)?;
        self.event_journal.record(
            self.tick,
            SimEvent::RegionChanged {
                entity: entity_id,
                old_region,
                new_region,
            },
        );
        Ok((old_region, new_region))
    }

    /// Add a command to the buffer.
    pub fn add_command(&mut self, envelope: CommandEnvelope) {
        self.command_buffer.push(envelope);
    }

    /// Clear the command buffer (commit boundary).
    pub fn clear_commands(&mut self) {
        self.command_buffer.clear();
    }

    /// Get the current tick.
    pub fn current_tick(&self) -> SimTick {
        self.tick
    }

    /// Get the RNG state0 (for reproducibility checks).
    pub fn rng_state0(&self) -> u64 {
        self.rng.state0()
    }

    /// Get the RNG state1 (for reproducibility checks).
    pub fn rng_state1(&self) -> u64 {
        self.rng.state1()
    }

    /// Register a container for an entity.
    pub fn create_container(&mut self, entity: EntityId, kind: ContainerKind) {
        self.inventory_registry
            .register(Inventory::new(entity, kind));
    }

    /// Get reference to entity's inventory.
    pub fn inventory(&self, entity: EntityId) -> Option<&Inventory> {
        self.inventory_registry.get(entity)
    }

    /// Get mutable reference to entity's inventory.
    pub fn inventory_mut(&mut self, entity: EntityId) -> Option<&mut Inventory> {
        self.inventory_registry.get_mut(entity)
    }

    /// Atomically transfer resources between two containers with audit logging.
    pub fn transfer_resources(
        &mut self,
        from: EntityId,
        to: EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
    ) -> GameResult<()> {
        self.inventory_registry.atomic_transfer(
            from,
            to,
            resource_id,
            amount,
            self.tick,
            &mut self.event_journal,
        )
    }

    /// Atomically reserve an amount of resource under a reservation ID with audit logging.
    pub fn reserve_resources(
        &mut self,
        from: EntityId,
        reservation_id: game_types::ReservationId,
        resource_id: game_types::ResourceId,
        amount: u32,
        target_entity: Option<EntityId>,
    ) -> GameResult<()> {
        let mut req = crate::inventory::ReserveRequest::new(
            from,
            reservation_id,
            resource_id,
            amount,
            self.tick,
        );
        req.target_entity = target_entity;
        self.inventory_registry
            .two_phase_reserve(req, &mut self.event_journal)
    }

    /// Commit an active reservation and deliver items to destination with audit logging.
    pub fn commit_resource_transfer(
        &mut self,
        reservation_id: game_types::ReservationId,
        from: EntityId,
        to: EntityId,
    ) -> GameResult<()> {
        self.inventory_registry.two_phase_commit(
            reservation_id,
            from,
            to,
            self.tick,
            &mut self.event_journal,
        )
    }

    /// Cancel an active reservation and release items back to available balance with audit logging.
    pub fn cancel_resource_reservation(
        &mut self,
        reservation_id: game_types::ReservationId,
        from: EntityId,
    ) -> GameResult<()> {
        self.inventory_registry.two_phase_cancel(
            reservation_id,
            from,
            self.tick,
            &mut self.event_journal,
        )
    }

    /// Authoritatively apply damage to a world structure.
    pub fn apply_structure_damage(
        &mut self,
        id: StructureId,
        damage: crate::wall::DamageSpec,
    ) -> GameResult<crate::wall::DamageResult> {
        self.structure_registry
            .apply_damage(id, damage, self.tick, &mut self.event_journal)
    }

    /// Authoritatively repair a world structure using resources from an entity container.
    pub fn repair_structure(
        &mut self,
        id: StructureId,
        from_inventory: EntityId,
    ) -> GameResult<crate::wall::RepairResult> {
        let inv = self
            .inventory_registry
            .get_mut(from_inventory)
            .ok_or(GameError::ContainerNotFound(from_inventory))?;
        self.structure_registry
            .request_repair(id, inv, self.tick, &mut self.event_journal)
    }

    /// Authoritatively checks if a structure is powered.
    pub fn is_structure_powered(&self, id: StructureId) -> bool {
        self.structure_registry.is_structure_powered(id)
    }

    /// Authoritatively checks if a turret structure has power and can acquire/fire at targets.
    pub fn can_turret_fire(&self, id: StructureId) -> bool {
        self.structure_registry.can_turret_fire(id)
    }

    /// Authoritatively checks if an industrial fabricator has power to execute manufacturing jobs.
    pub fn can_fabricator_run(&self, id: StructureId) -> bool {
        self.structure_registry.can_fabricator_run(id)
    }

    /// Create a logistics job in the simulation state.
    pub fn create_logistics_job(
        &mut self,
        source: EntityId,
        destination: EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
        priority: crate::logistics::JobPriority,
    ) -> GameResult<game_types::LogisticsJobId> {
        self.structure_registry.logistics.create_job(
            source,
            destination,
            resource_id,
            amount,
            priority,
            self.tick,
            &mut self.event_journal,
        )
    }

    /// Atomically claim a logistics job for a worker hauler.
    pub fn claim_logistics_job(
        &mut self,
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.structure_registry.logistics.claim_job(
            job_id,
            worker_id,
            self.tick,
            &mut self.inventory_registry,
            &mut self.event_journal,
        )
    }

    /// Execute atomic material pickup for a logistics job.
    pub fn execute_logistics_pickup(
        &mut self,
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.structure_registry.logistics.execute_pickup(
            job_id,
            worker_id,
            self.tick,
            &mut self.inventory_registry,
            &mut self.event_journal,
        )
    }

    /// Execute atomic material dropoff for a logistics job.
    pub fn execute_logistics_dropoff(
        &mut self,
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.structure_registry.logistics.execute_dropoff(
            job_id,
            worker_id,
            self.tick,
            &mut self.inventory_registry,
            &mut self.event_journal,
        )
    }

    /// Clone the state for deterministic rollback testing.
    pub fn clone_state(&self) -> Self {
        self.clone()
    }
}

/// Test harness for deterministic simulation runs.
pub struct TestHarness {
    state: TestSimState,
    initial_state: TestSimState,
}

impl TestHarness {
    /// Create a new test harness with default seed.
    pub fn new() -> Self {
        let state = TestSimState::default();
        TestHarness {
            state: state.clone(),
            initial_state: state,
        }
    }

    /// Create a new test harness with a specific seed.
    pub fn with_seed(seed: u64) -> Self {
        let state = TestSimState::with_seed(seed);
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

    /// Execute a single simulation tick.
    pub fn step_tick(&mut self) {
        self.state.tick = self.state.tick.next();

        // Drain and apply queued commands
        while let Some(envelope) = self.state.command_buffer.pop() {
            match envelope.command {
                crate::command::Command::TransferRegion {
                    entity_id,
                    destination_region,
                } => {
                    let _ = self.state.transfer_entity(entity_id, destination_region);
                }
                crate::command::Command::BuildStructure {
                    kind,
                    position,
                    rotation_deg,
                } => {
                    let _ = self.state.structure_registry.request_build(
                        crate::structure::BuildRequest {
                            player_pos: position,
                            requested_pos: position,
                            kind,
                            rotation_deg,
                            faction_id: FactionId::new(1),
                            region_id: RegionId::new(1),
                            creation_tick: self.state.tick,
                            world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                        },
                        None,
                    );
                }
                crate::command::Command::DismantleStructure { structure_id } => {
                    let _ = self
                        .state
                        .structure_registry
                        .request_dismantle(structure_id, FactionId::new(1));
                }
                crate::command::Command::RepairStructure {
                    structure_id,
                    actor_entity: Some(actor),
                } => {
                    let _ = self.state.repair_structure(structure_id, actor);
                }
                crate::command::Command::TransferResource {
                    from_entity,
                    to_entity,
                    resource_id,
                    amount,
                } => {
                    let _ =
                        self.state
                            .transfer_resources(from_entity, to_entity, resource_id, amount);
                }
                crate::command::Command::ReserveResource {
                    entity,
                    resource_id,
                    amount,
                    reservation_id,
                } => {
                    let _ = self.state.reserve_resources(
                        entity,
                        reservation_id,
                        resource_id,
                        amount,
                        None,
                    );
                }
                crate::command::Command::CommitTransfer {
                    reservation_id,
                    from_entity,
                    to_entity,
                } => {
                    let _ =
                        self.state
                            .commit_resource_transfer(reservation_id, from_entity, to_entity);
                }
                crate::command::Command::CancelReservation {
                    reservation_id,
                    from_entity,
                } => {
                    let _ = self
                        .state
                        .cancel_resource_reservation(reservation_id, from_entity);
                }
                crate::command::Command::CreateLogisticsJob {
                    source,
                    destination,
                    resource_id,
                    amount,
                    priority,
                } => {
                    let _ = self.state.structure_registry.logistics.create_job(
                        source,
                        destination,
                        resource_id,
                        amount,
                        crate::logistics::JobPriority::from_u8(priority),
                        self.state.tick,
                        &mut self.state.event_journal,
                    );
                }
                crate::command::Command::CancelLogisticsJob { job_id } => {
                    let _ = self.state.structure_registry.logistics.cancel_job(
                        job_id,
                        "Command cancelled",
                        self.state.tick,
                        &mut self.state.inventory_registry,
                        &mut self.state.event_journal,
                    );
                }
                crate::command::Command::ClaimLogisticsJob { job_id, worker_id } => {
                    let _ = self.state.structure_registry.logistics.claim_job(
                        job_id,
                        worker_id,
                        self.state.tick,
                        &mut self.state.inventory_registry,
                        &mut self.state.event_journal,
                    );
                }
                crate::command::Command::ExecuteLogisticsPickup { job_id, worker_id } => {
                    let _ = self.state.structure_registry.logistics.execute_pickup(
                        job_id,
                        worker_id,
                        self.state.tick,
                        &mut self.state.inventory_registry,
                        &mut self.state.event_journal,
                    );
                }
                crate::command::Command::ExecuteLogisticsDropoff { job_id, worker_id } => {
                    let _ = self.state.structure_registry.logistics.execute_dropoff(
                        job_id,
                        worker_id,
                        self.state.tick,
                        &mut self.state.inventory_registry,
                        &mut self.state.event_journal,
                    );
                }
                _ => {}
            }
        }

        self.state
            .structure_registry
            .tick_with_journal(self.state.tick, &mut self.state.event_journal);

        // Step authoritative logistics operations
        self.state.structure_registry.logistics.step(
            self.state.tick,
            &mut self.state.inventory_registry,
            &mut self.state.event_journal,
        );

        // Run multi-rate scheduler across regions
        self.state.scheduler.tick(
            self.state.tick,
            &mut self.state.region_map,
            &mut self.state.router,
        );
    }

    /// Run the simulation until a target tick.
    pub fn run_until(&mut self, target_tick: SimTick) {
        while self.state.tick < target_tick {
            self.step_tick();
        }
    }

    /// Get the current simulation state.
    pub fn state(&self) -> &TestSimState {
        &self.state
    }

    /// Get mutable access to the simulation state.
    pub fn state_mut(&mut self) -> &mut TestSimState {
        &mut self.state
    }

    /// Get the current tick.
    pub fn current_tick(&self) -> SimTick {
        self.state.tick
    }

    /// Create an entity for testing.
    pub fn create_entity(&mut self, faction_id: FactionId, region_id: RegionId) -> EntityId {
        self.state.create_entity(faction_id, region_id)
    }

    /// Add a command to the buffer.
    pub fn add_command(&mut self, envelope: CommandEnvelope) {
        self.state.add_command(envelope);
    }

    /// Create a logistics job in the simulation state.
    pub fn create_logistics_job(
        &mut self,
        source: EntityId,
        destination: EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
        priority: crate::logistics::JobPriority,
    ) -> GameResult<game_types::LogisticsJobId> {
        self.state
            .create_logistics_job(source, destination, resource_id, amount, priority)
    }

    /// Atomically claim a logistics job for a worker hauler.
    pub fn claim_logistics_job(
        &mut self,
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.state.claim_logistics_job(job_id, worker_id)
    }

    /// Execute atomic material pickup for a logistics job.
    pub fn execute_logistics_pickup(
        &mut self,
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.state.execute_logistics_pickup(job_id, worker_id)
    }

    /// Execute atomic material dropoff for a logistics job.
    pub fn execute_logistics_dropoff(
        &mut self,
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.state.execute_logistics_dropoff(job_id, worker_id)
    }

    /// Assert that two harnesses with the same seed produce identical state.
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
        assert_eq!(
            harness1.state.entity_registry.count(),
            harness2.state.entity_registry.count(),
            "Entity registry count should be equal"
        );
        assert_eq!(
            harness1.state.region_map.total_entities(),
            harness2.state.region_map.total_entities(),
            "Region map entity counts should be equal"
        );
        assert_eq!(
            harness1.state.scheduler.metrics().total_jobs_executed(),
            harness2.state.scheduler.metrics().total_jobs_executed(),
            "Total jobs executed should be equal"
        );
        assert_eq!(
            harness1.state.scheduler.metrics().total_entities_ticked(),
            harness2.state.scheduler.metrics().total_entities_ticked(),
            "Total entities ticked should be equal"
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
    use crate::message_queue::{BackpressurePolicy, CrossRegionPayload};
    use crate::region::RegionGrid;
    use crate::scheduler::WakeupReason;
    use game_types::GameError;

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
            let res = harness.state_mut().transfer_entity(e, reg2);
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
            let res = harness.state_mut().transfer_entity(e, reg1);
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
        let err_entity = harness
            .state_mut()
            .transfer_entity(EntityId::new(9999), reg1);
        assert!(matches!(err_entity, Err(GameError::EntityNotFound(_))));

        // Error cases: non-existent region
        let err_reg = harness
            .state_mut()
            .transfer_entity(entities[0], RegionId::new(9999));
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
        let err1 =
            harness
                .state_mut()
                .transfer_resources(src_ent, dst_ent, game_types::RES_STEEL, 150);
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
        let err2 =
            harness
                .state_mut()
                .transfer_resources(src_ent, dst_ent, game_types::RES_STEEL, 180);
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
            .reserve_resources(src, res_id, game_types::RES_IRON_ORE, 50, Some(dst))
            .unwrap();

        // 2. Commit transfer
        harness
            .state_mut()
            .commit_resource_transfer(res_id, src, dst)
            .unwrap();

        // 3. Direct transfer of 30 Iron Ore
        harness
            .state_mut()
            .transfer_resources(src, dst, game_types::RES_IRON_ORE, 30)
            .unwrap();

        // 4. Reserve and Cancel
        let res_id_2 = harness.state_mut().inventory_registry.next_reservation_id();
        harness
            .state_mut()
            .reserve_resources(src, res_id_2, game_types::RES_IRON_ORE, 20, None)
            .unwrap();
        harness
            .state_mut()
            .cancel_resource_reservation(res_id_2, src)
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
            .set_extraction_target(drill_id, game_types::DepositId::new(1))
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
            .set_production_recipe(ref_id, crate::production::RECIPE_SMELT_STEEL)
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
            .set_production_recipe(ref_tungsten, crate::production::RECIPE_SMELT_TUNGSTEN)
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
            .set_production_recipe(ref_tungsten, crate::production::RECIPE_SINTER_CERAMIC)
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
            .set_production_recipe(ref_tungsten, crate::production::RECIPE_HARDEN_STEEL)
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
            .set_production_recipe(ref_id, crate::production::RECIPE_SMELT_STEEL)
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
                src_ent,
                dst_ent,
                game_types::RES_IRON_ORE,
                100,
                crate::logistics::JobPriority::High,
            )
            .unwrap();

        // Hauler 1 claims successfully
        let res1 = harness.claim_logistics_job(job_id, h1);
        assert!(
            res1.is_ok(),
            "Hauler 1 claims single worker job: {:?}",
            res1.err()
        );

        // Haulers 2 and 3 race to claim the same job -> rejected with JobAlreadyClaimed
        let res2 = harness.claim_logistics_job(job_id, h2);
        assert!(matches!(
            res2,
            Err(game_types::GameError::JobAlreadyClaimed(_))
        ));
        let res3 = harness.claim_logistics_job(job_id, h3);
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
}
