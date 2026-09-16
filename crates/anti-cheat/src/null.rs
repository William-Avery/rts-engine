use crate::event::SecurityEvent;
use crate::provider::{
    AntiCheatProvider, AntiCheatResult, AntiCheatStatus, InspectionContext, Verdict,
};
use crate::trust::TrustLevel;
use game_types::{PlayerId, SessionId};
use sim_core::command::Command;

/// Anti-cheat disabled.
///
/// This is the default for local development and single-player. It is a
/// zero-sized type holding no state, every method is a constant-time no-op, and
/// [`NullAntiCheat::inspect_command`] always answers [`Verdict::Allow`], so the
/// server's command path behaves exactly as it did before anti-cheat existed.
///
/// Turning anti-cheat off changes *nothing* about gameplay: the server's own
/// authoritative validation is unaffected and still rejects illegal commands.
/// Anti-cheat is defence in depth, never the only line of defence.
#[derive(Copy, Clone, Default, PartialEq, Eq, Debug)]
pub struct NullAntiCheat;

impl AntiCheatProvider for NullAntiCheat {
    fn name(&self) -> &'static str {
        "null"
    }

    fn initialize(&mut self) -> AntiCheatResult<()> {
        Ok(())
    }

    fn begin_session(&mut self, _player: PlayerId, _session: SessionId) -> AntiCheatResult<()> {
        Ok(())
    }

    fn end_session(&mut self, _player: PlayerId) {}

    fn poll(&mut self) {}

    fn inspect_command(&mut self, _ctx: &InspectionContext<'_>, _command: &Command) -> Verdict {
        Verdict::Allow
    }

    fn report_event(&mut self, _event: SecurityEvent) {}

    fn player_status(&self, _player: PlayerId) -> AntiCheatStatus {
        AntiCheatStatus::Unknown
    }

    fn session_trust(&self, _session: SessionId) -> TrustLevel {
        TrustLevel::Trusted
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_view::StaticWorldView;
    use game_types::{EntityId, FactionId, SimTick};

    fn ctx<'a>(world: &'a StaticWorldView) -> InspectionContext<'a> {
        InspectionContext::new(
            SessionId::new(1),
            PlayerId::new(1),
            FactionId::new(1),
            SimTick::new(10),
            SimTick::new(10),
            1,
            world,
        )
    }

    #[test]
    fn test_null_provider_is_zero_sized_and_allocation_free() {
        assert_eq!(std::mem::size_of::<NullAntiCheat>(), 0);
    }

    #[test]
    fn test_null_provider_allows_every_command_including_blatant_cheats() {
        let world = StaticWorldView::new().with_entity(EntityId::new(9), FactionId::new(2));
        let mut provider = NullAntiCheat;
        provider.initialize().unwrap();
        let cheats = [
            Command::Move {
                position: (f32::NAN, 0.0, 0.0),
                velocity: (99_999.0, 0.0, 0.0),
            },
            Command::TransferResource {
                from_entity: EntityId::new(9),
                to_entity: EntityId::new(1),
                resource_id: game_types::ResourceId::new(1),
                amount: u32::MAX,
            },
        ];
        for command in &cheats {
            assert_eq!(
                provider.inspect_command(&ctx(&world), command),
                Verdict::Allow
            );
        }
    }

    #[test]
    fn test_null_provider_lifecycle_is_infallible_and_stateless() {
        let mut provider = NullAntiCheat;
        assert!(provider.initialize().is_ok());
        assert!(
            provider
                .on_client_connecting(SessionId::new(1), "dev")
                .is_ok()
        );
        assert!(
            provider
                .begin_session(PlayerId::new(1), SessionId::new(1))
                .is_ok()
        );
        provider.on_client_authenticated(
            SessionId::new(1),
            PlayerId::new(1),
            FactionId::new(1),
            SimTick::zero(),
        );
        provider.poll();
        provider.end_session(PlayerId::new(1));
        provider.shutdown();
        assert_eq!(provider, NullAntiCheat);
    }

    #[test]
    fn test_null_provider_never_kicks_and_keeps_no_log() {
        let mut provider = NullAntiCheat;
        assert!(provider.drain_pending_kicks().is_empty());
        assert!(provider.security_log().is_none());
        assert_eq!(
            provider.player_status(PlayerId::new(1)),
            AntiCheatStatus::Unknown
        );
        assert_eq!(
            provider.session_trust(SessionId::new(1)),
            TrustLevel::Trusted
        );
    }
}
