//! Every sound the game can make, computed at startup.
//!
//! These are written the way a synthesist would describe them rather than the
//! way a sampler would: a crash is four inharmonic resonators struck by a burst
//! of noise, and each of those is a named term in a sum. That is deliberate —
//! the parameters that matter are then editable, and a sound that comes out
//! wrong is a number to change rather than a recording to re-take.
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
    pub boing: Handle<SynthSound>,
    pub explosion: Handle<SynthSound>,
    pub crash: Handle<SynthSound>,
    pub footstep: Handle<SynthSound>,
    pub car_door: Handle<SynthSound>,
    pub engine: Handle<SynthSound>,
    pub screech: Handle<SynthSound>,
    pub ambience: Handle<SynthSound>,
}

/// Firing frequency, in hertz, that the engine loop plays at unit speed.
/// Everything driving it scales from here.
pub const ENGINE_REFERENCE_HZ: f32 = 40.0;

pub fn build(sounds: &mut Assets<SynthSound>) -> SoundBank {
    SoundBank {
        boing: sounds.add(boing()),
        explosion: sounds.add(explosion()),
        crash: sounds.add(crash()),
        footstep: sounds.add(footstep()),
        car_door: sounds.add(car_door()),
        engine: sounds.add(engine_loop()),
        screech: sounds.add(screech_loop()),
        ambience: sounds.add(ambience_loop()),
    }
}

fn audio_stream(seed: u64) -> ChaCha8Rng {
    stream_for(seed, stream::AUDIO)
}

// ------------------------------------------------------------- one-shots ----

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

/// The signature sound of the city: something elastic giving way and coming
/// back.
///
/// A boing is a pitch sweep, not a note. The spring is stiffest at the moment
/// of contact and slackens as it unloads, so the frequency falls steeply and
/// then flattens out — an exponential decay towards a floor, which is what the
/// two terms are. The warble on top is the wobble of a body that has not quite
/// finished deciding what shape it is.
///
/// Phase is accumulated rather than computed from `sin(TAU * f * t)`, because
/// with a frequency that changes every sample the latter is not a sweep at all:
/// it is a series of unrelated tones, and it clicks at every one of them.
fn boing() -> SynthSound {
    let length = samples(0.34);
    let mut phase: f64 = 0.0;
    let mut out = Vec::with_capacity(length);

    for index in 0..length {
        let t = at(index);
        // 640 Hz down towards 95 Hz, most of the fall inside the first 80 ms.
        let sweep = 95.0 + 545.0 * (-t * 11.0).exp();
        let warble = 1.0 + 0.11 * (TAU * 26.0 * t).sin();
        let hz = sweep * warble;

        phase += TAU as f64 * hz as f64 / SAMPLE_RATE as f64;
        if phase > TAU as f64 {
            phase -= TAU as f64;
        }
        let wave = phase.sin() as f32 + (phase * 2.0).sin() as f32 * 0.22;
        out.push(wave * hit(t, 0.003, 0.085));
    }

    fade_edges(&mut out, 0.004);
    normalize(&mut out, 0.85);
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
            ("boing", boing()),
            ("explosion", explosion()),
            ("crash", crash()),
            ("footstep", footstep()),
            ("car door", car_door()),
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
            ("boing", boing()),
            ("explosion", explosion()),
            ("crash", crash()),
            ("engine", engine_loop()),
            ("screech", screech_loop()),
            ("ambience", ambience_loop()),
        ] {
            let peak = sound
                .decoder()
                .fold(0.0f32, |m: f32, s: f32| m.max(s.abs()));
            assert!(peak <= 1.0, "{name} peaks at {peak}");
            assert!(peak > 0.3, "{name} is nearly silent at {peak}");
        }
    }

    #[test]
    fn a_boing_falls_in_pitch_rather_than_holding_a_note() {
        // Counted as zero crossings in the first and last thirds. A boing that
        // holds its pitch is a beep, and the whole city would be beeping.
        let buffer: Vec<f32> = boing().decoder().collect();
        let third = buffer.len() / 3;
        let crossings = |window: &[f32]| {
            window
                .windows(2)
                .filter(|w| w[0].signum() != w[1].signum())
                .count()
        };
        let early = crossings(&buffer[..third]);
        let late = crossings(&buffer[third * 2..]);
        assert!(
            early > late * 2,
            "{early} crossings early against {late} late is not a sweep"
        );
    }

    #[test]
    fn synthesis_is_reproducible() {
        let first: Vec<f32> = explosion().decoder().collect();
        let second: Vec<f32> = explosion().decoder().collect();
        assert_eq!(first, second, "the same build must make the same noise");
    }
}
