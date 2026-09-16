use crate::wall::DamageSpec;
use game_types::{EntityId, FactionId, ProjectileId, ResourceId, SimTick, WeaponId};
use std::collections::BTreeMap;

/// Fundamental classification of damage energy transfer.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum DamageKind {
    /// Direct physical ballistic or cutting force.
    #[default]
    Kinetic,
    /// High-pressure shockwave and shrapnel expansion.
    Explosive,
    /// High-temperature plasma, burning hydrocarbons, or heat bloom.
    Thermal,
    /// Coherent laser, particle beam, or electrical discharge.
    Energy,
    /// Acidic dissolution that degrades structural integrity.
    Corrosive,
    /// Disruptive electromagnetic surge targeting circuitry.
    Electrical,
    /// Kinetic collision force scaled by mass and relative velocity.
    Impact,
}

impl DamageKind {
    pub const fn as_u8(&self) -> u8 {
        match self {
            DamageKind::Kinetic => 1,
            DamageKind::Explosive => 2,
            DamageKind::Thermal => 3,
            DamageKind::Energy => 4,
            DamageKind::Corrosive => 5,
            DamageKind::Electrical => 6,
            DamageKind::Impact => 7,
        }
    }

    pub const fn from_u8(val: u8) -> Option<Self> {
        match val {
            1 => Some(DamageKind::Kinetic),
            2 => Some(DamageKind::Explosive),
            3 => Some(DamageKind::Thermal),
            4 => Some(DamageKind::Energy),
            5 => Some(DamageKind::Corrosive),
            6 => Some(DamageKind::Electrical),
            7 => Some(DamageKind::Impact),
            _ => None,
        }
    }
}

/// Dynamic secondary combat effects, debuffs, and status conditions.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum CombatEffect {
    /// Continuous damage dealt over simulation ticks (e.g. fire, acid).
    DamageOverTime {
        dps: f32,
        duration_ticks: u32,
        kind: DamageKind,
    },
    /// Flat armor reduction stripping defensive plating.
    ArmorDegradation {
        flat_armor_reduction: f32,
        duration_ticks: u32,
    },
    /// Movement speed modifier (e.g. 0.6 = 40% slow).
    Slow {
        speed_multiplier: f32,
        duration_ticks: u32,
    },
    /// Stuns the target, halting movement and weapon cycling.
    StunEmp { duration_ticks: u32 },
    /// Directional impulse applied to target position or velocity.
    Knockback { impulse: (f32, f32, f32) },
    /// Combat suppression increasing weapon dispersion cone under fire.
    Suppression {
        accuracy_penalty_deg: f32,
        duration_ticks: u32,
    },
}

/// Active status tracking store for a combat entity.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct StatusStore {
    pub dot_effects: Vec<(f32, u32, DamageKind)>, // (damage_per_tick, remaining_ticks, kind)
    pub armor_degradations: Vec<(f32, u32)>,      // (reduction, remaining_ticks)
    pub slows: Vec<(f32, u32)>,                   // (multiplier, remaining_ticks)
    pub stun_ticks_remaining: u32,
    pub suppression_ticks_remaining: u32,
    pub suppression_penalty_deg: f32,
}

