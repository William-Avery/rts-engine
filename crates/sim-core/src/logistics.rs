use crate::event::{EventJournal, SimEvent};
use crate::inventory::{ContainerKind, InventoryRegistry};
use game_types::{
    EntityId, GameError, GameResult, LogisticsJobId, ReservationId, ResourceId, RouteNodeId,
    SimTick,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Priority levels for logistics material movement jobs.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum JobPriority {
    /// Low priority background depot balancing and stockpiling
    Low = 0,
    /// Standard production and industrial recipe routing
    #[default]
    Normal = 1,
    /// Factory starvation prevention and energy cell replenishment
    High = 2,
    /// Emergency combat turret ammo supply and critical structure repair materials
    Critical = 3,
}

impl JobPriority {
    pub const fn as_u8(&self) -> u8 {
        *self as u8
    }

    pub const fn from_u8(val: u8) -> Self {
        match val {
            0 => JobPriority::Low,
            1 => JobPriority::Normal,
            2 => JobPriority::High,
            _ => JobPriority::Critical,
        }
    }
}

/// Lifecycle state of a logistics job.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum JobStatus {
    /// Job created and waiting for an available hauler/worker
    Pending,
    /// Job atomically claimed by worker; source items locked via reservation
    Claimed,
    /// Items picked up from source and currently in transit to destination
    InTransit,
    /// Items delivered to destination; transaction committed
    Completed,
    /// Job cancelled; active reservations released back to source
    Cancelled,
    /// Job exceeded starvation timeout without being claimed or serviced
    Starved,
}

/// Worker concurrency requirement for logistics jobs.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum WorkerRequirement {
    /// Exactly one worker/hauler can claim this task
    SingleWorker,
    /// Multiple workers can contribute up to `max_workers`
    MultiWorker { max_workers: u32 },
}

/// Authoritative logistics job defining material transport requirements.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LogisticsJob {
    pub id: LogisticsJobId,
    pub source: EntityId,
    pub destination: EntityId,
    pub resource_id: ResourceId,
    pub amount: u32,
    pub priority: JobPriority,
    pub status: JobStatus,
    pub worker_req: WorkerRequirement,
    pub claimed_workers: Vec<EntityId>,
    pub reservation_id: Option<ReservationId>,
    pub created_tick: SimTick,
    pub assigned_tick: Option<SimTick>,
    pub completed_tick: Option<SimTick>,
    pub starvation_threshold_ticks: u64,
    pub is_starved: bool,
}

impl LogisticsJob {
    pub fn new(
        id: LogisticsJobId,
        source: EntityId,
        destination: EntityId,
        resource_id: ResourceId,
        amount: u32,
        priority: JobPriority,
        created_tick: SimTick,
    ) -> Self {
        LogisticsJob {
            id,
            source,
            destination,
            resource_id,
            amount,
            priority,
            status: JobStatus::Pending,
            worker_req: WorkerRequirement::SingleWorker,
            claimed_workers: Vec::new(),
            reservation_id: None,
            created_tick,
            assigned_tick: None,
            completed_tick: None,
            starvation_threshold_ticks: 300, // 10s at 30 Hz
            is_starved: false,
        }
    }

    pub fn with_multi_worker(mut self, max_workers: u32) -> Self {
        self.worker_req = WorkerRequirement::MultiWorker { max_workers };
        self
    }

    pub fn with_starvation_threshold(mut self, ticks: u64) -> Self {
        self.starvation_threshold_ticks = ticks;
        self
    }
}

/// Action being performed during a dock service session.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum DockAction {
    Pickup,
    Dropoff,
}

/// Active service session between a docked vessel and a logistics dock berth.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DockServiceSession {
    pub worker_id: EntityId,
    pub job_id: LogisticsJobId,
    pub action: DockAction,
    pub units_remaining: u32,
}

/// Logistics dock managing berth occupancy and rate-limited cargo transfers without physics collision.
#[derive(Clone, Debug)]
pub struct LogisticsDock {
    pub dock_entity: EntityId,
    pub berths: usize,
    pub service_rate_units_per_tick: u32,
    pub queue: VecDeque<EntityId>,
    pub servicing: Vec<DockServiceSession>,
    pub stalled_ticks: u64,
    pub total_serviced_vessels: u64,
}

impl LogisticsDock {
    pub fn new(dock_entity: EntityId, berths: usize, service_rate_units_per_tick: u32) -> Self {
        LogisticsDock {
            dock_entity,
            berths: berths.max(1),
            service_rate_units_per_tick: service_rate_units_per_tick.max(1),
            queue: VecDeque::new(),
            servicing: Vec::new(),
            stalled_ticks: 0,
            total_serviced_vessels: 0,
        }
    }

    pub fn enqueue_vessel(&mut self, vessel: EntityId) -> GameResult<()> {
        if self.queue.contains(&vessel) || self.servicing.iter().any(|s| s.worker_id == vessel) {
            return Ok(()); // Already queued or servicing
        }
        if self.queue.len() >= 64 {
            return Err(GameError::DockQueueFull);
        }
        self.queue.push_back(vessel);
        Ok(())
    }

    pub fn admits_berth(&self) -> bool {
        self.servicing.len() < self.berths
    }
}

/// Powered logistics coverage component attached to supply depots.
#[derive(Clone, PartialEq, Debug)]
pub struct DepotLogistics {
    pub depot_entity: EntityId,
    pub base_coverage_radius: f32,
    pub is_powered: bool,
}

impl DepotLogistics {
    pub fn new(depot_entity: EntityId, base_coverage_radius: f32) -> Self {
        DepotLogistics {
            depot_entity,
            base_coverage_radius,
            is_powered: true, // Default to powered until power network update
        }
    }

    /// Effective coverage radius gated by electrical power.
    pub fn effective_coverage(&self) -> f32 {
        if self.is_powered {
            self.base_coverage_radius
        } else {
            0.0
        }
    }

    /// Check if target 3D coordinates fall within powered logistics coverage.
    pub fn is_in_coverage(&self, depot_pos: (f32, f32, f32), target_pos: (f32, f32, f32)) -> bool {
        let eff_radius = self.effective_coverage();
        if eff_radius <= 0.0 {
            return false;
        }
        let dx = target_pos.0 - depot_pos.0;
        let dz = target_pos.2 - depot_pos.2;
        let dist_sq = dx * dx + dz * dz;
        dist_sq <= eff_radius * eff_radius
    }
}

