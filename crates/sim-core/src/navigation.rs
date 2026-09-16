use game_types::EntityId;

/// Minimum fraction of maximum speed used while inside the arrival slowdown band,
/// preventing agents from asymptotically crawling toward their goal forever.
const MIN_APPROACH_FRACTION: f32 = 0.25;

/// A navigation goal for a single agent.
///
/// Milestone 12 only resolves immediate world-space goals. Milestone 16 replaces the
/// provider behind this enum with hierarchical routing (world route graph -> regional
/// corridor -> squad flow field -> local steering) without changing the call sites.
#[derive(Copy, Clone, PartialEq, Debug, Default)]
pub enum NavGoal {
    /// Hold current ground position; only local separation applies.
    #[default]
    Hold,
    /// Move to an explicit world-space position.
    Position((f32, f32, f32)),
}

/// Kinematic snapshot of the agent requesting steering.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct NavAgent {
    pub entity: EntityId,
    pub position: (f32, f32, f32),
    pub velocity: (f32, f32, f32),
}

impl NavAgent {
    pub const fn new(
        entity: EntityId,
        position: (f32, f32, f32),
        velocity: (f32, f32, f32),
    ) -> Self {
        NavAgent {
            entity,
            position,
            velocity,
        }
    }
}

/// A nearby solid body the agent must not walk through or crowd.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct NavObstacle {
    pub entity: EntityId,
    pub position: (f32, f32, f32),
    pub radius: f32,
}

impl NavObstacle {
    pub const fn new(entity: EntityId, position: (f32, f32, f32), radius: f32) -> Self {
        NavObstacle {
            entity,
            position,
            radius,
        }
    }
}

/// Per-agent tuning for a single deterministic steering evaluation.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct SteeringParams {
    pub max_speed: f32,
    /// Fixed deterministic integration step in seconds.
    pub step_seconds: f32,
    /// Distance at which the goal counts as reached.
    pub arrival_tolerance: f32,
    /// Distance at which the agent starts easing off maximum speed.
    pub slowdown_radius: f32,
    /// Personal space radius used for local separation.
    pub separation_radius: f32,
    /// Fraction of maximum speed contributed by a fully overlapping neighbour.
    pub separation_strength: f32,
    /// Physical radius of the agent body.
    pub body_radius: f32,
}

impl Default for SteeringParams {
    fn default() -> Self {
        SteeringParams {
            max_speed: 6.0,
            step_seconds: 1.0 / 30.0,
            arrival_tolerance: 0.6,
            slowdown_radius: 3.0,
            separation_radius: 1.8,
            separation_strength: 0.8,
            body_radius: 0.6,
        }
    }
}

/// Result of one steering evaluation.
#[derive(Copy, Clone, PartialEq, Debug)]
pub struct SteeringOutput {
    pub desired_velocity: (f32, f32, f32),
    pub distance_to_goal: f32,
    pub arrived: bool,
    /// True when local separation contributed to the desired velocity.
    pub separating: bool,
}

/// Swappable navigation strategy boundary.
///
/// Milestone 12 ships `DirectSteering`. Milestone 16 implements a hierarchical provider
/// and swaps it in at the registry call site; nothing else in the simulation changes.
pub trait NavigationProvider {
    /// Stable identifier of the active navigation strategy (diagnostics and metrics).
    fn name(&self) -> &'static str;

    /// Compute the desired planar velocity for one agent this step.
    fn steer(
        &self,
        agent: &NavAgent,
        goal: NavGoal,
        obstacles: &[NavObstacle],
        params: &SteeringParams,
    ) -> SteeringOutput;
}

/// Deliberately simple direct-seek steering with arrival easing and local separation.
///
/// No global search, no path caching, no flow fields: Milestone 16 owns all of that.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct DirectSteering;

