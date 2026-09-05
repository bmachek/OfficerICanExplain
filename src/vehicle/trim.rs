//! Everything bolted to a car that is not the bodywork.
//!
//! The bodywork is lofted (`body`), which makes it good at *shape* and hopeless
//! at *incident*: a loft can give a car a shoulder line and a wheel arch, and it
//! can no more grow a wing mirror than a lathe can. So the small hard parts —
//! the ones that say "car" faster than any silhouette — are separate boxes hung
//! off the same entity, and this module is where they are placed.
//!
//! Two rules keep several hundred parked cars affordable. Every part shares one
//! mesh and one material with the same part on every other car, so a street of
//! them is a handful of draws; and every position is *derived* from the profile
//! the body was lofted from rather than typed in per archetype, so retuning a
//! van's nose moves its grille with it.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;

use super::body::{Section, profile, section_where};
use super::spec::{VehicleClass, VehicleSpec};
use crate::ai::steering::RIGHT_HAND_TRAFFIC;

/// Shared meshes and materials for every fitting in the city.
#[derive(Resource, Clone)]
pub struct TrimKit {
    /// A unit cube. Nearly everything here is a box at some scale.
    pub block: Handle<Mesh>,
    pub wheel: Handle<Mesh>,
    pub pipe: Handle<Mesh>,
    /// The inside of a car, seen through its glass: not a surface so much as
    /// the absence of one.
    pub liner: Handle<StandardMaterial>,
    pub upholstery: Handle<StandardMaterial>,
    /// Bumpers, mirror backs, sills — the mouldings that are not painted.
    pub trim: Handle<StandardMaterial>,
    pub grille: Handle<StandardMaterial>,
    pub chrome: Handle<StandardMaterial>,
    pub mirror: Handle<StandardMaterial>,
}

/// A car's inside is lit by whatever daylight gets past its own glass, which is
/// very little. The number has to stand for the albedo of the trim *and* for
/// that fraction at once, because the fragment is shaded by the same sun as the
/// roof above it — the same reason the rooms behind the windows in `facade` are
/// darker than any real wall.
const CABIN_DARK: f32 = 0.030;

pub fn build_kit(meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) -> TrimKit {
    TrimKit {
        block: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        wheel: meshes.add(Torus::new(0.145, 0.163)),
        pipe: meshes.add(Cylinder::new(0.5, 1.0)),
        liner: materials.add(StandardMaterial {
            base_color: Color::srgb(CABIN_DARK, CABIN_DARK, CABIN_DARK * 1.08),
            perceptual_roughness: 0.94,
            // The liner is the cabin loft worn inside out: only its far wall is
            // drawn, so what you see through the near glass is the back of the
            // car rather than the road behind it. Front faces culled instead of
            // back, and the normals flipped to match, or the whole interior
            // shades as though the sun were inside it.
            cull_mode: Some(bevy::render::render_resource::Face::Front),
            double_sided: true,
            ..default()
        }),
        upholstery: materials.add(StandardMaterial {
            base_color: Color::srgb(0.055, 0.052, 0.050),
            perceptual_roughness: 0.86,
            ..default()
        }),
        trim: materials.add(StandardMaterial {
            base_color: Color::srgb(0.085, 0.086, 0.090),
            perceptual_roughness: 0.62,
            ..default()
        }),
        grille: materials.add(StandardMaterial {
            base_color: Color::srgb(0.020, 0.021, 0.023),
            perceptual_roughness: 0.44,
            ..default()
        }),
        chrome: materials.add(StandardMaterial {
            base_color: Color::srgb(0.72, 0.73, 0.75),
            perceptual_roughness: 0.22,
            metallic: 1.0,
            ..default()
        }),
        mirror: materials.add(StandardMaterial {
            base_color: Color::srgb(0.55, 0.58, 0.62),
            perceptual_roughness: 0.06,
            metallic: 1.0,
            ..default()
        }),
    }
}

