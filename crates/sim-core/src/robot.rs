use crate::chassis::{RobotArchetype, RobotChassis};
use crate::combat::{MotionPrimitive, Projectile, StatusStore, WeaponState};
use crate::command::RobotCommandType;
use crate::event::{EventJournal, SimEvent};
use crate::navigation::{
    DirectSteering, NavAgent, NavGoal, NavObstacle, NavigationProvider, SteeringParams,
    heading_deg, planar_distance, planar_length, standoff_position, turn_toward_deg,
};
use crate::wall::{DamageResult, DamageSpec, calculate_damage};
use game_types::{
    EntityId, FactionId, GameError, GameResult, PlayerId, ProjectileId, RegionId, SessionId,
    SimTick, SquadId, WeaponId,
};
use std::collections::BTreeMap;

/// Minimum planar speed before a robot re-aims its facing toward its velocity.
const FACING_MIN_SPEED: f32 = 0.05;

/// Server-owned deterministic mapping from an authenticated session to a player identity.
///
/// Clients never supply their own player id: it is derived from the session the server
/// itself issued at handshake, so an escort assignment cannot be forged for another player.
pub const fn player_for_session(session_id: SessionId) -> PlayerId {
    PlayerId::new(session_id.value() as u32)
}

/// Authoritative standing order for a single robot.
#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub enum RobotOrder {
    /// Hold position, no goal.
    #[default]
    Idle,
    /// Move to a world position and hold there.
    MoveTo { position: (f32, f32, f32) },
    /// Trail a player or robot at a standoff distance.
    Follow { target: EntityId, standoff: f32 },
    /// Hold a guard post, returning to it when displaced.
    Guard { position: (f32, f32, f32) },
    /// Close to engagement standoff against a target.
    /// Milestone 13 adds the weapon fire resolution on top of this positioning.
    Attack { target: EntityId },
    /// Return to the robot's recorded home position.
    ReturnToBase,
    /// Move to this robot's formation slot in its squad's rally formation.
    Regroup { squad_id: SquadId },
}

impl RobotOrder {
    pub const fn as_u8(&self) -> u8 {
        match self {
            RobotOrder::Idle => 0,
            RobotOrder::MoveTo { .. } => 1,
            RobotOrder::Follow { .. } => 2,
            RobotOrder::Guard { .. } => 3,
            RobotOrder::Attack { .. } => 4,
            RobotOrder::ReturnToBase => 5,
            RobotOrder::Regroup { .. } => 6,
        }
    }
}

/// Authoritative server-side state of one biped robot.
#[derive(Clone, Debug, PartialEq)]
pub struct Robot {
    pub entity: EntityId,
    pub chassis: RobotChassis,
    pub faction_id: FactionId,
    pub region_id: RegionId,
    /// Owning player when this robot is an assigned personal escort.
    pub owner: Option<PlayerId>,
    pub squad: Option<SquadId>,
    pub position: (f32, f32, f32),
    pub velocity: (f32, f32, f32),
    /// Yaw in degrees, 0 = +Z.
    pub facing_deg: f32,
    pub current_hp: u32,
    pub order: RobotOrder,
    pub order_tick: SimTick,
    pub home_position: (f32, f32, f32),
    /// True when the robot has reached its current goal.
    pub at_goal: bool,
    pub spawn_tick: SimTick,
    pub weapon: Option<WeaponState>,
    pub status: StatusStore,
}

impl Robot {
    pub fn new(
        entity: EntityId,
        chassis: RobotChassis,
        faction_id: FactionId,
        region_id: RegionId,
        position: (f32, f32, f32),
        spawn_tick: SimTick,
    ) -> Self {
        let weapon = chassis
            .archetype()
            .default_weapon(WeaponId(entity.0 as u32))
            .map(WeaponState::new);
        Robot {
            entity,
            chassis,
            faction_id,
            region_id,
            owner: None,
            squad: None,
            position,
            velocity: (0.0, 0.0, 0.0),
            facing_deg: 0.0,
            current_hp: chassis.archetype().max_health,
            order: RobotOrder::Idle,
            order_tick: spawn_tick,
            home_position: position,
            at_goal: true,
            spawn_tick,
            weapon,
            status: StatusStore::new(),
        }
    }

    pub fn archetype(&self) -> &'static RobotArchetype {
        self.chassis.archetype()
    }

    pub fn is_alive(&self) -> bool {
        self.current_hp > 0
    }

    pub fn is_escort(&self) -> bool {
        self.owner.is_some()
    }

    /// Current planar speed in meters per second.
    pub fn planar_speed(&self) -> f32 {
        planar_length(self.velocity)
    }

    /// Health fraction in range [0.0, 1.0] for presentation and repair logic.
    pub fn health_ratio(&self) -> f32 {
        let max = self.archetype().max_health;
        if max == 0 {
            0.0
        } else {
            (self.current_hp as f32 / max as f32).clamp(0.0, 1.0)
        }
    }
}

/// A first-class deterministic squad: an ordered roster with an explicit leader.
#[derive(Clone, Debug, PartialEq)]
pub struct Squad {
    pub id: SquadId,
    pub faction_id: FactionId,
    pub leader: EntityId,
    /// Ordered member roster; index determines the deterministic formation slot.
    pub members: Vec<EntityId>,
    pub rally_position: (f32, f32, f32),
    pub formation_spacing: f32,
}

impl Squad {
    pub fn new(id: SquadId, faction_id: FactionId) -> Self {
        Squad {
            id,
            faction_id,
            leader: EntityId::null(),
            members: Vec::new(),
            rally_position: (0.0, 0.0, 0.0),
            formation_spacing: 3.0,
        }
    }

    pub fn contains(&self, robot: EntityId) -> bool {
        self.members.contains(&robot)
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// Deterministic formation slot offset for the member at `index`.
    ///
    /// Slot 0 is the rally point itself (the leader); remaining slots spiral outward in a
    /// stable ring pattern so the same roster always produces the same formation.
    pub fn slot_offset(&self, index: usize) -> (f32, f32) {
        if index == 0 {
            return (0.0, 0.0);
        }
        let ring = index.div_ceil(6);
        let slots_in_ring = ring * 6;
        let slot_in_ring = (index - 1) % slots_in_ring;
        let angle = (slot_in_ring as f32 / slots_in_ring as f32) * std::f32::consts::TAU;
        let radius = ring as f32 * self.formation_spacing;
        (angle.sin() * radius, angle.cos() * radius)
    }

    /// World-space formation position for the member at `index`.
    pub fn slot_position(&self, index: usize) -> (f32, f32, f32) {
        let (ox, oz) = self.slot_offset(index);
        (
            self.rally_position.0 + ox,
            self.rally_position.1,
            self.rally_position.2 + oz,
        )
    }
}

/// Authoritative presence record of a connected player avatar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlayerPresence {
    pub player: PlayerId,
    pub entity: EntityId,
    pub faction_id: FactionId,
    pub region_id: RegionId,
    pub position: (f32, f32, f32),
    pub last_update_tick: SimTick,
}

impl PlayerPresence {
    pub fn new(
        player: PlayerId,
        entity: EntityId,
        faction_id: FactionId,
        region_id: RegionId,
        position: (f32, f32, f32),
    ) -> Self {
        PlayerPresence {
            player,
            entity,
            faction_id,
            region_id,
            position,
            last_update_tick: SimTick::zero(),
        }
    }
}

/// Tunable rules for the robot framework.
///
/// Every gameplay number that research, doctrine, or a data pack may later raise lives
/// here rather than as a literal inside the simulation code.
#[derive(Clone, Debug, PartialEq)]
pub struct RobotConfig {
    /// Fixed deterministic integration step in seconds (30 Hz base simulation rate).
    pub fixed_step_seconds: f32,
    /// Escort slots every player starts with.
    pub base_escort_cap: u8,
    /// Progression ceiling: research/doctrine may raise a player up to this many escorts.
    pub max_escort_cap: u8,
    /// Per-player escort capacity granted by progression, clamped to `max_escort_cap`.
    pub escort_cap_overrides: BTreeMap<PlayerId, u8>,
    /// Uniform grid cell size used for local neighbour queries.
    pub neighbor_cell_size: f32,
    /// Physical body radius assumed for a player avatar.
    pub player_body_radius: f32,
    /// Maximum authoritative player speed used to clamp client-reported movement.
    pub player_max_speed: f32,
    /// Distance at which an `Attack` order stops closing on its target.
    pub attack_standoff: f32,
    /// Fraction of maximum speed contributed by a fully overlapping neighbour.
    pub separation_strength: f32,
}

