use crate::combat::WeaponDef;
use crate::wall::ArmorProfile;
use game_types::{RES_ADVANCED_COMPONENTS, RES_BASIC_COMPONENTS, RES_STEEL, ResourceId, WeaponId};

/// Armor classification shared by every mobile chassis.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd, Default)]
pub enum ArmorClass {
    Light,
    #[default]
    Medium,
    Heavy,
}

impl ArmorClass {
    /// Mitigation profile fed into the single authoritative damage formula in `wall.rs`.
    pub const fn profile(&self) -> ArmorProfile {
        match self {
            ArmorClass::Light => ArmorProfile::new(4.0, 0.05),
            ArmorClass::Medium => ArmorProfile::new(10.0, 0.15),
            ArmorClass::Heavy => ArmorProfile::new(22.0, 0.30),
        }
    }

    pub const fn as_u8(&self) -> u8 {
        match self {
            ArmorClass::Light => 1,
            ArmorClass::Medium => 2,
            ArmorClass::Heavy => 3,
        }
    }

    pub const fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(ArmorClass::Light),
            2 => Some(ArmorClass::Medium),
            3 => Some(ArmorClass::Heavy),
            _ => None,
        }
    }
}

/// Data-driven biped chassis classification.
///
/// Adding a chassis is a table entry (one enum variant plus one `RobotArchetype` static);
/// no behaviour branches on the chassis anywhere in the simulation. Milestone 18 extends
/// this table with the specialist roster and should not need to touch `robot.rs` at all.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd, Default)]
pub enum RobotChassis {
    /// Personal escort biped (Milestone 12 / spec 3.1).
    #[default]
    Guardsman,
    /// Baseline line-infantry biped.
    Rifleman,
    /// Heavy anti-armor penetrator biped.
    AntiArmor,
    /// Lobbed grenade explosive support biped.
    Grenadier,
    /// Hostile swarm biped with mandibles.
    Swarmer,
    /// Hostile high-mass battering ram charger.
    Charger,
    /// Hostile ranged bio-acid spitter.
    Spitter,
    /// Hostile lumbering siege artillery organism.
    Bombardier,
}

impl RobotChassis {
    /// Every chassis in the table, in deterministic order.
    pub const ALL: [RobotChassis; 8] = [
        RobotChassis::Guardsman,
        RobotChassis::Rifleman,
        RobotChassis::AntiArmor,
        RobotChassis::Grenadier,
        RobotChassis::Swarmer,
        RobotChassis::Charger,
        RobotChassis::Spitter,
        RobotChassis::Bombardier,
    ];

    pub const fn as_u8(&self) -> u8 {
        match self {
            RobotChassis::Guardsman => 1,
            RobotChassis::Rifleman => 2,
            RobotChassis::AntiArmor => 3,
            RobotChassis::Grenadier => 4,
            RobotChassis::Swarmer => 5,
            RobotChassis::Charger => 6,
            RobotChassis::Spitter => 7,
            RobotChassis::Bombardier => 8,
        }
    }

    pub const fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(RobotChassis::Guardsman),
            2 => Some(RobotChassis::Rifleman),
            3 => Some(RobotChassis::AntiArmor),
            4 => Some(RobotChassis::Grenadier),
            5 => Some(RobotChassis::Swarmer),
            6 => Some(RobotChassis::Charger),
            7 => Some(RobotChassis::Spitter),
            8 => Some(RobotChassis::Bombardier),
            _ => None,
        }
    }

    pub const fn is_rogue(&self) -> bool {
        matches!(
            self,
            RobotChassis::Swarmer
                | RobotChassis::Charger
                | RobotChassis::Spitter
                | RobotChassis::Bombardier
        )
    }

    /// Static archetype data definition for this chassis.
    pub fn archetype(&self) -> &'static RobotArchetype {
        match self {
            RobotChassis::Guardsman => &GUARDSMAN_ARCHETYPE,
            RobotChassis::Rifleman => &RIFLEMAN_ARCHETYPE,
            RobotChassis::AntiArmor => &ANTI_ARMOR_ARCHETYPE,
            RobotChassis::Grenadier => &GRENADIER_ARCHETYPE,
            RobotChassis::Swarmer => &SWARMER_ARCHETYPE,
            RobotChassis::Charger => &CHARGER_ARCHETYPE,
            RobotChassis::Spitter => &SPITTER_ARCHETYPE,
            RobotChassis::Bombardier => &BOMBARDIER_ARCHETYPE,
        }
    }

    pub fn display_name(&self) -> &'static str {
        self.archetype().name
    }
}

