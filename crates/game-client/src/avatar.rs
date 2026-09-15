use std::f32::consts::PI;

/// Locomotion state for character presentation and animation blending.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LocomotionState {
    #[default]
    Idle,
    Walking,
    Running,
    Jumping,
    Falling,
}

/// Avatar placeholder representing the player's physical and visual presence.
#[derive(Debug, Clone, PartialEq)]
pub struct Avatar {
    /// Collision cylinder radius in meters.
    pub radius: f32,
    /// Collision cylinder height in meters.
    pub height: f32,
    /// Current facing yaw angle in radians.
    pub facing_yaw: f32,
    /// Planar movement speed in m/s.
    pub speed: f32,
    /// Grounded state.
    pub is_grounded: bool,
    /// Active locomotion animation state.
    pub locomotion_state: LocomotionState,
    /// Cyclic animation phase [0, 1) for walk/run strides.
    pub animation_phase: f32,
}

impl Default for Avatar {
    fn default() -> Self {
        Avatar {
            radius: 0.4,
            height: 1.8,
            facing_yaw: 0.0,
            speed: 0.0,
            is_grounded: true,
            locomotion_state: LocomotionState::Idle,
            animation_phase: 0.0,
        }
    }
}

impl Avatar {
    pub fn new() -> Self {
        Avatar::default()
    }

    /// Updates avatar visual state and animation cycle based on current velocity and grounded status.
    pub fn update(&mut self, velocity: (f32, f32, f32), grounded: bool, delta_seconds: f32) {
        let horizontal_speed = (velocity.0 * velocity.0 + velocity.2 * velocity.2).sqrt();
        self.speed = horizontal_speed;
        self.is_grounded = grounded;

        // Update facing yaw if moving horizontally
        if horizontal_speed > 0.1 {
            // In our coordinate system, planar_forward is (sin(yaw), 0, cos(yaw))
            self.facing_yaw = velocity.0.atan2(velocity.2).rem_euclid(2.0 * PI);
        }

        // Determine locomotion state
        self.locomotion_state = if !grounded {
            if velocity.1 > 0.1 {
                LocomotionState::Jumping
            } else {
                LocomotionState::Falling
            }
        } else if horizontal_speed < 0.2 {
            LocomotionState::Idle
        } else if horizontal_speed < 7.0 {
            LocomotionState::Walking
        } else {
            LocomotionState::Running
        };

        // Advance animation phase proportional to movement speed
        if self.locomotion_state == LocomotionState::Walking
            || self.locomotion_state == LocomotionState::Running
        {
            let stride_frequency = if self.locomotion_state == LocomotionState::Running {
                3.2
            } else {
                2.0
            };
            self.animation_phase = (self.animation_phase + delta_seconds * stride_frequency) % 1.0;
        } else {
            self.animation_phase = 0.0;
        }
    }
}
