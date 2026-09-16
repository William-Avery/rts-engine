use crate::event::SecuritySeverity;
use game_types::{FactionId, PlayerId, SessionId, SimTick};
use std::fmt;

/// Client trust level.
///
/// The happy path is a strictly monotonic promotion ladder
/// `Untrusted -> Probationary -> Trusted`, driven purely by counted clean
/// commands. Accumulated suspicion moves a session sideways into `Flagged`, and
/// only conclusive evidence (or a sustained flagged session) reaches `Banned`,
/// which is terminal.
///
/// ```text
///   Untrusted --clean--> Probationary --clean--> Trusted
///        \                    ^  |                  |
///         \                   |  |                  | suspicion
///          \        decay ----+  |                  v
///           \                    +---> Flagged <----+
///            \                          |
///             +--- critical ----------> Banned (terminal)
/// ```
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub enum TrustLevel {
    /// Freshly connected. Commands are accepted but every detector runs.
    #[default]
    Untrusted,
    /// Enough clean commands to look human, not yet enough for full trust.
    Probationary,
    /// Long-running clean session. Detectors still run; nothing is skipped.
    Trusted,
    /// Accumulated suspicion above the flag threshold. Under review, not punished.
    Flagged,
    /// Terminal. The session is to be kicked and must not be readmitted.
    Banned,
}

impl TrustLevel {
    /// Whether this is an end state no transition can leave.
    pub const fn is_terminal(&self) -> bool {
        matches!(self, TrustLevel::Banned)
    }

    /// Whether a session at this level may still submit commands.
    ///
    /// Note that `Flagged` returns `true` on purpose: weak heuristic evidence
    /// records telemetry, it does not remove a player from the match.
    pub const fn may_issue_commands(&self) -> bool {
        !matches!(self, TrustLevel::Banned)
    }

    pub const fn as_str(&self) -> &'static str {
        match self {
            TrustLevel::Untrusted => "untrusted",
            TrustLevel::Probationary => "probationary",
            TrustLevel::Trusted => "trusted",
            TrustLevel::Flagged => "flagged",
            TrustLevel::Banned => "banned",
        }
    }

    /// Stable wire code, used by the admin trust-override command.
    pub const fn code(&self) -> u8 {
        match self {
            TrustLevel::Untrusted => 0,
            TrustLevel::Probationary => 1,
            TrustLevel::Trusted => 2,
            TrustLevel::Flagged => 3,
            TrustLevel::Banned => 4,
        }
    }

    pub const fn from_code(code: u8) -> Option<TrustLevel> {
        match code {
            0 => Some(TrustLevel::Untrusted),
            1 => Some(TrustLevel::Probationary),
            2 => Some(TrustLevel::Trusted),
            3 => Some(TrustLevel::Flagged),
            4 => Some(TrustLevel::Banned),
            _ => None,
        }
    }
}

impl fmt::Display for TrustLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A recorded trust state machine transition.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct TrustTransition {
    pub from: TrustLevel,
    pub to: TrustLevel,
}

/// Thresholds governing the trust state machine.
///
/// All values are counts and scores, never wall-clock time, so transitions are
/// fully deterministic and reproducible from a command sequence alone.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct TrustPolicy {
    /// Clean commands required to move `Untrusted -> Probationary`.
    pub probation_after_clean_commands: u32,
    /// Clean commands required to move `Probationary -> Trusted`.
    pub trusted_after_clean_commands: u32,
    /// Suspicion score at or above which a session becomes `Flagged`.
    pub flag_threshold: u32,
    /// Suspicion score at or above which a session becomes `Banned`.
    pub ban_threshold: u32,
    /// Suspicion removed per clean command, so honest players recover.
    pub decay_per_clean_command: u32,
    /// Whether a single `Critical` event bans immediately.
    ///
    /// `Critical` is reserved for evidence with no benign explanation
    /// (NaN coordinates, manifest tampering), so this defaults to `true`.
    /// Weak evidence never reaches here.
    pub ban_on_critical: bool,
}

impl Default for TrustPolicy {
    fn default() -> Self {
        TrustPolicy {
            probation_after_clean_commands: 8,
            trusted_after_clean_commands: 64,
            flag_threshold: 40,
            ban_threshold: 200,
            decay_per_clean_command: 1,
            ban_on_critical: true,
        }
    }
}

