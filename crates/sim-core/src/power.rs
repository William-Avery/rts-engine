use crate::event::{EventJournal, SimEvent};
use crate::modifier::MODIFIER_SCALE;
use game_types::{FactionId, PowerGridId, SimTick, StructureId};
use std::collections::{BTreeMap, BTreeSet};

/// Consumer priority tiers for authoritative load shedding during power deficits.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub enum PowerPriority {
    /// Essential defensive infrastructure (Turrets, Shields) - shed last.
    High = 3,
    /// Standard industrial production and fabrication (Fabricators, Refineries).
    Normal = 2,
    /// Auxiliary or logistical infrastructure (Depots, Lights) - shed first.
    Low = 1,
}

/// Operational power status of an individual structure.
#[derive(Default, Copy, Clone, PartialEq, Debug)]
pub enum PowerStatus {
    /// Structure does not participate in or require electrical power (e.g. passive walls).
    #[default]
    NotRequired,
    /// Structure is connected to an active grid and receiving full power demand.
    Powered { satisfaction: f32 },
    /// Structure is experiencing a brownout / partial power deficit (0.0 < satisfaction < 1.0).
    Brownout { satisfaction: f32 },
    /// Structure requires electrical power but is receiving none (satisfaction = 0.0), causing shutdown.
    Unpowered,
}

impl PowerStatus {
    /// Returns true if the structure has sufficient energy to function.
    pub fn is_operational(&self) -> bool {
        match self {
            PowerStatus::NotRequired => true,
            PowerStatus::Powered { satisfaction } => *satisfaction >= 0.99,
            PowerStatus::Brownout { satisfaction } => *satisfaction >= 0.5,
            PowerStatus::Unpowered => false,
        }
    }

    /// Normalized satisfaction fraction in [0.0, 1.0].
    pub fn satisfaction_ratio(&self) -> f32 {
        match self {
            PowerStatus::NotRequired => 1.0,
            PowerStatus::Powered { satisfaction } => *satisfaction,
            PowerStatus::Brownout { satisfaction } => *satisfaction,
            PowerStatus::Unpowered => 0.0,
        }
    }
}

/// Overall health and operational status of a connected power grid.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Hash)]
pub enum PowerGridStatus {
    /// Generation meets or exceeds total demand; batteries may be charging.
    Optimal,
    /// Generation is below demand, but battery reserves are covering the full shortfall.
    BatterySupported,
    /// Demand exceeds generation + battery discharge; low-priority loads are being shed.
    Brownout,
    /// Zero available power; all active electrical loads in the subnet are unpowered.
    Blackout,
}

/// Static electrical specification for a structure archetype.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PowerSpec {
    /// Active generation output in kilowatts (kW).
    pub generation_kw: u32,
    /// Required power consumption in kilowatts (kW).
    pub demand_kw: u32,
    /// Maximum battery energy storage in kilowatt-hours (kWh).
    pub storage_capacity_kwh: u32,
    /// Maximum battery charge rate in kilowatts (kW).
    pub max_charge_rate_kw: u32,
    /// Maximum battery discharge rate in kilowatts (kW).
    pub max_discharge_rate_kw: u32,
    /// Maximum distance in meters to establish a transmission link to another relay.
    pub connection_range: f32,
    /// Maximum radius in meters to distribute power to local endpoint structures.
    pub distribution_range: f32,
    /// Whether this structure functions as an electrical distribution relay (e.g. Pylon).
    pub is_relay: bool,
    /// Load shedding priority during deficits.
    pub priority: PowerPriority,
}

impl Default for PowerSpec {
    fn default() -> Self {
        PowerSpec {
            generation_kw: 0,
            demand_kw: 0,
            storage_capacity_kwh: 0,
            max_charge_rate_kw: 0,
            max_discharge_rate_kw: 0,
            connection_range: 0.0,
            distribution_range: 0.0,
            is_relay: false,
            priority: PowerPriority::Normal,
        }
    }
}

/// Active power simulation node registered in the authoritative network.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerNode {
    pub structure_id: StructureId,
    pub faction_id: FactionId,
    pub position: (f32, f32, f32),
    pub operational: bool,
    pub spec: PowerSpec,
    pub stored_energy_kwh: u32,
}

