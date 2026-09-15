use crate::ids::{EntityId, RegionId};
use std::fmt;

/// Error type for simulation and game operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameError {
    /// Invalid ID (null or out of range)
    InvalidId,
    /// Resource overflow or underflow
    ResourceOverflow,
    /// Resource underflow (insufficient balance)
    ResourceUnderflow,
    /// Invalid position or transform
    InvalidPosition,
    /// Invalid state transition
    InvalidStateTransition,
    /// Missing required resource
    MissingResource,
    /// Invalid command for current state
    InvalidCommand,
    /// Region not found
    RegionNotFound(RegionId),
    /// Entity not found
    EntityNotFound(EntityId),
    /// Entity was expected in a specific region but found elsewhere
    EntityNotInRegion {
        entity: EntityId,
        expected: RegionId,
        actual: RegionId,
    },
    /// Queue capacity exceeded under backpressure
    QueueFull { queue_name: String, capacity: usize },
    /// Invalid region bounds
    InvalidRegionBounds,
    /// Placement is outside allowable world terrain boundaries
    PlacementOutOfBounds,
    /// Placement overlaps with terrain obstacle or existing structure
    PlacementOverlap,
    /// Placement location is beyond player interaction reach
    PlacementTooFar,
    /// Build site is already reserved or occupied by another structure
    SiteOccupied,
    /// Structure not found
    StructureNotFound(crate::ids::StructureId),
    /// Invalid structure state transition
    InvalidStructureState,
    /// Player lacks faction permission for action
    PermissionDenied,
    /// Container or inventory is at full slot or volume capacity
    InventoryFull {
        max_slots: usize,
        max_volume_liters: u32,
    },
    /// Inventory slot index out of bounds
    SlotOutOfBounds { slot: usize, max_slots: usize },
    /// Reservation ID not found
    ReservationNotFound(crate::ids::ReservationId),
    /// Reservation ID already exists
    ReservationAlreadyExists(crate::ids::ReservationId),
    /// Insufficient unreserved balance to satisfy request
    InsufficientUnreservedBalance { available: u32, requested: u32 },
    /// Container not found for entity
    ContainerNotFound(EntityId),
    /// Logistics job already claimed by another worker
    JobAlreadyClaimed(crate::ids::LogisticsJobId),
    /// Logistics job not found
    JobNotFound(crate::ids::LogisticsJobId),
    /// Logistics job is not claimed by the specified worker entity
    JobNotClaimedByWorker(crate::ids::LogisticsJobId, EntityId),
    /// Invalid logistics job state transition
    InvalidJobState,
    /// Logistics dock berths are fully occupied
    DockBerthsFull,
    /// Logistics dock waiting queue is at maximum capacity
    DockQueueFull,
    /// Target location is outside active powered logistics coverage
    OutOfLogisticsCoverage,
    /// Authoritative state corruption detected
    CorruptedState(String),
    /// Network protocol error
    ProtocolError(String),
    /// Serialization/deserialization error
    SerializationError(String),
    /// Internal simulation error (indicates bug)
    InternalError(String),
}

impl fmt::Display for GameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GameError::InvalidId => write!(f, "Invalid ID"),
            GameError::ResourceOverflow => write!(f, "Resource overflow"),
            GameError::ResourceUnderflow => write!(f, "Resource underflow"),
            GameError::InvalidPosition => write!(f, "Invalid position"),
            GameError::InvalidStateTransition => write!(f, "Invalid state transition"),
            GameError::MissingResource => write!(f, "Missing required resource"),
            GameError::InvalidCommand => write!(f, "Invalid command for current state"),
            GameError::RegionNotFound(id) => write!(f, "Region not found: {id}"),
            GameError::EntityNotFound(id) => write!(f, "Entity not found: {id}"),
            GameError::EntityNotInRegion {
                entity,
                expected,
                actual,
            } => write!(
                f,
                "Entity {entity} expected in region {expected}, but located in {actual}"
            ),
            GameError::QueueFull {
                queue_name,
                capacity,
            } => write!(f, "Queue '{queue_name}' is full (capacity: {capacity})"),
            GameError::InvalidRegionBounds => write!(f, "Invalid region bounds"),
            GameError::PlacementOutOfBounds => write!(f, "Placement is out of bounds"),
            GameError::PlacementOverlap => {
                write!(f, "Placement overlaps with obstacle or structure")
            }
            GameError::PlacementTooFar => write!(f, "Placement location is too far from player"),
            GameError::SiteOccupied => write!(f, "Build site is already occupied or reserved"),
            GameError::StructureNotFound(id) => write!(f, "Structure not found: {id}"),
            GameError::InvalidStructureState => write!(f, "Invalid structure state transition"),
            GameError::PermissionDenied => write!(f, "Action permission denied"),
            GameError::InventoryFull {
                max_slots,
                max_volume_liters,
            } => write!(
                f,
                "Inventory full (max slots: {max_slots}, max volume: {max_volume_liters} L)"
            ),
            GameError::SlotOutOfBounds { slot, max_slots } => {
                write!(f, "Slot index {slot} out of bounds (max: {max_slots})")
            }
            GameError::ReservationNotFound(id) => write!(f, "Reservation not found: {id}"),
            GameError::ReservationAlreadyExists(id) => {
                write!(f, "Reservation already exists: {id}")
            }
            GameError::InsufficientUnreservedBalance {
                available,
                requested,
            } => write!(
                f,
                "Insufficient unreserved balance (available: {available}, requested: {requested})"
            ),
            GameError::ContainerNotFound(id) => write!(f, "Container not found for entity: {id}"),
            GameError::JobAlreadyClaimed(id) => write!(f, "Logistics job already claimed: {id}"),
            GameError::JobNotFound(id) => write!(f, "Logistics job not found: {id}"),
            GameError::JobNotClaimedByWorker(job_id, worker_id) => {
                write!(
                    f,
                    "Logistics job {job_id} is not claimed by worker {worker_id}"
                )
            }
            GameError::InvalidJobState => write!(f, "Invalid logistics job state transition"),
            GameError::DockBerthsFull => write!(f, "Logistics dock berths are full"),
            GameError::DockQueueFull => write!(f, "Logistics dock queue is full"),
            GameError::OutOfLogisticsCoverage => {
                write!(
                    f,
                    "Target location is outside active powered logistics coverage"
                )
            }
            GameError::CorruptedState(msg) => write!(f, "Corrupted state detected: {msg}"),
            GameError::ProtocolError(msg) => write!(f, "Protocol error: {msg}"),
            GameError::SerializationError(msg) => write!(f, "Serialization error: {msg}"),
            GameError::InternalError(msg) => write!(f, "Internal error: {msg}"),
        }
    }
}

impl std::error::Error for GameError {}

/// Result type alias for game operations.
pub type GameResult<T> = Result<T, GameError>;
