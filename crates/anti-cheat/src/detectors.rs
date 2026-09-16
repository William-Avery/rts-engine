use crate::event::{PlacementRejection, SecurityEventKind};
use crate::provider::InspectionContext;
use crate::world_view::KnowledgeQuery;
use game_types::{EntityId, FactionId, ResourceId, SimTick, StructureId};
use sim_core::command::{ActionType, Command, RobotCommandType};
use std::collections::BTreeMap;

/// Authoritative simulation tick rate, used to convert displacement to speed.
pub const SERVER_TICK_HZ: f32 = 30.0;

/// Maximum authoritative on-foot speed in metres per second.
pub const MAX_PLAYER_SPEED_MPS: f32 = 12.0;

/// Multiplier applied to the speed clamp before flagging.
///
/// Honest clients overshoot the clamp under packet loss, prediction error and
/// slope-assisted movement. The tolerance is what keeps the movement detector's
/// false-positive rate near zero for legitimate play.
pub const SPEED_TOLERANCE: f32 = 1.5;

/// Displacement within a single tick that is treated as a teleport.
pub const MAX_TELEPORT_DISTANCE_M: f32 = 6.0;

/// Fallback minimum ticks between two weapon discharges.
///
/// Superseded per-actor by [`crate::world_view::WorldView::weapon_cooldown_ticks`]
/// once Milestone 13 lands weapon profiles. Chosen as a floor no designed weapon
/// may undercut, so it cannot flag a legitimate fast weapon.
pub const DEFAULT_MIN_FIRE_INTERVAL_TICKS: u64 = 4;

/// Resource treated as the generic ammunition pool until Milestone 13 models magazines.
pub const AMMO_RESOURCE: ResourceId = game_types::resource::RES_AMMO;

/// Furthest a session may place a structure from its last authoritative position.
pub const MAX_BUILD_REACH_M: f32 = 60.0;

/// Ticks a client envelope may lead the authoritative tick before being flagged.
pub const MAX_CLIENT_TICK_LEAD: u64 = 60;

/// Largest single resource movement a legitimate client can request.
pub const MAX_RESOURCE_TRANSFER_AMOUNT: u32 = 1_000_000;

/// Command-rate window length in ticks.
pub const FLOOD_WINDOW_TICKS: u64 = 30;

/// Commands within [`FLOOD_WINDOW_TICKS`] above which a session is flooding.
pub const FLOOD_WINDOW_LIMIT: u32 = 240;

/// Rotation magnitude beyond which a placement rotation is nonsense.
const MAX_ABS_ROTATION_DEG: f32 = 3_600.0;

/// Rolling per-session history the stateful detectors need.
///
/// Uses `BTreeMap` rather than a hash map so iteration order (and therefore any
/// derived telemetry) is identical on every machine.
#[derive(Clone, Default, Debug)]
pub struct SessionTelemetry {
    /// Last position the session claimed and the tick it claimed it on.
    pub last_position: Option<(f32, f32, f32)>,
    pub last_move_tick: SimTick,
    /// Last discharge tick per actor entity.
    pub last_fire_tick: BTreeMap<EntityId, SimTick>,
    /// Commands seen in the current rate window.
    pub commands_in_window: u32,
    pub window_start_tick: SimTick,
}

impl SessionTelemetry {
    pub fn new(start_tick: SimTick) -> Self {
        SessionTelemetry {
            last_position: None,
            last_move_tick: start_tick,
            last_fire_tick: BTreeMap::new(),
            commands_in_window: 0,
            window_start_tick: start_tick,
        }
    }
}

fn is_finite3(v: (f32, f32, f32)) -> bool {
    v.0.is_finite() && v.1.is_finite() && v.2.is_finite()
}

fn magnitude3(v: (f32, f32, f32)) -> f32 {
    (v.0 * v.0 + v.1 * v.1 + v.2 * v.2).sqrt()
}

fn distance3(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    magnitude3((a.0 - b.0, a.1 - b.1, a.2 - b.2))
}

fn out_of_bounds(ctx: &InspectionContext<'_>, pos: (f32, f32, f32)) -> bool {
    let (min_x, max_x, min_z, max_z) = ctx.world.world_bounds_xz();
    pos.0 < min_x || pos.0 > max_x || pos.2 < min_z || pos.2 > max_z
}

// ---------------------------------------------------------------------------
// Movement
// ---------------------------------------------------------------------------

