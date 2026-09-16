pub mod avatar;
pub mod camera;
pub mod fog_view;
pub mod hud;
pub mod input;
pub mod interaction;
pub mod interpolation;
pub mod logistics_view;
pub mod placement;
pub mod power_view;
pub mod prediction;
pub mod presentation;
pub mod research_view;
pub mod robot_view;
pub mod selection;
pub mod wall_batch;

pub use avatar::*;
pub use camera::*;
pub use fog_view::*;
pub use hud::*;
pub use input::*;
pub use interaction::*;
pub use interpolation::*;
pub use logistics_view::*;
pub use placement::*;
pub use power_view::*;
pub use prediction::*;
pub use presentation::*;
pub use research_view::*;
pub use robot_view::*;
pub use selection::*;
pub use wall_batch::*;

/// The authoritative collision world and movement validator live in `sim-core`.
/// The client re-exports them so prediction and the server run identical code.
pub use sim_core::terrain::{
    GreyboxTerrain, MovementConfig, ObstacleAabb, validate_authoritative_movement,
};

#[cfg(test)]
mod tests {
    use super::*;
    use game_types::{EntityId, RegionId, SimTick};
    use std::f32::consts::PI;

    #[test]
    fn test_camera_orbit_and_directional_vectors() {
        let mut cam = ThirdPersonCamera::new((0.0, 0.0, 0.0), 10.0, 0.0, 0.0);

        // At pitch = 0, yaw = 0: looking along +Z axis
        let fwd = cam.forward_vector();
        assert!((fwd.0 - 0.0).abs() < 1e-4);
        assert!((fwd.1 - 0.0).abs() < 1e-4);
        assert!((fwd.2 - 1.0).abs() < 1e-4);

        let pfwd = cam.planar_forward();
        let pright = cam.planar_right();
        // Dot product must be 0 (orthogonal)
        let dot = pfwd.0 * pright.0 + pfwd.1 * pright.1 + pfwd.2 * pright.2;
        assert!(dot.abs() < 1e-4);

        // Orbit pitch beyond +85 deg -> clamped to MAX_PITCH
        cam.orbit(0.0, 2.0); // 2 radians ~ 114 degrees
        assert!((cam.pitch - MAX_PITCH).abs() < 1e-4);

        // Orbit pitch below -85 deg -> clamped to MIN_PITCH
        cam.orbit(0.0, -4.0);
        assert!((cam.pitch - MIN_PITCH).abs() < 1e-4);

        // Zoom clamping
        cam.zoom(-100.0);
        assert!((cam.distance - cam.min_distance).abs() < 1e-4);
        cam.zoom(500.0);
        assert!((cam.distance - cam.max_distance).abs() < 1e-4);
    }

    #[test]
    fn test_input_conversion_camera_relative() {
        let mut input = InputState::new();
        input.forward = 1.0;

        // Camera facing North (yaw = 0): moving forward moves +Z
        let dir_north = input.compute_planar_movement_direction(0.0);
        assert!((dir_north.0 - 0.0).abs() < 1e-4);
        assert!((dir_north.2 - 1.0).abs() < 1e-4);

        // Camera facing East (yaw = PI/2): moving forward moves +X
        let dir_east = input.compute_planar_movement_direction(PI * 0.5);
        assert!((dir_east.0 - 1.0).abs() < 1e-4);
        assert!(dir_east.2.abs() < 1e-4);

        // Diagonal input normalized to unit length
        input.strafe = 1.0;
        let dir_diag = input.compute_planar_movement_direction(0.0);
        let len = (dir_diag.0 * dir_diag.0 + dir_diag.2 * dir_diag.2).sqrt();
        assert!((len - 1.0).abs() < 1e-4);
    }

    #[test]
    fn test_greybox_terrain_collision_and_bounds() {
        let mut terrain = GreyboxTerrain::new((-50.0, 50.0), (-50.0, 50.0));
        terrain.add_obstacle(ObstacleAabb::new(
            1,
            "Pillar",
            (-5.0, 0.0, -5.0),
            (5.0, 10.0, 5.0),
        ));

        // Point outside bounds is detected and clamped
        assert!(!terrain.is_point_inside_bounds(100.0, 0.0));
        let clamped = terrain.clamp_to_bounds((100.0, 0.0, 0.0), 0.4);
        assert!((clamped.0 - 49.6).abs() < 1e-4);

        // Movement into obstacle is resolved
        let target_in_pillar = (0.0, 0.0, 4.8); // penetrating pillar
        let resolved = terrain.resolve_movement(target_in_pillar, 0.4, 1.8);
        // Should be pushed out along Z (max_z = 5.0 + radius = 5.4)
        assert!(resolved.2 >= 5.39);
    }

