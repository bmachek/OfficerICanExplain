//! The material buildings are drawn with.
//!
//! A [`StandardMaterial`] extended with one thing: a scanned wall grain sampled
//! in world space rather than in the mesh's UV. See `assets/shaders/facade.wgsl`
//! for why — briefly, the painted facade texture has to cover a whole building
//! face to place the windows, and at that coverage there is nowhere near enough
//! resolution left for the material to read as concrete or brick.
//!
//! Sampling the grain in world space means its scale is a property of the world
//! instead of the building, so a two-storey house and a forty-storey tower can
//! share one material. Without that the material count picks up a size bucket
//! dimension, and the city's twenty-odd draw calls become several hundred.

use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::{Shader, ShaderRef};

use super::material::{MaterialLibrary, set};

const SHADER: &str = "shaders/facade.wgsl";

/// Metres of wall covered by one repeat of the grain.
///
/// Roughly the size the sets were photographed at. Stretching them buys a
/// slower repeat at the cost of the detail that is the entire reason they are
/// here.
const GRAIN_TILE: f32 = 2.2;

#[derive(Clone, Copy, Debug, ShaderType, Reflect)]
pub struct FacadeSettings {
    pub tile: f32,
    /// 0 leaves the painted wall alone; 1 lets the scan swing the wall's colour
    /// by its own full contrast. Above that it over-swings — darker than
    /// anything in the photograph, and brighter.
    pub strength: f32,
    pub relief: f32,
    pub _pad: f32,
}

impl Default for FacadeSettings {
    fn default() -> Self {
        Self {
            tile: GRAIN_TILE,
            strength: 0.70,
            relief: 0.90,
            _pad: 0.0,
        }
    }
}

/// The grain half of a facade material.
#[derive(Asset, AsBindGroup, Reflect, Clone, Default)]
pub struct FacadeGrain {
    #[uniform(100)]
    pub settings: FacadeSettings,
    #[texture(101)]
    #[sampler(102)]
    pub color: Option<Handle<Image>>,
    #[texture(103)]
    #[sampler(104)]
    pub normal: Option<Handle<Image>>,
}

impl MaterialExtension for FacadeGrain {
    fn fragment_shader() -> ShaderRef {
        SHADER.into()
    }
}

/// What every building in the city is drawn with.
pub type FacadeMaterial = ExtendedMaterial<StandardMaterial, FacadeGrain>;

pub struct FacadePlugin;

impl Plugin for FacadePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<FacadeMaterial>::default());
    }
}

/// Which scanned set a district's walls are made of.
///
/// Only two, because these are the two things a city is actually built from at
/// this distance. Anything more is a variety problem, not a material one.
pub fn grain_for(district: super::citygen::District) -> &'static str {
    use super::citygen::District::*;
    match district {
        Residential => set::BRICK,
        Downtown | Midtown | Industrial | Park => set::CONCRETE,
    }
}

impl FacadeGrain {
    /// The grain for one district, or a bare extension if it was never
    /// downloaded — in which case the shader multiplies by a white texture and
    /// the painted facade shows through untouched.
    pub fn for_district(library: &MaterialLibrary, district: super::citygen::District) -> Self {
        let scanned = library.get(grain_for(district));
        Self {
            settings: FacadeSettings::default(),
            color: scanned.map(|s| s.color.clone()),
            normal: scanned.map(|s| s.normal.clone()),
        }
    }
}

/// Loaded so the shader is compiled before the first building is drawn rather
/// than on the frame one comes into view.
#[derive(Resource)]
pub struct FacadeShader(#[allow(dead_code)] Handle<Shader>);

pub fn load_shader(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(FacadeShader(asset_server.load(SHADER)));
}

#[cfg(test)]
mod tests {
    use super::super::citygen::District;
    use super::*;

    #[test]
    fn brick_goes_on_houses_and_concrete_on_everything_else() {
        assert_eq!(grain_for(District::Residential), set::BRICK);
        assert_eq!(grain_for(District::Downtown), set::CONCRETE);
        assert_eq!(grain_for(District::Industrial), set::CONCRETE);
    }

    #[test]
    fn every_district_names_a_set_the_fetch_script_knows() {
        for district in [
            District::Downtown,
            District::Midtown,
            District::Residential,
            District::Industrial,
            District::Park,
        ] {
            assert!(
                set::ALL.contains(&grain_for(district)),
                "{district:?} asks for a set nothing downloads"
            );
        }
    }

    #[test]
    fn the_grain_is_tiled_at_roughly_the_size_it_was_photographed() {
        let settings = FacadeSettings::default();
        assert!(
            (1.0..=4.0).contains(&settings.tile),
            "a wall grain tiled at {}m is either mush or wallpaper",
            settings.tile
        );
        // The shader mixes towards the scan's own contrast, so anything above
        // 1 over-swings: the wall would go darker than the darkest thing in the
        // photograph and brighter than the brightest.
        assert!(
            (0.0..=1.0).contains(&settings.strength),
            "grain strength {} is outside what the mix can mean",
            settings.strength
        );
        assert!(
            (0.0..=2.0).contains(&settings.relief),
            "relief {} would tilt the surface past the wall it sits on",
            settings.relief
        );
    }
}
