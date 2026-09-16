use game_types::{EntityId, SimTick};

/// Event identifier.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct EventId(pub u64);

impl EventId {
    pub const fn new(value: u64) -> Self {
        EventId(value)
    }

    pub const fn null() -> Self {
        EventId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

/// Simulation events for logging, debugging, and networking.
#[derive(Debug, Clone, PartialEq)]
pub enum SimEvent {
    /// Entity created
    EntityCreated {
        entity: EntityId,
        faction_id: game_types::FactionId,
        region_id: game_types::RegionId,
    },
    /// Entity removed
    EntityRemoved { entity: EntityId },
    /// Entity moved between regions
    RegionChanged {
        entity: EntityId,
        old_region: game_types::RegionId,
        new_region: game_types::RegionId,
    },
    /// Damage dealt
    DamageDealt {
        entity: EntityId,
        damage: f32,
        source: Option<EntityId>,
    },
    /// Resource changed
    ResourceChanged {
        entity: Option<EntityId>,
        resource_id: game_types::ResourceId,
        delta: i64,
    },
    /// Resource reserved atomically in an inventory
    ResourceReserved {
        entity: EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
        reservation_id: game_types::ReservationId,
    },
    /// Resource reservation committed and transferred to destination
    ResourceCommitted {
        from: EntityId,
        to: EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
        reservation_id: game_types::ReservationId,
    },
    /// Resource reservation released back to available balance
    ResourceReleased {
        entity: EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
        reservation_id: game_types::ReservationId,
    },
    /// Resource directly transferred between inventories
    ResourceTransferred {
        from: EntityId,
        to: EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
    },
    /// Structure built
    StructureBuilt {
        structure_id: game_types::StructureId,
        position: (f32, f32, f32),
        tier: u8,
    },
    /// Command executed
    CommandExecuted {
        command_id: crate::command::CommandId,
        success: bool,
        result: Option<String>,
    },
    /// Research completed and its unlocks/modifiers were applied
    ResearchCompleted {
        faction_id: game_types::FactionId,
        tech_id: game_types::TechId,
    },
    /// Power grid connected component updated
    PowerGridUpdated {
        grid_id: game_types::PowerGridId,
        faction_id: game_types::FactionId,
        node_count: u32,
        generation_kw: u32,
        demand_kw: u32,
    },
    /// Power grid entered brownout condition (deficit covered partially or under-capacity)
    PowerBrownoutStarted {
        grid_id: game_types::PowerGridId,
        satisfaction_ratio: f32,
    },
    /// Power grid entered total blackout condition (zero available power for demand)
    PowerBlackoutStarted { grid_id: game_types::PowerGridId },
    /// Power grid fully restored to normal or battery-supported operation
    PowerRestored { grid_id: game_types::PowerGridId },
    /// Structure power status changed
    StructurePowerStateChanged {
        structure_id: game_types::StructureId,
        powered: bool,
    },
    /// Production job started at a facility
    ProductionJobStarted {
        structure_id: game_types::StructureId,
        recipe_id: game_types::RecipeId,
        finish_tick: game_types::SimTick,
    },
    /// Production job completed successfully
    ProductionJobCompleted {
        structure_id: game_types::StructureId,
        recipe_id: game_types::RecipeId,
    },
    /// Production facility blocked from operating
    ProductionBlocked {
        structure_id: game_types::StructureId,
        reason: ProductionBlockedReason,
    },
    /// Resource extracted from a world deposit node
    ResourceExtracted {
        structure_id: game_types::StructureId,
        deposit_id: game_types::DepositId,
        resource_id: game_types::ResourceId,
        amount: u32,
    },
    /// World resource deposit has been completely exhausted
    DepositDepleted { deposit_id: game_types::DepositId },
    /// Logistics job created
    LogisticsJobCreated {
        job_id: game_types::LogisticsJobId,
        resource_id: game_types::ResourceId,
        amount: u32,
        source: EntityId,
        destination: EntityId,
    },
    /// Logistics job claimed by worker
    LogisticsJobClaimed {
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
    },
    /// Material picked up by worker from source
    LogisticsPickupCompleted {
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
        amount: u32,
    },
    /// Material dropped off by worker at destination
    LogisticsDropoffCompleted {
        job_id: game_types::LogisticsJobId,
        worker_id: EntityId,
        amount: u32,
    },
    /// Logistics job cancelled
    LogisticsJobCancelled {
        job_id: game_types::LogisticsJobId,
        reason: String,
    },
    /// Logistics job starved (pending past deadline)
    LogisticsJobStarved {
        job_id: game_types::LogisticsJobId,
        pending_ticks: u64,
    },
    /// Logistics dock queue length updated
    DockQueueUpdated {
        dock_entity: EntityId,
        queue_length: usize,
    },
    /// Biped robot spawned into the authoritative robot registry
    RobotSpawned {
        robot: EntityId,
        chassis: u8,
        faction_id: game_types::FactionId,
        position: (f32, f32, f32),
    },
    /// Robot received a new authoritative standing order
    RobotOrderIssued { robot: EntityId, order_code: u8 },
    /// Robot reached the goal of its current order
    RobotArrived {
        robot: EntityId,
        position: (f32, f32, f32),
    },
    /// Robot destroyed and removed from the simulation
    RobotDestroyed {
        robot: EntityId,
        source: Option<EntityId>,
    },
    /// Robot assigned as a player's personal escort
    EscortAssigned {
        player: game_types::PlayerId,
        robot: EntityId,
    },
    /// Robot released from escort duty
    EscortReleased {
        player: game_types::PlayerId,
        robot: EntityId,
    },
    /// Squad created
    SquadFormed {
        squad_id: game_types::SquadId,
        leader: EntityId,
        faction_id: game_types::FactionId,
    },
    /// Robot added to a squad roster
    SquadMemberAssigned {
        squad_id: game_types::SquadId,
        robot: EntityId,
    },
    /// Robot removed from a squad roster
    SquadMemberRemoved {
        squad_id: game_types::SquadId,
        robot: EntityId,
    },
    /// Squad ordered to reform on a rally point
    SquadRegrouped {
        squad_id: game_types::SquadId,
        rally_position: (f32, f32, f32),
        member_count: u32,
    },
    /// Technology accepted into a faction research queue
    ResearchQueued {
        faction_id: game_types::FactionId,
        tech_id: game_types::TechId,
        job_id: game_types::ResearchJobId,
    },
    /// Research job locked its inputs and began consuming ticks
    ResearchStarted {
        faction_id: game_types::FactionId,
        tech_id: game_types::TechId,
        job_id: game_types::ResearchJobId,
        finish_tick: SimTick,
    },
    /// Research job stalled on an authoritative gate
    ResearchBlocked {
        faction_id: game_types::FactionId,
        tech_id: game_types::TechId,
        reason: ResearchBlockedReason,
    },
    /// Research job cancelled and its reserved inputs refunded in full
    ResearchCancelled {
        faction_id: game_types::FactionId,
        tech_id: game_types::TechId,
        job_id: game_types::ResearchJobId,
        refunded_units: u32,
    },
    /// A new faction-wide modifier patch was distributed across the network
    ModifierPatchDistributed {
        faction_id: game_types::FactionId,
        patch_version: u64,
        source_count: usize,
    },
}

/// Reason a research job cannot make progress this tick.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ResearchBlockedReason {
    /// The faction owns no constructed research facility.
    NoFacility,
    /// The hosting research facility has insufficient authoritative power.
    Unpowered,
    /// The facility hopper lacks the authoritative material cost.
    AwaitingResources,
}

/// Reason for production facility progress obstruction.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ProductionBlockedReason {
    OutputFull,
    AwaitingInputs,
    Unpowered,
    DepositDepleted,
}

/// Event journal for replay and debugging.
#[derive(Debug, Clone, PartialEq)]
pub struct EventJournal {
    events: Vec<(SimTick, SimEvent)>,
    next_id: u64,
}

impl Default for EventJournal {
    fn default() -> Self {
        Self::new()
    }
}

impl EventJournal {
    pub fn new() -> Self {
        EventJournal {
            events: Vec::new(),
            next_id: 1,
        }
    }

    pub fn record(&mut self, tick: SimTick, event: SimEvent) -> EventId {
        let id = EventId(self.next_id);
        self.next_id += 1;
        self.events.push((tick, event));
        id
    }

    pub fn events_since(&self, tick: SimTick) -> Vec<&(SimTick, SimEvent)> {
        self.events.iter().filter(|(t, _)| *t >= tick).collect()
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}
