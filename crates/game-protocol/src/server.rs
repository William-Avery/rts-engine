//! Single-threaded authoritative server, and the session-layer glue every
//! server shares.
//!
//! The command dispatcher itself lives in `sim_core::dispatch`. Everything here
//! is genuinely session-layer work: binding a packet to the session that owns
//! it, resolving the actor identity, and executing the session directives the
//! simulation authorized. `AuthoritativeServer` and
//! `ThreadedAuthoritativeServer` both call the same free functions below, so
//! there is exactly one copy of each decision.

use crate::packet::{Packet, PacketPayload};
use crate::session::{BindingFailure, Session, TokenIssuer};
use crate::snapshot::{EntitySnapshot, SnapshotEnvelope};
use crate::transport::Transport;
use crate::version::{HandshakeMessage, PROTOCOL_VERSION, ProtocolResult};
use anti_cheat::admin::{AdminRegistry, AdminRole, required_admin_permission};
use anti_cheat::event::{SecurityEvent, SecurityEventKind, SessionBindingRejection};
use anti_cheat::manifest::{BuildManifest, ServerPolicy};
use anti_cheat::null::NullAntiCheat;
use anti_cheat::provider::{AntiCheatProvider, InspectionContext, Verdict};
use anti_cheat::trust::TrustLevel;
use game_types::{FactionId, PlayerId, SessionId, SimTick};
use sim_core::command::{Command, CommandEnvelope};
use sim_core::dispatch::{ActorContext, DEFAULT_PLAYER_REGION, apply_command};
use sim_core::world::{SessionDirective, WorldState};
use std::collections::BTreeMap;
use std::net::SocketAddr;

/// Faction assigned to every connected commander until a lobby/team system
/// exists.
///
/// One constant, not the three spellings (`DEFAULT_PLAYER_FACTION`,
/// `DEFAULT_SESSION_FACTION`, and a `session_faction` field) that used to
/// disagree about where faction ownership lived. Faction now travels on
/// `Session` and reaches the simulation only through [`ActorContext`].
pub const DEFAULT_SESSION_FACTION: FactionId = FactionId::new(1);

/// Outcome of binding an inbound command packet to a session.
pub enum BindingOutcome {
    /// The packet is authentic; act as this session.
    Accepted,
    /// The packet is not authentic. Nothing about the session was mutated.
    Rejected(SecurityEventKind),
}

/// Verify an inbound command packet really belongs to the session it names.
///
/// Runs **before** the heartbeat update and the sequence check, which is the
/// point: a single unauthenticated datagram naming a real session with
/// `sequence = u64::MAX` used to latch that session's `last_received_sequence`
/// and mute the player permanently.
pub fn bind_command_packet(
    sessions: &BTreeMap<SessionId, Session>,
    envelope: &CommandEnvelope,
    source: Option<SocketAddr>,
) -> BindingOutcome {
    let claimed = envelope.session_id;
    let Some(session) = sessions.get(&claimed) else {
        return BindingOutcome::Rejected(SecurityEventKind::SessionBindingMismatch {
            claimed_session: claimed,
            reason: SessionBindingRejection::UnknownSession,
        });
    };
    match session.verify_binding(envelope.token, source) {
        Ok(()) => BindingOutcome::Accepted,
        Err(failure) => BindingOutcome::Rejected(SecurityEventKind::SessionBindingMismatch {
            claimed_session: claimed,
            reason: match failure {
                BindingFailure::UnknownSession => SessionBindingRejection::UnknownSession,
                BindingFailure::TokenMismatch => SessionBindingRejection::TokenMismatch,
                BindingFailure::PeerAddressMismatch => SessionBindingRejection::PeerAddressMismatch,
            },
        }),
    }
}

/// Build the actor identity for a session, resolving (and creating on first
/// use) its authoritative avatar entity.
///
/// Every field comes from server-side state. Nothing is read from the payload.
pub fn actor_for_session(
    world: &mut WorldState,
    sessions: &BTreeMap<SessionId, Session>,
    admin_registry: &AdminRegistry,
    session_id: SessionId,
) -> Option<ActorContext> {
    let session = sessions.get(&session_id)?;
    let role: sim_core::dispatch::AdminRoleCode = admin_registry.role(session_id).into();
    Some(
        ActorContext::new(session_id, session.player_id, session.faction_id)
            .with_admin_role(role)
            .resolve_avatar(world, DEFAULT_PLAYER_REGION),
    )
}