    #[test]
    fn test_responsive_local_predicted_movement() {
        let terrain = GreyboxTerrain::default_arena();
        let mut controller = PredictedController::new((0.0, 0.0, 0.0));

        // Player immediately moves locally on tick 1 without waiting for server response
        let input = MovementInputSnapshot::new(1, 0.033, (0.0, 0.0, 1.0), false, false);
        let predicted = controller.step_prediction(input, &terrain);

        // Velocity accelerated towards target speed
        assert!(predicted.velocity.2 > 0.0);
        // Position changed immediately
        assert!(predicted.position.2 > 0.0);
        assert_eq!(controller.input_history.len(), 1);
    }

    #[test]
    fn test_server_reconciliation_on_divergence() {
        let terrain = GreyboxTerrain::default_arena();
        let mut controller = PredictedController::new((0.0, 0.0, 0.0));

        // Client predicts 5 forward ticks
        for seq in 1..=5 {
            let input = MovementInputSnapshot::new(seq, 0.033, (0.0, 0.0, 1.0), false, false);
            controller.step_prediction(input, &terrain);
        }

        let uncorrected_pred_z = controller.predicted_state.position.2;
        assert!(uncorrected_pred_z > 0.5);

        // Server acknowledges seq 3, but with an authoritative position clamping movement
        // (for example: client encountered a wall or latency event on the server)
        let server_state = PlayerState {
            position: (0.0, 0.0, 0.05), // server clamped player back to 0.05 at seq 3
            velocity: (0.0, 0.0, 1.0),
            yaw: 0.0,
            grounded: true,
            sequence: 3,
        };

        let reconciled = controller.reconcile_with_server(server_state, &terrain);

        // Reconciliation MUST trigger due to divergence
        assert!(reconciled);
        assert_eq!(controller.reconciliation_count, 1);

        // Unacknowledged inputs (seq 4 and 5) must be re-simulated from server state (0.05)
        assert_eq!(controller.input_history.len(), 2);
        assert_eq!(controller.input_history.front().unwrap().sequence, 4);
        assert_eq!(controller.input_history.back().unwrap().sequence, 5);

        // New predicted position is corrected and re-simulated from 0.05 rather than old uncorrected path
        assert!(controller.predicted_state.position.2 < uncorrected_pred_z);
        assert!(controller.predicted_state.position.2 > 0.05);
    }

    #[test]
    fn test_server_reconciliation_no_divergence() {
        let terrain = GreyboxTerrain::default_arena();
        let mut controller = PredictedController::new((0.0, 0.0, 0.0));

        // Client steps 3 ticks
        let mut states = Vec::new();
        for seq in 1..=3 {
            let input = MovementInputSnapshot::new(seq, 0.033, (0.0, 0.0, 1.0), false, false);
            let s = controller.step_prediction(input, &terrain);
            states.push(s);
        }

        // Server acknowledges seq 2 with the exact state client predicted
        let server_state = states[1]; // seq 2
        let reconciled = controller.reconcile_with_server(server_state, &terrain);

        // No misprediction, so reconciliation count remains 0
        assert!(!reconciled);
        assert_eq!(controller.reconciliation_count, 0);
        // Only seq 3 remains in history
        assert_eq!(controller.input_history.len(), 1);
    }

    #[test]
    fn test_remote_entity_interpolation() {
        let mut buf = InterpolationBuffer::new(50); // 50ms delay

        // Receive snapshot at t=100ms at (0, 0, 0)
        buf.push_sample(EntitySample::new(
            SimTick::new(1),
            100,
            (0.0, 0.0, 0.0),
            (10.0, 0.0, 0.0),
            0.0,
        ));

        // Receive snapshot at t=200ms at (10, 0, 0)
        buf.push_sample(EntitySample::new(
            SimTick::new(2),
            200,
            (10.0, 0.0, 0.0),
            (10.0, 0.0, 0.0),
            0.0,
        ));

        // Render at current_time = 200ms with 50ms delay -> render_time = 150ms (halfway)
        let pos = buf.interpolate_position(200).expect("Interpolation exists");
        assert!((pos.0 - 5.0).abs() < 1e-4);
        assert!((pos.1 - 0.0).abs() < 1e-4);
        assert!((pos.2 - 0.0).abs() < 1e-4);
    }

    #[test]
    fn test_debug_hud_telemetry() {
        let mut hud = DebugHud::new();
        hud.update_telemetry(TelemetrySnapshot {
            ping_ms: 42,
            server_tick: SimTick::new(100),
            client_tick: SimTick::new(105),
            region_id: RegionId::new(2),
            predicted_pos: (10.0, 0.0, 20.0),
            authoritative_pos: (9.95, 0.0, 20.0),
            reconciliation_count: 1,
            active_entity_count: 5,
        });

        let compact = hud.render_compact();
        assert!(compact.contains("Ping: 42ms"));
        assert!(compact.contains("SrvTick: 100"));
        assert!(compact.contains("CliTick: 105"));
        assert!(compact.contains("Reg: 2"));
        assert!(compact.contains("Reconciles: 1"));

        let card = hud.render_ascii_card();
        assert!(card.contains("CLIENT DEBUG TELEMETRY"));
        assert!(card.contains("42 ms"));
    }

