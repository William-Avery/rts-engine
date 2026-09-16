use crate::combat::{WeaponDef, WeaponState};
use crate::event::{EventJournal, SimEvent};
use crate::inventory::Inventory;
use crate::logistics::{DepotLogistics, LogisticsDock, LogisticsManager};
use crate::modifier::{ModifierKind, ModifierStore};
use crate::power::{PowerNetwork, PowerNode, PowerPriority, PowerSpec, PowerStatus};
use crate::production::{FacilityKind, ProductionFacility, ResourceDeposit};
use crate::wall::{DamageResult, DamageSpec, RepairResult, WallTier, calculate_wall_damage};
use game_types::{
    DepositId, EntityId, FactionId, GameError, GameResult, RES_ADVANCED_COMPONENTS,
    RES_BASIC_COMPONENTS, RES_ENERGY_CELL, RES_STEEL, RES_STONE, RecipeId, RegionId, ResourceId,
    SimTick, StructureId, WeaponId,
};
use std::collections::BTreeMap;

/// Type classification for world structures.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum StructureKind {
    Wall(WallTier),
    Pylon,
    Depot,
    Turret,
    Fabricator,
    Generator,
    Battery,
    MiningDrill,
    Refinery,
    /// Laboratory running the faction research queue and distributing software patches.
    ResearchFacility,
}

impl StructureKind {
    pub const DEFAULT_WALL: Self = StructureKind::Wall(WallTier::Mk1Stone);

    /// Half extents (hx, hy, hz) in meters from structure center.
    pub fn half_extents(&self, rotation_deg: f32) -> (f32, f32, f32) {
        let (raw_x, raw_y, raw_z) = match self {
            StructureKind::Wall(_) => (1.0, 1.25, 0.25), // 2m wide, 2.5m tall, 0.5m thick
            StructureKind::Pylon => (0.75, 2.0, 0.75),   // 1.5m x 4.0m x 1.5m
            StructureKind::Depot => (3.0, 1.5, 3.0),     // 6m x 3m x 6m
            StructureKind::Turret => (1.0, 1.5, 1.0),    // 2m x 3m x 2m
            StructureKind::Fabricator => (4.0, 2.0, 4.0), // 8m x 4m x 8m
            StructureKind::Generator => (2.0, 1.5, 2.0), // 4m x 3m x 4m
            StructureKind::Battery => (1.5, 1.5, 1.5),   // 3m x 3m x 3m
            StructureKind::MiningDrill => (2.0, 2.0, 2.0), // 4m x 4m x 4m
            StructureKind::Refinery => (3.5, 3.0, 3.5),  // 7m x 6m x 7m
            StructureKind::ResearchFacility => (3.0, 2.5, 3.0), // 6m x 5m x 6m
        };

        // If rotated 90 or 270 degrees, swap X and Z extents
        let norm_rot = (rotation_deg.rem_euclid(360.0) / 90.0).round() as i32 % 4;
        if norm_rot == 1 || norm_rot == 3 {
            (raw_z, raw_y, raw_x)
        } else {
            (raw_x, raw_y, raw_z)
        }
    }

    pub fn max_health(&self) -> u32 {
        match self {
            StructureKind::Wall(tier) => tier.archetype().max_health,
            StructureKind::Pylon => 500,
            StructureKind::Depot => 2500,
            StructureKind::Turret => 1500,
            StructureKind::Fabricator => 3000,
            StructureKind::Generator => 2000,
            StructureKind::Battery => 1000,
            StructureKind::MiningDrill => 2000,
            StructureKind::Refinery => 3500,
            StructureKind::ResearchFacility => 2800,
        }
    }

    pub fn construction_ticks(&self) -> u32 {
        match self {
            StructureKind::Wall(tier) => tier.archetype().construction_ticks,
            StructureKind::Pylon => 30,             // 1.0s at 30 Hz
            StructureKind::Depot => 90,             // 3.0s at 30 Hz
            StructureKind::Turret => 60,            // 2.0s at 30 Hz
            StructureKind::Fabricator => 120,       // 4.0s at 30 Hz
            StructureKind::Generator => 60,         // 2.0s at 30 Hz
            StructureKind::Battery => 45,           // 1.5s at 30 Hz
            StructureKind::MiningDrill => 90,       // 3.0s at 30 Hz
            StructureKind::Refinery => 120,         // 4.0s at 30 Hz
            StructureKind::ResearchFacility => 150, // 5.0s at 30 Hz
        }
    }

    pub fn dismantle_ticks(&self) -> u32 {
        match self {
            StructureKind::Wall(tier) => tier.archetype().dismantle_ticks,
            _ => (self.construction_ticks() / 2).max(5),
        }
    }