impl NavigationProvider for DirectSteering {
    fn name(&self) -> &'static str {
        "direct_steering"
    }

    fn steer(
        &self,
        agent: &NavAgent,
        goal: NavGoal,
        obstacles: &[NavObstacle],
        params: &SteeringParams,
    ) -> SteeringOutput {
        let max_speed = params.max_speed.max(0.0);
        let dt = params.step_seconds.clamp(1.0e-4, 1.0);

        let (to_x, to_z, has_goal) = match goal {
            NavGoal::Hold => (0.0, 0.0, false),
            NavGoal::Position(p) => (p.0 - agent.position.0, p.2 - agent.position.2, true),
        };
        let distance_to_goal = (to_x * to_x + to_z * to_z).sqrt();
        let arrived = !has_goal || distance_to_goal <= params.arrival_tolerance;

        // 1. Goal seek with arrival easing and per-step overshoot clamping.
        let mut vx = 0.0;
        let mut vz = 0.0;
        if has_goal && !arrived && distance_to_goal > 1.0e-5 {
            let ease = if params.slowdown_radius > 1.0e-5 {
                (distance_to_goal / params.slowdown_radius).clamp(MIN_APPROACH_FRACTION, 1.0)
            } else {
                1.0
            };
            // Never step past the goal within a single fixed tick.
            let speed = (max_speed * ease).min(distance_to_goal / dt);
            let inv = 1.0 / distance_to_goal;
            vx = to_x * inv * speed;
            vz = to_z * inv * speed;
        }

        // 2. Local separation from nearby bodies (obstacle-aware spacing).
        let mut sep_x = 0.0;
        let mut sep_z = 0.0;
        for obstacle in obstacles {
            if obstacle.entity == agent.entity {
                continue;
            }
            let influence = params
                .separation_radius
                .max(params.body_radius + obstacle.radius);
            if influence <= 1.0e-5 {
                continue;
            }
            let dx = agent.position.0 - obstacle.position.0;
            let dz = agent.position.2 - obstacle.position.2;
            let dist_sq = dx * dx + dz * dz;
            if dist_sq >= influence * influence {
                continue;
            }
            let dist = dist_sq.sqrt();
            let push = (1.0 - dist / influence) * params.separation_strength * max_speed;
            if dist < 1.0e-4 {
                // Fully coincident bodies: deterministic tie-break along +X by id ordering.
                let sign = if agent.entity.value() >= obstacle.entity.value() {
                    1.0
                } else {
                    -1.0
                };
                sep_x += sign * push;
            } else {
                let inv = 1.0 / dist;
                sep_x += dx * inv * push;
                sep_z += dz * inv * push;
            }
        }

        let separating = sep_x != 0.0 || sep_z != 0.0;
        vx += sep_x;
        vz += sep_z;

        // 3. Hard speed clamp: the chassis maximum speed is never exceeded.
        let speed_sq = vx * vx + vz * vz;
        if speed_sq > max_speed * max_speed && speed_sq > 1.0e-10 {
            let scale = max_speed / speed_sq.sqrt();
            vx *= scale;
            vz *= scale;
        }

        SteeringOutput {
            desired_velocity: (vx, 0.0, vz),
            distance_to_goal,
            arrived,
            separating,
        }
    }
}

/// Planar (XZ) distance between two world positions.
pub fn planar_distance(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    let dx = a.0 - b.0;
    let dz = a.2 - b.2;
    (dx * dx + dz * dz).sqrt()
}

/// Planar (XZ) length of a vector.
pub fn planar_length(v: (f32, f32, f32)) -> f32 {
    (v.0 * v.0 + v.2 * v.2).sqrt()
}