/// Impossible movement: speed or displacement beyond the server clamp.
///
/// Catches speed hacks, teleport hacks, and coordinate corruption. Because a
/// laggy client legitimately batches displacement, everything except NaN and
/// out-of-bounds is rated at most `Medium` and never bans on its own.
///
/// Updates the session's movement history as a side effect.
pub fn detect_impossible_movement(
    ctx: &InspectionContext<'_>,
    telemetry: &mut SessionTelemetry,
    position: (f32, f32, f32),
    velocity: (f32, f32, f32),
) -> Vec<SecurityEventKind> {
    let mut findings = Vec::new();

    if !is_finite3(position) || !is_finite3(velocity) {
        findings.push(SecurityEventKind::NonFinitePosition);
        // Do not record a non-finite position into history.
        return findings;
    }

    if out_of_bounds(ctx, position) {
        findings.push(SecurityEventKind::PositionOutOfBounds {
            x: position.0,
            z: position.2,
        });
    }

    let max_speed = MAX_PLAYER_SPEED_MPS * SPEED_TOLERANCE;
    let claimed_speed = magnitude3(velocity);
    if claimed_speed > max_speed {
        findings.push(SecurityEventKind::ImpossibleMovement {
            observed_speed_mps: claimed_speed,
            max_speed_mps: max_speed,
            elapsed_ticks: 0,
        });
    }

    if let Some(previous) = telemetry.last_position {
        let distance = distance3(previous, position);
        let elapsed = ctx
            .server_tick
            .value()
            .saturating_sub(telemetry.last_move_tick.value());
        if elapsed == 0 {
            if distance > MAX_TELEPORT_DISTANCE_M {
                findings.push(SecurityEventKind::TeleportAttempt {
                    distance_m: distance,
                    max_distance_m: MAX_TELEPORT_DISTANCE_M,
                });
            }
        } else {
            let seconds = elapsed as f32 / SERVER_TICK_HZ;
            let implied_speed = distance / seconds;
            if implied_speed > max_speed {
                findings.push(SecurityEventKind::ImpossibleMovement {
                    observed_speed_mps: implied_speed,
                    max_speed_mps: max_speed,
                    elapsed_ticks: elapsed,
                });
            }
        }
    }

    telemetry.last_position = Some(position);
    telemetry.last_move_tick = ctx.server_tick;
    findings
}

// ---------------------------------------------------------------------------
// Fire rate
// ---------------------------------------------------------------------------

/// Fire-rate violation: a discharge arrived sooner than the weapon cycle allows.
///
/// Uses the per-actor cooldown from
/// [`crate::world_view::WorldView::weapon_cooldown_ticks`] when Milestone 13 has
/// supplied one, otherwise the generous [`DEFAULT_MIN_FIRE_INTERVAL_TICKS`]
/// floor. Updates the session's per-actor discharge history.
pub fn detect_fire_rate_violation(
    ctx: &InspectionContext<'_>,
    telemetry: &mut SessionTelemetry,
    actor: EntityId,
) -> Option<SecurityEventKind> {
    let min_ticks = ctx
        .world
        .weapon_cooldown_ticks(actor)
        .unwrap_or(DEFAULT_MIN_FIRE_INTERVAL_TICKS);

    let finding = telemetry.last_fire_tick.get(&actor).and_then(|last| {
        let elapsed = ctx.server_tick.value().saturating_sub(last.value());
        (elapsed < min_ticks).then_some(SecurityEventKind::FireRateViolation {
            actor,
            ticks_since_last: elapsed,
            min_ticks,
        })
    });

    telemetry.last_fire_tick.insert(actor, ctx.server_tick);
    finding
}

// ---------------------------------------------------------------------------
// Ammo
// ---------------------------------------------------------------------------

/// Ammo inconsistency: a discharge from an actor with no authoritative ammo.
///
/// Prefers the Milestone 13 magazine hook
/// ([`crate::world_view::WorldView::loaded_ammo`]) when available, and otherwise
/// falls back to the actor's container balance of [`AMMO_RESOURCE`].
///
/// An actor with no container at all is *not* flagged: absence of evidence is
/// not evidence, and flagging it would make every containerless unit suspicious.
pub fn detect_ammo_inconsistency(
    ctx: &InspectionContext<'_>,
    actor: EntityId,
) -> Option<SecurityEventKind> {
    let available = match ctx.world.loaded_ammo(actor) {
        Some(loaded) => loaded,
        None => ctx.world.available_resource(actor, AMMO_RESOURCE)?,
    };
    (available == 0).then_some(SecurityEventKind::AmmoInconsistency {
        actor,
        resource: AMMO_RESOURCE,
        available,
    })
}

// ---------------------------------------------------------------------------
// Placement
// ---------------------------------------------------------------------------

/// Invalid placement: construction requested at a nonsensical or unreachable spot.
///
/// Runs ahead of, and independently of, the structure registry's own placement
/// validation. Reach is only checked once the session has claimed a position at
/// least once, so it never fires before the first movement command.
pub fn detect_invalid_placement(
    ctx: &InspectionContext<'_>,
    telemetry: &SessionTelemetry,
    position: (f32, f32, f32),
    rotation_deg: f32,
) -> Vec<SecurityEventKind> {
    let mut findings = Vec::new();

    if !is_finite3(position) {
        findings.push(SecurityEventKind::InvalidPlacement {
            reason: PlacementRejection::NonFiniteCoordinate,
            position,
        });
        return findings;
    }

    if !rotation_deg.is_finite() || rotation_deg.abs() > MAX_ABS_ROTATION_DEG {
        findings.push(SecurityEventKind::InvalidPlacement {
            reason: PlacementRejection::InvalidRotation,
            position,
        });
    }

    if out_of_bounds(ctx, position) {
        findings.push(SecurityEventKind::InvalidPlacement {
            reason: PlacementRejection::OutsideWorldBounds,
            position,
        });
    }

    if let Some(player_pos) = telemetry.last_position
        && distance3(player_pos, position) > MAX_BUILD_REACH_M
    {
        findings.push(SecurityEventKind::InvalidPlacement {
            reason: PlacementRejection::BeyondBuildReach,
            position,
        });
    }

    findings
}

