use crate::admin::AdminPermission;
use crate::trust::TrustLevel;
use game_types::{EntityId, FactionId, PlayerId, ResourceId, SessionId, SimTick, StructureId};
use std::fmt;

/// Severity classification for a detected security anomaly.
///
/// Severity drives two things and nothing else: the suspicion weight added to a
/// session's trust state, and whether the provider merely observes a command or
/// actively rejects it. It never directly bans: see [`crate::trust::TrustPolicy`].
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub enum SecuritySeverity {
    /// Informational telemetry, no suspicion attached (lifecycle notes, trust transitions).
    #[default]
    Info,
    /// Weak evidence. Legitimate clients hit this occasionally under packet loss or lag.
    Low,
    /// Moderate evidence. Repeated occurrences are meaningful; a single one is not.
    Medium,
    /// Strong evidence. The command is provably inconsistent with authoritative state.
    High,
    /// Conclusive evidence of tampering that no latency or desync explanation covers.
    Critical,
}

impl SecuritySeverity {
    /// Suspicion score added to the session trust state when an event of this severity is recorded.
    pub const fn suspicion_weight(&self) -> u32 {
        match self {
            SecuritySeverity::Info => 0,
            SecuritySeverity::Low => 5,
            SecuritySeverity::Medium => 15,
            SecuritySeverity::High => 40,
            SecuritySeverity::Critical => 200,
        }
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            SecuritySeverity::Info => "INFO",
            SecuritySeverity::Low => "LOW",
            SecuritySeverity::Medium => "MEDIUM",
            SecuritySeverity::High => "HIGH",
            SecuritySeverity::Critical => "CRITICAL",
        }
    }
}

impl fmt::Display for SecuritySeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Reason a placement command was considered anomalous.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum PlacementRejection {
    /// Position contained NaN or an infinity.
    NonFiniteCoordinate,
    /// Position is outside the authoritative world bounds.
    OutsideWorldBounds,
    /// Rotation was NaN, infinite, or far outside a sane degree range.
    InvalidRotation,
    /// Position is further from the session's last authoritative position than build reach allows.
    BeyondBuildReach,
}

impl PlacementRejection {
    pub const fn as_str(&self) -> &'static str {
        match self {
            PlacementRejection::NonFiniteCoordinate => "non-finite coordinate",
            PlacementRejection::OutsideWorldBounds => "outside world bounds",
            PlacementRejection::InvalidRotation => "invalid rotation",
            PlacementRejection::BeyondBuildReach => "beyond build reach",
        }
    }
}

/// Why a command packet failed to bind to the session it claimed.
///
/// `session_id` is a client-supplied routing field in the packet body. Without
/// a binding check, one unauthenticated datagram naming a real session could
/// mute that player forever by latching its `last_received_sequence`.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum SessionBindingRejection {
    /// The claimed session does not exist on this server.
    UnknownSession,
    /// The capability token did not match the one the server issued.
    TokenMismatch,
    /// The datagram arrived from an address other than the session's peer.
    PeerAddressMismatch,
}

impl SessionBindingRejection {
    pub const fn as_str(&self) -> &'static str {
        match self {
            SessionBindingRejection::UnknownSession => "unknown session",
            SessionBindingRejection::TokenMismatch => "token mismatch",
            SessionBindingRejection::PeerAddressMismatch => "peer address mismatch",
        }
    }
}

