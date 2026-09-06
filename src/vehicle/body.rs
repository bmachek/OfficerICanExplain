//! Procedural bodywork.
//!
//! A car is described as a handful of cross-sections along its length and then
//! skinned. That is the whole idea: a silhouette is what makes a vehicle
//! recognisable — where the bonnet drops, how far back the screen is raked, how
//! much of the length the cabin takes — and all of that is a table of numbers
//! per archetype rather than a modelling job.
//!
//! The shapes are *archetypes*, not reproductions. A body-on-frame cruiser, a
//! long-bonnet muscle coupé, a mid-engined wedge, a box van, a pickup. Copying
//! the lines of a real car would mean copying design its manufacturer protects,
//! which is exactly why the games this borrows from ship archetypes too — and
//! an archetype reads faster anyway, because it is the idea of the car rather
//! than one example of it.
//!
//! Sections are given in normalised coordinates and scaled by the spec's
//! half-extents, so handling and shape stay independent: retune a car's size
//! and its bodywork follows without being redrawn.

use std::f32::consts::FRAC_PI_2;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;

use super::spec::{VehicleClass, VehicleSpec};

/// How much narrower the body is above its beltline.
///
/// Small on purpose. This is a crease, not a step: what the eye reads is the
/// hard change of shading across it, and five percent is already plenty for
/// that. Ten looks like the roof has slipped.
const TUMBLEHOME: f32 = 0.955;

/// Points around one cross-section.
///
/// Twenty-eight rather than twenty because the shoulder — the turn from flank
/// to roof — is what makes a body read as pressed steel rather than moulded,
/// and a tight turn needs points to land on. Still small: a car is under a
/// thousand triangles.
const RING: usize = 28;

/// One cross-section of a body, in units of the spec's half-extents.
///
/// `bottom` and `top` are measured from the body origin, so -1 is the very
/// bottom of the collider box and +1 the very top. They are what makes a shape:
/// a bonnet is a low `top` at the front, a wheel arch is a raised `bottom` over
/// an axle, a chin spoiler is a low `bottom` at the nose.
#[derive(Debug, Clone, Copy)]
pub struct Section {
    /// Along the car: 0 at the nose, 1 at the tail.
    pub at: f32,
    pub half_width: f32,
    pub bottom: f32,
    pub top: f32,
    /// 2 is an ellipse; larger squares the corners off. Vans want 6, a wedge
    /// wants 3.
    pub squareness: f32,
}

impl Section {
    const fn new(at: f32, half_width: f32, bottom: f32, top: f32, squareness: f32) -> Self {
        Self {
            at,
            half_width,
            bottom,
            top,
            squareness,
        }
    }
}

/// Everything needed to build one vehicle's visible shell.
///
/// Three lofts, and the middle one is what makes wheel arches possible at all.
/// A single ring cannot have an arch cut into it — raising its floor over an
/// axle raises it right across the car, and pushed far enough the body pinches
/// in half. So the body is split by height instead: the `shell` is the full
/// width and arches *up* over each axle, and the `lower` sill runs the length
/// underneath it at four-fifths of the width. The wheel then sits outboard of
/// the sill and under the shell's arch, which is exactly where a wheel goes.
pub struct BodyProfile {
    /// Height of the beltline crease, in the same units as `top` and `bottom`.
    ///
    /// The shoulder of a superellipse turns evenly — sampled at twenty-eight
    /// points the sharpest corner on a saloon's cross-section is twenty-three
    /// degrees, spread over several of them. There is no edge in there to find,
    /// which means a crease has to be *put* there: above this height the shell
    /// steps in by a few percent, and the step is the line down the car's
    /// flank that every pressed panel has.
    pub belt: f32,
    /// The upper body: full width, arching over each axle.
    pub shell: Vec<Section>,
    /// The sill and valances: narrower, running the length below the shell,
    /// and pinched almost to nothing at each axle so it does not simply fill
    /// the arch back in.
    pub lower: Vec<Section>,
    /// The greenhouse, narrower and set into the shell. Empty for a bare cab.
    pub cabin: Vec<Section>,
    /// Glazing cut straight out of the shell, for a cab that has no greenhouse
    /// of its own to glaze. A van's windscreen is not set into anything — it
    /// *is* the front of the box — so there is nothing to loft it from but the
    /// box itself.
    pub windows: Vec<Cut>,
}

/// The centre of a cross-section, on the car's midline.
fn section_centre(section: &Section, scale: Vec3) -> Vec3 {
    Vec3::new(
        0.0,
        (section.top + section.bottom) * 0.5 * scale.y,
        (section.at * 2.0 - 1.0) * scale.z,
    )
}

/// One point on a cross-section's ring.
///
/// `around` is the fraction of the way round it, measured from the bottom
/// centre so that zero lands on the seam underneath the car. A quarter is the
/// right flank at mid-height, a half the roof, three quarters the left flank.
///
/// Split out from [`ring`] rather than left inside it because [`panel`] needs
/// points at arbitrary fractions: a pillar is a tenth of the way round, and
/// rounding it to the nearest of twenty-eight sampled points would make its
/// width depend on where it happened to land.
fn ring_point(section: &Section, scale: Vec3, belt: Option<f32>, around: f32) -> Vec3 {
    let centre = section_centre(section, scale);
    let half_height = (section.top - section.bottom) * 0.5 * scale.y;
    let half_width = section.half_width * scale.x;
    // The exponent that turns a circle into a rounded box.
    let power = 2.0 / section.squareness.max(2.0);

    let theta = std::f32::consts::TAU * around - FRAC_PI_2;
    let (sin, cos) = theta.sin_cos();
    let y = centre.y + half_height * sin.signum() * sin.abs().powf(power);
    let taper = match belt {
        Some(height) if y > height * scale.y => TUMBLEHOME,
        _ => 1.0,
    };
    Vec3::new(
        half_width * taper * cos.signum() * cos.abs().powf(power),
        y,
        centre.z,
    )
}

/// One whole cross-section, sampled at [`RING`] points and creased at the belt.
fn ring(section: &Section, scale: Vec3, belt: Option<f32>) -> Vec<Vec3> {
    let mut points: Vec<Vec3> = (0..RING)
        .map(|i| ring_point(section, scale, belt, i as f32 / RING as f32))
        .collect();

    // Snap the two points that straddle the belt onto it. Left to fall
    // where the sampling happens to put them, the step spreads over the
    // ring's own vertical spacing and comes out at about thirty-five
    // degrees — under any threshold loose enough to leave the roof round.
    // Pinned to the same height, the ledge between them is horizontal, the
    // flank either side is vertical, and the fold is a right angle that no
    // amount of tessellation can soften.
    if let Some(height) = belt {
        let belt_y = height * scale.y;
        let half_width = section.half_width * scale.x;
        for i in 0..RING {
            let j = (i + 1) % RING;
            let (a, b) = (points[i], points[j]);
            let crosses = (a.y - belt_y) * (b.y - belt_y) < 0.0;
            // Only on the flanks. Over the roof and under the floor the
            // ring is nearly horizontal, and a crease there is a dent.
            let on_the_flank = a.x.abs().min(b.x.abs()) > half_width * 0.45;
            if crosses && on_the_flank {
                points[i].y = belt_y;
                points[j].y = belt_y;
            }
        }
    }
    points
}