/// Data-driven definition of a biped robot chassis.
#[derive(Clone, Debug, PartialEq)]
pub struct RobotArchetype {
    pub chassis: RobotChassis,
    pub name: &'static str,
    pub mass_kg: f32,
    /// Maximum ground speed in meters per second.
    pub move_speed: f32,
    /// Maximum planar acceleration in meters per second squared.
    pub acceleration: f32,
    /// Maximum yaw rate in degrees per second.
    pub turn_rate_deg: f32,
    pub max_health: u32,
    pub armor_class: ArmorClass,
    /// Passive detection radius in meters (feeds the Milestone 14 knowledge network).
    pub sensor_radius: f32,
    /// Physical body radius in meters.
    pub body_radius: f32,
    /// Default trailing distance kept behind a follow target.
    pub follow_standoff: f32,
    /// Distance at which a movement goal counts as reached.
    pub arrival_tolerance: f32,
    /// Personal space radius used for local separation.
    pub separation_radius: f32,
    pub build_cost: &'static [(ResourceId, u32)],
    /// Electrical draw of the producing facility while this chassis is on the line.
    pub build_power_kw: u32,
    pub build_ticks: u32,
}

impl RobotArchetype {
    /// Armor mitigation profile for this chassis.
    pub const fn armor_profile(&self) -> ArmorProfile {
        self.armor_class.profile()
    }

    /// Default primary weapon equipped on this chassis.
    pub fn default_weapon(&self, weapon_id: WeaponId) -> Option<WeaponDef> {
        match self.chassis {
            RobotChassis::Guardsman => Some(WeaponDef::new_pdw(weapon_id)),
            RobotChassis::Rifleman => Some(WeaponDef::new_rifle(weapon_id)),
            RobotChassis::AntiArmor => Some(WeaponDef::new_anti_armor_rail(weapon_id)),
            RobotChassis::Grenadier => Some(WeaponDef::new_grenade_launcher(weapon_id)),
            RobotChassis::Swarmer => Some(WeaponDef::new_swarmer_mandibles(weapon_id)),
            RobotChassis::Charger => None, // Ramming impact collision
            RobotChassis::Spitter => Some(WeaponDef::new_spitter_acid(weapon_id)),
            RobotChassis::Bombardier => Some(WeaponDef::new_bombardier_mortar(weapon_id)),
        }
    }
}

pub static GUARDSMAN_ARCHETYPE: RobotArchetype = RobotArchetype {
    chassis: RobotChassis::Guardsman,
    name: "Guardsman Escort Biped",
    mass_kg: 850.0,
    move_speed: 7.5,
    acceleration: 18.0,
    turn_rate_deg: 240.0,
    max_health: 900,
    armor_class: ArmorClass::Medium,
    sensor_radius: 45.0,
    body_radius: 0.6,
    follow_standoff: 4.0,
    arrival_tolerance: 0.6,
    separation_radius: 1.8,
    build_cost: &[(RES_STEEL, 25), (RES_BASIC_COMPONENTS, 10)],
    build_power_kw: 30,
    build_ticks: 300,
};

pub static RIFLEMAN_ARCHETYPE: RobotArchetype = RobotArchetype {
    chassis: RobotChassis::Rifleman,
    name: "Rifleman Line Biped",
    mass_kg: 780.0,
    move_speed: 6.5,
    acceleration: 16.0,
    turn_rate_deg: 220.0,
    max_health: 750,
    armor_class: ArmorClass::Light,
    sensor_radius: 55.0,
    body_radius: 0.55,
    follow_standoff: 5.0,
    arrival_tolerance: 0.75,
    separation_radius: 1.8,
    build_cost: &[
        (RES_STEEL, 20),
        (RES_BASIC_COMPONENTS, 8),
        (RES_ADVANCED_COMPONENTS, 2),
    ],
    build_power_kw: 25,
    build_ticks: 240,
};

pub static ANTI_ARMOR_ARCHETYPE: RobotArchetype = RobotArchetype {
    chassis: RobotChassis::AntiArmor,
    name: "Anti-Armor Heavy Biped",
    mass_kg: 1450.0,
    move_speed: 5.0,
    acceleration: 12.0,
    turn_rate_deg: 160.0,
    max_health: 1200,
    armor_class: ArmorClass::Heavy,
    sensor_radius: 70.0,
    body_radius: 0.8,
    follow_standoff: 6.0,
    arrival_tolerance: 0.8,
    separation_radius: 2.2,
    build_cost: &[
        (RES_STEEL, 40),
        (RES_BASIC_COMPONENTS, 15),
        (RES_ADVANCED_COMPONENTS, 5),
    ],
    build_power_kw: 45,
    build_ticks: 450,
};

pub static GRENADIER_ARCHETYPE: RobotArchetype = RobotArchetype {
    chassis: RobotChassis::Grenadier,
    name: "Grenadier Assault Biped",
    mass_kg: 890.0,
    move_speed: 6.0,
    acceleration: 15.0,
    turn_rate_deg: 200.0,
    max_health: 800,
    armor_class: ArmorClass::Medium,
    sensor_radius: 50.0,
    body_radius: 0.6,
    follow_standoff: 5.0,
    arrival_tolerance: 0.7,
    separation_radius: 1.8,
    build_cost: &[(RES_STEEL, 25), (RES_BASIC_COMPONENTS, 10)],
    build_power_kw: 30,
    build_ticks: 280,
};

