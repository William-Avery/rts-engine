use crate::message_queue::CrossRegionRouter;
use crate::region::{RegionMap, RegionState};
use game_types::{GameError, GameResult, RegionId, SimTick};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

/// Multi-rate scheduling bucket defining update frequency.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum ScheduleBucket {
    /// High frequency (base simulation rate, e.g. 30-60 Hz: combat, physics, hot regions).
    High,
    /// Medium frequency (e.g. 15 Hz / half-rate: nearby AI, warm regions).
    Medium,
    /// Low frequency (e.g. 5 Hz / 1-in-6 ticks: squad logic, strategic updates).
    Low,
    /// Event-driven / scheduled wakeups (cold regions, factories, aggregate economy).
    Event,
}

impl ScheduleBucket {
    /// Returns default tick interval for this bucket.
    pub const fn default_interval(&self) -> u64 {
        match self {
            ScheduleBucket::High => 1,
            ScheduleBucket::Medium => 2,
            ScheduleBucket::Low => 6,
            ScheduleBucket::Event => u64::MAX,
        }
    }

    /// Whether this bucket should run on the given simulation tick.
    pub fn should_run_on_tick(&self, tick: SimTick, interval: u64) -> bool {
        if *self == ScheduleBucket::Event || interval == 0 || interval == u64::MAX {
            false
        } else {
            tick.value().is_multiple_of(interval)
        }
    }
}

/// Reason for a cold region wakeup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeupReason {
    /// Periodic timer or aggregate simulation step
    PeriodicTimer,
    /// Cross-region event or message arrival
    CrossRegionMessage,
    /// Production job or research completed
    JobReady,
    /// Explicit external or player-triggered wakeup
    Manual,
}

/// A scheduled wakeup event for a cold region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledWakeup {
    pub tick: SimTick,
    pub region_id: RegionId,
    pub reason: WakeupReason,
}

impl Ord for ScheduledWakeup {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for min-heap (smallest tick first)
        other
            .tick
            .cmp(&self.tick)
            .then_with(|| other.region_id.cmp(&self.region_id))
    }
}

impl PartialOrd for ScheduledWakeup {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Simulation metrics collected per tick or window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimulationMetrics {
    pub ticks_processed: u64,
    pub jobs_executed_hot: u64,
    pub jobs_executed_warm: u64,
    pub jobs_executed_cold: u64,
    pub entities_ticked_hot: u64,
    pub entities_ticked_warm: u64,
    pub entities_ticked_cold: u64,
    pub cross_region_messages_processed: u64,
    pub scheduled_wakeups_processed: u64,
    pub backpressure_drops: u64,
}

impl SimulationMetrics {
    pub fn total_jobs_executed(&self) -> u64 {
        self.jobs_executed_hot + self.jobs_executed_warm + self.jobs_executed_cold
    }

    pub fn total_entities_ticked(&self) -> u64 {
        self.entities_ticked_hot + self.entities_ticked_warm + self.entities_ticked_cold
    }

    pub fn reset_window(&mut self) {
        self.jobs_executed_hot = 0;
        self.jobs_executed_warm = 0;
        self.jobs_executed_cold = 0;
        self.entities_ticked_hot = 0;
        self.entities_ticked_warm = 0;
        self.entities_ticked_cold = 0;
        self.cross_region_messages_processed = 0;
        self.scheduled_wakeups_processed = 0;
        self.backpressure_drops = 0;
    }
}

/// Configuration of tick intervals for multi-rate buckets.
#[derive(Debug, Clone)]
pub struct MultiRateConfig {
    pub high_interval: u64,
    pub medium_interval: u64,
    pub low_interval: u64,
    pub max_wakeups_per_tick: usize,
}

impl Default for MultiRateConfig {
    fn default() -> Self {
        MultiRateConfig {
            high_interval: 1,   // 30 Hz
            medium_interval: 2, // 15 Hz
            low_interval: 6,    // 5 Hz
            max_wakeups_per_tick: 512,
        }
    }
}

/// Multi-rate deterministic scheduler managing regional execution, scheduled wakeups,
/// and performance metrics.
#[derive(Debug, Clone)]
pub struct MultiRateScheduler {
    config: MultiRateConfig,
    wakeup_queue: BinaryHeap<ScheduledWakeup>,
    metrics: SimulationMetrics,
}

impl Default for MultiRateScheduler {
    fn default() -> Self {
        Self::new(MultiRateConfig::default())
    }
}

impl MultiRateScheduler {
    pub fn new(config: MultiRateConfig) -> Self {
        MultiRateScheduler {
            config,
            wakeup_queue: BinaryHeap::new(),
            metrics: SimulationMetrics::default(),
        }
    }