/// A point `standoff` meters away from `target`, on the side where `from` currently stands.
///
/// This is what keeps an escort trailing its owner instead of walking into them.
pub fn standoff_position(
    target: (f32, f32, f32),
    from: (f32, f32, f32),
    standoff: f32,
) -> (f32, f32, f32) {
    let dx = from.0 - target.0;
    let dz = from.2 - target.2;
    let dist = (dx * dx + dz * dz).sqrt();
    if dist < 1.0e-4 {
        // Degenerate overlap: fall back to a fixed deterministic offset along -Z.
        (target.0, target.1, target.2 - standoff)
    } else {
        let inv = standoff / dist;
        (target.0 + dx * inv, target.1, target.2 + dz * inv)
    }
}

/// Shortest signed angular difference in degrees, wrapped to [-180, 180].
pub fn shortest_angle_delta_deg(from_deg: f32, to_deg: f32) -> f32 {
    let mut delta = (to_deg - from_deg).rem_euclid(360.0);
    if delta > 180.0 {
        delta -= 360.0;
    }
    delta
}

/// Rotate `current_deg` toward `target_deg`, limited by `max_step_deg`.
pub fn turn_toward_deg(current_deg: f32, target_deg: f32, max_step_deg: f32) -> f32 {
    let delta = shortest_angle_delta_deg(current_deg, target_deg);
    let step = delta.clamp(-max_step_deg.abs(), max_step_deg.abs());
    (current_deg + step).rem_euclid(360.0)
}

