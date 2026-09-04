//! The paint on the road.
//!
//! Markings are laid as flat quads over the asphalt rather than drawn into it.
//! The road surface is one plane two kilometres across — there is nowhere in
//! its texture to put a line that has to follow a particular street, and the
//! street layout is not known when that texture is painted.
//!
//! Each street gets a centre line and each junction approach a crossing, and
//! both come and go with the chunk they sit in, exactly like the buildings.
//! They share one mesh and one material apiece, so the whole road network's
//! paint is two draw calls.
//!
//! Everything sits a centimetre and a half above the road. That is not
//! z-fighting margin — the depth buffer would cope with far less — it is so
//! the line stays visible from a driver's eye height, where the asphalt's own
//! normal-mapped grain would otherwise chew into it at grazing angles.

use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

use super::buildings::{ChunkOf, with_tangents};
use super::roadgraph::RoadEdge;
use super::texture::{byte, fbm, painted};

/// How high the paint floats above the asphalt.
const PAINT_HEIGHT: f32 = 0.015;
/// Width of a centre line, in metres.
const LINE_WIDTH: f32 = 0.16;
/// Metres of road per dash, gap included.
const DASH_PERIOD: f32 = 9.0;
/// Metres of that period which are actually painted.
const DASH_LENGTH: f32 = 3.2;
/// Depth of a crossing measured along the road, in metres.
const CROSSING_DEPTH: f32 = 2.6;
/// How far back from a junction the crossing sits, as a fraction of the
/// street's half-width.
const CROSSING_SETBACK: f32 = 1.35;
/// Streets shorter than this get no crossings; there is no room between them.
const MIN_CROSSING_LENGTH: f32 = 26.0;

const PAINT_SIZE: u32 = 256;

#[derive(Resource)]
pub struct MarkingAssets {
    quad: Handle<Mesh>,
    centre_line: Handle<StandardMaterial>,
    crossing: Handle<StandardMaterial>,
}

/// Road paint is worn, not printed: tyres polish it thin down the middle of a
/// lane and grit fills the low spots. A line at a flat 100% white reads as a
/// decal laid on the world rather than as paint that has been driven over.
fn wear(u: f32, v: f32, seed: u32) -> f32 {
    (0.72 + fbm(u, v, 26, 3, seed) * 0.45).clamp(0.0, 1.0)
}

/// One dash of a centre line.
///
/// A dash rather than a dash *pattern*, because the pattern cannot live in the
/// texture: every street is a different length, and a texture stretched to fit
/// one would give a fifty-metre street the same number of dashes as a
/// hundred-metre one. So the spacing is done by placing quads, and each quad
/// holds a single mark.
fn centre_line_texture() -> Image {
    painted(PAINT_SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        // Soft along both axes: a hard end aliases into a row of dots at
        // distance, which is exactly where a dashed line is read from.
        let across = (1.0 - (u * 2.0 - 1.0).abs()) / 0.20;
        let ends = (1.0 - (v * 2.0 - 1.0).abs()) / 0.06;
        let edge = across.clamp(0.0, 1.0) * ends.clamp(0.0, 1.0);

        let value = wear(u, v, 11) * edge;
        [
            byte(value),
            byte(value * 0.985),
            byte(value * 0.93),
            byte(value),
        ]
    })
}

/// A zebra crossing: bars running along the road, spaced across it.
fn crossing_texture() -> Image {
    painted(PAINT_SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        // `u` runs across the road; the bars are spaced along it.
        let bar = (u * 8.0).fract();
        let inside = bar > 0.18 && bar < 0.82;
        // Ends of the bars are square, but taper the very edge so the outer
        // bar does not end in a hard line against the kerb.
        let along = 1.0 - (v * 2.0 - 1.0).abs();
        let edge = (along / 0.08).clamp(0.0, 1.0);

        let value = if inside { wear(u, v, 29) * edge } else { 0.0 };
        [
            byte(value),
            byte(value * 0.99),
            byte(value * 0.95),
            byte(value),
        ]
    })
}

fn paint_material(texture: Handle<Image>) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(texture),
        // Cut out rather than blended: road paint has no partial transparency,
        // and a mask keeps the quads out of the transparent queue where they
        // would have to be sorted against each other every frame.
        alpha_mode: AlphaMode::Mask(0.35),
        perceptual_roughness: 0.72,
        ..default()
    }
}

pub fn build_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> MarkingAssets {
    MarkingAssets {
        quad: meshes.add(with_tangents(
            Plane3d::default().mesh().size(1.0, 1.0).build(),
        )),
        centre_line: materials.add(paint_material(images.add(centre_line_texture()))),
        crossing: materials.add(paint_material(images.add(crossing_texture()))),
    }
}

/// Yaw that lays a quad's local +Z along an XZ direction.
fn along(direction: Vec2) -> f32 {
    direction.x.atan2(direction.y)
}

