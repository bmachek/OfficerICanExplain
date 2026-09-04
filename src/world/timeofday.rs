//! Day/night cycle.
//!
//! Drives the sun, sky, fog and ambient fill from a single clock. Fog colour is
//! kept locked to the sky colour on purpose: if they diverge, distant buildings
//! fade into a band that does not match the horizon and the illusion collapses.
//!
//! The clock is not the only input any more. Cloud cover comes from `weather`,
//! and what it does here is most of what makes an overcast day read as one: the
//! direct beam drops away, the skylight that replaces it climbs, the warmth goes
//! out of the sun, and the haze thickens. Not the sky *itself* — Bevy's
//! atmosphere is a scattering model with no clouds in it, so the dome overhead
//! stays blue however hard it is raining. Everything the light does is right;
//! the thing you would photograph it against is not.

use std::f32::consts::PI;

use bevy::light::GlobalAmbientLight;
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;

use crate::core::config::GameConfig;
use crate::world::buildings::CityAssets;
use crate::world::weather::Weather;

/// How bright a lit window is at full dark, in the nits `emissive` is measured
/// in — the same scale the tracer and explosion flashes use.
const WINDOW_GLOW: f32 = 3.0;

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
///
/// The ramp starts *below* the horizon deliberately. Sunset is not the end of
/// daylight — civil twilight is another six degrees of it, and the sky stays
/// bright through all of them. Clamping at zero elevation made the city snap
/// from lit to black over about a second of wall clock.
pub fn daylight(hours: f32) -> f32 {
    // -0.11 is roughly six degrees below the horizon; 0.25 is fifteen above.
    ((sun_elevation(hours) + 0.11) / 0.36)
        .clamp(0.0, 1.0)
        .powf(0.6)
}

/// What cloud leaves of the direct beam.
///
/// Not linear, because cloud is not: a thin veil barely dims the ground, and it
/// is the last of the cover that kills the shadows. A solid overcast passes
/// about a tenth of the sun — the rest arrives as skylight instead, which is
/// what [`skylight_gain`] puts back.
pub fn sunlight_through(cover: f32) -> f32 {
    1.0 - 0.90 * cover.clamp(0.0, 1.0).powf(1.6)
}

/// And what it puts back as skylight.
///
/// An overcast sky is not an absence of light, it is one enormous soft source.
/// Without this the city under cloud goes dark rather than flat, which is the
/// usual tell that weather was implemented as a multiplier.
pub fn skylight_gain(cover: f32) -> f32 {
    1.0 + 0.85 * cover.clamp(0.0, 1.0)
}

/// How bright it is outside: 0 at night, 1 in full sun.
///
/// Distinct from [`daylight`], which only knows where the sun is. A solid
/// overcast at noon is a dark day, and the city's windows come on for it.
pub fn brightness(hours: f32, cover: f32) -> f32 {
    daylight(hours) * (1.0 - 0.50 * cover.clamp(0.0, 1.0).powf(1.6))
}

fn sky_color(hours: f32, cover: f32) -> Color {
    let day = daylight(hours);
    let e = sun_elevation(hours);
    // Twilight peaks when the sun is near the horizon — and only through a gap
    // in the cloud. A sunset under solid overcast is grey, not orange.
    let dusk = (1.0 - (e.abs() / 0.30).min(1.0)) * day.max(0.15) * (1.0 - cover * 0.85);

    let night = Vec3::new(0.035, 0.045, 0.085);
    let noon = Vec3::new(0.48, 0.62, 0.85);
    let horizon = Vec3::new(0.85, 0.45, 0.22);

    let base = night.lerp(noon, day);
    let clear = base.lerp(horizon, dusk * 0.75);
    // Cloud takes the blue out of it, towards a flat luminous grey that keeps
    // some of the ambient brightness of the hour.
    let overcast = Vec3::splat(clear.element_sum() / 3.0).lerp(Vec3::new(0.55, 0.57, 0.60), day);
    let c = clear.lerp(overcast, cover.clamp(0.0, 1.0) * 0.8);
    Color::srgb(c.x, c.y, c.z)
}

