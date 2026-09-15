use crate::packet::{Packet, PacketPayload};
use crate::session::SessionState;
use crate::snapshot::EntitySnapshot;
use crate::transport::Transport;
use crate::version::{HandshakeMessage, PROTOCOL_VERSION, ProtocolError, ProtocolResult};
use game_types::{EntityId, SessionId, SimTick};
use sim_core::command::{Command, CommandEnvelope};
use std::collections::BTreeMap;

/// Client network endpoint managing session handshake, command sequence emission,
/// and authoritative snapshot replication.
pub struct GameClientNet<T: Transport> {
    transport: T,
    pub state: SessionState,
    pub session_id: SessionId,
    pub client_name: String,
    pub server_tick: SimTick,
    next_sequence: u64,
    pub replicated_entities: BTreeMap<EntityId, EntitySnapshot>,
}

impl<T: Transport> GameClientNet<T> {
    pub fn new(transport: T, client_name: String) -> Self {
        GameClientNet {
            transport,
            state: SessionState::Connecting,
            session_id: SessionId::null(),
            client_name,
            server_tick: SimTick::zero(),
            next_sequence: 1,
            replicated_entities: BTreeMap::new(),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.state.is_connected()
    }

    pub fn is_active(&self) -> bool {
        self.state.is_active()
    }

    /// Initiate connection handshake with the server.
    pub fn connect(&mut self) -> ProtocolResult<()> {
        let packet = Packet::new_handshake(
            SessionId::null(),
            0,
            HandshakeMessage::ClientHello {
                protocol_version: PROTOCOL_VERSION,
                client_name: self.client_name.clone(),
            },
        );
        self.transport.send(packet)?;
        self.state = SessionState::Connecting;
        Ok(())
    }

    /// Initiate connection with an explicit protocol version (useful for testing rejection).
    pub fn connect_with_version(&mut self, version: u32) -> ProtocolResult<()> {
        let packet = Packet::new_handshake(
            SessionId::null(),
            0,
            HandshakeMessage::ClientHello {
                protocol_version: version,
                client_name: self.client_name.clone(),
            },
        );
        self.transport.send(packet)?;
        self.state = SessionState::Connecting;
        Ok(())
    }

    /// Send a gameplay command to the server with a monotonically increasing sequence ID.
    pub fn send_command(&mut self, command: Command) -> ProtocolResult<u64> {
        if !self.state.is_connected() {
            return Err(ProtocolError::NotConnected);
        }

        let seq = self.next_sequence;
        self.next_sequence += 1;

        let envelope = CommandEnvelope::new(self.session_id, seq, self.server_tick, command);
        let packet = Packet::new_command(envelope);
        self.transport.send(packet)?;
        Ok(seq)
    }

    /// Send a raw packet directly over the transport (for testing and low-level protocol interactions).
    pub fn send_raw_packet(&mut self, packet: Packet) -> ProtocolResult<()> {
        self.transport.send(packet)
    }

    /// Poll incoming packets and process server replies (snapshots, handshakes, pongs).
    pub fn poll(&mut self) -> ProtocolResult<()> {
        while let Some(packet) = self.transport.recv()? {
            match packet.payload {
                PacketPayload::Handshake(HandshakeMessage::ServerHello {
                    accepted,
                    session_id,
                    server_tick,
                    reject_reason,
                }) => {
                    if accepted {
                        self.session_id = session_id;
                        self.server_tick = server_tick;
                        self.state = SessionState::Active;
                    } else {
                        self.state = SessionState::Disconnected;
                        return Err(ProtocolError::VersionMismatch {
                            expected: PROTOCOL_VERSION,
                            actual: reject_reason.map(|_| 0).unwrap_or(PROTOCOL_VERSION),
                        });
                    }
                }
                PacketPayload::Snapshot(snapshot) => {
                    self.server_tick = snapshot.server_tick;
                    self.replicated_entities.clear();
                    for e in snapshot.entities {
                        self.replicated_entities.insert(e.id, e);
                    }
                }
                PacketPayload::Delta(delta) => {
                    self.server_tick = delta.target_tick;
                    for e in delta.updated_entities {
                        self.replicated_entities.insert(e.id, e);
                    }
                    for id in delta.removed_entities {
                        self.replicated_entities.remove(&id);
                    }
                }
                PacketPayload::Pong { server_tick, .. } => {
                    self.server_tick = server_tick;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn ping(&mut self, timestamp: u64) -> ProtocolResult<()> {
        let seq = self.next_sequence;
        self.next_sequence += 1;
        let packet = Packet::new_ping(self.session_id, seq, timestamp);
        self.transport.send(packet)
    }

    pub fn disconnect(&mut self) -> ProtocolResult<()> {
        if self.state.is_connected() {
            let seq = self.next_sequence;
            self.next_sequence += 1;
            let packet = Packet::new_handshake(
                self.session_id,
                seq,
                HandshakeMessage::Disconnect {
                    session_id: self.session_id,
                    reason: "Client disconnected".to_string(),
                },
            );
            let _ = self.transport.send(packet);
        }
        self.state = SessionState::Disconnected;
        self.transport.close();
        Ok(())
    }
}
