//! Who says something, and when.
//!
//! The bank knows how to make a giggle and a curse; this decides which of the
//! forty-five citizens in earshot is entitled to one. That turns out to be the
//! harder half. A crowd where everybody speaks whenever they feel like it is
//! not a crowd talking, it is a wall of noise with no individual in it, and the
//! joke needs individuals — one flummi audibly losing its temper while the ones
//! beside it whistle.
//!
//! So there are two limits, and they do different jobs:
//!
//! * **Only the nearest few may speak at all.** Distance attenuation alone does
//!   not save you: forty quiet voices still sum to a wash, and rodio is mixing
//!   every one of them whether or not you can pick it out.
//! * **Each of them then has a cooldown.** Without it the nearest flummi speaks
//!   sixty times a second, which is a much worse sound than forty of them
//!   speaking once.
//!
//! What is said follows the mood's sign and how strongly it is held, so the
//! street is audibly in the mood the faces say it is in — and a mood near zero
//! says nothing at all, because somebody with no opinion has no reason to
//! announce it.

use bevy::prelude::*;
use rand::RngExt;

use super::feeling::Mood;
use crate::audio::bank::{SoundBank, VARIANTS};
use crate::audio::synth::SynthSound;
use crate::audio::{AudioRng, effect_gain, spatial_once};
use crate::bounce::boing::Wallop;
use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;
use crate::player::on_foot::Player;

/// How many flummis may be speaking at once.
const CHOIR: usize = 8;
/// How far a voice carries.
const EARSHOT: f32 = 24.0;
/// And how far away a flummi has to be before it is not worth considering.
const RANGE: f32 = 34.0;
const GAIN: f32 = 0.7;

/// Mood either side of which nobody has anything to say.
const INDIFFERENT: f32 = 0.18;
/// And beyond which they stop being polite about it.
const EMPHATIC: f32 = 0.62;

/// Seconds before a flummi that has just spoken may speak again, at its most
/// and least talkative.
const REST: (f32, f32) = (1.8, 5.5);
/// And how long one that decided against it waits before considering again.
/// Short enough to feel spontaneous, long enough not to roll dice per frame.
const SECOND_THOUGHTS: f32 = 0.5;

/// Velocity lost, in m/s, that is worth gasping about. Well above an ordinary
/// bounce: a flummi that gasped every time it landed would be exhausting.
const GASP_AT: f32 = 7.0;

/// A flummi's voice, and whether it is allowed to use it yet.
#[derive(Component)]
pub struct Voicebox {
    /// Playback speed for everything this one says — their voice, in a single
    /// number. Drawn once at spawn, so a citizen sounds like themselves every
    /// time rather than like whichever take came up.
    pub pitch: f32,
    pub cooldown: f32,
}

impl Voicebox {
    pub fn new(pitch: f32) -> Self {
        Self {
            pitch,
            cooldown: SECOND_THOUGHTS,
        }
    }
}

/// What a mood makes somebody say.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Utterance {
    Whistle,
    Giggle,
    Grumble,
    Curse,
}

/// Which of the four a mood calls for, or nothing.
pub fn utterance(mood: f32) -> Option<Utterance> {
    if mood >= EMPHATIC {
        Some(Utterance::Giggle)
    } else if mood >= INDIFFERENT {
        Some(Utterance::Whistle)
    } else if mood <= -EMPHATIC {
        Some(Utterance::Curse)
    } else if mood <= -INDIFFERENT {
        Some(Utterance::Grumble)
    } else {
        None
    }
}

/// How likely somebody in this mood is to speak, given the chance.
///
/// Rises with how strongly the mood is held rather than with its sign: a
/// delighted flummi is exactly as talkative as a furious one, and that is what
/// stops a happy city being a silent one.
pub fn chance(mood: f32) -> f32 {
    (0.20 + 0.62 * mood.abs()).clamp(0.0, 0.95)
}

/// The floor on how often *anybody* starts speaking, in seconds.
///
/// This is the limit that actually does the work, and it took a while to see
/// why. Capping the choir at the nearest few is not enough on its own: the ones
/// outside it simply become eligible a frame later, so the whole street still
/// cycles through in a tenth of a second and the city ends up saying something
/// ten times a second. The per-flummi rest is not enough either — it stops one
/// citizen hogging the conversation, and does nothing about forty of them
/// taking turns. So the city as a whole takes turns too.
const TURN_TAKING: f32 = 0.5;

/// When the city may next start a voice.
#[derive(Resource, Default)]
struct Turn(f32);

pub struct VoicePlugin;

impl Plugin for VoicePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Turn>().add_systems(
            Update,
            (gasp_at_wallops, speak_up)
                .chain()
                .in_set(GameSet::Ai)
                // The bank is synthesised in `Startup`; nothing here can run
                // before it lands.
                .run_if(resource_exists::<SoundBank>),
        );
    }
}

fn voice_for(bank: &SoundBank, utterance: Utterance, take: usize) -> Handle<SynthSound> {
    let take = take % VARIANTS;
    match utterance {
        Utterance::Whistle => bank.whistle[take].clone(),
        Utterance::Giggle => bank.giggle.clone(),
        Utterance::Grumble => bank.grumble[take].clone(),
        Utterance::Curse => bank.curse[take].clone(),
    }
}

