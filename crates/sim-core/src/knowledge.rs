//! Authoritative sensor coverage, faction knowledge, fog-of-war, and last-seen structure ghosts.
//!
//! Milestone 14 establishes the simulation's authoritative information model:
//! 1. **Sensor Coverage**: Living robots, operational powered structures, and commander
//!    avatars project spherical sensor fields scaled by research modifiers.
//! 2. **Fog-of-War Grid**: A deterministic 2D grid partitioned into three states:
//!    - [`KnowledgeState::Unexplored`]: Shroud. Never seen, pitch black.
//!    - [`KnowledgeState::Explored`]: Fog of war. Previously scouted, terrain and structure
//!      ghosts remembered, mobile enemy units hidden.
//!    - [`KnowledgeState::Visible`]: Active vision. Real-time authoritative sensor contact.
//! 3. **Structure Ghosts**: Enemy structures spotted in line of sight persist as last-seen
//!    ghost snapshots until friendly sensors revisit the site. If the structure is destroyed
//!    while in fog, the ghost only vanishes once revisited by friendly sensors.
//! 4. **Authoritative Replication Interest**: Sessions only receive entity updates for
//!    entities their faction knows (preventing map hacks and wallhacks over the wire).

use crate::entity::EntityRegistry;
use crate::event::{EventJournal, SimEvent};
use crate::modifier::ModifierKind;
use crate::research::ResearchManager;
use crate::robot::RobotRegistry;
use crate::structure::{StructureKind, StructureRegistry, StructureState};
use game_types::{EntityId, FactionId, SimTick, StructureId};
use std::collections::{BTreeMap, BTreeSet};

/// World spatial bounds for fog evaluation matching the engine's standard terrain bounds.
pub const FOG_BOUNDS_MIN_X: f32 = -500.0;
pub const FOG_BOUNDS_MAX_X: f32 = 500.0;
pub const FOG_BOUNDS_MIN_Z: f32 = -500.0;
pub const FOG_BOUNDS_MAX_Z: f32 = 500.0;

/// Spatial resolution of each fog grid cell in meters.
pub const FOG_CELL_SIZE: f32 = 10.0;

/// Dimension of the square fog grid along X (columns).
pub const FOG_GRID_WIDTH: usize = 100;

/// Dimension of the square fog grid along Z (rows).
pub const FOG_GRID_HEIGHT: usize = 100;

/// Total number of cells per faction fog grid.
pub const FOG_TOTAL_CELLS: usize = FOG_GRID_WIDTH * FOG_GRID_HEIGHT;

/// Authoritative classification of faction spatial knowledge for a world cell.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
#[repr(u8)]
pub enum KnowledgeState {
    /// Pitch-black shroud: cell has never been observed by friendly sensors.
    #[default]
    Unexplored = 0,
    /// Fog of war: cell was explored in the past; terrain and static landmarks
    /// are remembered, but dynamic hostile entities are completely hidden.
    Explored = 1,
    /// Active vision: cell is currently illuminated by an active friendly sensor emitter.
    Visible = 2,
}

impl KnowledgeState {
    pub const fn is_visible(&self) -> bool {
        matches!(self, KnowledgeState::Visible)
    }

    pub const fn is_explored(&self) -> bool {
        matches!(self, KnowledgeState::Explored | KnowledgeState::Visible)
    }
}

/// Last-seen snapshot of an enemy structure preserved in fog of war.
#[derive(Clone, Debug, PartialEq)]
pub struct StructureGhost {
    pub id: StructureId,
    pub kind: StructureKind,
    pub faction_id: FactionId,
    pub position: (f32, f32, f32),
    pub bounds_min: (f32, f32, f32),
    pub bounds_max: (f32, f32, f32),
    pub last_seen_hp: u32,
    pub last_seen_tick: SimTick,
}

/// Active sensor emitter projecting a detection sphere into the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SensorSource {
    pub position: (f32, f32, f32),
    pub radius: f32,
}

/// Authoritative 2D fog-of-war grid for a single faction.
#[derive(Clone, Debug, PartialEq)]
pub struct FogGrid {
    pub cells: Vec<KnowledgeState>,
}

impl Default for FogGrid {
    fn default() -> Self {
        Self::new()
    }
}

impl FogGrid {
    /// Creates a fresh grid covered completely in pitch-black shroud.
    pub fn new() -> Self {
        FogGrid {
            cells: vec![KnowledgeState::Unexplored; FOG_TOTAL_CELLS],
        }
    }