impl PowerNode {
    pub fn new(
        structure_id: StructureId,
        faction_id: FactionId,
        position: (f32, f32, f32),
        operational: bool,
        spec: PowerSpec,
    ) -> Self {
        PowerNode {
            structure_id,
            faction_id,
            position,
            operational,
            spec,
            stored_energy_kwh: 0,
        }
    }
}

/// An isolated connected component of electrical infrastructure.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerSubnet {
    pub grid_id: PowerGridId,
    pub faction_id: FactionId,
    pub nodes: Vec<StructureId>,
    pub total_generation_kw: u32,
    pub total_demand_kw: u32,
    pub total_storage_capacity_kwh: u32,
    pub total_stored_kwh: u32,
    pub battery_charge_rate_kw: u32,
    pub battery_discharge_rate_kw: u32,
    pub net_balance_kw: i32,
    pub satisfaction_ratio: f32,
    pub status: PowerGridStatus,
}

/// Transmission connection between two relay or endpoint nodes for debug visualization.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PowerTransmissionLink {
    pub node_a: StructureId,
    pub node_b: StructureId,
    pub pos_a: (f32, f32, f32),
    pub pos_b: (f32, f32, f32),
}

/// Diagnostic metrics tracking power network execution and topology recomputations.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PowerNetworkMetrics {
    pub topology_rebuild_count: u64,
    pub ticks_processed: u64,
    pub active_subnets: usize,
    pub total_generation_kw: u32,
    pub total_demand_kw: u32,
    pub total_stored_kwh: u32,
    pub powered_count: usize,
    pub brownout_count: usize,
    pub unpowered_count: usize,
}

/// Authoritative power network graph simulator.
#[derive(Clone, Debug, PartialEq)]
pub struct PowerNetwork {
    nodes: BTreeMap<StructureId, PowerNode>,
    subnets: Vec<PowerSubnet>,
    structure_to_grid: BTreeMap<StructureId, PowerGridId>,
    structure_power_status: BTreeMap<StructureId, PowerStatus>,
    previous_grid_status: BTreeMap<PowerGridId, PowerGridStatus>,
    /// Research power patches per faction: `(generation_milli, efficiency_milli)`.
    faction_power_modifiers: BTreeMap<FactionId, (i64, i64)>,
    topology_dirty: bool,
    next_grid_id: u64,
    pub metrics: PowerNetworkMetrics,
}

impl Default for PowerNetwork {
    fn default() -> Self {
        Self::new()
    }
}

impl PowerNetwork {
    pub fn new() -> Self {
        PowerNetwork {
            nodes: BTreeMap::new(),
            subnets: Vec::new(),
            structure_to_grid: BTreeMap::new(),
            structure_power_status: BTreeMap::new(),
            previous_grid_status: BTreeMap::new(),
            faction_power_modifiers: BTreeMap::new(),
            topology_dirty: false,
            next_grid_id: 1,
            metrics: PowerNetworkMetrics::default(),
        }
    }

    /// Install a faction's research power patch.
    ///
    /// `generation_milli` scales generator output up; `efficiency_milli` scales
    /// consumer demand down. Both are fixed-point thousandths (`1000` neutral).
    pub fn set_faction_power_modifiers(
        &mut self,
        faction: FactionId,
        generation_milli: i64,
        efficiency_milli: i64,
    ) {
        self.faction_power_modifiers
            .insert(faction, (generation_milli.max(0), efficiency_milli.max(1)));
    }

    /// Current `(generation_milli, efficiency_milli)` patch for a faction.
    pub fn faction_power_modifiers(&self, faction: FactionId) -> (i64, i64) {
        self.faction_power_modifiers
            .get(&faction)
            .copied()
            .unwrap_or((MODIFIER_SCALE, MODIFIER_SCALE))
    }

    /// Generator output after the research generation patch.
    #[inline]
    fn patched_generation(base_kw: u32, generation_milli: i64) -> u32 {
        let scaled = (base_kw as i64).saturating_mul(generation_milli) / MODIFIER_SCALE;
        scaled.clamp(0, u32::MAX as i64) as u32
    }