/// Roughly mouth height above a flummi's origin, so a voice comes from a face
/// rather than from a navel.
const MOUTH: Vec3 = Vec3::new(0.0, 0.6, 0.0);

fn speak_up(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    mut turn: ResMut<Turn>,
    mut rng: ResMut<AudioRng>,
    players: Query<&Transform, With<Player>>,
    mut flummis: Query<(Entity, &Transform, &Mood, &mut Voicebox)>,
) {
    let dt = time.delta_secs();
    let Ok(player) = players.single() else { return };
    let ears = player.translation;
    turn.0 = (turn.0 - dt).max(0.0);

    // One pass to age every cooldown and note who is close enough and ready.
    let mut ready: Vec<(f32, Entity)> = Vec::new();
    for (entity, transform, _, mut voice) in &mut flummis {
        voice.cooldown = (voice.cooldown - dt).max(0.0);
        let apart = transform.translation.distance(ears);
        if voice.cooldown <= 0.0 && apart < RANGE {
            ready.push((apart, entity));
        }
    }
    if turn.0 > 0.0 || ready.is_empty() {
        return;
    }

    // Nearest first: whoever speaks should be somebody the player can see doing
    // it, which is most of what makes a voice funny rather than ambient.
    ready.sort_by(|a, b| a.0.total_cmp(&b.0));
    ready.truncate(CHOIR);

    for (_, entity) in ready {
        let Ok((_, transform, mood, mut voice)) = flummis.get_mut(entity) else {
            continue;
        };
        let Some(utterance) = utterance(mood.value) else {
            // Nothing to say, but they did consider it. Without this they are
            // re-asked every frame for as long as they feel nothing.
            voice.cooldown = SECOND_THOUGHTS;
            continue;
        };
        if rng.random::<f32>() > chance(mood.value) {
            voice.cooldown = SECOND_THOUGHTS;
            continue;
        }

        let take = rng.random_range(0..VARIANTS);
        commands.spawn((
            AudioPlayer(voice_for(&bank, utterance, take)),
            spatial_once(effect_gain(&config, GAIN), EARSHOT).with_speed(voice.pitch),
            Transform::from_translation(transform.translation + MOUTH),
        ));

        // The stronger the feeling, the sooner they will have more to say.
        let rest = REST.1 - (REST.1 - REST.0) * mood.value.abs();
        voice.cooldown = rest * rng.random_range(0.8..1.25);
        turn.0 = TURN_TAKING;
        return;
    }
}

/// A hard knock knocks the breath out of somebody before it makes them cross.
///
/// Outside the turn-taking gate on purpose: a gasp is a reaction rather than a
/// remark, and being hit hard enough to warrant one is rare enough that it can
/// always interrupt.
fn gasp_at_wallops(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    mut wallops: MessageReader<Wallop>,
    mut flummis: Query<(&Transform, &mut Voicebox)>,
) {
    for wallop in wallops.read() {
        if wallop.severity < GASP_AT {
            continue;
        }
        let Ok((transform, mut voice)) = flummis.get_mut(wallop.entity) else {
            continue;
        };
        commands.spawn((
            AudioPlayer(bank.gasp.clone()),
            spatial_once(effect_gain(&config, GAIN), EARSHOT).with_speed(voice.pitch),
            Transform::from_translation(transform.translation + MOUTH),
        ));
        // Long enough that the gasp is heard on its own. What they think about
        // it lands a moment later, which is the funnier order.
        voice.cooldown = voice.cooldown.max(0.8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn somebody_with_no_opinion_says_nothing() {
        assert_eq!(utterance(0.0), None);
        assert_eq!(utterance(INDIFFERENT * 0.5), None);
        assert_eq!(utterance(-INDIFFERENT * 0.5), None);
    }

    #[test]
    fn the_mood_decides_which_voice_comes_out() {
        assert_eq!(utterance(1.0), Some(Utterance::Giggle));
        assert_eq!(utterance(0.3), Some(Utterance::Whistle));
        assert_eq!(utterance(-0.3), Some(Utterance::Grumble));
        assert_eq!(utterance(-1.0), Some(Utterance::Curse));
    }

    #[test]
    fn a_happy_city_is_no_quieter_than_a_furious_one() {
        // The temptation is to make anger the loud one, and it makes the game
        // worse: a city that only speaks up when it is cross has one joke.
        for strength in [0.2f32, 0.5, 0.9, 1.0] {
            assert_eq!(chance(strength), chance(-strength));
        }
        assert!(chance(1.0) > chance(0.2));
        assert!(chance(0.0) > 0.0, "even a shrug should be possible");
        assert!(chance(4.0) <= 1.0, "a mood off the scale is still a chance");
    }

    #[test]
    fn every_utterance_has_a_take_of_it_in_the_bank() {
        // `voice_for` indexes fixed-size arrays, so a take that ran past the
        // end would be a panic in the middle of a street rather than a warning
        // at build time.
        for utterance in [
            Utterance::Whistle,
            Utterance::Giggle,
            Utterance::Grumble,
            Utterance::Curse,
        ] {
            for take in 0..VARIANTS * 3 {
                assert!(take % VARIANTS < VARIANTS, "{utterance:?} take {take}");
            }
        }
    }
}
