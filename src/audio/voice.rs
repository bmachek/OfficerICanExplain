//! Gibberish, synthesised.
//!
//! The flummis talk, and what they say is nothing at all. That is a decision
//! rather than a shortcut: made-up syllables carry tone perfectly well — a
//! rising pair is a question and a falling growl is a complaint in any language
//! — and they cannot be misheard as a real insult, which matters in a game
//! whose whole subject is people being rude to each other.
//!
//! ## Source and filter
//!
//! A voice is not a waveform. It is a buzz made in the throat, shaped by the
//! mouth, and the two halves are independent: change the buzz and the same
//! vowel comes out at a different pitch; change the shape and the same pitch
//! comes out as a different vowel. So that is how it is built here —
//! [`synth::glottal`] driven by an [`synth::Osc`] is the source, a
//! [`synth::Formant`] is the filter, and a [`Vowel`] is nothing but three
//! numbers handed to the second of them.
//!
//! The luck of it is that the kit was already here. `Resonator` was written to
//! make sheet metal ring, and a band-pass that rings is exactly what a vocal
//! tract is — a vowel is three of them and a name.
//!
//! ## What makes it sound like a person
//!
//! Three things, none of them optional:
//!
//! * **Pitch moves within a syllable.** A held pitch is a synthesiser. The
//!   contour is where the mood lives, so [`Syllable`] takes a start and an end.
//! * **There is breath in it.** A little noise through the same formants is the
//!   difference between a voice and an organ.
//! * **Syllables start with something.** A vowel that fades in from nothing is
//!   a theremin; a burst of noise in front of it is a consonant, and the ear
//!   hears a word.

use rand::RngExt;
use rand_chacha::ChaCha8Rng;

use super::synth::{self, Formant, LowPass, Osc, at, samples, white};

/// The three formant frequencies that make a vowel that vowel, in hertz.
///
/// Measured values for a male speaker, near enough. What matters is not that
/// they are exactly right but that they are far apart from each other: /i/ and
/// /u/ have to be unmistakably different or every syllable sounds the same.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vowel(pub [f32; 3]);

/// As in "aah".
pub const AH: Vowel = Vowel([800.0, 1200.0, 2800.0]);
/// As in "eh".
pub const EH: Vowel = Vowel([500.0, 1800.0, 2500.0]);
/// As in "ee".
pub const EE: Vowel = Vowel([300.0, 2300.0, 3000.0]);
/// As in "oh".
pub const OH: Vowel = Vowel([500.0, 900.0, 2400.0]);
/// As in "oo".
pub const OO: Vowel = Vowel([320.0, 800.0, 2500.0]);

/// How a syllable starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Onset {
    /// Straight into the vowel. Reads as a syllable in the middle of a word.
    None,
    /// A click of silence and then a burst — a /b/, /d/ or /k/. This is what
    /// makes a curse sound like swearing rather than like shouting.
    Plosive,
    /// A hiss running into the vowel: /f/, /s/, /sh/.
    Fricative,
}

/// One syllable.
#[derive(Clone, Copy, Debug)]
pub struct Syllable {
    pub vowel: Vowel,
    pub onset: Onset,
    /// Pitch at the start and at the end of the syllable, in hertz. The pair is
    /// the tone of voice: falling is a statement or a grumble, rising is a
    /// question or a giggle, and a wide fall is a sigh.
    pub from_hz: f32,
    pub to_hz: f32,
    pub seconds: f32,
    pub gain: f32,
}

impl Syllable {
    /// A plain syllable on a level pitch, to be adjusted by the caller.
    pub fn new(vowel: Vowel, hz: f32, seconds: f32) -> Self {
        Self {
            vowel,
            onset: Onset::None,
            from_hz: hz,
            to_hz: hz,
            seconds,
            gain: 1.0,
        }
    }

    pub fn onset(mut self, onset: Onset) -> Self {
        self.onset = onset;
        self
    }

    /// Ends the syllable at `ratio` times the pitch it started on.
    pub fn bend(mut self, ratio: f32) -> Self {
        self.to_hz = self.from_hz * ratio;
        self
    }

    pub fn gain(mut self, gain: f32) -> Self {
        self.gain = gain;
        self
    }
}

/// Seconds of silence in front of a plosive. A plosive *is* the silence as much
/// as the burst — the ear reads the gap as the mouth closing.
const STOP_GAP: f32 = 0.018;
/// And how long the burst itself lasts.
const BURST: f32 = 0.010;
/// A fricative runs for longer and fades into the vowel rather than snapping.
const HISS: f32 = 0.055;
/// How much breath noise goes through the formants alongside the buzz.
const BREATH: f32 = 0.10;
/// Vibrato depth, as a fraction of the pitch, and its rate in hertz. Small: a
/// flummi is not an opera singer.
const VIBRATO: f32 = 0.012;
const VIBRATO_HZ: f32 = 5.5;

