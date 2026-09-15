use game_types::{SessionId, SimTick};
use std::fmt;

/// Current supported protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Protocol handshake messages exchanged during connection establishment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeMessage {
    /// Initial client connection request
    ClientHello {
        protocol_version: u32,
        client_name: String,
    },
    /// Server handshake acceptance or rejection
    ServerHello {
        accepted: bool,
        session_id: SessionId,
        server_tick: SimTick,
        reject_reason: Option<String>,
    },
    /// Explicit connection teardown
    Disconnect {
        session_id: SessionId,
        reason: String,
    },
}

/// Errors occurring at the network protocol and session layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// Client and server protocol versions do not match
    VersionMismatch { expected: u32, actual: u32 },
    /// Operation attempted without an established connection
    NotConnected,
    /// Connection was closed by remote peer or local endpoint
    ConnectionClosed,
    /// Received a duplicate command sequence ID
    DuplicateSequence(u64),
    /// Received an out-of-order sequence ID
    OutOfOrderSequence { expected: u64, actual: u64 },
    /// Session was not found on the server
    SessionNotFound(SessionId),
    /// Session timed out due to missed heartbeats
    SessionTimeout(SessionId),
    /// Serialization or wire decoding error
    SerializationError(String),
    /// Underlying transport IO error
    TransportError(String),
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProtocolError::VersionMismatch { expected, actual } => {
                write!(
                    f,
                    "Protocol version mismatch: server expects {expected}, client has {actual}"
                )
            }
            ProtocolError::NotConnected => write!(f, "Transport is not connected"),
            ProtocolError::ConnectionClosed => write!(f, "Connection closed"),
            ProtocolError::DuplicateSequence(seq) => {
                write!(f, "Duplicate command sequence ignored: {seq}")
            }
            ProtocolError::OutOfOrderSequence { expected, actual } => {
                write!(
                    f,
                    "Out-of-order sequence: expected {expected}, received {actual}"
                )
            }
            ProtocolError::SessionNotFound(id) => write!(f, "Session not found: {id}"),
            ProtocolError::SessionTimeout(id) => write!(f, "Session timed out: {id}"),
            ProtocolError::SerializationError(msg) => {
                write!(f, "Protocol serialization error: {msg}")
            }
            ProtocolError::TransportError(msg) => write!(f, "Transport error: {msg}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

pub type ProtocolResult<T> = Result<T, ProtocolError>;