impl StatusStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply_effect(&mut self, effect: CombatEffect) {
        match effect {
            CombatEffect::DamageOverTime {
                dps,
                duration_ticks,
                kind,
            } => {
                if duration_ticks > 0 {
                    let dpt = dps / 30.0; // 30 Hz simulation cadence
                    self.dot_effects.push((dpt, duration_ticks, kind));
                }
            }
            CombatEffect::ArmorDegradation {
                flat_armor_reduction,
                duration_ticks,
            } => {
                if duration_ticks > 0 {
                    self.armor_degradations
                        .push((flat_armor_reduction, duration_ticks));
                }
            }
            CombatEffect::Slow {
                speed_multiplier,
                duration_ticks,
            } => {
                if duration_ticks > 0 {
                    self.slows
                        .push((speed_multiplier.clamp(0.0, 1.0), duration_ticks));
                }
            }
            CombatEffect::StunEmp { duration_ticks } => {
                self.stun_ticks_remaining = self.stun_ticks_remaining.max(duration_ticks);
            }
            CombatEffect::Knockback { .. } => {
                // Knockback is an immediate impulse applied to position/velocity
            }
            CombatEffect::Suppression {
                accuracy_penalty_deg,
                duration_ticks,
            } => {
                self.suppression_ticks_remaining =
                    self.suppression_ticks_remaining.max(duration_ticks);
                self.suppression_penalty_deg =
                    self.suppression_penalty_deg.max(accuracy_penalty_deg);
            }
        }
    }

    /// Advance one simulation tick: applies DoT damage and decrements timers.
    pub fn tick(&mut self) -> f32 {
        let mut total_dot_damage = 0.0;
        self.dot_effects.retain_mut(|(dpt, ticks, _)| {
            total_dot_damage += *dpt;
            *ticks = ticks.saturating_sub(1);
            *ticks > 0
        });

        self.armor_degradations.retain_mut(|(_, ticks)| {
            *ticks = ticks.saturating_sub(1);
            *ticks > 0
        });

        self.slows.retain_mut(|(_, ticks)| {
            *ticks = ticks.saturating_sub(1);
            *ticks > 0
        });

        self.stun_ticks_remaining = self.stun_ticks_remaining.saturating_sub(1);

        self.suppression_ticks_remaining = self.suppression_ticks_remaining.saturating_sub(1);
        if self.suppression_ticks_remaining == 0 {
            self.suppression_penalty_deg = 0.0;
        }

        total_dot_damage
    }

    /// Total flat armor reduction currently active.
    pub fn total_armor_degradation(&self) -> f32 {
        self.armor_degradations.iter().map(|(red, _)| *red).sum()
    }

    /// Combined speed multiplier from all active slows.
    pub fn speed_multiplier(&self) -> f32 {
        if self.stun_ticks_remaining > 0 {
            return 0.0;
        }
        let mut mult = 1.0;
        for (m, _) in &self.slows {
            mult *= *m;
        }
        mult.clamp(0.1, 1.0)
    }

    pub fn is_stunned(&self) -> bool {
        self.stun_ticks_remaining > 0
    }

    pub fn suppression_penalty_deg(&self) -> f32 {
        if self.suppression_ticks_remaining > 0 {
            self.suppression_penalty_deg
        } else {
            0.0
        }
    }
}

/// Physics and travel kinematics defining how an attack travels across the world.
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum MotionPrimitive {
    /// Instantaneous ray test within the tick.
    Hitscan { max_range: f32 },
    /// Straight linear trajectory at constant speed.
    Linear { speed: f32, max_range: f32 },
    /// Parabolic trajectory subject to gravitational acceleration.
    Ballistic {
        initial_velocity: (f32, f32, f32),
        gravity: f32,
    },
    /// Homing projectile steered toward a target entity with maximum turn rate.
    Guided {
        speed: f32,
        turn_rate_deg: f32,
        target: EntityId,
    },
    /// Continuous beam connecting source to target for a duration.
    Beam { duration_ticks: u32, max_range: f32 },
    /// Close-quarters melee strike.
    PhysicalMelee { reach: f32 },
    /// Area field / gravity well / stationary volume.
    AreaField {
        radius: f32,
        duration_ticks: u32,
        pull_force: f32,
    },
}

/// Simulated active projectile in flight across the world.
#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    pub id: ProjectileId,
    pub owner: Option<EntityId>,
    pub faction_id: FactionId,
    pub position: (f32, f32, f32),
    pub velocity: (f32, f32, f32),
    pub motion: MotionPrimitive,
    pub damage: DamageSpec,
    pub splash_radius: f32,
    pub spawn_tick: SimTick,
    pub expiration_tick: SimTick,
    pub active: bool,
    pub distance_traveled: f32,
    pub max_range: f32,
}

#[derive(Clone, Debug)]
pub struct LinearProjectileSpec {
    pub id: ProjectileId,
    pub owner: Option<EntityId>,
    pub faction_id: FactionId,
    pub origin: (f32, f32, f32),
    pub direction: (f32, f32, f32),
    pub speed: f32,
    pub max_range: f32,
    pub damage: DamageSpec,
    pub splash_radius: f32,
    pub spawn_tick: SimTick,
}

#[derive(Clone, Debug)]
pub struct BallisticProjectileSpec {
    pub id: ProjectileId,
    pub owner: Option<EntityId>,
    pub faction_id: FactionId,
    pub origin: (f32, f32, f32),
    pub initial_velocity: (f32, f32, f32),
    pub gravity: f32,
    pub max_range: f32,
    pub damage: DamageSpec,
    pub splash_radius: f32,
    pub spawn_tick: SimTick,
    pub max_flight_ticks: u64,
}

