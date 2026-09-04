//! Hooking the sound bank up to the game.
//!
//! Two shapes of sound, handled differently on purpose:
//!
//! * **One-shots** are spawned per event and despawn themselves. A gunshot, a
//!   crash, a door. Cheap, fire and forget.
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
use super::{AudioRng, BLAST_EARSHOT, close_once, effect_gain, spatial_once};
use crate::ai::police::PoliceUnit;
use crate::combat::health::Died;
use crate::combat::weapons::{WeaponFired, WeaponKind};
use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;
use crate::crime::wanted::Wanted;
use crate::mission::Campaign;
use crate::player::interact::{DrivenBy, Driving};
use crate::player::on_foot::Player;
use crate::vehicle::controller::{VehicleInput, VehicleState};
use crate::vehicle::damage::{VehicleDestroyed, VehicleImpact};
use crate::vehicle::spawn::{AlwaysSimulated, Vehicle};

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
    pub const PISTOL: f32 = 0.85;
    pub const SMG: f32 = 0.55;
    pub const EXPLOSION: f32 = 1.0;
    pub const CRASH: f32 = 0.9;
    pub const THUD: f32 = 0.7;
    pub const FOOTSTEP: f32 = 0.30;
    pub const DOOR: f32 = 0.6;
    pub const ENGINE: f32 = 0.55;
    pub const SIREN: f32 = 0.45;
    pub const SCREECH: f32 = 0.55;
    pub const STING: f32 = 0.7;
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
    Siren,
    Screech,
}

/// The ambient bed. One entity, spawned once, never despawned.
#[derive(Component)]
struct Ambience;

pub struct SfxPlugin;

impl Plugin for SfxPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                start_ambience,
                (
                    play_weapon_fire,
                    play_impacts,
                    play_explosions,
                    play_deaths,
                    play_doors,
                    play_footsteps,
                    play_stingers,
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

fn play_weapon_fire(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    mut shots: MessageReader<WeaponFired>,
) {
    for shot in shots.read() {
        let (sound, gain) = match shot.kind {
            WeaponKind::Pistol => (bank.pistol.clone(), gain::PISTOL),
            WeaponKind::Smg => (bank.smg.clone(), gain::SMG),
        };
        // The player's own weapon is at their shoulder, not somewhere in the
        // world, so it is not spatialised.
        commands.spawn((AudioPlayer(sound), close_once(effect_gain(&config, gain))));
    }
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

fn play_explosions(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    mut destroyed: MessageReader<VehicleDestroyed>,
) {
    for wreck in destroyed.read() {
        at(
            &mut commands,
            bank.explosion.clone(),
            wreck.position,
            spatial_once(effect_gain(&config, gain::EXPLOSION), BLAST_EARSHOT),
        );
    }
}

fn play_deaths(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    mut deaths: MessageReader<Died>,
) {
    for death in deaths.read() {
        at(
            &mut commands,
            bank.thud.clone(),
            death.position,
            spatial_once(effect_gain(&config, gain::THUD), 16.0),
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
        // A wrecked car loses its driver by being despawned. That already has
        // an explosion; it does not also need a door.
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

/// Short musical cues for the two things the player needs told immediately:
/// the heat going up, and a job going in the bank.
fn play_stingers(
    mut commands: Commands,
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    wanted: Res<Wanted>,
    campaign: Res<Campaign>,
    mut last_stars: Local<u8>,
    mut last_jobs: Local<usize>,
) {
    let volume = close_once(effect_gain(&config, gain::STING));

    let stars = wanted.stars();
    if stars > *last_stars {
        commands.spawn((AudioPlayer(bank.star.clone()), volume));
    }
    *last_stars = stars;

    let jobs = campaign.completed.len();
    if jobs > *last_jobs {
        commands.spawn((AudioPlayer(bank.chime.clone()), volume));
    }
    *last_jobs = jobs;
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
/// A car qualifies if something is driving it: the player, traffic or the
/// police, all of which are exempt from distance culling. Several hundred cars
/// are parked around the city and none of them has its engine running.
fn manage_vehicle_voices(
    mut commands: Commands,
    bank: Res<SoundBank>,
    driven: Query<
        (Entity, Has<PoliceUnit>),
        (With<Vehicle>, Or<(With<DrivenBy>, With<AlwaysSimulated>)>),
    >,
    voices: Query<(Entity, &Voice)>,
) {
    let running: HashSet<Entity> = driven.iter().map(|(entity, _)| entity).collect();

    let mut voiced: HashSet<Entity> = HashSet::default();
    for (entity, voice) in &voices {
        if running.contains(&voice.owner) {
            voiced.insert(voice.owner);
        } else {
            commands.entity(entity).despawn();
        }
    }

    for (vehicle, police) in &driven {
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
            if police {
                car.spawn((
                    Voice {
                        owner: vehicle,
                        kind: VoiceKind::Siren,
                    },
                    AudioPlayer(bank.siren.clone()),
                    looping,
                    place,
                ));
            }
        });
    }
}

fn update_vehicle_voices(
    config: Res<GameConfig>,
    wanted: Res<Wanted>,
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
            VoiceKind::Siren => {
                if wanted.is_wanted() {
                    effect_gain(&config, gain::SIREN)
                } else {
                    0.0
                }
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
    config: Res<GameConfig>,
    bank: Res<SoundBank>,
    existing: Query<(), With<Ambience>>,
) {
    if !existing.is_empty() {
        return;
    }
    commands.spawn((
        Name::new("City ambience"),
        Ambience,
        AudioPlayer(bank.ambience.clone()),
        PlaybackSettings::LOOP
            .with_volume(Volume::Linear(config.audio.master * config.audio.ambience)),
    ));
}

fn update_ambience(
    config: Res<GameConfig>,
    mut ambience: Query<&mut bevy::audio::AudioSink, With<Ambience>>,
) {
    if !config.is_changed() {
        return;
    }
    for mut sink in &mut ambience {
        sink.set_volume(Volume::Linear(config.audio.master * config.audio.ambience));
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
    fn a_stationary_car_still_idles() {
        assert!(engine_pitch(0.0, 0.0) > 0.5, "the engine is still running");
        // Reverse is still the engine turning forwards.
        assert_eq!(engine_pitch(-20.0, 0.0), engine_pitch(20.0, 0.0));
    }
}
