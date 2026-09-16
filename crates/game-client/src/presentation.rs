use crate::avatar::Avatar;
use crate::camera::{CameraMode, ThirdPersonCamera};
use crate::fog_view::FogViewSnapshot;
use crate::hud::DebugHud;
use crate::input::InputState;
use crate::interaction::camera_interaction_ray;
use crate::interpolation::{EntitySample, InterpolationBuffer};
use crate::logistics_view::LogisticsViewSnapshot;
use crate::placement::PlacementGhost;
use crate::power_view::PowerViewSnapshot;
use crate::prediction::{MovementInputSnapshot, PlayerState, PredictedController};
use crate::selection::TacticalSelection;
use game_types::{EntityId, FactionId, RegionId, SimTick, StructureId};
use sim_core::command::Command;
use sim_core::structure::{StructureKind, StructureRegistry, StructureState};
use sim_core::terrain::GreyboxTerrain;
use sim_core::world::WorldState;
use std::collections::BTreeMap;

/// Presentation representation of a world structure.
#[derive(Debug, Clone, PartialEq)]
pub struct StructureRenderView {
    pub id: StructureId,
    pub position: (f32, f32, f32),
    pub kind: StructureKind,
    pub state: StructureState,
}

/// Active visual overlay layers for strategic and tactical modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OverlayFlags {
    pub show_sensor_coverage: bool,
    pub show_power_grid: bool,
    pub show_logistics_network: bool,
    pub show_production_summary: bool,
}

/// Macro summary of industrial manufacturing and refining throughput.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ProductionSummaryTelemetry {
    pub active_facilities: usize,
    pub operational_facilities: usize,
    pub total_cycles_completed: u64,
    pub starved_facilities: usize,
}

/// Snapshot of the complete presentation frame ready for rendering.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationFrame {
    pub camera_eye: (f32, f32, f32),
    pub camera_focus: (f32, f32, f32),
    pub camera_forward: (f32, f32, f32),
    pub camera_mode: CameraMode,
    pub is_transitioning: bool,
    pub transition_progress: f32,
    pub player_position: (f32, f32, f32),
    pub player_facing_yaw: f32,
    pub player_locomotion: crate::avatar::LocomotionState,
    pub remote_entities: Vec<(EntityId, (f32, f32, f32))>,
    pub selection: TacticalSelection,
    pub placement_ghost: Option<PlacementGhost>,
    pub structures: Vec<StructureRenderView>,
    pub power_overlay: Option<PowerViewSnapshot>,
    pub logistics_overlay: Option<LogisticsViewSnapshot>,
    pub production_summary: Option<ProductionSummaryTelemetry>,
    pub hud_summary: String,
}

