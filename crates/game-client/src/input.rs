/// Client input state capturing raw user intention across movement, look, and actions.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InputState {
    /// Forward/backward axis (-1.0 for backward, +1.0 for forward).
    pub forward: f32,
    /// Left/right strafe axis (-1.0 for left, +1.0 for right).
    pub strafe: f32,
    /// Jump action trigger.
    pub jump: bool,
    /// Sprint modifier.
    pub sprint: bool,
    /// Horizontal mouse look delta (yaw changes).
    pub mouse_delta_x: f32,
    /// Vertical mouse look delta (pitch changes).
    pub mouse_delta_y: f32,
    /// Mouse wheel zoom delta.
    pub zoom_delta: f32,
}

impl InputState {
    pub fn new() -> Self {
        InputState::default()
    }

    /// Reset transient per-frame deltas (mouse look and zoom) while retaining held keys.
    pub fn reset_frame_deltas(&mut self) {
        self.mouse_delta_x = 0.0;
        self.mouse_delta_y = 0.0;
        self.zoom_delta = 0.0;
        self.jump = false;
    }

    /// Whether there is any active directional movement input.
    pub fn is_moving(&self) -> bool {
        self.forward.abs() > 0.001 || self.strafe.abs() > 0.001
    }

    /// Converts local WASD inputs into a normalized world-space movement vector on the XZ plane,
    /// oriented according to the camera yaw angle.
    pub fn compute_planar_movement_direction(&self, camera_yaw: f32) -> (f32, f32, f32) {
        if !self.is_moving() {
            return (0.0, 0.0, 0.0);
        }

        let fwd_x = camera_yaw.sin();
        let fwd_z = camera_yaw.cos();

        let right_x = camera_yaw.cos();
        let right_z = -camera_yaw.sin();

        let move_x = fwd_x * self.forward + right_x * self.strafe;
        let move_z = fwd_z * self.forward + right_z * self.strafe;

        let len_sq = move_x * move_x + move_z * move_z;
        if len_sq > 1.0 {
            let inv_len = 1.0 / len_sq.sqrt();
            (move_x * inv_len, 0.0, move_z * inv_len)
        } else {
            (move_x, 0.0, move_z)
        }
    }
}
