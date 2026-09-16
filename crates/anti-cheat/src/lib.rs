//! Anti-cheat and security plumbing for the RTS engine.
//!
//! # Position in the dependency graph
//!
//! ```text
//!   game-types  <--  sim-core  <--  anti-cheat  <--  game-protocol  <--  dedicated-server
//! ```
//!
//! The arrows only point one way. `game-types` and `sim-core` — the gameplay
//! crates — do not depend on this crate, and therefore can never depend on any
//! anti-cheat SDK. The server consumes anti-cheat exclusively through the
//! [`AntiCheatProvider`] trait, so swapping the provider (including for a future
//! EAC-backed one) touches no gameplay code.
//!
//! # Layers
//!
//! 1. **The authoritative server is the first anti-cheat layer.** Every command
//!    is validated server-side regardless of which provider is installed.
//!    Anti-cheat is defence in depth, never the only line of defence — with
//!    [`NullAntiCheat`] the server still rejects illegal commands.
//! 2. **The internal heuristic layer** ([`BasicAntiCheat`]) compares client
//!    claims against authoritative state. No SDK, no external service.
//! 3. **A platform anti-cheat layer** (see [`eos`], feature-gated) would add
//!    process and binary integrity. It is not present in this build.
//!
//! # Determinism
//!
//! Security telemetry lives in [`SecurityLog`], deliberately *outside* the
//! deterministic simulation journal. Installing, removing or changing an
//! anti-cheat provider therefore cannot perturb simulation state or replay
//! hashes for legitimate input.

pub mod admin;
pub mod basic;
pub mod detectors;
#[cfg(feature = "eos-eac")]
pub mod eos;
pub mod event;
pub mod manifest;
pub mod null;
pub mod provider;
pub mod trust;
pub mod world_view;

pub use admin::*;
pub use basic::*;
pub use detectors::*;
#[cfg(feature = "eos-eac")]
pub use eos::*;
pub use event::*;
pub use manifest::*;
pub use null::*;
pub use provider::*;
pub use trust::*;
pub use world_view::*;

#[cfg(test)]
mod tests {
    use super::*;
    use game_types::{EntityId, FactionId, PlayerId, RegionId, ResourceId, SessionId, SimTick};
    use sim_core::command::Command;
    use sim_core::inventory::ContainerKind;
    use sim_core::world::WorldState;

    const SESSION: SessionId = SessionId::new(1);
    const PLAYER: PlayerId = PlayerId::new(1);
    const OWN: FactionId = FactionId::new(1);
    const ENEMY: FactionId = FactionId::new(2);

    fn provider() -> BasicAntiCheat {
        let mut p = BasicAntiCheat::new();
        p.initialize().unwrap();
        p.begin_session(PLAYER, SESSION).unwrap();
        p.on_client_authenticated(SESSION, PLAYER, OWN, SimTick::zero());
        p
    }

    /// Acceptance: the basic provider works against the real simulation state
    /// with no proprietary SDK involved.
    #[test]
    fn test_acceptance_basic_provider_inspects_real_sim_state_without_sdk() {
        let mut sim = WorldState::new();
        let own = sim.create_entity(OWN, RegionId::new(1));
        let enemy = sim.create_entity(ENEMY, RegionId::new(1));
        sim.create_container(own, ContainerKind::Backpack);
        if let Some(inv) = sim.inventory_mut(own) {
            inv.add(ResourceId::new(10), 40).unwrap();
        }

        let mut p = provider();

        // Commanding your own unit is fine.
        let ctx = InspectionContext::new(
            SESSION,
            PLAYER,
            OWN,
            SimTick::new(5),
            SimTick::new(5),
            1,
            &sim,
        );
        assert_eq!(
            p.inspect_command(
                &ctx,
                &Command::TransferRegion {
                    entity_id: own,
                    destination_region: RegionId::new(1)
                }
            ),
            Verdict::Allow
        );

        // Commanding an enemy unit is not.
        let ctx = InspectionContext::new(
            SESSION,
            PLAYER,
            OWN,
            SimTick::new(6),
            SimTick::new(6),
            2,
            &sim,
        );
        assert_eq!(
            p.inspect_command(
                &ctx,
                &Command::TransferRegion {
                    entity_id: enemy,
                    destination_region: RegionId::new(1)
                }
            ),
            Verdict::Reject(EnforcementReason::UnauthorizedOrder)
        );
    }