/// Structured detail of a detected anomaly.
///
/// Every variant carries the authoritative numbers the detector compared, so the
/// security log is self-describing and no free-form strings are allocated on the
/// command hot path.
#[derive(Clone, PartialEq, Debug)]
pub enum SecurityEventKind {
    /// Client claimed a velocity or displacement exceeding the server speed clamp.
    ImpossibleMovement {
        observed_speed_mps: f32,
        max_speed_mps: f32,
        elapsed_ticks: u64,
    },
    /// Client claimed a discontinuous position jump within a single tick.
    TeleportAttempt {
        distance_m: f32,
        max_distance_m: f32,
    },
    /// Movement or placement payload contained NaN / infinity.
    NonFinitePosition,
    /// Claimed position lies outside the authoritative world bounds.
    PositionOutOfBounds { x: f32, z: f32 },
    /// Weapon discharge arrived sooner than the weapon's minimum cycle time allows.
    FireRateViolation {
        actor: EntityId,
        ticks_since_last: u64,
        min_ticks: u64,
    },
    /// Weapon discharge from an actor whose authoritative ammo balance is empty.
    AmmoInconsistency {
        actor: EntityId,
        resource: ResourceId,
        available: u32,
    },
    /// Construction placement failed a server-side sanity check before it ever reached the sim.
    InvalidPlacement {
        reason: PlacementRejection,
        position: (f32, f32, f32),
    },
    /// Client requested a resource movement larger than the authoritative balance permits.
    ImpossibleEconomyDelta {
        entity: EntityId,
        resource: ResourceId,
        requested: u32,
        available: u32,
    },
    /// Session issued an order to an entity owned by a different faction.
    UnauthorizedOrder {
        target_entity: EntityId,
        owning_faction: FactionId,
        claiming_faction: FactionId,
    },
    /// Session issued an order to a structure owned by a different faction.
    UnauthorizedStructureOrder {
        structure: StructureId,
        owning_faction: FactionId,
        claiming_faction: FactionId,
    },
    /// Session acted on an entity its faction has no sensor knowledge of.
    HiddenTargetAttempt {
        target: EntityId,
        claiming_faction: FactionId,
    },
    /// Command envelope was stamped with a client tick far ahead of the authoritative tick.
    FutureDatedCommand {
        client_tick: SimTick,
        server_tick: SimTick,
    },
    /// Session submitted commands far above the maximum plausible human input rate.
    CommandFloodDetected {
        commands_in_window: u32,
        window_ticks: u64,
    },
    /// Client build / protocol / content manifest did not satisfy the server policy.
    ManifestMismatch { expected: u64, actual: u64 },
    /// Session attempted a host/admin command it is not authorized for.
    AdminPermissionDenied { permission: AdminPermission },
    /// Session trust state machine changed level.
    TrustLevelChanged { from: TrustLevel, to: TrustLevel },
    /// A command packet claimed a session it could not prove it owns.
    SessionBindingMismatch {
        claimed_session: SessionId,
        reason: SessionBindingRejection,
    },
}

impl SecurityEventKind {
    /// Severity assigned to this anomaly class.
    ///
    /// Anything a laggy-but-honest client can produce is capped at `Medium`.
    /// `High` is reserved for claims contradicted by authoritative state, and
    /// `Critical` for tampering that has no benign explanation.
    pub const fn severity(&self) -> SecuritySeverity {
        match self {
            SecurityEventKind::TrustLevelChanged { .. } => SecuritySeverity::Info,
            // Latency, prediction error and packet reordering all produce these.
            SecurityEventKind::FutureDatedCommand { .. } => SecuritySeverity::Low,
            SecurityEventKind::ImpossibleMovement { .. } => SecuritySeverity::Low,
            SecurityEventKind::TeleportAttempt { .. } => SecuritySeverity::Medium,
            SecurityEventKind::FireRateViolation { .. } => SecuritySeverity::Medium,
            SecurityEventKind::CommandFloodDetected { .. } => SecuritySeverity::Medium,
            SecurityEventKind::InvalidPlacement { .. } => SecuritySeverity::Medium,
            // Contradicted by authoritative state the client cannot have raced.
            SecurityEventKind::AmmoInconsistency { .. } => SecuritySeverity::High,
            SecurityEventKind::ImpossibleEconomyDelta { .. } => SecuritySeverity::High,
            SecurityEventKind::UnauthorizedOrder { .. } => SecuritySeverity::High,
            SecurityEventKind::UnauthorizedStructureOrder { .. } => SecuritySeverity::High,
            SecurityEventKind::HiddenTargetAttempt { .. } => SecuritySeverity::High,
            SecurityEventKind::AdminPermissionDenied { .. } => SecuritySeverity::High,
            // No benign explanation.
            SecurityEventKind::NonFinitePosition => SecuritySeverity::Critical,
            SecurityEventKind::PositionOutOfBounds { .. } => SecuritySeverity::Critical,
            SecurityEventKind::ManifestMismatch { .. } => SecuritySeverity::Critical,
            SecurityEventKind::SessionBindingMismatch { .. } => SecuritySeverity::Critical,
        }
    }

