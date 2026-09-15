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

        // Manually send commands with sequence numbers: 1, 2, 2 (dup), 1 (old), 3
        let cmd = Command::Move {
            position: (10.0, 0.0, 10.0),
            velocity: (1.0, 0.0, 0.0),
        };

        let p1 = Packet::new_command(sim_core::command::CommandEnvelope::new(
            sid,
            1,
            SimTick::zero(),
            cmd.clone(),
        ));
        let p2 = Packet::new_command(sim_core::command::CommandEnvelope::new(
            sid,
            2,
            SimTick::zero(),
            cmd.clone(),
        ));
        let p2_dup = Packet::new_command(sim_core::command::CommandEnvelope::new(
            sid,
            2,
            SimTick::zero(),
            cmd.clone(),
        ));
        let p1_old = Packet::new_command(sim_core::command::CommandEnvelope::new(
            sid,
            1,
            SimTick::zero(),
            cmd.clone(),
        ));
        let p3 = Packet::new_command(sim_core::command::CommandEnvelope::new(
            sid,
            3,
            SimTick::zero(),
            cmd,
        ));

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
            Command::Build {
                position: (12.5, 0.0, -34.5),
                structure_id: game_types::ItemId::new(7),
            },
        );
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
}