    /// Linear cell index from grid coordinates `(cx, cz)`.
    #[inline]
    pub const fn index(cx: usize, cz: usize) -> usize {
        cz * FOG_GRID_WIDTH + cx
    }

    /// Converts world XZ coordinates to grid cell coordinates `(cx, cz)`.
    pub fn world_to_cell(x: f32, z: f32) -> Option<(usize, usize)> {
        if !(FOG_BOUNDS_MIN_X..FOG_BOUNDS_MAX_X).contains(&x)
            || !(FOG_BOUNDS_MIN_Z..FOG_BOUNDS_MAX_Z).contains(&z)
        {
            return None;
        }
        let cx = ((x - FOG_BOUNDS_MIN_X) / FOG_CELL_SIZE).floor() as usize;
        let cz = ((z - FOG_BOUNDS_MIN_Z) / FOG_CELL_SIZE).floor() as usize;
        if cx < FOG_GRID_WIDTH && cz < FOG_GRID_HEIGHT {
            Some((cx, cz))
        } else {
            None
        }
    }

    /// Converts cell coordinates `(cx, cz)` to world center coordinates `(x, z)`.
    pub fn cell_to_world_center(cx: usize, cz: usize) -> (f32, f32) {
        let x = FOG_BOUNDS_MIN_X + (cx as f32 + 0.5) * FOG_CELL_SIZE;
        let z = FOG_BOUNDS_MIN_Z + (cz as f32 + 0.5) * FOG_CELL_SIZE;
        (x, z)
    }

    /// Begins a new simulation tick by transitioning previous `Visible` cells to `Explored`.
    pub fn begin_tick(&mut self) {
        for state in &mut self.cells {
            if *state == KnowledgeState::Visible {
                *state = KnowledgeState::Explored;
            }
        }
    }

    /// Rasterizes an active sensor circular footprint into the grid, marking covered cells `Visible`.
    pub fn stamp_sensor(&mut self, center_x: f32, center_z: f32, radius: f32) {
        if radius <= 0.0 {
            return;
        }
        let min_x = (center_x - radius).max(FOG_BOUNDS_MIN_X);
        let max_x = (center_x + radius).min(FOG_BOUNDS_MAX_X - 0.001);
        let min_z = (center_z - radius).max(FOG_BOUNDS_MIN_Z);
        let max_z = (center_z + radius).min(FOG_BOUNDS_MAX_Z - 0.001);

        if min_x > max_x || min_z > max_z {
            return;
        }

        let start_cx = ((min_x - FOG_BOUNDS_MIN_X) / FOG_CELL_SIZE).floor() as usize;
        let end_cx = (((max_x - FOG_BOUNDS_MIN_X) / FOG_CELL_SIZE).floor() as usize)
            .min(FOG_GRID_WIDTH.saturating_sub(1));
        let start_cz = ((min_z - FOG_BOUNDS_MIN_Z) / FOG_CELL_SIZE).floor() as usize;
        let end_cz = (((max_z - FOG_BOUNDS_MIN_Z) / FOG_CELL_SIZE).floor() as usize)
            .min(FOG_GRID_HEIGHT.saturating_sub(1));

        let r_sq = radius * radius;

        for cz in start_cz..=end_cz {
            for cx in start_cx..=end_cx {
                let (wx, wz) = Self::cell_to_world_center(cx, cz);
                let dx = wx - center_x;
                let dz = wz - center_z;
                if dx * dx + dz * dz <= r_sq {
                    let idx = Self::index(cx, cz);
                    self.cells[idx] = KnowledgeState::Visible;
                }
            }
        }
    }

    /// Reads the knowledge state of a specific grid cell.
    pub fn cell_state(&self, cx: usize, cz: usize) -> KnowledgeState {
        if cx < FOG_GRID_WIDTH && cz < FOG_GRID_HEIGHT {
            self.cells[Self::index(cx, cz)]
        } else {
            KnowledgeState::Unexplored
        }
    }

    /// Reads the knowledge state at an arbitrary world position `(x, z)`.
    pub fn position_state(&self, x: f32, z: f32) -> KnowledgeState {
        match Self::world_to_cell(x, z) {
            Some((cx, cz)) => self.cell_state(cx, cz),
            None => KnowledgeState::Unexplored,
        }
    }

    pub fn is_position_visible(&self, x: f32, z: f32) -> bool {
        self.position_state(x, z).is_visible()
    }