    /// Consumer demand after the research efficiency patch (higher efficiency
    /// means lower demand). A powered consumer never drops below 1 kW.
    #[inline]
    fn patched_demand(base_kw: u32, efficiency_milli: i64) -> u32 {
        if base_kw == 0 {
            return 0;
        }
        let scaled = (base_kw as i64).saturating_mul(MODIFIER_SCALE) / efficiency_milli.max(1);
        scaled.clamp(1, u32::MAX as i64) as u32
    }

    /// Register or update a structure node in the power network.
    pub fn register_node(&mut self, node: PowerNode) {
        let is_passive = node.spec.generation_kw == 0
            && node.spec.demand_kw == 0
            && node.spec.storage_capacity_kwh == 0
            && !node.spec.is_relay;

        if is_passive {
            // Passive structures do not participate in power graph solving
            self.structure_power_status
                .insert(node.structure_id, PowerStatus::NotRequired);
            return;
        }

        self.nodes.insert(node.structure_id, node);
        self.topology_dirty = true;
    }

    /// Update the operational lifecycle state of a structure node.
    pub fn update_node_operational(&mut self, id: StructureId, operational: bool) {
        if let Some(node) = self.nodes.get_mut(&id)
            && node.operational != operational
        {
            node.operational = operational;
            self.topology_dirty = true;
        }
    }

    /// Remove a structure node upon destruction or dismantle.
    pub fn remove_node(&mut self, id: StructureId) {
        if self.nodes.remove(&id).is_some() {
            self.structure_to_grid.remove(&id);
            self.structure_power_status.remove(&id);
            self.topology_dirty = true;
        } else {
            self.structure_power_status.remove(&id);
        }
    }

    /// Force topology rebuild on the next simulation tick.
    pub fn invalidate_topology(&mut self) {
        self.topology_dirty = true;
    }

    /// Query the current authoritative power status of a structure.
    pub fn get_power_status(&self, id: StructureId) -> PowerStatus {
        self.structure_power_status
            .get(&id)
            .copied()
            .unwrap_or(PowerStatus::NotRequired)
    }

    /// Query the subnet ID containing a structure.
    pub fn get_grid_for_structure(&self, id: StructureId) -> Option<PowerGridId> {
        self.structure_to_grid.get(&id).copied()
    }

    /// Access all active power subnets.
    pub fn subnets(&self) -> &[PowerSubnet] {
        &self.subnets
    }

    /// Access all registered power nodes.
    pub fn nodes(&self) -> &BTreeMap<StructureId, PowerNode> {
        &self.nodes
    }

    /// Generates transmission link edges between connected nodes for debug rendering.
    pub fn transmission_links(&self) -> Vec<PowerTransmissionLink> {
        let mut links = Vec::new();
        let node_list: Vec<&PowerNode> = self.nodes.values().filter(|n| n.operational).collect();

        for i in 0..node_list.len() {
            for j in (i + 1)..node_list.len() {
                let a = node_list[i];
                let b = node_list[j];

                if a.faction_id == b.faction_id && self.can_connect(a, b) {
                    links.push(PowerTransmissionLink {
                        node_a: a.structure_id,
                        node_b: b.structure_id,
                        pos_a: a.position,
                        pos_b: b.position,
                    });
                }
            }
        }

        links
    }

    /// Helper calculating 2D Euclidean distance squared.
    #[inline]
    fn dist_sq_xz(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
        let dx = a.0 - b.0;
        let dz = a.2 - b.2;
        dx * dx + dz * dz
    }

    /// Determines whether two nodes can form an electrical connection.
    fn can_connect(&self, a: &PowerNode, b: &PowerNode) -> bool {
        let d_sq = Self::dist_sq_xz(a.position, b.position);

        if a.spec.is_relay && b.spec.is_relay {
            let max_r = a.spec.connection_range.max(b.spec.connection_range);
            d_sq <= max_r * max_r
        } else if a.spec.is_relay {
            let max_r = a.spec.distribution_range.max(b.spec.connection_range);
            d_sq <= max_r * max_r
        } else if b.spec.is_relay {
            let max_r = b.spec.distribution_range.max(a.spec.connection_range);
            d_sq <= max_r * max_r
        } else {
            // Direct endpoint-to-endpoint proximity link
            let max_r = a.spec.distribution_range.max(b.spec.distribution_range);
            if max_r > 0.0 {
                d_sq <= max_r * max_r
            } else {
                false
            }
        }
    }

