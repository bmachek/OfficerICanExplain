//! Procedural audio synthesis.
//!
//! Same rule as everything else here: no files ship with the game, so every
//! sound is computed into a buffer of samples at startup and handed to Bevy as
//! an asset. [`SynthSound`] is that asset, and it plays through the normal
//! `AudioPlayer` path — spatialised, looped and pitch-shifted like any other
//! source, because to rodio it is just a `Source`.
//!
//! The decoder deliberately holds an `Arc` of the samples rather than a `Vec`.
//! Every sink that starts playing calls `decoder()`, and forty police cars
//! sharing one siren should share one buffer, not copy a second of audio each.
//!
//! ## Making a loop that does not click
//!
//! A looping sound is only seamless if it lands back on its own first sample.
//! Two ways to guarantee that, and both are here because they suit different
//! material:
//!
//! * [`harmonics`] sums sine partials that are exact multiples of the loop's
//!   own frequency, so every one of them completes a whole number of cycles.
//!   Exact, but only useful for tonal content.
//! * [`wrap_seam`] generates *more* audio than the loop needs and folds the
//!   surplus tail back over the head. Filtered noise can never be periodic on
//!   the cheap, so tyres, wind and engine hiss are wrapped instead.

use std::sync::Arc;
use std::time::Duration;

use bevy::audio::{ChannelCount, Decodable, Sample, SampleRate, Source};
use bevy::prelude::*;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

pub const SAMPLE_RATE: u32 = 44_100;

const RATE: SampleRate = match SampleRate::new(SAMPLE_RATE) {
    Some(rate) => rate,
    None => unreachable!(),
};

/// Everything is mono: spatial panning is rodio's job, and a stereo source
/// cannot be positioned in the world.
const MONO: ChannelCount = match ChannelCount::new(1) {
    Some(count) => count,
    None => unreachable!(),
};

/// A block of synthesised samples, playable as an audio asset.
#[derive(Asset, TypePath, Clone, Debug)]
pub struct SynthSound {
    samples: Arc<[f32]>,
}

impl SynthSound {
    pub fn new(samples: Vec<f32>) -> Self {
        Self {
            samples: samples.into(),
        }
    }

    pub fn len(&self) -> usize {
        self.samples.len()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs_f32(self.samples.len() as f32 / SAMPLE_RATE as f32)
    }
}

/// Plays one [`SynthSound`] once, from the start.
pub struct SynthDecoder {
    samples: Arc<[f32]>,
    position: usize,
}

impl Iterator for SynthDecoder {
    type Item = Sample;

    #[inline]
    fn next(&mut self) -> Option<Sample> {
        let sample = *self.samples.get(self.position)?;
        self.position += 1;
        Some(sample)
    }

    #[inline]
    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.samples.len() - self.position;
        (left, Some(left))
    }
}

impl ExactSizeIterator for SynthDecoder {}

impl Source for SynthDecoder {
    fn current_span_len(&self) -> Option<usize> {
        // Rodio's contract: `Some(0)` exactly when the source is spent.
        if self.position >= self.samples.len() {
            Some(0)
        } else {
            Some(self.samples.len())
        }
    }

    fn channels(&self) -> ChannelCount {
        MONO
    }

    fn sample_rate(&self) -> SampleRate {
        RATE
    }

    fn total_duration(&self) -> Option<Duration> {
        Some(Duration::from_secs_f32(
            self.samples.len() as f32 / SAMPLE_RATE as f32,
        ))
    }
}

impl Decodable for SynthSound {
    type Decoder = SynthDecoder;

    fn decoder(&self) -> Self::Decoder {
        SynthDecoder {
            samples: self.samples.clone(),
            position: 0,
        }
    }
}

// ------------------------------------------------------------- building ----

/// Samples in `seconds` of audio.
pub fn samples(seconds: f32) -> usize {
    (seconds * SAMPLE_RATE as f32).round().max(1.0) as usize
}

/// Seconds elapsed at sample `index`.
pub fn at(index: usize) -> f32 {
    index as f32 / SAMPLE_RATE as f32
}

/// Percussive envelope: a short linear attack, then exponential decay.
///
/// Half-life rather than a rate because it is the number you can hear: "this
/// is half as loud again every 80 milliseconds" is a description of a gunshot.
pub fn hit(t: f32, attack: f32, half_life: f32) -> f32 {
    if t < 0.0 {
        0.0
    } else if t < attack {
        t / attack
    } else {
        0.5f32.powf((t - attack) / half_life)
    }
}

/// A one-pole low-pass. Cheap, and gentle enough that sweeping its cutoff over
/// a sound does not whistle.
pub struct LowPass {
    state: f32,
    keep: f32,
}