/// Where the fittings go on one archetype, in body-local metres.
///
/// Every field is read off the profile rather than tuned per class. That is not
/// tidiness: the profiles are still being retuned, and a table of hand-placed
/// mounts would drift out of agreement with them silently — a headlight
/// floating a hand's breadth off a nose is exactly the sort of error that
/// survives, because it looks like a design decision until you go and measure.
#[derive(Debug, Clone, Copy)]
pub struct Fittings {
    /// The station just behind the nose tip, where the bodywork is wide enough
    /// to carry a lamp. The tip itself is a dome and nothing fits on it.
    pub nose: f32,
    pub tail: f32,
    /// Height and half-width of the lamp band at those stations.
    pub lamp_y: f32,
    pub lamp_x: f32,
    /// How wide one lamp is. Shared with `lights`, which draws them, and with
    /// the grille, which has to stop before they start.
    pub lamp_width: f32,
    /// The bumper band, below the lamps and above the valance.
    pub bumper_y: f32,
    pub bumper_half: Vec2,
    /// Half-width and half-height of the grille, centred between the lamps.
    pub grille_half: Vec2,
    /// Mirror mount: at the front of the greenhouse, on its widest line.
    pub mirror: Vec3,
    /// Where a number plate hangs, front and back.
    pub plate_y: f32,
    /// Tailpipe, offset to the side the exhaust runs down.
    pub exhaust: Vec3,
}

/// Fraction of the way up a nose section that a headlight sits.
///
/// Above the middle, where a headlight goes: mid-height is where a rounded box
/// is at its very widest, so a lamp mounted there is flush with the flank and
/// its outer corner has nowhere left to go.
const LAMP_HEIGHT: f32 = 0.64;

impl Fittings {
    pub fn of(class: VehicleClass, spec: &VehicleSpec) -> Self {
        let profile = profile(class);
        let scale = spec.half_extents * Vec3::new(super::body::BODY_INSET, 1.0, 1.0);
        let shell = &profile.shell;
        // One station in from each end. The end sections are the rim of a
        // domed cap: narrow, short, and curving away from anything mounted on
        // them, which is how the lamps used to end up hanging in mid-air
        // beside the car instead of set into it.
        let front = shell[1];
        let back = shell[shell.len() - 2];

        let (nose, lamp_y, lamp_x) = station(&front, scale, LAMP_HEIGHT);
        let (tail, _, _) = station(&back, scale, LAMP_HEIGHT);

        // The bumper hangs under the lamps, in the band between the shell's
        // floor and the valance the sill loft already puts at the nose.
        let (_, bumper_y, bumper_x) = station(&front, scale, 0.16);
        let valance = profile.lower[0].top * scale.y;

        // The mirror mounts where the greenhouse meets the body, and that is a
        // point on the *shell* — a cabin section is much narrower than the
        // flank it is set into, so a mirror placed on the cabin's own ring ends
        // up a hand's breadth inside the door it is supposed to be bolted to.
        let cowl = profile.cabin.first().copied().unwrap_or(front);
        let flank = section_where(shell, cowl.at);
        let waist = ((cowl.bottom - flank.bottom) / (flank.top - flank.bottom)).clamp(0.1, 0.95);
        let (mirror_z, mirror_y, mirror_x) = station(&flank, scale, waist);

        let lamp_width = lamp_x * 0.44;

        Self {
            nose,
            tail,
            lamp_y,
            lamp_x,
            lamp_width,
            bumper_y: bumper_y.min(valance),
            bumper_half: Vec2::new(bumper_x, (lamp_y - bumper_y).abs() * 0.42),
            // Between the two lamps, which means stopping at their *inner*
            // edge and not at the point they are centred on. Measured against
            // the centre, a grille overlaps the headlights by most of a lamp
            // and the two fight over the same few centimetres of nose.
            grille_half: Vec2::new(
                (lamp_x - lamp_width) * 0.88,
                (lamp_y - bumper_y).abs() * 0.40,
            ),
            mirror: Vec3::new(mirror_x, mirror_y, mirror_z),
            plate_y: bumper_y * 0.55 + lamp_y * 0.45,
            exhaust: Vec3::new(
                spec.half_extents.x * 0.42,
                valance.min(bumper_y) + 0.02,
                spec.half_extents.z * 0.98,
            ),
        }
    }
}

