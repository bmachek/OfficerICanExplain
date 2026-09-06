//! Making peace, with a flower.
//!
//! The taunt and the cheer are broadcasts; this is the game's one targeted
//! gesture. Press the button and the player says sorry and throws a flower to
//! the nearest flummi — preferring whoever is currently holding a grudge
//! against them, because that is who the apology is *for*. If the flower
//! reaches them it is the single biggest mood lift in the game, the grudge
//! against the thrower is dropped on the spot, and the recipient does a
//! pirouette towards them: feud over, and legibly over from across the
//! street.
//!
//! The flower is a real thrown body, not a ray — it arcs, it can bounce off
//! a lamppost, and an apology can genuinely miss. That is on purpose. An
//! apology that cannot fail is a button that dispenses forgiveness, and
//! nobody would feel anything about pressing it.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use rand::RngExt;

use super::feeling::Mood;
use super::grudge::{Grudge, Pirouette};
use super::provoke::Provoker;
use crate::audio::bank::SoundBank;
use crate::audio::{AudioRng, close_once, effect_gain};
use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;
use crate::player::input::Action;
use crate::player::on_foot::Player;

/// Seconds the flower is in the air before the throw is a miss.
const FLIGHT_TIME: f32 = 0.9;
/// How close the flower must pass to count as received.
const CATCH_REACH: f32 = 1.6;
/// Seconds a missed flower lies where it fell before wilting away.
const WILT_AFTER: f32 = 4.0;
/// How long the receiver spins for. Longer than an ordinary delighted
/// pirouette: this one is a thank-you, and it should be seen.
const THANKS_SPIN: f32 = 2.2;

/// A flower in flight, and who it is meant for.
#[derive(Component)]
pub struct Flower {
    pub by: Entity,
    pub target: Entity,
    pub age: f32,
}

/// One flower's meshes, shared by every apology ever thrown.
#[derive(Resource)]
struct FlowerAssets {
    stem: (Handle<Mesh>, Handle<StandardMaterial>),
    petal: (Handle<Mesh>, Handle<StandardMaterial>),
    heart: (Handle<Mesh>, Handle<StandardMaterial>),
}

pub struct ApologyPlugin;

impl Plugin for ApologyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, build_flower).add_systems(
            Update,
            (player_apologizes, npcs_apologize, fly_flowers)
                .chain()
                .in_set(GameSet::Ai),
        );
    }
}

fn build_flower(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(FlowerAssets {
        stem: (
            meshes.add(Cylinder::new(0.015, 0.34)),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.25, 0.62, 0.22),
                perceptual_roughness: 0.9,
                ..default()
            }),
        ),
        petal: (
            meshes.add(Sphere::new(0.055)),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.97, 0.83, 0.90),
                perceptual_roughness: 0.8,
                ..default()
            }),
        ),
        heart: (
            meshes.add(Sphere::new(0.045)),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.98, 0.78, 0.15),
                perceptual_roughness: 0.7,
                ..default()
            }),
        ),
    });
}

/// The velocity that lands a throw from `from` on `to` in [`FLIGHT_TIME`],
/// under gravity. Pure ballistics, pure function — the arc is the visible
/// half of the apology and it should be arguable in a test.
pub fn throw_velocity(from: Vec3, to: Vec3) -> Vec3 {
    let delta = to - from;
    Vec3::new(
        delta.x / FLIGHT_TIME,
        delta.y / FLIGHT_TIME + 0.5 * 9.81 * FLIGHT_TIME,
        delta.z / FLIGHT_TIME,
    )
}

