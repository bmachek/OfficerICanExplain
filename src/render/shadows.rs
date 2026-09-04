//! Where the sun does not reach.
//!
//! Shadows were the largest single thing wrong with the picture, and none of it
//! was subtle once you knew to look:
//!
//! * **They stopped at 150 metres.** Nothing here ever configured the cascades,
//!   so Bevy's default applied — a distance chosen for a third-person game in a
//!   room, in a city streamed out to nine hundred. From the air the far half of
//!   the skyline was lit from every direction at once.
//! * **Nothing was planted.** A shadow map texel covering half a metre cannot
//!   resolve the gap between a bollard and the pavement it stands on, so the
//!   bollard floated. So did every wheel, every kerb and every lamp post.
//! * **Every edge was equally hard**, whether it was cast by a parapet forty
//!   metres up or by a wing mirror ten centimetres off a door.
//!
//! Three fixes, and they are deliberately different in kind. The cascade split
//! is *geometry*: it decides which distances get shadow-mapped at all.
//! [`ContactShadows`] is a *screen-space* pass that puts back the short, dark
//! contact that no shadow map resolution can afford. Soft shadows are
//! *filtering*: the penumbra widens with distance from the caster, the way a
//! real one does.
//!
//! The sun itself belongs to `world::timeofday` — where it is and what colour
//! it is are facts about the world. What its shadows *cost* is a rendering
//! decision, so it lives here and is driven from
//! [`crate::render::quality::GraphicsSettings`].

use bevy::light::{
    CascadeShadowConfig, CascadeShadowConfigBuilder, DirectionalLightShadowMap,
    ShadowFilteringMethod,
};
use bevy::pbr::ContactShadows;
use bevy::prelude::*;

use crate::core::config::GameConfig;
use crate::render::RenderStack;
use crate::render::quality::{GraphicsSettings, Upscaling};
use crate::world::timeofday::Sun;

/// The near plane of the first cascade.
///
/// Not zero: the first cascade is the one paying for the fine detail, and
/// starting it half a metre out rather than at the near plane spends its texels
/// on the street instead of on the inside of the player's own model.
const NEAREST: f32 = 0.5;

/// Overlap between neighbouring cascades, as a fraction.
///
/// Bevy's default, and there is no reason to move it: it is what dithers the
/// switch between two shadow maps so the seam does not draw a line across the
/// road as the camera moves.
const OVERLAP: f32 = 0.2;

/// The angular size the sun's penumbra is grown from.
///
/// The real sun subtends about half a degree, which produces a penumbra far too
/// tight to see at street scale. This is larger on purpose — it is the softness
/// of a bright overcast sky as much as of a disc, and it is what stops a
/// forty-storey parapet drawing a razor edge across a road.
const SUN_SOFTNESS: f32 = 3.0;

/// Splits the shadowed range into cascades, without ever handing Bevy something
/// it will panic on.
///
/// [`CascadeShadowConfigBuilder::build`] asserts its way through five
/// preconditions, and the settings feeding it are a preset table *and* a dev
/// panel slider — so the inputs genuinely can be anything. Clamping here rather
/// than trusting the caller is what makes dragging that slider safe.
///
/// The shadow distance is also pulled in to the streaming radius: shadowing a
/// building that has not been spawned costs cascade resolution and buys an
/// empty map. A little past it, because chunks are resident somewhat beyond the
/// radius and their shadows should not stop dead at a circle.
pub fn cascade_config(
    shadow_distance: f32,
    cascades: usize,
    stream_radius: f32,
) -> CascadeShadowConfig {
    let reach = stream_radius.max(NEAREST * 4.0) * 1.15;
    let maximum = shadow_distance.clamp(NEAREST * 4.0, reach);

    // The first bound decides how much of the map the street in front of the
    // camera gets. Proportional to the total, because a run configured for two
    // kilometres wants a longer near cascade than one configured for three
    // hundred, but bounded at both ends: too short and the second cascade takes
    // over inside the crossing you are standing on, too long and the near
    // cascade is as coarse as the far one.
    let first = (maximum * 0.03)
        .clamp(NEAREST * 4.0 + 1.0, 40.0)
        .min(maximum * 0.5);

    CascadeShadowConfigBuilder {
        num_cascades: cascades.clamp(1, 4),
        minimum_distance: NEAREST,
        first_cascade_far_bound: first,
        maximum_distance: maximum,
        overlap_proportion: OVERLAP,
    }
    .build()
}

/// Applies the current settings to the sun and to the shadow map resource.
///
/// Re-runs on every settings change rather than once at startup, so the dev
/// panel can put two shadow distances side by side without a restart — which is
/// the only way to judge what the extra distance actually costs.
pub fn sync_sun_shadows(
    config: Res<GameConfig>,
    mut shadow_map: ResMut<DirectionalLightShadowMap>,
    mut sun: Query<(&mut DirectionalLight, &mut CascadeShadowConfig), With<Sun>>,
    mut applied: Local<Option<GraphicsSettings>>,
) {
    let settings = &config.graphics;
    if applied.as_ref() == Some(settings) && !sun.is_empty() {
        return;
    }

    if shadow_map.size != settings.shadow_map_size {
        shadow_map.size = settings.shadow_map_size;
    }

    for (mut light, mut cascades) in &mut sun {
        *cascades = cascade_config(
            settings.shadow_distance,
            settings.cascades,
            config.world.stream_radius,
        );
        light.soft_shadow_size = settings.soft_shadows.then_some(SUN_SOFTNESS);
    }

    if !sun.is_empty() {
        *applied = Some(settings.clone());
    }
}

