//! Authoritative collision world and movement validation.
//!
//! This module is the **single source of truth** for "where may a body be".
//! It lives in `sim-core` because the server, not the client, owns player
//! position: `dispatch::apply_command` runs [`validate_authoritative_movement`]
//! for every `Command::Move`, and `game-client` re-exports the same types so
//! client-side prediction runs byte-identical arithmetic.

/// Axis-aligned 3D bounding box for static world obstacles.
#[derive(Debug, Clone, PartialEq)]
pub struct ObstacleAabb {
    pub id: u32,
    pub label: String,
    pub min: (f32, f32, f32),
    pub max: (f32, f32, f32),
}

impl ObstacleAabb {
    pub fn new(
        id: u32,
        label: impl Into<String>,
        min: (f32, f32, f32),
        max: (f32, f32, f32),
    ) -> Self {
        ObstacleAabb {
            id,
            label: label.into(),
            min: (min.0.min(max.0), min.1.min(max.1), min.2.min(max.2)),
            max: (min.0.max(max.0), min.1.max(max.1), min.2.max(max.2)),
        }
    }

    /// Checks if a cylinder centered at `pos` with `radius` and `height` intersects this box.
    pub fn intersects_cylinder(&self, pos: (f32, f32, f32), radius: f32, height: f32) -> bool {
        // Vertical check
        let cyl_bottom = pos.1;
        let cyl_top = pos.1 + height;
        if cyl_top < self.min.1 || cyl_bottom > self.max.1 {
            return false;
        }

        // Horizontal closest point on AABB
        let closest_x = pos.0.clamp(self.min.0, self.max.0);
        let closest_z = pos.2.clamp(self.min.2, self.max.2);

        let dx = pos.0 - closest_x;
        let dz = pos.2 - closest_z;
        (dx * dx + dz * dz) < (radius * radius)
    }

    /// Resolves horizontal collision by pushing the cylinder outside the box along the shortest penetration normal.
    pub fn resolve_cylinder(
        &self,
        pos: (f32, f32, f32),
        radius: f32,
        height: f32,
    ) -> (f32, f32, f32) {
        if !self.intersects_cylinder(pos, radius, height) {
            return pos;
        }

        // Determine penetration depths along X and Z
        let overlap_left = (pos.0 + radius) - self.min.0;
        let overlap_right = self.max.0 - (pos.0 - radius);
        let overlap_front = (pos.2 + radius) - self.min.2;
        let overlap_back = self.max.2 - (pos.2 - radius);

        let min_overlap_x = if overlap_left < overlap_right {
            -overlap_left
        } else {
            overlap_right
        };

        let min_overlap_z = if overlap_front < overlap_back {
            -overlap_front
        } else {
            overlap_back
        };

        // Push along the axis of minimum penetration
        if min_overlap_x.abs() < min_overlap_z.abs() {
            (pos.0 + min_overlap_x, pos.1, pos.2)
        } else {
            (pos.0, pos.1, pos.2 + min_overlap_z)
        }
    }
}

/// Simple greybox terrain providing bounded ground plane and static obstacle collision resolution.
#[derive(Debug, Clone, PartialEq)]
pub struct GreyboxTerrain {
    pub min_x: f32,
    pub max_x: f32,
    pub min_z: f32,
    pub max_z: f32,
    pub ground_y: f32,
    pub obstacles: Vec<ObstacleAabb>,
}

impl Default for GreyboxTerrain {
    /// The authoritative default play area, matching the `world_bounds_xz` the
    /// structure placement path validates against.
    fn default() -> Self {
        GreyboxTerrain::new((-500.0, 500.0), (-500.0, 500.0))
    }
}

impl GreyboxTerrain {
    pub fn new(bounds_x: (f32, f32), bounds_z: (f32, f32)) -> Self {
        GreyboxTerrain {
            min_x: bounds_x.0.min(bounds_x.1),
            max_x: bounds_x.0.max(bounds_x.1),
            min_z: bounds_z.0.min(bounds_z.1),
            max_z: bounds_z.0.max(bounds_z.1),
            ground_y: 0.0,
            obstacles: Vec::new(),
        }
    }

