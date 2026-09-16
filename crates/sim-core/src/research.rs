//! Research facilities, the data-driven tech tree, and the research queue.
//!
//! Research is the authoritative producer of the faction-wide modifier network in
//! [`crate::modifier`]. A completed technology does three things, all expressed as
//! data on the technology definition itself:
//!
//! 1. it marks itself completed for the faction,
//! 2. it grants its [`UnlockTarget`]s (structure kinds, recipes, robot chassis, upgrades),
//! 3. it registers its [`Modifier`]s as a distributed software patch.
//!
//! Adding content therefore never requires a new code path: a new entry in a
//! [`TechDef`] table is enough.
//!
//! # Authority gates
//!
//! A research job can only make progress when **all three** of the following hold:
//! authoritative input resources are present in the facility hopper (and locked
//! under a two-phase reservation), the facility structure is constructed and
//! receiving sufficient authoritative power, and the required number of simulation
//! ticks have elapsed. Removing any one of them stalls the job without losing
//! progress.
//!
//! # Cancellation refund policy
//!
//! Research inputs stay under reservation in the facility hopper for the whole job
//! and are only committed at completion. Cancelling therefore **releases every
//! outstanding reservation back to available balance: a full 100% material refund,
//! with elapsed research time forfeited.** No resources are created (not
//! exploitable) and none are destroyed (not punitive).

use crate::event::{EventJournal, ResearchBlockedReason, SimEvent};
use crate::inventory::{ContainerKind, Inventory};
use crate::modifier::{Modifier, ModifierKind, ModifierStore};
use crate::production::{
    RECIPE_FABRICATE_AMMO, RECIPE_FABRICATE_BASIC_COMPONENTS, RECIPE_HARDEN_STEEL,
    RECIPE_SINTER_CERAMIC, RECIPE_SMELT_STEEL, RECIPE_SMELT_TUNGSTEN,
    RECIPE_SYNTHESIZE_TUNGSTEN_COMPOSITE,
};
use crate::structure::{StructureKind, StructureRegistry};
use crate::wall::WallTier;
use game_types::{
    ChassisId, EntityId, FactionId, GameError, GameResult, ItemId, RES_ADVANCED_COMPONENTS,
    RES_BASIC_COMPONENTS, RES_CERAMIC_PLATE, RES_ENERGY_CELL, RES_STEEL, RecipeId, RegionId,
    ReservationId, ResourceId, SimTick, StructureId, TechId,
};
use std::collections::{BTreeMap, BTreeSet};

/// Content version of the built-in tech tree table. Bump when the catalog changes
/// shape so saves and clients can detect mismatched content.
pub const TECH_TREE_VERSION: u32 = 1;

/// Maximum number of simultaneously queued research jobs per faction.
pub const MAX_RESEARCH_QUEUE_LEN: usize = 16;

// Canonical technology identifiers.
pub const TECH_BASIC_METALLURGY: TechId = TechId(1);
pub const TECH_ADVANCED_ALLOYS: TechId = TechId(2);
pub const TECH_TUNGSTEN_PROCESSING: TechId = TechId(3);
pub const TECH_DRILL_OPTIMIZATION: TechId = TechId(4);
pub const TECH_POWER_REGULATION: TechId = TechId(5);
pub const TECH_LOGISTICS_PROTOCOLS: TechId = TechId(6);
pub const TECH_FIELD_REPAIR_PATCH: TechId = TechId(7);
pub const TECH_BALLISTICS_TUNING: TechId = TechId(8);
pub const TECH_TARGETING_ARRAYS: TechId = TechId(9);
pub const TECH_ROBOTICS_FOUNDRY: TechId = TechId(10);
pub const TECH_HEAVY_CHASSIS: TechId = TechId(11);
pub const TECH_REINFORCEMENT_RELAY: TechId = TechId(12);
pub const TECH_RESEARCH_AUTOMATION: TechId = TechId(13);
pub const TECH_ADAPTIVE_OVERCLOCK: TechId = TechId(14);

// Canonical robot chassis identifiers gated behind research (Milestones 12/18).
pub const CHASSIS_GUARDSMAN: ChassisId = ChassisId(1);
pub const CHASSIS_RIFLE: ChassisId = ChassisId(2);
pub const CHASSIS_ANTI_ARMOR: ChassisId = ChassisId(3);

// Canonical loadout upgrade tokens gated behind research (Milestones 21/22).
pub const UPGRADE_FIELD_REPAIR_KIT: ItemId = ItemId(901);
pub const UPGRADE_REINFORCEMENT_BEACON: ItemId = ItemId(902);

/// A single thing a technology makes available to a faction.
///
/// Gating is derived from the data: a target is "gated" purely because some
/// technology in the loaded tree lists it. Targets nobody mentions stay ungated,
/// which keeps pre-research content buildable without special-casing.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum UnlockTarget {
    /// A buildable structure archetype.
    Structure(StructureKind),
    /// A production recipe usable by refineries and fabricators.
    Recipe(RecipeId),
    /// A robot chassis archetype.
    RobotChassis(ChassisId),
    /// A loadout / module upgrade token.
    Upgrade(ItemId),
}

impl UnlockTarget {
    /// Short label for debug overlays.
    pub fn category(&self) -> &'static str {
        match self {
            UnlockTarget::Structure(_) => "structure",
            UnlockTarget::Recipe(_) => "recipe",
            UnlockTarget::RobotChassis(_) => "chassis",
            UnlockTarget::Upgrade(_) => "upgrade",
        }
    }
}

/// Immutable, data-driven definition of a single technology.
///
/// Every field is content data. The simulation contains no per-technology logic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TechDef {
    pub id: TechId,
    pub name: &'static str,
    /// Presentation tier used to lay out the debug tech tree.
    pub tier: u8,
    /// Technologies that must be completed before this one can be queued.
    pub prerequisites: &'static [TechId],
    /// Authoritative material cost consumed from the research facility hopper.
    pub cost: &'static [(ResourceId, u32)],
    /// Base research duration in simulation ticks before speed modifiers.
    pub duration_ticks: u32,
    /// Everything this technology makes available.
    pub unlocks: &'static [UnlockTarget],
    /// Software patch contributions distributed on completion.
    pub modifiers: &'static [Modifier],
}

