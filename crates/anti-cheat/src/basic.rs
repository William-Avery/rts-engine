use crate::detectors::{self, SessionTelemetry};
use crate::event::{SecurityEvent, SecurityEventKind, SecurityLog, SecuritySeverity};
use crate::manifest::{BuildManifest, ServerPolicy};
use crate::provider::{
    AntiCheatError, AntiCheatProvider, AntiCheatResult, AntiCheatStatus, EnforcementReason,
    InspectionContext, Verdict,
};
use crate::trust::{ClientTrustState, TrustLevel, TrustPolicy};
use game_types::{FactionId, PlayerId, SessionId, SimTick};
use sim_core::command::Command;
use std::collections::BTreeMap;

/// Per-session state owned by [`BasicAntiCheat`].
#[derive(Clone, Debug)]
pub struct SessionSecurityState {
    pub trust: ClientTrustState,
    pub telemetry: SessionTelemetry,
    pub status: AntiCheatStatus,
    pub manifest_verified: bool,
    /// Total commands inspected for this session.
    pub commands_inspected: u64,
    /// Commands the provider rejected or kicked on.
    pub commands_blocked: u64,
}

impl SessionSecurityState {
    fn new(session: SessionId, player: PlayerId, faction: FactionId, tick: SimTick) -> Self {
        SessionSecurityState {
            trust: ClientTrustState::new(session, player, faction, tick),
            telemetry: SessionTelemetry::new(tick),
            status: AntiCheatStatus::Clean,
            manifest_verified: false,
            commands_inspected: 0,
            commands_blocked: 0,
        }
    }
}

/// The internal heuristic anti-cheat provider.
///
/// Runs entirely on authoritative server state with **no proprietary SDK and no
/// external service**: every detector in [`crate::detectors`] compares the
/// client's claim against facts the server already owns. That makes it usable
/// on any dedicated server, including community-hosted ones, and makes it the
/// baseline that a future EAC-backed provider layers on top of rather than
/// replaces.
///
/// Enforcement philosophy, following the master spec: *do not instant-ban from
/// weak heuristic evidence*. Low and medium severity findings only record
/// telemetry and let the command through to the server's own validation. High
/// severity findings drop the command. Only conclusive (`Critical`) evidence, or
/// sustained accumulation past the ban threshold, produces a kick.
pub struct BasicAntiCheat {
    initialized: bool,
    policy: ServerPolicy,
    server_manifest: BuildManifest,
    trust_policy: TrustPolicy,
    sessions: BTreeMap<SessionId, SessionSecurityState>,
    player_index: BTreeMap<PlayerId, SessionId>,
    log: SecurityLog,
    pending_kicks: Vec<(SessionId, EnforcementReason)>,
    polls: u64,
}

impl Default for BasicAntiCheat {
    fn default() -> Self {
        BasicAntiCheat::new()
    }
}

impl BasicAntiCheat {
    /// Provider with a local-dev policy and an anonymous server manifest.
    pub fn new() -> Self {
        BasicAntiCheat::with_policy(
            ServerPolicy::LocalDev,
            BuildManifest::new("rts-engine-dev", 1, 0),
        )
    }

    /// Provider bound to a server policy and the server's own manifest.
    pub fn with_policy(policy: ServerPolicy, server_manifest: BuildManifest) -> Self {
        BasicAntiCheat {
            initialized: false,
            policy,
            server_manifest,
            trust_policy: TrustPolicy::default(),
            sessions: BTreeMap::new(),
            player_index: BTreeMap::new(),
            log: SecurityLog::default(),
            pending_kicks: Vec::new(),
            polls: 0,
        }
    }

    /// Override the trust thresholds (for example
    /// [`TrustPolicy::telemetry_only`] on a community server).
    pub fn with_trust_policy(mut self, trust_policy: TrustPolicy) -> Self {
        self.trust_policy = trust_policy;
        self
    }

    pub fn policy(&self) -> &ServerPolicy {
        &self.policy
    }

    pub fn server_manifest(&self) -> &BuildManifest {
        &self.server_manifest
    }

    pub fn trust_policy(&self) -> &TrustPolicy {
        &self.trust_policy
    }

    /// Number of times [`AntiCheatProvider::poll`] has run.
    pub fn poll_count(&self) -> u64 {
        self.polls
    }