/// Renders a run of syllables into one buffer.
///
/// The formants are rebuilt per syllable rather than interpolated between them,
/// which is audible and wanted: an abrupt change of vowel is heard as a new
/// syllable, and gliding between them turns a sentence into a yodel.
pub fn utter(rng: &mut ChaCha8Rng, syllables: &[Syllable]) -> Vec<f32> {
    let mut out = Vec::new();
    for syllable in syllables {
        match syllable.onset {
            Onset::None => {}
            Onset::Plosive => {
                out.extend(std::iter::repeat_n(0.0, samples(STOP_GAP)));
                let mut mouth = Formant::new(syllable.vowel.0);
                for index in 0..samples(BURST) {
                    let shape = 1.0 - at(index) / BURST;
                    out.push(mouth.process(white(rng)) * shape * 1.6 * syllable.gain);
                }
            }
            Onset::Fricative => {
                let mut mouth = Formant::new(syllable.vowel.0);
                let mut edge = LowPass::new(6_000.0);
                for index in 0..samples(HISS) {
                    // Swelling rather than fading: a fricative gets louder as
                    // the mouth closes towards the vowel behind it.
                    let shape = at(index) / HISS;
                    let noise = white(rng) - edge.process(white(rng));
                    out.push(mouth.process(noise) * shape * 0.8 * syllable.gain);
                }
            }
        }

        let length = samples(syllable.seconds);
        let mut folds = Osc::new();
        let mut wobble = Osc::new();
        let mut mouth = Formant::new(syllable.vowel.0);
        for index in 0..length {
            let along = index as f32 / length as f32;
            // Exponential rather than linear, so that a glide of a fifth
            // sounds the same size whether it starts high or low. Pitch is
            // heard in ratios.
            let hz = syllable.from_hz
                * (syllable.to_hz / syllable.from_hz).powf(along)
                * (1.0 + VIBRATO * wobble.sine(VIBRATO_HZ));
            let buzz = synth::glottal(folds.advance(hz));
            let voiced = mouth.process(buzz + white(rng) * BREATH);
            out.push(voiced * swell(along) * syllable.gain);
        }
    }
    out
}

/// The shape of a syllable: quick in, held, and let go.
///
/// Both edges are smoothed rather than linear. A linear attack on a buzz is
/// heard as a click, which is a consonant nobody asked for.
fn swell(along: f32) -> f32 {
    const IN: f32 = 0.12;
    const OUT: f32 = 0.30;
    let rise = (along / IN).clamp(0.0, 1.0);
    let fall = ((1.0 - along) / OUT).clamp(0.0, 1.0);
    let ramp = rise.min(fall);
    ramp * ramp * (3.0 - 2.0 * ramp)
}

/// Draws one of the five vowels.
pub fn any_vowel(rng: &mut ChaCha8Rng) -> Vowel {
    const ALL: [Vowel; 5] = [AH, EH, EE, OH, OO];
    ALL[rng.random_range(0..ALL.len())]
}

/// Draws one of the two closed, dark vowels — what a grumble is made of.
pub fn dark_vowel(rng: &mut ChaCha8Rng) -> Vowel {
    const DARK: [Vowel; 2] = [OH, OO];
    DARK[rng.random_range(0..DARK.len())]
}

/// A whistled tone: near-sine, with the pitch sliding between the notes rather
/// than stepping, and a breath of air behind it.
///
/// A whistle is the one voice here that is *not* source-and-filter, because a
/// whistle is not made in the throat. It is a resonance with almost nothing in
/// it above the fundamental, so it is written as what it is: one oscillator, a
/// little second harmonic to stop it being a test tone, and hiss.
pub fn whistle(rng: &mut ChaCha8Rng, notes: &[f32], seconds: f32) -> Vec<f32> {
    let length = samples(seconds);
    let mut tone = Osc::new();
    let mut wobble = Osc::new();
    let mut air = LowPass::new(2_600.0);
    let mut out = Vec::with_capacity(length);

    for index in 0..length {
        let along = index as f32 / length as f32;
        // Where we are in the tune, and how far between this note and the next.
        let step = along * (notes.len() - 1) as f32;
        let note = (step.floor() as usize).min(notes.len() - 2);
        let between = step - note as f32;
        // Portamento: the slide takes up the last part of each note, so the
        // pitch sits still long enough to be heard before it moves.
        let slide = ((between - 0.55) / 0.45).clamp(0.0, 1.0);
        let glide = slide * slide * (3.0 - 2.0 * slide);
        let hz = notes[note]
            * (notes[note + 1] / notes[note]).powf(glide)
            * (1.0 + VIBRATO * 1.6 * wobble.sine(VIBRATO_HZ));

        let phase = tone.advance(hz);
        let wave = (std::f32::consts::TAU * phase).sin()
            + (2.0 * std::f32::consts::TAU * phase).sin() * 0.07;
        let breath = air.process(white(rng)) * 0.16;
        // Held flat and let go at the end: a whistle does not decay, it stops.
        let shape = swell(along);
        out.push((wave + breath) * shape);
    }
    out
}

