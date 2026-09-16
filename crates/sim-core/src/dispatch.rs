//! The single authoritative command dispatcher.
//!
//! Before this module existed the dispatcher was triplicated across
//! `game-protocol::server`, `game-protocol::threaded` and the sim-core test
//! harness, and the three had already diverged: the harness copy was missing
//! two Milestone 10 commands, so sim-core's own tests exercised a different
//! command path than the server ran. All three ended in `_ => {}`, so a new
//! `Command` variant silently no-opped.
//!
//! [`apply_command`] is now the only implementation, and its match is
//! **exhaustive with no catch-all**: a new `Command` variant fails to compile
//! until it is either handled or explicitly refused here.
//!
//! Two invariants this module enforces that nothing enforced before:
//!
//! * **Faction comes from the actor, never from the payload.** Every command is
//!   evaluated against [`ActorContext::faction_id`], which the server derived
//!   from the session it issued at handshake.
//! * **Every rejection is a typed [`GameError`]**, returned to the caller so it
//!   can be surfaced to the client and counted, rather than dropped on the floor.

use crate::command::{Command, RobotCommandType};
use crate::event::SimEvent;
use crate::logistics::JobPriority;
use crate::structure::BuildRequest;
use crate::terrain::validate_authoritative_movement;
use crate::world::{SessionDirective, WorldState};
use game_types::{EntityId, FactionId, GameError, GameResult, PlayerId, RegionId, SessionId};

/// Privilege level the session layer has already established for an actor.
///
/// This is *not* a second permission system. `anti-cheat`'s `AdminRegistry` is
/// the single authorization choke point and it runs at ingress, before a
/// command is ever buffered; this newtype carries the resulting
/// `AdminRole::code()` into the simulation so the dispatcher can refuse a
/// privileged command that somehow reached it without that check. `sim-core`
/// must not depend on `anti-cheat` (the dependency edge runs the other way), so
/// the role arrives as its stable wire code rather than as the enum.
#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
#[repr(transparent)]
pub struct AdminRoleCode(pub u8);

impl AdminRoleCode {
    /// An ordinary player: no privileged capability whatsoever.
    pub const PLAYER: AdminRoleCode = AdminRoleCode(0);

    pub const fn new(code: u8) -> Self {
        AdminRoleCode(code)
    }

    pub const fn value(&self) -> u8 {
        self.0
    }

    /// Whether the session layer marked this actor as holding any admin role.
    pub const fn is_privileged(&self) -> bool {
        self.0 > 0
    }
}

/// Everything the dispatcher is allowed to know about who is acting.
///
/// Built by the server from the session it issued. No field is ever read from a
/// command payload.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub struct ActorContext {
    /// The session the server issued at handshake.
    pub session_id: SessionId,
    /// Internal player identity bound to that session.
    pub player_id: PlayerId,
    /// The one faction this actor may command.
    pub faction_id: FactionId,
    /// The actor's authoritative avatar entity, used as the builder and as the
    /// source of the authoritative position for reach checks.
    pub avatar_entity: EntityId,
    /// Admin role the session layer established for this session.
    pub admin_role: AdminRoleCode,
}

impl ActorContext {
    pub fn new(session_id: SessionId, player_id: PlayerId, faction_id: FactionId) -> Self {
        ActorContext {
            session_id,
            player_id,
            faction_id,
            avatar_entity: EntityId::null(),
            admin_role: AdminRoleCode::PLAYER,
        }
    }

    pub fn with_avatar(mut self, avatar_entity: EntityId) -> Self {
        self.avatar_entity = avatar_entity;
        self
    }

    pub fn with_admin_role(mut self, admin_role: AdminRoleCode) -> Self {
        self.admin_role = admin_role;
        self
    }

    /// Resolve (creating on first use) the actor's avatar entity in `world`.
    pub fn resolve_avatar(mut self, world: &mut WorldState, region_id: RegionId) -> Self {
        self.avatar_entity = world.ensure_player_avatar(self.player_id, self.faction_id, region_id);
        self
    }
}

/// Default spawn region for a newly connected commander.
pub const DEFAULT_PLAYER_REGION: RegionId = RegionId::new(1);

/// World bounds used for structure placement validation.
fn build_bounds(world: &WorldState) -> (f32, f32, f32, f32) {
    world.world_bounds_xz()
}

