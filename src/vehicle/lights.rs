//! Head and tail lights.
//!
//! Two separate things wear the name "light" here, and keeping them apart is
//! what keeps this cheap:
//!
//! * **Lamps** are emissive boxes bolted to the body. They cost nothing beyond
//!   the draw call they batch into, every car has them, and they are what you
//!   actually see — a pair of headlights coming the other way at night reads as
//!   headlights because of the glare, not because of what they illuminate.
//! * **Beams** are real [`SpotLight`]s. They are the expensive half, so only
//!   cars with a driver get one, and only after dark.
//!
//! Lamp materials are shared across every vehicle in the city, which is why the
//! day/night ramp is four material writes rather than four per car. The cost of
//! that sharing is that brake lights cannot vary per car through the material —
//! so braking swaps the child's material handle instead of editing a material.

use bevy::ecs::relationship::RelatedSpawnerCommands;
use bevy::prelude::*;

use super::controller::VehicleInput;
use super::spec::VehicleSpec;
use crate::core::schedule::GameSet;
use crate::player::interact::DrivenBy;
use crate::world::timeofday::{TimeOfDay, daylight};

/// Full-beam output of one car, in lumens.
const BEAM_INTENSITY: f32 = 900_000.0;
/// How far a beam reaches before it is cut off.
const BEAM_RANGE: f32 = 70.0;
/// A headlight lens. Purely visual.
#[derive(Component)]
pub struct Headlight;

/// A tail lamp, which doubles as the brake light.
#[derive(Component)]
pub struct TailLamp;

/// The spot light a driven car throws down the road.
#[derive(Component)]
pub struct HeadlightBeam;

/// Shared meshes and materials for every lamp in the city.
#[derive(Resource)]
pub struct LightAssets {
    lens: Handle<Mesh>,
    /// What a lens looks like switched off. Every car is built wearing these,
    /// and only cars with a driver ever trade them for the lit versions — a
    /// street of parked cars with their headlights blazing is worse than a
    /// street of parked cars with no lights modelled at all.
    dark_glass: Handle<StandardMaterial>,
    dark_lamp: Handle<StandardMaterial>,
    headlight: Handle<StandardMaterial>,
    tail: Handle<StandardMaterial>,
    /// Swapped in for [`Self::tail`] under braking.
    brake: Handle<StandardMaterial>,
}

/// A lens that is off has to stay a plausible object in daylight, so each of
/// these carries a sensible unlit colour as well as its glow.
fn lens_material(base: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: base,
        emissive: LinearRgba::BLACK,
        perceptual_roughness: 0.22,
        ..default()
    }
}

pub fn build_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> LightAssets {
    LightAssets {
        lens: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        dark_glass: materials.add(lens_material(Color::srgb(0.72, 0.74, 0.76))),
        dark_lamp: materials.add(lens_material(Color::srgb(0.38, 0.05, 0.05))),
        headlight: materials.add(lens_material(Color::srgb(0.92, 0.92, 0.86))),
        tail: materials.add(lens_material(Color::srgb(0.42, 0.06, 0.06))),
        brake: materials.add(lens_material(Color::srgb(0.55, 0.08, 0.08))),
    }
}

/// Bolts the lamps onto every car the frame after it is built.
///
/// A spawn-time hook would need threading through `spawn_vehicle` and its three
/// callers; reacting to `Added` costs one frame nobody can see — lamps are dark
/// in daylight and a car spawned at night is far away when it appears.
fn attach_lamps(
    mut commands: Commands,
    assets: Res<LightAssets>,
    added: Query<(Entity, &VehicleSpec), Added<super::spawn::Vehicle>>,
) {
    for (vehicle, spec) in &added {
        commands.entity(vehicle).with_children(|parent| {
            lamps_for(parent, &assets, spec);
        });
    }
}

fn lamps_for(
    parent: &mut RelatedSpawnerCommands<ChildOf>,
    assets: &LightAssets,
    spec: &VehicleSpec,
) {
    let half = spec.half_extents;
    // Measured off the profile the bodywork was lofted from, not off the
    // collider. Pinned to the box, a lamp sat at a height and a width the nose
    // does not reach at that station and hung in the air beside the car — which
    // is a thing you only notice once you look at a screenshot, because it
    // reads as a styling choice until you measure it.
    let fit = super::trim::Fittings::of(spec.class, spec);
    let width = fit.lamp_width;

    for side in [-1.0f32, 1.0] {
        // Set half a lamp in from the widest point the nose reaches at this
        // height, so the outer edge lands on the bodywork rather than past it.
        let x = side * (fit.lamp_x - width * 0.5);
        parent.spawn((
            Headlight,
            Mesh3d(assets.lens.clone()),
            MeshMaterial3d(assets.dark_glass.clone()),
            Transform::from_xyz(x, fit.lamp_y, fit.nose - 0.03).with_scale(Vec3::new(
                width,
                half.y * 0.22,
                0.07,
            )),
        ));
        parent.spawn((
            TailLamp,
            Mesh3d(assets.lens.clone()),
            MeshMaterial3d(assets.dark_lamp.clone()),
            Transform::from_xyz(x, fit.lamp_y, fit.tail + 0.03).with_scale(Vec3::new(
                width * 0.88,
                half.y * 0.19,
                0.07,
            )),
        ));
    }
}

pub struct VehicleLightsPlugin;

impl Plugin for VehicleLightsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                attach_lamps,
                set_lamp_glow,
                switch_beams,
                switch_driven_lamps,
            )
                .in_set(GameSet::Simulation),
        );
    }
}

