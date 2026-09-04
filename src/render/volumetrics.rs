//! Air you can see light travel through.
//!
//! Every other lighting effect in this renderer is about surfaces: what the sun
//! does when it lands on something. This one is about the space in between —
//! the shaft that comes down a side street when the sun is low, the cone under
//! a street lamp at night, the way a city thickens up in rain. It is the single
//! most recognisable "expensive renderer" marker there is, and it is also the
//! one that most needs restraint: fog dense enough to be obvious is fog dense
//! enough to eat the city.
//!
//! Three pieces, and they have to be present together or none of it happens:
//!
//! * a [`VolumetricFog`] on the camera, which is what runs the raymarch,
//! * a [`FogVolume`] in the world saying *where* the air is — a box, because the
//!   march is bounded by it and an unbounded one would spend its whole step
//!   budget out past the horizon,
//! * a [`VolumetricLight`] on each light allowed to shine through it. Not all of
//!   them: the cost is per light per step, and sixty-four street lamps marched
//!   through a fog volume is exactly the sort of thing that quietly costs a
//!   third of the frame. So the lamps come in at the top tier only, and the sun
//!   — one light, and the one that makes the shafts — comes in as soon as there
//!   is any fog at all.
//!
//! ## What the numbers mean
//!
//! Density is per metre, and the raymarch attenuates by
//! `exp(-distance × density × (absorption + scattering))`. With Bevy's default
//! coefficients summing to 0.6, the default density of 0.1 fogs a surface out
//! entirely by a hundred metres. That is the right number for a room and three
//! orders of magnitude wrong for a city, which is why everything here is in the
//! thousandths: [`DENSITY_CLEAR`] leaves about nine tenths of the sky showing
//! through two hundred metres of it.
//!
//! Low density is also why [`FogVolume::light_intensity`] is driven above one.
//! Shaft brightness scales with density, so air thin enough to see the city
//! through is also air too thin to show a light shaft honestly. Nudging the
//! light term rather than the density buys the shafts without the soup — which
//! is a cheat, and is the same cheat as the over-driven sky in `render`.

use bevy::light::{FogVolume, VolumetricFog, VolumetricLight};
use bevy::prelude::*;

use crate::core::config::GameConfig;
use crate::player::camera::CameraRig;
use crate::world::streetlights::StreetLight;
use crate::world::timeofday::{Sun, TimeOfDay, daylight};
use crate::world::weather::Weather;

use super::quality::{Upscaling, Volumetrics};

/// How thick the air is on a clear day, per metre.
///
/// Small enough to be almost invisible looking at it, which is the point: it is
/// there so the shafts have something to be shafts *in*. Everything below adds
/// to it.
const DENSITY_CLEAR: f32 = 0.0020;
/// What a solid overcast adds. Closes a long avenue down without closing the
/// street you are standing in.
const DENSITY_COVER: f32 = 0.0055;
/// And rain on top of that.
const DENSITY_RAIN: f32 = 0.0040;
/// Ground mist before the sun gets to work. The largest single term, because a
/// misty dawn is the one weather where the air is genuinely the subject.
const DENSITY_MIST: f32 = 0.0080;
/// And the most the three of them together are allowed to come to.
///
/// A cap rather than a sum, because the terms overlap in reality — rain clears
/// mist, and mist does not survive a gale — while three independent maxima do
/// not know that about each other. A misty dawn in a rainstorm added up to air
/// you could not see a hundred metres through, and a game where you cannot see
/// the far side of the street you are standing in has a bug in it, whatever the
/// weather is supposed to be doing.
const DENSITY_MAX: f32 = 0.0145;

/// Metres of air above the ground and below it.
///
/// A layer rather than a column, and deliberately lower than downtown is tall.
/// Fog *is* a ground layer — it is where the cold air is — and a tower standing
/// out of the top of one is a thing you can photograph rather than an artefact.
/// The height is also what bounds a ray aimed at the sky, and so what caps the
/// density: the taller the box, the more of it a skyward ray crosses and the
/// milkier the sky, and it was the sky that set the density before this was a
/// layer. Bringing the ceiling down to a plausible fog depth bought back most of
/// a factor of three, which is most of why a lamp now stands in a cone.
const CEILING: f32 = 70.0;
const FLOOR: f32 = -8.0;

// The road is at zero, and a camera standing on it has to be inside the air.
// Checked by the compiler rather than by a test, which would only re-run the
// compiler's arithmetic.
const _: () = assert!(FLOOR < 0.0);
const _: () = assert!(CEILING > 40.0);