/// Paints one street: a centre line down it, and a crossing at each end.
pub fn spawn_edge(
    commands: &mut Commands,
    assets: &MarkingAssets,
    edge: &RoadEdge,
    from: Vec2,
    to: Vec2,
    chunk: IVec2,
) {
    let Ok(direction) = Dir2::new(to - from) else {
        return;
    };
    let yaw = along(*direction);

    // One quad per dash. Spacing a pattern by placing geometry rather than by
    // tiling a texture is what keeps the dashes the same length on a short
    // street and a long one; they all share the mesh and material, so this is
    // more entities but not more draw calls.
    let dashes = dash_repeats(edge.length);
    let period = edge.length / dashes as f32;
    for i in 0..dashes {
        let at = from + *direction * (period * (i as f32 + 0.5));
        commands.spawn((
            ChunkOf(chunk),
            Mesh3d(assets.quad.clone()),
            MeshMaterial3d(assets.centre_line.clone()),
            Transform::from_xyz(at.x, PAINT_HEIGHT, at.y)
                .with_rotation(Quat::from_rotation_y(yaw))
                .with_scale(Vec3::new(LINE_WIDTH, 1.0, DASH_LENGTH.min(period * 0.6))),
        ));
    }

    if edge.length < MIN_CROSSING_LENGTH {
        return;
    }
    // One at each end, set back far enough to clear the junction itself.
    let setback = edge.width * CROSSING_SETBACK;
    for end in [from + *direction * setback, to - *direction * setback] {
        commands.spawn((
            ChunkOf(chunk),
            Mesh3d(assets.quad.clone()),
            MeshMaterial3d(assets.crossing.clone()),
            Transform::from_xyz(end.x, PAINT_HEIGHT, end.y)
                .with_rotation(Quat::from_rotation_y(yaw))
                .with_scale(Vec3::new(edge.width * 0.92, 1.0, CROSSING_DEPTH)),
        ));
    }
}

/// How many dashes fit a street of this length.
///
/// Rounded rather than truncated, and never zero: the spacing is then stretched
/// or squeezed a little to fit the street exactly, so a line always begins and
/// ends the same distance from its junctions.
pub fn dash_repeats(length: f32) -> usize {
    ((length / DASH_PERIOD).round() as usize).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quad_laid_along_z_points_down_its_street() {
        // The quad's length runs along local Z, so the yaw has to map +Z onto
        // the street. Getting this backwards paints every line across the road
        // instead of along it, which is the kind of thing that looks
        // deliberate until you drive down it.
        let cases = [
            (Vec2::new(0.0, 1.0), Vec3::Z),
            (Vec2::new(1.0, 0.0), Vec3::X),
            (Vec2::new(-1.0, 0.0), Vec3::NEG_X),
        ];
        for (direction, expected) in cases {
            let laid = Quat::from_rotation_y(along(direction)) * Vec3::Z;
            assert!(
                laid.distance(expected) < 1e-5,
                "{direction:?} laid the quad along {laid:?}, wanted {expected:?}"
            );
        }
    }

    #[test]
    fn dashes_land_on_whole_periods() {
        assert_eq!(dash_repeats(DASH_PERIOD * 4.0), 4);
        // Never zero, however short the street: a line with no dashes in it is
        // a street with no line.
        assert_eq!(dash_repeats(0.5), 1);
    }

    #[test]
    fn dashes_stay_shorter_than_the_gaps_between_them() {
        for length in [30.0, 55.0, 92.0, 140.0] {
            let period = length / dash_repeats(length) as f32;
            let dash = DASH_LENGTH.min(period * 0.6);
            assert!(
                dash < period * 0.75,
                "at {length}m the dashes run into each other"
            );
        }
    }

    #[test]
    fn paint_is_opaque_where_it_is_painted_and_gone_where_it_is_not() {
        let image = centre_line_texture();
        let data = image.data.as_ref().expect("pixels");
        let alpha: Vec<u8> = data.as_chunks::<4>().0.iter().map(|p| p[3]).collect();
        assert!(
            alpha.iter().any(|&a| a > 200),
            "no solid paint anywhere in the dash"
        );
        assert!(
            alpha.iter().any(|&a| a < 20),
            "the dash has no soft edge, so it will alias into dots"
        );
    }

    #[test]
    fn a_crossing_is_bars_and_not_a_slab() {
        let image = crossing_texture();
        let data = image.data.as_ref().expect("pixels");
        let width = image.texture_descriptor.size.width as usize;
        let pixels = data.as_chunks::<4>().0;

        // Walk one row across the crossing and count the bars.
        let row = width / 2;
        let mut bars = 0;
        let mut on = false;
        for x in 0..width {
            let solid = pixels[row * width + x][3] > 128;
            if solid && !on {
                bars += 1;
            }
            on = solid;
        }
        assert!(
            (4..=10).contains(&bars),
            "counted {bars} bars across the crossing"
        );
    }
}
