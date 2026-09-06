//! Deterministic random streams.
//!
//! City generation must be reproducible: the same seed has to rebuild the exact
//! same city, because chunks are regenerated on demand rather than stored.
//!
//! The trap with a single shared RNG is that *draw order* becomes load-bearing —
//! adding one call in the building generator would silently reshuffle every
//! street downstream. So each subsystem derives its own independent stream from
//! (seed, key) and is free to draw as much as it likes.

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

/// Fixed keys for each independent generation stream. Never reuse a value.
pub mod stream {
    pub const ROADS: u64 = 1;
    pub const BLOCKS: u64 = 2;
    pub const BUILDINGS: u64 = 3;
    pub const PROPS: u64 = 4;
    pub const VEHICLE_SPAWNS: u64 = 5;
    pub const PEDESTRIANS: u64 = 6;
    /// Waveform synthesis. Not world state, but the same reproducibility
    /// argument applies: a sound should not change between runs.
    pub const AUDIO: u64 = 7;
    pub const RAIN: u64 = 8;
    /// Cloud cover and wind. Sampled rather than drawn — see `key_for`.
    pub const WEATHER: u64 = 9;
    /// Street trees and park planting. Its own stream rather than sharing the
    /// props one, so that planting a tree cannot shift which bin lands where.
    pub const VEGETATION: u64 = 10;
    /// Manholes, patches, stains and rubber. Separate again, for the same
    /// reason: a street's wear must not depend on how many trees are on it.
    pub const WEAR: u64 = 11;
    /// Temperaments. Its own stream for the usual reason, sharpened: the crowd
    /// is drawn from `PEDESTRIANS`, so taking their tempers from the same
    /// stream would mean that giving somebody a shorter fuse also moves where
    /// the next pedestrian spawns and which street they walk down.
    pub const MOOD: u64 = 12;
}

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// An independent, reproducible stream for one subsystem.
pub fn stream_for(seed: u64, key: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(key_for(seed, key))
}

/// The same derivation, stopping one step short of an RNG.
///
/// Some subsystems do not *draw* randomness, they *sample* it: value noise over
/// a clock needs a fixed key it can hash a position against, and building a
/// ChaCha state per sample would be absurd. They still want the stream keys to
/// stay independent, so the mixing is shared rather than reinvented.
pub fn key_for(seed: u64, key: u64) -> u64 {
    seed ^ key.wrapping_mul(GOLDEN)
}

/// A stream for one chunk of one subsystem, so chunks regenerate identically
/// regardless of the order the player visits them in.
pub fn stream_for_chunk(seed: u64, key: u64, chunk: (i32, i32)) -> ChaCha8Rng {
    let c = (chunk.0 as u64) << 32 | (chunk.1 as u32 as u64);
    ChaCha8Rng::seed_from_u64(seed ^ key.wrapping_mul(GOLDEN) ^ c.wrapping_mul(GOLDEN))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::RngExt;

    #[test]
    fn same_seed_same_sequence() {
        let a: Vec<u32> = (0..8)
            .map(|_| stream_for(42, stream::ROADS).random::<u32>())
            .collect();
        let b: Vec<u32> = (0..8)
            .map(|_| stream_for(42, stream::ROADS).random::<u32>())
            .collect();
        assert_eq!(a, b);
    }

    #[test]
    fn different_streams_diverge() {
        let roads = stream_for(42, stream::ROADS).random::<u64>();
        let blocks = stream_for(42, stream::BLOCKS).random::<u64>();
        assert_ne!(roads, blocks, "streams must be independent");
    }

    #[test]
    fn chunks_are_order_independent() {
        let first = stream_for_chunk(7, stream::BUILDINGS, (3, -2)).random::<u64>();
        let second = stream_for_chunk(7, stream::BUILDINGS, (3, -2)).random::<u64>();
        assert_eq!(first, second);
        assert_ne!(
            first,
            stream_for_chunk(7, stream::BUILDINGS, (-2, 3)).random::<u64>(),
            "chunk coords must not collide under swap"
        );
    }
}
