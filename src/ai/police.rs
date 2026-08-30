//! Police pursuit.
//!
//! Units are ordinary vehicles driven by an AI goal, the same as traffic — a
//! cruiser crashes, takes damage and can be rammed exactly like a civilian car
//! because it *is* one, with a different paint job and a different objective.
//!
//! The pursuit is a three-state machine. `Responding` drives to where the
//! player was last reported. `Chasing` runs when an officer can actually see
//! them. `Searching` sweeps the last known position after losing sight, which
//! is what gives the player the seconds they need to get out of view and stay
//! there — the pursuit does not simply teleport its knowledge.

use avian3d::prelude::*;
use bevy::prelude::*;
use rand::RngExt;

use super::steering::{ground_axes, lane_point, steer_towards, throttle_for_speed};
use super::traffic::TrafficRng;
use crate::core::schedule::GameSet;
use crate::crime::wanted::Wanted;
use crate::player::on_foot::Player;
use crate::vehicle::controller::{VehicleInput, VehicleState};
use crate::vehicle::spawn::{
    AlwaysSimulated, Vehicle, VehicleAssets, heading_towards, resting_height, spawn_vehicle,
};
use crate::vehicle::spec::VehicleClass;
use crate::world::City;
use crate::world::roadgraph::NodeId;

/// How far an officer can see, given a clear line.
const SIGHT_RANGE: f32 = 95.0;
/// Anything blocking by more than this much breaks the line of sight.
const OCCLUSION_SLACK: f32 = 2.5;
/// Within this range a chasing unit abandons the road network and drives at
/// the player directly.
const DIRECT_RANGE: f32 = 55.0;
/// How long a unit sweeps a last known position before giving up.
const SEARCH_TIME: f32 = 14.0;
/// Below this speed, a unit that wants to be moving counts as stuck.
const STUCK_SPEED: f32 = 1.0;
/// How long to tolerate being stuck before backing out.
const STUCK_PATIENCE: f32 = 1.3;
/// How long to reverse for once wedged.
const REVERSE_TIME: f32 = 0.9;

const SPAWN_MIN: f32 = 95.0;
const SPAWN_MAX: f32 = 190.0;
const DESPAWN: f32 = 320.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PursuitState {
    /// Heading for the last reported position.
    Responding,
    /// Eyes on the target.
    Chasing,
    /// Lost them; sweeping where they were last seen.
    Searching,
}

#[derive(Component)]
pub struct PoliceUnit {
    pub state: PursuitState,
    /// Road-graph route to the current goal.
    pub route: Vec<NodeId>,
    pub route_index: usize,
    /// Seconds until the route is recomputed.
    pub repath_in: f32,
    pub search_left: f32,
    /// True while this unit personally has line of sight.
    pub has_sight: bool,
    /// How long this unit has been trying and failing to move.
    pub stuck_for: f32,
    /// Time left backing out of whatever it got wedged against.
    pub reversing_for: f32,
}

impl Default for PoliceUnit {
    fn default() -> Self {
        Self {
            state: PursuitState::Responding,
            route: Vec::new(),
            route_index: 0,
            repath_in: 0.0,
            search_left: SEARCH_TIME,
            has_sight: false,
            stuck_for: 0.0,
            reversing_for: 0.0,
        }
    }
}

/// How many cruisers each wanted level puts on the street.
pub fn units_for_stars(stars: u8) -> usize {
    match stars {
        0 => 0,
        1 => 1,
        2 => 2,
        3 => 4,
        4 => 6,
        _ => 8,
    }
}

/// Below two stars they shadow the player; above it they drive to stop them.
pub fn will_ram(stars: u8) -> bool {
    stars >= 2
}

/// Cruising speed, in m/s, at each wanted level.
pub fn pursuit_speed(stars: u8) -> f32 {
    match stars {
        0 | 1 => 13.0,
        2 => 17.0,
        3 => 21.0,
        4 => 25.0,
        _ => 29.0,
    }
}

#[derive(Resource)]
struct DispatchTimer(Timer);

impl Default for DispatchTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(1.0, TimerMode::Repeating))
    }
}

pub struct PolicePlugin;

impl Plugin for PolicePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DispatchTimer>().add_systems(
            Update,
            (observe_player, dispatch_units, drive_units)
                .chain()
                .in_set(GameSet::Ai),
        );
    }
}

