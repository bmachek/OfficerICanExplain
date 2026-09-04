//! Every sound the game can make, computed at startup.
//!
//! These are written the way a synthesist would describe them rather than the
//! way a sampler would: a gunshot is a crack plus a muzzle blast plus the
//! street answering back, and each of those is a named term in a sum. That is
//! deliberate — the parameters that matter are then editable, and a shot that
//! sounds wrong is a number to change rather than a recording to re-take.
//!
//! Everything is normalised to a stated peak at the end, so relative loudness
//! across the bank is decided in one place instead of falling out of whatever
//! amplitude the synthesis happened to produce.

use std::f32::consts::TAU;

use bevy::prelude::*;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

use super::synth::{
    LowPass, Partial, Resonator, SAMPLE_RATE, SynthSound, at, fade_edges, harmonics, hit,
    normalize, samples, white, wrap_seam,
};
use crate::core::rng::{stream, stream_for};

/// Every sound, loaded once and shared by everything that plays it.
#[derive(Resource)]
pub struct SoundBank {
    pub pistol: Handle<SynthSound>,
    pub smg: Handle<SynthSound>,
    pub explosion: Handle<SynthSound>,
    pub crash: Handle<SynthSound>,
    pub thud: Handle<SynthSound>,
    pub footstep: Handle<SynthSound>,
    pub car_door: Handle<SynthSound>,
    pub engine: Handle<SynthSound>,
    pub siren: Handle<SynthSound>,
    pub screech: Handle<SynthSound>,
    pub ambience: Handle<SynthSound>,
    pub star: Handle<SynthSound>,
    pub chime: Handle<SynthSound>,
}

/// Firing frequency, in hertz, that the engine loop plays at unit speed.
/// Everything driving it scales from here.
pub const ENGINE_REFERENCE_HZ: f32 = 40.0;

pub fn build(sounds: &mut Assets<SynthSound>) -> SoundBank {
    SoundBank {
        pistol: sounds.add(shot(Shot {
            seed: 1,
            length: 0.42,
            crack: 0.028,
            brightness: 1_500.0,
            thump: 240.0,
            tail: 0.16,
        })),
        smg: sounds.add(shot(Shot {
            seed: 2,
            length: 0.26,
            crack: 0.016,
            brightness: 2_300.0,
            thump: 300.0,
            tail: 0.09,
        })),
        explosion: sounds.add(explosion()),
        crash: sounds.add(crash()),
        thud: sounds.add(thud()),
        footstep: sounds.add(footstep()),
        car_door: sounds.add(car_door()),
        engine: sounds.add(engine_loop()),
        siren: sounds.add(siren_loop()),
        screech: sounds.add(screech_loop()),
        ambience: sounds.add(ambience_loop()),
        star: sounds.add(star_sting()),
        chime: sounds.add(mission_chime()),
    }
}

fn audio_stream(seed: u64) -> ChaCha8Rng {
    stream_for(seed, stream::AUDIO)
}

// ------------------------------------------------------------- one-shots ----

struct Shot {
    seed: u64,
    length: f32,
    /// Half-life of the initial crack.
    crack: f32,
    /// Where the crack sits in the spectrum, in hertz.
    brightness: f32,
    /// Starting frequency of the muzzle blast.
    thump: f32,
    /// Half-life of the reflected tail.
    tail: f32,
}

/// A gunshot is three events, not one.
///
/// The crack is broadband noise gone in a few milliseconds. The thump is a fast
/// downward sweep — the muzzle blast, and the whole reason a shot has weight.
/// The tail is the same noise pulled through a low-pass and held; without it a
/// shot sounds like it was fired in a padded room rather than in a street.
fn shot(voice: Shot) -> SynthSound {
    let mut rng = audio_stream(voice.seed);
    let mut band = Resonator::new(voice.brightness, voice.brightness * 0.85);
    let mut reflections = LowPass::new(1_100.0);
    let mut phase = 0.0f32;

    let mut out = Vec::with_capacity(samples(voice.length));
    for index in 0..samples(voice.length) {
        let t = at(index);
        let noise = white(&mut rng);

        let crack = band.process(noise) * hit(t, 0.0004, voice.crack);
        let tail = reflections.process(noise) * hit(t, 0.006, voice.tail) * 0.55;

        // Blast pitch falls by two thirds over the first 60ms.
        let blast_hz = voice.thump * (1.0 - 0.68 * (t / 0.06).min(1.0));
        phase += TAU * blast_hz / SAMPLE_RATE as f32;
        let thump = phase.sin() * hit(t, 0.0008, 0.042);

        out.push(crack * 0.85 + tail + thump * 0.95);
    }

    fade_edges(&mut out, 0.002);
    normalize(&mut out, 0.92);
    SynthSound::new(out)
}

