use crate::event::{SecurityEvent, SecurityLog};
use crate::manifest::{BuildManifest, ManifestError, ServerPolicy};
use crate::trust::TrustLevel;
use crate::world_view::WorldView;
use game_types::{EntityId, FactionId, PlayerId, SessionId, SimTick};
use sim_core::command::Command;
use std::fmt;

/// Errors raised by an anti-cheat provider.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AntiCheatError {
    /// `initialize` called twice.
    AlreadyInitialized,
    /// A session operation was attempted before `initialize` succeeded.
    NotInitialized,
    /// `begin_session` called for a session that is already registered.
    SessionAlreadyRegistered(SessionId),
    /// A session operation referenced an unknown session.
    SessionNotRegistered(SessionId),
    /// The client's build/protocol/content manifest was refused by server policy.
    ManifestRejected(ManifestError),
    /// The session is banned and must not be admitted.
    SessionBanned(SessionId),
    /// A backing anti-cheat SDK is required but not present in this environment.
    ///
    /// This is the error the feature-gated EOS/EAC adapter returns. It is an
    /// explicit, actionable failure by design: the adapter never pretends to
    /// have protected a session it could not protect.
    SdkUnavailable {
        sdk: &'static str,
        detail: &'static str,
    },
    /// A backing anti-cheat service returned an error.
    BackendFailure(String),
}

impl fmt::Display for AntiCheatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AntiCheatError::AlreadyInitialized => write!(f, "Anti-cheat already initialized"),
            AntiCheatError::NotInitialized => write!(f, "Anti-cheat provider is not initialized"),
            AntiCheatError::SessionAlreadyRegistered(id) => {
                write!(f, "Anti-cheat session already registered: {id}")
            }
            AntiCheatError::SessionNotRegistered(id) => {
                write!(f, "Anti-cheat session not registered: {id}")
            }
            AntiCheatError::ManifestRejected(err) => write!(f, "Client manifest rejected: {err}"),
            AntiCheatError::SessionBanned(id) => write!(f, "Session is banned: {id}"),
            AntiCheatError::SdkUnavailable { sdk, detail } => {
                write!(f, "{sdk} SDK unavailable: {detail}")
            }
            AntiCheatError::BackendFailure(msg) => write!(f, "Anti-cheat backend failure: {msg}"),
        }
    }
}

impl std::error::Error for AntiCheatError {}

pub type AntiCheatResult<T> = Result<T, AntiCheatError>;

/// Externally visible anti-cheat standing of a player.
///
/// Deliberately coarser than [`TrustLevel`]: this is what a server operator or a
/// future matchmaking service sees, and it never leaks detector internals.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub enum AntiCheatStatus {
    /// No provider opinion (null provider, or session not tracked).
    #[default]
    Unknown,
    /// Tracked, nothing of note recorded.
    Clean,
    /// Tracked, suspicion accumulated, under observation. Not punished.
    UnderReview,
    /// Tracked, conclusive evidence recorded. Pending kick.
    Banned,
}

impl AntiCheatStatus {
    pub const fn as_str(&self) -> &'static str {
        match self {
            AntiCheatStatus::Unknown => "unknown",
            AntiCheatStatus::Clean => "clean",
            AntiCheatStatus::UnderReview => "under-review",
            AntiCheatStatus::Banned => "banned",
        }
    }

    /// Project a trust level onto the externally visible status.
    pub const fn from_trust(level: TrustLevel) -> Self {
        match level {
            TrustLevel::Untrusted | TrustLevel::Probationary | TrustLevel::Trusted => {
                AntiCheatStatus::Clean
            }
            TrustLevel::Flagged => AntiCheatStatus::UnderReview,
            TrustLevel::Banned => AntiCheatStatus::Banned,
        }
    }
}