impl Projectile {
    pub fn new_linear(spec: LinearProjectileSpec) -> Self {
        let len = (spec.direction.0 * spec.direction.0
            + spec.direction.1 * spec.direction.1
            + spec.direction.2 * spec.direction.2)
            .sqrt();
        let (dir_x, dir_y, dir_z) = if len > 0.001 {
            (
                spec.direction.0 / len,
                spec.direction.1 / len,
                spec.direction.2 / len,
            )
        } else {
            (0.0, 0.0, 1.0)
        };
        let velocity = (dir_x * spec.speed, dir_y * spec.speed, dir_z * spec.speed);
        let ticks_to_live = if spec.speed > 0.0 {
            ((spec.max_range / spec.speed) * 30.0).ceil() as u64 + 1
        } else {
            30
        };

        Projectile {
            id: spec.id,
            owner: spec.owner,
            faction_id: spec.faction_id,
            position: spec.origin,
            velocity,
            motion: MotionPrimitive::Linear {
                speed: spec.speed,
                max_range: spec.max_range,
            },
            damage: spec.damage,
            splash_radius: spec.splash_radius,
            spawn_tick: spec.spawn_tick,
            expiration_tick: SimTick::new(spec.spawn_tick.value() + ticks_to_live),
            active: true,
            distance_traveled: 0.0,
            max_range: spec.max_range,
        }
    }

    pub fn new_ballistic(spec: BallisticProjectileSpec) -> Self {
        Projectile {
            id: spec.id,
            owner: spec.owner,
            faction_id: spec.faction_id,
            position: spec.origin,
            velocity: spec.initial_velocity,
            motion: MotionPrimitive::Ballistic {
                initial_velocity: spec.initial_velocity,
                gravity: spec.gravity,
            },
            damage: spec.damage,
            splash_radius: spec.splash_radius,
            spawn_tick: spec.spawn_tick,
            expiration_tick: SimTick::new(spec.spawn_tick.value() + spec.max_flight_ticks),
            active: true,
            distance_traveled: 0.0,
            max_range: spec.max_range,
        }
    }

    /// Advance projectile by one simulation step (dt = 1/30 s).
    /// Returns previous and updated position for swept raycast collision tests.
    pub fn step(&mut self, dt: f32) -> ((f32, f32, f32), (f32, f32, f32)) {
        let old_pos = self.position;
        match &mut self.motion {
            MotionPrimitive::Linear { speed, .. } => {
                let step_dist = *speed * dt;
                self.position.0 += self.velocity.0 * dt;
                self.position.1 += self.velocity.1 * dt;
                self.position.2 += self.velocity.2 * dt;
                self.distance_traveled += step_dist;
                if self.distance_traveled >= self.max_range {
                    self.active = false;
                }
            }
            MotionPrimitive::Ballistic { gravity, .. } => {
                self.position.0 += self.velocity.0 * dt;
                self.position.1 += self.velocity.1 * dt;
                self.position.2 += self.velocity.2 * dt;
                self.velocity.1 -= *gravity * dt; // gravity pulls in -Y
                let dx = self.position.0 - old_pos.0;
                let dy = self.position.1 - old_pos.1;
                let dz = self.position.2 - old_pos.2;
                self.distance_traveled += (dx * dx + dy * dy + dz * dz).sqrt();
                // Ground collision test: if projectile drops below y = 0.0, impact terrain
                if self.position.1 <= 0.0 {
                    self.position.1 = 0.0;
                    self.active = false;
                }
            }
            MotionPrimitive::Guided { speed, .. } => {
                // In guided mode, velocity tracks target direction within turn_rate_deg
                let step_dist = *speed * dt;
                self.position.0 += self.velocity.0 * dt;
                self.position.1 += self.velocity.1 * dt;
                self.position.2 += self.velocity.2 * dt;
                self.distance_traveled += step_dist;
                if self.distance_traveled >= self.max_range {
                    self.active = false;
                }
            }
            _ => {
                self.position.0 += self.velocity.0 * dt;
                self.position.1 += self.velocity.1 * dt;
                self.position.2 += self.velocity.2 * dt;
            }
        }
        (old_pos, self.position)
    }
}