/// Built-in technology table. Extend by adding rows; no code changes required.
pub static STATIC_TECH_CATALOG: &[TechDef] = &[
    TechDef {
        id: TECH_BASIC_METALLURGY,
        name: "Basic Metallurgy",
        tier: 1,
        prerequisites: &[],
        cost: &[(RES_STEEL, 20)],
        duration_ticks: 90,
        unlocks: &[
            UnlockTarget::Structure(StructureKind::Refinery),
            UnlockTarget::Recipe(RECIPE_SMELT_STEEL),
            UnlockTarget::Structure(StructureKind::Wall(WallTier::Mk2Steel)),
        ],
        modifiers: &[Modifier::patch(ModifierKind::RefiningSpeed, 100)],
    },
    TechDef {
        id: TECH_POWER_REGULATION,
        name: "Power Regulation",
        tier: 1,
        prerequisites: &[],
        cost: &[(RES_STEEL, 15), (RES_ENERGY_CELL, 5)],
        duration_ticks: 90,
        unlocks: &[UnlockTarget::Structure(StructureKind::Battery)],
        modifiers: &[
            Modifier::patch(ModifierKind::PowerGeneration, 100),
            Modifier::patch(ModifierKind::PowerEfficiency, 100),
        ],
    },
    TechDef {
        id: TECH_ADVANCED_ALLOYS,
        name: "Advanced Alloys",
        tier: 2,
        prerequisites: &[TECH_BASIC_METALLURGY],
        cost: &[(RES_STEEL, 40), (RES_BASIC_COMPONENTS, 10)],
        duration_ticks: 150,
        unlocks: &[
            UnlockTarget::Recipe(RECIPE_HARDEN_STEEL),
            UnlockTarget::Recipe(RECIPE_SINTER_CERAMIC),
        ],
        modifiers: &[Modifier::patch(ModifierKind::StructureIntegrity, 80)],
    },
    TechDef {
        id: TECH_DRILL_OPTIMIZATION,
        name: "Drill Optimization",
        tier: 2,
        prerequisites: &[TECH_BASIC_METALLURGY],
        cost: &[(RES_STEEL, 30), (RES_BASIC_COMPONENTS, 5)],
        duration_ticks: 120,
        unlocks: &[UnlockTarget::Structure(StructureKind::MiningDrill)],
        modifiers: &[
            Modifier::patch(ModifierKind::MiningYield, 150),
            Modifier::patch(ModifierKind::MiningSpeed, 100),
        ],
    },
    TechDef {
        id: TECH_BALLISTICS_TUNING,
        name: "Ballistics Tuning",
        tier: 2,
        prerequisites: &[TECH_BASIC_METALLURGY],
        cost: &[(RES_STEEL, 25), (RES_BASIC_COMPONENTS, 8)],
        duration_ticks: 120,
        unlocks: &[
            UnlockTarget::Structure(StructureKind::Turret),
            UnlockTarget::Recipe(RECIPE_FABRICATE_AMMO),
        ],
        modifiers: &[
            Modifier::patch(ModifierKind::WeaponDamage, 100),
            Modifier::patch(ModifierKind::WeaponFireRate, 50),
        ],
    },
    TechDef {
        id: TECH_LOGISTICS_PROTOCOLS,
        name: "Logistics Protocols",
        tier: 2,
        prerequisites: &[TECH_POWER_REGULATION],
        cost: &[(RES_STEEL, 35), (RES_BASIC_COMPONENTS, 10)],
        duration_ticks: 150,
        unlocks: &[UnlockTarget::Structure(StructureKind::Depot)],
        modifiers: &[
            Modifier::patch(ModifierKind::TransportThroughput, 250),
            Modifier::patch(ModifierKind::LogisticsCoverage, 100),
        ],
    },
    TechDef {
        id: TECH_RESEARCH_AUTOMATION,
        name: "Research Automation",
        tier: 2,
        prerequisites: &[TECH_POWER_REGULATION],
        cost: &[(RES_STEEL, 30), (RES_ENERGY_CELL, 10)],
        duration_ticks: 180,
        unlocks: &[],
        modifiers: &[Modifier::patch(ModifierKind::ResearchSpeed, 200)],
    },
    TechDef {
        id: TECH_TUNGSTEN_PROCESSING,
        name: "Tungsten Processing",
        tier: 3,
        prerequisites: &[TECH_ADVANCED_ALLOYS],
        cost: &[(RES_STEEL, 60), (RES_CERAMIC_PLATE, 10)],
        duration_ticks: 240,
        unlocks: &[
            UnlockTarget::Recipe(RECIPE_SMELT_TUNGSTEN),
            UnlockTarget::Recipe(RECIPE_SYNTHESIZE_TUNGSTEN_COMPOSITE),
            UnlockTarget::Structure(StructureKind::Wall(WallTier::Mk3Composite)),
        ],
        modifiers: &[Modifier::patch(ModifierKind::StructureIntegrity, 120)],
    },
    TechDef {
        id: TECH_FIELD_REPAIR_PATCH,
        name: "Field Repair Patch",
        tier: 3,
        prerequisites: &[TECH_ADVANCED_ALLOYS],
        cost: &[(RES_STEEL, 30), (RES_BASIC_COMPONENTS, 12)],
        duration_ticks: 150,
        unlocks: &[UnlockTarget::Upgrade(UPGRADE_FIELD_REPAIR_KIT)],
        modifiers: &[Modifier::patch(ModifierKind::RepairRate, 300)],
    },
    TechDef {
        id: TECH_ROBOTICS_FOUNDRY,
        name: "Robotics Foundry",
        tier: 3,
        prerequisites: &[TECH_ADVANCED_ALLOYS],
        cost: &[(RES_STEEL, 50), (RES_ADVANCED_COMPONENTS, 8)],
        duration_ticks: 240,
        unlocks: &[
            UnlockTarget::Structure(StructureKind::Fabricator),
            UnlockTarget::Recipe(RECIPE_FABRICATE_BASIC_COMPONENTS),
            UnlockTarget::RobotChassis(CHASSIS_GUARDSMAN),
            UnlockTarget::RobotChassis(CHASSIS_RIFLE),
        ],
        modifiers: &[
            Modifier::patch(ModifierKind::RobotFabricationSpeed, 100),
            Modifier::patch(ModifierKind::RobotFabricationCost, -100),
        ],
    },
    TechDef {
        id: TECH_TARGETING_ARRAYS,
        name: "Targeting Arrays",
        tier: 3,
        prerequisites: &[TECH_BALLISTICS_TUNING],
        cost: &[(RES_STEEL, 40), (RES_ADVANCED_COMPONENTS, 6)],
        duration_ticks: 210,
        unlocks: &[],
        modifiers: &[
            Modifier::patch(ModifierKind::WeaponAccuracy, 100),
            Modifier::patch(ModifierKind::WeaponFireRate, 50),
        ],
    },
    TechDef {
        id: TECH_HEAVY_CHASSIS,
        name: "Heavy Chassis",
        tier: 4,
        prerequisites: &[TECH_ROBOTICS_FOUNDRY, TECH_TUNGSTEN_PROCESSING],
        cost: &[(RES_ADVANCED_COMPONENTS, 15), (RES_CERAMIC_PLATE, 10)],
        duration_ticks: 300,
        unlocks: &[UnlockTarget::RobotChassis(CHASSIS_ANTI_ARMOR)],
        modifiers: &[
            Modifier::patch(ModifierKind::StructureIntegrity, 100),
            Modifier::patch(ModifierKind::WeaponDamage, 50),
        ],
    },
    TechDef {
        id: TECH_REINFORCEMENT_RELAY,
        name: "Reinforcement Relay",
        tier: 4,
        prerequisites: &[TECH_LOGISTICS_PROTOCOLS],
        cost: &[(RES_ADVANCED_COMPONENTS, 12), (RES_ENERGY_CELL, 15)],
        duration_ticks: 270,
        unlocks: &[UnlockTarget::Upgrade(UPGRADE_REINFORCEMENT_BEACON)],
        modifiers: &[Modifier::patch(ModifierKind::ReinforcementRate, 200)],
    },
    TechDef {
        id: TECH_ADAPTIVE_OVERCLOCK,
        name: "Adaptive Overclock",
        tier: 4,
        prerequisites: &[TECH_RESEARCH_AUTOMATION, TECH_DRILL_OPTIMIZATION],
        cost: &[(RES_ADVANCED_COMPONENTS, 10), (RES_ENERGY_CELL, 20)],
        duration_ticks: 300,
        unlocks: &[],
        // Efficiency-group contributions compound multiplicatively with the
        // software-patch group instead of merely adding to it.
        modifiers: &[
            Modifier::new(
                ModifierKind::MiningYield,
                crate::modifier::ModifierGroup::Efficiency,
                100,
            ),
            Modifier::new(
                ModifierKind::RefiningSpeed,
                crate::modifier::ModifierGroup::Efficiency,
                100,
            ),
            Modifier::new(
                ModifierKind::PowerEfficiency,
                crate::modifier::ModifierGroup::Efficiency,
                100,
            ),
        ],
    },
];

/// Validated, versioned technology graph.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TechTree {
    version: u32,
    defs: BTreeMap<TechId, &'static TechDef>,
    /// Every unlock target mentioned anywhere in the tree (i.e. everything gated).
    gated: BTreeSet<UnlockTarget>,
    /// Populated when the built-in catalog failed validation instead of panicking.
    pub load_error: Option<String>,
}

impl TechTree {
    /// An empty tree with no technologies.
    pub fn empty(version: u32) -> Self {
        TechTree {
            version,
            defs: BTreeMap::new(),
            gated: BTreeSet::new(),
            load_error: None,
        }
    }

    /// Load and fully validate a technology table.
    ///
    /// Validation rejects null and duplicate ids, zero-length research, zero-amount
    /// costs, self-prerequisites, dangling prerequisites, and prerequisite cycles.
    /// Every failure is an actionable [`GameError::TechTreeInvalid`], never a panic.
    pub fn load(version: u32, defs: &'static [TechDef]) -> GameResult<Self> {
        if version == 0 {
            return Err(GameError::TechTreeInvalid(
                "tech tree version must be non-zero".to_string(),
            ));
        }

        let mut map: BTreeMap<TechId, &'static TechDef> = BTreeMap::new();
        for def in defs {
            if def.id.is_null() {
                return Err(GameError::TechTreeInvalid(format!(
                    "technology '{}' uses the reserved null id 0",
                    def.name
                )));
            }
            if def.duration_ticks == 0 {
                return Err(GameError::TechTreeInvalid(format!(
                    "technology '{}' ({}) has duration_ticks = 0; research must take time",
                    def.name, def.id
                )));
            }
            for &(res, amount) in def.cost {
                if amount == 0 {
                    return Err(GameError::TechTreeInvalid(format!(
                        "technology '{}' ({}) declares a zero-amount cost for {res}",
                        def.name, def.id
                    )));
                }
            }
            if map.insert(def.id, def).is_some() {
                return Err(GameError::TechTreeInvalid(format!(
                    "duplicate technology id {} (second definition named '{}')",
                    def.id, def.name
                )));
            }
        }

        // Prerequisite integrity.
        for def in map.values() {
            for prereq in def.prerequisites {
                if *prereq == def.id {
                    return Err(GameError::TechTreeInvalid(format!(
                        "technology '{}' ({}) lists itself as a prerequisite",
                        def.name, def.id
                    )));
                }
                if !map.contains_key(prereq) {
                    return Err(GameError::TechTreeInvalid(format!(
                        "technology '{}' ({}) requires unknown prerequisite {}",
                        def.name, def.id, prereq
                    )));
                }
            }
        }

        let mut tree = TechTree {
            version,
            defs: map,
            gated: BTreeSet::new(),
            load_error: None,
        };

        // Cycle detection: a successful topological ordering proves acyclicity.
        tree.topological_order()?;

        for def in tree.defs.values() {
            for unlock in def.unlocks {
                tree.gated.insert(*unlock);
            }
        }

        Ok(tree)
    }

