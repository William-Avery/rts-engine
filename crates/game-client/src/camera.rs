use std::f32::consts::PI;

/// Maximum and minimum pitch limits in radians (~85 degrees).
pub const MAX_PITCH: f32 = 85.0 * PI / 180.0;
pub const MIN_PITCH: f32 = -85.0 * PI / 180.0;

/// Operational perspective tier of the camera system.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub enum CameraMode {
    /// Orbital follow camera tracking the player avatar.
    #[default]
    ThirdPerson,
    /// Elevated tactical RTS view for base construction and squad commanding.
    Tactical,
    /// High-altitude strategic view for macro map awareness and regional logistics.
    Strategic,
}

impl CameraMode {
    /// Returns default distance, min distance, max distance, and default pitch in radians.
    pub fn default_parameters(&self) -> (f32, f32, f32, f32) {
        match self {
            CameraMode::ThirdPerson => (8.0, 1.5, 15.0, 25.0 * PI / 180.0),
            CameraMode::Tactical => (45.0, 20.0, 80.0, 60.0 * PI / 180.0),
            CameraMode::Strategic => (180.0, 100.0, 300.0, 82.0 * PI / 180.0),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            CameraMode::ThirdPerson => "Third-Person",
            CameraMode::Tactical => "Tactical",
            CameraMode::Strategic => "Strategic",
        }
    }
}

/// Active camera transition state interpolating smoothly between two camera modes.
#[derive(Debug, Clone, PartialEq)]
pub struct CameraTransition {
    pub from_mode: CameraMode,
    pub to_mode: CameraMode,
    pub start_target: (f32, f32, f32),
    pub end_target: (f32, f32, f32),
    pub start_distance: f32,
    pub end_distance: f32,
    pub start_pitch: f32,
    pub end_pitch: f32,
    pub start_yaw: f32,
    pub end_yaw: f32,
    pub elapsed: f32,
    pub duration: f32,
}

impl CameraTransition {
    /// Normalized interpolation factor [0.0, 1.0] using cubic Hermite smoothstep `3t^2 - 2t^3`.
    pub fn smooth_progress(&self) -> f32 {
        if self.duration <= 0.0001 {
            return 1.0;
        }
        let t = (self.elapsed / self.duration).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }

    pub fn is_complete(&self) -> bool {
        self.elapsed >= self.duration
    }
}

/// Multi-tier camera supporting third-person avatar follow, tactical RTS free-pan,
/// and strategic high-altitude overviews with smooth interpolation.
#[derive(Debug, Clone, PartialEq)]
pub struct ThirdPersonCamera {
    /// Current operational mode.
    pub mode: CameraMode,
    /// Tracked world-space target position (player feet/root or ground focal point).
    pub target: (f32, f32, f32),
    /// Local offset from target to camera look-at focus point (e.g. avatar eye/chest height).
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
    /// Active transition if currently interpolating between modes.
    pub transition: Option<CameraTransition>,
    /// Permitted world-space bounding box for camera focus `(min_x, max_x, min_z, max_z)`.
    pub world_bounds_xz: (f32, f32, f32, f32),
}

impl Default for ThirdPersonCamera {
    fn default() -> Self {
        Self::new((0.0, 0.0, 0.0), 8.0, 25.0 * PI / 180.0, 0.0)
    }
}

