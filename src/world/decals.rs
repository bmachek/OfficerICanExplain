//! Everything the road has been through.
//!
//! The asphalt is one plane two kilometres across and its texture is painted
//! before the street layout exists, so nothing in it can know where a junction
//! is, where cars stop, or which way the water runs. Wear has to be laid on top
//! of it afterwards.
//!
//! It is laid as *forward decals*: a quad that reads the depth prepass, finds
//! the surface actually underneath it, and projects itself onto that. Which is
//! the difference between this module and `markings` next door, and the reason
//! they are not the same code. A road marking is an alpha-masked quad floating
//! a centimetre and a half above the asphalt, and it gets away with that
//! because paint really is a flat sheet lying on top of a road. A manhole cover
//! is not: it sits in a surface that dips towards its gutters, and a floating
//! quad shows its own edge as a bright rim the moment the ground is not level.
//!
//! Three things this costs, all of them worth knowing before adding more:
//!
//! * **Decals draw with the depth test off.** That is what lets them paint a
//!   surface that is not exactly where the quad is — and it means the only
//!   thing stopping a stain from painting the roof of a car parked over it is
//!   [`REACH`], the distance over which the decal fades out. Keep it short.
//! * **They are blended, so they are sorted**, unlike everything else in the
//!   city. That is a per-frame cost per decal, which is why these are counted
//!   out per street rather than scattered.
//! * **The camera needs a depth prepass.** The deferred path has one, so this
//!   comes free — but a forward-only camera would draw every decal at full
//!   opacity, floating.

use bevy::image::{ImageAddressMode, ImageSampler};
use bevy::pbr::decal::{ForwardDecal, ForwardDecalMaterial, ForwardDecalMaterialExt};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

use super::buildings::ChunkOf;
use super::roadgraph::RoadEdge;
use super::texture::{byte, fbm, painted, smoothstep01};

/// The material a decal is drawn with: an ordinary [`StandardMaterial`] plus
/// the extension that does the projection.
type Decal = ForwardDecalMaterial<StandardMaterial>;

/// How far from its own plane a decal still finds a surface to sit on, in
/// metres.
///
/// Short on purpose. A decal is drawn with its depth test switched off, so this
/// fade is the only thing keeping a stain on the road instead of across the
/// roof of the car parked over it. Far enough to run down a kerb, and no
/// further.
const REACH: f32 = 0.30;

/// How high above the road a decal's own quad hangs.
///
/// It only has to clear the road plane itself; the projection does the rest,
/// and the fade above is measured from here.
const FLOAT: f32 = 0.02;

/// Metres of street between manhole covers.
const MANHOLES: f32 = 47.0;
/// Metres of kerb between gullies.
const GULLIES: f32 = 26.0;
/// Stains and patches per hundred metres of street.
const BLEMISHES: f32 = 3.4;

/// Diameter of a manhole cover, in metres. A real one is 600mm plus its frame.
const MANHOLE_SIZE: f32 = 0.72;
/// A gully grating, across the kerb and along it.
const GULLY_SIZE: Vec2 = Vec2::new(0.44, 0.66);
/// How far in from the kerb face the grating sits.
const GULLY_INSET: f32 = 0.26;
/// How far back from a junction a car leaves its rubber, in metres.
const SKID_LENGTH: f32 = 7.5;
/// Half the gap between the two tracks of one car, in metres.
const TRACK: f32 = 0.74;

const SIZE: u32 = 256;
/// Fraction of a decal's texture kept clear at its edge.
///
/// Not decoration. The projection shifts the UV outwards as the view flattens,
/// and a decal whose image runs to its own border smears that border across the
/// road in a streak. The margin is what it smears instead.
const MARGIN: f32 = 0.07;
/// How much of that margin is *exactly* transparent rather than merely faint.
///
/// The sampler clamps, so it is the outermost texel that gets dragged, and it
/// gets dragged for as far as the projection reaches. One part in 255 is
/// invisible in a texel and a visible smear in a metre of it, so the outer part
/// of the margin has to be zero rather than nearly zero.
const CLEAR: f32 = 0.4;

#[derive(Resource)]
pub struct WearKit {
    manhole: Handle<Decal>,
    gully: Handle<Decal>,
    patch: Handle<Decal>,
    oil: Handle<Decal>,
    crack: Handle<Decal>,
    skid: Handle<Decal>,
}

