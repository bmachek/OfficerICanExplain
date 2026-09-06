//! Hooking the sound bank up to the game.
//!
//! Two shapes of sound, handled differently on purpose:
//!
//! * **One-shots** are spawned per event and despawn themselves. A crash, a
//!   door, a footfall. Cheap, fire and forget.
//! * **Voices** are loops owned by an entity, living as a child of it so the
//!   spatial mixer follows the car around, and modulated every frame — engine
//!   pitch from road speed, tyre squeal from how far the tyres are actually
//!   sliding. Spawning and despawning these per event would click; they run
//!   continuously and change volume instead.
//!
//! Nothing in here writes to the simulation. If a system in this file were
//! deleted the game would play identically, in silence.

use bevy::audio::{AudioSinkPlayback, SpatialAudioSink, Volume};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use rand::RngExt;

use super::bank::SoundBank;
use super::synth::SynthSound;
use super::{AudioRng, close_once, effect_gain, spatial_once};
use crate::bounce::launch::KnockedDown;
use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;
use crate::mood::feeling::CityMood;
use crate::player::interact::{DrivenBy, Driving};
use crate::player::on_foot::Player;
use crate::vehicle::controller::{VehicleInput, VehicleState};
use crate::vehicle::impact::VehicleImpact;
use crate::vehicle::spawn::{AlwaysSimulated, Vehicle};
use crate::world::mayhem::{Geyser, PropSheared, pressure};

/// Metres of pavement per footfall. Distance rather than time, so sprinting
/// speeds up the cadence for free.
const STRIDE: f32 = 1.75;

/// Sideways sliding speed, in m/s, at which the tyres are screaming.
///
/// `VehicleState::slip` is uncancelled lateral speed, so this is readable as a
/// physical claim: three metres a second sideways is a full-lock slide.
const FULL_SQUEAL: f32 = 3.0;
/// Below this road speed a sliding car is being shoved, not drifting.
const SQUEAL_FLOOR_KPH: f32 = 9.0;

/// Impact severity, in m/s of velocity lost, that counts as a proper crash.
const CRASH_FLOOR: f32 = 1.5;
const CRASH_FULL: f32 = 16.0;

/// Per-sound gains, so the mix is one block of numbers rather than a constant
/// buried in each system.
mod gain {
    pub const CRASH: f32 = 0.9;
    pub const HONK: f32 = 0.7;
    pub const WHEEE: f32 = 0.6;
    pub const SPROING: f32 = 0.75;
    pub const SPRAY: f32 = 0.5;
    pub const FOOTSTEP: f32 = 0.30;
    pub const DOOR: f32 = 0.6;
    pub const ENGINE: f32 = 0.55;
    pub const SCREECH: f32 = 0.55;
}

/// A looping sound belonging to a vehicle.
#[derive(Component)]
struct Voice {
    owner: Entity,
    kind: VoiceKind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum VoiceKind {
    Engine,
    Screech,
}

/// One of the three ambient beds, spawned once and never despawned. The mixer
/// crossfades between them on the city's average mood: birds when the city is
/// pleased with itself, distant traffic when it is nothing in particular, and
/// a demonstration somewhere behind the buildings when it has had enough.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Ambience {
    Traffic,
    Birdsong,
    Uproar,
}

pub struct SfxPlugin;

impl Plugin for SfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                start_ambience,
                (
                    play_impacts,
                    play_honks,
                    play_wheees,
                    play_sproings,
                    voice_geysers,
                    play_doors,
                    play_footsteps,
                    manage_vehicle_voices,
                    update_vehicle_voices,
                    update_ambience,
                )
                    .in_set(GameSet::Simulation),
            )
                // The bank is synthesised in `Startup`; nothing here can run
                // before it lands.
                .run_if(resource_exists::<SoundBank>),
        );
    }
}

// ------------------------------------------------------------- one-shots ----

fn at(
    commands: &mut Commands,
    sound: Handle<SynthSound>,
    position: Vec3,
    settings: PlaybackSettings,
) {
    commands.spawn((
        AudioPlayer(sound),
        settings,
        Transform::from_translation(position),
    ));
}

fn play_impacts(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    mut impacts: MessageReader<VehicleImpact>,
) {
    for impact in impacts.read() {
        if impact.severity < CRASH_FLOOR {
            continue;
        }
        // Loudness follows how hard the car actually stopped, so a scrape and a
        // head-on are the same sound played with different force.
        let force = (impact.severity / CRASH_FULL).clamp(0.25, 1.0);
        at(
            &mut commands,
            bank.crash.clone(),
            impact.position,
            spatial_once(effect_gain(&config, gain::CRASH * force), 22.0)
                // Heavier hits ring lower.
                .with_speed(1.15 - force * 0.3),
        );
    }
}

