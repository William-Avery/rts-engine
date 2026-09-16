use std::fmt;

/// Unique entity identifier for simulation objects.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct EntityId(pub u64);

impl EntityId {
    pub const fn new(value: u64) -> Self {
        EntityId(value)
    }

    pub const fn null() -> Self {
        EntityId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Entity({})", self.0)
    }
}

/// Unique player identifier.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct PlayerId(pub u32);

impl PlayerId {
    pub const fn new(value: u32) -> Self {
        PlayerId(value)
    }

    pub const fn null() -> Self {
        PlayerId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for PlayerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Player({})", self.0)
    }
}

/// Unique faction identifier.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct FactionId(pub u32);

impl FactionId {
    pub const fn new(value: u32) -> Self {
        FactionId(value)
    }

    pub const fn null() -> Self {
        FactionId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for FactionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Faction({})", self.0)
    }
}

/// Unique region identifier for world partitioning.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct RegionId(pub u32);

impl RegionId {
    pub const fn new(value: u32) -> Self {
        RegionId(value)
    }

    pub const fn null() -> Self {
        RegionId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for RegionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Region({})", self.0)
    }
}

/// Unique resource identifier.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ResourceId(pub u16);

impl ResourceId {
    pub const fn new(value: u16) -> Self {
        ResourceId(value)
    }

    pub const fn null() -> Self {
        ResourceId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u16 {
        self.0
    }
}

impl fmt::Display for ResourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Resource({})", self.0)
    }
}

/// Unique item/recipe identifier.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ItemId(pub u16);

impl ItemId {
    pub const fn new(value: u16) -> Self {
        ItemId(value)
    }

    pub const fn null() -> Self {
        ItemId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u16 {
        self.0
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Item({})", self.0)
    }
}

/// Unique structure identifier.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct StructureId(pub u64);

impl StructureId {
    pub const fn new(value: u64) -> Self {
        StructureId(value)
    }

    pub const fn null() -> Self {
        StructureId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for StructureId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Structure({})", self.0)
    }
}

/// Unique job identifier for logistics and production.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct JobId(pub u64);

impl JobId {
    pub const fn new(value: u64) -> Self {
        JobId(value)
    }

    pub const fn null() -> Self {
        JobId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for JobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Job({})", self.0)
    }
}

/// Unique session identifier for anti-cheat and networking.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct SessionId(pub u64);

impl SessionId {
    pub const fn new(value: u64) -> Self {
        SessionId(value)
    }

    pub const fn null() -> Self {
        SessionId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Session({})", self.0)
    }
}

/// Unique reservation identifier for transactional inventory locking.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ReservationId(pub u64);

impl ReservationId {
    pub const fn new(value: u64) -> Self {
        ReservationId(value)
    }

    pub const fn null() -> Self {
        ReservationId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for ReservationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Reservation({})", self.0)
    }
}

/// Unique power grid / subnet identifier for electrical network connected components.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct PowerGridId(pub u64);

impl PowerGridId {
    pub const fn new(value: u64) -> Self {
        PowerGridId(value)
    }

    pub const fn null() -> Self {
        PowerGridId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for PowerGridId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PowerGrid({})", self.0)
    }
}

/// Unique resource deposit identifier for harvestable world mineral nodes.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct DepositId(pub u64);

impl DepositId {
    pub const fn new(value: u64) -> Self {
        DepositId(value)
    }

    pub const fn null() -> Self {
        DepositId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for DepositId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Deposit({})", self.0)
    }
}

/// Unique recipe identifier for refining, manufacturing, and synthesis.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct RecipeId(pub u32);

impl RecipeId {
    pub const fn new(value: u32) -> Self {
        RecipeId(value)
    }

    pub const fn null() -> Self {
        RecipeId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for RecipeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Recipe({})", self.0)
    }
}

/// Unique logistics job identifier for material movement tasks.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct LogisticsJobId(pub u64);

impl LogisticsJobId {
    pub const fn new(value: u64) -> Self {
        LogisticsJobId(value)
    }

    pub const fn null() -> Self {
        LogisticsJobId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for LogisticsJobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LogisticsJob({})", self.0)
    }
}

/// Unique route node identifier in the logistics route graph.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct RouteNodeId(pub u32);

impl RouteNodeId {
    pub const fn new(value: u32) -> Self {
        RouteNodeId(value)
    }

    pub const fn null() -> Self {
        RouteNodeId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for RouteNodeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RouteNode({})", self.0)
    }
}

/// Unique squad identifier for deterministic robot formation groups.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct SquadId(pub u32);

impl SquadId {
    pub const fn new(value: u32) -> Self {
        SquadId(value)
    }

    pub const fn null() -> Self {
        SquadId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for SquadId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Squad({})", self.0)
    }
}

/// Unique technology identifier in the data-driven research tech tree.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct TechId(pub u32);

impl TechId {
    pub const fn new(value: u32) -> Self {
        TechId(value)
    }

    pub const fn null() -> Self {
        TechId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for TechId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Tech({})", self.0)
    }
}

/// Unique identifier for a queued research job entry.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ResearchJobId(pub u64);

impl ResearchJobId {
    pub const fn new(value: u64) -> Self {
        ResearchJobId(value)
    }

    pub const fn null() -> Self {
        ResearchJobId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for ResearchJobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ResearchJob({})", self.0)
    }
}

/// Unique robot chassis archetype identifier gated behind research unlocks.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ChassisId(pub u32);

impl ChassisId {
    pub const fn new(value: u32) -> Self {
        ChassisId(value)
    }

    pub const fn null() -> Self {
        ChassisId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for ChassisId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Chassis({})", self.0)
    }
}

/// Unique projectile identifier for simulated ballistics and attacks.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct ProjectileId(pub u64);

impl ProjectileId {
    pub const fn new(value: u64) -> Self {
        ProjectileId(value)
    }

    pub const fn null() -> Self {
        ProjectileId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u64 {
        self.0
    }
}

impl fmt::Display for ProjectileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Projectile({})", self.0)
    }
}

/// Unique weapon identifier.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct WeaponId(pub u32);

impl WeaponId {
    pub const fn new(value: u32) -> Self {
        WeaponId(value)
    }

    pub const fn null() -> Self {
        WeaponId(0)
    }

    pub const fn is_null(&self) -> bool {
        self.0 == 0
    }

    pub const fn value(&self) -> u32 {
        self.0
    }
}

impl fmt::Display for WeaponId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Weapon({})", self.0)
    }
}