    pub fn session_state(&self, session: SessionId) -> Option<&SessionSecurityState> {
        self.sessions.get(&session)
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }

    /// Whether the session has presented an accepted manifest.
    pub fn is_manifest_verified(&self, session: SessionId) -> bool {
        self.sessions
            .get(&session)
            .is_some_and(|s| s.manifest_verified)
    }

    fn record(
        &mut self,
        session: SessionId,
        player: PlayerId,
        tick: SimTick,
        kind: SecurityEventKind,
    ) {
        self.log
            .append(SecurityEvent::new(session, player, tick, kind));
    }

    /// Apply a finding to a session's trust state, logging the event and any
    /// resulting trust transition.
    fn apply_finding(
        &mut self,
        session: SessionId,
        player: PlayerId,
        tick: SimTick,
        kind: SecurityEventKind,
    ) -> SecuritySeverity {
        let severity = kind.severity();
        self.record(session, player, tick, kind);

        let trust_policy = self.trust_policy;
        let transition = self
            .sessions
            .get_mut(&session)
            .and_then(|state| state.trust.record_violation(severity, tick, &trust_policy));

        if let Some(transition) = transition {
            self.record(
                session,
                player,
                tick,
                SecurityEventKind::TrustLevelChanged {
                    from: transition.from,
                    to: transition.to,
                },
            );
            if let Some(state) = self.sessions.get_mut(&session) {
                state.status = AntiCheatStatus::from_trust(transition.to);
            }
            if transition.to == TrustLevel::Banned {
                self.pending_kicks
                    .push((session, EnforcementReason::TrustLevelBanned));
            }
        }
        severity
    }
}

/// Map an anomaly class onto the enforcement reason reported to the server.
fn enforcement_reason(kind: &SecurityEventKind) -> EnforcementReason {
    match kind {
        SecurityEventKind::ImpossibleMovement { .. }
        | SecurityEventKind::TeleportAttempt { .. }
        | SecurityEventKind::NonFinitePosition
        | SecurityEventKind::PositionOutOfBounds { .. } => EnforcementReason::ImpossibleMovement,
        SecurityEventKind::FireRateViolation { .. } => EnforcementReason::FireRateViolation,
        SecurityEventKind::AmmoInconsistency { .. } => EnforcementReason::AmmoInconsistency,
        SecurityEventKind::InvalidPlacement { .. } => EnforcementReason::InvalidPlacement,
        SecurityEventKind::ImpossibleEconomyDelta { .. } => {
            EnforcementReason::ImpossibleEconomyDelta
        }
        SecurityEventKind::UnauthorizedOrder { .. }
        | SecurityEventKind::UnauthorizedStructureOrder { .. } => {
            EnforcementReason::UnauthorizedOrder
        }
        SecurityEventKind::HiddenTargetAttempt { .. } => EnforcementReason::HiddenTargetAttempt,
        SecurityEventKind::FutureDatedCommand { .. } => EnforcementReason::MalformedEnvelope,
        SecurityEventKind::CommandFloodDetected { .. } => EnforcementReason::CommandFlood,
        SecurityEventKind::ManifestMismatch { .. } => EnforcementReason::ManifestMismatch,
        SecurityEventKind::AdminPermissionDenied { .. } => EnforcementReason::AdminPermissionDenied,
        SecurityEventKind::SessionBindingMismatch { .. } => {
            EnforcementReason::SessionBindingMismatch
        }
        SecurityEventKind::TrustLevelChanged { .. } => EnforcementReason::TrustLevelBanned,
    }
}