fn sun_color(hours: f32, cover: f32) -> Color {
    let e = sun_elevation(hours);
    // Cloud is what the warmth is *seen through*. A low sun behind an overcast
    // is diffused into a flat white, so the amber goes with the shadows.
    let warmth = (1.0 - (e.abs() / 0.35).min(1.0)).clamp(0.0, 1.0) * (1.0 - cover * 0.9);
    let white = Vec3::new(1.0, 0.98, 0.94);
    let amber = Vec3::new(1.0, 0.62, 0.34);
    let c = white.lerp(amber, warmth);
    Color::srgb(c.x, c.y, c.z)
}

/// Colour the far edge of the streamed world fades into.
///
/// Not the same value as [`sky_color`], and the difference matters. Fog is
/// mixed into the shaded output *after* exposure, while the sky is radiance the
/// camera meters — so quoting the sky's own brightness here paints a milky band
/// noticeably brighter than the sky behind it.
fn fog_color(hours: f32, cover: f32) -> Color {
    let sky = LinearRgba::from(sky_color(hours, cover));
    LinearRgba::rgb(sky.red * 0.42, sky.green * 0.42, sky.blue * 0.44).into()
}

pub struct TimeOfDayPlugin;

impl Plugin for TimeOfDayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_sun).add_systems(
            Update,
            (attach_fog, advance_clock, apply_sky, light_windows).chain(),
        );
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
        // A sun to actually look at, drawn into the atmosphere at the light's
        // own direction and angular size.
        bevy::light::SunDisk::EARTH,
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
    weather: Res<Weather>,
    config: Res<GameConfig>,
    mut clear: ResMut<ClearColor>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut sun: Query<(&mut Transform, &mut DirectionalLight), With<Sun>>,
    mut fog: Query<&mut DistanceFog>,
) {
    let hours = clock.hours;
    let cover = weather.cover.clamp(0.0, 1.0);
    let day = daylight(hours);
    let sky = sky_color(hours, cover);

    clear.0 = sky;

    // Ambient is only a floor. Daytime sky light comes from the atmosphere's
    // environment map, which is directional and coloured and does the job
    // properly; this exists so that at night an unlit face is dark rather than
    // pure black, and the city stays readable. Cloud raises it, because cloud is
    // where the light goes when it stops being a beam.
    ambient.color = Color::srgb(0.35 + 0.30 * day, 0.42 + 0.30 * day, 0.58 + 0.22 * day);
    // Only the daylight half is boosted. Cloud does not make light, it
    // redistributes sunlight — so with the sun down there is nothing for it to
    // redistribute, and an overcast night is exactly as dark as a clear one.
    // Boosting the floor as well washed the whole night street pale grey.
    ambient.brightness = 180.0 + 260.0 * day * skylight_gain(cover);

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
        // Lux, for real. Direct sunlight is about a hundred thousand of them,
        // and the camera is metered for exactly that — see `render`.
        light.illuminance = (300.0 + 110_000.0 * day) * sunlight_through(cover);
        light.color = sun_color(hours, cover);
    }

    // Fog hides the far edge of the streamed area, so chunks pop in inside haze
    // instead of appearing out of clear air. It is a tight band right at that
    // edge and nothing more: the atmosphere's aerial perspective already does
    // the honest middle-distance haze, and stacking a second one over it turned
    // every long street into smog.
    //
    // Cloud and rain pull that band in. Visibility really does close down in
    // weather, and it is also the cheapest honest way to say "overcast" in a
    // renderer whose sky has no clouds in it.
    let far = config.world.stream_radius;
    let closing = 1.0 - 0.34 * cover - 0.22 * weather.rain;
    let haze = fog_color(hours, cover);
    for mut f in &mut fog {
        f.color = haze;
        f.falloff = FogFalloff::Linear {
            start: far * 0.80 * closing,
            end: far * 1.02 * closing,
        };
    }
}

