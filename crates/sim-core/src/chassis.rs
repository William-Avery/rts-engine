use crate::wall::ArmorProfile;
use game_types::{RES_ADVANCED_COMPONENTS, RES_BASIC_COMPONENTS, RES_STEEL, ResourceId};

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
}

impl RobotChassis {
    /// Every chassis in the table, in deterministic order.
    pub const ALL: [RobotChassis; 2] = [RobotChassis::Guardsman, RobotChassis::Rifleman];

    pub const fn as_u8(&self) -> u8 {
        match self {
            RobotChassis::Guardsman => 1,
            RobotChassis::Rifleman => 2,
        }
    }

    pub const fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(RobotChassis::Guardsman),
            2 => Some(RobotChassis::Rifleman),
            _ => None,
        }
    }

    /// Static archetype data definition for this chassis.
    pub fn archetype(&self) -> &'static RobotArchetype {
        match self {
            RobotChassis::Guardsman => &GUARDSMAN_ARCHETYPE,
            RobotChassis::Rifleman => &RIFLEMAN_ARCHETYPE,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chassis_archetype_table_is_data_driven() {
        // Adding a chassis is a table entry; every variant resolves to static data.
        assert_eq!(RobotChassis::ALL.len(), 2);
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
            assert!(!archetype.build_cost.is_empty());
            assert!(archetype.build_power_kw > 0);
            assert!(archetype.build_ticks > 0);
            assert_eq!(RobotChassis::from_u8(chassis.as_u8()), Some(chassis));
        }
        assert_eq!(RobotChassis::from_u8(0), None);

        let guardsman = RobotChassis::Guardsman.archetype();
        assert_eq!(guardsman.name, "Guardsman Escort Biped");
        assert_eq!(RobotChassis::Guardsman.display_name(), guardsman.name);
        assert_eq!(guardsman.armor_class, ArmorClass::Medium);
        assert_eq!(
            guardsman.build_cost,
            &[(RES_STEEL, 25), (RES_BASIC_COMPONENTS, 10)]
        );
        // The escort must out-run a sprinting player to keep station.
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
            // The chassis archetype and its armor class must agree on the same profile.
            assert!(class.profile().damage_reduction < 1.0);
        }
        assert_eq!(ArmorClass::from_u8(9), None);
        assert_eq!(
            GUARDSMAN_ARCHETYPE.armor_profile(),
            ArmorClass::Medium.profile()
        );
    }
}