/// Whether the lamps should be lit at this hour, ramped through dusk and dawn.
fn night_factor(hours: f32) -> f32 {
    // Lamps come on a little before the sun is properly down, the way real
    // drivers switch them on: at the point the light gets flat, not the point
    // it disappears.
    (1.0 - daylight(hours) * 1.35).clamp(0.0, 1.0)
}

fn set_lamp_glow(
    clock: Res<TimeOfDay>,
    assets: Res<LightAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let night = night_factor(clock.hours);
    let mut set = |handle: &Handle<StandardMaterial>, glow: LinearRgba| {
        if let Some(mut material) = materials.get_mut(handle) {
            let wanted = LinearRgba::rgb(glow.red * night, glow.green * night, glow.blue * night);
            if material.emissive != wanted {
                material.emissive = wanted;
            }
        }
    };

    set(&assets.headlight, LinearRgba::rgb(24.0, 23.0, 19.0));
    // Tail lamps are dim next to a headlight, and have to be: matched, a car
    // driving away looks exactly like a car driving towards you.
    set(&assets.tail, LinearRgba::rgb(4.5, 0.25, 0.15));
    set(&assets.brake, LinearRgba::rgb(22.0, 1.2, 0.6));
}

/// Gives a beam to cars with someone at the wheel, and takes it back at dawn.
///
/// Parked cars are excluded on purpose. There are several hundred of them and
/// they are not going anywhere, so a beam each would be several hundred spot
/// lights lighting the inside of a parked car's own bumper.
fn switch_beams(
    mut commands: Commands,
    clock: Res<TimeOfDay>,
    driven: Query<
        (Entity, &VehicleSpec, Option<&Children>),
        Or<(With<DrivenBy>, With<super::spawn::AlwaysSimulated>)>,
    >,
    mut beams: Query<&mut SpotLight, With<HeadlightBeam>>,
    existing: Query<&HeadlightBeam>,
) {
    let night = night_factor(clock.hours);

    for (vehicle, spec, children) in &driven {
        let beam = children
            .into_iter()
            .flatten()
            .copied()
            .find(|&child| existing.contains(child));

        match (night > 0.01, beam) {
            (true, Some(beam)) => {
                if let Ok(mut light) = beams.get_mut(beam) {
                    light.intensity = BEAM_INTENSITY * night;
                }
            }
            (true, None) => {
                let half = spec.half_extents;
                commands.entity(vehicle).with_child((
                    HeadlightBeam,
                    SpotLight {
                        color: Color::srgb(1.0, 0.97, 0.90),
                        intensity: BEAM_INTENSITY * night,
                        range: BEAM_RANGE,
                        inner_angle: 0.22,
                        outer_angle: 0.62,
                        // Shadowed spot lights on every car in a chase is the
                        // one thing here that would actually cost frames.
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    // Spot lights fire along -Z, which is also the car's
                    // forward; the pitch is the dip that keeps the beam on the
                    // road instead of in oncoming windscreens.
                    Transform::from_xyz(0.0, half.y * 0.1, -half.z)
                        .with_rotation(Quat::from_rotation_x(-0.16)),
                ));
            }
            (false, Some(beam)) => commands.entity(beam).despawn(),
            (false, None) => {}
        }
    }
}

/// Points each lens on a driven car at the material it should be wearing.
///
/// Materials are shared city-wide, so per-car state cannot live in the material
/// — it lives in which handle the lens points at. Three states per car and a
/// handle comparison to avoid writing when nothing changed.
fn switch_driven_lamps(
    clock: Res<TimeOfDay>,
    assets: Res<LightAssets>,
    driven: Query<
        (&VehicleInput, &Children),
        Or<(With<DrivenBy>, With<super::spawn::AlwaysSimulated>)>,
    >,
    mut heads: Query<&mut MeshMaterial3d<StandardMaterial>, (With<Headlight>, Without<TailLamp>)>,
    mut tails: Query<&mut MeshMaterial3d<StandardMaterial>, (With<TailLamp>, Without<Headlight>)>,
) {
    let lit = night_factor(clock.hours) > 0.01;

    for (input, children) in &driven {
        // Brake pedal or handbrake. Reversing counts too, which is wrong on a
        // real car and right here: the AI reverses to unwedge itself, and the
        // flare is the only warning the player gets that it is about to.
        let braking = input.throttle < -0.05 || input.handbrake;

        let head = if lit {
            &assets.headlight
        } else {
            &assets.dark_glass
        };
        let tail = match (braking, lit) {
            (true, _) => &assets.brake,
            (false, true) => &assets.tail,
            (false, false) => &assets.dark_lamp,
        };

        for &child in children {
            if let Ok(mut material) = heads.get_mut(child)
                && material.0.id() != head.id()
            {
                material.0 = head.clone();
            }
            if let Ok(mut material) = tails.get_mut(child)
                && material.0.id() != tail.id()
            {
                material.0 = tail.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lamps_are_out_in_daylight_and_lit_at_night() {
        assert_eq!(night_factor(12.0), 0.0, "noon");
        assert_eq!(night_factor(1.0), 1.0, "the small hours");
    }

    #[test]
    fn lamps_come_on_before_the_sun_is_fully_down() {
        // Sunset is at 18:00. There should be light on the lenses before then.
        assert!(
            night_factor(17.6) > 0.0,
            "headlights should be on while it is still technically day"
        );
        assert!(night_factor(17.6) < 1.0, "but not yet at full");
    }
}