impl Default for RobotConfig {
    fn default() -> Self {
        RobotConfig {
            fixed_step_seconds: 1.0 / 30.0,
            base_escort_cap: 1,
            max_escort_cap: 2,
            escort_cap_overrides: BTreeMap::new(),
            neighbor_cell_size: 4.0,
            player_body_radius: 0.5,
            player_max_speed: 10.0,
            attack_standoff: 12.0,
            separation_strength: 0.8,
        }
    }
}

impl RobotConfig {
    /// Effective escort capacity for a player, honouring progression grants and the ceiling.
    pub fn escort_cap_for(&self, player: PlayerId) -> u8 {
        self.escort_cap_overrides
            .get(&player)
            .copied()
            .unwrap_or(self.base_escort_cap)
            .min(self.max_escort_cap)
    }

    /// Grant a player a progression escort capacity. Returns the clamped effective value.
    pub fn grant_escort_cap(&mut self, player: PlayerId, cap: u8) -> u8 {
        let clamped = cap.min(self.max_escort_cap);
        self.escort_cap_overrides.insert(player, clamped);
        clamped
    }
}

/// Parameters specifying an authoritative robot spawn request.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RobotSpawnRequest {
    pub entity: EntityId,
    pub chassis: RobotChassis,
    pub faction_id: FactionId,
    pub region_id: RegionId,
    pub position: (f32, f32, f32),
    pub spawn_tick: SimTick,
}

impl RobotSpawnRequest {
    pub const fn new(
        entity: EntityId,
        chassis: RobotChassis,
        faction_id: FactionId,
        region_id: RegionId,
        position: (f32, f32, f32),
        spawn_tick: SimTick,
    ) -> Self {
        RobotSpawnRequest {
            entity,
            chassis,
            faction_id,
            region_id,
            position,
            spawn_tick,
        }
    }
}

/// Central authoritative registry of robots, squads, escort assignments, and player presence.
#[derive(Clone, Debug, PartialEq)]
pub struct RobotRegistry {
    pub robots: BTreeMap<EntityId, Robot>,
    pub squads: BTreeMap<SquadId, Squad>,
    /// Ordered escort roster per player. The server is the only writer.
    pub escorts: BTreeMap<PlayerId, Vec<EntityId>>,
    pub players: BTreeMap<PlayerId, PlayerPresence>,
    pub config: RobotConfig,
    pub next_squad_id: u32,
    pub pending_projectiles: Vec<Projectile>,
}

impl Default for RobotRegistry {
    fn default() -> Self {
        RobotRegistry {
            robots: BTreeMap::new(),
            squads: BTreeMap::new(),
            escorts: BTreeMap::new(),
            players: BTreeMap::new(),
            config: RobotConfig::default(),
            next_squad_id: 1,
            pending_projectiles: Vec::new(),
        }
    }
}

impl RobotRegistry {
    pub fn new() -> Self {
        RobotRegistry::default()
    }

    /// Drain all pending projectiles spawned during the tick.
    pub fn drain_projectiles(&mut self) -> Vec<Projectile> {
        std::mem::take(&mut self.pending_projectiles)
    }

    // ---------------------------------------------------------------- robots

    /// Register an already-allocated entity as an authoritative robot.
    pub fn spawn_robot(&mut self, request: RobotSpawnRequest) -> GameResult<()> {
        if request.entity.is_null() {
            return Err(GameError::InvalidId);
        }
        if self.robots.contains_key(&request.entity) {
            return Err(GameError::InvalidStateTransition);
        }
        self.robots.insert(
            request.entity,
            Robot::new(
                request.entity,
                request.chassis,
                request.faction_id,
                request.region_id,
                request.position,
                request.spawn_tick,
            ),
        );
        Ok(())
    }

    /// Register a robot and record the spawn in the event journal.
    pub fn spawn_robot_with_journal(
        &mut self,
        request: RobotSpawnRequest,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        self.spawn_robot(request)?;
        journal.record(
            request.spawn_tick,
            SimEvent::RobotSpawned {
                robot: request.entity,
                chassis: request.chassis.as_u8(),
                faction_id: request.faction_id,
                position: request.position,
            },
        );
        Ok(())
    }

    pub fn get(&self, entity: EntityId) -> Option<&Robot> {
        self.robots.get(&entity)
    }

    pub fn get_mut(&mut self, entity: EntityId) -> Option<&mut Robot> {
        self.robots.get_mut(&entity)
    }

    pub fn contains(&self, entity: EntityId) -> bool {
        self.robots.contains_key(&entity)
    }

