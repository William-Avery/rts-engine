use game_types::{RegionId, SimTick};

/// Real-time diagnostic telemetry HUD tracking network ping, synchronization, and prediction state.
#[derive(Debug, Clone, PartialEq)]
pub struct DebugHud {
    pub ping_ms: u64,
    pub server_tick: SimTick,
    pub client_tick: SimTick,
    pub region_id: RegionId,
    pub predicted_pos: (f32, f32, f32),
    pub authoritative_pos: (f32, f32, f32),
    pub error_distance: f32,
    pub reconciliation_count: u64,
    pub active_entity_count: usize,
    pub fps: f32,
    pub inventory_slots_used: usize,
    pub inventory_slots_max: usize,
    pub inventory_volume_liters: f32,
    pub inventory_max_volume_liters: u32,
    pub power_generation_kw: u32,
    pub power_demand_kw: u32,
    pub power_stored_kwh: u32,
    pub power_battery_capacity_kwh: u32,
    pub power_subnets_count: usize,
    pub production_facilities_count: usize,
    pub production_cycles_total: u64,
    pub logistics_jobs_active: usize,
    pub logistics_jobs_completed: u64,
    pub logistics_jobs_starved: usize,
    pub logistics_docks_count: usize,
    pub research_techs_completed: usize,
    pub research_techs_total: usize,
    pub research_queue_depth: usize,
    pub research_active_progress: f32,
    pub research_active_modifiers: usize,
    pub camera_mode: crate::camera::CameraMode,
    pub selected_units_count: usize,
}

impl Default for DebugHud {
    fn default() -> Self {
        DebugHud {
            ping_ms: 0,
            server_tick: SimTick::zero(),
            client_tick: SimTick::zero(),
            region_id: RegionId::new(1),
            predicted_pos: (0.0, 0.0, 0.0),
            authoritative_pos: (0.0, 0.0, 0.0),
            error_distance: 0.0,
            reconciliation_count: 0,
            active_entity_count: 0,
            fps: 60.0,
            inventory_slots_used: 0,
            inventory_slots_max: 0,
            inventory_volume_liters: 0.0,
            inventory_max_volume_liters: 0,
            power_generation_kw: 0,
            power_demand_kw: 0,
            power_stored_kwh: 0,
            power_battery_capacity_kwh: 0,
            power_subnets_count: 0,
            production_facilities_count: 0,
            production_cycles_total: 0,
            logistics_jobs_active: 0,
            logistics_jobs_completed: 0,
            logistics_jobs_starved: 0,
            logistics_docks_count: 0,
            research_techs_completed: 0,
            research_techs_total: 0,
            research_queue_depth: 0,
            research_active_progress: 0.0,
            research_active_modifiers: 0,
            camera_mode: crate::camera::CameraMode::ThirdPerson,
            selected_units_count: 0,
        }
    }
}

/// Telemetry snapshot for updating the debug HUD.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TelemetrySnapshot {
    pub ping_ms: u64,
    pub server_tick: SimTick,
    pub client_tick: SimTick,
    pub region_id: RegionId,
    pub predicted_pos: (f32, f32, f32),
    pub authoritative_pos: (f32, f32, f32),
    pub reconciliation_count: u64,
    pub active_entity_count: usize,
}

impl DebugHud {
    pub fn new() -> Self {
        DebugHud::default()
    }

    pub fn update_telemetry(&mut self, snapshot: TelemetrySnapshot) {
        self.ping_ms = snapshot.ping_ms;
        self.server_tick = snapshot.server_tick;
        self.client_tick = snapshot.client_tick;
        self.region_id = snapshot.region_id;
        self.predicted_pos = snapshot.predicted_pos;
        self.authoritative_pos = snapshot.authoritative_pos;
        self.reconciliation_count = snapshot.reconciliation_count;
        self.active_entity_count = snapshot.active_entity_count;

        let dx = snapshot.predicted_pos.0 - snapshot.authoritative_pos.0;
        let dy = snapshot.predicted_pos.1 - snapshot.authoritative_pos.1;
        let dz = snapshot.predicted_pos.2 - snapshot.authoritative_pos.2;
        self.error_distance = (dx * dx + dy * dy + dz * dz).sqrt();
    }

    /// Updates container capacity and storage status metrics.
    pub fn update_inventory(
        &mut self,
        slots_used: usize,
        slots_max: usize,
        vol_liters: f32,
        max_vol_liters: u32,
    ) {
        self.inventory_slots_used = slots_used;
        self.inventory_slots_max = slots_max;
        self.inventory_volume_liters = vol_liters;
        self.inventory_max_volume_liters = max_vol_liters;
    }

    /// Updates power grid telemetry metrics.
    pub fn update_power(
        &mut self,
        gen_kw: u32,
        dem_kw: u32,
        stored_kwh: u32,
        cap_kwh: u32,
        subnets: usize,
    ) {
        self.power_generation_kw = gen_kw;
        self.power_demand_kw = dem_kw;
        self.power_stored_kwh = stored_kwh;
        self.power_battery_capacity_kwh = cap_kwh;
        self.power_subnets_count = subnets;
    }