    pub fn config(&self) -> &MultiRateConfig {
        &self.config
    }

    pub fn metrics(&self) -> &SimulationMetrics {
        &self.metrics
    }

    pub fn metrics_mut(&mut self) -> &mut SimulationMetrics {
        &mut self.metrics
    }

    /// Schedule a future wakeup for a cold region.
    pub fn schedule_wakeup(
        &mut self,
        tick: SimTick,
        region_id: RegionId,
        reason: WakeupReason,
    ) -> GameResult<()> {
        if region_id.is_null() {
            return Err(GameError::InvalidId);
        }
        self.wakeup_queue.push(ScheduledWakeup {
            tick,
            region_id,
            reason,
        });
        Ok(())
    }

    pub fn pending_wakeups_count(&self) -> usize {
        self.wakeup_queue.len()
    }

    /// Execute a single simulation tick over all regions using multi-rate bucket rules.
    /// Cold regions are NOT iterated unless they have a scheduled wakeup or pending messages.
    pub fn tick(
        &mut self,
        current_tick: SimTick,
        regions: &mut RegionMap,
        router: &mut CrossRegionRouter,
    ) {
        self.metrics.ticks_processed += 1;

        let run_high = self.config.high_interval > 0
            && current_tick
                .value()
                .is_multiple_of(self.config.high_interval);
        let run_medium = self.config.medium_interval > 0
            && current_tick
                .value()
                .is_multiple_of(self.config.medium_interval);
        let run_low = self.config.low_interval > 0
            && current_tick
                .value()
                .is_multiple_of(self.config.low_interval);

        // 1. Drain scheduled wakeups matching current tick
        let mut awakened_regions = std::collections::BTreeSet::new();
        let mut wakeups_processed = 0;

        while let Some(top) = self.wakeup_queue.peek() {
            if top.tick <= current_tick && wakeups_processed < self.config.max_wakeups_per_tick {
                let wakeup = self.wakeup_queue.pop().unwrap();
                awakened_regions.insert(wakeup.region_id);
                wakeups_processed += 1;
                self.metrics.scheduled_wakeups_processed += 1;
            } else {
                break;
            }
        }

        // 2. Regional execution without global "tick every entity every frame" loop.
        // Iterate only registered regions.
        let region_ids: Vec<(RegionId, RegionState, usize)> = regions
            .iter_regions()
            .map(|(id, r)| (*id, r.state, r.entity_count()))
            .collect();

        for (region_id, state, entity_count) in region_ids {
            match state {
                RegionState::Hot => {
                    if run_high {
                        // Hot region runs full tick
                        self.metrics.jobs_executed_hot += 1;
                        self.metrics.entities_ticked_hot += entity_count as u64;

                        // Drain pending cross-region messages for this region
                        let msgs = router.drain_messages_for_region(region_id);
                        self.metrics.cross_region_messages_processed += msgs.len() as u64;
                    }
                }
                RegionState::Warm => {
                    if run_medium {
                        // Warm region runs at reduced frequency
                        self.metrics.jobs_executed_warm += 1;
                        self.metrics.entities_ticked_warm += entity_count as u64;

                        let msgs = router.drain_messages_for_region(region_id);
                        self.metrics.cross_region_messages_processed += msgs.len() as u64;
                    }
                }
                RegionState::Cold => {
                    // Cold region runs ONLY if awakened by schedule or pending cross-region messages!
                    let has_pending = router.has_pending_messages(region_id);
                    let is_awakened = awakened_regions.contains(&region_id);

                    if is_awakened || has_pending {
                        self.metrics.jobs_executed_cold += 1;
                        self.metrics.entities_ticked_cold += entity_count as u64;

                        let msgs = router.drain_messages_for_region(region_id);
                        self.metrics.cross_region_messages_processed += msgs.len() as u64;
                    }
                    // Otherwise, COLD region costs 0 jobs and 0 entity ticks!
                }
            }
        }

        // Check if low bucket should trigger low-frequency tasks
        if run_low {
            // Low bucket runs periodically (e.g. strategic AI, cleanup)
        }
    }
}