    pub fn construction_cost(&self) -> &'static [(ResourceId, u32)] {
        match self {
            StructureKind::Wall(tier) => tier.archetype().construction_cost,
            StructureKind::Pylon => &[(RES_STEEL, 10)],
            StructureKind::Depot => &[(RES_STEEL, 30), (RES_STONE, 50)],
            StructureKind::Turret => &[(RES_STEEL, 20), (RES_BASIC_COMPONENTS, 5)],
            StructureKind::Fabricator => &[(RES_STEEL, 40), (RES_ADVANCED_COMPONENTS, 5)],
            StructureKind::Generator => &[(RES_STEEL, 25), (RES_BASIC_COMPONENTS, 10)],
            StructureKind::Battery => &[
                (RES_STEEL, 20),
                (RES_BASIC_COMPONENTS, 10),
                (RES_ENERGY_CELL, 5),
            ],
            StructureKind::MiningDrill => &[(RES_STEEL, 25), (RES_BASIC_COMPONENTS, 5)],
            StructureKind::Refinery => {
                &[(RES_STEEL, 50), (RES_STONE, 30), (RES_BASIC_COMPONENTS, 10)]
            }
            StructureKind::ResearchFacility => &[
                (RES_STEEL, 45),
                (RES_BASIC_COMPONENTS, 15),
                (RES_ENERGY_CELL, 10),
            ],
        }
    }

    /// Authoritative electrical properties for this structure archetype.
    pub fn power_spec(&self) -> PowerSpec {
        match self {
            StructureKind::Wall(_) => PowerSpec::default(),
            StructureKind::Pylon => PowerSpec {
                generation_kw: 0,
                demand_kw: 1,
                storage_capacity_kwh: 0,
                max_charge_rate_kw: 0,
                max_discharge_rate_kw: 0,
                connection_range: 25.0,
                distribution_range: 15.0,
                is_relay: true,
                priority: PowerPriority::High,
            },
            StructureKind::Generator => PowerSpec {
                generation_kw: 100,
                demand_kw: 0,
                storage_capacity_kwh: 0,
                max_charge_rate_kw: 0,
                max_discharge_rate_kw: 0,
                connection_range: 10.0,
                distribution_range: 10.0,
                is_relay: false,
                priority: PowerPriority::Normal,
            },
            StructureKind::Battery => PowerSpec {
                generation_kw: 0,
                demand_kw: 0,
                storage_capacity_kwh: 5000,
                max_charge_rate_kw: 50,
                max_discharge_rate_kw: 50,
                connection_range: 10.0,
                distribution_range: 10.0,
                is_relay: false,
                priority: PowerPriority::Normal,
            },
            StructureKind::Turret => PowerSpec {
                generation_kw: 0,
                demand_kw: 20,
                storage_capacity_kwh: 0,
                max_charge_rate_kw: 0,
                max_discharge_rate_kw: 0,
                connection_range: 0.0,
                distribution_range: 0.0,
                is_relay: false,
                priority: PowerPriority::High,
            },
            StructureKind::Fabricator => PowerSpec {
                generation_kw: 0,
                demand_kw: 40,
                storage_capacity_kwh: 0,
                max_charge_rate_kw: 0,
                max_discharge_rate_kw: 0,
                connection_range: 0.0,
                distribution_range: 0.0,
                is_relay: false,
                priority: PowerPriority::Normal,
            },
            StructureKind::MiningDrill => PowerSpec {
                generation_kw: 0,
                demand_kw: 20,
                storage_capacity_kwh: 0,
                max_charge_rate_kw: 0,
                max_discharge_rate_kw: 0,
                connection_range: 0.0,
                distribution_range: 0.0,
                is_relay: false,
                priority: PowerPriority::Normal,
            },
            StructureKind::Refinery => PowerSpec {
                generation_kw: 0,
                demand_kw: 50,
                storage_capacity_kwh: 0,
                max_charge_rate_kw: 0,
                max_discharge_rate_kw: 0,
                connection_range: 0.0,
                distribution_range: 0.0,
                is_relay: false,
                priority: PowerPriority::Normal,
            },
            StructureKind::ResearchFacility => PowerSpec {
                generation_kw: 0,
                demand_kw: 60,
                storage_capacity_kwh: 0,
                max_charge_rate_kw: 0,
                max_discharge_rate_kw: 0,
                connection_range: 0.0,
                distribution_range: 0.0,
                is_relay: false,
                priority: PowerPriority::Normal,
            },
            StructureKind::Depot => PowerSpec {
                generation_kw: 0,
                demand_kw: 5,
                storage_capacity_kwh: 0,
                max_charge_rate_kw: 0,
                max_discharge_rate_kw: 0,
                connection_range: 0.0,
                distribution_range: 0.0,
                is_relay: false,
                priority: PowerPriority::Low,
            },
        }
    }

    /// Authoritative sensor detection radius in meters for this structure type.
    pub fn sensor_radius(&self) -> f32 {
        match self {
            StructureKind::Turret => 50.0,
            StructureKind::ResearchFacility => 60.0,
            StructureKind::Depot | StructureKind::Fabricator | StructureKind::Refinery => 40.0,
            StructureKind::Generator | StructureKind::MiningDrill | StructureKind::Battery => 30.0,
            StructureKind::Pylon => 35.0,
            StructureKind::Wall(_) => 20.0,
        }
    }
}

/// Lifecycle state machine for physical structures.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum StructureState {
    /// Ghost/blueprint reserved on the authoritative grid.
    Planned,
    /// Actively under construction with progressing tick count.
    UnderConstruction {
        progress_ticks: u32,
        required_ticks: u32,
    },
    /// Operational with current and maximum health.
    Constructed { current_hp: u32, max_hp: u32 },
    /// Actively being deconstructed.
    Dismantling {
        remaining_ticks: u32,
        total_ticks: u32,
    },
    /// Decommissioned or destroyed.
    Destroyed,
}

impl StructureState {
    pub fn is_operational(&self) -> bool {
        matches!(self, StructureState::Constructed { .. })
    }

    pub fn is_active_or_reserved(&self) -> bool {
        !matches!(self, StructureState::Destroyed)
    }
}

/// Simulation world structure entity.
#[derive(Clone, Debug, PartialEq)]
pub struct Structure {
    pub id: StructureId,
    pub kind: StructureKind,
    pub faction_id: FactionId,
    pub region_id: RegionId,
    pub position: (f32, f32, f32),
    pub rotation_deg: f32,
    pub bounds_min: (f32, f32, f32),
    pub bounds_max: (f32, f32, f32),
    pub state: StructureState,
    pub power_status: PowerStatus,
    pub creation_tick: SimTick,
    pub weapon: Option<WeaponState>,
}

impl Structure {
    pub fn new(
        id: StructureId,
        kind: StructureKind,
        faction_id: FactionId,
        region_id: RegionId,
        position: (f32, f32, f32),
        rotation_deg: f32,
        creation_tick: SimTick,
    ) -> Self {
        let (hx, hy, hz) = kind.half_extents(rotation_deg);
        let bounds_min = (position.0 - hx, position.1, position.2 - hz);
        let bounds_max = (position.0 + hx, position.1 + hy * 2.0, position.2 + hz);

        let weapon = if kind == StructureKind::Turret {
            Some(WeaponState::new(WeaponDef::new_turret_autocannon(
                WeaponId(id.value() as u32),
            )))
        } else {
            None
        };

        Structure {
            id,
            kind,
            faction_id,
            region_id,
            position,
            rotation_deg,
            bounds_min,
            bounds_max,
            state: StructureState::Planned,
            power_status: PowerStatus::NotRequired,
            creation_tick,
            weapon,
        }
    }

    /// Checks if this structure is connected and receiving sufficient electrical power.
    pub fn is_powered(&self) -> bool {
        self.power_status.is_operational()
    }

    /// Checks if a defensive turret has health and electrical power to acquire and engage targets.
    pub fn can_fire(&self) -> bool {
        self.kind == StructureKind::Turret && self.state.is_operational() && self.is_powered()
    }

    /// Checks if an industrial structure has health and electrical power to execute jobs.
    pub fn can_operate(&self) -> bool {
        self.state.is_operational() && self.is_powered()
    }

    /// Checks if this structure actively emits sensor detection coverage.
    ///
    /// The structure must be operational, and if it consumes electrical power
    /// (`demand_kw > 0`), it must be powered.
    pub fn emits_sensor_coverage(&self) -> bool {
        if !self.state.is_operational() {
            return false;
        }
        if self.kind.power_spec().demand_kw > 0 && !self.is_powered() {
            return false;
        }
        true
    }

    /// Checks if this structure's bounding box intersects another AABB.
    pub fn intersects_aabb(&self, other_min: (f32, f32, f32), other_max: (f32, f32, f32)) -> bool {
        self.bounds_min.0 < other_max.0
            && self.bounds_max.0 > other_min.0
            && self.bounds_min.1 < other_max.1
            && self.bounds_max.1 > other_min.1
            && self.bounds_min.2 < other_max.2
            && self.bounds_max.2 > other_min.2
    }

