use sim_core::command::Command;
use sim_core::structure::{StructureKind, StructureRegistry};
use sim_core::terrain::GreyboxTerrain;

/// Placement preview validation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlacementStatus {
    #[default]
    Valid,
    BlockedByTerrain,
    BlockedByStructure,
    TooFarFromPlayer,
    InvalidOutOfBounds,
}

impl PlacementStatus {
    pub fn is_valid(&self) -> bool {
        matches!(self, PlacementStatus::Valid)
    }

    pub fn status_text(&self) -> &'static str {
        match self {
            PlacementStatus::Valid => "VALID PLACEMENT",
            PlacementStatus::BlockedByTerrain => "BLOCKED: Terrain Obstacle",
            PlacementStatus::BlockedByStructure => "BLOCKED: Overlaps Structure",
            PlacementStatus::TooFarFromPlayer => "OUT OF RANGE (>15m)",
            PlacementStatus::InvalidOutOfBounds => "OUT OF BOUNDS",
        }
    }
}

/// Client-side placement ghost supporting instant interactive placement preview,
/// grid snapping, rotation, and multi-criteria client validation.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacementGhost {
    pub active: bool,
    pub kind: StructureKind,
    pub raw_target_pos: (f32, f32, f32),
    pub snapped_pos: (f32, f32, f32),
    pub grid_size: f32,
    pub rotation_deg: f32,
    pub status: PlacementStatus,
    pub max_reach: f32,
}

impl Default for PlacementGhost {
    fn default() -> Self {
        PlacementGhost {
            active: false,
            kind: StructureKind::DEFAULT_WALL,
            raw_target_pos: (0.0, 0.0, 0.0),
            snapped_pos: (0.0, 0.0, 0.0),
            grid_size: 2.0,
            rotation_deg: 0.0,
            status: PlacementStatus::Valid,
            max_reach: 15.0,
        }
    }
}

impl PlacementGhost {
    pub fn new(kind: StructureKind) -> Self {
        PlacementGhost {
            active: true,
            kind,
            raw_target_pos: (0.0, 0.0, 0.0),
            snapped_pos: (0.0, 0.0, 0.0),
            grid_size: 2.0,
            rotation_deg: 0.0,
            status: PlacementStatus::Valid,
            max_reach: 15.0,
        }
    }

    pub fn activate(&mut self, kind: StructureKind) {
        self.active = true;
        self.kind = kind;
    }

    pub fn deactivate(&mut self) {
        self.active = false;
    }

    pub fn rotate_clockwise(&mut self) {
        self.rotation_deg = (self.rotation_deg + 90.0).rem_euclid(360.0);
    }

    pub fn rotate_counter_clockwise(&mut self) {
        self.rotation_deg = (self.rotation_deg - 90.0).rem_euclid(360.0);
    }

    /// Cycle through wall tiers if current kind is a wall.
    pub fn cycle_wall_tier(&mut self) {
        if let StructureKind::Wall(tier) = self.kind {
            self.kind = StructureKind::Wall(tier.cycle_next());
        }
    }

    /// Explicitly set the wall tier.
    pub fn set_wall_tier(&mut self, tier: sim_core::wall::WallTier) {
        self.kind = StructureKind::Wall(tier);
    }

    /// Evaluates placement validity at `target_pos` relative to `player_pos`.
    pub fn update_preview(
        &mut self,
        player_pos: (f32, f32, f32),
        target_pos: (f32, f32, f32),
        terrain: &GreyboxTerrain,
        structure_registry: &StructureRegistry,
    ) {
        self.raw_target_pos = target_pos;

        // Snap to grid
        let snapped_x = (target_pos.0 / self.grid_size).round() * self.grid_size;
        let snapped_z = (target_pos.2 / self.grid_size).round() * self.grid_size;
        self.snapped_pos = (snapped_x, terrain.ground_y, snapped_z);

        // 1. Distance check
        let dx = self.snapped_pos.0 - player_pos.0;
        let dy = self.snapped_pos.1 - player_pos.1;
        let dz = self.snapped_pos.2 - player_pos.2;
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        if dist > self.max_reach {
            self.status = PlacementStatus::TooFarFromPlayer;
            return;
        }

        // 2. World bounds check
        let (hx, _, hz) = self.kind.half_extents(self.rotation_deg);
        let min_x = self.snapped_pos.0 - hx;
        let max_x = self.snapped_pos.0 + hx;
        let min_z = self.snapped_pos.2 - hz;
        let max_z = self.snapped_pos.2 + hz;

        if min_x < terrain.min_x
            || max_x > terrain.max_x
            || min_z < terrain.min_z
            || max_z > terrain.max_z
        {
            self.status = PlacementStatus::InvalidOutOfBounds;
            return;
        }

        // 3. Terrain obstacle check
        let test_min = (min_x, terrain.ground_y, min_z);
        let test_max = (max_x, terrain.ground_y + 4.0, max_z);

        for obs in &terrain.obstacles {
            if obs.min.0 < test_max.0
                && obs.max.0 > test_min.0
                && obs.min.1 < test_max.1
                && obs.max.1 > test_min.1
                && obs.min.2 < test_max.2
                && obs.max.2 > test_min.2
            {
                self.status = PlacementStatus::BlockedByTerrain;
                return;
            }
        }

        // 4. Structure overlap check
        for s in structure_registry.structures.values() {
            if s.state.is_active_or_reserved() && s.intersects_aabb(test_min, test_max) {
                self.status = PlacementStatus::BlockedByStructure;
                return;
            }
        }

        self.status = PlacementStatus::Valid;
    }

    /// Generates the authoritative build command if the placement is currently valid.
    pub fn create_build_command(&self) -> Option<Command> {
        if self.active && self.status.is_valid() {
            Some(Command::BuildStructure {
                kind: self.kind,
                position: self.snapped_pos,
                rotation_deg: self.rotation_deg,
            })
        } else {
            None
        }
    }
}