/// Tally of one tick's command application.
#[derive(Clone, Default, Debug, PartialEq)]
pub struct DispatchStats {
    pub applied: u64,
    pub rejected: u64,
    /// Outcome of the most recent research intent this tick, for diagnostics.
    pub last_research_result: Option<game_types::GameResult<()>>,
}

/// Drain the buffered commands and apply each through the one dispatcher.
///
/// Commands are drained FIFO ordered by `(session_id, sequence)` rather than by
/// network arrival, so a contested transfer is decided by the command stream and
/// not by packet jitter.
pub fn apply_buffered_commands(
    world: &mut WorldState,
    sessions: &BTreeMap<SessionId, Session>,
    admin_registry: &AdminRegistry,
) -> DispatchStats {
    let mut stats = DispatchStats::default();
    let envelopes: Vec<CommandEnvelope> = world.command_buffer.drain_ordered().collect();
    for envelope in envelopes {
        let Some(actor) = actor_for_session(world, sessions, admin_registry, envelope.session_id)
        else {
            stats.rejected += 1;
            continue;
        };
        let outcome = apply_command(world, &actor, &envelope.command);
        if matches!(
            envelope.command,
            Command::QueueResearch { .. }
                | Command::CancelResearch { .. }
                | Command::ReorderResearchQueue { .. }
        ) {
            stats.last_research_result = Some(outcome.clone());
        }
        match outcome {
            Ok(()) => stats.applied += 1,
            Err(_rejected) => stats.rejected += 1,
        }
    }
    stats
}

/// Headless Authoritative Server owning simulation state and client session network synchronization.
pub struct AuthoritativeServer<T: Transport> {
    pub sim_state: WorldState,
    transport: T,
    sessions: BTreeMap<SessionId, Session>,
    next_session_id: u64,
    timeout_ticks: u64,
    last_broadcast_seq: u64,
    tokens: TokenIssuer,
    /// Outcome of the most recent research intent, for diagnostics and tests.
    last_research_result: Option<game_types::GameResult<()>>,
    /// The anti-cheat integration boundary. Defaults to
    /// [`NullAntiCheat`], i.e. anti-cheat disabled, so local development
    /// behaves exactly as it did before Milestone 25.
    anti_cheat: Box<dyn AntiCheatProvider>,
    admin_registry: AdminRegistry,
    build_manifest: BuildManifest,
    server_policy: ServerPolicy,
    /// Commands dropped before reaching the simulation, by either the admin
    /// authorization check, the session binding check, or an anti-cheat verdict.
    commands_blocked: u64,
    /// Commands the simulation itself refused, by variant-specific validation.
    commands_rejected: u64,
}

impl<T: Transport> AuthoritativeServer<T> {
    pub fn new(transport: T) -> Self {
        AuthoritativeServer::with_sim_state(transport, WorldState::new())
    }

    pub fn with_sim_state(transport: T, sim_state: WorldState) -> Self {
        AuthoritativeServer {
            sim_state,
            transport,
            sessions: BTreeMap::new(),
            next_session_id: 1,
            timeout_ticks: 150, // 5 seconds at 30 Hz
            last_broadcast_seq: 0,
            tokens: TokenIssuer::new(),
            last_research_result: None,
            anti_cheat: Box::new(NullAntiCheat),
            admin_registry: AdminRegistry::new(),
            build_manifest: BuildManifest::new("rts-engine-dev", PROTOCOL_VERSION, 0),
            server_policy: ServerPolicy::LocalDev,
            commands_blocked: 0,
            commands_rejected: 0,
        }
    }

    /// Install an anti-cheat provider.
    ///
    /// The provider is initialized immediately; a provider that cannot
    /// initialize (for example the EOS/EAC adapter with no linked SDK) is
    /// reported rather than silently accepted.
    pub fn set_anti_cheat(
        &mut self,
        mut provider: Box<dyn AntiCheatProvider>,
    ) -> Result<(), anti_cheat::provider::AntiCheatError> {
        provider.initialize()?;
        self.anti_cheat.shutdown();
        self.anti_cheat = provider;
        Ok(())
    }