impl TrustPolicy {
    /// A policy that never bans, for private servers that only want telemetry.
    pub fn telemetry_only() -> Self {
        TrustPolicy {
            ban_threshold: u32::MAX,
            ban_on_critical: false,
            ..TrustPolicy::default()
        }
    }
}

/// Per-session trust and suspicion state.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ClientTrustState {
    pub session_id: SessionId,
    pub player_id: PlayerId,
    pub faction_id: FactionId,
    pub level: TrustLevel,
    pub suspicion_score: u32,
    /// Consecutive clean commands since the last violation.
    pub clean_commands: u32,
    /// Total violations recorded this session.
    pub violations: u32,
    pub session_start_tick: SimTick,
    pub last_violation_tick: Option<SimTick>,
}

impl ClientTrustState {
    pub fn new(
        session_id: SessionId,
        player_id: PlayerId,
        faction_id: FactionId,
        session_start_tick: SimTick,
    ) -> Self {
        ClientTrustState {
            session_id,
            player_id,
            faction_id,
            level: TrustLevel::Untrusted,
            suspicion_score: 0,
            clean_commands: 0,
            violations: 0,
            session_start_tick,
            last_violation_tick: None,
        }
    }

    /// Record a command that passed every detector.
    ///
    /// Promotes along the trust ladder and decays accumulated suspicion.
    pub fn record_clean_command(&mut self, policy: &TrustPolicy) -> Option<TrustTransition> {
        if self.level.is_terminal() {
            return None;
        }
        self.clean_commands = self.clean_commands.saturating_add(1);
        self.suspicion_score = self
            .suspicion_score
            .saturating_sub(policy.decay_per_clean_command);

        let from = self.level;
        let to = match self.level {
            TrustLevel::Flagged if self.suspicion_score < policy.flag_threshold => {
                TrustLevel::Probationary
            }
            TrustLevel::Untrusted
                if self.clean_commands >= policy.probation_after_clean_commands =>
            {
                TrustLevel::Probationary
            }
            TrustLevel::Probationary
                if self.clean_commands >= policy.trusted_after_clean_commands =>
            {
                TrustLevel::Trusted
            }
            other => other,
        };
        self.transition(from, to)
    }

    /// Record a detected violation of the given severity.
    pub fn record_violation(
        &mut self,
        severity: SecuritySeverity,
        tick: SimTick,
        policy: &TrustPolicy,
    ) -> Option<TrustTransition> {
        if self.level.is_terminal() {
            return None;
        }
        self.violations = self.violations.saturating_add(1);
        self.clean_commands = 0;
        self.last_violation_tick = Some(tick);
        self.suspicion_score = self
            .suspicion_score
            .saturating_add(severity.suspicion_weight());

        let from = self.level;
        let critical_ban = policy.ban_on_critical && severity == SecuritySeverity::Critical;
        let to = if critical_ban || self.suspicion_score >= policy.ban_threshold {
            TrustLevel::Banned
        } else if self.suspicion_score >= policy.flag_threshold {
            TrustLevel::Flagged
        } else if self.level == TrustLevel::Trusted {
            // Demote out of full trust on any violation, but do not flag yet.
            TrustLevel::Probationary
        } else {
            self.level
        };
        self.transition(from, to)
    }

    /// Administrative override, used by the host/admin permission path.
    pub fn force_level(&mut self, level: TrustLevel) -> Option<TrustTransition> {
        let from = self.level;
        self.transition(from, level)
    }

