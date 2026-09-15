use crate::terrain::GreyboxTerrain;
use std::collections::VecDeque;

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

/// A timestamped input snapshot corresponding to a single client tick.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementInputSnapshot {
    pub sequence: u64,
    pub delta_seconds: f32,
    pub move_direction: (f32, f32, f32),
    pub sprint: bool,
    pub jump: bool,
}

impl MovementInputSnapshot {
    pub fn new(
        sequence: u64,
        delta_seconds: f32,
        move_direction: (f32, f32, f32),
        sprint: bool,
        jump: bool,
    ) -> Self {
        MovementInputSnapshot {
            sequence,
            delta_seconds,
            move_direction,
            sprint,
            jump,
        }
    }
}

/// Complete kinematic state of a player avatar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayerState {
    pub position: (f32, f32, f32),
    pub velocity: (f32, f32, f32),
    pub yaw: f32,
    pub grounded: bool,
    pub sequence: u64,
}

impl Default for PlayerState {
    fn default() -> Self {
        PlayerState {
            position: (0.0, 0.0, 0.0),
            velocity: (0.0, 0.0, 0.0),
            yaw: 0.0,
            grounded: true,
            sequence: 0,
        }
    }
}

impl PlayerState {
    pub fn new(position: (f32, f32, f32)) -> Self {
        PlayerState {
            position,
            velocity: (0.0, 0.0, 0.0),
            yaw: 0.0,
            grounded: true,
            sequence: 0,
        }
    }
}

/// Pure deterministic movement simulation step.
/// Shared between client-side prediction and server-side authoritative movement validation.
pub fn simulate_movement_step(
    state: &mut PlayerState,
    input: &MovementInputSnapshot,
    terrain: &GreyboxTerrain,
    config: &MovementConfig,
) {
    let dt = input.delta_seconds.clamp(0.001, 0.1);

    // 1. Target horizontal velocity
    let target_speed = if input.sprint {
        config.sprint_speed
    } else {
        config.walk_speed
    };

    let target_vx = input.move_direction.0 * target_speed;
    let target_vz = input.move_direction.2 * target_speed;

    // 2. Horizontal acceleration/deceleration
    let has_input = (input.move_direction.0 * input.move_direction.0
        + input.move_direction.2 * input.move_direction.2)
        > 0.001;
    let accel = if has_input {
        config.acceleration
    } else {
        config.deceleration
    };

    let diff_vx = target_vx - state.velocity.0;
    let diff_vz = target_vz - state.velocity.2;
    let max_step = accel * dt;

    let diff_len = (diff_vx * diff_vx + diff_vz * diff_vz).sqrt();
    if diff_len <= max_step || diff_len < 0.0001 {
        state.velocity.0 = target_vx;
        state.velocity.2 = target_vz;
    } else {
        let factor = max_step / diff_len;
        state.velocity.0 += diff_vx * factor;
        state.velocity.2 += diff_vz * factor;
    }

    // 3. Vertical velocity / Jump / Gravity
    if state.grounded && input.jump {
        state.velocity.1 = config.jump_velocity;
        state.grounded = false;
    } else if !state.grounded {
        state.velocity.1 -= config.gravity * dt;
    }

    // 4. Provisional position integration
    let provisional_pos = (
        state.position.0 + state.velocity.0 * dt,
        state.position.1 + state.velocity.1 * dt,
        state.position.2 + state.velocity.2 * dt,
    );

    // 5. Vertical ground resolution
    let mut resolved_pos = provisional_pos;
    if resolved_pos.1 <= terrain.ground_y {
        resolved_pos.1 = terrain.ground_y;
        state.velocity.1 = 0.0;
        state.grounded = true;
    } else {
        state.grounded = false;
    }

    // 6. Horizontal boundaries and obstacle collision resolution
    resolved_pos = terrain.resolve_movement(resolved_pos, config.radius, config.height);

    state.position = resolved_pos;
    state.sequence = input.sequence;
}