/// The cross-section a fraction of the way along a run.
///
/// Interpolated in *index* space rather than in `at`, because index space is
/// what the loft's own V coordinate is in. A cut that ends at 0.4 therefore
/// lands exactly on the third of six sections, and the panel trimmed out of it
/// sits exactly on the surface it was cut from instead of a centimetre off it.
fn section_at(sections: &[Section], along: f32) -> Section {
    let last = sections.len() - 1;
    let position = (along.clamp(0.0, 1.0) * last as f32).min(last as f32 - 1e-4);
    let index = position.floor() as usize;
    let t = position - index as f32;
    let (a, b) = (sections[index], sections[index + 1]);
    Section {
        at: a.at.lerp(b.at, t),
        half_width: a.half_width.lerp(b.half_width, t),
        bottom: a.bottom.lerp(b.bottom, t),
        top: a.top.lerp(b.top, t),
        squareness: a.squareness.lerp(b.squareness, t),
    }
}

/// The cross-section at a position along the car.
///
/// [`section_at`] walks index space, which is what the loft's own V is in.
/// This walks `at` instead, which is what one loft and another have in common:
/// a shell and a greenhouse have different numbers of sections at different
/// spacings, so the only way to ask "what is the body doing where the cabin
/// starts" is to ask in the coordinate they share.
pub fn section_where(sections: &[Section], at: f32) -> Section {
    let last = sections.len() - 1;
    let index = sections
        .windows(2)
        .position(|pair| at < pair[1].at)
        .unwrap_or(last - 1);
    let (a, b) = (sections[index], sections[index + 1]);
    let span = (b.at - a.at).max(1e-5);
    section_at(sections, (index as f32 + (at - a.at) / span) / last as f32)
}

/// A rectangle of a lofted surface, in the surface's own coordinates.
///
/// The frame of a greenhouse is not a different shape from the glass it holds:
/// a roof, a header and an A-pillar are the parts of one pressing that were
/// left as steel, and the glazing is the holes in it. Cutting them out of the
/// same loft is what keeps them agreeing — a pillar described independently
/// would have to be re-derived every time a cabin's rake changed.
#[derive(Debug, Clone, Copy)]
pub struct Cut {
    /// Fraction of the way round the ring, as [`ring_point`] measures it.
    pub around: (f32, f32),
    /// Fraction of the way along the section run, matching the loft's own V.
    pub along: (f32, f32),
}

impl Cut {
    const fn new(around: (f32, f32), along: (f32, f32)) -> Self {
        Self { around, along }
    }
}

