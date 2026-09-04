//! Street lighting.
//!
//! Rather than spawning a lamp per intersection (hundreds of live point lights,
//! most of them nowhere near the player), this keeps a small fixed pool and
//! snaps it to whichever intersections are currently closest. Cost is constant
//! and predictable no matter how large the city grows.
//!
//! Shadows are off for these deliberately: shadow-casting point lights are one
//! of the most expensive things in a renderer, and at night a pool of unshadowed
//! pools of light is what sells the look anyway.

use bevy::prelude::*;

use super::City;
use super::timeofday::{TimeOfDay, daylight};

/// How many lamps are live at once.
const POOL_SIZE: usize = 64;
const LAMP_HEIGHT: f32 = 7.5;
/// How far the lamp head reaches out over the road from its column.
const ARM_REACH: f32 = 1.5;
/// Distance between lamp posts along a street.
const LAMP_SPACING: f32 = 32.0;
/// Sodium-vapour warmth.
const LAMP_COLOR: Color = Color::srgb(1.0, 0.82, 0.55);

#[derive(Component)]
pub struct StreetLight;

/// Every possible lamp post position, precomputed from the road graph.
/// Posts run along the kerbs at a fixed spacing and alternate sides, because
/// lighting only the intersections leaves the 60-90m of road between them
/// pitch black — which is most of the road.
/// Where a lamp stands, and which way it leans out over the road.
#[derive(Clone, Copy)]
pub struct LampPost {
    /// The column's foot, just inside the kerb.
    pub foot: Vec2,
    /// Unit vector from the kerb towards the middle of the road.
    pub inward: Vec2,
}

#[derive(Resource, Default)]
pub struct LampPosts(pub Vec<LampPost>);

impl LampPosts {
    pub fn build(city: &City) -> Self {
        let graph = &city.graph;
        let mut posts = Vec::new();

        for edge in graph.edges() {
            let a = graph.node(edge.a).pos;
            let b = graph.node(edge.b).pos;
            let Ok(dir) = Dir2::new(b - a) else { continue };
            let normal = Vec2::new(-dir.y, dir.x);
            // Just inside the kerb line.
            let offset = edge.width * 0.5 - 0.9;

            let count = (edge.length / LAMP_SPACING).floor() as i32;
            for i in 1..count {
                let along = a + *dir * (i as f32 * LAMP_SPACING);
                let side = if i % 2 == 0 { 1.0 } else { -1.0 };
                posts.push(LampPost {
                    foot: along + normal * offset * side,
                    // Whichever kerb it stands on, the arm reaches the other
                    // way — out over the carriageway.
                    inward: -normal * side,
                });
            }
        }

        Self(posts)
    }
}

/// Shared glass material, dimmed with the lamps themselves.
#[derive(Resource)]
struct LampGlass(Handle<StandardMaterial>);

#[derive(Resource)]
struct LampTimer(Timer);

impl Default for LampTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.3, TimerMode::Repeating))
    }
}

pub struct StreetLightPlugin;

impl Plugin for StreetLightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LampTimer>()
            .init_resource::<LampPosts>()
            .add_systems(Startup, spawn_pool)
            .add_systems(Update, (reposition_lamps, set_lamp_brightness));
    }
}

fn spawn_pool(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // A visible glowing head, so the light has an apparent source.
    let head = meshes.add(Sphere::new(0.30));
    let glass = materials.add(StandardMaterial {
        base_color: LAMP_COLOR,
        emissive: LinearRgba::BLACK,
        ..default()
    });
    commands.insert_resource(LampGlass(glass.clone()));

    // A lamp needs something holding it up. Until now these were glowing
    // spheres floating at seven and a half metres, which reads as a bug at
    // dusk and as nothing at all in daylight — the pool of light on the road
    // had no visible cause.
    let column = meshes.add(Cylinder::new(0.075, LAMP_HEIGHT));
    let arm = meshes.add(Cylinder::new(0.055, ARM_REACH));
    let steel = materials.add(StandardMaterial {
        base_color: Color::srgb(0.20, 0.21, 0.22),
        perceptual_roughness: 0.62,
        metallic: 0.75,
        ..default()
    });

    for i in 0..POOL_SIZE {
        commands.spawn((
            Name::new(format!("Street Light {i}")),
            StreetLight,
            PointLight {
                color: LAMP_COLOR,
                intensity: 0.0,
                range: 62.0,
                shadow_maps_enabled: false,
                ..default()
            },
            // Parked far below the world until assigned a lamp post.
            Transform::from_xyz(0.0, -1000.0, 0.0),
            children![
                (
                    Mesh3d(head.clone()),
                    MeshMaterial3d(glass.clone()),
                    Transform::default(),
                ),
                // The column stands under the light, not under the entity: the
                // lamp head is what gets positioned, and the pole hangs off it
                // reaching back to the kerb.
                (
                    Mesh3d(column.clone()),
                    MeshMaterial3d(steel.clone()),
                    Transform::from_xyz(ARM_REACH, -LAMP_HEIGHT * 0.5, 0.0),
                ),
                (
                    Mesh3d(arm.clone()),
                    MeshMaterial3d(steel.clone()),
                    // Cylinders run along Y; lay it across to the column.
                    Transform::from_xyz(ARM_REACH * 0.5, 0.0, 0.0)
                        .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                ),
            ],
        ));
    }
}