impl LowPass {
    pub fn new(cutoff_hz: f32) -> Self {
        let mut filter = Self {
            state: 0.0,
            keep: 0.0,
        };
        filter.set_cutoff(cutoff_hz);
        filter
    }

    pub fn set_cutoff(&mut self, cutoff_hz: f32) {
        let cutoff = cutoff_hz.clamp(1.0, SAMPLE_RATE as f32 * 0.45);
        self.keep = (-std::f32::consts::TAU * cutoff / SAMPLE_RATE as f32).exp();
    }

    pub fn process(&mut self, input: f32) -> f32 {
        self.state = input * (1.0 - self.keep) + self.state * self.keep;
        self.state
    }
}

/// A two-pole resonator: a narrow band-pass that rings.
///
/// This is what makes an impact sound like metal rather than a bang. A handful
/// of them at frequencies that are *not* harmonically related is the whole
/// trick — harmonic partials read as a musical note, inharmonic ones as a sheet
/// of steel.
pub struct Resonator {
    a1: f32,
    a2: f32,
    gain: f32,
    y1: f32,
    y2: f32,
}

impl Resonator {
    pub fn new(frequency_hz: f32, bandwidth_hz: f32) -> Self {
        let r = (-std::f32::consts::PI * bandwidth_hz / SAMPLE_RATE as f32).exp();
        let theta = std::f32::consts::TAU * frequency_hz / SAMPLE_RATE as f32;
        Self {
            a1: 2.0 * r * theta.cos(),
            a2: -r * r,
            // Normalised so the peak of the band sits near unity.
            gain: (1.0 - r)
                * (1.0 - 2.0 * r * (2.0 * theta).cos() + r * r)
                    .max(0.0)
                    .sqrt(),
            y1: 0.0,
            y2: 0.0,
        }
    }

    pub fn process(&mut self, input: f32) -> f32 {
        let y = self.gain * input + self.a1 * self.y1 + self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// One partial of a periodic waveform.
pub struct Partial {
    /// Multiple of the loop's own frequency. Must be a whole number, or the
    /// loop will not close.
    pub harmonic: u32,
    pub amplitude: f32,
    /// Turns, not radians: 0.0 to 1.0 around the circle.
    pub phase: f32,
}

/// Sums partials into a buffer that repeats perfectly.
///
/// Because every partial is an exact harmonic of `1 / length`, each one is back
/// where it started at the end of the buffer, so the loop point is silent.
pub fn harmonics(length: usize, partials: &[Partial]) -> Vec<f32> {
    let mut out = vec![0.0; length];
    for partial in partials {
        if partial.amplitude.abs() < 1e-5 {
            continue;
        }
        let step = std::f32::consts::TAU * partial.harmonic as f32 / length as f32;
        let phase = std::f32::consts::TAU * partial.phase;
        for (index, sample) in out.iter_mut().enumerate() {
            *sample += partial.amplitude * (step * index as f32 + phase).sin();
        }
    }
    out
}

/// Folds a buffer's surplus tail back over its head so it loops without a click.
///
/// `samples` must be `fade` longer than the loop you want; the result is that
/// shorter. The last sample of the result and the first are adjacent in the
/// original buffer, so the join is continuous by construction.
pub fn wrap_seam(mut samples: Vec<f32>, fade: usize) -> Vec<f32> {
    let length = samples.len().saturating_sub(fade);
    if length == 0 || fade == 0 {
        return samples;
    }
    for index in 0..fade {
        let blend = index as f32 / fade as f32;
        samples[index] = samples[index] * blend + samples[length + index] * (1.0 - blend);
    }
    samples.truncate(length);
    samples
}

/// Ramps the first and last few milliseconds to zero, so starting or stopping
/// a one-shot does not click.
pub fn fade_edges(samples: &mut [f32], seconds: f32) {
    let edge = self::samples(seconds).min(samples.len() / 2);
    if edge == 0 {
        return;
    }
    let length = samples.len();
    for index in 0..edge {
        let blend = index as f32 / edge as f32;
        samples[index] *= blend;
        samples[length - 1 - index] *= blend;
    }
}

/// Scales a buffer so its loudest sample sits at `peak`.
///
/// Every sound is written at whatever amplitude its synthesis happened to
/// produce; normalising here means the mix is set by one number per sound in
/// the bank instead of by accident.
pub fn normalize(samples: &mut [f32], peak: f32) {
    let loudest = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if loudest <= f32::EPSILON {
        return;
    }
    let scale = peak / loudest;
    for sample in samples {
        *sample *= scale;
    }
}

/// White noise in -1..1, from one of the game's deterministic streams.
pub fn white(rng: &mut ChaCha8Rng) -> f32 {
    rng.random::<f32>() * 2.0 - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::rng::{stream, stream_for};

    fn decode(sound: &SynthSound) -> Vec<f32> {
        sound.decoder().collect()
    }

    #[test]
    fn a_decoder_replays_the_whole_buffer_and_then_reports_empty() {
        let sound = SynthSound::new(vec![0.25, -0.5, 0.75]);
        assert_eq!(decode(&sound), vec![0.25, -0.5, 0.75]);

        let mut decoder = sound.decoder();
        assert_ne!(decoder.current_span_len(), Some(0));
        for _ in 0..3 {
            decoder.next();
        }
        // Rodio uses this to decide the sink is done; get it wrong and one-shot
        // sounds never despawn.
        assert_eq!(decoder.current_span_len(), Some(0));
        assert_eq!(decoder.next(), None);
    }

    #[test]
    fn two_decoders_of_one_sound_do_not_copy_the_samples() {
        let sound = SynthSound::new(vec![0.0; 1024]);
        let first = sound.decoder();
        let second = sound.decoder();
        assert!(
            Arc::ptr_eq(&first.samples, &second.samples),
            "every sink playing a siren would otherwise clone a second of audio"
        );
    }

    #[test]
    fn harmonic_loops_close_on_themselves() {
        let length = 512;
        let loop_buffer = harmonics(
            length,
            &[
                Partial {
                    harmonic: 3,
                    amplitude: 1.0,
                    phase: 0.0,
                },
                Partial {
                    harmonic: 7,
                    amplitude: 0.5,
                    phase: 0.31,
                },
                Partial {
                    harmonic: 22,
                    amplitude: 0.2,
                    phase: 0.77,
                },
            ],
        );
        // The step from the last sample back to the first must be no larger
        // than any step inside the buffer, or the loop ticks once a cycle.
        let seam = (loop_buffer[0] - loop_buffer[length - 1]).abs();
        let worst = loop_buffer
            .windows(2)
            .fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(seam <= worst + 1e-5, "seam {seam} vs worst step {worst}");
    }

    #[test]
    fn wrapping_joins_a_noise_loop_at_an_adjacent_pair() {
        let mut rng = stream_for(1, stream::AUDIO);
        let mut filter = LowPass::new(400.0);
        // Warm the filter up: from a zeroed state the first samples are
        // near-silent, which would flatter every measurement here.
        for _ in 0..2_000 {
            filter.process(white(&mut rng));
        }
        let raw: Vec<f32> = (0..4_096)
            .map(|_| filter.process(white(&mut rng)))
            .collect();

        let wrapped = wrap_seam(raw.clone(), 1_024);
        assert_eq!(wrapped.len(), 3_072);

        // The guarantee: the loop's last sample and its first are neighbours in
        // the source, so playing round the join is playing the source forwards.
        assert_eq!(wrapped[wrapped.len() - 1], raw[3_071]);
        assert_eq!(wrapped[0], raw[3_072]);

        let worst_step = wrapped
            .windows(2)
            .fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()));
        let seam = (wrapped[0] - wrapped[wrapped.len() - 1]).abs();
        assert!(
            seam <= worst_step,
            "the join should be an ordinary step: {seam} vs {worst_step}"
        );
    }