/// A car going up: blast, rumble, sub, and burning debris.
///
/// The low-pass cutoff falls as the sound plays, which is the part that reads
/// as distance and size — a fireball loses its high end long before it loses
/// its energy.
fn explosion() -> SynthSound {
    let mut rng = audio_stream(3);
    let mut blast = LowPass::new(1_500.0);
    let mut rumble = LowPass::new(190.0);
    let mut debris = LowPass::new(3_400.0);
    let mut phase = 0.0f32;

    let mut out = Vec::with_capacity(samples(1.9));
    for index in 0..samples(1.9) {
        let t = at(index);
        let noise = white(&mut rng);

        blast.set_cutoff(70.0 + 1_600.0 * 0.5f32.powf(t / 0.30));
        let body = blast.process(noise) * hit(t, 0.004, 0.30);
        let low = rumble.process(noise) * hit(t, 0.02, 0.55) * 1.5;

        // Sub-bass drop: the part you feel rather than hear.
        let sub_hz = 26.0 + 84.0 * 0.5f32.powf(t / 0.45);
        phase += TAU * sub_hz / SAMPLE_RATE as f32;
        let sub = phase.sin() * hit(t, 0.006, 0.30) * 1.3;

        // Sparse crackle: burning wreckage, gated so it reads as separate
        // events rather than a wash of hiss.
        let gate = if rng.random::<f32>() < 0.02 { 1.0 } else { 0.0 };
        let crackle = debris.process(noise * gate) * hit(t - 0.08, 0.05, 0.6) * 0.7;

        out.push(body + low + sub + crackle);
    }

    fade_edges(&mut out, 0.004);
    normalize(&mut out, 1.0);
    SynthSound::new(out)
}

/// Sheet metal hitting something.
///
/// Four resonators at frequencies chosen *not* to be harmonically related.
/// Harmonic partials read as a musical note; inharmonic ones read as a panel.
fn crash() -> SynthSound {
    let mut rng = audio_stream(4);
    let mut panels = [
        (Resonator::new(287.0, 24.0), 1.00),
        (Resonator::new(631.0, 38.0), 0.70),
        (Resonator::new(1_103.0, 60.0), 0.45),
        (Resonator::new(1_877.0, 110.0), 0.28),
    ];
    let mut body = LowPass::new(170.0);

    let mut out = Vec::with_capacity(samples(0.75));
    for index in 0..samples(0.75) {
        let t = at(index);
        let noise = white(&mut rng);
        let strike = noise * hit(t, 0.0006, 0.010);

        let mut metal = 0.0;
        for (resonator, weight) in &mut panels {
            metal += resonator.process(strike) * *weight;
        }
        let low = body.process(noise) * hit(t, 0.003, 0.085) * 1.8;

        out.push(metal * hit(t, 0.0, 0.24) + low + strike * 0.6);
    }

    fade_edges(&mut out, 0.003);
    normalize(&mut out, 0.9);
    SynthSound::new(out)
}

/// Something soft hitting the pavement.
fn thud() -> SynthSound {
    let mut rng = audio_stream(5);
    let mut body = LowPass::new(240.0);
    let mut scuff = LowPass::new(1_800.0);

    let mut out = Vec::with_capacity(samples(0.3));
    for index in 0..samples(0.3) {
        let t = at(index);
        let noise = white(&mut rng);
        out.push(
            body.process(noise) * hit(t, 0.004, 0.055) * 2.0
                + scuff.process(noise) * hit(t, 0.001, 0.018) * 0.35,
        );
    }

    fade_edges(&mut out, 0.003);
    normalize(&mut out, 0.7);
    SynthSound::new(out)
}