/// Impact severity above which the offended car honks about it.
const HONK_FLOOR: f32 = 3.0;

/// The indignant honk after a crash.
///
/// Not every crash: a horn that answers every scrape is a metronome, and the
/// joke needs room to land. The pause between the bang and the honk is baked
/// into the sound itself — see `bank::honk`.
fn play_honks(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    mut rng: ResMut<AudioRng>,
    mut impacts: MessageReader<VehicleImpact>,
) {
    for impact in impacts.read() {
        if impact.severity < HONK_FLOOR || rng.random::<f32>() > 0.6 {
            continue;
        }
        at(
            &mut commands,
            bank.honk.clone(),
            impact.position,
            spatial_once(effect_gain(&config, gain::HONK), 26.0)
                // Every car has its own voice, near enough.
                .with_speed(0.85 + rng.random::<f32>() * 0.35),
        );
    }
}

/// The twang of street furniture leaving its footing.
fn play_sproings(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    mut rng: ResMut<AudioRng>,
    mut sheared: MessageReader<PropSheared>,
) {
    for shear in sheared.read() {
        at(
            &mut commands,
            bank.sproing.clone(),
            shear.position,
            spatial_once(effect_gain(&config, gain::SPROING), 24.0)
                // A parking meter and a phone box do not twang at the same
                // pitch, and the ear notices even if it cannot say why.
                .with_speed(0.85 + rng.random::<f32>() * 0.4),
        );
    }
}

/// Puts the spray loop on every geyser, and lets it die with the pressure.
///
/// The sink rides the geyser entity itself, so the spatial mixer follows the
/// stump and the loop stops the moment the geyser despawns — with the chunk
/// or with its own timer, either way for free.
fn voice_geysers(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    fresh: Query<Entity, (With<Geyser>, Without<SpatialAudioSink>)>,
    mut running: Query<(&Geyser, &mut SpatialAudioSink)>,
) {
    for geyser in &fresh {
        commands.entity(geyser).insert((
            AudioPlayer(bank.spray.clone()),
            // Muted for the same reason the vehicle voices start muted: the
            // first frame must not blare before the level below has run once.
            PlaybackSettings::LOOP.with_spatial(true).muted(),
        ));
    }
    for (geyser, mut sink) in &mut running {
        let level = effect_gain(&config, gain::SPRAY) * pressure(geyser.life.fraction());
        sink.set_volume(Volume::Linear(level));
        if level > 0.001 && sink.is_muted() {
            sink.unmute();
        }
    }
}

/// The slide whistle for anybody who has just been put in the air.
///
/// `Added<KnockedDown>` rather than the launch itself, so it covers every way
/// a body leaves the ground unwillingly — bumpers, grudges, whatever comes
/// next — without each of them having to remember the orchestra.
fn play_wheees(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    mut rng: ResMut<AudioRng>,
    launched: Query<&Transform, Added<KnockedDown>>,
) {
    for transform in &launched {
        at(
            &mut commands,
            bank.wheee.clone(),
            transform.translation,
            spatial_once(effect_gain(&config, gain::WHEEE), 24.0)
                .with_speed(0.9 + rng.random::<f32>() * 0.3),
        );
    }
}

fn play_doors(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    entered: Query<&Transform, Added<DrivenBy>>,
    mut vacated: RemovedComponents<DrivenBy>,
    transforms: Query<&Transform>,
) {
    let slam = |position: Vec3, commands: &mut Commands| {
        at(
            commands,
            bank.car_door.clone(),
            position,
            spatial_once(effect_gain(&config, gain::DOOR), 14.0),
        );
    };

    for transform in &entered {
        slam(transform.translation, &mut commands);
    }
    for vehicle in vacated.read() {
        // A car can lose its driver by being despawned out from under them —
        // streaming, mostly. A despawn is not a door.
        if let Ok(transform) = transforms.get(vehicle) {
            slam(transform.translation, &mut commands);
        }
    }
}

fn play_footsteps(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    mut rng: ResMut<AudioRng>,
    mut travelled: Local<f32>,
    players: Query<&avian3d::prelude::LinearVelocity, (With<Player>, Without<Driving>)>,
) {
    let Ok(velocity) = players.single() else {
        *travelled = 0.0;
        return;
    };

    let pace = velocity.0.xz().length();
    // Airborne, or barely moving: no feet on the ground to hear.
    if pace < 0.6 || velocity.0.y.abs() > 2.5 {
        return;
    }

    *travelled += pace * time.delta_secs();
    if *travelled < STRIDE {
        return;
    }
    *travelled -= STRIDE;

    commands.spawn((
        AudioPlayer(bank.footstep.clone()),
        close_once(effect_gain(&config, gain::FOOTSTEP))
            // Every footfall lands slightly differently. Without this the walk
            // cycle turns into a drum machine within three paces.
            .with_speed(0.90 + rng.random::<f32>() * 0.22),
    ));
}