/// How far the visible air reaches from the camera at ground level, in metres.
///
/// Modest, and it has to be. Bevy dims every light's contribution to the fog by
/// `exp(-density × bounding_radius × 0.6)`, where the radius is the volume's own
/// half-diagonal — the model's stand-in for how far the light travelled through
/// the fog to reach the sample. Sized to the streamed city that radius is
/// fifteen hundred metres, which dims a clear night's lamp cones by a factor of
/// four and a rainstorm's by seven hundred: the volume gets *bigger* and the
/// effect inside it disappears. Shafts and cones are a near-field effect anyway;
/// past this the atmosphere's own aerial perspective and the distance fog do the
/// haze.
const REACH: f32 = 260.0;
/// And the furthest it is allowed to stretch from altitude.
const REACH_MAX: f32 = 1_400.0;

/// How far the air reaches, given how high the camera is.
///
/// From street level the air is about the cone under the lamp twenty metres
/// away. From six hundred metres up there are no cones to see and the subject is
/// the haze lying over the whole city, so the volume has to stretch to cover
/// what is actually in frame. The in-scattering penalty that comes with the
/// larger volume is then paid exactly where it costs least.
fn reach(altitude: f32) -> f32 {
    (REACH + altitude.max(0.0) * 1.6).min(REACH_MAX)
}

/// Marks the one fog volume, so it can be found and moved.
#[derive(Component)]
pub struct CityAir;

/// How thick the air is right now, per metre.
///
/// Weather does most of it and the hour does the rest. Kept as a free function
/// because it is the number worth arguing about, and an argument about a number
/// is a unit test.
pub fn density(cover: f32, rain: f32, hours: f32) -> f32 {
    (DENSITY_CLEAR
        + DENSITY_COVER * cover.clamp(0.0, 1.0)
        + DENSITY_RAIN * rain.clamp(0.0, 1.0)
        + DENSITY_MIST * mist(hours))
    .min(DENSITY_MAX)
}

/// Ground mist, 0 to 1.
///
/// Forms on a cold night and burns off within an hour or two of the sun getting
/// up, so it is centred just *after* first light rather than on the small hours:
/// mist at 2am is invisible, and mist at 6am is the whole reason to have it.
fn mist(hours: f32) -> f32 {
    const PEAK: f32 = 5.6;
    const WIDTH: f32 = 2.4;
    (1.0 - ((hours - PEAK) / WIDTH).abs())
        .clamp(0.0, 1.0)
        .powf(0.7)
}

/// How hard to lean on the light term to make up for thin air.
///
/// Falls back towards honesty as the fog thickens: in real weather there is
/// enough of it for the shafts to carry themselves. The figure at the thin end
/// is large, and it is the same bargain the street lamps themselves already
/// struck — a lamp here is quoted at a floodlight's output so that its pool
/// reads at the night exposure, and a lamp cone is quoted up for the same
/// reason. See `world::streetlights`.
fn light_boost(density: f32) -> f32 {
    let thickness = (density / (DENSITY_CLEAR + DENSITY_COVER)).clamp(0.0, 1.0);
    7.0f32.lerp(1.5, thickness)
}

/// How many samples along each ray.
///
/// Bought where it is spent: the lamps are what needs resolution, because a cone
/// under a lamp is a small bright thing that bands visibly, while a flat haze
/// under the sun does not.
fn steps(volumetrics: Volumetrics) -> u32 {
    match volumetrics {
        Volumetrics::Off => 0,
        Volumetrics::Fog => 32,
        Volumetrics::FogAndLights => 64,
    }
}

pub struct VolumetricsPlugin;

impl Plugin for VolumetricsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_city_air)
            .add_systems(Update, (sync_volumetrics, drive_city_air).chain());
    }
}

/// The box the weather happens inside.
///
/// One entity for the whole city rather than one per district or per street.
/// A [`FogVolume`] is a unit cube posed by its transform, and the raymarch is
/// bounded by it — so two overlapping volumes would be marched twice, and the
/// fog would double up exactly where they met.
fn spawn_city_air(mut commands: Commands) {
    commands.spawn((
        Name::new("City Air"),
        CityAir,
        FogVolume {
            density_factor: DENSITY_CLEAR,
            // White, and it stays white. `fog_color` is what the air scatters,
            // not what is lighting it — the raymarch multiplies by each light's
            // own colour already, so tinting this as well double-counts. Tying
            // it to the sky (which reads about 0.05 after dark, on the theory
            // that night fog is blue) quietly multiplied every lamp shaft by a
            // twentieth and took the whole effect out of the night.
            fog_color: Color::WHITE,
            // Forward-scattering, which is what puts the shaft *in front of*
            // the light rather than around it: looking down a street towards a
            // low sun should not look the same as looking away from it, and
            // this asymmetry is the only thing that makes it so.
            //
            // Mild, though, and that is a compromise between the two effects
            // this module exists for. A street lamp points *down* and is looked
            // at from the side, so a lamp cone is entirely side-scatter — and
            // Henyey-Greenstein at 0.66 returns a third as much to the side as
            // an isotropic phase would. The first version had it there and the
            // cones were invisible.
            scattering_asymmetry: 0.42,
            ..default()
        },
        Transform::default(),
    ));
}