    #[test]
    fn a_percussive_envelope_peaks_then_halves_on_schedule() {
        assert_eq!(hit(0.0, 0.01, 0.1), 0.0);
        assert_eq!(hit(0.01, 0.01, 0.1), 1.0);
        assert!((hit(0.11, 0.01, 0.1) - 0.5).abs() < 1e-5);
        assert!((hit(0.21, 0.01, 0.1) - 0.25).abs() < 1e-5);
    }

    #[test]
    fn normalising_lands_exactly_on_the_requested_peak() {
        let mut buffer = vec![0.1, -0.4, 0.2];
        normalize(&mut buffer, 0.8);
        assert!((buffer.iter().fold(0.0f32, |m, s| m.max(s.abs())) - 0.8).abs() < 1e-6);

        // Silence must not turn into a division by zero.
        let mut silent = vec![0.0; 4];
        normalize(&mut silent, 0.8);
        assert!(silent.iter().all(|s| *s == 0.0));
    }

    #[test]
    fn a_resonator_rings_at_the_frequency_it_was_tuned_to() {
        let mut resonator = Resonator::new(1000.0, 40.0);
        // One impulse in; count zero crossings in the ring that comes out.
        let mut previous = 0.0;
        let mut crossings = 0;
        for index in 0..SAMPLE_RATE as usize {
            let out = resonator.process(if index == 0 { 1.0 } else { 0.0 });
            if previous < 0.0 && out >= 0.0 {
                crossings += 1;
            }
            previous = out;
        }
        // A second of a 1kHz ring is about a thousand cycles, less whatever
        // decays below the noise floor.
        assert!(
            (900..=1_100).contains(&crossings),
            "expected roughly a kilohertz, counted {crossings}"
        );
    }
}
