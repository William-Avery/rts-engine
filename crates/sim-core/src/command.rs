use game_types::{SessionId, SimTick};

/// Command identifier for deduplication.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct CommandId(pub u64);

impl CommandId {
    pub const fn new(value: u64) -> Self {
        CommandId(value)
    }

    pub const fn null() -> Self {
        CommandId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

/// Versioned command envelope for network and persistence.
#[derive(Debug, Clone, PartialEq)]
pub struct CommandEnvelope {
    pub session_id: SessionId,
    pub sequence: u64,
    pub client_tick: SimTick,
    pub command: Command,
}

impl CommandEnvelope {
    pub fn new(
        session_id: SessionId,
        sequence: u64,
        client_tick: SimTick,
        command: Command,
    ) -> Self {
        CommandEnvelope {
            session_id,
            sequence,
            client_tick,
            command,
        }
    }
}

/// Command types for the simulation.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Player movement command
    Move {
        position: (f32, f32, f32),
        velocity: (f32, f32, f32),
    },
    /// Player action command
    Action {
        action_type: ActionType,
        target: Option<game_types::EntityId>,
    },
    /// Build structure command
    Build {
        position: (f32, f32, f32),
        structure_id: game_types::ItemId,
    },
    /// Authoritative structure placement command
    BuildStructure {
        kind: crate::structure::StructureKind,
        position: (f32, f32, f32),
        rotation_deg: f32,
    },
    /// Authoritative structure dismantle/deconstruct command
    DismantleStructure {
        structure_id: game_types::StructureId,
    },
    /// Authoritative structure repair command
    RepairStructure {
        structure_id: game_types::StructureId,
        actor_entity: Option<game_types::EntityId>,
    },
    /// Command robot
    RobotCommand {
        robot_id: game_types::EntityId,
        command_type: RobotCommandType,
    },
    /// Request resource
    RequestResource {
        resource_id: game_types::ResourceId,
        amount: u32,
    },
    /// Research command
    Research { tech_id: game_types::ItemId },
    /// Region transfer command
    TransferRegion {
        entity_id: game_types::EntityId,
        destination_region: game_types::RegionId,
    },
    /// Direct atomic transfer of resources between two container entities
    TransferResource {
        from_entity: game_types::EntityId,
        to_entity: game_types::EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
    },
    /// Atomically reserve an amount of resource in an entity's inventory
    ReserveResource {
        entity: game_types::EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
        reservation_id: game_types::ReservationId,
    },
    /// Commit an active reservation and deliver resources to target entity
    CommitTransfer {
        reservation_id: game_types::ReservationId,
        from_entity: game_types::EntityId,
        to_entity: game_types::EntityId,
    },
    /// Cancel an active reservation, unlocking items back to available balance
    CancelReservation {
        reservation_id: game_types::ReservationId,
        from_entity: game_types::EntityId,
    },
    /// Configure production recipe for an industrial facility
    SetProductionRecipe {
        structure_id: game_types::StructureId,
        recipe_id: game_types::RecipeId,
    },
    /// Assign target resource deposit for extraction
    SetExtractionTarget {
        structure_id: game_types::StructureId,
        deposit_id: game_types::DepositId,
    },
    /// Create a logistics job for material transport
    CreateLogisticsJob {
        source: game_types::EntityId,
        destination: game_types::EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
        priority: u8,
    },
    /// Cancel an active logistics job
    CancelLogisticsJob { job_id: game_types::LogisticsJobId },
    /// Atomically claim a logistics job for a worker hauler
    ClaimLogisticsJob {
        job_id: game_types::LogisticsJobId,
        worker_id: game_types::EntityId,
    },
    /// Execute atomic pickup transaction from source to worker
    ExecuteLogisticsPickup {
        job_id: game_types::LogisticsJobId,
        worker_id: game_types::EntityId,
    },
    /// Execute atomic dropoff transaction from worker to destination
    ExecuteLogisticsDropoff {
        job_id: game_types::LogisticsJobId,
        worker_id: game_types::EntityId,
    },
}

/// Action types.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ActionType {
    FireWeapon,
    Reload,
    UseItem,
    Interact,
    Attack,
    MoveTo,
    HoldPosition,
    Return,
}

/// Robot command types.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum RobotCommandType {
    Follow { target: game_types::EntityId },
    Guard { position: (f32, f32, f32) },
    Attack { target: game_types::EntityId },
    Move { position: (f32, f32, f32) },
    ReturnToBase,
}

/// Command buffer for batching and rollback.
#[derive(Debug, Default, Clone)]
pub struct CommandBuffer {
    commands: Vec<CommandEnvelope>,
}

impl CommandBuffer {
    pub fn new() -> Self {
        CommandBuffer {
            commands: Vec::new(),
        }
    }

    pub fn push(&mut self, envelope: CommandEnvelope) {
        self.commands.push(envelope);
    }

    pub fn pop(&mut self) -> Option<CommandEnvelope> {
        self.commands.pop()
    }

    pub fn len(&self) -> usize {
        self.commands.len()
    }

    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    pub fn clear(&mut self) {
        self.commands.clear();
    }
}