impl fmt::Display for AntiCheatStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Machine-readable reason attached to an enforcement action.
///
/// Copy + `'static` on purpose: enforcement must never allocate on the command
/// hot path, and reasons must be stable enough to appear in operator tooling.
#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
pub enum EnforcementReason {
    ImpossibleMovement,
    FireRateViolation,
    AmmoInconsistency,
    InvalidPlacement,
    ImpossibleEconomyDelta,
    UnauthorizedOrder,
    HiddenTargetAttempt,
    MalformedEnvelope,
    CommandFlood,
    ManifestMismatch,
    AdminPermissionDenied,
    /// A command packet claimed a session it could not prove it owns.
    SessionBindingMismatch,
    TrustLevelBanned,
}

impl EnforcementReason {
    pub const fn as_str(&self) -> &'static str {
        match self {
            EnforcementReason::ImpossibleMovement => "impossible_movement",
            EnforcementReason::FireRateViolation => "fire_rate_violation",
            EnforcementReason::AmmoInconsistency => "ammo_inconsistency",
            EnforcementReason::InvalidPlacement => "invalid_placement",
            EnforcementReason::ImpossibleEconomyDelta => "impossible_economy_delta",
            EnforcementReason::UnauthorizedOrder => "unauthorized_order",
            EnforcementReason::HiddenTargetAttempt => "hidden_target_attempt",
            EnforcementReason::MalformedEnvelope => "malformed_envelope",
            EnforcementReason::CommandFlood => "command_flood",
            EnforcementReason::ManifestMismatch => "manifest_mismatch",
            EnforcementReason::AdminPermissionDenied => "admin_permission_denied",
            EnforcementReason::SessionBindingMismatch => "session_binding_mismatch",
            EnforcementReason::TrustLevelBanned => "trust_level_banned",
        }
    }
}

impl fmt::Display for EnforcementReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// What the server should do with an inspected command.
///
/// Anti-cheat is **defence in depth**. `Allow` and `Observe` both hand the
/// command straight on to the server's own authoritative validation, which
/// rejects illegal commands regardless of provider.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// Nothing detected. Proceed to authoritative validation.
    Allow,
    /// Weak evidence recorded as telemetry. Proceed to authoritative validation.
    Observe(EnforcementReason),
    /// Strong evidence. Drop the command before it reaches the simulation.
    Reject(EnforcementReason),
    /// Conclusive evidence. Drop the command and terminate the session.
    Kick(EnforcementReason),
}

impl Verdict {
    /// Whether the command should still be handed to authoritative validation.
    pub const fn allows_command(&self) -> bool {
        matches!(self, Verdict::Allow | Verdict::Observe(_))
    }

    /// Enforcement reason, if any.
    pub const fn reason(&self) -> Option<EnforcementReason> {
        match self {
            Verdict::Allow => None,
            Verdict::Observe(r) | Verdict::Reject(r) | Verdict::Kick(r) => Some(*r),
        }
    }
}

/// Everything a detector needs to judge one command envelope.
pub struct InspectionContext<'a> {
    pub session_id: SessionId,
    pub player_id: PlayerId,
    /// Faction the session is authorised to command.
    pub faction_id: FactionId,
    /// Authoritative server tick at the moment of inspection.
    pub server_tick: SimTick,
    /// Tick the client stamped on the envelope.
    pub client_tick: SimTick,
    /// Monotonic envelope sequence number.
    pub sequence: u64,
    /// **Milestone 12 hook** — basic biped robot framework.
    ///
    /// The entity this session directly controls. Actor-scoped detectors
    /// (ammo, per-avatar fire rate) use it when present; until Milestone 12
    /// binds an avatar to a session this is `None` and those detectors fall
    /// back to session-scoped tracking keyed on [`EntityId::null`].
    pub avatar_entity: Option<EntityId>,
    /// Read-only authoritative world facts.
    pub world: &'a dyn WorldView,
}

impl<'a> InspectionContext<'a> {
    /// Convenience constructor for the common server call site.
    pub fn new(
        session_id: SessionId,
        player_id: PlayerId,
        faction_id: FactionId,
        server_tick: SimTick,
        client_tick: SimTick,
        sequence: u64,
        world: &'a dyn WorldView,
    ) -> Self {
        InspectionContext {
            session_id,
            player_id,
            faction_id,
            server_tick,
            client_tick,
            sequence,
            avatar_entity: None,
            world,
        }
    }

