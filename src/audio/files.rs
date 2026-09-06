//! Recorded sounds, as an optional upgrade over the synthesised bank.
//!
//! The same bargain the scanned PBR materials make (`world::material`): a
//! fresh clone synthesises every sound and runs identically, and
//! `tools/fetch-materials.sh` optionally drops CC0 recordings into
//! `assets/sounds/`, one file per bank name. Anything found there is
//! preferred; anything missing falls back to synthesis, per sound. Nothing is
//! ever *required* to be on disk.
//!
//! Every loaded file is pushed through the same discipline the synthesised
//! bank is tested to — mixed to mono, resampled to the bank's rate, edges
//! faded, peak normalised, loops seam-wrapped — so a recording obeys the
//! rules (no clicks, no clipping, honest loudness) by construction rather
//! than by trusting whoever uploaded it.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use rodio::Source;

use super::synth::{SAMPLE_RATE, SynthSound, fade_edges, normalize, samples, wrap_seam};

/// The directory the fetch script fills: `assets/sounds/`.
pub fn dir() -> PathBuf {
    crate::core::assets::root().join("sounds")
}

/// Container formats the fetch script is allowed to deliver, in the order
/// they are looked for.
const EXTENSIONS: [&str; 4] = ["wav", "flac", "ogg", "mp3"];

/// Decodes `<dir>/<name>.<ext>` for the first extension that exists, into
/// mono samples at the bank's rate. `None` when no file is there or the file
/// does not decode — the caller synthesises instead, and a corrupt download
/// must never stop the game from starting.
fn decode(dir: &Path, name: &str) -> Option<Vec<f32>> {
    let Some(path) = EXTENSIONS
        .iter()
        .map(|ext| dir.join(format!("{name}.{ext}")))
        .find(|path| path.is_file())
    else {
        // Absence is fine — the fetch script is optional — but *silent*
        // absence made the synthesised fallbacks read as missing files. One
        // line per absent sound at startup makes the gap legible in the log
        // next to the "using recorded" lines the loaded ones print.
        info!("no recording for {name} in assets/sounds/; synthesising");
        return None;
    };

    let file = std::fs::File::open(&path).ok()?;
    let decoder = match rodio::Decoder::try_from(file) {
        Ok(decoder) => decoder,
        Err(error) => {
            warn!(
                "{} does not decode ({error}); synthesising instead",
                path.display()
            );
            return None;
        }
    };

    let rate = decoder.sample_rate().get();
    let channels = decoder.channels().get() as usize;
    let interleaved: Vec<f32> = decoder.collect();
    if interleaved.is_empty() {
        return None;
    }

    // Mix to mono: spatial panning is the mixer's job, and a stereo source
    // cannot be positioned in the world (same rule as `synth`).
    let mono: Vec<f32> = interleaved
        .chunks_exact(channels)
        .map(|frame| frame.iter().sum::<f32>() / channels as f32)
        .collect();

    Some(resample(&mono, rate, SAMPLE_RATE))
}

/// Linear resampling. Crude next to a windowed sinc, and the difference is
/// inaudible under a city: these are sound effects, not a mastering chain.
fn resample(input: &[f32], from_hz: u32, to_hz: u32) -> Vec<f32> {
    if from_hz == to_hz || input.is_empty() {
        return input.to_vec();
    }
    let ratio = from_hz as f64 / to_hz as f64;
    // Rounded rather than floored: 480 samples at 48k is exactly 441 at
    // 44.1k, and floating point must not be allowed to make it 440.
    let length = (input.len() as f64 / ratio).round() as usize;
    (0..length)
        .map(|index| {
            let at = index as f64 * ratio;
            let whole = at as usize;
            let fraction = (at - whole as f64) as f32;
            let a = input[whole.min(input.len() - 1)];
            let b = input[(whole + 1).min(input.len() - 1)];
            a + (b - a) * fraction
        })
        .collect()
}

/// A one-shot longer than this is an ambience somebody misfiled.
const ONE_SHOT_CAP: f32 = 6.0;
/// Loops are capped too: some CC0 field recordings run for minutes, and a
/// minutes-long buffer is tens of megabytes of RAM buying nothing — a bed
/// repeats unnoticed well under half a minute.
const LOOP_CAP: f32 = 24.0;

/// A recorded one-shot, held to the bank's one-shot rules: starts and ends in
/// silence, peaks exactly at `peak`.
pub fn one_shot(dir: &Path, name: &str, peak: f32) -> Option<SynthSound> {
    let mut out = decode(dir, name)?;
    out.truncate(samples(ONE_SHOT_CAP));
    fade_edges(&mut out, 0.004);
    normalize(&mut out, peak);
    info!("using recorded {name} from assets/sounds/");
    Some(SynthSound::new(out))
}

/// One recorded take of a many-voiced one-shot: `<name>-<take>.<ext>`, held
/// to the same rules as `one_shot`. The fallback stays per *take*, so a bank
/// entry with three takes can run with one recording and two synthesised
/// siblings until the other files turn up.
pub fn one_shot_take(dir: &Path, name: &str, take: usize, peak: f32) -> Option<SynthSound> {
    one_shot(dir, &format!("{name}-{take}"), peak)
}

/// A recorded loop, seam-wrapped so it repeats without a click. The recording
/// loses its last tenth of a second to the crossfade, which no loop misses.
pub fn looping(dir: &Path, name: &str, peak: f32) -> Option<SynthSound> {
    let mut out = decode(dir, name)?;
    out.truncate(samples(LOOP_CAP));
    let fade = samples(0.1);
    if out.len() <= fade * 2 {
        warn!("{name} is too short to loop; synthesising instead");
        return None;
    }
    let mut out = wrap_seam(out, fade);
    normalize(&mut out, peak);
    info!("using recorded {name} from assets/sounds/");
    Some(SynthSound::new(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_preserves_duration_and_endpoints() {
        let input: Vec<f32> = (0..480).map(|i| (i as f32 / 480.0).sin()).collect();
        let out = resample(&input, 48_000, 44_100);
        // 480 samples at 48k is 10ms; 10ms at 44.1k is 441 samples.
        assert_eq!(out.len(), 441);
        assert_eq!(out[0], input[0]);
    }

    #[test]
    fn resampling_at_the_same_rate_is_a_copy() {
        let input = vec![0.1, -0.2, 0.3];
        assert_eq!(resample(&input, 44_100, 44_100), input);
    }

    #[test]
    fn a_missing_file_is_a_quiet_none_rather_than_a_panic() {
        let nowhere = Path::new("/definitely/not/a/directory");
        assert!(one_shot(nowhere, "boing", 0.8).is_none());
        assert!(looping(nowhere, "engine", 0.8).is_none());
        assert!(one_shot_take(nowhere, "whistle", 0, 0.55).is_none());
    }
}
