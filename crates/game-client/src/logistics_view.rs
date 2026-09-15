use game_types::{EntityId, RouteNodeId, StructureId};
use sim_core::logistics::LogisticsManager;
use sim_core::structure::StructureRegistry;

/// Visual representation of a supply depot logistics coverage field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepotCoverageVisual {
    pub depot_entity: EntityId,
    pub center_pos: (f32, f32, f32),
    pub coverage_radius: f32,
    pub is_powered: bool,
    pub color_rgba: (f32, f32, f32, f32),
}

/// Visual representation of a transport corridor edge in the route graph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RouteLaneVisual {
    pub from_node: RouteNodeId,
    pub to_node: RouteNodeId,
    pub from_pos: (f32, f32, f32),
    pub to_pos: (f32, f32, f32),
    pub max_haulers: usize,
    pub current_haulers: usize,
    pub color_rgba: (f32, f32, f32, f32),
}

/// Visual marker and diagnostic badge for a logistics docking berth.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DockMarkerVisual {
    pub dock_entity: EntityId,
    pub position: (f32, f32, f32),
    pub berths: usize,
    pub active_berths: usize,
    pub queue_len: usize,
    pub is_stalled: bool,
    pub color_rgba: (f32, f32, f32, f32),
}

/// Aggregated telemetry summary of logistics throughput and health.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogisticsSummaryTelemetry {
    pub total_jobs_created: u64,
    pub total_jobs_completed: u64,
    pub pending_jobs: usize,
    pub claimed_jobs: usize,
    pub in_transit_jobs: usize,
    pub starved_jobs: usize,
    pub deadlocked_docks: usize,
    pub active_docks: usize,
    pub active_depots: usize,
}

/// Comprehensive client-side logistics network visualization snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct LogisticsViewSnapshot {
    pub depot_coverages: Vec<DepotCoverageVisual>,
    pub route_lanes: Vec<RouteLaneVisual>,
    pub dock_markers: Vec<DockMarkerVisual>,
    pub telemetry: LogisticsSummaryTelemetry,
}

