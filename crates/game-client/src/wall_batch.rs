use sim_core::structure::CompactWallGrid;
use sim_core::wall::WallTier;
use std::collections::BTreeMap;

/// Single rendered instance data for a wall segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WallInstance {
    pub position: (f32, f32, f32),
    pub rotation_deg: f32,
    pub tier: WallTier,
    /// Durability health ratio in range [0.0, 1.0] for wear/damage shaders.
    pub health_ratio: f32,
}

impl WallInstance {
    pub fn new(
        position: (f32, f32, f32),
        rotation_deg: f32,
        tier: WallTier,
        health_ratio: f32,
    ) -> Self {
        WallInstance {
            position,
            rotation_deg,
            tier,
            health_ratio: health_ratio.clamp(0.0, 1.0),
        }
    }
}

/// A draw buffer batch representing all wall instances sharing a common material tier.
#[derive(Clone, Debug, PartialEq)]
pub struct WallBatch {
    pub tier: WallTier,
    pub instances: Vec<WallInstance>,
}

impl WallBatch {
    pub fn new(tier: WallTier) -> Self {
        WallBatch {
            tier,
            instances: Vec::new(),
        }
    }

    pub fn with_capacity(tier: WallTier, capacity: usize) -> Self {
        WallBatch {
            tier,
            instances: Vec::with_capacity(capacity),
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

/// Client-side batched and instanced wall renderer.
///
/// Partitions thousands of walls into at most 3 compact instanced draw calls (one per WallTier),
/// preserving capacity across frames to eliminate GPU buffer allocation churn.
#[derive(Clone, Debug)]
pub struct BatchedWallRenderer {
    batches: BTreeMap<WallTier, WallBatch>,
    total_instances: usize,
}

impl Default for BatchedWallRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchedWallRenderer {
    pub fn new() -> Self {
        let mut batches = BTreeMap::new();
        batches.insert(WallTier::Mk1Stone, WallBatch::new(WallTier::Mk1Stone));
        batches.insert(WallTier::Mk2Steel, WallBatch::new(WallTier::Mk2Steel));
        batches.insert(
            WallTier::Mk3Composite,
            WallBatch::new(WallTier::Mk3Composite),
        );

        BatchedWallRenderer {
            batches,
            total_instances: 0,
        }
    }

    /// Reset all batch instance lists without deallocating internal vectors.
    pub fn clear(&mut self) {
        for batch in self.batches.values_mut() {
            batch.clear();
        }
        self.total_instances = 0;
    }

    /// Add an instance to its corresponding tier batch.
    pub fn add_instance(&mut self, instance: WallInstance) {
        self.batches
            .entry(instance.tier)
            .or_insert_with(|| WallBatch::new(instance.tier))
            .instances
            .push(instance);
        self.total_instances += 1;
    }

    /// Batch populated directly from the simulation compact wall grid.
    pub fn populate_from_compact_grid(&mut self, grid: &CompactWallGrid, ground_y: f32) {
        self.clear();

        // Note: CompactWallGrid stores cells at cell_size increments
        let half_h = 1.25; // 2.5m tall wall sits on ground
        let wall_y = ground_y + half_h;

        for (&(gx, gz), &(tier_u8, _faction)) in grid.iter() {
            let tier = WallTier::from_u8(tier_u8).unwrap_or(WallTier::Mk1Stone);
            let world_x = (gx as f32) * grid.cell_size;
            let world_z = (gz as f32) * grid.cell_size;

            self.add_instance(WallInstance::new(
                (world_x, wall_y, world_z),
                0.0,
                tier,
                1.0,
            ));
        }
    }

    pub fn batch_for_tier(&self, tier: WallTier) -> Option<&WallBatch> {
        self.batches.get(&tier)
    }

    pub fn batches(&self) -> impl Iterator<Item = (&WallTier, &WallBatch)> {
        self.batches.iter()
    }

    /// Number of active draw calls required (batches containing >= 1 instance).
    pub fn active_draw_calls(&self) -> usize {
        self.batches
            .values()
            .filter(|b| !b.instances.is_empty())
            .count()
    }

    pub fn total_instance_count(&self) -> usize {
        self.total_instances
    }

    /// Estimated memory footprint in bytes.
    pub fn memory_bytes(&self) -> usize {
        let instance_bytes: usize = self
            .batches
            .values()
            .map(|b| b.instances.capacity() * std::mem::size_of::<WallInstance>())
            .sum();
        instance_bytes + std::mem::size_of::<Self>()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use game_types::FactionId;

    #[test]
    fn test_batched_wall_renderer_partitioning() {
        let mut renderer = BatchedWallRenderer::new();
        assert_eq!(renderer.active_draw_calls(), 0);
        assert_eq!(renderer.total_instance_count(), 0);

        // Add instances of different tiers
        renderer.add_instance(WallInstance::new(
            (0.0, 0.0, 0.0),
            0.0,
            WallTier::Mk1Stone,
            1.0,
        ));
        renderer.add_instance(WallInstance::new(
            (2.0, 0.0, 0.0),
            0.0,
            WallTier::Mk1Stone,
            0.8,
        ));
        renderer.add_instance(WallInstance::new(
            (4.0, 0.0, 0.0),
            0.0,
            WallTier::Mk2Steel,
            1.0,
        ));
        renderer.add_instance(WallInstance::new(
            (6.0, 0.0, 0.0),
            0.0,
            WallTier::Mk3Composite,
            0.5,
        ));

        assert_eq!(renderer.total_instance_count(), 4);
        assert_eq!(renderer.active_draw_calls(), 3);

        {
            let mk1_batch = renderer.batch_for_tier(WallTier::Mk1Stone).unwrap();
            assert_eq!(mk1_batch.len(), 2);
            assert_eq!(mk1_batch.instances[1].health_ratio, 0.8);

            let mk2_batch = renderer.batch_for_tier(WallTier::Mk2Steel).unwrap();
            assert_eq!(mk2_batch.len(), 1);

            let mk3_batch = renderer.batch_for_tier(WallTier::Mk3Composite).unwrap();
            assert_eq!(mk3_batch.len(), 1);
            assert_eq!(mk3_batch.instances[0].health_ratio, 0.5);
        }

        // Clear resets instances without deallocating vector capacity
        renderer.clear();
        assert_eq!(renderer.total_instance_count(), 0);
        assert_eq!(renderer.active_draw_calls(), 0);
        assert!(
            renderer
                .batch_for_tier(WallTier::Mk1Stone)
                .unwrap()
                .instances
                .capacity()
                >= 2
        );
    }

    #[test]
    fn test_populate_from_compact_grid_10k_scale() {
        let mut grid = CompactWallGrid::new(2.0);

        // Populate 10,000 walls with mixed tiers (100x100 grid)
        for x in 0..100 {
            for z in 0..100 {
                let tier = match (x + z) % 3 {
                    0 => 1,
                    1 => 2,
                    _ => 3,
                };
                grid.insert_wall(x, z, tier, FactionId::new(1));
            }
        }
        assert_eq!(grid.count(), 10_000);

        let mut renderer = BatchedWallRenderer::new();
        let start = std::time::Instant::now();
        renderer.populate_from_compact_grid(&grid, 0.0);
        let elapsed = start.elapsed();

        assert_eq!(renderer.total_instance_count(), 10_000);
        // All 10,000 walls partition into exactly 3 instanced draw batches!
        assert_eq!(renderer.active_draw_calls(), 3);

        println!(
            "Populated and partitioned 10,000 walls in {:?}. Memory: {} KB",
            elapsed,
            renderer.memory_bytes() / 1024
        );
        assert!(elapsed.as_millis() < 50);
    }
}
