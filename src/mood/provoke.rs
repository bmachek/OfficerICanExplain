//! Being rude to strangers, and being nice to them.
//!
//! This is the verb the game is built around, and it replaces shooting. The two
//! buttons a shooter puts a gun on now blow a raspberry and whistle, and both
//! reach everybody within a few metres rather than whatever is under a
//! crosshair — because the interesting thing was never who you hit, it is what
//! the crowd does about it afterwards.
//!
//! Two directions rather than one, and that is the whole design. A city with
//! only a taunt has one slider that goes down, which is a stress toy. With a
//! whistle as well the mood becomes something to steer: wind a street up, watch
//! a Wutbürger come after you, cheer the neighbours until they calm him down
//! again. The whistle is deliberately the wider of the two, so a mess is easier
//! to make than to clear up but never impossible to clear up.
//!
//! **NPCs use exactly the same code.** A flummi that has had enough taunts by
//! itself, and a delighted one whistles, and both go through
//! [`Provocation`] like the player's do. That is the whole of the "NPCs react
//! to each other" requirement: nothing here knows or cares which of the
//! provokers is holding the mouse.

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use rand::RngExt;

use super::feeling::{Mood, Temperament};
use crate::audio::bank::{SoundBank, VARIANTS};
use crate::audio::{AudioRng, close_once, effect_gain, spatial_once};
use crate::core::config::{GameConfig, MoodConfig};
use crate::core::schedule::GameSet;
use crate::player::input::Action;
use crate::player::on_foot::Player;

/// How far a taunt or a cheer is heard, in metres.
const EARSHOT: f32 = 26.0;
const GAIN: f32 = 0.9;

/// Mood below which a flummi starts being rude without being asked, and above
/// which it starts whistling at people.
const SPONTANEOUS_SPITE: f32 = -0.5;
const SPONTANEOUS_JOY: f32 = 0.55;
/// Chance per second that a flummi in one of those moods acts on it, at the
/// extremes of the scale. Low: an NPC that provokes constantly is a hazard
/// rather than a character.
const SPONTANEITY: f32 = 0.22;

/// Seconds the ripple takes to reach its full width and vanish.
const RIPPLE_LIFE: f32 = 0.45;
/// Height above the ground the ring is drawn at, so it is not fighting the road
/// surface for the same depth.
const RIPPLE_LIFT: f32 = 0.06;

/// Which way somebody was rude.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rudeness {
    Taunt,
    Cheer,
}

/// Somebody made their feelings known.
#[derive(Message, Clone, Copy, Debug)]
pub struct Provocation {
    /// Who did it. Carried because a grudge needs somebody to be held against
    /// — see [`super::grudge`] — and because nobody should be offended by
    /// their own raspberry.
    pub by: Entity,
    pub at: Vec3,
    pub kind: Rudeness,
}

/// When somebody may next be rude.
///
/// On every flummi including the player, because the limit is the same for
/// both: a raspberry every frame is not a provocation, it is a texture.
#[derive(Component, Default)]
pub struct Provoker {
    pub cooldown: f32,
}

/// An expanding ring on the ground, marking where something was said.
#[derive(Component)]
struct Ripple {
    age: f32,
    reach: f32,
    material: Handle<StandardMaterial>,
    colour: LinearRgba,
}

/// Everything that produces or applies a [`Provocation`].
///
/// Exported so a grudge can be taken in the same frame the offence happened
/// rather than in the next one, which at sixty a second is not visible but is
/// one less thing to be wrong about later.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Provoking;

pub struct ProvokePlugin;

impl Plugin for ProvokePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Provocation>()
            .add_systems(Startup, build_ring)
            .add_systems(
                Update,
                (
                    (player_provokes, npcs_provoke),
                    (feel_provocations, spread_ripples),
                )
                    .chain()
                    .in_set(Provoking)
                    .in_set(GameSet::Ai)
                    .after(crate::ai::pedestrian::Walking),
            );
    }
}