    pub fn is_position_explored(&self, x: f32, z: f32) -> bool {
        self.position_state(x, z).is_explored()
    }
}

/// Faction-scoped knowledge container holding fog state, visible entity tracking, and structure ghosts.
#[derive(Clone, Debug, PartialEq)]
pub struct FactionKnowledge {
    pub faction_id: FactionId,
    pub fog_grid: FogGrid,
    pub visible_entities: BTreeSet<EntityId>,
    pub previous_visible_entities: BTreeSet<EntityId>,
    pub ghost_structures: BTreeMap<StructureId, StructureGhost>,
    pub active_sensors: Vec<SensorSource>,
}

impl FactionKnowledge {
    pub fn new(faction_id: FactionId) -> Self {
        FactionKnowledge {
            faction_id,
            fog_grid: FogGrid::new(),
            visible_entities: BTreeSet::new(),
            previous_visible_entities: BTreeSet::new(),
            ghost_structures: BTreeMap::new(),
            active_sensors: Vec::new(),
        }
    }
}

/// Master coordinator managing faction knowledge and sensor illumination across the world.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct KnowledgeManager {
    pub factions: BTreeMap<FactionId, FactionKnowledge>,
}

impl KnowledgeManager {
    pub fn new() -> Self {
        KnowledgeManager {
            factions: BTreeMap::new(),
        }
    }

    /// Ensures a knowledge entry exists for the given faction and returns a mutable reference.
    pub fn get_or_create(&mut self, faction_id: FactionId) -> &mut FactionKnowledge {
        self.factions
            .entry(faction_id)
            .or_insert_with(|| FactionKnowledge::new(faction_id))
    }

    /// Read-only access to a faction's knowledge state.
    pub fn get(&self, faction_id: FactionId) -> Option<&FactionKnowledge> {
        self.factions.get(&faction_id)
    }

    /// Authoritatively tests if `faction` currently has valid knowledge of `entity`.
    ///
    /// Returns `true` if:
    /// - `faction` is null (spectator / admin mode),
    /// - The entity is owned by `faction` or is neutral world property,
    /// - The entity is currently within active sensor visibility of `faction`,
    /// - Or the entity ID addresses a remembered structure ghost in fog of war.
    pub fn faction_knows_entity(
        &self,
        faction: FactionId,
        entity: EntityId,
        entity_registry: &EntityRegistry,
        structure_registry: &StructureRegistry,
    ) -> bool {
        if faction.is_null() {
            return true;
        }

        // 1. If this is a registered entity in EntityRegistry:
        if let Some(e) = entity_registry.get(entity) {
            if e.faction_id == faction || e.faction_id.is_null() {
                return true;
            }
            if let Some(fk) = self.factions.get(&faction) {
                return fk.visible_entities.contains(&entity);
            }
            return false;
        }

        // 2. Otherwise check if it addresses a structure:
        self.faction_knows_structure(faction, StructureId::new(entity.0), structure_registry)
    }

    /// Authoritatively tests if `faction` knows about a structure (own, neutral, active visible, or remembered ghost).
    pub fn faction_knows_structure(
        &self,
        faction: FactionId,
        structure: StructureId,
        structure_registry: &StructureRegistry,
    ) -> bool {
        if faction.is_null() {
            return true;
        }
        if let Some(s) = structure_registry.get(structure)
            && (s.faction_id == faction || s.faction_id.is_null())
        {
            return true;
        }
        if let Some(fk) = self.factions.get(&faction) {
            if fk.ghost_structures.contains_key(&structure) {
                return true;
            }
            let struct_ent = EntityId::new(structure.0);
            if fk.visible_entities.contains(&struct_ent) {
                return true;
            }
        }
        false
    }

    /// Whether world position `(x, z)` is actively illuminated by `faction`'s sensors.
    pub fn is_position_visible(&self, faction: FactionId, x: f32, z: f32) -> bool {
        if faction.is_null() {
            return true;
        }
        self.factions
            .get(&faction)
            .map(|fk| fk.fog_grid.is_position_visible(x, z))
            .unwrap_or(false)
    }

    /// Whether world position `(x, z)` has ever been explored by `faction`.
    pub fn is_position_explored(&self, faction: FactionId, x: f32, z: f32) -> bool {
        if faction.is_null() {
            return true;
        }
        self.factions
            .get(&faction)
            .map(|fk| fk.fog_grid.is_position_explored(x, z))
            .unwrap_or(false)
    }

