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
//! * **Shadows that reach the whole city**, and a short screen-space contact
//!   term underneath everything standing on the ground. See [`shadows`].
//! * **Ambient occlusion and temporal anti-aliasing**, together. SSAO is what
//!   grounds a box on a pavement instead of leaving it floating, and it is
//!   noisy on its own; TAA is what resolves that noise, and it also cleans up
//!   the shimmer along a thousand building edges that MSAA cannot reach.
//! * **Air the light travels through**, so a low sun comes down a side street
//!   as a shaft and a lamp stands in a cone. See [`volumetrics`].
//! * **A grade, a shutter and a lens** on top of all of it. See [`post`].
//!
//! Everything is attached to the camera by a system rather than at spawn,
//! because the camera belongs to `player::camera` and this module should not
//! have to be in the room when it is created.
//!
//! What is attached is decided by [`quality::GraphicsSettings`], not by this
//! module: a preset resolves to a block of numbers, the numbers are walked back
//! to what the GPU reports it can do, and [`sync_camera_stack`] then makes the
//! camera match. The same system runs on every change, so switching preset in
//! the dev panel takes effect without a restart — which is the only way these
//! trades can honestly be compared.

pub mod post;
pub mod quality;
pub mod shadows;
pub mod volumetrics;

use bevy::anti_alias::taa::TemporalAntiAliasing;
use bevy::camera::{Exposure, Hdr};
use bevy::core_pipeline::prepass::{DeferredPrepass, DepthPrepass, MotionVectorPrepass};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::light::atmosphere::ScatteringMedium;
use bevy::light::{Atmosphere, AtmosphereEnvironmentMapLight};
use bevy::pbr::AtmosphereSettings;
use bevy::pbr::{
    DefaultOpaqueRendererMethod, ScreenSpaceAmbientOcclusion,
    ScreenSpaceAmbientOcclusionQualityLevel, ScreenSpaceReflections,
};
use bevy::post_process::bloom::Bloom;
use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;

use crate::core::config::GameConfig;
use crate::player::camera::CameraRig;
use crate::world::timeofday::{TimeOfDay, brightness};
use crate::world::weather::Weather;

use quality::{AoQuality, Capabilities, GraphicsSettings, QualityPreset, Upscaling};

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

/// Marks a camera that has been fitted out, so the base stack is inserted once
/// rather than every frame. The quality-dependent parts are re-synced on every
/// settings change and so do not use this.
#[derive(Component)]
pub struct RenderStack;

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        // The render features themselves ship in `DefaultPlugins`; all this
        // plugin decides is which of them a gameplay camera opts into.
        // Deferred, for every tier, decided here and never again.
        //
        // Not a quality setting, even though only the upper tiers strictly need
        // it. Which pipeline a material compiles for is settled when the
        // material is prepared, so flipping this at runtime leaves already-
        // specialised pipelines behind and geometry disappears; and a city lit
        // by sixty-four street lamps and a pair of headlights is the case
        // deferred shading exists for in the first place.
        //
        // The cost is that every camera drawing the world now needs a g-buffer
        // to draw into. A deferred material is skipped outright in the forward
        // opaque pass, so a camera without `DeferredPrepass` renders nothing at
        // all rather than rendering something worse — which is why the minimap
        // gets one too, over in `ui::minimap`.
        app.add_plugins((volumetrics::VolumetricsPlugin, post::PostPlugin))
            .insert_resource(DefaultOpaqueRendererMethod::deferred())
            .init_resource::<Capabilities>()
            .add_systems(Startup, (probe_capabilities, spawn_atmosphere))
            .add_systems(
                Update,
                (
                    attach_camera_stack,
                    sync_camera_stack,
                    shadows::sync_camera_shadows,
                    shadows::sync_sun_shadows,
                    adapt_exposure,
                )
                    .chain(),
            );
    }
}

/// Asks the GPU what it can do, then walks the requested preset back to fit.
///
/// This has to happen before anything reads `config.graphics`, and it has to
/// happen exactly once: the downgrade is idempotent, but re-running it would
/// keep overwriting a preset the player had since chosen by hand. Runs in
/// `Startup` rather than in the plugin's `finish`, because `RenderDevice` is
/// inserted into the main world by then and a plain system can simply ask for
/// it.
fn probe_capabilities(
    device: Option<Res<RenderDevice>>,
    mut caps: ResMut<Capabilities>,
    mut config: ResMut<GameConfig>,
) {
    *caps = detect(device.as_deref());

    let requested = config.graphics.requested;
    let resolved = config.graphics.clone().downgrade(*caps);
    if resolved != config.graphics {
        info!(
            "graphics: {} requested, running with raytracing={} upscaling={:?}",
            requested.name(),
            resolved.raytracing,
            resolved.upscaling,
        );
    }
    config.graphics = resolved;
}

