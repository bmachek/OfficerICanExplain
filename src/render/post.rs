//! Everything that happens to the image after the city has been shaded.
//!
//! Four separate jobs, kept in one module because they all read the same clock
//! and the same weather and they all argue about the same frame:
//!
//! * **The grade.** A day/night cycle in real units gets the *brightness* of an
//!   hour right and says nothing about its colour. Six in the morning and six in
//!   the evening are the same sun at the same angle, and they do not look alike;
//!   what separates them is grading, and it is most of what people mean when
//!   they call a game's look "cinematic".
//! * **Auto exposure**, which is deliberately not allowed to do its whole job —
//!   see [`METER_AUTHORITY`].
//! * **The shutter.** Motion blur, and depth of field with the focus pulled to
//!   whatever the camera is actually pointed at.
//! * **The lens.** A vignette and a trace of chromatic aberration.
//!
//! ## What is not here
//!
//! Lens distortion, which Bevy also ships. A barrel-distorted frame is a real
//! photographic artefact and it is the wrong one for this game: a city is made
//! of straight lines meeting at right angles, and bending them at the edge of
//! the frame does not read as a lens, it reads as a mistake. It is the one
//! effect in the stack that makes the picture worse the better the geometry
//! gets, and Phase 4 is entirely about the geometry getting better.

use bevy::math::cubic_splines::LinearSpline;
use bevy::post_process::auto_exposure::{
    AutoExposure, AutoExposureCompensationCurve, AutoExposureCompensationCurveError,
    AutoExposurePlugin,
};
use bevy::post_process::dof::{DepthOfField, DepthOfFieldMode};
use bevy::post_process::effect_stack::{ChromaticAberration, Vignette};
use bevy::post_process::motion_blur::MotionBlur;
use bevy::prelude::*;
use bevy::render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection};

use avian3d::prelude::{SpatialQuery, SpatialQueryFilter};

use crate::core::config::GameConfig;
use crate::player::camera::CameraRig;
use crate::world::timeofday::{TimeOfDay, daylight, sun_elevation};
use crate::world::weather::Weather;

/// How much of the metering the automatic exposure is allowed to act on.
///
/// Bevy's auto exposure meters the frame to middle grey, and left to itself that
/// is exactly wrong here. The camera's own `Exposure` is already driven from
/// the sun's position — it *knows* what time it is, which no histogram can —
/// and metering on top of that would hand back the five stops between noon and
/// midnight that the whole physical lighting model exists to earn. Night would
/// come out as a slightly blue afternoon.
///
/// What the histogram does know, and the clock does not, is where the camera is
/// pointed: into a shadowed courtyard, at a lit shopfront, down a black alley.
/// So it is given partial authority instead of none. The compensation curve is
/// a straight line of this slope, and the arithmetic in Bevy's shader is
///
/// ```text
/// target = compensation(measured) - measured
/// ```
///
/// so a line `c(x) = k·x + (1-k)·anchor` yields `target = (k-1)·(x - anchor)`:
/// zero correction for a correctly exposed frame, and `1-k` stops of correction
/// for every stop the frame is away from one. At 0.72 a scene four stops darker
/// than the clock expected is lifted by a bit over a stop — an eye adjusting,
/// not a light switch.
const METER_AUTHORITY: f32 = 0.72;

/// The average log luminance of a correctly exposed frame, in the units the
/// histogram measures.
///
/// A grey card reflects 18% of the light on it, so the middle of a well-exposed
/// image sits at log2(0.18) ≈ -2.47. Checked against this renderer's own
/// numbers: a grey card in full sun is about 5700 cd/m², and `Exposure` at
/// EV100 15 divides by 2^15 × 1.2, which lands it at 0.146 — log2 of that is
/// -2.78. The two agree to a third of a stop, so the anchor sits between them.
const METER_ANCHOR: f32 = -2.6;

/// The widest and narrowest the histogram's own window goes, in stops. Wide
/// enough that the correction never runs off the end of the curve and starts
/// clamping, which is where a bounded correction quietly stops being bounded.
const METER_RANGE: f32 = 10.0;

