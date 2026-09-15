use game_types::{FactionId, StructureId};
use sim_core::power::{PowerGridStatus, PowerNetwork, PowerStatus};
use sim_core::structure::{StructureKind, StructureRegistry};

/// Visual representation of an electrical transmission line between two nodes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PowerLinkVisual {
    pub from_id: StructureId,
    pub to_id: StructureId,
    pub from_pos: (f32, f32, f32),
    pub to_pos: (f32, f32, f32),
    /// RGBA color representation
    pub color_rgba: (f32, f32, f32, f32),
}

/// Visual representation of a pylon or relay distribution coverage field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PylonCoverageVisual {
    pub structure_id: StructureId,
    pub center_pos: (f32, f32, f32),
    pub connection_radius: f32,
    pub distribution_radius: f32,
    pub color_rgba: (f32, f32, f32, f32),
}

/// Visual marker and color-coding for individual structures based on electrical state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StructurePowerMarker {
    pub structure_id: StructureId,
    pub kind: StructureKind,
    pub position: (f32, f32, f32),
    pub status: PowerStatus,
    pub color_rgba: (f32, f32, f32, f32),
}

/// Aggregated telemetry summary of a faction's electrical grid status.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerSummaryTelemetry {
    pub faction_id: FactionId,
    pub active_subnets: usize,
    pub total_generation_kw: u32,
    pub total_demand_kw: u32,
    pub net_balance_kw: i32,
    pub total_stored_kwh: u32,
    pub total_storage_capacity_kwh: u32,
    pub battery_percent: f32,
    pub powered_structures: usize,
    pub brownout_structures: usize,
    pub unpowered_structures: usize,
    pub worst_status: PowerGridStatus,
}

/// Comprehensive client-side power network visualization snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct PowerViewSnapshot {
    pub links: Vec<PowerLinkVisual>,
    pub coverages: Vec<PylonCoverageVisual>,
    pub markers: Vec<StructurePowerMarker>,
    pub telemetry: PowerSummaryTelemetry,
}