/// What this GPU supports, as far as the settings care.
///
/// Split out from the system so the mapping from wgpu features to our own flags
/// is one place, and so a headless test can pass `None` and get the honest
/// answer — nothing — rather than a panic.
fn detect(device: Option<&RenderDevice>) -> Capabilities {
    Capabilities {
        // The exact set Solari asks for. Naming the whole set rather than ray
        // query alone matters: the binding-array features are the ones
        // integrated GPUs tend to be missing, and a partial match would load
        // the plugin and then fail inside it.
        #[cfg(feature = "raytracing")]
        raytracing: device.is_some_and(|device| {
            device
                .features()
                .contains(bevy::solari::SolariPlugins::required_wgpu_features())
        }),
        #[cfg(not(feature = "raytracing"))]
        raytracing: false,

        // DLSS is not a wgpu feature — Bevy detects it through Vulkan instance
        // extensions inside the render app, and its own plugin bails out if the
        // driver cannot supply it. So the honest answer from here is "compiled
        // in", and the hardware half is settled where the component is actually
        // attached.
        dlss: cfg!(feature = "dlss") && device.is_some(),
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
///
/// This is the *base*, and it stays the base at every tier. It is driven from
/// the sun's position, which means it knows what time it is — something no
/// histogram can work out from a picture. `post::sync_post` adds a bounded
/// measured correction on top of it for where the camera happens to be looking;
/// see `post::METER_AUTHORITY` for why that correction is not allowed to be the
/// whole of it.
fn adapt_exposure(
    clock: Res<TimeOfDay>,
    weather: Res<Weather>,
    mut cameras: Query<&mut Exposure, With<CameraRig>>,
) {
    // Against how bright it is *outside*, not against where the sun is. Cloud
    // takes nine tenths of the direct beam, so a camera metered for the sun's
    // elevation alone comes out three stops under on an overcast afternoon —
    // and the automatic correction is deliberately bounded well short of
    // recovering three stops, because a correction that large is exactly the
    // one it must not be allowed to make after dark. Better to meter it right.
    let ev100 = NIGHT_EV100 + (DAY_EV100 - NIGHT_EV100) * brightness(clock.hours, weather.cover);
    for mut exposure in &mut cameras {
        if (exposure.ev100 - ev100).abs() > f32::EPSILON {
            exposure.ev100 = ev100;
        }
    }
}

/// Fits out every gameplay camera the first time it is seen.
///
/// Only the parts that no quality tier ever turns off live here. Deliberately
/// not applied to the minimap camera, which renders a flat top-down diagram:
/// bloom and ambient occlusion would cost real time to make it worse.
fn attach_camera_stack(
    mut commands: Commands,
    cameras: Query<Entity, (With<CameraRig>, Without<RenderStack>)>,
) {
    for camera in &cameras {
        commands.entity(camera).insert((
            RenderStack,
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
            // The g-buffer the deferred lighting pass reads, and the depth
            // buffer every screen-space effect raymarches through. Several
            // components below would insert these themselves through their
            // `require`s; naming them here means the camera has them even at a
            // tier that asks for none of those effects, and the pipeline does
            // not change shape as the preset moves.
            DepthPrepass,
            DeferredPrepass,
            // TAA jitters the projection between frames and resolves the result
            // itself. Multisampling on top would fight it, and is not supported
            // alongside ambient occlusion in any case.
            Msaa::Off,
        ));
    }
}

/// Makes the camera match the current settings, and re-runs whenever they move.
///
/// Written as insert-or-remove over each optional component rather than as a
/// rebuild of the whole stack, because a rebuild would drop TAA's history every
/// frame the config was touched — and the dev panel touches it continuously
/// while a slider is being dragged.
fn sync_camera_stack(
    mut commands: Commands,
    config: Res<GameConfig>,
    cameras: Query<Entity, With<RenderStack>>,
    mut applied: Local<Option<GraphicsSettings>>,
) {
    let settings = &config.graphics;
    if applied.as_ref() == Some(settings) && !cameras.is_empty() {
        return;
    }

    for camera in &cameras {
        let mut camera = commands.entity(camera);

        match settings.ssao {
            Some(quality) => {
                camera.insert(ScreenSpaceAmbientOcclusion {
                    quality_level: ao_quality(quality),
                    ..default()
                });
            }
            None => {
                camera.remove::<ScreenSpaceAmbientOcclusion>();
            }
        }

        match settings.upscaling {
            Upscaling::Taa => {
                camera.insert(TemporalAntiAliasing::default());
            }
            // DLSS owns the jitter and the history itself, so TAA has to come
            // off before it goes on; the DLSS component is attached in the
            // raytracing pass once that lands.
            Upscaling::Dlss | Upscaling::Off => {
                camera.remove::<TemporalAntiAliasing>();
            }
        }

        if settings.ssr {
            camera.insert(reflections());
        } else {
            camera.remove::<ScreenSpaceReflections>();
        }

        // Motion vectors are wanted by TAA, by DLSS and by motion blur, and
        // asking for them per-effect is how they end up requested down one code
        // path and forgotten down another.
        if settings.needs_motion_vectors() {
            camera.insert(MotionVectorPrepass);
        } else {
            camera.remove::<MotionVectorPrepass>();
        }
    }

    if !cameras.is_empty() {
        *applied = Some(settings.clone());
    }
}

/// Screen-space reflections, tuned for a street rather than for a floor.
///
/// The defaults assume a broadly flat reflector seen from above. What matters
/// here is wet asphalt seen at a grazing angle down a road, where a ray travels
/// a long way across the screen before it hits anything — so the march gets
/// more steps, and the exponent keeps them dense near the origin where the
/// reflection is sharp instead of spending them all out at the horizon.
///
/// The roughness window is much narrower than the default, and that is the
/// whole of the tuning. Bevy fades SSR out between 0.55 and 0.6, which sounds
/// harmless until you notice what is in that band: car paint sits at 0.37 to
/// 0.47, so every car in the city was being ray-marched. A reflection that
/// rough is a scatter of samples rather than an image, and it showed as
/// salt-and-pepper speckle over every bonnet and roof.
///
/// So the ceiling comes down below paint. What is left inside the window is
/// what genuinely mirrors: standing water at 0.08, wet pavement at 0.15, glass.
/// A car's lacquer really is a mirror, but its *clearcoat* is — the base layer
/// SSR reads is not, and the environment map already reflects the sky into it.
/// The floor comes down too, so a puddle is fully reflective rather than
/// halfway through fading in.
fn reflections() -> ScreenSpaceReflections {
    ScreenSpaceReflections {
        min_perceptual_roughness: 0.0..0.05,
        max_perceptual_roughness: 0.18..0.32,
        linear_steps: 24,
        linear_march_exponent: 2.0,
        bisection_steps: 6,
        use_secant: true,
        thickness: 0.35,
        ..default()
    }
}

fn ao_quality(quality: AoQuality) -> ScreenSpaceAmbientOcclusionQualityLevel {
    match quality {
        AoQuality::Low => ScreenSpaceAmbientOcclusionQualityLevel::Low,
        AoQuality::Medium => ScreenSpaceAmbientOcclusionQualityLevel::Medium,
        AoQuality::High => ScreenSpaceAmbientOcclusionQualityLevel::High,
        AoQuality::Ultra => ScreenSpaceAmbientOcclusionQualityLevel::Ultra,
    }
}

/// Resolves the preset a run should start at.
///
/// Separate from the systems above so `--quality` and the config file agree on
/// one meaning, and so the fallback is testable: an unparseable name is a typo
/// on the command line, and silently starting at Low would be a worse answer
/// than saying so and carrying on at the default.
pub fn preset_from_arg(raw: Option<&str>) -> QualityPreset {
    match raw {
        None => QualityPreset::default(),
        Some(raw) => QualityPreset::parse(raw).unwrap_or_else(|| {
            warn!(
                "unknown --quality {raw:?}; using {}",
                QualityPreset::default().name()
            );
            QualityPreset::default()
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_render_device_reports_no_capabilities_rather_than_panicking() {
        assert_eq!(detect(None), Capabilities::default());
    }

    #[test]
    fn an_absent_quality_flag_is_the_default_preset() {
        assert_eq!(preset_from_arg(None), QualityPreset::default());
    }

    #[test]
    fn a_named_preset_is_honoured() {
        assert_eq!(preset_from_arg(Some("ultra")), QualityPreset::Ultra);
        assert_eq!(preset_from_arg(Some("Photo")), QualityPreset::Photo);
    }

    #[test]
    fn a_typo_falls_back_to_the_default_rather_than_to_the_floor() {
        assert_eq!(preset_from_arg(Some("ulta")), QualityPreset::default());
    }
}
