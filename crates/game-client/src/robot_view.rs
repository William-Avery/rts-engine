use game_types::{EntityId, PlayerId};
use sim_core::chassis::RobotChassis;
use sim_core::robot::{Robot, RobotRegistry};
use std::collections::BTreeMap;

/// Level of detail selected for a biped robot instance by camera distance.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
pub enum RobotLod {
    /// Full skeleton, articulated digitigrade legs, accent lighting.
    High,
    /// Reduced bone count, no small mechanism detail.
    Medium,
    /// Static silhouette imposter.
    Low,
}

/// Locomotion pose selected for a biped robot instance.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
pub enum BipedGait {
    Idle,
    Walk,
    Run,
}

/// A single rendered biped robot instance.
///
/// Presentation only: this struct is derived from authoritative simulation state each
/// frame and never feeds anything back into the simulation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RobotInstance {
    pub entity: EntityId,
    pub chassis: RobotChassis,
    pub position: (f32, f32, f32),
    /// Yaw in degrees, 0 = +Z.
    pub facing_deg: f32,
    /// Health fraction in range [0.0, 1.0] for damage decals and hull sparks.
    pub health_ratio: f32,
    /// Normalized walk-cycle phase in range [0.0, 1.0).
    pub gait_phase: f32,
    pub gait: BipedGait,
    pub lod: RobotLod,
    /// Owning player when this robot is an assigned escort, for HUD markers.
    pub escort_owner: Option<PlayerId>,
}

/// A draw buffer batch of every instance sharing one chassis mesh and material.
#[derive(Clone, Debug, PartialEq)]
pub struct RobotBatch {
    pub chassis: RobotChassis,
    pub instances: Vec<RobotInstance>,
}

impl RobotBatch {
    pub fn new(chassis: RobotChassis) -> Self {
        RobotBatch {
            chassis,
            instances: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.instances.len()
    }

    pub fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    pub fn clear(&mut self) {
        self.instances.clear();
    }
}

/// Client-side batched biped robot renderer placeholder.
///
/// Partitions every visible robot into one instanced draw call per chassis, preserving
/// vector capacity across frames to avoid per-frame GPU buffer churn. This module holds
/// zero authority: it reads the authoritative registry and produces draw data only.
#[derive(Clone, Debug)]
pub struct BipedRobotRenderer {
    batches: BTreeMap<RobotChassis, RobotBatch>,
    total_instances: usize,
    /// Camera distance at which instances drop from High to Medium detail.
    pub lod_medium_distance: f32,
    /// Camera distance at which instances drop from Medium to Low detail.
    pub lod_low_distance: f32,
    /// Camera distance beyond which robots are culled entirely.
    pub cull_distance: f32,
    /// Meters of ground travel per full walk cycle.
    pub stride_length: f32,
    /// Planar speed above which the run cycle replaces the walk cycle.
    pub run_speed_threshold: f32,
}

impl Default for BipedRobotRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl BipedRobotRenderer {
    pub fn new() -> Self {
        let mut batches = BTreeMap::new();
        for chassis in RobotChassis::ALL {
            batches.insert(chassis, RobotBatch::new(chassis));
        }
        BipedRobotRenderer {
            batches,
            total_instances: 0,
            lod_medium_distance: 60.0,
            lod_low_distance: 160.0,
            cull_distance: 400.0,
            stride_length: 2.4,
            run_speed_threshold: 5.0,
        }
    }

    /// Reset all batches without deallocating their instance vectors.
    pub fn clear(&mut self) {
        for batch in self.batches.values_mut() {
            batch.clear();
        }
        self.total_instances = 0;
    }

    /// Add a prepared instance to its chassis batch.
    pub fn add_instance(&mut self, instance: RobotInstance) {
        self.batches
            .entry(instance.chassis)
            .or_insert_with(|| RobotBatch::new(instance.chassis))
            .instances
            .push(instance);
        self.total_instances += 1;
    }

    /// Level of detail for a given camera distance.
    pub fn lod_for_distance(&self, distance: f32) -> RobotLod {
        if distance <= self.lod_medium_distance {
            RobotLod::High
        } else if distance <= self.lod_low_distance {
            RobotLod::Medium
        } else {
            RobotLod::Low
        }
    }