/// Fades an image out at its own edge, so the projection has something
/// harmless to smear.
fn margin(u: f32, v: f32) -> f32 {
    let edge = u.min(1.0 - u).min(v).min(1.0 - v);
    smoothstep01((edge - MARGIN * CLEAR) / (MARGIN * (1.0 - CLEAR)))
}

/// A blob with a torn edge rather than a circular one.
///
/// Everything that has ever been spilled on a road has this shape: it ran until
/// it stopped. A circle reads as a sticker.
fn spill(u: f32, v: f32, seed: u32, softness: f32) -> f32 {
    let radius = Vec2::new(u - 0.5, v - 0.5).length() * 2.0;
    let torn = (fbm(u, v, 5, 3, seed) - 0.5) * 0.85;
    smoothstep01((1.0 - radius + torn) / softness) * margin(u, v)
}

/// A cast-iron cover: a rim, and a raised pattern to stand on.
fn manhole() -> Image {
    painted(SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        let radius = Vec2::new(u - 0.5, v - 0.5).length() * 2.0;
        // The frame it sits in is square-ish and set into the asphalt; the
        // cover itself is the disc inside it.
        let seat = smoothstep01((0.99 - radius) / 0.06) * margin(u, v);
        let cover = smoothstep01((0.84 - radius) / 0.03);

        // Raised diamonds, the pattern nearly every foundry uses, lit from the
        // same side as the rest of the world by a plain value step rather than
        // a normal map — at this size a normal map is four texels a diamond.
        let (du, dv) = ((u * 13.0).fract() - 0.5, (v * 13.0).fract() - 0.5);
        let waffle = smoothstep01((0.30 - (du.abs() + dv.abs())) / 0.10);
        // A ring where the cover meets its frame, always full of grit.
        let seam = smoothstep01((0.045 - (radius - 0.86).abs()) / 0.03);

        let iron = 0.16 + waffle * cover * 0.09 - seam * 0.07;
        // Rust does not cover a cover evenly; it starts at the seam.
        let rust = fbm(u, v, 7, 3, 3) * (0.35 + seam * 0.6);
        let value = iron * (1.0 + rust * 0.25);
        [
            byte(value * (1.0 + rust * 0.55)),
            byte(value * (1.0 + rust * 0.18)),
            byte(value),
            byte(seat),
        ]
    })
}

/// A gully grating at the kerb: bars, and a dark slot between each pair.
fn gully() -> Image {
    painted(SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        let frame = smoothstep01((0.46 - (u - 0.5).abs()) / 0.05)
            * smoothstep01((0.46 - (v - 0.5).abs()) / 0.05)
            * margin(u, v);
        // Bars run across the gutter so a wheel crosses them rather than
        // dropping between them — which is also why they are the long way on.
        let bar = ((v - 0.5) * 7.0).fract();
        let slot = smoothstep01((0.30 - (bar - 0.5).abs()) / 0.12)
            * smoothstep01((0.38 - (u - 0.5).abs()) / 0.04);

        let value = 0.17 - slot * 0.15;
        let c = byte(value);
        [c, c, byte(value * 0.96), byte(frame)]
    })
}

/// A repair: a rectangle of newer, blacker asphalt with a tarred edge.
fn patch() -> Image {
    painted(SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        // Cut with a saw, so it is a rectangle — but a rectangle whose edges
        // were then sealed by hand with a bucket of tar.
        let ragged = (fbm(u * 3.0, v * 3.0, 6, 3, 17) - 0.5) * 0.14;
        let inside = smoothstep01((0.40 - (u - 0.5).abs() + ragged) / 0.02)
            * smoothstep01((0.40 - (v - 0.5).abs() + ragged) / 0.02);
        let seal = smoothstep01((0.455 - (u - 0.5).abs() + ragged) / 0.04)
            * smoothstep01((0.455 - (v - 0.5).abs() + ragged) / 0.04);

        let grit = fbm(u, v, 22, 3, 19);
        // The tar seam is blacker than the patch, and the patch is blacker than
        // the road it was cut into.
        let value = 0.24 + grit * 0.10 - (seal - inside) * 0.14;
        let c = byte(value);
        [c, c, byte(value * 1.02), byte(seal * margin(u, v))]
    })
}

