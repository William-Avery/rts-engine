use game_types::{
    EntityId, RES_REPAIR_KIT, RES_STEEL, RES_STONE, RES_TUNGSTEN_COMPOSITE, ResourceId,
};

/// Authoritative classification of defensive wall tiers.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum WallTier {
    #[default]
    Mk1Stone,
    Mk2Steel,
    Mk3Composite,
}

impl WallTier {
    /// Numeric tier identifier (1 = stone, 2 = steel, 3 = composite).
    pub const fn as_u8(&self) -> u8 {
        match self {
            WallTier::Mk1Stone => 1,
            WallTier::Mk2Steel => 2,
            WallTier::Mk3Composite => 3,
        }
    }

    /// Parse numeric tier identifier.
    pub const fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(WallTier::Mk1Stone),
            2 => Some(WallTier::Mk2Steel),
            3 => Some(WallTier::Mk3Composite),
            _ => None,
        }
    }

    /// Display name of the wall tier.
    pub const fn display_name(&self) -> &'static str {
        match self {
            WallTier::Mk1Stone => "Mk.1 Stone",
            WallTier::Mk2Steel => "Mk.2 Steel",
            WallTier::Mk3Composite => "Mk.3 Tungsten Composite",
        }
    }

    /// Cycle to the next tier (Mk1 -> Mk2 -> Mk3 -> Mk1).
    pub const fn cycle_next(&self) -> Self {
        match self {
            WallTier::Mk1Stone => WallTier::Mk2Steel,
            WallTier::Mk2Steel => WallTier::Mk3Composite,
            WallTier::Mk3Composite => WallTier::Mk1Stone,
        }
    }

    /// Static archetype data definition.
    pub fn archetype(&self) -> &'static WallArchetype {
        match self {
            WallTier::Mk1Stone => &MK1_STONE_ARCHETYPE,
            WallTier::Mk2Steel => &MK2_STEEL_ARCHETYPE,
            WallTier::Mk3Composite => &MK3_COMPOSITE_ARCHETYPE,
        }
    }
}

/// Data-driven definition and physical combat characteristics for a wall tier.
#[derive(Clone, Debug, PartialEq)]
pub struct WallArchetype {
    pub tier: WallTier,
    pub name: &'static str,
    pub max_health: u32,
    pub flat_armor: f32,
    /// Percentage damage mitigation in range [0.0, 1.0)
    pub damage_reduction: f32,
    pub construction_ticks: u32,
    pub dismantle_ticks: u32,
    pub construction_cost: &'static [(ResourceId, u32)],
    pub repair_resource: ResourceId,
    pub repair_hp_per_unit: f32,
}

pub static MK1_STONE_ARCHETYPE: WallArchetype = WallArchetype {
    tier: WallTier::Mk1Stone,
    name: "Mk.1 Stone Wall",
    max_health: 1000,
    flat_armor: 5.0,
    damage_reduction: 0.05,
    construction_ticks: 15,
    dismantle_ticks: 7,
    construction_cost: &[(RES_STONE, 20)],
    repair_resource: RES_STONE,
    repair_hp_per_unit: 50.0,
};

pub static MK2_STEEL_ARCHETYPE: WallArchetype = WallArchetype {
    tier: WallTier::Mk2Steel,
    name: "Mk.2 Steel Wall",
    max_health: 3000,
    flat_armor: 20.0,
    damage_reduction: 0.25,
    construction_ticks: 45,
    dismantle_ticks: 22,
    construction_cost: &[(RES_STEEL, 15)],
    repair_resource: RES_STEEL,
    repair_hp_per_unit: 100.0,
};

pub static MK3_COMPOSITE_ARCHETYPE: WallArchetype = WallArchetype {
    tier: WallTier::Mk3Composite,
    name: "Mk.3 Tungsten Composite Wall",
    max_health: 8000,
    flat_armor: 50.0,
    damage_reduction: 0.50,
    construction_ticks: 120,
    dismantle_ticks: 60,
    construction_cost: &[(RES_TUNGSTEN_COMPOSITE, 10), (RES_STEEL, 5)],
    repair_resource: RES_REPAIR_KIT,
    repair_hp_per_unit: 250.0,
};