/// Whether every component of a position or velocity is a real number.
///
/// NaN passes every bounds comparison as `false` and `NaN.floor() as i32`
/// saturates to 0, so a non-finite coordinate must be refused at the boundary
/// rather than clamped.
fn is_finite3(v: (f32, f32, f32)) -> bool {
    v.0.is_finite() && v.1.is_finite() && v.2.is_finite()
}

/// Apply one client command under full server authority.
///
/// Returns `Ok(())` when the command was applied, and a typed [`GameError`]
/// describing exactly why otherwise. The match is exhaustive on purpose: adding
/// a `Command` variant without deciding what the server does with it is a
/// compile error, not a silent no-op.
pub fn apply_command(
    world: &mut WorldState,
    actor: &ActorContext,
    command: &Command,
) -> GameResult<()> {
    // A null faction is server/internal authority and bypasses every ownership
    // check below. A command that arrived over the network must never carry it.
    if actor.faction_id.is_null() {
        return Err(GameError::PermissionDenied);
    }

    let tick = world.tick;

    match *command {
        // ------------------------------------------------------------ movement
        Command::Move { position, .. } => {
            // A non-finite coordinate is refused before any arithmetic touches
            // it: NaN makes every comparison in the clamp evaluate to `false`.
            if !is_finite3(position) {
                return Err(GameError::InvalidPosition);
            }
            let presence = world
                .robot_registry
                .player(actor.player_id)
                .ok_or(GameError::PermissionDenied)?;
            let previous = presence.position;

            // The server owns the final position. The client reports intent; it
            // is clamped to a physically reachable step and resolved against the
            // authoritative collision world before anything is stored.
            //
            // The travel allowance covers the ticks actually elapsed since this
            // player's last accepted update, capped at one second, so an honest
            // client whose packets were delayed is not clamped as if it had only
            // ever had a single frame. This matches the allowance
            // `RobotRegistry::apply_player_move` grants, which runs immediately
            // after and applies the speed clamp a second time.
            let elapsed = tick
                .value()
                .saturating_sub(presence.last_update_tick.value())
                .clamp(1, 30);
            let dt = world.robot_registry.config.fixed_step_seconds * elapsed as f32;
            let (accepted, _illegal) = validate_authoritative_movement(
                previous,
                position,
                dt,
                &world.terrain,
                &world.movement_config,
            );
            world
                .robot_registry
                .apply_player_move(actor.player_id, accepted, tick)?;
            Ok(())
        }

        // -------------------------------------------------------------- combat
        // Weapons, damage, armour and projectiles are Milestone 13. Until the
        // server can authoritatively resolve a discharge there is nothing
        // honest to do with this command, so it is refused rather than silently
        // dropped: a client that sends it learns the server did not act.
        Command::Action { .. } => Err(GameError::InvalidCommand),

        // ---------------------------------------------------------- structures
        Command::BuildStructure {
            kind,
            position,
            rotation_deg,
        } => {
            if !is_finite3(position) {
                return Err(GameError::InvalidPosition);
            }
            let builder = actor.avatar_entity;
            if builder.is_null() {
                return Err(GameError::PermissionDenied);
            }
            // The reach check is only meaningful against the builder's real
            // authoritative position. Passing the requested position as the
            // player position (as the old server did) made `dist` always 0.
            let player_pos = world
                .robot_registry
                .player(actor.player_id)
                .ok_or(GameError::PermissionDenied)?
                .position;
            let region_id = world
                .entity_registry
                .get(builder)
                .map(|e| e.region_id)
                .unwrap_or(DEFAULT_PLAYER_REGION);
            let world_bounds_xz = build_bounds(world);
            let request = BuildRequest {
                player_pos,
                requested_pos: position,
                kind,
                rotation_deg,
                faction_id: actor.faction_id,
                region_id,
                creation_tick: tick,
                world_bounds_xz,
            };

            let WorldState {
                inventory_registry,
                structure_registry,
                ..
            } = world;
            // Construction costs resources. A builder with no container at all
            // cannot pay, so it cannot build: `None` here used to skip both the
            // affordability pre-check and the deduction entirely.
            let inventory = inventory_registry
                .get_mut(builder)
                .ok_or(GameError::ContainerNotFound(builder))?;
            structure_registry.request_build(request, Some(inventory))?;
            Ok(())
        }

        Command::DismantleStructure { structure_id } => world
            .structure_registry
            .request_dismantle(structure_id, actor.faction_id),

        Command::RepairStructure {
            structure_id,
            actor_entity,
        } => {
            // A repair is paid from a container. The client may nominate which
            // one, but it must be a container its own faction owns; when it
            // nominates nothing the actor's own avatar pays.
            let source = actor_entity.unwrap_or(actor.avatar_entity);
            if source.is_null() {
                return Err(GameError::PermissionDenied);
            }
            world
                .repair_structure(actor.faction_id, structure_id, source)
                .map(|_| ())
        }

        Command::SetProductionRecipe {
            structure_id,
            recipe_id,
        } => world.structure_registry.set_production_recipe(
            actor.faction_id,
            structure_id,
            recipe_id,
        ),

        Command::SetExtractionTarget {
            structure_id,
            deposit_id,
        } => world.structure_registry.set_extraction_target(
            actor.faction_id,
            structure_id,
            deposit_id,
        ),

        // -------------------------------------------------------------- robots
        Command::RobotCommand {
            robot_id,
            command_type,
        } => {
            if let RobotCommandType::Guard { position } | RobotCommandType::Move { position } =
                command_type
                && !is_finite3(position)
            {
                return Err(GameError::InvalidPosition);
            }
            let order = world
                .robot_registry
                .order_for_command(robot_id, command_type)
                .ok_or(GameError::RobotNotFound(robot_id))?;
            world.issue_robot_order(actor.player_id, robot_id, order)
        }

        Command::AssignEscort { player, robot_id } => {
            world.assign_escort(actor.player_id, player, robot_id)
        }

        Command::ReleaseEscort { player, robot_id } => {
            world.release_escort(actor.player_id, player, robot_id)
        }

        Command::AssignSquadMember { squad_id, robot_id } => {
            let WorldState {
                robot_registry,
                event_journal,
                ..
            } = world;
            robot_registry.assign_squad_member(
                actor.player_id,
                squad_id,
                robot_id,
                tick,
                event_journal,
            )
        }

        Command::RemoveSquadMember { squad_id, robot_id } => {
            let WorldState {
                robot_registry,
                event_journal,
                ..
            } = world;
            robot_registry.remove_squad_member(
                actor.player_id,
                squad_id,
                robot_id,
                tick,
                event_journal,
            )
        }

        Command::SquadRegroup {
            squad_id,
            rally_position,
        } => {
            if !is_finite3(rally_position) {
                return Err(GameError::InvalidPosition);
            }
            let WorldState {
                robot_registry,
                event_journal,
                ..
            } = world;
            robot_registry
                .regroup_squad(
                    actor.player_id,
                    squad_id,
                    rally_position,
                    tick,
                    event_journal,
                )
                .map(|_| ())
        }

        // ------------------------------------------------------------- regions
        Command::TransferRegion {
            entity_id,
            destination_region,
        } => world
            .transfer_entity(actor.faction_id, entity_id, destination_region)
            .map(|_| ()),

        // ------------------------------------------------------------- economy
        Command::TransferResource {
            from_entity,
            to_entity,
            resource_id,
            amount,
        } => world.transfer_resources(
            actor.faction_id,
            from_entity,
            to_entity,
            resource_id,
            amount,
        ),

        Command::ReserveResource {
            entity,
            resource_id,
            amount,
            reservation_id,
        } => world.reserve_resources(
            actor.faction_id,
            entity,
            reservation_id,
            resource_id,
            amount,
            None,
        ),

        Command::CommitTransfer {
            reservation_id,
            from_entity,
            to_entity,
        } => {
            world.commit_resource_transfer(actor.faction_id, reservation_id, from_entity, to_entity)
        }

        Command::CancelReservation {
            reservation_id,
            from_entity,
        } => world.cancel_resource_reservation(actor.faction_id, reservation_id, from_entity),

        // A resource request has no authoritative system behind it: there is no
        // requisition queue, no cost and no fulfilment. Honouring it would mean
        // the client asserting an economy outcome, so it is refused outright.
        Command::RequestResource { .. } => Err(GameError::InvalidCommand),

        // ----------------------------------------------------------- logistics
        Command::CreateLogisticsJob {
            source,
            destination,
            resource_id,
            amount,
            priority,
        } => world
            .create_logistics_job(
                actor.faction_id,
                source,
                destination,
                resource_id,
                amount,
                JobPriority::from_u8(priority),
            )
            .map(|_| ()),

        Command::CancelLogisticsJob { job_id } => {
            let WorldState {
                structure_registry,
                inventory_registry,
                event_journal,
                ..
            } = world;
            structure_registry.logistics.cancel_job(
                actor.faction_id,
                job_id,
                "Cancelled by owning faction",
                tick,
                inventory_registry,
                event_journal,
            )
        }

        Command::ClaimLogisticsJob { job_id, worker_id } => {
            world.claim_logistics_job(actor.faction_id, job_id, worker_id)
        }

        Command::ExecuteLogisticsPickup { job_id, worker_id } => {
            world.execute_logistics_pickup(actor.faction_id, job_id, worker_id)
        }

        Command::ExecuteLogisticsDropoff { job_id, worker_id } => {
            world.execute_logistics_dropoff(actor.faction_id, job_id, worker_id)
        }

        // ------------------------------------------------------------ research
        Command::QueueResearch { tech_id } => {
            world.queue_research(actor.faction_id, tech_id).map(|_| ())
        }

        Command::CancelResearch { job_id } => {
            world.cancel_research(actor.faction_id, job_id).map(|_| ())
        }

        Command::ReorderResearchQueue { job_id, new_index } => {
            world.reorder_research(actor.faction_id, job_id, new_index as usize)
        }

        // ------------------------------------------------------------- session
        // The manifest is a session-layer handshake message validated at
        // ingress against the server policy; it never enters the simulation.
        Command::SubmitClientManifest { .. } => Err(GameError::InvalidCommand),

        // ------------------------------------------------------- admin/session
        // `AdminRegistry::authorize` already ran at ingress. The role code is
        // re-checked here so the simulation cannot be driven by an admin
        // command that reached it without that check.
        Command::AdminKickSession {
            target_session,
            reason_code,
        } => {
            require_admin(actor)?;
            world
                .pending_session_directives
                .push(SessionDirective::KickSession {
                    target: target_session,
                    reason_code,
                });
            Ok(())
        }

        Command::AdminSetTrustLevel {
            target_session,
            trust_code,
        } => {
            require_admin(actor)?;
            world
                .pending_session_directives
                .push(SessionDirective::SetTrustLevel {
                    target: target_session,
                    trust_code,
                });
            Ok(())
        }

        Command::AdminSetSessionRole {
            target_session,
            role_code,
        } => {
            require_admin(actor)?;
            world
                .pending_session_directives
                .push(SessionDirective::SetSessionRole {
                    target: target_session,
                    role_code,
                });
            Ok(())
        }

        Command::AdminGrantResource {
            target_entity,
            resource_id,
            amount,
        } => {
            require_admin(actor)?;
            let inventory = world
                .inventory_registry
                .get_mut(target_entity)
                .ok_or(GameError::ContainerNotFound(target_entity))?;
            inventory.add(resource_id, amount)?;
            world.event_journal.record(
                tick,
                SimEvent::ResourceChanged {
                    entity: Some(target_entity),
                    resource_id,
                    delta: i64::from(amount),
                },
            );
            Ok(())
        }
    }
}

