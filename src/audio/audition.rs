//! Hearing the bank without playing the game.
//!
//! `core::capture` exists because a rendering change cannot be judged from a
//! terminal. Sound has exactly the same problem and, until now, no answer: the
//! only way to hear a curse was to find a flummi in a bad enough mood to say
//! one, and the only way to compare two takes of a grumble was to wait for the
//! bank to pick each of them.
//!
//! ```sh
//! cargo run -- --audition shots/audio
//! ```
//!
//! writes every sound in the bank out as a WAV and exits, without starting
//! Bevy at all — synthesis has no dependency on a running app, which is what
//! makes this twenty lines rather than a second capture harness.
//!
//! WAV is written by hand rather than by a crate. The format needed here is
//! forty-four bytes of header in front of the samples, and adding a dependency
//! for that is worse than writing it.

use std::path::{Path, PathBuf};

use super::bank;
use super::synth::{SAMPLE_RATE, SynthSound};

/// The directory a run was asked to write the bank into, if it was.
pub fn requested() -> Option<PathBuf> {
    let args: Vec<String> = std::env::args().collect();
    let flag = args.iter().position(|a| a == "--audition")?;
    Some(PathBuf::from(
        args.get(flag + 1)
            .map(String::as_str)
            .unwrap_or("shots/audio"),
    ))
}

/// Writes every sound in the bank into `directory`, one WAV each.
pub fn write(directory: &Path) {
    if let Err(problem) = std::fs::create_dir_all(directory) {
        eprintln!("could not make {}: {problem}", directory.display());
        return;
    }

    let mut all = bank::every_one_shot();
    all.extend(bank::every_loop());
    let count = all.len();
    for (name, sound) in all {
        let path = directory.join(format!("{name}.wav"));
        let samples: Vec<f32> = sound_samples(&sound);
        if let Err(problem) = std::fs::write(&path, wav(&samples)) {
            eprintln!("could not write {}: {problem}", path.display());
            return;
        }
        println!(
            "{:>7.3}s  {}",
            sound.duration().as_secs_f32(),
            path.display()
        );
    }
    println!("{count} sounds written to {}", directory.display());
}

fn sound_samples(sound: &SynthSound) -> Vec<f32> {
    use bevy::audio::Decodable;
    sound.decoder().collect()
}

/// One mono 16-bit PCM WAV file.
fn wav(samples: &[f32]) -> Vec<u8> {
    const BITS: u16 = 16;
    let bytes_per_sample = BITS as u32 / 8;
    let data_len = samples.len() as u32 * bytes_per_sample;

    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");

    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * bytes_per_sample).to_le_bytes()); // bytes/sec
    out.extend_from_slice(&(bytes_per_sample as u16).to_le_bytes()); // block align
    out.extend_from_slice(&BITS.to_le_bytes());

    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        // Clamped before scaling: everything in the bank is normalised to at
        // most 1.0, but a sound being auditioned is often one that is not
        // finished, and wrapping round to full negative is a bang.
        let scaled = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        out.extend_from_slice(&scaled.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wav_header_describes_the_samples_behind_it() {
        let file = wav(&[0.0, 0.5, -0.5, 1.0]);
        assert_eq!(&file[0..4], b"RIFF");
        assert_eq!(&file[8..12], b"WAVE");
        assert_eq!(&file[36..40], b"data");
        assert_eq!(file.len(), 44 + 4 * 2, "four samples, two bytes each");
        assert_eq!(
            u32::from_le_bytes(file[4..8].try_into().unwrap()) as usize,
            file.len() - 8,
            "the RIFF size must count everything after itself"
        );
        assert_eq!(
            u32::from_le_bytes(file[40..44].try_into().unwrap()),
            8,
            "and the data size only the samples"
        );
    }

    #[test]
    fn a_sample_over_the_rail_clips_rather_than_wrapping() {
        // Casting 1.5 * i16::MAX straight to i16 in release is a saturating
        // cast in Rust, but the intent is worth pinning: an unfinished sound
        // that is too loud should sound too loud, not like a gunshot.
        let file = wav(&[1.5, -1.5]);
        let first = i16::from_le_bytes(file[44..46].try_into().unwrap());
        let second = i16::from_le_bytes(file[46..48].try_into().unwrap());
        assert_eq!(first, i16::MAX);
        assert_eq!(second, -i16::MAX);
    }
}