/// Shutter angle. Half a frame open is the cinema convention — 180° at 24fps —
/// and anything more at 60fps stretches an object's blur further than it
/// actually travelled.
const SHUTTER: f32 = 0.5;

/// How far the lens can focus out to. Past this everything is equally soft, so
/// it also decides how blurred the sky gets: at infinity the background of a
/// street shot is a smear.
const FOCUS_FAR: f32 = 420.0;

/// The curve auto exposure is metered through. Built once; see
/// [`METER_AUTHORITY`] for what shape it is and why.
#[derive(Resource, Default)]
struct MeteringCurve(Option<Handle<AutoExposureCompensationCurve>>);

pub struct PostPlugin;

impl Plugin for PostPlugin {
    fn build(&self, app: &mut App) {
        // Not part of `DefaultPlugins`, unlike the rest of the post stack.
        app.add_plugins(AutoExposurePlugin)
            .init_resource::<MeteringCurve>()
            .add_systems(Startup, build_metering_curve)
            .add_systems(Update, (sync_post, grade_the_image, pull_focus));
    }
}

/// Builds the line described by [`METER_AUTHORITY`].
///
/// Two points, and exactly two. Bevy walks the spline segment by segment and
/// rejects the whole curve if one segment does not start *exactly* where the
/// last one ended — and a linear segment evaluates its end as `a + (b - a)`,
/// which is not bitwise `b` unless the numbers happen to be round. Four evenly
/// spaced points on this line are not round, so the curve was thrown out and
/// auto exposure quietly went with it. A straight line needs one segment
/// anyway, and one segment has no joins to disagree about.
fn metering_curve() -> Result<AutoExposureCompensationCurve, AutoExposureCompensationCurveError> {
    let line = |x: f32| {
        Vec2::new(
            x,
            METER_AUTHORITY * x + (1.0 - METER_AUTHORITY) * METER_ANCHOR,
        )
    };
    AutoExposureCompensationCurve::from_curve(LinearSpline::new([
        line(-METER_RANGE),
        line(METER_RANGE),
    ]))
}

fn build_metering_curve(
    mut curve: ResMut<MeteringCurve>,
    mut curves: ResMut<Assets<AutoExposureCompensationCurve>>,
) {
    match metering_curve() {
        Ok(built) => curve.0 = Some(curves.add(built)),
        // Without a curve, `AutoExposure` meters all the way to middle grey and
        // the night is gone. Going without it entirely is much the smaller loss,
        // so this fails closed rather than falling back to the flat default.
        Err(error) => warn!("no metering curve ({error}); auto exposure stays off"),
    }
}

