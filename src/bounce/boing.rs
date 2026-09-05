//! The sound a city made of rubber makes.
//!
//! An impact is read off a sudden change in velocity rather than off a
//! collision event, which is the same rule `vehicle::damage` uses and for the
//! same reason: it catches every way a body can be stopped hard — a wall,
//! another flummi, a bumper, landing badly off a roof — through one code path
//! instead of one rule per collision pair.
//!
//! The threshold matters more than usual here. Everything bounces constantly,
//! so a threshold set too low turns the street into a bag of springs; set too
//! high and being launched across a junction is silent. It sits just above the
//! velocity a body loses to its own hop.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::controller::Bouncer;
use crate::audio::bank::SoundBank;
use crate::audio::{effect_gain, spatial_once};
use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;

/// Velocity change in one tick, in m/s, below which a knock is not worth a
/// sound. Above the give-and-take of an ordinary hop, below a real collision.
const WALLOP_FLOOR: f32 = 3.2;
/// The change that counts as being hit as hard as anything ever is. Louder
/// impacts than this exist; they do not sound any louder.
const WALLOP_FULL: f32 = 18.0;
/// How far a boing carries.
const EARSHOT: f32 = 20.0;
const GAIN: f32 = 0.8;

/// Somebody got hit. Written here and read by anything that cares how the city
/// is feeling about it.
#[derive(Message, Debug, Clone, Copy)]
pub struct Wallop {
    pub entity: Entity,
    pub position: Vec3,
    /// Velocity lost in the impact, in m/s.
    pub severity: f32,
}

/// Last tick's velocity, so a change in it can be spotted.
#[derive(Component, Default)]
pub struct PreviousVelocity(pub Vec3);

pub struct BoingPlugin;

impl Plugin for BoingPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Wallop>().add_systems(
            Update,
            (spot_wallops, play_boings)
                .chain()
                .in_set(GameSet::Simulation),
        );
    }
}

/// How hard a knock reads, 0 to 1. Pure, so the mix can be argued about
/// without a physics world.
pub fn wallop_strength(delta: f32) -> f32 {
    if delta < WALLOP_FLOOR {
        return 0.0;
    }
    ((delta - WALLOP_FLOOR) / (WALLOP_FULL - WALLOP_FLOOR)).clamp(0.05, 1.0)
}

fn spot_wallops(
    mut commands: Commands,
    mut wallops: MessageWriter<Wallop>,
    mut bodies: Query<
        (
            Entity,
            &Transform,
            &LinearVelocity,
            Option<&mut PreviousVelocity>,
        ),
        With<Bouncer>,
    >,
) {
    for (entity, transform, velocity, previous) in &mut bodies {
        let Some(mut previous) = previous else {
            // First sight of this body. Seeding from its current velocity
            // rather than from zero stops a flummi that spawned in mid-air
            // yelping on the frame it appears.
            commands.entity(entity).insert(PreviousVelocity(velocity.0));
            continue;
        };
        let delta = (velocity.0 - previous.0).length();
        previous.0 = velocity.0;

        let severity = wallop_strength(delta);
        if severity > 0.0 {
            wallops.write(Wallop {
                entity,
                position: transform.translation,
                severity: delta,
            });
        }
    }
}

fn play_boings(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    mut wallops: MessageReader<Wallop>,
) {
    for wallop in wallops.read() {
        let force = wallop_strength(wallop.severity);
        commands.spawn((
            AudioPlayer(bank.boing.clone()),
            spatial_once(effect_gain(&config, GAIN * force), EARSHOT)
                // Harder knocks ring lower and longer, the way a bigger ball
                // does. The range is wide because this is the sound the whole
                // game is built out of and it must not become a single note.
                .with_speed(1.35 - force * 0.55),
            Transform::from_translation(wallop.position),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ordinary_hop_makes_no_noise() {
        // A flummi gives back a little under 3 m/s at the bottom of every
        // bounce. If that registered, the street would be a bag of springs.
        assert_eq!(wallop_strength(2.9), 0.0);
    }

    #[test]
    fn being_run_over_registers_at_full_force() {
        assert_eq!(wallop_strength(40.0), 1.0);
    }

    #[test]
    fn a_knock_just_over_the_floor_is_audible_rather_than_silent() {
        // Clamped away from zero on purpose: the first thing over the line
        // should be a quiet boing, not a muted one.
        assert!(wallop_strength(WALLOP_FLOOR + 0.01) > 0.0);
    }

    #[test]
    fn harder_knocks_read_as_harder() {
        assert!(wallop_strength(6.0) < wallop_strength(12.0));
    }
}