    /// Load the built-in catalog, degrading to an empty tree with a recorded
    /// `load_error` rather than panicking on malformed content.
    pub fn builtin() -> Self {
        match TechTree::load(TECH_TREE_VERSION, STATIC_TECH_CATALOG) {
            Ok(tree) => tree,
            Err(err) => {
                let mut empty = TechTree::empty(TECH_TREE_VERSION);
                empty.load_error = Some(err.to_string());
                empty
            }
        }
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn get(&self, id: TechId) -> Option<&'static TechDef> {
        self.defs.get(&id).copied()
    }

    pub fn contains(&self, id: TechId) -> bool {
        self.defs.contains_key(&id)
    }

    pub fn len(&self) -> usize {
        self.defs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.defs.is_empty()
    }

    /// All technology definitions in ascending id order.
    pub fn iter(&self) -> impl Iterator<Item = &'static TechDef> + '_ {
        self.defs.values().copied()
    }

    /// True when the target is gated by at least one technology in this tree.
    pub fn is_gated(&self, target: UnlockTarget) -> bool {
        self.gated.contains(&target)
    }

    /// Every gated unlock target in the tree, ascending.
    pub fn gated_targets(&self) -> &BTreeSet<UnlockTarget> {
        &self.gated
    }

    /// Deterministic topological ordering, or the cycle that prevents one.
    ///
    /// Uses Kahn's algorithm over `BTreeMap`/`BTreeSet`, so the ordering is stable
    /// across runs and platforms.
    pub fn topological_order(&self) -> GameResult<Vec<TechId>> {
        let mut indegree: BTreeMap<TechId, usize> = BTreeMap::new();
        let mut dependents: BTreeMap<TechId, Vec<TechId>> = BTreeMap::new();

        for (id, def) in &self.defs {
            indegree.entry(*id).or_insert(0);
            for prereq in def.prerequisites {
                *indegree.entry(*id).or_insert(0) += 1;
                dependents.entry(*prereq).or_default().push(*id);
            }
        }

        let mut ready: BTreeSet<TechId> = indegree
            .iter()
            .filter(|(_, deg)| **deg == 0)
            .map(|(id, _)| *id)
            .collect();

        let mut order = Vec::with_capacity(self.defs.len());
        while let Some(&next) = ready.iter().next() {
            ready.remove(&next);
            order.push(next);
            if let Some(children) = dependents.get(&next) {
                for child in children {
                    if let Some(deg) = indegree.get_mut(child) {
                        *deg -= 1;
                        if *deg == 0 {
                            ready.insert(*child);
                        }
                    }
                }
            }
        }

        if order.len() != self.defs.len() {
            let mut stuck: Vec<String> = indegree
                .iter()
                .filter(|(id, deg)| **deg > 0 && !order.contains(id))
                .map(|(id, _)| {
                    let name = self.defs.get(id).map(|d| d.name).unwrap_or("<unknown>");
                    format!("{id} '{name}'")
                })
                .collect();
            stuck.sort();
            return Err(GameError::TechTreeInvalid(format!(
                "prerequisite cycle detected among technologies: {}",
                stuck.join(", ")
            )));
        }

        Ok(order)
    }
}

/// Lifecycle state of a queued research job.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ResearchJobState {
    /// Waiting in the queue behind another job, or with no facility available.
    Queued,
    /// At the head of the queue but the facility hopper lacks the material cost.
    AwaitingResources,
    /// Started, but the facility lost authoritative power. Progress is preserved.
    Unpowered,
    /// Actively consuming ticks with inputs locked under reservation.
    InProgress,
}

/// A queued or active research job.
#[derive(Clone, Debug, PartialEq)]
pub struct ResearchJob {
    pub id: game_types::ResearchJobId,
    pub tech_id: TechId,
    pub faction_id: FactionId,
    pub state: ResearchJobState,
    /// Facility currently hosting the job and holding its input reservations.
    pub facility: Option<StructureId>,
    pub reservations: Vec<ReservationId>,
    pub progress_ticks: u32,
    pub total_ticks: u32,
    pub queued_tick: SimTick,
}

impl ResearchJob {
    /// Progress as a 0.0..=1.0 fraction for debug overlays.
    pub fn progress_fraction(&self) -> f32 {
        if self.total_ticks == 0 {
            return 0.0;
        }
        (self.progress_ticks as f32 / self.total_ticks as f32).clamp(0.0, 1.0)
    }
}

/// Authoritative research component attached to a `ResearchFacility` structure.
#[derive(Clone, Debug, PartialEq)]
pub struct ResearchFacilityState {
    pub structure_id: StructureId,
    pub faction_id: FactionId,
    pub region_id: RegionId,
    /// Hopper holding the authoritative material inputs consumed by research.
    pub input_inventory: Inventory,
    pub powered: bool,
    pub operational: bool,
    pub total_techs_completed: u64,
    next_reservation_seq: u64,
}

impl ResearchFacilityState {
    pub fn new(structure_id: StructureId, faction_id: FactionId, region_id: RegionId) -> Self {
        let owner = EntityId::new(structure_id.value());
        ResearchFacilityState {
            structure_id,
            faction_id,
            region_id,
            input_inventory: Inventory::with_custom_capacity(
                owner,
                ContainerKind::Hopper,
                16,
                2500,
            ),
            powered: false,
            operational: false,
            total_techs_completed: 0,
            next_reservation_seq: 1,
        }
    }

    fn next_reservation_id(&mut self) -> ReservationId {
        let id = (self.structure_id.value() << 32) | self.next_reservation_seq;
        self.next_reservation_seq += 1;
        ReservationId::new(id)
    }

    /// True when the facility can host research right now.
    pub fn can_research(&self) -> bool {
        self.operational && self.powered
    }
}

/// Per-faction research progression state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FactionResearch {
    pub completed: BTreeSet<TechId>,
    pub unlocked: BTreeSet<UnlockTarget>,
    /// Ordered queue; index 0 is the active job.
    pub queue: Vec<ResearchJob>,
}

/// Diagnostic counters for the research subsystem.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResearchMetrics {
    pub ticks_processed: u64,
    pub jobs_queued: u64,
    pub jobs_started: u64,
    pub jobs_completed: u64,
    pub jobs_cancelled: u64,
    pub modifier_patches_published: u64,
    pub active_facilities: usize,
}

/// Authoritative research subsystem: facilities, queues, unlocks, and the
/// faction-wide modifier network they feed.
#[derive(Clone, Debug, PartialEq)]
pub struct ResearchManager {
    pub tech_tree: TechTree,
    /// Authoritative faction-wide modifier network produced by completed research.
    pub modifiers: ModifierStore,
    pub facilities: BTreeMap<StructureId, ResearchFacilityState>,
    pub factions: BTreeMap<FactionId, FactionResearch>,
    next_job_id: u64,
    pub metrics: ResearchMetrics,
}

impl Default for ResearchManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ResearchManager {
    /// Build a manager backed by the built-in tech tree.
    pub fn new() -> Self {
        ResearchManager::with_tech_tree(TechTree::builtin())
    }

    /// Build a manager backed by an explicit (already validated) tech tree.
    pub fn with_tech_tree(tech_tree: TechTree) -> Self {
        ResearchManager {
            tech_tree,
            modifiers: ModifierStore::new(),
            facilities: BTreeMap::new(),
            factions: BTreeMap::new(),
            next_job_id: 1,
            metrics: ResearchMetrics::default(),
        }
    }

    /// Build a manager from a raw technology table, validating it at load time.
    pub fn from_catalog(version: u32, defs: &'static [TechDef]) -> GameResult<Self> {
        Ok(ResearchManager::with_tech_tree(TechTree::load(
            version, defs,
        )?))
    }

