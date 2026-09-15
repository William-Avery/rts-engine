use game_types::{EntityId, GameError, GameResult, RegionId};
use std::collections::{BTreeMap, BTreeSet};

/// Simulation region activity state.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum RegionState {
    /// Active, high-fidelity simulation (e.g. 30 Hz base tick).
    Hot,
    /// Relevant infrastructure/units but reduced frequency (e.g. 15 Hz / half rate).
    Warm,
    /// Distant or dormant; event-driven or scheduled wakeups only. No per-entity per-tick iteration.
    Cold,
}

impl RegionState {
    pub const fn is_hot(&self) -> bool {
        matches!(self, RegionState::Hot)
    }

    pub const fn is_warm(&self) -> bool {
        matches!(self, RegionState::Warm)
    }

    pub const fn is_cold(&self) -> bool {
        matches!(self, RegionState::Cold)
    }
}

/// 2D Axis-Aligned Bounding Box for world region partitioning on the XZ ground plane.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct RegionBounds {
    pub min_x: f32,
    pub min_z: f32,
    pub max_x: f32,
    pub max_z: f32,
}

impl RegionBounds {
    pub fn new(min_x: f32, min_z: f32, max_x: f32, max_z: f32) -> GameResult<Self> {
        if min_x > max_x || min_z > max_z {
            return Err(GameError::InvalidRegionBounds);
        }
        Ok(RegionBounds {
            min_x,
            min_z,
            max_x,
            max_z,
        })
    }

    pub fn contains(&self, x: f32, z: f32) -> bool {
        x >= self.min_x && x < self.max_x && z >= self.min_z && z < self.max_z
    }

    pub fn width(&self) -> f32 {
        self.max_x - self.min_x
    }

    pub fn depth(&self) -> f32 {
        self.max_z - self.min_z
    }

    pub fn center(&self) -> (f32, f32) {
        (
            (self.min_x + self.max_x) * 0.5,
            (self.min_z + self.max_z) * 0.5,
        )
    }
}

/// A partition of the simulation world owning a set of entities.
#[derive(Debug, Clone)]
pub struct Region {
    pub id: RegionId,
    pub bounds: RegionBounds,
    pub state: RegionState,
    entities: BTreeSet<EntityId>,
}

impl Region {
    pub fn new(id: RegionId, bounds: RegionBounds, state: RegionState) -> Self {
        Region {
            id,
            bounds,
            state,
            entities: BTreeSet::new(),
        }
    }

    pub fn entity_count(&self) -> usize {
        self.entities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    pub fn contains_entity(&self, entity_id: EntityId) -> bool {
        self.entities.contains(&entity_id)
    }

    pub fn add_entity(&mut self, entity_id: EntityId) -> bool {
        self.entities.insert(entity_id)
    }

    pub fn remove_entity(&mut self, entity_id: EntityId) -> bool {
        self.entities.remove(&entity_id)
    }

    pub fn set_state(&mut self, state: RegionState) {
        self.state = state;
    }

    pub fn entities(&self) -> impl Iterator<Item = &EntityId> {
        self.entities.iter()
    }

    pub fn entities_vec(&self) -> Vec<EntityId> {
        self.entities.iter().copied().collect()
    }
}

/// Fixed uniform grid layout for world partitioning into regions.
#[derive(Debug, Clone)]
pub struct RegionGrid {
    pub origin_x: f32,
    pub origin_z: f32,
    pub cell_width: f32,
    pub cell_depth: f32,
    pub cols: u32,
    pub rows: u32,
}

impl RegionGrid {
    pub fn new(
        origin_x: f32,
        origin_z: f32,
        cell_width: f32,
        cell_depth: f32,
        cols: u32,
        rows: u32,
    ) -> GameResult<Self> {
        if cell_width <= 0.0 || cell_depth <= 0.0 || cols == 0 || rows == 0 {
            return Err(GameError::InvalidRegionBounds);
        }
        Ok(RegionGrid {
            origin_x,
            origin_z,
            cell_width,
            cell_depth,
            cols,
            rows,
        })
    }

    pub fn region_id_at_grid(&self, col: u32, row: u32) -> Option<RegionId> {
        if col < self.cols && row < self.rows {
            // Region IDs start at 1
            Some(RegionId::new(row * self.cols + col + 1))
        } else {
            None
        }
    }

    pub fn region_at_coords(&self, x: f32, z: f32) -> Option<RegionId> {
        if x < self.origin_x || z < self.origin_z {
            return None;
        }

        let col = ((x - self.origin_x) / self.cell_width).floor() as i64;
        let row = ((z - self.origin_z) / self.cell_depth).floor() as i64;

        if col >= 0 && (col as u32) < self.cols && row >= 0 && (row as u32) < self.rows {
            self.region_id_at_grid(col as u32, row as u32)
        } else {
            None
        }
    }

    pub fn bounds_for_grid(&self, col: u32, row: u32) -> GameResult<RegionBounds> {
        if col >= self.cols || row >= self.rows {
            return Err(GameError::InvalidRegionBounds);
        }
        let min_x = self.origin_x + (col as f32) * self.cell_width;
        let min_z = self.origin_z + (row as f32) * self.cell_depth;
        let max_x = min_x + self.cell_width;
        let max_z = min_z + self.cell_depth;
        RegionBounds::new(min_x, min_z, max_x, max_z)
    }
}

/// Manages world regions, spatial lookup, and strict entity-to-region ownership.
#[derive(Debug, Clone, Default)]
pub struct RegionMap {
    regions: BTreeMap<RegionId, Region>,
    entity_to_region: BTreeMap<EntityId, RegionId>,
    grid: Option<RegionGrid>,
}

impl RegionMap {
    pub fn new() -> Self {
        RegionMap {
            regions: BTreeMap::new(),
            entity_to_region: BTreeMap::new(),
            grid: None,
        }
    }