    pub fn with_avatar(mut self, avatar: EntityId) -> Self {
        self.avatar_entity = Some(avatar);
        self
    }

    /// Entity used as the actor key for actor-scoped telemetry.
    pub fn actor_key(&self) -> EntityId {
        self.avatar_entity.unwrap_or_else(EntityId::null)
    }
}

/// The complete anti-cheat integration boundary.
///
/// **Server code calls only this trait.** No gameplay crate references any
/// concrete provider, and no provider is allowed to mutate simulation state:
/// the provider observes the world through [`WorldView`] and answers with a
/// [`Verdict`]. That is what makes it possible to drop in an SDK-backed
/// implementation (see `crate::eos`) without touching gameplay code.
///
/// The methods `initialize`, `begin_session`, `end_session`, `poll`,
/// `player_status` and `report_event` are the interface named in the master
/// spec (§9). The remaining methods extend it with the command inspection and
/// verdict plumbing the spec's validation list requires.
///
/// Providers must be `Send` so the threaded dedicated server can own one inside
/// its simulation thread.
pub trait AntiCheatProvider: Send {
    /// Short stable provider name for logs and telemetry (`"null"`, `"basic"`, ...).
    fn name(&self) -> &'static str;

    /// Bring the provider up. Must be called before any session method.
    fn initialize(&mut self) -> AntiCheatResult<()>;

    /// Tear the provider down. Idempotent.
    fn shutdown(&mut self) {}

    // ---- session lifecycle -------------------------------------------------

    /// Phase 1 — a client has completed the transport handshake and is connecting.
    ///
    /// Returning `Err` refuses the connection before a session is created.
    fn on_client_connecting(
        &mut self,
        _session: SessionId,
        _client_name: &str,
    ) -> AntiCheatResult<()> {
        Ok(())
    }

    /// Phase 2 — register the session and bind it to a player identity.
    ///
    /// Platform identity (Epic/Steam account) is intentionally *not* passed
    /// here: the provider only ever sees the internal [`PlayerId`]. An
    /// SDK-backed provider maintains its own platform-id mapping internally.
    fn begin_session(&mut self, player: PlayerId, session: SessionId) -> AntiCheatResult<()>;

    /// Phase 3 — validate the client's build/protocol/content manifest.
    ///
    /// Called when the client submits its manifest. Returning `Err` means the
    /// server should refuse the session.
    fn verify_client_manifest(
        &mut self,
        _session: SessionId,
        _manifest: &BuildManifest,
    ) -> AntiCheatResult<()> {
        Ok(())
    }

    /// Phase 4 — the session is authenticated and bound to a faction. Trust
    /// tracking starts here.
    fn on_client_authenticated(
        &mut self,
        _session: SessionId,
        _player: PlayerId,
        _faction: FactionId,
        _tick: SimTick,
    ) {
    }

    /// Phase 5 — the player left. Releases all per-session state.
    fn end_session(&mut self, player: PlayerId);

    /// Phase 5 (by session id) — used when only the session is known.
    fn end_session_by_id(&mut self, _session: SessionId) {}

    /// Per-tick servicing hook. SDK-backed providers pump their callback queue here.
    fn poll(&mut self);

    // ---- inspection --------------------------------------------------------

    /// Inspect one command envelope and decide what the server should do with it.
    ///
    /// Must be side-effect free with respect to simulation state.
    fn inspect_command(&mut self, ctx: &InspectionContext<'_>, command: &Command) -> Verdict;

    /// Submit an externally detected security event (for example an admin
    /// permission denial raised by the server's own authorization check).
    fn report_event(&mut self, event: SecurityEvent);

    // ---- queries and enforcement ------------------------------------------

    /// Externally visible standing of a player.
    fn player_status(&self, player: PlayerId) -> AntiCheatStatus;

    /// Internal trust level of a session.
    fn session_trust(&self, _session: SessionId) -> TrustLevel {
        TrustLevel::Trusted
    }

    /// Drain sessions the provider has decided must be removed.
    ///
    /// The server, not the provider, performs the disconnection, so enforcement
    /// stays in one place.
    fn drain_pending_kicks(&mut self) -> Vec<(SessionId, EnforcementReason)> {
        Vec::new()
    }

    /// Administrative trust override, gated by
    /// [`crate::admin::AdminPermission::SetTrustLevel`] at the call site.
    fn apply_admin_trust_override(&mut self, _session: SessionId, _level: TrustLevel) {}

    /// The provider's security telemetry sink, if it keeps one.
    fn security_log(&self) -> Option<&SecurityLog> {
        None
    }
}

/// Which provider a server should construct.
///
/// Kept `Copy + Debug` so it can live inside a server config struct; the
/// concrete provider is built from it at startup.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub enum AntiCheatMode {
    /// No anti-cheat. The default for local development and single-player.
    #[default]
    Disabled,
    /// Internal heuristic provider. Requires no proprietary SDK.
    Basic,
}