    /// Checks if a 2D horizontal point intersects this structure's footprint.
    pub fn contains_point_xz(&self, x: f32, z: f32) -> bool {
        x >= self.bounds_min.0
            && x <= self.bounds_max.0
            && z >= self.bounds_min.2
            && z <= self.bounds_max.2
    }
}

/// Compact spatial representation for wall segments.
/// Stores wall cells using integer coordinates (e.g. 2m cells) and tier bytes,
/// achieving ultra-compact memory footprint (<1MB for 100k+ walls) and O(1) spatial queries.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompactWallGrid {
    /// Map of (grid_x, grid_z) -> (wall_tier, faction_id).
    cells: BTreeMap<(i32, i32), (u8, FactionId)>,
    pub cell_size: f32,
}

impl CompactWallGrid {
    pub fn new(cell_size: f32) -> Self {
        CompactWallGrid {
            cells: BTreeMap::new(),
            cell_size: cell_size.max(0.5),
        }
    }

    pub fn to_grid_coords(&self, x: f32, z: f32) -> (i32, i32) {
        (
            (x / self.cell_size).round() as i32,
            (z / self.cell_size).round() as i32,
        )
    }

    pub fn insert_wall(
        &mut self,
        grid_x: i32,
        grid_z: i32,
        tier: u8,
        faction_id: FactionId,
    ) -> bool {
        self.cells
            .insert((grid_x, grid_z), (tier, faction_id))
            .is_none()
    }

    pub fn remove_wall(&mut self, grid_x: i32, grid_z: i32) -> bool {
        self.cells.remove(&(grid_x, grid_z)).is_some()
    }

    pub fn has_wall(&self, grid_x: i32, grid_z: i32) -> bool {
        self.cells.contains_key(&(grid_x, grid_z))
    }

    pub fn get_wall(&self, grid_x: i32, grid_z: i32) -> Option<(u8, FactionId)> {
        self.cells.get(&(grid_x, grid_z)).copied()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&(i32, i32), &(u8, FactionId))> {
        self.cells.iter()
    }

    pub fn count(&self) -> usize {
        self.cells.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    pub fn clear(&mut self) {
        self.cells.clear();
    }
}

/// Event emitted during structure lifecycle mutations.
#[derive(Clone, Debug, PartialEq)]
pub enum StructureEvent {
    BuildReserved {
        id: StructureId,
        faction_id: FactionId,
        position: (f32, f32, f32),
    },
    ConstructionStarted {
        id: StructureId,
    },
    ConstructionCompleted {
        id: StructureId,
    },
    DismantleStarted {
        id: StructureId,
    },
    DismantleCompleted {
        id: StructureId,
    },
    Destroyed {
        id: StructureId,
    },
}

/// Central authoritative structure manager coordinating placement validation,
/// atomic concurrency reservation, lifecycle progression, and compact wall representations.
#[derive(Clone, Debug, PartialEq)]
pub struct StructureRegistry {
    pub structures: BTreeMap<StructureId, Structure>,
    pub reserved_cells: BTreeMap<(i32, i32), StructureId>,
    pub wall_grid: CompactWallGrid,
    pub power_network: PowerNetwork,
    pub deposits: BTreeMap<DepositId, ResourceDeposit>,
    pub facilities: BTreeMap<StructureId, ProductionFacility>,
    pub logistics: LogisticsManager,
    pub next_id: u64,
    pub max_interaction_reach: f32,
    /// Replica of the authoritative faction modifier network owned by
    /// `ResearchManager`, distributed here as a software patch so every
    /// structure-side system (production, power, logistics, repair) can read it
    /// without reaching across registries. Updated via [`StructureRegistry::install_modifier_patch`].
    pub modifiers: ModifierStore,
}

impl Default for StructureRegistry {
    fn default() -> Self {
        StructureRegistry {
            structures: BTreeMap::new(),
            reserved_cells: BTreeMap::new(),
            wall_grid: CompactWallGrid::new(2.0),
            power_network: PowerNetwork::new(),
            deposits: BTreeMap::new(),
            facilities: BTreeMap::new(),
            logistics: LogisticsManager::new(),
            next_id: 1,
            max_interaction_reach: 15.0,
            modifiers: ModifierStore::new(),
        }
    }
}

/// Parameters specifying an authoritative build request.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuildRequest {
    pub player_pos: (f32, f32, f32),
    pub requested_pos: (f32, f32, f32),
    pub kind: StructureKind,
    pub rotation_deg: f32,
    pub faction_id: FactionId,
    pub region_id: RegionId,
    pub creation_tick: SimTick,
    pub world_bounds_xz: (f32, f32, f32, f32),
}

impl StructureRegistry {
    pub fn new() -> Self {
        StructureRegistry::default()
    }

    /// Spatial cell coordinate for atomic collision detection (e.g. 1m cells).
    pub fn to_cell(x: f32, z: f32) -> (i32, i32) {
        (x.round() as i32, z.round() as i32)
    }