/// One footfall. Pitch is varied per step at playback time, so a walk cycle
/// does not turn into a metronome.
fn footstep() -> SynthSound {
    let mut rng = audio_stream(6);
    let mut heel = LowPass::new(1_050.0);
    let mut grit = Resonator::new(3_200.0, 1_800.0);

    let mut out = Vec::with_capacity(samples(0.18));
    for index in 0..samples(0.18) {
        let t = at(index);
        let noise = white(&mut rng);
        out.push(
            heel.process(noise) * hit(t, 0.001, 0.026) * 1.6
                + grit.process(noise) * hit(t, 0.0, 0.011) * 0.5,
        );
    }

    fade_edges(&mut out, 0.002);
    normalize(&mut out, 0.55);
    SynthSound::new(out)
}

/// A door pulled shut: the thunk of the panel, then the latch catching.
fn car_door() -> SynthSound {
    let mut rng = audio_stream(7);
    let mut panel = LowPass::new(320.0);
    let mut latch = Resonator::new(2_400.0, 800.0);

    let mut out = Vec::with_capacity(samples(0.35));
    for index in 0..samples(0.35) {
        let t = at(index);
        let noise = white(&mut rng);
        out.push(
            panel.process(noise) * hit(t, 0.002, 0.042) * 2.0
                // 55ms later, which is about how long a door takes to seat.
                + latch.process(noise) * hit(t - 0.055, 0.0004, 0.008) * 0.6,
        );
    }

    fade_edges(&mut out, 0.003);
    normalize(&mut out, 0.75);
    SynthSound::new(out)
}

// ----------------------------------------------------------------- loops ----

/// Seconds per repeat of the engine loop. Short, because the loop is a
/// waveform rather than a recording: it has no events in it to repeat audibly.
const ENGINE_SECONDS: f32 = 0.2;
/// Cylinder firings per loop. Times the loop rate, this is the reference pitch.
const ENGINE_FIRINGS: u32 = 8;

/// A four-stroke engine at [`ENGINE_REFERENCE_HZ`], to be pitched by revs.
///
/// Built as exact harmonics of the loop frequency so it repeats perfectly, with
/// the intake hiss added separately and wrapped, because filtered noise can
/// never be periodic on the cheap.
///
/// The half-order partials matter more than they look: a stack of whole
/// harmonics is an organ pipe. Firing pulses that alternate in strength — which
/// is what an uneven-firing engine does — put energy at half the fundamental,
/// and that is the difference between "engine" and "drone".
fn engine_loop() -> SynthSound {
    let mut rng = audio_stream(8);
    let length = samples(ENGINE_SECONDS);
    let loop_hz = 1.0 / ENGINE_SECONDS;

    let mut partials = Vec::new();
    for order in 1..=30u32 {
        let harmonic = ENGINE_FIRINGS * order;
        let hz = harmonic as f32 * loop_hz;
        partials.push(Partial {
            harmonic,
            // Falling spectrum, with everything above a couple of kilohertz
            // rolled off the way a bonnet and a bulkhead roll it off.
            amplitude: (order as f32).powf(-0.85) / (1.0 + (hz / 1_600.0).powi(2)),
            phase: rng.random(),
        });
    }
    for order in 1..=12u32 {
        let harmonic = ENGINE_FIRINGS * order - ENGINE_FIRINGS / 2;
        let hz = harmonic as f32 * loop_hz;
        partials.push(Partial {
            harmonic,
            amplitude: 0.45 * (order as f32).powf(-0.9) / (1.0 + (hz / 1_200.0).powi(2)),
            phase: rng.random(),
        });
    }

    let mut buffer = harmonics(length, &partials);

    let fade = samples(0.04);
    let mut intake = LowPass::new(2_400.0);
    let hiss: Vec<f32> = (0..length + fade)
        .map(|_| intake.process(white(&mut rng)))
        .collect();
    for (sample, hiss) in buffer.iter_mut().zip(wrap_seam(hiss, fade)) {
        *sample += hiss * 0.5;
    }

    normalize(&mut buffer, 0.85);
    SynthSound::new(buffer)
}

