use game_types::{EntityId, FactionId, ResourceId, StructureId};
use sim_core::world::WorldState;

/// Authoritative world bounds used by the server command path.
///
/// Mirrors the bounds `game_protocol::server` passes to
/// `StructureRegistry::request_build`. Kept as a constant here so the detectors
/// clamp against exactly the same box the simulation does.
pub const DEFAULT_WORLD_BOUNDS_XZ: (f32, f32, f32, f32) = (-500.0, 500.0, -500.0, 500.0);

/// Answer to a sensor-knowledge query.
///
/// The third arm exists because faction knowledge is built in Milestone 14.
/// Until then honest detectors must say "I cannot know" rather than guess, so a
/// missing knowledge system can never produce a false accusation.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum KnowledgeQuery {
    /// The faction has a current or remembered sensor contact on the entity.
    Known,
    /// The faction provably has no knowledge of the entity.
    Unknown,
    /// No knowledge system is wired up yet; the question cannot be answered.
    Unavailable,
}

/// Read-only authoritative world facts the detectors compare client claims against.
///
/// This trait is the *only* thing the anti-cheat detectors know about the
/// simulation. It exists so that:
///
/// 1. gameplay crates never depend on `anti-cheat` (the dependency edge is
///    `anti-cheat -> sim-core`, never the reverse), and
/// 2. detectors can be unit-tested against a hand-built world without standing
///    up a full simulation.
///
/// Methods with a default implementation are **integration hooks for later
/// milestones**. Their defaults are deliberately inert: a detector that depends
/// on an unimplemented hook reports nothing rather than guessing.
pub trait WorldView {
    /// Authoritative `(min_x, max_x, min_z, max_z)` play area.
    fn world_bounds_xz(&self) -> (f32, f32, f32, f32) {
        DEFAULT_WORLD_BOUNDS_XZ
    }

    /// Whether an entity currently exists in the authoritative registry.
    fn entity_exists(&self, entity: EntityId) -> bool;

    /// Owning faction of an entity, if it exists.
    fn entity_faction(&self, entity: EntityId) -> Option<FactionId>;

    /// Owning faction of a structure, if it exists.
    fn structure_faction(&self, structure: StructureId) -> Option<FactionId>;

    /// Unreserved balance of `resource` held by `entity`, or `None` if the
    /// entity has no container at all.
    fn available_resource(&self, entity: EntityId, resource: ResourceId) -> Option<u32>;

    /// Total (reserved + unreserved) balance of `resource` held by `entity`.
    fn total_resource(&self, entity: EntityId, resource: ResourceId) -> Option<u32>;

    /// **Milestone 14 hook** — sensors, faction knowledge, fog and replication
    /// interest.
    ///
    /// The hidden-target detector calls this to decide whether a session is
    /// acting on an entity it should have no knowledge of (a wallhack tell).
    /// Until Milestone 14 lands a knowledge store, the default returns
    /// [`KnowledgeQuery::Unavailable`] and the detector stays silent.
    ///
    /// The Milestone 14 agent should override this to consult the faction
    /// knowledge store and return `Known` / `Unknown`.
    fn faction_knows_entity(&self, _faction: FactionId, _entity: EntityId) -> KnowledgeQuery {
        KnowledgeQuery::Unavailable
    }

    /// **Milestone 13 hook** — combat, weapons, damage, armor and projectiles.
    ///
    /// Minimum ticks between two discharges of the actor's equipped weapon.
    /// Until Milestone 13 lands weapon profiles, the default returns `None` and
    /// the fire-rate detector falls back to
    /// [`crate::detectors::DEFAULT_MIN_FIRE_INTERVAL_TICKS`], which is a
    /// deliberately generous floor that no real weapon may undercut.
    ///
    /// The Milestone 13 agent should override this to return the equipped
    /// weapon's authoritative cycle time.
    fn weapon_cooldown_ticks(&self, _actor: EntityId) -> Option<u64> {
        None
    }

    /// **Milestone 13 hook** — authoritative magazine / ammo-pool state.
    ///
    /// The ammo detector uses the container balance of
    /// [`crate::detectors::AMMO_RESOURCE`] today. Once Milestone 13 models
    /// per-weapon magazines, override this to return the loaded round count so
    /// the detector can catch a client firing on an empty magazine while a
    /// reserve pool is still full.
    fn loaded_ammo(&self, _actor: EntityId) -> Option<u32> {
        None
    }
}