/// The z, height and half-width of one cross-section at a fraction of its own
/// height — the three numbers any fitting mounted on that section needs.
fn station(section: &Section, scale: Vec3, up: f32) -> (f32, f32, f32) {
    let centre = (section.top + section.bottom) * 0.5 * scale.y;
    let half_height = (section.top - section.bottom) * 0.5 * scale.y;
    let y = centre + half_height * (up * 2.0 - 1.0);
    // Invert the superellipse for the half-width at that height, so a part
    // mounted here lands *on* the bodywork rather than beside it.
    let power = 2.0 / section.squareness.max(2.0);
    let rise = ((y - centre) / half_height.max(1e-4)).abs().clamp(0.0, 1.0);
    let sin = rise.powf(1.0 / power);
    let cos = (1.0 - sin * sin).max(0.0).sqrt();
    let x = section.half_width * scale.x * cos.powf(power);
    ((section.at * 2.0 - 1.0) * scale.z, y, x)
}

/// A dashboard, seats and a wheel, hung inside the glass.
///
/// Placed from the greenhouse rather than from the collider, because the
/// greenhouse is what you look through: a seat back positioned off the box
/// would be right on a saloon and buried in the floor of a wedge.
pub fn furnish(
    parent: &mut RelatedSpawnerCommands<ChildOf>,
    kit: &TrimKit,
    class: VehicleClass,
    spec: &VehicleSpec,
) {
    let profile = profile(class);
    let Some(&front) = profile.cabin.first() else {
        // A van's cab is inside its one volume, and its glazing is opaque; there
        // is nothing to see in there to furnish.
        return;
    };
    let cabin = &profile.cabin;
    let scale = spec.half_extents * Vec3::new(super::body::BODY_INSET, 1.0, 1.0);

    let back = cabin[cabin.len() - 1];
    let (nose, tail) = (
        (front.at * 2.0 - 1.0) * scale.z,
        (back.at * 2.0 - 1.0) * scale.z,
    );
    let floor = cabin.iter().map(|s| s.bottom).fold(f32::MAX, f32::min) * scale.y;
    let width = cabin.iter().map(|s| s.half_width).fold(0.0, f32::max) * scale.x;

    let seat_x = width * 0.44;
    let seat = Vec3::new(width * 0.52, 0.46, 0.10);
    // Driver on the kerb-side of the centreline, which is the opposite side to
    // the one the traffic keeps.
    let driver = if RIGHT_HAND_TRAFFIC { -seat_x } else { seat_x };

    let mut upholster = |at: Vec3, size: Vec3| {
        parent.spawn((
            Mesh3d(kit.block.clone()),
            MeshMaterial3d(kit.upholstery.clone()),
            Transform::from_translation(at).with_scale(size),
        ));
    };

    for row in [0.34f32, 0.74] {
        let z = nose.lerp(tail, row);
        for side in [-seat_x, seat_x] {
            upholster(Vec3::new(side, floor + seat.y * 0.5, z), seat);
            // The head restraint. Small, and the single strongest cue that
            // there is an inside at all: it is the one thing in a car that
            // breaks the line of the glass from outside.
            upholster(
                Vec3::new(side, floor + seat.y + 0.055, z + 0.01),
                Vec3::new(seat.x * 0.52, 0.13, 0.085),
            );
        }
    }

    // Dashboard, filling the width under the screen.
    let dash = nose.lerp(tail, 0.16);
    upholster(
        Vec3::new(0.0, floor + 0.14, dash),
        Vec3::new(width * 1.7, 0.22, 0.26),
    );

    parent.spawn((
        Mesh3d(kit.wheel.clone()),
        MeshMaterial3d(kit.upholstery.clone()),
        Transform::from_xyz(driver, floor + 0.30, dash + 0.20)
            // Laid back the way a wheel is, rather than standing upright like a
            // ship's helm. The torus is built in the XZ plane, so it starts flat
            // and is stood up from there.
            .with_rotation(Quat::from_rotation_x(1.20)),
    ));
}