    #[test]
    fn test_client_presentation_integration() {
        let mut presentation = ClientPresentation::new();
        presentation.input.forward = 1.0;
        presentation.set_ping(28);
        presentation.set_server_tick(SimTick::new(50));

        let frame = presentation.update(0.033, 100);

        // Player moved forward
        assert!(frame.player_position.2 > 0.0);
        // Camera tracked player
        assert!(frame.camera_focus.2 > 0.0);
        // Locomotion state is Walking
        assert_eq!(frame.player_locomotion, LocomotionState::Walking);
        // HUD contains telemetry
        assert!(frame.hud_summary.contains("Ping: 28ms"));
    }

    #[test]
    fn test_server_authority_clamps_illegal_speed_and_teleportation() {
        let terrain = GreyboxTerrain::default_arena();
        let config = MovementConfig::default();
        let prev_pos = (0.0, 0.0, 10.0);

        // Client attempts legal move (sprinting at 10m/s for 0.033s = ~0.33m)
        let legal_req = (0.0, 0.0, 10.30);
        let (resolved_legal, illegal_flag) =
            validate_authoritative_movement(prev_pos, legal_req, 0.033, &terrain, &config);
        assert!(!illegal_flag);
        assert!((resolved_legal.2 - 10.30).abs() < 1e-4);

        // Client attempts illegal speed-hack/teleport (moving 50 meters in 1 tick)
        let illegal_req = (0.0, 0.0, 60.0);
        let (clamped_pos, illegal_detected) =
            validate_authoritative_movement(prev_pos, illegal_req, 0.033, &terrain, &config);
        assert!(illegal_detected);
        // Server clamps distance to max sprint distance with tolerance (~0.363m)
        assert!(clamped_pos.2 < 11.0);
        assert!(clamped_pos.2 > 10.0);

        // Client attempts to move directly inside the central bunker obstacle (-6 to 6 on X and Z)
        let start_outside = (0.0, 0.0, 7.0);
        let try_inside = (0.0, 0.0, 5.0); // attempting to step inside bunker
        let (clamped_obstacle, obstacle_illegal) =
            validate_authoritative_movement(start_outside, try_inside, 0.033, &terrain, &config);
        assert!(obstacle_illegal);
        // Pushed back outside bunker boundary
        assert!(clamped_obstacle.2 >= 6.4);
    }

    #[test]
    fn test_remote_entity_lifecycle_in_presentation() {
        let mut presentation = ClientPresentation::new();
        let entity_id = EntityId::new(42);

        presentation.receive_remote_entity_sample(
            entity_id,
            EntitySample::new(
                SimTick::new(10),
                100,
                (15.0, 0.0, 25.0),
                (0.0, 0.0, 0.0),
                0.0,
            ),
        );

        let frame = presentation.build_frame(170);
        assert_eq!(frame.remote_entities.len(), 1);
        assert_eq!(frame.remote_entities[0].0, entity_id);

        presentation.remove_remote_entity(entity_id);
        let frame_after = presentation.build_frame(170);
        assert_eq!(frame_after.remote_entities.len(), 0);
    }

    #[test]
    fn test_interaction_ray_ground_intersection() {
        // Ray from (0, 10, 0) pointing downwards towards ground (Y=0)
        let ray = Ray::new((0.0, 10.0, 0.0), (0.0, -1.0, 0.0));
        let hit = ray.intersect_ground(0.0).expect("Should hit ground");
        assert!((hit.0 - 0.0).abs() < 1e-4);
        assert!((hit.1 - 0.0).abs() < 1e-4);
        assert!((hit.2 - 0.0).abs() < 1e-4);

        // Ray pointing upwards away from ground
        let up_ray = Ray::new((0.0, 10.0, 0.0), (0.0, 1.0, 0.0));
        assert!(up_ray.intersect_ground(0.0).is_none());

        // Ray horizontal parallel to ground
        let horiz_ray = Ray::new((0.0, 10.0, 0.0), (1.0, 0.0, 0.0));
        assert!(horiz_ray.intersect_ground(0.0).is_none());
    }

