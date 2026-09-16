//! EOS / Easy Anti-Cheat adapter boundary.
//!
//! # There are no SDK bindings in this file, on purpose
//!
//! Milestone 25 instructs us to create an isolated FFI crate **only if the
//! EAC/EOS SDK is genuinely available and licensed in the build environment**.
//! It is not: the Epic Online Services SDK is proprietary, is not vendored in
//! this repository, and cannot be redistributed with it. Writing plausible
//! `extern "C"` declarations against headers we do not have would produce code
//! that compiles, looks protective, and protects nothing — the worst possible
//! outcome for a security component.
//!
//! So this module is the *shape* of the adapter and nothing more:
//!
//! * It is behind the off-by-default `eos-eac` cargo feature.
//! * It compiles and passes clippy with the feature enabled, so the boundary
//!   never bit-rots.
//! * [`EosEacAdapter::initialize`] returns an explicit, actionable
//!   [`AntiCheatError::SdkUnavailable`] instead of silently succeeding.
//! * Every other method delegates to [`BasicAntiCheat`], because that is exactly
//!   how the real adapter is meant to work: EAC covers process and binary
//!   integrity, while the internal heuristics keep covering gameplay-state
//!   anomalies. The two layers are complementary, not alternatives.
//!
//! # Wiring a real SDK in
//!
//! 1. Obtain an EOS/EAC licence and product credentials from Epic.
//! 2. Create a **separate** crate, `crates/eos-sys`, containing only the raw
//!    `extern "C"` bindings and the build script that links the SDK. Nothing
//!    else in the workspace may depend on it.
//! 3. Add `eos-sys` as an optional dependency of *this* crate, gated on the
//!    `eos-eac` feature. The dependency edge is
//!    `anti-cheat -> eos-sys`, never `sim-core -> eos-sys` or
//!    `game-types -> eos-sys`.
//! 4. Replace the bodies of [`EosEacAdapter::initialize`],
//!    `register_peer`/`unregister_peer` and the poll drain with SDK calls. The
//!    trait surface does not change, so no gameplay or protocol code is touched.
//!
//! See `docs/SECURITY.md` for the full contract and threat model.

use crate::basic::BasicAntiCheat;
use crate::event::SecurityEvent;
use crate::manifest::{BuildManifest, ServerPolicy};
use crate::provider::{
    AntiCheatError, AntiCheatProvider, AntiCheatResult, AntiCheatStatus, EnforcementReason,
    InspectionContext, Verdict,
};
use crate::trust::TrustLevel;
use game_types::{FactionId, PlayerId, SessionId, SimTick};
use sim_core::command::Command;

/// Environment variable an operator sets to point at a licensed SDK.
pub const EOS_SDK_PATH_ENV: &str = "RTS_EOS_SDK_PATH";

/// Explanation returned by [`EosEacAdapter::initialize`] in this build.
pub const SDK_UNAVAILABLE_DETAIL: &str = concat!(
    "no EOS/EAC SDK is linked into this build. The SDK is proprietary and is not ",
    "vendored in this repository. To enable it: obtain an Epic Online Services licence, ",
    "add a crates/eos-sys bindings crate, make it an optional dependency of the anti-cheat ",
    "crate gated on the `eos-eac` feature, and set ",
    "RTS_EOS_SDK_PATH. Until then run with --anti-cheat basic, which needs no SDK."
);

/// Credentials a real EOS/EAC deployment requires.
///
/// Carried here so the configuration surface is already correct when an SDK
/// arrives. Nothing reads these fields today.
#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub struct EosEacConfig {
    pub product_id: String,
    pub sandbox_id: String,
    pub deployment_id: String,
    /// Server-side client credentials. Never logged.
    pub client_id: String,
}

/// EOS / Easy Anti-Cheat provider adapter.
///
/// Inert without a linked SDK: `initialize` fails loudly and the adapter never
/// claims a session is protected when it is not.
pub struct EosEacAdapter {
    config: EosEacConfig,
    /// Internal heuristic layer. A real EAC integration runs *alongside* this,
    /// not instead of it.
    inner: BasicAntiCheat,
    sdk_linked: bool,
}

impl EosEacAdapter {
    /// Build the adapter. Construction always succeeds; only
    /// [`EosEacAdapter::initialize`] can fail.
    pub fn new(config: EosEacConfig, policy: ServerPolicy, server_manifest: BuildManifest) -> Self {
        EosEacAdapter {
            config,
            inner: BasicAntiCheat::with_policy(policy, server_manifest),
            // Flipped to true only by a real `eos-sys` handshake. There is no
            // code path in this build that sets it.
            sdk_linked: false,
        }
    }

    pub fn config(&self) -> &EosEacConfig {
        &self.config
    }

    /// Whether a licensed SDK is actually linked and handshaken.
    ///
    /// Always `false` in this build. Callers must treat `false` as "this server
    /// has no EAC protection", never as "probably fine".
    pub fn is_sdk_linked(&self) -> bool {
        self.sdk_linked
    }