    /// Stable machine-readable label for this anomaly class.
    pub const fn label(&self) -> &'static str {
        match self {
            SecurityEventKind::ImpossibleMovement { .. } => "impossible_movement",
            SecurityEventKind::TeleportAttempt { .. } => "teleport_attempt",
            SecurityEventKind::NonFinitePosition => "non_finite_position",
            SecurityEventKind::PositionOutOfBounds { .. } => "position_out_of_bounds",
            SecurityEventKind::FireRateViolation { .. } => "fire_rate_violation",
            SecurityEventKind::AmmoInconsistency { .. } => "ammo_inconsistency",
            SecurityEventKind::InvalidPlacement { .. } => "invalid_placement",
            SecurityEventKind::ImpossibleEconomyDelta { .. } => "impossible_economy_delta",
            SecurityEventKind::UnauthorizedOrder { .. } => "unauthorized_order",
            SecurityEventKind::UnauthorizedStructureOrder { .. } => "unauthorized_structure_order",
            SecurityEventKind::HiddenTargetAttempt { .. } => "hidden_target_attempt",
            SecurityEventKind::FutureDatedCommand { .. } => "future_dated_command",
            SecurityEventKind::CommandFloodDetected { .. } => "command_flood",
            SecurityEventKind::ManifestMismatch { .. } => "manifest_mismatch",
            SecurityEventKind::AdminPermissionDenied { .. } => "admin_permission_denied",
            SecurityEventKind::TrustLevelChanged { .. } => "trust_level_changed",
            SecurityEventKind::SessionBindingMismatch { .. } => "session_binding_mismatch",
        }
    }
}

/// A single append-only security telemetry record.
///
/// Security events are deliberately **not** `sim_core::event::SimEvent`s: they
/// live outside the deterministic simulation journal so that enabling or
/// disabling anti-cheat cannot perturb simulation state or replay hashes.
#[derive(Clone, PartialEq, Debug)]
pub struct SecurityEvent {
    pub session_id: SessionId,
    pub player_id: PlayerId,
    pub server_tick: SimTick,
    pub severity: SecuritySeverity,
    pub kind: SecurityEventKind,
}

impl SecurityEvent {
    /// Build an event, deriving severity from the anomaly class.
    pub fn new(
        session_id: SessionId,
        player_id: PlayerId,
        server_tick: SimTick,
        kind: SecurityEventKind,
    ) -> Self {
        SecurityEvent {
            session_id,
            player_id,
            server_tick,
            severity: kind.severity(),
            kind,
        }
    }

    /// Human-readable one-line rendering for operator logs.
    pub fn describe(&self) -> String {
        format!(
            "[{}] tick={} session={} player={} {} {:?}",
            self.severity.as_str(),
            self.server_tick.value(),
            self.session_id.value(),
            self.player_id.value(),
            self.kind.label(),
            self.kind
        )
    }
}

impl fmt::Display for SecurityEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.describe())
    }
}

/// Default retained security log capacity.
pub const DEFAULT_SECURITY_LOG_CAPACITY: usize = 4096;

/// Append-only, bounded security telemetry sink.
///
/// Records are only ever appended. When the retention bound is reached the
/// oldest records are evicted and counted in [`SecurityLog::dropped_count`], so
/// a full log is always visibly full rather than silently lossy. Production
/// deployments are expected to drain this into durable storage each tick via
/// [`SecurityLog::iter_since`].
#[derive(Clone, Debug)]
pub struct SecurityLog {
    entries: Vec<SecurityEvent>,
    capacity: usize,
    appended: u64,
    dropped: u64,
}

impl Default for SecurityLog {
    fn default() -> Self {
        SecurityLog::new(DEFAULT_SECURITY_LOG_CAPACITY)
    }
}

impl SecurityLog {
    pub fn new(capacity: usize) -> Self {
        SecurityLog {
            entries: Vec::new(),
            capacity: capacity.max(1),
            appended: 0,
            dropped: 0,
        }
    }

    /// Append a record. Never mutates or reorders existing records.
    pub fn append(&mut self, event: SecurityEvent) {
        if self.entries.len() >= self.capacity {
            self.entries.remove(0);
            self.dropped += 1;
        }
        self.entries.push(event);
        self.appended += 1;
    }

    pub fn iter(&self) -> impl Iterator<Item = &SecurityEvent> {
        self.entries.iter()
    }

    /// Records at or after `tick`, in append order.
    pub fn iter_since(&self, tick: SimTick) -> impl Iterator<Item = &SecurityEvent> {
        self.entries.iter().filter(move |e| e.server_tick >= tick)
    }

    /// Records belonging to one session, in append order.
    pub fn events_for_session(&self, session: SessionId) -> impl Iterator<Item = &SecurityEvent> {
        self.entries.iter().filter(move |e| e.session_id == session)
    }

    /// Number of retained records with at least the given severity.
    pub fn count_at_least(&self, severity: SecuritySeverity) -> usize {
        self.entries
            .iter()
            .filter(|e| e.severity >= severity)
            .count()
    }

