//! Rain, and what it does to the ground.
//!
//! Two separate things, and the second is the one that matters. Falling rain is
//! a few thousand streaks around the camera and reads as weather. *Wet ground*
//! is what changes the picture: asphalt soaked through goes darker and far
//! glossier, and at the grazing angles a street is actually seen from it stops
//! being a surface and starts being a mirror for the sky and every lit window
//! above it.
//!
//! Part of that mirror comes free: the camera carries an environment map
//! generated from the atmosphere, so dropping a surface's roughness is enough
//! to reflect the sky. The rest is screen-space reflections, which need a
//! g-buffer and so were impossible while this renderer was forward. It is not
//! any more — see `render` — and the road now reflects the lit windows above it
//! rather than only the sky.
//!
//! What lives here is the *uniform* half: a pavement really does just go
//! evenly damp, so its material is recomputed from its dry values each time the
//! dial moves. The road does not — rain puddles — and its wetness moved into
//! `world::road`, where a shader varies it across the surface.
//!
//! Weather does not yet change on its own. The wetness is a dial, exposed to
//! the dev panel and to `--wet` for screenshots; a system that decides when it
//! rains is a separate thing from a renderer that can show it.

use bevy::prelude::*;
use rand::RngExt;

use crate::core::config::GameConfig;
use crate::core::rng::{stream, stream_for};
use crate::player::camera::CameraRig;

/// How many streaks are kept around the camera.
const DROPS: usize = 2600;
/// The box they fall inside, in metres.
const FIELD: Vec3 = Vec3::new(46.0, 26.0, 46.0);
/// Metres per second, straight down. Rain falls faster than this; drawn faster
/// than this it turns into a static hatch pattern.
const FALL_SPEED: f32 = 26.0;

/// One falling streak.
#[derive(Component)]
pub struct Raindrop;

/// A surface that changes when it is wet, and what it looks like dry.
///
/// The dry values are kept rather than recovered, because wetness is applied by
/// recomputing from them every time it changes. Nudging a material in place
/// instead drifts: dry it out and it never comes back to where it started.
pub struct WetSurface {
    pub material: Handle<StandardMaterial>,
    pub dry_color: Color,
    pub dry_roughness: f32,
}

#[derive(Resource, Default)]
pub struct WetSurfaces(pub Vec<WetSurface>);

impl WetSurfaces {
    pub fn add(&mut self, material: Handle<StandardMaterial>, color: Color, roughness: f32) {
        self.0.push(WetSurface {
            material,
            dry_color: color,
            dry_roughness: roughness,
        });
    }
}

#[derive(Resource)]
pub struct RainAssets {
    streak: Handle<Mesh>,
    water: Handle<StandardMaterial>,
}

/// How dark and how glossy a surface goes when it is soaked.
///
/// Water fills the pores, so less light scatters back out: a wet road is about
/// two thirds the brightness of a dry one. The roughness floor is what stops it
/// becoming a perfect mirror, which reads as ice rather than as water.
fn soaked(dry_color: Color, dry_roughness: f32, wetness: f32) -> (Color, f32) {
    let dry = LinearRgba::from(dry_color);
    let darken = 1.0 - wetness * 0.52;
    let color = Color::LinearRgba(LinearRgba::rgb(
        dry.red * darken,
        dry.green * darken,
        dry.blue * darken,
    ));
    // Not lower than this. Water on asphalt still has the texture of the road
    // under it; a true mirror finish reads as sheet ice.
    (color, dry_roughness.lerp(0.15, wetness))
}

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WetSurfaces>()
            .add_systems(Startup, spawn_rain)
            .add_systems(Update, (wet_the_ground, fall, follow_camera));
    }
}

fn spawn_rain(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let assets = RainAssets {
        // A streak, not a drop. At any shutter speed the eye has, rain is a
        // line; drawing spheres gives hail.
        streak: meshes.add(Cuboid::new(0.007, 0.42, 0.007)),
        water: materials.add(StandardMaterial {
            // Faint. Unlit blending over a wet road at night is high contrast
            // already, and a drop passing near the camera at full strength
            // reads as a white rod hanging in the air.
            base_color: Color::srgba(0.72, 0.78, 0.86, 0.16),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        }),
    };

    let mut rng = stream_for(config.world_seed, stream::RAIN);
    for _ in 0..DROPS {
        let at = Vec3::new(
            rng.random_range(-FIELD.x..FIELD.x),
            rng.random_range(-FIELD.y..FIELD.y),
            rng.random_range(-FIELD.z..FIELD.z),
        );
        commands.spawn((
            Raindrop,
            Mesh3d(assets.streak.clone()),
            MeshMaterial3d(assets.water.clone()),
            Transform::from_translation(at),
            // Hidden until it rains; `wet_the_ground` turns the field on.
            Visibility::Hidden,
        ));
    }
    commands.insert_resource(assets);
}