/// Attaches and detaches the post stack, per the current tier.
fn sync_post(
    mut commands: Commands,
    config: Res<GameConfig>,
    curve: Res<MeteringCurve>,
    cameras: Query<Entity, With<super::RenderStack>>,
    mut applied: Local<Option<(bool, bool, bool)>>,
) {
    let settings = &config.graphics;
    let wanted = (settings.motion_blur, settings.depth_of_field, settings.lens);
    if *applied == Some(wanted) && !cameras.is_empty() {
        return;
    }

    for camera in &cameras {
        let mut camera = commands.entity(camera);

        if settings.motion_blur {
            camera.insert(MotionBlur {
                shutter_angle: SHUTTER,
                // Three samples in each direction. One is enough for a slow pan
                // and visibly stepped behind a car at speed, which is the only
                // place anybody looks at motion blur.
                samples: 3,
            });
        } else {
            camera.remove::<MotionBlur>();
        }

        if settings.depth_of_field {
            camera.insert(DepthOfField {
                // The accurate one, which turns a bright out-of-focus point into
                // a disc rather than a smudge. It is also the more expensive,
                // which is why depth of field is a top-tier setting at all.
                mode: DepthOfFieldMode::Bokeh,
                // Deep. A photographer opens up to throw a background away; a
                // player needs to see the street they are about to drive into,
                // so this is a hint of focus rather than a portrait lens.
                aperture_f_stops: 5.6,
                max_circle_of_confusion_diameter: 14.0,
                max_depth: FOCUS_FAR,
                // Pulled to whatever is in front of the camera, next frame.
                ..default()
            });
        } else {
            camera.remove::<DepthOfField>();
        }

        if settings.lens {
            camera.insert((
                Vignette {
                    // A stop and a bit of falloff in the extreme corners. Every
                    // lens does this; a lens that did it as hard as the default
                    // would be a fault.
                    intensity: 0.30,
                    radius: 0.88,
                    smoothness: 2.4,
                    ..default()
                },
                ChromaticAberration {
                    // A quarter of a percent of the frame, at the very edge. Any
                    // more and it stops reading as glass and starts reading as a
                    // filter someone left on.
                    intensity: 0.0025,
                    ..default()
                },
            ));
        } else {
            camera.remove::<Vignette>();
            camera.remove::<ChromaticAberration>();
        }

        // Auto exposure has no tier of its own: it is a correction to the
        // exposure the clock already set, and it is wanted at every tier that
        // can run a compute shader.
        match &curve.0 {
            Some(curve) => {
                camera.insert(AutoExposure {
                    range: -METER_RANGE..=METER_RANGE,
                    // Ignore the darkest and brightest tenth. A lit window at
                    // night and a patch of sky in a street are both outliers,
                    // and both would otherwise drag the whole frame after them.
                    filter: 0.10..=0.90,
                    // Faster into the light than out of it, the way eyes are.
                    speed_brighten: 2.4,
                    speed_darken: 0.9,
                    compensation_curve: curve.clone(),
                    ..default()
                });
            }
            None => {
                camera.remove::<AutoExposure>();
            }
        }
    }

    if !cameras.is_empty() {
        *applied = Some(wanted);
    }
}

// -- the grade --------------------------------------------------------------

/// The furthest the white point is allowed to move, in CIE 1931 x.
///
/// These are not a -1 to 1 dial, which is what they look like: Bevy adds the
/// value straight onto the D65 white point at x = 0.3127, so 0.2 is not "quite
/// warm", it is an illuminant off the end of the Planckian locus and an image
/// balanced against it comes out solid teal. Which is exactly what the first
/// version of this did. Real illuminants run from about x = 0.25 at an overcast
/// 15000 K to about x = 0.44 under tungsten, so a tenth is already the whole
/// plausible range and a twentieth is a strong grade.
const WHITE_POINT_SHIFT: std::ops::RangeInclusive<f32> = -0.05..=0.05;

/// Warmth of the grade: positive is warmer, negative is cooler.
///
/// Three things pulling at once. A low sun is warm, and warmer than the light
/// itself already is — that push past the physical answer is the grade doing its
/// job. Deep night is cool, because the only warm light left in the city is
/// sodium and everything it does not reach goes blue. Cloud is cool at any hour,
/// which is why an overcast morning and a clear one look nothing alike.
pub fn temperature(hours: f32, cover: f32) -> f32 {
    let low_sun = (1.0 - (sun_elevation(hours).abs() / 0.32).min(1.0)).clamp(0.0, 1.0);
    let night = (1.0 - daylight(hours)).powf(1.6);
    (0.038 * low_sun - 0.030 * night - 0.022 * cover.clamp(0.0, 1.0))
        .clamp(*WHITE_POINT_SHIFT.start(), *WHITE_POINT_SHIFT.end())
}

/// How much colour survives the grade.
///
/// Cloud flattens it, and so does the dark: past dusk the eye is running on rods
/// and has hardly any colour vision left, and a fully saturated night is the
/// single most common thing that makes a game look like a game.
pub fn saturation(hours: f32, cover: f32) -> f32 {
    let night = 1.0 - daylight(hours);
    (1.06 - 0.20 * cover.clamp(0.0, 1.0) - 0.20 * night).clamp(0.55, 1.15)
}