    #[test]
    fn test_interaction_ray_aabb_intersection() {
        let ray = Ray::new((0.0, 5.0, -10.0), (0.0, 0.0, 1.0));
        let box_min = (-2.0, 0.0, 0.0);
        let box_max = (2.0, 10.0, 4.0);

        let dist = ray
            .intersect_aabb(box_min, box_max)
            .expect("Should hit box");
        assert!((dist - 10.0).abs() < 1e-4); // Hits front face at Z=0, distance 10

        // Ray missing box
        let miss_ray = Ray::new((50.0, 5.0, -10.0), (0.0, 0.0, 1.0));
        assert!(miss_ray.intersect_aabb(box_min, box_max).is_none());
    }

    #[test]
    fn test_client_instant_placement_ghost_preview() {
        let terrain = GreyboxTerrain::default_arena();
        let registry = sim_core::structure::StructureRegistry::new();
        let mut ghost = PlacementGhost::new(sim_core::structure::StructureKind::DEFAULT_WALL);
        ghost.grid_size = 2.0;

        let player_pos = (0.0, 0.0, 10.0);

        // 1. Valid preview within reach, snapped to grid
        let target_pos = (1.9, 0.0, 11.8);
        ghost.update_preview(player_pos, target_pos, &terrain, &registry);
        assert_eq!(ghost.status, PlacementStatus::Valid);
        assert_eq!(ghost.snapped_pos, (2.0, 0.0, 12.0));
        assert!(ghost.create_build_command().is_some());

        // 2. Out of reach (> 15m)
        let far_pos = (0.0, 0.0, 30.0);
        ghost.update_preview(player_pos, far_pos, &terrain, &registry);
        assert_eq!(ghost.status, PlacementStatus::TooFarFromPlayer);
        assert!(ghost.create_build_command().is_none());

        // 3. Blocked by terrain obstacle (central bunker at -6 to 6 on X and Z)
        let bunker_pos = (0.0, 0.0, 4.0);
        ghost.update_preview(player_pos, bunker_pos, &terrain, &registry);
        assert_eq!(ghost.status, PlacementStatus::BlockedByTerrain);
        assert!(ghost.create_build_command().is_none());

        // 4. Rotation toggles extents
        ghost.rotate_clockwise();
        assert_eq!(ghost.rotation_deg, 90.0);
        ghost.rotate_clockwise();
        assert_eq!(ghost.rotation_deg, 180.0);

        // 5. Wall tier cycling
        assert_eq!(
            ghost.kind,
            sim_core::structure::StructureKind::Wall(sim_core::wall::WallTier::Mk1Stone)
        );
        ghost.cycle_wall_tier();
        assert_eq!(
            ghost.kind,
            sim_core::structure::StructureKind::Wall(sim_core::wall::WallTier::Mk2Steel)
        );
        ghost.cycle_wall_tier();
        assert_eq!(
            ghost.kind,
            sim_core::structure::StructureKind::Wall(sim_core::wall::WallTier::Mk3Composite)
        );
        ghost.cycle_wall_tier();
        assert_eq!(
            ghost.kind,
            sim_core::structure::StructureKind::Wall(sim_core::wall::WallTier::Mk1Stone)
        );
    }