    /// Advances authoritative sensor detection and fog state for all factions by one tick.
    pub fn step(
        &mut self,
        tick: SimTick,
        robots: &RobotRegistry,
        structures: &StructureRegistry,
        entities: &EntityRegistry,
        research: &ResearchManager,
        journal: &mut EventJournal,
    ) {
        // Collect all factions present in the simulation
        let mut active_factions = BTreeSet::new();
        for e in entities.iter() {
            if !e.faction_id.is_null() {
                active_factions.insert(e.faction_id);
            }
        }
        for r in robots.robots.values() {
            if !r.faction_id.is_null() {
                active_factions.insert(r.faction_id);
            }
        }
        for p in robots.players.values() {
            if !p.faction_id.is_null() {
                active_factions.insert(p.faction_id);
            }
        }
        for s in structures.structures.values() {
            if !s.faction_id.is_null() {
                active_factions.insert(s.faction_id);
            }
        }
        for fid in self.factions.keys() {
            if !fid.is_null() {
                active_factions.insert(*fid);
            }
        }

        for faction_id in active_factions {
            let fk = self.get_or_create(faction_id);

            // Cycle tick state
            fk.previous_visible_entities = std::mem::take(&mut fk.visible_entities);
            fk.fog_grid.begin_tick();
            fk.active_sensors.clear();

            // Research multiplier for sensor range
            let mod_milli = research
                .modifiers
                .multiplier_milli(faction_id, ModifierKind::SensorRange);
            let sensor_mult = (mod_milli as f32) / 1000.0;

            // 1. Collect sensors from friendly living robots
            for robot in robots.robots.values() {
                if robot.faction_id == faction_id && robot.current_hp > 0 {
                    let base_r = robot.chassis.archetype().sensor_radius;
                    let r = base_r * sensor_mult;
                    fk.active_sensors.push(SensorSource {
                        position: robot.position,
                        radius: r,
                    });
                    fk.fog_grid
                        .stamp_sensor(robot.position.0, robot.position.2, r);
                }
            }

            // 2. Collect sensors from friendly operational structures
            for structure in structures.structures.values() {
                if structure.faction_id == faction_id && structure.emits_sensor_coverage() {
                    let base_r = structure.kind.sensor_radius();
                    let r = base_r * sensor_mult;
                    fk.active_sensors.push(SensorSource {
                        position: structure.position,
                        radius: r,
                    });
                    fk.fog_grid
                        .stamp_sensor(structure.position.0, structure.position.2, r);
                }
            }

            // 3. Collect sensors from friendly commander player avatars
            for player in robots.players.values() {
                if player.faction_id == faction_id {
                    let base_r = 45.0;
                    let r = base_r * sensor_mult;
                    fk.active_sensors.push(SensorSource {
                        position: player.position,
                        radius: r,
                    });
                    fk.fog_grid
                        .stamp_sensor(player.position.0, player.position.2, r);
                }
            }

            // Helper to check if a world position falls within any active sensor
            let in_sensor = |sensors: &[SensorSource], pos: (f32, f32, f32)| -> bool {
                for s in sensors {
                    let dx = pos.0 - s.position.0;
                    let dz = pos.2 - s.position.2;
                    if dx * dx + dz * dz <= s.radius * s.radius {
                        return true;
                    }
                }
                false
            };

            // 4. Test visibility of enemy robots
            for robot in robots.robots.values() {
                if robot.faction_id != faction_id
                    && robot.current_hp > 0
                    && (in_sensor(&fk.active_sensors, robot.position)
                        || fk
                            .fog_grid
                            .is_position_visible(robot.position.0, robot.position.2))
                {
                    fk.visible_entities.insert(robot.entity);
                    if !fk.previous_visible_entities.contains(&robot.entity) {
                        journal.record(
                            tick,
                            SimEvent::EntitySpotted {
                                observer_faction: faction_id,
                                target: robot.entity,
                                position: robot.position,
                            },
                        );
                    }
                }
            }

            // 5. Test visibility of enemy player avatars
            for player in robots.players.values() {
                if player.faction_id != faction_id
                    && (in_sensor(&fk.active_sensors, player.position)
                        || fk
                            .fog_grid
                            .is_position_visible(player.position.0, player.position.2))
                {
                    fk.visible_entities.insert(player.entity);
                    if !fk.previous_visible_entities.contains(&player.entity) {
                        journal.record(
                            tick,
                            SimEvent::EntitySpotted {
                                observer_faction: faction_id,
                                target: player.entity,
                                position: player.position,
                            },
                        );
                    }
                }
            }

            // 6. Test visibility of enemy structures and update ghosts
            for structure in structures.structures.values() {
                if structure.faction_id != faction_id && structure.state.is_active_or_reserved() {
                    let struct_ent = EntityId::new(structure.id.0);
                    if in_sensor(&fk.active_sensors, structure.position)
                        || fk
                            .fog_grid
                            .is_position_visible(structure.position.0, structure.position.2)
                    {
                        fk.visible_entities.insert(struct_ent);
                        if !fk.previous_visible_entities.contains(&struct_ent) {
                            journal.record(
                                tick,
                                SimEvent::EntitySpotted {
                                    observer_faction: faction_id,
                                    target: struct_ent,
                                    position: structure.position,
                                },
                            );
                        }

                        let hp = match structure.state {
                            StructureState::Constructed { current_hp, .. } => current_hp,
                            _ => 0,
                        };
                        fk.ghost_structures.insert(
                            structure.id,
                            StructureGhost {
                                id: structure.id,
                                kind: structure.kind,
                                faction_id: structure.faction_id,
                                position: structure.position,
                                bounds_min: structure.bounds_min,
                                bounds_max: structure.bounds_max,
                                last_seen_hp: hp,
                                last_seen_tick: tick,
                            },
                        );
                    }
                }
            }

            // 7. Validate and clean up structure ghosts: if a friendly sensor looks directly at
            // a ghost's site and the structure has been destroyed or removed, purge the ghost.
            let mut ghosts_to_remove = Vec::new();
            for (sid, ghost) in &fk.ghost_structures {
                let site_visible = in_sensor(&fk.active_sensors, ghost.position)
                    || fk
                        .fog_grid
                        .is_position_visible(ghost.position.0, ghost.position.2);
                if site_visible {
                    let is_alive = structures
                        .structures
                        .get(sid)
                        .map(|s| s.state.is_active_or_reserved())
                        .unwrap_or(false);
                    if !is_alive {
                        ghosts_to_remove.push(*sid);
                    }
                }
            }
            for sid in ghosts_to_remove {
                fk.ghost_structures.remove(&sid);
            }

            // 8. Record EntityLost for entities that slipped out of active sensor coverage
            for prev in &fk.previous_visible_entities {
                if !fk.visible_entities.contains(prev) {
                    journal.record(
                        tick,
                        SimEvent::EntityLost {
                            observer_faction: faction_id,
                            target: *prev,
                        },
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fog_grid_coordinates_and_indexing() {
        assert_eq!(
            FogGrid::world_to_cell(FOG_BOUNDS_MIN_X, FOG_BOUNDS_MIN_Z),
            Some((0, 0))
        );
        assert_eq!(FogGrid::world_to_cell(0.0, 0.0), Some((50, 50)));
        assert_eq!(FogGrid::world_to_cell(-600.0, 0.0), None);
        assert_eq!(FogGrid::world_to_cell(600.0, 0.0), None);

        let (wx, wz) = FogGrid::cell_to_world_center(50, 50);
        assert!((wx - 5.0).abs() < 1e-4);
        assert!((wz - 5.0).abs() < 1e-4);
    }

    #[test]
    fn test_fog_grid_stamping_and_persistence() {
        let mut grid = FogGrid::new();
        assert_eq!(grid.position_state(0.0, 0.0), KnowledgeState::Unexplored);

        grid.stamp_sensor(0.0, 0.0, 30.0);
        assert_eq!(grid.position_state(0.0, 0.0), KnowledgeState::Visible);
        assert_eq!(grid.position_state(25.0, 0.0), KnowledgeState::Visible);
        assert_eq!(
            grid.position_state(100.0, 100.0),
            KnowledgeState::Unexplored
        );

        // Advance tick: Visible becomes Explored
        grid.begin_tick();
        assert_eq!(grid.position_state(0.0, 0.0), KnowledgeState::Explored);
        assert_eq!(
            grid.position_state(100.0, 100.0),
            KnowledgeState::Unexplored
        );

        // Re-illuminating sets back to Visible
        grid.stamp_sensor(0.0, 0.0, 15.0);
        assert_eq!(grid.position_state(0.0, 0.0), KnowledgeState::Visible);
    }
}