impl LogisticsViewSnapshot {
    /// Extracts a full visual and telemetry snapshot from authoritative simulation registries.
    pub fn extract(logistics: &LogisticsManager, structure_registry: &StructureRegistry) -> Self {
        let mut depot_coverages = Vec::new();
        let mut dock_markers = Vec::new();
        let mut route_lanes = Vec::new();

        // 1. Extract depot coverages
        for (&depot_ent, depot) in &logistics.depots {
            let pos = structure_registry
                .get(StructureId::new(depot_ent.value()))
                .map(|s| s.position)
                .unwrap_or((0.0, 0.0, 0.0));

            let color = if depot.is_powered {
                (0.1, 0.85, 0.3, 0.35) // Translucent emerald green
            } else {
                (0.85, 0.15, 0.15, 0.2) // Faded red (coverage disabled)
            };

            depot_coverages.push(DepotCoverageVisual {
                depot_entity: depot_ent,
                center_pos: pos,
                coverage_radius: depot.effective_coverage(),
                is_powered: depot.is_powered,
                color_rgba: color,
            });
        }

        // 2. Extract dock markers
        for (&dock_ent, dock) in &logistics.docks {
            let pos = structure_registry
                .get(StructureId::new(dock_ent.value()))
                .map(|s| s.position)
                .unwrap_or((0.0, 0.0, 0.0));

            let is_stalled = dock.stalled_ticks >= 300;
            let color = if is_stalled {
                (1.0, 0.5, 0.0, 1.0) // Warning amber/orange for stalled dock
            } else if !dock.servicing.is_empty() {
                (0.2, 0.6, 1.0, 1.0) // Vibrant blue for active servicing
            } else {
                (0.5, 0.6, 0.7, 0.8) // Neutral slate for idle dock
            };

            dock_markers.push(DockMarkerVisual {
                dock_entity: dock_ent,
                position: pos,
                berths: dock.berths,
                active_berths: dock.servicing.len(),
                queue_len: dock.queue.len(),
                is_stalled,
                color_rgba: color,
            });
        }

        // 3. Extract route lanes
        for edge in &logistics.route_graph.edges {
            if let (Some(n1), Some(n2)) = (
                logistics.route_graph.get_node(edge.from),
                logistics.route_graph.get_node(edge.to),
            ) {
                let congestion = if edge.max_active_haulers > 0 {
                    (edge.current_haulers as f32) / (edge.max_active_haulers as f32)
                } else {
                    0.0
                };

                let color = if congestion >= 0.8 {
                    (1.0, 0.3, 0.1, 0.9) // Congested orange
                } else {
                    (0.3, 0.8, 1.0, 0.7) // Flowing cyan
                };

                route_lanes.push(RouteLaneVisual {
                    from_node: edge.from,
                    to_node: edge.to,
                    from_pos: n1.position,
                    to_pos: n2.position,
                    max_haulers: edge.max_active_haulers,
                    current_haulers: edge.current_haulers,
                    color_rgba: color,
                });
            }
        }

        let telemetry = LogisticsSummaryTelemetry {
            total_jobs_created: logistics.telemetry.jobs_created_total,
            total_jobs_completed: logistics.telemetry.jobs_completed_total,
            pending_jobs: logistics.telemetry.jobs_pending_count,
            claimed_jobs: logistics.telemetry.jobs_claimed_count,
            in_transit_jobs: logistics.telemetry.jobs_in_transit_count,
            starved_jobs: logistics.telemetry.jobs_starved_count,
            deadlocked_docks: logistics.telemetry.deadlocked_docks_count,
            active_docks: logistics.docks.len(),
            active_depots: logistics.depots.len(),
        };

        LogisticsViewSnapshot {
            depot_coverages,
            route_lanes,
            dock_markers,
            telemetry,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sim_core::logistics::{DepotLogistics, LogisticsDock, RouteEdge, RouteNode};

    #[test]
    fn test_logistics_view_snapshot_extraction() {
        let mut logistics = LogisticsManager::new();
        let structure_registry = StructureRegistry::new();

        let depot_ent = EntityId::new(10);
        logistics.register_depot(DepotLogistics::new(depot_ent, 40.0));

        let dock_ent = EntityId::new(20);
        let mut dock = LogisticsDock::new(dock_ent, 2, 10);
        dock.stalled_ticks = 350; // trigger stall
        logistics.register_dock(dock);

        let n1 = RouteNodeId::new(1);
        let n2 = RouteNodeId::new(2);
        logistics.route_graph.add_node(RouteNode {
            id: n1,
            position: (0.0, 0.0, 0.0),
            associated_entity: None,
        });
        logistics.route_graph.add_node(RouteNode {
            id: n2,
            position: (20.0, 0.0, 0.0),
            associated_entity: None,
        });
        logistics.route_graph.add_edge(RouteEdge {
            from: n1,
            to: n2,
            distance: 20.0,
            max_active_haulers: 5,
            current_haulers: 1,
            traversal_speed: 1.0,
        });

        let snapshot = LogisticsViewSnapshot::extract(&logistics, &structure_registry);

        assert_eq!(snapshot.depot_coverages.len(), 1);
        assert_eq!(snapshot.depot_coverages[0].coverage_radius, 40.0);
        assert!(snapshot.depot_coverages[0].is_powered);

        assert_eq!(snapshot.dock_markers.len(), 1);
        assert!(snapshot.dock_markers[0].is_stalled);

        assert_eq!(snapshot.route_lanes.len(), 1);
        assert_eq!(snapshot.route_lanes[0].max_haulers, 5);

        assert_eq!(snapshot.telemetry.active_depots, 1);
        assert_eq!(snapshot.telemetry.active_docks, 1);
    }
}
