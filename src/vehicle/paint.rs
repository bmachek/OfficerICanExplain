//! Textures for the parts of a car that are not painted metal.
//!
//! Bodywork needs no texture — it is a flat colour under a clearcoat, which is
//! what car paint actually is. Wheels are the opposite: a tyre without a tread
//! and a wheel without spokes read as two black discs, and they are the part of
//! the car nearest the camera whenever it is moving.
//!
//! Both are drawn rather than modelled. Spokes as geometry would be a few
//! hundred triangles per wheel, four wheels per car and thirty cars on screen,
//! to resolve detail that is a blur above walking pace. A face texture with a
//! normal map holds up to the distance a wheel is ever actually inspected from.

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

use crate::world::texture::{byte, fbm, hash01, normal_map, painted, smoothstep01};

const WHEEL_SIZE: u32 = 256;

/// What the traffic is painted.
///
/// Weighted the way a real street is: mostly white, silver, grey and black,
/// with the occasional colour. An evenly sampled rainbow of cars reads as a toy
/// box, and it is the *proportion* of dull ones that makes the red one feel
/// like a choice somebody made.
const PALETTE: [(Color, f32, u32); 12] = [
    (Color::srgb(0.86, 0.87, 0.88), 0.15, 5), // white
    (Color::srgb(0.62, 0.64, 0.67), 0.75, 5), // silver
    (Color::srgb(0.30, 0.32, 0.35), 0.60, 4), // gunmetal
    (Color::srgb(0.07, 0.07, 0.08), 0.30, 4), // black
    (Color::srgb(0.44, 0.13, 0.13), 0.55, 2), // maroon
    (Color::srgb(0.72, 0.18, 0.15), 0.35, 2), // red
    (Color::srgb(0.13, 0.26, 0.42), 0.65, 2), // navy
    (Color::srgb(0.20, 0.40, 0.34), 0.60, 1), // British racing green
    (Color::srgb(0.78, 0.62, 0.20), 0.50, 1), // ochre
    (Color::srgb(0.55, 0.42, 0.32), 0.30, 1), // beige
    (Color::srgb(0.24, 0.44, 0.62), 0.55, 1), // pale blue
    (Color::srgb(0.86, 0.44, 0.12), 0.45, 1), // orange
];

/// Picks a colour and finish for one car off the street.
pub fn street_paint(rng: &mut ChaCha8Rng) -> (Color, f32) {
    let total: u32 = PALETTE.iter().map(|(_, _, weight)| weight).sum();
    let mut ticket = rng.random_range(0..total);
    for (color, metallic, weight) in PALETTE {
        if ticket < weight {
            return (color, metallic);
        }
        ticket -= weight;
    }
    let (color, metallic, _) = PALETTE[0];
    (color, metallic)
}

/// Spokes on a wheel face. Also the number of tread blocks across a tyre.
const SPOKES: u32 = 5;

/// Distance from the wheel centre, and angle around it, for a texel.
///
/// The face textures are drawn in polar coordinates because everything on a
/// wheel is: spokes radiate, the rim is a ring, the hub is a disc.
fn polar(u: f32, v: f32) -> (f32, f32) {
    let (x, y) = (u * 2.0 - 1.0, v * 2.0 - 1.0);
    (x.hypot(y), y.atan2(x))
}

/// How much of a wheel face is spoke rather than gap, at a given radius.
///
/// Spokes taper: wide at the hub, narrow at the rim, which is what stops five
/// bars from reading as a pie chart.
fn spoke_mask(radius: f32, angle: f32) -> f32 {
    let spacing = std::f32::consts::TAU / SPOKES as f32;
    // Distance to the nearest spoke centreline, in radians. Spokes sit on
    // multiples of the spacing, so angle zero is a spoke and half a spacing
    // along is the gap between two.
    let along = angle.rem_euclid(spacing);
    let offset = along.min(spacing - along);
    let half_width = (0.34 - radius * 0.16).max(0.05);
    smoothstep01((half_width - offset) / 0.04)
}