    pub fn count(&self) -> usize {
        self.robots.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Robot> {
        self.robots.values()
    }

    /// Move a robot to another simulation region without disturbing its ownership or order.
    pub fn transfer_region(
        &mut self,
        entity: EntityId,
        destination: RegionId,
    ) -> GameResult<RegionId> {
        let robot = self
            .robots
            .get_mut(&entity)
            .ok_or(GameError::RobotNotFound(entity))?;
        let previous = robot.region_id;
        robot.region_id = destination;
        Ok(previous)
    }

    // --------------------------------------------------------------- players

    /// Register or replace an authoritative player presence record.
    pub fn register_player(&mut self, presence: PlayerPresence) -> GameResult<()> {
        if presence.player.is_null() {
            return Err(GameError::InvalidId);
        }
        self.players.insert(presence.player, presence);
        self.escorts.entry(presence.player).or_default();
        Ok(())
    }

    pub fn player(&self, player: PlayerId) -> Option<&PlayerPresence> {
        self.players.get(&player)
    }

    /// Apply a client-reported movement intent under server authority.
    ///
    /// The server owns the final position: a reported position is clamped to the distance
    /// the player could physically have covered since their last accepted update, so a
    /// client cannot teleport itself (or drag its escort) across the map.
    pub fn apply_player_move(
        &mut self,
        player: PlayerId,
        reported_position: (f32, f32, f32),
        tick: SimTick,
    ) -> GameResult<(f32, f32, f32)> {
        let max_speed = self.config.player_max_speed;
        let step = self.config.fixed_step_seconds;
        let presence = self
            .players
            .get_mut(&player)
            .ok_or(GameError::PermissionDenied)?;

        let elapsed = tick
            .value()
            .saturating_sub(presence.last_update_tick.value());
        // Always allow at least one step of travel, and cap catch-up at one second.
        let allowance = max_speed * step * (elapsed.clamp(1, 30) as f32);

        let dx = reported_position.0 - presence.position.0;
        let dz = reported_position.2 - presence.position.2;
        let dist = (dx * dx + dz * dz).sqrt();

        let accepted = if dist <= allowance || dist < 1.0e-5 {
            reported_position
        } else {
            let scale = allowance / dist;
            (
                presence.position.0 + dx * scale,
                reported_position.1,
                presence.position.2 + dz * scale,
            )
        };

        presence.position = accepted;
        presence.last_update_tick = tick;
        Ok(accepted)
    }

    // --------------------------------------------------------------- escorts

    /// Escort roster of a player in deterministic assignment order.
    pub fn escorts_for(&self, player: PlayerId) -> &[EntityId] {
        self.escorts.get(&player).map_or(&[], |v| v.as_slice())
    }

    /// Effective escort capacity for a player.
    pub fn escort_cap_for(&self, player: PlayerId) -> u8 {
        self.config.escort_cap_for(player)
    }

    /// Assign a robot as `owner`'s personal escort.
    ///
    /// `actor` is the player identity the server derived from the authenticated session.
    /// A client cannot assign an escort to anyone but itself, and cannot take over a robot
    /// already escorting another player.
    pub fn assign_escort(
        &mut self,
        actor: PlayerId,
        owner: PlayerId,
        robot: EntityId,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        if actor.is_null() || actor != owner {
            return Err(GameError::PermissionDenied);
        }
        let presence = *self
            .players
            .get(&owner)
            .ok_or(GameError::PermissionDenied)?;
        let target = self
            .robots
            .get(&robot)
            .ok_or(GameError::RobotNotFound(robot))?;
        if target.faction_id != presence.faction_id {
            return Err(GameError::PermissionDenied);
        }
        match target.owner {
            Some(existing) if existing == owner => {
                return Err(GameError::EscortAlreadyAssigned(robot));
            }
            // A robot already escorting a different player can never be stolen.
            Some(_) => return Err(GameError::PermissionDenied),
            None => {}
        }

        let standoff = target.archetype().follow_standoff;
        let roster = self.escorts.entry(owner).or_default();
        let cap = self.config.escort_cap_for(owner);
        if roster.len() >= cap as usize {
            return Err(GameError::EscortCapExceeded {
                assigned: roster.len().min(u8::MAX as usize) as u8,
                cap,
            });
        }
        roster.push(robot);

        if let Some(robot_ref) = self.robots.get_mut(&robot) {
            robot_ref.owner = Some(owner);
            robot_ref.order = RobotOrder::Follow {
                target: presence.entity,
                standoff,
            };
            robot_ref.order_tick = tick;
            robot_ref.at_goal = false;
        }

        journal.record(
            tick,
            SimEvent::EscortAssigned {
                player: owner,
                robot,
            },
        );
        Ok(())
    }

    /// Release an escort back to unassigned status.
    pub fn release_escort(
        &mut self,
        actor: PlayerId,
        owner: PlayerId,
        robot: EntityId,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        if actor.is_null() || actor != owner {
            return Err(GameError::PermissionDenied);
        }
        let current_owner = self
            .robots
            .get(&robot)
            .ok_or(GameError::RobotNotFound(robot))?
            .owner;
        if current_owner != Some(owner) {
            return Err(GameError::PermissionDenied);
        }

        self.detach_escort(owner, robot);
        if let Some(robot_ref) = self.robots.get_mut(&robot) {
            robot_ref.owner = None;
            robot_ref.order = RobotOrder::Guard {
                position: robot_ref.position,
            };
            robot_ref.order_tick = tick;
            robot_ref.at_goal = false;
        }

        journal.record(
            tick,
            SimEvent::EscortReleased {
                player: owner,
                robot,
            },
        );
        Ok(())
    }

    fn detach_escort(&mut self, owner: PlayerId, robot: EntityId) {
        if let Some(roster) = self.escorts.get_mut(&owner) {
            roster.retain(|&e| e != robot);
        }
    }

    // ---------------------------------------------------------------- orders

    /// Verify that `actor` is permitted to command `robot`.
    ///
    /// An assigned escort answers only to its owner. An unassigned robot answers to any
    /// player of its own faction.
    pub fn authorize_order(&self, actor: PlayerId, robot: EntityId) -> GameResult<()> {
        if actor.is_null() {
            return Err(GameError::PermissionDenied);
        }
        let target = self
            .robots
            .get(&robot)
            .ok_or(GameError::RobotNotFound(robot))?;
        match target.owner {
            Some(owner) if owner == actor => Ok(()),
            Some(_) => Err(GameError::PermissionDenied),
            None => {
                let presence = self
                    .players
                    .get(&actor)
                    .ok_or(GameError::PermissionDenied)?;
                if presence.faction_id == target.faction_id {
                    Ok(())
                } else {
                    Err(GameError::PermissionDenied)
                }
            }
        }
    }

    /// Translate a wire-level robot command into an authoritative standing order.
    ///
    /// Returns `None` when the robot is unknown, so an unroutable command is dropped
    /// instead of fabricating state.
    pub fn order_for_command(
        &self,
        robot: EntityId,
        command: RobotCommandType,
    ) -> Option<RobotOrder> {
        let standoff = self.robots.get(&robot)?.archetype().follow_standoff;
        Some(match command {
            RobotCommandType::Follow { target } => RobotOrder::Follow { target, standoff },
            RobotCommandType::Guard { position } => RobotOrder::Guard { position },
            RobotCommandType::Attack { target } => RobotOrder::Attack { target },
            RobotCommandType::Move { position } => RobotOrder::MoveTo { position },
            RobotCommandType::ReturnToBase => RobotOrder::ReturnToBase,
        })
    }

    /// Validate and apply a standing order to a robot.
    pub fn issue_order(
        &mut self,
        actor: PlayerId,
        robot: EntityId,
        order: RobotOrder,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        self.authorize_order(actor, robot)?;
        self.set_order(robot, order, tick, journal)
    }

    /// Apply a standing order under server authority, skipping the actor permission check.
    pub fn set_order(
        &mut self,
        robot: EntityId,
        order: RobotOrder,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        if let RobotOrder::Regroup { squad_id } = order
            && !self.squads.contains_key(&squad_id)
        {
            return Err(GameError::SquadNotFound(squad_id));
        }
        let robot_ref = self
            .robots
            .get_mut(&robot)
            .ok_or(GameError::RobotNotFound(robot))?;
        robot_ref.order = order;
        robot_ref.order_tick = tick;
        robot_ref.at_goal = false;
        journal.record(
            tick,
            SimEvent::RobotOrderIssued {
                robot,
                order_code: order.as_u8(),
            },
        );
        Ok(())
    }

    // ---------------------------------------------------------------- squads

    /// Create an empty squad for a faction.
    pub fn create_squad(
        &mut self,
        faction_id: FactionId,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> SquadId {
        let id = SquadId::new(self.next_squad_id);
        self.next_squad_id += 1;
        self.squads.insert(id, Squad::new(id, faction_id));
        journal.record(
            tick,
            SimEvent::SquadFormed {
                squad_id: id,
                leader: EntityId::null(),
                faction_id,
            },
        );
        id
    }

    pub fn squad(&self, squad_id: SquadId) -> Option<&Squad> {
        self.squads.get(&squad_id)
    }

    /// Add a robot to a squad roster. The first member becomes the squad leader.
    pub fn assign_squad_member(
        &mut self,
        actor: PlayerId,
        squad_id: SquadId,
        robot: EntityId,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        self.authorize_order(actor, robot)?;
        let faction = self
            .robots
            .get(&robot)
            .ok_or(GameError::RobotNotFound(robot))?
            .faction_id;
        let squad = self
            .squads
            .get_mut(&squad_id)
            .ok_or(GameError::SquadNotFound(squad_id))?;
        if squad.faction_id != faction {
            return Err(GameError::PermissionDenied);
        }
        if squad.contains(robot) {
            return Err(GameError::InvalidStateTransition);
        }
        squad.members.push(robot);
        if squad.leader.is_null() {
            squad.leader = robot;
        }
        if let Some(robot_ref) = self.robots.get_mut(&robot) {
            robot_ref.squad = Some(squad_id);
        }
        journal.record(tick, SimEvent::SquadMemberAssigned { squad_id, robot });
        Ok(())
    }

    /// Remove a robot from a squad roster, promoting a new leader when required.
    pub fn remove_squad_member(
        &mut self,
        actor: PlayerId,
        squad_id: SquadId,
        robot: EntityId,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        self.authorize_order(actor, robot)?;
        self.detach_squad_member(squad_id, robot)?;
        journal.record(tick, SimEvent::SquadMemberRemoved { squad_id, robot });
        Ok(())
    }

    fn detach_squad_member(&mut self, squad_id: SquadId, robot: EntityId) -> GameResult<()> {
        let squad = self
            .squads
            .get_mut(&squad_id)
            .ok_or(GameError::SquadNotFound(squad_id))?;
        if !squad.contains(robot) {
            return Err(GameError::InvalidStateTransition);
        }
        squad.members.retain(|&m| m != robot);
        if squad.leader == robot {
            squad.leader = squad.members.first().copied().unwrap_or(EntityId::null());
        }
        if let Some(robot_ref) = self.robots.get_mut(&robot)
            && robot_ref.squad == Some(squad_id)
        {
            robot_ref.squad = None;
            if matches!(robot_ref.order, RobotOrder::Regroup { .. }) {
                robot_ref.order = RobotOrder::Guard {
                    position: robot_ref.position,
                };
            }
        }
        Ok(())
    }

    /// Order an entire squad to reform on a rally point.
    ///
    /// Every member receives a `Regroup` order; formation slots are derived from the
    /// ordered roster, so the resulting formation is fully deterministic.
    pub fn regroup_squad(
        &mut self,
        actor: PlayerId,
        squad_id: SquadId,
        rally_position: (f32, f32, f32),
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<usize> {
        let members = {
            let squad = self
                .squads
                .get(&squad_id)
                .ok_or(GameError::SquadNotFound(squad_id))?;
            squad.members.clone()
        };
        // Authorize every member up front so a partial regroup can never happen.
        for &member in &members {
            self.authorize_order(actor, member)?;
        }

        if let Some(squad) = self.squads.get_mut(&squad_id) {
            squad.rally_position = rally_position;
        }
        for &member in &members {
            if let Some(robot_ref) = self.robots.get_mut(&member) {
                robot_ref.order = RobotOrder::Regroup { squad_id };
                robot_ref.order_tick = tick;
                robot_ref.at_goal = false;
            }
        }
        journal.record(
            tick,
            SimEvent::SquadRegrouped {
                squad_id,
                rally_position,
                member_count: members.len() as u32,
            },
        );
        Ok(members.len())
    }

    /// Clean up a destroyed robot and record relevant release and destruction events.
    pub fn cleanup_destroyed_robot(
        &mut self,
        robot: EntityId,
        source: Option<EntityId>,
        tick: SimTick,
        journal: &mut EventJournal,
    ) {
        if let Some(target) = self.robots.remove(&robot) {
            if let Some(owner) = target.owner {
                self.detach_escort(owner, robot);
                journal.record(
                    tick,
                    SimEvent::EscortReleased {
                        player: owner,
                        robot,
                    },
                );
            }
            if let Some(squad_id) = target.squad {
                let _ = self.detach_squad_member(squad_id, robot);
            }
            journal.record(tick, SimEvent::RobotDestroyed { robot, source });
        }
    }

    /// Apply authoritative damage to a robot using the shared armor/resistance model.
    pub fn apply_damage(
        &mut self,
        robot: EntityId,
        damage: DamageSpec,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<DamageResult> {
        let (result, destroyed) = {
            let target = self
                .robots
                .get_mut(&robot)
                .ok_or(GameError::RobotNotFound(robot))?;
            let mut armor = target.archetype().armor_profile();
            let degradation = target.status.total_armor_degradation();
            armor.flat_armor = (armor.flat_armor - degradation).max(0.0);

            let result = calculate_damage(armor, target.current_hp, damage);
            target.current_hp = result.remaining_hp;
            if let Some(effect) = damage.effect {
                target.status.apply_effect(effect);
            }
            (result, result.destroyed)
        };

        journal.record(
            tick,
            SimEvent::DamageDealt {
                entity: robot,
                damage: result.effective_damage,
                source: damage.source,
            },
        );

        if destroyed {
            self.cleanup_destroyed_robot(robot, damage.source, tick, journal);
        }

        Ok(result)
    }

    // ------------------------------------------------------------------ tick

    /// Resolve the world-space navigation goal of a robot's current order.
    pub fn goal_for(&self, robot: &Robot) -> NavGoal {
        match robot.order {
            RobotOrder::Idle => NavGoal::Hold,
            RobotOrder::MoveTo { position } => NavGoal::Position(position),
            RobotOrder::Guard { position } => NavGoal::Position(position),
            RobotOrder::ReturnToBase => NavGoal::Position(robot.home_position),
            RobotOrder::Follow { target, standoff } => match self.position_of(target) {
                Some(target_pos) => {
                    NavGoal::Position(standoff_position(target_pos, robot.position, standoff))
                }
                None => NavGoal::Hold,
            },
            RobotOrder::Attack { target } => match self.position_of(target) {
                Some(target_pos) => {
                    let standoff = robot
                        .weapon
                        .as_ref()
                        .map(|w| (w.def.range * 0.75).max(1.0))
                        .unwrap_or(self.config.attack_standoff);
                    NavGoal::Position(standoff_position(target_pos, robot.position, standoff))
                }
                None => NavGoal::Hold,
            },
            RobotOrder::Regroup { squad_id } => match self.squads.get(&squad_id) {
                Some(squad) => match squad.members.iter().position(|&m| m == robot.entity) {
                    Some(index) => NavGoal::Position(squad.slot_position(index)),
                    None => NavGoal::Hold,
                },
                None => NavGoal::Hold,
            },
        }
    }

    /// World position of any followable entity (robot body or player avatar).
    pub fn position_of(&self, entity: EntityId) -> Option<(f32, f32, f32)> {
        if let Some(robot) = self.robots.get(&entity) {
            return Some(robot.position);
        }
        self.players
            .values()
            .find(|p| p.entity == entity)
            .map(|p| p.position)
    }

    /// Planar distance between a robot and its follow/attack target, if any.
    pub fn distance_to_target(&self, robot: EntityId) -> Option<f32> {
        let robot_ref = self.robots.get(&robot)?;
        let target = match robot_ref.order {
            RobotOrder::Follow { target, .. } | RobotOrder::Attack { target } => target,
            _ => return None,
        };
        let target_pos = self.position_of(target)?;
        Some(planar_distance(robot_ref.position, target_pos))
    }

    /// Advance all robots one fixed simulation step with the default navigation provider.
    pub fn step(&mut self, tick: SimTick, journal: &mut EventJournal) {
        self.step_with_navigator(tick, &DirectSteering, journal);
    }

    /// Advance all robots one fixed simulation step with an explicit navigation provider.
    ///
    /// Milestone 16 swaps the provider here without touching movement integration.
    pub fn step_with_navigator<N: NavigationProvider + ?Sized>(
        &mut self,
        tick: SimTick,
        nav: &N,
        journal: &mut EventJournal,
    ) {
        if self.robots.is_empty() {
            return;
        }
        let dt = self.config.fixed_step_seconds;
        let cell = self.config.neighbor_cell_size.max(1.0);

        // 1. Deterministic snapshot of every solid body: robots first, then player avatars.
        let mut bodies: Vec<NavObstacle> =
            Vec::with_capacity(self.robots.len() + self.players.len());
        for robot in self.robots.values() {
            bodies.push(NavObstacle::new(
                robot.entity,
                robot.position,
                robot.archetype().body_radius,
            ));
        }
        for presence in self.players.values() {
            if !presence.entity.is_null() {
                bodies.push(NavObstacle::new(
                    presence.entity,
                    presence.position,
                    self.config.player_body_radius,
                ));
            }
        }

        // 2. Uniform grid bucketing so neighbour lookup is local, not O(n^2).
        let mut grid: BTreeMap<(i32, i32), Vec<usize>> = BTreeMap::new();
        for (idx, body) in bodies.iter().enumerate() {
            grid.entry(grid_cell(body.position, cell))
                .or_default()
                .push(idx);
        }

        let ids: Vec<EntityId> = self.robots.keys().copied().collect();
        let mut neighbors: Vec<NavObstacle> = Vec::with_capacity(16);
        let mut arrivals: Vec<(EntityId, (f32, f32, f32))> = Vec::new();

        for &id in &ids {
            let (position, velocity, facing_deg, chassis, goal) = match self.robots.get(&id) {
                Some(robot) => (
                    robot.position,
                    robot.velocity,
                    robot.facing_deg,
                    robot.chassis,
                    self.goal_for(robot),
                ),
                None => continue,
            };
            let archetype = chassis.archetype();

            // 3. Gather local neighbours from the 3x3 cell block around the robot.
            neighbors.clear();
            let (cx, cz) = grid_cell(position, cell);
            for gx in (cx - 1)..=(cx + 1) {
                for gz in (cz - 1)..=(cz + 1) {
                    if let Some(bucket) = grid.get(&(gx, gz)) {
                        for &idx in bucket {
                            if bodies[idx].entity != id {
                                neighbors.push(bodies[idx]);
                            }
                        }
                    }
                }
            }

            let speed_mult = match self.robots.get(&id) {
                Some(r) => r.status.speed_multiplier(),
                None => 1.0,
            };
            let effective_speed = archetype.move_speed * speed_mult;

            let params = SteeringParams {
                max_speed: effective_speed,
                step_seconds: dt,
                arrival_tolerance: archetype.arrival_tolerance,
                slowdown_radius: archetype.arrival_tolerance.max(0.1) * 3.0,
                separation_radius: archetype.separation_radius,
                separation_strength: self.config.separation_strength,
                body_radius: archetype.body_radius,
            };
            let output = nav.steer(
                &NavAgent::new(id, position, velocity),
                goal,
                &neighbors,
                &params,
            );

            // 4. Deterministic fixed-step integration with acceleration and speed clamps.
            let mut vx = output.desired_velocity.0;
            let mut vz = output.desired_velocity.2;
            let dvx = vx - velocity.0;
            let dvz = vz - velocity.2;
            let dv_len = (dvx * dvx + dvz * dvz).sqrt();
            let max_dv = archetype.acceleration * dt;
            if dv_len > max_dv && dv_len > 1.0e-5 {
                let scale = max_dv / dv_len;
                vx = velocity.0 + dvx * scale;
                vz = velocity.2 + dvz * scale;
            }
            let speed_sq = vx * vx + vz * vz;
            let max_speed = effective_speed;
            if speed_sq > max_speed * max_speed && speed_sq > 1.0e-10 {
                let scale = max_speed / speed_sq.sqrt();
                vx *= scale;
                vz *= scale;
            }
            if speed_mult == 0.0 {
                vx = 0.0;
                vz = 0.0;
            }

            let new_velocity = (vx, 0.0, vz);
            let new_position = (position.0 + vx * dt, position.1, position.2 + vz * dt);
            let new_facing = match heading_deg(new_velocity, FACING_MIN_SPEED) {
                Some(target_deg) => {
                    turn_toward_deg(facing_deg, target_deg, archetype.turn_rate_deg * dt)
                }
                None => facing_deg,
            };

            if let Some(robot) = self.robots.get_mut(&id) {
                robot.velocity = new_velocity;
                robot.position = new_position;
                robot.facing_deg = new_facing;
                if output.arrived && !robot.at_goal {
                    robot.at_goal = true;
                    arrivals.push((id, new_position));
                } else if !output.arrived {
                    robot.at_goal = false;
                }

                // Advance status timers and apply DoT damage
                let dot_damage = robot.status.tick();
                if dot_damage > 0.0 {
                    robot.current_hp = robot.current_hp.saturating_sub(dot_damage.ceil() as u32);
                }

                // Advance weapon cycle / reload timer
                if let Some(ref mut weapon) = robot.weapon {
                    weapon.tick();
                }
            }
        }

        // 5. Authoritative combat actions (weapons discharge, melee strikes, Charger ram)
        let mut direct_attacks: Vec<(EntityId, DamageSpec)> = Vec::new();
        let mut spawned_projectiles: Vec<Projectile> = Vec::new();

        for &id in &ids {
            let (target, is_charger, body_radius, turn_rate, mass_kg) = match self.robots.get(&id) {
                Some(r) => {
                    if r.current_hp == 0 {
                        continue;
                    }
                    match r.order {
                        RobotOrder::Attack { target } => (
                            target,
                            r.chassis == RobotChassis::Charger,
                            r.archetype().body_radius,
                            r.archetype().turn_rate_deg,
                            r.archetype().mass_kg,
                        ),
                        _ => continue,
                    }
                }
                None => continue,
            };

            let target_pos = match self.position_of(target) {
                Some(pos) => pos,
                None => continue,
            };

            let robot = match self.robots.get_mut(&id) {
                Some(r) => r,
                None => continue,
            };

            let dist = planar_distance(robot.position, target_pos);
            let dx = target_pos.0 - robot.position.0;
            let dz = target_pos.2 - robot.position.2;

            // Orient toward target if not moving fast
            if planar_length(robot.velocity) < FACING_MIN_SPEED && (dx * dx + dz * dz) > 0.001 {
                let aim_heading = dx.atan2(dz).to_degrees().rem_euclid(360.0);
                robot.facing_deg = turn_toward_deg(robot.facing_deg, aim_heading, turn_rate * dt);
            }

            // Special: Charger high-mass kinetic ram collision
            if is_charger {
                let speed = planar_length(robot.velocity);
                if speed > 2.0 && dist <= body_radius + 1.5 {
                    let impact =
                        DamageSpec::new_impact(mass_kg, speed, 0.5).with_source(robot.entity);
                    direct_attacks.push((target, impact));
                }
            }

            let Some(ref mut weapon) = robot.weapon else {
                continue;
            };

            if dist <= weapon.def.range && weapon.can_fire() && weapon.discharge(1000) {
                match weapon.def.motion {
                    MotionPrimitive::Linear { speed, max_range } => {
                        let (dir_x, dir_z) = if dist > 0.001 {
                            (dx / dist, dz / dist)
                        } else {
                            (0.0, 1.0)
                        };
                        let mut damage = weapon.def.base_damage;
                        damage.source = Some(robot.entity);
                        spawned_projectiles.push(Projectile::new_linear(
                            crate::combat::LinearProjectileSpec {
                                id: ProjectileId(0),
                                owner: Some(robot.entity),
                                faction_id: robot.faction_id,
                                origin: robot.position,
                                direction: (dir_x, 0.0, dir_z),
                                speed,
                                max_range,
                                damage,
                                splash_radius: weapon.def.splash_radius,
                                spawn_tick: tick,
                            },
                        ));
                    }
                    MotionPrimitive::Ballistic { gravity, .. } => {
                        let t_flight = 1.0f32.max(dist / 25.0);
                        let vy = 0.5 * gravity * t_flight;
                        let vx = dx / t_flight;
                        let vz = dz / t_flight;
                        let mut damage = weapon.def.base_damage;
                        damage.source = Some(robot.entity);
                        spawned_projectiles.push(Projectile::new_ballistic(
                            crate::combat::BallisticProjectileSpec {
                                id: ProjectileId(0),
                                owner: Some(robot.entity),
                                faction_id: robot.faction_id,
                                origin: robot.position,
                                initial_velocity: (vx, vy, vz),
                                gravity,
                                max_range: weapon.def.range,
                                damage,
                                splash_radius: weapon.def.splash_radius,
                                spawn_tick: tick,
                                max_flight_ticks: (t_flight * 30.0).ceil() as u64 + 10,
                            },
                        ));
                    }
                    MotionPrimitive::PhysicalMelee { reach }
                        if dist <= reach + body_radius + 0.5 =>
                    {
                        let mut damage = weapon.def.base_damage;
                        damage.source = Some(robot.entity);
                        direct_attacks.push((target, damage));
                    }
                    _ => {}
                }
            }
        }

        for (robot, position) in arrivals {
            journal.record(tick, SimEvent::RobotArrived { robot, position });
        }

        for proj in spawned_projectiles {
            self.pending_projectiles.push(proj);
        }

        for (target, damage) in direct_attacks {
            let _ = self.apply_damage(target, damage, tick, journal);
        }

        // Clean up robots destroyed by DoT effects
        let dead_robots: Vec<EntityId> = self
            .robots
            .iter()
            .filter(|(_, r)| r.current_hp == 0)
            .map(|(id, _)| *id)
            .collect();
        for dead in dead_robots {
            self.cleanup_destroyed_robot(dead, None, tick, journal);
        }
    }
}

/// Uniform grid cell coordinate for a world position.
fn grid_cell(position: (f32, f32, f32), cell_size: f32) -> (i32, i32) {
    (
        (position.0 / cell_size).floor() as i32,
        (position.2 / cell_size).floor() as i32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAYER_A: PlayerId = PlayerId::new(1);
    const PLAYER_B: PlayerId = PlayerId::new(2);
    const FACTION: FactionId = FactionId::new(1);
    const REGION: RegionId = RegionId::new(1);

    struct Fixture {
        registry: RobotRegistry,
        journal: EventJournal,
        next_entity: u64,
    }

    impl Fixture {
        fn new() -> Self {
            Fixture {
                registry: RobotRegistry::new(),
                journal: EventJournal::new(),
                next_entity: 1,
            }
        }

        fn entity(&mut self) -> EntityId {
            let id = EntityId::new(self.next_entity);
            self.next_entity += 1;
            id
        }

        fn add_player(&mut self, player: PlayerId, position: (f32, f32, f32)) -> EntityId {
            let entity = self.entity();
            self.registry
                .register_player(PlayerPresence::new(
                    player, entity, FACTION, REGION, position,
                ))
                .unwrap();
            entity
        }

        fn add_robot(&mut self, chassis: RobotChassis, position: (f32, f32, f32)) -> EntityId {
            let entity = self.entity();
            self.registry
                .spawn_robot_with_journal(
                    RobotSpawnRequest::new(
                        entity,
                        chassis,
                        FACTION,
                        REGION,
                        position,
                        SimTick::zero(),
                    ),
                    &mut self.journal,
                )
                .unwrap();
            entity
        }

        fn run(&mut self, ticks: u64, start: u64) {
            for t in 0..ticks {
                self.registry
                    .step(SimTick::new(start + t), &mut self.journal);
            }
        }
    }

    #[test]
    fn test_robot_health_and_armor_reuse_wall_damage_model() {
        let mut fx = Fixture::new();
        let robot = fx.add_robot(RobotChassis::Guardsman, (0.0, 0.0, 0.0));

        // Medium armor: (200 - 10) * (1 - 0.15) = 161.5 -> 162 rounded.
        let result = fx
            .registry
            .apply_damage(
                robot,
                DamageSpec::new(200.0),
                SimTick::new(1),
                &mut fx.journal,
            )
            .unwrap();
        assert_eq!(result.absorbed_armor, 10.0);
        assert!((result.effective_damage - 161.5).abs() < 1e-3);
        assert_eq!(result.remaining_hp, 900 - 162);
        assert!(!result.destroyed);
        assert_eq!(fx.registry.get(robot).unwrap().current_hp, 738);

        // Armor penetration bypasses flat armor exactly as it does for walls.
        let pierced = fx
            .registry
            .apply_damage(
                robot,
                DamageSpec::new(100.0).with_penetration(10.0),
                SimTick::new(2),
                &mut fx.journal,
            )
            .unwrap();
        assert_eq!(pierced.absorbed_armor, 0.0);
        assert!((pierced.effective_damage - 85.0).abs() < 1e-3);
    }

    #[test]
    fn test_lethal_damage_removes_robot_and_frees_escort_slot() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        let robot = fx.add_robot(RobotChassis::Guardsman, (2.0, 0.0, 0.0));
        fx.registry
            .assign_escort(PLAYER_A, PLAYER_A, robot, SimTick::new(1), &mut fx.journal)
            .unwrap();
        assert_eq!(fx.registry.escorts_for(PLAYER_A).len(), 1);

        let result = fx
            .registry
            .apply_damage(
                robot,
                DamageSpec::new(5000.0),
                SimTick::new(2),
                &mut fx.journal,
            )
            .unwrap();
        assert!(result.destroyed);
        assert!(!fx.registry.contains(robot));
        assert!(fx.registry.escorts_for(PLAYER_A).is_empty());

        // The freed slot can be filled again immediately.
        let replacement = fx.add_robot(RobotChassis::Guardsman, (3.0, 0.0, 0.0));
        fx.registry
            .assign_escort(
                PLAYER_A,
                PLAYER_A,
                replacement,
                SimTick::new(3),
                &mut fx.journal,
            )
            .unwrap();
        assert_eq!(fx.registry.escorts_for(PLAYER_A), &[replacement]);
    }

    /// ACCEPTANCE: Guardsman follows the player without blocking their movement excessively.
    #[test]
    fn test_acceptance_guardsman_follows_player_without_blocking_movement() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        let guardsman = fx.add_robot(RobotChassis::Guardsman, (-12.0, 0.0, 0.0));
        fx.registry
            .assign_escort(
                PLAYER_A,
                PLAYER_A,
                guardsman,
                SimTick::zero(),
                &mut fx.journal,
            )
            .unwrap();

        let standoff = RobotChassis::Guardsman.archetype().follow_standoff;
        let mut closest_approach = f32::MAX;
        let mut max_leash = 0.0f32;

        // The player walks 6 m/s along +X for 450 ticks (15 seconds at 30 Hz).
        for t in 1..=450u64 {
            let tick = SimTick::new(t);
            let px = 6.0 * (t as f32) * fx.registry.config.fixed_step_seconds;
            fx.registry
                .apply_player_move(PLAYER_A, (px, 0.0, 0.0), tick)
                .unwrap();
            fx.registry.step(tick, &mut fx.journal);

            if t > 200 {
                // After the initial catch-up, measure steady-state escort behaviour.
                let d = fx.registry.distance_to_target(guardsman).unwrap();
                closest_approach = closest_approach.min(d);
                max_leash = max_leash.max(d);
            }
        }

        assert_eq!(
            fx.registry.get(guardsman).unwrap().owner,
            Some(PLAYER_A),
            "escort assignment must persist across ticks"
        );
        // It keeps up: never falls outside a reasonable leash of the standoff distance.
        assert!(
            max_leash <= standoff + 2.0,
            "guardsman fell behind: max distance {max_leash}"
        );
        // It never crowds the player: the player is never blocked at point-blank range.
        assert!(
            closest_approach >= 1.5,
            "guardsman blocked the player: closest approach {closest_approach}"
        );
        // And it holds station near the standoff distance rather than pressing in.
        assert!(
            closest_approach >= standoff - 1.5,
            "guardsman crowded inside standoff: {closest_approach}"
        );
    }

    #[test]
    fn test_separation_keeps_escort_from_standing_inside_stationary_player() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        // Spawn the escort effectively on top of the player.
        let guardsman = fx.add_robot(RobotChassis::Guardsman, (0.2, 0.0, 0.0));
        fx.registry
            .assign_escort(
                PLAYER_A,
                PLAYER_A,
                guardsman,
                SimTick::zero(),
                &mut fx.journal,
            )
            .unwrap();

        fx.run(150, 1);

        let distance = fx.registry.distance_to_target(guardsman).unwrap();
        assert!(
            distance >= 1.2,
            "escort must push out of the player's body, distance {distance}"
        );
    }

    /// ACCEPTANCE: ownership/assignment survives server authority (no client forgery).
    #[test]
    fn test_acceptance_escort_assignment_cannot_be_forged_by_another_player() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        fx.add_player(PLAYER_B, (50.0, 0.0, 0.0));
        let robot = fx.add_robot(RobotChassis::Guardsman, (2.0, 0.0, 0.0));

        // Player B claims to be assigning an escort on Player A's behalf: rejected.
        let forged =
            fx.registry
                .assign_escort(PLAYER_B, PLAYER_A, robot, SimTick::new(1), &mut fx.journal);
        assert_eq!(forged, Err(GameError::PermissionDenied));
        assert!(fx.registry.escorts_for(PLAYER_A).is_empty());
        assert_eq!(fx.registry.get(robot).unwrap().owner, None);

        // Player A legitimately assigns it.
        fx.registry
            .assign_escort(PLAYER_A, PLAYER_A, robot, SimTick::new(2), &mut fx.journal)
            .unwrap();
        assert_eq!(fx.registry.get(robot).unwrap().owner, Some(PLAYER_A));

        // Player B cannot steal it, cannot command it, and cannot release it.
        assert_eq!(
            fx.registry
                .assign_escort(PLAYER_B, PLAYER_B, robot, SimTick::new(3), &mut fx.journal),
            Err(GameError::PermissionDenied)
        );
        assert_eq!(
            fx.registry.issue_order(
                PLAYER_B,
                robot,
                RobotOrder::MoveTo {
                    position: (999.0, 0.0, 999.0)
                },
                SimTick::new(4),
                &mut fx.journal
            ),
            Err(GameError::PermissionDenied)
        );
        assert_eq!(
            fx.registry
                .release_escort(PLAYER_B, PLAYER_A, robot, SimTick::new(5), &mut fx.journal),
            Err(GameError::PermissionDenied)
        );

        // Ownership and the follow order are untouched by the rejected attempts.
        let owned = fx.registry.get(robot).unwrap();
        assert_eq!(owned.owner, Some(PLAYER_A));
        assert!(matches!(owned.order, RobotOrder::Follow { .. }));
    }

    #[test]
    fn test_escort_ownership_survives_region_transfer_and_ticks() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        let robot = fx.add_robot(RobotChassis::Guardsman, (5.0, 0.0, 0.0));
        fx.registry
            .assign_escort(PLAYER_A, PLAYER_A, robot, SimTick::zero(), &mut fx.journal)
            .unwrap();

        fx.run(30, 1);
        let previous = fx
            .registry
            .transfer_region(robot, RegionId::new(7))
            .unwrap();
        assert_eq!(previous, REGION);
        fx.run(30, 31);

        let after = fx.registry.get(robot).unwrap();
        assert_eq!(after.region_id, RegionId::new(7));
        assert_eq!(after.owner, Some(PLAYER_A));
        assert!(matches!(after.order, RobotOrder::Follow { .. }));
        assert_eq!(fx.registry.escorts_for(PLAYER_A), &[robot]);
    }