/// High-level client presentation coordinator uniting camera, input, predicted movement,
/// terrain collision, avatar representation, remote entity interpolation, building placement,
/// tactical unit selection, and strategic overlays.
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
    pub selection: TacticalSelection,
    pub overlay_flags: OverlayFlags,
    pub faction_id: FactionId,
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
            selection: TacticalSelection::default(),
            overlay_flags: OverlayFlags::default(),
            faction_id: FactionId::new(1),
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

        // 1. Process camera mode transitions
        self.camera.update_transition(dt);

        // 2. Process mouse look orbit & zoom
        if self.input.mouse_delta_x.abs() > 0.001 || self.input.mouse_delta_y.abs() > 0.001 {
            let delta_yaw = self.input.mouse_delta_x * self.mouse_sensitivity;
            let delta_pitch = -self.input.mouse_delta_y * self.mouse_sensitivity;
            self.camera.orbit(delta_yaw, delta_pitch);
        }
        if self.input.zoom_delta.abs() > 0.001 {
            self.camera.zoom(-self.input.zoom_delta * 1.5);
        }

        // 3. Compute camera-relative world movement direction
        let move_dir = self
            .input
            .compute_planar_movement_direction(self.camera.yaw);

        // 4. Generate input snapshot and step local prediction for avatar
        let seq = self.next_input_seq;
        self.next_input_seq += 1;

        let input_snapshot =
            MovementInputSnapshot::new(seq, dt, move_dir, self.input.sprint, self.input.jump);

        let predicted_state = self
            .controller
            .step_prediction(input_snapshot, &self.terrain);

        // 5. Update avatar visual and animation state
        self.avatar
            .update(predicted_state.velocity, predicted_state.grounded, dt);

        // 6. Update camera focus: follow avatar in ThirdPerson; preserve free pan in Tactical/Strategic
        if self.camera.mode == CameraMode::ThirdPerson && !self.camera.is_transitioning() {
            self.camera.set_target(predicted_state.position);
        }

        // 7. Reset transient input deltas
        self.input.reset_frame_deltas();

        // 8. Update interactive placement ghost if active
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

        // 9. Update HUD telemetry
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
        self.hud.camera_mode = self.camera.mode();
        self.hud.selected_units_count = self.selection.len();

        // 10. Produce presentation frame
        self.build_frame(current_time_ms)
    }

    /// Set camera mode with smooth transition over `duration_sec`.
    pub fn set_camera_mode(&mut self, mode: CameraMode, duration_sec: f32) {
        self.camera.set_mode(mode, duration_sec);
    }

    /// Pan camera across ground plane (Tactical or Strategic mode).
    pub fn pan_camera(&mut self, delta_right: f32, delta_forward: f32) {
        self.camera.pan(delta_right, delta_forward);
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

    /// Select units using 2D screen marquee drag box.
    pub fn select_marquee(
        &mut self,
        candidates: &[(EntityId, (f32, f32, f32))],
        shift_append: bool,
    ) -> usize {
        self.selection
            .complete_marquee(&self.camera, candidates, shift_append)
    }

    /// Issue tactical move order for currently selected units.
    pub fn issue_tactical_move(&self, target_pos: (f32, f32, f32)) -> Vec<Command> {
        self.selection.issue_move_order(target_pos)
    }

    /// Issue tactical attack order for currently selected units.
    pub fn issue_tactical_attack(&self, target_entity: EntityId) -> Vec<Command> {
        self.selection.issue_attack_order(target_entity)
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

        let power_overlay = if self.overlay_flags.show_power_grid {
            Some(self.extract_power_snapshot())
        } else {
            None
        };

        let logistics_overlay = if self.overlay_flags.show_logistics_network {
            Some(self.extract_logistics_snapshot())
        } else {
            None
        };

        let production_summary = if self.overlay_flags.show_production_summary {
            Some(self.extract_production_summary())
        } else {
            None
        };

        PresentationFrame {
            camera_eye: self.camera.eye_position(),
            camera_focus: self.camera.focus_position(),
            camera_forward: self.camera.forward_vector(),
            camera_mode: self.camera.mode(),
            is_transitioning: self.camera.is_transitioning(),
            transition_progress: self.camera.transition_progress(),
            player_position: self.controller.predicted_state.position,
            player_facing_yaw: self.avatar.facing_yaw,
            player_locomotion: self.avatar.locomotion_state,
            remote_entities: remote_positions,
            selection: self.selection.clone(),
            placement_ghost: if self.placement_ghost.active {
                Some(self.placement_ghost.clone())
            } else {
                None
            },
            structures,
            power_overlay,
            logistics_overlay,
            production_summary,
            hud_summary: self.hud.render_compact(),
        }
    }

    /// Extract power network diagnostic overlay snapshot
    pub fn extract_power_snapshot(&self) -> PowerViewSnapshot {
        PowerViewSnapshot::extract(
            &self.structure_registry.power_network,
            &self.structure_registry,
            self.faction_id,
        )
    }

    /// Extract logistics diagnostic overlay snapshot
    pub fn extract_logistics_snapshot(&self) -> LogisticsViewSnapshot {
        LogisticsViewSnapshot::extract(&self.structure_registry.logistics, &self.structure_registry)
    }

    /// Extract production telemetry summary
    pub fn extract_production_summary(&self) -> ProductionSummaryTelemetry {
        let active_facilities = self.structure_registry.facilities.len();
        let mut operational = 0;
        for id in self.structure_registry.facilities.keys() {
            if self
                .structure_registry
                .structures
                .get(id)
                .is_some_and(|s| s.state.is_operational())
            {
                operational += 1;
            }
        }

        ProductionSummaryTelemetry {
            active_facilities,
            operational_facilities: operational,
            total_cycles_completed: 0,
            starved_facilities: 0,
        }
    }

    /// Extract fog view snapshot given world state
    pub fn extract_fog_snapshot(&self, world: &WorldState) -> FogViewSnapshot {
        FogViewSnapshot::extract(world, self.faction_id)
    }
}