    /// Number of retained records.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total records ever appended, including evicted ones.
    pub fn appended_count(&self) -> u64 {
        self.appended
    }

    /// Records evicted by the retention bound.
    pub fn dropped_count(&self) -> u64 {
        self.dropped
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evt(tick: u64, kind: SecurityEventKind) -> SecurityEvent {
        SecurityEvent::new(
            SessionId::new(1),
            PlayerId::new(1),
            SimTick::new(tick),
            kind,
        )
    }

    #[test]
    fn test_severity_ordering_and_weights() {
        assert!(SecuritySeverity::Critical > SecuritySeverity::High);
        assert!(SecuritySeverity::High > SecuritySeverity::Medium);
        assert!(SecuritySeverity::Medium > SecuritySeverity::Low);
        assert!(SecuritySeverity::Low > SecuritySeverity::Info);
        assert_eq!(SecuritySeverity::Info.suspicion_weight(), 0);
        assert_eq!(SecuritySeverity::Critical.suspicion_weight(), 200);
    }

    #[test]
    fn test_event_severity_is_derived_from_kind() {
        let e = evt(7, SecurityEventKind::NonFinitePosition);
        assert_eq!(e.severity, SecuritySeverity::Critical);
        let e = evt(
            7,
            SecurityEventKind::FutureDatedCommand {
                client_tick: SimTick::new(900),
                server_tick: SimTick::new(3),
            },
        );
        assert_eq!(e.severity, SecuritySeverity::Low);
    }

    #[test]
    fn test_laggy_client_anomalies_never_exceed_medium() {
        // These classes have benign latency explanations and must never alone ban.
        let benign = [
            SecurityEventKind::ImpossibleMovement {
                observed_speed_mps: 40.0,
                max_speed_mps: 12.0,
                elapsed_ticks: 1,
            },
            SecurityEventKind::TeleportAttempt {
                distance_m: 30.0,
                max_distance_m: 6.0,
            },
            SecurityEventKind::FutureDatedCommand {
                client_tick: SimTick::new(500),
                server_tick: SimTick::new(1),
            },
            SecurityEventKind::FireRateViolation {
                actor: EntityId::new(1),
                ticks_since_last: 1,
                min_ticks: 6,
            },
        ];
        for kind in benign {
            assert!(kind.severity() <= SecuritySeverity::Medium, "{kind:?}");
        }
    }

    #[test]
    fn test_security_log_is_append_only_and_ordered() {
        let mut log = SecurityLog::new(16);
        log.append(evt(1, SecurityEventKind::NonFinitePosition));
        log.append(evt(2, SecurityEventKind::NonFinitePosition));
        log.append(evt(3, SecurityEventKind::NonFinitePosition));
        let ticks: Vec<u64> = log.iter().map(|e| e.server_tick.value()).collect();
        assert_eq!(ticks, vec![1, 2, 3]);
        assert_eq!(log.appended_count(), 3);
        assert_eq!(log.dropped_count(), 0);
    }

    #[test]
    fn test_security_log_bounded_eviction_is_counted() {
        let mut log = SecurityLog::new(2);
        for t in 1..=5 {
            log.append(evt(t, SecurityEventKind::NonFinitePosition));
        }
        assert_eq!(log.len(), 2);
        assert_eq!(log.appended_count(), 5);
        assert_eq!(log.dropped_count(), 3);
        let ticks: Vec<u64> = log.iter().map(|e| e.server_tick.value()).collect();
        assert_eq!(ticks, vec![4, 5]);
    }

    #[test]
    fn test_security_log_filters_by_session_and_tick() {
        let mut log = SecurityLog::new(16);
        log.append(evt(1, SecurityEventKind::NonFinitePosition));
        let mut other = evt(5, SecurityEventKind::NonFinitePosition);
        other.session_id = SessionId::new(2);
        log.append(other);
        assert_eq!(log.events_for_session(SessionId::new(1)).count(), 1);
        assert_eq!(log.events_for_session(SessionId::new(2)).count(), 1);
        assert_eq!(log.iter_since(SimTick::new(5)).count(), 1);
        assert_eq!(log.count_at_least(SecuritySeverity::High), 2);
    }

    #[test]
    fn test_event_describe_contains_label_and_severity() {
        let e = evt(11, SecurityEventKind::NonFinitePosition);
        let s = e.describe();
        assert!(s.contains("CRITICAL"), "{s}");
        assert!(s.contains("non_finite_position"), "{s}");
        assert!(s.contains("tick=11"), "{s}");
    }
}
