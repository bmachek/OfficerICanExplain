//! The weather, and what it leaves on the ground.
//!
//! Three values, and only one of them is remembered. **Cloud cover** is sampled
//! from a slow noise over the world clock, so the sky changes by itself and
//! changes the same way on every run of the same seed. **Rainfall** falls out of
//! cover: a clear sky does not rain, and it takes a nearly solid overcast before
//! it does. **Wetness** is the state — it soaks up under rain and dries off in
//! sun and wind, which is why a street stays glossy for a while after a shower
//! and why the reflections outlast the streaks.
//!
//! All three move on *game* hours rather than on wall-clock seconds, so freezing
//! the clock freezes the weather with it. That is the whole reproducibility
//! story: `--hour` already stopped the sun, and now it stops the sky too, which
//! means a screenshot taken at hour 21.5 with a given seed is the same picture
//! every time.
//!
//! What the weather then *does* is spread across the modules that own each
//! surface, rather than centralised here:
//!
//! * the sun, the ambient fill and the haze read cover (`timeofday`),
//! * the shadow penumbra widens with it (`render::shadows`) — an overcast sky is
//!   one enormous area light,
//! * the fog thickens with it and rain puts light shafts under the lamps
//!   (`render::volumetrics`),
//! * the grade cools and desaturates (`render::post`),
//! * the road pools water (`world::road`), and the pavement below just goes
//!   evenly damp, which is what a pavement does.
//!
//! Falling rain itself is the least of it. A few thousand streaks around the
//! camera read as weather; *wet ground* is what changes the picture, because
//! asphalt soaked through goes darker and far glossier, and at the grazing
//! angles a street is actually seen from it stops being a surface and starts
//! being a mirror for the sky and every lit window above it.

use bevy::prelude::*;
use rand::RngExt;

use crate::core::config::GameConfig;
use crate::core::rng::{key_for, stream, stream_for};
use crate::player::camera::CameraRig;

use super::timeofday::{TimeOfDay, daylight};

/// How many streaks are kept around the camera.
const DROPS: usize = 2600;
/// The box they fall inside, in metres.
const FIELD: Vec3 = Vec3::new(46.0, 26.0, 46.0);
/// Metres per second, straight down. Rain falls faster than this; drawn faster
/// than this it turns into a static hatch pattern.
const FALL_SPEED: f32 = 26.0;

/// Game hours a weather front takes to turn over. Long, because a sky that
/// changes its mind every few minutes reads as a bug rather than as weather.
const FRONT_HOURS: f32 = 11.0;
/// And the shorter variation riding on top of it: broken cloud, brightening
/// and darkening within a front.
const BREAK_HOURS: f32 = 2.7;
/// Game hours for the wind to swing right around.
const WIND_HOURS: f32 = 8.0;
/// Metres per second at a dead calm and in the worst of it.
const WIND_CALM: f32 = 1.2;
const WIND_GALE: f32 = 11.0;

/// Cover below which nothing falls, and the cover at which it is coming down as
/// hard as it ever does. Rain wants most of the sky: a shower under a half-clear
/// sky happens, but a game that does it often reads as random rather than as
/// weather.
const RAIN_FROM: f32 = 0.70;
const RAIN_FULL: f32 = 0.95;

/// How fast the cover can move, in cover per game hour. A front arrives over
/// something like an hour and a half rather than instantly, which is also what
/// makes a starting cover mean anything: `--cover 1.0` is still overcast several
/// game minutes later.
const FRONT_SPEED: f32 = 0.62;

/// Wetness gained per game hour under the hardest rain. A downpour soaks a road
/// through in about twenty-five game minutes.
const SOAK_RATE: f32 = 2.4;
/// And lost per game hour at full drying. Slower than it soaks, on purpose: the
/// long tail where the road is still reflecting after the rain has stopped is
/// the most useful weather state the game has.
const DRY_RATE: f32 = 0.55;

/// One falling streak.
#[derive(Component)]
pub struct Raindrop;