    /// Atomically validates and requests building placement.
    ///
    /// Checks:
    /// 1. Builder inventory resource availability (if inventory provided)
    /// 2. Player reach distance (<= 15m)
    /// 3. World bounds
    /// 4. Overlap with existing structures and atomic site reservation
    ///
    /// If two clients race for the same spot on the same tick, the first acquires
    /// the site reservation and the second returns Err(GameError::SiteOccupied).
    pub fn request_build(
        &mut self,
        request: BuildRequest,
        mut builder_inventory: Option<&mut Inventory>,
    ) -> GameResult<StructureId> {
        // 0. Pre-validate economy if builder inventory is provided
        if let Some(ref inv) = builder_inventory {
            for &(res_id, cost) in request.kind.construction_cost() {
                let avail = inv.available_quantity(res_id);
                if avail < cost {
                    return Err(GameError::InsufficientUnreservedBalance {
                        available: avail,
                        requested: cost,
                    });
                }
            }
        }

        // 1. Distance check
        let dx = request.requested_pos.0 - request.player_pos.0;
        let dy = request.requested_pos.1 - request.player_pos.1;
        let dz = request.requested_pos.2 - request.player_pos.2;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        if dist > self.max_interaction_reach {
            return Err(GameError::PlacementTooFar);
        }

        // 2. World bounds check
        let (hx, _, hz) = request.kind.half_extents(request.rotation_deg);
        let min_x = request.requested_pos.0 - hx;
        let max_x = request.requested_pos.0 + hx;
        let min_z = request.requested_pos.2 - hz;
        let max_z = request.requested_pos.2 + hz;

        if min_x < request.world_bounds_xz.0
            || max_x > request.world_bounds_xz.1
            || min_z < request.world_bounds_xz.2
            || max_z > request.world_bounds_xz.3
        {
            return Err(GameError::PlacementOutOfBounds);
        }

        // 3. Footprint grid cell reservation & overlap check
        let cell_min_x = min_x.floor() as i32;
        let cell_max_x = max_x.ceil() as i32;
        let cell_min_z = min_z.floor() as i32;
        let cell_max_z = max_z.ceil() as i32;

        let mut occupied_cells = Vec::new();
        for cx in cell_min_x..=cell_max_x {
            for cz in cell_min_z..=cell_max_z {
                if self.reserved_cells.contains_key(&(cx, cz)) {
                    return Err(GameError::SiteOccupied);
                }
                occupied_cells.push((cx, cz));
            }
        }

        // 4. Overlap check against existing registered structures
        let test_min = (min_x, request.requested_pos.1, min_z);
        let test_max = (max_x, request.requested_pos.1 + 4.0, max_z);
        for s in self.structures.values() {
            if s.state.is_active_or_reserved() && s.intersects_aabb(test_min, test_max) {
                return Err(GameError::PlacementOverlap);
            }
        }

        // 5. Authoritatively deduct construction costs if inventory provided
        if let Some(ref mut inv) = builder_inventory {
            for &(res_id, cost) in request.kind.construction_cost() {
                inv.remove(res_id, cost)?;
            }
        }

        // 6. Atomic reservation & insertion
        let id = StructureId::new(self.next_id);
        self.next_id += 1;

        for cell in occupied_cells {
            self.reserved_cells.insert(cell, id);
        }

        let structure = Structure::new(
            id,
            request.kind,
            request.faction_id,
            request.region_id,
            request.requested_pos,
            request.rotation_deg,
            request.creation_tick,
        );
        self.structures.insert(id, structure);

        // Register with power network (unpowered while planned/under construction)
        self.power_network.register_node(PowerNode::new(
            id,
            request.faction_id,
            request.requested_pos,
            false,
            request.kind.power_spec(),
        ));

        // If it's a wall, also register in compact wall grid
        if let StructureKind::Wall(tier) = request.kind {
            let (gx, gz) = self
                .wall_grid
                .to_grid_coords(request.requested_pos.0, request.requested_pos.2);
            self.wall_grid
                .insert_wall(gx, gz, tier.as_u8(), request.faction_id);
        }

        // If industrial production facility, initialize component
        match request.kind {
            StructureKind::MiningDrill => {
                self.facilities.insert(
                    id,
                    ProductionFacility::new(
                        id,
                        request.faction_id,
                        request.region_id,
                        FacilityKind::MiningDrill,
                    ),
                );
            }
            StructureKind::Refinery => {
                self.facilities.insert(
                    id,
                    ProductionFacility::new(
                        id,
                        request.faction_id,
                        request.region_id,
                        FacilityKind::Refinery,
                    ),
                );
            }
            StructureKind::Fabricator => {
                self.facilities.insert(
                    id,
                    ProductionFacility::new(
                        id,
                        request.faction_id,
                        request.region_id,
                        FacilityKind::Fabricator,
                    ),
                );
            }
            StructureKind::Depot => {
                self.logistics
                    .register_depot(DepotLogistics::new(EntityId::new(id.value()), 40.0));
                self.logistics
                    .register_dock(LogisticsDock::new(EntityId::new(id.value()), 2, 10));
            }
            _ => {}
        }

        Ok(id)
    }

    /// Begin active construction on a planned structure.
    pub fn start_construction(&mut self, id: StructureId) -> GameResult<()> {
        let structure = self
            .structures
            .get_mut(&id)
            .ok_or(GameError::StructureNotFound(id))?;
        if structure.state != StructureState::Planned {
            return Err(GameError::InvalidStructureState);
        }

        structure.state = StructureState::UnderConstruction {
            progress_ticks: 0,
            required_ticks: structure.kind.construction_ticks(),
        };
        Ok(())
    }

    /// Instantly complete construction (useful for pre-spawned maps or instant build cheats).
    pub fn complete_construction(&mut self, id: StructureId) -> GameResult<()> {
        let structure = self
            .structures
            .get_mut(&id)
            .ok_or(GameError::StructureNotFound(id))?;
        let hp = structure.kind.max_health();
        structure.state = StructureState::Constructed {
            current_hp: hp,
            max_hp: hp,
        };
        self.power_network.update_node_operational(id, true);
        Ok(())
    }

    /// Request deconstruction / dismantle of a structure.
    pub fn request_dismantle(
        &mut self,
        id: StructureId,
        actor_faction: FactionId,
    ) -> GameResult<()> {
        let structure = self
            .structures
            .get_mut(&id)
            .ok_or(GameError::StructureNotFound(id))?;

        if structure.faction_id != actor_faction {
            return Err(GameError::PermissionDenied);
        }

        match structure.state {
            StructureState::Constructed { .. } | StructureState::UnderConstruction { .. } => {
                let total = structure.kind.dismantle_ticks();
                structure.state = StructureState::Dismantling {
                    remaining_ticks: total,
                    total_ticks: total,
                };
                Ok(())
            }
            StructureState::Planned => {
                // Planned blueprints can be cancelled immediately
                self.remove_structure(id);
                Ok(())
            }
            _ => Err(GameError::InvalidStructureState),
        }
    }

    /// Advance construction and dismantling progress and solve power network for all active structures.
    pub fn tick(&mut self, current_tick: SimTick) -> Vec<StructureEvent> {
        let mut journal = EventJournal::new();
        self.tick_with_journal(current_tick, &mut journal)
    }

    /// Advance construction and dismantling progress and solve power network with full event journal recording.
    pub fn tick_with_journal(
        &mut self,
        current_tick: SimTick,
        journal: &mut EventJournal,
    ) -> Vec<StructureEvent> {
        let mut events = Vec::new();
        let mut completed_dismantles = Vec::new();

        for structure in self.structures.values_mut() {
            match &mut structure.state {
                StructureState::UnderConstruction {
                    progress_ticks,
                    required_ticks,
                } => {
                    *progress_ticks += 1;
                    if *progress_ticks >= *required_ticks {
                        let max_hp = structure.kind.max_health();
                        structure.state = StructureState::Constructed {
                            current_hp: max_hp,
                            max_hp,
                        };
                        self.power_network
                            .update_node_operational(structure.id, true);
                        events.push(StructureEvent::ConstructionCompleted { id: structure.id });
                    }
                }
                StructureState::Dismantling {
                    remaining_ticks, ..
                } => {
                    if *remaining_ticks > 0 {
                        *remaining_ticks -= 1;
                    }
                    if *remaining_ticks == 0 {
                        completed_dismantles.push(structure.id);
                        events.push(StructureEvent::DismantleCompleted { id: structure.id });
                    }
                }
                _ => {}
            }
        }

        // Clean up dismantled structures and free spatial reservations
        for id in completed_dismantles {
            self.remove_structure(id);
        }

        // Advance authoritative power network simulation
        self.power_network.tick(current_tick, journal);

        // Synchronize power statuses back to structures
        for structure in self.structures.values_mut() {
            structure.power_status = self.power_network.get_power_status(structure.id);
        }

        // Synchronize depot power status with logistics coverage
        for (id, structure) in &self.structures {
            if structure.kind == StructureKind::Depot {
                self.logistics
                    .update_depot_power(EntityId::new(id.value()), structure.is_powered());
            }
        }

        // Advance industrial production facilities under the distributed patch set
        for (facility_id, facility) in &mut self.facilities {
            if let Some(structure) = self.structures.get(facility_id)
                && structure.state.is_operational()
            {
                let mods = crate::production::ProductionModifiers::from_store(
                    &self.modifiers,
                    structure.faction_id,
                );
                let _ = facility.tick(
                    structure.power_status,
                    current_tick,
                    &mut self.deposits,
                    journal,
                    mods,
                );
            }
        }

        events
    }

