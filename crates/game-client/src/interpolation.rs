use game_types::SimTick;
use std::collections::VecDeque;
use std::f32::consts::PI;

/// A timestamped snapshot sample of a remote entity.
#[derive(Debug, Clone, PartialEq)]
pub struct EntitySample {
    pub tick: SimTick,
    pub timestamp_ms: u64,
    pub position: (f32, f32, f32),
    pub velocity: (f32, f32, f32),
    pub yaw: f32,
}

impl EntitySample {
    pub fn new(
        tick: SimTick,
        timestamp_ms: u64,
        position: (f32, f32, f32),
        velocity: (f32, f32, f32),
        yaw: f32,
    ) -> Self {
        EntitySample {
            tick,
            timestamp_ms,
            position,
            velocity,
            yaw,
        }
    }
}

/// Buffer storing remote entity snapshot history to calculate smooth interpolated render transforms.
#[derive(Debug, Clone)]
pub struct InterpolationBuffer {
    pub samples: VecDeque<EntitySample>,
    pub capacity: usize,
    /// Delay behind server time (in milliseconds) used to ensure two bounding snapshots exist.
    pub interpolation_delay_ms: u64,
    /// Maximum duration (in milliseconds) to extrapolate using velocity when a packet is dropped.
    pub max_extrapolation_ms: u64,
}

impl Default for InterpolationBuffer {
    fn default() -> Self {
        InterpolationBuffer {
            samples: VecDeque::with_capacity(32),
            capacity: 32,
            interpolation_delay_ms: 66, // ~2 ticks at 30 Hz
            max_extrapolation_ms: 150,
        }
    }
}

impl InterpolationBuffer {
    pub fn new(interpolation_delay_ms: u64) -> Self {
        InterpolationBuffer {
            samples: VecDeque::with_capacity(32),
            capacity: 32,
            interpolation_delay_ms,
            max_extrapolation_ms: 150,
        }
    }

    pub fn push_sample(&mut self, sample: EntitySample) {
        if let Some(back) = self.samples.back() {
            // Discard duplicate or strictly older timestamps
            if sample.timestamp_ms <= back.timestamp_ms {
                return;
            }
        }

        if self.samples.len() >= self.capacity {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn latest_sample(&self) -> Option<&EntitySample> {
        self.samples.back()
    }

    /// Computes smoothly interpolated position at `current_time_ms` taking delay into account.
    pub fn interpolate_position(&self, current_time_ms: u64) -> Option<(f32, f32, f32)> {
        if self.samples.is_empty() {
            return None;
        }

        let render_time = current_time_ms.saturating_sub(self.interpolation_delay_ms);
        let oldest = self.samples.front().unwrap();
        let newest = self.samples.back().unwrap();

        // 1. Before oldest sample: clamp to oldest
        if render_time <= oldest.timestamp_ms {
            return Some(oldest.position);
        }

        // 2. Beyond newest sample: extrapolate with velocity up to max_extrapolation_ms
        if render_time >= newest.timestamp_ms {
            let overdue = render_time - newest.timestamp_ms;
            return if overdue <= self.max_extrapolation_ms {
                let dt = (overdue as f32) / 1000.0;
                Some((
                    newest.position.0 + newest.velocity.0 * dt,
                    newest.position.1 + newest.velocity.1 * dt,
                    newest.position.2 + newest.velocity.2 * dt,
                ))
            } else {
                Some(newest.position)
            };
        }

        // 3. Find bounding samples [s0, s1] such that s0.timestamp <= render_time <= s1.timestamp
        for i in 0..self.samples.len() - 1 {
            let s0 = &self.samples[i];
            let s1 = &self.samples[i + 1];

            if render_time >= s0.timestamp_ms && render_time <= s1.timestamp_ms {
                let span = s1.timestamp_ms - s0.timestamp_ms;
                let t = if span > 0 {
                    (render_time - s0.timestamp_ms) as f32 / span as f32
                } else {
                    0.0
                };

                return Some(lerp_v3(s0.position, s1.position, t));
            }
        }

        Some(newest.position)
    }

    /// Computes smoothly interpolated yaw at `current_time_ms` with proper shortest-arc angle wrapping.
    pub fn interpolate_yaw(&self, current_time_ms: u64) -> Option<f32> {
        if self.samples.is_empty() {
            return None;
        }

        let render_time = current_time_ms.saturating_sub(self.interpolation_delay_ms);
        let oldest = self.samples.front().unwrap();
        let newest = self.samples.back().unwrap();

        if render_time <= oldest.timestamp_ms {
            return Some(oldest.yaw);
        }
        if render_time >= newest.timestamp_ms {
            return Some(newest.yaw);
        }

        for i in 0..self.samples.len() - 1 {
            let s0 = &self.samples[i];
            let s1 = &self.samples[i + 1];

            if render_time >= s0.timestamp_ms && render_time <= s1.timestamp_ms {
                let span = s1.timestamp_ms - s0.timestamp_ms;
                let t = if span > 0 {
                    (render_time - s0.timestamp_ms) as f32 / span as f32
                } else {
                    0.0
                };

                return Some(lerp_angle(s0.yaw, s1.yaw, t));
            }
        }

        Some(newest.yaw)
    }
}

fn lerp_v3(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
}

fn lerp_angle(from: f32, to: f32, t: f32) -> f32 {
    let tau = 2.0 * PI;
    let mut diff = (to - from).rem_euclid(tau);
    if diff > PI {
        diff -= tau;
    }
    (from + diff * t).rem_euclid(tau)
}