const SIREN_SECONDS: f32 = 1.6;
/// Whole cycles of the fundamental in one wail.
///
/// It has to be a whole number. The siren's pitch is swept continuously, so the
/// only thing that puts the waveform back where it started at the end of the
/// loop is the total accumulated phase coming out an exact multiple of a turn —
/// and since the sweep itself averages out over a full wail, that total is just
/// the mean frequency times the duration.
const SIREN_CYCLES: f32 = 1_760.0;

/// A two-tone wail.
fn siren_loop() -> SynthSound {
    let length = samples(SIREN_SECONDS);
    let mean_hz = SIREN_CYCLES / SIREN_SECONDS;
    let depth_hz = 380.0;

    // Accumulated in f64 and wrapped every turn. Neither is fussiness: a swept
    // tone needs seventy thousand additions to get round the loop, and in f32
    // the rounding error alone drifts the closing phase by a tenth of a radian
    // — which is audible, once every wail, as a tick.
    let mut phase = 0.0f64;
    let tau = std::f64::consts::TAU;

    let mut out = Vec::with_capacity(length);
    for index in 0..length {
        // Exactly one sweep per loop, so the modulation closes too.
        let turn = index as f32 / length as f32;
        let hz = mean_hz + depth_hz * (TAU * turn).sin();

        // A horn is not a sine. Two harmonics above the fundamental are enough
        // to make it cut through an engine.
        let voice = phase.sin() * 0.70 + (phase * 2.0).sin() * 0.26 + (phase * 3.0).sin() * 0.12;
        // Slight swell towards the top of the sweep.
        out.push(voice as f32 * (0.82 + 0.18 * (TAU * turn).sin()));

        phase += tau * hz as f64 / SAMPLE_RATE as f64;
        if phase >= tau {
            phase -= tau;
        }
    }

    normalize(&mut out, 0.8);
    SynthSound::new(out)
}

/// Tyres letting go: narrow-band noise with a warble in it.
fn screech_loop() -> SynthSound {
    let mut rng = audio_stream(9);
    let length = samples(0.5);
    let fade = samples(0.06);

    let mut low = Resonator::new(1_150.0, 320.0);
    let mut high = Resonator::new(2_700.0, 900.0);
    let raw: Vec<f32> = (0..length + fade)
        .map(|index| {
            let noise = white(&mut rng);
            // A slow warble stops it reading as a test tone.
            let warble = 0.75 + 0.25 * (TAU * at(index) * 7.0).sin();
            (low.process(noise) * 1.0 + high.process(noise) * 0.45) * warble
        })
        .collect();

    let mut buffer = wrap_seam(raw, fade);
    normalize(&mut buffer, 0.75);
    SynthSound::new(buffer)
}

/// The city itself: distant traffic, felt more than heard.
fn ambience_loop() -> SynthSound {
    let mut rng = audio_stream(10);
    let length = samples(4.0);
    let fade = samples(0.6);

    let mut rumble = LowPass::new(130.0);
    let mut air = LowPass::new(850.0);
    let raw: Vec<f32> = (0..length + fade)
        .map(|_| {
            let noise = white(&mut rng);
            rumble.process(noise) * 1.0 + air.process(noise) * 0.12
        })
        .collect();

    let mut buffer = wrap_seam(raw, fade);
    normalize(&mut buffer, 0.55);
    SynthSound::new(buffer)
}

// --------------------------------------------------------------- stingers ----

/// One plucked note in a stinger.
fn note(t: f32, start: f32, hz: f32, half_life: f32) -> f32 {
    let local = t - start;
    if local < 0.0 {
        return 0.0;
    }
    let phase = TAU * hz * local;
    // A little of the octave and the twelfth, so it reads as an instrument
    // rather than a test tone.
    (phase.sin() + (phase * 2.0).sin() * 0.35 + (phase * 3.0).sin() * 0.14)
        * hit(local, 0.004, half_life)
}

/// Heard when the wanted level goes up: two notes, rising, urgent.
fn star_sting() -> SynthSound {
    let mut out = Vec::with_capacity(samples(0.6));
    for index in 0..samples(0.6) {
        let t = at(index);
        out.push(note(t, 0.0, 587.33, 0.10) + note(t, 0.13, 880.0, 0.22));
    }
    fade_edges(&mut out, 0.004);
    normalize(&mut out, 0.55);
    SynthSound::new(out)
}

