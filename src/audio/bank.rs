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
    LowPass, Osc, Partial, Resonator, SAMPLE_RATE, SynthSound, at, fade_edges, harmonics, hit,
    normalize, samples, white, wrap_seam,
};
use super::voice::{self, Onset, Syllable};
use crate::core::rng::{stream, stream_for};

/// Every sound, loaded once and shared by everything that plays it.
#[derive(Resource)]
pub struct SoundBank {
    pub boing: Handle<SynthSound>,
    /// Nothing triggers this since vehicles stopped being wreckable; it stays
    /// in the bank, tested and auditioned, for the planned world-damage
    /// milestone — things in this city may yet go bang, just not people.
    pub explosion: Handle<SynthSound>,
    pub crash: Handle<SynthSound>,
    pub honk: Handle<SynthSound>,
    pub wheee: Handle<SynthSound>,
    pub sproing: Handle<SynthSound>,
    pub spray: Handle<SynthSound>,
    pub footstep: Handle<SynthSound>,
    pub car_door: Handle<SynthSound>,
    pub engine: Handle<SynthSound>,
    pub screech: Handle<SynthSound>,
    pub ambience: Handle<SynthSound>,
    pub birdsong: Handle<SynthSound>,
    pub uproar: Handle<SynthSound>,

    // --- voices ---
    //
    // Several of each where one would be recognised as a repeat. A flummi
    // saying the same four syllables every time it is annoyed stops being a
    // citizen and becomes a doorbell, and playback pitches these further apart
    // again per speaker. The ones that are single are single because they are
    // short enough that nobody hears them as a phrase.
    pub whistle: [Handle<SynthSound>; VARIANTS],
    pub giggle: Handle<SynthSound>,
    pub grumble: [Handle<SynthSound>; VARIANTS],
    pub curse: [Handle<SynthSound>; VARIANTS],
    /// The taunt rotation: raspberry, fart, cough, spit, burp. One would wear
    /// out inside a minute; the game's whole verb deserves a repertoire.
    pub raspberry: Handle<SynthSound>,
    pub fart: Handle<SynthSound>,
    pub cough: Handle<SynthSound>,
    pub spit: Handle<SynthSound>,
    pub burp: Handle<SynthSound>,
    /// Making up: two contrite syllables, thrown with a flower.
    pub sorry: Handle<SynthSound>,
    pub gasp: Handle<SynthSound>,
    /// Taking fright: a short falling shriek, played off `TookFright` by
    /// `mood::voice::squeal_in_fright`. A mouth noise like the cough and the
    /// spit, so it may take a recording; the synth version stands in.
    pub squeal: Handle<SynthSound>,
}

/// How many takes of each spoken sound the bank holds.
pub const VARIANTS: usize = 3;

/// Firing frequency, in hertz, that the engine loop plays at unit speed.
/// Everything driving it scales from here.
pub const ENGINE_REFERENCE_HZ: f32 = 40.0;

