//! Client-side presentation layer for fog of war, sensor coverage, and structure ghosts.
//!
//! Milestone 14 presentation:
//! - Extracts authoritative fog grid states (`Unexplored`, `Explored`, `Visible`) for terrain rendering.
//! - Renders sensor coverage circles around friendly units and structures.
//! - Extracts last-seen ghost snapshots of enemy structures with holographic styling.

use game_types::{FactionId, StructureId};
use sim_core::knowledge::{
    FOG_BOUNDS_MAX_X, FOG_BOUNDS_MAX_Z, FOG_BOUNDS_MIN_X, FOG_BOUNDS_MIN_Z, FOG_CELL_SIZE,
    FOG_GRID_HEIGHT, FOG_GRID_WIDTH, FOG_TOTAL_CELLS, KnowledgeState,
};
use sim_core::structure::StructureKind;
use sim_core::world::WorldState;

/// Holographic visual representation of a remembered structure ghost in fog of war.
#[derive(Debug, Clone, PartialEq)]
pub struct GhostStructureVisual {
    pub id: StructureId,
    pub kind: StructureKind,
    pub faction_id: FactionId,
    pub position: (f32, f32, f32),
    pub bounds_min: (f32, f32, f32),
    pub bounds_max: (f32, f32, f32),
    pub last_seen_hp: u32,
    /// RGBA color representation (semi-transparent desaturated holographic look)
    pub color_rgba: (f32, f32, f32, f32),
}

/// Visual representation of an active sensor detection footprint on the ground.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SensorCoverageVisual {
    pub center_pos: (f32, f32, f32),
    pub radius: f32,
    /// RGBA color representation
    pub color_rgba: (f32, f32, f32, f32),
}

/// Summary metrics and telemetry of a faction's battlefield exploration.
#[derive(Debug, Clone, PartialEq)]
pub struct FogSummaryTelemetry {
    pub faction_id: FactionId,
    pub explored_cells: usize,
    pub visible_cells: usize,
    pub total_cells: usize,
    pub explored_percent: f32,
    pub ghost_count: usize,
    pub visible_entities_count: usize,
}

/// Comprehensive client-side fog, sensor, and structure ghost view snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct FogViewSnapshot {
    pub faction_id: FactionId,
    pub grid_width: usize,
    pub grid_height: usize,
    pub cell_size: f32,
    pub bounds_xz: (f32, f32, f32, f32),
    pub cells: Vec<KnowledgeState>,
    pub ghosts: Vec<GhostStructureVisual>,
    pub coverages: Vec<SensorCoverageVisual>,
    pub telemetry: FogSummaryTelemetry,
}

impl FogViewSnapshot {
    /// Extracts a client presentation snapshot from authoritative world state for `faction`.
    pub fn extract(world: &WorldState, faction: FactionId) -> Self {
        let fk_opt = world.knowledge_manager.get(faction);

        let (cells, ghosts, coverages, visible_count) = if let Some(fk) = fk_opt {
            let cells = fk.fog_grid.cells.clone();

            let ghosts: Vec<GhostStructureVisual> = fk
                .ghost_structures
                .values()
                .map(|g| GhostStructureVisual {
                    id: g.id,
                    kind: g.kind,
                    faction_id: g.faction_id,
                    position: g.position,
                    bounds_min: g.bounds_min,
                    bounds_max: g.bounds_max,
                    last_seen_hp: g.last_seen_hp,
                    color_rgba: (0.4, 0.7, 0.9, 0.5), // Hologram cyan
                })
                .collect();

            let coverages: Vec<SensorCoverageVisual> = fk
                .active_sensors
                .iter()
                .map(|s| SensorCoverageVisual {
                    center_pos: s.position,
                    radius: s.radius,
                    color_rgba: (0.2, 0.8, 0.3, 0.2), // Faint green radar perimeter
                })
                .collect();

            let visible_count = fk.visible_entities.len();

            (cells, ghosts, coverages, visible_count)
        } else {
            (
                vec![KnowledgeState::Unexplored; FOG_TOTAL_CELLS],
                Vec::new(),
                Vec::new(),
                0,
            )
        };

        let mut explored_cells = 0usize;
        let mut visible_cells = 0usize;

        for &c in &cells {
            if c.is_visible() {
                visible_cells += 1;
                explored_cells += 1;
            } else if c.is_explored() {
                explored_cells += 1;
            }
        }

        let total_cells = FOG_TOTAL_CELLS;
        let explored_percent = if total_cells > 0 {
            (explored_cells as f32 / total_cells as f32) * 100.0
        } else {
            0.0
        };

        let ghost_count = ghosts.len();

        let telemetry = FogSummaryTelemetry {
            faction_id: faction,
            explored_cells,
            visible_cells,
            total_cells,
            explored_percent,
            ghost_count,
            visible_entities_count: visible_count,
        };

        FogViewSnapshot {
            faction_id: faction,
            grid_width: FOG_GRID_WIDTH,
            grid_height: FOG_GRID_HEIGHT,
            cell_size: FOG_CELL_SIZE,
            bounds_xz: (
                FOG_BOUNDS_MIN_X,
                FOG_BOUNDS_MAX_X,
                FOG_BOUNDS_MIN_Z,
                FOG_BOUNDS_MAX_Z,
            ),
            cells,
            ghosts,
            coverages,
            telemetry,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_types::RegionId;
    use sim_core::RobotChassis;

    #[test]
    fn test_fog_view_snapshot_extraction() {
        let mut world = WorldState::new();
        let f1 = FactionId::new(1);
        let _ = world
            .spawn_robot(
                RobotChassis::Guardsman,
                f1,
                RegionId::new(1),
                (0.0, 0.0, 0.0),
            )
            .unwrap();
        world.step_systems();

        let snapshot = FogViewSnapshot::extract(&world, f1);
        assert_eq!(snapshot.faction_id, f1);
        assert_eq!(snapshot.coverages.len(), 1);
        assert_eq!(snapshot.coverages[0].radius, 45.0);
        assert!(snapshot.telemetry.visible_cells > 0);
        assert!(snapshot.telemetry.explored_percent > 0.0);
    }
}