impl PowerViewSnapshot {
    /// Extracts a full visual and telemetry snapshot from authoritative simulation registries.
    pub fn extract(
        power_network: &PowerNetwork,
        structure_registry: &StructureRegistry,
        target_faction: FactionId,
    ) -> Self {
        let mut links = Vec::new();
        let mut coverages = Vec::new();
        let mut markers = Vec::new();

        let raw_links = power_network.transmission_links();
        for link in raw_links {
            if let (Some(a), Some(b)) = (
                structure_registry.get(link.node_a),
                structure_registry.get(link.node_b),
            ) && a.faction_id == target_faction
                && b.faction_id == target_faction
            {
                links.push(PowerLinkVisual {
                    from_id: link.node_a,
                    to_id: link.node_b,
                    from_pos: link.pos_a,
                    to_pos: link.pos_b,
                    color_rgba: (0.2, 0.8, 1.0, 0.8), // Bright cyan transmission line
                });
            }
        }

        let mut powered_count = 0usize;
        let mut brownout_count = 0usize;
        let mut unpowered_count = 0usize;

        for structure in structure_registry.structures.values() {
            if structure.faction_id != target_faction {
                continue;
            }

            let status = structure.power_status;
            let color = match status {
                PowerStatus::Powered { .. } => {
                    powered_count += 1;
                    (0.1, 0.95, 0.2, 1.0) // Vibrant green
                }
                PowerStatus::Brownout { .. } => {
                    brownout_count += 1;
                    (1.0, 0.8, 0.1, 1.0) // Warning amber
                }
                PowerStatus::Unpowered => {
                    if structure.kind.power_spec().demand_kw > 0 {
                        unpowered_count += 1;
                    }
                    (0.9, 0.15, 0.15, 1.0) // Shutdown red
                }
                PowerStatus::NotRequired => (0.5, 0.5, 0.6, 0.5), // Neutral gray
            };

            markers.push(StructurePowerMarker {
                structure_id: structure.id,
                kind: structure.kind,
                position: structure.position,
                status,
                color_rgba: color,
            });

            if structure.kind == StructureKind::Pylon && structure.state.is_operational() {
                let spec = structure.kind.power_spec();
                coverages.push(PylonCoverageVisual {
                    structure_id: structure.id,
                    center_pos: structure.position,
                    connection_radius: spec.connection_range,
                    distribution_radius: spec.distribution_range,
                    color_rgba: (0.1, 0.7, 1.0, 0.25), // Translucent cyan coverage field
                });
            }
        }

        // Aggregate telemetry across all subnets belonging to target faction
        let mut gen_kw = 0u32;
        let mut dem_kw = 0u32;
        let mut stored_kwh = 0u32;
        let mut cap_kwh = 0u32;
        let mut faction_subnets = 0usize;
        let mut worst_status = PowerGridStatus::Optimal;

        for subnet in power_network.subnets() {
            if subnet.faction_id == target_faction {
                faction_subnets += 1;
                gen_kw += subnet.total_generation_kw;
                dem_kw += subnet.total_demand_kw;
                stored_kwh += subnet.total_stored_kwh;
                cap_kwh += subnet.total_storage_capacity_kwh;

                match subnet.status {
                    PowerGridStatus::Blackout => worst_status = PowerGridStatus::Blackout,
                    PowerGridStatus::Brownout if worst_status != PowerGridStatus::Blackout => {
                        worst_status = PowerGridStatus::Brownout;
                    }
                    PowerGridStatus::BatterySupported
                        if worst_status == PowerGridStatus::Optimal =>
                    {
                        worst_status = PowerGridStatus::BatterySupported;
                    }
                    _ => {}
                }
            }
        }

        let battery_percent = if cap_kwh > 0 {
            (stored_kwh as f32 / cap_kwh as f32) * 100.0
        } else {
            0.0
        };

        PowerViewSnapshot {
            links,
            coverages,
            markers,
            telemetry: PowerSummaryTelemetry {
                faction_id: target_faction,
                active_subnets: faction_subnets,
                total_generation_kw: gen_kw,
                total_demand_kw: dem_kw,
                net_balance_kw: gen_kw as i32 - dem_kw as i32,
                total_stored_kwh: stored_kwh,
                total_storage_capacity_kwh: cap_kwh,
                battery_percent,
                powered_structures: powered_count,
                brownout_structures: brownout_count,
                unpowered_structures: unpowered_count,
                worst_status,
            },
        }
    }

    /// Generates a formatted ASCII diagnostic dashboard report.
    pub fn render_ascii_report(&self) -> String {
        let t = &self.telemetry;
        let status_str = match t.worst_status {
            PowerGridStatus::Optimal => "OPTIMAL (Surplus)",
            PowerGridStatus::BatterySupported => "BATTERY BUFFERED",
            PowerGridStatus::Brownout => "BROWNOUT (Load Shedding)",
            PowerGridStatus::Blackout => "TOTAL BLACKOUT",
        };

        let mut out = String::new();
        out.push_str("+--------------------------------------------------------------+\n");
        out.push_str("|                   POWER NETWORK DIAGNOSTICS                  |\n");
        out.push_str("+--------------------------------------------------------------+\n");
        out.push_str(&format!(
            "| Status: {:<20}    Active Subnets: {:>10} |\n",
            status_str, t.active_subnets
        ));
        out.push_str(&format!(
            "| Total Generation: {:>7} kW      Total Demand:   {:>7} kW |\n",
            t.total_generation_kw, t.total_demand_kw
        ));
        out.push_str(&format!(
            "| Net Power Balance: {:>+6} kW      Transmission Links: {:>6} |\n",
            t.net_balance_kw,
            self.links.len()
        ));
        out.push_str(&format!(
            "| Battery Storage:  {:>6} / {:<6} kWh ({:>5.1}%)             |\n",
            t.total_stored_kwh, t.total_storage_capacity_kwh, t.battery_percent
        ));
        out.push_str("+--------------------------------------------------------------+\n");
        out.push_str(&format!(
            "| Powered: {:<5} | Brownout: {:<5} | Unpowered: {:<5} | Pylons: {:<4} |\n",
            t.powered_structures,
            t.brownout_structures,
            t.unpowered_structures,
            self.coverages.len()
        ));
        out.push_str("+--------------------------------------------------------------+\n");
        out
    }
}