pub fn build(sounds: &mut Assets<SynthSound>) -> SoundBank {
    // Recordings first, synthesis as the fallback, decided per sound — see
    // `super::files`. The peaks handed to the file loader are the same ones
    // the synthesised versions normalise to, so swapping a recording in
    // never moves that sound's place in the mix.
    let dir = super::files::dir();
    let shot = |name: &str, peak: f32, synth: fn() -> SynthSound| {
        super::files::one_shot(&dir, name, peak).unwrap_or_else(synth)
    };
    let looped = |name: &str, peak: f32, synth: fn() -> SynthSound| {
        super::files::looping(&dir, name, peak).unwrap_or_else(synth)
    };
    let shot_take = |name: &str, take: usize, peak: f32, synth: fn(usize) -> SynthSound| {
        super::files::one_shot_take(&dir, name, take, peak).unwrap_or_else(|| synth(take))
    };

    SoundBank {
        boing: sounds.add(shot("boing", 0.85, boing)),
        explosion: sounds.add(shot("explosion", 1.0, explosion)),
        crash: sounds.add(shot("crash", 0.9, crash)),
        honk: sounds.add(shot("honk", 0.8, honk)),
        wheee: sounds.add(shot("wheee", 0.7, wheee)),
        sproing: sounds.add(shot("sproing", 0.8, sproing)),
        spray: sounds.add(looped("spray", 0.6, spray_loop)),
        footstep: sounds.add(shot("footstep", 0.55, footstep)),
        car_door: sounds.add(shot("car-door", 0.75, car_door)),
        engine: sounds.add(looped("engine", 0.85, engine_loop)),
        screech: sounds.add(looped("screech", 0.75, screech_loop)),
        ambience: sounds.add(looped("ambience", 0.55, ambience_loop)),
        birdsong: sounds.add(looped("birdsong", 0.5, birdsong_loop)),
        uproar: sounds.add(looped("uproar", 0.6, uproar_loop)),

        // The spoken voices are deliberately not replaceable by files: a
        // flummi's voice is a source-filter instrument pitched per speaker at
        // playback, and a recording of a human *talking* would put an actual
        // human in a city that must not contain one. That rule used to cover
        // the taunt rotation and the sorry too, which was drawing the line in
        // the wrong place: those are mouth noises rather than speech — nobody
        // hears a word in a recorded cough — and they play with per-shot
        // jitter, not a per-speaker pitch. So they take recordings like every
        // other effect. The whistle crossed the same line next: whistled
        // notes carry no words either, and the synthesised tunes read as a
        // doorbell where a recorded human whistling reads as a mood — so each
        // take looks for `whistle-<n>` on disk first, falling back per take.
        // The giggle, grumble, curse and gasp stay instruments.
        whistle: std::array::from_fn(|take| sounds.add(shot_take("whistle", take, 0.55, whistle))),
        giggle: sounds.add(giggle()),
        grumble: std::array::from_fn(|take| sounds.add(grumble(take))),
        curse: std::array::from_fn(|take| sounds.add(curse(take))),
        raspberry: sounds.add(shot("raspberry", 0.85, raspberry)),
        fart: sounds.add(shot("fart", 0.85, fart)),
        cough: sounds.add(shot("cough", 0.8, cough)),
        spit: sounds.add(shot("spit", 0.7, spit)),
        burp: sounds.add(shot("burp", 0.85, burp)),
        sorry: sounds.add(shot("sorry", 0.6, sorry)),
        gasp: sounds.add(gasp()),
        squeal: sounds.add(shot("squeal", 0.7, squeal)),
    }
}

fn audio_stream(seed: u64) -> ChaCha8Rng {
    stream_for(seed, stream::AUDIO)
}

/// Every one-shot in the bank, by name.
///
/// Built rather than listed at each use, because there are three places that
/// want the whole bank — the two tests that hold it to its rules, and the
/// audition tool — and a hand-kept list in each of them means a new sound is
/// exempt from the rules until somebody notices.
pub fn every_one_shot() -> Vec<(String, SynthSound)> {
    let mut all = vec![
        ("boing".to_string(), boing()),
        ("explosion".to_string(), explosion()),
        ("crash".to_string(), crash()),
        ("honk".to_string(), honk()),
        ("wheee".to_string(), wheee()),
        ("sproing".to_string(), sproing()),
        ("footstep".to_string(), footstep()),
        ("car-door".to_string(), car_door()),
        ("giggle".to_string(), giggle()),
        ("raspberry".to_string(), raspberry()),
        ("fart".to_string(), fart()),
        ("cough".to_string(), cough()),
        ("spit".to_string(), spit()),
        ("burp".to_string(), burp()),
        ("sorry".to_string(), sorry()),
        ("gasp".to_string(), gasp()),
        ("squeal".to_string(), squeal()),
    ];
    for take in 0..VARIANTS {
        all.push((format!("whistle-{take}"), whistle(take)));
        all.push((format!("grumble-{take}"), grumble(take)));
        all.push((format!("curse-{take}"), curse(take)));
    }
    all
}