    #[test]
    fn test_power_view_and_telemetry() {
        use game_types::{FactionId, RegionId, SimTick};
        use sim_core::structure::{BuildRequest, StructureKind, StructureRegistry};

        let mut registry = StructureRegistry::default();
        let faction = FactionId::new(1);
        let region = RegionId::new(1);

        // Place generator, pylons, battery, and turret
        let gen_id = registry
            .request_build(
                BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 0.0),
                    kind: StructureKind::Generator,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        let pylon1_id = registry
            .request_build(
                BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (8.0, 0.0, 0.0),
                    kind: StructureKind::Pylon,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        let pylon2_id = registry
            .request_build(
                BuildRequest {
                    player_pos: (20.0, 0.0, 0.0),
                    requested_pos: (25.0, 0.0, 0.0),
                    kind: StructureKind::Pylon,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        let turret_id = registry
            .request_build(
                BuildRequest {
                    player_pos: (28.0, 0.0, 0.0),
                    requested_pos: (30.0, 0.0, 0.0),
                    kind: StructureKind::Turret,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        let battery_id = registry
            .request_build(
                BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 5.0),
                    kind: StructureKind::Battery,
                    rotation_deg: 0.0,
                    faction_id: faction,
                    region_id: region,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        // Complete construction
        registry.complete_construction(gen_id).unwrap();
        registry.complete_construction(pylon1_id).unwrap();
        registry.complete_construction(pylon2_id).unwrap();
        registry.complete_construction(turret_id).unwrap();
        registry.complete_construction(battery_id).unwrap();

        // Tick to solve network
        registry.tick(SimTick::new(1));

        let snapshot = PowerViewSnapshot::extract(&registry.power_network, &registry, faction);

        assert!(
            !snapshot.links.is_empty(),
            "Expected transmission links between connected nodes"
        );
        assert_eq!(
            snapshot.coverages.len(),
            2,
            "Expected 2 pylon coverage visual circles"
        );
        assert_eq!(snapshot.telemetry.total_generation_kw, 100);
        assert_eq!(snapshot.telemetry.total_storage_capacity_kwh, 5000);
        assert!(snapshot.telemetry.powered_structures >= 4);

        let report = snapshot.render_ascii_report();
        assert!(report.contains("POWER NETWORK DIAGNOSTICS"));
        assert!(report.contains("OPTIMAL"));

        // Test DebugHud power section
        let mut hud = DebugHud::default();
        hud.update_power(100, 22, 1000, 5000, 1);
        let card = hud.render_ascii_card();
        assert!(card.contains("Power:   100 kW Gen"));
        assert!(card.contains("Battery Buffer:"));
    }
    #[test]
    fn test_research_view_reflects_tree_queue_and_modifier_state() {
        use sim_core::modifier::{ModifierGroup, ModifierKind};
        use sim_core::research::{
            ResearchJobState, ResearchManager, TECH_BASIC_METALLURGY, TECH_POWER_REGULATION,
            TECH_TUNGSTEN_PROCESSING,
        };

        let faction = game_types::FactionId::new(1);
        let mut research = ResearchManager::new();
        let mut journal = sim_core::event::EventJournal::new();

        // Nothing researched yet: roots are available, deep techs are locked.
        let snapshot = ResearchViewSnapshot::extract(&research, faction);
        assert_eq!(snapshot.telemetry.completed_techs, 0);
        assert!(snapshot.telemetry.total_techs > 0);
        assert!(snapshot.telemetry.available_techs > 0);
        assert!(snapshot.telemetry.locked_techs > 0);
        assert!(snapshot.modifiers.is_empty());
        let tungsten = snapshot
            .nodes
            .iter()
            .find(|n| n.tech_id == TECH_TUNGSTEN_PROCESSING)
            .unwrap();
        assert_eq!(tungsten.state, TechNodeState::Locked);

        // Complete one technology and queue another.
        research
            .grant_tech(
                faction,
                TECH_BASIC_METALLURGY,
                SimTick::zero(),
                &mut journal,
            )
            .unwrap();
        research
            .queue_research(
                faction,
                TECH_POWER_REGULATION,
                SimTick::zero(),
                &mut journal,
            )
            .unwrap();

        let snapshot = ResearchViewSnapshot::extract(&research, faction);
        assert_eq!(snapshot.telemetry.completed_techs, 1);
        assert_eq!(snapshot.telemetry.queued_jobs, 1);
        assert_eq!(snapshot.telemetry.active_tech, Some(TECH_POWER_REGULATION));
        assert_eq!(snapshot.queue[0].state, ResearchJobState::Queued);
        assert_eq!(snapshot.queue[0].position, 0);

        let metallurgy = snapshot
            .nodes
            .iter()
            .find(|n| n.tech_id == TECH_BASIC_METALLURGY)
            .unwrap();
        assert_eq!(metallurgy.state, TechNodeState::Completed);
        assert_eq!(metallurgy.color_rgba, (0.1, 0.95, 0.2, 1.0));

        let power = snapshot
            .nodes
            .iter()
            .find(|n| n.tech_id == TECH_POWER_REGULATION)
            .unwrap();
        assert_eq!(power.state, TechNodeState::Queued);

        // The completed technology's software patch is surfaced with a breakdown.
        let refining = snapshot
            .modifiers
            .iter()
            .find(|m| m.kind == ModifierKind::RefiningSpeed)
            .unwrap();
        assert_eq!(refining.multiplier_milli, 1100);
        assert!((refining.percent_delta - 10.0).abs() < 0.001);
        assert_eq!(
            refining.breakdown,
            vec![(ModifierGroup::SoftwarePatch, 100)]
        );
        assert_eq!(
            snapshot.telemetry.active_modifier_count,
            snapshot.modifiers.len()
        );
    }

    #[test]
    fn test_research_view_ascii_reports_render() {
        use sim_core::research::{ResearchManager, TECH_BASIC_METALLURGY};

        let faction = game_types::FactionId::new(1);
        let mut research = ResearchManager::new();
        let mut journal = sim_core::event::EventJournal::new();
        research
            .grant_tech(
                faction,
                TECH_BASIC_METALLURGY,
                SimTick::zero(),
                &mut journal,
            )
            .unwrap();

        let snapshot = ResearchViewSnapshot::extract(&research, faction);

        let report = snapshot.render_ascii_report();
        assert!(report.contains("RESEARCH NETWORK STATUS"));
        assert!(report.contains("Refining Speed"));
        assert!(report.contains("Modifier Patch"));

        let tree = snapshot.render_ascii_tree();
        assert!(tree.contains("TECH TREE"));
        assert!(tree.contains("Basic Metallurgy"));
        assert!(tree.contains("COMPLETE"));
        assert!(snapshot.max_tier() >= 1);
        assert!(!snapshot.nodes_in_tier(1).is_empty());
    }

    #[test]
    fn test_debug_hud_renders_research_telemetry() {
        let hud = DebugHud {
            research_techs_total: 14,
            research_techs_completed: 3,
            research_queue_depth: 2,
            research_active_progress: 0.5,
            research_active_modifiers: 4,
            ..Default::default()
        };

        let out = hud.render_ascii_card();
        assert!(out.contains("Research:"));
        assert!(out.contains("Active Software Patches"));
    }

    // =========================================================================
    // Milestone 15 — Tactical and Strategic Camera Modes Proving Grounds
    // =========================================================================

    /// M15 Proving Ground 1: Camera mode transitions smoothly between ThirdPerson, Tactical, and Strategic.
    #[test]
    fn test_m15_camera_mode_switching_and_smooth_interpolation() {
        let mut client = ClientPresentation::new();
        assert_eq!(client.camera.mode(), CameraMode::ThirdPerson);
        assert!(!client.camera.is_transitioning());

        // Initiate transition to Tactical mode over 0.5s
        client.set_camera_mode(CameraMode::Tactical, 0.5);
        assert!(client.camera.is_transitioning());
        assert_eq!(client.camera.transition_progress(), 0.0);

        // Step halfway (0.25s)
        let frame_mid = client.update(0.25, 250);
        assert!(frame_mid.is_transitioning);
        assert!(frame_mid.transition_progress > 0.4 && frame_mid.transition_progress < 0.6);
        assert!(client.camera.distance > 8.0 && client.camera.distance < 45.0);

        // Complete transition (0.25s)
        let frame_end = client.update(0.25, 500);
        assert!(!frame_end.is_transitioning);
        assert_eq!(frame_end.camera_mode, CameraMode::Tactical);
        assert!((client.camera.distance - 45.0).abs() < 1e-2);

        // Transition from Tactical to Strategic
        client.set_camera_mode(CameraMode::Strategic, 0.4);
        assert!(client.camera.is_transitioning());
        client.update(0.4, 900);
        assert!(!client.camera.is_transitioning());
        assert_eq!(client.camera.mode(), CameraMode::Strategic);
        assert!((client.camera.distance - 180.0).abs() < 1e-2);
    }

    /// M15 Proving Ground 2: Tactical camera pans freely on ground plane and respects world boundaries.
    #[test]
    fn test_m15_tactical_panning_and_boundary_clamping() {
        let mut client = ClientPresentation::new();
        client.camera.world_bounds_xz = (-150.0, 150.0, -150.0, 150.0);
        client.set_camera_mode(CameraMode::Tactical, 0.0);
        assert_eq!(client.camera.mode(), CameraMode::Tactical);

        // Player avatar is initialized at (20, 0, 20) on open ground
        let initial_avatar_pos = (20.0, 0.0, 20.0);
        client.controller.predicted_state.position = initial_avatar_pos;
        client.controller.authoritative_state.position = initial_avatar_pos;

        // Pan tactical camera target right 60m and forward 40m
        client.pan_camera(60.0, 40.0);
        let frame = client.update(0.016, 16);

        // Camera focus panned to (60, 0, 40), while avatar remains at initial position
        assert!((frame.camera_focus.0 - 60.0).abs() < 1e-2);
        assert!((frame.camera_focus.2 - 40.0).abs() < 1e-2);
        assert_eq!(frame.player_position, initial_avatar_pos);

        // Pan far beyond world boundary: must clamp to [-150, 150]
        client.pan_camera(300.0, 300.0);
        let clamped_frame = client.update(0.016, 32);
        assert_eq!(clamped_frame.camera_focus.0, 150.0);
        assert_eq!(clamped_frame.camera_focus.2, 150.0);
    }

    /// M15 Proving Ground 3: Point-and-click picking and marquee drag box select units.
    #[test]
    fn test_m15_point_and_marquee_box_selection() {
        let mut client = ClientPresentation::new();
        client.set_camera_mode(CameraMode::Tactical, 0.0);
        client.camera.set_target((0.0, 0.0, 0.0));

        let u1 = EntityId::new(10);
        let u2 = EntityId::new(20);
        let u3 = EntityId::new(30);

        let candidates = vec![
            (u1, (0.0, 0.0, 0.0)),
            (u2, (5.0, 0.0, 5.0)),
            (u3, (50.0, 0.0, 50.0)),
        ];

        // Single point pick at center (0, 0)
        let pick_candidates = vec![
            (u1, (0.0, 0.0, 0.0), 2.0f32),
            (u2, (5.0, 0.0, 5.0), 2.0f32),
            (u3, (50.0, 0.0, 50.0), 2.0f32),
        ];
        let hit = client
            .selection
            .select_point(&client.camera, 0.0, 0.0, &pick_candidates, false);
        assert_eq!(hit, Some(u1));
        assert!(client.selection.contains(u1));
        assert_eq!(client.selection.len(), 1);

        // Marquee box selection around (0, 0) and (5, 5)
        client.selection.start_marquee(-0.3, -0.3);
        client.selection.update_marquee(0.3, 0.3);
        let selected_count = client.select_marquee(&candidates, false);
        assert!(
            selected_count >= 2,
            "Marquee must select at least u1 and u2"
        );
        assert!(client.selection.contains(u1));
        assert!(client.selection.contains(u2));
        assert!(!client.selection.contains(u3));

        // Shift select u3
        client.selection.select_single(u3, true);
        assert_eq!(client.selection.len(), 3);
        assert!(client.selection.contains(u3));
    }

    /// M15 Proving Ground 4: Tactical orders generate valid authoritative server commands.
    #[test]
    fn test_m15_tactical_orders_generate_valid_authoritative_server_commands() {
        use sim_core::test_harness::TestHarness;

        let mut harness = TestHarness::new();
        let f1 = game_types::FactionId::new(1);
        let reg = game_types::RegionId::new(1);

        // Spawn 3 friendly robots
        let r1 = harness
            .state_mut()
            .spawn_robot(
                sim_core::chassis::RobotChassis::Guardsman,
                f1,
                reg,
                (0.0, 0.0, 0.0),
            )
            .unwrap();
        let r2 = harness
            .state_mut()
            .spawn_robot(
                sim_core::chassis::RobotChassis::Guardsman,
                f1,
                reg,
                (2.0, 0.0, 0.0),
            )
            .unwrap();
        let r3 = harness
            .state_mut()
            .spawn_robot(
                sim_core::chassis::RobotChassis::Guardsman,
                f1,
                reg,
                (4.0, 0.0, 0.0),
            )
            .unwrap();

        let mut client = ClientPresentation::new();
        client.selection.select_single(r1, true);
        client.selection.select_single(r2, true);
        client.selection.select_single(r3, true);

        // Issue move order to (40.0, 0.0, 40.0)
        let move_cmds = client.issue_tactical_move((40.0, 0.0, 40.0));
        assert_eq!(move_cmds.len(), 3);

        // Send all commands to authoritative simulation
        for (i, cmd) in move_cmds.into_iter().enumerate() {
            let env = sim_core::command::CommandEnvelope::new(
                game_types::SessionId::new(1),
                i as u64 + 1,
                SimTick::zero(),
                cmd,
            );
            harness.state_mut().command_buffer.push(env);
        }

        // Run simulation ticks to process orders
        harness.run_for_ticks(5);

        // Robots must have accepted orders and begun moving towards destination
        for &id in &[r1, r2, r3] {
            let robot = harness.state().robot_registry.get(id).unwrap();
            assert!(matches!(
                robot.order,
                sim_core::robot::RobotOrder::MoveTo { .. }
            ));
            assert!(
                robot.position.0 > 0.0 || robot.position.2 > 0.0,
                "Robot must be advancing"
            );
        }
    }

    /// M15 Proving Ground 5: Strategic overlays ingest and reflect subsystems in presentation frames.
    #[test]
    fn test_m15_strategic_overlays_ingest_and_reflect_subsystems() {
        let mut client = ClientPresentation::new();
        client.set_camera_mode(CameraMode::Tactical, 0.0);

        // Enable all overlays
        client.overlay_flags.show_power_grid = true;
        client.overlay_flags.show_logistics_network = true;
        client.overlay_flags.show_production_summary = true;

        // Select a unit
        let r1 = EntityId::new(42);
        client.selection.select_single(r1, false);

        let frame = client.update(0.016, 16);

        // Overlays must be populated
        assert!(frame.power_overlay.is_some());
        assert!(frame.logistics_overlay.is_some());
        assert!(frame.production_summary.is_some());
        assert_eq!(frame.selection.len(), 1);

        // HUD summary reflects camera mode and unit selection
        assert!(frame.hud_summary.contains("Tactical"));
        let card = client.hud.render_ascii_card();
        assert!(card.contains("Tactical"));
        assert!(card.contains("Selected Units:          1"));
    }

    /// M15 Proving Ground 6: Same entities continue simulating identically during camera transitions.
    #[test]
    fn test_m15_continuous_simulation_during_camera_transition() {
        use sim_core::command::Command;
        use sim_core::test_harness::TestHarness;

        let mut harness = TestHarness::new();
        let f1 = game_types::FactionId::new(1);
        let reg = game_types::RegionId::new(1);

        let r1 = harness
            .state_mut()
            .spawn_robot(
                sim_core::chassis::RobotChassis::Rifleman,
                f1,
                reg,
                (0.0, 0.0, 0.0),
            )
            .unwrap();

        // Client starts in ThirdPerson and transitions to Strategic over 30 ticks
        let mut client = ClientPresentation::new();
        client.set_camera_mode(CameraMode::Strategic, 1.0); // 1.0s = 30 simulation ticks at 30Hz

        let initial_pos = harness.state().robot_registry.get(r1).unwrap().position;

        // Order robot to move via command buffer
        let env = sim_core::command::CommandEnvelope::new(
            game_types::SessionId::new(1),
            1,
            SimTick::zero(),
            Command::RobotCommand {
                robot_id: r1,
                command_type: sim_core::command::RobotCommandType::Move {
                    position: (50.0, 0.0, 50.0),
                },
            },
        );
        harness.state_mut().command_buffer.push(env);

        // Run 30 ticks of both simulation and presentation
        for t in 1..=30 {
            harness.run_for_ticks(1);
            client.update(1.0 / 30.0, t * 33);
        }

        let final_pos = harness.state().robot_registry.get(r1).unwrap().position;

        // Simulation stepped continuously without being paused, blocked, or altered by camera transition
        assert!(
            final_pos.0 > initial_pos.0 && final_pos.2 > initial_pos.2,
            "Entity must simulate continuously"
        );
        assert!(
            !client.camera.is_transitioning(),
            "Transition must complete at tick 30"
        );
        assert_eq!(client.camera.mode(), CameraMode::Strategic);
    }

    /// M15 Proving Ground 7: Panning camera over unknown terrain does not leak hidden entities.
    #[test]
    fn test_m15_panning_over_unknown_terrain_does_not_leak_hidden_entities() {
        use anti_cheat::detectors::detect_hidden_target_attempt;
        use anti_cheat::event::SecurityEventKind;
        use anti_cheat::provider::InspectionContext;
        use sim_core::world::WorldState;

        let mut world = WorldState::new();
        let f1 = game_types::FactionId::new(1);
        let f2 = game_types::FactionId::new(2);
        let reg = game_types::RegionId::new(1);

        // Friendly robot at (0, 0, 0)
        let _scout = world
            .spawn_robot(
                sim_core::chassis::RobotChassis::Guardsman,
                f1,
                reg,
                (0.0, 0.0, 0.0),
            )
            .unwrap();

        // Enemy robot concealed in shroud at (350.0, 0.0, 350.0)
        let enemy = world
            .spawn_robot(
                sim_core::chassis::RobotChassis::Rifleman,
                f2,
                reg,
                (350.0, 0.0, 350.0),
            )
            .unwrap();

        // Step simulation: enemy is outside Faction 1's sensors and hidden
        world.step_systems();
        assert!(
            !world.faction_knows_entity(f1, enemy),
            "Enemy must be concealed in shroud"
        );

        // Client presentation for Faction 1
        let mut client = ClientPresentation::new();
        client.faction_id = f1;

        // Pan tactical camera directly to (350.0, 0.0, 350.0) over enemy position
        client.set_camera_mode(CameraMode::Tactical, 0.0);
        client.camera.set_target((350.0, 0.0, 350.0));
        let frame = client.update(0.016, 16);

        // Camera focus is positioned right over the enemy
        assert_eq!(frame.camera_focus, (350.0, 0.0, 350.0));

        // Client's remote_entities contains 0 entries for the hidden enemy (replication filtering)
        assert!(!client.remote_entities.contains_key(&enemy));
        assert!(!frame.remote_entities.iter().any(|(id, _)| *id == enemy));

        // Selection query at (350, 0, 350) finds nothing
        let candidates: Vec<(EntityId, (f32, f32, f32))> = frame.remote_entities.clone();
        let count = client.select_marquee(&candidates, false);
        assert_eq!(count, 0);
        assert!(!client.selection.contains(enemy));

        // If client synthesizes an attack command against hidden enemy, anti-cheat catches and rejects it!
        let ctx = InspectionContext::new(
            game_types::SessionId::new(1),
            game_types::PlayerId::new(1),
            f1,
            SimTick::zero(),
            SimTick::zero(),
            1,
            &world,
        );

        let event = detect_hidden_target_attempt(&ctx, enemy);
        assert!(
            matches!(event, Some(SecurityEventKind::HiddenTargetAttempt { target, .. }) if target == enemy),
            "Server anti-cheat must reject attack on hidden entity"
        );
    }
}