/// A stain: something dripped where a car stood long enough to drip it.
fn oil() -> Image {
    painted(SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        let body = spill(u, v, 23, 0.26);
        // A stain has a dark centre and a halo where it soaked outwards, and
        // the halo is most of what makes it read as soaked in rather than
        // painted on.
        let core = spill(u, v, 23, 0.55);
        let value = 0.030 + (1.0 - core) * 0.045;
        [
            byte(value * 1.15),
            byte(value),
            byte(value * 1.05),
            byte(body * 0.92),
        ]
    })
}

/// A crack, with one branch off it.
fn crack() -> Image {
    painted(SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        // A crack wanders. A straight one reads as a wire lying on the road.
        let wander = (fbm(v * 2.0, 0.31, 4, 3, 5) - 0.5) * 0.42;
        let main = smoothstep01((0.014 - (u - 0.5 - wander).abs()) / 0.011);
        // The branch leaves halfway up and runs out to one side.
        let out = ((v - 0.5) * 1.6).max(0.0);
        let branch = smoothstep01((0.010 - (u - 0.5 - wander - out).abs()) / 0.009)
            * smoothstep01((v - 0.5) / 0.06)
            * smoothstep01((0.95 - v) / 0.15);

        let line = main.max(branch) * margin(u, v);
        // Not black: a crack is a shadow with grit in the bottom of it.
        [byte(0.06), byte(0.06), byte(0.065), byte(line * 0.85)]
    })
}

/// Rubber, laid down twice because a car has two wheels on a side.
fn skid() -> Image {
    painted(SIZE, TextureFormat::Rgba8UnormSrgb, |u, v| {
        // Across the mark: a band with a hard-ish edge, because a tyre has one.
        let band = smoothstep01((0.20 - (u - 0.5).abs()) / 0.07);
        // Along it: heaviest where the wheel locked, fading as it slowed.
        let along = smoothstep01(v / 0.10) * smoothstep01((1.0 - v) / 0.55);
        // Tread: rubber comes off in ribs, not as a sheet.
        let ribs = 0.62 + fbm(u * 5.0, v * 0.7, 16, 3, 29) * 0.7;

        let value = 0.085;
        [
            byte(value),
            byte(value),
            byte(value * 1.05),
            byte(band * along * ribs * 0.8 * margin(u, v)),
        ]
    })
}

/// The image, with the sampler a decal needs rather than the tiling one
/// everything else in the city wants.
///
/// A decal is sampled once across its own quad and its UV is pushed outside
/// 0..1 by the projection. Left repeating, a mark seen at a low angle grows a
/// second copy of itself alongside it.
fn clamped(mut image: Image) -> Image {
    if let ImageSampler::Descriptor(sampler) = &mut image.sampler {
        sampler.address_mode_u = ImageAddressMode::ClampToEdge;
        sampler.address_mode_v = ImageAddressMode::ClampToEdge;
    }
    image
}

fn material(images: &mut Assets<Image>, image: Image, roughness: f32, metallic: f32) -> Decal {
    ForwardDecalMaterial {
        base: StandardMaterial {
            base_color_texture: Some(images.add(clamped(image))),
            perceptual_roughness: roughness,
            metallic,
            alpha_mode: AlphaMode::Blend,
            ..default()
        },
        extension: ForwardDecalMaterialExt {
            depth_fade_factor: REACH,
        },
    }
}

pub fn build_assets(images: &mut Assets<Image>, decals: &mut Assets<Decal>) -> WearKit {
    WearKit {
        // Cast iron: rough, and only half a metal by the time it has been
        // driven over for twenty years.
        manhole: decals.add(material(images, manhole(), 0.62, 0.35)),
        gully: decals.add(material(images, gully(), 0.66, 0.35)),
        // Fresh asphalt is rougher than the road it was cut into, not smoother.
        patch: decals.add(material(images, patch(), 0.94, 0.0)),
        // The one thing on a road that is glossier than the road. In the rain
        // it is the only thing that is not.
        oil: decals.add(material(images, oil(), 0.26, 0.0)),
        crack: decals.add(material(images, crack(), 0.95, 0.0)),
        skid: decals.add(material(images, skid(), 0.58, 0.0)),
    }
}

/// Yaw that lays a decal's local +Z along an XZ direction.
fn along(direction: Vec2) -> f32 {
    direction.x.atan2(direction.y)
}

/// How many of something to put down a street of this length.
///
/// Rounded rather than truncated, so the spacing is stretched or squeezed a
/// little to fit the street exactly instead of leaving a gap at one end — and
/// so a street too short for one still gets none rather than one in its middle.
fn spaced(length: f32, every: f32) -> i32 {
    (length / every).round() as i32
}