    /// Event-driven graph topology rebuild into connected components (subnets).
    /// Executed ONLY when topology is marked dirty.
    pub fn rebuild_topology(&mut self) {
        self.subnets.clear();
        self.structure_to_grid.clear();
        self.metrics.topology_rebuild_count += 1;

        // Group operational nodes by FactionId
        let mut faction_nodes: BTreeMap<FactionId, Vec<StructureId>> = BTreeMap::new();
        for node in self.nodes.values() {
            if node.operational {
                faction_nodes
                    .entry(node.faction_id)
                    .or_default()
                    .push(node.structure_id);
            }
        }

        for (faction_id, ids) in faction_nodes {
            let mut unvisited: BTreeSet<StructureId> = ids.into_iter().collect();

            while let Some(&start_id) = unvisited.iter().next() {
                unvisited.remove(&start_id);
                let grid_id = PowerGridId::new(self.next_grid_id);
                self.next_grid_id += 1;

                let mut component_nodes = vec![start_id];
                let mut queue = vec![start_id];

                while let Some(curr_id) = queue.pop() {
                    let curr_node = match self.nodes.get(&curr_id) {
                        Some(n) => n.clone(),
                        None => continue,
                    };

                    let mut neighbors = Vec::new();
                    for &candidate_id in &unvisited {
                        if let Some(cand_node) = self.nodes.get(&candidate_id)
                            && self.can_connect(&curr_node, cand_node)
                        {
                            neighbors.push(candidate_id);
                        }
                    }

                    for neighbor_id in neighbors {
                        unvisited.remove(&neighbor_id);
                        component_nodes.push(neighbor_id);
                        queue.push(neighbor_id);
                    }
                }

                for &node_id in &component_nodes {
                    self.structure_to_grid.insert(node_id, grid_id);
                }

                self.subnets.push(PowerSubnet {
                    grid_id,
                    faction_id,
                    nodes: component_nodes,
                    total_generation_kw: 0,
                    total_demand_kw: 0,
                    total_storage_capacity_kwh: 0,
                    total_stored_kwh: 0,
                    battery_charge_rate_kw: 0,
                    battery_discharge_rate_kw: 0,
                    net_balance_kw: 0,
                    satisfaction_ratio: 1.0,
                    status: PowerGridStatus::Optimal,
                });
            }
        }

        self.topology_dirty = false;
    }

