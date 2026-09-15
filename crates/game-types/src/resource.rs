use crate::error::{GameError, GameResult};
use crate::ids::ResourceId;
use std::fmt;
use std::ops::Deref;

/// Categorization of resources for logistics, processing, and tech tiers.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum ResourceCategory {
    /// Raw unrefined minerals gathered directly from resource nodes or terrain
    RawMineral,
    /// Smelted, refined, or synthesized metallurgical alloys and compounds
    RefinedAlloy,
    /// Power cells, batteries, or specialized energy charges
    Energy,
    /// Manufactured industrial parts, ammunition, electronic modules, and repair kits
    ManufacturedComponent,
}

impl fmt::Display for ResourceCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResourceCategory::RawMineral => write!(f, "Raw Mineral"),
            ResourceCategory::RefinedAlloy => write!(f, "Refined Alloy"),
            ResourceCategory::Energy => write!(f, "Energy"),
            ResourceCategory::ManufacturedComponent => write!(f, "Manufactured Component"),
        }
    }
}

// Canonical Resource Constants
pub const RES_IRON_ORE: ResourceId = ResourceId(1);
pub const RES_TUNGSTEN_ORE: ResourceId = ResourceId(2);
pub const RES_STONE: ResourceId = ResourceId(3);
pub const RES_SILICATES: ResourceId = ResourceId(4);

pub const RES_STEEL: ResourceId = ResourceId(10);
pub const RES_REFINED_TUNGSTEN: ResourceId = ResourceId(11);
pub const RES_HARDENED_STEEL: ResourceId = ResourceId(12);
pub const RES_CERAMIC_PLATE: ResourceId = ResourceId(13);
pub const RES_TUNGSTEN_COMPOSITE: ResourceId = ResourceId(14);

pub const RES_ENERGY_CELL: ResourceId = ResourceId(20);

pub const RES_BASIC_COMPONENTS: ResourceId = ResourceId(30);
pub const RES_ADVANCED_COMPONENTS: ResourceId = ResourceId(31);
pub const RES_AMMO: ResourceId = ResourceId(32);
pub const RES_REPAIR_KIT: ResourceId = ResourceId(33);

/// Definition and physical characteristics of a resource type.
#[derive(Clone, PartialEq, Debug)]
pub struct ResourceDefinition {
    pub id: ResourceId,
    pub name: &'static str,
    pub category: ResourceCategory,
    /// Volume consumed per unit in cubic meters (m^3)
    pub unit_volume: f32,
    /// Mass per unit in kilograms (kg)
    pub unit_mass: f32,
    /// Maximum standard stack quantity per inventory slot
    pub stack_limit: u32,
    /// Tech/material tier (1 = early/stone, 2 = mid/steel, 3 = late/tungsten composite)
    pub tier: u8,
}

const STATIC_RESOURCES: &[ResourceDefinition] = &[
    ResourceDefinition {
        id: RES_IRON_ORE,
        name: "Iron Ore",
        category: ResourceCategory::RawMineral,
        unit_volume: 0.005,
        unit_mass: 5.0,
        stack_limit: 1000,
        tier: 1,
    },
    ResourceDefinition {
        id: RES_TUNGSTEN_ORE,
        name: "Tungsten Ore",
        category: ResourceCategory::RawMineral,
        unit_volume: 0.004,
        unit_mass: 19.0,
        stack_limit: 1000,
        tier: 2,
    },
    ResourceDefinition {
        id: RES_STONE,
        name: "Stone",
        category: ResourceCategory::RawMineral,
        unit_volume: 0.006,
        unit_mass: 2.5,
        stack_limit: 2000,
        tier: 1,
    },
    ResourceDefinition {
        id: RES_SILICATES,
        name: "Silicates",
        category: ResourceCategory::RawMineral,
        unit_volume: 0.005,
        unit_mass: 2.3,
        stack_limit: 1000,
        tier: 1,
    },
    ResourceDefinition {
        id: RES_STEEL,
        name: "Steel Ingot",
        category: ResourceCategory::RefinedAlloy,
        unit_volume: 0.003,
        unit_mass: 7.8,
        stack_limit: 500,
        tier: 2,
    },
    ResourceDefinition {
        id: RES_REFINED_TUNGSTEN,
        name: "Refined Tungsten",
        category: ResourceCategory::RefinedAlloy,
        unit_volume: 0.002,
        unit_mass: 19.2,
        stack_limit: 500,
        tier: 2,
    },
    ResourceDefinition {
        id: RES_HARDENED_STEEL,
        name: "Hardened Steel",
        category: ResourceCategory::RefinedAlloy,
        unit_volume: 0.003,
        unit_mass: 8.0,
        stack_limit: 500,
        tier: 2,
    },
    ResourceDefinition {
        id: RES_CERAMIC_PLATE,
        name: "Ceramic Plate",
        category: ResourceCategory::RefinedAlloy,
        unit_volume: 0.004,
        unit_mass: 3.5,
        stack_limit: 500,
        tier: 2,
    },
    ResourceDefinition {
        id: RES_TUNGSTEN_COMPOSITE,
        name: "Tungsten Composite",
        category: ResourceCategory::RefinedAlloy,
        unit_volume: 0.003,
        unit_mass: 15.0,
        stack_limit: 250,
        tier: 3,
    },
    ResourceDefinition {
        id: RES_ENERGY_CELL,
        name: "Energy Cell",
        category: ResourceCategory::Energy,
        unit_volume: 0.001,
        unit_mass: 0.5,
        stack_limit: 500,
        tier: 1,
    },
    ResourceDefinition {
        id: RES_BASIC_COMPONENTS,
        name: "Basic Components",
        category: ResourceCategory::ManufacturedComponent,
        unit_volume: 0.002,
        unit_mass: 1.0,
        stack_limit: 500,
        tier: 1,
    },
    ResourceDefinition {
        id: RES_ADVANCED_COMPONENTS,
        name: "Advanced Components",
        category: ResourceCategory::ManufacturedComponent,
        unit_volume: 0.002,
        unit_mass: 1.2,
        stack_limit: 250,
        tier: 2,
    },
    ResourceDefinition {
        id: RES_AMMO,
        name: "Ballistic Ammo",
        category: ResourceCategory::ManufacturedComponent,
        unit_volume: 0.0005,
        unit_mass: 0.1,
        stack_limit: 5000,
        tier: 1,
    },
    ResourceDefinition {
        id: RES_REPAIR_KIT,
        name: "Repair Kit",
        category: ResourceCategory::ManufacturedComponent,
        unit_volume: 0.01,
        unit_mass: 5.0,
        stack_limit: 50,
        tier: 1,
    },
];

