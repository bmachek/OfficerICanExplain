//! How a frame is put together.
//!
//! The geometry and materials are described elsewhere; this is the part that
//! decides what a camera *does* with them. Four things, in the order they
//! matter:
//!
//! * **A real atmosphere.** Bevy ships Hillaire's 2020 scattering model, so the
//!   sky is integrated rather than painted: the blue overhead, the orange at
//!   the horizon, and the way both change through dusk all fall out of the sun
//!   direction. That also means the sky is a *light source* — see
//!   [`AtmosphereEnvironmentMapLight`], which turns it into an environment map
//!   and is the reason glass towers have anything to reflect.
//! * **Exposure in real units.** Once the sky is physical, the lighting has to
//!   be too: the sun is set in lux and the camera in EV100, the way a camera
//!   pointed at the real thing would be. Nothing here is a number picked to
//!   look right at noon and then fudged for night.
//! * **Bloom**, which is what makes an emissive surface read as *emitting*
//!   rather than merely being bright. Every lit window in the city depends on
//!   it.
//! * **Ambient occlusion and temporal anti-aliasing**, together. SSAO is what
//!   grounds a box on a pavement instead of leaving it floating, and it is
//!   noisy on its own; TAA is what resolves that noise, and it also cleans up
//!   the shimmer along a thousand building edges that MSAA cannot reach.
//!
//! Everything is attached to the camera by a system rather than at spawn,
//! because the camera belongs to `player::camera` and this module should not
//! have to be in the room when it is created.

use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::camera::{Exposure, Hdr};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{Atmosphere, AtmosphereEnvironmentMapLight};
use bevy::pbr::AtmosphereSettings;
use bevy::pbr::ScreenSpaceAmbientOcclusion;
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;

use crate::player::camera::CameraRig;
use crate::world::timeofday::{TimeOfDay, daylight};

/// Aperture the world is metered for at noon. EV100 15 is the sunny-16 rule:
/// what a camera would be set to standing in this street in full daylight.
const DAY_EV100: f32 = 15.0;
/// And what it opens up to after dark. Five stops is roughly the range a pair
/// of eyes covers walking out of a lit room, and without it a physically-lit
/// night is not moody, it is simply black.
const NIGHT_EV100: f32 = 9.7;

/// The city is two kilometres across, not thirty-two. Pulling the aerial
/// perspective range in spends the same thirty-two depth slices over the
/// distances that actually exist, so haze resolves across a street rather than
/// across a mountain range.
const AERIAL_RANGE: f32 = 3_000.0;

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        // The render features themselves ship in `DefaultPlugins`; all this
        // plugin decides is which of them a gameplay camera opts into.
        app.add_systems(Startup, spawn_atmosphere)
            .add_systems(Update, (attach_camera_stack, adapt_exposure).chain());
    }
}

/// The air the city sits in.
///
/// Placed as an entity rather than a camera setting because it is a property of
/// the world: the component's own hook drops the planet centre one Earth radius
/// below the origin, so the ground plane comes out tangent to the surface.
fn spawn_atmosphere(mut commands: Commands, mut mediums: ResMut<Assets<ScatteringMedium>>) {
    commands.spawn((
        Name::new("Atmosphere"),
        Atmosphere {
            // The scattering model draws the planet's own surface out to the
            // true horizon, tens of kilometres past where this city stops.
            // Earth's average 0.3 albedo renders that as a khaki desert; a dark
            // cool value reads as more city, hazed out.
            ground_albedo: Vec3::new(0.13, 0.15, 0.18),
            ..Atmosphere::earth(mediums.add(ScatteringMedium::default()))
        },
    ));
}

/// Opens the aperture as the sun goes down.
///
/// The alternative is a fixed exposure, and it cannot work once the lighting is
/// in real units: the sun is five orders of magnitude brighter than a street
/// lamp, so any single setting either blows out the day or blacks out the
/// night. Eyes solve this by adapting, and so does this.
fn adapt_exposure(clock: Res<TimeOfDay>, mut cameras: Query<&mut Exposure, With<CameraRig>>) {
    let ev100 = NIGHT_EV100 + (DAY_EV100 - NIGHT_EV100) * daylight(clock.hours);
    for mut exposure in &mut cameras {
        if (exposure.ev100 - ev100).abs() > f32::EPSILON {
            exposure.ev100 = ev100;
        }
    }
}

/// Fits out every gameplay camera the first time it is seen.
///
/// Deliberately not applied to the minimap camera, which renders a flat
/// top-down diagram: bloom and ambient occlusion would cost real time to make
/// it worse.
fn attach_camera_stack(
    mut commands: Commands,
    cameras: Query<Entity, (With<CameraRig>, Without<Bloom>)>,
) {
    for camera in &cameras {
        commands.entity(camera).insert((
            // Everything below needs headroom above white to work with.
            Hdr,
            Exposure { ev100: DAY_EV100 },
            // Tony holds its hue as it clips, which matters here: sodium street
            // lighting and lit windows are the brightest things in the frame at
            // night, and a curve that desaturates highlights turns both white.
            Tonemapping::TonyMcMapface,
            Bloom {
                // Well under the default. The city has thousands of lit windows
                // and a bloom tuned for one neon sign turns them into a haze.
                intensity: 0.09,
                ..Bloom::NATURAL
            },
            AtmosphereSettings {
                aerial_view_lut_max_distance: AERIAL_RANGE,
                ..default()
            },
            AtmosphereEnvironmentMapLight {
                // Above unity on purpose. A real street is also lit by every
                // wall and pavement around it bouncing light back, and none of
                // that is modelled; over-driving the sky is the cheapest
                // stand-in, and it is the difference between a shaded street
                // and a black one.
                intensity: 2.2,
                ..default()
            },
            ScreenSpaceAmbientOcclusion::default(),
            TemporalAntiAliasing::default(),
            // TAA jitters the projection between frames and resolves the result
            // itself. Multisampling on top would fight it, and is not supported
            // alongside ambient occlusion in any case.
            Msaa::Off,
        ));
    }
}