pub static SWARMER_ARCHETYPE: RobotArchetype = RobotArchetype {
    chassis: RobotChassis::Swarmer,
    name: "Rogue Swarmer",
    mass_kg: 80.0,
    move_speed: 9.5,
    acceleration: 28.0,
    turn_rate_deg: 360.0,
    max_health: 75,
    armor_class: ArmorClass::Light,
    sensor_radius: 30.0,
    body_radius: 0.35,
    follow_standoff: 1.5,
    arrival_tolerance: 0.4,
    separation_radius: 0.9,
    build_cost: &[],
    build_power_kw: 0,
    build_ticks: 30,
};

pub static CHARGER_ARCHETYPE: RobotArchetype = RobotArchetype {
    chassis: RobotChassis::Charger,
    name: "Rogue Charger",
    mass_kg: 2500.0,
    move_speed: 11.0,
    acceleration: 22.0,
    turn_rate_deg: 90.0,
    max_health: 2000,
    armor_class: ArmorClass::Heavy,
    sensor_radius: 40.0,
    body_radius: 1.2,
    follow_standoff: 2.0,
    arrival_tolerance: 0.8,
    separation_radius: 2.5,
    build_cost: &[],
    build_power_kw: 0,
    build_ticks: 100,
};

pub static SPITTER_ARCHETYPE: RobotArchetype = RobotArchetype {
    chassis: RobotChassis::Spitter,
    name: "Rogue Acid Spitter",
    mass_kg: 450.0,
    move_speed: 6.0,
    acceleration: 14.0,
    turn_rate_deg: 180.0,
    max_health: 450,
    armor_class: ArmorClass::Light,
    sensor_radius: 50.0,
    body_radius: 0.55,
    follow_standoff: 4.0,
    arrival_tolerance: 0.6,
    separation_radius: 1.6,
    build_cost: &[],
    build_power_kw: 0,
    build_ticks: 60,
};

pub static BOMBARDIER_ARCHETYPE: RobotArchetype = RobotArchetype {
    chassis: RobotChassis::Bombardier,
    name: "Rogue Bombardier Siege Beast",
    mass_kg: 4500.0,
    move_speed: 3.5,
    acceleration: 6.0,
    turn_rate_deg: 60.0,
    max_health: 3500,
    armor_class: ArmorClass::Heavy,
    sensor_radius: 130.0,
    body_radius: 2.0,
    follow_standoff: 8.0,
    arrival_tolerance: 1.5,
    separation_radius: 3.5,
    build_cost: &[],
    build_power_kw: 0,
    build_ticks: 300,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chassis_archetype_table_is_data_driven() {
        assert_eq!(RobotChassis::ALL.len(), 8);
        for chassis in RobotChassis::ALL {
            let archetype = chassis.archetype();
            assert_eq!(archetype.chassis, chassis);
            assert!(archetype.max_health > 0);
            assert!(archetype.move_speed > 0.0);
            assert!(archetype.turn_rate_deg > 0.0);
            assert!(archetype.acceleration > 0.0);
            assert!(archetype.mass_kg > 0.0);
            assert!(archetype.sensor_radius > 0.0);
            assert!(archetype.body_radius > 0.0);
            assert!(archetype.follow_standoff > 0.0);
            if !chassis.is_rogue() {
                assert!(!archetype.build_cost.is_empty());
                assert!(archetype.build_power_kw > 0);
                assert!(archetype.build_ticks > 0);
            }
            assert_eq!(RobotChassis::from_u8(chassis.as_u8()), Some(chassis));
        }
        assert_eq!(RobotChassis::from_u8(0), None);
        assert_eq!(RobotChassis::from_u8(99), None);

        let guardsman = RobotChassis::Guardsman.archetype();
        assert_eq!(guardsman.name, "Guardsman Escort Biped");
        assert_eq!(RobotChassis::Guardsman.display_name(), guardsman.name);
        assert_eq!(guardsman.armor_class, ArmorClass::Medium);
        assert_eq!(
            guardsman.build_cost,
            &[(RES_STEEL, 25), (RES_BASIC_COMPONENTS, 10)]
        );
        assert!(guardsman.move_speed > 6.0);
    }

    #[test]
    fn test_armor_class_profiles_progress_and_round_trip() {
        assert!(ArmorClass::Light.profile().flat_armor < ArmorClass::Medium.profile().flat_armor);
        assert!(ArmorClass::Medium.profile().flat_armor < ArmorClass::Heavy.profile().flat_armor);
        assert!(
            ArmorClass::Light.profile().damage_reduction
                < ArmorClass::Heavy.profile().damage_reduction
        );

        for class in [ArmorClass::Light, ArmorClass::Medium, ArmorClass::Heavy] {
            assert_eq!(ArmorClass::from_u8(class.as_u8()), Some(class));
            assert!(class.profile().damage_reduction < 1.0);
        }
        assert_eq!(ArmorClass::from_u8(9), None);
        assert_eq!(
            GUARDSMAN_ARCHETYPE.armor_profile(),
            ArmorClass::Medium.profile()
        );
    }
}