/// And every loop.
pub fn every_loop() -> Vec<(String, SynthSound)> {
    vec![
        ("engine".to_string(), engine_loop()),
        ("screech".to_string(), screech_loop()),
        ("ambience".to_string(), ambience_loop()),
        ("birdsong".to_string(), birdsong_loop()),
        ("uproar".to_string(), uproar_loop()),
        ("spray".to_string(), spray_loop()),
    ]
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

/// A car hitting something, in a city where the car is made of rubber.
///
/// This used to be four inharmonic resonators and nothing else — sheet metal,
/// correct and grim. Rubber cars want a slapstick crash: there is still a
/// clatter at the front so the ear knows something arrived hard, but most of
/// the sound is now what happens *after* — a fat elastic sweep down and a
/// wobble that rings on like a saw blade twanged over a knee, which is the
/// sound of a bumper deciding to be a spring after all.
fn crash() -> SynthSound {
    let mut rng = audio_stream(4);
    let mut panels = [
        (Resonator::new(287.0, 24.0), 1.00),
        (Resonator::new(631.0, 38.0), 0.70),
        (Resonator::new(1_103.0, 60.0), 0.45),
        (Resonator::new(1_877.0, 110.0), 0.28),
    ];
    let mut body = LowPass::new(170.0);
    let mut sproing = Osc::new();
    let mut wobble = Osc::new();

    let mut out = Vec::with_capacity(samples(0.9));
    for index in 0..samples(0.9) {
        let t = at(index);
        let noise = white(&mut rng);
        let strike = noise * hit(t, 0.0006, 0.010);

        let mut metal = 0.0;
        for (resonator, weight) in &mut panels {
            metal += resonator.process(strike) * *weight;
        }
        let low = body.process(noise) * hit(t, 0.003, 0.085) * 1.8;

        // The rubber taking over: a boing-style sweep, an octave below the
        // signature boing because a car is a much bigger ball.
        let sweep_hz = 62.0 + 260.0 * (-t * 9.0).exp();
        let elastic = sproing.sine(sweep_hz) * hit(t - 0.015, 0.008, 0.16) * 1.4;

        // The twang: a low tone whose pitch itself shudders, the shudder dying
        // out as the bumper settles. Pitch wobble rather than amplitude wobble
        // — tremolo reads as a siren, vibrato as something physically flapping.
        let flap = 1.0 + 0.22 * (TAU * 11.0 * t).sin() * (-t * 3.5).exp();
        let twang = wobble.sine(96.0 * flap) * hit(t - 0.05, 0.02, 0.30) * 1.1;

        out.push(metal * hit(t, 0.0, 0.16) * 0.7 + low + strike * 0.5 + elastic + twang);
    }

    fade_edges(&mut out, 0.003);
    normalize(&mut out, 0.9);
    SynthSound::new(out)
}

/// The honk of a car with feelings about what just happened to it.
///
/// Two reedy notes a rude interval apart, pressed twice — the double press is
/// what turns "horn" into "indignation". The half-second of silence at the
/// front is deliberate and load-bearing: the honk is spawned by the crash, and
/// the pause between the crash and the honk is the joke, the beat in which the
/// car collects itself before complaining.
fn honk() -> SynthSound {
    /// The two reeds. A tritone-adjacent pair: consonant enough to be a chord,
    /// sour enough to be a complaint.
    const REEDS: [f32; 2] = [365.0, 462.0];
    /// When each press starts and how long it is held. The second is held
    /// longer, the way the second press of a real honk always is.
    const PRESSES: [(f32, f32); 2] = [(0.42, 0.18), (0.68, 0.42)];

    let mut oscs = [Osc::new(), Osc::new()];
    let length = samples(1.25);
    let mut out = Vec::with_capacity(length);

    for index in 0..length {
        let t = at(index);

        let mut pressed = 0.0f32;
        for (start, held) in PRESSES {
            let into = t - start;
            if into <= 0.0 {
                continue;
            }
            // Fast attack, held flat, quick release: an electric horn has no
            // dynamics, which is exactly what makes it rude.
            let press =
                (into / 0.012).clamp(0.0, 1.0) * (1.0 - (into - held) / 0.05).clamp(0.0, 1.0);
            pressed = pressed.max(press);
        }

        let mut voice = 0.0;
        for (osc, reed_hz) in oscs.iter_mut().zip(REEDS) {
            // The diaphragm starts a shade flat and rises as it gets going.
            let hz = reed_hz * (0.965 + 0.035 * (pressed * 3.0).min(1.0));
            let phase = osc.advance(hz);
            // A soft square — a reed is closer to that than to a sine, and the
            // odd harmonics are what make it a horn rather than an organ.
            let reed = (TAU * phase).sin()
                + (TAU * phase * 3.0).sin() / 3.0
                + (TAU * phase * 5.0).sin() / 5.0;
            voice += reed;
        }

        out.push(voice * pressed);
    }

    fade_edges(&mut out, 0.004);
    normalize(&mut out, 0.8);
    SynthSound::new(out)
}

/// Somebody sailing through the air, scored by a slide whistle.
///
/// The oldest gag in the cartoon songbook: a rising glissando with a vibrato
/// that gets more excited the higher it goes. Played wherever a body has just
/// been launched, which in this city is often.
fn wheee() -> SynthSound {
    let mut rng = audio_stream(27);
    let mut pipe = Osc::new();
    let mut breath = LowPass::new(2_600.0);

    let length = samples(0.85);
    let mut out = Vec::with_capacity(length);
    for index in 0..length {
        let t = at(index);
        let through = (t / 0.75).min(1.0);

        // A little over an octave and a half, swept as a power so the climb
        // accelerates — a linear sweep sounds like a test instrument.
        let glide = 440.0 * 3.1f32.powf(through.powf(1.35));
        // The vibrato deepens and quickens on the way up: the whistler is
        // enjoying this.
        let excitement = 1.0 + (0.01 + 0.045 * through) * (TAU * (5.5 + 4.0 * through) * t).sin();
        let hz = glide * excitement;

        let tone = pipe.sine(hz);
        // The chiff of air over the fipple, or nobody believes the pipe.
        let air = breath.process(white(&mut rng)) * 0.18;

        // Swells in and rides out: the flight is loudest mid-arc.
        let envelope = (t / 0.05).clamp(0.0, 1.0) * (1.0 - (t - 0.70) / 0.15).clamp(0.0, 1.0);
        out.push((tone + air) * envelope);
    }

    fade_edges(&mut out, 0.006);
    normalize(&mut out, 0.7);
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

/// A steel post leaving its footing: a snap, then the freed pole ringing and
/// wobbling as it flies.
///
/// The wobble is the crash's twang trick at a higher pitch and a faster
/// flutter — a parking meter is a tuning fork next to a bumper. The snap in
/// front is a single broadband tick: bolts do not creak on the way out, they
/// let go all at once, and the suddenness is what sells the shear.
fn sproing() -> SynthSound {
    let mut rng = audio_stream(30);
    let mut stub = Resonator::new(1_450.0, 90.0);
    let mut pole = Osc::new();

    let length = samples(0.55);
    let mut out = Vec::with_capacity(length);
    for index in 0..length {
        let t = at(index);
        let noise = white(&mut rng);

        let snap = noise * hit(t, 0.0004, 0.006);
        let ring = stub.process(snap) * hit(t, 0.0, 0.12) * 0.8;

        // The freed pole: pitch shudders hard at first and settles as the
        // tumble takes over.
        let flutter = 1.0 + 0.30 * (TAU * 19.0 * t).sin() * (-t * 5.0).exp();
        let boing = pole.sine(210.0 * flutter) * hit(t - 0.01, 0.006, 0.20) * 1.3;

        out.push(snap * 0.7 + ring + boing);
    }

    fade_edges(&mut out, 0.004);
    normalize(&mut out, 0.8);
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

// ---------------------------------------------------------------- voices ----
//
// Everything below is gibberish on purpose — see `super::voice`. What carries
// the mood is the shape of the phrase and not the syllables in it: a giggle is
// short syllables climbing, a grumble is long ones falling, and a curse is a
// plosive followed by two hard hits. Written that way, the mood is a contour
// somebody can argue with rather than a sample somebody has to re-record.

/// Base pitch of a flummi's speaking voice, in hertz. Playback moves each
/// citizen away from this again, so it is only the middle of the crowd.
const SPEAKING_HZ: f32 = 168.0;

/// A cheerful tune, whistled. Six notes wandering about a major triad, so it
/// reads as somebody idly pleased rather than as a signal.
fn whistle(take: usize) -> SynthSound {
    let mut rng = audio_stream(11 + take as u64);
    let root = voice::step(880.0, take as f32 * 2.0);
    let tunes = [
        [0.0, 4.0, 7.0, 4.0, 9.0, 7.0],
        [0.0, 7.0, 5.0, 9.0, 12.0, 7.0],
        [0.0, 2.0, 4.0, 9.0, 7.0, 12.0],
    ];
    let notes: Vec<f32> = tunes[take % tunes.len()]
        .iter()
        .map(|semitones| voice::step(root, *semitones))
        .collect();

    let mut out = voice::whistle(&mut rng, &notes, 1.15);
    fade_edges(&mut out, 0.012);
    normalize(&mut out, 0.55);
    SynthSound::new(out)
}

/// Two low ragged syllables sliding downwards. The *recorded* burp is the
/// joke (rubberduck's creature pack, fetched with cough and spit); this is
/// only what a fresh clone hears until the fetch script has run.
fn burp() -> SynthSound {
    let mut rng = audio_stream(37);
    let base = SPEAKING_HZ * 0.42;
    let syllables = [
        Syllable::new(voice::OH, base, 0.22)
            .onset(Onset::Plosive)
            .bend(0.8),
        Syllable::new(voice::OH, base * 0.85, 0.3)
            .bend(0.7)
            .gain(0.8),
    ];
    let mut out = voice::utter(&mut rng, &syllables);
    fade_edges(&mut out, 0.006);
    normalize(&mut out, 0.85);
    SynthSound::new(out)
}

/// One high syllable bent hard downwards: the shape of a startle. The
/// giggle's rise is the shape of a joke; the same energy falling is somebody
/// getting out of the way of one.
fn squeal() -> SynthSound {
    let mut rng = audio_stream(36);
    let syllable = Syllable::new(voice::EE, SPEAKING_HZ * 3.4, 0.34)
        .onset(Onset::Plosive)
        .bend(0.5);
    let mut out = voice::utter(&mut rng, &[syllable]);
    fade_edges(&mut out, 0.004);
    normalize(&mut out, 0.7);
    SynthSound::new(out)
}

/// Four short syllables climbing. The rise is the laugh — the same syllables
/// falling are a sob, which is a different game entirely.
fn giggle() -> SynthSound {
    let mut rng = audio_stream(14);
    let base = SPEAKING_HZ * 1.35;
    let syllables: Vec<Syllable> = (0..4)
        .map(|index| {
            let hz = voice::step(base, index as f32 * 1.8);
            Syllable::new(voice::EE, hz, 0.075)
                .onset(Onset::Fricative)
                .bend(1.18)
                .gain(1.0 - index as f32 * 0.12)
        })
        .collect();

    let mut out = voice::utter(&mut rng, &syllables);
    fade_edges(&mut out, 0.006);
    normalize(&mut out, 0.6);
    SynthSound::new(out)
}

/// Muttering: low, dark vowels on a falling contour, with the top taken off it.
///
/// The low-pass is what makes it muttering rather than speech. A grumble is
/// something said into a collar, and the ear reads a missing top end as
/// somebody not meaning you to catch it.
fn grumble(take: usize) -> SynthSound {
    let mut rng = audio_stream(17 + take as u64);
    let base = SPEAKING_HZ * (0.62 + take as f32 * 0.05);
    let syllables: Vec<Syllable> = (0..3)
        .map(|index| {
            let hz = voice::step(base, -(index as f32) * 1.5);
            Syllable::new(voice::dark_vowel(&mut rng), hz, 0.17 + index as f32 * 0.02).bend(0.88)
        })
        .collect();

    let mut out = voice::utter(&mut rng, &syllables);
    let mut collar = LowPass::new(1_400.0);
    for sample in &mut out {
        *sample = collar.process(*sample);
    }
    fade_edges(&mut out, 0.008);
    normalize(&mut out, 0.6);
    SynthSound::new(out)
}

/// Swearing: a plosive and two hard syllables, bitten off at the end.
///
/// The plosive is doing nearly all the work. Without it this is shouting; with
/// it the ear supplies a consonant and hears a word it cannot quite make out,
/// which is funnier and, conveniently, cannot be quoted back at anybody.
fn curse(take: usize) -> SynthSound {
    let mut rng = audio_stream(21 + take as u64);
    let base = SPEAKING_HZ * (1.10 + take as f32 * 0.08);
    let syllables = [
        Syllable::new(voice::AH, base, 0.13)
            .onset(Onset::Plosive)
            .bend(0.82),
        Syllable::new(voice::any_vowel(&mut rng), voice::step(base, -3.0), 0.10)
            .onset(Onset::Plosive)
            .bend(0.72)
            .gain(0.85),
    ];

    let mut out = voice::utter(&mut rng, &syllables);
    fade_edges(&mut out, 0.005);
    normalize(&mut out, 0.8);
    SynthSound::new(out)
}

/// The provocation. Nothing in the game is ruder and nothing is cheaper.
fn raspberry() -> SynthSound {
    let mut rng = audio_stream(25);
    let mut out = voice::raspberry(&mut rng, 0.55, 35.0);
    fade_edges(&mut out, 0.008);
    normalize(&mut out, 0.85);
    SynthSound::new(out)
}

/// The other rude noise. A raspberry is lips; this is lower, flabbier, and
/// filtered as though through a coat — [`voice::raspberry`] at a fraction of
/// the buzz, which is all the difference there ever was.
fn fart() -> SynthSound {
    let mut rng = audio_stream(32);
    let mut out = voice::raspberry(&mut rng, 0.62, 17.0);
    let mut coat = LowPass::new(700.0);
    for sample in &mut out {
        *sample = coat.process(*sample);
    }
    fade_edges(&mut out, 0.010);
    normalize(&mut out, 0.85);
    SynthSound::new(out)
}

/// Two deliberate coughs, aimed. What makes it an insult rather than a cold
/// is that it is *performed*: two even, unhurried bursts, each an AH shaped
/// by a real throat, with none of the raggedness of somebody actually ill.
fn cough() -> SynthSound {
    let mut rng = audio_stream(33);
    let syllables = [
        Syllable::new(voice::AH, SPEAKING_HZ * 0.72, 0.12)
            .onset(Onset::Plosive)
            .bend(0.70),
        Syllable::new(voice::AH, SPEAKING_HZ * 0.68, 0.14)
            .onset(Onset::Plosive)
            .bend(0.62)
            .gain(0.9),
    ];
    let mut out = voice::utter(&mut rng, &syllables);
    // Coughs are mostly breath: mix the voiced part with a burst of raw noise
    // riding the same envelope, or it reads as somebody saying "uh uh".
    let mut breath = LowPass::new(1_900.0);
    for (index, sample) in out.iter_mut().enumerate() {
        let t = at(index);
        let burst = hit(t, 0.004, 0.05) + hit(t - 0.16, 0.004, 0.055);
        *sample = *sample * 0.6 + breath.process(white(&mut rng)) * burst * 0.9;
    }
    fade_edges(&mut out, 0.005);
    normalize(&mut out, 0.8);
    SynthSound::new(out)
}

/// A spit: the lips letting go, a short hiss, and — the half the joke lives
/// in — a tiny wet *plip* a beat later, which is the payload landing.
fn spit() -> SynthSound {
    let mut rng = audio_stream(34);
    let mut hiss = Resonator::new(3_000.0, 1_900.0);
    let mut plip = Resonator::new(1_200.0, 250.0);

    let length = samples(0.5);
    let mut out = Vec::with_capacity(length);
    for index in 0..length {
        let t = at(index);
        let noise = white(&mut rng);
        let ptt = hiss.process(noise) * (hit(t, 0.001, 0.008) + hit(t - 0.012, 0.002, 0.030) * 0.6);
        // 0.35s of flight before it lands somewhere off to the side.
        let land = plip.process(noise * hit(t - 0.35, 0.0008, 0.004)) * 0.8;
        out.push(ptt + land);
    }
    fade_edges(&mut out, 0.004);
    normalize(&mut out, 0.7);
    SynthSound::new(out)
}

/// Making up: "so-rry", or near enough. Two soft syllables, the first
/// falling, the second bending back *up* — the rising tail is what makes it
/// contrite rather than dismissive; the same two syllables falling are
/// "whatever".
fn sorry() -> SynthSound {
    let mut rng = audio_stream(35);
    let syllables = [
        Syllable::new(voice::OO, SPEAKING_HZ * 0.95, 0.16).bend(0.86),
        Syllable::new(voice::EH, SPEAKING_HZ * 0.80, 0.20)
            .bend(1.22)
            .gain(0.8),
    ];
    let mut out = voice::utter(&mut rng, &syllables);
    fade_edges(&mut out, 0.008);
    normalize(&mut out, 0.6);
    SynthSound::new(out)
}

/// Somebody seeing it coming.
fn gasp() -> SynthSound {
    let mut rng = audio_stream(26);
    let mut out = voice::gasp(&mut rng, 0.30);
    fade_edges(&mut out, 0.004);
    normalize(&mut out, 0.5);
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
///
/// One of three beds the mixer crossfades on the city's mood — this is the
/// neutral one, and [`birdsong_loop`] and [`uproar_loop`] are the two poles.
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

/// A street the city is pleased with: birds in it.
///
/// A handful of chirps scattered over a long loop, each one a short trilled
/// sweep — a bird call is a whistle whose pitch shakes faster than a hand
/// could shake it, and the trill is all that separates "bird" from "referee".
/// The loop is long and the chirps are sparse so the repeat is not read as a
/// rhythm; nobody counts eight seconds between phrases.
fn birdsong_loop() -> SynthSound {
    let mut rng = audio_stream(28);
    let length = samples(8.0);
    let fade = samples(0.5);
    let mut raw = vec![0.0f32; length + fade];

    // A dozen phrases, each two to four syllables from one bird.
    for _ in 0..12 {
        let start = rng.random_range(0..length);
        let base = rng.random_range(2_100.0..3_900.0f32);
        let trill = rng.random_range(22.0..38.0f32);
        let syllables = rng.random_range(2..5u32);
        let mut osc = Osc::new();

        for syllable in 0..syllables {
            let onset = start + samples(0.11) * syllable as usize;
            let voiced = samples(0.07);
            for index in 0..voiced {
                let slot = onset + index;
                if slot >= raw.len() {
                    break;
                }
                let t = at(index);
                // Each syllable dips as it ends, the shape of a chirp
                // everywhere on Earth.
                let hz = base * (1.0 + 0.10 * (TAU * trill * t).sin()) * (1.0 - 0.25 * t / 0.07);
                raw[slot] += osc.sine(hz) * hit(t, 0.004, 0.035) * 0.8;
            }
        }
    }

    // Not silence between the phrases: the faintest hiss of leaves, so the
    // bed does not switch off entirely between birds.
    let mut leaves = LowPass::new(1_900.0);
    for sample in raw.iter_mut() {
        *sample += leaves.process(white(&mut rng)) * 0.05;
    }

    let mut buffer = wrap_seam(raw, fade);
    normalize(&mut buffer, 0.5);
    SynthSound::new(buffer)
}

/// A street the city is furious with: a demonstration somewhere behind the
/// buildings.
///
/// A crowd's roar is vowel-coloured noise that will not sit still — two
/// formant bands with unrelated slow swells, so the level prowls up and down
/// the way a chant crossing a square does, over a rhythmic push about twice a
/// second that the ear reads as fists going up.
fn uproar_loop() -> SynthSound {
    let mut rng = audio_stream(29);
    let length = samples(6.0);
    let fade = samples(0.8);

    let mut throats = Resonator::new(480.0, 260.0);
    let mut mouths = Resonator::new(950.0, 420.0);
    let mut mass = LowPass::new(240.0);

    let raw: Vec<f32> = (0..length + fade)
        .map(|index| {
            let t = at(index);
            let noise = white(&mut rng);
            // Two swells at rates chosen not to divide each other, so their
            // sum never quite repeats inside the loop.
            let prowl = 0.7 + 0.2 * (TAU * 0.23 * t).sin() + 0.1 * (TAU * 0.57 * t + 1.7).sin();
            // The chant: a soft-edged pulse rather than a gate, or it reads as
            // a helicopter.
            let chant = 0.75 + 0.25 * (TAU * 2.1 * t).sin().max(0.0);
            (throats.process(noise) * 1.0 + mouths.process(noise) * 0.6 + mass.process(noise) * 0.8)
                * prowl
                * chant
        })
        .collect();

    let mut buffer = wrap_seam(raw, fade);
    normalize(&mut buffer, 0.6);
    SynthSound::new(buffer)
}

/// A broken water main throwing its column into the air.
///
/// Spray is noise twice over: the hiss of the fine mist and a fatter, slower
/// chugging underneath where the column itself pulses out of the stump. The
/// pulse rides at a few hertz — fast enough to read as turbulence, slow
/// enough not to read as a helicopter — and its rate is picked against the
/// swell the way the uproar's are: unrelated, so the pair never audibly
/// repeats inside the loop.
fn spray_loop() -> SynthSound {
    let mut rng = audio_stream(31);
    let length = samples(2.0);
    let fade = samples(0.25);

    let mut mist = Resonator::new(4_200.0, 2_600.0);
    let mut column = LowPass::new(420.0);

    let raw: Vec<f32> = (0..length + fade)
        .map(|index| {
            let t = at(index);
            let noise = white(&mut rng);
            let chug = 0.75 + 0.25 * (TAU * 6.3 * t).sin();
            let swell = 0.85 + 0.15 * (TAU * 0.9 * t + 2.1).sin();
            (mist.process(noise) * 0.7 + column.process(noise) * chug * 1.6) * swell
        })
        .collect();

    let mut buffer = wrap_seam(raw, fade);
    normalize(&mut buffer, 0.6);
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
        for (name, sound) in every_loop() {
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
        for (name, sound) in every_one_shot() {
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
        let mut all = every_one_shot();
        all.extend(every_loop());
        for (name, sound) in all {
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
