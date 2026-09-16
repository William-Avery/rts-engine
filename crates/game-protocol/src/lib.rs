pub mod client;
pub mod codec;
pub mod packet;
pub mod server;
pub mod session;
pub mod snapshot;
pub mod threaded;
pub mod transport;
pub mod version;

pub use client::*;
pub use codec::*;
pub use packet::*;
pub use server::*;
pub use session::*;
pub use snapshot::*;
pub use threaded::*;
pub use transport::*;
pub use version::*;

#[cfg(test)]
mod tests {
    use super::*;
    use game_types::{FactionId, RegionId, SimTick};
    use sim_core::command::Command;

    #[test]
    fn test_loopback_connection_lifecycle() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        let mut client = GameClientNet::new(client_transport, "Commander_Alpha".to_string());

        assert!(!client.is_active());
        assert_eq!(server.active_session_count(), 0);

        // Initiate connection
        client.connect().unwrap();

        // Server steps tick to process handshake and broadcast snapshot
        server.step_tick().unwrap();
        assert_eq!(server.active_session_count(), 1);

        // Client polls server response
        client.poll().unwrap();
        assert!(client.is_active());
        assert!(!client.session_id.is_null());
        assert_eq!(client.server_tick, server.current_tick());
    }

    #[test]
    fn test_protocol_version_mismatch_rejection() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        let mut client = GameClientNet::new(client_transport, "BadVersionClient".to_string());

        // Connect with incompatible protocol version
        client.connect_with_version(999).unwrap();

        server.step_tick().unwrap();
        assert_eq!(server.active_session_count(), 0);

        let poll_res = client.poll();
        assert!(poll_res.is_err());
        assert!(matches!(
            poll_res.unwrap_err(),
            ProtocolError::VersionMismatch { .. }
        ));
        assert!(client.state.is_disconnected());
    }

    #[test]
    fn test_duplicate_and_out_of_order_command_rejection() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        let mut client = GameClientNet::new(client_transport, "Commander".to_string());

        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();
        assert!(client.is_active());

        let sid = client.session_id;
        let token = client.session_token;

        // Manually send commands with sequence numbers: 1, 2, 2 (dup), 1 (old), 3
        let cmd = Command::Move {
            position: (10.0, 0.0, 10.0),
            velocity: (1.0, 0.0, 0.0),
        };
        let envelope = |seq: u64| {
            Packet::new_command(
                sim_core::command::CommandEnvelope::new(sid, seq, SimTick::zero(), cmd.clone())
                    .with_token(token),
            )
        };

        let p1 = envelope(1);
        let p2 = envelope(2);
        let p2_dup = envelope(2);
        let p1_old = envelope(1);
        let p3 = envelope(3);

        client.send_raw_packet(p1).unwrap();
        client.send_raw_packet(p2).unwrap();
        client.send_raw_packet(p2_dup).unwrap();
        client.send_raw_packet(p1_old).unwrap();
        client.send_raw_packet(p3).unwrap();

        // Server steps tick to process packets
        server.step_tick().unwrap();

        let session = server.get_session(sid).unwrap();
        assert_eq!(
            session.commands_processed, 3,
            "Only sequences 1, 2, 3 should be processed"
        );
        assert_eq!(
            session.commands_rejected, 2,
            "Duplicate and out-of-order should be rejected"
        );
        assert_eq!(session.last_received_sequence, 3);
    }

    #[test]
    fn test_snapshot_replication() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        let mut client = GameClientNet::new(client_transport, "Observer".to_string());

        // Spawn entities on authoritative server
        let e1 = server
            .sim_state
            .create_entity(FactionId::new(1), RegionId::new(1));
        let e2 = server
            .sim_state
            .create_entity(FactionId::new(1), RegionId::new(1));
        let e3 = server
            .sim_state
            .create_entity(FactionId::new(2), RegionId::new(2));

        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();

        // Step tick again so server broadcasts snapshot of entities
        server.step_tick().unwrap();
        client.poll().unwrap();

        assert_eq!(client.replicated_entities.len(), 3);
        assert!(client.replicated_entities.contains_key(&e1));
        assert!(client.replicated_entities.contains_key(&e2));
        assert!(client.replicated_entities.contains_key(&e3));

        assert_eq!(
            client.replicated_entities.get(&e3).unwrap().faction_id,
            FactionId::new(2)
        );
        assert_eq!(
            client.replicated_entities.get(&e3).unwrap().region_id,
            RegionId::new(2)
        );
    }

    #[test]
    fn test_udp_remote_transport() {
        let server_transport = UdpTransport::bind("127.0.0.1:0").unwrap();
        let server_addr = server_transport.local_addr().unwrap();

        let mut client_transport = UdpTransport::bind("127.0.0.1:0").unwrap();
        client_transport
            .connect_to(&server_addr.to_string())
            .unwrap();

        let mut server = AuthoritativeServer::new(server_transport);
        let mut client = GameClientNet::new(client_transport, "UdpPlayer".to_string());

        client.connect().unwrap();

        // Allow packet to transit socket
        std::thread::sleep(std::time::Duration::from_millis(5));
        server.step_tick().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(5));
        client.poll().unwrap();

        assert!(
            client.is_active(),
            "Client should be active over UDP transport"
        );
        assert_eq!(server.active_session_count(), 1);
    }

    #[test]
    fn test_codec_packet_roundtrip() {
        let envelope = sim_core::command::CommandEnvelope::new(
            game_types::SessionId::new(42),
            100,
            SimTick::new(50),
            Command::BuildStructure {
                kind: sim_core::structure::StructureKind::Generator,
                position: (12.5, 0.0, -34.5),
                rotation_deg: 90.0,
            },
        )
        .with_token(0xDEAD_BEEF_CAFE_F00D);
        let original = Packet::new_command(envelope);
        let bytes = encode_packet(&original);
        let decoded = decode_packet(&bytes).unwrap();

        assert_eq!(original.header, decoded.header);
        assert_eq!(original.payload, decoded.payload);

        // Test BuildStructure roundtrip
        let build_cmd = Command::BuildStructure {
            kind: sim_core::structure::StructureKind::Fabricator,
            position: (25.0, 0.0, 75.0),
            rotation_deg: 90.0,
        };
        let p_build = Packet::new_command(sim_core::command::CommandEnvelope::new(
            game_types::SessionId::new(42),
            101,
            SimTick::new(51),
            build_cmd,
        ));
        let bytes_build = encode_packet(&p_build);
        let decoded_build = decode_packet(&bytes_build).unwrap();
        assert_eq!(p_build.payload, decoded_build.payload);

        // Test BuildStructure roundtrip with Wall tiers
        for tier in [
            sim_core::wall::WallTier::Mk1Stone,
            sim_core::wall::WallTier::Mk2Steel,
            sim_core::wall::WallTier::Mk3Composite,
        ] {
            let wall_cmd = Command::BuildStructure {
                kind: sim_core::structure::StructureKind::Wall(tier),
                position: (10.0, 0.0, 20.0),
                rotation_deg: 180.0,
            };
            let p_wall = Packet::new_command(sim_core::command::CommandEnvelope::new(
                game_types::SessionId::new(42),
                101,
                SimTick::new(51),
                wall_cmd,
            ));
            let bytes_wall = encode_packet(&p_wall);
            let decoded_wall = decode_packet(&bytes_wall).unwrap();
            assert_eq!(p_wall.payload, decoded_wall.payload);
        }

        // Test BuildStructure roundtrip with Generator and Battery
        for kind in [
            sim_core::structure::StructureKind::Generator,
            sim_core::structure::StructureKind::Battery,
        ] {
            let power_cmd = Command::BuildStructure {
                kind,
                position: (30.0, 0.0, 40.0),
                rotation_deg: 270.0,
            };
            let p_power = Packet::new_command(sim_core::command::CommandEnvelope::new(
                game_types::SessionId::new(42),
                101,
                SimTick::new(51),
                power_cmd,
            ));
            let bytes_power = encode_packet(&p_power);
            let decoded_power = decode_packet(&bytes_power).unwrap();
            assert_eq!(p_power.payload, decoded_power.payload);
        }

        // Test DismantleStructure roundtrip
        let dis_cmd = Command::DismantleStructure {
            structure_id: game_types::StructureId::new(99),
        };
        let p_dis = Packet::new_command(sim_core::command::CommandEnvelope::new(
            game_types::SessionId::new(42),
            102,
            SimTick::new(52),
            dis_cmd,
        ));
        let bytes_dis = encode_packet(&p_dis);
        let decoded_dis = decode_packet(&bytes_dis).unwrap();
        assert_eq!(p_dis.payload, decoded_dis.payload);

        // Test RepairStructure roundtrip
        let rep_cmd = Command::RepairStructure {
            structure_id: game_types::StructureId::new(123),
            actor_entity: Some(game_types::EntityId::new(456)),
        };
        let p_rep = Packet::new_command(sim_core::command::CommandEnvelope::new(
            game_types::SessionId::new(42),
            103,
            SimTick::new(53),
            rep_cmd,
        ));
        let bytes_rep = encode_packet(&p_rep);
        let decoded_rep = decode_packet(&bytes_rep).unwrap();
        assert_eq!(p_rep.payload, decoded_rep.payload);

        // Test TransferResource roundtrip
        let xfer_cmd = Command::TransferResource {
            from_entity: game_types::EntityId::new(10),
            to_entity: game_types::EntityId::new(20),
            resource_id: game_types::RES_STEEL,
            amount: 75,
        };
        let p_xfer = Packet::new_command(sim_core::command::CommandEnvelope::new(
            game_types::SessionId::new(42),
            103,
            SimTick::new(53),
            xfer_cmd,
        ));
        let bytes_xfer = encode_packet(&p_xfer);
        let decoded_xfer = decode_packet(&bytes_xfer).unwrap();
        assert_eq!(p_xfer.payload, decoded_xfer.payload);

        // Test ReserveResource roundtrip
        let res_cmd = Command::ReserveResource {
            entity: game_types::EntityId::new(10),
            resource_id: game_types::RES_IRON_ORE,
            amount: 50,
            reservation_id: game_types::ReservationId::new(555),
        };
        let p_res = Packet::new_command(sim_core::command::CommandEnvelope::new(
            game_types::SessionId::new(42),
            104,
            SimTick::new(54),
            res_cmd,
        ));
        let bytes_res = encode_packet(&p_res);
        let decoded_res = decode_packet(&bytes_res).unwrap();
        assert_eq!(p_res.payload, decoded_res.payload);

        // Test CommitTransfer and CancelReservation roundtrip
        let commit_cmd = Command::CommitTransfer {
            reservation_id: game_types::ReservationId::new(555),
            from_entity: game_types::EntityId::new(10),
            to_entity: game_types::EntityId::new(20),
        };
        let p_commit = Packet::new_command(sim_core::command::CommandEnvelope::new(
            game_types::SessionId::new(42),
            105,
            SimTick::new(55),
            commit_cmd,
        ));
        let bytes_commit = encode_packet(&p_commit);
        let decoded_commit = decode_packet(&bytes_commit).unwrap();
        assert_eq!(p_commit.payload, decoded_commit.payload);

        let cancel_cmd = Command::CancelReservation {
            reservation_id: game_types::ReservationId::new(555),
            from_entity: game_types::EntityId::new(10),
        };
        let p_cancel = Packet::new_command(sim_core::command::CommandEnvelope::new(
            game_types::SessionId::new(42),
            106,
            SimTick::new(56),
            cancel_cmd,
        ));
        let bytes_cancel = encode_packet(&p_cancel);
        let decoded_cancel = decode_packet(&bytes_cancel).unwrap();
        assert_eq!(p_cancel.payload, decoded_cancel.payload);

        // Test Logistics commands roundtrip
        let log_cmds = vec![
            Command::CreateLogisticsJob {
                source: game_types::EntityId::new(10),
                destination: game_types::EntityId::new(20),
                resource_id: game_types::RES_IRON_ORE,
                amount: 77,
                priority: 2,
            },
            Command::CancelLogisticsJob {
                job_id: game_types::LogisticsJobId::new(888),
            },
            Command::ClaimLogisticsJob {
                job_id: game_types::LogisticsJobId::new(888),
                worker_id: game_types::EntityId::new(999),
            },
            Command::ExecuteLogisticsPickup {
                job_id: game_types::LogisticsJobId::new(888),
                worker_id: game_types::EntityId::new(999),
            },
            Command::ExecuteLogisticsDropoff {
                job_id: game_types::LogisticsJobId::new(888),
                worker_id: game_types::EntityId::new(999),
            },
        ];

        for (idx, cmd) in log_cmds.into_iter().enumerate() {
            let p = Packet::new_command(sim_core::command::CommandEnvelope::new(
                game_types::SessionId::new(42),
                107 + idx as u64,
                SimTick::new(57 + idx as u64),
                cmd,
            ));
            let bytes = encode_packet(&p);
            let decoded = decode_packet(&bytes).unwrap();
            assert_eq!(p.payload, decoded.payload);
        }
    }

    #[test]
    fn test_codec_robot_command_roundtrip() {
        let robot_cmds = vec![
            // Existing robot order variants (Milestone 12 makes these executable).
            Command::RobotCommand {
                robot_id: game_types::EntityId::new(31),
                command_type: sim_core::command::RobotCommandType::Follow {
                    target: game_types::EntityId::new(7),
                },
            },
            Command::RobotCommand {
                robot_id: game_types::EntityId::new(31),
                command_type: sim_core::command::RobotCommandType::Guard {
                    position: (12.5, 0.0, -8.25),
                },
            },
            Command::RobotCommand {
                robot_id: game_types::EntityId::new(31),
                command_type: sim_core::command::RobotCommandType::Attack {
                    target: game_types::EntityId::new(88),
                },
            },
            Command::RobotCommand {
                robot_id: game_types::EntityId::new(31),
                command_type: sim_core::command::RobotCommandType::Move {
                    position: (-40.0, 0.0, 60.0),
                },
            },
            Command::RobotCommand {
                robot_id: game_types::EntityId::new(31),
                command_type: sim_core::command::RobotCommandType::ReturnToBase,
            },
            // Milestone 12 escort and squad commands (codec discriminants 30-34).
            Command::AssignEscort {
                player: game_types::PlayerId::new(5),
                robot_id: game_types::EntityId::new(31),
            },
            Command::ReleaseEscort {
                player: game_types::PlayerId::new(5),
                robot_id: game_types::EntityId::new(31),
            },
            Command::AssignSquadMember {
                squad_id: game_types::SquadId::new(9),
                robot_id: game_types::EntityId::new(31),
            },
            Command::RemoveSquadMember {
                squad_id: game_types::SquadId::new(9),
                robot_id: game_types::EntityId::new(31),
            },
            Command::SquadRegroup {
                squad_id: game_types::SquadId::new(9),
                rally_position: (128.5, 0.0, -256.25),
            },
        ];

        for (idx, cmd) in robot_cmds.into_iter().enumerate() {
            let p = Packet::new_command(sim_core::command::CommandEnvelope::new(
                game_types::SessionId::new(42),
                200 + idx as u64,
                SimTick::new(80 + idx as u64),
                cmd,
            ));
            let bytes = encode_packet(&p);
            let decoded = decode_packet(&bytes).unwrap();
            assert_eq!(p.payload, decoded.payload);
        }
    }

    /// ACCEPTANCE: escort ownership is server-authoritative; a client cannot forge it.
    #[test]
    fn test_server_rejects_forged_escort_assignment_over_the_wire() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        let mut client = GameClientNet::new(client_transport, "Commander".to_string());

        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();
        assert!(client.is_active());

        let own_player = sim_core::robot::player_for_session(client.session_id);
        let victim = game_types::PlayerId::new(own_player.value() + 1);

        // A robot of the server's default faction, plus a victim player who owns nothing.
        let robot = server
            .sim_state
            .spawn_robot(
                sim_core::chassis::RobotChassis::Guardsman,
                FactionId::new(1),
                RegionId::new(1),
                (5.0, 0.0, 0.0),
            )
            .unwrap();
        server
            .sim_state
            .register_player(victim, FactionId::new(1), RegionId::new(1), (0.0, 0.0, 0.0))
            .unwrap();

        // The client forges an assignment naming the victim as the owner.
        client
            .send_command(Command::AssignEscort {
                player: victim,
                robot_id: robot,
            })
            .unwrap();
        server.step_tick().unwrap();

        assert!(
            server
                .sim_state
                .robot_registry
                .escorts_for(victim)
                .is_empty(),
            "server accepted a forged escort assignment"
        );
        assert_eq!(
            server.sim_state.robot_registry.get(robot).unwrap().owner,
            None
        );

        // The same client assigning to itself is accepted.
        client
            .send_command(Command::AssignEscort {
                player: own_player,
                robot_id: robot,
            })
            .unwrap();
        server.step_tick().unwrap();
        assert_eq!(
            server.sim_state.robot_registry.escorts_for(own_player),
            &[robot]
        );

        // And it cannot then be reassigned away by a forged release from the victim.
        client
            .send_command(Command::ReleaseEscort {
                player: victim,
                robot_id: robot,
            })
            .unwrap();
        server.step_tick().unwrap();
        assert_eq!(
            server.sim_state.robot_registry.get(robot).unwrap().owner,
            Some(own_player),
            "server accepted a forged escort release"
        );
    }

    #[test]
    fn test_server_executes_robot_orders_and_owns_final_position() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        let mut client = GameClientNet::new(client_transport, "Commander".to_string());

        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();

        let player = sim_core::robot::player_for_session(client.session_id);
        let robot = server
            .sim_state
            .spawn_robot(
                sim_core::chassis::RobotChassis::Rifleman,
                FactionId::new(1),
                RegionId::new(1),
                (0.0, 0.0, 0.0),
            )
            .unwrap();

        // A previously inert RobotCommandType now produces a real standing order.
        client
            .send_command(Command::RobotCommand {
                robot_id: robot,
                command_type: sim_core::command::RobotCommandType::Move {
                    position: (25.0, 0.0, 0.0),
                },
            })
            .unwrap();
        server.step_tick().unwrap();
        assert_eq!(
            server.sim_state.robot_registry.get(robot).unwrap().order,
            sim_core::robot::RobotOrder::MoveTo {
                position: (25.0, 0.0, 0.0)
            }
        );

        // The robot actually moves under authoritative simulation.
        for _ in 0..200 {
            server.step_tick().unwrap();
        }
        let position = server.sim_state.robot_registry.get(robot).unwrap().position;
        assert!(
            (position.0 - 25.0).abs() < 1.5,
            "robot did not execute the move order: {position:?}"
        );

        // A client-asserted teleport is clamped rather than trusted.
        client
            .send_command(Command::Move {
                position: (50_000.0, 0.0, 0.0),
                velocity: (0.0, 0.0, 0.0),
            })
            .unwrap();
        server.step_tick().unwrap();
        let presence = server.sim_state.robot_registry.player(player).unwrap();
        assert!(
            presence.position.0 < 100.0,
            "server trusted a client teleport: {:?}",
            presence.position
        );
    }

    #[test]
    fn test_worker_pool_parallel_job_execution() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let pool = WorkerPool::new(4);
        assert_eq!(pool.worker_count(), 4);

        let counter = Arc::new(AtomicUsize::new(0));
        for _ in 0..50 {
            let c = Arc::clone(&counter);
            pool.execute(move || {
                c.fetch_add(1, Ordering::SeqCst);
            });
        }

        // Wait briefly for worker tasks to complete
        let start = std::time::Instant::now();
        while counter.load(Ordering::SeqCst) < 50
            && start.elapsed() < std::time::Duration::from_secs(2)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert_eq!(counter.load(Ordering::SeqCst), 50);
    }

    #[test]
    fn test_threaded_authoritative_server_multithreaded_lifecycle() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let (server_tx, server_rx) = server_transport.split();

        let config = ThreadedServerConfig {
            tick_rate_hz: 60,
            worker_threads: 2,
            timeout_ticks: 300,
            ..Default::default()
        };

        let handle = ThreadedAuthoritativeServer::start(server_tx, server_rx, config, None);
        assert!(handle.is_running());
        assert_eq!(handle.worker_count(), 2);

        let mut client = GameClientNet::new(client_transport, "ThreadedCommander".to_string());
        client.connect().unwrap();

        // Wait for connection handshake across threads
        let start = std::time::Instant::now();
        while !client.is_active() && start.elapsed() < std::time::Duration::from_secs(2) {
            let _ = client.poll();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        assert!(
            client.is_active(),
            "Client failed to activate within timeout"
        );
        assert!(!client.session_id.is_null());

        // Send a move command across threads
        client
            .send_command(Command::Move {
                position: (50.0, 0.0, 75.0),
                velocity: (2.0, 0.0, 0.0),
            })
            .unwrap();

        // Allow ticks to process
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = client.poll();

        let metrics = handle.metrics();
        assert!(metrics.total_ticks > 0, "Simulation ticks did not advance");
        assert!(metrics.packets_received > 0, "Server received no packets");
        assert!(handle.current_tick().value() > 0);

        // Graceful stop and join
        handle.join();
    }

    /// Milestone 19: research commands round-trip over the wire on their
    /// reserved discriminants (100-109).
    #[test]
    fn test_codec_research_command_roundtrip() {
        let research_cmds = vec![
            Command::QueueResearch {
                tech_id: game_types::TechId::new(4_000_000_001),
            },
            Command::CancelResearch {
                job_id: game_types::ResearchJobId::new(9_000_000_000_000_000_001),
            },
            Command::ReorderResearchQueue {
                job_id: game_types::ResearchJobId::new(7),
                new_index: 65_535,
            },
        ];

        for (idx, cmd) in research_cmds.into_iter().enumerate() {
            let p = Packet::new_command(sim_core::command::CommandEnvelope::new(
                game_types::SessionId::new(42),
                200 + idx as u64,
                SimTick::new(80 + idx as u64),
                cmd,
            ));
            let bytes = encode_packet(&p);
            let decoded = decode_packet(&bytes).unwrap();
            assert_eq!(p.payload, decoded.payload);
        }

        // The new research facility structure kind also round-trips.
        let build = Packet::new_command(sim_core::command::CommandEnvelope::new(
            game_types::SessionId::new(42),
            210,
            SimTick::new(90),
            Command::BuildStructure {
                kind: sim_core::structure::StructureKind::ResearchFacility,
                position: (-12.5, 0.0, 33.25),
                rotation_deg: 270.0,
            },
        ));
        let bytes = encode_packet(&build);
        assert_eq!(build.payload, decode_packet(&bytes).unwrap().payload);
    }

    /// Milestone 19: the Milestone 1 placeholder `Research` command (discriminant 6)
    /// is retired, so there is exactly one path to queue research.
    #[test]
    fn test_legacy_research_command_discriminant_is_retired() {
        let mut bytes = encode_packet(&Packet::new_command(
            sim_core::command::CommandEnvelope::new(
                game_types::SessionId::new(1),
                1,
                SimTick::new(1),
                Command::QueueResearch {
                    tech_id: game_types::TechId::new(1),
                },
            ),
        ));
        // Overwrite the command discriminant with the retired legacy value.
        let discriminant_index = bytes.len() - 5;
        assert_eq!(bytes[discriminant_index], 100);
        bytes[discriminant_index] = 6;

        let err = decode_packet(&bytes).unwrap_err();
        match err {
            ProtocolError::SerializationError(msg) => {
                assert!(msg.contains("retired"), "unexpected message: {msg}");
                assert!(msg.contains("QueueResearch"), "unexpected message: {msg}");
            }
            other => panic!("expected SerializationError, got {other:?}"),
        }
    }

    /// Milestone 19: the server validates research intent authoritatively. A client
    /// cannot assert that a technology is researched; it can only request one.
    #[test]
    fn test_server_validates_research_intent_authoritatively() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        let mut client = GameClientNet::new(client_transport, "Researcher".to_string());

        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();
        assert!(client.is_active());

        // 1. A technology whose prerequisites are unmet is REJECTED by the server.
        client
            .send_command(Command::QueueResearch {
                tech_id: sim_core::research::TECH_TUNGSTEN_PROCESSING,
            })
            .unwrap();
        server.step_tick().unwrap();
        assert!(matches!(
            server.last_research_result(),
            Some(Err(game_types::GameError::TechPrerequisiteUnmet { .. }))
        ));
        assert!(
            server
                .sim_state
                .research_manager
                .queue(server.session_faction())
                .is_empty()
        );

        // 2. A technology that does not exist at all is REJECTED.
        client
            .send_command(Command::QueueResearch {
                tech_id: game_types::TechId::new(123_456),
            })
            .unwrap();
        server.step_tick().unwrap();
        assert!(matches!(
            server.last_research_result(),
            Some(Err(game_types::GameError::TechNotFound(_)))
        ));

        // 3. A root technology is ACCEPTED and enters the authoritative queue.
        client
            .send_command(Command::QueueResearch {
                tech_id: sim_core::research::TECH_BASIC_METALLURGY,
            })
            .unwrap();
        server.step_tick().unwrap();
        assert!(matches!(server.last_research_result(), Some(Ok(()))));
        assert_eq!(
            server
                .sim_state
                .research_manager
                .queue(server.session_faction())
                .len(),
            1
        );

        // 4. Without a research facility, resources, or power, nothing completes.
        for _ in 0..200 {
            server.step_tick().unwrap();
        }
        assert!(
            !server.sim_state.research_manager.is_completed(
                server.session_faction(),
                sim_core::research::TECH_BASIC_METALLURGY
            ),
            "research completed without an authoritative facility"
        );

        // 5. Cancelling through the command pipeline clears the queue.
        let job_id = server
            .sim_state
            .research_manager
            .queue(server.session_faction())[0]
            .id;
        client
            .send_command(Command::CancelResearch { job_id })
            .unwrap();
        server.step_tick().unwrap();
        assert!(matches!(server.last_research_result(), Some(Ok(()))));
        assert!(
            server
                .sim_state
                .research_manager
                .queue(server.session_faction())
                .is_empty()
        );
    }

    // -----------------------------------------------------------------------
    // Milestone 25 — Basic Anti-Cheat and EAC Integration Boundary
    // -----------------------------------------------------------------------

    use anti_cheat::admin::{AdminRole, required_admin_permission};
    use anti_cheat::basic::BasicAntiCheat;
    use anti_cheat::manifest::{BuildManifest, ServerPolicy};
    use anti_cheat::null::NullAntiCheat;
    use anti_cheat::provider::{AntiCheatProvider, AntiCheatStatus};
    use anti_cheat::trust::TrustLevel;
    use anti_cheat::world_view::{KnowledgeQuery, WorldView};
    use game_types::{EntityId, PlayerId, ResourceId, SessionId};
    use sim_core::inventory::ContainerKind;
    use sim_core::world::WorldState;

    /// Deterministic fingerprint of everything an authoritative tick can change.
    fn sim_fingerprint(sim: &WorldState, tracked: &[EntityId]) -> Vec<String> {
        let mut out = vec![
            format!("tick={}", sim.tick.value()),
            format!("entities={}", sim.entity_registry.count()),
            format!("structures={}", sim.structure_registry.count()),
            format!("events={}", sim.event_journal.len()),
            format!("rng={},{}", sim.rng_state0(), sim.rng_state1()),
            format!("commands_pending={}", sim.command_buffer.len()),
        ];
        for entity in tracked {
            for resource in [ResourceId::new(10), ResourceId::new(1)] {
                out.push(format!(
                    "inv:{}:{}={:?}",
                    entity.value(),
                    resource.value(),
                    sim.inventory(*entity).map(|i| i.total_quantity(resource))
                ));
            }
        }
        out
    }

    /// Two entities of the session's own faction, one stocked with resources.
    fn seeded_sim_state() -> (WorldState, EntityId, EntityId) {
        let mut sim = WorldState::new();
        let src = sim.create_entity(FactionId::new(1), RegionId::new(1));
        let dst = sim.create_entity(FactionId::new(1), RegionId::new(1));
        sim.create_container(src, ContainerKind::Depot);
        sim.create_container(dst, ContainerKind::Depot);
        if let Some(inv) = sim.inventory_mut(src) {
            inv.add(ResourceId::new(10), 500).unwrap();
        }
        // The first session is PlayerId(1). Construction is paid for out of the
        // builder's own container, so the avatar is registered up front and
        // stocked rather than being created empty on first command.
        let avatar = sim
            .register_player(
                PlayerId::new(1),
                FactionId::new(1),
                RegionId::new(1),
                (0.0, 0.0, 0.0),
            )
            .unwrap();
        sim.create_container(avatar, ContainerKind::Backpack);
        if let Some(inv) = sim.inventory_mut(avatar) {
            inv.add(ResourceId::new(10), 100).unwrap();
        }
        (sim, src, dst)
    }

    /// A sequence of entirely legitimate commands for the session's own faction.
    fn legitimate_commands(src: EntityId, dst: EntityId) -> Vec<Command> {
        vec![
            Command::Move {
                position: (0.0, 0.0, 0.0),
                velocity: (2.0, 0.0, 1.0),
            },
            Command::TransferResource {
                from_entity: src,
                to_entity: dst,
                resource_id: ResourceId::new(10),
                amount: 25,
            },
            Command::BuildStructure {
                kind: sim_core::structure::StructureKind::Pylon,
                position: (12.0, 0.0, 8.0),
                rotation_deg: 0.0,
            },
            Command::TransferResource {
                from_entity: src,
                to_entity: dst,
                resource_id: ResourceId::new(10),
                amount: 10,
            },
            Command::Move {
                position: (1.0, 0.0, 1.0),
                velocity: (3.0, 0.0, 0.0),
            },
        ]
    }

    /// Run a command sequence against a server with the given provider and
    /// return the resulting simulation fingerprint.
    fn run_with_provider(
        provider: Box<dyn AntiCheatProvider>,
        commands: &[Command],
        tracked: &[EntityId],
        sim: WorldState,
    ) -> Vec<String> {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::with_sim_state(server_transport, sim);
        server.set_anti_cheat(provider).unwrap();
        let mut client = GameClientNet::new(client_transport, "Commander".to_string());

        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();
        assert!(client.is_active());

        for command in commands {
            client.send_command(command.clone()).unwrap();
            server.step_tick().unwrap();
            let _ = client.poll();
        }
        sim_fingerprint(&server.sim_state, tracked)
    }

    /// Acceptance: anti-cheat can be disabled for local dev without changing
    /// gameplay. The same legitimate command sequence must produce an identical
    /// simulation state under the null and the basic provider.
    #[test]
    fn test_m25_acceptance_null_and_basic_providers_produce_identical_sim_state() {
        let (sim_a, src, dst) = seeded_sim_state();
        let (sim_b, _, _) = seeded_sim_state();
        let commands = legitimate_commands(src, dst);
        let tracked = [src, dst];

        let with_null = run_with_provider(Box::new(NullAntiCheat), &commands, &tracked, sim_a);
        let with_basic =
            run_with_provider(Box::new(BasicAntiCheat::new()), &commands, &tracked, sim_b);

        assert_eq!(
            with_null, with_basic,
            "enabling anti-cheat changed simulation state for legitimate input"
        );
        // Sanity: the sequence actually did something.
        assert!(with_null.iter().any(|f| f.contains("inv:")));
        assert!(with_null.contains(&"structures=1".to_string()));
    }

    /// Acceptance: the server remains authoritative with anti-cheat disabled.
    /// Under the null provider every command is allowed by the provider, and the
    /// server's own validation still rejects the illegal ones.
    #[test]
    fn test_m25_acceptance_server_is_authoritative_with_anti_cheat_disabled() {
        let (sim, src, dst) = seeded_sim_state();
        let enemy_structure = {
            let mut sim = sim;
            let id = sim
                .structure_registry
                .request_build(
                    sim_core::structure::BuildRequest {
                        player_pos: (200.0, 0.0, 200.0),
                        requested_pos: (200.0, 0.0, 200.0),
                        kind: sim_core::structure::StructureKind::Depot,
                        rotation_deg: 0.0,
                        faction_id: FactionId::new(2),
                        region_id: RegionId::new(1),
                        creation_tick: SimTick::zero(),
                        world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                    },
                    None,
                )
                .unwrap();
            (sim, id)
        };
        let (sim, enemy_structure) = enemy_structure;

        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::with_sim_state(server_transport, sim);
        assert_eq!(server.anti_cheat_name(), "null", "anti-cheat must be off");
        let mut client = GameClientNet::new(client_transport, "Cheater".to_string());
        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();

        let before = server
            .sim_state
            .inventory(src)
            .unwrap()
            .total_quantity(ResourceId::new(10));

        // 1. Resource duplication: transfer far more than the balance allows.
        client
            .send_command(Command::TransferResource {
                from_entity: src,
                to_entity: dst,
                resource_id: ResourceId::new(10),
                amount: 10_000_000,
            })
            .unwrap();
        // 2. Unauthorized order: dismantle another faction's structure.
        client
            .send_command(Command::DismantleStructure {
                structure_id: enemy_structure,
            })
            .unwrap();
        server.step_tick().unwrap();
        server.step_tick().unwrap();

        assert_eq!(
            server
                .sim_state
                .inventory(src)
                .unwrap()
                .total_quantity(ResourceId::new(10)),
            before,
            "server-side validation must reject the over-transfer even with anti-cheat off"
        );
        assert_eq!(
            server
                .sim_state
                .inventory(dst)
                .unwrap()
                .total_quantity(ResourceId::new(10)),
            0,
            "no resources may be duplicated into the destination"
        );
        assert!(
            server
                .sim_state
                .structure_registry
                .get(enemy_structure)
                .is_some(),
            "another faction's structure must survive an unauthorized dismantle"
        );
    }

    /// The basic provider additionally blocks unauthorized orders at the server
    /// boundary, before they ever reach the simulation.
    #[test]
    fn test_m25_basic_provider_blocks_unauthorized_order_at_the_server_boundary() {
        let mut sim = WorldState::new();
        let enemy_unit = sim.create_entity(FactionId::new(2), RegionId::new(1));

        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::with_sim_state(server_transport, sim);
        server
            .set_anti_cheat(Box::new(BasicAntiCheat::new()))
            .unwrap();
        let mut client = GameClientNet::new(client_transport, "Cheater".to_string());
        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();

        client
            .send_command(Command::TransferRegion {
                entity_id: enemy_unit,
                destination_region: RegionId::new(2),
            })
            .unwrap();
        server.step_tick().unwrap();

        assert_eq!(server.commands_blocked(), 1);
        let log = server.anti_cheat().security_log().unwrap();
        assert!(
            log.iter().any(|e| e.kind.label() == "unauthorized_order"),
            "expected an unauthorized_order security event"
        );
    }

    /// Conclusive evidence kicks the session, and the kick is performed by the
    /// server rather than the provider.
    #[test]
    fn test_m25_conclusive_evidence_kicks_the_session() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        server
            .set_anti_cheat(Box::new(BasicAntiCheat::new()))
            .unwrap();
        let mut client = GameClientNet::new(client_transport, "Cheater".to_string());
        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();
        let sid = client.session_id;
        assert_eq!(server.active_session_count(), 1);

        client
            .send_command(Command::Move {
                position: (f32::NAN, 0.0, 0.0),
                velocity: (0.0, 0.0, 0.0),
            })
            .unwrap();
        server.step_tick().unwrap();

        assert_eq!(server.active_session_count(), 0, "session must be kicked");
        assert!(server.get_session(sid).unwrap().state.is_disconnected());
    }

    /// Acceptance: admin/host commands are permission-gated server-side, and the
    /// check is independent of which anti-cheat provider is installed.
    #[test]
    fn test_m25_acceptance_admin_commands_are_permission_gated_under_null_provider() {
        let mut sim = WorldState::new();
        let target = sim.create_entity(FactionId::new(1), RegionId::new(1));
        sim.create_container(target, ContainerKind::Depot);

        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::with_sim_state(server_transport, sim);
        assert_eq!(server.anti_cheat_name(), "null");
        let mut client = GameClientNet::new(client_transport, "Player".to_string());
        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();
        let sid = client.session_id;

        let grant = Command::AdminGrantResource {
            target_entity: target,
            resource_id: ResourceId::new(10),
            amount: 9_999,
        };
        assert!(required_admin_permission(&grant).is_some());

        // Unprivileged: refused, nothing granted.
        client.send_command(grant.clone()).unwrap();
        server.step_tick().unwrap();
        server.step_tick().unwrap();
        assert_eq!(
            server
                .sim_state
                .inventory(target)
                .unwrap()
                .total_quantity(ResourceId::new(10)),
            0,
            "an unprivileged session must not be able to grant itself resources"
        );
        assert_eq!(server.admin_registry().denied_count(), 1);

        // Promoted to host: the same command now applies.
        server.set_admin_role(sid, AdminRole::Host);
        client.send_command(grant).unwrap();
        server.step_tick().unwrap();
        server.step_tick().unwrap();
        assert_eq!(
            server
                .sim_state
                .inventory(target)
                .unwrap()
                .total_quantity(ResourceId::new(10)),
            9_999
        );
        assert_eq!(server.admin_registry().granted_count(), 1);
    }

    /// Official servers require an exact manifest match; a modded client is
    /// refused even though the protocol version is compatible.
    #[test]
    fn test_m25_official_policy_rejects_mismatched_client_manifest() {
        let official = BuildManifest::official("rts-engine-ci-42", PROTOCOL_VERSION, 0xfeed);

        for (label, client_manifest, expect_accepted) in [
            ("matching official build", official.clone(), true),
            (
                "modded build",
                BuildManifest::new("rts-engine-modded", PROTOCOL_VERSION, 0xbeef),
                false,
            ),
        ] {
            let (client_transport, server_transport) = LoopbackTransport::create_pair();
            let mut server = AuthoritativeServer::new(server_transport);
            server.set_server_policy(ServerPolicy::Official, official.clone());
            server
                .set_anti_cheat(Box::new(BasicAntiCheat::with_policy(
                    ServerPolicy::Official,
                    official.clone(),
                )))
                .unwrap();
            let mut client = GameClientNet::new(client_transport, "Client".to_string());
            client.connect().unwrap();
            server.step_tick().unwrap();
            client.poll().unwrap();
            let sid = client.session_id;

            client
                .send_command(Command::SubmitClientManifest {
                    build_id: client_manifest.build_id.clone(),
                    protocol_version: client_manifest.protocol_version,
                    content_hash: client_manifest.content_hash,
                    official_build: client_manifest.official,
                })
                .unwrap();
            server.step_tick().unwrap();

            let session = server.get_session(sid).unwrap();
            assert_eq!(
                session.manifest_verified, expect_accepted,
                "{label}: manifest acceptance mismatch"
            );
            assert_eq!(
                !session.state.is_disconnected(),
                expect_accepted,
                "{label}: session disconnection mismatch"
            );
        }
    }

    /// A private server opts into a modded content manifest that an official
    /// server would refuse.
    #[test]
    fn test_m25_private_server_accepts_modded_manifest() {
        let server_manifest = BuildManifest::new("community", PROTOCOL_VERSION, 0x1234);
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        server.set_server_policy(
            ServerPolicy::PrivateCustom {
                accepted_manifest_hash: None,
            },
            server_manifest,
        );
        let mut client = GameClientNet::new(client_transport, "Modder".to_string());
        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();
        let sid = client.session_id;

        client
            .send_command(Command::SubmitClientManifest {
                build_id: "community-with-mods".to_string(),
                protocol_version: PROTOCOL_VERSION,
                content_hash: 0x9999,
                official_build: false,
            })
            .unwrap();
        server.step_tick().unwrap();

        assert!(server.get_session(sid).unwrap().manifest_verified);
        assert!(!server.get_session(sid).unwrap().state.is_disconnected());
    }

    /// Acceptance (partial, see MILESTONE_STATUS remaining debt): hidden enemy
    /// state is not replicated.
    ///
    /// What is assertable today: the replicated `EntitySnapshot` carries no
    /// position, health, inventory, orders or any other actionable detail — only
    /// id, faction, region, active flag and a component mask. Per-faction
    /// visibility filtering of the entity *list* itself is Milestone 14's
    /// sensor/knowledge and replication-interest system; the anti-cheat hook
    /// that consumes it is wired and currently inert, which this test pins.
    #[test]
    fn test_m25_acceptance_hidden_enemy_detail_is_not_replicated_and_m14_hook_is_pending() {
        let mut sim = WorldState::new();
        let enemy = sim.create_entity(FactionId::new(2), RegionId::new(1));
        sim.create_container(enemy, ContainerKind::Depot);
        if let Some(inv) = sim.inventory_mut(enemy) {
            inv.add(ResourceId::new(10), 777).unwrap();
        }

        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::with_sim_state(server_transport, sim);
        let mut client = GameClientNet::new(client_transport, "Scout".to_string());
        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();

        let enemy_snapshot = client
            .replicated_entities
            .get(&enemy)
            .expect("entity existence is replicated today");

        // No position, health, inventory or order state crosses the wire.
        assert_eq!(
            *enemy_snapshot,
            EntitySnapshot::new(enemy, FactionId::new(2), RegionId::new(1), true, 0),
            "EntitySnapshot must expose nothing beyond id/faction/region/active/flags"
        );

        // The Milestone 14 knowledge hook is present and honestly reports that
        // no knowledge system exists yet, so no detector guesses.
        assert_eq!(
            server
                .sim_state
                .faction_knows_entity(FactionId::new(1), enemy),
            KnowledgeQuery::Unavailable,
            "Milestone 14 must override WorldView::faction_knows_entity"
        );
    }

    /// Codec round-trip for every command in the Milestone 25 reserved
    /// discriminant range (160-169).
    #[test]
    fn test_m25_codec_roundtrip_for_security_and_admin_commands() {
        let commands = [
            Command::SubmitClientManifest {
                build_id: "rts-engine-ci-42".to_string(),
                protocol_version: PROTOCOL_VERSION,
                content_hash: 0xdead_beef_cafe_0001,
                official_build: true,
            },
            Command::SubmitClientManifest {
                build_id: String::new(),
                protocol_version: 0,
                content_hash: 0,
                official_build: false,
            },
            Command::AdminKickSession {
                target_session: SessionId::new(7),
                reason_code: 3,
            },
            Command::AdminSetTrustLevel {
                target_session: SessionId::new(8),
                trust_code: TrustLevel::Banned.code(),
            },
            Command::AdminGrantResource {
                target_entity: EntityId::new(9),
                resource_id: ResourceId::new(32),
                amount: 4_294_967_295,
            },
            Command::AdminSetSessionRole {
                target_session: SessionId::new(10),
                role_code: AdminRole::ServerOwner.code(),
            },
        ];

        for command in commands {
            let packet = Packet::new_command(sim_core::command::CommandEnvelope::new(
                SessionId::new(42),
                123,
                SimTick::new(456),
                command.clone(),
            ));
            let bytes = encode_packet(&packet);
            let decoded = decode_packet(&bytes).unwrap();
            assert_eq!(
                packet.payload, decoded.payload,
                "roundtrip failed: {command:?}"
            );
        }
    }

    /// The anti-cheat provider is reachable through the trait only, and the
    /// server surfaces its identity and telemetry without knowing its type.
    #[test]
    fn test_m25_server_talks_to_anti_cheat_only_through_the_trait() {
        let (_client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        assert_eq!(server.anti_cheat_name(), "null");
        assert!(server.anti_cheat().security_log().is_none());
        assert_eq!(
            server.anti_cheat().player_status(PlayerId::new(1)),
            AntiCheatStatus::Unknown
        );

        server
            .set_anti_cheat(Box::new(BasicAntiCheat::new()))
            .unwrap();
        assert_eq!(server.anti_cheat_name(), "basic");
        assert!(server.anti_cheat().security_log().is_some());
        assert_eq!(server.server_policy(), &ServerPolicy::LocalDev);
        assert_eq!(server.build_manifest().protocol_version, PROTOCOL_VERSION);
    }
    // -----------------------------------------------------------------------
    // Phase A — Authority Core
    // -----------------------------------------------------------------------

    use crate::session::TokenIssuer;
    use std::collections::VecDeque;
    use std::net::SocketAddr;

    /// A loopback transport that reports a scripted source address for each
    /// datagram, so the peer-binding check can actually be exercised.
    struct ScriptedPeerTransport {
        inner: LoopbackTransport,
        peers: VecDeque<Option<SocketAddr>>,
        default_peer: Option<SocketAddr>,
        last: Option<SocketAddr>,
    }

    impl ScriptedPeerTransport {
        fn new(inner: LoopbackTransport, default_peer: Option<SocketAddr>) -> Self {
            ScriptedPeerTransport {
                inner,
                peers: VecDeque::new(),
                default_peer,
                last: None,
            }
        }

        /// Queue the source address the next received datagram will appear from.
        fn push_peer(&mut self, peer: Option<SocketAddr>) {
            self.peers.push_back(peer);
        }
    }

    impl Transport for ScriptedPeerTransport {
        fn send(&mut self, packet: Packet) -> crate::version::ProtocolResult<()> {
            self.inner.send(packet)
        }

        fn recv(&mut self) -> crate::version::ProtocolResult<Option<Packet>> {
            let packet = self.inner.recv()?;
            if packet.is_some() {
                self.last = self.peers.pop_front().unwrap_or(self.default_peer);
            }
            Ok(packet)
        }

        fn is_connected(&self) -> bool {
            self.inner.is_connected()
        }

        fn close(&mut self) {
            self.inner.close();
        }

        fn last_peer_addr(&self) -> Option<SocketAddr> {
            self.last
        }
    }

    fn addr(s: &str) -> SocketAddr {
        s.parse().expect("test address parses")
    }

    /// A1 — a command packet naming a real session, sent from a different peer
    /// address, is refused and cannot touch that session's state.
    #[test]
    fn test_a1_forged_session_id_from_wrong_peer_address_is_rejected() {
        let honest_peer = addr("203.0.113.10:40000");
        let attacker_peer = addr("198.51.100.7:40000");

        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::with_sim_state(
            ScriptedPeerTransport::new(server_transport, Some(honest_peer)),
            WorldState::new(),
        );
        let mut client = GameClientNet::new(client_transport, "Commander".to_string());

        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();
        assert!(client.is_active());

        let sid = client.session_id;
        let token = client.session_token;
        assert_ne!(token, 0, "server must issue a non-zero capability token");

        // A legitimate command from the bound peer advances the session.
        client
            .send_command(Command::QueueResearch {
                tech_id: sim_core::research::TECH_BASIC_METALLURGY,
            })
            .unwrap();
        server.step_tick().unwrap();
        let before = server.get_session(sid).unwrap().clone();
        assert_eq!(before.last_received_sequence, 1);
        let blocked_before = server.commands_blocked();

        // The attacker replays the very same envelope, token and all, from a
        // different source address.
        let forged = Packet::new_command(
            sim_core::command::CommandEnvelope::new(
                sid,
                u64::MAX,
                SimTick::zero(),
                Command::QueueResearch {
                    tech_id: sim_core::research::TECH_BASIC_METALLURGY,
                },
            )
            .with_token(token),
        );
        server.transport_mut().push_peer(Some(attacker_peer));
        client.send_raw_packet(forged).unwrap();
        server.step_tick().unwrap();

        let after = server.get_session(sid).unwrap();
        assert_eq!(
            after.last_received_sequence, before.last_received_sequence,
            "a forged packet from the wrong peer latched the sequence counter"
        );
        assert_eq!(after.commands_processed, before.commands_processed);
        assert!(
            server.commands_blocked() > blocked_before,
            "the forged packet was not counted as blocked"
        );

        // And the honest client is not muted: its next command still lands.
        client
            .send_command(Command::QueueResearch {
                tech_id: sim_core::research::TECH_BASIC_METALLURGY,
            })
            .unwrap();
        server.step_tick().unwrap();
        assert_eq!(
            server.get_session(sid).unwrap().commands_processed,
            before.commands_processed + 1,
            "the real player was muted by a forged datagram"
        );
    }

    /// A1 — the capability token, not the client-supplied `session_id`, is what
    /// authorises a command packet.
    #[test]
    fn test_a1_command_without_the_issued_token_is_rejected() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        let mut client = GameClientNet::new(client_transport, "Commander".to_string());

        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();

        let sid = client.session_id;
        let blocked_before = server.commands_blocked();

        // sequence = u64::MAX with no token: the packet that used to mute a
        // player forever by latching `last_received_sequence`.
        let forged = Packet::new_command(sim_core::command::CommandEnvelope::new(
            sid,
            u64::MAX,
            SimTick::zero(),
            Command::Move {
                position: (5.0, 0.0, 5.0),
                velocity: (0.0, 0.0, 0.0),
            },
        ));
        client.send_raw_packet(forged).unwrap();
        server.step_tick().unwrap();

        let session = server.get_session(sid).unwrap();
        assert_eq!(session.last_received_sequence, 0);
        assert_eq!(session.commands_processed, 0);
        assert!(server.commands_blocked() > blocked_before);

        // A token that is merely wrong, rather than absent, is refused too.
        let mut wrong = TokenIssuer::with_seed(7);
        let bad_token = wrong.next_token();
        assert_ne!(bad_token, client.session_token);
        let forged = Packet::new_command(
            sim_core::command::CommandEnvelope::new(
                sid,
                500,
                SimTick::zero(),
                Command::Move {
                    position: (5.0, 0.0, 5.0),
                    velocity: (0.0, 0.0, 0.0),
                },
            )
            .with_token(bad_token),
        );
        client.send_raw_packet(forged).unwrap();
        server.step_tick().unwrap();
        assert_eq!(server.get_session(sid).unwrap().last_received_sequence, 0);
    }

    /// A1 — loopback single-player keeps working without any ceremony: the
    /// client picks the token up from the handshake and stamps it itself.
    #[test]
    fn test_a1_loopback_single_player_still_works_without_ceremony() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        let mut client = GameClientNet::new(client_transport, "SinglePlayer".to_string());

        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();
        assert!(client.is_active());
        assert_eq!(
            server.get_session(client.session_id).unwrap().peer_addr,
            None
        );

        client
            .send_command(Command::QueueResearch {
                tech_id: sim_core::research::TECH_BASIC_METALLURGY,
            })
            .unwrap();
        server.step_tick().unwrap();
        assert_eq!(
            server
                .sim_state
                .research_manager
                .queue(server.session_faction())
                .len(),
            1
        );
    }

    /// A2 — the depot-theft chain, driven over the real wire.
    ///
    /// `CreateLogisticsJob{source: <enemy depot>, destination: <mine>}` then
    /// claim, pickup and dropoff used to debit an enemy depot and credit the
    /// attacker through the authoritative path.
    #[test]
    fn test_a2_depot_theft_over_the_wire_is_rejected() {
        use sim_core::inventory::ContainerKind;

        let ours = FactionId::new(1);
        let theirs = FactionId::new(2);

        let mut sim = WorldState::new();
        let enemy_depot = sim.create_entity(theirs, RegionId::new(1));
        sim.create_container(enemy_depot, ContainerKind::Depot);
        sim.inventory_mut(enemy_depot)
            .unwrap()
            .add(game_types::RES_STEEL, 400)
            .unwrap();
        let my_depot = sim.create_entity(ours, RegionId::new(1));
        sim.create_container(my_depot, ContainerKind::Depot);
        let hauler = sim.create_entity(ours, RegionId::new(1));
        sim.create_container(hauler, ContainerKind::CargoBuffer);

        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::with_sim_state(server_transport, sim);
        let mut client = GameClientNet::new(client_transport, "Raider".to_string());
        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();
        assert!(client.is_active());

        let job_id = game_types::LogisticsJobId::new(1);
        for command in [
            Command::CreateLogisticsJob {
                source: enemy_depot,
                destination: my_depot,
                resource_id: game_types::RES_STEEL,
                amount: 400,
                priority: 2,
            },
            Command::ClaimLogisticsJob {
                job_id,
                worker_id: hauler,
            },
            Command::ExecuteLogisticsPickup {
                job_id,
                worker_id: hauler,
            },
            Command::ExecuteLogisticsDropoff {
                job_id,
                worker_id: hauler,
            },
        ] {
            client.send_command(command).unwrap();
            server.step_tick().unwrap();
        }

        assert!(
            server
                .sim_state
                .structure_registry
                .logistics
                .jobs
                .is_empty(),
            "a theft job reached the authoritative logistics manager"
        );
        assert_eq!(
            server
                .sim_state
                .inventory(enemy_depot)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            400,
            "the enemy depot was debited"
        );
        assert_eq!(
            server
                .sim_state
                .inventory(my_depot)
                .unwrap()
                .total_quantity(game_types::RES_STEEL),
            0,
            "the attacker was credited"
        );
        assert!(
            server.commands_rejected() >= 4,
            "the four theft commands were not all refused"
        );
    }

    /// A3 — the variants the three old dispatchers decoded, buffered and then
    /// silently discarded now produce a real outcome or a typed refusal.
    #[test]
    fn test_a3_previously_discarded_variants_reach_the_single_dispatcher() {
        let (client_transport, server_transport) = LoopbackTransport::create_pair();
        let mut server = AuthoritativeServer::new(server_transport);
        let mut client = GameClientNet::new(client_transport, "Commander".to_string());
        client.connect().unwrap();
        server.step_tick().unwrap();
        client.poll().unwrap();

        let before = server.commands_rejected();

        // `Action` and `RequestResource` have no authoritative system behind
        // them yet, so the server refuses them rather than dropping them.
        client
            .send_command(Command::Action {
                action_type: sim_core::command::ActionType::FireWeapon,
                target: None,
            })
            .unwrap();
        server.step_tick().unwrap();
        client
            .send_command(Command::RequestResource {
                resource_id: game_types::RES_STEEL,
                amount: 5,
            })
            .unwrap();
        server.step_tick().unwrap();
        assert_eq!(
            server.commands_rejected(),
            before + 2,
            "discarded variants were not counted as refusals"
        );

        // `Move` is now a real, clamped authoritative update.
        let player = sim_core::robot::player_for_session(client.session_id);
        client
            .send_command(Command::Move {
                position: (0.25, 0.0, 0.0),
                velocity: (1.0, 0.0, 0.0),
            })
            .unwrap();
        server.step_tick().unwrap();
        let presence = server.sim_state.robot_registry.player(player).unwrap();
        assert!(
            presence.position.0 > 0.0,
            "a legal move was not applied: {:?}",
            presence.position
        );
    }
}