/// Snaps the pool onto the nearest intersections to the camera.
fn reposition_lamps(
    time: Res<Time>,
    mut timer: ResMut<LampTimer>,
    city: Option<Res<City>>,
    mut posts: ResMut<LampPosts>,
    cameras: Query<&GlobalTransform, With<crate::player::camera::CameraRig>>,
    mut lamps: Query<&mut Transform, With<StreetLight>>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let (Some(city), Ok(camera)) = (city, cameras.single()) else {
        return;
    };
    if posts.0.is_empty() {
        *posts = LampPosts::build(&city);
        info!("{} lamp posts along the street network", posts.0.len());
    }

    let focus = camera.translation().xz();
    let mut nearest: Vec<(f32, LampPost)> = posts
        .0
        .iter()
        .map(|&p| (p.foot.distance_squared(focus), p))
        .collect();
    // Only the closest POOL_SIZE matter; a full sort would be wasted work.
    let take = POOL_SIZE.min(nearest.len());
    nearest.select_nth_unstable_by(take.saturating_sub(1), |a, b| a.0.total_cmp(&b.0));

    for (mut transform, (_, post)) in lamps.iter_mut().zip(nearest.iter().take(take)) {
        // The entity *is* the lamp head, out over the road; the column and arm
        // hang off it back towards the kerb. Yaw is set so the lamp's local +X
        // points that way, which is where those two children sit.
        let head = post.foot + post.inward * ARM_REACH;
        transform.translation = Vec3::new(head.x, LAMP_HEIGHT, head.y);
        transform.rotation = Quat::from_rotation_y(post.inward.y.atan2(-post.inward.x));
    }
}

fn set_lamp_brightness(
    clock: Res<TimeOfDay>,
    glass: Res<LampGlass>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut lamps: Query<&mut PointLight, With<StreetLight>>,
) {
    // Lamps come up through dusk and go out through dawn.
    let night = 1.0 - daylight(clock.hours);
    // Quoted so a lamp lays down a pool the road actually reads at the night
    // exposure — see `render::adapt_exposure`. Physically this is a floodlight rather
    // than a street lamp, which is the usual bargain: real sodium lamps look
    // like nothing at all once the camera has opened up for a moonlit sky.
    let intensity = 1_250_000.0 * night;
    for mut lamp in &mut lamps {
        lamp.intensity = intensity;
    }
    if let Some(mut material) = materials.get_mut(&glass.0) {
        material.emissive = LinearRgba::rgb(13.0 * night, 9.4 * night, 5.0 * night);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a lamp's column ends up on the ground, given its post.
    fn column_foot(post: LampPost) -> Vec2 {
        let head = post.foot + post.inward * ARM_REACH;
        let yaw = Quat::from_rotation_y(post.inward.y.atan2(-post.inward.x));
        let arm = yaw * Vec3::new(ARM_REACH, 0.0, 0.0);
        Vec2::new(head.x + arm.x, head.y + arm.z)
    }

    #[test]
    fn the_column_lands_back_on_the_kerb_it_stands_on() {
        // The head is placed out over the road and the column is a child at a
        // fixed local offset, so the yaw is the only thing putting the column
        // back where the post says. Get it wrong and every lamp stands in the
        // middle of the road or inside the building behind it.
        for inward in [Vec2::X, Vec2::NEG_X, Vec2::Y, Vec2::NEG_Y] {
            let post = LampPost {
                foot: Vec2::new(12.0, -5.0),
                inward,
            };
            assert!(
                column_foot(post).distance(post.foot) < 1e-4,
                "with inward {inward:?} the column landed at {:?}, not {:?}",
                column_foot(post),
                post.foot
            );
        }
    }

    #[test]
    fn lamps_lean_out_over_the_road_from_both_kerbs() {
        // Posts alternate sides down a street, so both signs of `inward` have
        // to put the head *inside* the carriageway.
        for side in [1.0f32, -1.0] {
            let normal = Vec2::new(0.0, 1.0);
            let post = LampPost {
                foot: normal * 6.0 * side,
                inward: -normal * side,
            };
            let head = post.foot + post.inward * ARM_REACH;
            assert!(
                head.length() < post.foot.length(),
                "the head moved away from the centreline, not towards it"
            );
        }
    }
}