// ---------------------------------------------------------------------------
// Economy
// ---------------------------------------------------------------------------

/// Impossible inventory/economy delta: a resource movement larger than the
/// authoritative balance permits, i.e. an attempted duplication.
///
/// Compares against the *unreserved* balance, so it also catches attempts to
/// spend material already locked by a logistics reservation.
pub fn detect_impossible_economy_delta(
    ctx: &InspectionContext<'_>,
    entity: EntityId,
    resource: ResourceId,
    requested: u32,
) -> Option<SecurityEventKind> {
    if requested > MAX_RESOURCE_TRANSFER_AMOUNT {
        return Some(SecurityEventKind::ImpossibleEconomyDelta {
            entity,
            resource,
            requested,
            available: ctx
                .world
                .available_resource(entity, resource)
                .unwrap_or_default(),
        });
    }
    let available = ctx.world.available_resource(entity, resource)?;
    (requested > available).then_some(SecurityEventKind::ImpossibleEconomyDelta {
        entity,
        resource,
        requested,
        available,
    })
}

// ---------------------------------------------------------------------------
// Ownership
// ---------------------------------------------------------------------------

/// Unauthorized order: commanding an entity the session's faction does not own.
///
/// Unknown entities produce no finding — the server's own validation rejects
/// them, and a stale client referencing a just-destroyed entity is normal.
pub fn detect_unauthorized_entity_order(
    ctx: &InspectionContext<'_>,
    entity: EntityId,
) -> Option<SecurityEventKind> {
    if entity.is_null() {
        return None;
    }
    let owner = ctx.world.entity_faction(entity)?;
    if owner.is_null() || owner == ctx.faction_id {
        return None;
    }
    Some(SecurityEventKind::UnauthorizedOrder {
        target_entity: entity,
        owning_faction: owner,
        claiming_faction: ctx.faction_id,
    })
}

/// Unauthorized order against a structure owned by another faction.
pub fn detect_unauthorized_structure_order(
    ctx: &InspectionContext<'_>,
    structure: StructureId,
) -> Option<SecurityEventKind> {
    if structure.is_null() {
        return None;
    }
    let owner = ctx.world.structure_faction(structure)?;
    if owner.is_null() || owner == ctx.faction_id {
        return None;
    }
    Some(SecurityEventKind::UnauthorizedStructureOrder {
        structure,
        owning_faction: owner,
        claiming_faction: ctx.faction_id,
    })
}

// ---------------------------------------------------------------------------
// Hidden target
// ---------------------------------------------------------------------------

/// Hidden-target attempt: acting on an entity the faction has no knowledge of.
///
/// This is the wallhack tell — a client that targets something it was never
/// sent. It consults
/// [`crate::world_view::WorldView::faction_knows_entity`], the **Milestone 14**
/// integration hook. Until M14 lands a knowledge store that query answers
/// [`KnowledgeQuery::Unavailable`] and this detector reports nothing, so it can
/// never produce a false accusation against a client the server simply cannot
/// evaluate yet.
pub fn detect_hidden_target_attempt(
    ctx: &InspectionContext<'_>,
    target: EntityId,
) -> Option<SecurityEventKind> {
    if target.is_null() {
        return None;
    }
    let owner = ctx.world.entity_faction(target)?;
    if owner == ctx.faction_id {
        return None;
    }
    match ctx.world.faction_knows_entity(ctx.faction_id, target) {
        KnowledgeQuery::Unknown => Some(SecurityEventKind::HiddenTargetAttempt {
            target,
            claiming_faction: ctx.faction_id,
        }),
        KnowledgeQuery::Known | KnowledgeQuery::Unavailable => None,
    }
}

// ---------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------

/// Replayed / malformed envelope: a client tick stamped far into the future.
///
/// Duplicate and out-of-order sequence numbers are rejected by
/// `game_protocol::session::Session::validate_and_advance_sequence` before
/// inspection ever runs; this covers the remaining tick-stamp tampering.
pub fn detect_future_dated_command(ctx: &InspectionContext<'_>) -> Option<SecurityEventKind> {
    let lead = ctx
        .client_tick
        .value()
        .saturating_sub(ctx.server_tick.value());
    (lead > MAX_CLIENT_TICK_LEAD).then_some(SecurityEventKind::FutureDatedCommand {
        client_tick: ctx.client_tick,
        server_tick: ctx.server_tick,
    })
}

