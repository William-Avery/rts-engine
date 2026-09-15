use crate::camera::ThirdPersonCamera;

/// 3D ray represented by an origin point and a normalized direction vector.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    pub origin: (f32, f32, f32),
    pub direction: (f32, f32, f32),
}

impl Ray {
    pub fn new(origin: (f32, f32, f32), direction: (f32, f32, f32)) -> Self {
        let len_sq =
            direction.0 * direction.0 + direction.1 * direction.1 + direction.2 * direction.2;
        let norm_dir = if len_sq > 0.00001 {
            let inv_len = 1.0 / len_sq.sqrt();
            (
                direction.0 * inv_len,
                direction.1 * inv_len,
                direction.2 * inv_len,
            )
        } else {
            (0.0, 0.0, 1.0)
        };

        Ray {
            origin,
            direction: norm_dir,
        }
    }

    /// Evaluates point along the ray at distance `t`.
    pub fn point_at(&self, t: f32) -> (f32, f32, f32) {
        (
            self.origin.0 + self.direction.0 * t,
            self.origin.1 + self.direction.1 * t,
            self.origin.2 + self.direction.2 * t,
        )
    }

    /// Intersects ray with horizontal ground plane at `ground_y`.
    /// Returns intersection coordinate if ray points towards the plane.
    pub fn intersect_ground(&self, ground_y: f32) -> Option<(f32, f32, f32)> {
        if self.direction.1.abs() < 1e-6 {
            return None; // Ray is parallel to ground
        }

        let t = (ground_y - self.origin.1) / self.direction.1;
        if t < 0.0 {
            return None; // Intersection is behind ray origin
        }

        Some(self.point_at(t))
    }

    /// Intersects ray with an axis-aligned bounding box (AABB) using the slab method.
    /// Returns distance `t` along the ray to the nearest intersection point.
    pub fn intersect_aabb(
        &self,
        aabb_min: (f32, f32, f32),
        aabb_max: (f32, f32, f32),
    ) -> Option<f32> {
        let mut t_near = f32::NEG_INFINITY;
        let mut t_far = f32::INFINITY;

        // Check X axis
        if self.direction.0.abs() < 1e-6 {
            if self.origin.0 < aabb_min.0 || self.origin.0 > aabb_max.0 {
                return None;
            }
        } else {
            let t1 = (aabb_min.0 - self.origin.0) / self.direction.0;
            let t2 = (aabb_max.0 - self.origin.0) / self.direction.0;
            let (t_min, t_max) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
            t_near = t_near.max(t_min);
            t_far = t_far.min(t_max);
            if t_near > t_far || t_far < 0.0 {
                return None;
            }
        }

        // Check Y axis
        if self.direction.1.abs() < 1e-6 {
            if self.origin.1 < aabb_min.1 || self.origin.1 > aabb_max.1 {
                return None;
            }
        } else {
            let t1 = (aabb_min.1 - self.origin.1) / self.direction.1;
            let t2 = (aabb_max.1 - self.origin.1) / self.direction.1;
            let (t_min, t_max) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
            t_near = t_near.max(t_min);
            t_far = t_far.min(t_max);
            if t_near > t_far || t_far < 0.0 {
                return None;
            }
        }

        // Check Z axis
        if self.direction.2.abs() < 1e-6 {
            if self.origin.2 < aabb_min.2 || self.origin.2 > aabb_max.2 {
                return None;
            }
        } else {
            let t1 = (aabb_min.2 - self.origin.2) / self.direction.2;
            let t2 = (aabb_max.2 - self.origin.2) / self.direction.2;
            let (t_min, t_max) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
            t_near = t_near.max(t_min);
            t_far = t_far.min(t_max);
            if t_near > t_far || t_far < 0.0 {
                return None;
            }
        }

        Some(t_near.max(0.0))
    }
}

/// Computes a world-space interaction ray from the camera.
/// `ndc_x` and `ndc_y` are normalized device coordinates in [-1, 1], with (0,0) at viewport center.
pub fn camera_interaction_ray(camera: &ThirdPersonCamera, ndc_x: f32, ndc_y: f32) -> Ray {
    let eye = camera.eye_position();
    let fwd = camera.forward_vector();
    let right = camera.planar_right();

    // Camera up vector (cross product of right and forward)
    let up = (
        right.1 * fwd.2 - right.2 * fwd.1,
        right.2 * fwd.0 - right.0 * fwd.2,
        right.0 * fwd.1 - right.1 * fwd.0,
    );

    // Approximate perspective FOV direction with 60 deg horizontal FOV (tan ~ 0.577)
    let fov_factor = 0.577;
    let dir = (
        fwd.0 + (right.0 * ndc_x + up.0 * ndc_y) * fov_factor,
        fwd.1 + (right.1 * ndc_x + up.1 * ndc_y) * fov_factor,
        fwd.2 + (right.2 * ndc_x + up.2 * ndc_y) * fov_factor,
    );

    Ray::new(eye, dir)
}
