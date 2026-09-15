use crate::packet::{Packet, PacketPayload};
use crate::session::Session;
use crate::snapshot::{EntitySnapshot, SnapshotEnvelope};
use crate::transport::Transport;
use crate::version::{HandshakeMessage, PROTOCOL_VERSION, ProtocolResult};
use game_types::{SessionId, SimTick};
use sim_core::test_harness::TestSimState;
use std::collections::BTreeMap;

/// Headless Authoritative Server owning simulation state and client session network synchronization.
pub struct AuthoritativeServer<T: Transport> {
    pub sim_state: TestSimState,
    transport: T,
    sessions: BTreeMap<SessionId, Session>,
    next_session_id: u64,
    timeout_ticks: u64,
    last_broadcast_seq: u64,
}

impl<T: Transport> AuthoritativeServer<T> {
    pub fn new(transport: T) -> Self {
        AuthoritativeServer {
            sim_state: TestSimState::new(),
            transport,
            sessions: BTreeMap::new(),
            next_session_id: 1,
            timeout_ticks: 150, // 5 seconds at 30 Hz
            last_broadcast_seq: 0,
        }
    }

    pub fn with_sim_state(transport: T, sim_state: TestSimState) -> Self {
        AuthoritativeServer {
            sim_state,
            transport,
            sessions: BTreeMap::new(),
            next_session_id: 1,
            timeout_ticks: 150,
            last_broadcast_seq: 0,
        }
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn active_session_count(&self) -> usize {
        self.sessions
            .values()
            .filter(|s| s.state.is_active())
            .count()
    }

    pub fn get_session(&self, id: SessionId) -> Option<&Session> {
        self.sessions.get(&id)
    }

    pub fn current_tick(&self) -> SimTick {
        self.sim_state.tick
    }

    /// Process all pending network packets and execute a single authoritative simulation tick.
    pub fn step_tick(&mut self) -> ProtocolResult<()> {
        // 1. Process all pending inbound network packets
        while let Some(packet) = self.transport.recv()? {
            self.handle_inbound_packet(packet)?;
        }

        // 2. Advance simulation state and run scheduler
        self.sim_state.tick = self.sim_state.tick.next();

        // Drain and apply buffered commands
        while let Some(envelope) = self.sim_state.command_buffer.pop() {
            match envelope.command {
                sim_core::command::Command::TransferRegion {
                    entity_id,
                    destination_region,
                } => {
                    let _ = self
                        .sim_state
                        .transfer_entity(entity_id, destination_region);
                }
                sim_core::command::Command::BuildStructure {
                    kind,
                    position,
                    rotation_deg,
                } => {
                    let _ = self.sim_state.structure_registry.request_build(
                        sim_core::structure::BuildRequest {
                            player_pos: position,
                            requested_pos: position,
                            kind,
                            rotation_deg,
                            faction_id: game_types::FactionId::new(1),
                            region_id: game_types::RegionId::new(1),
                            creation_tick: self.sim_state.tick,
                            world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                        },
                        None,
                    );
                }
                sim_core::command::Command::DismantleStructure { structure_id } => {
                    let _ = self
                        .sim_state
                        .structure_registry
                        .request_dismantle(structure_id, game_types::FactionId::new(1));
                }
                sim_core::command::Command::RepairStructure {
                    structure_id,
                    actor_entity: Some(actor),
                } => {
                    let _ = self.sim_state.repair_structure(structure_id, actor);
                }
                sim_core::command::Command::TransferResource {
                    from_entity,
                    to_entity,
                    resource_id,
                    amount,
                } => {
                    let _ = self.sim_state.transfer_resources(
                        from_entity,
                        to_entity,
                        resource_id,
                        amount,
                    );
                }
                sim_core::command::Command::ReserveResource {
                    entity,
                    resource_id,
                    amount,
                    reservation_id,
                } => {
                    let _ = self.sim_state.reserve_resources(
                        entity,
                        reservation_id,
                        resource_id,
                        amount,
                        None,
                    );
                }
                sim_core::command::Command::CommitTransfer {
                    reservation_id,
                    from_entity,
                    to_entity,
                } => {
                    let _ = self.sim_state.commit_resource_transfer(
                        reservation_id,
                        from_entity,
                        to_entity,
                    );
                }
                sim_core::command::Command::CancelReservation {
                    reservation_id,
                    from_entity,
                } => {
                    let _ = self
                        .sim_state
                        .cancel_resource_reservation(reservation_id, from_entity);
                }
                sim_core::command::Command::SetProductionRecipe {
                    structure_id,
                    recipe_id,
                } => {
                    let _ = self
                        .sim_state
                        .structure_registry
                        .set_production_recipe(structure_id, recipe_id);
                }
                sim_core::command::Command::SetExtractionTarget {
                    structure_id,
                    deposit_id,
                } => {
                    let _ = self
                        .sim_state
                        .structure_registry
                        .set_extraction_target(structure_id, deposit_id);
                }
                sim_core::command::Command::CreateLogisticsJob {
                    source,
                    destination,
                    resource_id,
                    amount,
                    priority,
                } => {
                    let _ = self.sim_state.structure_registry.logistics.create_job(
                        source,
                        destination,
                        resource_id,
                        amount,
                        sim_core::logistics::JobPriority::from_u8(priority),
                        self.sim_state.tick,
                        &mut self.sim_state.event_journal,
                    );
                }
                sim_core::command::Command::CancelLogisticsJob { job_id } => {
                    let _ = self.sim_state.structure_registry.logistics.cancel_job(
                        job_id,
                        "Server command cancelled",
                        self.sim_state.tick,
                        &mut self.sim_state.inventory_registry,
                        &mut self.sim_state.event_journal,
                    );
                }
                sim_core::command::Command::ClaimLogisticsJob { job_id, worker_id } => {
                    let _ = self.sim_state.structure_registry.logistics.claim_job(
                        job_id,
                        worker_id,
                        self.sim_state.tick,
                        &mut self.sim_state.inventory_registry,
                        &mut self.sim_state.event_journal,
                    );
                }
                sim_core::command::Command::ExecuteLogisticsPickup { job_id, worker_id } => {
                    let _ = self.sim_state.structure_registry.logistics.execute_pickup(
                        job_id,
                        worker_id,
                        self.sim_state.tick,
                        &mut self.sim_state.inventory_registry,
                        &mut self.sim_state.event_journal,
                    );
                }
                sim_core::command::Command::ExecuteLogisticsDropoff { job_id, worker_id } => {
                    let _ = self.sim_state.structure_registry.logistics.execute_dropoff(
                        job_id,
                        worker_id,
                        self.sim_state.tick,
                        &mut self.sim_state.inventory_registry,
                        &mut self.sim_state.event_journal,
                    );
                }
                _ => {}
            }
        }

        self.sim_state
            .structure_registry
            .tick_with_journal(self.sim_state.tick, &mut self.sim_state.event_journal);

        self.sim_state.structure_registry.logistics.step(
            self.sim_state.tick,
            &mut self.sim_state.inventory_registry,
            &mut self.sim_state.event_journal,
        );

        self.sim_state.scheduler.tick(
            self.sim_state.tick,
            &mut self.sim_state.region_map,
            &mut self.sim_state.router,
        );

        // 3. Heartbeat timeout check
        let current_tick = self.sim_state.tick;
        let timeout_ticks = self.timeout_ticks;
        for session in self.sessions.values_mut() {
            if session.is_timed_out(current_tick, timeout_ticks) {
                session.disconnect();
            }
        }

        // 4. Generate snapshot and broadcast to active sessions
        self.broadcast_snapshot()?;

        Ok(())
    }

    fn handle_inbound_packet(&mut self, packet: Packet) -> ProtocolResult<()> {
        match packet.payload {
            PacketPayload::Handshake(handshake) => self.handle_handshake(handshake),
            PacketPayload::Command(envelope) => {
                let session_id = envelope.session_id;
                let sequence = envelope.sequence;

                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.update_heartbeat(self.sim_state.tick);
                    if session.validate_and_advance_sequence(sequence).is_ok() {
                        self.sim_state.add_command(envelope);
                    }
                    // Duplicate or out-of-order sequence is rejected cleanly
                }
                Ok(())
            }
            PacketPayload::Ping { timestamp } => {
                let session_id = packet.header.session_id;
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.update_heartbeat(self.sim_state.tick);
                }
                self.last_broadcast_seq += 1;
                let pong = Packet::new_pong(
                    session_id,
                    self.last_broadcast_seq,
                    timestamp,
                    self.sim_state.tick,
                );
                self.transport.send(pong)?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn handle_handshake(&mut self, handshake: HandshakeMessage) -> ProtocolResult<()> {
        match handshake {
            HandshakeMessage::ClientHello {
                protocol_version,
                client_name,
            } => {
                self.last_broadcast_seq += 1;
                if protocol_version != PROTOCOL_VERSION {
                    // Protocol version mismatch - reject
                    let response = Packet::new_handshake(
                        SessionId::null(),
                        self.last_broadcast_seq,
                        HandshakeMessage::ServerHello {
                            accepted: false,
                            session_id: SessionId::null(),
                            server_tick: self.sim_state.tick,
                            reject_reason: Some(format!(
                                "Incompatible protocol version. Server: {PROTOCOL_VERSION}, Client: {protocol_version}"
                            )),
                        },
                    );
                    self.transport.send(response)?;
                    return Ok(());
                }

                // Accept connection and generate session
                let session_id = SessionId::new(self.next_session_id);
                self.next_session_id += 1;

                let mut session = Session::new(session_id, client_name, self.sim_state.tick);
                session.activate();
                self.sessions.insert(session_id, session);

                let response = Packet::new_handshake(
                    session_id,
                    self.last_broadcast_seq,
                    HandshakeMessage::ServerHello {
                        accepted: true,
                        session_id,
                        server_tick: self.sim_state.tick,
                        reject_reason: None,
                    },
                );
                self.transport.send(response)?;
                Ok(())
            }
            HandshakeMessage::Disconnect { session_id, .. } => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.disconnect();
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    fn broadcast_snapshot(&mut self) -> ProtocolResult<()> {
        if self.sessions.is_empty() {
            return Ok(());
        }

        // Build snapshot from entity registry
        let mut entity_snapshots = Vec::new();
        for entity in self.sim_state.entity_registry.iter() {
            entity_snapshots.push(EntitySnapshot::new(
                entity.id,
                entity.faction_id,
                entity.region_id,
                entity.active,
                entity.flags.0,
            ));
        }

        let snapshot = SnapshotEnvelope::new(self.sim_state.tick, entity_snapshots);

        self.last_broadcast_seq += 1;
        let seq = self.last_broadcast_seq;

        // Send to active sessions
        for (&session_id, session) in &self.sessions {
            if session.state.is_active() {
                let packet = Packet::new_snapshot(session_id, seq, snapshot.clone());
                self.transport.send(packet)?;
            }
        }

        Ok(())
    }
}