/// Height field the wheel face's colour and normal map are both built from.
fn rim_height(u: f32, v: f32) -> f32 {
    let (radius, angle) = polar(u, v);

    // Outer lip, spoke web, then the hub cap in the middle.
    let lip = smoothstep01((radius - 0.86) / 0.05) * smoothstep01((1.02 - radius) / 0.04);
    let hub = smoothstep01((0.24 - radius) / 0.05);
    let web = spoke_mask(radius, angle) * smoothstep01((0.88 - radius) / 0.06);

    // The gaps between spokes are the brake and the dark behind it.
    0.18 + lip.max(hub).max(web) * 0.78
}

/// A wheel face: lip, spokes, hub, and the dark between them.
pub fn rim() -> Image {
    painted(WHEEL_SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        let (radius, _) = polar(u, v);
        let height = rim_height(u, v);

        // Brake dust and road film collect towards the outside.
        let grime = fbm(u, v, 24, 3, 91) * 0.10 * radius;
        let value = (0.20 + height * 0.72 - grime).clamp(0.0, 1.0);

        [byte(value), byte(value * 0.995), byte(value * 0.985), 255]
    })
}

pub fn rim_normal() -> Image {
    normal_map(WHEEL_SIZE, 0.09, rim_height)
}

/// Metalness and roughness for a wheel face, packed the way glTF packs it.
///
/// The spokes are bare machined alloy and the gaps behind them are not, so this
/// is what stops the whole wheel reading as one lump of chrome.
pub fn rim_surface() -> Image {
    painted(WHEEL_SIZE, TextureFormat::Rgba8Unorm, |u, v| {
        let metal = rim_height(u, v);
        let rough = 0.78 - metal * 0.5;
        [0, byte(rough), byte(smoothstep01((metal - 0.5) / 0.2)), 255]
    })
}

/// Tread depth across the tyre. `u` runs around the circumference, `v` across
/// the width, matching the wheel mesh's UVs.
fn tread_height(u: f32, v: f32) -> f32 {
    // Shoulders are the smooth sidewall; the tread is the middle.
    let crown = smoothstep01((v - 0.16) / 0.08) * smoothstep01((0.84 - v) / 0.08);

    // Blocks in two rows, offset, cut by a circumferential groove.
    let row = if v < 0.5 { 0.0 } else { 0.5 };
    let block = ((u * 14.0 + row).fract() - 0.5).abs();
    let groove = (v - 0.5).abs();
    let cut = smoothstep01((block - 0.18) / 0.06) * smoothstep01((groove - 0.045) / 0.02);

    let sidewall = 0.55 + fbm(u, v, 40, 3, 17) * 0.10;
    sidewall * (1.0 - crown) + (0.25 + cut * 0.75) * crown
}

pub fn tyre() -> Image {
    painted(WHEEL_SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        // Rubber is very dark, and a tread that shows up as *lighter* rubber
        // reads as a painted-on pattern. What actually shows is the shadow in
        // the grooves, so the range here is deliberately tiny.
        let value = 0.055 + tread_height(u, v) * 0.055;
        let dust = hash01(
            (u * WHEEL_SIZE as f32) as u32,
            (v * WHEEL_SIZE as f32) as u32,
            5,
        );
        let speck = if dust > 0.994 { 0.05 } else { 0.0 };
        let c = byte(value + speck);
        [c, c, byte(value * 1.04 + speck), 255]
    })
}