impl ThirdPersonCamera {
    pub fn new(target: (f32, f32, f32), distance: f32, pitch: f32, yaw: f32) -> Self {
        let mut cam = ThirdPersonCamera {
            mode: CameraMode::ThirdPerson,
            target,
            target_offset: (0.0, 1.5, 0.0), // 1.5m above ground for avatar center
            distance,
            min_distance: 1.5,
            max_distance: 15.0,
            pitch,
            yaw,
            transition: None,
            world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
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

    pub fn with_world_bounds(mut self, min_x: f32, max_x: f32, min_z: f32, max_z: f32) -> Self {
        self.world_bounds_xz = (min_x, max_x, min_z, max_z);
        self.clamp_to_world_bounds();
        self
    }

    /// Current operational mode.
    pub fn mode(&self) -> CameraMode {
        self.mode
    }

    /// Whether the camera is currently interpolating between modes.
    pub fn is_transitioning(&self) -> bool {
        self.transition.is_some()
    }

    /// Progress of current transition in range [0.0, 1.0], or 1.0 if not transitioning.
    pub fn transition_progress(&self) -> f32 {
        self.transition
            .as_ref()
            .map(|t| t.smooth_progress())
            .unwrap_or(1.0)
    }

    /// Orbit the camera around the target.
    pub fn orbit(&mut self, delta_yaw: f32, delta_pitch: f32) {
        self.yaw += delta_yaw;
        self.pitch += delta_pitch;
        self.clamp_parameters();
    }

    /// Zoom the camera in or out, respecting the current mode's distance constraints.
    pub fn zoom(&mut self, delta_distance: f32) {
        self.distance += delta_distance;
        self.distance = self.distance.clamp(self.min_distance, self.max_distance);
    }

    /// Update tracked target position. In ThirdPerson mode this is the avatar; in Tactical/Strategic
    /// this is the ground focus point.
    pub fn set_target(&mut self, target: (f32, f32, f32)) {
        self.target = target;
        self.clamp_to_world_bounds();
    }

    /// Pan the camera focus position along the ground plane (XZ).
    /// `delta_right` moves perpendicular to camera facing; `delta_forward` moves along planar forward.
    pub fn pan(&mut self, delta_right: f32, delta_forward: f32) {
        let fwd = self.planar_forward();
        let right = self.planar_right();

        self.target.0 += right.0 * delta_right + fwd.0 * delta_forward;
        self.target.2 += right.2 * delta_right + fwd.2 * delta_forward;
        self.clamp_to_world_bounds();
    }

    /// Clamps the focal target within the configured world boundary coordinates.
    pub fn clamp_to_world_bounds(&mut self) {
        let (min_x, max_x, min_z, max_z) = self.world_bounds_xz;
        self.target.0 = self.target.0.clamp(min_x, max_x);
        self.target.2 = self.target.2.clamp(min_z, max_z);
    }

    /// Initiate a smooth camera transition to `target_mode` over `duration_sec`.
    pub fn set_mode(&mut self, target_mode: CameraMode, duration_sec: f32) {
        if self.mode == target_mode && self.transition.is_none() {
            return;
        }

        let (target_dist, min_dist, max_dist, target_pitch) = target_mode.default_parameters();

        // Target position defaults to current target on ground plane
        let end_target = match target_mode {
            CameraMode::ThirdPerson => self.target,
            CameraMode::Tactical | CameraMode::Strategic => {
                // Ensure target is on ground plane (Y=0)
                (self.target.0, 0.0, self.target.2)
            }
        };

        if duration_sec <= 0.001 {
            self.mode = target_mode;
            self.min_distance = min_dist;
            self.max_distance = max_dist;
            self.distance = target_dist;
            self.pitch = target_pitch;
            self.target = end_target;
            self.transition = None;
            self.clamp_parameters();
            return;
        }

        self.transition = Some(CameraTransition {
            from_mode: self.mode,
            to_mode: target_mode,
            start_target: self.target,
            end_target,
            start_distance: self.distance,
            end_distance: target_dist,
            start_pitch: self.pitch,
            end_pitch: target_pitch,
            start_yaw: self.yaw,
            end_yaw: self.yaw,
            elapsed: 0.0,
            duration: duration_sec,
        });

        // Update distance limits immediately for bounds safety
        self.min_distance = min_dist.min(self.min_distance);
        self.max_distance = max_dist.max(self.max_distance);
    }

    /// Advance active camera mode transition by delta time `dt`.
    pub fn update_transition(&mut self, dt: f32) {
        if let Some(ref mut tr) = self.transition {
            tr.elapsed += dt;
            let s = tr.smooth_progress();

            // Smoothly interpolate target position
            self.target = (
                tr.start_target.0 + (tr.end_target.0 - tr.start_target.0) * s,
                tr.start_target.1 + (tr.end_target.1 - tr.start_target.1) * s,
                tr.start_target.2 + (tr.end_target.2 - tr.start_target.2) * s,
            );

            // Smoothly interpolate distance and pitch
            self.distance = tr.start_distance + (tr.end_distance - tr.start_distance) * s;
            self.pitch = tr.start_pitch + (tr.end_pitch - tr.start_pitch) * s;
            self.yaw = tr.start_yaw + (tr.end_yaw - tr.start_yaw) * s;

            if tr.is_complete() {
                let to_mode = tr.to_mode;
                let (_, min_dist, max_dist, _) = to_mode.default_parameters();
                self.mode = to_mode;
                self.min_distance = min_dist;
                self.max_distance = max_dist;
                self.transition = None;
                self.clamp_parameters();
            }
        }
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
        match self.mode {
            CameraMode::ThirdPerson => (
                self.target.0 + self.target_offset.0,
                self.target.1 + self.target_offset.1,
                self.target.2 + self.target_offset.2,
            ),
            CameraMode::Tactical | CameraMode::Strategic => self.target,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_mode_switching_and_smooth_interpolation() {
        let mut cam = ThirdPersonCamera::default();
        assert_eq!(cam.mode(), CameraMode::ThirdPerson);
        assert_eq!(cam.distance, 8.0);

        // Initiate transition to Tactical mode over 0.5s
        cam.set_mode(CameraMode::Tactical, 0.5);
        assert!(cam.is_transitioning());
        assert_eq!(cam.transition_progress(), 0.0);

        // Halfway through transition
        cam.update_transition(0.25);
        assert!(cam.is_transitioning());
        let mid_progress = cam.transition_progress();
        assert!(
            mid_progress > 0.4 && mid_progress < 0.6,
            "Smoothstep progress at halfway must be near 0.5"
        );
        assert!(
            cam.distance > 8.0 && cam.distance < 45.0,
            "Distance must be smoothly interpolating"
        );

        // Complete transition
        cam.update_transition(0.25);
        assert!(!cam.is_transitioning());
        assert_eq!(cam.mode(), CameraMode::Tactical);
        assert!((cam.distance - 45.0).abs() < 1e-3);
    }

    #[test]
    fn test_tactical_panning_and_boundary_clamping() {
        let mut cam = ThirdPersonCamera::new((0.0, 0.0, 0.0), 45.0, 60.0 * PI / 180.0, 0.0)
            .with_world_bounds(-100.0, 100.0, -100.0, 100.0);
        cam.mode = CameraMode::Tactical;

        // Pan right by 20m, forward by 30m
        cam.pan(20.0, 30.0);
        assert!((cam.target.0 - 20.0).abs() < 1e-3);
        assert!((cam.target.2 - 30.0).abs() < 1e-3);

        // Pan beyond world bounds - must be clamped
        cam.pan(200.0, 200.0);
        assert_eq!(cam.target.0, 100.0);
        assert_eq!(cam.target.2, 100.0);
    }
}