    /// Updates active industrial production telemetry.
    pub fn update_production(&mut self, facilities_count: usize, cycles_total: u64) {
        self.production_facilities_count = facilities_count;
        self.production_cycles_total = cycles_total;
    }

    /// Renders a single-line summary of vital telemetry.
    pub fn render_compact(&self) -> String {
        format!(
            "[{}] Ping: {}ms | SrvTick: {} | CliTick: {} | Reg: {} | Pred: ({:.2}, {:.2}, {:.2}) | Err: {:.3}m | Reconciles: {} | Sel: {}",
            self.camera_mode.as_str(),
            self.ping_ms,
            self.server_tick.value(),
            self.client_tick.value(),
            self.region_id.value(),
            self.predicted_pos.0,
            self.predicted_pos.1,
            self.predicted_pos.2,
            self.error_distance,
            self.reconciliation_count,
            self.selected_units_count,
        )
    }

    /// Renders a formatted multi-line diagnostic dashboard card.
    pub fn render_ascii_card(&self) -> String {
        let mut out = String::new();
        out.push_str("+--------------------------------------------------------------+\n");
        out.push_str("|                     CLIENT DEBUG TELEMETRY                   |\n");
        out.push_str("+--------------------------------------------------------------+\n");
        out.push_str(&format!(
            "| Network Ping:       {:>5} ms     Active Entities:   {:>7} |\n",
            self.ping_ms, self.active_entity_count
        ));
        out.push_str(&format!(
            "| Server Sim Tick:    {:>8}        Client Sim Tick:   {:>7} |\n",
            self.server_tick.value(),
            self.client_tick.value()
        ));
        out.push_str(&format!(
            "| Active Region ID:   {:>8}        Estimated FPS:     {:>7.1} |\n",
            self.region_id.value(),
            self.fps
        ));
        out.push_str(&format!(
            "| Camera Perspective: {:>12}        Selected Units:    {:>7} |\n",
            self.camera_mode.as_str(),
            self.selected_units_count
        ));
        out.push_str("+--------------------------------------------------------------+\n");
        out.push_str(&format!(
            "| Predicted Pos:     ({:>7.2}, {:>7.2}, {:>7.2})              |\n",
            self.predicted_pos.0, self.predicted_pos.1, self.predicted_pos.2
        ));
        out.push_str(&format!(
            "| Authoritative Pos: ({:>7.2}, {:>7.2}, {:>7.2})              |\n",
            self.authoritative_pos.0, self.authoritative_pos.1, self.authoritative_pos.2
        ));
        out.push_str(&format!(
            "| Prediction Delta:   {:>7.4} m    Reconciliations:   {:>7} |\n",
            self.error_distance, self.reconciliation_count
        ));
        if self.inventory_slots_max > 0 {
            out.push_str("+--------------------------------------------------------------+\n");
            out.push_str(&format!(
                "| Storage Slots:      {:>3} / {:<3}      Volume: {:>6.1} / {:<5} L |\n",
                self.inventory_slots_used,
                self.inventory_slots_max,
                self.inventory_volume_liters,
                self.inventory_max_volume_liters
            ));
        }
        if self.power_generation_kw > 0
            || self.power_demand_kw > 0
            || self.power_battery_capacity_kwh > 0
        {
            out.push_str("+--------------------------------------------------------------+\n");
            let net = self.power_generation_kw as i32 - self.power_demand_kw as i32;
            let bat_pct = if self.power_battery_capacity_kwh > 0 {
                (self.power_stored_kwh as f32 / self.power_battery_capacity_kwh as f32) * 100.0
            } else {
                0.0
            };
            out.push_str(&format!(
                "| Power: {:>5} kW Gen | {:>5} kW Dem | Net: {:>+5} kW | Grids: {:>2} |\n",
                self.power_generation_kw, self.power_demand_kw, net, self.power_subnets_count
            ));
            out.push_str(&format!(
                "| Battery Buffer: {:>5} / {:<5} kWh ({:>5.1}%)                     |\n",
                self.power_stored_kwh, self.power_battery_capacity_kwh, bat_pct
            ));
        }
        if self.production_facilities_count > 0 {
            out.push_str("+--------------------------------------------------------------+\n");
            out.push_str(&format!(
                "| Facilities: {:>3} active | Cycles Completed: {:>8}             |\n",
                self.production_facilities_count, self.production_cycles_total
            ));
        }
        if self.research_techs_total > 0 {
            out.push_str("+--------------------------------------------------------------+\n");
            out.push_str(&format!(
                "| Research: {:>3} / {:<3} done | Queue: {:>2} | Progress: {:>5.1}%     |\n",
                self.research_techs_completed,
                self.research_techs_total,
                self.research_queue_depth,
                self.research_active_progress * 100.0
            ));
            out.push_str(&format!(
                "| Active Software Patches (modifier kinds): {:>3}                |\n",
                self.research_active_modifiers
            ));
        }
        out.push_str("+--------------------------------------------------------------+\n");
        out
    }
}
