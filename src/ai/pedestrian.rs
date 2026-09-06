//! Sidewalk pedestrians.
//!
//! Pedestrians walk the same road graph as traffic, offset past the kerb onto
//! the pavement. There is no separate navmesh: the city generator already
//! produces the only walkable topology that exists here, and a Recast navmesh
//! would add a heavy dependency to solve a problem the grid does not have.
//!
//! They used to be kinematic bodies whose motion was authored straight onto
//! `Transform`, which is the cheapest way to move a crowd and the only way to
//! move one that must never be pushed around. Neither property survives a city
//! made of rubber: being knocked flying by a car is the point now, and a
//! kinematic body cannot be. So they are dynamic, and where they walk is
//! expressed as a velocity the bounce controller steers towards rather than as
//! a position written each frame.
//!
//! That also retires the ground-following raycast this module used to need.
//! A dynamic body finds the kerb by landing on it.

use avian3d::prelude::*;
use bevy::prelude::*;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

use super::steering::right_of;
use crate::bounce::controller::{Bouncer, Launched};
use crate::core::config::GameConfig;
use crate::core::rng::{stream, stream_for};
use crate::core::schedule::GameSet;
use crate::mood::face::{FaceAssets, FaceLevel};
use crate::mood::feeling::{Mood, MoodRng, Temperament};
use crate::player::on_foot::Player;
use crate::world::City;
use crate::world::buildings::SIDEWALK_HEIGHT;
use crate::world::roadgraph::NodeId;

const POPULATION: usize = 45;
const SPAWN_MIN: f32 = 25.0;
const SPAWN_MAX: f32 = 110.0;
const DESPAWN: f32 = 165.0;

/// How far past the kerb the pavement centre sits.
const PAVEMENT_OFFSET: f32 = 1.9;
const RADIUS: f32 = 0.32;
const HEIGHT: f32 = 1.05;
/// Distance from the capsule's centre to its lowest point.
const STAND_HEIGHT: f32 = HEIGHT * 0.5 + RADIUS;

const WALK_SPEED: f32 = 1.5;
const FLEE_SPEED: f32 = 5.4;
/// A vehicle closer than this and moving fast enough is worth running from.
const SCARE_RADIUS: f32 = 14.0;
const SCARE_SPEED: f32 = 6.0;

#[derive(Component)]
pub struct Pedestrian {
    pub from: NodeId,
    pub to: NodeId,
    /// Which pavement: +1 right of travel, -1 left.
    pub side: f32,
    pub speed: f32,
    /// Counts down while fleeing; keeps them running a moment after the danger
    /// passes rather than snapping back to a stroll.
    pub panic: f32,
    /// Metres per second this frame. Read by the walk cycle, which paces the
    /// stride off distance covered rather than off time.
    pub current_speed: f32,
}

#[derive(Resource)]
pub struct PedestrianRng(pub ChaCha8Rng);

#[derive(Resource)]
struct PedestrianTimer(Timer);

impl Default for PedestrianTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.6, TimerMode::Repeating))
    }
}

#[derive(Resource)]
struct PedestrianAssets {
    clothes: Vec<Handle<StandardMaterial>>,
}

pub struct PedestrianPlugin;

impl Plugin for PedestrianPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PedestrianTimer>()
            .add_systems(Startup, setup)
            .add_systems(
                Update,
                (
                    maintain_population,
                    walk_pavements,
                    super::figure::pace_pedestrians,
                    super::figure::pace_player,
                    super::figure::animate,
                )
                    .chain()
                    .in_set(GameSet::Ai),
            );
    }
}

fn setup(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(PedestrianRng(stream_for(
        config.world_seed,
        stream::PEDESTRIANS,
    )));

    let palette = [
        Color::srgb(0.24, 0.30, 0.42),
        Color::srgb(0.48, 0.26, 0.24),
        Color::srgb(0.30, 0.36, 0.28),
        Color::srgb(0.55, 0.50, 0.42),
        Color::srgb(0.20, 0.22, 0.26),
        Color::srgb(0.42, 0.38, 0.52),
    ];
    commands.insert_resource(super::figure::build_assets(&mut meshes, &mut materials));
    commands.insert_resource(PedestrianAssets {
        clothes: palette
            .into_iter()
            .map(|color| {
                materials.add(StandardMaterial {
                    base_color: color,
                    perceptual_roughness: 0.85,
                    ..default()
                })
            })
            .collect(),
    });
}

