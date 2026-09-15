/// Simulation tick type for deterministic simulation.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd, Default)]
#[repr(transparent)]
pub struct SimTick(pub u64);

impl SimTick {
    pub const fn new(value: u64) -> Self {
        SimTick(value)
    }

    pub const fn zero() -> Self {
        SimTick(0)
    }

    pub const fn next(&self) -> Self {
        SimTick(self.0 + 1)
    }

    pub const fn previous(&self) -> Self {
        SimTick(self.0.wrapping_sub(1))
    }

    pub const fn value(&self) -> u64 {
        self.0
    }

    pub fn checked_add(&self, rhs: u64) -> Option<Self> {
        self.0.checked_add(rhs).map(SimTick)
    }

    pub fn checked_sub(&self, rhs: u64) -> Option<Self> {
        self.0.checked_sub(rhs).map(SimTick)
    }
}

impl std::ops::Add<u64> for SimTick {
    type Output = Self;

    fn add(self, rhs: u64) -> Self {
        SimTick(self.0 + rhs)
    }
}

impl std::ops::Sub<u64> for SimTick {
    type Output = Self;

    fn sub(self, rhs: u64) -> Self {
        SimTick(self.0.checked_sub(rhs).expect("SimTick underflow"))
    }
}

impl std::ops::Sub for SimTick {
    type Output = u64;

    fn sub(self, rhs: Self) -> u64 {
        self.0 - rhs.0
    }
}

/// Monotonic simulation clock with tick and fractional time.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct SimTime {
    tick: SimTick,
    tick_fraction: f64, // 0.0 <= tick_fraction < 1.0
}

impl SimTime {
    pub const fn new(tick: SimTick, tick_fraction: f64) -> Self {
        assert!(tick_fraction >= 0.0 && tick_fraction < 1.0);
        SimTime {
            tick,
            tick_fraction,
        }
    }

    pub const fn zero() -> Self {
        SimTime::new(SimTick::zero(), 0.0)
    }

    pub const fn tick(&self) -> SimTick {
        self.tick
    }

    pub const fn tick_fraction(&self) -> f64 {
        self.tick_fraction
    }

    pub fn advance_to_tick(&self, target: SimTick) -> Self {
        SimTime::new(target, 0.0)
    }

    #[allow(clippy::manual_range_contains)]
    pub fn advance_by_fraction(&self, fraction: f64) -> Self {
        assert!(fraction >= 0.0 && fraction < 1.0);
        SimTime::new(self.tick, fraction)
    }

    pub fn next_tick(&self) -> Self {
        SimTime::new(self.tick.next(), 0.0)
    }
}

impl std::cmp::PartialOrd for SimTime {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        match self.tick.cmp(&other.tick) {
            std::cmp::Ordering::Equal => self.tick_fraction.partial_cmp(&other.tick_fraction),
            other => Some(other),
        }
    }
}

/// Duration in simulation ticks.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Ord, PartialOrd)]
pub struct SimDuration(pub u64);

impl SimDuration {
    pub const fn ticks(value: u64) -> Self {
        SimDuration(value)
    }

    pub const fn zero() -> Self {
        SimDuration(0)
    }
}