/// Parameters specifying damage applied to a structure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageSpec {
    pub raw_damage: f32,
    pub armor_penetration: f32,
    pub source: Option<EntityId>,
}

impl DamageSpec {
    pub const fn new(raw_damage: f32) -> Self {
        DamageSpec {
            raw_damage,
            armor_penetration: 0.0,
            source: None,
        }
    }

    pub const fn with_penetration(mut self, armor_penetration: f32) -> Self {
        self.armor_penetration = armor_penetration;
        self
    }

    pub const fn with_source(mut self, source: EntityId) -> Self {
        self.source = Some(source);
        self
    }
}

/// Detailed outcome of damage applied against an armored, resistant structure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DamageResult {
    pub raw_damage: f32,
    pub absorbed_armor: f32,
    pub mitigated_resistance: f32,
    pub effective_damage: f32,
    pub remaining_hp: u32,
    pub destroyed: bool,
}

/// Authoritative calculation of damage mitigation for a wall archetype.
pub fn calculate_wall_damage(
    archetype: &WallArchetype,
    current_hp: u32,
    damage: DamageSpec,
) -> DamageResult {
    let effective_armor = (archetype.flat_armor - damage.armor_penetration).max(0.0);
    let post_armor = (damage.raw_damage - effective_armor).max(0.0);
    let absorbed_armor = damage.raw_damage - post_armor;
    let effective_damage = post_armor * (1.0 - archetype.damage_reduction);
    let mitigated_resistance = post_armor - effective_damage;
    let dmg_rounded = effective_damage.round() as u32;

    let (remaining_hp, destroyed) = if dmg_rounded >= current_hp {
        (0, true)
    } else {
        (current_hp - dmg_rounded, false)
    };

    DamageResult {
        raw_damage: damage.raw_damage,
        absorbed_armor,
        mitigated_resistance,
        effective_damage,
        remaining_hp,
        destroyed,
    }
}