// ---------------------------------------------------------------- voices ----

/// Engine pitch from road speed and throttle, as a playback speed multiplier.
///
/// A single loop pitched by road speed alone would climb to a scream on the
/// motorway and sit there. Real cars change gear, and faking the gearbox —
/// revs rising through a band, then dropping back as the next ratio takes over
/// — is most of what makes a car sound like it is being *driven* rather than
/// merely moving.
pub fn engine_pitch(speed_kph: f32, throttle: f32) -> f32 {
    /// Road speed each gear runs out at, in km/h.
    const GEARS: [f32; 5] = [26.0, 50.0, 82.0, 122.0, 180.0];

    let speed = speed_kph.abs();
    let gear = GEARS
        .iter()
        .position(|&top| speed < top)
        .unwrap_or(GEARS.len() - 1);
    let bottom = if gear == 0 { 0.0 } else { GEARS[gear - 1] };
    let through = ((speed - bottom) / (GEARS[gear] - bottom)).clamp(0.0, 1.0);

    // Idle, plus the revs earned within this gear, plus a lift for load: a car
    // held on the throttle sounds busier than one coasting at the same speed.
    0.62 + through * 0.92 + throttle.max(0.0) * 0.14
}

/// Adds a loop set to every vehicle that should be making noise, and takes them
/// away again when it stops.
///
/// A car qualifies if something is driving it: the player or traffic, both of
/// which are exempt from distance culling. Several hundred cars are parked
/// around the city and none of them has its engine running.
fn manage_vehicle_voices(
    mut commands: Commands,
    bank: Res<SoundBank>,
    driven: Query<Entity, (With<Vehicle>, Or<(With<DrivenBy>, With<AlwaysSimulated>)>)>,
    voices: Query<(Entity, &Voice)>,
) {
    let running: HashSet<Entity> = driven.iter().collect();

    let mut voiced: HashSet<Entity> = HashSet::default();
    for (entity, voice) in &voices {
        if running.contains(&voice.owner) {
            voiced.insert(voice.owner);
        } else {
            commands.entity(entity).despawn();
        }
    }

    for vehicle in &driven {
        if voiced.contains(&vehicle) {
            continue;
        }
        // Started muted rather than silent-by-volume so the first frame cannot
        // blare before the modulation systems have run.
        let looping = PlaybackSettings::LOOP.with_spatial(true).muted();
        // The identity transform is load-bearing: it brings `GlobalTransform`
        // with it, and a spatial source without one is mixed at the origin.
        let place = Transform::default();
        commands.entity(vehicle).with_children(|car| {
            car.spawn((
                Voice {
                    owner: vehicle,
                    kind: VoiceKind::Engine,
                },
                AudioPlayer(bank.engine.clone()),
                looping,
                place,
            ));
            car.spawn((
                Voice {
                    owner: vehicle,
                    kind: VoiceKind::Screech,
                },
                AudioPlayer(bank.screech.clone()),
                looping,
                place,
            ));
        });
    }
}

fn update_vehicle_voices(
    config: Res<GameConfig>,
    vehicles: Query<(&VehicleState, &VehicleInput)>,
    mut voices: Query<(&Voice, &mut SpatialAudioSink)>,
) {
    for (voice, mut sink) in &mut voices {
        let Ok((state, input)) = vehicles.get(voice.owner) else {
            continue;
        };
        let speed_kph = state.speed_kph();

        let level = match voice.kind {
            VoiceKind::Engine => {
                sink.set_speed(engine_pitch(speed_kph, input.throttle));
                // Idling is audible; working is loud.
                let load = input.throttle.abs().max((speed_kph / 70.0).min(1.0) * 0.6);
                effect_gain(&config, gain::ENGINE) * (0.35 + 0.65 * load)
            }
            VoiceKind::Screech => {
                let sliding = if speed_kph < SQUEAL_FLOOR_KPH {
                    0.0
                } else {
                    (state.slip / FULL_SQUEAL).clamp(0.0, 1.0)
                };
                // Tyres go up in pitch as they let go, not just up in volume.
                sink.set_speed(0.88 + sliding * 0.3);
                effect_gain(&config, gain::SCREECH) * sliding
            }
        };

        // A muted sink still remembers its volume, so unmuting lands on the
        // right level rather than on whatever it was before.
        sink.set_volume(Volume::Linear(level));
        if level > 0.001 {
            if sink.is_muted() {
                sink.unmute();
            }
        } else if !sink.is_muted() {
            sink.mute();
        }
    }
}