fn lay(
    commands: &mut Commands,
    material: &Handle<Decal>,
    at: Vec2,
    size: Vec2,
    yaw: f32,
    chunk: IVec2,
) {
    commands.spawn((
        ChunkOf(chunk),
        ForwardDecal,
        MeshMaterial3d(material.clone()),
        Transform::from_xyz(at.x, FLOAT, at.y)
            .with_rotation(Quat::from_rotation_y(yaw))
            .with_scale(Vec3::new(size.x, 1.0, size.y)),
    ));
}

/// Everything one street has under it and everything spilled on it.
pub fn spawn_edge(
    commands: &mut Commands,
    kit: &WearKit,
    rng: &mut ChaCha8Rng,
    edge: &RoadEdge,
    from: Vec2,
    to: Vec2,
    chunk: IVec2,
) {
    let Ok(direction) = Dir2::new(to - from) else {
        return;
    };
    let across = Vec2::new(-direction.y, direction.x);
    let yaw = along(*direction);

    // Manholes follow the sewer, and the sewer follows the street: one line of
    // them, a little off centre, at the spacing an access chamber is built at.
    let covers = spaced(edge.length, MANHOLES);
    for i in 0..covers {
        let down = edge.length * (i as f32 + 0.5) / covers as f32;
        let side = across * (edge.width * rng.random_range(-0.22..0.22));
        lay(
            commands,
            &kit.manhole,
            from + *direction * down + side,
            Vec2::splat(MANHOLE_SIZE),
            yaw + rng.random_range(-0.4..0.4),
            chunk,
        );
    }

    // Gullies take the water off the road, so they sit in the gutter — against
    // the kerb, on both sides, whatever the camber.
    let drains = spaced(edge.length, GULLIES);
    for i in 0..drains {
        let down = edge.length * (i as f32 + 0.35) / drains as f32;
        for side in [-1.0f32, 1.0] {
            lay(
                commands,
                &kit.gully,
                from + *direction * down + across * (side * (edge.width * 0.5 - GULLY_INSET)),
                GULLY_SIZE,
                yaw,
                chunk,
            );
        }
    }

    // And then whatever has happened to it since. Counted per hundred metres so
    // a long street is not simply a short street with the same number of
    // stains on it.
    let marks = spaced(edge.length * BLEMISHES, 100.0);
    for _ in 0..marks {
        let at = from
            + *direction * rng.random_range(0.0..edge.length)
            + across * (edge.width * rng.random_range(-0.44..0.44));
        let turn = rng.random_range(0.0..std::f32::consts::TAU);
        match rng.random_range(0.0..1.0) {
            // A patch is the commonest thing on a road that has been dug up,
            // and every road has been dug up.
            r if r < 0.42 => lay(
                commands,
                &kit.patch,
                at,
                Vec2::new(rng.random_range(1.3..3.4), rng.random_range(1.1..2.6)),
                yaw + rng.random_range(-0.1..0.1),
                chunk,
            ),
            r if r < 0.72 => lay(
                commands,
                &kit.crack,
                at,
                Vec2::new(rng.random_range(0.7..1.5), rng.random_range(1.8..4.0)),
                yaw + rng.random_range(-0.5..0.5),
                chunk,
            ),
            _ => lay(
                commands,
                &kit.oil,
                at,
                Vec2::splat(rng.random_range(0.5..1.3)),
                turn,
                chunk,
            ),
        }
    }
}