    /// Authoritative simulation tick executing supply/demand accounting, battery buffering,
    /// and priority load shedding across all subnets.
    pub fn tick(&mut self, sim_tick: SimTick, journal: &mut EventJournal) {
        self.metrics.ticks_processed += 1;

        if self.topology_dirty {
            self.rebuild_topology();
        }

        let mut total_gen = 0u32;
        let mut total_dem = 0u32;
        let mut total_stored = 0u32;
        let mut powered_cnt = 0usize;
        let mut brownout_cnt = 0usize;
        let mut unpowered_cnt = 0usize;

        // Reset non-operational nodes to unpowered
        for (id, node) in &self.nodes {
            if !node.operational {
                self.structure_power_status
                    .insert(*id, PowerStatus::Unpowered);
                unpowered_cnt += 1;
            }
        }

        for subnet in &mut self.subnets {
            let mut gen_kw = 0u32;
            let mut dem_kw = 0u32;
            let mut capacity_kwh = 0u32;
            let mut stored_kwh = 0u32;
            let mut max_charge_kw = 0u32;
            let mut max_discharge_kw = 0u32;

            // Research power patch for the subnet's owning faction.
            let (generation_milli, efficiency_milli) = self
                .faction_power_modifiers
                .get(&subnet.faction_id)
                .copied()
                .unwrap_or((MODIFIER_SCALE, MODIFIER_SCALE));

            // Step 1: Aggregate baseline generation, demand, and storage capacity
            for &id in &subnet.nodes {
                if let Some(node) = self.nodes.get(&id)
                    && node.operational
                {
                    gen_kw += Self::patched_generation(node.spec.generation_kw, generation_milli);
                    dem_kw += Self::patched_demand(node.spec.demand_kw, efficiency_milli);
                    capacity_kwh += node.spec.storage_capacity_kwh;
                    stored_kwh += node.stored_energy_kwh;
                    max_charge_kw += node.spec.max_charge_rate_kw;
                    max_discharge_kw += node.spec.max_discharge_rate_kw;
                }
            }

            subnet.total_generation_kw = gen_kw;
            subnet.total_demand_kw = dem_kw;
            subnet.total_storage_capacity_kwh = capacity_kwh;
            subnet.total_stored_kwh = stored_kwh;

            let prev_status = self
                .previous_grid_status
                .get(&subnet.grid_id)
                .copied()
                .unwrap_or(PowerGridStatus::Optimal);

            let new_status;
            let available_power: u32;

            if gen_kw >= dem_kw {
                // Surplus power available
                let surplus = gen_kw - dem_kw;
                let charge_budget = surplus.min(max_charge_kw);
                let remaining_capacity = capacity_kwh.saturating_sub(stored_kwh);
                let actual_charge = charge_budget.min(remaining_capacity);

                // Distribute charged energy across batteries in subnet
                if actual_charge > 0 && capacity_kwh > 0 {
                    let mut charged_remaining = actual_charge;
                    for &id in &subnet.nodes {
                        if charged_remaining == 0 {
                            break;
                        }
                        if let Some(node) = self.nodes.get_mut(&id)
                            && node.operational
                            && node.spec.storage_capacity_kwh > 0
                        {
                            let node_room = node
                                .spec
                                .storage_capacity_kwh
                                .saturating_sub(node.stored_energy_kwh);
                            let to_add = charged_remaining
                                .min(node_room)
                                .min(node.spec.max_charge_rate_kw);
                            node.stored_energy_kwh += to_add;
                            charged_remaining -= to_add;
                        }
                    }
                }

                subnet.battery_charge_rate_kw = actual_charge;
                subnet.battery_discharge_rate_kw = 0;
                subnet.net_balance_kw = surplus as i32;
                subnet.satisfaction_ratio = 1.0;
                new_status = PowerGridStatus::Optimal;
                available_power = gen_kw;
            } else {
                // Deficit condition: attempt battery discharge
                let deficit = dem_kw - gen_kw;
                let discharge_budget = deficit.min(max_discharge_kw).min(stored_kwh);

                // Discharge energy from batteries in subnet
                if discharge_budget > 0 {
                    let mut discharge_remaining = discharge_budget;
                    for &id in &subnet.nodes {
                        if discharge_remaining == 0 {
                            break;
                        }
                        if let Some(node) = self.nodes.get_mut(&id)
                            && node.operational
                            && node.stored_energy_kwh > 0
                        {
                            let to_take = discharge_remaining
                                .min(node.stored_energy_kwh)
                                .min(node.spec.max_discharge_rate_kw);
                            node.stored_energy_kwh -= to_take;
                            discharge_remaining -= to_take;
                        }
                    }
                }

                subnet.battery_charge_rate_kw = 0;
                subnet.battery_discharge_rate_kw = discharge_budget;
                let total_available = gen_kw + discharge_budget;
                available_power = total_available;

                if total_available >= dem_kw {
                    subnet.net_balance_kw = 0;
                    subnet.satisfaction_ratio = 1.0;
                    new_status = PowerGridStatus::BatterySupported;
                } else if total_available == 0 {
                    subnet.net_balance_kw = -(dem_kw as i32);
                    subnet.satisfaction_ratio = 0.0;
                    new_status = PowerGridStatus::Blackout;
                } else {
                    subnet.net_balance_kw = total_available as i32 - dem_kw as i32;
                    subnet.satisfaction_ratio = total_available as f32 / dem_kw as f32;
                    new_status = PowerGridStatus::Brownout;
                }
            }

            subnet.status = new_status;
            self.previous_grid_status.insert(subnet.grid_id, new_status);

            // Step 2: Priority-tiered load shedding
            let mut remaining_power = available_power;

            // Group consumers in this subnet by priority: High, Normal, Low
            let mut high_nodes = Vec::new();
            let mut normal_nodes = Vec::new();
            let mut low_nodes = Vec::new();

            for &id in &subnet.nodes {
                if let Some(node) = self.nodes.get(&id) {
                    if node.operational && node.spec.demand_kw > 0 {
                        match node.spec.priority {
                            PowerPriority::High => high_nodes.push(id),
                            PowerPriority::Normal => normal_nodes.push(id),
                            PowerPriority::Low => low_nodes.push(id),
                        }
                    } else if node.operational {
                        // Relays or producers with 0 demand
                        self.structure_power_status
                            .insert(id, PowerStatus::Powered { satisfaction: 1.0 });
                        powered_cnt += 1;
                    }
                }
            }

            // Allocation function for priority tiers
            let allocate_tier =
                |node_ids: &[StructureId],
                 rem_pwr: &mut u32,
                 p_cnt: &mut usize,
                 b_cnt: &mut usize,
                 u_cnt: &mut usize,
                 statuses: &mut BTreeMap<StructureId, PowerStatus>,
                 nodes_map: &BTreeMap<StructureId, PowerNode>| {
                    let tier_demand: u32 = node_ids
                        .iter()
                        .filter_map(|id| nodes_map.get(id))
                        .map(|n| Self::patched_demand(n.spec.demand_kw, efficiency_milli))
                        .sum();

                    if tier_demand == 0 {
                        return;
                    }

                    if *rem_pwr >= tier_demand {
                        for &id in node_ids {
                            statuses.insert(id, PowerStatus::Powered { satisfaction: 1.0 });
                            *p_cnt += 1;
                        }
                        *rem_pwr -= tier_demand;
                    } else if *rem_pwr == 0 {
                        for &id in node_ids {
                            statuses.insert(id, PowerStatus::Unpowered);
                            *u_cnt += 1;
                        }
                    } else {
                        let ratio = *rem_pwr as f32 / tier_demand as f32;
                        for &id in node_ids {
                            statuses.insert(
                                id,
                                PowerStatus::Brownout {
                                    satisfaction: ratio,
                                },
                            );
                            *b_cnt += 1;
                        }
                        *rem_pwr = 0;
                    }
                };

            allocate_tier(
                &high_nodes,
                &mut remaining_power,
                &mut powered_cnt,
                &mut brownout_cnt,
                &mut unpowered_cnt,
                &mut self.structure_power_status,
                &self.nodes,
            );
            allocate_tier(
                &normal_nodes,
                &mut remaining_power,
                &mut powered_cnt,
                &mut brownout_cnt,
                &mut unpowered_cnt,
                &mut self.structure_power_status,
                &self.nodes,
            );
            allocate_tier(
                &low_nodes,
                &mut remaining_power,
                &mut powered_cnt,
                &mut brownout_cnt,
                &mut unpowered_cnt,
                &mut self.structure_power_status,
                &self.nodes,
            );

            // Step 3: Journal status transitions
            if prev_status != new_status {
                match new_status {
                    PowerGridStatus::Brownout => {
                        journal.record(
                            sim_tick,
                            SimEvent::PowerBrownoutStarted {
                                grid_id: subnet.grid_id,
                                satisfaction_ratio: subnet.satisfaction_ratio,
                            },
                        );
                    }
                    PowerGridStatus::Blackout => {
                        journal.record(
                            sim_tick,
                            SimEvent::PowerBlackoutStarted {
                                grid_id: subnet.grid_id,
                            },
                        );
                    }
                    PowerGridStatus::Optimal | PowerGridStatus::BatterySupported => {
                        journal.record(
                            sim_tick,
                            SimEvent::PowerRestored {
                                grid_id: subnet.grid_id,
                            },
                        );
                    }
                }
            }

            total_gen += subnet.total_generation_kw;
            total_dem += subnet.total_demand_kw;
            total_stored += subnet.total_stored_kwh;
        }

        self.metrics.active_subnets = self.subnets.len();
        self.metrics.total_generation_kw = total_gen;
        self.metrics.total_demand_kw = total_dem;
        self.metrics.total_stored_kwh = total_stored;
        self.metrics.powered_count = powered_cnt;
        self.metrics.brownout_count = brownout_cnt;
        self.metrics.unpowered_count = unpowered_cnt;
    }
}
