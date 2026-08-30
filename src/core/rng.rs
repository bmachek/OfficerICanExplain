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
}

const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// An independent, reproducible stream for one subsystem.
pub fn stream_for(seed: u64, key: u64) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(seed ^ key.wrapping_mul(GOLDEN))
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