/// Authoritative simulation state seen through the read-only anti-cheat lens.
///
/// Implemented here rather than in `sim-core` on purpose: the trait is owned by
/// `anti-cheat`, so `sim-core` gains no dependency and gameplay code is unaware
/// anti-cheat exists.
impl WorldView for WorldState {
    /// The authoritative play area, read from the simulation's own collision
    /// world rather than from a constant that could drift away from it.
    fn world_bounds_xz(&self) -> (f32, f32, f32, f32) {
        WorldState::world_bounds_xz(self)
    }

    fn entity_exists(&self, entity: EntityId) -> bool {
        self.entity_registry.contains(entity)
    }

    fn entity_faction(&self, entity: EntityId) -> Option<FactionId> {
        self.entity_registry.get(entity).map(|e| e.faction_id)
    }

    fn structure_faction(&self, structure: StructureId) -> Option<FactionId> {
        self.structure_registry.get(structure).map(|s| s.faction_id)
    }

    fn available_resource(&self, entity: EntityId, resource: ResourceId) -> Option<u32> {
        self.inventory_registry
            .get(entity)
            .map(|inv| inv.available_quantity(resource))
    }

    fn total_resource(&self, entity: EntityId, resource: ResourceId) -> Option<u32> {
        self.inventory_registry
            .get(entity)
            .map(|inv| inv.total_quantity(resource))
    }

    fn weapon_cooldown_ticks(&self, actor: EntityId) -> Option<u64> {
        if let Some(weapon) = self
            .robot_registry
            .robots
            .get(&actor)
            .and_then(|r| r.weapon.as_ref())
        {
            return Some(weapon.def.cycle_ticks as u64);
        }
        let struct_id = StructureId::new(actor.0);
        if let Some(weapon) = self
            .structure_registry
            .get(struct_id)
            .and_then(|s| s.weapon.as_ref())
        {
            return Some(weapon.def.cycle_ticks as u64);
        }
        None
    }

    fn loaded_ammo(&self, actor: EntityId) -> Option<u32> {
        if let Some(weapon) = self
            .robot_registry
            .robots
            .get(&actor)
            .and_then(|r| r.weapon.as_ref())
        {
            return Some(weapon.loaded_ammo);
        }
        let struct_id = StructureId::new(actor.0);
        if let Some(weapon) = self
            .structure_registry
            .get(struct_id)
            .and_then(|s| s.weapon.as_ref())
        {
            return Some(weapon.loaded_ammo);
        }
        None
    }
}

/// Hand-built world used by detector unit tests and by callers that have no
/// simulation available (for example the manifest-only handshake path).
#[derive(Clone, Default, Debug)]
pub struct StaticWorldView {
    pub bounds: Option<(f32, f32, f32, f32)>,
    pub entity_factions: std::collections::BTreeMap<EntityId, FactionId>,
    pub structure_factions: std::collections::BTreeMap<StructureId, FactionId>,
    pub balances: std::collections::BTreeMap<(EntityId, ResourceId), u32>,
    pub known_entities: std::collections::BTreeSet<(FactionId, EntityId)>,
    /// When true, `faction_knows_entity` answers definitively instead of
    /// returning [`KnowledgeQuery::Unavailable`]. Simulates a post-M14 world.
    pub knowledge_available: bool,
    pub weapon_cooldowns: std::collections::BTreeMap<EntityId, u64>,
}

impl StaticWorldView {
    pub fn new() -> Self {
        StaticWorldView::default()
    }

    pub fn with_entity(mut self, entity: EntityId, faction: FactionId) -> Self {
        self.entity_factions.insert(entity, faction);
        self
    }

    pub fn with_structure(mut self, structure: StructureId, faction: FactionId) -> Self {
        self.structure_factions.insert(structure, faction);
        self
    }

    pub fn with_balance(mut self, entity: EntityId, resource: ResourceId, amount: u32) -> Self {
        self.balances.insert((entity, resource), amount);
        self
    }

    pub fn with_knowledge(mut self, faction: FactionId, entity: EntityId) -> Self {
        self.knowledge_available = true;
        self.known_entities.insert((faction, entity));
        self
    }

    /// Enable definitive knowledge answers without registering any contact.
    pub fn with_knowledge_system(mut self) -> Self {
        self.knowledge_available = true;
        self
    }

    pub fn with_weapon_cooldown(mut self, entity: EntityId, ticks: u64) -> Self {
        self.weapon_cooldowns.insert(entity, ticks);
        self
    }
}