/// Global resource registry providing physical definitions and catalog queries.
pub struct ResourceRegistry;

impl ResourceRegistry {
    /// Retrieve resource definition by ID.
    pub fn get(id: ResourceId) -> Option<&'static ResourceDefinition> {
        STATIC_RESOURCES.iter().find(|def| def.id == id)
    }

    /// Retrieve resource definition by canonical name.
    pub fn get_by_name(name: &str) -> Option<&'static ResourceDefinition> {
        STATIC_RESOURCES
            .iter()
            .find(|def| def.name.eq_ignore_ascii_case(name))
    }

    /// Get all registered resources.
    pub fn all() -> &'static [ResourceDefinition] {
        STATIC_RESOURCES
    }

    /// Number of registered resource archetypes.
    pub fn count() -> usize {
        STATIC_RESOURCES.len()
    }
}

/// Compact resource quantity type with checked arithmetic and overflow prevention.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd, Default)]
#[repr(transparent)]
pub struct ResourceQuantity(pub u32);

impl ResourceQuantity {
    pub const ZERO: Self = Self(0);
    pub const MAX: Self = Self(u32::MAX);

    pub const fn new(value: u32) -> Self {
        ResourceQuantity(value)
    }

    pub const fn value(&self) -> u32 {
        self.0
    }

    pub const fn is_zero(&self) -> bool {
        self.0 == 0
    }

    /// Checked addition returning `GameError::ResourceOverflow` on overflow.
    pub fn checked_add(&self, amount: u32) -> GameResult<Self> {
        self.0
            .checked_add(amount)
            .map(ResourceQuantity)
            .ok_or(GameError::ResourceOverflow)
    }

    /// Checked subtraction returning `GameError::ResourceUnderflow` on underflow.
    pub fn checked_sub(&self, amount: u32) -> GameResult<Self> {
        self.0
            .checked_sub(amount)
            .map(ResourceQuantity)
            .ok_or(GameError::ResourceUnderflow)
    }

    /// Saturating addition clamping at `u32::MAX`.
    pub fn saturating_add(&self, amount: u32) -> Self {
        ResourceQuantity(self.0.saturating_add(amount))
    }

    /// Saturating subtraction clamping at 0.
    pub fn saturating_sub(&self, amount: u32) -> Self {
        ResourceQuantity(self.0.saturating_sub(amount))
    }
}

impl Deref for ResourceQuantity {
    type Target = u32;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<u32> for ResourceQuantity {
    fn from(val: u32) -> Self {
        ResourceQuantity(val)
    }
}

impl fmt::Display for ResourceQuantity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resource_registry_lookups() {
        assert_eq!(ResourceRegistry::count(), 14);
        let iron = ResourceRegistry::get(RES_IRON_ORE).expect("Iron ore exists");
        assert_eq!(iron.name, "Iron Ore");
        assert_eq!(iron.category, ResourceCategory::RawMineral);
        assert_eq!(iron.tier, 1);

        let steel = ResourceRegistry::get_by_name("steel ingot").expect("Steel exists");
        assert_eq!(steel.id, RES_STEEL);
        assert_eq!(steel.category, ResourceCategory::RefinedAlloy);

        let composite = ResourceRegistry::get(RES_TUNGSTEN_COMPOSITE).expect("Composite exists");
        assert_eq!(composite.tier, 3);
    }

    #[test]
    fn test_resource_quantity_checked_arithmetic() {
        let q = ResourceQuantity::new(100);
        assert_eq!(q.checked_add(50).unwrap().value(), 150);
        assert_eq!(q.checked_sub(30).unwrap().value(), 70);

        // Underflow error
        assert_eq!(q.checked_sub(101), Err(GameError::ResourceUnderflow));

        // Overflow error
        let max_q = ResourceQuantity::new(u32::MAX - 10);
        assert_eq!(max_q.checked_add(20), Err(GameError::ResourceOverflow));

        // Saturating
        assert_eq!(q.saturating_sub(200).value(), 0);
        assert_eq!(max_q.saturating_add(50).value(), u32::MAX);
    }
}