// -------------------------------------------------------------- ambience ----

fn start_ambience(
    mut commands: Commands,
    bank: Res<SoundBank>,
    existing: Query<(), With<Ambience>>,
) {
    if !existing.is_empty() {
        return;
    }
    for (name, bed, sound) in [
        ("City ambience", Ambience::Traffic, &bank.ambience),
        ("Birdsong", Ambience::Birdsong, &bank.birdsong),
        ("Uproar", Ambience::Uproar, &bank.uproar),
    ] {
        commands.spawn((
            Name::new(name),
            bed,
            AudioPlayer(sound.clone()),
            // Muted until the first mix pass, so no bed blares at full
            // synthesis level for a frame before the mood is read.
            PlaybackSettings::LOOP.muted(),
        ));
    }
}

/// How loud each ambient bed is at a given city mood, −1 to 1: (traffic,
/// birdsong, uproar).
///
/// Pure, so the crossfade can be argued about without ears. The dead band
/// around neutral is deliberate: an ordinary day is traffic and nothing else,
/// and the first birds arriving are *news* — they say the street has actually
/// warmed up, not that the average twitched past zero.
pub fn ambience_mix(mood: f32) -> (f32, f32, f32) {
    let mood = mood.clamp(-1.0, 1.0);
    let birds = ((mood - 0.1) / 0.7).clamp(0.0, 1.0);
    let uproar = ((-mood - 0.1) / 0.7).clamp(0.0, 1.0);
    // The rumble never quite leaves — the city is still a city under the
    // birds — but it makes room for whichever pole is playing.
    let traffic = 1.0 - 0.6 * birds.max(uproar);
    (traffic, birds, uproar)
}

fn update_ambience(
    config: Res<GameConfig>,
    city: Res<CityMood>,
    mut beds: Query<(&Ambience, &mut bevy::audio::AudioSink)>,
) {
    let (traffic, birds, uproar) = ambience_mix(city.average);
    let base = config.audio.master * config.audio.ambience;
    for (bed, mut sink) in &mut beds {
        let level = match bed {
            Ambience::Traffic => traffic,
            Ambience::Birdsong => birds,
            Ambience::Uproar => uproar,
        };
        sink.set_volume(Volume::Linear(base * level));
        // A muted sink remembers its volume, so unmuting lands on the level
        // just set rather than on last week's.
        if base * level > 0.001 {
            if sink.is_muted() {
                sink.unmute();
            }
        } else if !sink.is_muted() {
            sink.mute();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revs_climb_through_a_gear_and_drop_at_the_change() {
        let pitch = |kph| engine_pitch(kph, 1.0);
        assert!(
            pitch(20.0) > pitch(5.0),
            "revs should rise within first gear"
        );
        assert!(
            pitch(28.0) < pitch(24.0),
            "the shift into second should drop the revs"
        );
        assert!(
            pitch(52.0) < pitch(48.0),
            "and so should the shift into third"
        );
    }

    #[test]
    fn revs_stay_in_a_playable_band_at_any_speed() {
        // Outside roughly half to double speed, resampling a loop stops
        // sounding like an engine and starts sounding like a fault.
        for kph in 0..400 {
            for throttle in [-1.0, 0.0, 1.0] {
                let pitch = engine_pitch(kph as f32, throttle);
                assert!(
                    (0.5..=2.0).contains(&pitch),
                    "{kph}km/h at throttle {throttle} gives {pitch}"
                );
            }
        }
    }

    #[test]
    fn the_ambience_follows_the_city_from_birds_to_barricades() {
        let (traffic, birds, uproar) = ambience_mix(0.0);
        assert_eq!(
            (birds, uproar),
            (0.0, 0.0),
            "an ordinary day is traffic and nothing else"
        );
        assert_eq!(traffic, 1.0);

        let (_, birds, uproar) = ambience_mix(0.9);
        assert!(birds > 0.9, "a delighted city should be full of birds");
        assert_eq!(uproar, 0.0, "and demonstrating about nothing");

        let (_, birds, uproar) = ambience_mix(-0.9);
        assert!(uproar > 0.9, "a furious city should be on the barricades");
        assert_eq!(birds, 0.0, "with every bird long gone");

        for mood in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            assert!(
                ambience_mix(mood).0 > 0.3,
                "the city is still a city at mood {mood}"
            );
        }
    }

    #[test]
    fn a_stationary_car_still_idles() {
        assert!(engine_pitch(0.0, 0.0) > 0.5, "the engine is still running");
        // Reverse is still the engine turning forwards.
        assert_eq!(engine_pitch(-20.0, 0.0), engine_pitch(20.0, 0.0));
    }
}