/// Puts the short-range contact shadow and the right shadow filter on the camera.
///
/// The filter choice is not a quality dial, it is a pairing: `Temporal` varies
/// its sample pattern between frames and is only good *because* something
/// resolves that variation afterwards. With no temporal pass it is noise, so it
/// follows the upscaling setting rather than the tier.
pub fn sync_camera_shadows(
    mut commands: Commands,
    config: Res<GameConfig>,
    cameras: Query<Entity, With<RenderStack>>,
    mut applied: Local<Option<GraphicsSettings>>,
) {
    let settings = &config.graphics;
    if applied.as_ref() == Some(settings) && !cameras.is_empty() {
        return;
    }

    let filtering = match settings.upscaling {
        Upscaling::Taa | Upscaling::Dlss => ShadowFilteringMethod::Temporal,
        Upscaling::Off => ShadowFilteringMethod::Gaussian,
    };

    for camera in &cameras {
        let mut camera = commands.entity(camera);
        camera.insert(filtering);

        if settings.contact_shadows {
            camera.insert(ContactShadows::default());
        } else {
            camera.remove::<ContactShadows>();
        }
    }

    if !cameras.is_empty() {
        *applied = Some(settings.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::quality::QualityPreset;

    /// The builder panics on five separate preconditions, and the inputs come
    /// from a slider. This is the test that matters in this file.
    #[test]
    fn no_setting_a_player_can_reach_makes_the_cascade_builder_panic() {
        let distances = [
            f32::MIN,
            -1.0,
            0.0,
            0.001,
            1.0,
            100.0,
            900.0,
            2000.0,
            1.0e9,
            f32::MAX,
        ];
        let counts = [0, 1, 2, 4, 8, usize::MAX];
        let radii = [0.0, 1.0, 250.0, 900.0, 1800.0, 1.0e9];

        for distance in distances {
            for count in counts {
                for radius in radii {
                    let config = cascade_config(distance, count, radius);
                    assert!(!config.bounds.is_empty());
                    assert!(config.bounds.iter().all(|b| b.is_finite() && *b > 0.0));
                }
            }
        }
    }

    #[test]
    fn every_preset_produces_a_usable_split() {
        for preset in QualityPreset::ALL {
            let settings = preset.settings();
            let config = cascade_config(settings.shadow_distance, settings.cascades, 900.0);
            assert_eq!(
                config.bounds.len(),
                settings.cascades.clamp(1, 4),
                "{} lost a cascade",
                preset.name()
            );
            // Bounds must climb, or a nearer cascade would be sampled for
            // geometry a further one already covers.
            assert!(
                config.bounds.windows(2).all(|w| w[1] > w[0]),
                "{} produced non-monotonic bounds: {:?}",
                preset.name(),
                config.bounds
            );
        }
    }

    /// The whole point of the change. If this ever passes at 150 again, the
    /// cascade config has been dropped somewhere.
    #[test]
    fn the_sun_shadows_the_whole_streamed_city_rather_than_the_first_hundred_metres() {
        let settings = QualityPreset::High.settings();
        let config = cascade_config(settings.shadow_distance, settings.cascades, 900.0);
        let furthest = *config.bounds.last().unwrap();
        assert!(
            furthest > 800.0,
            "shadows stop at {furthest} m in a 900 m city"
        );
    }

    /// Shadowing geometry that was never spawned costs resolution and buys an
    /// empty map, so an aggressive slider has to be pulled back to the streamed
    /// radius rather than believed.
    #[test]
    fn the_shadow_distance_is_pulled_back_to_what_is_actually_resident() {
        let config = cascade_config(5_000.0, 4, 400.0);
        let furthest = *config.bounds.last().unwrap();
        // Bevy computes the last bound as `first * base^(n-1)` rather than
        // taking the maximum verbatim, so it lands a few ulps either side of
        // it. A relative tolerance, because an absolute one in units of
        // `f32::EPSILON` is meaningless at this magnitude.
        let limit = 400.0 * 1.15;
        assert!(
            furthest <= limit * 1.001,
            "shadowing out to {furthest} m with a 400 m stream radius"
        );
    }

    /// A short shadow distance must not leave the first cascade covering
    /// everything, or the near detail this whole split exists for is lost.
    #[test]
    fn the_near_cascade_never_swallows_the_whole_range() {
        for distance in [10.0, 40.0, 120.0, 300.0] {
            let config = cascade_config(distance, 4, 2000.0);
            let first = config.bounds[0];
            let furthest = *config.bounds.last().unwrap();
            assert!(
                first < furthest,
                "at {distance} m the first cascade reaches {first} of {furthest}"
            );
        }
    }
}