impl WorldView for StaticWorldView {
    fn world_bounds_xz(&self) -> (f32, f32, f32, f32) {
        self.bounds.unwrap_or(DEFAULT_WORLD_BOUNDS_XZ)
    }

    fn entity_exists(&self, entity: EntityId) -> bool {
        self.entity_factions.contains_key(&entity)
    }

    fn entity_faction(&self, entity: EntityId) -> Option<FactionId> {
        self.entity_factions.get(&entity).copied()
    }

    fn structure_faction(&self, structure: StructureId) -> Option<FactionId> {
        self.structure_factions.get(&structure).copied()
    }

    fn available_resource(&self, entity: EntityId, resource: ResourceId) -> Option<u32> {
        self.balances.get(&(entity, resource)).copied()
    }

    fn total_resource(&self, entity: EntityId, resource: ResourceId) -> Option<u32> {
        self.available_resource(entity, resource)
    }

    fn faction_knows_entity(&self, faction: FactionId, entity: EntityId) -> KnowledgeQuery {
        if !self.knowledge_available {
            return KnowledgeQuery::Unavailable;
        }
        if self.known_entities.contains(&(faction, entity)) {
            KnowledgeQuery::Known
        } else {
            KnowledgeQuery::Unknown
        }
    }

    fn weapon_cooldown_ticks(&self, actor: EntityId) -> Option<u64> {
        self.weapon_cooldowns.get(&actor).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_types::{RegionId, resource::RES_AMMO};
    use sim_core::inventory::ContainerKind;

    #[test]
    fn test_sim_state_world_view_reports_entity_ownership() {
        let mut sim = WorldState::new();
        let e = sim.create_entity(FactionId::new(3), RegionId::new(1));
        assert!(sim.entity_exists(e));
        assert_eq!(sim.entity_faction(e), Some(FactionId::new(3)));
        assert!(!sim.entity_exists(EntityId::new(9999)));
        assert_eq!(sim.entity_faction(EntityId::new(9999)), None);
    }

    #[test]
    fn test_sim_state_world_view_reports_resource_balances() {
        let mut sim = WorldState::new();
        let e = sim.create_entity(FactionId::new(1), RegionId::new(1));
        sim.create_container(e, ContainerKind::Backpack);
        if let Some(inv) = sim.inventory_mut(e) {
            inv.add(RES_AMMO, 25).unwrap();
        }
        assert_eq!(sim.available_resource(e, RES_AMMO), Some(25));
        assert_eq!(sim.total_resource(e, RES_AMMO), Some(25));
        // No container at all is distinguishable from an empty container.
        let bare = sim.create_entity(FactionId::new(1), RegionId::new(1));
        assert_eq!(sim.available_resource(bare, RES_AMMO), None);
    }

    #[test]
    fn test_knowledge_hook_defaults_to_unavailable_until_milestone_14() {
        let sim = WorldState::new();
        assert_eq!(
            sim.faction_knows_entity(FactionId::new(1), EntityId::new(1)),
            KnowledgeQuery::Unavailable
        );
    }

    #[test]
    fn test_weapon_hooks_default_to_none_until_milestone_13() {
        let sim = WorldState::new();
        assert_eq!(sim.weapon_cooldown_ticks(EntityId::new(1)), None);
        assert_eq!(sim.loaded_ammo(EntityId::new(1)), None);
    }

    #[test]
    fn test_milestone_13_weapon_hooks_return_robot_weapon_state() {
        let mut sim = WorldState::new();
        let entity = sim
            .spawn_robot(
                sim_core::RobotChassis::Rifleman,
                FactionId::new(1),
                RegionId::new(1),
                (0.0, 0.0, 0.0),
            )
            .unwrap();

        assert_eq!(sim.weapon_cooldown_ticks(entity), Some(6));
        assert_eq!(sim.loaded_ammo(entity), Some(30));
    }

    #[test]
    fn test_static_world_view_simulates_post_milestone_14_knowledge() {
        let w = StaticWorldView::new()
            .with_entity(EntityId::new(1), FactionId::new(2))
            .with_knowledge(FactionId::new(1), EntityId::new(1));
        assert_eq!(
            w.faction_knows_entity(FactionId::new(1), EntityId::new(1)),
            KnowledgeQuery::Known
        );
        assert_eq!(
            w.faction_knows_entity(FactionId::new(1), EntityId::new(2)),
            KnowledgeQuery::Unknown
        );
        assert_eq!(
            StaticWorldView::new().faction_knows_entity(FactionId::new(1), EntityId::new(1)),
            KnowledgeQuery::Unavailable
        );
    }
}
