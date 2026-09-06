//! Number plates.
//!
//! A car without one reads as a prop. It is a small thing and it is the kind of
//! small thing the eye checks without being asked, in the same way an empty
//! cabin behind the glass reads as wrong long before anybody works out why.
//!
//! The registration is drawn rather than modelled: seven glyphs on a strip is a
//! job for a texture, and a 5×7 cell is the smallest one that still reads as
//! letters instead of noise once it is a hundred and fifty millimetres wide on
//! a car three car-lengths away. Real plate fonts are drawn to be legible from
//! further off than that and are, without exception, somebody's property.
//!
//! There are [`REGISTRATIONS`] of them and they are shared city-wide: a plate is
//! one material and one quad, and a city has several hundred cars in it. Which
//! one a car wears comes from where it spawned, so a car keeps its plate across
//! a save and two cars parked nose to tail do not match.

use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

use crate::world::texture::painted_rect;

/// Glyphs, five wide and seven tall, most significant bit leftmost.
///
/// Indexed by [`glyph`]. Digits first so that `'0'..='9'` maps straight onto the
/// front of the table.
#[rustfmt::skip]
const FONT: [[u8; 7]; 36] = [
    [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110], // 0
    [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110], // 1
    [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111], // 2
    [0b11111, 0b00010, 0b00100, 0b00010, 0b00001, 0b10001, 0b01110], // 3
    [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010], // 4
    [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110], // 5
    [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110], // 6
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000], // 7
    [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110], // 8
    [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100], // 9
    [0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001], // A
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110], // B
    [0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110], // C
    [0b11100, 0b10010, 0b10001, 0b10001, 0b10001, 0b10010, 0b11100], // D
    [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111], // E
    [0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000], // F
    [0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111], // G
    [0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001], // H
    [0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110], // I
    [0b00111, 0b00010, 0b00010, 0b00010, 0b00010, 0b10010, 0b01100], // J
    [0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001], // K
    [0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111], // L
    [0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001], // M
    [0b10001, 0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001], // N
    [0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110], // O
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000], // P
    [0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101], // Q
    [0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001], // R
    [0b01111, 0b10000, 0b10000, 0b01110, 0b00001, 0b00001, 0b11110], // S
    [0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100], // T
    [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110], // U
    [0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100], // V
    [0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b11011, 0b10001], // W
    [0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001], // X
    [0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100], // Y
    [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111], // Z
];

/// The rows of one character, or a blank cell for anything not in the font.
fn glyph(character: u8) -> [u8; 7] {
    match character {
        b'0'..=b'9' => FONT[(character - b'0') as usize],
        b'A'..=b'Z' => FONT[(character - b'A') as usize + 10],
        _ => [0; 7],
    }
}

/// The registrations in circulation.
///
/// Invented, and deliberately in no real jurisdiction's format — a plate that
/// happens to be somebody's is a plate on a car committing crimes.
pub const REGISTRATIONS: [&str; 8] = [
    "4ZQK719", "8TFM244", "2HRV865", "6XDN037", "9BLP512", "3WGC680", "7KJS391", "5NMD428",
];

/// Characters across a plate, including the blanks either end.
const CELLS: usize = 9;
const TEXTURE_WIDTH: u32 = 256;
const TEXTURE_HEIGHT: u32 = 128;

/// How much of the plate's height one glyph takes.
const GLYPH_HEIGHT: f32 = 0.52;

/// Plate size in metres. A hair over twelve inches by six, which is the size
/// every plate in the western world is within a centimetre of.
pub const SIZE: Vec2 = Vec2::new(0.305, 0.152);

/// Which registration a car spawned at `at` wears.
///
/// From the position rather than from a counter or the spawn RNG, so a car
/// keeps its plate when the chunk it stands in is streamed out and back, and so
/// two cars parked nose to tail do not come out matching.
pub fn registration_for(at: Vec3) -> usize {
    // Centimetres, so two cars a hand's width apart land in different buckets,
    // and wrapped so that the far side of the map is not one long run.
    let mix = (at.x * 100.0) as i64 as u64
        ^ ((at.z * 100.0) as i64 as u64).rotate_left(31)
        ^ ((at.y * 100.0) as i64 as u64).rotate_left(17);
    // One round of a 64-bit mixer. Enough: the input is already spread out and
    // this only has to pick one of eight.
    let mut hash = mix.wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^= hash >> 33;
    (hash % REGISTRATIONS.len() as u64) as usize
}