    /// Creates a default greybox testing arena (-100m to +100m) with perimeter and test obstacles.
    pub fn default_arena() -> Self {
        let mut terrain = GreyboxTerrain::new((-100.0, 100.0), (-100.0, 100.0));

        // Central bunker/monolith
        terrain.add_obstacle(ObstacleAabb::new(
            1,
            "Central Command Bunker",
            (-6.0, 0.0, -6.0),
            (6.0, 4.0, 6.0),
        ));

        // West defensive barricade
        terrain.add_obstacle(ObstacleAabb::new(
            2,
            "West Barricade",
            (-30.0, 0.0, -10.0),
            (-20.0, 2.5, -6.0),
        ));

        // East radar pillar
        terrain.add_obstacle(ObstacleAabb::new(
            3,
            "East Radar Tower",
            (25.0, 0.0, 15.0),
            (29.0, 12.0, 19.0),
        ));

        terrain
    }

    pub fn add_obstacle(&mut self, obstacle: ObstacleAabb) {
        self.obstacles.push(obstacle);
    }

    /// Authoritative `(min_x, max_x, min_z, max_z)` play area.
    pub fn bounds_xz(&self) -> (f32, f32, f32, f32) {
        (self.min_x, self.max_x, self.min_z, self.max_z)
    }

    pub fn is_point_inside_bounds(&self, x: f32, z: f32) -> bool {
        x >= self.min_x && x <= self.max_x && z >= self.min_z && z <= self.max_z
    }

    /// Clamps position to stay inside world terrain boundaries accounting for character radius.
    pub fn clamp_to_bounds(&self, pos: (f32, f32, f32), radius: f32) -> (f32, f32, f32) {
        let clamped_x = pos.0.clamp(self.min_x + radius, self.max_x - radius);
        let clamped_y = pos.1.max(self.ground_y);
        let clamped_z = pos.2.clamp(self.min_z + radius, self.max_z - radius);
        (clamped_x, clamped_y, clamped_z)
    }

    /// Checks if a character cylinder intersects any obstacle or boundary.
    pub fn check_collision(&self, pos: (f32, f32, f32), radius: f32, height: f32) -> bool {
        if pos.0 - radius < self.min_x
            || pos.0 + radius > self.max_x
            || pos.2 - radius < self.min_z
            || pos.2 + radius > self.max_z
        {
            return true;
        }

        for obstacle in &self.obstacles {
            if obstacle.intersects_cylinder(pos, radius, height) {
                return true;
            }
        }
        false
    }

    /// Resolves movement collisions against boundaries and obstacles with sliding response.
    pub fn resolve_movement(
        &self,
        new_pos: (f32, f32, f32),
        radius: f32,
        height: f32,
    ) -> (f32, f32, f32) {
        let mut pos = self.clamp_to_bounds(new_pos, radius);

        for obstacle in &self.obstacles {
            pos = obstacle.resolve_cylinder(pos, radius, height);
        }

        // Re-clamp to bounds after obstacle displacement
        self.clamp_to_bounds(pos, radius)
    }
}

/// Movement physics configuration for walking, sprinting, jumping, and collision.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementConfig {
    pub walk_speed: f32,
    pub sprint_speed: f32,
    pub acceleration: f32,
    pub deceleration: f32,
    pub jump_velocity: f32,
    pub gravity: f32,
    pub radius: f32,
    pub height: f32,
}

impl Default for MovementConfig {
    fn default() -> Self {
        MovementConfig {
            walk_speed: 6.0,
            sprint_speed: 10.0,
            acceleration: 40.0,
            deceleration: 30.0,
            jump_velocity: 7.5,
            gravity: 19.6,
            radius: 0.4,
            height: 1.8,
        }
    }
}