/// Attaches and detaches the volumetric path, per the current tier.
///
/// Insert-or-remove per component rather than a rebuild, for the same reason as
/// the rest of the camera stack: the dev panel writes settings continuously
/// while a slider is being dragged.
fn sync_volumetrics(
    mut commands: Commands,
    config: Res<GameConfig>,
    cameras: Query<Entity, With<super::RenderStack>>,
    sun: Query<Entity, With<Sun>>,
    lamps: Query<Entity, With<StreetLight>>,
    mut applied: Local<Option<(Volumetrics, Upscaling)>>,
) {
    let settings = &config.graphics;
    let wanted = (settings.volumetrics, settings.upscaling);
    if *applied == Some(wanted) && !cameras.is_empty() {
        return;
    }

    let on = settings.volumetrics != Volumetrics::Off;
    for camera in &cameras {
        let mut camera = commands.entity(camera);
        if on {
            camera.insert(VolumetricFog {
                step_count: steps(settings.volumetrics),
                // Offsetting the ray origin trades banding for noise, which is
                // only a trade worth making when something downstream resolves
                // the noise. Without a temporal pass it is simply noise.
                jitter: match settings.upscaling {
                    Upscaling::Off => 0.0,
                    Upscaling::Taa | Upscaling::Dlss => 1.2,
                },
                // Set properly by `drive_city_air` on the same frame.
                ..default()
            });
        } else {
            camera.remove::<VolumetricFog>();
        }
    }

    // The sun is one light and it is the one that makes the shafts, so it comes
    // in with the fog itself.
    for sun in &sun {
        let mut sun = commands.entity(sun);
        if on {
            sun.insert(VolumetricLight);
        } else {
            sun.remove::<VolumetricLight>();
        }
    }

    // The lamps are sixty-four lights, each costing a sample per raymarch step
    // in every cluster it touches. That is the expensive half of this module and
    // it is where the tier boundary goes.
    let lit = settings.volumetrics == Volumetrics::FogAndLights;
    for lamp in &lamps {
        let mut lamp = commands.entity(lamp);
        if lit {
            lamp.insert(VolumetricLight);
        } else {
            lamp.remove::<VolumetricLight>();
        }
    }

    if !cameras.is_empty() {
        *applied = Some(wanted);
        // Worth a line, the same way the resolved preset is. Three components
        // have to line up for any of this to appear, they are attached from
        // three different queries, and the failure mode when one of them does
        // not is not an error — it is a picture that looks very slightly flatter
        // than it should, which is not something anybody spots.
        info!(
            "volumetrics: {:?}, {} steps, sun {}, {} lamps",
            settings.volumetrics,
            steps(settings.volumetrics),
            if on { "shafts" } else { "off" },
            if lit { lamps.iter().count() } else { 0 },
        );
    }
}