fn grade(hours: f32, cover: f32) -> ColorGrading {
    let day = daylight(hours);
    let night = 1.0 - day;
    // Cloud flattens *sunlight*: it turns one hard source into a soft one, and
    // the shadows go with it. After dark there is no such source to flatten —
    // the contrast in a night street comes from the lamps, and cloud does
    // nothing to those. Applying the flattening at every hour lifted the whole
    // night street towards mid grey, which is not overcast, it is fogged film.
    let flat = cover.clamp(0.0, 1.0) * day;

    ColorGrading {
        global: ColorGradingGlobal {
            temperature: temperature(hours, cover.clamp(0.0, 1.0)),
            // A touch of magenta at dusk, which is what the sky actually does
            // once the sun is under the horizon and is worth exaggerating.
            tint: 0.010 * (1.0 - (sun_elevation(hours) / 0.18).abs().min(1.0)).max(0.0),
            post_saturation: saturation(hours, cover.clamp(0.0, 1.0)),
            ..default()
        },
        // Lifted, and only here. Crushed blacks are what a night scene has
        // instead of shadow detail, and a city at night is almost all shadow;
        // half a percent of lift is the difference between a dark street and a
        // hole in the frame.
        shadows: ColorGradingSection {
            lift: 0.006 * night,
            contrast: 1.0 - 0.10 * flat,
            ..default()
        },
        midtones: ColorGradingSection {
            contrast: 1.0 - 0.12 * flat,
            ..default()
        },
        // Pulled down under cloud so a white sky stops just short of clipping,
        // which is where the flat look of an overcast day comes from.
        highlights: ColorGradingSection {
            gain: 1.0 - 0.08 * flat,
            ..default()
        },
    }
}

fn grade_the_image(
    clock: Res<TimeOfDay>,
    weather: Res<Weather>,
    mut cameras: Query<&mut ColorGrading, With<super::RenderStack>>,
) {
    let wanted = grade(clock.hours, weather.cover);
    for mut grading in &mut cameras {
        *grading = wanted.clone();
    }
}

// -- the focus --------------------------------------------------------------

