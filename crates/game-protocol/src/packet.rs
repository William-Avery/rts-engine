use crate::snapshot::{DeltaEnvelope, SnapshotEnvelope};
use crate::version::{HandshakeMessage, PROTOCOL_VERSION};
use game_types::{SessionId, SimTick};
use sim_core::command::CommandEnvelope;

/// Packet header containing versioning, routing, and sequence metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacketHeader {
    pub protocol_version: u32,
    pub session_id: SessionId,
    pub sequence: u64,
    pub packet_type: u8,
}

impl PacketHeader {
    pub const SIZE: usize = 4 + 8 + 8 + 1; // 21 bytes

    pub fn new(
        protocol_version: u32,
        session_id: SessionId,
        sequence: u64,
        packet_type: u8,
    ) -> Self {
        PacketHeader {
            protocol_version,
            session_id,
            sequence,
            packet_type,
        }
    }
}

/// Typed payloads carried inside protocol packets.
#[derive(Debug, Clone, PartialEq)]
pub enum PacketPayload {
    Handshake(HandshakeMessage),
    Command(CommandEnvelope),
    Snapshot(SnapshotEnvelope),
    Delta(DeltaEnvelope),
    Ping {
        timestamp: u64,
    },
    Pong {
        timestamp: u64,
        server_tick: SimTick,
    },
}

/// Network envelope uniting header and payload.
#[derive(Debug, Clone, PartialEq)]
pub struct Packet {
    pub header: PacketHeader,
    pub payload: PacketPayload,
}

impl Packet {
    pub fn new(header: PacketHeader, payload: PacketPayload) -> Self {
        Packet { header, payload }
    }

    pub fn new_handshake(session_id: SessionId, seq: u64, msg: HandshakeMessage) -> Self {
        Packet {
            header: PacketHeader::new(PROTOCOL_VERSION, session_id, seq, 1),
            payload: PacketPayload::Handshake(msg),
        }
    }

    pub fn new_command(envelope: CommandEnvelope) -> Self {
        Packet {
            header: PacketHeader::new(PROTOCOL_VERSION, envelope.session_id, envelope.sequence, 2),
            payload: PacketPayload::Command(envelope),
        }
    }

    pub fn new_snapshot(session_id: SessionId, seq: u64, snapshot: SnapshotEnvelope) -> Self {
        Packet {
            header: PacketHeader::new(PROTOCOL_VERSION, session_id, seq, 3),
            payload: PacketPayload::Snapshot(snapshot),
        }
    }

    pub fn new_delta(session_id: SessionId, seq: u64, delta: DeltaEnvelope) -> Self {
        Packet {
            header: PacketHeader::new(PROTOCOL_VERSION, session_id, seq, 4),
            payload: PacketPayload::Delta(delta),
        }
    }

    pub fn new_ping(session_id: SessionId, seq: u64, timestamp: u64) -> Self {
        Packet {
            header: PacketHeader::new(PROTOCOL_VERSION, session_id, seq, 5),
            payload: PacketPayload::Ping { timestamp },
        }
    }

    pub fn new_pong(session_id: SessionId, seq: u64, timestamp: u64, server_tick: SimTick) -> Self {
        Packet {
            header: PacketHeader::new(PROTOCOL_VERSION, session_id, seq, 6),
            payload: PacketPayload::Pong {
                timestamp,
                server_tick,
            },
        }
    }
}
