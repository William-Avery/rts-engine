//! The authoritative world state.
//!
//! This is the production simulation state the dedicated server, the threaded
//! server and the single-threaded `AuthoritativeServer` all own. It was
//! previously called `TestSimState` and lived in `test_harness`; the name was a
//! lie, and the test-only helpers that remain behind in `test_harness` are the
//! only part of that module that is genuinely about testing.
//!
//! Every method here that a client command can reach takes an `actor_faction`.
//! [`FactionId::null()`] denotes **server/internal authority** (tick loops,
//! world bootstrap, tests) and skips the ownership check;
//! [`crate::dispatch::apply_command`] refuses a null actor faction outright, so
//! that escape hatch is not reachable from the network.

use crate::chassis::RobotChassis;
use crate::command::{CommandBuffer, CommandEnvelope};
use crate::entity::EntityRegistry;
use crate::event::{EventJournal, SimEvent};
use crate::inventory::{ContainerKind, Inventory, InventoryRegistry};
use crate::message_queue::CrossRegionRouter;
use crate::region::{Region, RegionBounds, RegionMap, RegionState};
use crate::research::ResearchManager;
use crate::robot::{PlayerPresence, RobotOrder, RobotRegistry};
use crate::scheduler::MultiRateScheduler;
use crate::structure::StructureRegistry;
use crate::terrain::{GreyboxTerrain, MovementConfig};
use game_types::{
    EntityId, FactionId, GameError, GameResult, PlayerId, RegionId, ResearchJobId, SessionId,
    SimRng, SimTick, SquadId, StructureId,
};

/// An action the simulation authorized that only the session layer can carry
/// out, because it touches network sessions rather than world state.
///
/// [`crate::dispatch::apply_command`] appends these; the server drains
/// [`WorldState::pending_session_directives`] once per tick and executes them.
/// Keeping them here is what lets the dispatcher stay a single exhaustive
/// match with no catch-all and no second copy in the transport crates.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SessionDirective {
    /// Remove a session from the match.
    KickSession { target: SessionId, reason_code: u8 },
    /// Override a session anti-cheat trust level.
    SetTrustLevel { target: SessionId, trust_code: u8 },
    /// Assign an admin role to a session.
    SetSessionRole { target: SessionId, role_code: u8 },
}

/// The authoritative simulation state.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldState {
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
    pub robot_registry: RobotRegistry,
    pub research_manager: ResearchManager,
    /// Authoritative collision world. The server clamps every accepted player
    /// position against this; the client uses the same structure to predict.
    pub terrain: GreyboxTerrain,
    /// Authoritative avatar movement limits used to validate `Command::Move`.
    pub movement_config: MovementConfig,
    /// Session-layer actions the simulation authorized this tick.
    pub pending_session_directives: Vec<SessionDirective>,
}

impl Default for WorldState {
    fn default() -> Self {
        Self::new()
    }
}

impl WorldState {
    /// Create a new authoritative world state at tick 0.
    pub fn new() -> Self {
        WorldState {
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
            robot_registry: RobotRegistry::new(),
            research_manager: ResearchManager::new(),
            terrain: GreyboxTerrain::default(),
            movement_config: MovementConfig::default(),
            pending_session_directives: Vec::new(),
        }
    }

    /// Create a new authoritative world state with a specific seed.
    pub fn with_seed(seed: u64) -> Self {
        WorldState {
            rng: SimRng::new(seed),
            ..WorldState::new()
        }
    }

    /// Authoritative `(min_x, max_x, min_z, max_z)` play area.
    pub fn world_bounds_xz(&self) -> (f32, f32, f32, f32) {
        self.terrain.bounds_xz()
    }

    /// Advance simulation to a target tick directly without running systems.
    pub fn advance_to_tick(&mut self, target: SimTick) {
        self.tick = target;
    }