    /// Acceptance: resource duplication attempts are caught against real
    /// inventory balances.
    #[test]
    fn test_acceptance_resource_duplication_detected_against_real_inventory() {
        let mut sim = WorldState::new();
        let src = sim.create_entity(OWN, RegionId::new(1));
        let dst = sim.create_entity(OWN, RegionId::new(1));
        sim.create_container(src, ContainerKind::Backpack);
        sim.create_container(dst, ContainerKind::Backpack);
        if let Some(inv) = sim.inventory_mut(src) {
            inv.add(ResourceId::new(10), 12).unwrap();
        }

        let mut p = provider();
        let ctx = InspectionContext::new(
            SESSION,
            PLAYER,
            OWN,
            SimTick::new(5),
            SimTick::new(5),
            1,
            &sim,
        );
        let verdict = p.inspect_command(
            &ctx,
            &Command::TransferResource {
                from_entity: src,
                to_entity: dst,
                resource_id: ResourceId::new(10),
                amount: 100_000,
            },
        );
        assert_eq!(
            verdict,
            Verdict::Reject(EnforcementReason::ImpossibleEconomyDelta)
        );
    }

    /// Milestone 14 acceptance test: WorldView::faction_knows_entity authoritatively
    /// catches and rejects attack commands against targets hidden in fog.
    #[test]
    fn test_acceptance_hidden_target_hook_is_wired_and_active_in_milestone_14() {
        let mut sim = WorldState::new();
        let enemy = sim.create_entity(ENEMY, RegionId::new(1));
        assert_eq!(
            WorldView::faction_knows_entity(&sim, OWN, enemy),
            KnowledgeQuery::Unknown,
            "M14 overrides WorldView::faction_knows_entity to return Unknown for hidden enemies"
        );

        let mut p = provider();
        let ctx = InspectionContext::new(
            SESSION,
            PLAYER,
            OWN,
            SimTick::new(5),
            SimTick::new(5),
            1,
            &sim,
        );
        // Authoritatively caught: targeting an unknown enemy in fog is rejected!
        assert_eq!(
            p.inspect_command(
                &ctx,
                &Command::Action {
                    action_type: sim_core::command::ActionType::Attack,
                    target: Some(enemy),
                }
            ),
            Verdict::Reject(EnforcementReason::HiddenTargetAttempt)
        );

        // With a knowledge system present the same command is caught.
        let world = StaticWorldView::new()
            .with_entity(enemy, ENEMY)
            .with_knowledge_system();
        let ctx = InspectionContext::new(
            SESSION,
            PLAYER,
            OWN,
            SimTick::new(30),
            SimTick::new(30),
            2,
            &world,
        );
        assert_eq!(
            p.inspect_command(
                &ctx,
                &Command::Action {
                    action_type: sim_core::command::ActionType::Attack,
                    target: Some(enemy),
                }
            ),
            Verdict::Reject(EnforcementReason::HiddenTargetAttempt)
        );
    }

    /// The provider trait is object safe and `Send`, so the threaded dedicated
    /// server can own one inside its simulation thread.
    #[test]
    fn test_provider_trait_is_object_safe_and_send() {
        fn assert_send<T: Send>(_: &T) {}
        let providers: Vec<Box<dyn AntiCheatProvider>> = vec![
            Box::new(NullAntiCheat),
            Box::new(BasicAntiCheat::new()),
            #[cfg(feature = "eos-eac")]
            Box::new(eos::EosEacAdapter::new(
                eos::EosEacConfig::default(),
                ServerPolicy::LocalDev,
                BuildManifest::new("test", 1, 0),
            )),
        ];
        assert_send(&providers);
        let names: Vec<&str> = providers.iter().map(|p| p.name()).collect();
        assert!(names.contains(&"null"));
        assert!(names.contains(&"basic"));
    }

    /// Admin permission enforcement is independent of the anti-cheat provider:
    /// it is a plain server-side authorization check.
    #[test]
    fn test_admin_authorization_is_independent_of_the_provider() {
        let mut registry = AdminRegistry::new();
        let command = Command::AdminGrantResource {
            target_entity: EntityId::new(1),
            resource_id: ResourceId::new(1),
            amount: 9_999,
        };
        let permission = required_admin_permission(&command).unwrap();
        assert_eq!(
            registry.authorize(SESSION, permission),
            Err(game_types::GameError::PermissionDenied)
        );
        registry.set_role(SESSION, AdminRole::Host);
        assert!(registry.authorize(SESSION, permission).is_ok());
    }
}