    /// Create a pre-partitioned uniform grid of regions with a default state.
    pub fn create_grid(
        origin_x: f32,
        origin_z: f32,
        cell_width: f32,
        cell_depth: f32,
        cols: u32,
        rows: u32,
        default_state: RegionState,
    ) -> GameResult<Self> {
        let grid = RegionGrid::new(origin_x, origin_z, cell_width, cell_depth, cols, rows)?;
        let mut map = RegionMap::new();

        for row in 0..rows {
            for col in 0..cols {
                let id = grid
                    .region_id_at_grid(col, row)
                    .ok_or(GameError::InvalidRegionBounds)?;
                let bounds = grid.bounds_for_grid(col, row)?;
                map.add_region(Region::new(id, bounds, default_state))?;
            }
        }

        map.grid = Some(grid);
        Ok(map)
    }

    pub fn add_region(&mut self, region: Region) -> GameResult<()> {
        if region.id.is_null() {
            return Err(GameError::InvalidId);
        }
        self.regions.insert(region.id, region);
        Ok(())
    }

    pub fn get_region(&self, id: RegionId) -> Option<&Region> {
        self.regions.get(&id)
    }

    pub fn get_region_mut(&mut self, id: RegionId) -> Option<&mut Region> {
        self.regions.get_mut(&id)
    }

    pub fn region_count(&self) -> usize {
        self.regions.len()
    }

    pub fn region_for_entity(&self, entity_id: EntityId) -> Option<RegionId> {
        self.entity_to_region.get(&entity_id).copied()
    }

    pub fn region_at_coords(&self, x: f32, z: f32) -> Option<RegionId> {
        if let Some(grid) = &self.grid {
            return grid.region_at_coords(x, z);
        }
        // Fallback to searching bounded regions
        for region in self.regions.values() {
            if region.bounds.contains(x, z) {
                return Some(region.id);
            }
        }
        None
    }

    /// Assigns an entity to a region, establishing strict ownership.
    pub fn assign_entity(&mut self, entity_id: EntityId, region_id: RegionId) -> GameResult<()> {
        if entity_id.is_null() || region_id.is_null() {
            return Err(GameError::InvalidId);
        }

        // Check destination exists
        if !self.regions.contains_key(&region_id) {
            return Err(GameError::RegionNotFound(region_id));
        }

        // If previously registered in a different region, remove first
        if let Some(old_region_id) = self
            .entity_to_region
            .get(&entity_id)
            .copied()
            .filter(|&id| id != region_id)
            && let Some(old_region) = self.regions.get_mut(&old_region_id)
        {
            old_region.remove_entity(entity_id);
        }

        if let Some(region) = self.regions.get_mut(&region_id) {
            region.add_entity(entity_id);
        }
        self.entity_to_region.insert(entity_id, region_id);
        Ok(())
    }

    /// Removes an entity from whatever region it currently belongs to.
    pub fn remove_entity(&mut self, entity_id: EntityId) -> GameResult<RegionId> {
        let region_id = self
            .entity_to_region
            .remove(&entity_id)
            .ok_or(GameError::EntityNotFound(entity_id))?;

        if let Some(region) = self.regions.get_mut(&region_id) {
            region.remove_entity(entity_id);
        }

        Ok(region_id)
    }

    /// Atomically transfer an entity from its current region to destination region.
    /// Ensures no entity duplication or loss.
    pub fn transfer_entity(
        &mut self,
        entity_id: EntityId,
        destination_region: RegionId,
    ) -> GameResult<(RegionId, RegionId)> {
        if entity_id.is_null() || destination_region.is_null() {
            return Err(GameError::InvalidId);
        }

        // Validate destination region exists before mutating anything
        if !self.regions.contains_key(&destination_region) {
            return Err(GameError::RegionNotFound(destination_region));
        }

        // Validate current ownership
        let source_region_id = self
            .entity_to_region
            .get(&entity_id)
            .copied()
            .ok_or(GameError::EntityNotFound(entity_id))?;

        if source_region_id == destination_region {
            // Already in destination, no-op
            return Ok((source_region_id, destination_region));
        }

        // Remove from source region
        if let Some(source_region) = self.regions.get_mut(&source_region_id) {
            source_region.remove_entity(entity_id);
        }

        // Add to destination region
        if let Some(dest_region) = self.regions.get_mut(&destination_region) {
            dest_region.add_entity(entity_id);
        }

        // Update ownership map
        self.entity_to_region.insert(entity_id, destination_region);

        Ok((source_region_id, destination_region))
    }

    pub fn set_region_state(
        &mut self,
        id: RegionId,
        state: RegionState,
    ) -> GameResult<RegionState> {
        let region = self
            .regions
            .get_mut(&id)
            .ok_or(GameError::RegionNotFound(id))?;
        let old = region.state;
        region.set_state(state);
        Ok(old)
    }

    pub fn regions_by_state(&self, state: RegionState) -> Vec<RegionId> {
        self.regions
            .iter()
            .filter(|(_, r)| r.state == state)
            .map(|(id, _)| *id)
            .collect()
    }

    pub fn iter_regions(&self) -> impl Iterator<Item = (&RegionId, &Region)> {
        self.regions.iter()
    }

    pub fn total_entities(&self) -> usize {
        self.entity_to_region.len()
    }
}
