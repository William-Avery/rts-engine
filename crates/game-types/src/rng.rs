use std::fmt;

/// Seeded random number generator for deterministic simulation.
///
/// Uses a simple XORSHIFT128+ algorithm for reproducible results
/// across platforms and Rust versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SimRng {
    state0: u64,
    state1: u64,
}

impl SimRng {
    /// Create a new RNG with the given seed.
    pub fn new(seed: u64) -> Self {
        // Initialize state with non-zero values derived from seed
        let mut state0 = seed.wrapping_mul(0x9E3779B97F4A7C15u64);
        let mut state1 = state0.wrapping_add(0x3B6A97B193E2B3C6u64);
        if state0 == 0 {
            state0 = 1;
        }
        if state1 == 0 {
            state1 = 1;
        }

        SimRng { state0, state1 }
    }

    /// Create a new RNG with a default seed (0).
    pub fn with_default_seed() -> Self {
        Self::new(0)
    }

    /// Generate the next u64 random number.
    pub fn next_u64(&mut self) -> u64 {
        let mut s0 = self.state0;
        let mut s1 = self.state1;
        let result = s0.wrapping_add(s1);

        s0 ^= s0 << 23;
        s0 ^= s0 >> 17;
        s0 ^= s1;
        s0 ^= s1 >> 26;

        s1 = s1.rotate_right(41);
        s1 = s1.wrapping_add(result);

        self.state0 = s0;
        self.state1 = s1;

        result
    }

    /// Generate a random f64 in the range [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        // Convert u64 to f64 in [0, 1)
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Generate a random i32 in the range [0, max).
    pub fn next_i32_range(&mut self, max: i32) -> i32 {
        if max <= 0 {
            return 0;
        }
        (self.next_u64() as i32).rem_euclid(max)
    }

    /// Generate a random bool with 50% probability.
    pub fn next_bool(&mut self) -> bool {
        (self.next_u64() & 1) == 1
    }

    /// Generate a random unit in the range [0, 1) with f32 precision.
    pub fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 11) as f32 / (1u64 << 53) as f32
    }

    /// Return a new RNG with the same state (for deterministic replays).
    pub fn spawn(&self) -> Self {
        self.clone()
    }

    /// Get the current state0 (for testing/reproducibility checks).
    pub fn state0(&self) -> u64 {
        self.state0
    }

    /// Get the current state1 (for testing/reproducibility checks).
    pub fn state1(&self) -> u64 {
        self.state1
    }
}

impl Default for SimRng {
    fn default() -> Self {
        Self::with_default_seed()
    }
}

/// Simulation RNG seed type for reproducibility.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
#[repr(transparent)]
pub struct SimRngSeed(pub u64);

impl SimRngSeed {
    /// Create a new seed from a value.
    pub fn new(value: u64) -> Self {
        SimRngSeed(value)
    }

    /// Create a seed from a string (deterministic hashing).
    pub fn from_string(s: &str) -> Self {
        let mut hash = 0u64;
        for byte in s.bytes() {
            hash = hash.wrapping_mul(31).wrapping_add(byte as u64);
        }
        SimRngSeed(hash)
    }
}

impl fmt::Display for SimRngSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SimRngSeed({})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rng_reproducibility() {
        let mut rng1 = SimRng::new(42);
        let mut rng2 = SimRng::new(42);

        // Same seed should produce same sequence
        for _ in 0..100 {
            assert_eq!(rng1.next_u64(), rng2.next_u64());
        }
    }

    #[test]
    fn test_rng_non_zero() {
        let mut rng = SimRng::new(0);
        // Should still produce non-zero values after initialization
        let val = rng.next_u64();
        assert!(val != 0, "RNG should produce non-zero values");
    }

    #[test]
    fn test_rng_f64_range() {
        let mut rng = SimRng::new(123);
        for _ in 0..100 {
            let val = rng.next_f64();
            assert!((0.0..1.0).contains(&val), "f64 should be in [0, 1)");
        }
    }

    #[test]
    fn test_rng_i32_range() {
        let mut rng = SimRng::new(456);
        let max = 10;
        for _ in 0..100 {
            let val = rng.next_i32_range(max);
            assert!(val >= 0 && val < max, "i32 should be in [0, {})", max);
        }
    }

    #[test]
    fn test_rng_bool_distribution() {
        let mut rng = SimRng::new(789);
        let mut true_count = 0;

        for _ in 0..1000 {
            if rng.next_bool() {
                true_count += 1;
            }
        }

        // Should have roughly 50/50 distribution (allow some variance)
        assert!(
            true_count > 400 && true_count < 600,
            "Bool distribution should be roughly 50/50"
        );
    }

    #[test]
    fn test_rng_seed_from_str() {
        let seed1 = SimRngSeed::from_string("test");
        let seed2 = SimRngSeed::from_string("test");
        let seed3 = SimRngSeed::from_string("different");

        assert_eq!(seed1, seed2);
        assert_ne!(seed1, seed3);
    }
}