    /// Owning faction of an entity, or `None` when the entity does not exist.
    pub fn entity_faction(&self, entity: EntityId) -> Option<FactionId> {
        self.entity_registry.get(entity).map(|e| e.faction_id)
    }

    /// Authorize an actor faction to act on an entity.
    ///
    /// An unknown entity passes: the downstream system produces its own,
    /// more specific error. An entity owned by `FactionId::null()` is neutral
    /// world property and is open to everybody.
    pub fn authorize_entity(&self, actor_faction: FactionId, entity: EntityId) -> GameResult<()> {
        if actor_faction.is_null() {
            return Ok(());
        }
        match self.entity_faction(entity) {
            None => Ok(()),
            Some(owner) if owner.is_null() || owner == actor_faction => Ok(()),
            Some(_) => Err(GameError::PermissionDenied),
        }
    }

    /// Authorize an actor faction to act on a structure.
    pub fn authorize_structure(
        &self,
        actor_faction: FactionId,
        structure: StructureId,
    ) -> GameResult<()> {
        if actor_faction.is_null() {
            return Ok(());
        }
        match self.structure_registry.get(structure).map(|s| s.faction_id) {
            None => Ok(()),
            Some(owner) if owner.is_null() || owner == actor_faction => Ok(()),
            Some(_) => Err(GameError::PermissionDenied),
        }
    }