    /// The error every SDK-dependent entry point returns in this build.
    pub const fn sdk_unavailable() -> AntiCheatError {
        AntiCheatError::SdkUnavailable {
            sdk: "EOS/EAC",
            detail: SDK_UNAVAILABLE_DETAIL,
        }
    }
}

impl AntiCheatProvider for EosEacAdapter {
    fn name(&self) -> &'static str {
        "eos-eac"
    }

    fn initialize(&mut self) -> AntiCheatResult<()> {
        if !self.sdk_linked {
            return Err(Self::sdk_unavailable());
        }
        self.inner.initialize()
    }

    fn shutdown(&mut self) {
        self.inner.shutdown();
    }

    fn on_client_connecting(
        &mut self,
        session: SessionId,
        client_name: &str,
    ) -> AntiCheatResult<()> {
        // A real adapter registers the peer with the EAC server module here.
        if !self.sdk_linked {
            return Err(Self::sdk_unavailable());
        }
        self.inner.on_client_connecting(session, client_name)
    }

    fn begin_session(&mut self, player: PlayerId, session: SessionId) -> AntiCheatResult<()> {
        if !self.sdk_linked {
            return Err(Self::sdk_unavailable());
        }
        self.inner.begin_session(player, session)
    }

    fn verify_client_manifest(
        &mut self,
        session: SessionId,
        manifest: &BuildManifest,
    ) -> AntiCheatResult<()> {
        self.inner.verify_client_manifest(session, manifest)
    }

    fn on_client_authenticated(
        &mut self,
        session: SessionId,
        player: PlayerId,
        faction: FactionId,
        tick: SimTick,
    ) {
        self.inner
            .on_client_authenticated(session, player, faction, tick);
    }

    fn end_session(&mut self, player: PlayerId) {
        self.inner.end_session(player);
    }

    fn end_session_by_id(&mut self, session: SessionId) {
        self.inner.end_session_by_id(session);
    }

    fn poll(&mut self) {
        // A real adapter drains the EAC callback queue here and converts any
        // client violation callbacks into `SecurityEvent`s.
        self.inner.poll();
    }

    fn inspect_command(&mut self, ctx: &InspectionContext<'_>, command: &Command) -> Verdict {
        // Gameplay-state inspection is SDK-independent and always runs.
        self.inner.inspect_command(ctx, command)
    }

    fn report_event(&mut self, event: SecurityEvent) {
        self.inner.report_event(event);
    }

    fn player_status(&self, player: PlayerId) -> AntiCheatStatus {
        if !self.sdk_linked {
            // Never claim a clean bill of health we cannot back up.
            return AntiCheatStatus::Unknown;
        }
        self.inner.player_status(player)
    }

    fn session_trust(&self, session: SessionId) -> TrustLevel {
        self.inner.session_trust(session)
    }

    fn drain_pending_kicks(&mut self) -> Vec<(SessionId, EnforcementReason)> {
        self.inner.drain_pending_kicks()
    }

    fn apply_admin_trust_override(&mut self, session: SessionId, level: TrustLevel) {
        self.inner.apply_admin_trust_override(session, level);
    }

    fn security_log(&self) -> Option<&crate::event::SecurityLog> {
        self.inner.security_log()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapter() -> EosEacAdapter {
        EosEacAdapter::new(
            EosEacConfig::default(),
            ServerPolicy::LocalDev,
            BuildManifest::new("test", 1, 0),
        )
    }

    #[test]
    fn test_adapter_reports_no_linked_sdk_in_this_environment() {
        assert!(!adapter().is_sdk_linked());
    }

    #[test]
    fn test_initialize_fails_with_an_explicit_actionable_error() {
        let mut a = adapter();
        let err = a.initialize().expect_err("must not pretend to initialize");
        assert!(matches!(err, AntiCheatError::SdkUnavailable { .. }));
        let msg = err.to_string();
        assert!(msg.contains("EOS/EAC"), "{msg}");
        assert!(msg.contains(EOS_SDK_PATH_ENV), "{msg}");
        assert!(msg.contains("--anti-cheat basic"), "{msg}");
    }

    #[test]
    fn test_session_entry_points_refuse_rather_than_silently_succeed() {
        let mut a = adapter();
        assert!(matches!(
            a.on_client_connecting(SessionId::new(1), "x"),
            Err(AntiCheatError::SdkUnavailable { .. })
        ));
        assert!(matches!(
            a.begin_session(PlayerId::new(1), SessionId::new(1)),
            Err(AntiCheatError::SdkUnavailable { .. })
        ));
    }

    #[test]
    fn test_unlinked_adapter_never_claims_a_player_is_clean() {
        let a = adapter();
        assert_eq!(
            a.player_status(PlayerId::new(1)),
            AntiCheatStatus::Unknown,
            "an unlinked adapter must not vouch for anyone"
        );
    }

    #[test]
    fn test_adapter_name_is_distinct_from_the_internal_provider() {
        assert_eq!(adapter().name(), "eos-eac");
    }
}