    /// Install a faction modifier patch published by the research network.
    ///
    /// Returns `true` when the local replica changed. Power generation, power
    /// efficiency, and logistics throughput are re-derived immediately so the
    /// next tick already runs under the new patch.
    pub fn install_modifier_patch(&mut self, patch: &ModifierStore) -> bool {
        if !self.modifiers.install_patch(patch) {
            return false;
        }
        self.republish_modifier_derived_state();
        true
    }

    /// Push freshly-derived modifier values into the subsystems that cache them.
    fn republish_modifier_derived_state(&mut self) {
        for faction in self.modifiers.factions() {
            self.power_network.set_faction_power_modifiers(
                faction,
                self.modifiers
                    .multiplier_milli(faction, ModifierKind::PowerGeneration),
                self.modifiers
                    .multiplier_milli(faction, ModifierKind::PowerEfficiency),
            );
        }

        // The logistics manager is currently single-faction scoped; use the
        // lowest registered faction's throughput and coverage patch.
        if let Some(faction) = self.modifiers.factions().first().copied() {
            self.logistics.set_throughput_multiplier_milli(
                self.modifiers
                    .multiplier_milli(faction, ModifierKind::TransportThroughput),
            );
            self.logistics.set_coverage_multiplier_milli(
                self.modifiers
                    .multiplier_milli(faction, ModifierKind::LogisticsCoverage),
            );
        }
    }

    pub fn remove_structure(&mut self, id: StructureId) -> Option<Structure> {
        if let Some(structure) = self.structures.remove(&id) {
            // Free cell reservations
            self.reserved_cells.retain(|_, &mut res_id| res_id != id);
            // Free wall grid if wall
            if let StructureKind::Wall(_) = structure.kind {
                let (gx, gz) = self
                    .wall_grid
                    .to_grid_coords(structure.position.0, structure.position.2);
                self.wall_grid.remove_wall(gx, gz);
            }
            self.power_network.remove_node(id);
            self.facilities.remove(&id);
            Some(structure)
        } else {
            None
        }
    }

