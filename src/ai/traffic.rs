//! Ambient traffic.
//!
//! Traffic cars are ordinary vehicles: same collider, same suspension, same
//! arcade tyre model, differing only in that a system writes their
//! `VehicleInput` instead of a player. That means ramming one behaves correctly
//! for free, and a police cruiser in M5 is the same code with a different goal.
//!
//! They are not persistent. A car the player has driven away from is despawned
//! and a new one faded in ahead, because simulating a whole city's worth of
//! traffic buys nothing the player can see.

use avian3d::prelude::*;
use bevy::prelude::*;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

use super::steering::{ground_axes, lane_point, steer_towards, throttle_for_speed};
use crate::core::config::GameConfig;
use crate::core::rng::{stream, stream_for};
use crate::player::on_foot::Player;
use crate::vehicle::controller::{VehicleInput, VehicleState};
use crate::vehicle::spawn::{AlwaysSimulated, VehicleAssets, resting_height, spawn_vehicle};
use crate::vehicle::spec::VehicleClass;
use crate::world::City;
use crate::world::roadgraph::NodeId;

/// How many traffic cars to keep alive around the player.
const TRAFFIC_POPULATION: usize = 20;
/// New traffic appears between these distances — far enough not to pop in view.
const SPAWN_MIN: f32 = 70.0;
const SPAWN_MAX: f32 = 145.0;
/// Beyond this it is recycled.
const DESPAWN: f32 = 210.0;
/// Distance to a junction at which the next road is chosen.
const JUNCTION_RADIUS: f32 = 9.0;

#[derive(Component)]
pub struct TrafficDriver {
    /// The segment currently being driven, as a pair of intersections.
    pub from: NodeId,
    pub to: NodeId,
    pub lane_width: f32,
    /// Target cruising speed in m/s.
    pub cruise_speed: f32,
}

#[derive(Resource)]
pub struct TrafficRng(pub ChaCha8Rng);

#[derive(Resource)]
struct TrafficTimer(Timer);

impl Default for TrafficTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.5, TimerMode::Repeating))
    }
}

pub struct TrafficPlugin;

impl Plugin for TrafficPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrafficTimer>()
            .add_systems(Startup, seed_rng)
            .add_systems(
                Update,
                (maintain_population, drive_traffic)
                    .chain()
                    .in_set(crate::core::schedule::GameSet::Ai),
            );
    }
}

fn seed_rng(mut commands: Commands, config: Res<GameConfig>) {
    commands.insert_resource(TrafficRng(stream_for(
        config.world_seed,
        stream::VEHICLE_SPAWNS ^ 0x7AFF1C,
    )));
}

/// Picks the road to take at a junction, preferring to carry straight on.
fn choose_exit(city: &City, from: NodeId, at: NodeId, rng: &mut ChaCha8Rng) -> NodeId {
    let here = city.graph.node(at).pos;
    let incoming = (here - city.graph.node(from).pos).normalize_or_zero();

    let mut exits: Vec<(NodeId, f32)> = city
        .graph
        .neighbors(at)
        .filter(|(node, _)| *node != from)
        .map(|(node, _)| {
            let direction = (city.graph.node(node).pos - here).normalize_or_zero();
            (node, incoming.dot(direction))
        })
        .collect();

    if exits.is_empty() {
        // Dead end: the only way out is back.
        return from;
    }

    // Mostly continue straight, so traffic reads as going somewhere rather than
    // wandering; the rest of the time, turn.
    exits.sort_by(|a, b| b.1.total_cmp(&a.1));
    if rng.random_range(0.0..1.0) < 0.65 || exits.len() == 1 {
        exits[0].0
    } else {
        exits[rng.random_range(1..exits.len())].0
    }
}

