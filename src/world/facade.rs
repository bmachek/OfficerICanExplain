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
    /// Above 0.5, the grain is sampled turned ninety degrees.
    pub swap: f32,
}

impl Default for FacadeSettings {
    fn default() -> Self {
        Self {
            tile: GRAIN_TILE,
            strength: 0.70,
            relief: 0.90,
            swap: 0.0,
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

/// Which scanned set each of a district's four palette slots is built from.
///
/// A district is not made of one material — a residential street is brick and
/// render and the odd painted terrace, and an industrial one is concrete with
/// brick warehouses in it. Four slots is what the palette already gives, so
/// four walls per district costs nothing: the materials exist either way.
pub fn walls_for(district: super::citygen::District) -> [&'static str; 4] {
    use super::citygen::District::*;
    match district {
        Residential => [set::BRICK, set::BRICK_PALE, set::BRICK_OLD, set::PLASTER],
        Midtown => [
            set::BRICK_PALE,
            set::PLASTER,
            set::CONCRETE_ROUGH,
            set::BRICK,
        ],
        Downtown => [
            set::CONCRETE,
            set::CONCRETE_ROUGH,
            set::PLASTER,
            set::CONCRETE,
        ],
        Industrial => [
            set::CONCRETE_ROUGH,
            set::BRICK_OLD,
            set::CONCRETE,
            set::BRICK_PALE,
        ],
        // Parks have almost nothing standing on them; whatever does is a
        // pavilion or a substation.
        Park => [
            set::CONCRETE,
            set::CONCRETE_ROUGH,
            set::BRICK_OLD,
            set::CONCRETE,
        ],
    }
}

/// The wall one palette slot of a district is built from.
pub fn grain_for(district: super::citygen::District, slot: usize) -> &'static str {
    let walls = walls_for(district);
    walls[slot % walls.len()]
}

/// How the grain is dressed for each of a district's palette slots.
///
/// The scans differ per slot too, but dressing them differently on top is what
/// stops the same brick reappearing across districts. Scale is the strongest
/// lever — the same brick at 1.7m and at 3.0m reads as two different bricks,
/// because what the eye measures is the course height against the storey — and
/// a quarter turn breaks the last of the resemblance. Both are free: numbers in
/// a uniform that already exists, so the city's material count does not move.
const DRESS: [(f32, f32, bool); 4] = [
    (1.75, 0.72, false),
    (2.40, 0.62, true),
    (2.05, 0.80, true),
    (3.00, 0.66, false),
];

impl FacadeGrain {
    /// The grain for one district, or a bare extension if it was never
    /// downloaded — in which case the shader multiplies by a white texture and
    /// the painted facade shows through untouched.
    pub fn for_district(
        library: &MaterialLibrary,
        district: super::citygen::District,
        palette: usize,
    ) -> Self {
        let scanned = library.get(grain_for(district, palette));
        let (tile, strength, swap) = DRESS[palette % DRESS.len()];
        Self {
            settings: FacadeSettings {
                tile,
                strength,
                swap: if swap { 1.0 } else { 0.0 },
                ..FacadeSettings::default()
            },
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

    const DISTRICTS: [District; 5] = [
        District::Downtown,
        District::Midtown,
        District::Residential,
        District::Industrial,
        District::Park,
    ];

    #[test]
    fn brick_goes_where_people_live_and_concrete_where_they_work() {
        let homes = walls_for(District::Residential);
        assert!(
            homes.iter().filter(|w| w.starts_with("Bricks")).count() >= 3,
            "a residential street should be mostly brick, got {homes:?}"
        );
        let towers = walls_for(District::Downtown);
        assert!(
            towers.iter().filter(|w| w.starts_with("Concrete")).count() >= 3,
            "downtown should be mostly concrete, got {towers:?}"
        );
    }

    #[test]
    fn no_district_is_built_from_a_single_wall() {
        // The whole reason for the extra sets: one material repeated four ways
        // is exactly what this replaced.
        for district in DISTRICTS {
            let walls = walls_for(district);
            let distinct: std::collections::HashSet<_> = walls.iter().collect();
            assert!(
                distinct.len() >= 2,
                "{district:?} is built from one wall: {walls:?}"
            );
        }
    }

    #[test]
    fn the_slot_lookup_stays_inside_its_district() {
        for district in DISTRICTS {
            let walls = walls_for(district);
            for slot in 0..8 {
                assert!(
                    walls.contains(&grain_for(district, slot)),
                    "slot {slot} of {district:?} escaped its own district"
                );
            }
        }
    }

    #[test]
    fn no_two_palette_slots_are_dressed_the_same() {
        // The whole point of the dressing table is that a district's four
        // slots do not look like four copies of one photograph. Two slots
        // agreeing on both scale and orientation would be exactly that.
        for (i, a) in DRESS.iter().enumerate() {
            for b in &DRESS[i + 1..] {
                assert!(
                    (a.0 - b.0).abs() > 0.15 || a.2 != b.2,
                    "two slots share a dressing: {a:?} and {b:?}"
                );
            }
        }
    }

    #[test]
    fn every_dressing_is_a_usable_one() {
        for (tile, strength, _) in DRESS {
            assert!(
                (1.0..=4.0).contains(&tile),
                "a wall grain tiled at {tile}m is either mush or wallpaper"
            );
            assert!((0.0..=1.0).contains(&strength));
        }
    }

    #[test]
    fn every_district_names_sets_the_fetch_script_knows() {
        for district in DISTRICTS {
            for wall in walls_for(district) {
                assert!(
                    set::ALL.contains(&wall),
                    "{district:?} asks for {wall}, which nothing downloads"
                );
            }
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