    /// Gait selected for a planar ground speed.
    pub fn gait_for_speed(&self, speed: f32) -> BipedGait {
        if speed < 0.1 {
            BipedGait::Idle
        } else if speed < self.run_speed_threshold {
            BipedGait::Walk
        } else {
            BipedGait::Run
        }
    }

    /// Walk-cycle phase driven by distance travelled, so the feet do not skate.
    pub fn gait_phase(&self, travelled_meters: f32) -> f32 {
        let stride = self.stride_length.max(0.01);
        (travelled_meters / stride).rem_euclid(1.0)
    }

    /// Build one instance from an authoritative robot and the viewer's camera position.
    ///
    /// Returns `None` when the robot is beyond the cull distance.
    pub fn instance_for(
        &self,
        robot: &Robot,
        camera_position: (f32, f32, f32),
        elapsed_seconds: f32,
    ) -> Option<RobotInstance> {
        let dx = robot.position.0 - camera_position.0;
        let dz = robot.position.2 - camera_position.2;
        let distance = (dx * dx + dz * dz).sqrt();
        if distance > self.cull_distance {
            return None;
        }
        let speed = robot.planar_speed();
        Some(RobotInstance {
            entity: robot.entity,
            chassis: robot.chassis,
            position: robot.position,
            facing_deg: robot.facing_deg,
            health_ratio: robot.health_ratio(),
            gait_phase: self.gait_phase(speed * elapsed_seconds),
            gait: self.gait_for_speed(speed),
            lod: self.lod_for_distance(distance),
            escort_owner: robot.owner,
        })
    }

    /// Repopulate every batch from the authoritative robot registry.
    pub fn populate_from_registry(
        &mut self,
        registry: &RobotRegistry,
        camera_position: (f32, f32, f32),
        elapsed_seconds: f32,
    ) {
        self.clear();
        let prepared: Vec<RobotInstance> = registry
            .iter()
            .filter_map(|robot| self.instance_for(robot, camera_position, elapsed_seconds))
            .collect();
        for instance in prepared {
            self.add_instance(instance);
        }
    }

    pub fn batch_for_chassis(&self, chassis: RobotChassis) -> Option<&RobotBatch> {
        self.batches.get(&chassis)
    }

    pub fn batches(&self) -> impl Iterator<Item = (&RobotChassis, &RobotBatch)> {
        self.batches.iter()
    }

    /// Number of instanced draw calls required (batches with at least one instance).
    pub fn active_draw_calls(&self) -> usize {
        self.batches.values().filter(|b| !b.is_empty()).count()
    }

    pub fn total_instance_count(&self) -> usize {
        self.total_instances
    }

    /// Every visible instance flagged as somebody's personal escort.
    pub fn escort_instances(&self) -> Vec<&RobotInstance> {
        self.batches
            .values()
            .flat_map(|b| b.instances.iter())
            .filter(|i| i.escort_owner.is_some())
            .collect()
    }

