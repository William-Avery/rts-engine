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
use crate::combat::ProjectileRegistry;
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
    pub projectiles: ProjectileRegistry,
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

/// Test if a 2D line segment intersects a circle and return the normalized parameter t in [0, 1].
fn segment_intersects_circle(
    x0: f32,
    z0: f32,
    x1: f32,
    z1: f32,
    cx: f32,
    cz: f32,
    radius: f32,
) -> Option<f32> {
    let dx = x1 - x0;
    let dz = z1 - z0;
    let len_sq = dx * dx + dz * dz;
    let t = if len_sq < 1e-6 {
        0.0
    } else {
        let proj = (cx - x0) * dx + (cz - z0) * dz;
        (proj / len_sq).clamp(0.0, 1.0)
    };
    let px = x0 + t * dx;
    let pz = z0 + t * dz;
    let dist_sq = (cx - px).powi(2) + (cz - pz).powi(2);
    if dist_sq <= radius * radius {
        Some(t)
    } else {
        None
    }
}

/// Test if a 2D line segment intersects an axis-aligned bounding box and return entry parameter t.
fn segment_intersects_aabb(
    p0: (f32, f32),
    p1: (f32, f32),
    b_min: (f32, f32),
    b_max: (f32, f32),
) -> Option<f32> {
    let dx = p1.0 - p0.0;
    let dz = p1.1 - p0.1;
    let mut t_min = 0.0f32;
    let mut t_max = 1.0f32;

    if dx.abs() < 1e-6 {
        if p0.0 < b_min.0 || p0.0 > b_max.0 {
            return None;
        }
    } else {
        let inv_d = 1.0 / dx;
        let mut t1 = (b_min.0 - p0.0) * inv_d;
        let mut t2 = (b_max.0 - p0.0) * inv_d;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        t_min = t_min.max(t1);
        t_max = t_max.min(t2);
        if t_min > t_max {
            return None;
        }
    }

    if dz.abs() < 1e-6 {
        if p0.1 < b_min.1 || p0.1 > b_max.1 {
            return None;
        }
    } else {
        let inv_d = 1.0 / dz;
        let mut t1 = (b_min.1 - p0.1) * inv_d;
        let mut t2 = (b_max.1 - p0.1) * inv_d;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        t_min = t_min.max(t1);
        t_max = t_max.min(t2);
        if t_min > t_max {
            return None;
        }
    }

    if t_min <= 1.0 && t_max >= 0.0 {
        Some(t_min.max(0.0))
    } else {
        None
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
            projectiles: ProjectileRegistry::new(),
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

        // Collect newly spawned projectiles from robots
        for proj in self.robot_registry.drain_projectiles() {
            let pid = self.projectiles.spawn(proj.clone());
            self.event_journal.record(
                self.tick,
                SimEvent::ProjectileSpawned {
                    projectile_id: pid,
                    owner: proj.owner,
                    position: proj.position,
                },
            );
        }

        // Turret automated targeting and firing
        self.step_turret_combat();

        // Advance active projectiles and resolve impacts
        self.step_projectiles();

        // Run multi-rate scheduler across regions
        self.scheduler
            .tick(self.tick, &mut self.region_map, &mut self.router);
    }

    /// Automated target acquisition and firing for operational defensive turrets.
    pub fn step_turret_combat(&mut self) {
        struct TurretShot {
            faction_id: FactionId,
            origin: (f32, f32, f32),
            dir: (f32, f32, f32),
            dmg: crate::wall::DamageSpec,
            speed: f32,
            max_range: f32,
        }

        let mut turret_shots: Vec<TurretShot> = Vec::new();

        for structure in self.structure_registry.structures.values_mut() {
            if !structure.can_fire() {
                continue;
            }
            if let Some(ref mut weapon) = structure.weapon {
                weapon.tick();
                if !weapon.can_fire() {
                    continue;
                }

                // Acquire nearest hostile robot within range
                let mut best_target: Option<((f32, f32, f32), f32)> = None;
                for robot in self.robot_registry.robots.values() {
                    if robot.faction_id == structure.faction_id || robot.current_hp == 0 {
                        continue;
                    }
                    let d = crate::navigation::planar_distance(structure.position, robot.position);
                    if d <= weapon.def.range {
                        match best_target {
                            None => best_target = Some((robot.position, d)),
                            Some((_, best_d)) if d < best_d => {
                                best_target = Some((robot.position, d))
                            }
                            _ => {}
                        }
                    }
                }

                if let Some((target_pos, dist)) = best_target {
                    let fire_rate_mod = self.research_manager.modifiers.multiplier_milli(
                        structure.faction_id,
                        crate::modifier::ModifierKind::WeaponFireRate,
                    );
                    let damage_mod = self.research_manager.modifiers.multiplier_milli(
                        structure.faction_id,
                        crate::modifier::ModifierKind::WeaponDamage,
                    );

                    if weapon.discharge(fire_rate_mod) {
                        let dx = target_pos.0 - structure.position.0;
                        let dz = target_pos.2 - structure.position.2;
                        let dir = if dist > 0.001 {
                            (dx / dist, 0.0, dz / dist)
                        } else {
                            (0.0, 0.0, 1.0)
                        };
                        let mut dmg = weapon.def.base_damage;
                        dmg.raw_damage = (dmg.raw_damage * (damage_mod as f32 / 1000.0)).max(1.0);
                        dmg.source = Some(EntityId::new(structure.id.value()));

                        if let crate::combat::MotionPrimitive::Linear { speed, max_range } =
                            weapon.def.motion
                        {
                            turret_shots.push(TurretShot {
                                faction_id: structure.faction_id,
                                origin: structure.position,
                                dir,
                                dmg,
                                speed,
                                max_range,
                            });
                        }
                    }
                }
            }
        }

        for shot in turret_shots {
            let proj = crate::combat::Projectile::new_linear(crate::combat::LinearProjectileSpec {
                id: game_types::ProjectileId(0),
                owner: None,
                faction_id: shot.faction_id,
                origin: shot.origin,
                direction: shot.dir,
                speed: shot.speed,
                max_range: shot.max_range,
                damage: shot.dmg,
                splash_radius: 0.0,
                spawn_tick: self.tick,
            });
            self.projectiles.spawn(proj);
        }
    }

    /// Advance active projectiles, perform swept collision testing, and apply damage.
    pub fn step_projectiles(&mut self) {
        let current_tick = self.tick;
        let impacts = self
            .projectiles
            .step_all(current_tick, |proj, old_pos, new_pos| {
                let dx = new_pos.0 - old_pos.0;
                let dy = new_pos.1 - old_pos.1;
                let dz = new_pos.2 - old_pos.2;

                let mut best_hit: Option<(f32, EntityId, (f32, f32, f32))> = None;

                // 1. Robot body swept collision check
                for robot in self.robot_registry.robots.values() {
                    if proj.owner == Some(robot.entity)
                        || proj.faction_id == robot.faction_id
                        || robot.current_hp == 0
                    {
                        continue;
                    }
                    let body_r = robot.archetype().body_radius;
                    if let Some(t) = segment_intersects_circle(
                        old_pos.0,
                        old_pos.2,
                        new_pos.0,
                        new_pos.2,
                        robot.position.0,
                        robot.position.2,
                        body_r,
                    ) {
                        let hit_pos = (old_pos.0 + t * dx, old_pos.1 + t * dy, old_pos.2 + t * dz);
                        match best_hit {
                            None => best_hit = Some((t, robot.entity, hit_pos)),
                            Some((best_t, _, _)) if t < best_t => {
                                best_hit = Some((t, robot.entity, hit_pos));
                            }
                            _ => {}
                        }
                    }
                }

                // 2. Structure AABB swept collision check
                for structure in self.structure_registry.structures.values() {
                    if structure.faction_id == proj.faction_id {
                        continue;
                    }
                    let (min_x, max_x, min_z, max_z) = (
                        structure.bounds_min.0,
                        structure.bounds_max.0,
                        structure.bounds_min.2,
                        structure.bounds_max.2,
                    );
                    if let Some(t) = segment_intersects_aabb(
                        (old_pos.0, old_pos.2),
                        (new_pos.0, new_pos.2),
                        (min_x, min_z),
                        (max_x, max_z),
                    ) {
                        let hit_pos = (old_pos.0 + t * dx, old_pos.1 + t * dy, old_pos.2 + t * dz);
                        let struct_entity = EntityId::new(structure.id.value());
                        match best_hit {
                            None => best_hit = Some((t, struct_entity, hit_pos)),
                            Some((best_t, _, _)) if t < best_t => {
                                best_hit = Some((t, struct_entity, hit_pos));
                            }
                            _ => {}
                        }
                    }
                }

                // 3. Terrain floor collision check (for ballistic trajectories)
                if matches!(
                    proj.motion,
                    crate::combat::MotionPrimitive::Ballistic { .. }
                ) {
                    let floor_t = if old_pos.1 > 0.0 && new_pos.1 <= 0.0 {
                        let denom = old_pos.1 - new_pos.1;
                        if denom.abs() > 1e-6 {
                            Some((old_pos.1 / denom).clamp(0.0, 1.0))
                        } else {
                            Some(0.0)
                        }
                    } else if new_pos.1 <= 0.0 && old_pos.1 <= 0.0 {
                        Some(0.0)
                    } else {
                        None
                    };

                    if let Some(t) = floor_t {
                        match best_hit {
                            None => {
                                let hit_pos = (old_pos.0 + t * dx, 0.0, old_pos.2 + t * dz);
                                best_hit = Some((t, EntityId::null(), hit_pos));
                            }
                            Some((best_t, _, _)) if t < best_t => {
                                let hit_pos = (old_pos.0 + t * dx, 0.0, old_pos.2 + t * dz);
                                best_hit = Some((t, EntityId::null(), hit_pos));
                            }
                            _ => {}
                        }
                    }
                }

                best_hit.map(|(_, entity, pos)| (entity, pos))
            });

        for impact in impacts {
            self.event_journal.record(
                current_tick,
                SimEvent::ProjectileImpacted {
                    projectile_id: impact.projectile_id,
                    target: if impact.target.is_null() {
                        None
                    } else {
                        Some(impact.target)
                    },
                    position: impact.hit_pos,
                },
            );

            // Direct target damage
            if !impact.target.is_null() {
                if self.robot_registry.robots.contains_key(&impact.target) {
                    let _ = self.robot_registry.apply_damage(
                        impact.target,
                        impact.damage,
                        current_tick,
                        &mut self.event_journal,
                    );
                } else {
                    let structure_id = StructureId::new(impact.target.0);
                    if self.structure_registry.get(structure_id).is_some() {
                        let _ = self.structure_registry.apply_damage(
                            structure_id,
                            impact.damage,
                            current_tick,
                            &mut self.event_journal,
                        );
                    }
                }
            }

            // Splash AoE damage with radial distance falloff
            if impact.splash_radius > 0.0 {
                let splash_robots: Vec<(EntityId, f32)> = self
                    .robot_registry
                    .robots
                    .values()
                    .filter(|r| {
                        r.entity != impact.target
                            && r.faction_id != impact.faction_id
                            && r.current_hp > 0
                    })
                    .map(|r| {
                        (
                            r.entity,
                            crate::navigation::planar_distance(impact.hit_pos, r.position),
                        )
                    })
                    .filter(|(_, dist)| *dist <= impact.splash_radius)
                    .collect();

                for (target_id, dist) in splash_robots {
                    let falloff = (1.0 - (dist / impact.splash_radius)).clamp(0.1, 1.0);
                    let splash_dmg = impact.damage.scaled(falloff);
                    let _ = self.robot_registry.apply_damage(
                        target_id,
                        splash_dmg,
                        current_tick,
                        &mut self.event_journal,
                    );
                }

                let splash_structures: Vec<(StructureId, f32)> = self
                    .structure_registry
                    .structures
                    .values()
                    .filter(|s| {
                        EntityId::new(s.id.value()) != impact.target
                            && s.faction_id != impact.faction_id
                    })
                    .map(|s| {
                        (
                            s.id,
                            crate::navigation::planar_distance(impact.hit_pos, s.position),
                        )
                    })
                    .filter(|(_, dist)| *dist <= impact.splash_radius)
                    .collect();

                for (s_id, dist) in splash_structures {
                    let falloff = (1.0 - (dist / impact.splash_radius)).clamp(0.1, 1.0);
                    let splash_dmg = impact.damage.scaled(falloff);
                    let _ = self.structure_registry.apply_damage(
                        s_id,
                        splash_dmg,
                        current_tick,
                        &mut self.event_journal,
                    );
                }
            }
        }
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
