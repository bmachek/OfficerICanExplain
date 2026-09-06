//! Audio.
//!
//! Every sound in the game is synthesised into a buffer at startup — `synth`
//! is how, `bank` is what — and this module is the wiring: it registers the
//! synthetic asset with Bevy's audio backend, puts the listener on the camera,
//! and decides how big the world sounds.

pub mod audition;
pub mod bank;
pub mod files;
pub mod sfx;
pub mod synth;
pub mod voice;

use bevy::audio::{AddAudioSource, DefaultSpatialScale, GlobalVolume, SpatialScale, Volume};
use bevy::prelude::*;
use rand_chacha::ChaCha8Rng;

use crate::core::config::GameConfig;
use crate::core::rng::{stream, stream_for};
use crate::player::camera::CameraRig;

/// Distance, in metres, out to which a spatial sound plays at full strength.
///
/// Rodio attenuates by the inverse square of the *scaled* distance and clamps
/// at unity, so this one number is really "how big does the world sound". Too
/// small and a siren one street over is inaudible; too large and every car in
/// the district is sitting in your lap.
const EARSHOT: f32 = 9.0;

/// Explosions are the one thing that should carry across a district.
pub const BLAST_EARSHOT: f32 = 55.0;

/// Random source for playback jitter: the small pitch differences that stop a
/// repeated sound turning into a metronome.
#[derive(Resource, Deref, DerefMut)]
pub struct AudioRng(pub ChaCha8Rng);

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_audio_source::<synth::SynthSound>()
            .insert_resource(DefaultSpatialScale(SpatialScale::new(1.0 / EARSHOT)))
            .add_systems(Startup, build_bank)
            .add_systems(Update, attach_listener);

        // A screenshot is taken by a process nobody is listening to, and the
        // capture run is scripted rather than played.
        if crate::core::capture::is_capture_mode() {
            app.insert_resource(GlobalVolume::new(Volume::SILENT));
            return;
        }

        app.add_plugins(sfx::SfxPlugin);
    }
}

fn build_bank(mut commands: Commands, mut sounds: ResMut<Assets<synth::SynthSound>>) {
    let started = std::time::Instant::now();
    let bank = bank::build(&mut sounds);
    commands.insert_resource(bank);
    commands.insert_resource(AudioRng(stream_for(0, stream::AUDIO)));
    info!(
        "sound bank synthesised in {:.1}ms",
        started.elapsed().as_secs_f32() * 1000.0
    );
}

/// The camera is where the player's ears are, so it carries the listener.
fn attach_listener(
    mut commands: Commands,
    cameras: Query<Entity, (With<CameraRig>, Without<bevy::audio::SpatialListener>)>,
) {
    for camera in &cameras {
        // Roughly a head across. Rodio pans on which ear is nearer, so the gap
        // only has to be non-zero and honest about the scale of the world.
        commands
            .entity(camera)
            .insert(bevy::audio::SpatialListener::new(0.25));
    }
}

/// Playback for a one-shot heard at a place in the world.
pub fn spatial_once(volume: f32, earshot: f32) -> PlaybackSettings {
    PlaybackSettings::DESPAWN
        .with_volume(Volume::Linear(volume))
        .with_spatial(true)
        .with_spatial_scale(SpatialScale::new(1.0 / earshot))
}

/// Playback for a one-shot that happens to the player rather than near them —
/// their own weapon, their own feet. Positioning those would only make the
/// player's own actions quieter when they turn their head.
pub fn close_once(volume: f32) -> PlaybackSettings {
    PlaybackSettings::DESPAWN.with_volume(Volume::Linear(volume))
}

/// Effect volume after the mixer settings, for a sound with the given gain.
pub fn effect_gain(config: &GameConfig, gain: f32) -> f32 {
    config.audio.master * config.audio.effects * gain
}