/// What the sky is doing right now.
///
/// The live values, as against `WorldConfig`'s starting ones — the same split
/// as [`TimeOfDay`] against `start_hour`, and for the same reason: a screenshot
/// needs to be able to ask for a particular sky, and a session needs the sky to
/// move.
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct Weather {
    /// Fraction of the sky under cloud, 0 to 1.
    pub cover: f32,
    /// How hard it is falling, 0 to 1. Derived from cover every frame; there is
    /// no such thing as rain out of a clear sky.
    pub rain: f32,
    /// How wet the ground is, 0 to 1. The one value with a memory.
    pub wetness: f32,
    /// Metres per second across the ground plane.
    pub wind: Vec2,
    /// Game hours since the world started. What the noise is sampled against —
    /// the clock itself wraps at midnight and would loop the weather with it.
    pub elapsed: f32,
}

impl Weather {
    /// Wind speed alone, for the things that do not care which way it blows.
    pub fn wind_speed(&self) -> f32 {
        self.wind.length()
    }
}

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

// -- the state machine ------------------------------------------------------

/// Smoothstepped value noise over one dimension.
///
/// A whole noise library would be overkill for two curves. What this needs is
/// only that it be continuous, cheap, and identical on every machine for a given
/// key — which rules out anything touching floating-point accumulation and
/// leaves a hash at the integer lattice points.
fn noise(key: u64, t: f32) -> f32 {
    fn at(key: u64, i: i64) -> f32 {
        // SplitMix64's finaliser, which is what makes neighbouring lattice
        // points uncorrelated rather than merely different.
        let mut x = key ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        x ^= x >> 30;
        x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        x ^= x >> 27;
        x = x.wrapping_mul(0x94D0_49BB_1331_11EB);
        x ^= x >> 31;
        // The top 24 bits, which is every bit a f32 mantissa can hold anyway.
        (x >> 40) as f32 / 16_777_216.0
    }

    let floor = t.floor();
    let i = floor as i64;
    let f = t - floor;
    let f = f * f * (3.0 - 2.0 * f);
    at(key, i).lerp(at(key, i + 1), f)
}

/// The cover the sky is heading towards at this point on the clock.
///
/// Biased well below the midpoint. Averaged noise sits around a half, and a city
/// that is under half-cloud every day of its life is not weather, it is a
/// permanent overcast — so the curve pushes the common case down to a fair day
/// with some cloud in it and reserves the top of the range for a real front.
pub fn cover_target(seed: u64, hours: f32) -> f32 {
    let key = key_for(seed, stream::WEATHER);
    let front = noise(key, hours / FRONT_HOURS);
    let breaks = noise(key ^ 0xB2EA_C51D, hours / BREAK_HOURS);
    let raw = front * 0.74 + breaks * 0.26;
    ((raw - 0.30) / 0.62).clamp(0.0, 1.0).powf(1.35)
}

/// How hard it falls under a given cover.
pub fn rain_under(cover: f32) -> f32 {
    ((cover - RAIN_FROM) / (RAIN_FULL - RAIN_FROM)).clamp(0.0, 1.0)
}

/// Where the wind is, and how hard.
pub fn wind_at(seed: u64, hours: f32) -> Vec2 {
    let key = key_for(seed, stream::WEATHER);
    let bearing = noise(key ^ 0x7B1D_0007, hours / WIND_HOURS) * std::f32::consts::TAU;
    let gust = noise(key ^ 0x51EE_D002, hours / (WIND_HOURS * 0.4));
    Vec2::from_angle(bearing) * WIND_CALM.lerp(WIND_GALE, gust)
}

/// How fast the ground gives up its water, 0 (never) to a bit over 1.
///
/// Sun does most of it and wind does the rest, and the floor is not zero: a road
/// dries overnight, only slowly.
pub fn drying(daylight: f32, wind_speed: f32) -> f32 {
    (0.22 + 0.62 * daylight + 0.030 * wind_speed).clamp(0.0, 1.4)
}

/// Where the ground's wetness goes over `hours` of this weather.
pub fn soak(wetness: f32, rain: f32, drying: f32, hours: f32) -> f32 {
    // Rain does not merely outweigh drying, it stops it: nothing dries out in
    // the rain, whatever the wind is doing.
    let change = rain * SOAK_RATE - (1.0 - rain) * drying * DRY_RATE;
    (wetness + change * hours).clamp(0.0, 1.0)
}

