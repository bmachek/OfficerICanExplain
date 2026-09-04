//! The material the road surface is drawn with.
//!
//! A [`StandardMaterial`] extended with standing water. See
//! `assets/shaders/road.wgsl` for the reasoning; briefly, wetness used to be
//! one number applied to the whole road — darken it, drop its roughness — which
//! is right about what water does and wrong about where it is. Rain puddles. A
//! road that goes uniformly glossy reads as varnish.
//!
//! The mask is computed in world space rather than in the mesh's UV, for the
//! same reason the facade's grain is: the road is a single quad forty
//! kilometres across with its UVs multiplied by about six thousand, so anything
//! sampled in UV space repeats every six metres. Puddles on a six-metre grid
//! are a pattern.
//!
//! This is also where wetness stops being a material mutation. `WetSurfaces`
//! still recomputes the pavement's colour and roughness each time the dial
//! moves, because a pavement really does just go uniformly damp; the road's
//! wetness is now a uniform the shader reads, so it varies per fragment and
//! costs one buffer write per change rather than a pass over the materials.

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use crate::core::config::GameConfig;

const SHADER: &str = "shaders/road.wgsl";

/// Metres across one repeat of the puddle field.
///
/// Puddles want to be the size of the dips that hold them — a couple of metres
/// across on a road that has settled, not the six-metre grid a UV-space mask
/// would give and not the fifty-metre lakes a much larger figure would.
const PUDDLE_TILE: f32 = 9.5;

// Puddles want to be the size of the dips that hold them. Below the asphalt
// tile this stops being weather and becomes a texture pattern; far above it,
// the road floods in lakes. Checked at compile time rather than in a test,
// because both sides are constants and a test would only ever be re-running
// the compiler's arithmetic.
const _: () = assert!(PUDDLE_TILE > super::ASPHALT_TILE);
const _: () = assert!(PUDDLE_TILE < 30.0);

/// Above this much wetness it is actually raining, so the water is moving.
///
/// The same threshold `weather` uses to decide whether to draw falling rain,
/// and it has to be: ripples on a road under a clear sky are the kind of detail
/// that is only ever noticed when it is wrong.
pub const RAINING_ABOVE: f32 = 0.35;

#[derive(Clone, Copy, Debug, ShaderType, Reflect)]
pub struct RoadSettings {
    pub wetness: f32,
    pub tile: f32,
    /// Seconds. Held at zero when it is not raining, so a merely damp road is
    /// still rather than trembling.
    pub time: f32,
    /// How hard it is falling, which is what decides ripple strength.
    pub fall: f32,
}

impl Default for RoadSettings {
    fn default() -> Self {
        Self {
            wetness: 0.0,
            tile: PUDDLE_TILE,
            time: 0.0,
            fall: 0.0,
        }
    }
}

/// The standing-water half of the road material.
#[derive(Asset, AsBindGroup, Reflect, Clone, Default)]
pub struct RoadSheen {
    #[uniform(100)]
    pub settings: RoadSettings,
}

impl MaterialExtension for RoadSheen {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }

    /// The same file, branching on `PREPASS_PIPELINE`. This one is not optional
    /// in the way the forward path is: screen-space reflections read the
    /// g-buffer, so a puddle whose low roughness never got written into it
    /// would reflect nothing — which is the entire point of the puddle.
    fn deferred_fragment_shader() -> ShaderRef {
        SHADER.into()
    }
}

pub type RoadMaterial = ExtendedMaterial<StandardMaterial, RoadSheen>;

pub struct RoadPlugin;

impl Plugin for RoadPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<RoadMaterial>::default())
            .add_systems(Update, soak_the_road);
    }
}

/// How hard it is falling, from how wet the ground is.
///
/// Wetness is the only weather input there is, so rainfall has to be inferred
/// from it. Below the threshold the ground is merely damp — drying out after
/// the fact, or splashed — and nothing is falling.
pub fn rainfall(wetness: f32) -> f32 {
    if wetness <= RAINING_ABOVE {
        return 0.0;
    }
    ((wetness - RAINING_ABOVE) / (1.0 - RAINING_ABOVE)).clamp(0.0, 1.0)
}

/// Pushes the current wetness into the road material.
///
/// One uniform write when the dial moves, against `WetSurfaces`' pass over
/// every registered material. The clock is only advanced while it is actually
/// raining: a dry road holding a nonzero time would keep re-uploading the
/// uniform every frame for a ripple nobody can see.
fn soak_the_road(
    config: Res<GameConfig>,
    time: Res<Time>,
    mut materials: ResMut<Assets<RoadMaterial>>,
    roads: Query<&MeshMaterial3d<RoadMaterial>>,
) {
    let wetness = config.world.wetness.clamp(0.0, 1.0);
    let fall = rainfall(wetness);

    for handle in &roads {
        let Some(mut material) = materials.get_mut(&handle.0) else {
            continue;
        };
        let settings = &mut material.extension.settings;

        if fall > 0.0 {
            settings.time = time.elapsed_secs();
        }
        if (settings.wetness - wetness).abs() > f32::EPSILON {
            settings.wetness = wetness;
            settings.fall = fall;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dry_road_has_nothing_falling_on_it() {
        assert_eq!(rainfall(0.0), 0.0);
        assert_eq!(rainfall(RAINING_ABOVE), 0.0);
    }

    /// Damp is not rain. A road left wet after a shower, or splashed at a kerb,
    /// must not sprout ripples — that is the tell that the effect is driven by
    /// a number rather than by weather.
    #[test]
    fn a_merely_damp_road_is_still() {
        assert_eq!(rainfall(0.2), 0.0);
        assert_eq!(rainfall(0.34), 0.0);
    }

    #[test]
    fn rainfall_climbs_with_wetness_and_tops_out_at_one() {
        assert!(rainfall(0.5) > 0.0);
        assert!(rainfall(0.8) > rainfall(0.5));
        assert_eq!(rainfall(1.0), 1.0);
    }

    /// The dial is clamped elsewhere, but a value out of range reaching the
    /// shader would drive the ripple past its amplitude.
    #[test]
    fn a_wetness_out_of_range_does_not_escape_the_ramp() {
        assert_eq!(rainfall(4.0), 1.0);
        assert_eq!(rainfall(-1.0), 0.0);
    }
}