    /// Authoritatively apply damage to a structure, factoring in flat armor and percentage resistance.
    pub fn apply_damage(
        &mut self,
        id: StructureId,
        damage: DamageSpec,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<DamageResult> {
        let structure = self
            .structures
            .get_mut(&id)
            .ok_or(GameError::StructureNotFound(id))?;

        if structure.state == StructureState::Destroyed {
            return Err(GameError::InvalidStructureState);
        }

        let (current_hp, max_hp) = match structure.state {
            StructureState::Constructed { current_hp, max_hp } => (current_hp, max_hp),
            StructureState::UnderConstruction { .. } | StructureState::Dismantling { .. } => {
                let max = structure.kind.max_health();
                (max / 2, max)
            }
            StructureState::Planned => (1, structure.kind.max_health()),
            StructureState::Destroyed => unreachable!(),
        };

        let result = match structure.kind {
            StructureKind::Wall(tier) => {
                calculate_wall_damage(tier.archetype(), current_hp, damage)
            }
            _ => {
                let raw = damage.raw_damage;
                let dmg_rounded = raw.round() as u32;
                let (rem_hp, destroyed) = if dmg_rounded >= current_hp {
                    (0, true)
                } else {
                    (current_hp - dmg_rounded, false)
                };
                DamageResult {
                    raw_damage: raw,
                    absorbed_armor: 0.0,
                    mitigated_resistance: 0.0,
                    effective_damage: raw,
                    remaining_hp: rem_hp,
                    destroyed,
                }
            }
        };

        if result.destroyed {
            structure.state = StructureState::Destroyed;
            let pos = structure.position;
            let kind = structure.kind;
            self.reserved_cells.retain(|_, &mut res_id| res_id != id);
            if let StructureKind::Wall(_) = kind {
                let (gx, gz) = self.wall_grid.to_grid_coords(pos.0, pos.2);
                self.wall_grid.remove_wall(gx, gz);
            }
            self.power_network.remove_node(id);
            journal.record(
                tick,
                SimEvent::DamageDealt {
                    entity: EntityId::new(id.value()),
                    damage: result.effective_damage,
                    source: damage.source,
                },
            );
        } else {
            structure.state = StructureState::Constructed {
                current_hp: result.remaining_hp,
                max_hp,
            };
            journal.record(
                tick,
                SimEvent::DamageDealt {
                    entity: EntityId::new(id.value()),
                    damage: result.effective_damage,
                    source: damage.source,
                },
            );
        }

        Ok(result)
    }

    /// Authoritatively repair a structure using repair materials from an inventory.
    pub fn request_repair(
        &mut self,
        actor_faction: FactionId,
        id: StructureId,
        inventory: &mut Inventory,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<RepairResult> {
        let structure = self
            .structures
            .get_mut(&id)
            .ok_or(GameError::StructureNotFound(id))?;

        // A session may only repair its own faction's buildings, and only from
        // a container its own faction owns.
        authorize_faction(actor_faction, structure.faction_id)?;
        if !inventory.is_accessible_by(actor_faction) {
            return Err(GameError::PermissionDenied);
        }

        let (current_hp, max_hp) = match structure.state {
            StructureState::Constructed { current_hp, max_hp } => (current_hp, max_hp),
            _ => return Err(GameError::InvalidStructureState),
        };

        let (repair_res, base_hp_per_unit) = match structure.kind {
            StructureKind::Wall(tier) => {
                let arch = tier.archetype();
                (arch.repair_resource, arch.repair_hp_per_unit)
            }
            _ => (RES_STEEL, 50.0),
        };

        // Research patches make each unit of material restore more integrity;
        // they never create material out of nothing.
        let hp_per_unit = self.modifiers.value_for(
            structure.faction_id,
            ModifierKind::RepairRate,
            base_hp_per_unit,
        );

        if current_hp >= max_hp {
            return Ok(RepairResult {
                hp_restored: 0,
                units_consumed: 0,
                material_used: repair_res,
                new_hp: max_hp,
            });
        }

        let missing_hp = max_hp - current_hp;
        let units_needed = (missing_hp as f32 / hp_per_unit).ceil() as u32;
        let avail_units = inventory.available_quantity(repair_res);

        if avail_units == 0 {
            return Err(GameError::InsufficientUnreservedBalance {
                available: 0,
                requested: 1,
            });
        }

        let units_to_use = units_needed.min(avail_units);
        let hp_to_restore = ((units_to_use as f32) * hp_per_unit).round() as u32;
        let new_hp = (current_hp + hp_to_restore).min(max_hp);
        let actual_hp_restored = new_hp - current_hp;

        inventory.remove(repair_res, units_to_use)?;

        structure.state = StructureState::Constructed {
            current_hp: new_hp,
            max_hp,
        };

        journal.record(
            tick,
            SimEvent::ResourceChanged {
                entity: Some(inventory.owner),
                resource_id: repair_res,
                delta: -(units_to_use as i64),
            },
        );

        Ok(RepairResult {
            hp_restored: actual_hp_restored,
            units_consumed: units_to_use,
            material_used: repair_res,
            new_hp,
        })
    }

    pub fn get(&self, id: StructureId) -> Option<&Structure> {
        self.structures.get(&id)
    }

    pub fn is_structure_powered(&self, id: StructureId) -> bool {
        self.structures
            .get(&id)
            .map(|s| s.is_powered())
            .unwrap_or(false)
    }

    pub fn can_turret_fire(&self, id: StructureId) -> bool {
        self.structures
            .get(&id)
            .map(|s| s.can_fire())
            .unwrap_or(false)
    }

    pub fn can_fabricator_run(&self, id: StructureId) -> bool {
        self.structures
            .get(&id)
            .map(|s| s.can_operate())
            .unwrap_or(false)
    }

    /// Register a harvestable raw mineral deposit in the world.
    pub fn register_deposit(&mut self, deposit: ResourceDeposit) {
        self.deposits.insert(deposit.id, deposit);
    }

    pub fn get_deposit(&self, id: DepositId) -> Option<&ResourceDeposit> {
        self.deposits.get(&id)
    }

    pub fn get_deposit_mut(&mut self, id: DepositId) -> Option<&mut ResourceDeposit> {
        self.deposits.get_mut(&id)
    }

    pub fn get_facility(&self, id: StructureId) -> Option<&ProductionFacility> {
        self.facilities.get(&id)
    }

    pub fn get_facility_mut(&mut self, id: StructureId) -> Option<&mut ProductionFacility> {
        self.facilities.get_mut(&id)
    }

    /// Authoritatively configure an industrial recipe for a facility.
    pub fn set_production_recipe(
        &mut self,
        actor_faction: FactionId,
        id: StructureId,
        recipe_id: RecipeId,
    ) -> GameResult<()> {
        let facility = self
            .facilities
            .get_mut(&id)
            .ok_or(GameError::StructureNotFound(id))?;
        authorize_faction(actor_faction, facility.faction_id)?;
        facility.set_recipe(recipe_id)
    }

    /// Authoritatively assign a target deposit for a mining drill.
    pub fn set_extraction_target(
        &mut self,
        actor_faction: FactionId,
        id: StructureId,
        deposit_id: DepositId,
    ) -> GameResult<()> {
        let facility = self
            .facilities
            .get_mut(&id)
            .ok_or(GameError::StructureNotFound(id))?;
        authorize_faction(actor_faction, facility.faction_id)?;
        if facility.kind != FacilityKind::MiningDrill {
            return Err(GameError::InvalidStructureState);
        }
        facility.set_deposit(deposit_id);
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.structures.len()
    }
}

/// Reject an actor faction acting on something another faction owns.
///
/// A null actor faction is server/internal authority; a null owner is neutral
/// world property.
pub fn authorize_faction(actor_faction: FactionId, owner_faction: FactionId) -> GameResult<()> {
    if actor_faction.is_null() || owner_faction.is_null() || actor_faction == owner_faction {
        Ok(())
    } else {
        Err(GameError::PermissionDenied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_structure_lifecycle_state_machine() {
        let mut registry = StructureRegistry::new();
        let player_pos = (10.0, 0.0, 10.0);
        let build_pos = (12.0, 0.0, 10.0);
        let bounds = (-100.0, 100.0, -100.0, 100.0);

        // 1. Placement in Planned state
        let id = registry
            .request_build(
                BuildRequest {
                    player_pos,
                    requested_pos: build_pos,
                    kind: StructureKind::DEFAULT_WALL,
                    rotation_deg: 0.0,
                    faction_id: FactionId::new(1),
                    region_id: RegionId::new(1),
                    creation_tick: SimTick::new(1),
                    world_bounds_xz: bounds,
                },
                None,
            )
            .expect("Build should succeed");

        assert_eq!(registry.get(id).unwrap().state, StructureState::Planned);

        // 2. Start construction
        registry.start_construction(id).unwrap();
        match registry.get(id).unwrap().state {
            StructureState::UnderConstruction {
                progress_ticks,
                required_ticks,
            } => {
                assert_eq!(progress_ticks, 0);
                assert_eq!(
                    required_ticks,
                    StructureKind::DEFAULT_WALL.construction_ticks()
                );
            }
            other => panic!("Expected UnderConstruction, got {:?}", other),
        }

        // 3. Tick through construction
        let req_ticks = StructureKind::DEFAULT_WALL.construction_ticks();
        for tick_idx in 1..=req_ticks {
            let events = registry.tick(SimTick::new(tick_idx as u64));
            if tick_idx == req_ticks {
                assert_eq!(events.len(), 1);
                assert_eq!(events[0], StructureEvent::ConstructionCompleted { id });
            }
        }

        // Must be Constructed and operational
        assert!(registry.get(id).unwrap().state.is_operational());

        // 4. Request dismantle
        registry.request_dismantle(id, FactionId::new(1)).unwrap();
        match registry.get(id).unwrap().state {
            StructureState::Dismantling {
                remaining_ticks, ..
            } => {
                assert!(remaining_ticks > 0);
            }
            other => panic!("Expected Dismantling, got {:?}", other),
        }

        // 5. Tick through dismantling until removed
        let dis_ticks = StructureKind::DEFAULT_WALL.dismantle_ticks();
        for _ in 0..dis_ticks {
            registry.tick(SimTick::new(100));
        }

        // Structure removed from registry
        assert!(registry.get(id).is_none());
        assert_eq!(registry.count(), 0);
    }

    #[test]
    fn test_atomic_concurrency_race_condition_resolution() {
        let mut registry = StructureRegistry::new();
        let player1_pos = (10.0, 0.0, 10.0);
        let player2_pos = (12.0, 0.0, 12.0);
        let contested_build_pos = (11.0, 0.0, 11.0);
        let bounds = (-100.0, 100.0, -100.0, 100.0);

        // Two clients concurrently race to build on the exact same location on the same tick
        let res1 = registry.request_build(
            BuildRequest {
                player_pos: player1_pos,
                requested_pos: contested_build_pos,
                kind: StructureKind::Turret,
                rotation_deg: 0.0,
                faction_id: FactionId::new(1),
                region_id: RegionId::new(1),
                creation_tick: SimTick::new(50),
                world_bounds_xz: bounds,
            },
            None,
        );

        let res2 = registry.request_build(
            BuildRequest {
                player_pos: player2_pos,
                requested_pos: contested_build_pos,
                kind: StructureKind::Turret,
                rotation_deg: 0.0,
                faction_id: FactionId::new(2),
                region_id: RegionId::new(1),
                creation_tick: SimTick::new(50),
                world_bounds_xz: bounds,
            },
            None,
        );

        // Exactly one must succeed and the other must be rejected with SiteOccupied
        assert!(res1.is_ok());
        assert_eq!(res2.unwrap_err(), GameError::SiteOccupied);

        // Registry contains exactly 1 structure with no corruption
        assert_eq!(registry.count(), 1);
    }

    #[test]
    fn test_authoritative_distance_and_bounds_validation() {
        let mut registry = StructureRegistry::new();
        let player_pos = (0.0, 0.0, 0.0);
        let bounds = (-50.0, 50.0, -50.0, 50.0);

        // Distance > 15m rejected
        let far_pos = (20.0, 0.0, 0.0);
        let err_dist = registry.request_build(
            BuildRequest {
                player_pos,
                requested_pos: far_pos,
                kind: StructureKind::DEFAULT_WALL,
                rotation_deg: 0.0,
                faction_id: FactionId::new(1),
                region_id: RegionId::new(1),
                creation_tick: SimTick::new(1),
                world_bounds_xz: bounds,
            },
            None,
        );
        assert_eq!(err_dist.unwrap_err(), GameError::PlacementTooFar);

        // Outside bounds rejected
        let player_at_edge = (48.0, 0.0, 0.0);
        let outside_pos = (55.0, 0.0, 0.0);
        let err_bounds = registry.request_build(
            BuildRequest {
                player_pos: player_at_edge,
                requested_pos: outside_pos,
                kind: StructureKind::DEFAULT_WALL,
                rotation_deg: 0.0,
                faction_id: FactionId::new(1),
                region_id: RegionId::new(1),
                creation_tick: SimTick::new(1),
                world_bounds_xz: bounds,
            },
            None,
        );
        assert_eq!(err_bounds.unwrap_err(), GameError::PlacementOutOfBounds);
    }

    #[test]
    fn test_dismantle_permission_check() {
        let mut registry = StructureRegistry::new();
        let pos = (5.0, 0.0, 5.0);
        let bounds = (-50.0, 50.0, -50.0, 50.0);

        let id = registry
            .request_build(
                BuildRequest {
                    player_pos: pos,
                    requested_pos: pos,
                    kind: StructureKind::DEFAULT_WALL,
                    rotation_deg: 0.0,
                    faction_id: FactionId::new(1), // Owned by Faction 1
                    region_id: RegionId::new(1),
                    creation_tick: SimTick::new(1),
                    world_bounds_xz: bounds,
                },
                None,
            )
            .unwrap();

        registry.complete_construction(id).unwrap();

        // Enemy faction 2 attempts dismantle -> PermissionDenied
        let err = registry.request_dismantle(id, FactionId::new(2));
        assert_eq!(err.unwrap_err(), GameError::PermissionDenied);

        // Owner faction 1 attempts dismantle -> OK
        assert!(registry.request_dismantle(id, FactionId::new(1)).is_ok());
    }

    #[test]
    fn test_compact_wall_grid_10k_scale() {
        let mut grid = CompactWallGrid::new(2.0);
        let start = std::time::Instant::now();

        // Insert 10,000 walls (100x100 grid)
        for x in 0..100 {
            for z in 0..100 {
                grid.insert_wall(x, z, 1, FactionId::new(1));
            }
        }

        let insert_elapsed = start.elapsed();
        assert_eq!(grid.count(), 10_000);

        // Query 10,000 walls
        let q_start = std::time::Instant::now();
        for x in 0..100 {
            for z in 0..100 {
                assert!(grid.has_wall(x, z));
            }
        }
        let query_elapsed = q_start.elapsed();

        // Insertion and querying 10,000 walls should take well under 25ms total
        println!(
            "10,000 wall insertion: {:?}, query: {:?}",
            insert_elapsed, query_elapsed
        );
        assert!(insert_elapsed.as_millis() < 50);
        assert!(query_elapsed.as_millis() < 50);
    }

    #[test]
    fn test_authoritative_construction_economy_deduction_and_validation() {
        let mut registry = StructureRegistry::new();
        let builder = EntityId::new(42);
        let mut inv = Inventory::new(builder, crate::inventory::ContainerKind::Depot);
        let player_pos = (0.0, 0.0, 0.0);
        let bounds = (-500.0, 500.0, -500.0, 500.0);

        // Provide 20 Stone and 10 Steel to builder
        inv.add(RES_STONE, 20).unwrap();
        inv.add(RES_STEEL, 10).unwrap();

        // 1. Try to build Mk.2 Steel Wall (requires 15 Steel, but builder only has 10)
        let err_steel = registry.request_build(
            BuildRequest {
                player_pos,
                requested_pos: (2.0, 0.0, 0.0),
                kind: StructureKind::Wall(WallTier::Mk2Steel),
                rotation_deg: 0.0,
                faction_id: FactionId::new(1),
                region_id: RegionId::new(1),
                creation_tick: SimTick::new(1),
                world_bounds_xz: bounds,
            },
            Some(&mut inv),
        );
        assert!(matches!(
            err_steel,
            Err(GameError::InsufficientUnreservedBalance { .. })
        ));

        // Inventory is completely untouched
        assert_eq!(inv.total_quantity(RES_STONE), 20);
        assert_eq!(inv.total_quantity(RES_STEEL), 10);
        assert_eq!(registry.count(), 0);

        // 2. Build Mk.1 Stone Wall (requires 20 Stone)
        let id_mk1 = registry
            .request_build(
                BuildRequest {
                    player_pos,
                    requested_pos: (2.0, 0.0, 0.0),
                    kind: StructureKind::Wall(WallTier::Mk1Stone),
                    rotation_deg: 0.0,
                    faction_id: FactionId::new(1),
                    region_id: RegionId::new(1),
                    creation_tick: SimTick::new(1),
                    world_bounds_xz: bounds,
                },
                Some(&mut inv),
            )
            .expect("Mk.1 build should succeed with 20 Stone");

        assert_eq!(inv.total_quantity(RES_STONE), 0); // 20 stone deducted!
        assert_eq!(inv.total_quantity(RES_STEEL), 10);
        assert_eq!(registry.count(), 1);

        // 3. Compact wall grid has Mk1 (tier 1) recorded
        let (gx, gz) = registry.wall_grid.to_grid_coords(2.0, 0.0);
        assert_eq!(
            registry.wall_grid.get_wall(gx, gz),
            Some((1, FactionId::new(1)))
        );

        // 4. Add materials for Mk.3 Composite (10 Tungsten Composite + 5 Steel)
        inv.add(game_types::RES_TUNGSTEN_COMPOSITE, 10).unwrap();
        let id_mk3 = registry
            .request_build(
                BuildRequest {
                    player_pos,
                    requested_pos: (6.0, 0.0, 0.0),
                    kind: StructureKind::Wall(WallTier::Mk3Composite),
                    rotation_deg: 0.0,
                    faction_id: FactionId::new(1),
                    region_id: RegionId::new(1),
                    creation_tick: SimTick::new(1),
                    world_bounds_xz: bounds,
                },
                Some(&mut inv),
            )
            .expect("Mk.3 build should succeed");

        assert_eq!(inv.total_quantity(game_types::RES_TUNGSTEN_COMPOSITE), 0);
        assert_eq!(inv.total_quantity(RES_STEEL), 5); // 10 - 5 = 5 remaining
        assert_eq!(registry.count(), 2);

        // Compact wall grid has Mk3 (tier 3) recorded
        let (gx3, gz3) = registry.wall_grid.to_grid_coords(6.0, 0.0);
        assert_eq!(
            registry.wall_grid.get_wall(gx3, gz3),
            Some((3, FactionId::new(1)))
        );

        let _ = id_mk1;
        let _ = id_mk3;
    }

    #[test]
    fn test_authoritative_damage_mitigation_and_destruction() {
        let mut registry = StructureRegistry::new();
        let mut journal = EventJournal::new();
        let bounds = (-500.0, 500.0, -500.0, 500.0);

        // Build Mk.2 Steel Wall (3000 HP, 20 armor, 25% resistance)
        let id = registry
            .request_build(
                BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (2.0, 0.0, 0.0),
                    kind: StructureKind::Wall(WallTier::Mk2Steel),
                    rotation_deg: 0.0,
                    faction_id: FactionId::new(1),
                    region_id: RegionId::new(1),
                    creation_tick: SimTick::new(1),
                    world_bounds_xz: bounds,
                },
                None,
            )
            .unwrap();

        registry.complete_construction(id).unwrap();

        // 1. Apply 100 raw damage:
        // effective armor = 20, post_armor = 80, effective dmg = 80 * 0.75 = 60.0
        let dmg_res = registry
            .apply_damage(id, DamageSpec::new(100.0), SimTick::new(10), &mut journal)
            .unwrap();

        assert_eq!(dmg_res.absorbed_armor, 20.0);
        assert_eq!(dmg_res.effective_damage, 60.0);
        assert_eq!(dmg_res.remaining_hp, 2940);
        assert!(!dmg_res.destroyed);

        // Structure state reflects 2940 HP
        match registry.get(id).unwrap().state {
            StructureState::Constructed { current_hp, max_hp } => {
                assert_eq!(current_hp, 2940);
                assert_eq!(max_hp, 3000);
            }
            other => panic!("Expected Constructed, got {:?}", other),
        }

        // 2. Apply lethal damage to destroy wall
        let lethal_res = registry
            .apply_damage(id, DamageSpec::new(5000.0), SimTick::new(11), &mut journal)
            .unwrap();

        assert!(lethal_res.destroyed);
        assert_eq!(lethal_res.remaining_hp, 0);

        // Structure state is Destroyed and wall grid reservation is cleared
        assert_eq!(registry.get(id).unwrap().state, StructureState::Destroyed);
        let (gx, gz) = registry.wall_grid.to_grid_coords(2.0, 0.0);
        assert!(!registry.wall_grid.has_wall(gx, gz));
    }

    #[test]
    fn test_authoritative_repair_with_material_consumption() {
        let mut registry = StructureRegistry::new();
        let mut journal = EventJournal::new();
        let mut inv = Inventory::new(EntityId::new(1), crate::inventory::ContainerKind::Depot);
        let bounds = (-500.0, 500.0, -500.0, 500.0);

        // Build Mk.1 Stone Wall (1000 HP, repair: 1 Stone = 50 HP)
        let id = registry
            .request_build(
                BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (2.0, 0.0, 0.0),
                    kind: StructureKind::Wall(WallTier::Mk1Stone),
                    rotation_deg: 0.0,
                    faction_id: FactionId::new(1),
                    region_id: RegionId::new(1),
                    creation_tick: SimTick::new(1),
                    world_bounds_xz: bounds,
                },
                None,
            )
            .unwrap();

        registry.complete_construction(id).unwrap();

        // Damage wall by 200 HP (post armor/resistance), leaves wall at 800 HP
        // (Damage 215.5 raw: post armor 210.5 * 0.95 = 200 dmg)
        let _ = registry.apply_damage(id, DamageSpec::new(215.5), SimTick::new(5), &mut journal);

        let current_hp = match registry.get(id).unwrap().state {
            StructureState::Constructed { current_hp, .. } => current_hp,
            _ => panic!("Expected Constructed"),
        };
        assert_eq!(current_hp, 800);

        // Missing 200 HP. At 50 HP per stone, 4 Stone are needed to fully repair!
        // Provide 3 Stone (can restore 150 HP -> new HP 950)
        inv.add(RES_STONE, 3).unwrap();

        let rep_res1 = registry
            .request_repair(
                FactionId::null(),
                id,
                &mut inv,
                SimTick::new(6),
                &mut journal,
            )
            .unwrap();

        assert_eq!(rep_res1.hp_restored, 150);
        assert_eq!(rep_res1.units_consumed, 3);
        assert_eq!(rep_res1.material_used, RES_STONE);
        assert_eq!(rep_res1.new_hp, 950);
        assert_eq!(inv.total_quantity(RES_STONE), 0); // All 3 consumed

        // Now missing 50 HP. 1 Stone needed. Provide 5 Stone.
        inv.add(RES_STONE, 5).unwrap();

        let rep_res2 = registry
            .request_repair(
                FactionId::null(),
                id,
                &mut inv,
                SimTick::new(7),
                &mut journal,
            )
            .unwrap();

        assert_eq!(rep_res2.hp_restored, 50);
        assert_eq!(rep_res2.units_consumed, 1); // Only 1 consumed, not all 5!
        assert_eq!(rep_res2.new_hp, 1000);
        assert_eq!(inv.total_quantity(RES_STONE), 4); // 4 Stone remain

        // Wall is at 100% health: repairing again does not consume materials
        let rep_res3 = registry
            .request_repair(
                FactionId::null(),
                id,
                &mut inv,
                SimTick::new(8),
                &mut journal,
            )
            .unwrap();
        assert_eq!(rep_res3.hp_restored, 0);
        assert_eq!(rep_res3.units_consumed, 0);
        assert_eq!(inv.total_quantity(RES_STONE), 4);
    }
}