/// What a junction collects: rubber on the way in, and drips where cars wait.
pub fn spawn_junction(
    commands: &mut Commands,
    kit: &WearKit,
    rng: &mut ChaCha8Rng,
    at: Vec2,
    arms: &[(Vec2, f32)],
    chunk: IVec2,
) {
    for &(towards, width) in arms {
        let Ok(direction) = Dir2::new(towards - at) else {
            continue;
        };
        let yaw = along(*direction);

        // Rubber is laid by somebody stopping, so it runs *towards* the
        // junction and stops at it. The lane it is in is the one they were
        // driving in, which is the near side.
        let right = Vec2::new(-direction.y, direction.x);
        let lane = if crate::ai::steering::RIGHT_HAND_TRAFFIC {
            -right
        } else {
            right
        };
        if rng.random_range(0.0..1.0) < 0.45 {
            let centre = at + *direction * (width * 0.6 + SKID_LENGTH * 0.5);
            let offset = lane * (width * 0.25);
            for track in [-TRACK, TRACK] {
                lay(
                    commands,
                    &kit.skid,
                    centre + offset + right * track,
                    Vec2::new(0.55, SKID_LENGTH),
                    yaw,
                    chunk,
                );
            }
        }

        // And a car that waits here long enough drips where it waits.
        if rng.random_range(0.0..1.0) < 0.55 {
            lay(
                commands,
                &kit.oil,
                at + *direction * (width * 0.75) + lane * (width * 0.24),
                Vec2::splat(rng.random_range(0.6..1.4)),
                rng.random_range(0.0..std::f32::consts::TAU),
                chunk,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every decal image, with a name to blame when one of them fails.
    fn all() -> [(&'static str, Image); 6] {
        [
            ("manhole", manhole()),
            ("gully", gully()),
            ("patch", patch()),
            ("oil", oil()),
            ("crack", crack()),
            ("skid", skid()),
        ]
    }

    /// The projection pushes a decal's UV outside its own image as the view
    /// flattens, and the sampler clamps rather than wraps — so whatever is in
    /// the edge texel is what gets dragged across the road in a streak. It has
    /// to be nothing.
    ///
    /// This is the failure the technique is actually prone to, and it is
    /// invisible in a screenshot taken from head height: it only appears when
    /// the camera drops to a windscreen.
    #[test]
    fn no_decal_is_drawn_to_its_own_edge() {
        for (name, image) in all() {
            let data = image.data.as_ref().expect("painted images carry pixels");
            let size = image.texture_descriptor.size.width;
            let border = (size as f32 * MARGIN * CLEAR).floor() as u32;
            for y in 0..size {
                for x in 0..size {
                    let edge = x.min(size - 1 - x).min(y).min(size - 1 - y);
                    if edge >= border {
                        continue;
                    }
                    let alpha = data[((y * size + x) * 4 + 3) as usize];
                    assert_eq!(
                        alpha, 0,
                        "{name} is {alpha}/255 opaque {edge} texels from its edge"
                    );
                }
            }
        }
    }

    /// And having a clear edge is only half of it: an image that is clear
    /// everywhere is a decal nobody can see. Cheap, and it is the test that
    /// fails when a threshold is retuned past the point of drawing anything.
    #[test]
    fn every_decal_draws_something() {
        for (name, image) in all() {
            let data = image.data.as_ref().expect("painted images carry pixels");
            let size = image.texture_descriptor.size.width;
            let covered = (0..size * size)
                .filter(|i| data[(i * 4 + 3) as usize] > 128)
                .count();
            let fraction = covered as f32 / (size * size) as f32;
            // Low, because a crack is a line: two percent of its own quad
            // is a crack, and the same two percent is a manhole cover that
            // failed to paint. The bar is only there to catch the second.
            assert!(
                fraction > 0.005,
                "{name} covers {:.1}% of its own quad",
                fraction * 100.0
            );
        }
    }

    /// The quad's length runs along local Z, so the yaw has to map +Z onto the
    /// street. Backwards, and every skid mark is laid across the road.
    #[test]
    fn a_decal_laid_along_z_points_down_its_street() {
        for direction in [Vec2::X, Vec2::Y, Vec2::new(0.6, -0.8)] {
            let yaw = along(direction.normalize());
            let turned = Quat::from_rotation_y(yaw) * Vec3::Z;
            assert!(
                turned.xz().distance(direction.normalize()) < 1e-5,
                "{direction} came out as {}",
                turned.xz()
            );
        }
    }

    /// Spacing is per street, not per city: a street too short for one gets
    /// none, and a long one gets them at something near the stated distance
    /// rather than all at one end.
    #[test]
    fn things_are_spaced_out_along_a_street_and_not_crammed_into_a_short_one() {
        assert_eq!(spaced(MANHOLES * 0.4, MANHOLES), 0);
        for length in [30.0f32, 55.0, 120.0, 400.0] {
            let count = spaced(length, MANHOLES);
            if count == 0 {
                continue;
            }
            let actual = length / count as f32;
            assert!(
                (actual - MANHOLES).abs() <= MANHOLES * 0.5,
                "a {length}m street puts {count} covers {actual}m apart"
            );
        }
    }
}