/// Centre of the pavement alongside the segment `a -> b`.
fn pavement_point(a: Vec2, b: Vec2, width: f32, side: f32, t: f32) -> Vec2 {
    let Ok(direction) = Dir2::new(b - a) else {
        return a;
    };
    a.lerp(b, t) + right_of(*direction) * side * (width * 0.5 + PAVEMENT_OFFSET)
}

fn maintain_population(
    mut commands: Commands,
    time: Res<Time>,
    mut timer: ResMut<PedestrianTimer>,
    city: Res<City>,
    assets: Res<PedestrianAssets>,
    figures: Res<super::figure::FigureAssets>,
    faces: Res<FaceAssets>,
    mut rng: ResMut<PedestrianRng>,
    mut tempers: ResMut<MoodRng>,
    players: Query<&Transform, With<Player>>,
    pedestrians: Query<(Entity, &Transform), With<Pedestrian>>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let Ok(player) = players.single() else { return };
    let focus = player.translation.xz();

    let mut alive = 0usize;
    for (entity, transform) in &pedestrians {
        if transform.translation.xz().distance(focus) > DESPAWN {
            commands.entity(entity).despawn();
        } else {
            alive += 1;
        }
    }
    if alive >= POPULATION {
        return;
    }

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

    while alive < POPULATION {
        let edge = candidates[rng.0.random_range(0..candidates.len())];
        let (from, to) = if rng.0.random_range(0.0..1.0) < 0.5 {
            (edge.a, edge.b)
        } else {
            (edge.b, edge.a)
        };
        let a = city.graph.node(from).pos;
        let b = city.graph.node(to).pos;
        let side = if rng.0.random_range(0.0..1.0) < 0.5 {
            1.0
        } else {
            -1.0
        };
        let t: f32 = rng.0.random_range(0.1..0.9);
        let position = pavement_point(a, b, edge.width, side, t);
        let material = assets.clothes[rng.0.random_range(0..assets.clothes.len())].clone();

        // Drawn from its own stream: a citizen's disposition must not depend on
        // how many of them have been spawned already, and retuning the mix must
        // not move anybody's route.
        let temper = Temperament::draw(&mut tempers.0);
        let mood = temper.baseline;
        let worn = faces.wear(mood);

        let mut person = commands.spawn((
            Name::new("Pedestrian"),
            Pedestrian {
                from,
                to,
                side,
                speed: rng.0.random_range(1.1..1.9),
                panic: 0.0,
                current_speed: 0.0,
            },
            Transform::from_xyz(position.x, SIDEWALK_HEIGHT + STAND_HEIGHT, position.y),
            // Dynamic, so a car can send them across the junction.
            RigidBody::Dynamic,
            Collider::capsule(RADIUS, HEIGHT),
            // Upright until something knocks them over; `bounce::launch` takes
            // this off for as long as they are tumbling.
            LockedAxes::ROTATION_LOCKED,
            Bouncer::new(STAND_HEIGHT),
            temper,
            Mood::new(mood),
            FaceLevel(worn.level),
            Visibility::default(),
        ));
        super::figure::dress(&mut person, &figures, material, &worn, &mut rng.0);
        alive += 1;
    }
}