    /// Register a research facility component for a structure.
    pub fn register_facility(
        &mut self,
        structure_id: StructureId,
        faction_id: FactionId,
        region_id: RegionId,
    ) {
        self.facilities
            .entry(structure_id)
            .or_insert_with(|| ResearchFacilityState::new(structure_id, faction_id, region_id));
        self.factions.entry(faction_id).or_default();
    }

    pub fn remove_facility(&mut self, structure_id: StructureId) -> Option<ResearchFacilityState> {
        self.facilities.remove(&structure_id)
    }

    pub fn facility(&self, structure_id: StructureId) -> Option<&ResearchFacilityState> {
        self.facilities.get(&structure_id)
    }

    pub fn facility_mut(
        &mut self,
        structure_id: StructureId,
    ) -> Option<&mut ResearchFacilityState> {
        self.facilities.get_mut(&structure_id)
    }

    /// Discover research facilities in the structure registry and refresh their
    /// authoritative power and lifecycle state.
    pub fn sync_facilities(&mut self, registry: &StructureRegistry) {
        let mut live: BTreeSet<StructureId> = BTreeSet::new();

        for structure in registry.structures.values() {
            if structure.kind != StructureKind::ResearchFacility {
                continue;
            }
            live.insert(structure.id);
            let entry = self.facilities.entry(structure.id).or_insert_with(|| {
                ResearchFacilityState::new(structure.id, structure.faction_id, structure.region_id)
            });
            entry.operational = structure.state.is_operational();
            entry.powered = structure.is_powered();
            self.factions.entry(structure.faction_id).or_default();
        }

        self.facilities.retain(|id, _| live.contains(id));
        self.metrics.active_facilities = self
            .facilities
            .values()
            .filter(|f| f.can_research())
            .count();
    }

    /// Faction state, creating an empty record on first access.
    pub fn faction_mut(&mut self, faction: FactionId) -> &mut FactionResearch {
        self.factions.entry(faction).or_default()
    }

    pub fn faction(&self, faction: FactionId) -> Option<&FactionResearch> {
        self.factions.get(&faction)
    }

    /// Ordered research queue for a faction (index 0 is the active job).
    pub fn queue(&self, faction: FactionId) -> &[ResearchJob] {
        self.factions
            .get(&faction)
            .map(|f| f.queue.as_slice())
            .unwrap_or(&[])
    }

    pub fn is_completed(&self, faction: FactionId, tech: TechId) -> bool {
        self.factions
            .get(&faction)
            .map(|f| f.completed.contains(&tech))
            .unwrap_or(false)
    }

    /// True when a faction has been granted an unlock target.
    pub fn is_unlocked(&self, faction: FactionId, target: UnlockTarget) -> bool {
        self.factions
            .get(&faction)
            .map(|f| f.unlocked.contains(&target))
            .unwrap_or(false)
    }

    /// Data-driven availability gate: available unless the tech tree gates it and
    /// the faction has not unlocked it yet.
    pub fn is_available(&self, faction: FactionId, target: UnlockTarget) -> bool {
        !self.tech_tree.is_gated(target) || self.is_unlocked(faction, target)
    }

    pub fn can_build(&self, faction: FactionId, kind: StructureKind) -> bool {
        self.is_available(faction, UnlockTarget::Structure(kind))
    }

    pub fn can_use_recipe(&self, faction: FactionId, recipe: RecipeId) -> bool {
        self.is_available(faction, UnlockTarget::Recipe(recipe))
    }

    pub fn can_fabricate_chassis(&self, faction: FactionId, chassis: ChassisId) -> bool {
        self.is_available(faction, UnlockTarget::RobotChassis(chassis))
    }

    pub fn has_upgrade(&self, faction: FactionId, upgrade: ItemId) -> bool {
        self.is_available(faction, UnlockTarget::Upgrade(upgrade))
    }

    /// Technologies the faction could queue right now (prerequisites satisfied,
    /// not already completed or queued), in ascending id order.
    pub fn available_techs(&self, faction: FactionId) -> Vec<TechId> {
        let empty = FactionResearch::default();
        let fr = self.factions.get(&faction).unwrap_or(&empty);
        self.tech_tree
            .iter()
            .filter(|def| {
                !fr.completed.contains(&def.id)
                    && !fr.queue.iter().any(|j| j.tech_id == def.id)
                    && def.prerequisites.iter().all(|p| fr.completed.contains(p))
            })
            .map(|def| def.id)
            .collect()
    }

    /// Convenience: evaluate the faction modifier network on a float base value.
    pub fn modifier_value(&self, faction: FactionId, kind: ModifierKind, base: f32) -> f32 {
        self.modifiers.value_for(faction, kind, base)
    }