/// Turns the city's windows on after dark.
///
/// *Which* windows are lit is baked into each facade's emissive mask and never
/// changes; only the strength moves. So a whole skyline coming up at dusk costs
/// one pass over the eighty facade materials, not anything per building.
///
/// The threshold matters: writing to a material re-uploads its uniform, and a
/// continuous day/night ramp would otherwise re-upload all eighty every frame
/// for a change too small to see.
fn light_windows(
    clock: Res<TimeOfDay>,
    weather: Res<Weather>,
    assets: Option<Res<CityAssets>>,
    mut materials: ResMut<Assets<crate::world::facade::FacadeMaterial>>,
    mut applied: Local<Option<f32>>,
) {
    // The city is generated a frame or two before this first runs.
    let Some(assets) = assets else { return };

    // Against how bright it is *outside*, not against the clock: an office
    // turns its lights on for a dark afternoon as well as for the evening.
    let level = 1.0 - brightness(clock.hours, weather.cover);
    if applied.is_some_and(|last: f32| (last - level).abs() < 0.01) {
        return;
    }
    *applied = Some(level);

    // White, because the mask carries each window's own colour.
    let glow = level * WINDOW_GLOW;
    for handle in assets.building_materials() {
        if let Some(mut material) = materials.get_mut(handle) {
            material.base.emissive = LinearRgba::rgb(glow, glow, glow);
        }
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
    fn windows_are_lit_at_night_and_dark_at_noon() {
        assert_eq!((1.0 - brightness(12.0, 0.0)) * WINDOW_GLOW, 0.0);
        assert_eq!((1.0 - brightness(1.0, 0.0)) * WINDOW_GLOW, WINDOW_GLOW);
        // And they come up through dusk rather than snapping on.
        let dusk = 1.0 - brightness(17.5, 0.0);
        assert!(dusk > 0.0 && dusk < 1.0, "dusk should be partial: {dusk}");
    }

    #[test]
    fn a_dark_afternoon_turns_the_lights_on() {
        let clear = brightness(13.0, 0.0);
        let storm = brightness(13.0, 1.0);
        assert!(storm < clear, "an overcast noon is not a bright one");
        assert!(storm > 0.35, "but it is still daytime, not dusk");
    }

    /// The direct beam and the skylight trade against each other. If both fell
    /// together, an overcast city would go dark instead of going flat — the
    /// single most common way weather is got wrong.
    #[test]
    fn cloud_moves_light_from_the_sun_to_the_sky_rather_than_removing_it() {
        assert_eq!(sunlight_through(0.0), 1.0);
        assert_eq!(skylight_gain(0.0), 1.0);
        assert!(
            sunlight_through(1.0) < 0.15,
            "an overcast sun still shadows"
        );
        assert!(skylight_gain(1.0) > 1.5, "and the sky did not pick it up");
        for cover in 0..=10 {
            let cover = cover as f32 / 10.0;
            assert!((0.0..=1.0).contains(&sunlight_through(cover)));
            assert!(skylight_gain(cover) >= 1.0);
        }
    }

    /// A sunset seen through solid overcast is grey. Getting this wrong gives an
    /// amber-lit rainstorm, which is the tell that cover was wired to brightness
    /// and nothing else.
    #[test]
    fn the_warmth_of_a_low_sun_needs_a_gap_in_the_cloud() {
        let warmth = |cover| {
            let c = Srgba::from(sun_color(18.0, cover));
            c.red - c.blue
        };
        assert!(
            warmth(0.0) > 0.5,
            "a low clear sun is amber: {}",
            warmth(0.0)
        );
        assert!(
            warmth(1.0) < warmth(0.0) * 0.25,
            "an overcast one still is: {}",
            warmth(1.0)
        );
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