/// Authoritative movement validation that evaluates a client's requested movement,
/// enforcing maximum legal speed limits and terrain collision clamping.
///
/// Returns the authoritative clamped position and whether illegal movement was detected.
pub fn validate_authoritative_movement(
    previous_pos: (f32, f32, f32),
    requested_pos: (f32, f32, f32),
    dt: f32,
    terrain: &GreyboxTerrain,
    config: &MovementConfig,
) -> ((f32, f32, f32), bool) {
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

/// Manages client-side movement prediction, input history buffering,
/// and server reconciliation with re-simulation on divergence.
#[derive(Debug, Clone)]
pub struct PredictedController {
    pub predicted_state: PlayerState,
    pub authoritative_state: PlayerState,
    pub config: MovementConfig,
    pub input_history: VecDeque<MovementInputSnapshot>,
    pub history_capacity: usize,
    pub reconciliation_count: u64,
    pub last_reconciled_seq: u64,
    pub error_threshold: f32,
}

impl Default for PredictedController {
    fn default() -> Self {
        PredictedController {
            predicted_state: PlayerState::default(),
            authoritative_state: PlayerState::default(),
            config: MovementConfig::default(),
            input_history: VecDeque::with_capacity(128),
            history_capacity: 128,
            reconciliation_count: 0,
            last_reconciled_seq: 0,
            error_threshold: 0.01, // 1cm error threshold
        }
    }
}

impl PredictedController {
    pub fn new(initial_position: (f32, f32, f32)) -> Self {
        let initial_state = PlayerState::new(initial_position);
        PredictedController {
            predicted_state: initial_state,
            authoritative_state: initial_state,
            config: MovementConfig::default(),
            input_history: VecDeque::with_capacity(128),
            history_capacity: 128,
            reconciliation_count: 0,
            last_reconciled_seq: 0,
            error_threshold: 0.01,
        }
    }

    /// Step prediction locally on input without waiting for server acknowledgement.
    pub fn step_prediction(
        &mut self,
        input: MovementInputSnapshot,
        terrain: &GreyboxTerrain,
    ) -> PlayerState {
        // Enforce history capacity
        if self.input_history.len() >= self.history_capacity {
            self.input_history.pop_front();
        }
        self.input_history.push_back(input.clone());

        // Predict forward
        simulate_movement_step(&mut self.predicted_state, &input, terrain, &self.config);
        self.predicted_state
    }

    /// Reconcile prediction with authoritative server state.
    ///
    /// If the predicted state at the acknowledged sequence diverges from the server's state,
    /// the controller resets to the authoritative state and re-simulates all pending inputs.
    pub fn reconcile_with_server(
        &mut self,
        server_state: PlayerState,
        terrain: &GreyboxTerrain,
    ) -> bool {
        self.authoritative_state = server_state;
        self.last_reconciled_seq = server_state.sequence;

        // Discard acknowledged inputs
        while let Some(front) = self.input_history.front() {
            if front.sequence <= server_state.sequence {
                self.input_history.pop_front();
            } else {
                break;
            }
        }

        // Re-simulate pending unacknowledged inputs starting from the authoritative state
        let mut sim_state = server_state;
        for input in &self.input_history {
            simulate_movement_step(&mut sim_state, input, terrain, &self.config);
        }

        // Check divergence between predicted state and re-simulated state
        let dx = self.predicted_state.position.0 - sim_state.position.0;
        let dy = self.predicted_state.position.1 - sim_state.position.1;
        let dz = self.predicted_state.position.2 - sim_state.position.2;
        let divergence = (dx * dx + dy * dy + dz * dz).sqrt();

        if divergence > self.error_threshold {
            // Misprediction detected: snap predicted state to re-simulated correct state
            self.predicted_state = sim_state;
            self.reconciliation_count += 1;
            true
        } else {
            // Prediction matched server authority
            false
        }
    }

    /// Distance between current predicted position and authoritative server position.
    pub fn error_distance(&self) -> f32 {
        let dx = self.predicted_state.position.0 - self.authoritative_state.position.0;
        let dy = self.predicted_state.position.1 - self.authoritative_state.position.1;
        let dz = self.predicted_state.position.2 - self.authoritative_state.position.2;
        (dx * dx + dy * dy + dz * dz).sqrt()
    }
}