fn maintain_population(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<TrafficTimer>,
    city: Res<City>,
    assets: Res<VehicleAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut rng: ResMut<TrafficRng>,
    players: Query<&Transform, With<Player>>,
    traffic: Query<(Entity, &Transform), With<TrafficDriver>>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let Ok(player) = players.single() else { return };
    let focus = player.translation.xz();

    let mut alive = 0usize;
    for (entity, transform) in &traffic {
        if transform.translation.xz().distance(focus) > DESPAWN {
            commands.entity(entity).despawn();
        } else {
            alive += 1;
        }
    }

    if alive >= TRAFFIC_POPULATION {
        return;
    }

    // Gather the eligible ring of road once, rather than sampling the whole
    // graph and rejecting: only a few percent of a 1400-edge network lies in
    // the spawn band, so rejection sampling misses far more often than it hits
    // and `Iterator::nth` makes each miss a linear scan.
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

    let mut attempts = 0;
    while alive < TRAFFIC_POPULATION && attempts < 60 {
        attempts += 1;
        let edge = candidates[rng.0.random_range(0..candidates.len())];
        let a = city.graph.node(edge.a).pos;
        let b = city.graph.node(edge.b).pos;

        // Randomly pick a direction of travel along this segment.
        let (from, to, start, end) = if rng.0.random_range(0.0..1.0) < 0.5 {
            (edge.a, edge.b, a, b)
        } else {
            (edge.b, edge.a, b, a)
        };
        let Ok(direction) = Dir2::new(end - start) else {
            continue;
        };

        let t: f32 = rng.0.random_range(0.15..0.85);
        let position = lane_point(start, end, edge.width, t);
        let class = VehicleClass::CIVILIAN[rng.0.random_range(0..VehicleClass::CIVILIAN.len())];
        let mut spec = class.spec();
        (spec.body_color, spec.body_metallic) = crate::vehicle::paint::street_paint(&mut rng.0);

        let transform = Transform::from_xyz(position.x, resting_height(&spec), position.y)
            .with_rotation(Quat::from_rotation_y(
                crate::vehicle::spawn::heading_towards(*direction),
            ));

        let cruise = rng.0.random_range(8.0..15.0);
        let vehicle = spawn_vehicle(&mut commands, &assets, &mut materials, spec, transform);
        commands.entity(vehicle).insert((
            TrafficDriver {
                from,
                to,
                lane_width: edge.width,
                cruise_speed: cruise,
            },
            AlwaysSimulated,
        ));

        alive += 1;
    }

    debug!(
        "traffic: {alive}/{TRAFFIC_POPULATION} alive, {} candidate segments, {attempts} attempts",
        candidates.len()
    );
}

fn drive_traffic(
    city: Res<City>,
    spatial: SpatialQuery,
    mut rng: ResMut<TrafficRng>,
    mut cars: Query<(
        Entity,
        &mut TrafficDriver,
        &Transform,
        &VehicleState,
        &mut VehicleInput,
    )>,
) {
    for (entity, mut driver, transform, state, mut input) in &mut cars {
        let position = transform.translation.xz();
        let start = city.graph.node(driver.from).pos;
        let end = city.graph.node(driver.to).pos;

        // Hand over to the next segment on arrival at the junction.
        if position.distance(end) < JUNCTION_RADIUS {
            let next = choose_exit(&city, driver.from, driver.to, &mut rng.0);
            driver.from = driver.to;
            driver.to = next;
            if let Some(edge) = city
                .graph
                .neighbors(driver.from)
                .find(|(node, _)| *node == next)
                .map(|(_, edge)| city.graph.edge(edge).width)
            {
                driver.lane_width = edge;
            }
            continue;
        }

        // Pure pursuit: aim at a point further along the lane the faster we go,
        // which is what stops the car sawing at the wheel on a straight.
        let segment = end - start;
        let length = segment.length().max(1.0);
        let travelled = ((position - start).dot(segment) / (length * length)).clamp(0.0, 1.0);
        let lookahead = 7.0 + state.forward_speed.abs() * 0.85;
        let target_t = (travelled + lookahead / length).min(1.0);
        let target = lane_point(start, end, driver.lane_width, target_t);

        let (forward, right) = ground_axes(transform);
        input.steer = steer_towards(forward, right, target - position);

        // Ease off through corners, and stop for whatever is in the way.
        let cornering = 1.0 - input.steer.abs() * 0.55;
        let desired = driver.cruise_speed * cornering;

        let stopping = 5.0 + state.forward_speed.abs() * 1.4;
        let nose = transform.translation + *transform.forward() * 2.6 + Vec3::Y * 0.2;
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
        let blocked = Dir3::new(*transform.forward())
            .ok()
            .and_then(|d| spatial.cast_ray(nose, d, stopping, true, &filter))
            .is_some();

        input.throttle = if blocked {
            -1.0
        } else {
            throttle_for_speed(state.forward_speed, desired)
        };
        input.handbrake = false;
    }
}