/// Compass heading in degrees for a planar velocity, or `None` when effectively stationary.
pub fn heading_deg(velocity: (f32, f32, f32), min_speed: f32) -> Option<f32> {
    if planar_length(velocity) < min_speed {
        None
    } else {
        Some(velocity.0.atan2(velocity.2).to_degrees().rem_euclid(360.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent_at(x: f32, z: f32) -> NavAgent {
        NavAgent::new(EntityId::new(1), (x, 0.0, z), (0.0, 0.0, 0.0))
    }

    #[test]
    fn test_direct_steering_seeks_goal_and_reports_arrival() {
        let nav = DirectSteering;
        let params = SteeringParams::default();

        let out = nav.steer(
            &agent_at(0.0, 0.0),
            NavGoal::Position((20.0, 0.0, 0.0)),
            &[],
            &params,
        );
        assert!((out.distance_to_goal - 20.0).abs() < 1e-4);
        assert!(!out.arrived);
        assert!(out.desired_velocity.0 > 0.0);
        assert!((out.desired_velocity.2).abs() < 1e-5);
        assert!(planar_length(out.desired_velocity) <= params.max_speed + 1e-4);

        // Inside arrival tolerance the agent stops seeking.
        let arrived = nav.steer(
            &agent_at(19.7, 0.0),
            NavGoal::Position((20.0, 0.0, 0.0)),
            &[],
            &params,
        );
        assert!(arrived.arrived);
        assert_eq!(arrived.desired_velocity, (0.0, 0.0, 0.0));
    }

    #[test]
    fn test_direct_steering_never_exceeds_max_speed_or_overshoots() {
        let nav = DirectSteering;
        let params = SteeringParams {
            max_speed: 8.0,
            ..SteeringParams::default()
        };

        // Far goal: speed clamped to chassis maximum.
        let far = nav.steer(
            &agent_at(0.0, 0.0),
            NavGoal::Position((0.0, 0.0, 500.0)),
            &[],
            &params,
        );
        assert!((planar_length(far.desired_velocity) - 8.0).abs() < 1e-3);

        // Near goal: step is clamped so a single fixed tick cannot pass the goal.
        let near = nav.steer(
            &agent_at(0.0, 0.0),
            NavGoal::Position((0.0, 0.0, 0.9)),
            &[],
            &params,
        );
        let step = planar_length(near.desired_velocity) * params.step_seconds;
        assert!(step <= near.distance_to_goal + 1e-4, "step {step} overshot");
    }

    #[test]
    fn test_direct_steering_separates_from_crowding_obstacle() {
        let nav = DirectSteering;
        let params = SteeringParams::default();

        // Agent standing on its goal but crowded from +X pushes away along -X.
        let obstacles = [NavObstacle::new(EntityId::new(2), (0.7, 0.0, 0.0), 0.6)];
        let out = nav.steer(
            &agent_at(0.0, 0.0),
            NavGoal::Position((0.0, 0.0, 0.0)),
            &obstacles,
            &params,
        );
        assert!(out.arrived);
        assert!(out.separating);
        assert!(
            out.desired_velocity.0 < 0.0,
            "expected push away from obstacle, got {:?}",
            out.desired_velocity
        );

        // Distant obstacle exerts no influence at all.
        let far = [NavObstacle::new(EntityId::new(2), (40.0, 0.0, 0.0), 0.6)];
        let quiet = nav.steer(
            &agent_at(0.0, 0.0),
            NavGoal::Position((0.0, 0.0, 0.0)),
            &far,
            &params,
        );
        assert!(!quiet.separating);
        assert_eq!(quiet.desired_velocity, (0.0, 0.0, 0.0));
    }

    #[test]
    fn test_direct_steering_is_deterministic_and_self_excluding() {
        let nav = DirectSteering;
        let params = SteeringParams::default();
        let agent = agent_at(3.0, -2.0);
        // The agent's own body appears in the obstacle slice and must be ignored.
        let obstacles = [
            NavObstacle::new(EntityId::new(1), (3.0, 0.0, -2.0), 0.6),
            NavObstacle::new(EntityId::new(9), (3.4, 0.0, -2.0), 0.6),
        ];

        let a = nav.steer(
            &agent,
            NavGoal::Position((10.0, 0.0, 10.0)),
            &obstacles,
            &params,
        );
        let b = nav.steer(
            &agent,
            NavGoal::Position((10.0, 0.0, 10.0)),
            &obstacles,
            &params,
        );
        assert_eq!(a, b, "identical inputs must give bit-identical output");
        assert!(a.desired_velocity.0.is_finite() && a.desired_velocity.2.is_finite());
    }

    #[test]
    fn test_standoff_position_keeps_escort_behind_owner() {
        let owner = (10.0, 0.0, 10.0);
        let escort = (4.0, 0.0, 10.0);
        let goal = standoff_position(owner, escort, 4.0);
        assert!((planar_distance(goal, owner) - 4.0).abs() < 1e-4);
        assert!(goal.0 < owner.0, "standoff must stay on the escort's side");

        // Degenerate full overlap resolves deterministically instead of dividing by zero.
        let overlap = standoff_position(owner, owner, 3.0);
        assert!((planar_distance(overlap, owner) - 3.0).abs() < 1e-4);
    }

    #[test]
    fn test_turn_rate_limiting_and_heading() {
        assert!((shortest_angle_delta_deg(350.0, 10.0) - 20.0).abs() < 1e-4);
        assert!((shortest_angle_delta_deg(10.0, 350.0) + 20.0).abs() < 1e-4);

        // Turning is clamped to the chassis turn rate.
        let turned = turn_toward_deg(0.0, 180.0, 30.0);
        assert!((turned - 30.0).abs() < 1e-4);

        // Heading: +Z is 0 degrees, +X is 90 degrees.
        assert_eq!(heading_deg((0.0, 0.0, 0.01), 0.05), None);
        let h = heading_deg((5.0, 0.0, 0.0), 0.05).unwrap();
        assert!((h - 90.0).abs() < 1e-3);
    }

    #[test]
    fn test_navigation_provider_is_swappable_behind_trait_object() {
        // Milestone 16 swaps in a hierarchical provider at this boundary.
        let provider: &dyn NavigationProvider = &DirectSteering;
        assert_eq!(provider.name(), "direct_steering");
        let out = provider.steer(
            &agent_at(0.0, 0.0),
            NavGoal::Position((5.0, 0.0, 0.0)),
            &[],
            &SteeringParams::default(),
        );
        assert!(out.desired_velocity.0 > 0.0);
    }
}
