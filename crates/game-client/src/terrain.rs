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