    /// ACCEPTANCE: multiple players can each hold independent escorts, each capped.
    #[test]
    fn test_acceptance_multiple_players_hold_independent_escorts_with_cap() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        fx.add_player(PLAYER_B, (60.0, 0.0, 0.0));

        let a1 = fx.add_robot(RobotChassis::Guardsman, (3.0, 0.0, 0.0));
        let a2 = fx.add_robot(RobotChassis::Guardsman, (-3.0, 0.0, 0.0));
        let b1 = fx.add_robot(RobotChassis::Guardsman, (63.0, 0.0, 0.0));
        let spare = fx.add_robot(RobotChassis::Guardsman, (10.0, 0.0, 10.0));

        // Default progression cap is one escort per player.
        assert_eq!(fx.registry.escort_cap_for(PLAYER_A), 1);
        fx.registry
            .assign_escort(PLAYER_A, PLAYER_A, a1, SimTick::new(1), &mut fx.journal)
            .unwrap();
        assert_eq!(
            fx.registry
                .assign_escort(PLAYER_A, PLAYER_A, a2, SimTick::new(1), &mut fx.journal),
            Err(GameError::EscortCapExceeded {
                assigned: 1,
                cap: 1
            })
        );

        // Player B is completely unaffected by Player A's roster and cap.
        fx.registry
            .assign_escort(PLAYER_B, PLAYER_B, b1, SimTick::new(1), &mut fx.journal)
            .unwrap();
        assert_eq!(fx.registry.escorts_for(PLAYER_A), &[a1]);
        assert_eq!(fx.registry.escorts_for(PLAYER_B), &[b1]);

