use crate::version::{ProtocolError, ProtocolResult};
use game_types::{FactionId, PlayerId, SessionId, SimTick};
use std::net::SocketAddr;

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

/// Why a command packet failed to bind to the session it claimed.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum BindingFailure {
    /// The claimed session does not exist on this server.
    UnknownSession,
    /// The capability token did not match the one the server issued.
    TokenMismatch,
    /// The datagram arrived from an address other than the session's peer.
    PeerAddressMismatch,
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
    /// Internal player identity bound to this session.
    ///
    /// Deliberately separate from any platform identity (Epic/Steam account):
    /// the simulation and the anti-cheat provider only ever see this id.
    pub player_id: PlayerId,
    /// Faction this session is authorised to command.
    pub faction_id: FactionId,
    /// Whether the client has presented a build/protocol/content manifest the
    /// server policy accepted. Official servers gate gameplay commands on this.
    pub manifest_verified: bool,
    /// Datagram source address this session is bound to.
    ///
    /// `None` for a loopback/in-process transport, which has no address and no
    /// third party who could forge one — single-player keeps working without
    /// ceremony. For a real socket the server records the address the handshake
    /// arrived from and refuses command packets from anywhere else.
    pub peer_addr: Option<SocketAddr>,
    /// Server-issued capability token for this session.
    ///
    /// `session_id` travels in the packet body and is trivially forgeable; this
    /// token is random, is returned only to the client that completed the
    /// handshake, and must accompany every command packet.
    pub token: u64,
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
            player_id: PlayerId::null(),
            faction_id: FactionId::null(),
            manifest_verified: false,
            peer_addr: None,
            token: 0,
        }
    }

    /// Bind the internal player and faction identity for this session.
    pub fn with_identity(mut self, player_id: PlayerId, faction_id: FactionId) -> Self {
        self.player_id = player_id;
        self.faction_id = faction_id;
        self
    }

    /// Bind the capability token the server issued and the peer it was issued to.
    pub fn with_binding(mut self, token: u64, peer_addr: Option<SocketAddr>) -> Self {
        self.token = token;
        self.peer_addr = peer_addr;
        self
    }

    /// Verify a command packet actually belongs to this session.
    ///
    /// Both the token and the source address must match. This runs **before**
    /// the sequence check: previously one unauthenticated datagram naming a
    /// real session with `sequence = u64::MAX` latched `last_received_sequence`
    /// and muted that player permanently.
    pub fn verify_binding(
        &self,
        token: u64,
        source: Option<SocketAddr>,
    ) -> Result<(), BindingFailure> {
        if token != self.token {
            return Err(BindingFailure::TokenMismatch);
        }
        match (self.peer_addr, source) {
            // Loopback / in-process: there is no address to compare.
            (None, _) => Ok(()),
            // A bound session whose datagram carries no source (in-process
            // relay in front of a bound session) still has to match the token,
            // which it did to get here.
            (Some(_), None) => Ok(()),
            (Some(bound), Some(actual)) if bound == actual => Ok(()),
            (Some(_), Some(_)) => Err(BindingFailure::PeerAddressMismatch),
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

/// Server-side allocator for session capability tokens.
///
/// Seeded from wall-clock entropy and kept **outside** the simulation: the
/// token stream must be unpredictable, and the simulation's `SimRng` must stay
/// a pure function of its seed. Drawing tokens from `SimRng` would make replays
/// depend on network history and make tokens guessable from a replay.
#[derive(Debug, Clone)]
pub struct TokenIssuer {
    state: u64,
}

impl Default for TokenIssuer {
    fn default() -> Self {
        TokenIssuer::new()
    }
}

impl TokenIssuer {
    pub fn new() -> Self {
        // Wall clock plus a process-lifetime counter, so two issuers created in
        // the same nanosecond (two servers in one process, a test harness) do
        // not share a token stream.
        static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        let nonce = SEQUENCE
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            .wrapping_mul(0xD6E8_FEB8_6659_FD93);
        TokenIssuer {
            // never zero: 0 is the "no token" sentinel
            state: (nanos ^ nonce) | 1,
        }
    }

    /// Construct an issuer with an explicit seed, for reproducible tests.
    pub fn with_seed(seed: u64) -> Self {
        TokenIssuer { state: seed | 1 }
    }

    /// Draw the next token. Never returns 0.
    pub fn next_token(&mut self) -> u64 {
        // SplitMix64: cheap, well-distributed, and not the simulation RNG.
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        if z == 0 { 1 } else { z }
    }
}