/// Immutable data definition of a weapon system.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponDef {
    pub weapon_id: WeaponId,
    pub name: &'static str,
    /// Minimum ticks between shots (at 30 Hz: 3 ticks = 10 shots/sec).
    pub cycle_ticks: u32,
    pub magazine_capacity: u32,
    pub reload_ticks: u32,
    pub ammo_resource: ResourceId,
    pub ammo_per_shot: u32,
    pub range: f32,
    pub spread_cone_deg: f32,
    pub motion: MotionPrimitive,
    pub base_damage: DamageSpec,
    pub splash_radius: f32,
    pub burst_count: u32,
    pub burst_interval_ticks: u32,
}

impl WeaponDef {
    pub fn new_rifle(weapon_id: WeaponId) -> Self {
        WeaponDef {
            weapon_id,
            name: "Kinetic Battle Rifle",
            cycle_ticks: 6, // 5 shots/sec
            magazine_capacity: 30,
            reload_ticks: 45, // 1.5 sec
            ammo_resource: game_types::RES_AMMO,
            ammo_per_shot: 1,
            range: 55.0,
            spread_cone_deg: 2.0,
            motion: MotionPrimitive::Linear {
                speed: 500.0,
                max_range: 55.0,
            },
            base_damage: DamageSpec::new(28.0)
                .with_penetration(6.0)
                .with_kind(DamageKind::Kinetic),
            splash_radius: 0.0,
            burst_count: 1,
            burst_interval_ticks: 0,
        }
    }

    pub fn new_pdw(weapon_id: WeaponId) -> Self {
        WeaponDef {
            weapon_id,
            name: "Rapid Suppressive PDW",
            cycle_ticks: 3, // 10 shots/sec
            magazine_capacity: 40,
            reload_ticks: 30, // 1.0 sec
            ammo_resource: game_types::RES_AMMO,
            ammo_per_shot: 1,
            range: 25.0,
            spread_cone_deg: 6.0,
            motion: MotionPrimitive::Linear {
                speed: 380.0,
                max_range: 25.0,
            },
            base_damage: DamageSpec::new(14.0)
                .with_penetration(3.0)
                .with_kind(DamageKind::Kinetic)
                .with_effect(CombatEffect::Suppression {
                    accuracy_penalty_deg: 4.0,
                    duration_ticks: 15,
                }),
            splash_radius: 0.0,
            burst_count: 3,
            burst_interval_ticks: 2,
        }
    }

    pub fn new_anti_armor_rail(weapon_id: WeaponId) -> Self {
        WeaponDef {
            weapon_id,
            name: "Anti-Armor Kinetic Penetrator",
            cycle_ticks: 45, // 0.67 shots/sec
            magazine_capacity: 5,
            reload_ticks: 75, // 2.5 sec
            ammo_resource: game_types::RES_AMMO,
            ammo_per_shot: 5,
            range: 80.0,
            spread_cone_deg: 0.5,
            motion: MotionPrimitive::Linear {
                speed: 1000.0,
                max_range: 80.0,
            },
            base_damage: DamageSpec::new(160.0)
                .with_penetration(35.0)
                .with_kind(DamageKind::Kinetic),
            splash_radius: 0.0,
            burst_count: 1,
            burst_interval_ticks: 0,
        }
    }

    pub fn new_grenade_launcher(weapon_id: WeaponId) -> Self {
        WeaponDef {
            weapon_id,
            name: "Lobbed Grenade Launcher",
            cycle_ticks: 20, // 1.5 shots/sec
            magazine_capacity: 6,
            reload_ticks: 60, // 2.0 sec
            ammo_resource: game_types::RES_AMMO,
            ammo_per_shot: 2,
            range: 40.0,
            spread_cone_deg: 3.5,
            motion: MotionPrimitive::Ballistic {
                initial_velocity: (0.0, 15.0, 30.0),
                gravity: 9.81,
            },
            base_damage: DamageSpec::new(75.0)
                .with_penetration(12.0)
                .with_kind(DamageKind::Explosive),
            splash_radius: 4.5,
            burst_count: 1,
            burst_interval_ticks: 0,
        }
    }