        // Progression raises Player A to the 2-escort ceiling; Player B stays at 1.
        assert_eq!(fx.registry.config.grant_escort_cap(PLAYER_A, 2), 2);
        fx.registry
            .assign_escort(PLAYER_A, PLAYER_A, a2, SimTick::new(2), &mut fx.journal)
            .unwrap();
        assert_eq!(fx.registry.escorts_for(PLAYER_A), &[a1, a2]);
        assert_eq!(
            fx.registry
                .assign_escort(PLAYER_A, PLAYER_A, spare, SimTick::new(3), &mut fx.journal),
            Err(GameError::EscortCapExceeded {
                assigned: 2,
                cap: 2
            })
        );
        assert_eq!(fx.registry.escort_cap_for(PLAYER_B), 1);

        // Both escort groups track their own owner simultaneously.
        for t in 1..=200u64 {
            let tick = SimTick::new(t);
            let step = fx.registry.config.fixed_step_seconds;
            let x = 5.0 * (t as f32) * step;
            fx.registry
                .apply_player_move(PLAYER_A, (x, 0.0, 0.0), tick)
                .unwrap();
            fx.registry
                .apply_player_move(PLAYER_B, (60.0, 0.0, x), tick)
                .unwrap();
            fx.registry.step(tick, &mut fx.journal);
        }