/// Skins one [`Cut`], lifted clear of the surface it was taken from.
///
/// `swell` is a fraction of the section's own radius rather than a distance,
/// so the same cut sits proud by a sensible amount on a hatchback and on a van.
fn panel(sections: &[Section], cut: &Cut, scale: Vec3, belt: Option<f32>, swell: f32) -> Mesh {
    let (u0, u1) = cut.around;
    let (v0, v1) = cut.along;
    // Matched to the loft's own density, and no finer: a pillar two ring
    // segments wide gains nothing from eight columns of triangles, and the
    // frame is drawn on every car in the city.
    let columns = (((u1 - u0).abs() * RING as f32).ceil() as usize).max(1);
    let rows = ((((v1 - v0).abs() * (sections.len() - 1) as f32) * 3.0).ceil() as usize).max(1);

    let mut positions: Vec<[f32; 3]> = Vec::with_capacity((columns + 1) * (rows + 1));
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity((columns + 1) * (rows + 1));
    let mut indices: Vec<u32> = Vec::new();

    for r in 0..=rows {
        let v = v0.lerp(v1, r as f32 / rows as f32);
        let section = section_at(sections, v);
        let centre = section_centre(&section, scale);
        for c in 0..=columns {
            let u = u0.lerp(u1, c as f32 / columns as f32);
            let point = ring_point(&section, scale, belt, u);
            positions.push((centre + (point - centre) * (1.0 + swell)).to_array());
            uvs.push([c as f32 / columns as f32, v]);
        }
    }

    let stride = (columns + 1) as u32;
    for r in 0..rows as u32 {
        for c in 0..columns as u32 {
            let (here, next) = (r * stride + c, (r + 1) * stride + c);
            // Same winding as the loft, for the same reason: taken the obvious
            // way round, every panel faces into the car.
            indices.extend_from_slice(&[here, next + 1, next, here, here + 1, next + 1]);
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// Skins several cuts into one mesh.
///
/// Merged rather than spawned as separate entities so that a car's whole frame
/// is one draw: the parts never move relative to each other, and each patch
/// keeps its own vertices anyway, so the fold where the roof meets a header
/// stays hard without any crease-splitting.
fn panels(sections: &[Section], cuts: &[Cut], scale: Vec3, belt: Option<f32>, swell: f32) -> Mesh {
    let mut merged = panel(sections, &cuts[0], scale, belt, swell);
    for cut in &cuts[1..] {
        merged
            .merge(&panel(sections, cut, scale, belt, swell))
            .expect("every panel is built the same way");
    }
    merged.with_computed_smooth_normals()
}

/// Skins a series of cross-sections into a closed mesh.
///
/// The ring is walked from the bottom centre so its seam — the one column where
/// the texture wraps and mirrors — ends up underneath the car, where nobody
/// looks. The ends are capped with their own duplicated vertices, so smoothing
/// averages the nose into a dome rather than dragging the bonnet's normals
/// round the front.
fn loft(sections: &[Section], scale: Vec3, belt: Option<f32>) -> Mesh {
    assert!(sections.len() >= 2, "a body needs at least two sections");

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    // --- the skin ---
    for (s, section) in sections.iter().enumerate() {
        let v = s as f32 / (sections.len() - 1) as f32;
        for (i, point) in ring(section, scale, belt).into_iter().enumerate() {
            positions.push(point.to_array());
            uvs.push([i as f32 / RING as f32, v]);
        }
    }
    for s in 0..sections.len() - 1 {
        let (here, next) = ((s * RING) as u32, ((s + 1) * RING) as u32);
        for i in 0..RING as u32 {
            let j = (i + 1) % RING as u32;
            // Wound so the face looks outwards. The ring runs anticlockwise
            // seen from behind the car, so taking it in the obvious order
            // turns every panel inside out — and a body whose faces all point
            // inwards does not look inside out, it looks crumpled, because
            // what you actually see is the far side of the car through it.
            indices.extend_from_slice(&[
                here + i,
                next + j,
                next + i,
                here + i,
                here + j,
                next + j,
            ]);
        }
    }

    // --- the caps ---
    for (section, front) in [(sections[0], true), (*sections.last().unwrap(), false)] {
        let ring = ring(&section, scale, belt);
        let center = ring.iter().copied().sum::<Vec3>() / RING as f32;
        let base = positions.len() as u32;

        positions.push(center.to_array());
        uvs.push([0.5, 0.5]);
        for (i, point) in ring.iter().enumerate() {
            positions.push(point.to_array());
            uvs.push([i as f32 / RING as f32, if front { 0.0 } else { 1.0 }]);
        }
        for i in 0..RING as u32 {
            let j = (i + 1) % RING as u32;
            // The nose cap faces -Z and the tail cap +Z, so they wind opposite
            // ways or one of them is inside out.
            if front {
                indices.extend_from_slice(&[base, base + 1 + j, base + 1 + i]);
            } else {
                indices.extend_from_slice(&[base, base + 1 + i, base + 1 + j]);
            }
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        // Kept in the main world as well as the render world: crash damage
        // deforms these, and a mesh dropped after upload cannot be read back.
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
    .with_computed_smooth_normals()
}

/// A surface of revolution about the X axis, for wheels.
///
/// `profile` is a series of (position across the wheel, radius) pairs, both in
/// units of the wheel radius. Rounding the shoulders off a tyre this way is
/// what stops it reading as a bin lid.
fn revolve(profile: &[(f32, f32)], segments: usize, uv_repeat: f32) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    for (r, &(x, radius)) in profile.iter().enumerate() {
        for i in 0..=segments {
            let theta = std::f32::consts::TAU * i as f32 / segments as f32;
            let (sin, cos) = theta.sin_cos();
            positions.push([x, radius * cos, radius * sin]);
            uvs.push([
                i as f32 / segments as f32 * uv_repeat,
                r as f32 / (profile.len() - 1) as f32,
            ]);
        }
    }

    let stride = (segments + 1) as u32;
    for r in 0..profile.len() as u32 - 1 {
        for i in 0..segments as u32 {
            let (a, b) = (r * stride + i, (r + 1) * stride + i);
            // Outward-facing, for the same reason as the body loft.
            indices.extend_from_slice(&[a, b + 1, b, a, a + 1, b + 1]);
        }
    }

    Mesh::new(
        PrimitiveTopology::TriangleList,
        // Kept in the main world as well as the render world: crash damage
        // deforms these, and a mesh dropped after upload cannot be read back.
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
    .with_computed_smooth_normals()
}

/// A tyre: a barrel with its shoulders rolled off, one wheel-radius tall.
///
/// Built at unit radius and scaled by the spec, like everything else here.
pub fn tyre_mesh(width: f32) -> Mesh {
    let half = width * 0.5;
    revolve(
        &[
            (-half, 0.74),
            (-half * 0.92, 0.93),
            (-half * 0.72, 1.0),
            (half * 0.72, 1.0),
            (half * 0.92, 0.93),
            (half, 0.74),
        ],
        24,
        // The tread pattern goes round the tyre several times rather than being
        // stretched once around it.
        6.0,
    )
}

/// A wheel face: the dish the spokes are painted onto, plus the barrel behind
/// it so the wheel is not hollow when seen from an angle.
pub fn rim_mesh(width: f32) -> Mesh {
    let outer = width * 0.5 * 0.98;
    revolve(
        &[
            (outer - width * 0.34, 0.0),
            (outer - width * 0.30, 0.30),
            (outer - width * 0.16, 0.58),
            (outer - width * 0.02, 0.68),
            (outer, 0.72),
            (-outer, 0.72),
        ],
        24,
        1.0,
    )
}

/// Where the steel is, on a greenhouse that has one.
///
/// Shared by every archetype with a cabin, and it can be shared because every
/// cabin in the table below is described by the same six sections playing the
/// same six parts: cowl, lower screen, top of the screen, back of the roof,
/// lower backlight, tail. Index space therefore means the same thing on a
/// saloon and on a wedge, and `a_cabin_is_described_in_six_parts` is what keeps
/// it that way — add a seventh section to one cabin and every pillar on that
/// car slides to the wrong place.
///
/// The pattern is a checkerboard, and it falls out of what the two coordinates
/// mean. Along the car, a greenhouse is screen, then roof, then backlight.
/// Around a cross-section, it is flank, then roof, then flank. So the *top* of
/// the tube is glass at each end and steel in the middle, and the *flanks* are
/// steel at each end and glass in the middle — which is a windscreen, a roof, a
/// backlight, and door glass between an A-, a B- and a C-pillar. Nothing here
/// describes the shape of a pillar; it only says which parts of the greenhouse
/// were never cut out of it.
const GREENHOUSE: [Cut; 7] = [
    // The roof, from one shoulder over to the other. Its ends are where the
    // pillars stop, so the screen and the backlight are exactly the gaps left
    // over: shift one number and the frame stays closed.
    Cut::new((0.375, 0.625), (0.215, 0.785)),
    // Everything on the flanks ahead of the screen's top edge: the A-pillar,
    // and the cowl side below it. A fifth of the cabin, not a third — a
    // third is what a coupé's short greenhouse turns into two window slots
    // with more paint between them than glass.
    Cut::new((0.020, 0.385), (0.000, 0.215)),
    Cut::new((0.615, 0.980), (0.000, 0.215)),
    // The same behind the backlight: C-pillar and rear quarter.
    Cut::new((0.020, 0.385), (0.785, 1.000)),
    Cut::new((0.615, 0.980), (0.785, 1.000)),
    // The B-pillar, splitting the side glass into a front and a rear door.
    Cut::new((0.020, 0.385), (0.487, 0.513)),
    Cut::new((0.615, 0.980), (0.487, 0.513)),
];

/// How far a pressing stands off the glass it is trimming, as a fraction of the
/// section's radius.
///
/// Small: a pillar is flush with its glass on any car built since the seventies,
/// and what makes it read is that it is painted, not that it sticks out. This
/// only has to beat the depth buffer.
const SEAL: f32 = 0.006;

/// How much smaller than the glazing the cabin liner is built.
const LINER: f32 = 0.985;

/// The three lofts for one archetype.
///
/// Arches need three or four sections each rather than one: a single raised
/// section makes a V-shaped notch, which reads as damage rather than as an arch.
pub fn profile(class: VehicleClass) -> BodyProfile {
    match class {
        // Three-box saloon: short bonnet, upright screen, a boot behind the
        // cabin. The default shape of a car.
        VehicleClass::Sedan => BodyProfile {
            // Just above the widest part of the section, where a saloon's
            // shoulder line runs.
            belt: 0.10,
            shell: vec![
                Section::new(0.00, 0.70, -0.46, 0.04, 3.5),
                Section::new(0.04, 0.88, -0.58, 0.18, 4.0),
                Section::new(0.09, 0.97, -0.60, 0.25, 4.5),
                Section::new(0.13, 1.00, -0.44, 0.29, 5.0),
                Section::new(0.19, 1.00, -0.10, 0.31, 5.0),
                Section::new(0.25, 1.00, -0.44, 0.32, 6.0),
                Section::new(0.31, 1.00, -0.56, 0.33, 6.5),
                Section::new(0.50, 1.00, -0.58, 0.34, 6.5),
                Section::new(0.69, 1.00, -0.56, 0.34, 6.5),
                Section::new(0.75, 1.00, -0.44, 0.33, 6.0),
                Section::new(0.81, 1.00, -0.10, 0.32, 5.0),
                Section::new(0.87, 1.00, -0.44, 0.31, 5.0),
                Section::new(0.92, 0.98, -0.60, 0.28, 4.5),
                Section::new(0.97, 0.90, -0.58, 0.21, 4.0),
                Section::new(1.00, 0.72, -0.46, 0.06, 3.5),
            ],
            lower: vec![
                Section::new(0.02, 0.55, -0.62, -0.30, 4.0),
                Section::new(0.09, 0.66, -0.74, -0.26, 5.0),
                Section::new(0.14, 0.44, -0.76, -0.14, 5.0),
                Section::new(0.19, 0.28, -0.76, -0.08, 5.0),
                Section::new(0.25, 0.46, -0.76, -0.16, 5.0),
                Section::new(0.32, 0.78, -0.78, -0.26, 5.5),
                Section::new(0.50, 0.80, -0.78, -0.26, 5.5),
                Section::new(0.68, 0.78, -0.78, -0.26, 5.5),
                Section::new(0.75, 0.46, -0.76, -0.16, 5.0),
                Section::new(0.81, 0.28, -0.76, -0.08, 5.0),
                Section::new(0.87, 0.44, -0.76, -0.14, 5.0),
                Section::new(0.92, 0.66, -0.74, -0.26, 5.0),
                Section::new(0.98, 0.55, -0.62, -0.30, 4.0),
            ],
            cabin: vec![
                Section::new(0.32, 0.56, 0.20, 0.38, 3.0),
                Section::new(0.40, 0.74, 0.22, 0.64, 3.5),
                Section::new(0.47, 0.80, 0.22, 0.80, 4.0),
                Section::new(0.66, 0.80, 0.22, 0.82, 4.0),
                Section::new(0.74, 0.74, 0.22, 0.68, 3.5),
                Section::new(0.82, 0.54, 0.20, 0.40, 3.0),
            ],
            windows: Vec::new(),
        },

        // Long bonnet, short deck, fastback roof running unbroken into the
        // tail. The proportion is the whole point: two thirds of the length is
        // in front of the driver.
        VehicleClass::Coupe => BodyProfile {
            // Lower and harder than a saloon's; it is most of the car's face.
            belt: 0.06,
            shell: vec![
                Section::new(0.00, 0.70, -0.42, 0.02, 4.0),
                Section::new(0.04, 0.90, -0.54, 0.16, 4.5),
                Section::new(0.09, 0.98, -0.58, 0.22, 5.5),
                Section::new(0.13, 1.00, -0.42, 0.25, 6.0),
                Section::new(0.19, 1.00, -0.16, 0.27, 6.0),
                Section::new(0.25, 1.00, -0.42, 0.28, 6.5),
                Section::new(0.33, 1.00, -0.54, 0.30, 6.5),
                Section::new(0.52, 1.00, -0.56, 0.32, 6.5),
                Section::new(0.70, 1.00, -0.54, 0.34, 6.5),
                Section::new(0.76, 1.00, -0.42, 0.34, 6.0),
                Section::new(0.82, 1.00, -0.16, 0.34, 6.0),
                Section::new(0.88, 1.00, -0.42, 0.33, 6.0),
                Section::new(0.94, 0.98, -0.58, 0.30, 5.5),
                Section::new(0.98, 0.90, -0.54, 0.24, 4.5),
                Section::new(1.00, 0.74, -0.42, 0.12, 4.0),
            ],
            lower: vec![
                Section::new(0.02, 0.56, -0.60, -0.28, 4.0),
                Section::new(0.09, 0.68, -0.72, -0.24, 5.0),
                Section::new(0.14, 0.44, -0.74, -0.12, 5.0),
                Section::new(0.19, 0.28, -0.74, -0.06, 5.0),
                Section::new(0.25, 0.46, -0.74, -0.14, 5.0),
                Section::new(0.33, 0.80, -0.76, -0.24, 5.5),
                Section::new(0.52, 0.82, -0.76, -0.24, 5.5),
                Section::new(0.70, 0.80, -0.76, -0.24, 5.5),
                Section::new(0.76, 0.46, -0.74, -0.14, 5.0),
                Section::new(0.82, 0.28, -0.74, -0.06, 5.0),
                Section::new(0.88, 0.44, -0.74, -0.12, 5.0),
                Section::new(0.94, 0.68, -0.72, -0.24, 5.0),
                Section::new(0.98, 0.56, -0.60, -0.28, 4.0),
            ],
            // Set well back and running all the way to the tail: a fastback
            // has no boot lid to speak of.
            cabin: vec![
                Section::new(0.44, 0.54, 0.18, 0.36, 3.0),
                Section::new(0.53, 0.74, 0.20, 0.66, 3.5),
                Section::new(0.60, 0.78, 0.20, 0.78, 4.0),
                Section::new(0.74, 0.76, 0.20, 0.74, 4.0),
                Section::new(0.86, 0.66, 0.18, 0.52, 3.5),
                Section::new(0.95, 0.50, 0.16, 0.30, 3.0),
            ],
            windows: Vec::new(),
        },

        // Cab forward, open bed behind. The step down from roof to bed sides is
        // the silhouette, so the shell drops hard at two thirds of the length
        // and the bed walls are what is left.
        VehicleClass::Pickup => BodyProfile {
            // Level with the top of the bed sides.
            belt: 0.24,
            shell: vec![
                Section::new(0.00, 0.78, -0.30, 0.14, 5.0),
                Section::new(0.04, 0.94, -0.42, 0.30, 5.5),
                Section::new(0.09, 1.00, -0.46, 0.36, 6.0),
                Section::new(0.13, 1.00, -0.30, 0.38, 6.0),
                Section::new(0.18, 1.00, -0.06, 0.40, 6.0),
                Section::new(0.23, 1.00, -0.30, 0.40, 6.0),
                Section::new(0.30, 1.00, -0.42, 0.40, 6.5),
                Section::new(0.55, 1.00, -0.44, 0.40, 6.5),
                Section::new(0.62, 1.00, -0.44, 0.34, 6.5),
                Section::new(0.70, 1.00, -0.44, 0.30, 7.0),
                Section::new(0.78, 1.00, -0.30, 0.30, 7.0),
                Section::new(0.84, 1.00, -0.06, 0.30, 7.0),
                Section::new(0.90, 1.00, -0.30, 0.30, 7.0),
                Section::new(0.97, 1.00, -0.44, 0.28, 6.5),
                Section::new(1.00, 0.94, -0.36, 0.24, 5.5),
            ],
            lower: vec![
                Section::new(0.02, 0.62, -0.52, -0.18, 5.0),
                Section::new(0.09, 0.76, -0.72, -0.14, 5.5),
                Section::new(0.13, 0.48, -0.74, -0.02, 5.5),
                Section::new(0.18, 0.30, -0.74, 0.04, 5.5),
                Section::new(0.23, 0.50, -0.74, -0.04, 5.5),
                Section::new(0.32, 0.84, -0.76, -0.14, 6.0),
                Section::new(0.55, 0.86, -0.76, -0.14, 6.0),
                Section::new(0.74, 0.84, -0.76, -0.14, 6.0),
                Section::new(0.78, 0.50, -0.74, -0.04, 5.5),
                Section::new(0.84, 0.30, -0.74, 0.04, 5.5),
                Section::new(0.90, 0.48, -0.74, -0.02, 5.5),
                Section::new(0.96, 0.76, -0.72, -0.14, 5.5),
                Section::new(1.00, 0.62, -0.52, -0.18, 5.0),
            ],
            // A tall cab over the front half only, which is what makes the
            // flat deck behind it read as a bed rather than as a missing roof.
            cabin: vec![
                Section::new(0.24, 0.60, 0.30, 0.48, 3.5),
                Section::new(0.31, 0.82, 0.32, 0.86, 4.5),
                Section::new(0.38, 0.86, 0.32, 1.00, 5.0),
                Section::new(0.56, 0.86, 0.32, 1.00, 5.0),
                Section::new(0.60, 0.80, 0.32, 0.92, 4.5),
                Section::new(0.62, 0.66, 0.30, 0.60, 3.5),
            ],
            windows: Vec::new(),
        },

        // Mid-engined wedge: nose almost on the floor, screen raked hard, the
        // cabin pushed forward with the mass behind it.
        VehicleClass::Sports => BodyProfile {
            belt: -0.02,
            shell: vec![
                Section::new(0.00, 0.64, -0.56, -0.22, 3.0),
                Section::new(0.05, 0.86, -0.66, -0.02, 3.2),
                Section::new(0.10, 0.98, -0.68, 0.08, 3.6),
                Section::new(0.15, 1.00, -0.50, 0.13, 4.0),
                Section::new(0.21, 1.00, -0.14, 0.17, 4.0),
                Section::new(0.27, 1.00, -0.50, 0.21, 4.0),
                Section::new(0.34, 1.00, -0.64, 0.26, 4.0),
                Section::new(0.52, 1.00, -0.66, 0.32, 4.0),
                Section::new(0.69, 1.00, -0.64, 0.35, 4.0),
                Section::new(0.75, 1.00, -0.50, 0.35, 4.0),
                Section::new(0.81, 1.00, -0.14, 0.35, 4.0),
                Section::new(0.87, 1.00, -0.50, 0.34, 4.0),
                Section::new(0.93, 0.98, -0.68, 0.30, 3.6),
                Section::new(0.98, 0.88, -0.62, 0.22, 3.2),
                Section::new(1.00, 0.72, -0.56, 0.10, 3.0),
            ],
            lower: vec![
                Section::new(0.02, 0.55, -0.70, -0.38, 3.5),
                Section::new(0.09, 0.68, -0.84, -0.32, 4.5),
                Section::new(0.14, 0.44, -0.86, -0.18, 4.5),
                Section::new(0.21, 0.28, -0.86, -0.12, 4.5),
                Section::new(0.27, 0.46, -0.86, -0.20, 4.5),
                Section::new(0.34, 0.80, -0.88, -0.30, 5.0),
                Section::new(0.52, 0.82, -0.88, -0.30, 5.0),
                Section::new(0.68, 0.80, -0.88, -0.30, 5.0),
                Section::new(0.75, 0.46, -0.86, -0.20, 4.5),
                Section::new(0.81, 0.28, -0.86, -0.12, 4.5),
                Section::new(0.87, 0.44, -0.86, -0.18, 4.5),
                Section::new(0.93, 0.68, -0.84, -0.32, 4.5),
                Section::new(0.98, 0.55, -0.70, -0.38, 3.5),
            ],
            cabin: vec![
                Section::new(0.33, 0.52, 0.10, 0.24, 2.5),
                Section::new(0.42, 0.72, 0.16, 0.60, 3.0),
                Section::new(0.52, 0.78, 0.18, 0.78, 3.4),
                Section::new(0.67, 0.76, 0.18, 0.78, 3.4),
                Section::new(0.77, 0.66, 0.16, 0.56, 3.0),
                Section::new(0.88, 0.50, 0.12, 0.30, 2.5),
            ],
            windows: Vec::new(),
        },

        // Box van: one volume, flat sides, a short snub nose. Nearly all the
        // length is cargo, which is what makes it read as a van and not a bus.
        VehicleClass::Truck => BodyProfile {
            // A van's is a rubbing strip rather than a styling line, but it is
            // the same fold in the panel.
            belt: 0.30,
            shell: vec![
                Section::new(0.00, 0.78, -0.36, 0.26, 5.0),
                Section::new(0.04, 0.94, -0.50, 0.58, 5.5),
                Section::new(0.08, 1.00, -0.54, 0.78, 6.0),
                Section::new(0.13, 1.00, -0.40, 0.85, 6.0),
                Section::new(0.19, 1.00, -0.04, 0.89, 6.0),
                Section::new(0.25, 1.00, -0.40, 0.91, 6.0),
                Section::new(0.31, 1.00, -0.52, 0.93, 6.0),
                Section::new(0.55, 1.00, -0.54, 0.94, 6.0),
                Section::new(0.75, 1.00, -0.52, 0.94, 6.0),
                Section::new(0.81, 1.00, -0.40, 0.94, 6.0),
                Section::new(0.87, 1.00, -0.04, 0.94, 6.0),
                Section::new(0.93, 1.00, -0.40, 0.94, 6.0),
                Section::new(0.98, 1.00, -0.54, 0.92, 6.0),
                Section::new(1.00, 0.94, -0.46, 0.84, 5.5),
            ],
            lower: vec![
                Section::new(0.02, 0.62, -0.58, -0.22, 5.0),
                Section::new(0.09, 0.74, -0.80, -0.18, 5.5),
                Section::new(0.14, 0.48, -0.84, -0.06, 5.5),
                Section::new(0.19, 0.30, -0.84, 0.00, 5.5),
                Section::new(0.25, 0.50, -0.84, -0.08, 5.5),
                Section::new(0.32, 0.84, -0.86, -0.18, 6.0),
                Section::new(0.55, 0.86, -0.86, -0.18, 6.0),
                Section::new(0.75, 0.84, -0.86, -0.18, 6.0),
                Section::new(0.81, 0.50, -0.84, -0.08, 5.5),
                Section::new(0.87, 0.30, -0.84, 0.00, 5.5),
                Section::new(0.93, 0.48, -0.84, -0.06, 5.5),
                Section::new(0.97, 0.74, -0.80, -0.18, 5.5),
                Section::new(1.00, 0.62, -0.58, -0.22, 5.0),
            ],
            // The cab glass is part of the box, so there is no separate
            // greenhouse to raise above it — the glazing is cut out of the box.
            cabin: Vec::new(),
            windows: vec![
                // The windscreen is the top of the box where the box is still
                // climbing — on a nose this snub that is the whole front face,
                // and there is no bonnet for it to stop at.
                Cut::new((0.330, 0.670), (0.035, 0.150)),
                // Cab door glass: the upper flank, ending before the shell's
                // first wheel arch.
                Cut::new((0.300, 0.380), (0.140, 0.280)),
                Cut::new((0.620, 0.700), (0.140, 0.280)),
            ],
        },
    }
}

/// How much narrower the paintwork is than the collider it sits in.
///
/// Bodywork exactly as wide as the box swallows the wheels: the track is inside
/// the width on every real car, and without arch cutouts — which a lofted ring
/// cannot make, since raising its floor raises it right across the car — the
/// tyres end up buried in the flanks. Pulling the panels in by a few centimetres
/// puts the wheels back outside them, which is what reads as a car having
/// wheels at all.
pub const BODY_INSET: f32 = 0.93;

/// Two faces meeting at more than this are a crease, not a curve.
///
/// Well above the twenty-three degrees the cross-section's own shoulder turns
/// through, so nothing that is meant to be round gets hardened by accident;
/// well below the sixty-odd the beltline step produces.
const CREASE_ANGLE: f32 = 38.0;

/// Hardens the edges where a mesh genuinely folds, and leaves the rest smooth.
///
/// Smooth normals average every face meeting at a vertex, which is right for a
/// roof and wrong for a shoulder line — averaged across a crease, the crease
/// stops existing. The fix is to give the vertex a second copy: faces on one
/// side of the fold use the original, faces on the other use the duplicate, and
/// each then averages only within its own side.
///
/// Which faces belong together is decided by walking edges rather than by
/// clustering normals. Two triangles sharing an edge are on the same side if
/// the edge is not sharp, and that relation is transitive — so a cylinder stays
/// one smooth surface all the way round however many faces it takes, while a
/// single sharp edge splits it. Clustering by normal instead makes the answer
/// depend on which face happened to be visited first.
fn split_creases(mut mesh: Mesh, degrees: f32) -> Mesh {
    let Some(Indices::U32(indices)) = mesh.indices() else {
        return mesh;
    };
    let indices = indices.clone();
    let Some(positions) = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(|a| a.as_float3())
        .map(<[[f32; 3]]>::to_vec)
    else {
        return mesh;
    };
    let uvs = match mesh.attribute(Mesh::ATTRIBUTE_UV_0) {
        Some(VertexAttributeValues::Float32x2(uv)) => uv.clone(),
        _ => return mesh,
    };

    let triangles = indices.len() / 3;
    let normal_of = |t: usize| -> Vec3 {
        let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(positions[indices[t * 3 + k] as usize]));
        (b - a).cross(c - a).normalize_or_zero()
    };
    let normals: Vec<Vec3> = (0..triangles).map(normal_of).collect();

    // --- which triangles share a smooth edge ---
    let mut group: Vec<usize> = (0..triangles).collect();
    fn root(group: &mut [usize], mut i: usize) -> usize {
        while group[i] != i {
            group[i] = group[group[i]];
            i = group[i];
        }
        i
    }

    let mut edges: bevy::platform::collections::HashMap<(u32, u32), usize> = default();
    let cosine = degrees.to_radians().cos();
    for t in 0..triangles {
        for k in 0..3 {
            let (a, b) = (indices[t * 3 + k], indices[t * 3 + (k + 1) % 3]);
            let key = (a.min(b), a.max(b));
            match edges.insert(key, t) {
                Some(other) if normals[t].dot(normals[other]) >= cosine => {
                    let (x, y) = (root(&mut group, t), root(&mut group, other));
                    group[x] = y;
                }
                _ => {}
            }
        }
    }

    // --- one vertex copy per smoothing group that touches it ---
    let mut positions = positions;
    let mut uvs = uvs;
    let mut indices = indices;
    let mut assigned: bevy::platform::collections::HashMap<(u32, usize), u32> = default();
    // Which side got to keep each original vertex. The first one to ask uses it
    // in place; everyone after that gets a copy.
    let mut claimed: Vec<Option<usize>> = vec![None; positions.len()];
    for t in 0..triangles {
        let side = root(&mut group, t);
        for k in 0..3 {
            let vertex = indices[t * 3 + k];
            let slot = match assigned.get(&(vertex, side)) {
                Some(&slot) => slot,
                None => {
                    let slot = match claimed[vertex as usize] {
                        None => {
                            claimed[vertex as usize] = Some(side);
                            vertex
                        }
                        Some(_) => {
                            positions.push(positions[vertex as usize]);
                            uvs.push(uvs[vertex as usize]);
                            (positions.len() - 1) as u32
                        }
                    };
                    assigned.insert((vertex, side), slot);
                    slot
                }
            };
            indices[t * 3 + k] = slot;
        }
    }

    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(Indices::U32(indices));
    mesh.compute_smooth_normals();
    mesh
}

/// Turns a closed mesh into a view of its own inside.
///
/// The winding is reversed and the normals recomputed from it, so the surface
/// that survives backface culling is the *far* one and it is lit as though it
/// faced the viewer. That is what a cabin liner is: look in through a
/// windscreen and you should see the back of the car, not the street beyond it.
///
/// Done in the geometry rather than with the material's `cull_mode`, which
/// expresses the same thing and left the liner invisible on the deferred path.
/// A car you can see straight through is not a subtle failure, and the
/// screenshot that found it is the reason this is geometry now.
pub fn inside_out(mut mesh: Mesh) -> Mesh {
    if let Some(Indices::U32(indices)) = mesh.indices_mut() {
        for triangle in indices.as_chunks_mut::<3>().0 {
            triangle.swap(1, 2);
        }
    }
    mesh.compute_smooth_normals();
    mesh
}

/// Pushes a dent into a body panel.
///
/// `from` is the direction the blow arrived along, in body space. The dent is
/// centred on whichever part of the panel sits furthest into that direction —
/// which is the part that hit something — and falls off in a raised cosine, so
/// the crease has no rim and no hard edge.
///
/// Nothing is clamped against the collider: a dent only ever moves metal
/// inwards, and the box the car collides as never changes. A wrecked car
/// therefore still collides like a straight one, which is the right trade at
/// this fidelity — the alternative is rebuilding a convex hull per impact.
pub fn dent(mesh: &mut Mesh, from: Vec3, depth: f32, radius: f32) {
    let Some(VertexAttributeValues::Float32x3(positions)) =
        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
    else {
        return;
    };

    let Some(center) = positions
        .iter()
        .map(|p| Vec3::from(*p))
        .max_by(|a, b| a.dot(from).total_cmp(&b.dot(from)))
    else {
        return;
    };

    for point in positions.iter_mut() {
        let at = Vec3::from(*point);
        let distance = at.distance(center);
        if distance >= radius {
            continue;
        }
        let falloff = 0.5 + 0.5 * (std::f32::consts::PI * distance / radius).cos();
        *point = (at - from * depth * falloff).to_array();
    }

    // The panel's shading has to follow the metal, or a deep dent stays
    // invisible until it breaks the silhouette.
    mesh.compute_smooth_normals();
}

pub struct BodyMeshes {
    pub shell: Mesh,
    pub lower: Mesh,
    /// The greenhouse, which is glazing and nothing else.
    pub cabin: Option<Mesh>,
    /// The steel over the greenhouse: roof, headers, pillars.
    pub frame: Option<Mesh>,
    /// Glazing lying on the shell, for a cab with no greenhouse.
    pub windows: Option<Mesh>,
    /// The cabin, seen from inside it.
    pub liner: Option<Mesh>,
}

pub fn build(class: VehicleClass, spec: &VehicleSpec) -> BodyMeshes {
    let profile = profile(class);
    let scale = spec.half_extents * Vec3::new(BODY_INSET, 1.0, 1.0);
    // Only the shell is creased. A sill is a pressing with no shoulder to
    // crease, and a greenhouse is glass.
    BodyMeshes {
        shell: split_creases(
            loft(&profile.shell, scale, Some(profile.belt)),
            CREASE_ANGLE,
        ),
        lower: loft(&profile.lower, scale, None),
        cabin: (!profile.cabin.is_empty()).then(|| loft(&profile.cabin, scale, None)),
        // A shade smaller than the glazing so the two never fight over the
        // depth buffer, and inside out so only its far wall is drawn.
        liner: (!profile.cabin.is_empty())
            .then(|| inside_out(loft(&profile.cabin, scale * LINER, None))),
        frame: (!profile.cabin.is_empty())
            .then(|| panels(&profile.cabin, &GREENHOUSE, scale, None, SEAL)),
        // The van's glass lies *on* its bodywork rather than in a hole cut
        // through it, so it is lifted by a seal's worth like a pressing is.
        windows: (!profile.windows.is_empty()).then(|| {
            panels(
                &profile.shell,
                &profile.windows,
                scale,
                Some(profile.belt),
                SEAL,
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vertex_count(mesh: &Mesh) -> usize {
        mesh.attribute(Mesh::ATTRIBUTE_POSITION)
            .expect("positions")
            .len()
    }

    #[test]
    fn every_archetype_builds_a_closed_shell() {
        for class in VehicleClass::ALL {
            let spec = class.spec();
            let BodyMeshes { shell, .. } = build(class, &spec);
            assert!(vertex_count(&shell) > RING, "{class:?} shell is empty");
            assert!(
                shell.attribute(Mesh::ATTRIBUTE_NORMAL).is_some(),
                "{class:?} shell has no normals"
            );
        }
    }

    #[test]
    fn bodywork_stays_inside_the_collider() {
        // The box collider is the physical car. Bodywork poking out of it would
        // mean visible contact that never happens and panels that pass through
        // walls, so every section has to stay within the half-extents.
        for class in VehicleClass::ALL {
            let BodyProfile {
                shell,
                lower,
                cabin,
                ..
            } = profile(class);
            for section in shell.iter().chain(lower.iter()) {
                assert!(
                    section.half_width <= 1.0 && section.bottom >= -1.0,
                    "{class:?} section at {} escapes the collider",
                    section.at
                );
            }
            // The cabin is the one thing allowed above the box: a roof sits on
            // top of the beltline, and the collider is sized for the body.
            for section in cabin {
                assert!(section.half_width <= 1.0);
            }
        }
    }

    #[test]
    fn sections_run_nose_to_tail_in_order() {
        for class in VehicleClass::ALL {
            let BodyProfile {
                shell,
                lower,
                cabin,
                ..
            } = profile(class);
            for run in [shell, lower, cabin] {
                for pair in run.windows(2) {
                    assert!(
                        pair[1].at > pair[0].at,
                        "{class:?} sections must advance; {} then {}",
                        pair[0].at,
                        pair[1].at
                    );
                }
            }
        }
    }

    #[test]
    fn a_cabin_sits_above_its_shells_beltline() {
        // If the greenhouse is not taller than the body it is set into, it is
        // invisible and the car reads as a brick.
        // Every shape with a passenger cabin. A truck is a cab and a box, and
        // its roofline is the top of the shell rather than of a greenhouse.
        for class in [
            VehicleClass::Sedan,
            VehicleClass::Coupe,
            VehicleClass::Sports,
            VehicleClass::Pickup,
        ] {
            let BodyProfile { shell, cabin, .. } = profile(class);
            let beltline = shell.iter().map(|s| s.top).fold(f32::MIN, f32::max);
            let roof = cabin.iter().map(|s| s.top).fold(f32::MIN, f32::max);
            assert!(roof > beltline, "{class:?} has no visible greenhouse");
        }
    }

    #[test]
    fn a_beltline_creases_and_a_body_without_one_does_not() {
        let class = VehicleClass::Sedan;
        let spec = class.spec();
        let profile = profile(class);
        let scale = spec.half_extents * Vec3::new(BODY_INSET, 1.0, 1.0);

        let smooth = split_creases(loft(&profile.shell, scale, None), CREASE_ANGLE);
        let creased = split_creases(
            loft(&profile.shell, scale, Some(profile.belt)),
            CREASE_ANGLE,
        );

        // Both split at the nose and tail caps, which meet the flanks at a
        // right angle. The belt has to add more on top of that.
        assert!(
            vertex_count(&creased) > vertex_count(&smooth),
            "the beltline added no hard edge: {} vertices either way",
            vertex_count(&smooth)
        );
    }

    #[test]
    fn splitting_duplicates_vertices_without_moving_any() {
        let class = VehicleClass::Coupe;
        let spec = class.spec();
        let profile = profile(class);
        let scale = spec.half_extents * Vec3::new(BODY_INSET, 1.0, 1.0);

        let plain = loft(&profile.shell, scale, Some(profile.belt));
        let split = split_creases(
            loft(&profile.shell, scale, Some(profile.belt)),
            CREASE_ANGLE,
        );

        let triangles = |mesh: &Mesh| match mesh.indices() {
            Some(Indices::U32(i)) => i.len() / 3,
            _ => 0,
        };
        assert_eq!(
            triangles(&plain),
            triangles(&split),
            "splitting must not add or remove any surface"
        );

        // Every position in the split mesh has to exist in the original: a
        // crease is a second copy of a vertex, never a moved one.
        let read = |mesh: &Mesh| {
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|a| a.as_float3())
                .expect("positions")
                .to_vec()
        };
        let before = read(&plain);
        for point in read(&split) {
            assert!(
                before
                    .iter()
                    .any(|p| Vec3::from(*p).distance(Vec3::from(point)) < 1e-5),
                "splitting invented a vertex at {point:?}"
            );
        }
    }

    #[test]
    fn a_dent_pushes_metal_inwards_and_leaves_the_far_side_alone() {
        let spec = VehicleClass::Sedan.spec();
        let before = build(VehicleClass::Sedan, &spec).shell;
        let mut after = build(VehicleClass::Sedan, &spec).shell;

        // Hit square on the nose.
        dent(&mut after, Vec3::NEG_Z, 0.25, 1.0);

        let read = |mesh: &Mesh| {
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|a| a.as_float3())
                .expect("positions")
                .to_vec()
        };
        let (old, new) = (read(&before), read(&after));

        let nose_moved = old
            .iter()
            .zip(&new)
            .filter(|(o, _)| o[2] < -spec.half_extents.z * 0.85)
            .any(|(o, n)| (o[2] - n[2]).abs() > 0.05);
        assert!(nose_moved, "the nose should have caved in");

        let tail_still = old
            .iter()
            .zip(&new)
            .filter(|(o, _)| o[2] > spec.half_extents.z * 0.5)
            .all(|(o, n)| (o[2] - n[2]).abs() < 1e-4);
        assert!(tail_still, "a hit on the nose must not move the boot");
    }

    #[test]
    fn a_dent_never_pushes_metal_outwards() {
        let spec = VehicleClass::Sedan.spec();
        let before = build(VehicleClass::Sedan, &spec).shell;
        let mut after = build(VehicleClass::Sedan, &spec).shell;
        let from = Vec3::new(1.0, 0.0, 0.0);
        dent(&mut after, from, 0.3, 1.2);

        let read = |mesh: &Mesh| {
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
                .and_then(|a| a.as_float3())
                .expect("positions")
                .to_vec()
        };
        for (o, n) in read(&before).iter().zip(read(&after)) {
            // Displacement is along -from, so it can only ever reduce the
            // vertex's extent in that direction.
            assert!(
                Vec3::from(n).dot(from) <= Vec3::from(*o).dot(from) + 1e-4,
                "a dent must not push a panel out"
            );
        }
    }

    #[test]
    fn a_cabin_is_described_in_six_parts() {
        // `GREENHOUSE` is one table shared by every archetype, and it can only
        // be shared because index space means the same thing on all of them.
        // A cabin with a seventh section would slide every pillar on that car
        // to somewhere it does not belong, and nothing would fail to build.
        for class in VehicleClass::ALL {
            let cabin = profile(class).cabin;
            if cabin.is_empty() {
                continue;
            }
            assert_eq!(
                cabin.len(),
                6,
                "{class:?} has {} cabin sections; GREENHOUSE assumes six",
                cabin.len()
            );
        }
    }

    #[test]
    fn a_pressing_sits_on_the_glass_it_trims() {
        // The frame is cut from the same loft as the glazing, so every point of
        // it has to land just outside the surface it came from: too far in and
        // it is inside the cabin, too far out and it floats.
        let class = VehicleClass::Sedan;
        let spec = class.spec();
        let profile = profile(class);
        let scale = spec.half_extents * Vec3::new(BODY_INSET, 1.0, 1.0);

        for cut in &GREENHOUSE {
            for &v in &[0.0f32, 0.5, 1.0] {
                for &u in &[0.0f32, 0.5, 1.0] {
                    let along = cut.along.0.lerp(cut.along.1, v);
                    let around = cut.around.0.lerp(cut.around.1, u);
                    let section = section_at(&profile.cabin, along);
                    let glass = ring_point(&section, scale, None, around);
                    let centre = section_centre(&section, scale);
                    let steel = centre + (glass - centre) * (1.0 + SEAL);
                    let gap = steel.distance(glass);
                    assert!(
                        (0.0005..0.010).contains(&gap),
                        "a pressing stands {gap:.4}m off the glass at ({around}, {along})"
                    );
                }
            }
        }
    }

    #[test]
    fn the_frame_leaves_a_windscreen_and_side_glass_uncovered() {
        // The whole point of the cut table is what it does *not* cover. Cover
        // the lot and the car has no windows; cover none of it and it has no
        // pillars, and either way it still builds.
        let covered = |around: f32, along: f32| {
            GREENHOUSE.iter().any(|cut| {
                (cut.around.0..=cut.around.1).contains(&around)
                    && (cut.along.0..=cut.along.1).contains(&along)
            })
        };

        // Roof, both A-pillars, both C-pillars, the B-pillar.
        assert!(covered(0.50, 0.50), "no roof");
        assert!(covered(0.25, 0.15) && covered(0.75, 0.15), "no A-pillar");
        assert!(covered(0.25, 0.85) && covered(0.75, 0.85), "no C-pillar");
        assert!(covered(0.25, 0.50) && covered(0.75, 0.50), "no B-pillar");

        // Windscreen, backlight, and a door window on each side of the
        // B-pillar, on both flanks.
        assert!(!covered(0.50, 0.12), "the windscreen is steel");
        assert!(!covered(0.50, 0.88), "the backlight is steel");
        for around in [0.25f32, 0.75] {
            assert!(!covered(around, 0.42), "no front door glass at {around}");
            assert!(!covered(around, 0.58), "no rear door glass at {around}");
        }

        // And how *much* of it is left over. Every point above is satisfied by
        // pillars wide enough to leave two slots, which is exactly what the
        // first table did: a third of the cabin to each end pillar turned a
        // coupé's short greenhouse into more paint than glass. Half the length
        // of a flank has to be daylight.
        let steps = 500;
        let daylight = (0..steps)
            .filter(|i| !covered(0.25, (*i as f32 + 0.5) / steps as f32))
            .count() as f32
            / steps as f32;
        assert!(
            daylight > 0.5,
            "only {:.0}% of a flank is glass; the pillars have eaten the windows",
            daylight * 100.0
        );

        // The frame has to be closed, too: a pillar that stops short of the
        // roof leaves a rib of glass running over the car between them.
        for edge in [0.215f32, 0.785] {
            assert!(
                covered(0.25, edge - 1e-3) != covered(0.50, edge - 1e-3),
                "the roof and the pillars disagree about where {edge} is"
            );
        }
    }

    #[test]
    fn a_panel_is_wound_the_same_way_as_the_loft_under_it() {
        // A patch taken off the outside of a body and wound the wrong way is
        // invisible from outside and perfectly visible from within, which on a
        // car reads as the roof having been left off.
        let class = VehicleClass::Sedan;
        let spec = class.spec();
        let profile = profile(class);
        let scale = spec.half_extents * Vec3::new(BODY_INSET, 1.0, 1.0);
        // The roof: its faces must point up.
        let mesh = panel(&profile.cabin, &GREENHOUSE[0], scale, None, SEAL);
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .expect("positions");
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("a panel is indexed");
        };

        let mut up = 0;
        for triangle in indices.as_chunks::<3>().0 {
            let [a, b, c] = triangle.map(|v| Vec3::from(positions[v as usize]));
            if (b - a).cross(c - a).normalize_or_zero().y > 0.0 {
                up += 1;
            }
        }
        assert_eq!(
            up,
            indices.len() / 3,
            "{} of {} roof faces point downwards",
            indices.len() / 3 - up,
            indices.len() / 3
        );
    }

    #[test]
    fn a_van_is_glazed_even_though_it_has_no_greenhouse() {
        let class = VehicleClass::Truck;
        let spec = class.spec();
        let built = build(class, &spec);
        assert!(built.cabin.is_none() && built.frame.is_none());
        let windows = built.windows.expect("a van needs a windscreen");
        assert!(vertex_count(&windows) > 8);

        // And that glazing has to be in the upper half of the box and at the
        // front of it, or it is a sunroof or a rear door.
        let positions = windows
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .expect("positions");
        let lowest = positions.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
        let furthest = positions.iter().map(|p| p[2]).fold(f32::MIN, f32::max);
        assert!(
            lowest > 0.0,
            "the cab glazing reaches down to {lowest:.2}m, below the waist"
        );
        assert!(
            furthest < 0.0,
            "the cab glazing reaches {furthest:.2}m, behind the middle of the van"
        );
    }

    #[test]
    fn a_tyre_is_widest_in_the_middle() {
        let width = 0.7;
        let mesh = tyre_mesh(width);
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|a| a.as_float3())
            .expect("positions");

        // Radius is measured in the YZ plane. Group by station and compare the
        // innermost ring with the outermost: the tread has to stand proud of
        // the sidewall, or the tyre is a bin lid.
        let radius = |p: &[f32; 3]| (p[1] * p[1] + p[2] * p[2]).sqrt();
        let crown = positions
            .iter()
            .filter(|p| p[0].abs() < width * 0.40)
            .map(radius)
            .fold(0.0, f32::max);
        let shoulder = positions
            .iter()
            .filter(|p| p[0].abs() > width * 0.48)
            .map(radius)
            .fold(0.0, f32::max);

        assert!(
            crown > 0.0 && shoulder > 0.0,
            "both bands should have rings"
        );
        assert!(
            crown > shoulder * 1.1,
            "tread {crown:.3} should stand proud of sidewall {shoulder:.3}"
        );
    }
}