    pub fn new_turret_autocannon(weapon_id: WeaponId) -> Self {
        WeaponDef {
            weapon_id,
            name: "Twin Point-Defense Autocannon",
            cycle_ticks: 2, // 15 shots/sec
            magazine_capacity: 200,
            reload_ticks: 60,
            ammo_resource: game_types::RES_AMMO,
            ammo_per_shot: 1,
            range: 45.0,
            spread_cone_deg: 2.5,
            motion: MotionPrimitive::Linear {
                speed: 600.0,
                max_range: 45.0,
            },
            base_damage: DamageSpec::new(22.0)
                .with_penetration(8.0)
                .with_kind(DamageKind::Kinetic),
            splash_radius: 0.0,
            burst_count: 2,
            burst_interval_ticks: 1,
        }
    }

    pub fn new_spitter_acid(weapon_id: WeaponId) -> Self {
        WeaponDef {
            weapon_id,
            name: "Caustic Bio-Acid Spitter",
            cycle_ticks: 25,
            magazine_capacity: 10,
            reload_ticks: 30,
            ammo_resource: ResourceId(0), // biological infinite ammo
            ammo_per_shot: 0,
            range: 35.0,
            spread_cone_deg: 4.0,
            motion: MotionPrimitive::Ballistic {
                initial_velocity: (0.0, 10.0, 20.0),
                gravity: 9.81,
            },
            base_damage: DamageSpec::new(35.0)
                .with_penetration(15.0)
                .with_kind(DamageKind::Corrosive)
                .with_effect(CombatEffect::ArmorDegradation {
                    flat_armor_reduction: 6.0,
                    duration_ticks: 150,
                }),
            splash_radius: 2.5,
            burst_count: 1,
            burst_interval_ticks: 0,
        }
    }

    pub fn new_bombardier_mortar(weapon_id: WeaponId) -> Self {
        WeaponDef {
            weapon_id,
            name: "Siege Organism Artillery Sac",
            cycle_ticks: 90, // 3 seconds per shot
            magazine_capacity: 1,
            reload_ticks: 90,
            ammo_resource: ResourceId(0),
            ammo_per_shot: 0,
            range: 120.0,
            spread_cone_deg: 5.0,
            motion: MotionPrimitive::Ballistic {
                initial_velocity: (0.0, 30.0, 40.0),
                gravity: 9.81,
            },
            base_damage: DamageSpec::new(180.0)
                .with_penetration(25.0)
                .with_kind(DamageKind::Explosive),
            splash_radius: 8.0,
            burst_count: 1,
            burst_interval_ticks: 0,
        }
    }

    pub fn new_swarmer_mandibles(weapon_id: WeaponId) -> Self {
        WeaponDef {
            weapon_id,
            name: "Swarmer Mandibles",
            cycle_ticks: 15,
            magazine_capacity: 1000,
            reload_ticks: 0,
            ammo_resource: ResourceId(0),
            ammo_per_shot: 0,
            range: 1.5,
            spread_cone_deg: 0.0,
            motion: MotionPrimitive::PhysicalMelee { reach: 1.5 },
            base_damage: DamageSpec::new(12.0)
                .with_penetration(1.0)
                .with_kind(DamageKind::Kinetic),
            splash_radius: 0.0,
            burst_count: 1,
            burst_interval_ticks: 0,
        }
    }
}

/// Mutable operational state of an entity's mounted weapon.
#[derive(Clone, Debug, PartialEq)]
pub struct WeaponState {
    pub def: WeaponDef,
    pub cooldown_remaining: u32,
    pub loaded_ammo: u32,
    pub reload_ticks_remaining: u32,
}

impl WeaponState {
    pub fn new(def: WeaponDef) -> Self {
        let loaded = def.magazine_capacity;
        WeaponState {
            def,
            cooldown_remaining: 0,
            loaded_ammo: loaded,
            reload_ticks_remaining: 0,
        }
    }

    pub fn can_fire(&self) -> bool {
        self.cooldown_remaining == 0 && self.loaded_ammo >= self.def.ammo_per_shot
    }

    /// Advance weapon timers by 1 simulation tick.
    pub fn tick(&mut self) {
        self.cooldown_remaining = self.cooldown_remaining.saturating_sub(1);
        if self.reload_ticks_remaining > 0 {
            self.reload_ticks_remaining = self.reload_ticks_remaining.saturating_sub(1);
            if self.reload_ticks_remaining == 0 {
                self.loaded_ammo = self.def.magazine_capacity;
            }
        }
    }

