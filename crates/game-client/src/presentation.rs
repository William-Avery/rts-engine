use crate::avatar::Avatar;
use crate::camera::ThirdPersonCamera;
use crate::hud::DebugHud;
use crate::input::InputState;
use crate::interaction::camera_interaction_ray;
use crate::interpolation::{EntitySample, InterpolationBuffer};
use crate::logistics_view::LogisticsViewSnapshot;
use crate::placement::PlacementGhost;
use crate::prediction::{MovementInputSnapshot, PlayerState, PredictedController};
use crate::terrain::GreyboxTerrain;
use game_types::{EntityId, RegionId, SimTick, StructureId};
use sim_core::command::Command;
use sim_core::structure::{StructureKind, StructureRegistry, StructureState};
use std::collections::BTreeMap;

/// Presentation representation of a world structure.
#[derive(Debug, Clone, PartialEq)]
pub struct StructureRenderView {
    pub id: StructureId,
    pub position: (f32, f32, f32),
    pub kind: StructureKind,
    pub state: StructureState,
}

/// Snapshot of the complete presentation frame ready for rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationFrame {
    pub camera_eye: (f32, f32, f32),
    pub camera_focus: (f32, f32, f32),
    pub camera_forward: (f32, f32, f32),
    pub player_position: (f32, f32, f32),
    pub player_facing_yaw: f32,
    pub player_locomotion: crate::avatar::LocomotionState,
    pub remote_entities: Vec<(EntityId, (f32, f32, f32))>,
    pub placement_ghost: Option<PlacementGhost>,
    pub structures: Vec<StructureRenderView>,
    pub hud_summary: String,
}

/// High-level client presentation coordinator uniting camera, input, predicted movement,
/// terrain collision, avatar representation, remote entity interpolation, building placement, and debug HUD.
#[derive(Debug, Clone)]
pub struct ClientPresentation {
    pub camera: ThirdPersonCamera,
    pub input: InputState,
    pub controller: PredictedController,
    pub terrain: GreyboxTerrain,
    pub avatar: Avatar,
    pub hud: DebugHud,
    pub remote_entities: BTreeMap<EntityId, InterpolationBuffer>,
    pub structure_registry: StructureRegistry,
    pub placement_ghost: PlacementGhost,
    pub next_input_seq: u64,
    pub client_tick: SimTick,
    pub mouse_sensitivity: f32,
}

impl Default for ClientPresentation {
    fn default() -> Self {
        ClientPresentation {
            camera: ThirdPersonCamera::default(),
            input: InputState::default(),
            controller: PredictedController::default(),
            terrain: GreyboxTerrain::default_arena(),
            avatar: Avatar::default(),
            hud: DebugHud::default(),
            remote_entities: BTreeMap::new(),
            structure_registry: StructureRegistry::default(),
            placement_ghost: PlacementGhost::default(),
            next_input_seq: 1,
            client_tick: SimTick::zero(),
            mouse_sensitivity: 0.003,
        }
    }
}

impl ClientPresentation {
    pub fn new() -> Self {
        ClientPresentation::default()
    }

    /// Primary per-frame update advancing input processing, prediction, and camera tracking.
    pub fn update(&mut self, dt: f32, current_time_ms: u64) -> PresentationFrame {
        self.client_tick = self.client_tick.next();

        // 1. Process mouse look orbit & zoom
        if self.input.mouse_delta_x.abs() > 0.001 || self.input.mouse_delta_y.abs() > 0.001 {
            let delta_yaw = self.input.mouse_delta_x * self.mouse_sensitivity;
            let delta_pitch = -self.input.mouse_delta_y * self.mouse_sensitivity;
            self.camera.orbit(delta_yaw, delta_pitch);
        }
        if self.input.zoom_delta.abs() > 0.001 {
            self.camera.zoom(-self.input.zoom_delta * 1.5);
        }

        // 2. Compute camera-relative world movement direction
        let move_dir = self
            .input
            .compute_planar_movement_direction(self.camera.yaw);

        // 3. Generate input snapshot and step local prediction
        let seq = self.next_input_seq;
        self.next_input_seq += 1;

        let input_snapshot =
            MovementInputSnapshot::new(seq, dt, move_dir, self.input.sprint, self.input.jump);

        let predicted_state = self
            .controller
            .step_prediction(input_snapshot, &self.terrain);

        // 4. Update avatar visual and animation state
        self.avatar
            .update(predicted_state.velocity, predicted_state.grounded, dt);

        // 5. Update camera focus to follow predicted avatar position
        self.camera.set_target(predicted_state.position);

        // 6. Reset transient input deltas
        self.input.reset_frame_deltas();

        // 7. Update interactive placement ghost if active
        if self.placement_ghost.active {
            let ray = camera_interaction_ray(&self.camera, 0.0, 0.0);
            if let Some(ground_pt) = ray.intersect_ground(self.terrain.ground_y) {
                self.placement_ghost.update_preview(
                    self.controller.predicted_state.position,
                    ground_pt,
                    &self.terrain,
                    &self.structure_registry,
                );
            }
        }

        // 8. Update HUD telemetry
        self.hud.update_telemetry(crate::hud::TelemetrySnapshot {
            ping_ms: self.hud.ping_ms,
            server_tick: self.hud.server_tick,
            client_tick: self.client_tick,
            region_id: self.hud.region_id,
            predicted_pos: self.controller.predicted_state.position,
            authoritative_pos: self.controller.authoritative_state.position,
            reconciliation_count: self.controller.reconciliation_count,
            active_entity_count: self.remote_entities.len(),
        });

        // 9. Produce presentation frame
        self.build_frame(current_time_ms)
    }

