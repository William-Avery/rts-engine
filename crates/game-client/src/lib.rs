pub mod avatar;
pub mod camera;
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
pub mod wall_batch;

pub use avatar::*;
pub use camera::*;
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
}