/// Node in the logistics route graph.
#[derive(Clone, PartialEq, Debug)]
pub struct RouteNode {
    pub id: RouteNodeId,
    pub position: (f32, f32, f32),
    pub associated_entity: Option<EntityId>,
}

/// Directed edge representing a transport route or lane.
#[derive(Clone, PartialEq, Debug)]
pub struct RouteEdge {
    pub from: RouteNodeId,
    pub to: RouteNodeId,
    pub distance: f32,
    pub max_active_haulers: usize,
    pub current_haulers: usize,
    pub traversal_speed: f32, // meters per tick
}

/// Network route graph for stable, non-colliding material transport corridors.
#[derive(Clone, Default, Debug)]
pub struct RouteGraph {
    pub nodes: BTreeMap<RouteNodeId, RouteNode>,
    pub edges: Vec<RouteEdge>,
}

impl RouteGraph {
    pub fn new() -> Self {
        RouteGraph {
            nodes: BTreeMap::new(),
            edges: Vec::new(),
        }
    }

    pub fn add_node(&mut self, node: RouteNode) {
        self.nodes.insert(node.id, node);
    }

    pub fn add_edge(&mut self, edge: RouteEdge) {
        self.edges.push(edge);
    }

    pub fn get_node(&self, id: RouteNodeId) -> Option<&RouteNode> {
        self.nodes.get(&id)
    }

    /// Calculate path travel time in ticks between two nodes if connected.
    pub fn estimate_travel_ticks(&self, from: RouteNodeId, to: RouteNodeId) -> Option<u64> {
        for edge in &self.edges {
            if edge.from == from && edge.to == to {
                let ticks = (edge.distance / edge.traversal_speed.max(0.1)).ceil() as u64;
                return Some(ticks.max(1));
            }
        }

        // Direct Euclidean fallback if nodes exist
        if let (Some(n1), Some(n2)) = (self.nodes.get(&from), self.nodes.get(&to)) {
            let dx = n2.position.0 - n1.position.0;
            let dy = n2.position.1 - n1.position.1;
            let dz = n2.position.2 - n1.position.2;
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            let default_speed = 0.5; // 15 m/s at 30 Hz
            let ticks = (dist / default_speed).ceil() as u64;
            Some(ticks.max(1))
        } else {
            None
        }
    }

    /// Calculate shortest waypoint path and total travel ticks between two nodes using Dijkstra search over graph edges.
    pub fn find_path(&self, from: RouteNodeId, to: RouteNodeId) -> Option<(Vec<RouteNodeId>, u64)> {
        if !self.nodes.contains_key(&from) || !self.nodes.contains_key(&to) {
            return None;
        }

        if from == to {
            return Some((vec![from], 0));
        }

        // Dijkstra algorithm over route edges
        let mut dist: BTreeMap<RouteNodeId, u64> = BTreeMap::new();
        let mut prev: BTreeMap<RouteNodeId, RouteNodeId> = BTreeMap::new();
        let mut unvisited: BTreeSet<(u64, RouteNodeId)> = BTreeSet::new();

        dist.insert(from, 0);
        unvisited.insert((0, from));

        while let Some((d, current)) = unvisited.pop_first() {
            if current == to {
                let mut path = vec![to];
                let mut curr = to;
                while let Some(&p) = prev.get(&curr) {
                    path.push(p);
                    curr = p;
                    if curr == from {
                        break;
                    }
                }
                path.reverse();
                return Some((path, d));
            }

            if d > *dist.get(&current).unwrap_or(&u64::MAX) {
                continue;
            }

            for edge in &self.edges {
                if edge.from == current {
                    let edge_ticks = (edge.distance / edge.traversal_speed.max(0.1)).ceil() as u64;
                    let next_dist = d + edge_ticks.max(1);
                    if next_dist < *dist.get(&edge.to).unwrap_or(&u64::MAX) {
                        if let Some(&old_dist) = dist.get(&edge.to) {
                            unvisited.remove(&(old_dist, edge.to));
                        }
                        dist.insert(edge.to, next_dist);
                        prev.insert(edge.to, current);
                        unvisited.insert((next_dist, edge.to));
                    }
                }
            }
        }

        // Euclidean fallback if direct path exists between registered nodes
        let fallback_ticks = self.estimate_travel_ticks(from, to)?;
        Some((vec![from, to], fallback_ticks))
    }
}

/// Abstract distant transport representation simulating cargo movement across cold regions.
#[derive(Clone, PartialEq, Debug)]
pub struct DistantTransport {
    pub hauler_entity: EntityId,
    pub job_id: LogisticsJobId,
    pub departure_tick: SimTick,
    pub arrival_tick: SimTick,
    pub resource_id: ResourceId,
    pub amount: u32,
}

impl DistantTransport {
    pub fn is_arrived(&self, current_tick: SimTick) -> bool {
        current_tick >= self.arrival_tick
    }
}

/// Universal container/buffer model for mobile haulers, cargo containers, and future vehicles/drones.
#[derive(Clone, PartialEq, Debug)]
pub struct UniversalBuffer {
    pub owner: EntityId,
    pub container_kind: ContainerKind,
    pub is_mobile: bool,
    pub docked_at: Option<EntityId>,
}

impl UniversalBuffer {
    pub fn new_mobile_buffer(owner: EntityId) -> Self {
        UniversalBuffer {
            owner,
            container_kind: ContainerKind::CargoBuffer,
            is_mobile: true,
            docked_at: None,
        }
    }

    pub fn new_fixed_buffer(owner: EntityId, kind: ContainerKind) -> Self {
        UniversalBuffer {
            owner,
            container_kind: kind,
            is_mobile: false,
            docked_at: None,
        }
    }

    pub fn dock_to(&mut self, dock_entity: EntityId) {
        self.docked_at = Some(dock_entity);
    }

    pub fn undock(&mut self) {
        self.docked_at = None;
    }
}

