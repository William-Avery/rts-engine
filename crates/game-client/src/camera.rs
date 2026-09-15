use std::f32::consts::PI;

/// Maximum and minimum pitch limits in radians (~85 degrees).
pub const MAX_PITCH: f32 = 85.0 * PI / 180.0;
pub const MIN_PITCH: f32 = -85.0 * PI / 180.0;

/// Third-person orbital camera with spherical coordinates, pitch clamping, and directional projections.
#[derive(Debug, Clone, PartialEq)]
pub struct ThirdPersonCamera {
    /// Tracked world-space target position (typically player feet/root).
    pub target: (f32, f32, f32),
    /// Local offset from target to camera look-at focus point (e.g. eye/chest height).
    pub target_offset: (f32, f32, f32),
    /// Distance from focus point to camera eye.
    pub distance: f32,
    /// Minimum allowed distance when zooming in.
    pub min_distance: f32,
    /// Maximum allowed distance when zooming out.
    pub max_distance: f32,
    /// Vertical elevation angle in radians (-85 deg to +85 deg).
    /// Positive pitch elevates the camera above the target.
    pub pitch: f32,
    /// Horizontal azimuth angle in radians (0 to 2*PI).
    pub yaw: f32,
}

impl Default for ThirdPersonCamera {
    fn default() -> Self {
        Self::new((0.0, 0.0, 0.0), 8.0, 25.0 * PI / 180.0, 0.0)
    }
}

impl ThirdPersonCamera {
    pub fn new(target: (f32, f32, f32), distance: f32, pitch: f32, yaw: f32) -> Self {
        let mut cam = ThirdPersonCamera {
            target,
            target_offset: (0.0, 1.5, 0.0), // 1.5m above ground for avatar center
            distance,
            min_distance: 1.5,
            max_distance: 50.0,
            pitch,
            yaw,
        };
        cam.clamp_parameters();
        cam
    }

    pub fn with_target_offset(mut self, offset: (f32, f32, f32)) -> Self {
        self.target_offset = offset;
        self
    }

    pub fn with_distance_limits(mut self, min: f32, max: f32) -> Self {
        self.min_distance = min.max(0.1);
        self.max_distance = max.max(self.min_distance);
        self.distance = self.distance.clamp(self.min_distance, self.max_distance);
        self
    }

    /// Orbit the camera around the target.
    pub fn orbit(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.yaw += delta_yaw;
        self.pitch += delta_pitch;
        self.clamp_parameters();
    }

    /// Zoom the camera in or out.
    pub fn zoom(&mut self, delta_distance: f32) {
        self.distance += delta_distance;
        self.distance = self.distance.clamp(self.min_distance, self.max_distance);
    }

    /// Update tracked target position.
    pub fn set_target(&mut self, target: (f32, f32, f32)) {
        self.target = target;
    }

    /// Clamps pitch to [-85 deg, +85 deg] and wraps yaw to [0, 2*PI).
    fn clamp_parameters(&mut self) {
        self.pitch = self.pitch.clamp(MIN_PITCH, MAX_PITCH);
        let tau = 2.0 * PI;
        self.yaw = self.yaw.rem_euclid(tau);
        self.distance = self.distance.clamp(self.min_distance, self.max_distance);
    }

    /// Focus point in world space that the camera looks directly at.
    pub fn focus_position(&self) -> (f32, f32, f32) {
        (
            self.target.0 + self.target_offset.0,
            self.target.1 + self.target_offset.1,
            self.target.2 + self.target_offset.2,
        )
    }

    /// Forward unit vector pointing from camera eye toward focus point.
    pub fn forward_vector(&self) -> (f32, f32, f32) {
        let cp = self.pitch.cos();
        let sp = self.pitch.sin();
        let cy = self.yaw.cos();
        let sy = self.yaw.sin();

        // Positive pitch elevates the camera above the target, so forward vector points downwards (-sp)
        (sy * cp, -sp, cy * cp)
    }

    /// Eye position of the camera in world coordinates.
    pub fn eye_position(&self) -> (f32, f32, f32) {
        let focus = self.focus_position();
        let fwd = self.forward_vector();
        (
            focus.0 - fwd.0 * self.distance,
            focus.1 - fwd.1 * self.distance,
            focus.2 - fwd.2 * self.distance,
        )
    }

    /// Horizontal unit forward vector on the XZ ground plane (Y=0).
    pub fn planar_forward(&self) -> (f32, f32, f32) {
        (self.yaw.sin(), 0.0, self.yaw.cos())
    }

    /// Horizontal unit right vector perpendicular to planar_forward on XZ plane.
    pub fn planar_right(&self) -> (f32, f32, f32) {
        (self.yaw.cos(), 0.0, -self.yaw.sin())
    }

    pub fn pitch_deg(&self) -> f32 {
        self.pitch * 180.0 / PI
    }

    pub fn yaw_deg(&self) -> f32 {
        self.yaw * 180.0 / PI
    }
}