pub fn tyre_normal() -> Image {
    normal_map(WHEEL_SIZE, 0.07, tread_height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::rng::{stream, stream_for};

    #[test]
    fn the_palette_is_weighted_towards_dull_colours() {
        let mut rng = stream_for(9, stream::VEHICLE_SPAWNS);
        let mut plain = 0;
        for _ in 0..600 {
            let (color, _) = street_paint(&mut rng);
            // Measured in sRGB, not linear: the linear curve stretches the
            // gap between a silver's channels far more than it stretches a
            // black's, so one threshold cannot cover both.
            let c = color.to_srgba();
            let channels = [c.red, c.green, c.blue];
            let max = channels.iter().cloned().fold(f32::MIN, f32::max);
            let min = channels.iter().cloned().fold(f32::MAX, f32::min);
            if max - min < 0.06 {
                plain += 1;
            }
        }
        assert!(
            (330..=430).contains(&plain),
            "expected roughly two thirds neutral cars, got {plain} of 600"
        );
    }

    #[test]
    fn every_ticket_in_the_palette_draws_something() {
        // An off-by-one in the weighted pick silently makes the last colour
        // unreachable, which is invisible until somebody counts.
        let mut seen = std::collections::HashSet::new();
        let mut rng = stream_for(3, stream::VEHICLE_SPAWNS);
        for _ in 0..4000 {
            let (color, _) = street_paint(&mut rng);
            seen.insert(format!("{:?}", color.to_srgba()));
        }
        assert_eq!(seen.len(), PALETTE.len(), "some colour never comes up");
    }

    #[test]
    fn spokes_alternate_with_gaps_around_the_wheel() {
        // Sampled on a circle through the spoke web, the mask has to come up
        // and go down again exactly once per spoke.
        let radius = 0.55;
        let mut crossings = 0;
        // Seeded from the starting angle, which lands on a spoke: assuming a
        // gap there would score the very first sample as an edge.
        let mut lit = spoke_mask(radius, 0.0) > 0.5;
        for step in 0..720 {
            let angle = std::f32::consts::TAU * step as f32 / 720.0;
            let solid = spoke_mask(radius, angle) > 0.5;
            if solid != lit {
                crossings += 1;
                lit = solid;
            }
        }
        assert_eq!(
            crossings,
            SPOKES * 2,
            "expected {SPOKES} spokes, each with a leading and trailing edge"
        );
    }

    #[test]
    fn the_hub_and_the_lip_are_solid_and_the_web_between_is_not() {
        // Straight up from the centre, between two spokes.
        let between = std::f32::consts::TAU / SPOKES as f32 * 0.5;
        let at = |radius: f32| {
            let (x, y) = (radius * between.cos(), radius * between.sin());
            rim_height((x + 1.0) * 0.5, (y + 1.0) * 0.5)
        };
        assert!(at(0.05) > 0.8, "the hub cap should be solid");
        assert!(at(0.95) > 0.8, "so should the outer lip");
        assert!(at(0.55) < 0.4, "and the gap between spokes should not be");
    }

    #[test]
    fn a_tyre_is_grooved_across_the_crown_and_smooth_at_the_shoulder() {
        let crown: Vec<f32> = (0..200)
            .map(|i| tread_height(i as f32 / 200.0, 0.3))
            .collect();
        let spread = crown.iter().cloned().fold(f32::MIN, f32::max)
            - crown.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 0.4, "the tread should have real depth to it");

        let shoulder: Vec<f32> = (0..200)
            .map(|i| tread_height(i as f32 / 200.0, 0.02))
            .collect();
        let shoulder_spread = shoulder.iter().cloned().fold(f32::MIN, f32::max)
            - shoulder.iter().cloned().fold(f32::MAX, f32::min);
        assert!(
            shoulder_spread < spread * 0.5,
            "the sidewall should be far smoother than the tread"
        );
    }

    #[test]
    fn rubber_stays_dark() {
        // Every texel, not just the average: one bright patch on a tyre reads
        // as a hole in it.
        let image = tyre();
        let data = image.data.as_ref().expect("pixels");
        let brightest = data
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| p[0])
            .max()
            .unwrap_or(0);
        assert!(brightest < 60, "tyre peaked at {brightest}, which is grey");
    }
}