    fn transition(&mut self, from: TrustLevel, to: TrustLevel) -> Option<TrustTransition> {
        if from == to {
            return None;
        }
        self.level = to;
        Some(TrustTransition { from, to })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ClientTrustState {
        ClientTrustState::new(
            SessionId::new(1),
            PlayerId::new(1),
            FactionId::new(1),
            SimTick::zero(),
        )
    }

    #[test]
    fn test_new_session_starts_untrusted() {
        assert_eq!(state().level, TrustLevel::Untrusted);
        assert_eq!(state().suspicion_score, 0);
    }

    #[test]
    fn test_clean_commands_promote_untrusted_to_probationary_to_trusted() {
        let policy = TrustPolicy::default();
        let mut s = state();
        for _ in 0..policy.probation_after_clean_commands {
            s.record_clean_command(&policy);
        }
        assert_eq!(s.level, TrustLevel::Probationary);
        for _ in 0..policy.trusted_after_clean_commands {
            s.record_clean_command(&policy);
        }
        assert_eq!(s.level, TrustLevel::Trusted);
    }

    #[test]
    fn test_promotion_transitions_are_deterministic() {
        let policy = TrustPolicy::default();
        let mut a = state();
        let mut b = state();
        let mut ta = Vec::new();
        let mut tb = Vec::new();
        for _ in 0..100 {
            ta.extend(a.record_clean_command(&policy));
            tb.extend(b.record_clean_command(&policy));
        }
        assert_eq!(ta, tb);
        assert_eq!(a, b);
    }

    #[test]
    fn test_single_weak_event_never_flags_or_bans() {
        let policy = TrustPolicy::default();
        let mut s = state();
        s.record_violation(SecuritySeverity::Low, SimTick::new(5), &policy);
        assert_eq!(s.level, TrustLevel::Untrusted);
        assert_eq!(s.suspicion_score, 5);
        assert!(s.level.may_issue_commands());
    }

    #[test]
    fn test_accumulated_weak_evidence_flags_but_does_not_ban() {
        let policy = TrustPolicy::default();
        let mut s = state();
        for t in 0..8 {
            s.record_violation(SecuritySeverity::Low, SimTick::new(t), &policy);
        }
        assert_eq!(s.level, TrustLevel::Flagged);
        assert!(s.level.may_issue_commands());
        assert!(!s.level.is_terminal());
    }

    #[test]
    fn test_flagged_session_recovers_to_probationary_on_clean_streak() {
        let policy = TrustPolicy::default();
        let mut s = state();
        for t in 0..8 {
            s.record_violation(SecuritySeverity::Low, SimTick::new(t), &policy);
        }
        assert_eq!(s.level, TrustLevel::Flagged);
        for _ in 0..10 {
            s.record_clean_command(&policy);
        }
        assert_eq!(s.level, TrustLevel::Probationary);
    }

    #[test]
    fn test_critical_evidence_bans_immediately() {
        let policy = TrustPolicy::default();
        let mut s = state();
        let t = s.record_violation(SecuritySeverity::Critical, SimTick::new(1), &policy);
        assert_eq!(
            t,
            Some(TrustTransition {
                from: TrustLevel::Untrusted,
                to: TrustLevel::Banned
            })
        );
        assert!(s.level.is_terminal());
        assert!(!s.level.may_issue_commands());
    }

    #[test]
    fn test_telemetry_only_policy_never_bans() {
        let policy = TrustPolicy::telemetry_only();
        let mut s = state();
        for t in 0..50 {
            s.record_violation(SecuritySeverity::Critical, SimTick::new(t), &policy);
        }
        assert_eq!(s.level, TrustLevel::Flagged);
        assert!(s.level.may_issue_commands());
    }

    #[test]
    fn test_banned_state_is_terminal() {
        let policy = TrustPolicy::default();
        let mut s = state();
        s.record_violation(SecuritySeverity::Critical, SimTick::new(1), &policy);
        assert!(s.record_clean_command(&policy).is_none());
        assert!(
            s.record_violation(SecuritySeverity::Low, SimTick::new(2), &policy)
                .is_none()
        );
        assert_eq!(s.level, TrustLevel::Banned);
    }

    #[test]
    fn test_violation_demotes_trusted_session_to_probationary() {
        let policy = TrustPolicy::default();
        let mut s = state();
        for _ in 0..policy.trusted_after_clean_commands + policy.probation_after_clean_commands {
            s.record_clean_command(&policy);
        }
        assert_eq!(s.level, TrustLevel::Trusted);
        s.record_violation(SecuritySeverity::Medium, SimTick::new(9), &policy);
        assert_eq!(s.level, TrustLevel::Probationary);
    }

    #[test]
    fn test_trust_level_wire_codes_roundtrip() {
        for level in [
            TrustLevel::Untrusted,
            TrustLevel::Probationary,
            TrustLevel::Trusted,
            TrustLevel::Flagged,
            TrustLevel::Banned,
        ] {
            assert_eq!(TrustLevel::from_code(level.code()), Some(level));
        }
        assert_eq!(TrustLevel::from_code(200), None);
    }
}