fn player_apologizes(
    mut commands: Commands,
    config: Res<GameConfig>,
    assets: Res<FlowerAssets>,
    bank: Option<Res<SoundBank>>,
    mut rng: ResMut<AudioRng>,
    mut players: Query<(Entity, &Transform, &ActionState<Action>, &mut Provoker), With<Player>>,
    flummis: Query<(Entity, &Transform, Option<&Grudge>), (With<Mood>, Without<Player>)>,
) {
    let Ok((player, transform, actions, mut provoker)) = players.single_mut() else {
        return;
    };
    // The cooldown is shared with the taunt and the cheer on purpose: sorry,
    // and a raspberry in the same breath, is neither. Only read here — the
    // provoke system already ages it, and both of them subtracting the same
    // frame's time made every rest half as long as the dial says.
    if provoker.cooldown > 0.0 || !actions.just_pressed(&Action::Apologize) {
        return;
    }
    provoker.cooldown = config.mood.provoke_rest;

    // The voice comes before the question of who is listening: an apology to
    // an empty street is still audibly an apology, and a button that stays
    // silent whenever nobody happens to be in range reads as a button that
    // does not work. Only the flower needs somebody to catch it. And it is
    // the player's own mouth, so it plays flat in both ears rather than
    // through the spatial mixer — the same rule as their footsteps.
    let from = transform.translation + Vec3::Y * 0.5;
    if let Some(bank) = bank {
        commands.spawn((
            AudioPlayer(bank.sorry.clone()),
            close_once(effect_gain(&config, 0.8)).with_speed(rng.random_range(0.95..1.1)),
            Transform::from_translation(from),
        ));
    }

    // Whoever holds a grudge against the player, else whoever is nearest:
    // the apology finds the person it is owed to before the person it is
    // convenient to.
    let reach = config.mood.apology_range;
    let target = flummis
        .iter()
        .filter_map(|(entity, at, grudge)| {
            let apart = at.translation.distance(transform.translation);
            (apart < reach).then_some((entity, at.translation, grudge, apart))
        })
        .min_by(|a, b| {
            let owed = |g: &Option<&Grudge>| !matches!(g, Some(g) if g.against == player);
            (owed(&a.2), a.3).partial_cmp(&(owed(&b.2), b.3)).unwrap()
        });
    let Some((target, at, _, _)) = target else {
        return;
    };

    spawn_flower(&mut commands, &assets, player, target, from, at);
}

/// Out of the hand, not the navel, and aimed at the chest, not the feet.
fn spawn_flower(
    commands: &mut Commands,
    assets: &FlowerAssets,
    by: Entity,
    target: Entity,
    from: Vec3,
    at: Vec3,
) {
    commands
        .spawn((
            Name::new("Flower"),
            Flower {
                by,
                target,
                age: 0.0,
            },
            Transform::from_translation(from),
            Visibility::default(),
            RigidBody::Dynamic,
            Collider::sphere(0.09),
            Mass(0.25),
            LinearVelocity(throw_velocity(from, at + Vec3::Y * 0.4)),
            // A lazy tumble. A flower thrown flat like a dart is a weapon.
            AngularVelocity(Vec3::new(3.5, 0.6, 1.2)),
        ))
        .with_children(|flower| {
            let (stem, green) = &assets.stem;
            flower.spawn((
                Mesh3d(stem.clone()),
                MeshMaterial3d(green.clone()),
                Transform::from_xyz(0.0, -0.17, 0.0),
            ));
            let (heart, gold) = &assets.heart;
            flower.spawn((Mesh3d(heart.clone()), MeshMaterial3d(gold.clone())));
            let (petal, pink) = &assets.petal;
            for step in 0..6 {
                let angle = std::f32::consts::TAU * step as f32 / 6.0;
                flower.spawn((
                    Mesh3d(petal.clone()),
                    MeshMaterial3d(pink.clone()),
                    Transform::from_xyz(angle.cos() * 0.09, 0.0, angle.sin() * 0.09),
                ));
            }
        });
}

/// Chance per second that a flummi being hunted sues for peace.
const CONTRITION: f32 = 0.35;
/// Mood below which they are too sour to say sorry to anybody.
const CONTRITE: f32 = 0.1;