/// Heard on finishing a job: a major triad, arpeggiated.
fn mission_chime() -> SynthSound {
    let mut out = Vec::with_capacity(samples(1.0));
    for index in 0..samples(1.0) {
        let t = at(index);
        out.push(
            note(t, 0.0, 523.25, 0.30) + note(t, 0.11, 659.25, 0.30) + note(t, 0.22, 783.99, 0.45),
        );
    }
    fade_edges(&mut out, 0.004);
    normalize(&mut out, 0.5);
    SynthSound::new(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Largest jump between adjacent samples, and across the loop point.
    fn steps(buffer: &[f32]) -> (f32, f32) {
        let inside = buffer
            .windows(2)
            .fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        let seam = (buffer[0] - buffer[buffer.len() - 1]).abs();
        (inside, seam)
    }

    #[test]
    fn every_loop_closes_on_itself() {
        // A discontinuity at the loop point is a click, once per repeat, for as
        // long as the sound plays. It is the single most audible thing that can
        // go wrong here, so it gets its own test.
        for (name, sound) in [
            ("engine", engine_loop()),
            ("siren", siren_loop()),
            ("screech", screech_loop()),
            ("ambience", ambience_loop()),
        ] {
            let buffer: Vec<f32> = sound.decoder().collect();
            let (inside, seam) = steps(&buffer);
            assert!(
                seam <= inside * 1.5,
                "{name} jumps {seam} at the loop point, against {inside} within it"
            );
        }
    }

    #[test]
    fn one_shots_start_and_end_in_silence() {
        for (name, sound) in [
            (
                "pistol",
                shot(Shot {
                    seed: 1,
                    length: 0.42,
                    crack: 0.028,
                    brightness: 1_500.0,
                    thump: 240.0,
                    tail: 0.16,
                }),
            ),
            ("explosion", explosion()),
            ("crash", crash()),
            ("footstep", footstep()),
            ("chime", mission_chime()),
        ] {
            let buffer: Vec<f32> = sound.decoder().collect();
            assert_eq!(buffer[0], 0.0, "{name} starts mid-waveform");
            assert!(
                buffer[buffer.len() - 1].abs() < 1e-3,
                "{name} is cut off rather than finished"
            );
        }
    }

    #[test]
    fn nothing_in_the_bank_clips() {
        for (name, sound) in [
            (
                "pistol",
                shot(Shot {
                    seed: 1,
                    length: 0.42,
                    crack: 0.028,
                    brightness: 1_500.0,
                    thump: 240.0,
                    tail: 0.16,
                }),
            ),
            ("explosion", explosion()),
            ("crash", crash()),
            ("engine", engine_loop()),
            ("siren", siren_loop()),
            ("ambience", ambience_loop()),
            ("star", star_sting()),
        ] {
            let peak = sound
                .decoder()
                .fold(0.0f32, |m: f32, s: f32| m.max(s.abs()));
            assert!(peak <= 1.0, "{name} peaks at {peak}");
            assert!(peak > 0.3, "{name} is nearly silent at {peak}");
        }
    }

    #[test]
    fn a_gunshot_is_over_before_the_weapon_can_fire_again() {
        // The SMG fires every 85ms. Its report has to be shorter than a pistol's
        // or automatic fire turns into one continuous roar.
        let smg = shot(Shot {
            seed: 2,
            length: 0.26,
            crack: 0.016,
            brightness: 2_300.0,
            thump: 300.0,
            tail: 0.09,
        });
        let pistol = shot(Shot {
            seed: 1,
            length: 0.42,
            crack: 0.028,
            brightness: 1_500.0,
            thump: 240.0,
            tail: 0.16,
        });
        assert!(smg.duration() < pistol.duration());
    }

    #[test]
    fn synthesis_is_reproducible() {
        let first: Vec<f32> = explosion().decoder().collect();
        let second: Vec<f32> = explosion().decoder().collect();
        assert_eq!(first, second, "the same build must make the same noise");
    }
}