    /// Consume ammo and set weapon cooldown for the next shot.
    /// Returns true if the shot successfully discharged.
    pub fn discharge(&mut self, fire_rate_mod_milli: i64) -> bool {
        if !self.can_fire() {
            return false;
        }

        self.loaded_ammo = self.loaded_ammo.saturating_sub(self.def.ammo_per_shot);

        // Apply weapon fire rate modifier (higher milli == faster cycle == fewer ticks)
        let effective_cycle = if fire_rate_mod_milli > 0 {
            let scaled = (self.def.cycle_ticks as i64 * 1000) / fire_rate_mod_milli;
            (scaled.max(1)) as u32
        } else {
            self.def.cycle_ticks
        };
        self.cooldown_remaining = effective_cycle;

        // Auto-reload trigger when magazine is empty
        if self.loaded_ammo < self.def.ammo_per_shot && self.def.reload_ticks > 0 {
            self.reload_ticks_remaining = self.def.reload_ticks;
        }

        true
    }
}

/// Authoritative registry of all active projectiles currently in flight.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ProjectileRegistry {
    pub projectiles: BTreeMap<ProjectileId, Projectile>,
    next_id: u64,
}

/// Result of a projectile impacting a target or terrain.
#[derive(Clone, Debug, PartialEq)]
pub struct ProjectileImpact {
    pub projectile_id: ProjectileId,
    pub target: EntityId,
    pub hit_pos: (f32, f32, f32),
    pub damage: DamageSpec,
    pub splash_radius: f32,
    pub owner: Option<EntityId>,
    pub faction_id: FactionId,
}

impl ProjectileRegistry {
    pub fn new() -> Self {
        ProjectileRegistry {
            projectiles: BTreeMap::new(),
            next_id: 1,
        }
    }

    pub fn spawn(&mut self, mut projectile: Projectile) -> ProjectileId {
        let id = ProjectileId::new(self.next_id);
        self.next_id += 1;
        projectile.id = id;
        self.projectiles.insert(id, projectile);
        id
    }

    pub fn active_count(&self) -> usize {
        self.projectiles.values().filter(|p| p.active).count()
    }

