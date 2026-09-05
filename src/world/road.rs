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
//! still recomputes the pavement's colour and roughness each time the weather
//! moves, because a pavement really does just go uniformly damp; the road's
//! wetness is a uniform the shader reads, so it varies per fragment and costs
//! one buffer write per change rather than a pass over the materials.

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

use super::weather::Weather;

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

#[derive(Clone, Copy, Debug, ShaderType, Reflect)]
pub struct RoadSettings {
    pub wetness: f32,
    pub tile: f32,
    /// Seconds. Held at zero when it is not raining, so a merely damp road is
    /// still rather than trembling.
    pub time: f32,
    /// How hard it is falling, which is what decides ripple strength. Not
    /// inferred from `wetness`: a road left glossy after a shower must be still,
    /// and that is a state the weather has and a single dial did not.
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

/// Pushes the current weather into the road material.
///
/// One uniform write when the weather moves, against `WetSurfaces`' pass over
/// every registered material. The clock is only advanced while it is actually
/// raining: a dry road holding a nonzero time would keep re-uploading the
/// uniform every frame for a ripple nobody can see.
fn soak_the_road(
    weather: Res<Weather>,
    time: Res<Time>,
    mut materials: ResMut<Assets<RoadMaterial>>,
    roads: Query<&MeshMaterial3d<RoadMaterial>>,
) {
    let wetness = weather.wetness.clamp(0.0, 1.0);
    let fall = weather.rain.clamp(0.0, 1.0);

    for handle in &roads {
        let Some(mut material) = materials.get_mut(&handle.0) else {
            continue;
        };
        let settings = &mut material.extension.settings;

        if fall > 0.0 {
            settings.time = time.elapsed_secs();
        }
        if (settings.wetness - wetness).abs() > f32::EPSILON
            || (settings.fall - fall).abs() > f32::EPSILON
        {
            settings.wetness = wetness;
            settings.fall = fall;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The puddle field and the asphalt are two patterns laid over the same
    /// surface, and the road only reads as road while they are on different
    /// scales. Both sides are constants, so the interesting half of this is the
    /// compile-time assertion above; this checks the value that reaches the GPU.
    #[test]
    fn a_fresh_road_is_dry_and_still() {
        let settings = RoadSettings::default();
        assert_eq!(settings.wetness, 0.0);
        assert_eq!(settings.fall, 0.0);
        assert_eq!(settings.time, 0.0);
        assert_eq!(settings.tile, PUDDLE_TILE);
    }
}