/// Moves `cover` towards `target` at no more than a front's pace.
pub fn drift(cover: f32, target: f32, hours: f32) -> f32 {
    let step = FRONT_SPEED * hours;
    (cover + (target - cover).clamp(-step, step)).clamp(0.0, 1.0)
}

pub struct WeatherPlugin;

impl Plugin for WeatherPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WetSurfaces>()
            .add_systems(Startup, (start_weather, spawn_rain))
            .add_systems(
                Update,
                (advance_weather, wet_the_ground, fall, follow_camera).chain(),
            );
    }
}

fn start_weather(mut commands: Commands, config: Res<GameConfig>) {
    let cover = config.world.start_cover.clamp(0.0, 1.0);
    commands.insert_resource(Weather {
        cover,
        rain: rain_under(cover),
        wetness: config.world.start_wetness.clamp(0.0, 1.0),
        wind: wind_at(config.world_seed, 0.0),
        elapsed: 0.0,
    });
}

/// One game step of weather.
///
/// Measured in game hours rather than in seconds, which is what ties the whole
/// system to the day/night clock: a paused or frozen clock leaves the sky
/// exactly where it was, and a fast clock brings the fronts through faster
/// alongside the sun.
fn advance_weather(
    time: Res<Time>,
    config: Res<GameConfig>,
    clock: Res<TimeOfDay>,
    mut weather: ResMut<Weather>,
) {
    // Rainfall is not state, it is a reading off the cover, so it is recomputed
    // whether or not time is passing. That matters at both ends: a frozen clock
    // still has to rain if the sky is asked to, and the dev panel's cloud slider
    // has to do something the moment it moves.
    let rain = rain_under(weather.cover);
    if weather.rain != rain {
        weather.rain = rain;
    }

    if clock.paused || config.world.day_length_seconds <= 0.0 {
        return;
    }
    let hours = time.delta_secs() * 24.0 / config.world.day_length_seconds;
    weather.elapsed += hours;

    let elapsed = weather.elapsed;
    let seed = config.world_seed;
    weather.wind = wind_at(seed, elapsed);
    weather.cover = drift(weather.cover, cover_target(seed, elapsed), hours);
    weather.rain = rain_under(weather.cover);

    let drying = drying(daylight(clock.hours), weather.wind_speed());
    weather.wetness = soak(weather.wetness, weather.rain, drying, hours);
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
        //
        // Thicker than a raindrop, because a raindrop is thinner than a pixel.
        // Seven millimetres across the frame is about half a pixel at ten
        // metres, and half a pixel of faint geometry is precisely what a
        // temporal resolve averages away — the rain was invisible at every tier
        // that runs one, which is every tier above the floor. Wider and fainter
        // carries the same amount of light in something that survives.
        streak: meshes.add(Cuboid::new(0.020, 0.42, 0.020)),
        water: materials.add(StandardMaterial {
            // Faint. Unlit blending over a wet road at night is high contrast
            // already, and a drop passing near the camera at full strength
            // reads as a white rod hanging in the air.
            base_color: Color::srgba(0.72, 0.78, 0.86, 0.10),
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

/// Applies the current wetness to every surface that goes evenly damp.
///
/// Which is the pavement and not the road: rain puddles, and the road's water
/// is a shader term that varies across the surface rather than one number over
/// all of it. See `world::road`.
fn wet_the_ground(
    weather: Res<Weather>,
    surfaces: Res<WetSurfaces>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut drops: Query<&mut Visibility, With<Raindrop>>,
    mut applied: Local<Option<(f32, bool)>>,
) {
    let wetness = weather.wetness.clamp(0.0, 1.0);
    let falling = weather.rain > 0.0;
    if applied
        .is_some_and(|(last, was): (f32, bool)| (last - wetness).abs() < 0.005 && was == falling)
    {
        return;
    }
    *applied = Some((wetness, falling));

    for surface in &surfaces.0 {
        let Some(mut material) = materials.get_mut(&surface.material) else {
            continue;
        };
        let (color, roughness) = soaked(surface.dry_color, surface.dry_roughness, wetness);
        material.base_color = color;
        material.perceptual_roughness = roughness;
    }

    // The streaks follow the *rain*, not the wetness. That is the whole point of
    // making wetness a state: the ground stays glossy for a while after the last
    // drop, and it did before the first one had landed.
    let visibility = if falling {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut drop in &mut drops {
        *drop = visibility;
    }
}

/// Moves the rain, leaning it into the wind.
fn fall(time: Res<Time>, weather: Res<Weather>, mut drops: Query<&mut Transform, With<Raindrop>>) {
    if weather.rain <= 0.0 {
        return;
    }
    let velocity = Vec3::new(weather.wind.x, -FALL_SPEED, weather.wind.y);
    let step = velocity * time.delta_secs();
    // A streak lies along its own path. Taken from the reversed velocity so the
    // arc is from +Y to something near +Y — the rotation between opposite
    // vectors is the one case `from_rotation_arc` cannot pick an axis for.
    let lean = Quat::from_rotation_arc(Vec3::Y, (-velocity).normalize());

    for mut transform in &mut drops {
        transform.translation += step;
        transform.rotation = lean;
    }
    // Nothing is wrapped here. Wind means a drop can leave the field sideways as
    // well as downwards, so folding it back in is one job, and it belongs to the
    // system that knows where the field is.
}

/// Keeps the rain field centred on the camera, and folds escapees back into it.
///
/// The field follows in whole metres rather than continuously, so drops do not
/// slide sideways with the camera — rain falls at the wind's angle regardless of
/// how fast you are driving through it, which is wrong in a gale and right in a
/// city.
///
/// Wrapping is the other half, and it has to happen even when the camera has not
/// moved: the wind carries drops out through the *sides* of the box, so standing
/// still in a gale would otherwise empty the field from one face and leave the
/// rain visibly sliding off downwind. Which is exactly the case a screenshot is,
/// with the camera nailed down for sixty frames.
fn follow_camera(
    weather: Res<Weather>,
    cameras: Query<&GlobalTransform, With<CameraRig>>,
    mut drops: Query<&mut Transform, With<Raindrop>>,
    mut centre: Local<Vec3>,
) {
    let Ok(camera) = cameras.single() else { return };
    let wanted = camera.translation().round();
    let shift = wanted - *centre;
    // Nothing has moved and nothing is falling: 2600 transforms not written.
    if shift == Vec3::ZERO && weather.rain <= 0.0 {
        return;
    }
    *centre = wanted;
    for mut transform in &mut drops {
        transform.translation += shift;
        // Wrapped rather than respawned. The field is a box that moves with the
        // camera, so a drop leaving one face is the same drop arriving at the
        // opposite one — and the eye cannot follow one long enough to notice.
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

    // -- the state machine --------------------------------------------------

    /// The sky must stay a sky. Everything downstream — sun brightness, fog
    /// density, the grade — multiplies by cover, and a value out of range there
    /// is a black frame rather than a wrong one.
    #[test]
    fn cover_stays_a_fraction_over_a_week_of_weather() {
        for seed in [0, 1, 0xA17E_5EED, u64::MAX] {
            for step in 0..2000 {
                let hours = step as f32 * 0.1;
                let cover = cover_target(seed, hours);
                assert!(
                    (0.0..=1.0).contains(&cover),
                    "seed {seed} hour {hours}: cover {cover}"
                );
            }
        }
    }

    /// Weather is sampled, not integrated, so the same seed and the same hour
    /// have to give the same sky — that is what makes a screenshot repeatable.
    #[test]
    fn the_same_hour_of_the_same_seed_is_the_same_weather() {
        for hours in [0.0, 3.25, 51.5, 400.0] {
            assert_eq!(cover_target(7, hours), cover_target(7, hours));
            assert_eq!(wind_at(7, hours), wind_at(7, hours));
        }
    }

    #[test]
    fn different_seeds_get_different_weather() {
        let a: Vec<f32> = (0..40).map(|i| cover_target(1, i as f32)).collect();
        let b: Vec<f32> = (0..40).map(|i| cover_target(2, i as f32)).collect();
        assert_ne!(a, b);
    }

    /// A fair day is the common case. Without the bias the noise averages a half
    /// and the city lives under permanent half-cloud, which reads as a broken
    /// renderer rather than as weather.
    #[test]
    fn most_of_the_time_it_is_not_raining() {
        let hours: Vec<f32> = (0..4000).map(|i| i as f32 * 0.25).collect();
        let wet = hours
            .iter()
            .filter(|&&h| rain_under(cover_target(0xA17E_5EED, h)) > 0.0)
            .count();
        let share = wet as f32 / hours.len() as f32;
        assert!(
            (0.01..0.30).contains(&share),
            "it rained {:.0}% of the time",
            share * 100.0
        );
    }

    #[test]
    fn a_clear_sky_never_rains_and_a_solid_one_always_does() {
        assert_eq!(rain_under(0.0), 0.0);
        assert_eq!(rain_under(RAIN_FROM), 0.0);
        assert_eq!(rain_under(1.0), 1.0);
        assert!(rain_under(0.85) > 0.0);
    }

    #[test]
    fn the_wind_stays_a_breeze() {
        for step in 0..2000 {
            let speed = wind_at(0xA17E_5EED, step as f32 * 0.1).length();
            assert!(
                (WIND_CALM - 1e-3..=WIND_GALE + 1e-3).contains(&speed),
                "{speed} m/s is not weather"
            );
        }
    }

    #[test]
    fn rain_soaks_the_ground_and_sun_dries_it_out() {
        let dry = drying(1.0, 4.0);
        let mut wetness = 0.0;
        for _ in 0..40 {
            wetness = soak(wetness, 1.0, dry, 0.05);
        }
        assert!(wetness > 0.9, "two game hours of rain left it at {wetness}");

        for _ in 0..200 {
            wetness = soak(wetness, 0.0, dry, 0.05);
        }
        assert!(wetness < 0.05, "ten hours of sun left it at {wetness}");
    }

    /// The interesting state, and the reason wetness is integrated rather than
    /// derived: the rain stops and the street stays a mirror for a while.
    #[test]
    fn the_road_stays_wet_after_the_rain_stops() {
        let dry = drying(0.0, 2.0);
        let mut wetness = 1.0;
        for _ in 0..12 {
            wetness = soak(wetness, 0.0, dry, 0.05);
        }
        assert!(
            wetness > 0.85,
            "half an hour after the rain it was already down to {wetness}"
        );
    }

    #[test]
    fn nothing_dries_out_in_the_rain() {
        let gale = drying(1.0, WIND_GALE);
        assert!(soak(0.5, 1.0, gale, 0.1) > 0.5);
    }

    #[test]
    fn wetness_never_leaves_its_range() {
        for rain in [0.0, 0.5, 1.0] {
            for start in [0.0, 0.5, 1.0] {
                // A stupidly long step, which is what a stalled frame looks
                // like to an integrator.
                let after = soak(start, rain, drying(1.0, WIND_GALE), 40.0);
                assert!((0.0..=1.0).contains(&after), "{after} out of range");
            }
        }
    }

    #[test]
    fn a_front_takes_time_to_arrive() {
        // One frame at sixty a second, with a ten-minute day: about a thousandth
        // of an hour. The sky must not snap across in it.
        let after = drift(0.0, 1.0, 0.001);
        assert!(after < 0.01, "cover jumped to {after} in one frame");
        // But it does get there.
        let mut cover = 0.0;
        for _ in 0..200 {
            cover = drift(cover, 1.0, 0.02);
        }
        assert!(cover > 0.99);
    }

    #[test]
    fn a_frozen_clock_leaves_the_sky_alone() {
        assert_eq!(drift(0.4, 1.0, 0.0), 0.4);
        assert_eq!(soak(0.4, 1.0, 1.0, 0.0), 0.4);
    }

    #[test]
    fn wind_and_sun_both_dry_the_ground_faster() {
        assert!(drying(1.0, 0.0) > drying(0.0, 0.0));
        assert!(drying(0.0, 10.0) > drying(0.0, 0.0));
        // And a road dries overnight, only slowly.
        assert!(drying(0.0, 0.0) > 0.0);
    }
}