/// Air being pushed past slack lips: a buzz too low to be a pitch, filtered
/// down to a splutter.
///
/// The rate is the joke. Above about fifty a second it is a note and below
/// twenty it is a series of taps; in between it is unmistakably rude.
pub fn raspberry(rng: &mut ChaCha8Rng, seconds: f32, buzz_hz: f32) -> Vec<f32> {
    let length = samples(seconds);
    let mut flap = Osc::new();
    let mut lips = LowPass::new(700.0);
    let mut body = Formant::new([340.0, 720.0, 1_500.0]);
    let mut out = Vec::with_capacity(length);

    for index in 0..length {
        let along = index as f32 / length as f32;
        // Running out of breath towards the end, which is what makes it read as
        // somebody doing it rather than a machine.
        let hz = buzz_hz * (1.0 - 0.28 * along);
        // A duty cycle rather than a sine: the lips are either open or shut.
        let gate = if flap.advance(hz) < 0.55 { 1.0 } else { -0.35 };
        let splutter = lips.process(white(rng) * gate + gate * 0.5);
        out.push(body.process(splutter) * swell(along));
    }
    out
}

/// A sharp intake of breath.
pub fn gasp(rng: &mut ChaCha8Rng, seconds: f32) -> Vec<f32> {
    let length = samples(seconds);
    let mut throat = Formant::new([420.0, 1_500.0, 2_600.0]);
    let mut hiss = LowPass::new(3_800.0);
    let mut out = Vec::with_capacity(length);

    for index in 0..length {
        let t = at(index);
        // Fast in, slow out: breathing in is a rush and then a stall, which is
        // the opposite shape to every other sound in the bank.
        let shape = synth::hit(t, 0.012, seconds * 0.28);
        let noise = hiss.process(white(rng));
        out.push((throat.process(noise) * 0.8 + noise * 0.4) * shape);
    }
    out
}