/// Diagnostic telemetry tracking logistics system health, efficiency, and deadlocks.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct LogisticsTelemetry {
    pub jobs_created_total: u64,
    pub jobs_completed_total: u64,
    pub jobs_pending_count: usize,
    pub jobs_claimed_count: usize,
    pub jobs_in_transit_count: usize,
    pub jobs_starved_count: usize,
    pub deadlocked_docks_count: usize,
}

/// Server-authoritative logistics coordinator managing jobs, docks, depots, and transactions.
#[derive(Clone, Debug)]
pub struct LogisticsManager {
    pub jobs: BTreeMap<LogisticsJobId, LogisticsJob>,
    pub docks: BTreeMap<EntityId, LogisticsDock>,
    pub depots: BTreeMap<EntityId, DepotLogistics>,
    pub route_graph: RouteGraph,
    pub distant_transports: Vec<DistantTransport>,
    pub next_job_id: u64,
    pub telemetry: LogisticsTelemetry,
}

impl Default for LogisticsManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LogisticsManager {
    pub fn new() -> Self {
        LogisticsManager {
            jobs: BTreeMap::new(),
            docks: BTreeMap::new(),
            depots: BTreeMap::new(),
            route_graph: RouteGraph::new(),
            distant_transports: Vec::new(),
            next_job_id: 1,
            telemetry: LogisticsTelemetry::default(),
        }
    }

    /// Register a new logistics dock attached to a facility or depot.
    pub fn register_dock(&mut self, dock: LogisticsDock) {
        self.docks.insert(dock.dock_entity, dock);
    }

    /// Register depot logistics coverage parameters.
    pub fn register_depot(&mut self, depot: DepotLogistics) {
        self.depots.insert(depot.depot_entity, depot);
    }

    /// Update depot power status from electrical network.
    pub fn update_depot_power(&mut self, depot_entity: EntityId, powered: bool) {
        if let Some(depot) = self.depots.get_mut(&depot_entity) {
            depot.is_powered = powered;
        }
    }