/// Command flood: sustained input rate above any human or legitimate client.
pub fn detect_command_flood(
    ctx: &InspectionContext<'_>,
    telemetry: &mut SessionTelemetry,
) -> Option<SecurityEventKind> {
    let elapsed = ctx
        .server_tick
        .value()
        .saturating_sub(telemetry.window_start_tick.value());
    if elapsed >= FLOOD_WINDOW_TICKS {
        telemetry.window_start_tick = ctx.server_tick;
        telemetry.commands_in_window = 0;
    }
    telemetry.commands_in_window = telemetry.commands_in_window.saturating_add(1);
    (telemetry.commands_in_window > FLOOD_WINDOW_LIMIT).then_some(
        SecurityEventKind::CommandFloodDetected {
            commands_in_window: telemetry.commands_in_window,
            window_ticks: FLOOD_WINDOW_TICKS,
        },
    )
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

/// Run every detector that applies to `command`.
///
/// Returns an empty `Vec` (which does not allocate) for a clean command, so the
/// common path costs only the detector comparisons themselves.
pub fn inspect(
    ctx: &InspectionContext<'_>,
    telemetry: &mut SessionTelemetry,
    command: &Command,
) -> Vec<SecurityEventKind> {
    let mut findings = Vec::new();
    findings.extend(detect_future_dated_command(ctx));
    findings.extend(detect_command_flood(ctx, telemetry));

    match command {
        Command::Move { position, velocity } => {
            findings.extend(detect_impossible_movement(
                ctx, telemetry, *position, *velocity,
            ));
        }
        Command::Action {
            action_type,
            target,
        } => {
            if matches!(action_type, ActionType::FireWeapon | ActionType::Attack) {
                let actor = ctx.actor_key();
                findings.extend(detect_fire_rate_violation(ctx, telemetry, actor));
                if !actor.is_null() {
                    findings.extend(detect_ammo_inconsistency(ctx, actor));
                }
            }
            if let Some(target) = target {
                findings.extend(detect_hidden_target_attempt(ctx, *target));
            }
        }
        Command::BuildStructure {
            kind: _,
            position,
            rotation_deg,
        } => {
            findings.extend(detect_invalid_placement(
                ctx,
                telemetry,
                *position,
                *rotation_deg,
            ));
        }
        Command::DismantleStructure { structure_id } => {
            findings.extend(detect_unauthorized_structure_order(ctx, *structure_id));
        }
        Command::RepairStructure {
            structure_id,
            actor_entity,
        } => {
            findings.extend(detect_unauthorized_structure_order(ctx, *structure_id));
            if let Some(actor) = actor_entity {
                findings.extend(detect_unauthorized_entity_order(ctx, *actor));
            }
        }
        Command::SetProductionRecipe { structure_id, .. }
        | Command::SetExtractionTarget { structure_id, .. } => {
            findings.extend(detect_unauthorized_structure_order(ctx, *structure_id));
        }
        Command::RobotCommand {
            robot_id,
            command_type,
        } => {
            findings.extend(detect_unauthorized_entity_order(ctx, *robot_id));
            match command_type {
                RobotCommandType::Attack { target } => {
                    findings.extend(detect_fire_rate_violation(ctx, telemetry, *robot_id));
                    findings.extend(detect_ammo_inconsistency(ctx, *robot_id));
                    findings.extend(detect_hidden_target_attempt(ctx, *target));
                }
                RobotCommandType::Follow { target } => {
                    findings.extend(detect_hidden_target_attempt(ctx, *target));
                }
                RobotCommandType::Guard { position } | RobotCommandType::Move { position } => {
                    if !is_finite3(*position) {
                        findings.push(SecurityEventKind::NonFinitePosition);
                    } else if out_of_bounds(ctx, *position) {
                        findings.push(SecurityEventKind::PositionOutOfBounds {
                            x: position.0,
                            z: position.2,
                        });
                    }
                }
                RobotCommandType::ReturnToBase => {}
            }
        }
        Command::TransferRegion { entity_id, .. } => {
            findings.extend(detect_unauthorized_entity_order(ctx, *entity_id));
        }
        Command::TransferResource {
            from_entity,
            to_entity,
            resource_id,
            amount,
        } => {
            findings.extend(detect_unauthorized_entity_order(ctx, *from_entity));
            findings.extend(detect_unauthorized_entity_order(ctx, *to_entity));
            findings.extend(detect_impossible_economy_delta(
                ctx,
                *from_entity,
                *resource_id,
                *amount,
            ));
        }
        Command::ReserveResource {
            entity,
            resource_id,
            amount,
            ..
        } => {
            findings.extend(detect_unauthorized_entity_order(ctx, *entity));
            findings.extend(detect_impossible_economy_delta(
                ctx,
                *entity,
                *resource_id,
                *amount,
            ));
        }
        Command::CommitTransfer {
            from_entity,
            to_entity,
            ..
        } => {
            findings.extend(detect_unauthorized_entity_order(ctx, *from_entity));
            findings.extend(detect_unauthorized_entity_order(ctx, *to_entity));
        }
        Command::CancelReservation { from_entity, .. } => {
            findings.extend(detect_unauthorized_entity_order(ctx, *from_entity));
        }
        Command::CreateLogisticsJob {
            source,
            destination,
            resource_id,
            amount,
            ..
        } => {
            findings.extend(detect_unauthorized_entity_order(ctx, *source));
            findings.extend(detect_unauthorized_entity_order(ctx, *destination));
            findings.extend(detect_impossible_economy_delta(
                ctx,
                *source,
                *resource_id,
                *amount,
            ));
        }
        Command::ClaimLogisticsJob { worker_id, .. }
        | Command::ExecuteLogisticsPickup { worker_id, .. }
        | Command::ExecuteLogisticsDropoff { worker_id, .. } => {
            findings.extend(detect_unauthorized_entity_order(ctx, *worker_id));
        }
        Command::AdminGrantResource { target_entity, .. } => {
            findings.extend(detect_unauthorized_entity_order(ctx, *target_entity));
        }
        // Permission-gated admin commands are authorized by `AdminRegistry`
        // before inspection; manifest submission is validated by server policy.
        Command::AdminKickSession { .. }
        | Command::AdminSetTrustLevel { .. }
        | Command::AdminSetSessionRole { .. }
        | Command::SubmitClientManifest { .. }
        | Command::CancelLogisticsJob { .. }
        | Command::RequestResource { .. } => {}
        // Milestone 12 escort and squad commands name a robot entity, so the
        // ownership detector has something concrete to contradict: a session
        // reaching for a robot of another faction. `RobotRegistry` still does
        // the authoritative refusal; this records the attempt.
        Command::AssignEscort { robot_id, .. }
        | Command::ReleaseEscort { robot_id, .. }
        | Command::AssignSquadMember { robot_id, .. }
        | Command::RemoveSquadMember { robot_id, .. } => {
            findings.extend(detect_unauthorized_entity_order(ctx, *robot_id));
        }
        // `SquadRegroup` names no entity or structure, only a rally point, so
        // there is nothing for an ownership detector to compare against. The
        // rally point itself is still worth checking.
        Command::SquadRegroup { rally_position, .. } => {
            if !is_finite3(*rally_position) {
                findings.push(SecurityEventKind::NonFinitePosition);
            } else if out_of_bounds(ctx, *rally_position) {
                findings.push(SecurityEventKind::PositionOutOfBounds {
                    x: rally_position.0,
                    z: rally_position.2,
                });
            }
        }
        // Milestone 19 research intents are faction-scoped and validated
        // authoritatively by `ResearchManager`; they carry no entity or
        // position a detector could contradict.
        Command::QueueResearch { .. }
        | Command::CancelResearch { .. }
        | Command::ReorderResearchQueue { .. } => {}
    }

    findings
}

/// Faction used for entities with no owner, for readability at call sites.
pub const UNOWNED_FACTION: FactionId = FactionId::null();

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world_view::{StaticWorldView, WorldView};
    use game_types::{PlayerId, SessionId};

    const OWN_FACTION: FactionId = FactionId::new(1);
    const ENEMY_FACTION: FactionId = FactionId::new(2);

    fn ctx_at<'a>(world: &'a StaticWorldView, tick: u64) -> InspectionContext<'a> {
        InspectionContext::new(
            SessionId::new(1),
            PlayerId::new(1),
            OWN_FACTION,
            SimTick::new(tick),
            SimTick::new(tick),
            1,
            world,
        )
    }

    // --- impossible movement ------------------------------------------------

    #[test]
    fn test_detect_impossible_movement_accepts_legitimate_walking() {
        let world = StaticWorldView::new();
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        let ctx = ctx_at(&world, 10);
        assert!(
            detect_impossible_movement(&ctx, &mut telemetry, (0.0, 0.0, 0.0), (3.0, 0.0, 2.0))
                .is_empty()
        );
        let ctx = ctx_at(&world, 40);
        // 30 ticks = 1 second at 30 Hz; 8 m is well under the clamp.
        assert!(
            detect_impossible_movement(&ctx, &mut telemetry, (8.0, 0.0, 0.0), (8.0, 0.0, 0.0))
                .is_empty()
        );
    }

    #[test]
    fn test_detect_impossible_movement_flags_speed_hack() {
        let world = StaticWorldView::new();
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        let ctx = ctx_at(&world, 1);
        let findings =
            detect_impossible_movement(&ctx, &mut telemetry, (0.0, 0.0, 0.0), (90.0, 0.0, 0.0));
        assert!(matches!(
            findings.as_slice(),
            [SecurityEventKind::ImpossibleMovement { .. }]
        ));
    }

    #[test]
    fn test_detect_impossible_movement_flags_teleport_beyond_clamp() {
        let world = StaticWorldView::new();
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        let ctx = ctx_at(&world, 10);
        detect_impossible_movement(&ctx, &mut telemetry, (0.0, 0.0, 0.0), (0.0, 0.0, 0.0));
        // Same tick, 300 m jump.
        let ctx = ctx_at(&world, 10);
        let findings =
            detect_impossible_movement(&ctx, &mut telemetry, (300.0, 0.0, 0.0), (0.0, 0.0, 0.0));
        assert!(
            findings
                .iter()
                .any(|f| matches!(f, SecurityEventKind::TeleportAttempt { .. }))
        );
    }

    #[test]
    fn test_detect_impossible_movement_flags_implied_speed_over_time() {
        let world = StaticWorldView::new();
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        detect_impossible_movement(
            &ctx_at(&world, 10),
            &mut telemetry,
            (0.0, 0.0, 0.0),
            (0.0, 0.0, 0.0),
        );
        // 400 m in 30 ticks (1 s) => 400 m/s implied.
        let findings = detect_impossible_movement(
            &ctx_at(&world, 40),
            &mut telemetry,
            (400.0, 0.0, 0.0),
            (1.0, 0.0, 0.0),
        );
        assert!(findings.iter().any(|f| matches!(
            f,
            SecurityEventKind::ImpossibleMovement {
                elapsed_ticks: 30,
                ..
            }
        )));
    }

    #[test]
    fn test_detect_impossible_movement_flags_nan_and_out_of_bounds() {
        let world = StaticWorldView::new();
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        let ctx = ctx_at(&world, 1);
        let findings =
            detect_impossible_movement(&ctx, &mut telemetry, (f32::NAN, 0.0, 0.0), (0.0, 0.0, 0.0));
        assert_eq!(findings, vec![SecurityEventKind::NonFinitePosition]);
        // NaN must not poison the movement history.
        assert!(telemetry.last_position.is_none());

        let findings =
            detect_impossible_movement(&ctx, &mut telemetry, (9_000.0, 0.0, 0.0), (0.0, 0.0, 0.0));
        assert!(
            findings
                .iter()
                .any(|f| matches!(f, SecurityEventKind::PositionOutOfBounds { .. }))
        );
    }

    // --- fire rate ----------------------------------------------------------

    #[test]
    fn test_detect_fire_rate_violation_allows_legitimate_cadence() {
        let world = StaticWorldView::new();
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        let actor = EntityId::new(5);
        assert!(detect_fire_rate_violation(&ctx_at(&world, 10), &mut telemetry, actor).is_none());
        assert!(detect_fire_rate_violation(&ctx_at(&world, 20), &mut telemetry, actor).is_none());
    }

    #[test]
    fn test_detect_fire_rate_violation_flags_rapid_fire() {
        let world = StaticWorldView::new();
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        let actor = EntityId::new(5);
        detect_fire_rate_violation(&ctx_at(&world, 10), &mut telemetry, actor);
        let finding = detect_fire_rate_violation(&ctx_at(&world, 11), &mut telemetry, actor);
        assert!(matches!(
            finding,
            Some(SecurityEventKind::FireRateViolation {
                ticks_since_last: 1,
                min_ticks: DEFAULT_MIN_FIRE_INTERVAL_TICKS,
                ..
            })
        ));
    }

    #[test]
    fn test_detect_fire_rate_violation_uses_milestone_13_weapon_cooldown_hook() {
        let actor = EntityId::new(5);
        let world = StaticWorldView::new().with_weapon_cooldown(actor, 30);
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        detect_fire_rate_violation(&ctx_at(&world, 10), &mut telemetry, actor);
        // 10 ticks later is fine under the default floor, but violates a 30-tick weapon.
        let finding = detect_fire_rate_violation(&ctx_at(&world, 20), &mut telemetry, actor);
        assert!(matches!(
            finding,
            Some(SecurityEventKind::FireRateViolation { min_ticks: 30, .. })
        ));
    }

    // --- ammo ---------------------------------------------------------------

    #[test]
    fn test_detect_ammo_inconsistency_flags_firing_with_empty_pool() {
        let actor = EntityId::new(7);
        let world = StaticWorldView::new()
            .with_entity(actor, OWN_FACTION)
            .with_balance(actor, AMMO_RESOURCE, 0);
        let finding = detect_ammo_inconsistency(&ctx_at(&world, 1), actor);
        assert!(matches!(
            finding,
            Some(SecurityEventKind::AmmoInconsistency { available: 0, .. })
        ));
    }

    #[test]
    fn test_detect_ammo_inconsistency_silent_when_stocked_or_unknown() {
        let actor = EntityId::new(7);
        let stocked = StaticWorldView::new().with_balance(actor, AMMO_RESOURCE, 30);
        assert!(detect_ammo_inconsistency(&ctx_at(&stocked, 1), actor).is_none());
        // No container at all: unknowable, so silent.
        let unknown = StaticWorldView::new();
        assert!(detect_ammo_inconsistency(&ctx_at(&unknown, 1), actor).is_none());
    }

    // --- placement ----------------------------------------------------------

    #[test]
    fn test_detect_invalid_placement_accepts_nearby_in_bounds_site() {
        let world = StaticWorldView::new();
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        telemetry.last_position = Some((0.0, 0.0, 0.0));
        assert!(
            detect_invalid_placement(&ctx_at(&world, 1), &telemetry, (5.0, 0.0, 5.0), 90.0)
                .is_empty()
        );
    }

    #[test]
    fn test_detect_invalid_placement_flags_out_of_bounds_and_nan_and_rotation() {
        let world = StaticWorldView::new();
        let telemetry = SessionTelemetry::new(SimTick::zero());
        let ctx = ctx_at(&world, 1);

        let nan = detect_invalid_placement(&ctx, &telemetry, (f32::INFINITY, 0.0, 0.0), 0.0);
        assert!(matches!(
            nan.as_slice(),
            [SecurityEventKind::InvalidPlacement {
                reason: PlacementRejection::NonFiniteCoordinate,
                ..
            }]
        ));

        let oob = detect_invalid_placement(&ctx, &telemetry, (10_000.0, 0.0, 0.0), 0.0);
        assert!(oob.iter().any(|f| matches!(
            f,
            SecurityEventKind::InvalidPlacement {
                reason: PlacementRejection::OutsideWorldBounds,
                ..
            }
        )));

        let rot = detect_invalid_placement(&ctx, &telemetry, (1.0, 0.0, 1.0), 1.0e9);
        assert!(rot.iter().any(|f| matches!(
            f,
            SecurityEventKind::InvalidPlacement {
                reason: PlacementRejection::InvalidRotation,
                ..
            }
        )));
    }

    #[test]
    fn test_detect_invalid_placement_flags_building_beyond_reach() {
        let world = StaticWorldView::new();
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        telemetry.last_position = Some((0.0, 0.0, 0.0));
        let findings =
            detect_invalid_placement(&ctx_at(&world, 1), &telemetry, (400.0, 0.0, 0.0), 0.0);
        assert!(findings.iter().any(|f| matches!(
            f,
            SecurityEventKind::InvalidPlacement {
                reason: PlacementRejection::BeyondBuildReach,
                ..
            }
        )));
    }

    // --- economy ------------------------------------------------------------

    #[test]
    fn test_detect_impossible_economy_delta_flags_resource_duplication() {
        let entity = EntityId::new(3);
        let resource = ResourceId::new(10);
        let world = StaticWorldView::new().with_balance(entity, resource, 25);
        let finding = detect_impossible_economy_delta(&ctx_at(&world, 1), entity, resource, 5_000);
        assert!(matches!(
            finding,
            Some(SecurityEventKind::ImpossibleEconomyDelta {
                requested: 5_000,
                available: 25,
                ..
            })
        ));
    }

    #[test]
    fn test_detect_impossible_economy_delta_allows_affordable_transfer() {
        let entity = EntityId::new(3);
        let resource = ResourceId::new(10);
        let world = StaticWorldView::new().with_balance(entity, resource, 25);
        assert!(
            detect_impossible_economy_delta(&ctx_at(&world, 1), entity, resource, 25).is_none()
        );
    }

    #[test]
    fn test_detect_impossible_economy_delta_flags_absurd_amount_without_container() {
        let world = StaticWorldView::new();
        let finding = detect_impossible_economy_delta(
            &ctx_at(&world, 1),
            EntityId::new(3),
            ResourceId::new(10),
            u32::MAX,
        );
        assert!(finding.is_some());
    }

    // --- ownership ----------------------------------------------------------

    #[test]
    fn test_detect_unauthorized_order_flags_commanding_enemy_entity() {
        let enemy = EntityId::new(9);
        let world = StaticWorldView::new().with_entity(enemy, ENEMY_FACTION);
        let finding = detect_unauthorized_entity_order(&ctx_at(&world, 1), enemy);
        assert!(matches!(
            finding,
            Some(SecurityEventKind::UnauthorizedOrder {
                owning_faction: ENEMY_FACTION,
                claiming_faction: OWN_FACTION,
                ..
            })
        ));
    }

    #[test]
    fn test_detect_unauthorized_order_allows_own_and_unknown_entities() {
        let own = EntityId::new(9);
        let world = StaticWorldView::new().with_entity(own, OWN_FACTION);
        assert!(detect_unauthorized_entity_order(&ctx_at(&world, 1), own).is_none());
        // Unknown entity: server validation handles it, anti-cheat stays quiet.
        assert!(detect_unauthorized_entity_order(&ctx_at(&world, 1), EntityId::new(404)).is_none());
        assert!(detect_unauthorized_entity_order(&ctx_at(&world, 1), EntityId::null()).is_none());
    }

    #[test]
    fn test_detect_unauthorized_structure_order_flags_enemy_structure() {
        let structure = StructureId::new(4);
        let world = StaticWorldView::new().with_structure(structure, ENEMY_FACTION);
        assert!(detect_unauthorized_structure_order(&ctx_at(&world, 1), structure).is_some());
        let own = StructureId::new(5);
        let world = world.clone().with_structure(own, OWN_FACTION);
        assert!(detect_unauthorized_structure_order(&ctx_at(&world, 1), own).is_none());
    }

    // --- hidden target ------------------------------------------------------

    #[test]
    fn test_detect_hidden_target_is_silent_until_milestone_14_knowledge_exists() {
        let enemy = EntityId::new(11);
        let world = StaticWorldView::new().with_entity(enemy, ENEMY_FACTION);
        assert_eq!(
            world.faction_knows_entity(OWN_FACTION, enemy),
            KnowledgeQuery::Unavailable
        );
        assert!(detect_hidden_target_attempt(&ctx_at(&world, 1), enemy).is_none());
    }

    #[test]
    fn test_detect_hidden_target_flags_unknown_enemy_once_knowledge_exists() {
        let enemy = EntityId::new(11);
        let world = StaticWorldView::new()
            .with_entity(enemy, ENEMY_FACTION)
            .with_knowledge_system();
        let finding = detect_hidden_target_attempt(&ctx_at(&world, 1), enemy);
        assert!(matches!(
            finding,
            Some(SecurityEventKind::HiddenTargetAttempt { .. })
        ));
    }

    #[test]
    fn test_detect_hidden_target_allows_known_enemy_and_own_units() {
        let enemy = EntityId::new(11);
        let own = EntityId::new(12);
        let world = StaticWorldView::new()
            .with_entity(enemy, ENEMY_FACTION)
            .with_entity(own, OWN_FACTION)
            .with_knowledge(OWN_FACTION, enemy);
        assert!(detect_hidden_target_attempt(&ctx_at(&world, 1), enemy).is_none());
        assert!(detect_hidden_target_attempt(&ctx_at(&world, 1), own).is_none());
    }

    // --- envelope -----------------------------------------------------------

    #[test]
    fn test_detect_future_dated_command() {
        let world = StaticWorldView::new();
        let mut ctx = ctx_at(&world, 100);
        assert!(detect_future_dated_command(&ctx).is_none());
        ctx.client_tick = SimTick::new(100 + MAX_CLIENT_TICK_LEAD);
        assert!(detect_future_dated_command(&ctx).is_none());
        ctx.client_tick = SimTick::new(100_000);
        assert!(matches!(
            detect_future_dated_command(&ctx),
            Some(SecurityEventKind::FutureDatedCommand { .. })
        ));
    }

    #[test]
    fn test_detect_command_flood() {
        let world = StaticWorldView::new();
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        let ctx = ctx_at(&world, 5);
        for _ in 0..FLOOD_WINDOW_LIMIT {
            assert!(detect_command_flood(&ctx, &mut telemetry).is_none());
        }
        assert!(matches!(
            detect_command_flood(&ctx, &mut telemetry),
            Some(SecurityEventKind::CommandFloodDetected { .. })
        ));
        // Window rolls over and the session is clean again.
        let later = ctx_at(&world, 5 + FLOOD_WINDOW_TICKS);
        assert!(detect_command_flood(&later, &mut telemetry).is_none());
    }

    // --- dispatch -----------------------------------------------------------

    #[test]
    fn test_inspect_is_silent_for_legitimate_commands() {
        let own = EntityId::new(1);
        let dest = EntityId::new(2);
        let resource = ResourceId::new(10);
        let world = StaticWorldView::new()
            .with_entity(own, OWN_FACTION)
            .with_entity(dest, OWN_FACTION)
            .with_balance(own, resource, 100);
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        let commands = [
            Command::Move {
                position: (1.0, 0.0, 1.0),
                velocity: (1.0, 0.0, 0.0),
            },
            Command::TransferResource {
                from_entity: own,
                to_entity: dest,
                resource_id: resource,
                amount: 10,
            },
            Command::BuildStructure {
                kind: sim_core::structure::StructureKind::Pylon,
                position: (2.0, 0.0, 2.0),
                rotation_deg: 0.0,
            },
        ];
        for command in &commands {
            let findings = inspect(&ctx_at(&world, 5), &mut telemetry, command);
            assert!(findings.is_empty(), "{command:?} -> {findings:?}");
        }
    }

    #[test]
    fn test_inspect_routes_each_command_to_its_detector() {
        let enemy_robot = EntityId::new(20);
        let enemy_structure = StructureId::new(21);
        let own = EntityId::new(22);
        let resource = ResourceId::new(10);
        let world = StaticWorldView::new()
            .with_entity(enemy_robot, ENEMY_FACTION)
            .with_entity(own, OWN_FACTION)
            .with_structure(enemy_structure, ENEMY_FACTION)
            .with_balance(own, resource, 1);
        let mut telemetry = SessionTelemetry::new(SimTick::zero());
        let ctx = ctx_at(&world, 5);

        let unauthorized = inspect(
            &ctx,
            &mut telemetry,
            &Command::RobotCommand {
                robot_id: enemy_robot,
                command_type: RobotCommandType::ReturnToBase,
            },
        );
        assert!(
            unauthorized
                .iter()
                .any(|f| matches!(f, SecurityEventKind::UnauthorizedOrder { .. }))
        );

        let structure = inspect(
            &ctx,
            &mut telemetry,
            &Command::DismantleStructure {
                structure_id: enemy_structure,
            },
        );
        assert!(
            structure
                .iter()
                .any(|f| matches!(f, SecurityEventKind::UnauthorizedStructureOrder { .. }))
        );

        let economy = inspect(
            &ctx,
            &mut telemetry,
            &Command::ReserveResource {
                entity: own,
                resource_id: resource,
                amount: 9_999,
                reservation_id: game_types::ReservationId::new(1),
            },
        );
        assert!(
            economy
                .iter()
                .any(|f| matches!(f, SecurityEventKind::ImpossibleEconomyDelta { .. }))
        );
    }

    #[test]
    fn test_inspect_is_deterministic_for_identical_input() {
        let own = EntityId::new(1);
        let world = StaticWorldView::new().with_entity(own, OWN_FACTION);
        let command = Command::Move {
            position: (900.0, 0.0, 0.0),
            velocity: (400.0, 0.0, 0.0),
        };
        let mut a = SessionTelemetry::new(SimTick::zero());
        let mut b = SessionTelemetry::new(SimTick::zero());
        let fa = inspect(&ctx_at(&world, 5), &mut a, &command);
        let fb = inspect(&ctx_at(&world, 5), &mut b, &command);
        assert_eq!(fa, fb);
        assert!(!fa.is_empty());
    }

    #[test]
    fn test_unowned_faction_constant_is_null() {
        assert!(UNOWNED_FACTION.is_null());
    }
}
