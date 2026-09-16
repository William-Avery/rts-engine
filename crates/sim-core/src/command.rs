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
    /// Capability token the server issued to this session at handshake.
    ///
    /// `session_id` is a client-supplied routing field and proves nothing. The
    /// server accepts a command only when this token matches the one it issued
    /// **and** the datagram arrived from the address bound to that session, so
    /// a forged `session_id` cannot mute, impersonate or act as another player.
    pub token: u64,
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
            token: 0,
        }
    }

    /// Stamp the session capability token the server issued at handshake.
    pub fn with_token(mut self, token: u64) -> Self {
        self.token = token;
        self
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
    /// Request assignment of a robot as the requesting player's personal escort.
    /// The server rejects the command when `player` is not the session's own identity.
    AssignEscort {
        player: game_types::PlayerId,
        robot_id: game_types::EntityId,
    },
    /// Release one of the requesting player's escorts back to unassigned duty
    ReleaseEscort {
        player: game_types::PlayerId,
        robot_id: game_types::EntityId,
    },
    /// Add a robot to a squad roster
    AssignSquadMember {
        squad_id: game_types::SquadId,
        robot_id: game_types::EntityId,
    },
    /// Remove a robot from a squad roster
    RemoveSquadMember {
        squad_id: game_types::SquadId,
        robot_id: game_types::EntityId,
    },
    /// Order an entire squad to reform on a rally point
    SquadRegroup {
        squad_id: game_types::SquadId,
        rally_position: (f32, f32, f32),
    },
    /// Append a technology to the faction research queue.
    ///
    /// Supersedes the former placeholder `Research { tech_id: ItemId }` variant.
    QueueResearch { tech_id: game_types::TechId },
    /// Cancel a queued or active research job, refunding its reserved inputs.
    CancelResearch { job_id: game_types::ResearchJobId },
    /// Move a queued research job to a new position in the faction queue.
    ReorderResearchQueue {
        job_id: game_types::ResearchJobId,
        new_index: u16,
    },
    /// Client advertises its build/protocol/content manifest for server policy validation.
    ///
    /// Sent immediately after the handshake. Official servers refuse gameplay
    /// commands until a manifest has been accepted; local/private servers may
    /// not require one at all.
    SubmitClientManifest {
        build_id: String,
        protocol_version: u32,
        content_hash: u64,
        official_build: bool,
    },
    /// Privileged host/admin command: remove a session from the match.
    ///
    /// Requires `AdminPermission::KickSession`. The server authorizes before
    /// the command is buffered; an unprivileged sender is rejected and logged.
    AdminKickSession {
        target_session: game_types::SessionId,
        reason_code: u8,
    },
    /// Privileged host/admin command: override a session's anti-cheat trust level.
    ///
    /// Requires `AdminPermission::SetTrustLevel`.
    AdminSetTrustLevel {
        target_session: game_types::SessionId,
        trust_code: u8,
    },
    /// Privileged host/admin debug command: grant resources into an inventory.
    ///
    /// Requires `AdminPermission::GrantResources`. Exists so debug cheats are
    /// server-permissioned rather than client-asserted.
    AdminGrantResource {
        target_entity: game_types::EntityId,
        resource_id: game_types::ResourceId,
        amount: u32,
    },
    /// Privileged host/admin command: assign an admin role to a session.
    ///
    /// Requires `AdminPermission::SetSessionRole`.
    AdminSetSessionRole {
        target_session: game_types::SessionId,
        role_code: u8,
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
#[derive(Debug, Default, Clone, PartialEq)]
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

    /// Drain the whole buffer in deterministic, fair order.
    ///
    /// Ordered by `(session_id, sequence)`, **not** by network arrival. The
    /// previous `Vec::pop` applied commands LIFO, so a client that sent
    /// `ReserveResource` then `CommitTransfer` in one tick had the commit
    /// applied first and fail, and a contested transfer between two clients was
    /// decided by packet jitter. Sorting by the pair makes the outcome a
    /// function of the command stream alone, which is what makes a journal
    /// replay reproduce the match.
    ///
    /// `sort_by_key` is stable, so two envelopes that somehow share a key keep
    /// their insertion order rather than swapping between runs.
    pub fn drain_ordered(&mut self) -> std::vec::IntoIter<CommandEnvelope> {
        self.commands.sort_by_key(|e| (e.session_id, e.sequence));
        std::mem::take(&mut self.commands).into_iter()
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