        let standoff = RobotChassis::Guardsman.archetype().follow_standoff + 2.5;
        for robot in [a1, a2] {
            let d = fx.registry.distance_to_target(robot).unwrap();
            assert!(d <= standoff, "player A escort {robot} lagged at {d}");
        }
        let db = fx.registry.distance_to_target(b1).unwrap();
        assert!(db <= standoff, "player B escort lagged at {db}");
    }

    #[test]
    fn test_escort_cap_ceiling_cannot_be_exceeded_by_progression_grant() {
        let mut config = RobotConfig::default();
        // A doctrine tries to grant 9 escorts; the configured ceiling wins.
        assert_eq!(config.grant_escort_cap(PLAYER_A, 9), 2);
        assert_eq!(config.escort_cap_for(PLAYER_A), 2);
        // Raising the ceiling is itself a config change, not a code change.
        config.max_escort_cap = 4;
        assert_eq!(config.grant_escort_cap(PLAYER_A, 9), 4);
    }

    #[test]
    fn test_release_escort_frees_slot_and_parks_robot_on_guard() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        let robot = fx.add_robot(RobotChassis::Guardsman, (4.0, 0.0, 0.0));
        fx.registry
            .assign_escort(PLAYER_A, PLAYER_A, robot, SimTick::new(1), &mut fx.journal)
            .unwrap();
        assert_eq!(
            fx.registry
                .assign_escort(PLAYER_A, PLAYER_A, robot, SimTick::new(1), &mut fx.journal),
            Err(GameError::EscortAlreadyAssigned(robot))
        );

        fx.registry
            .release_escort(PLAYER_A, PLAYER_A, robot, SimTick::new(2), &mut fx.journal)
            .unwrap();
        assert!(fx.registry.escorts_for(PLAYER_A).is_empty());
        let released = fx.registry.get(robot).unwrap();
        assert_eq!(released.owner, None);
        assert!(matches!(released.order, RobotOrder::Guard { .. }));
    }

    #[test]
    fn test_follow_guard_move_and_return_to_base_orders_execute() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        let robot = fx.add_robot(RobotChassis::Rifleman, (0.0, 0.0, 0.0));

        // Move: the robot walks to the commanded position and stops there.
        fx.registry
            .issue_order(
                PLAYER_A,
                robot,
                RobotOrder::MoveTo {
                    position: (30.0, 0.0, 0.0),
                },
                SimTick::new(1),
                &mut fx.journal,
            )
            .unwrap();
        fx.run(300, 2);
        let after_move = fx.registry.get(robot).unwrap();
        assert!(planar_distance(after_move.position, (30.0, 0.0, 0.0)) < 1.0);
        assert!(after_move.at_goal);

        // Guard: displaced robots walk back to their guard post.
        fx.registry
            .issue_order(
                PLAYER_A,
                robot,
                RobotOrder::Guard {
                    position: (30.0, 0.0, 30.0),
                },
                SimTick::new(400),
                &mut fx.journal,
            )
            .unwrap();
        fx.run(300, 401);
        assert!(planar_distance(fx.registry.get(robot).unwrap().position, (30.0, 0.0, 30.0)) < 1.0);

        // ReturnToBase: the robot walks back to its recorded home position.
        fx.registry
            .issue_order(
                PLAYER_A,
                robot,
                RobotOrder::ReturnToBase,
                SimTick::new(800),
                &mut fx.journal,
            )
            .unwrap();
        fx.run(400, 801);
        let home = fx.registry.get(robot).unwrap().home_position;
        assert!(planar_distance(fx.registry.get(robot).unwrap().position, home) < 1.0);

        // Follow: a non-escort robot can still be ordered to trail a player avatar.
        let player_entity = fx.registry.player(PLAYER_A).unwrap().entity;
        fx.registry
            .issue_order(
                PLAYER_A,
                robot,
                RobotOrder::Follow {
                    target: player_entity,
                    standoff: 5.0,
                },
                SimTick::new(1300),
                &mut fx.journal,
            )
            .unwrap();
        fx.run(300, 1301);
        let follow_distance = fx.registry.distance_to_target(robot).unwrap();
        assert!(
            (follow_distance - 5.0).abs() < 1.5,
            "follow standoff not held: {follow_distance}"
        );
    }

    #[test]
    fn test_squad_roster_leader_promotion_and_regroup_formation() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        let squad = fx
            .registry
            .create_squad(FACTION, SimTick::zero(), &mut fx.journal);

        let mut members = Vec::new();
        for i in 0..5 {
            let robot = fx.add_robot(RobotChassis::Rifleman, (i as f32 * 4.0, 0.0, 0.0));
            fx.registry
                .assign_squad_member(PLAYER_A, squad, robot, SimTick::new(1), &mut fx.journal)
                .unwrap();
            members.push(robot);
        }

        let roster = fx.registry.squad(squad).unwrap();
        assert_eq!(
            roster.members, members,
            "member order must be deterministic"
        );
        assert_eq!(roster.leader, members[0]);

        // Duplicate membership is rejected.
        assert_eq!(
            fx.registry.assign_squad_member(
                PLAYER_A,
                squad,
                members[0],
                SimTick::new(2),
                &mut fx.journal
            ),
            Err(GameError::InvalidStateTransition)
        );

        // Regroup assigns every member a deterministic formation slot around the rally.
        let rally = (100.0, 0.0, 100.0);
        let count = fx
            .registry
            .regroup_squad(PLAYER_A, squad, rally, SimTick::new(3), &mut fx.journal)
            .unwrap();
        assert_eq!(count, 5);
        for &member in &members {
            assert_eq!(
                fx.registry.get(member).unwrap().order,
                RobotOrder::Regroup { squad_id: squad }
            );
        }

        fx.run(900, 4);
        let spacing = fx.registry.squad(squad).unwrap().formation_spacing;
        for (idx, &member) in members.iter().enumerate() {
            let slot = fx.registry.squad(squad).unwrap().slot_position(idx);
            let position = fx.registry.get(member).unwrap().position;
            assert!(
                planar_distance(position, slot) < 2.0,
                "member {idx} failed to reach its formation slot ({position:?} vs {slot:?})"
            );
            assert!(planar_distance(position, rally) <= spacing + 2.5);
        }

        // Removing the leader promotes the next member in roster order.
        fx.registry
            .remove_squad_member(
                PLAYER_A,
                squad,
                members[0],
                SimTick::new(1000),
                &mut fx.journal,
            )
            .unwrap();
        let roster = fx.registry.squad(squad).unwrap();
        assert_eq!(roster.leader, members[1]);
        assert_eq!(roster.len(), 4);
        assert_eq!(fx.registry.get(members[0]).unwrap().squad, None);
    }

    #[test]
    fn test_regroup_rejects_actor_who_does_not_own_every_member() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        fx.add_player(PLAYER_B, (10.0, 0.0, 0.0));
        let squad = fx
            .registry
            .create_squad(FACTION, SimTick::zero(), &mut fx.journal);

        let free = fx.add_robot(RobotChassis::Rifleman, (1.0, 0.0, 0.0));
        let owned_by_b = fx.add_robot(RobotChassis::Guardsman, (11.0, 0.0, 0.0));
        fx.registry
            .assign_squad_member(PLAYER_A, squad, free, SimTick::new(1), &mut fx.journal)
            .unwrap();
        fx.registry
            .assign_escort(
                PLAYER_B,
                PLAYER_B,
                owned_by_b,
                SimTick::new(1),
                &mut fx.journal,
            )
            .unwrap();
        fx.registry
            .assign_squad_member(
                PLAYER_B,
                squad,
                owned_by_b,
                SimTick::new(2),
                &mut fx.journal,
            )
            .unwrap();

        // Player A cannot regroup a squad containing Player B's escort.
        assert_eq!(
            fx.registry.regroup_squad(
                PLAYER_A,
                squad,
                (50.0, 0.0, 50.0),
                SimTick::new(3),
                &mut fx.journal
            ),
            Err(GameError::PermissionDenied)
        );
        // The rejection is atomic: no member received a partial regroup order.
        assert_eq!(
            fx.registry.get(free).unwrap().order.as_u8(),
            RobotOrder::Idle.as_u8()
        );
    }

    #[test]
    fn test_server_clamps_client_reported_player_teleport() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));

        // A client asserting a 10 km jump in one tick is clamped to its speed allowance.
        let accepted = fx
            .registry
            .apply_player_move(PLAYER_A, (10_000.0, 0.0, 0.0), SimTick::new(1))
            .unwrap();
        let allowance = fx.registry.config.player_max_speed * fx.registry.config.fixed_step_seconds;
        assert!(
            accepted.0 <= allowance + 1e-3,
            "teleport was not clamped: accepted {accepted:?}"
        );
        assert_eq!(fx.registry.player(PLAYER_A).unwrap().position, accepted);

        // A legitimate within-budget move is accepted verbatim.
        let legit = fx
            .registry
            .apply_player_move(PLAYER_A, (accepted.0 + 0.2, 0.0, 0.0), SimTick::new(2))
            .unwrap();
        assert!((legit.0 - (accepted.0 + 0.2)).abs() < 1e-4);

        // Unknown players are rejected outright.
        assert_eq!(
            fx.registry
                .apply_player_move(PLAYER_B, (1.0, 0.0, 0.0), SimTick::new(3)),
            Err(GameError::PermissionDenied)
        );
    }

    #[test]
    fn test_movement_integration_respects_speed_and_turn_rate_limits() {
        let mut fx = Fixture::new();
        // Keep the player far away so only the chassis limits shape the trajectory.
        fx.add_player(PLAYER_A, (100.0, 0.0, 100.0));
        let robot = fx.add_robot(RobotChassis::Guardsman, (0.0, 0.0, 0.0));
        fx.registry
            .issue_order(
                PLAYER_A,
                robot,
                RobotOrder::MoveTo {
                    position: (0.0, 0.0, 500.0),
                },
                SimTick::new(1),
                &mut fx.journal,
            )
            .unwrap();

        let archetype = RobotChassis::Guardsman.archetype();
        let dt = fx.registry.config.fixed_step_seconds;
        let mut previous = fx.registry.get(robot).unwrap().position;
        for t in 2..=120u64 {
            fx.registry.step(SimTick::new(t), &mut fx.journal);
            let robot_ref = fx.registry.get(robot).unwrap();
            let moved = planar_distance(robot_ref.position, previous);
            assert!(
                moved <= archetype.move_speed * dt + 1e-3,
                "robot exceeded chassis speed: moved {moved} in one tick"
            );
            assert!(robot_ref.planar_speed() <= archetype.move_speed + 1e-3);
            previous = robot_ref.position;
        }

        // Facing converged on the direction of travel (+Z is 0 degrees).
        let facing = fx.registry.get(robot).unwrap().facing_deg;
        assert!(
            !(1.0..=359.0).contains(&facing),
            "facing did not settle: {facing}"
        );
    }

    /// ACCEPTANCE: robot simulation runs headless and deterministically.
    #[test]
    fn test_acceptance_robot_simulation_runs_headless_and_deterministically() {
        fn run() -> Vec<(EntityId, (f32, f32, f32), f32)> {
            let mut fx = Fixture::new();
            fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
            fx.add_player(PLAYER_B, (40.0, 0.0, 40.0));
            let squad = fx
                .registry
                .create_squad(FACTION, SimTick::zero(), &mut fx.journal);

            let escort_a = fx.add_robot(RobotChassis::Guardsman, (6.0, 0.0, 1.0));
            let escort_b = fx.add_robot(RobotChassis::Guardsman, (46.0, 0.0, 41.0));
            fx.registry
                .assign_escort(
                    PLAYER_A,
                    PLAYER_A,
                    escort_a,
                    SimTick::zero(),
                    &mut fx.journal,
                )
                .unwrap();
            fx.registry
                .assign_escort(
                    PLAYER_B,
                    PLAYER_B,
                    escort_b,
                    SimTick::zero(),
                    &mut fx.journal,
                )
                .unwrap();

            for i in 0..12 {
                let robot = fx.add_robot(RobotChassis::Rifleman, (i as f32 * 2.0, 0.0, -20.0));
                fx.registry
                    .assign_squad_member(PLAYER_A, squad, robot, SimTick::zero(), &mut fx.journal)
                    .unwrap();
            }
            fx.registry
                .regroup_squad(
                    PLAYER_A,
                    squad,
                    (20.0, 0.0, -40.0),
                    SimTick::zero(),
                    &mut fx.journal,
                )
                .unwrap();

            for t in 1..=240u64 {
                let tick = SimTick::new(t);
                let x = 4.0 * (t as f32) * fx.registry.config.fixed_step_seconds;
                fx.registry
                    .apply_player_move(PLAYER_A, (x, 0.0, 0.0), tick)
                    .unwrap();
                fx.registry
                    .apply_player_move(PLAYER_B, (40.0 + x, 0.0, 40.0), tick)
                    .unwrap();
                fx.registry.step(tick, &mut fx.journal);
            }

            fx.registry
                .iter()
                .map(|r| (r.entity, r.position, r.facing_deg))
                .collect()
        }

        let first = run();
        let second = run();
        assert_eq!(first.len(), 14);
        assert_eq!(
            first, second,
            "headless robot simulation must be bit-identical across runs"
        );
    }

    #[test]
    fn test_local_separation_disperses_a_crowded_spawn_stack() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        let mut robots = Vec::new();
        for i in 0..8 {
            // Spawn eight bipeds nearly on top of each other.
            let robot = fx.add_robot(
                RobotChassis::Rifleman,
                (0.1 * i as f32, 0.0, 0.05 * i as f32),
            );
            robots.push(robot);
        }

        fx.run(150, 1);

        let mut min_spacing = f32::MAX;
        for (i, &a) in robots.iter().enumerate() {
            for &b in &robots[i + 1..] {
                let pa = fx.registry.get(a).unwrap().position;
                let pb = fx.registry.get(b).unwrap().position;
                min_spacing = min_spacing.min(planar_distance(pa, pb));
            }
        }
        assert!(
            min_spacing > 0.8,
            "separation failed to disperse the stack: min spacing {min_spacing}"
        );
    }

    #[test]
    fn test_session_to_player_mapping_is_server_derived() {
        assert_eq!(player_for_session(SessionId::new(1)), PlayerId::new(1));
        assert_eq!(player_for_session(SessionId::new(77)), PlayerId::new(77));
        assert!(player_for_session(SessionId::null()).is_null());
    }

    #[test]
    fn test_orders_against_unknown_robots_and_squads_are_rejected() {
        let mut fx = Fixture::new();
        fx.add_player(PLAYER_A, (0.0, 0.0, 0.0));
        let ghost = EntityId::new(9999);
        assert_eq!(
            fx.registry.issue_order(
                PLAYER_A,
                ghost,
                RobotOrder::ReturnToBase,
                SimTick::new(1),
                &mut fx.journal
            ),
            Err(GameError::RobotNotFound(ghost))
        );
        assert_eq!(
            fx.registry.regroup_squad(
                PLAYER_A,
                SquadId::new(42),
                (0.0, 0.0, 0.0),
                SimTick::new(1),
                &mut fx.journal
            ),
            Err(GameError::SquadNotFound(SquadId::new(42)))
        );
        assert_eq!(
            fx.registry
                .assign_escort(PLAYER_A, PLAYER_A, ghost, SimTick::new(1), &mut fx.journal),
            Err(GameError::RobotNotFound(ghost))
        );
    }
}
