use crate::version::{ProtocolError, ProtocolResult};
use game_types::{SessionId, SimTick};

/// State of a client network session.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum SessionState {
    #[default]
    Connecting,
    Connected,
    Active,
    Disconnected,
}

impl SessionState {
    pub fn is_active(&self) -> bool {
        matches!(self, SessionState::Active)
    }

    pub fn is_connected(&self) -> bool {
        matches!(self, SessionState::Connected | SessionState::Active)
    }

    pub fn is_disconnected(&self) -> bool {
        matches!(self, SessionState::Disconnected)
    }
}

/// Server-side representation of an authenticated client session.
#[derive(Debug, Clone)]
pub struct Session {
    pub session_id: SessionId,
    pub state: SessionState,
    pub client_name: String,
    pub last_received_sequence: u64,
    pub last_heartbeat_tick: SimTick,
    pub created_tick: SimTick,
    pub commands_processed: u64,
    pub commands_rejected: u64,
}

impl Session {
    pub fn new(session_id: SessionId, client_name: String, current_tick: SimTick) -> Self {
        Session {
            session_id,
            state: SessionState::Connected,
            client_name,
            last_received_sequence: 0,
            last_heartbeat_tick: current_tick,
            created_tick: current_tick,
            commands_processed: 0,
            commands_rejected: 0,
        }
    }

    pub fn activate(&mut self) {
        self.state = SessionState::Active;
    }

    pub fn disconnect(&mut self) {
        self.state = SessionState::Disconnected;
    }

    /// Validates an incoming command sequence number. Rejects duplicate and out-of-order packets.
    pub fn validate_and_advance_sequence(&mut self, sequence: u64) -> ProtocolResult<()> {
        if sequence <= self.last_received_sequence {
            self.commands_rejected += 1;
            return Err(ProtocolError::DuplicateSequence(sequence));
        }

        self.last_received_sequence = sequence;
        self.commands_processed += 1;
        Ok(())
    }

    pub fn update_heartbeat(&mut self, current_tick: SimTick) {
        self.last_heartbeat_tick = current_tick;
    }

    pub fn is_timed_out(&self, current_tick: SimTick, timeout_ticks: u64) -> bool {
        current_tick
            .value()
            .saturating_sub(self.last_heartbeat_tick.value())
            > timeout_ticks
    }
}