/// Line-of-sight checks, and the one place `Wanted` is told whether the player
/// is currently visible.
fn observe_player(
    time: Res<Time>,
    spatial: SpatialQuery,
    mut wanted: ResMut<Wanted>,
    players: Query<(Entity, &GlobalTransform), With<Player>>,
    mut units: Query<(Entity, &GlobalTransform, &mut PoliceUnit)>,
) {
    let Ok((player, player_transform)) = players.single() else {
        return;
    };
    let target = player_transform.translation() + Vec3::Y * 0.9;

    let mut anyone_sees = false;
    for (entity, transform, mut unit) in &mut units {
        let eye = transform.translation() + Vec3::Y * 1.2;
        let offset = target - eye;
        let distance = offset.length();

        unit.has_sight = distance <= SIGHT_RANGE
            && Dir3::new(offset)
                .ok()
                .map(|direction| {
                    let filter = SpatialQueryFilter::from_excluded_entities([entity, player]);
                    match spatial.cast_ray(eye, direction, distance, true, &filter) {
                        // Only something substantially in the way counts as cover;
                        // clipping a kerb or a wing mirror should not hide anyone.
                        Some(hit) => distance - hit.distance < OCCLUSION_SLACK,
                        None => true,
                    }
                })
                .unwrap_or(false);

        anyone_sees |= unit.has_sight;
    }

    wanted.tick(
        time.delta_secs(),
        anyone_sees,
        player_transform.translation(),
    );
}

fn dispatch_units(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<DispatchTimer>,
    wanted: Res<Wanted>,
    city: Res<City>,
    assets: Res<VehicleAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut rng: ResMut<TrafficRng>,
    players: Query<&Transform, With<Player>>,
    units: Query<(Entity, &Transform), With<PoliceUnit>>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let Ok(player) = players.single() else { return };
    let focus = player.translation.xz();
    let target_count = units_for_stars(wanted.stars());

    let mut alive = 0usize;
    for (entity, transform) in &units {
        let too_far = transform.translation.xz().distance(focus) > DESPAWN;
        if too_far || target_count == 0 {
            commands.entity(entity).despawn();
        } else {
            alive += 1;
        }
    }
    if alive >= target_count {
        return;
    }

    // Cruisers arrive from off-screen, on real roads.
    let candidates: Vec<_> = city
        .graph
        .edges()
        .filter(|edge| {
            let midpoint = city
                .graph
                .node(edge.a)
                .pos
                .midpoint(city.graph.node(edge.b).pos);
            (SPAWN_MIN..SPAWN_MAX).contains(&midpoint.distance(focus))
        })
        .collect();
    if candidates.is_empty() {
        return;
    }

    while alive < target_count {
        let edge = candidates[rng.0.random_range(0..candidates.len())];
        let a = city.graph.node(edge.a).pos;
        let b = city.graph.node(edge.b).pos;
        let Ok(direction) = Dir2::new(b - a) else {
            continue;
        };

        let position = lane_point(a, b, edge.width, 0.5);
        let spec = VehicleClass::Police.spec();
        let transform = Transform::from_xyz(position.x, resting_height(&spec), position.y)
            .with_rotation(Quat::from_rotation_y(heading_towards(*direction)));

        let vehicle = spawn_vehicle(&mut commands, &assets, &mut materials, spec, transform);
        commands.entity(vehicle).insert((
            PoliceUnit::default(),
            AlwaysSimulated,
            crate::crime::wanted::Witness,
        ));
        alive += 1;
    }
}