/// Puts the focus on whatever the camera is looking at.
///
/// A fixed focal distance would be worse than no depth of field at all: it would
/// blur the car in a showroom shot and sharpen the wall behind it. So the focus
/// is pulled by casting one ray down the view axis and focusing on what it hits,
/// which is both what a rangefinder does and what an operator does.
///
/// Racked rather than snapped. A lens takes a moment, and a focus that jumps the
/// instant a car crosses the frame is far more distracting than one that lags.
fn pull_focus(
    time: Res<Time>,
    spatial: SpatialQuery,
    mut cameras: Query<(&GlobalTransform, &mut DepthOfField), With<CameraRig>>,
) {
    for (transform, mut dof) in &mut cameras {
        let hit = spatial.cast_ray(
            transform.translation(),
            transform.forward(),
            FOCUS_FAR,
            true,
            &SpatialQueryFilter::default(),
        );
        let wanted = hit.map_or(FOCUS_FAR, |hit| hit.distance.max(0.4));
        // About a third of a second to rack the whole way, frame rate aside.
        let rate = 1.0 - (-8.0 * time.delta_secs()).exp();
        dof.focal_distance = dof.focal_distance.lerp(wanted, rate);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dawn and dusk are the same sun at the same angle. If the grade cannot
    /// tell them apart from the weather alone, neither can the player.
    #[test]
    fn a_low_sun_is_graded_warm_and_the_small_hours_cold() {
        // Sunrise is hour 6 and sunset hour 18, so the golden hour is the side
        // of each with the sun still up.
        assert!(temperature(17.6, 0.0) > 0.015, "dusk is not warm");
        assert!(temperature(6.4, 0.0) > 0.015, "dawn is not warm");
        assert!(
            temperature(1.0, 0.0) < -0.015,
            "the small hours are not cold"
        );
        assert!(temperature(12.0, 0.0).abs() < 0.005, "noon is not neutral");
    }

    /// The half hour after sunset is not a continuation of the golden hour, it
    /// is the opposite of it: the warm light has gone under the horizon and what
    /// is left is scattered sky. A grade that stays amber through it is a grade
    /// keyed to the sun's angle without ever asking whether it is up.
    #[test]
    fn the_blue_hour_is_blue() {
        assert!(temperature(18.0, 0.0) > temperature(18.7, 0.0));
        assert!(temperature(18.7, 0.0) < 0.0, "still amber after sunset");
    }

    #[test]
    fn cloud_cools_and_desaturates_whatever_hour_it_is() {
        for hours in [3.0, 8.0, 12.0, 18.3, 22.0] {
            assert!(
                temperature(hours, 1.0) < temperature(hours, 0.0),
                "hour {hours} did not cool under cloud"
            );
            assert!(
                saturation(hours, 1.0) < saturation(hours, 0.0),
                "hour {hours} kept its colour under cloud"
            );
        }
    }

    /// Contrast is the exception: cloud flattens the *sun*, and after dark there
    /// is no sun to flatten. Left coupled to cover alone it lifted a night
    /// street towards mid grey — everything pale, nothing black, which reads as
    /// fogged film rather than as weather.
    #[test]
    fn cloud_flattens_the_day_and_leaves_the_night_alone() {
        let contrast = |hours, cover| grade(hours, cover).midtones.contrast;
        assert!(contrast(12.0, 1.0) < contrast(12.0, 0.0) - 0.05);
        assert!((contrast(1.0, 1.0) - contrast(1.0, 0.0)).abs() < 1e-6);
    }

    #[test]
    fn night_keeps_some_colour_but_not_all_of_it() {
        let noon = saturation(12.0, 0.0);
        let night = saturation(1.0, 0.0);
        assert!(night < noon, "a night as colourful as noon reads as a game");
        assert!(night > 0.7, "and one with no colour reads as a bug");
    }

    /// Everything downstream multiplies by these. A grade that leaves the range
    /// is not a look, it is a broken frame.
    #[test]
    fn the_grade_stays_within_itself_at_every_hour_and_every_sky() {
        for step in 0..240 {
            let hours = step as f32 * 0.1;
            for cover in [0.0, 0.5, 1.0] {
                let grade = grade(hours, cover);
                // The white point has to stay somewhere a real light could be.
                // These are chromaticity offsets from D65 at x = 0.3127, not a
                // -1 to 1 dial, and the whole span from tungsten to an overcast
                // sky is about a tenth wide — see `WHITE_POINT_SHIFT`.
                assert!(WHITE_POINT_SHIFT.contains(&grade.global.temperature));
                assert!((-0.05..=0.05).contains(&grade.global.tint));
                assert!((0.5..=1.2).contains(&grade.global.post_saturation));
                for section in grade.all_sections() {
                    assert!((0.0..=0.05).contains(&section.lift));
                    assert!((0.8..=1.2).contains(&section.contrast));
                    assert!((0.8..=1.2).contains(&section.gain));
                }
            }
        }
    }

    /// Failing to build the curve costs the whole feature, because the fallback
    /// is to go without rather than to meter to middle grey — so it fails
    /// quietly, in a log line nobody reads. It broke exactly once, on a spline
    /// whose segments did not join to the last bit, and that is why this is a
    /// test rather than a warning.
    #[test]
    fn the_metering_curve_builds() {
        assert!(
            metering_curve().is_ok(),
            "auto exposure would silently vanish"
        );
    }

    /// The whole point of the compensation curve. A straight line of slope one
    /// would be no metering at all and a slope of zero would be all of it; the
    /// arithmetic below is what Bevy's shader does with the curve, checked at
    /// the value the constant claims.
    #[test]
    fn the_meter_corrects_partially_and_never_fully() {
        let compensation = |x: f32| METER_AUTHORITY * x + (1.0 - METER_AUTHORITY) * METER_ANCHOR;
        let target = |measured: f32| compensation(measured) - measured;

        // A correctly exposed frame is left exactly alone.
        assert!(target(METER_ANCHOR).abs() < 1e-5);

        // And a dark one is lifted, by less than it is dark.
        let dark = target(METER_ANCHOR - 4.0);
        assert!(dark > 0.5, "four stops down was barely corrected: {dark}");
        assert!(dark < 4.0, "four stops down was fully corrected: {dark}");

        // Symmetrically, at the far end of the curve, where it clamps.
        let bright = target(METER_ANCHOR + METER_RANGE);
        assert!(bright < -0.5 && bright > -METER_RANGE);
    }
}