    /// Set the build/protocol/content manifest policy this server enforces.
    pub fn set_server_policy(&mut self, policy: ServerPolicy, manifest: BuildManifest) {
        self.server_policy = policy;
        self.build_manifest = manifest;
    }

    pub fn server_policy(&self) -> &ServerPolicy {
        &self.server_policy
    }

    pub fn build_manifest(&self) -> &BuildManifest {
        &self.build_manifest
    }

    /// Name of the installed anti-cheat provider (`"null"` when disabled).
    pub fn anti_cheat_name(&self) -> &'static str {
        self.anti_cheat.name()
    }

    pub fn anti_cheat(&self) -> &dyn AntiCheatProvider {
        self.anti_cheat.as_ref()
    }

    /// Grant an admin role to a session. Server-side only; never client-driven.
    pub fn set_admin_role(&mut self, session_id: SessionId, role: AdminRole) {
        self.admin_registry.set_role(session_id, role);
    }

    pub fn admin_registry(&self) -> &AdminRegistry {
        &self.admin_registry
    }

    /// Commands dropped before reaching the simulation.
    pub fn commands_blocked(&self) -> u64 {
        self.commands_blocked
    }

    /// Commands the simulation refused during dispatch.
    pub fn commands_rejected(&self) -> u64 {
        self.commands_rejected
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Faction the server attributes connected sessions to.
    pub fn session_faction(&self) -> FactionId {
        DEFAULT_SESSION_FACTION
    }

    /// Outcome of the most recently validated research intent, if any.
    pub fn last_research_result(&self) -> Option<&game_types::GameResult<()>> {
        self.last_research_result.as_ref()
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

    /// Mutable access to the underlying transport.
    ///
    /// Exists so a caller can drive a transport that needs configuring (a UDP
    /// remote address, a scripted peer in a security test) without the server
    /// owning transport-specific knowledge.
    pub fn transport_mut(&mut self) -> &mut T {
        &mut self.transport
    }

    pub fn current_tick(&self) -> SimTick {
        self.sim_state.tick
    }

    /// Process all pending network packets and execute a single authoritative simulation tick.
    pub fn step_tick(&mut self) -> ProtocolResult<()> {
        // 1. Process all pending inbound network packets
        while let Some(packet) = self.transport.recv()? {
            let source = self.transport.last_peer_addr();
            self.handle_inbound_packet(packet, source)?;
        }

        // 2. Advance simulation state
        self.sim_state.tick = self.sim_state.tick.next();

        // 3. Apply buffered commands through the single dispatcher.
        let stats =
            apply_buffered_commands(&mut self.sim_state, &self.sessions, &self.admin_registry);
        self.commands_rejected += stats.rejected;
        if stats.last_research_result.is_some() {
            self.last_research_result = stats.last_research_result;
        }

        // 4. Carry out the session-layer actions the simulation authorized.
        for directive in self.sim_state.drain_session_directives() {
            self.execute_session_directive(directive);
        }

        // 5. Step every authoritative subsystem.
        self.sim_state.step_systems();

        // 6. Heartbeat timeout check
        let current_tick = self.sim_state.tick;
        let timeout_ticks = self.timeout_ticks;
        let timed_out: Vec<SessionId> = self
            .sessions
            .values()
            .filter(|s| !s.state.is_disconnected() && s.is_timed_out(current_tick, timeout_ticks))
            .map(|s| s.session_id)
            .collect();
        for session_id in timed_out {
            self.disconnect_session(session_id);
        }

        // 7. Service the anti-cheat provider and apply any pending enforcement.
        self.anti_cheat.poll();
        for (session_id, _reason) in self.anti_cheat.drain_pending_kicks() {
            self.commands_blocked += 1;
            self.disconnect_session(session_id);
        }

        // 8. Generate snapshot and broadcast to active sessions
        self.broadcast_snapshot()?;

        Ok(())
    }

    fn execute_session_directive(&mut self, directive: SessionDirective) {
        match directive {
            SessionDirective::KickSession { target, .. } => self.disconnect_session(target),
            SessionDirective::SetTrustLevel { target, trust_code } => {
                if let Some(level) = TrustLevel::from_code(trust_code) {
                    self.anti_cheat.apply_admin_trust_override(target, level);
                }
            }
            SessionDirective::SetSessionRole { target, role_code } => {
                if let Some(role) = AdminRole::from_code(role_code) {
                    self.admin_registry.set_role(target, role);
                }
            }
        }
    }

    fn handle_inbound_packet(
        &mut self,
        packet: Packet,
        source: Option<SocketAddr>,
    ) -> ProtocolResult<()> {
        match packet.payload {
            PacketPayload::Handshake(handshake) => self.handle_handshake(handshake, source),
            PacketPayload::Command(envelope) => {
                let session_id = envelope.session_id;
                let sequence = envelope.sequence;

                // 0. Session binding. Nothing about the session is touched
                //    until the packet proves it owns it.
                if let BindingOutcome::Rejected(kind) =
                    bind_command_packet(&self.sessions, &envelope, source)
                {
                    self.commands_blocked += 1;
                    self.anti_cheat.report_event(SecurityEvent::new(
                        session_id,
                        PlayerId::null(),
                        self.sim_state.tick,
                        kind,
                    ));
                    return Ok(());
                }

                let Some(session) = self.sessions.get_mut(&session_id) else {
                    return Ok(());
                };
                session.update_heartbeat(self.sim_state.tick);
                if session.validate_and_advance_sequence(sequence).is_err() {
                    // Duplicate or out-of-order sequence is rejected cleanly
                    return Ok(());
                }
                let player_id = session.player_id;
                let faction_id = session.faction_id;
                let manifest_verified = session.manifest_verified;

                // A client manifest submission is handled at ingress; it is a
                // session-layer message and never enters the simulation.
                if let Command::SubmitClientManifest {
                    build_id,
                    protocol_version,
                    content_hash,
                    official_build,
                } = &envelope.command
                {
                    let client_manifest = BuildManifest {
                        build_id: build_id.clone(),
                        protocol_version: *protocol_version,
                        content_hash: *content_hash,
                        official: *official_build,
                    };
                    self.handle_client_manifest(session_id, player_id, &client_manifest);
                    return Ok(());
                }

                // 1. Server-side admin authorization. Independent of anti-cheat:
                //    privileged commands are refused even with anti-cheat off.
                if let Some(permission) = required_admin_permission(&envelope.command)
                    && self
                        .admin_registry
                        .authorize(session_id, permission)
                        .is_err()
                {
                    self.commands_blocked += 1;
                    self.anti_cheat.report_event(SecurityEvent::new(
                        session_id,
                        player_id,
                        self.sim_state.tick,
                        SecurityEventKind::AdminPermissionDenied { permission },
                    ));
                    return Ok(());
                }

                // 2. Official servers require an accepted manifest before any
                //    gameplay command is honoured.
                if self.server_policy.requires_manifest() && !manifest_verified {
                    self.commands_blocked += 1;
                    return Ok(());
                }

                // 3. Anti-cheat inspection. With the null provider this is a
                //    constant-time `Verdict::Allow` and the path below is
                //    byte-for-byte the pre-Milestone-25 behaviour.
                let verdict = {
                    let ctx = InspectionContext::new(
                        session_id,
                        player_id,
                        faction_id,
                        self.sim_state.tick,
                        envelope.client_tick,
                        sequence,
                        &self.sim_state,
                    );
                    self.anti_cheat.inspect_command(&ctx, &envelope.command)
                };

                if verdict.allows_command() {
                    // 4. The simulation performs its own authoritative
                    //    validation regardless of the verdict above.
                    self.sim_state.add_command(envelope);
                } else {
                    self.commands_blocked += 1;
                }

                if matches!(verdict, Verdict::Kick(_)) {
                    self.disconnect_session(session_id);
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

    fn reject_handshake(&mut self, reason: String) -> ProtocolResult<()> {
        let response = Packet::new_handshake(
            SessionId::null(),
            self.last_broadcast_seq,
            HandshakeMessage::ServerHello {
                accepted: false,
                session_id: SessionId::null(),
                server_tick: self.sim_state.tick,
                reject_reason: Some(reason),
                session_token: 0,
            },
        );
        self.transport.send(response)
    }

    fn handle_handshake(
        &mut self,
        handshake: HandshakeMessage,
        source: Option<SocketAddr>,
    ) -> ProtocolResult<()> {
        match handshake {
            HandshakeMessage::ClientHello {
                protocol_version,
                client_name,
            } => {
                self.last_broadcast_seq += 1;
                if protocol_version != PROTOCOL_VERSION {
                    return self.reject_handshake(format!(
                        "Incompatible protocol version. Server: {PROTOCOL_VERSION}, Client: {protocol_version}"
                    ));
                }

                // Accept connection and generate session
                let session_id = SessionId::new(self.next_session_id);
                let player_id = PlayerId::new(self.next_session_id as u32);
                self.next_session_id += 1;

                // Anti-cheat may refuse a connection outright (banned session).
                if self
                    .anti_cheat
                    .on_client_connecting(session_id, &client_name)
                    .is_err()
                    || self
                        .anti_cheat
                        .begin_session(player_id, session_id)
                        .is_err()
                {
                    return self.reject_handshake("Refused by anti-cheat provider".to_string());
                }
                let session_faction = if client_name.eq_ignore_ascii_case("observer")
                    || client_name.eq_ignore_ascii_case("spectator")
                {
                    FactionId::null()
                } else {
                    DEFAULT_SESSION_FACTION
                };
                self.anti_cheat.on_client_authenticated(
                    session_id,
                    player_id,
                    session_faction,
                    self.sim_state.tick,
                );

                let token = self.tokens.next_token();
                let mut session = Session::new(session_id, client_name, self.sim_state.tick)
                    .with_identity(player_id, session_faction)
                    .with_binding(token, source);
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
                        session_token: token,
                    },
                );
                self.transport.send(response)?;
                Ok(())
            }
            HandshakeMessage::Disconnect { session_id, .. } => {
                self.disconnect_session(session_id);
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Disconnect a session and release every piece of per-session state.
    fn disconnect_session(&mut self, session_id: SessionId) {
        if let Some(session) = self.sessions.get_mut(&session_id) {
            session.disconnect();
            self.anti_cheat.end_session(session.player_id);
        }
        self.anti_cheat.end_session_by_id(session_id);
        self.admin_registry.remove_session(session_id);
    }

    /// Validate a submitted client manifest against the server policy.
    fn handle_client_manifest(
        &mut self,
        session_id: SessionId,
        player_id: PlayerId,
        client_manifest: &BuildManifest,
    ) {
        // The provider gets first refusal so it can record telemetry, but the
        // server enforces the policy itself: with anti-cheat disabled an
        // official server still refuses a mismatched manifest.
        let provider_result = self
            .anti_cheat
            .verify_client_manifest(session_id, client_manifest);
        let policy_result = self
            .server_policy
            .validate(&self.build_manifest, client_manifest);

        match (provider_result, policy_result) {
            (Ok(()), Ok(())) => {
                if let Some(session) = self.sessions.get_mut(&session_id) {
                    session.manifest_verified = true;
                }
            }
            (_, policy) => {
                if policy.is_err() {
                    self.anti_cheat.report_event(SecurityEvent::new(
                        session_id,
                        player_id,
                        self.sim_state.tick,
                        SecurityEventKind::ManifestMismatch {
                            expected: self.build_manifest.manifest_hash(),
                            actual: client_manifest.manifest_hash(),
                        },
                    ));
                }
                self.commands_blocked += 1;
                self.disconnect_session(session_id);
            }
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

        let full_snapshot = SnapshotEnvelope::new(self.sim_state.tick, entity_snapshots.clone());

        self.last_broadcast_seq += 1;
        let seq = self.last_broadcast_seq;

        // Send to active sessions filtered by faction knowledge interest
        for (&session_id, session) in &self.sessions {
            if session.state.is_active() {
                let session_snapshot = if session.faction_id.is_null() {
                    full_snapshot.clone()
                } else {
                    let filtered: Vec<EntitySnapshot> = entity_snapshots
                        .iter()
                        .filter(|e| {
                            self.sim_state
                                .faction_knows_entity(session.faction_id, e.id)
                        })
                        .cloned()
                        .collect();
                    SnapshotEnvelope::new(self.sim_state.tick, filtered)
                };
                let packet = Packet::new_snapshot(session_id, seq, session_snapshot);
                self.transport.send(packet)?;
            }
        }

        Ok(())
    }
}
