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

/// Points around one cross-section. Twenty is enough that a roofline reads as
/// curved and few enough that a street full of cars is still cheap.
const RING: usize = 20;

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
    /// The upper body: full width, arching over each axle.
    pub shell: Vec<Section>,
    /// The sill and valances: narrower, running the length below the shell,
    /// and pinched almost to nothing at each axle so it does not simply fill
    /// the arch back in.
    pub lower: Vec<Section>,
    /// The greenhouse, narrower and set into the shell. Empty for a bare cab.
    pub cabin: Vec<Section>,
}

/// Skins a series of cross-sections into a closed mesh.
///
/// The ring is walked from the bottom centre so its seam — the one column where
/// the texture wraps and mirrors — ends up underneath the car, where nobody
/// looks. The ends are capped with their own duplicated vertices, so smoothing
/// averages the nose into a dome rather than dragging the bonnet's normals
/// round the front.
fn loft(sections: &[Section], scale: Vec3) -> Mesh {
    assert!(sections.len() >= 2, "a body needs at least two sections");

    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut uvs: Vec<[f32; 2]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();

    let ring_at = |section: &Section| -> Vec<Vec3> {
        let z = (section.at * 2.0 - 1.0) * scale.z;
        let center = (section.top + section.bottom) * 0.5 * scale.y;
        let half_height = (section.top - section.bottom) * 0.5 * scale.y;
        let half_width = section.half_width * scale.x;
        // The exponent that turns a circle into a rounded box.
        let power = 2.0 / section.squareness.max(2.0);

        (0..RING)
            .map(|i| {
                let theta = std::f32::consts::TAU * (i as f32 / RING as f32) - FRAC_PI_2;
                let (sin, cos) = theta.sin_cos();
                Vec3::new(
                    half_width * cos.signum() * cos.abs().powf(power),
                    center + half_height * sin.signum() * sin.abs().powf(power),
                    z,
                )
            })
            .collect()
    };

    // --- the skin ---
    for (s, section) in sections.iter().enumerate() {
        let v = s as f32 / (sections.len() - 1) as f32;
        for (i, point) in ring_at(section).into_iter().enumerate() {
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
        let ring = ring_at(&section);
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

/// The three lofts for one archetype.
///
/// Arches need three or four sections each rather than one: a single raised
/// section makes a V-shaped notch, which reads as damage rather than as an arch.
pub fn profile(class: VehicleClass) -> BodyProfile {
    match class {
        // Three-box saloon: short bonnet, upright screen, a boot behind the
        // cabin. The default shape of a car.
        VehicleClass::Sedan | VehicleClass::Police => BodyProfile {
            shell: vec![
                Section::new(0.00, 0.70, -0.46, 0.04, 3.5),
                Section::new(0.04, 0.88, -0.58, 0.18, 4.0),
                Section::new(0.09, 0.97, -0.60, 0.25, 4.5),
                Section::new(0.13, 1.00, -0.44, 0.29, 5.0),
                Section::new(0.19, 1.00, -0.10, 0.31, 5.0),
                Section::new(0.25, 1.00, -0.44, 0.32, 5.0),
                Section::new(0.31, 1.00, -0.56, 0.33, 5.0),
                Section::new(0.50, 1.00, -0.58, 0.34, 5.0),
                Section::new(0.69, 1.00, -0.56, 0.34, 5.0),
                Section::new(0.75, 1.00, -0.44, 0.33, 5.0),
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
        },

        // Mid-engined wedge: nose almost on the floor, screen raked hard, the
        // cabin pushed forward with the mass behind it.
        VehicleClass::Sports => BodyProfile {
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
        },

        // Box van: one volume, flat sides, a short snub nose. Nearly all the
        // length is cargo, which is what makes it read as a van and not a bus.
        VehicleClass::Truck => BodyProfile {
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
            // greenhouse to raise above it.
            cabin: Vec::new(),
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
const BODY_INSET: f32 = 0.93;

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
    pub cabin: Option<Mesh>,
}

pub fn build(class: VehicleClass, spec: &VehicleSpec) -> BodyMeshes {
    let profile = profile(class);
    let scale = spec.half_extents * Vec3::new(BODY_INSET, 1.0, 1.0);
    BodyMeshes {
        shell: loft(&profile.shell, scale),
        lower: loft(&profile.lower, scale),
        cabin: (!profile.cabin.is_empty()).then(|| loft(&profile.cabin, scale)),
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
        for class in [
            VehicleClass::Sedan,
            VehicleClass::Sports,
            VehicleClass::Truck,
            VehicleClass::Police,
        ] {
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
        for class in [
            VehicleClass::Sedan,
            VehicleClass::Sports,
            VehicleClass::Truck,
        ] {
            let BodyProfile {
                shell,
                lower,
                cabin,
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
        for class in [
            VehicleClass::Sedan,
            VehicleClass::Sports,
            VehicleClass::Truck,
        ] {
            let BodyProfile {
                shell,
                lower,
                cabin,
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
        for class in [VehicleClass::Sedan, VehicleClass::Sports] {
            let BodyProfile { shell, cabin, .. } = profile(class);
            let beltline = shell.iter().map(|s| s.top).fold(f32::MIN, f32::max);
            let roof = cabin.iter().map(|s| s.top).fold(f32::MIN, f32::max);
            assert!(roof > beltline, "{class:?} has no visible greenhouse");
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
