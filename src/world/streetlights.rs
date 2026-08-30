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
#[derive(Resource, Default)]
pub struct LampPosts(pub Vec<Vec2>);

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
                posts.push(along + normal * offset * side);
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
    let head = meshes.add(Sphere::new(0.35));
    let glass = materials.add(StandardMaterial {
        base_color: LAMP_COLOR,
        emissive: LinearRgba::BLACK,
        ..default()
    });
    commands.insert_resource(LampGlass(glass.clone()));

    for i in 0..POOL_SIZE {
        commands.spawn((
            Name::new(format!("Street Light {i}")),
            StreetLight,
            PointLight {
                color: LAMP_COLOR,
                intensity: 0.0,
                range: 46.0,
                shadow_maps_enabled: false,
                ..default()
            },
            // Parked far below the world until assigned a lamp post.
            Transform::from_xyz(0.0, -1000.0, 0.0),
            children![(
                Mesh3d(head.clone()),
                MeshMaterial3d(glass.clone()),
                Transform::default(),
            )],
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
    let mut nearest: Vec<(f32, Vec2)> = posts
        .0
        .iter()
        .map(|&p| (p.distance_squared(focus), p))
        .collect();
    // Only the closest POOL_SIZE matter; a full sort would be wasted work.
    let take = POOL_SIZE.min(nearest.len());
    nearest.select_nth_unstable_by(take.saturating_sub(1), |a, b| a.0.total_cmp(&b.0));

    for (mut transform, (_, pos)) in lamps.iter_mut().zip(nearest.iter().take(take)) {
        transform.translation = Vec3::new(pos.x, LAMP_HEIGHT, pos.y);
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
    let intensity = 420_000.0 * night;
    for mut lamp in &mut lamps {
        lamp.intensity = intensity;
    }
    if let Some(mut material) = materials.get_mut(&glass.0) {
        material.emissive = LinearRgba::rgb(6.0 * night, 4.4 * night, 2.4 * night);
    }
}