/// Keeps the air over the camera, and matched to the weather.
fn drive_city_air(
    clock: Res<TimeOfDay>,
    weather: Res<Weather>,
    sky: Res<ClearColor>,
    cameras: Query<&GlobalTransform, With<CameraRig>>,
    mut air: Query<(&mut FogVolume, &mut Transform), With<CityAir>>,
    mut fog: Query<&mut VolumetricFog>,
) {
    let Ok(camera) = cameras.single() else { return };

    let thickness = density(weather.cover, weather.rain, clock.hours);
    let day = daylight(clock.hours);
    let at = camera.translation();
    let span = reach(at.y) * 2.0;

    for (mut volume, mut transform) in &mut air {
        // Follows in the ground plane only. Vertically the air belongs to the
        // city, not to the camera: from six hundred metres up in the aerial shot
        // the point is to look *down* through the layer over the rooftops, and a
        // volume that came with the camera would leave it behind.
        *transform = Transform::from_xyz(at.x, (FLOOR + CEILING) * 0.5, at.z)
            .with_scale(Vec3::new(span, CEILING - FLOOR, span));

        volume.density_factor = thickness;
        volume.light_intensity = light_boost(thickness);
    }

    for mut fog in &mut fog {
        // This one really is the colour of a light: it stands in for the sky
        // lighting the fog where nothing else reaches it, so it follows the sky.
        fog.ambient_color = sky.0;
        // What the fog looks like where no light reaches it directly. Small: the
        // ambient term is a flat fill over the whole volume, and driving it up
        // is the fastest way to turn a city into milk.
        fog.ambient_intensity = 0.012 + 0.055 * day;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Transmittance over `metres` of this air, with Bevy's absorption and
    /// scattering summing to 0.6.
    fn through(metres: f32, density: f32) -> f32 {
        (-metres * density * 0.6f32).exp()
    }

    /// The failure this guards against is not subtle: at Bevy's default density
    /// the city disappears inside a hundred metres. Anything in the hundredths
    /// here is a lost frame, so the ceiling is checked rather than trusted.
    #[test]
    fn you_can_always_see_across_the_street() {
        for cover in [0.0, 0.5, 1.0] {
            for rain in [0.0, 1.0] {
                for hour in 0..48 {
                    let hours = hour as f32 * 0.5;
                    let d = density(cover, rain, hours);
                    let across = through(120.0, d);
                    assert!(
                        across > 0.35,
                        "cover {cover} rain {rain} hour {hours}: density {d} \
                         leaves {across} of a building 120m away"
                    );
                }
            }
        }
    }

    /// The density is capped by what a ray aimed at the sky passes through, and
    /// that is the ceiling — which is why the ceiling is a fog layer's depth
    /// rather than a tower's height.
    #[test]
    fn a_clear_noon_barely_hazes_the_sky() {
        assert!(through(CEILING, density(0.0, 0.0, 12.0)) > 0.85);
        // And even the worst of it leaves a sky to see.
        assert!(through(CEILING, density(1.0, 1.0, 12.0)) > 0.45);
    }

    #[test]
    fn weather_thickens_the_air() {
        let clear = density(0.0, 0.0, 12.0);
        assert!(
            density(1.0, 0.0, 12.0) > clear * 3.0,
            "overcast is not thick"
        );
        assert!(density(1.0, 1.0, 12.0) > density(1.0, 0.0, 12.0), "rain");
    }

    /// Mist is a dawn thing. Left running all day it reads as a broken renderer
    /// rather than as morning.
    #[test]
    fn mist_belongs_to_the_early_morning() {
        assert_eq!(mist(12.0), 0.0);
        assert_eq!(mist(22.0), 0.0);
        assert_eq!(mist(2.0), 0.0);
        assert!(mist(5.6) > 0.95, "no mist at dawn");
        assert!(mist(7.0) > 0.0 && mist(7.0) < 1.0, "it burns off gradually");
        for hour in 0..240 {
            assert!((0.0..=1.0).contains(&mist(hour as f32 * 0.1)));
        }
    }

    /// The cheat has to fade out, or a rainstorm gets shafts three times too
    /// bright — the point of leaning on the light term is to make up for air
    /// that is too thin, and real weather is not.
    #[test]
    fn the_light_term_stops_being_leaned_on_once_there_is_real_weather() {
        let clear = light_boost(density(0.0, 0.0, 12.0));
        let storm = light_boost(density(1.0, 1.0, 12.0));
        assert!(clear > 5.0, "a clear day gets no shafts at all: {clear}");
        assert!(storm < 1.7, "a storm is still being faked: {storm}");
        assert!(storm >= 1.0, "and never dimmed below honest");
    }

    /// The number the effect lives or dies by. Bevy dims in-scattering by the
    /// volume's half-diagonal, so a volume sized to the city is a volume with no
    /// visible fog in it — which is how this was written the first time.
    #[test]
    fn a_lamp_cone_survives_the_volume_it_is_standing_in() {
        let surviving = |altitude: f32, thickness: f32| {
            let half = Vec3::new(reach(altitude), (CEILING - FLOOR) * 0.5, reach(altitude));
            (-thickness * half.length() * 0.6).exp()
        };

        // Standing in a street on a clear night, half the light gets through.
        assert!(surviving(1.7, density(0.18, 0.0, 22.5)) > 0.4);
        // And in the worst weather the city has, enough of it does that a lamp
        // still stands in something.
        assert!(surviving(1.7, density(1.0, 1.0, 22.5)) > 0.07);
    }

    #[test]
    fn the_air_reaches_further_from_higher_up_but_not_without_limit() {
        assert_eq!(reach(0.0), REACH);
        assert!(reach(80.0) > reach(1.7));
        assert_eq!(reach(100_000.0), REACH_MAX);
        // A camera below the road is still standing in air.
        assert_eq!(reach(-40.0), REACH);
    }

    #[test]
    fn only_the_top_tier_pays_for_lamp_shafts() {
        assert_eq!(steps(Volumetrics::Off), 0);
        assert!(steps(Volumetrics::FogAndLights) > steps(Volumetrics::Fog));
    }
}