impl AntiCheatMode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            AntiCheatMode::Disabled => "disabled",
            AntiCheatMode::Basic => "basic",
        }
    }

    /// Parse a command-line value.
    pub fn parse(value: &str) -> Option<AntiCheatMode> {
        match value {
            "off" | "none" | "disabled" => Some(AntiCheatMode::Disabled),
            "basic" | "on" => Some(AntiCheatMode::Basic),
            _ => None,
        }
    }

    /// Build the provider this mode names.
    pub fn create_provider(
        &self,
        policy: ServerPolicy,
        server_manifest: BuildManifest,
    ) -> Box<dyn AntiCheatProvider> {
        match self {
            AntiCheatMode::Disabled => Box::new(crate::null::NullAntiCheat),
            AntiCheatMode::Basic => Box::new(crate::basic::BasicAntiCheat::with_policy(
                policy,
                server_manifest,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_verdict_allow_and_observe_still_reach_authoritative_validation() {
        assert!(Verdict::Allow.allows_command());
        assert!(Verdict::Observe(EnforcementReason::ImpossibleMovement).allows_command());
        assert!(!Verdict::Reject(EnforcementReason::UnauthorizedOrder).allows_command());
        assert!(!Verdict::Kick(EnforcementReason::ManifestMismatch).allows_command());
    }

    #[test]
    fn test_verdict_reason_extraction() {
        assert_eq!(Verdict::Allow.reason(), None);
        assert_eq!(
            Verdict::Reject(EnforcementReason::AmmoInconsistency).reason(),
            Some(EnforcementReason::AmmoInconsistency)
        );
    }

    #[test]
    fn test_status_projection_from_trust_level() {
        assert_eq!(
            AntiCheatStatus::from_trust(TrustLevel::Trusted),
            AntiCheatStatus::Clean
        );
        assert_eq!(
            AntiCheatStatus::from_trust(TrustLevel::Flagged),
            AntiCheatStatus::UnderReview
        );
        assert_eq!(
            AntiCheatStatus::from_trust(TrustLevel::Banned),
            AntiCheatStatus::Banned
        );
    }

    #[test]
    fn test_anti_cheat_mode_parsing_and_provider_construction() {
        assert_eq!(AntiCheatMode::parse("off"), Some(AntiCheatMode::Disabled));
        assert_eq!(AntiCheatMode::parse("basic"), Some(AntiCheatMode::Basic));
        assert_eq!(AntiCheatMode::parse("eac"), None);

        let manifest = BuildManifest::new("test", 1, 0);
        assert_eq!(
            AntiCheatMode::Disabled
                .create_provider(ServerPolicy::LocalDev, manifest.clone())
                .name(),
            "null"
        );
        assert_eq!(
            AntiCheatMode::Basic
                .create_provider(ServerPolicy::LocalDev, manifest)
                .name(),
            "basic"
        );
    }

    #[test]
    fn test_sdk_unavailable_error_is_explicit() {
        let err = AntiCheatError::SdkUnavailable {
            sdk: "EOS/EAC",
            detail: "set RTS_EOS_SDK_PATH",
        };
        let msg = err.to_string();
        assert!(msg.contains("EOS/EAC"), "{msg}");
        assert!(msg.contains("RTS_EOS_SDK_PATH"), "{msg}");
    }
}