/// Bumpers, grille, mirrors and a tailpipe.
///
/// `paint` is the car's own body colour, which the mirror shells and the boot
/// plinth wear; everything else is shared trim.
pub fn fit(
    parent: &mut RelatedSpawnerCommands<ChildOf>,
    kit: &TrimKit,
    class: VehicleClass,
    spec: &VehicleSpec,
    paint: &Handle<StandardMaterial>,
) {
    let f = Fittings::of(class, spec);

    let mut bolt = |material: &Handle<StandardMaterial>, at: Vec3, size: Vec3| {
        parent.spawn((
            Mesh3d(kit.block.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_translation(at).with_scale(size),
        ));
    };

    // Bumpers. Set a little proud of the station they are measured at, which is
    // what a bumper is for.
    for (z, depth) in [(f.nose, -0.06), (f.tail, 0.06)] {
        bolt(
            &kit.trim,
            Vec3::new(0.0, f.bumper_y, z + depth),
            Vec3::new(f.bumper_half.x * 1.92, f.bumper_half.y * 2.0, 0.14),
        );
    }

    // Grille, between the lamps.
    bolt(
        &kit.grille,
        Vec3::new(0.0, f.lamp_y, f.nose - 0.05),
        Vec3::new(f.grille_half.x * 2.0, f.grille_half.y * 2.0, 0.05),
    );

    // Mirrors: a stalk out of the body and a shell on the end of it, with the
    // glass itself facing back down the flank.
    for side in [-1.0f32, 1.0] {
        let root = Vec3::new(side * f.mirror.x, f.mirror.y, f.mirror.z);
        let out = Vec3::new(side * 0.10, -0.03, 0.0);
        bolt(&kit.trim, root + out * 0.4, Vec3::new(0.09, 0.035, 0.035));
        bolt(&kit.trim, root + out, Vec3::new(0.075, 0.085, 0.15));
        bolt(
            &kit.mirror,
            root + out + Vec3::new(side * 0.038, 0.0, 0.008),
            Vec3::new(0.008, 0.062, 0.115),
        );
    }

    // Tailpipe, one side only. A pair reads as a car pretending to be quick,
    // and most of these are not.
    parent.spawn((
        Mesh3d(kit.pipe.clone()),
        MeshMaterial3d(kit.chrome.clone()),
        Transform::from_translation(f.exhaust)
            .with_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
            .with_scale(Vec3::new(0.055, 0.16, 0.055)),
    ));

    // Keeps the signature honest while the plate itself is still to come: the
    // body colour is already used by the mirror shells on classes that have a
    // painted one.
    let _ = paint;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_fitting_lands_on_the_car() {
        // The failure this exists for is a lamp or a mirror hanging in the air
        // beside the bodywork, which is what happens the moment a mount is
        // typed in rather than measured off the profile.
        for class in VehicleClass::ALL {
            let spec = class.spec();
            let f = Fittings::of(class, &spec);
            let half = spec.half_extents;

            for (name, point) in [
                ("nose lamp", Vec3::new(f.lamp_x, f.lamp_y, f.nose)),
                ("bumper", Vec3::new(f.bumper_half.x, f.bumper_y, f.nose)),
                ("mirror", f.mirror),
                ("exhaust", f.exhaust),
            ] {
                assert!(
                    point.x.abs() <= half.x + 1e-3
                        && point.y.abs() <= half.y + 1e-3
                        && point.z.abs() <= half.z + 1e-3,
                    "{}: {name} at {point:?} is outside the collider {half:?}",
                    spec.display_name
                );
            }
        }
    }

    #[test]
    fn a_lamp_is_set_into_the_nose_rather_than_bolted_beside_it() {
        // What actually went wrong before this was derived: a lamp pinned to
        // the collider sat at a height the nose does not reach and a width it
        // is not, and hung in the air off the corner of the car. So the test is
        // that the mount lies on the cross-section it is mounted to — inside
        // its height, and no wider than the bodywork is at that height.
        for class in VehicleClass::ALL {
            let spec = class.spec();
            let profile = profile(class);
            let scale = spec.half_extents * Vec3::new(super::super::body::BODY_INSET, 1.0, 1.0);
            let nose = profile.shell[1];
            let (floor, ceiling) = (nose.bottom * scale.y, nose.top * scale.y);
            let widest = nose.half_width * scale.x;
            let f = Fittings::of(class, &spec);

            assert!(
                f.lamp_y > floor && f.lamp_y < ceiling,
                "{}: lamp at {:.3}m is outside the nose, which spans {floor:.3}..{ceiling:.3}m",
                spec.display_name,
                f.lamp_y
            );
            assert!(
                f.lamp_x <= widest + 1e-4 && f.lamp_x > widest * 0.4,
                "{}: lamp at {:.3}m does not sit on a {widest:.3}m nose",
                spec.display_name,
                f.lamp_x
            );
            // And above the middle of it, which is where a headlight goes and
            // also the only place the width derivation can bite.
            assert!(
                f.lamp_y > (floor + ceiling) * 0.5,
                "{}: the lamp has slid into the bumper",
                spec.display_name
            );
        }
    }

    #[test]
    fn a_grille_fits_between_the_lamps() {
        for class in VehicleClass::ALL {
            let spec = class.spec();
            let f = Fittings::of(class, &spec);
            assert!(
                f.grille_half.x > 0.05,
                "{}: grille is {:.3}m wide",
                spec.display_name,
                f.grille_half.x * 2.0
            );
            // Against the lamp's inner edge, not its centre. Compared with the
            // centre the assertion passes while the grille is still overlapping
            // most of the headlight.
            let inner = f.lamp_x - f.lamp_width;
            assert!(
                f.grille_half.x < inner,
                "{}: grille reaches {:.3}m, into a lamp that starts at {inner:.3}m",
                spec.display_name,
                f.grille_half.x
            );
        }
    }

    #[test]
    fn a_mirror_is_bolted_to_the_flank_and_not_buried_in_it() {
        // The mount is taken off the shell at the cabin's own station, so it
        // has to be out at the shell's width rather than at the much narrower
        // greenhouse's — a mirror on the cabin's ring is inside the door.
        for class in VehicleClass::ALL {
            let spec = class.spec();
            let profile = profile(class);
            let Some(cowl) = profile.cabin.first() else {
                continue;
            };
            let scale = spec.half_extents * Vec3::new(super::super::body::BODY_INSET, 1.0, 1.0);
            let cabin_width = cowl.half_width * scale.x;
            let f = Fittings::of(class, &spec);
            assert!(
                f.mirror.x > cabin_width,
                "{}: mirror at {:.3}m is inside a greenhouse {cabin_width:.3}m wide",
                spec.display_name,
                f.mirror.x
            );
        }
    }

    #[test]
    fn the_bumper_hangs_below_the_lamps() {
        for class in VehicleClass::ALL {
            let spec = class.spec();
            let f = Fittings::of(class, &spec);
            assert!(
                f.bumper_y < f.lamp_y,
                "{}: the bumper is above the headlights",
                spec.display_name
            );
            assert!(
                f.plate_y < f.lamp_y && f.plate_y > f.bumper_y,
                "{}: the plate is not between the bumper and the lamps",
                spec.display_name
            );
        }
    }
}