    /// Authoritatively enqueue a technology for research.
    ///
    /// Validates the technology exists, is not already researched or queued, that
    /// every prerequisite is complete, and that the queue has room.
    pub fn queue_research(
        &mut self,
        faction: FactionId,
        tech_id: TechId,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<game_types::ResearchJobId> {
        let def = self
            .tech_tree
            .get(tech_id)
            .ok_or(GameError::TechNotFound(tech_id))?;

        let fr = self.factions.entry(faction).or_default();

        if fr.completed.contains(&tech_id) {
            return Err(GameError::TechAlreadyResearched(tech_id));
        }
        if fr.queue.iter().any(|j| j.tech_id == tech_id) {
            return Err(GameError::TechAlreadyQueued(tech_id));
        }
        if fr.queue.len() >= MAX_RESEARCH_QUEUE_LEN {
            return Err(GameError::ResearchQueueFull {
                capacity: MAX_RESEARCH_QUEUE_LEN,
            });
        }
        for prereq in def.prerequisites {
            if !fr.completed.contains(prereq) {
                return Err(GameError::TechPrerequisiteUnmet {
                    tech: tech_id,
                    prerequisite: *prereq,
                });
            }
        }

        let job_id = game_types::ResearchJobId::new(self.next_job_id);
        self.next_job_id += 1;

        fr.queue.push(ResearchJob {
            id: job_id,
            tech_id,
            faction_id: faction,
            state: ResearchJobState::Queued,
            facility: None,
            reservations: Vec::new(),
            progress_ticks: 0,
            total_ticks: def.duration_ticks,
            queued_tick: tick,
        });

        self.metrics.jobs_queued += 1;
        journal.record(
            tick,
            SimEvent::ResearchQueued {
                faction_id: faction,
                tech_id,
                job_id,
            },
        );
        Ok(job_id)
    }

    /// Cancel a queued or active research job.
    ///
    /// Every outstanding input reservation is released back to available balance,
    /// i.e. a full material refund. Elapsed research ticks are forfeited.
    pub fn cancel_research(
        &mut self,
        faction: FactionId,
        job_id: game_types::ResearchJobId,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<u32> {
        let fr = self
            .factions
            .get_mut(&faction)
            .ok_or(GameError::ResearchJobNotFound(job_id))?;
        let index = fr
            .queue
            .iter()
            .position(|j| j.id == job_id)
            .ok_or(GameError::ResearchJobNotFound(job_id))?;
        let job = fr.queue.remove(index);

        let mut refunded_units = 0u32;
        if let Some(facility_id) = job.facility
            && let Some(facility) = self.facilities.get_mut(&facility_id)
        {
            for reservation in &job.reservations {
                if let Ok((_, amount)) = facility.input_inventory.release_reservation(*reservation)
                {
                    refunded_units += amount;
                }
            }
        }

        self.metrics.jobs_cancelled += 1;
        journal.record(
            tick,
            SimEvent::ResearchCancelled {
                faction_id: faction,
                tech_id: job.tech_id,
                job_id,
                refunded_units,
            },
        );
        Ok(refunded_units)
    }

    /// Move a queued job to a new position in the faction queue.
    ///
    /// Reordering is allowed at any time; an active job that loses the head slot
    /// keeps its progress and reservations and simply resumes when it returns.
    pub fn reorder_queue(
        &mut self,
        faction: FactionId,
        job_id: game_types::ResearchJobId,
        new_index: usize,
    ) -> GameResult<()> {
        let fr = self
            .factions
            .get_mut(&faction)
            .ok_or(GameError::ResearchJobNotFound(job_id))?;
        let current = fr
            .queue
            .iter()
            .position(|j| j.id == job_id)
            .ok_or(GameError::ResearchJobNotFound(job_id))?;
        if fr.queue.is_empty() {
            return Err(GameError::ResearchJobNotFound(job_id));
        }
        let target = new_index.min(fr.queue.len() - 1);
        if target == current {
            return Ok(());
        }
        let job = fr.queue.remove(current);
        fr.queue.insert(target, job);
        Ok(())
    }

    /// Instantly grant a technology without cost (map setup, saves, debug).
    ///
    /// Prerequisites are still enforced so granted state can never be inconsistent.
    pub fn grant_tech(
        &mut self,
        faction: FactionId,
        tech_id: TechId,
        tick: SimTick,
        journal: &mut EventJournal,
    ) -> GameResult<()> {
        let def = self
            .tech_tree
            .get(tech_id)
            .ok_or(GameError::TechNotFound(tech_id))?;
        {
            let fr = self.factions.entry(faction).or_default();
            if fr.completed.contains(&tech_id) {
                return Err(GameError::TechAlreadyResearched(tech_id));
            }
            for prereq in def.prerequisites {
                if !fr.completed.contains(prereq) {
                    return Err(GameError::TechPrerequisiteUnmet {
                        tech: tech_id,
                        prerequisite: *prereq,
                    });
                }
            }
        }
        self.apply_completion(faction, def, tick, journal);
        Ok(())
    }

    /// Mark a technology complete: record it, grant unlocks, distribute modifiers.
    fn apply_completion(
        &mut self,
        faction: FactionId,
        def: &'static TechDef,
        tick: SimTick,
        journal: &mut EventJournal,
    ) {
        let fr = self.factions.entry(faction).or_default();
        fr.completed.insert(def.id);
        for unlock in def.unlocks {
            fr.unlocked.insert(*unlock);
        }

        self.modifiers.add_source(faction, def.id, def.modifiers);
        self.metrics.jobs_completed += 1;
        self.metrics.modifier_patches_published += 1;

        journal.record(
            tick,
            SimEvent::ResearchCompleted {
                faction_id: faction,
                tech_id: def.id,
            },
        );
        journal.record(
            tick,
            SimEvent::ModifierPatchDistributed {
                faction_id: faction,
                patch_version: self.modifiers.version(),
                source_count: self.modifiers.source_count(faction),
            },
        );
    }

    /// Select the facility that should host a faction's active job.
    ///
    /// Deterministic: the lowest-id powered operational facility, otherwise the
    /// lowest-id operational facility, otherwise none.
    fn select_facility(&self, faction: FactionId) -> Option<(StructureId, bool)> {
        let mut fallback: Option<StructureId> = None;
        for (id, facility) in &self.facilities {
            if facility.faction_id != faction || !facility.operational {
                continue;
            }
            if facility.powered {
                return Some((*id, true));
            }
            if fallback.is_none() {
                fallback = Some(*id);
            }
        }
        fallback.map(|id| (id, false))
    }

    /// Advance all faction research queues by one authoritative tick, then
    /// distribute the resulting modifier patch across the structure network.
    pub fn tick(
        &mut self,
        tick: SimTick,
        registry: &mut StructureRegistry,
        journal: &mut EventJournal,
    ) {
        self.metrics.ticks_processed += 1;
        self.sync_facilities(registry);

        let factions: Vec<FactionId> = self.factions.keys().copied().collect();
        for faction in factions {
            self.tick_faction(faction, tick, journal);
        }

        registry.install_modifier_patch(&self.modifiers);
    }

    /// Advance a single faction's active research job.
    fn tick_faction(&mut self, faction: FactionId, tick: SimTick, journal: &mut EventJournal) {
        let selected = self.select_facility(faction);

        // Completion is deferred so the facility/queue borrows end first.
        let mut completed_tech: Option<&'static TechDef> = None;

        {
            let Some(fr) = self.factions.get_mut(&faction) else {
                return;
            };
            let Some(job) = fr.queue.first_mut() else {
                return;
            };
            let Some(def) = self.tech_tree.get(job.tech_id) else {
                return;
            };

            let Some((facility_id, powered)) = selected else {
                if job.state != ResearchJobState::Queued {
                    job.state = ResearchJobState::Queued;
                    journal.record(
                        tick,
                        SimEvent::ResearchBlocked {
                            faction_id: faction,
                            tech_id: job.tech_id,
                            reason: ResearchBlockedReason::NoFacility,
                        },
                    );
                }
                return;
            };

            if !powered {
                if job.state != ResearchJobState::Unpowered {
                    job.state = ResearchJobState::Unpowered;
                    journal.record(
                        tick,
                        SimEvent::ResearchBlocked {
                            faction_id: faction,
                            tech_id: job.tech_id,
                            reason: ResearchBlockedReason::Unpowered,
                        },
                    );
                }
                return;
            }

            let Some(facility) = self.facilities.get_mut(&facility_id) else {
                return;
            };

            match job.state {
                ResearchJobState::Queued | ResearchJobState::AwaitingResources => {
                    // Authoritative resource gate: every input must be present
                    // and unreserved before a single tick of progress happens.
                    let affordable = def
                        .cost
                        .iter()
                        .all(|&(res, amt)| facility.input_inventory.available_quantity(res) >= amt);

                    if !affordable {
                        if job.state != ResearchJobState::AwaitingResources {
                            job.state = ResearchJobState::AwaitingResources;
                            journal.record(
                                tick,
                                SimEvent::ResearchBlocked {
                                    faction_id: faction,
                                    tech_id: job.tech_id,
                                    reason: ResearchBlockedReason::AwaitingResources,
                                },
                            );
                        }
                        return;
                    }

                    let mut reservations = Vec::with_capacity(def.cost.len());
                    let target = Some(EntityId::new(facility_id.value()));
                    let mut failed = false;
                    for &(res, amt) in def.cost {
                        let reservation_id = facility.next_reservation_id();
                        if facility
                            .input_inventory
                            .reserve(reservation_id, res, amt, tick, target)
                            .is_err()
                        {
                            failed = true;
                            break;
                        }
                        reservations.push(reservation_id);
                    }

                    if failed {
                        for reservation in &reservations {
                            let _ = facility.input_inventory.release_reservation(*reservation);
                        }
                        job.state = ResearchJobState::AwaitingResources;
                        return;
                    }

                    job.reservations = reservations;
                    job.facility = Some(facility_id);
                    job.progress_ticks = 0;
                    job.total_ticks = self.modifiers.duration_ticks_for(
                        faction,
                        ModifierKind::ResearchSpeed,
                        def.duration_ticks,
                    );
                    job.state = ResearchJobState::InProgress;
                    self.metrics.jobs_started += 1;
                    journal.record(
                        tick,
                        SimEvent::ResearchStarted {
                            faction_id: faction,
                            tech_id: job.tech_id,
                            job_id: job.id,
                            finish_tick: tick + job.total_ticks as u64,
                        },
                    );
                }
                ResearchJobState::Unpowered => {
                    // Power restored: resume with preserved progress.
                    job.state = ResearchJobState::InProgress;
                }
                ResearchJobState::InProgress => {
                    job.progress_ticks += 1;
                    if job.progress_ticks >= job.total_ticks {
                        // Authoritative consumption happens exactly once, here.
                        let mut committed = true;
                        for reservation in &job.reservations {
                            if facility
                                .input_inventory
                                .commit_reservation(*reservation)
                                .is_err()
                            {
                                committed = false;
                            }
                        }
                        if committed {
                            facility.total_techs_completed += 1;
                            completed_tech = Some(def);
                        } else {
                            // Reservation state was lost; restart the job cleanly.
                            job.reservations.clear();
                            job.progress_ticks = 0;
                            job.state = ResearchJobState::AwaitingResources;
                        }
                    }
                }
            }
        }

        if let Some(def) = completed_tech {
            if let Some(fr) = self.factions.get_mut(&faction)
                && !fr.queue.is_empty()
            {
                fr.queue.remove(0);
            }
            self.apply_completion(faction, def, tick, journal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modifier::ModifierGroup;
    use crate::structure::{BuildRequest, StructureRegistry};
    use game_types::{RES_IRON_ORE, RES_STONE};

    const F1: FactionId = FactionId(1);
    const R1: RegionId = RegionId(1);
    const WORLD: (f32, f32, f32, f32) = (-500.0, 500.0, -500.0, 500.0);

    fn build(
        registry: &mut StructureRegistry,
        kind: StructureKind,
        pos: (f32, f32, f32),
    ) -> StructureId {
        let id = registry
            .request_build(
                BuildRequest {
                    player_pos: pos,
                    requested_pos: pos,
                    kind,
                    rotation_deg: 0.0,
                    faction_id: F1,
                    region_id: R1,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: WORLD,
                },
                None,
            )
            .unwrap();
        registry.complete_construction(id).unwrap();
        id
    }

    /// Builds a powered research facility next to a generator.
    fn powered_lab(registry: &mut StructureRegistry) -> StructureId {
        build(registry, StructureKind::Generator, (0.0, 0.0, 0.0));
        build(registry, StructureKind::ResearchFacility, (8.0, 0.0, 0.0))
    }

    fn stock(manager: &mut ResearchManager, lab: StructureId, def: &TechDef) {
        let facility = manager.facility_mut(lab).unwrap();
        for &(res, amount) in def.cost {
            facility.input_inventory.add(res, amount).unwrap();
        }
    }

    fn run(
        manager: &mut ResearchManager,
        registry: &mut StructureRegistry,
        journal: &mut EventJournal,
        start: u64,
        ticks: u64,
    ) -> SimTick {
        let mut tick = SimTick::new(start);
        for _ in 0..ticks {
            tick = tick.next();
            registry.tick_with_journal(tick, journal);
            manager.tick(tick, registry, journal);
        }
        tick
    }

    #[test]
    fn test_builtin_tech_tree_loads_and_validates() {
        let tree = TechTree::builtin();
        assert!(
            tree.load_error.is_none(),
            "built-in catalog failed validation: {:?}",
            tree.load_error
        );
        assert_eq!(tree.version(), TECH_TREE_VERSION);
        assert_eq!(tree.len(), STATIC_TECH_CATALOG.len());
        // A topological order exists, so the graph is acyclic.
        let order = tree.topological_order().unwrap();
        assert_eq!(order.len(), STATIC_TECH_CATALOG.len());
        // Prerequisites always precede dependents in the ordering.
        for def in tree.iter() {
            let own = order.iter().position(|id| *id == def.id).unwrap();
            for prereq in def.prerequisites {
                let p = order.iter().position(|id| id == prereq).unwrap();
                assert!(p < own, "{prereq} must precede {}", def.id);
            }
        }
    }

    #[test]
    fn test_tech_tree_rejects_prerequisite_cycle_with_actionable_error() {
        static CYCLIC: &[TechDef] = &[
            TechDef {
                id: TechId(700),
                name: "Alpha",
                tier: 1,
                prerequisites: &[TechId(701)],
                cost: &[],
                duration_ticks: 10,
                unlocks: &[],
                modifiers: &[],
            },
            TechDef {
                id: TechId(701),
                name: "Beta",
                tier: 1,
                prerequisites: &[TechId(700)],
                cost: &[],
                duration_ticks: 10,
                unlocks: &[],
                modifiers: &[],
            },
        ];

        let err = TechTree::load(1, CYCLIC).unwrap_err();
        match err {
            GameError::TechTreeInvalid(msg) => {
                assert!(msg.contains("cycle"), "message not actionable: {msg}");
                assert!(msg.contains("Alpha") && msg.contains("Beta"), "msg: {msg}");
            }
            other => panic!("expected TechTreeInvalid, got {other:?}"),
        }
    }

    #[test]
    fn test_tech_tree_rejects_dangling_and_duplicate_and_zero_duration() {
        static DANGLING: &[TechDef] = &[TechDef {
            id: TechId(710),
            name: "Orphan",
            tier: 1,
            prerequisites: &[TechId(999)],
            cost: &[],
            duration_ticks: 10,
            unlocks: &[],
            modifiers: &[],
        }];
        assert!(matches!(
            TechTree::load(1, DANGLING),
            Err(GameError::TechTreeInvalid(_))
        ));

        static DUPLICATE: &[TechDef] = &[
            TechDef {
                id: TechId(720),
                name: "First",
                tier: 1,
                prerequisites: &[],
                cost: &[],
                duration_ticks: 10,
                unlocks: &[],
                modifiers: &[],
            },
            TechDef {
                id: TechId(720),
                name: "Second",
                tier: 1,
                prerequisites: &[],
                cost: &[],
                duration_ticks: 10,
                unlocks: &[],
                modifiers: &[],
            },
        ];
        assert!(matches!(
            TechTree::load(1, DUPLICATE),
            Err(GameError::TechTreeInvalid(_))
        ));

        static INSTANT: &[TechDef] = &[TechDef {
            id: TechId(730),
            name: "Instant",
            tier: 1,
            prerequisites: &[],
            cost: &[],
            duration_ticks: 0,
            unlocks: &[],
            modifiers: &[],
        }];
        assert!(matches!(
            TechTree::load(1, INSTANT),
            Err(GameError::TechTreeInvalid(_))
        ));

        static SELF_REF: &[TechDef] = &[TechDef {
            id: TechId(740),
            name: "Ouroboros",
            tier: 1,
            prerequisites: &[TechId(740)],
            cost: &[],
            duration_ticks: 10,
            unlocks: &[],
            modifiers: &[],
        }];
        assert!(matches!(
            TechTree::load(1, SELF_REF),
            Err(GameError::TechTreeInvalid(_))
        ));
    }

    #[test]
    fn test_queue_rejects_unmet_prerequisite_and_duplicates() {
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();

        // Tungsten Processing requires Advanced Alloys.
        let err = manager
            .queue_research(F1, TECH_TUNGSTEN_PROCESSING, SimTick::zero(), &mut journal)
            .unwrap_err();
        assert_eq!(
            err,
            GameError::TechPrerequisiteUnmet {
                tech: TECH_TUNGSTEN_PROCESSING,
                prerequisite: TECH_ADVANCED_ALLOYS,
            }
        );

        manager
            .queue_research(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal)
            .unwrap();
        assert!(matches!(
            manager.queue_research(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal),
            Err(GameError::TechAlreadyQueued(_))
        ));
        assert!(matches!(
            manager.queue_research(F1, TechId::new(60_000), SimTick::zero(), &mut journal),
            Err(GameError::TechNotFound(_))
        ));
    }

    #[test]
    fn test_research_queue_is_deterministically_ordered_and_reorderable() {
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();

        let a = manager
            .queue_research(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal)
            .unwrap();
        let b = manager
            .queue_research(F1, TECH_POWER_REGULATION, SimTick::zero(), &mut journal)
            .unwrap();
        assert_eq!(
            manager.queue(F1).iter().map(|j| j.id).collect::<Vec<_>>(),
            vec![a, b]
        );

        manager.reorder_queue(F1, b, 0).unwrap();
        assert_eq!(
            manager.queue(F1).iter().map(|j| j.id).collect::<Vec<_>>(),
            vec![b, a]
        );

        // Out-of-range index clamps to the tail instead of erroring.
        manager.reorder_queue(F1, b, 99).unwrap();
        assert_eq!(
            manager.queue(F1).iter().map(|j| j.id).collect::<Vec<_>>(),
            vec![a, b]
        );

        assert!(matches!(
            manager.reorder_queue(F1, game_types::ResearchJobId::new(4242), 0),
            Err(GameError::ResearchJobNotFound(_))
        ));
    }

    #[test]
    fn test_research_completes_with_resources_power_and_time() {
        let mut registry = StructureRegistry::new();
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();
        let lab = powered_lab(&mut registry);

        manager.sync_facilities(&registry);
        let def = manager.tech_tree.get(TECH_BASIC_METALLURGY).unwrap();
        stock(&mut manager, lab, def);
        manager
            .queue_research(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal)
            .unwrap();

        // One warm-up tick starts the job, then duration_ticks of progress.
        run(
            &mut manager,
            &mut registry,
            &mut journal,
            0,
            def.duration_ticks as u64 + 4,
        );

        assert!(manager.is_completed(F1, TECH_BASIC_METALLURGY));
        assert!(manager.queue(F1).is_empty());
        // Materials were consumed authoritatively, not merely reserved.
        let facility = manager.facility(lab).unwrap();
        assert_eq!(facility.input_inventory.total_quantity(RES_STEEL), 0);
        assert_eq!(facility.total_techs_completed, 1);
    }

    /// ACCEPTANCE (negative 1/3): no resources -> research can never complete.
    #[test]
    fn test_acceptance_research_cannot_complete_without_resources() {
        let mut registry = StructureRegistry::new();
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();
        powered_lab(&mut registry);
        manager.sync_facilities(&registry);

        let def = manager.tech_tree.get(TECH_BASIC_METALLURGY).unwrap();
        manager
            .queue_research(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal)
            .unwrap();

        run(
            &mut manager,
            &mut registry,
            &mut journal,
            0,
            def.duration_ticks as u64 * 3,
        );

        assert!(!manager.is_completed(F1, TECH_BASIC_METALLURGY));
        assert_eq!(
            manager.queue(F1)[0].state,
            ResearchJobState::AwaitingResources
        );
        assert_eq!(manager.queue(F1)[0].progress_ticks, 0);
    }

    /// ACCEPTANCE (negative 2/3): no power -> research can never complete.
    #[test]
    fn test_acceptance_research_cannot_complete_without_power() {
        let mut registry = StructureRegistry::new();
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();

        // Research facility with no generator anywhere: permanently unpowered.
        let lab = build(
            &mut registry,
            StructureKind::ResearchFacility,
            (20.0, 0.0, 20.0),
        );
        manager.sync_facilities(&registry);
        let def = manager.tech_tree.get(TECH_BASIC_METALLURGY).unwrap();
        stock(&mut manager, lab, def);
        manager
            .queue_research(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal)
            .unwrap();

        run(
            &mut manager,
            &mut registry,
            &mut journal,
            0,
            def.duration_ticks as u64 * 3,
        );

        assert!(!manager.is_completed(F1, TECH_BASIC_METALLURGY));
        assert_eq!(manager.queue(F1)[0].state, ResearchJobState::Unpowered);
        // Resources were never even reserved, let alone consumed.
        let facility = manager.facility(lab).unwrap();
        assert_eq!(facility.input_inventory.total_quantity(RES_STEEL), 20);
    }

    /// ACCEPTANCE (negative 3/3): resources + power but insufficient time ->
    /// research is still incomplete, and completes only once time elapses.
    #[test]
    fn test_acceptance_research_cannot_complete_without_time() {
        let mut registry = StructureRegistry::new();
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();
        let lab = powered_lab(&mut registry);
        manager.sync_facilities(&registry);

        let def = manager.tech_tree.get(TECH_BASIC_METALLURGY).unwrap();
        stock(&mut manager, lab, def);
        manager
            .queue_research(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal)
            .unwrap();

        let stopped = run(
            &mut manager,
            &mut registry,
            &mut journal,
            0,
            def.duration_ticks as u64 - 1,
        );
        assert!(!manager.is_completed(F1, TECH_BASIC_METALLURGY));
        assert_eq!(manager.queue(F1)[0].state, ResearchJobState::InProgress);
        assert!(manager.queue(F1)[0].progress_ticks < def.duration_ticks);

        run(
            &mut manager,
            &mut registry,
            &mut journal,
            stopped.value(),
            8,
        );
        assert!(manager.is_completed(F1, TECH_BASIC_METALLURGY));
    }

    #[test]
    fn test_research_pauses_on_power_loss_and_resumes_with_progress_intact() {
        let mut registry = StructureRegistry::new();
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();
        let generator = build(&mut registry, StructureKind::Generator, (0.0, 0.0, 0.0));
        let lab = build(
            &mut registry,
            StructureKind::ResearchFacility,
            (8.0, 0.0, 0.0),
        );
        manager.sync_facilities(&registry);

        let def = manager.tech_tree.get(TECH_BASIC_METALLURGY).unwrap();
        stock(&mut manager, lab, def);
        manager
            .queue_research(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal)
            .unwrap();

        let t = run(&mut manager, &mut registry, &mut journal, 0, 20);
        let progress_before = manager.queue(F1)[0].progress_ticks;
        assert!(progress_before > 0);

        // Cut power by removing the only generator.
        registry.remove_structure(generator);
        registry.power_network.invalidate_topology();
        let t = run(&mut manager, &mut registry, &mut journal, t.value(), 20);
        assert_eq!(manager.queue(F1)[0].state, ResearchJobState::Unpowered);
        assert_eq!(manager.queue(F1)[0].progress_ticks, progress_before);

        // Restore power: progress resumes from where it stopped.
        build(&mut registry, StructureKind::Generator, (0.0, 0.0, 0.0));
        let _ = run(&mut manager, &mut registry, &mut journal, t.value(), 20);
        assert_eq!(manager.queue(F1)[0].state, ResearchJobState::InProgress);
        assert!(manager.queue(F1)[0].progress_ticks > progress_before);
    }

    #[test]
    fn test_cancel_refunds_all_reserved_materials_in_full() {
        let mut registry = StructureRegistry::new();
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();
        let lab = powered_lab(&mut registry);
        manager.sync_facilities(&registry);

        let def = manager.tech_tree.get(TECH_POWER_REGULATION).unwrap();
        stock(&mut manager, lab, def);
        let job = manager
            .queue_research(F1, TECH_POWER_REGULATION, SimTick::zero(), &mut journal)
            .unwrap();

        let t = run(&mut manager, &mut registry, &mut journal, 0, 10);
        // Inputs are locked, not spent.
        let facility = manager.facility(lab).unwrap();
        assert_eq!(facility.input_inventory.available_quantity(RES_STEEL), 0);
        assert_eq!(facility.input_inventory.total_quantity(RES_STEEL), 15);

        let refunded = manager.cancel_research(F1, job, t, &mut journal).unwrap();
        assert_eq!(refunded, 20); // 15 steel + 5 energy cells
        let facility = manager.facility(lab).unwrap();
        assert_eq!(facility.input_inventory.available_quantity(RES_STEEL), 15);
        assert_eq!(
            facility.input_inventory.available_quantity(RES_ENERGY_CELL),
            5
        );
        assert!(manager.queue(F1).is_empty());
        assert!(!manager.is_completed(F1, TECH_POWER_REGULATION));
    }

    /// ACCEPTANCE: unlocks are data-driven. A brand new technology declared purely
    /// as data gates a structure kind, a recipe, a chassis and an upgrade, and
    /// distributes a modifier, with zero new simulation code.
    #[test]
    fn test_acceptance_unlocks_are_data_driven() {
        const NEW_TECH: TechId = TechId(5150);
        const NEW_CHASSIS: ChassisId = ChassisId(77);
        const NEW_UPGRADE: ItemId = ItemId(950);

        static CONTENT_PACK: &[TechDef] = &[TechDef {
            id: NEW_TECH,
            name: "Experimental Siege Doctrine",
            tier: 1,
            prerequisites: &[],
            cost: &[(RES_IRON_ORE, 4)],
            duration_ticks: 12,
            unlocks: &[
                UnlockTarget::Structure(StructureKind::Turret),
                UnlockTarget::Recipe(RecipeId(4242)),
                UnlockTarget::RobotChassis(NEW_CHASSIS),
                UnlockTarget::Upgrade(NEW_UPGRADE),
            ],
            modifiers: &[Modifier::patch(ModifierKind::WeaponDamage, 400)],
        }];

        let mut registry = StructureRegistry::new();
        let mut manager = ResearchManager::from_catalog(7, CONTENT_PACK).unwrap();
        let mut journal = EventJournal::new();
        assert_eq!(manager.tech_tree.version(), 7);

        let lab = powered_lab(&mut registry);
        manager.sync_facilities(&registry);

        // Before research the data-declared targets are gated.
        assert!(!manager.can_build(F1, StructureKind::Turret));
        assert!(!manager.can_use_recipe(F1, RecipeId(4242)));
        assert!(!manager.can_fabricate_chassis(F1, NEW_CHASSIS));
        assert!(!manager.has_upgrade(F1, NEW_UPGRADE));
        // Targets no technology mentions stay ungated.
        assert!(manager.can_build(F1, StructureKind::Depot));
        assert_eq!(
            manager
                .modifiers
                .multiplier_milli(F1, ModifierKind::WeaponDamage),
            1000
        );

        let def = manager.tech_tree.get(NEW_TECH).unwrap();
        stock(&mut manager, lab, def);
        manager
            .queue_research(F1, NEW_TECH, SimTick::zero(), &mut journal)
            .unwrap();
        run(&mut manager, &mut registry, &mut journal, 0, 20);

        // After research every data-declared unlock took effect.
        assert!(manager.is_completed(F1, NEW_TECH));
        assert!(manager.can_build(F1, StructureKind::Turret));
        assert!(manager.can_use_recipe(F1, RecipeId(4242)));
        assert!(manager.can_fabricate_chassis(F1, NEW_CHASSIS));
        assert!(manager.has_upgrade(F1, NEW_UPGRADE));
        assert_eq!(
            manager
                .modifiers
                .multiplier_milli(F1, ModifierKind::WeaponDamage),
            1400
        );
        // Another faction is unaffected.
        assert!(!manager.can_build(FactionId::new(2), StructureKind::Turret));
    }

    /// ACCEPTANCE: the upgrade system never needs a new unit class per tier. The
    /// same structure archetype gets strictly stronger purely through modifiers.
    #[test]
    fn test_acceptance_upgrades_need_no_new_unit_class_per_tier() {
        const T1: TechId = TechId(6100);
        const T2: TechId = TechId(6101);
        const T3: TechId = TechId(6102);

        static TIERS: &[TechDef] = &[
            TechDef {
                id: T1,
                name: "Turret Firmware Mk.I",
                tier: 1,
                prerequisites: &[],
                cost: &[],
                duration_ticks: 5,
                unlocks: &[],
                modifiers: &[
                    Modifier::patch(ModifierKind::WeaponDamage, 100),
                    Modifier::patch(ModifierKind::StructureIntegrity, 100),
                ],
            },
            TechDef {
                id: T2,
                name: "Turret Firmware Mk.II",
                tier: 2,
                prerequisites: &[T1],
                cost: &[],
                duration_ticks: 5,
                unlocks: &[],
                modifiers: &[
                    Modifier::patch(ModifierKind::WeaponDamage, 100),
                    Modifier::patch(ModifierKind::StructureIntegrity, 100),
                ],
            },
            TechDef {
                id: T3,
                name: "Turret Firmware Mk.III",
                tier: 3,
                prerequisites: &[T2],
                cost: &[],
                duration_ticks: 5,
                unlocks: &[],
                modifiers: &[
                    Modifier::new(ModifierKind::WeaponDamage, ModifierGroup::Efficiency, 250),
                    Modifier::patch(ModifierKind::StructureIntegrity, 100),
                ],
            },
        ];

        let mut manager = ResearchManager::from_catalog(1, TIERS).unwrap();
        let mut journal = EventJournal::new();

        // A single archetype. Its base stats never change.
        let base_damage = 40.0f32;
        let base_hp = StructureKind::Turret.max_health();

        let mut damage_by_tier = Vec::new();
        let mut hp_by_tier = Vec::new();
        damage_by_tier.push(manager.modifier_value(F1, ModifierKind::WeaponDamage, base_damage));
        hp_by_tier.push(manager.modifiers.value_for_u32(
            F1,
            ModifierKind::StructureIntegrity,
            base_hp,
        ));

        for tech in [T1, T2, T3] {
            manager
                .grant_tech(F1, tech, SimTick::zero(), &mut journal)
                .unwrap();
            damage_by_tier.push(manager.modifier_value(
                F1,
                ModifierKind::WeaponDamage,
                base_damage,
            ));
            hp_by_tier.push(manager.modifiers.value_for_u32(
                F1,
                ModifierKind::StructureIntegrity,
                base_hp,
            ));
        }

        // Exactly one archetype, four strictly increasing stat tiers.
        assert_eq!(damage_by_tier.len(), 4);
        for w in damage_by_tier.windows(2) {
            assert!(w[1] > w[0], "damage did not increase: {w:?}");
        }
        for w in hp_by_tier.windows(2) {
            assert!(w[1] > w[0], "integrity did not increase: {w:?}");
        }
        // Mk.I + Mk.II add (x1.2), Mk.III compounds multiplicatively (x1.25).
        assert_eq!(
            manager
                .modifiers
                .multiplier_milli(F1, ModifierKind::WeaponDamage),
            1500
        );
        assert_eq!(
            manager
                .modifiers
                .multiplier_milli(F1, ModifierKind::StructureIntegrity),
            1300
        );
    }

    #[test]
    fn test_completion_is_order_independent_across_insertion_orders() {
        let orders = [
            [
                TECH_BASIC_METALLURGY,
                TECH_POWER_REGULATION,
                TECH_DRILL_OPTIMIZATION,
            ],
            [
                TECH_POWER_REGULATION,
                TECH_BASIC_METALLURGY,
                TECH_DRILL_OPTIMIZATION,
            ],
        ];

        let mut results = Vec::new();
        for order in orders {
            let mut manager = ResearchManager::new();
            let mut journal = EventJournal::new();
            for tech in order {
                manager
                    .grant_tech(F1, tech, SimTick::zero(), &mut journal)
                    .unwrap();
            }
            results.push(
                manager
                    .modifiers
                    .active_kinds(F1)
                    .iter()
                    .map(|(k, v)| (*k, *v))
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(results[0], results[1]);
        assert!(!results[0].is_empty());
    }

    #[test]
    fn test_grant_tech_enforces_prerequisites_and_double_grant() {
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();
        assert!(matches!(
            manager.grant_tech(F1, TECH_ADVANCED_ALLOYS, SimTick::zero(), &mut journal),
            Err(GameError::TechPrerequisiteUnmet { .. })
        ));
        manager
            .grant_tech(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal)
            .unwrap();
        assert!(matches!(
            manager.grant_tech(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal),
            Err(GameError::TechAlreadyResearched(_))
        ));
    }

    #[test]
    fn test_queue_capacity_is_enforced() {
        const fn filler(id: u32) -> TechDef {
            TechDef {
                id: TechId(id),
                name: "Filler",
                tier: 1,
                prerequisites: &[],
                cost: &[],
                duration_ticks: 50,
                unlocks: &[],
                modifiers: &[],
            }
        }
        static MANY: &[TechDef] = &[
            filler(800),
            filler(801),
            filler(802),
            filler(803),
            filler(804),
            filler(805),
            filler(806),
            filler(807),
            filler(808),
            filler(809),
            filler(810),
            filler(811),
            filler(812),
            filler(813),
            filler(814),
            filler(815),
            filler(816),
        ];
        assert_eq!(MANY.len(), MAX_RESEARCH_QUEUE_LEN + 1);

        let mut manager = ResearchManager::from_catalog(1, MANY).unwrap();
        let mut journal = EventJournal::new();
        for def in MANY.iter().take(MAX_RESEARCH_QUEUE_LEN) {
            manager
                .queue_research(F1, def.id, SimTick::zero(), &mut journal)
                .unwrap();
        }
        assert_eq!(manager.queue(F1).len(), MAX_RESEARCH_QUEUE_LEN);
        assert!(matches!(
            manager.queue_research(F1, TechId(816), SimTick::zero(), &mut journal),
            Err(GameError::ResearchQueueFull { .. })
        ));
    }

    #[test]
    fn test_available_techs_follows_prerequisite_frontier() {
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();
        let initial = manager.available_techs(F1);
        assert!(initial.contains(&TECH_BASIC_METALLURGY));
        assert!(initial.contains(&TECH_POWER_REGULATION));
        assert!(!initial.contains(&TECH_ADVANCED_ALLOYS));

        manager
            .grant_tech(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal)
            .unwrap();
        let next = manager.available_techs(F1);
        assert!(!next.contains(&TECH_BASIC_METALLURGY));
        assert!(next.contains(&TECH_ADVANCED_ALLOYS));
        assert!(next.contains(&TECH_DRILL_OPTIMIZATION));
    }

    #[test]
    fn test_facility_sync_tracks_construction_and_removal() {
        let mut registry = StructureRegistry::new();
        let mut manager = ResearchManager::new();

        let lab = build(
            &mut registry,
            StructureKind::ResearchFacility,
            (30.0, 0.0, 30.0),
        );
        manager.sync_facilities(&registry);
        assert!(manager.facility(lab).is_some());
        assert!(manager.facility(lab).unwrap().operational);

        registry.remove_structure(lab);
        manager.sync_facilities(&registry);
        assert!(manager.facility(lab).is_none());
    }

    #[test]
    fn test_unrelated_resources_do_not_satisfy_research_cost() {
        let mut registry = StructureRegistry::new();
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();
        let lab = powered_lab(&mut registry);
        manager.sync_facilities(&registry);

        // Fill the hopper with the wrong material entirely.
        manager
            .facility_mut(lab)
            .unwrap()
            .input_inventory
            .add(RES_STONE, 50)
            .unwrap();
        manager
            .queue_research(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal)
            .unwrap();
        run(&mut manager, &mut registry, &mut journal, 0, 200);

        assert!(!manager.is_completed(F1, TECH_BASIC_METALLURGY));
        assert_eq!(
            manager.queue(F1)[0].state,
            ResearchJobState::AwaitingResources
        );
    }

    #[test]
    fn test_research_speed_modifier_shortens_later_jobs() {
        let mut registry = StructureRegistry::new();
        let mut manager = ResearchManager::new();
        let mut journal = EventJournal::new();
        let lab = powered_lab(&mut registry);
        manager.sync_facilities(&registry);

        manager
            .grant_tech(F1, TECH_POWER_REGULATION, SimTick::zero(), &mut journal)
            .unwrap();
        manager
            .grant_tech(F1, TECH_RESEARCH_AUTOMATION, SimTick::zero(), &mut journal)
            .unwrap();

        let def = manager.tech_tree.get(TECH_BASIC_METALLURGY).unwrap();
        stock(&mut manager, lab, def);
        manager
            .queue_research(F1, TECH_BASIC_METALLURGY, SimTick::zero(), &mut journal)
            .unwrap();
        run(&mut manager, &mut registry, &mut journal, 0, 2);

        // +20% research speed: 90 ticks -> 75 ticks.
        assert_eq!(manager.queue(F1)[0].total_ticks, 75);
        assert_eq!(def.duration_ticks, 90);
    }
}