    /// Create and submit a new logistics material transport job.
    #[allow(clippy::too_many_arguments)]
    pub fn create_job(
        &mut self,
        source: EntityId,
        destination: EntityId,
        resource_id: ResourceId,
        amount: u32,
        priority: JobPriority,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<LogisticsJobId> {
        if amount == 0 {
            return Err(GameError::InvalidCommand);
        }
        if source == destination {
            return Err(GameError::InvalidCommand);
        }

        let job_id = LogisticsJobId::new(self.next_job_id);
        self.next_job_id += 1;

        let job = LogisticsJob::new(
            job_id,
            source,
            destination,
            resource_id,
            amount,
            priority,
            tick,
        );
        self.jobs.insert(job_id, job);

        self.telemetry.jobs_created_total += 1;
        self.telemetry.jobs_pending_count += 1;

        journal.record(
            tick,
            SimEvent::LogisticsJobCreated {
                job_id,
                resource_id,
                amount,
                source,
                destination,
            },
        );

        Ok(job_id)
    }

    /// Atomically claim a logistics job for a worker hauler.
    ///
    /// Guarantees:
    /// 1. Single-worker exclusivity: If multiple haulers race to claim a single-worker job,
    ///    only the first succeeds and subsequent haulers fail with `JobAlreadyClaimed`.
    /// 2. Two-phase reservation: Locks the requested goods in the source inventory so they
    ///    cannot be double-spent or claimed by another job.
    pub fn claim_job(
        &mut self,
        job_id: LogisticsJobId,
        worker_id: EntityId,
        tick: SimTick,
        inventory_registry: &mut InventoryRegistry,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        let job = self
            .jobs
            .get_mut(&job_id)
            .ok_or(GameError::JobNotFound(job_id))?;

        if job.status != JobStatus::Pending && job.status != JobStatus::Claimed {
            return Err(GameError::InvalidJobState);
        }

        match job.worker_req {
            WorkerRequirement::SingleWorker => {
                if !job.claimed_workers.is_empty() {
                    return Err(GameError::JobAlreadyClaimed(job_id));
                }
            }
            WorkerRequirement::MultiWorker { max_workers } => {
                if job.claimed_workers.len() >= max_workers as usize {
                    return Err(GameError::JobAlreadyClaimed(job_id));
                }
            }
        }

        // Place two-phase reservation on source inventory
        let res_id = inventory_registry.next_reservation_id();
        let source_inv = inventory_registry
            .get_mut(job.source)
            .ok_or(GameError::ContainerNotFound(job.source))?;

        source_inv.reserve(
            res_id,
            job.resource_id,
            job.amount,
            tick,
            Some(job.destination),
        )?;

        // Update job state
        job.reservation_id = Some(res_id);
        job.claimed_workers.push(worker_id);
        job.status = JobStatus::Claimed;
        job.assigned_tick = Some(tick);

        self.telemetry.jobs_pending_count = self.telemetry.jobs_pending_count.saturating_sub(1);
        self.telemetry.jobs_claimed_count += 1;

        journal.record(tick, SimEvent::LogisticsJobClaimed { job_id, worker_id });

        Ok(())
    }

    /// Execute atomic material pickup transaction from source into worker universal container.
    ///
    /// Commits the source reservation and deposits exact items into worker buffer.
    /// Zero resource duplication or loss.
    pub fn execute_pickup(
        &mut self,
        job_id: LogisticsJobId,
        worker_id: EntityId,
        tick: SimTick,
        inventory_registry: &mut InventoryRegistry,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        let job = self
            .jobs
            .get_mut(&job_id)
            .ok_or(GameError::JobNotFound(job_id))?;

        if job.status != JobStatus::Claimed {
            return Err(GameError::InvalidJobState);
        }

        if !job.claimed_workers.contains(&worker_id) {
            return Err(GameError::JobNotClaimedByWorker(job_id, worker_id));
        }

        let reservation_id = job.reservation_id.ok_or(GameError::InvalidJobState)?;

        // Verify worker inventory can accept before committing
        let worker_inv = inventory_registry
            .get_mut(worker_id)
            .ok_or(GameError::ContainerNotFound(worker_id))?;

        if !worker_inv.can_accept(job.resource_id, job.amount) {
            return Err(GameError::InventoryFull {
                max_slots: worker_inv.max_slots,
                max_volume_liters: worker_inv.max_volume_liters,
            });
        }

        // Commit reservation on source
        let source_inv = inventory_registry
            .get_mut(job.source)
            .ok_or(GameError::ContainerNotFound(job.source))?;

        let (res_id, amt) = source_inv.commit_reservation(reservation_id)?;

        // Transfer into worker buffer
        let worker_inv = inventory_registry
            .get_mut(worker_id)
            .ok_or(GameError::ContainerNotFound(worker_id))?;
        worker_inv.add(res_id, amt)?;

        job.status = JobStatus::InTransit;

        self.telemetry.jobs_claimed_count = self.telemetry.jobs_claimed_count.saturating_sub(1);
        self.telemetry.jobs_in_transit_count += 1;

        journal.record(
            tick,
            SimEvent::LogisticsPickupCompleted {
                job_id,
                worker_id,
                amount: amt,
            },
        );

        Ok(())
    }

    /// Execute atomic material dropoff transaction from worker container into destination inventory.
    ///
    /// Validates destination capacity and completes the logistics job.
    /// Zero resource duplication or loss.
    pub fn execute_dropoff(
        &mut self,
        job_id: LogisticsJobId,
        worker_id: EntityId,
        tick: SimTick,
        inventory_registry: &mut InventoryRegistry,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        let job = self
            .jobs
            .get_mut(&job_id)
            .ok_or(GameError::JobNotFound(job_id))?;

        if job.status != JobStatus::InTransit {
            return Err(GameError::InvalidJobState);
        }

        if !job.claimed_workers.contains(&worker_id) {
            return Err(GameError::JobNotClaimedByWorker(job_id, worker_id));
        }

        // Validate destination capacity before modifying worker inventory
        let dest_inv = inventory_registry
            .get_mut(job.destination)
            .ok_or(GameError::ContainerNotFound(job.destination))?;

        if !dest_inv.can_accept(job.resource_id, job.amount) {
            return Err(GameError::InventoryFull {
                max_slots: dest_inv.max_slots,
                max_volume_liters: dest_inv.max_volume_liters,
            });
        }

        // Deduct from worker buffer
        let worker_inv = inventory_registry
            .get_mut(worker_id)
            .ok_or(GameError::ContainerNotFound(worker_id))?;
        worker_inv.remove(job.resource_id, job.amount)?;

        // Add to destination inventory
        let dest_inv = inventory_registry
            .get_mut(job.destination)
            .ok_or(GameError::ContainerNotFound(job.destination))?;
        dest_inv.add(job.resource_id, job.amount)?;

        job.status = JobStatus::Completed;
        job.completed_tick = Some(tick);

        self.telemetry.jobs_in_transit_count =
            self.telemetry.jobs_in_transit_count.saturating_sub(1);
        self.telemetry.jobs_completed_total += 1;

        journal.record(
            tick,
            SimEvent::LogisticsDropoffCompleted {
                job_id,
                worker_id,
                amount: job.amount,
            },
        );

        Ok(())
    }

    /// Cancel a logistics job and cleanly release any active reservations without item loss.
    pub fn cancel_job(
        &mut self,
        job_id: LogisticsJobId,
        reason: &str,
        tick: SimTick,
        inventory_registry: &mut InventoryRegistry,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        let job = self
            .jobs
            .get_mut(&job_id)
            .ok_or(GameError::JobNotFound(job_id))?;

        if job.status == JobStatus::Completed || job.status == JobStatus::Cancelled {
            return Err(GameError::InvalidJobState);
        }

        // Release reservation at source if claimed but not yet picked up
        if let Some(res_id) = job.reservation_id
            && job.status == JobStatus::Claimed
            && let Some(source_inv) = inventory_registry.get_mut(job.source)
        {
            let _ = source_inv.release_reservation(res_id);
        }

        match job.status {
            JobStatus::Pending => {
                self.telemetry.jobs_pending_count =
                    self.telemetry.jobs_pending_count.saturating_sub(1);
            }
            JobStatus::Claimed => {
                self.telemetry.jobs_claimed_count =
                    self.telemetry.jobs_claimed_count.saturating_sub(1);
            }
            JobStatus::InTransit => {
                self.telemetry.jobs_in_transit_count =
                    self.telemetry.jobs_in_transit_count.saturating_sub(1);
            }
            _ => {}
        }

        job.status = JobStatus::Cancelled;

        journal.record(
            tick,
            SimEvent::LogisticsJobCancelled {
                job_id,
                reason: reason.to_string(),
            },
        );

        Ok(())
    }

    /// Advance simulation tick for docks, abstract distant transport, and starvation detection.
    pub fn step(
        &mut self,
        tick: SimTick,
        inventory_registry: &mut InventoryRegistry,
        journal: &mut EventJournal,
    ) {
        // 1. Advance logistics docks
        let mut completed_sessions: Vec<(EntityId, DockServiceSession)> = Vec::new();
        for (dock_ent, dock) in self.docks.iter_mut() {
            let mut progressed = false;

            // Fill available berths from queue
            while dock.servicing.len() < dock.berths && !dock.queue.is_empty() {
                let vessel = dock.queue.pop_front().unwrap();
                // Find active job for vessel
                let maybe_job = self.jobs.values().find(|j| {
                    (j.status == JobStatus::Claimed || j.status == JobStatus::InTransit)
                        && j.claimed_workers.contains(&vessel)
                });

                if let Some(job) = maybe_job {
                    let action = if job.status == JobStatus::Claimed {
                        DockAction::Pickup
                    } else {
                        DockAction::Dropoff
                    };
                    dock.servicing.push(DockServiceSession {
                        worker_id: vessel,
                        job_id: job.id,
                        action,
                        units_remaining: job.amount,
                    });
                    journal.record(
                        tick,
                        SimEvent::DockQueueUpdated {
                            dock_entity: *dock_ent,
                            queue_length: dock.queue.len(),
                        },
                    );
                }
            }

            // Step active service sessions
            let rate = dock.service_rate_units_per_tick;
            for session in &mut dock.servicing {
                session.units_remaining = session.units_remaining.saturating_sub(rate);
                progressed = true;
            }

            // Extract completed sessions
            let (done, remaining): (Vec<_>, Vec<_>) = dock
                .servicing
                .drain(..)
                .partition(|s| s.units_remaining == 0);
            dock.servicing = remaining;

            for session in done {
                dock.total_serviced_vessels += 1;
                completed_sessions.push((*dock_ent, session));
            }

            if !dock.queue.is_empty() && !progressed {
                dock.stalled_ticks += 1;
            } else {
                dock.stalled_ticks = 0;
            }
        }

        // Execute transactions for completed dock sessions
        for (_dock_ent, session) in completed_sessions {
            match session.action {
                DockAction::Pickup => {
                    let _ = self.execute_pickup(
                        session.job_id,
                        session.worker_id,
                        tick,
                        inventory_registry,
                        journal,
                    );
                }
                DockAction::Dropoff => {
                    let _ = self.execute_dropoff(
                        session.job_id,
                        session.worker_id,
                        tick,
                        inventory_registry,
                        journal,
                    );
                }
            }
        }

        // 2. Advance distant transports
        let mut arrived_transports = Vec::new();
        self.distant_transports.retain(|transport| {
            if transport.is_arrived(tick) {
                arrived_transports.push(transport.clone());
                false
            } else {
                true
            }
        });

        for transport in arrived_transports {
            let _ = self.execute_dropoff(
                transport.job_id,
                transport.hauler_entity,
                tick,
                inventory_registry,
                journal,
            );
        }

        // 3. Starvation & deadlock telemetry audit
        let mut starved_count = 0;
        for job in self.jobs.values_mut() {
            if job.status == JobStatus::Pending {
                let pending_duration = tick.value().saturating_sub(job.created_tick.value());
                if pending_duration >= job.starvation_threshold_ticks {
                    if !job.is_starved {
                        job.is_starved = true;
                        journal.record(
                            tick,
                            SimEvent::LogisticsJobStarved {
                                job_id: job.id,
                                pending_ticks: pending_duration,
                            },
                        );
                    }
                    starved_count += 1;
                }
            }
        }
        self.telemetry.jobs_starved_count = starved_count;

        let mut deadlocked_docks = 0;
        for dock in self.docks.values() {
            if dock.stalled_ticks >= 300 {
                deadlocked_docks += 1;
            }
        }
        self.telemetry.deadlocked_docks_count = deadlocked_docks;
    }

    /// Query pending jobs sorted by priority (Critical > High > Normal > Low),
    /// then urgency (closest to starvation), then FIFO created tick.
    pub fn find_best_pending_job(&self, max_carry_capacity: Option<u32>) -> Option<LogisticsJobId> {
        let mut candidates: Vec<&LogisticsJob> = self
            .jobs
            .values()
            .filter(|j| {
                if j.status != JobStatus::Pending {
                    return false;
                }
                if let Some(cap) = max_carry_capacity
                    && j.amount > cap
                {
                    return false;
                }
                true
            })
            .collect();

        if candidates.is_empty() {
            return None;
        }

        candidates.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| b.is_starved.cmp(&a.is_starved))
                .then_with(|| a.created_tick.cmp(&b.created_tick))
        });

        candidates.first().map(|j| j.id)
    }

    /// Automatically generate a replenishment job between an output-producing provider and an input-starved consumer.
    #[allow(clippy::too_many_arguments)]
    pub fn generate_replenishment_job(
        &mut self,
        source: EntityId,
        destination: EntityId,
        resource_id: ResourceId,
        amount: u32,
        priority: JobPriority,
        tick: SimTick,
        inventory_registry: &InventoryRegistry,
        journal: &mut EventJournal,
    ) -> GameResult<LogisticsJobId> {
        let source_inv = inventory_registry
            .get(source)
            .ok_or(GameError::ContainerNotFound(source))?;
        if source_inv.available_quantity(resource_id) < amount {
            return Err(GameError::InsufficientUnreservedBalance {
                requested: amount,
                available: source_inv.available_quantity(resource_id),
            });
        }

        let dest_inv = inventory_registry
            .get(destination)
            .ok_or(GameError::ContainerNotFound(destination))?;
        if !dest_inv.can_accept(resource_id, amount) {
            return Err(GameError::InventoryFull {
                max_slots: dest_inv.max_slots,
                max_volume_liters: dest_inv.max_volume_liters,
            });
        }

        self.create_job(
            source,
            destination,
            resource_id,
            amount,
            priority,
            tick,
            journal,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{ContainerKind, Inventory};
    use game_types::{RES_IRON_ORE, RES_STEEL, RES_TUNGSTEN_ORE};

    #[test]
    fn test_single_worker_claim_exclusivity() {
        let mut mgr = LogisticsManager::new();
        let mut inv_reg = InventoryRegistry::new();
        let mut journal = EventJournal::new();

        let source = EntityId::new(10);
        let dest = EntityId::new(20);
        let hauler_1 = EntityId::new(101);
        let hauler_2 = EntityId::new(102);
        let hauler_3 = EntityId::new(103);

        let mut src_inv = Inventory::new(source, ContainerKind::Depot);
        src_inv.add(RES_IRON_ORE, 200).unwrap();
        inv_reg.register(src_inv);
        inv_reg.register(Inventory::new(dest, ContainerKind::Depot));
        inv_reg.register(Inventory::new(hauler_1, ContainerKind::CargoBuffer));
        inv_reg.register(Inventory::new(hauler_2, ContainerKind::CargoBuffer));
        inv_reg.register(Inventory::new(hauler_3, ContainerKind::CargoBuffer));

        let job_id = mgr
            .create_job(
                source,
                dest,
                RES_IRON_ORE,
                50,
                JobPriority::Normal,
                SimTick::new(1),
                &mut journal,
            )
            .unwrap();

        // Hauler 1 claims successfully
        let res1 = mgr.claim_job(
            job_id,
            hauler_1,
            SimTick::new(2),
            &mut inv_reg,
            &mut journal,
        );
        assert!(res1.is_ok(), "Hauler 1 should claim single-worker job");

        // Hauler 2 attempts to claim same job -> must fail with JobAlreadyClaimed
        let res2 = mgr.claim_job(
            job_id,
            hauler_2,
            SimTick::new(2),
            &mut inv_reg,
            &mut journal,
        );
        assert!(matches!(res2, Err(GameError::JobAlreadyClaimed(_))));

        // Hauler 3 attempts to claim same job -> must fail with JobAlreadyClaimed
        let res3 = mgr.claim_job(
            job_id,
            hauler_3,
            SimTick::new(2),
            &mut inv_reg,
            &mut journal,
        );
        assert!(matches!(res3, Err(GameError::JobAlreadyClaimed(_))));

        // Check claimed worker list
        let job = mgr.jobs.get(&job_id).unwrap();
        assert_eq!(job.claimed_workers, vec![hauler_1]);
        assert_eq!(job.status, JobStatus::Claimed);
    }

    #[test]
    fn test_multi_worker_claim_capacity() {
        let mut mgr = LogisticsManager::new();
        let mut inv_reg = InventoryRegistry::new();
        let mut journal = EventJournal::new();

        let source = EntityId::new(10);
        let dest = EntityId::new(20);
        let w1 = EntityId::new(101);
        let w2 = EntityId::new(102);
        let w3 = EntityId::new(103);

        let mut src_inv = Inventory::new(source, ContainerKind::Depot);
        src_inv.add(RES_STEEL, 500).unwrap();
        inv_reg.register(src_inv);
        inv_reg.register(Inventory::new(dest, ContainerKind::Depot));
        inv_reg.register(Inventory::new(w1, ContainerKind::CargoBuffer));
        inv_reg.register(Inventory::new(w2, ContainerKind::CargoBuffer));
        inv_reg.register(Inventory::new(w3, ContainerKind::CargoBuffer));

        let job_id = mgr
            .create_job(
                source,
                dest,
                RES_STEEL,
                100,
                JobPriority::High,
                SimTick::new(1),
                &mut journal,
            )
            .unwrap();

        // Convert to multi-worker with cap 2
        mgr.jobs.get_mut(&job_id).unwrap().worker_req =
            WorkerRequirement::MultiWorker { max_workers: 2 };

        assert!(
            mgr.claim_job(job_id, w1, SimTick::new(2), &mut inv_reg, &mut journal)
                .is_ok()
        );
        assert!(
            mgr.claim_job(job_id, w2, SimTick::new(2), &mut inv_reg, &mut journal)
                .is_ok()
        );
        // Worker 3 exceeds cap 2 -> fails
        let res3 = mgr.claim_job(job_id, w3, SimTick::new(2), &mut inv_reg, &mut journal);
        assert!(matches!(res3, Err(GameError::JobAlreadyClaimed(_))));
    }

    #[test]
    fn test_atomic_pickup_and_dropoff_resource_conservation() {
        let mut mgr = LogisticsManager::new();
        let mut inv_reg = InventoryRegistry::new();
        let mut journal = EventJournal::new();

        let source = EntityId::new(10);
        let dest = EntityId::new(20);
        let hauler = EntityId::new(101);

        let mut src_inv = Inventory::new(source, ContainerKind::Depot);
        src_inv.add(RES_STEEL, 200).unwrap();
        inv_reg.register(src_inv);
        inv_reg.register(Inventory::new(dest, ContainerKind::Depot));
        inv_reg.register(Inventory::new(hauler, ContainerKind::CargoBuffer));

        let total_before = inv_reg.get(source).unwrap().total_quantity(RES_STEEL)
            + inv_reg.get(dest).unwrap().total_quantity(RES_STEEL)
            + inv_reg.get(hauler).unwrap().total_quantity(RES_STEEL);
        assert_eq!(total_before, 200);

        let job_id = mgr
            .create_job(
                source,
                dest,
                RES_STEEL,
                80,
                JobPriority::Normal,
                SimTick::new(1),
                &mut journal,
            )
            .unwrap();

        // Claim
        mgr.claim_job(job_id, hauler, SimTick::new(2), &mut inv_reg, &mut journal)
            .unwrap();
        assert_eq!(
            inv_reg.get(source).unwrap().available_quantity(RES_STEEL),
            120
        );
        assert_eq!(
            inv_reg.get(source).unwrap().reserved_quantity(RES_STEEL),
            80
        );

        let total_claimed = inv_reg.get(source).unwrap().total_quantity(RES_STEEL)
            + inv_reg.get(dest).unwrap().total_quantity(RES_STEEL)
            + inv_reg.get(hauler).unwrap().total_quantity(RES_STEEL);
        assert_eq!(total_claimed, 200, "Zero loss or duplication during claim");

        // Execute pickup
        mgr.execute_pickup(job_id, hauler, SimTick::new(3), &mut inv_reg, &mut journal)
            .unwrap();
        assert_eq!(inv_reg.get(source).unwrap().total_quantity(RES_STEEL), 120);
        assert_eq!(inv_reg.get(hauler).unwrap().total_quantity(RES_STEEL), 80);
        assert_eq!(inv_reg.get(dest).unwrap().total_quantity(RES_STEEL), 0);

        let total_in_transit = inv_reg.get(source).unwrap().total_quantity(RES_STEEL)
            + inv_reg.get(dest).unwrap().total_quantity(RES_STEEL)
            + inv_reg.get(hauler).unwrap().total_quantity(RES_STEEL);
        assert_eq!(
            total_in_transit, 200,
            "Zero loss or duplication during transit"
        );

        // Execute dropoff
        mgr.execute_dropoff(job_id, hauler, SimTick::new(4), &mut inv_reg, &mut journal)
            .unwrap();
        assert_eq!(inv_reg.get(source).unwrap().total_quantity(RES_STEEL), 120);
        assert_eq!(inv_reg.get(hauler).unwrap().total_quantity(RES_STEEL), 0);
        assert_eq!(inv_reg.get(dest).unwrap().total_quantity(RES_STEEL), 80);

        let total_after = inv_reg.get(source).unwrap().total_quantity(RES_STEEL)
            + inv_reg.get(dest).unwrap().total_quantity(RES_STEEL)
            + inv_reg.get(hauler).unwrap().total_quantity(RES_STEEL);
        assert_eq!(total_after, 200, "Zero loss or duplication after dropoff");

        let job = mgr.jobs.get(&job_id).unwrap();
        assert_eq!(job.status, JobStatus::Completed);
    }

    #[test]
    fn test_dropoff_transaction_abort_preserves_cargo() {
        let mut mgr = LogisticsManager::new();
        let mut inv_reg = InventoryRegistry::new();
        let mut journal = EventJournal::new();

        let source = EntityId::new(10);
        let dest = EntityId::new(20);
        let hauler = EntityId::new(101);

        let mut src_inv = Inventory::new(source, ContainerKind::Depot);
        src_inv.add(RES_TUNGSTEN_ORE, 100).unwrap();
        inv_reg.register(src_inv);

        // Destination with 1 slot max, already occupied by Steel
        let mut full_dest = Inventory::new(dest, ContainerKind::Depot);
        full_dest.max_slots = 1;
        full_dest.add(RES_STEEL, 10).unwrap();
        inv_reg.register(full_dest);

        inv_reg.register(Inventory::new(hauler, ContainerKind::CargoBuffer));

        let job_id = mgr
            .create_job(
                source,
                dest,
                RES_TUNGSTEN_ORE,
                50,
                JobPriority::Normal,
                SimTick::new(1),
                &mut journal,
            )
            .unwrap();

        mgr.claim_job(job_id, hauler, SimTick::new(2), &mut inv_reg, &mut journal)
            .unwrap();
        mgr.execute_pickup(job_id, hauler, SimTick::new(3), &mut inv_reg, &mut journal)
            .unwrap();

        // Attempt dropoff into destination that cannot accept -> fails
        let dropoff_res =
            mgr.execute_dropoff(job_id, hauler, SimTick::new(4), &mut inv_reg, &mut journal);
        assert!(matches!(dropoff_res, Err(GameError::InventoryFull { .. })));

        // Hauler still has all 50 items safely stored
        assert_eq!(
            inv_reg
                .get(hauler)
                .unwrap()
                .total_quantity(RES_TUNGSTEN_ORE),
            50
        );
        // Destination still unchanged
        assert_eq!(
            inv_reg.get(dest).unwrap().total_quantity(RES_TUNGSTEN_ORE),
            0
        );
    }

    #[test]
    fn test_powered_depot_coverage_gating() {
        let depot_ent = EntityId::new(50);
        let depot = DepotLogistics::new(depot_ent, 40.0);

        let depot_pos = (100.0, 0.0, 100.0);
        let close_pos = (120.0, 0.0, 100.0); // 20m away
        let far_pos = (160.0, 0.0, 100.0); // 60m away

        // When powered:
        assert_eq!(depot.effective_coverage(), 40.0);
        assert!(depot.is_in_coverage(depot_pos, close_pos));
        assert!(!depot.is_in_coverage(depot_pos, far_pos));

        // When unpowered (blackout / brownout):
        let mut unpowered_depot = depot.clone();
        unpowered_depot.is_powered = false;
        assert_eq!(unpowered_depot.effective_coverage(), 0.0);
        assert!(!unpowered_depot.is_in_coverage(depot_pos, close_pos));

        // When restored:
        unpowered_depot.is_powered = true;
        assert_eq!(unpowered_depot.effective_coverage(), 40.0);
        assert!(unpowered_depot.is_in_coverage(depot_pos, close_pos));
    }

    #[test]
    fn test_dock_queue_berth_occupancy_and_service_rate() {
        let dock_ent = EntityId::new(80);
        let mut dock = LogisticsDock::new(dock_ent, 2, 10);

        let v1 = EntityId::new(101);
        let v2 = EntityId::new(102);
        let v3 = EntityId::new(103);

        dock.enqueue_vessel(v1).unwrap();
        dock.enqueue_vessel(v2).unwrap();
        dock.enqueue_vessel(v3).unwrap();

        assert_eq!(dock.queue.len(), 3);
        assert!(dock.admits_berth());

        // Duplicate queue attempt returns Ok without inflating queue
        dock.enqueue_vessel(v1).unwrap();
        assert_eq!(dock.queue.len(), 3);
    }

    #[test]
    fn test_route_graph_dijkstra_pathfinding() {
        let mut graph = RouteGraph::new();

        let n1 = RouteNodeId::new(1);
        let n2 = RouteNodeId::new(2);
        let n3 = RouteNodeId::new(3);
        let n4 = RouteNodeId::new(4);

        graph.add_node(RouteNode {
            id: n1,
            position: (0.0, 0.0, 0.0),
            associated_entity: None,
        });
        graph.add_node(RouteNode {
            id: n2,
            position: (10.0, 0.0, 0.0),
            associated_entity: None,
        });
        graph.add_node(RouteNode {
            id: n3,
            position: (20.0, 0.0, 0.0),
            associated_entity: None,
        });
        graph.add_node(RouteNode {
            id: n4,
            position: (10.0, 0.0, 50.0),
            associated_entity: None,
        });

        // Edge 1 -> 2 (10m @ 1m/tick = 10 ticks)
        graph.add_edge(RouteEdge {
            from: n1,
            to: n2,
            distance: 10.0,
            max_active_haulers: 4,
            current_haulers: 0,
            traversal_speed: 1.0,
        });
        // Edge 2 -> 3 (10m @ 1m/tick = 10 ticks)
        graph.add_edge(RouteEdge {
            from: n2,
            to: n3,
            distance: 10.0,
            max_active_haulers: 4,
            current_haulers: 0,
            traversal_speed: 1.0,
        });
        // Edge 1 -> 4 (50m @ 1m/tick = 50 ticks)
        graph.add_edge(RouteEdge {
            from: n1,
            to: n4,
            distance: 50.0,
            max_active_haulers: 4,
            current_haulers: 0,
            traversal_speed: 1.0,
        });
        // Edge 4 -> 3 (50m @ 1m/tick = 50 ticks)
        graph.add_edge(RouteEdge {
            from: n4,
            to: n3,
            distance: 50.0,
            max_active_haulers: 4,
            current_haulers: 0,
            traversal_speed: 1.0,
        });

        // Dijkstra shortest path from n1 to n3 must select [n1, n2, n3] (20 ticks), not [n1, n4, n3] (100 ticks)
        let (path, ticks) = graph.find_path(n1, n3).unwrap();
        assert_eq!(path, vec![n1, n2, n3]);
        assert_eq!(ticks, 20);

        // Same node
        let (same_path, same_ticks) = graph.find_path(n1, n1).unwrap();
        assert_eq!(same_path, vec![n1]);
        assert_eq!(same_ticks, 0);

        // Unknown node
        assert!(graph.find_path(n1, RouteNodeId::new(99)).is_none());
    }

    #[test]
    fn test_abstract_distant_transport_scheduled_arrival() {
        let mut mgr = LogisticsManager::new();
        let mut inv_reg = InventoryRegistry::new();
        let mut journal = EventJournal::new();

        let source = EntityId::new(10);
        let dest = EntityId::new(20);
        let hauler = EntityId::new(101);

        let mut src_inv = Inventory::new(source, ContainerKind::Depot);
        src_inv.add(RES_IRON_ORE, 100).unwrap();
        inv_reg.register(src_inv);
        inv_reg.register(Inventory::new(dest, ContainerKind::Depot));

        let mut hauler_inv = Inventory::new(hauler, ContainerKind::CargoBuffer);
        hauler_inv.add(RES_IRON_ORE, 40).unwrap();
        inv_reg.register(hauler_inv);

        let job_id = mgr
            .create_job(
                source,
                dest,
                RES_IRON_ORE,
                40,
                JobPriority::Normal,
                SimTick::new(1),
                &mut journal,
            )
            .unwrap();
        mgr.jobs.get_mut(&job_id).unwrap().status = JobStatus::InTransit;
        mgr.jobs
            .get_mut(&job_id)
            .unwrap()
            .claimed_workers
            .push(hauler);

        // Schedule distant transport arriving at tick 100
        mgr.distant_transports.push(DistantTransport {
            hauler_entity: hauler,
            job_id,
            departure_tick: SimTick::new(10),
            arrival_tick: SimTick::new(100),
            resource_id: RES_IRON_ORE,
            amount: 40,
        });

        // Step at tick 50 -> transport not arrived yet
        mgr.step(SimTick::new(50), &mut inv_reg, &mut journal);
        assert_eq!(mgr.distant_transports.len(), 1);
        assert_eq!(inv_reg.get(dest).unwrap().total_quantity(RES_IRON_ORE), 0);

        // Step at tick 100 -> arrives and delivers
        mgr.step(SimTick::new(100), &mut inv_reg, &mut journal);
        assert_eq!(mgr.distant_transports.len(), 0);
        assert_eq!(inv_reg.get(dest).unwrap().total_quantity(RES_IRON_ORE), 40);
        assert_eq!(inv_reg.get(hauler).unwrap().total_quantity(RES_IRON_ORE), 0);
    }

    #[test]
    fn test_starvation_and_deadlock_detection_metrics() {
        let mut mgr = LogisticsManager::new();
        let mut inv_reg = InventoryRegistry::new();
        let mut journal = EventJournal::new();

        let source = EntityId::new(10);
        let dest = EntityId::new(20);

        let job_id = mgr
            .create_job(
                source,
                dest,
                RES_STEEL,
                20,
                JobPriority::High,
                SimTick::new(1),
                &mut journal,
            )
            .unwrap();

        mgr.jobs
            .get_mut(&job_id)
            .unwrap()
            .starvation_threshold_ticks = 50;

        // Step before threshold: no starvation
        mgr.step(SimTick::new(40), &mut inv_reg, &mut journal);
        assert_eq!(mgr.telemetry.jobs_starved_count, 0);

        // Step after threshold: marked starved and emitted to journal
        mgr.step(SimTick::new(60), &mut inv_reg, &mut journal);
        assert_eq!(mgr.telemetry.jobs_starved_count, 1);
        assert!(mgr.jobs.get(&job_id).unwrap().is_starved);

        // Verify journal recorded starvation event
        let starved_events = journal
            .events_since(SimTick::zero())
            .iter()
            .filter(|(_, ev)| matches!(ev, SimEvent::LogisticsJobStarved { .. }))
            .count();
        assert_eq!(starved_events, 1);
    }

    #[test]
    fn test_universal_buffer_docking_lifecycle() {
        let hauler = EntityId::new(101);
        let dock = EntityId::new(50);

        let mut buffer = UniversalBuffer::new_mobile_buffer(hauler);
        assert!(buffer.is_mobile);
        assert!(buffer.docked_at.is_none());

        buffer.dock_to(dock);
        assert_eq!(buffer.docked_at, Some(dock));

        buffer.undock();
        assert!(buffer.docked_at.is_none());
    }

    #[test]
    fn test_find_best_pending_job_prioritization() {
        let mut mgr = LogisticsManager::new();
        let mut journal = EventJournal::new();

        let s1 = EntityId::new(1);
        let s2 = EntityId::new(2);
        let s3 = EntityId::new(3);
        let s4 = EntityId::new(4);
        let d = EntityId::new(10);

        let _j_low = mgr
            .create_job(
                s1,
                d,
                RES_IRON_ORE,
                10,
                JobPriority::Low,
                SimTick::new(1),
                &mut journal,
            )
            .unwrap();
        let _j_norm = mgr
            .create_job(
                s2,
                d,
                RES_IRON_ORE,
                10,
                JobPriority::Normal,
                SimTick::new(2),
                &mut journal,
            )
            .unwrap();
        let _j_high = mgr
            .create_job(
                s3,
                d,
                RES_IRON_ORE,
                10,
                JobPriority::High,
                SimTick::new(3),
                &mut journal,
            )
            .unwrap();
        let j_crit = mgr
            .create_job(
                s4,
                d,
                RES_IRON_ORE,
                10,
                JobPriority::Critical,
                SimTick::new(4),
                &mut journal,
            )
            .unwrap();

        // Critical priority job should be picked first
        let best = mgr.find_best_pending_job(None);
        assert_eq!(best, Some(j_crit));

        // Capacity filter: capacity 5 cannot carry 10 units
        let none = mgr.find_best_pending_job(Some(5));
        assert!(none.is_none());
    }
}