/// Pitch of a note `semitones` above `hz`. Written out because every voice in
/// here is built from intervals and a magic 1.0595 in the middle of a tune is
/// unreadable.
pub fn step(hz: f32, semitones: f32) -> f32 {
    hz * 2.0f32.powf(semitones / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::synth::SAMPLE_RATE;
    use crate::core::rng::{stream, stream_for};

    fn voice_stream() -> ChaCha8Rng {
        stream_for(42, stream::AUDIO)
    }

    /// How often the waveform crosses zero going up, per second. The right
    /// measure for something that is near enough a single tone.
    fn crossing_rate(buffer: &[f32]) -> f32 {
        let crossings = buffer
            .windows(2)
            .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
            .count();
        crossings as f32 * SAMPLE_RATE as f32 / buffer.len() as f32
    }

    /// Fundamental of a buffer, by autocorrelation: the lag at which the
    /// waveform best matches itself is its period.
    ///
    /// Zero crossings are no use on a voice. A vowel's strongest partial is a
    /// formant, several times the fundamental, so counting crossings measures
    /// the filter and not the pitch — which is precisely the pair this module
    /// exists to keep separate.
    fn pitch(buffer: &[f32]) -> f32 {
        let window = buffer.len().min(samples(0.2));
        let slice = &buffer[..window];
        let shortest = SAMPLE_RATE as usize / 900;
        let longest = (SAMPLE_RATE as usize / 60).min(window / 2);
        let mut best = (f32::MIN, shortest);
        for lag in shortest..longest {
            let overlap = window - lag;
            let sum: f32 = slice[..overlap]
                .iter()
                .zip(&slice[lag..])
                .map(|(a, b)| a * b)
                .sum();
            // Per sample, or the score simply falls off with the lag and every
            // sound comes out as the highest pitch on the dial.
            let score = sum / overlap as f32;
            if score > best.0 {
                best = (score, lag);
            }
        }
        SAMPLE_RATE as f32 / best.1 as f32
    }

    #[test]
    fn a_glottal_pulse_opens_slower_than_it_shuts() {
        // The asymmetry is what puts energy in the harmonics the formants pick
        // out. A symmetric pulse filtered by a formant is a kazoo.
        let peak = (0..1000)
            .map(|i| i as f32 / 1000.0)
            .max_by(|a, b| synth::glottal(*a).partial_cmp(&synth::glottal(*b)).unwrap())
            .unwrap();
        assert!(
            (0.35..0.5).contains(&peak),
            "the pulse peaks at {peak:.2} of the way through"
        );
        assert_eq!(synth::glottal(0.9), 0.0, "the folds never close");
        assert_eq!(synth::glottal(0.0), 0.0);
    }

    #[test]
    fn a_vowel_comes_out_at_the_pitch_it_was_asked_for() {
        // The point of source and filter: the formants decide *which* vowel and
        // the oscillator decides what note it is on. If the filter dragged the
        // pitch about, every voice in the bank would be out of its own range.
        let mut rng = voice_stream();
        for hz in [110.0, 220.0] {
            let buffer = utter(&mut rng, &[Syllable::new(AH, hz, 0.35)]);
            let heard = pitch(&buffer[samples(0.05)..]);
            assert!(
                (heard / hz - 1.0).abs() < 0.15,
                "asked for {hz} Hz and heard about {heard:.0} Hz"
            );
        }
    }

    #[test]
    fn the_five_vowels_are_actually_different_from_each_other() {
        // Same buzz, same pitch, same length, same noise: if two vowels come
        // out as the same samples then the formants are not doing anything and
        // every flummi says one syllable over and over.
        let voiced: Vec<Vec<f32>> = [AH, EH, EE, OH, OO]
            .into_iter()
            .map(|vowel| utter(&mut voice_stream(), &[Syllable::new(vowel, 130.0, 0.2)]))
            .collect();
        for (first, one) in voiced.iter().enumerate() {
            for two in voiced.iter().skip(first + 1) {
                let apart: f32 =
                    one.iter().zip(two).map(|(a, b)| (a - b).abs()).sum::<f32>() / one.len() as f32;
                assert!(apart > 1e-3, "two vowels came out nearly identical");
            }
        }
    }

    #[test]
    fn a_plosive_starts_with_silence_rather_than_with_a_bang() {
        let mut rng = voice_stream();
        let buffer = utter(
            &mut rng,
            &[Syllable::new(AH, 120.0, 0.2).onset(Onset::Plosive)],
        );
        let gap = samples(STOP_GAP);
        assert!(
            buffer[..gap].iter().all(|s| *s == 0.0),
            "the mouth never closed before the burst"
        );
        assert!(buffer[gap..gap + samples(BURST)].iter().any(|s| *s != 0.0));
    }

    #[test]
    fn a_syllable_starts_and_ends_at_silence() {
        // Everything here is concatenated into longer utterances, so a syllable
        // that begins mid-waveform is a click in the middle of a sentence.
        let mut rng = voice_stream();
        let buffer = utter(&mut rng, &[Syllable::new(EE, 200.0, 0.25)]);
        assert_eq!(buffer[0], 0.0);
        assert!(buffer[buffer.len() - 1].abs() < 1e-4);
    }

    #[test]
    fn a_whistle_follows_the_tune_it_was_given() {
        // Measured over the first note and the last: a whistle whose pitch does
        // not track the notes is a hum.
        let mut rng = voice_stream();
        let low = 900.0;
        let high = step(low, 12.0);
        let buffer = whistle(&mut rng, &[low, high], 0.6);
        let quarter = buffer.len() / 4;
        let opened = crossing_rate(&buffer[..quarter]);
        let closed = crossing_rate(&buffer[quarter * 3..]);
        assert!(
            closed > opened * 1.6,
            "started near {opened:.0} Hz and finished near {closed:.0} Hz"
        );
    }

    #[test]
    fn a_raspberry_is_a_splutter_rather_than_a_note() {
        // A raspberry that reads as a pitch is a duck call. The test is that
        // the energy sits far below anything anybody would call a note.
        let mut rng = voice_stream();
        let buffer = raspberry(&mut rng, 0.4, 35.0);
        let heard = crossing_rate(&buffer);
        assert!(
            heard < 260.0,
            "the raspberry came out as a {heard:.0} Hz tone"
        );
    }

    #[test]
    fn voices_are_reproducible() {
        let one = utter(&mut voice_stream(), &[Syllable::new(OH, 140.0, 0.2)]);
        let two = utter(&mut voice_stream(), &[Syllable::new(OH, 140.0, 0.2)]);
        assert_eq!(one, two, "the same build must say the same thing");
    }
}
