//! Day/night cycle.
//!
//! Drives the sun, sky, fog and ambient fill from a single clock. Fog colour is
//! kept locked to the sky colour on purpose: if they diverge, distant buildings
//! fade into a band that does not match the horizon and the illusion collapses.

use std::f32::consts::PI;

use bevy::light::GlobalAmbientLight;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;

use crate::core::config::GameConfig;

#[derive(Resource, Debug, Clone)]
pub struct TimeOfDay {
    /// Hours since midnight, 0..24.
    pub hours: f32,
    pub paused: bool,
}

/// Marks the sun so the cycle can find it.
#[derive(Component)]
pub struct Sun;

/// -1 at midnight, 0 at sunrise/sunset, +1 at noon.
pub fn sun_elevation(hours: f32) -> f32 {
    ((hours - 6.0) / 12.0 * PI).sin()
}

/// 0 at night, 1 in full day, with a soft ramp through twilight.
pub fn daylight(hours: f32) -> f32 {
    let e = sun_elevation(hours);
    // Smoothstep over the first 15 degrees or so of elevation.
    (e / 0.25).clamp(0.0, 1.0).powf(0.6)
}

fn sky_color(hours: f32) -> Color {
    let day = daylight(hours);
    let e = sun_elevation(hours);
    // Twilight peaks when the sun is near the horizon.
    let dusk = (1.0 - (e.abs() / 0.30).min(1.0)) * day.max(0.15);

    let night = Vec3::new(0.035, 0.045, 0.085);
    let noon = Vec3::new(0.48, 0.62, 0.85);
    let horizon = Vec3::new(0.85, 0.45, 0.22);

    let base = night.lerp(noon, day);
    let c = base.lerp(horizon, dusk * 0.75);
    Color::srgb(c.x, c.y, c.z)
}

fn sun_color(hours: f32) -> Color {
    let e = sun_elevation(hours);
    let warmth = (1.0 - (e.abs() / 0.35).min(1.0)).clamp(0.0, 1.0);
    let white = Vec3::new(1.0, 0.98, 0.94);
    let amber = Vec3::new(1.0, 0.62, 0.34);
    let c = white.lerp(amber, warmth);
    Color::srgb(c.x, c.y, c.z)
}

pub struct TimeOfDayPlugin;

impl Plugin for TimeOfDayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_sun)
            .add_systems(Update, (attach_fog, advance_clock, apply_sky).chain());
    }
}

fn spawn_sun(mut commands: Commands, config: Res<GameConfig>) {
    commands.insert_resource(TimeOfDay {
        hours: config.world.start_hour,
        paused: false,
    });
    commands.spawn((
        Name::new("Sun"),
        Sun,
        DirectionalLight {
            illuminance: 12_000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::default(),
    ));
}

/// Every 3D camera gets fog; `apply_sky` then keeps it matched to the sky.
fn attach_fog(
    mut commands: Commands,
    cameras: Query<Entity, (With<crate::player::camera::CameraRig>, Without<DistanceFog>)>,
) {
    for camera in &cameras {
        commands.entity(camera).insert(DistanceFog {
            color: Color::BLACK,
            falloff: FogFalloff::Linear {
                start: 1.0,
                end: 2.0,
            },
            ..default()
        });
    }
}

fn advance_clock(time: Res<Time>, config: Res<GameConfig>, mut clock: ResMut<TimeOfDay>) {
    if clock.paused || config.world.day_length_seconds <= 0.0 {
        return;
    }
    let hours_per_second = 24.0 / config.world.day_length_seconds;
    clock.hours = (clock.hours + time.delta_secs() * hours_per_second).rem_euclid(24.0);
}

fn apply_sky(
    clock: Res<TimeOfDay>,
    config: Res<GameConfig>,
    mut clear: ResMut<ClearColor>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut sun: Query<(&mut Transform, &mut DirectionalLight), With<Sun>>,
    mut fog: Query<&mut DistanceFog>,
) {
    let hours = clock.hours;
    let day = daylight(hours);
    let sky = sky_color(hours);

    clear.0 = sky;

    // Ambient carries the night: without a floor, unlit faces go pure black and
    // the city becomes unreadable rather than dark.
    ambient.color = Color::srgb(0.35 + 0.30 * day, 0.42 + 0.30 * day, 0.58 + 0.22 * day);
    ambient.brightness = 180.0 + 1_750.0 * day;

    let angle = (hours - 6.0) / 12.0 * PI;
    let distance = 400.0;
    let position = Vec3::new(
        angle.cos() * distance,
        angle.sin() * distance,
        0.35 * distance,
    );

    for (mut transform, mut light) in &mut sun {
        // Keep the sun above the horizon even at night so shadow cascades stay
        // sane; brightness rather than position is what sells nightfall.
        let eye = if position.y < 20.0 {
            Vec3::new(position.x, 20.0, position.z)
        } else {
            position
        };
        *transform = Transform::from_translation(eye).looking_at(Vec3::ZERO, Vec3::Y);
        light.illuminance = 300.0 + 12_500.0 * day;
        light.color = sun_color(hours);
    }

    // Fog fades the far edge of the streamed area, so chunks pop in inside
    // haze instead of appearing out of clear air.
    let far = config.world.stream_radius;
    for mut f in &mut fog {
        f.color = sky;
        f.falloff = FogFalloff::Linear {
            start: far * 0.62,
            end: far * 1.25,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sun_is_up_at_noon_and_down_at_midnight() {
        assert!(sun_elevation(12.0) > 0.99);
        assert!(sun_elevation(0.0) < -0.99);
        assert!(sun_elevation(6.0).abs() < 1e-5);
        assert!(sun_elevation(18.0).abs() < 1e-5);
    }

    #[test]
    fn daylight_is_clamped_and_dark_at_night() {
        assert_eq!(daylight(12.0), 1.0);
        assert_eq!(daylight(2.0), 0.0);
        for h in 0..240 {
            let d = daylight(h as f32 * 0.1);
            assert!((0.0..=1.0).contains(&d), "daylight out of range at {h}");
        }
    }
}