fn drive_units(
    time: Res<Time>,
    city: Res<City>,
    spatial: SpatialQuery,
    wanted: Res<Wanted>,
    players: Query<&Transform, With<Player>>,
    mut units: Query<
        (
            Entity,
            &mut PoliceUnit,
            &Transform,
            &VehicleState,
            &mut VehicleInput,
        ),
        With<Vehicle>,
    >,
) {
    let Ok(player) = players.single() else { return };
    let dt = time.delta_secs();
    let stars = wanted.stars();
    let speed = pursuit_speed(stars);

    for (entity, mut unit, transform, state, mut input) in &mut units {
        // Where do we believe the target is?
        let goal = if unit.has_sight {
            unit.state = PursuitState::Chasing;
            unit.search_left = SEARCH_TIME;
            Some(player.translation.xz())
        } else {
            match unit.state {
                PursuitState::Chasing => {
                    // Just lost them.
                    unit.state = PursuitState::Searching;
                    unit.search_left = SEARCH_TIME;
                    wanted.last_known.map(|p| p.xz())
                }
                PursuitState::Searching => {
                    unit.search_left -= dt;
                    if unit.search_left <= 0.0 {
                        unit.state = PursuitState::Responding;
                    }
                    wanted.last_known.map(|p| p.xz())
                }
                PursuitState::Responding => wanted.last_known.map(|p| p.xz()),
            }
        };

        let Some(goal) = goal else {
            input.throttle = 0.0;
            input.steer = 0.0;
            input.handbrake = true;
            continue;
        };

        let position = transform.translation.xz();
        let straight_line = position.distance(goal);

        // Close enough to abandon the road network and just go for them.
        let aim = if unit.has_sight && straight_line < DIRECT_RANGE {
            goal
        } else {
            unit.repath_in -= dt;
            if unit.repath_in <= 0.0 || unit.route_index >= unit.route.len() {
                unit.repath_in = 1.5;
                unit.route_index = 0;
                unit.route = route_between(&city, position, goal);
            }
            next_waypoint(&city, &mut unit, position, forward_of(transform)).unwrap_or(goal)
        };

        let (forward, right) = ground_axes(transform);
        input.steer = steer_towards(forward, right, aim - position);

        // Backing out of a wedge. Braking when blocked has no recovery on its
        // own: two units that meet nose to nose both stop and stay stopped, and
        // the whole pursuit quietly parks a hundred metres short.
        if unit.reversing_for > 0.0 {
            unit.reversing_for -= dt;
            input.throttle = -1.0;
            // Steer the opposite way while reversing, so it backs out at an
            // angle rather than straight into whatever it just came from.
            input.steer = -input.steer;
            input.handbrake = false;
            continue;
        }
        if state.forward_speed.abs() < STUCK_SPEED && straight_line > DIRECT_RANGE * 0.4 {
            unit.stuck_for += dt;
            if unit.stuck_for > STUCK_PATIENCE {
                unit.stuck_for = 0.0;
                unit.reversing_for = REVERSE_TIME;
            }
        } else {
            unit.stuck_for = 0.0;
        }

        // Ease off in corners, and do not plough into whatever is directly ahead
        // unless ramming is authorised and it is the target.
        let cornering = 1.0 - input.steer.abs() * 0.45;
        let desired = speed * cornering;

        let stopping = 4.0 + state.forward_speed.abs() * 1.1;
        let nose = transform.translation + *transform.forward() * 2.6 + Vec3::Y * 0.2;
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
        let blocked = Dir3::new(*transform.forward())
            .ok()
            .and_then(|d| spatial.cast_ray(nose, d, stopping, true, &filter))
            .is_some();

        let ramming = will_ram(stars) && unit.has_sight && straight_line < DIRECT_RANGE;
        input.throttle = if blocked && !ramming {
            -1.0
        } else {
            throttle_for_speed(state.forward_speed, desired)
        };
        input.handbrake = false;
    }
}

/// A road-graph route between two world positions, excluding where we already are.
///
/// A* is asked for a path from the intersection nearest the car, which is
/// normally the one it has just driven through. Keeping that node makes the
/// unit turn around for it, arrive, repath, and orbit the junction forever —
/// the pursuit stalls a hundred metres out and never closes. Checking whether
/// the node is "behind" is not enough on its own: on a route that turns, the
/// junction just left sits perpendicular rather than behind, and survives the
/// check. Dropping the first node removes the problem at its source.
pub fn route_between(city: &City, from: Vec2, to: Vec2) -> Vec<NodeId> {
    let (Some(start), Some(goal)) = (city.graph.nearest_node(from), city.graph.nearest_node(to))
    else {
        return Vec::new();
    };
    let Some(path) = city.graph.path(start, goal) else {
        return Vec::new();
    };
    if path.len() <= 1 {
        return path;
    }
    path.into_iter().skip(1).collect()
}

pub fn forward_of(transform: &Transform) -> Vec2 {
    let forward = transform.forward();
    Vec2::new(forward.x, forward.z).normalize_or_zero()
}