fn require_admin(actor: &ActorContext) -> GameResult<()> {
    if actor.admin_role.is_privileged() {
        Ok(())
    } else {
        Err(GameError::PermissionDenied)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandEnvelope;
    use crate::inventory::ContainerKind;
    use crate::structure::StructureKind;
    use game_types::{RES_STEEL, ResourceId, SimTick};

    const OURS: FactionId = FactionId::new(1);
    const THEIRS: FactionId = FactionId::new(2);

    fn actor(world: &mut WorldState, session: u64, faction: FactionId) -> ActorContext {
        ActorContext::new(
            SessionId::new(session),
            PlayerId::new(session as u32),
            faction,
        )
        .resolve_avatar(world, DEFAULT_PLAYER_REGION)
    }

    /// A2 — the live exploit the hardening pass exists to close.
    ///
    /// `CreateLogisticsJob{source: <enemy depot>, destination: <mine>}` followed
    /// by claim / pickup / dropoff used to debit an enemy depot and credit the
    /// attacker, entirely through the authoritative path.
    #[test]
    fn test_a2_depot_theft_chain_is_rejected_at_every_step() {
        let mut world = WorldState::new();

        // Victim faction owns a full depot.
        let enemy_depot = world.create_entity(THEIRS, DEFAULT_PLAYER_REGION);
        world.create_container(enemy_depot, ContainerKind::Depot);
        world
            .inventory_mut(enemy_depot)
            .unwrap()
            .add(RES_STEEL, 500)
            .unwrap();

        // Attacker owns an empty depot and a hauler.
        let my_depot = world.create_entity(OURS, DEFAULT_PLAYER_REGION);
        world.create_container(my_depot, ContainerKind::Depot);
        let hauler = world.create_entity(OURS, DEFAULT_PLAYER_REGION);
        world.create_container(hauler, ContainerKind::CargoBuffer);

        let attacker = actor(&mut world, 1, OURS);

        // Step 1: creating the job is refused, which is where the chain dies.
        let create = apply_command(
            &mut world,
            &attacker,
            &Command::CreateLogisticsJob {
                source: enemy_depot,
                destination: my_depot,
                resource_id: RES_STEEL,
                amount: 500,
                priority: 2,
            },
        );
        assert_eq!(create, Err(GameError::PermissionDenied));
        assert!(
            world.structure_registry.logistics.jobs.is_empty(),
            "a theft job was created"
        );

        // Steps 2-4 cannot even name a job, but prove the rest of the chain is
        // closed too by having the victim create the job legitimately and the
        // attacker try to drive it.
        let victim = actor(&mut world, 2, THEIRS);
        let victim_sink = world.create_entity(THEIRS, DEFAULT_PLAYER_REGION);
        world.create_container(victim_sink, ContainerKind::Depot);
        let job = world
            .create_logistics_job(
                THEIRS,
                enemy_depot,
                victim_sink,
                RES_STEEL,
                100,
                JobPriority::Normal,
            )
            .unwrap();
        let _ = victim;

        assert_eq!(
            apply_command(
                &mut world,
                &attacker,
                &Command::ClaimLogisticsJob {
                    job_id: job,
                    worker_id: hauler,
                },
            ),
            Err(GameError::PermissionDenied),
            "attacker claimed another faction's job"
        );
        assert_eq!(
            apply_command(
                &mut world,
                &attacker,
                &Command::ExecuteLogisticsPickup {
                    job_id: job,
                    worker_id: hauler,
                },
            ),
            Err(GameError::PermissionDenied)
        );
        assert_eq!(
            apply_command(
                &mut world,
                &attacker,
                &Command::ExecuteLogisticsDropoff {
                    job_id: job,
                    worker_id: hauler,
                },
            ),
            Err(GameError::PermissionDenied)
        );

        // Not one unit of steel left the victim's depot.
        assert_eq!(
            world
                .inventory(enemy_depot)
                .unwrap()
                .total_quantity(RES_STEEL),
            500
        );
        assert_eq!(
            world.inventory(my_depot).unwrap().total_quantity(RES_STEEL),
            0
        );
    }

    /// A2 — a direct transfer out of enemy storage is refused.
    #[test]
    fn test_a2_direct_transfer_from_enemy_container_is_rejected() {
        let mut world = WorldState::new();
        let theirs = world.create_entity(THEIRS, DEFAULT_PLAYER_REGION);
        world.create_container(theirs, ContainerKind::Depot);
        world
            .inventory_mut(theirs)
            .unwrap()
            .add(RES_STEEL, 80)
            .unwrap();
        let mine = world.create_entity(OURS, DEFAULT_PLAYER_REGION);
        world.create_container(mine, ContainerKind::Depot);

        let attacker = actor(&mut world, 1, OURS);
        assert_eq!(
            apply_command(
                &mut world,
                &attacker,
                &Command::TransferResource {
                    from_entity: theirs,
                    to_entity: mine,
                    resource_id: RES_STEEL,
                    amount: 80,
                },
            ),
            Err(GameError::PermissionDenied)
        );
        assert_eq!(
            world.inventory(theirs).unwrap().total_quantity(RES_STEEL),
            80
        );
    }

    /// A2 — faction is taken from the actor, never from the payload.
    #[test]
    fn test_a2_enemy_structure_orders_are_rejected() {
        let mut world = WorldState::new();
        let attacker = actor(&mut world, 1, OURS);

        // A structure that belongs to the other faction.
        let builder = world.create_entity(THEIRS, DEFAULT_PLAYER_REGION);
        world.create_container(builder, ContainerKind::Depot);
        let their_drill = world
            .structure_registry
            .request_build(
                BuildRequest {
                    player_pos: (0.0, 0.0, 0.0),
                    requested_pos: (0.0, 0.0, 0.0),
                    kind: StructureKind::MiningDrill,
                    rotation_deg: 0.0,
                    faction_id: THEIRS,
                    region_id: DEFAULT_PLAYER_REGION,
                    creation_tick: SimTick::zero(),
                    world_bounds_xz: (-500.0, 500.0, -500.0, 500.0),
                },
                None,
            )
            .unwrap();

        assert_eq!(
            apply_command(
                &mut world,
                &attacker,
                &Command::SetProductionRecipe {
                    structure_id: their_drill,
                    recipe_id: game_types::RecipeId::new(1),
                },
            ),
            Err(GameError::PermissionDenied)
        );
        assert_eq!(
            apply_command(
                &mut world,
                &attacker,
                &Command::DismantleStructure {
                    structure_id: their_drill,
                },
            ),
            Err(GameError::PermissionDenied)
        );
        assert_eq!(
            apply_command(
                &mut world,
                &attacker,
                &Command::RepairStructure {
                    structure_id: their_drill,
                    actor_entity: None,
                },
            ),
            Err(GameError::PermissionDenied)
        );
    }

    /// A5 — construction is paid for, and the 15 m reach check is live.
    #[test]
    fn test_a5_build_costs_resources_and_enforces_reach() {
        let mut query = WorldState::new();
        let cost: Vec<(ResourceId, u32)> = StructureKind::Generator.construction_cost().to_vec();
        assert!(
            !cost.is_empty(),
            "the test needs a structure that costs something"
        );
        let _ = &mut query;

        // 1. A builder with an empty container cannot build.
        let mut world = WorldState::new();
        let broke = actor(&mut world, 1, OURS);
        world.create_container(broke.avatar_entity, ContainerKind::Backpack);
        let denied = apply_command(
            &mut world,
            &broke,
            &Command::BuildStructure {
                kind: StructureKind::Generator,
                position: (2.0, 0.0, 0.0),
                rotation_deg: 0.0,
            },
        );
        assert!(
            matches!(denied, Err(GameError::InsufficientUnreservedBalance { .. })),
            "free construction: {denied:?}"
        );
        assert_eq!(world.structure_registry.count(), 0);

        // 2. Funded, in reach: accepted, and the cost is actually deducted.
        for &(res, amount) in &cost {
            world
                .inventory_mut(broke.avatar_entity)
                .unwrap()
                .add(res, amount * 4)
                .unwrap();
        }
        apply_command(
            &mut world,
            &broke,
            &Command::BuildStructure {
                kind: StructureKind::Generator,
                position: (2.0, 0.0, 0.0),
                rotation_deg: 0.0,
            },
        )
        .unwrap();
        assert_eq!(world.structure_registry.count(), 1);
        for &(res, amount) in &cost {
            assert_eq!(
                world
                    .inventory(broke.avatar_entity)
                    .unwrap()
                    .total_quantity(res),
                amount * 3,
                "construction cost was not deducted"
            );
        }

        // 3. Funded, out of reach: the 15 m check is no longer dead code.
        let far = apply_command(
            &mut world,
            &broke,
            &Command::BuildStructure {
                kind: StructureKind::Generator,
                position: (400.0, 0.0, 400.0),
                rotation_deg: 0.0,
            },
        );
        assert_eq!(far, Err(GameError::PlacementTooFar));
        assert_eq!(world.structure_registry.count(), 1);
    }

    /// A6 — the server clamps a client-asserted position.
    #[test]
    fn test_a6_move_command_is_clamped_by_the_server() {
        let mut world = WorldState::new();
        let a = actor(&mut world, 1, OURS);
        apply_command(
            &mut world,
            &a,
            &Command::Move {
                position: (50_000.0, 0.0, 0.0),
                velocity: (0.0, 0.0, 0.0),
            },
        )
        .unwrap();
        let pos = world.robot_registry.player(a.player_id).unwrap().position;
        assert!(pos.0 < 1.0, "server trusted a client teleport: {pos:?}");

        // A non-finite position is refused outright and changes nothing.
        let before = world.robot_registry.player(a.player_id).unwrap().position;
        assert_eq!(
            apply_command(
                &mut world,
                &a,
                &Command::Move {
                    position: (f32::NAN, 0.0, 0.0),
                    velocity: (0.0, 0.0, 0.0),
                },
            ),
            Err(GameError::InvalidPosition)
        );
        assert_eq!(
            world.robot_registry.player(a.player_id).unwrap().position,
            before
        );
    }

    /// A3 — the previously discarded variants now answer with a typed error.
    #[test]
    fn test_a3_unimplemented_variants_are_explicitly_refused() {
        let mut world = WorldState::new();
        let a = actor(&mut world, 1, OURS);
        assert_eq!(
            apply_command(
                &mut world,
                &a,
                &Command::Action {
                    action_type: crate::command::ActionType::FireWeapon,
                    target: None,
                },
            ),
            Err(GameError::InvalidCommand)
        );
        assert_eq!(
            apply_command(
                &mut world,
                &a,
                &Command::RequestResource {
                    resource_id: RES_STEEL,
                    amount: 10,
                },
            ),
            Err(GameError::InvalidCommand)
        );
        assert_eq!(
            apply_command(
                &mut world,
                &a,
                &Command::SubmitClientManifest {
                    build_id: "x".to_string(),
                    protocol_version: 1,
                    content_hash: 0,
                    official_build: false,
                },
            ),
            Err(GameError::InvalidCommand)
        );
    }

    /// A2 — an unprivileged session cannot drive an admin command even if one
    /// somehow reaches the simulation.
    #[test]
    fn test_a2_admin_commands_require_a_privileged_role() {
        let mut world = WorldState::new();
        let a = actor(&mut world, 1, OURS);
        let target = world.create_entity(OURS, DEFAULT_PLAYER_REGION);
        world.create_container(target, ContainerKind::Depot);

        let grant = Command::AdminGrantResource {
            target_entity: target,
            resource_id: RES_STEEL,
            amount: 1_000,
        };
        assert_eq!(
            apply_command(&mut world, &a, &grant),
            Err(GameError::PermissionDenied)
        );
        assert_eq!(
            world.inventory(target).unwrap().total_quantity(RES_STEEL),
            0
        );

        let host = a.with_admin_role(AdminRoleCode::new(2));
        apply_command(&mut world, &host, &grant).unwrap();
        assert_eq!(
            world.inventory(target).unwrap().total_quantity(RES_STEEL),
            1_000
        );

        // A privileged kick becomes a session directive for the server to run.
        apply_command(
            &mut world,
            &host,
            &Command::AdminKickSession {
                target_session: SessionId::new(9),
                reason_code: 3,
            },
        )
        .unwrap();
        assert_eq!(
            world.drain_session_directives(),
            vec![SessionDirective::KickSession {
                target: SessionId::new(9),
                reason_code: 3,
            }]
        );
    }

    /// A4 — commands drain FIFO ordered by `(session_id, sequence)`.
    #[test]
    fn test_a4_command_buffer_drains_in_session_sequence_order() {
        let mut world = WorldState::new();
        // Pushed deliberately out of order and interleaved between sessions.
        for (session, sequence) in [(2u64, 2u64), (1, 3), (2, 1), (1, 1), (1, 2)] {
            world.add_command(CommandEnvelope::new(
                SessionId::new(session),
                sequence,
                SimTick::zero(),
                Command::RequestResource {
                    resource_id: RES_STEEL,
                    amount: sequence as u32,
                },
            ));
        }
        let order: Vec<(u64, u64)> = world
            .command_buffer
            .drain_ordered()
            .map(|e| (e.session_id.value(), e.sequence))
            .collect();
        assert_eq!(order, vec![(1, 1), (1, 2), (1, 3), (2, 1), (2, 2)]);
        assert!(world.command_buffer.is_empty());
    }
}