    /// Estimated GPU-facing memory footprint in bytes.
    pub fn memory_bytes(&self) -> usize {
        let instance_bytes: usize = self
            .batches
            .values()
            .map(|b| b.instances.capacity() * std::mem::size_of::<RobotInstance>())
            .sum();
        instance_bytes + std::mem::size_of::<Self>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_types::{FactionId, RegionId, SimTick};
    use sim_core::event::EventJournal;
    use sim_core::robot::{PlayerPresence, RobotSpawnRequest};

    const FACTION: FactionId = FactionId::new(1);
    const REGION: RegionId = RegionId::new(1);

    fn registry_with_robots(count: u64) -> RobotRegistry {
        let mut registry = RobotRegistry::new();
        registry
            .register_player(PlayerPresence::new(
                PlayerId::new(1),
                EntityId::new(1000),
                FACTION,
                REGION,
                (0.0, 0.0, 0.0),
            ))
            .unwrap();
        for i in 0..count {
            let chassis = if i % 2 == 0 {
                RobotChassis::Guardsman
            } else {
                RobotChassis::Rifleman
            };
            registry
                .spawn_robot(RobotSpawnRequest::new(
                    EntityId::new(i + 1),
                    chassis,
                    FACTION,
                    REGION,
                    (i as f32 * 3.0, 0.0, 0.0),
                    SimTick::zero(),
                ))
                .unwrap();
        }
        registry
    }

    #[test]
    fn test_robot_batches_partition_by_chassis() {
        let registry = registry_with_robots(10);
        let mut renderer = BipedRobotRenderer::new();
        renderer.populate_from_registry(&registry, (0.0, 0.0, 0.0), 0.0);

        assert_eq!(renderer.total_instance_count(), 10);
        // One instanced draw call per chassis, regardless of robot count.
        assert_eq!(renderer.active_draw_calls(), 2);
        assert_eq!(
            renderer
                .batch_for_chassis(RobotChassis::Guardsman)
                .unwrap()
                .len(),
            5
        );
        assert_eq!(
            renderer
                .batch_for_chassis(RobotChassis::Rifleman)
                .unwrap()
                .len(),
            5
        );

        // Clearing keeps capacity so frames do not churn GPU buffers.
        renderer.clear();
        assert_eq!(renderer.total_instance_count(), 0);
        assert_eq!(renderer.active_draw_calls(), 0);
        assert!(
            renderer
                .batch_for_chassis(RobotChassis::Guardsman)
                .unwrap()
                .instances
                .capacity()
                >= 5
        );
    }

    #[test]
    fn test_lod_and_cull_selection_by_camera_distance() {
        let renderer = BipedRobotRenderer::new();
        assert_eq!(renderer.lod_for_distance(0.0), RobotLod::High);
        assert_eq!(renderer.lod_for_distance(59.0), RobotLod::High);
        assert_eq!(renderer.lod_for_distance(61.0), RobotLod::Medium);
        assert_eq!(renderer.lod_for_distance(300.0), RobotLod::Low);

        let registry = registry_with_robots(4);
        let mut renderer = BipedRobotRenderer::new();
        // Camera 1 km away culls everything.
        renderer.populate_from_registry(&registry, (1000.0, 0.0, 0.0), 0.0);
        assert_eq!(renderer.total_instance_count(), 0);
        assert_eq!(renderer.active_draw_calls(), 0);
    }

    #[test]
    fn test_gait_selection_and_distance_driven_phase() {
        let renderer = BipedRobotRenderer::new();
        assert_eq!(renderer.gait_for_speed(0.0), BipedGait::Idle);
        assert_eq!(renderer.gait_for_speed(2.0), BipedGait::Walk);
        assert_eq!(renderer.gait_for_speed(7.5), BipedGait::Run);

        // Phase is driven by distance travelled and wraps within [0, 1).
        assert!((renderer.gait_phase(0.0) - 0.0).abs() < 1e-6);
        assert!((renderer.gait_phase(renderer.stride_length * 0.5) - 0.5).abs() < 1e-6);
        let wrapped = renderer.gait_phase(renderer.stride_length * 3.25);
        assert!((0.0..1.0).contains(&wrapped));
        assert!((wrapped - 0.25).abs() < 1e-4);
    }

    #[test]
    fn test_escort_marker_is_presentation_of_authoritative_ownership() {
        let mut registry = registry_with_robots(3);
        let mut journal = EventJournal::new();
        let escort = EntityId::new(1);
        registry
            .assign_escort(
                PlayerId::new(1),
                PlayerId::new(1),
                escort,
                SimTick::new(1),
                &mut journal,
            )
            .unwrap();

        let mut renderer = BipedRobotRenderer::new();
        renderer.populate_from_registry(&registry, (0.0, 0.0, 0.0), 0.0);

        let escorts = renderer.escort_instances();
        assert_eq!(escorts.len(), 1);
        assert_eq!(escorts[0].entity, escort);
        assert_eq!(escorts[0].escort_owner, Some(PlayerId::new(1)));
        assert!((escorts[0].health_ratio - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_populate_from_registry_scales_to_1k_robots() {
        let registry = registry_with_robots(1_000);
        let mut renderer = BipedRobotRenderer::new();

        let start = std::time::Instant::now();
        renderer.populate_from_registry(&registry, (1500.0, 0.0, 0.0), 1.0);
        let elapsed = start.elapsed();

        // Only robots inside the cull distance are batched.
        assert!(renderer.total_instance_count() > 0);
        assert!(renderer.total_instance_count() < 1_000);
        assert!(renderer.active_draw_calls() <= RobotChassis::ALL.len());
        println!(
            "Batched {} of 1,000 robots in {:?}. Memory: {} KB",
            renderer.total_instance_count(),
            elapsed,
            renderer.memory_bytes() / 1024
        );
        assert!(elapsed.as_millis() < 50);
    }
}