/// Advances along the route and returns the point to steer at.
///
/// Skips waypoints that are already reached *or* that lie behind the unit.
/// A fresh A* route starts at the intersection nearest the car, which is
/// usually the one it has just driven through; without the behind-check a unit
/// turns around for it, reaches it, repaths, and circles that junction forever
/// while the pursuit never closes.
fn next_waypoint(
    city: &City,
    unit: &mut PoliceUnit,
    position: Vec2,
    forward: Vec2,
) -> Option<Vec2> {
    while unit.route_index < unit.route.len() {
        let node = city.graph.node(unit.route[unit.route_index]).pos;
        let offset = node - position;
        let reached = offset.length() < 12.0;
        // Only abandon a waypoint for being behind us if there is another one
        // to aim at; otherwise we would discard the destination itself.
        let behind = offset.normalize_or_zero().dot(forward) < 0.0
            && unit.route_index + 1 < unit.route.len();

        if reached || behind {
            unit.route_index += 1;
            continue;
        }
        return Some(node);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_escalates_with_the_star_level() {
        let counts: Vec<usize> = (0..=5).map(units_for_stars).collect();
        assert_eq!(counts[0], 0, "no stars, no police");
        for pair in counts.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "response must never shrink as stars rise: {counts:?}"
            );
        }
        assert!(counts[5] > counts[1], "five stars must dwarf one");
    }

    #[test]
    fn one_star_is_survivable_and_higher_is_not() {
        assert!(!will_ram(1), "a single unit should shadow, not ram");
        assert!(will_ram(2));
        assert!(will_ram(5));
    }

    /// Regression: units used to orbit the junction they had just left,
    /// because every repath handed them a route beginning at that junction.
    #[test]
    fn a_fresh_route_does_not_begin_where_we_already_stand() {
        let city = City(crate::world::citygen::generate(0xA17E_5EED, 600.0));
        let here = Vec2::ZERO;
        let route = route_between(&city, here, Vec2::new(250.0, 250.0));

        assert!(!route.is_empty(), "expected a route across the grid");
        let standing_on = city.graph.nearest_node(here).unwrap();
        assert_ne!(
            route[0], standing_on,
            "the route must not send us back to the junction we are on"
        );
        // And it must still actually reach the destination.
        let goal = city.graph.nearest_node(Vec2::new(250.0, 250.0)).unwrap();
        assert_eq!(*route.last().unwrap(), goal);
    }

    #[test]
    fn route_following_advances_past_waypoints_it_has_reached() {
        let city = City(crate::world::citygen::generate(0xA17E_5EED, 600.0));
        let route = route_between(&city, Vec2::ZERO, Vec2::new(250.0, 250.0));
        assert!(route.len() >= 2);

        // Standing right on the first waypoint, facing the second.
        let standing_on = city.graph.node(route[0]).pos;
        let onwards = (city.graph.node(route[1]).pos - standing_on).normalize();
        let mut unit = PoliceUnit {
            route: route.clone(),
            ..default()
        };

        let aim = next_waypoint(&city, &mut unit, standing_on, onwards).unwrap();
        assert!(
            unit.route_index >= 1,
            "should have consumed the reached node"
        );
        assert_ne!(aim, standing_on, "must not aim at where it already is");
    }

    #[test]
    fn the_last_waypoint_is_never_discarded() {
        // Even facing away from it, the destination itself must be kept, or a
        // unit that overshoots simply forgets where it was going.
        let city = City(crate::world::citygen::generate(7, 400.0));
        let goal = city.graph.nearest_node(Vec2::new(120.0, 0.0)).unwrap();
        let goal_pos = city.graph.node(goal).pos;

        let mut unit = PoliceUnit {
            route: vec![goal],
            ..default()
        };
        let standing = goal_pos - Vec2::new(40.0, 0.0);
        let facing_away = Vec2::new(-1.0, 0.0);
        assert_eq!(
            next_waypoint(&city, &mut unit, standing, facing_away),
            Some(goal_pos)
        );
    }

    #[test]
    fn pursuit_speed_rises_with_stars() {
        let speeds: Vec<f32> = (0..=5).map(pursuit_speed).collect();
        for pair in speeds.windows(2) {
            assert!(pair[1] >= pair[0], "speeds must be monotonic: {speeds:?}");
        }
    }
}