impl AntiCheatProvider for BasicAntiCheat {
    fn name(&self) -> &'static str {
        "basic"
    }

    fn initialize(&mut self) -> AntiCheatResult<()> {
        if self.initialized {
            return Err(AntiCheatError::AlreadyInitialized);
        }
        self.initialized = true;
        Ok(())
    }

    fn shutdown(&mut self) {
        self.initialized = false;
        self.sessions.clear();
        self.player_index.clear();
        self.pending_kicks.clear();
    }

    fn on_client_connecting(
        &mut self,
        session: SessionId,
        _client_name: &str,
    ) -> AntiCheatResult<()> {
        if !self.initialized {
            return Err(AntiCheatError::NotInitialized);
        }
        if self
            .sessions
            .get(&session)
            .is_some_and(|s| s.trust.level == TrustLevel::Banned)
        {
            return Err(AntiCheatError::SessionBanned(session));
        }
        Ok(())
    }

    fn begin_session(&mut self, player: PlayerId, session: SessionId) -> AntiCheatResult<()> {
        if !self.initialized {
            return Err(AntiCheatError::NotInitialized);
        }
        if self.sessions.contains_key(&session) {
            return Err(AntiCheatError::SessionAlreadyRegistered(session));
        }
        self.sessions.insert(
            session,
            SessionSecurityState::new(session, player, FactionId::null(), SimTick::zero()),
        );
        self.player_index.insert(player, session);
        Ok(())
    }

    fn verify_client_manifest(
        &mut self,
        session: SessionId,
        manifest: &BuildManifest,
    ) -> AntiCheatResult<()> {
        if !self.initialized {
            return Err(AntiCheatError::NotInitialized);
        }
        let player = self
            .sessions
            .get(&session)
            .map(|s| s.trust.player_id)
            .ok_or(AntiCheatError::SessionNotRegistered(session))?;
        let tick = self
            .sessions
            .get(&session)
            .map(|s| s.trust.session_start_tick)
            .unwrap_or_default();

        match self.policy.validate(&self.server_manifest, manifest) {
            Ok(()) => {
                if let Some(state) = self.sessions.get_mut(&session) {
                    state.manifest_verified = true;
                }
                Ok(())
            }
            Err(err) => {
                self.apply_finding(
                    session,
                    player,
                    tick,
                    SecurityEventKind::ManifestMismatch {
                        expected: self.server_manifest.manifest_hash(),
                        actual: manifest.manifest_hash(),
                    },
                );
                self.pending_kicks
                    .push((session, EnforcementReason::ManifestMismatch));
                Err(AntiCheatError::ManifestRejected(err))
            }
        }
    }

    fn on_client_authenticated(
        &mut self,
        session: SessionId,
        player: PlayerId,
        faction: FactionId,
        tick: SimTick,
    ) {
        let entry = self
            .sessions
            .entry(session)
            .or_insert_with(|| SessionSecurityState::new(session, player, faction, tick));
        entry.trust.player_id = player;
        entry.trust.faction_id = faction;
        entry.trust.session_start_tick = tick;
        entry.telemetry = SessionTelemetry::new(tick);
        self.player_index.insert(player, session);
    }

    fn end_session(&mut self, player: PlayerId) {
        if let Some(session) = self.player_index.remove(&player) {
            self.sessions.remove(&session);
            self.pending_kicks.retain(|(s, _)| *s != session);
        }
    }

    fn end_session_by_id(&mut self, session: SessionId) {
        if let Some(state) = self.sessions.remove(&session) {
            self.player_index.remove(&state.trust.player_id);
        }
        self.pending_kicks.retain(|(s, _)| *s != session);
    }

    fn poll(&mut self) {
        // No external SDK to pump; the counter makes the call observable in tests
        // and gives an SDK-backed provider an obvious place to drain callbacks.
        self.polls = self.polls.saturating_add(1);
    }

    fn inspect_command(&mut self, ctx: &InspectionContext<'_>, command: &Command) -> Verdict {
        let session = ctx.session_id;
        let Some(state) = self.sessions.get_mut(&session) else {
            // Unregistered sessions are the server's problem, not ours: it will
            // reject the command anyway. Never guess about an unknown session.
            return Verdict::Allow;
        };
        state.commands_inspected = state.commands_inspected.saturating_add(1);

        if state.trust.level == TrustLevel::Banned {
            state.commands_blocked = state.commands_blocked.saturating_add(1);
            return Verdict::Kick(EnforcementReason::TrustLevelBanned);
        }

        let mut telemetry = std::mem::take(&mut state.telemetry);
        let findings = detectors::inspect(ctx, &mut telemetry, command);
        if let Some(state) = self.sessions.get_mut(&session) {
            state.telemetry = telemetry;
        }

        if findings.is_empty() {
            let trust_policy = self.trust_policy;
            let transition = self
                .sessions
                .get_mut(&session)
                .and_then(|state| state.trust.record_clean_command(&trust_policy));
            if let Some(transition) = transition {
                self.record(
                    session,
                    ctx.player_id,
                    ctx.server_tick,
                    SecurityEventKind::TrustLevelChanged {
                        from: transition.from,
                        to: transition.to,
                    },
                );
                if let Some(state) = self.sessions.get_mut(&session) {
                    state.status = AntiCheatStatus::from_trust(transition.to);
                }
            }
            return Verdict::Allow;
        }

        let mut worst = SecuritySeverity::Info;
        let mut reason = EnforcementReason::MalformedEnvelope;
        for kind in findings {
            let kind_reason = enforcement_reason(&kind);
            let severity = self.apply_finding(session, ctx.player_id, ctx.server_tick, kind);
            if severity > worst {
                worst = severity;
                reason = kind_reason;
            }
        }

        let banned = self
            .sessions
            .get(&session)
            .is_some_and(|s| s.trust.level == TrustLevel::Banned);

        let verdict = if banned {
            Verdict::Kick(reason)
        } else if worst >= SecuritySeverity::High {
            Verdict::Reject(reason)
        } else {
            // Weak evidence: telemetry only. The command still faces the
            // server's own authoritative validation.
            Verdict::Observe(reason)
        };

        if !verdict.allows_command()
            && let Some(state) = self.sessions.get_mut(&session)
        {
            state.commands_blocked = state.commands_blocked.saturating_add(1);
        }
        verdict
    }

    fn report_event(&mut self, event: SecurityEvent) {
        let session = event.session_id;
        let player = event.player_id;
        let tick = event.server_tick;
        self.apply_finding(session, player, tick, event.kind);
    }

    fn player_status(&self, player: PlayerId) -> AntiCheatStatus {
        self.player_index
            .get(&player)
            .and_then(|session| self.sessions.get(session))
            .map(|state| state.status)
            .unwrap_or_default()
    }

    fn session_trust(&self, session: SessionId) -> TrustLevel {
        self.sessions
            .get(&session)
            .map(|state| state.trust.level)
            .unwrap_or_default()
    }

    fn drain_pending_kicks(&mut self) -> Vec<(SessionId, EnforcementReason)> {
        std::mem::take(&mut self.pending_kicks)
    }

    fn apply_admin_trust_override(&mut self, session: SessionId, level: TrustLevel) {
        let Some(state) = self.sessions.get_mut(&session) else {
            return;
        };
        let player = state.trust.player_id;
        let tick = state.trust.session_start_tick;
        if let Some(transition) = state.trust.force_level(level) {
            state.status = AntiCheatStatus::from_trust(transition.to);
            self.record(
                session,
                player,
                tick,
                SecurityEventKind::TrustLevelChanged {
                    from: transition.from,
                    to: transition.to,
                },
            );
            if transition.to == TrustLevel::Banned {
                self.pending_kicks
                    .push((session, EnforcementReason::TrustLevelBanned));
            }
        }
    }

    fn security_log(&self) -> Option<&SecurityLog> {
        Some(&self.log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_view::StaticWorldView;
    use game_types::{EntityId, ResourceId};

    const SESSION: SessionId = SessionId::new(1);
    const PLAYER: PlayerId = PlayerId::new(1);
    const OWN: FactionId = FactionId::new(1);
    const ENEMY: FactionId = FactionId::new(2);

    fn provider() -> BasicAntiCheat {
        let mut p = BasicAntiCheat::new();
        p.initialize().unwrap();
        p.on_client_connecting(SESSION, "tester").unwrap();
        p.begin_session(PLAYER, SESSION).unwrap();
        p.on_client_authenticated(SESSION, PLAYER, OWN, SimTick::zero());
        p
    }

    fn ctx_at<'a>(world: &'a StaticWorldView, tick: u64) -> InspectionContext<'a> {
        InspectionContext::new(
            SESSION,
            PLAYER,
            OWN,
            SimTick::new(tick),
            SimTick::new(tick),
            tick,
            world,
        )
    }

    #[test]
    fn test_basic_provider_runs_with_no_proprietary_sdk() {
        // No feature flags, no external service, no FFI: construct, initialize
        // and inspect entirely in-process.
        let mut p = BasicAntiCheat::new();
        assert_eq!(p.name(), "basic");
        assert!(p.initialize().is_ok());
        p.on_client_connecting(SESSION, "solo").unwrap();
        p.begin_session(PLAYER, SESSION).unwrap();
        p.on_client_authenticated(SESSION, PLAYER, OWN, SimTick::zero());
        let world = StaticWorldView::new();
        let verdict = p.inspect_command(
            &ctx_at(&world, 1),
            &Command::Move {
                position: (0.0, 0.0, 0.0),
                velocity: (1.0, 0.0, 0.0),
            },
        );
        assert_eq!(verdict, Verdict::Allow);
        assert_eq!(p.player_status(PLAYER), AntiCheatStatus::Clean);
    }

    #[test]
    fn test_double_initialize_is_rejected() {
        let mut p = BasicAntiCheat::new();
        assert!(p.initialize().is_ok());
        assert_eq!(p.initialize(), Err(AntiCheatError::AlreadyInitialized));
    }

    #[test]
    fn test_session_methods_require_initialization() {
        let mut p = BasicAntiCheat::new();
        assert_eq!(
            p.begin_session(PLAYER, SESSION),
            Err(AntiCheatError::NotInitialized)
        );
    }

    #[test]
    fn test_duplicate_session_registration_is_rejected() {
        let mut p = provider();
        assert_eq!(
            p.begin_session(PLAYER, SESSION),
            Err(AntiCheatError::SessionAlreadyRegistered(SESSION))
        );
    }

    #[test]
    fn test_legitimate_commands_are_allowed_and_leave_log_empty() {
        let mut p = provider();
        let world = StaticWorldView::new();
        for tick in 1..20 {
            let verdict = p.inspect_command(
                &ctx_at(&world, tick),
                &Command::Move {
                    position: (tick as f32 * 0.2, 0.0, 0.0),
                    velocity: (2.0, 0.0, 0.0),
                },
            );
            assert_eq!(verdict, Verdict::Allow, "tick {tick}");
        }
        // Only the informational promotion transition is logged.
        let log = p.security_log().unwrap();
        assert!(
            log.count_at_least(SecuritySeverity::Low) == 0,
            "unexpected findings: {:?}",
            log.iter().map(|e| e.describe()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_weak_evidence_is_observed_not_rejected() {
        let mut p = provider();
        let world = StaticWorldView::new();
        let verdict = p.inspect_command(
            &ctx_at(&world, 5),
            &Command::Move {
                position: (10.0, 0.0, 0.0),
                velocity: (500.0, 0.0, 0.0),
            },
        );
        assert_eq!(
            verdict,
            Verdict::Observe(EnforcementReason::ImpossibleMovement)
        );
        assert!(verdict.allows_command());
        assert_eq!(p.session_trust(SESSION), TrustLevel::Untrusted);
        assert!(p.drain_pending_kicks().is_empty());
    }

    #[test]
    fn test_strong_evidence_rejects_the_command_without_kicking() {
        let mut p = provider();
        let enemy = EntityId::new(9);
        let world = StaticWorldView::new().with_entity(enemy, ENEMY);
        let verdict = p.inspect_command(
            &ctx_at(&world, 5),
            &Command::TransferRegion {
                entity_id: enemy,
                destination_region: game_types::RegionId::new(2),
            },
        );
        assert_eq!(
            verdict,
            Verdict::Reject(EnforcementReason::UnauthorizedOrder)
        );
        assert!(!verdict.allows_command());
        assert!(p.drain_pending_kicks().is_empty());
        assert_eq!(p.session_trust(SESSION), TrustLevel::Flagged);
    }

    #[test]
    fn test_conclusive_evidence_bans_and_queues_a_kick() {
        let mut p = provider();
        let world = StaticWorldView::new();
        let verdict = p.inspect_command(
            &ctx_at(&world, 5),
            &Command::Move {
                position: (f32::NAN, 0.0, 0.0),
                velocity: (0.0, 0.0, 0.0),
            },
        );
        assert!(matches!(verdict, Verdict::Kick(_)));
        assert_eq!(p.session_trust(SESSION), TrustLevel::Banned);
        assert_eq!(p.player_status(PLAYER), AntiCheatStatus::Banned);
        let kicks = p.drain_pending_kicks();
        assert_eq!(kicks, vec![(SESSION, EnforcementReason::TrustLevelBanned)]);
        // Drained exactly once.
        assert!(p.drain_pending_kicks().is_empty());
    }

    #[test]
    fn test_banned_session_is_kicked_on_every_subsequent_command() {
        let mut p = provider();
        let world = StaticWorldView::new();
        p.inspect_command(
            &ctx_at(&world, 5),
            &Command::Move {
                position: (f32::NAN, 0.0, 0.0),
                velocity: (0.0, 0.0, 0.0),
            },
        );
        let verdict = p.inspect_command(
            &ctx_at(&world, 6),
            &Command::Move {
                position: (1.0, 0.0, 0.0),
                velocity: (1.0, 0.0, 0.0),
            },
        );
        assert_eq!(verdict, Verdict::Kick(EnforcementReason::TrustLevelBanned));
    }

    #[test]
    fn test_resource_duplication_attempt_is_rejected_and_logged() {
        let mut p = provider();
        let own = EntityId::new(3);
        let dest = EntityId::new(4);
        let resource = ResourceId::new(10);
        let world = StaticWorldView::new()
            .with_entity(own, OWN)
            .with_entity(dest, OWN)
            .with_balance(own, resource, 5);
        let verdict = p.inspect_command(
            &ctx_at(&world, 5),
            &Command::TransferResource {
                from_entity: own,
                to_entity: dest,
                resource_id: resource,
                amount: 5_000,
            },
        );
        assert_eq!(
            verdict,
            Verdict::Reject(EnforcementReason::ImpossibleEconomyDelta)
        );
        let log = p.security_log().unwrap();
        assert!(
            log.iter()
                .any(|e| matches!(e.kind, SecurityEventKind::ImpossibleEconomyDelta { .. }))
        );
    }

    #[test]
    fn test_clean_session_is_promoted_through_the_trust_ladder() {
        let mut p = provider();
        let world = StaticWorldView::new();
        for tick in 1..100u64 {
            p.inspect_command(
                &ctx_at(&world, tick),
                &Command::QueueResearch {
                    tech_id: game_types::TechId::new(1),
                },
            );
        }
        assert_eq!(p.session_trust(SESSION), TrustLevel::Trusted);
        assert_eq!(p.player_status(PLAYER), AntiCheatStatus::Clean);
    }

    #[test]
    fn test_manifest_mismatch_bans_under_official_policy() {
        let server = BuildManifest::official("official-1", 1, 0xaaaa);
        let mut p = BasicAntiCheat::with_policy(ServerPolicy::Official, server);
        p.initialize().unwrap();
        p.begin_session(PLAYER, SESSION).unwrap();
        let modded = BuildManifest::new("modded", 1, 0xbbbb);
        let result = p.verify_client_manifest(SESSION, &modded);
        assert!(matches!(result, Err(AntiCheatError::ManifestRejected(_))));
        assert_eq!(p.session_trust(SESSION), TrustLevel::Banned);
        assert!(!p.is_manifest_verified(SESSION));
        assert!(!p.drain_pending_kicks().is_empty());
    }

    #[test]
    fn test_matching_manifest_is_accepted_under_official_policy() {
        let server = BuildManifest::official("official-1", 1, 0xaaaa);
        let mut p = BasicAntiCheat::with_policy(ServerPolicy::Official, server.clone());
        p.initialize().unwrap();
        p.begin_session(PLAYER, SESSION).unwrap();
        assert!(p.verify_client_manifest(SESSION, &server).is_ok());
        assert!(p.is_manifest_verified(SESSION));
        assert_eq!(p.session_trust(SESSION), TrustLevel::Untrusted);
    }

    #[test]
    fn test_modded_manifest_is_accepted_on_private_server() {
        let server = BuildManifest::new("community", 1, 0xaaaa);
        let mut p = BasicAntiCheat::with_policy(
            ServerPolicy::PrivateCustom {
                accepted_manifest_hash: None,
            },
            server,
        );
        p.initialize().unwrap();
        p.begin_session(PLAYER, SESSION).unwrap();
        let modded = BuildManifest::new("community-with-mods", 1, 0xbbbb);
        assert!(p.verify_client_manifest(SESSION, &modded).is_ok());
        assert!(p.is_manifest_verified(SESSION));
    }

    #[test]
    fn test_reported_external_event_feeds_the_trust_machine() {
        let mut p = provider();
        p.report_event(SecurityEvent::new(
            SESSION,
            PLAYER,
            SimTick::new(3),
            SecurityEventKind::AdminPermissionDenied {
                permission: crate::admin::AdminPermission::GrantResources,
            },
        ));
        assert_eq!(p.session_trust(SESSION), TrustLevel::Flagged);
        assert!(
            p.security_log()
                .unwrap()
                .iter()
                .any(|e| matches!(e.kind, SecurityEventKind::AdminPermissionDenied { .. }))
        );
    }

    #[test]
    fn test_admin_trust_override_forces_level_and_logs_transition() {
        let mut p = provider();
        p.apply_admin_trust_override(SESSION, TrustLevel::Trusted);
        assert_eq!(p.session_trust(SESSION), TrustLevel::Trusted);
        p.apply_admin_trust_override(SESSION, TrustLevel::Banned);
        assert_eq!(p.session_trust(SESSION), TrustLevel::Banned);
        assert_eq!(
            p.drain_pending_kicks(),
            vec![(SESSION, EnforcementReason::TrustLevelBanned)]
        );
    }

    #[test]
    fn test_end_session_releases_all_per_session_state() {
        let mut p = provider();
        assert_eq!(p.session_count(), 1);
        p.end_session(PLAYER);
        assert_eq!(p.session_count(), 0);
        assert_eq!(p.player_status(PLAYER), AntiCheatStatus::Unknown);
        assert_eq!(p.session_trust(SESSION), TrustLevel::Untrusted);
    }

    #[test]
    fn test_unknown_session_commands_are_passed_through_untouched() {
        let mut p = provider();
        let world = StaticWorldView::new();
        let mut ctx = ctx_at(&world, 1);
        ctx.session_id = SessionId::new(999);
        assert_eq!(
            p.inspect_command(
                &ctx,
                &Command::Move {
                    position: (f32::NAN, 0.0, 0.0),
                    velocity: (0.0, 0.0, 0.0)
                }
            ),
            Verdict::Allow
        );
    }

    #[test]
    fn test_poll_is_observable() {
        let mut p = provider();
        assert_eq!(p.poll_count(), 0);
        p.poll();
        p.poll();
        assert_eq!(p.poll_count(), 2);
    }

    #[test]
    fn test_telemetry_only_trust_policy_never_kicks() {
        let mut p = BasicAntiCheat::new().with_trust_policy(TrustPolicy::telemetry_only());
        p.initialize().unwrap();
        p.begin_session(PLAYER, SESSION).unwrap();
        p.on_client_authenticated(SESSION, PLAYER, OWN, SimTick::zero());
        let world = StaticWorldView::new();
        for tick in 1..10 {
            p.inspect_command(
                &ctx_at(&world, tick),
                &Command::Move {
                    position: (f32::NAN, 0.0, 0.0),
                    velocity: (0.0, 0.0, 0.0),
                },
            );
        }
        assert_eq!(p.session_trust(SESSION), TrustLevel::Flagged);
        assert!(p.drain_pending_kicks().is_empty());
    }

    #[test]
    fn test_provider_inspection_is_deterministic() {
        let world = StaticWorldView::new().with_entity(EntityId::new(9), ENEMY);
        let commands = [
            Command::Move {
                position: (1.0, 0.0, 0.0),
                velocity: (2.0, 0.0, 0.0),
            },
            Command::TransferRegion {
                entity_id: EntityId::new(9),
                destination_region: game_types::RegionId::new(2),
            },
            Command::Move {
                position: (300.0, 0.0, 0.0),
                velocity: (900.0, 0.0, 0.0),
            },
        ];
        let run = || {
            let mut p = provider();
            let mut verdicts = Vec::new();
            for (i, command) in commands.iter().enumerate() {
                verdicts.push(p.inspect_command(&ctx_at(&world, i as u64 + 1), command));
            }
            let log: Vec<String> = p
                .security_log()
                .unwrap()
                .iter()
                .map(|e| e.describe())
                .collect();
            (verdicts, log, p.session_trust(SESSION))
        };
        assert_eq!(run(), run());
    }
}