/// Authoritative movement validation that evaluates a client requested movement,
/// enforcing maximum legal speed limits and terrain collision clamping.
///
/// Returns the authoritative clamped position and whether illegal movement was detected.
///
/// A non-finite request is refused outright by returning `previous_pos` and
/// `true`: NaN makes every comparison below evaluate to `false`, so it must
/// never be allowed to reach the clamping arithmetic.
pub fn validate_authoritative_movement(
    previous_pos: (f32, f32, f32),
    requested_pos: (f32, f32, f32),
    dt: f32,
    terrain: &GreyboxTerrain,
    config: &MovementConfig,
) -> ((f32, f32, f32), bool) {
    if !requested_pos.0.is_finite() || !requested_pos.1.is_finite() || !requested_pos.2.is_finite()
    {
        return (previous_pos, true);
    }

    let dt_clamped = dt.clamp(0.001, 0.5);
    // Allow sprint speed plus a 10% network/float tolerance margin
    let max_legal_distance = config.sprint_speed * dt_clamped * 1.10;

    let dx = requested_pos.0 - previous_pos.0;
    let dy = requested_pos.1 - previous_pos.1;
    let dz = requested_pos.2 - previous_pos.2;
    let attempted_distance = (dx * dx + dy * dy + dz * dz).sqrt();

    let mut illegal = false;
    let target_pos = if attempted_distance > max_legal_distance {
        illegal = true;
        let scale = max_legal_distance / attempted_distance;
        (
            previous_pos.0 + dx * scale,
            previous_pos.1 + dy * scale,
            previous_pos.2 + dz * scale,
        )
    } else {
        requested_pos
    };

    // Authoritative collision resolution against terrain
    let resolved = terrain.resolve_movement(target_pos, config.radius, config.height);
    if resolved != requested_pos {
        illegal = true;
    }

    (resolved, illegal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a6_server_clamps_a_teleport_to_a_reachable_step() {
        let terrain = GreyboxTerrain::default();
        let config = MovementConfig::default();
        let (pos, illegal) = validate_authoritative_movement(
            (0.0, 0.0, 0.0),
            (50_000.0, 0.0, 0.0),
            1.0 / 30.0,
            &terrain,
            &config,
        );
        assert!(illegal, "a 50 km step must be flagged illegal");
        assert!(pos.0 < 1.0, "server accepted a client teleport: {pos:?}");
    }

    #[test]
    fn test_a6_server_clamps_movement_to_world_bounds() {
        let terrain = GreyboxTerrain::new((-10.0, 10.0), (-10.0, 10.0));
        let config = MovementConfig::default();
        // dt large enough that the speed clamp does not fire: only bounds do.
        let (pos, illegal) = validate_authoritative_movement(
            (9.0, 0.0, 0.0),
            (11.0, 0.0, 0.0),
            0.5,
            &terrain,
            &config,
        );
        assert!(illegal);
        assert!(
            pos.0 <= 10.0 - config.radius + 1e-4,
            "out of bounds: {pos:?}"
        );
    }

    #[test]
    fn test_a6_non_finite_request_is_refused_without_touching_position() {
        let terrain = GreyboxTerrain::default();
        let config = MovementConfig::default();
        let (pos, illegal) = validate_authoritative_movement(
            (3.0, 0.0, 4.0),
            (f32::NAN, 0.0, f32::INFINITY),
            1.0 / 30.0,
            &terrain,
            &config,
        );
        assert!(illegal);
        assert_eq!(pos, (3.0, 0.0, 4.0));
    }

    #[test]
    fn test_a6_legal_step_is_accepted_unchanged() {
        let terrain = GreyboxTerrain::default();
        let config = MovementConfig::default();
        let (pos, illegal) = validate_authoritative_movement(
            (0.0, 0.0, 0.0),
            (0.2, 0.0, 0.0),
            1.0 / 30.0,
            &terrain,
            &config,
        );
        assert!(!illegal);
        assert_eq!(pos, (0.2, 0.0, 0.0));
    }

    #[test]
    fn test_obstacle_resolution_pushes_body_out() {
        let terrain = GreyboxTerrain::default_arena();
        let resolved = terrain.resolve_movement((0.0, 0.0, 0.0), 0.4, 1.8);
        assert!(
            !terrain.check_collision(resolved, 0.4, 1.8),
            "body left inside an obstacle: {resolved:?}"
        );
    }
}