/// Draws one registration onto a plate.
pub fn texture(registration: &str) -> Image {
    let text = registration.as_bytes();
    // Centred, whatever its length, so a six-character plate is not left-shifted.
    let lead = (CELLS - text.len().min(CELLS)) as f32 * 0.5;

    painted_rect(
        TEXTURE_WIDTH,
        TEXTURE_HEIGHT,
        TextureFormat::Rgba8UnormSrgb,
        |u, v| {
            // The reflective white, and a dark rim around it — a plate is
            // pressed, and the pressing is what you see of it at any distance
            // where the letters have stopped resolving.
            let border = u < 0.035 || u > 0.965 || v < 0.06 || v > 0.94;
            let field: [u8; 4] = if border {
                [26, 28, 34, 255]
            } else {
                [232, 231, 226, 255]
            };

            // Which cell, and where inside it.
            let column = u * CELLS as f32 - lead;
            let index = column.floor();
            if index < 0.0 || index >= text.len() as f32 {
                return field;
            }
            let rows = glyph(text[index as usize]);

            // A glyph fills a little over half the plate's height, sat on the
            // vertical middle; the cell is wider than the glyph so the letters
            // do not touch.
            let inside_x = (column - index - 0.14) / 0.72;
            let inside_y = (v - (0.5 - GLYPH_HEIGHT * 0.5)) / GLYPH_HEIGHT;
            if !(0.0..1.0).contains(&inside_x) || !(0.0..1.0).contains(&inside_y) {
                return field;
            }

            let bit = (inside_x * 5.0) as usize;
            let row = (inside_y * 7.0) as usize;
            if rows[row.min(6)] & (1 << (4 - bit.min(4))) != 0 {
                [22, 30, 74, 255]
            } else {
                field
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_glyph_is_drawn_and_none_of_them_overflow_the_cell() {
        // Five bits wide, so bit 5 and up must be clear — a stray bit there
        // does not fail visibly, it just bleeds a pixel into the next letter.
        for (index, rows) in FONT.iter().enumerate() {
            assert!(rows.iter().any(|&row| row != 0), "glyph {index} is blank");
            for (line, &row) in rows.iter().enumerate() {
                assert!(
                    row < 0b100000,
                    "glyph {index} row {line} is wider than five cells"
                );
            }
        }
    }

    #[test]
    fn the_font_covers_everything_a_registration_can_contain() {
        for registration in REGISTRATIONS {
            assert!(
                registration.len() <= CELLS - 2,
                "{registration} will not fit on a plate"
            );
            for character in registration.bytes() {
                assert!(
                    glyph(character) != [0; 7],
                    "{registration} needs a '{}' and the font has none",
                    character as char
                );
            }
        }
    }

    #[test]
    fn two_cars_parked_nose_to_tail_do_not_wear_the_same_plate() {
        // The failure this exists for is a hash that ignores the low bits, so
        // that a whole street of parked cars comes out with one registration —
        // which is worse than no plates at all, because it reads as a bug
        // rather than as an omission.
        let along: Vec<usize> = (0..24)
            .map(|i| registration_for(Vec3::new(14.0, 0.6, i as f32 * 5.4)))
            .collect();
        let distinct: std::collections::HashSet<_> = along.iter().collect();
        assert!(
            distinct.len() >= REGISTRATIONS.len() * 3 / 4,
            "24 cars down one street wore only {} of {} plates",
            distinct.len(),
            REGISTRATIONS.len()
        );

        // And it has to be stable, or a car changes identity every time its
        // chunk is streamed back in.
        let at = Vec3::new(-183.25, 0.61, 92.5);
        assert_eq!(registration_for(at), registration_for(at));
    }

    #[test]
    fn a_plate_is_mostly_white_with_dark_letters_on_it() {
        // The one thing that would silently ruin this is a coordinate mistake
        // that draws the whole field in glyph colour, or none of it — both look
        // like a solid rectangle from any distance, and both build.
        let image = texture(REGISTRATIONS[0]);
        let data = image.data.as_ref().expect("the plate was not painted");
        let dark = data
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[0] < 128 && pixel[2] < 128)
            .count() as f32
            / (TEXTURE_WIDTH * TEXTURE_HEIGHT) as f32;
        assert!(
            (0.10..0.45).contains(&dark),
            "{dark:.2} of the plate is dark; it is a rectangle, not a registration"
        );
    }
}