fn walk_pavements(
    time: Res<Time>,
    mut report: Local<f32>,
    city: Res<City>,
    mut rng: ResMut<PedestrianRng>,
    vehicles: Query<(&Transform, &LinearVelocity), With<crate::vehicle::spawn::Vehicle>>,
    mut pedestrians: Query<
        (&mut Pedestrian, &mut Bouncer, &mut Transform),
        (Without<crate::vehicle::spawn::Vehicle>, Without<Launched>),
    >,
) {
    let dt = time.delta_secs();

    // Anything moving fast enough to be worth running from.
    let threats: Vec<(Vec2, f32)> = vehicles
        .iter()
        .filter(|(_, velocity)| velocity.length() > SCARE_SPEED)
        .map(|(transform, velocity)| (transform.translation.xz(), velocity.length()))
        .collect();

    *report += dt;
    let announce = *report > 1.0;
    if announce {
        *report = 0.0;
    }
    let mut sample = None;

    for (mut pedestrian, mut bouncer, mut transform) in &mut pedestrians {
        let position = transform.translation.xz();
        let a = city.graph.node(pedestrian.from).pos;
        let b = city.graph.node(pedestrian.to).pos;

        // Arrived at the junction: pick a new street to walk down.
        if position.distance(b) < 4.0 {
            let next = city
                .graph
                .neighbors(pedestrian.to)
                .map(|(node, _)| node)
                .filter(|node| *node != pedestrian.from)
                .choose(&mut rng.0)
                .unwrap_or(pedestrian.from);
            pedestrian.from = pedestrian.to;
            pedestrian.to = next;
            continue;
        }

        let width = city
            .graph
            .neighbors(pedestrian.from)
            .find(|(node, _)| *node == pedestrian.to)
            .map(|(_, edge)| city.graph.edge(edge).width)
            .unwrap_or(9.0);

        let segment = b - a;
        let length = segment.length().max(1.0);
        let travelled = ((position - a).dot(segment) / (length * length)).clamp(0.0, 1.0);
        let target = pavement_point(
            a,
            b,
            width,
            pedestrian.side,
            (travelled + 6.0 / length).min(1.0),
        );

        let mut heading = (target - position).normalize_or_zero();

        // Bolt away from anything bearing down on them.
        pedestrian.panic = (pedestrian.panic - dt).max(0.0);
        for (threat, _) in &threats {
            let away = position - *threat;
            if away.length() < SCARE_RADIUS {
                pedestrian.panic = 1.6;
                heading = (heading + away.normalize_or_zero() * 2.0).normalize_or_zero();
            }
        }

        let speed = if pedestrian.panic > 0.0 {
            FLEE_SPEED
        } else {
            pedestrian.speed.min(WALK_SPEED * 1.3)
        };

        pedestrian.current_speed = if heading == Vec2::ZERO { 0.0 } else { speed };
        // Asked for rather than applied. The bounce controller owns the body's
        // velocity; writing the position here would fight it, and Avian would
        // hand back whichever of the two ran last.
        bouncer.desired = heading * speed;

        // Rotation is locked, so nothing else will turn them to face the way
        // they are going.
        if heading != Vec2::ZERO {
            transform.rotation =
                Quat::from_rotation_y(crate::vehicle::spawn::heading_towards(heading));
        }

        if sample.is_none() {
            sample = Some((transform.translation, speed, pedestrian.panic));
        }
    }

    if announce && let Some((position, speed, panic)) = sample {
        debug!(
            "pedestrians: {} walking, sample at {:.1},{:.2},{:.1} moving {:.2} m/s panic {:.1}",
            pedestrians.iter().len(),
            position.x,
            position.y,
            position.z,
            speed,
            panic
        );
    }
}

/// Convenience for picking a random element without collecting.
trait ChooseExt: Iterator + Sized {
    fn choose(self, rng: &mut ChaCha8Rng) -> Option<Self::Item> {
        let items: Vec<_> = self.collect();
        if items.is_empty() {
            return None;
        }
        let index = rng.random_range(0..items.len());
        items.into_iter().nth(index)
    }
}
impl<I: Iterator> ChooseExt for I {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pavements_sit_outside_the_carriageway() {
        let a = Vec2::ZERO;
        let b = Vec2::new(0.0, 100.0);
        let width = 10.0;

        for side in [1.0, -1.0] {
            let point = pavement_point(a, b, width, side, 0.5);
            let lateral = point.x.abs();
            assert!(
                lateral > width * 0.5,
                "pavement at {lateral:.2}m is still inside a {width}m road"
            );
        }
    }

    #[test]
    fn the_two_pavements_are_on_opposite_sides() {
        let a = Vec2::ZERO;
        let b = Vec2::new(100.0, 0.0);
        let left = pavement_point(a, b, 9.0, -1.0, 0.5);
        let right = pavement_point(a, b, 9.0, 1.0, 0.5);
        assert!(
            left.y * right.y < 0.0,
            "both pavements landed on the same side: {left:?} / {right:?}"
        );
    }
}