    /// Advance all active projectiles by 1 tick (dt = 1/30 s).
    /// Calls `hit_test` closure for each projectile step.
    /// Returns any impacts and cleans up dead projectiles.
    pub fn step_all<F>(&mut self, current_tick: SimTick, mut hit_test: F) -> Vec<ProjectileImpact>
    where
        F: FnMut(
            &Projectile,
            (f32, f32, f32),
            (f32, f32, f32),
        ) -> Option<(EntityId, (f32, f32, f32))>,
    {
        let dt = 1.0 / 30.0;
        let mut impacts = Vec::new();

        for projectile in self.projectiles.values_mut() {
            if !projectile.active || current_tick >= projectile.expiration_tick {
                projectile.active = false;
                continue;
            }

            let (old_pos, new_pos) = projectile.step(dt);
            if let Some((hit_target, hit_pos)) = hit_test(projectile, old_pos, new_pos) {
                projectile.active = false;
                impacts.push(ProjectileImpact {
                    projectile_id: projectile.id,
                    target: hit_target,
                    hit_pos,
                    damage: projectile.damage,
                    splash_radius: projectile.splash_radius,
                    owner: projectile.owner,
                    faction_id: projectile.faction_id,
                });
            }
        }

        // Retain only active projectiles
        self.projectiles.retain(|_, p| p.active);

        impacts
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wall::{ArmorProfile, calculate_damage};

    #[test]
    fn test_status_store_dot_and_armor_degradation_tick_decay() {
        let mut store = StatusStore::new();
        assert_eq!(store.speed_multiplier(), 1.0);
        assert_eq!(store.total_armor_degradation(), 0.0);

        // Apply 60 DPS thermal DoT for 30 ticks (1 second), 10 flat armor strip for 10 ticks, 30% slow for 15 ticks
        store.apply_effect(CombatEffect::DamageOverTime {
            dps: 60.0,
            duration_ticks: 30,
            kind: DamageKind::Thermal,
        });
        store.apply_effect(CombatEffect::ArmorDegradation {
            flat_armor_reduction: 10.0,
            duration_ticks: 10,
        });
        store.apply_effect(CombatEffect::Slow {
            speed_multiplier: 0.7,
            duration_ticks: 15,
        });
        store.apply_effect(CombatEffect::Suppression {
            accuracy_penalty_deg: 12.0,
            duration_ticks: 20,
        });

        assert_eq!(store.total_armor_degradation(), 10.0);
        assert!((store.speed_multiplier() - 0.7).abs() < 1e-4);
        assert_eq!(store.suppression_penalty_deg(), 12.0);

        // Advance 10 ticks
        let mut accrued_dot = 0.0;
        for _ in 0..10 {
            accrued_dot += store.tick();
        }
        // 60 DPS / 30 Hz = 2.0 damage per tick -> 20.0 over 10 ticks
        assert!((accrued_dot - 20.0).abs() < 1e-3);
        // Armor degradation expired after 10 ticks
        assert_eq!(store.total_armor_degradation(), 0.0);
        // Slow still active (5 ticks remain)
        assert!((store.speed_multiplier() - 0.7).abs() < 1e-4);

        // Advance next 10 ticks (total 20 ticks)
        for _ in 0..10 {
            accrued_dot += store.tick();
        }
        assert!((accrued_dot - 40.0).abs() < 1e-3);
        // Slow expired
        assert_eq!(store.speed_multiplier(), 1.0);
        // Suppression expired at tick 20
        assert_eq!(store.suppression_penalty_deg(), 0.0);

        // Test Stun halting speed completely
        store.apply_effect(CombatEffect::StunEmp { duration_ticks: 5 });
        assert_eq!(store.speed_multiplier(), 0.0);
        assert!(store.is_stunned());
        for _ in 0..5 {
            store.tick();
        }
        assert_eq!(store.speed_multiplier(), 1.0);
        assert!(!store.is_stunned());
    }

    #[test]
    fn test_linear_projectile_kinematics_and_expiration() {
        let mut reg = ProjectileRegistry::new();
        let proj = Projectile::new_linear(LinearProjectileSpec {
            id: ProjectileId(1),
            owner: Some(EntityId::new(10)),
            faction_id: FactionId::new(1),
            origin: (0.0, 1.0, 0.0),
            direction: (0.0, 0.0, 1.0),
            speed: 30.0,    // 30 m/s -> 1.0 m per tick
            max_range: 5.0, // max range 5 m -> should expire after 5 ticks
            damage: DamageSpec::new(25.0).with_kind(DamageKind::Kinetic),
            splash_radius: 0.0,
            spawn_tick: SimTick::zero(),
        });
        let id = reg.spawn(proj);
        assert_eq!(reg.active_count(), 1);

        // Step 3 ticks without collision
        for t in 1..=3 {
            let impacts = reg.step_all(SimTick::new(t), |_p, _old, _new| None);
            assert!(impacts.is_empty());
        }
        let p = reg.projectiles.get(&id).unwrap();
        assert!((p.position.2 - 3.0).abs() < 1e-3);
        assert!((p.distance_traveled - 3.0).abs() < 1e-3);
        assert!(p.active);

        // Step 2 more ticks -> reaches 5.0m max range and deactivates
        reg.step_all(SimTick::new(4), |_p, _old, _new| None);
        reg.step_all(SimTick::new(5), |_p, _old, _new| None);
        assert_eq!(reg.active_count(), 0);
    }

    #[test]
    fn test_ballistic_projectile_parabolic_arc() {
        let mut proj = Projectile::new_ballistic(BallisticProjectileSpec {
            id: ProjectileId(1),
            owner: Some(EntityId::new(10)),
            faction_id: FactionId::new(1),
            origin: (0.0, 0.0, 0.0),
            initial_velocity: (0.0, 10.0, 20.0), // initial vy = 10, vz = 20
            gravity: 10.0,                       // gravity = 10 m/s^2
            max_range: 50.0,
            damage: DamageSpec::new(100.0).with_kind(DamageKind::Explosive),
            splash_radius: 5.0,
            spawn_tick: SimTick::zero(),
            max_flight_ticks: 30,
        });

        // Step 15 ticks (0.5 seconds)
        let dt = 1.0 / 30.0;
        for _ in 0..15 {
            proj.step(dt);
        }

        // Horizontal displacement: 20 * 0.5 = 10.0
        assert!((proj.position.2 - 10.0).abs() < 1e-2);
        // Vertical displacement under discrete Euler: 5.0 - 1.167 = 3.833
        assert!((proj.position.1 - 3.833).abs() < 1e-2);
    }

    #[test]
    fn test_weapon_discharge_cycle_and_auto_reload() {
        let def = WeaponDef::new_pdw(WeaponId(1));
        let mut weapon = WeaponState::new(def);

        assert!(weapon.can_fire());
        assert_eq!(weapon.loaded_ammo, 40);

        // Discharge 1 shot
        assert!(weapon.discharge(1000)); // standard 1.0x fire rate
        assert_eq!(weapon.loaded_ammo, 39);
        assert_eq!(weapon.cooldown_remaining, 3); // 3 ticks cooldown
        assert!(!weapon.can_fire());

        // Step 2 ticks -> still on cooldown
        weapon.tick();
        weapon.tick();
        assert!(!weapon.can_fire());

        // Step 3rd tick -> ready to fire
        weapon.tick();
        assert!(weapon.can_fire());

        // Test fire rate modification (e.g. 2000 milli = 2.0x fire rate = half cooldown)
        assert!(weapon.discharge(2000));
        assert_eq!(weapon.cooldown_remaining, 1); // 3 * 1000 / 2000 = 1 tick
    }

    #[test]
    fn test_damage_kind_penetration_and_armor_bypass() {
        let armor = ArmorProfile {
            flat_armor: 20.0,
            damage_reduction: 0.20,
        };

        // Standard Kinetic attack: 50 damage, 5 penetration
        // Net flat armor = 20 - 5 = 15
        // After flat: 50 - 15 = 35
        // After 20% reduction: 35 * 0.8 = 28.0
        let kinetic_dmg = DamageSpec::new(50.0)
            .with_penetration(5.0)
            .with_kind(DamageKind::Kinetic);
        let res = calculate_damage(armor, 100, kinetic_dmg);
        assert!((res.effective_damage - 28.0).abs() < 1e-3);

        // Energy attack: 50 damage, 5 penetration
        // Energy bypasses 50% of flat armor: 20 * 0.5 = 10 flat armor
        // Net flat armor = (10 - 5) = 5
        // After flat: 50 - 5 = 45
        // After 20% reduction: 45 * 0.8 = 36.0
        let energy_dmg = DamageSpec::new(50.0)
            .with_penetration(5.0)
            .with_kind(DamageKind::Energy);
        let res_energy = calculate_damage(armor, 100, energy_dmg);
        assert!((res_energy.effective_damage - 36.0).abs() < 1e-3);
    }

    #[test]
    fn test_charger_impact_physics_momentum_formula() {
        // Charger mass = 800 kg, running at 10 m/s, coeff = 0.5
        let mass = 800.0;
        let speed = 10.0;
        let impact = DamageSpec::new_impact(mass, speed, 0.5);

        assert_eq!(impact.damage_kind, DamageKind::Impact);
        // Raw = 800 * 10 * 0.5 = 4000.0
        assert_eq!(impact.raw_damage, 4000.0);
        // Penetration capped at 30.0
        assert_eq!(impact.armor_penetration, 30.0);
        assert!(matches!(
            impact.effect,
            Some(CombatEffect::Knockback { .. })
        ));
    }

    #[test]
    fn test_projectile_swept_collision_and_impact_return() {
        let mut reg = ProjectileRegistry::new();
        let target_entity = EntityId::new(42);

        let proj = Projectile::new_linear(LinearProjectileSpec {
            id: ProjectileId(1),
            owner: Some(EntityId::new(10)),
            faction_id: FactionId::new(1),
            origin: (0.0, 0.0, 0.0),
            direction: (0.0, 0.0, 1.0),
            speed: 60.0, // 2.0 m per tick
            max_range: 50.0,
            damage: DamageSpec::new(75.0),
            splash_radius: 3.0,
            spawn_tick: SimTick::zero(),
        });
        reg.spawn(proj);

        // Hit test detects target at z = 2.0
        let impacts = reg.step_all(SimTick::new(1), |_p, _old, new_pos| {
            if new_pos.2 >= 2.0 {
                Some((target_entity, (0.0, 0.0, 2.0)))
            } else {
                None
            }
        });

        assert_eq!(impacts.len(), 1);
        let impact = &impacts[0];
        assert_eq!(impact.target, target_entity);
        assert_eq!(impact.hit_pos, (0.0, 0.0, 2.0));
        assert_eq!(impact.damage.raw_damage, 75.0);
        assert_eq!(impact.splash_radius, 3.0);
        assert_eq!(reg.active_count(), 0); // Despawned upon impact
    }
}