    /// Create an entity in the simulation and assign it to a region.
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
    ///
    /// `actor_faction` must own the entity; a session cannot relocate another
    /// faction's units.
    pub fn transfer_entity(
        &mut self,
        actor_faction: FactionId,
        entity_id: EntityId,
        destination_region: RegionId,
    ) -> GameResult<(RegionId, RegionId)> {
        self.authorize_entity(actor_faction, entity_id)?;
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

    /// Register a container for an entity, stamped with that entity's faction.
    pub fn create_container(&mut self, entity: EntityId, kind: ContainerKind) {
        let faction = self.entity_faction(entity).unwrap_or_else(FactionId::null);
        self.inventory_registry
            .register(Inventory::new(entity, kind).with_faction(faction));
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
        actor_faction: FactionId,
        from: EntityId,
        to: EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
    ) -> GameResult<()> {
        self.authorize_entity(actor_faction, from)?;
        self.authorize_entity(actor_faction, to)?;
        self.inventory_registry.atomic_transfer(
            crate::inventory::TransferRequest {
                actor_faction,
                from,
                to,
                resource_id,
                amount,
                tick: self.tick,
            },
            &mut self.event_journal,
        )
    }

    /// Atomically reserve an amount of resource under a reservation ID with audit logging.
    pub fn reserve_resources(
        &mut self,
        actor_faction: FactionId,
        from: EntityId,
        reservation_id: game_types::ReservationId,
        resource_id: game_types::ResourceId,
        amount: u32,
        target_entity: Option<EntityId>,
    ) -> GameResult<()> {
        self.authorize_entity(actor_faction, from)?;
        let mut req = crate::inventory::ReserveRequest::new(
            from,
            reservation_id,
            resource_id,
            amount,
            self.tick,
        );
        req.target_entity = target_entity;
        self.inventory_registry
            .two_phase_reserve(actor_faction, req, &mut self.event_journal)
    }

    /// Commit an active reservation and deliver items to destination with audit logging.
    pub fn commit_resource_transfer(
        &mut self,
        actor_faction: FactionId,
        reservation_id: game_types::ReservationId,
        from: EntityId,
        to: EntityId,
    ) -> GameResult<()> {
        self.authorize_entity(actor_faction, from)?;
        self.authorize_entity(actor_faction, to)?;
        self.inventory_registry.two_phase_commit(
            actor_faction,
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
        actor_faction: FactionId,
        reservation_id: game_types::ReservationId,
        from: EntityId,
    ) -> GameResult<()> {
        self.authorize_entity(actor_faction, from)?;
        self.inventory_registry.two_phase_cancel(
            actor_faction,
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
        actor_faction: FactionId,
        id: StructureId,
        from_inventory: EntityId,
    ) -> GameResult<crate::wall::RepairResult> {
        self.authorize_structure(actor_faction, id)?;
        self.authorize_entity(actor_faction, from_inventory)?;
        let WorldState {
            inventory_registry,
            structure_registry,
            event_journal,
            tick,
            ..
        } = self;
        let inv = inventory_registry
            .get_mut(from_inventory)
            .ok_or(GameError::ContainerNotFound(from_inventory))?;
        structure_registry.request_repair(actor_faction, id, inv, *tick, event_journal)
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
        actor_faction: FactionId,
        source: EntityId,
        destination: EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
        priority: crate::logistics::JobPriority,
    ) -> GameResult<game_types::LogisticsJobId> {
        // A job debits its source and credits its destination, so the actor
        // must own both ends. This is the gate that closes the depot-theft
        // chain (create -> claim -> pickup -> dropoff) against enemy storage.
        self.authorize_entity(actor_faction, source)?;
        self.authorize_entity(actor_faction, destination)?;
        let WorldState {
            structure_registry,
            inventory_registry,
            event_journal,
            tick,
            ..
        } = self;
        structure_registry.logistics.create_job(
            crate::logistics::JobRequest {
                actor_faction,
                source,
                destination,
                resource_id,
                amount,
                priority,
                tick: *tick,
            },
            inventory_registry,
            event_journal,
        )
    }

    /// Atomically claim a logistics job for a worker hauler.
    pub fn claim_logistics_job(
        &mut self,
        actor_faction: FactionId,
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.authorize_entity(actor_faction, worker_id)?;
        let WorldState {
            structure_registry,
            inventory_registry,
            event_journal,
            tick,
            ..
        } = self;
        structure_registry.logistics.claim_job(
            actor_faction,
            job_id,
            worker_id,
            *tick,
            inventory_registry,
            event_journal,
        )
    }

    /// Execute atomic material pickup for a logistics job.
    pub fn execute_logistics_pickup(
        &mut self,
        actor_faction: FactionId,
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.authorize_entity(actor_faction, worker_id)?;
        let WorldState {
            structure_registry,
            inventory_registry,
            event_journal,
            tick,
            ..
        } = self;
        structure_registry.logistics.execute_pickup(
            actor_faction,
            job_id,
            worker_id,
            *tick,
            inventory_registry,
            event_journal,
        )
    }

    /// Execute atomic material dropoff for a logistics job.
    pub fn execute_logistics_dropoff(
        &mut self,
        actor_faction: FactionId,
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
    ) -> GameResult<()> {
        self.authorize_entity(actor_faction, worker_id)?;
        let WorldState {
            structure_registry,
            inventory_registry,
            event_journal,
            tick,
            ..
        } = self;
        structure_registry.logistics.execute_dropoff(
            actor_faction,
            job_id,
            worker_id,
            *tick,
            inventory_registry,
            event_journal,
        )
    }

    /// Create an entity and register it as an authoritative biped robot.
    pub fn spawn_robot(
        &mut self,
        chassis: RobotChassis,
        faction_id: FactionId,
        region_id: RegionId,
        position: (f32, f32, f32),
    ) -> GameResult<EntityId> {
        let entity = self.create_entity(faction_id, region_id);
        self.robot_registry.spawn_robot_with_journal(
            crate::robot::RobotSpawnRequest::new(
                entity, chassis, faction_id, region_id, position, self.tick,
            ),
            &mut self.event_journal,
        )?;
        Ok(entity)
    }

    /// Create an entity and register it as an authoritative player presence.
    pub fn register_player(
        &mut self,
        player: PlayerId,
        faction_id: FactionId,
        region_id: RegionId,
        position: (f32, f32, f32),
    ) -> GameResult<EntityId> {
        let entity = self.create_entity(faction_id, region_id);
        self.robot_registry.register_player(PlayerPresence::new(
            player, entity, faction_id, region_id, position,
        ))?;
        Ok(entity)
    }

    /// Resolve the authoritative avatar entity of a player, creating the avatar
    /// and its presence record on first use.
    ///
    /// The identity is the one the server itself issued at handshake; it is
    /// never read from a command payload, so a client cannot act as another
    /// player or another faction.
    pub fn ensure_player_avatar(
        &mut self,
        player: PlayerId,
        faction_id: FactionId,
        region_id: RegionId,
    ) -> EntityId {
        if player.is_null() {
            return EntityId::null();
        }
        if let Some(presence) = self.robot_registry.player(player) {
            return presence.entity;
        }
        let entity = self.create_entity(faction_id, region_id);
        let _ = self.robot_registry.register_player(PlayerPresence::new(
            player,
            entity,
            faction_id,
            region_id,
            (0.0, 0.0, 0.0),
        ));
        entity
    }

    /// Assign a robot as a player's escort under server authority.
    pub fn assign_escort(
        &mut self,
        actor: PlayerId,
        owner: PlayerId,
        robot: EntityId,
    ) -> GameResult<()> {
        self.robot_registry
            .assign_escort(actor, owner, robot, self.tick, &mut self.event_journal)
    }

    /// Release one of a player's escorts under server authority.
    pub fn release_escort(
        &mut self,
        actor: PlayerId,
        owner: PlayerId,
        robot: EntityId,
    ) -> GameResult<()> {
        self.robot_registry
            .release_escort(actor, owner, robot, self.tick, &mut self.event_journal)
    }

    /// Validate and apply a standing order to a robot.
    pub fn issue_robot_order(
        &mut self,
        actor: PlayerId,
        robot: EntityId,
        order: RobotOrder,
    ) -> GameResult<()> {
        self.robot_registry
            .issue_order(actor, robot, order, self.tick, &mut self.event_journal)
    }

    /// Create a squad for a faction.
    pub fn create_squad(&mut self, faction_id: FactionId) -> SquadId {
        self.robot_registry
            .create_squad(faction_id, self.tick, &mut self.event_journal)
    }

    /// Authoritatively append a technology to a faction's research queue.
    pub fn queue_research(
        &mut self,
        faction_id: FactionId,
        tech_id: game_types::TechId,
    ) -> GameResult<ResearchJobId> {
        self.research_manager.queue_research(
            faction_id,
            tech_id,
            self.tick,
            &mut self.event_journal,
        )
    }

    /// Cancel a research job, refunding its reserved inputs in full.
    pub fn cancel_research(
        &mut self,
        faction_id: FactionId,
        job_id: ResearchJobId,
    ) -> GameResult<u32> {
        self.research_manager.cancel_research(
            faction_id,
            job_id,
            self.tick,
            &mut self.event_journal,
        )
    }

    /// Reorder a research job within a faction's queue.
    pub fn reorder_research(
        &mut self,
        faction_id: FactionId,
        job_id: ResearchJobId,
        new_index: usize,
    ) -> GameResult<()> {
        self.research_manager
            .reorder_queue(faction_id, job_id, new_index)
    }

    /// Advance the authoritative research subsystem by one tick.
    pub fn step_research(&mut self) {
        self.research_manager.tick(
            self.tick,
            &mut self.structure_registry,
            &mut self.event_journal,
        );
    }

    /// Drain the session-layer directives the simulation authorized.
    pub fn drain_session_directives(&mut self) -> Vec<SessionDirective> {
        std::mem::take(&mut self.pending_session_directives)
    }

    /// Advance every authoritative subsystem by one tick.
    ///
    /// Callers are expected to have advanced `tick` and applied commands first.
    /// Shared by the harness, `AuthoritativeServer` and the threaded server so
    /// the three cannot drift apart.
    pub fn step_systems(&mut self) {
        self.structure_registry
            .tick_with_journal(self.tick, &mut self.event_journal);

        // Advance authoritative research queues and distribute modifier patches
        self.step_research();

        // Step authoritative logistics operations
        self.structure_registry.logistics.step(
            self.tick,
            &mut self.inventory_registry,
            &mut self.event_journal,
        );

        // Step authoritative robot movement, navigation, and escort behaviour
        self.robot_registry.step(self.tick, &mut self.event_journal);

        // Run multi-rate scheduler across regions
        self.scheduler
            .tick(self.tick, &mut self.region_map, &mut self.router);
    }

    /// Clone the state for deterministic rollback testing.
    pub fn clone_state(&self) -> Self {
        self.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{Command, CommandEnvelope};
    use crate::dispatch::{ActorContext, DEFAULT_PLAYER_REGION, apply_command};
    use crate::inventory::ContainerKind;
    use game_types::{RES_STEEL, SessionId};

    /// A7 — the production world state is comparable as a whole.
    ///
    /// The old `TestSimState` could only be compared through a handful of
    /// counters, which is why the workspace's only determinism test compared two
    /// empty simulations. `WorldState: PartialEq` makes a real equality
    /// assertion possible, and it must notice a divergence in any subsystem.
    #[test]
    fn test_a7_world_state_supports_whole_state_equality() {
        let mut a = WorldState::with_seed(42);
        let mut b = WorldState::with_seed(42);
        assert!(a == b, "two identically seeded worlds must start equal");

        // Identical scripted setup keeps them equal.
        for world in [&mut a, &mut b] {
            let depot = world.create_entity(FactionId::new(1), DEFAULT_PLAYER_REGION);
            world.create_container(depot, ContainerKind::Depot);
            let _ = world.inventory_mut(depot).unwrap().add(RES_STEEL, 100);
            world.tick = world.tick.next();
            world.step_systems();
        }
        assert!(a == b, "identical command streams diverged");

        // A divergence in any one subsystem is visible. Each case starts from a
        // fresh identical pair so the checks stay independent.
        let diverge: [fn(&mut WorldState); 4] = [
            |w| {
                w.create_entity(FactionId::new(2), DEFAULT_PLAYER_REGION);
            },
            |w| {
                w.robot_registry.config.player_max_speed += 1.0;
            },
            |w| {
                w.structure_registry.logistics.telemetry.jobs_created_total += 1;
            },
            |w| {
                w.terrain.max_x -= 1.0;
            },
        ];
        for (index, mutate) in diverge.iter().enumerate() {
            let base = WorldState::with_seed(42);
            let mut changed = base.clone();
            mutate(&mut changed);
            assert!(
                base != changed,
                "divergence case {index} went unnoticed by WorldState equality"
            );
        }
    }

    /// A7 — the world state is the one the servers own, and the session
    /// directives the dispatcher raises survive a round trip through it.
    #[test]
    fn test_a7_world_state_carries_authorized_session_directives() {
        let mut world = WorldState::new();
        let actor = ActorContext::new(SessionId::new(1), PlayerId::new(1), FactionId::new(1))
            .with_admin_role(crate::dispatch::AdminRoleCode::new(3))
            .resolve_avatar(&mut world, DEFAULT_PLAYER_REGION);

        world.add_command(CommandEnvelope::new(
            actor.session_id,
            1,
            SimTick::zero(),
            Command::AdminSetSessionRole {
                target_session: SessionId::new(2),
                role_code: 1,
            },
        ));
        let envelopes: Vec<CommandEnvelope> = world.command_buffer.drain_ordered().collect();
        for envelope in envelopes {
            apply_command(&mut world, &actor, &envelope.command).unwrap();
        }

        assert_eq!(
            world.drain_session_directives(),
            vec![SessionDirective::SetSessionRole {
                target: SessionId::new(2),
                role_code: 1,
            }]
        );
        assert!(world.pending_session_directives.is_empty());
    }
}