/// Outcome of an authoritative repair operation on a structure.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RepairResult {
    pub hp_restored: u32,
    pub units_consumed: u32,
    pub material_used: ResourceId,
    pub new_hp: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wall_archetype_definitions() {
        let mk1 = WallTier::Mk1Stone.archetype();
        assert_eq!(mk1.max_health, 1000);
        assert_eq!(mk1.flat_armor, 5.0);
        assert_eq!(mk1.damage_reduction, 0.05);
        assert_eq!(mk1.construction_cost, &[(RES_STONE, 20)]);
        assert_eq!(mk1.repair_resource, RES_STONE);
        assert_eq!(mk1.repair_hp_per_unit, 50.0);

        let mk2 = WallTier::Mk2Steel.archetype();
        assert_eq!(mk2.max_health, 3000);
        assert_eq!(mk2.flat_armor, 20.0);
        assert_eq!(mk2.damage_reduction, 0.25);
        assert_eq!(mk2.construction_cost, &[(RES_STEEL, 15)]);
        assert_eq!(mk2.repair_resource, RES_STEEL);
        assert_eq!(mk2.repair_hp_per_unit, 100.0);

        let mk3 = WallTier::Mk3Composite.archetype();
        assert_eq!(mk3.max_health, 8000);
        assert_eq!(mk3.flat_armor, 50.0);
        assert_eq!(mk3.damage_reduction, 0.50);
        assert_eq!(
            mk3.construction_cost,
            &[(RES_TUNGSTEN_COMPOSITE, 10), (RES_STEEL, 5)]
        );
        assert_eq!(mk3.repair_resource, RES_REPAIR_KIT);
        assert_eq!(mk3.repair_hp_per_unit, 250.0);
    }

    #[test]
    fn test_material_resistance_formula_progression() {
        let raw_damage = 100.0;
        let spec = DamageSpec::new(raw_damage);

        // Mk.1 Stone: (100 - 5) * (1 - 0.05) = 95 * 0.95 = 90.25
        let res_mk1 = calculate_wall_damage(&MK1_STONE_ARCHETYPE, 1000, spec);
        assert_eq!(res_mk1.absorbed_armor, 5.0);
        assert!((res_mk1.effective_damage - 90.25).abs() < 1e-4);
        assert!((res_mk1.mitigated_resistance - 4.75).abs() < 1e-4);
        assert_eq!(res_mk1.remaining_hp, 910);
        assert!(!res_mk1.destroyed);

        // Mk.2 Steel: (100 - 20) * (1 - 0.25) = 80 * 0.75 = 60.0
        let res_mk2 = calculate_wall_damage(&MK2_STEEL_ARCHETYPE, 3000, spec);
        assert_eq!(res_mk2.absorbed_armor, 20.0);
        assert!((res_mk2.effective_damage - 60.0).abs() < 1e-4);
        assert!((res_mk2.mitigated_resistance - 20.0).abs() < 1e-4);
        assert_eq!(res_mk2.remaining_hp, 2940);
        assert!(!res_mk2.destroyed);

        // Mk.3 Composite: (100 - 50) * (1 - 0.50) = 50 * 0.50 = 25.0
        let res_mk3 = calculate_wall_damage(&MK3_COMPOSITE_ARCHETYPE, 8000, spec);
        assert_eq!(res_mk3.absorbed_armor, 50.0);
        assert!((res_mk3.effective_damage - 25.0).abs() < 1e-4);
        assert!((res_mk3.mitigated_resistance - 25.0).abs() < 1e-4);
        assert_eq!(res_mk3.remaining_hp, 7975);
        assert!(!res_mk3.destroyed);

        // Observable resistance difference verified: 90.25 > 60.0 > 25.0
        assert!(res_mk1.effective_damage > res_mk2.effective_damage);
        assert!(res_mk2.effective_damage > res_mk3.effective_damage);
    }

    #[test]
    fn test_armor_penetration_mitigation() {
        // Raw damage 100 with 30 armor penetration against Mk.3 (50 armor)
        // Effective armor = (50 - 30) = 20
        // Post armor = 100 - 20 = 80
        // Effective damage = 80 * (1 - 0.5) = 40.0
        let spec = DamageSpec::new(100.0).with_penetration(30.0);
        let res = calculate_wall_damage(&MK3_COMPOSITE_ARCHETYPE, 8000, spec);
        assert_eq!(res.absorbed_armor, 20.0);
        assert!((res.effective_damage - 40.0).abs() < 1e-4);
        assert_eq!(res.remaining_hp, 7960);
    }

    #[test]
    fn test_lethal_damage_destruction() {
        let spec = DamageSpec::new(2000.0);
        let res = calculate_wall_damage(&MK1_STONE_ARCHETYPE, 1000, spec);
        assert!(res.effective_damage > 1000.0);
        assert_eq!(res.remaining_hp, 0);
        assert!(res.destroyed);
    }

    #[test]
    fn test_tier_cycling_and_conversion() {
        assert_eq!(WallTier::Mk1Stone.as_u8(), 1);
        assert_eq!(WallTier::Mk2Steel.as_u8(), 2);
        assert_eq!(WallTier::Mk3Composite.as_u8(), 3);

        assert_eq!(WallTier::from_u8(1), Some(WallTier::Mk1Stone));
        assert_eq!(WallTier::from_u8(2), Some(WallTier::Mk2Steel));
        assert_eq!(WallTier::from_u8(3), Some(WallTier::Mk3Composite));
        assert_eq!(WallTier::from_u8(4), None);

        assert_eq!(WallTier::Mk1Stone.cycle_next(), WallTier::Mk2Steel);
        assert_eq!(WallTier::Mk2Steel.cycle_next(), WallTier::Mk3Composite);
        assert_eq!(WallTier::Mk3Composite.cycle_next(), WallTier::Mk1Stone);
    }
}