/// Applies the current wetness to every surface that cares.
fn wet_the_ground(
    config: Res<GameConfig>,
    surfaces: Res<WetSurfaces>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut drops: Query<&mut Visibility, With<Raindrop>>,
    mut applied: Local<Option<f32>>,
) {
    let wetness = config.world.wetness.clamp(0.0, 1.0);
    if applied.is_some_and(|last: f32| (last - wetness).abs() < 0.005) {
        return;
    }
    *applied = Some(wetness);

    for surface in &surfaces.0 {
        let Some(mut material) = materials.get_mut(&surface.material) else {
            continue;
        };
        let (color, roughness) = soaked(surface.dry_color, surface.dry_roughness, wetness);
        material.base_color = color;
        material.perceptual_roughness = roughness;
    }

    // Rain only falls once the ground is properly wet; a glossy road under a
    // clear sky is a road that has just stopped raining on.
    let falling = if wetness > super::road::RAINING_ABOVE {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut drops {
        *visibility = falling;
    }
}

fn fall(
    time: Res<Time>,
    config: Res<GameConfig>,
    mut drops: Query<&mut Transform, With<Raindrop>>,
) {
    if config.world.wetness <= super::road::RAINING_ABOVE {
        return;
    }
    let step = FALL_SPEED * time.delta_secs();
    for mut transform in &mut drops {
        transform.translation.y -= step;
        // Wrapped rather than respawned. The field is a box that moves with the
        // camera, so a drop leaving the bottom is the same drop arriving at the
        // top — and the eye cannot follow one long enough to notice.
        if transform.translation.y < -FIELD.y {
            transform.translation.y += FIELD.y * 2.0;
        }
    }
}

/// Keeps the rain field centred on the camera.
///
/// Moved in whole metres rather than continuously, so drops do not slide
/// sideways with the camera — rain falls straight down regardless of how fast
/// you are driving through it, which is wrong in a gale and right in a city.
fn follow_camera(
    cameras: Query<&GlobalTransform, With<CameraRig>>,
    mut drops: Query<&mut Transform, With<Raindrop>>,
    mut centre: Local<Vec3>,
) {
    let Ok(camera) = cameras.single() else { return };
    let wanted = camera.translation().round();
    let shift = wanted - *centre;
    if shift == Vec3::ZERO {
        return;
    }
    *centre = wanted;
    for mut transform in &mut drops {
        transform.translation += shift;
        // Re-wrap into the box around the new centre.
        let local = transform.translation - wanted;
        transform.translation = wanted
            + Vec3::new(
                wrap(local.x, FIELD.x),
                wrap(local.y, FIELD.y),
                wrap(local.z, FIELD.z),
            );
    }
}

/// Folds `value` back into `-half..half`.
fn wrap(value: f32, half: f32) -> f32 {
    let span = half * 2.0;
    (value + half).rem_euclid(span) - half
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wet_ground_is_darker_and_glossier_than_dry() {
        let dry_color = Color::srgb(0.31, 0.31, 0.325);
        let (wet, roughness) = soaked(dry_color, 0.96, 1.0);

        let before = LinearRgba::from(dry_color).red;
        let after = LinearRgba::from(wet).red;
        assert!(after < before, "wet asphalt should be darker, not lighter");
        assert!(roughness < 0.25, "and glossy enough to reflect something");
        assert!(roughness > 0.1, "but not a mirror, which reads as ice");
    }

    #[test]
    fn dry_is_exactly_what_it_started_as() {
        // Wetness is recomputed from the stored dry values every time, so zero
        // has to be an exact round trip or the road creeps darker each time it
        // stops raining.
        let color = Color::srgb(0.5, 0.52, 0.55);
        let (back, roughness) = soaked(color, 0.87, 0.0);
        assert!((roughness - 0.87).abs() < 1e-6);
        assert!((LinearRgba::from(back).red - LinearRgba::from(color).red).abs() < 1e-6);
    }

    #[test]
    fn wrapping_keeps_a_drop_inside_its_field() {
        for value in [-97.0, -12.0, 0.0, 11.0, 240.0] {
            let wrapped = wrap(value, FIELD.y);
            assert!(
                (-FIELD.y..=FIELD.y).contains(&wrapped),
                "{value} wrapped to {wrapped}, outside the field"
            );
        }
    }

    #[test]
    fn wrapping_leaves_a_drop_already_inside_alone() {
        for value in [-FIELD.y + 0.1, -3.0, 0.0, 7.5, FIELD.y - 0.1] {
            assert!(
                (wrap(value, FIELD.y) - value).abs() < 1e-4,
                "{value} moved when it should not have"
            );
        }
    }
}