/// The ring mesh, shared by every ripple. Only the material differs, because
/// only the material fades.
#[derive(Resource)]
struct RingMesh(Handle<Mesh>);

fn build_ring(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>) {
    // An annulus is a 2D primitive, so it is born standing up in the XY plane.
    // Laid flat here rather than rotated per ripple, which would be the same
    // quarter turn a few hundred times a minute.
    let ring = Annulus::new(0.82, 1.0)
        .mesh()
        .resolution(48)
        .build()
        .rotated_by(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
    commands.insert_resource(RingMesh(meshes.add(ring)));
}

// ------------------------------------------------------------- the maths ----

/// How much of a provocation reaches somebody `distance` away.
///
/// One at the source, nothing at the edge, and falling off faster than linearly
/// in between — which is what makes standing next to somebody meaningfully
/// different from standing near them, and is the difference between a
/// provocation and a weather system.
pub fn carry(distance: f32, radius: f32) -> f32 {
    if radius <= 0.0 {
        return 0.0;
    }
    let reach = (1.0 - distance / radius).clamp(0.0, 1.0);
    reach * reach
}

/// What a taunt does to somebody's mood: always negative, and worse the shorter
/// their fuse. This is where a Wutbürger differs from everybody else in the one
/// way that matters.
pub fn sting(distance: f32, temper: &Temperament, tune: &MoodConfig) -> f32 {
    -carry(distance, tune.taunt_radius) * tune.taunt_bite * temper.fuse
}

/// And what a whistle does. Scaled by `contagion` rather than by the fuse,
/// because being cheered up by a stranger is the same faculty as catching a
/// mood off one — and it gives the serene something they are unusually good at.
pub fn warmth(distance: f32, temper: &Temperament, tune: &MoodConfig) -> f32 {
    carry(distance, tune.cheer_radius) * tune.cheer_warmth * (0.4 + temper.contagion)
}

// ----------------------------------------------------------- the systems ----

fn player_provokes(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<GameConfig>,
    bank: Option<Res<SoundBank>>,
    ring: Option<Res<RingMesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut rng: ResMut<AudioRng>,
    mut provocations: MessageWriter<Provocation>,
    mut players: Query<(Entity, &Transform, &ActionState<Action>, &mut Provoker), With<Player>>,
) {
    let Ok((player, transform, actions, mut provoker)) = players.single_mut() else {
        return;
    };
    provoker.cooldown = (provoker.cooldown - time.delta_secs()).max(0.0);
    if provoker.cooldown > 0.0 {
        return;
    }

    let kind = if actions.just_pressed(&Action::Taunt) {
        Rudeness::Taunt
    } else if actions.just_pressed(&Action::Cheer) {
        Rudeness::Cheer
    } else {
        return;
    };

    provoker.cooldown = config.mood.provoke_rest;
    let at = transform.translation;
    provocations.write(Provocation {
        by: player,
        at,
        kind,
    });
    announce(
        &mut commands,
        &config,
        bank.as_deref(),
        ring.as_deref(),
        &mut materials,
        &mut rng,
        at,
        kind,
        true,
    );
}

/// A flummi that feels strongly enough to say so without being asked.
///
/// This is the whole of the crowd's social life. A Wutbürger below its own
/// patience taunts whoever is standing about, which lands hardest on whoever
/// else has a short fuse, which sets them off too; a delighted one whistles,
/// which lands hardest on the suggestible. Both go out through the same
/// message the player's do, so a chain reaction needs no code of its own.
fn npcs_provoke(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<GameConfig>,
    bank: Option<Res<SoundBank>>,
    ring: Option<Res<RingMesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut rng: ResMut<AudioRng>,
    mut provocations: MessageWriter<Provocation>,
    mut flummis: Query<(Entity, &Transform, &Mood, &mut Provoker), Without<Player>>,
) {
    let dt = time.delta_secs();
    for (entity, transform, mood, mut provoker) in &mut flummis {
        provoker.cooldown = (provoker.cooldown - dt).max(0.0);
        if provoker.cooldown > 0.0 {
            continue;
        }

        let kind = if mood.value <= SPONTANEOUS_SPITE {
            Rudeness::Taunt
        } else if mood.value >= SPONTANEOUS_JOY {
            Rudeness::Cheer
        } else {
            continue;
        };
        // Rolled per second rather than per frame, so how often the city is
        // rude does not depend on how fast it is running.
        if rng.random::<f32>() > SPONTANEITY * dt * mood.value.abs() * 60.0 {
            provoker.cooldown = config.mood.provoke_rest * 0.5;
            continue;
        }

        provoker.cooldown = config.mood.provoke_rest * rng.random_range(1.5..4.0);
        let at = transform.translation;
        provocations.write(Provocation {
            by: entity,
            at,
            kind,
        });
        announce(
            &mut commands,
            &config,
            bank.as_deref(),
            ring.as_deref(),
            &mut materials,
            &mut rng,
            at,
            kind,
            false,
        );
    }
}

/// The noise and the ring. Shared by both, because a raspberry from an NPC has
/// to look and sound exactly like one from the player or the crowd's behaviour
/// reads as scripted rather than as the same rule applying to everybody.
///
/// `own` marks the player's mouth. Their raspberry happens *to* them rather
/// than near them — the same rule the footsteps follow (see
/// [`crate::audio::close_once`]) — so it plays flat in both ears instead of
/// through the spatial mixer, which was quietly swallowing the player's own
/// provocations while everybody else's carried fine.
fn announce(
    commands: &mut Commands,
    config: &GameConfig,
    bank: Option<&SoundBank>,
    ring: Option<&RingMesh>,
    materials: &mut Assets<StandardMaterial>,
    rng: &mut AudioRng,
    at: Vec3,
    kind: Rudeness,
    own: bool,
) {
    if let Some(bank) = bank {
        let sound = match kind {
            // The rudeness rotation. Random rather than cycling, because a
            // predictable sequence reads as a jukebox and an unpredictable
            // one reads as a person deciding how rude to be today.
            Rudeness::Taunt => {
                let rude = [
                    &bank.raspberry,
                    &bank.fart,
                    &bank.cough,
                    &bank.spit,
                    &bank.burp,
                ];
                rude[rng.random_range(0..rude.len())].clone()
            }
            Rudeness::Cheer => bank.whistle[rng.random_range(0..VARIANTS)].clone(),
        };
        let settings = if own {
            close_once(effect_gain(config, GAIN))
        } else {
            spatial_once(effect_gain(config, GAIN), EARSHOT)
        };
        commands.spawn((
            AudioPlayer(sound),
            settings.with_speed(rng.random_range(0.9..1.15)),
            Transform::from_translation(at),
        ));
    }

    let Some(ring) = ring else { return };
    let (colour, reach) = match kind {
        Rudeness::Taunt => (
            LinearRgba::new(0.95, 0.22, 0.16, 1.0),
            config.mood.taunt_radius,
        ),
        Rudeness::Cheer => (
            LinearRgba::new(0.55, 0.92, 0.42, 1.0),
            config.mood.cheer_radius,
        ),
    };
    // One material per ripple, because the fade is per ripple. They are cheap,
    // short-lived, and the handle goes with the entity.
    let material = materials.add(StandardMaterial {
        base_color: colour.into(),
        emissive: colour * 2.0,
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        // Nothing should be able to walk behind a ring painted on the road and
        // have it disappear, and nothing should be shadowed by it either.
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        Name::new("Provocation"),
        Ripple {
            age: 0.0,
            reach,
            material: material.clone(),
            colour,
        },
        Mesh3d(ring.0.clone()),
        MeshMaterial3d(material),
        Transform::from_translation(at.with_y(at.y - 0.8 + RIPPLE_LIFT))
            .with_scale(Vec3::splat(0.1)),
        NotShadowCaster,
    ));
}

fn spread_ripples(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ripples: Query<(Entity, &mut Ripple, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (entity, mut ripple, mut transform) in &mut ripples {
        ripple.age += dt;
        let along = ripple.age / RIPPLE_LIFE;
        if along >= 1.0 {
            commands.entity(entity).despawn();
            continue;
        }
        // Fast at first and slowing, which reads as something spreading out
        // from a source rather than as a circle being drawn.
        let spread = 1.0 - (1.0 - along) * (1.0 - along);
        transform.scale = Vec3::splat((ripple.reach * spread).max(0.05));
        if let Some(mut material) = materials.get_mut(&ripple.material) {
            let fade = 1.0 - along;
            material.base_color = ripple.colour.with_alpha(fade * 0.55).into();
        }
    }
}

/// Everybody in range takes it however their disposition takes it.
fn feel_provocations(
    config: Res<GameConfig>,
    mut provocations: MessageReader<Provocation>,
    mut flummis: Query<(Entity, &Transform, &mut Mood, &Temperament)>,
) {
    for provocation in provocations.read() {
        for (entity, transform, mut mood, temper) in &mut flummis {
            // Nobody is offended by their own raspberry, and nobody cheers
            // themselves up by whistling. Both would be funny once.
            if entity == provocation.by {
                continue;
            }
            let apart = transform.translation.distance(provocation.at);
            let shift = match provocation.kind {
                Rudeness::Taunt => sting(apart, temper, &config.mood),
                Rudeness::Cheer => warmth(apart, temper, &config.mood),
            };
            if shift != 0.0 {
                mood.value = (mood.value + shift).clamp(-1.0, 1.0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tune() -> MoodConfig {
        GameConfig::default().mood
    }

    #[test]
    fn a_provocation_lands_hardest_on_whoever_is_closest() {
        let radius = tune().taunt_radius;
        assert_eq!(carry(0.0, radius), 1.0);
        assert_eq!(carry(radius, radius), 0.0);
        assert_eq!(carry(radius * 2.0, radius), 0.0, "it must actually stop");
        assert!(carry(radius * 0.25, radius) > carry(radius * 0.75, radius));
    }

    #[test]
    fn the_falloff_is_steeper_than_a_straight_line() {
        // Halfway out should be well under half as strong, or the difference
        // between being taunted and being near a taunt is nothing.
        let radius = tune().taunt_radius;
        assert!(carry(radius * 0.5, radius) < 0.4);
    }

    #[test]
    fn a_taunt_bites_a_wutburger_harder_than_a_peaceable_soul() {
        let calm = sting(1.0, &Temperament::serene(), &tune());
        let furious = sting(1.0, &Temperament::ragemonger(), &tune());
        assert!(calm < 0.0 && furious < calm, "{furious} against {calm}");
    }

    #[test]
    fn a_whistle_reaches_the_suggestible_rather_than_the_short_fused() {
        // The two directions are scaled by different traits on purpose. A cheer
        // that also landed hardest on the Wutbürger would make every
        // temperament the same slider with a different name on it.
        let serene = warmth(1.0, &Temperament::serene(), &tune());
        let touchy = warmth(1.0, &Temperament::touchy(), &tune());
        assert!(serene > 0.0 && touchy > serene);
    }

    #[test]
    fn a_cheer_carries_further_than_an_insult() {
        // Being able to make a mess more easily than to clear one up is the
        // right way round; not being able to clear it up at all is not.
        let tune = tune();
        assert!(tune.cheer_radius > tune.taunt_radius);
        let outside = tune.taunt_radius + 1.0;
        assert_eq!(sting(outside, &Temperament::ordinary(), &tune), 0.0);
        assert!(warmth(outside, &Temperament::ordinary(), &tune) > 0.0);
    }

    #[test]
    fn one_taunt_cannot_flatten_anybody_on_its_own() {
        // A single raspberry that took a citizen from content to furious would
        // leave the mood with nowhere to go, and the crowd's own quarrels —
        // which are the interesting half — would never be heard over it.
        let worst = sting(0.0, &Temperament::ragemonger(), &tune()).abs();
        assert!(worst < 1.0, "one taunt moved a mood by {worst:.2}");
    }
}