/// Peace-making was player-exclusive, which was asymmetric with everything
/// else in `mood` — NPCs taunt, cheer, grudge and pirouette through exactly
/// the player's code paths. Now the flower is shared too: a flummi that
/// notices it is being *hunted* (it is the `against` of somebody's grudge)
/// and is in a good enough mood turns and throws one at its pursuer. The
/// scene it makes is the point: chase, lob, catch, thank-you pirouette, feud
/// over — a whole street soap with no player anywhere in it.
fn npcs_apologize(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<GameConfig>,
    assets: Res<FlowerAssets>,
    bank: Option<Res<SoundBank>>,
    mut rng: ResMut<AudioRng>,
    grudges: Query<(Entity, &Grudge)>,
    holders: Query<&Transform, Without<Player>>,
    mut culprits: Query<(&Transform, &Mood, &mut Provoker), Without<Player>>,
) {
    let dt = time.delta_secs();
    for (holder, grudge) in &grudges {
        let Ok((transform, mood, mut provoker)) = culprits.get_mut(grudge.against) else {
            continue;
        };
        if mood.value < CONTRITE || provoker.cooldown > 0.0 {
            continue;
        }
        if rng.random::<f32>() > CONTRITION * dt {
            continue;
        }
        let Ok(pursuer) = holders.get(holder) else {
            continue;
        };
        let at = pursuer.translation;
        if transform.translation.distance(at) > config.mood.apology_range {
            continue;
        }
        // Twice the player's rest: an NPC that machine-gunned flowers would
        // drain every feud before anybody saw one.
        provoker.cooldown = config.mood.provoke_rest * 2.0;

        let from = transform.translation + Vec3::Y * 0.5;
        if let Some(bank) = &bank {
            commands.spawn((
                AudioPlayer(bank.sorry.clone()),
                crate::audio::spatial_once(effect_gain(&config, 0.7), 24.0)
                    .with_speed(rng.random_range(0.9..1.15)),
                Transform::from_translation(from),
            ));
        }
        spawn_flower(&mut commands, &assets, grudge.against, holder, from, at);
    }
}

/// Flies every flower to its verdict: received, or wilted where it fell.
fn fly_flowers(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<GameConfig>,
    mut flowers: Query<(Entity, &mut Flower, &Transform)>,
    mut receivers: Query<(&Transform, &mut Mood, Option<&Grudge>), Without<Flower>>,
) {
    for (entity, mut flower, at) in &mut flowers {
        flower.age += time.delta_secs();

        if let Ok((target, mut mood, grudge)) = receivers.get_mut(flower.target)
            && target.translation.distance(at.translation) < CATCH_REACH
        {
            mood.value = (mood.value + config.mood.apology_balm).clamp(-1.0, 1.0);
            let mut received = commands.entity(flower.target);
            received.insert(Pirouette {
                left: THANKS_SPIN,
                towards: Some(flower.by),
            });
            // Only the feud with the thrower is settled. A grudge against
            // somebody else is not this flower's business.
            if matches!(grudge, Some(grudge) if grudge.against == flower.by) {
                received.remove::<Grudge>();
            }
            commands.entity(entity).despawn();
            info!("apology received");
            continue;
        }

        if flower.age > WILT_AFTER {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_throw_arcs_up_and_lands_on_the_target() {
        let from = Vec3::new(0.0, 1.5, 0.0);
        let to = Vec3::new(6.0, 1.0, 0.0);
        let velocity = throw_velocity(from, to);
        assert!(velocity.y > 0.0, "a throw with no arc is a push");

        // Integrate the ballistics and check it arrives on schedule.
        let landing =
            from + velocity * FLIGHT_TIME - Vec3::Y * 0.5 * 9.81 * FLIGHT_TIME * FLIGHT_TIME;
        assert!(
            landing.distance(to) < 0.01,
            "aimed at {to:?}, landed at {landing:?}"
        );
    }

    #[test]
    fn a_longer_throw_is_a_flatter_faster_one_rather_than_a_higher_one() {
        // Same flight time whatever the range, so the lob stays readable:
        // range shows up as speed, not as a mortar shot over the rooftops.
        let short = throw_velocity(Vec3::ZERO, Vec3::new(3.0, 0.0, 0.0));
        let long = throw_velocity(Vec3::ZERO, Vec3::new(12.0, 0.0, 0.0));
        assert_eq!(short.y, long.y);
        assert!(long.x > short.x * 3.0);
    }
}