    /// Activate placement ghost for a structure kind.
    pub fn activate_placement(&mut self, kind: StructureKind) {
        self.placement_ghost.activate(kind);
    }

    /// Deactivate placement ghost.
    pub fn deactivate_placement(&mut self) {
        self.placement_ghost.deactivate();
    }

    /// Rotate placement ghost by 90 degrees.
    pub fn rotate_placement(&mut self) {
        self.placement_ghost.rotate_clockwise();
    }

    /// Generates authoritative build command if placement ghost is currently valid.
    pub fn create_build_command(&self) -> Option<Command> {
        self.placement_ghost.create_build_command()
    }

    /// Ingests authoritative player state from server snapshot, reconciling prediction.
    pub fn receive_server_authoritative_state(&mut self, server_state: PlayerState) -> bool {
        self.controller
            .reconcile_with_server(server_state, &self.terrain)
    }

    /// Ingests state sample for a remote replicated entity into its interpolation buffer.
    pub fn receive_remote_entity_sample(&mut self, entity_id: EntityId, sample: EntitySample) {
        self.remote_entities
            .entry(entity_id)
            .or_default()
            .push_sample(sample);
    }

    /// Removes a remote entity when despawned or out of scope.
    pub fn remove_remote_entity(&mut self, entity_id: EntityId) {
        self.remote_entities.remove(&entity_id);
    }

    /// Query smoothly interpolated position of a remote entity.
    pub fn get_remote_entity_position(
        &self,
        entity_id: EntityId,
        current_time_ms: u64,
    ) -> Option<(f32, f32, f32)> {
        self.remote_entities
            .get(&entity_id)
            .and_then(|buf| buf.interpolate_position(current_time_ms))
    }

    pub fn set_ping(&mut self, ping_ms: u64) {
        self.hud.ping_ms = ping_ms;
    }

    pub fn set_server_tick(&mut self, server_tick: SimTick) {
        self.hud.server_tick = server_tick;
    }

    pub fn set_region_id(&mut self, region_id: RegionId) {
        self.hud.region_id = region_id;
    }

    pub fn build_frame(&self, current_time_ms: u64) -> PresentationFrame {
        let mut remote_positions = Vec::with_capacity(self.remote_entities.len());
        for (&id, buf) in &self.remote_entities {
            if let Some(pos) = buf.interpolate_position(current_time_ms) {
                remote_positions.push((id, pos));
            }
        }

        let structures = self
            .structure_registry
            .structures
            .values()
            .map(|s| StructureRenderView {
                id: s.id,
                position: s.position,
                kind: s.kind,
                state: s.state,
            })
            .collect();

        PresentationFrame {
            camera_eye: self.camera.eye_position(),
            camera_focus: self.camera.focus_position(),
            camera_forward: self.camera.forward_vector(),
            player_position: self.controller.predicted_state.position,
            player_facing_yaw: self.avatar.facing_yaw,
            player_locomotion: self.avatar.locomotion_state,
            remote_entities: remote_positions,
            placement_ghost: if self.placement_ghost.active {
                Some(self.placement_ghost.clone())
            } else {
                None
            },
            structures,
            hud_summary: self.hud.render_compact(),
        }
    }

    /// Extract logistics diagnostic overlay snapshot
    pub fn extract_logistics_snapshot(&self) -> LogisticsViewSnapshot {
        LogisticsViewSnapshot::extract(&self.structure_registry.logistics, &self.structure_registry)
    }
}
