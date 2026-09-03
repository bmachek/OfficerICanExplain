//! Scanned PBR materials, and what to do when they are not there.
//!
//! The city's surfaces come from photogrammetry sets under `assets/materials/`,
//! fetched by `tools/fetch-materials.sh`. They are CC0, so nothing is owed to
//! anybody for shipping them — that licence is why these particular sets were
//! chosen over better-looking ones.
//!
//! Three things this module exists to get right:
//!
//! * **Missing is not broken.** The download is 131 MB and gitignored, so a
//!   fresh clone has none of it. Every lookup returns an `Option`, and the
//!   caller falls back to the procedural version in [`super::texture`]. The
//!   game looks worse and runs fine.
//! * **Colour space per map.** Only the colour map is sRGB. Loading roughness
//!   or a normal map through the sRGB curve is the classic way to get walls
//!   that are subtly, inexplicably wrong, so those are loaded linear.
//! * **Mip chains.** Bevy has no runtime mip generator and the loaders produce
//!   a single level. A 2K texture tiled a few hundred times across the ground
//!   without mips does not shimmer, it boils. So every loaded map gets a chain
//!   built on the CPU the frame it arrives.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageLoaderSettings, ImageSampler};
use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;

/// Where the fetch script puts things, relative to `assets/`.
const ROOT: &str = "materials";
/// Resolution and encoding the fetch script asks for; it is part of the
/// filename, so it has to agree with the script.
const VARIANT: &str = "2K-JPG";

/// The scanned sets the city knows how to use.
///
/// Adding one here and to `tools/fetch-materials.sh` is the whole job; the
/// surface that wants it then asks the library by name.
pub mod set {
    pub const ROAD: &str = "Asphalt031";
    pub const PAVEMENT: &str = "PavingStones138";
    pub const CONCRETE: &str = "Concrete034";
    pub const BRICK: &str = "Bricks097";
    pub const ROOF: &str = "Gravel023";
    pub const GRASS: &str = "Grass005";

    pub const ALL: [&str; 6] = [ROAD, PAVEMENT, CONCRETE, BRICK, ROOF, GRASS];
}

/// One scanned material's maps.
///
/// Occlusion is separate rather than packed because the sets ship it that way
/// and Bevy's `StandardMaterial` takes it in its own slot.
#[derive(Clone)]
pub struct ScannedSet {
    pub color: Handle<Image>,
    pub normal: Handle<Image>,
    pub roughness: Handle<Image>,
    pub occlusion: Option<Handle<Image>>,
}

impl ScannedSet {
    /// Points a material's texture slots at this set.
    ///
    /// The two multipliers matter. `perceptual_roughness` scales whatever the
    /// roughness map says, so anything but 1 quietly throws the scan away; and
    /// these sets ship roughness as its own greyscale map rather than packed
    /// glTF-style, which means its blue channel — where `StandardMaterial`
    /// looks for metalness — is a copy of the roughness. Zeroing the metallic
    /// multiplier is what stops wet-looking asphalt from reading as chrome.
    pub fn apply(&self, material: &mut StandardMaterial) {
        material.base_color_texture = Some(self.color.clone());
        material.normal_map_texture = Some(self.normal.clone());
        material.metallic_roughness_texture = Some(self.roughness.clone());
        material.occlusion_texture = self.occlusion.clone();
        material.perceptual_roughness = 1.0;
        material.metallic = 0.0;
    }
}

#[derive(Resource, Default)]
pub struct MaterialLibrary {
    sets: HashMap<&'static str, ScannedSet>,
    /// Maps still waiting for their mip chain. Emptied as they arrive.
    pending: HashSet<AssetId<Image>>,
}

impl MaterialLibrary {
    /// The set under `name`, or `None` if it was never downloaded.
    pub fn get(&self, name: &str) -> Option<&ScannedSet> {
        self.sets.get(name)
    }

    pub fn len(&self) -> usize {
        self.sets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }
}

pub struct MaterialLibraryPlugin;

impl Plugin for MaterialLibraryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MaterialLibrary>()
            // Before `WorldPlugin`'s startup systems, which read the library to
            // decide what each surface is made of.
            .add_systems(PreStartup, discover)
            .add_systems(Update, finish_loaded_maps);
    }
}

/// Path of one map within a set, as the fetch script lays it out.
fn map_path(name: &str, map: &str) -> String {
    format!("{ROOT}/{name}/{name}_{VARIANT}_{map}.jpg")
}

/// Whether a set is actually on disk.
///
/// Checked against the filesystem rather than by trying to load and handling
/// the failure: `AssetServer` reports a missing file asynchronously, long after
/// the materials that need it have already been built.
fn present(name: &str, map: &str) -> bool {
    crate::core::assets::has(&map_path(name, map))
}

fn discover(asset_server: Res<AssetServer>, mut library: ResMut<MaterialLibrary>) {
    // Colour is the one map that is genuinely sRGB-encoded.
    let srgb = |path: String| asset_server.load(path);
    let linear = |path: String| {
        asset_server
            .load_builder()
            .with_settings(|settings: &mut ImageLoaderSettings| {
                settings.is_srgb = false;
                // The mip pass reads the pixels back, so they have to survive
                // the trip into the main world.
                settings.asset_usage = RenderAssetUsages::default();
            })
            .load(path)
    };

    for name in set::ALL {
        if !present(name, "Color") {
            continue;
        }

        let scanned = ScannedSet {
            color: srgb(map_path(name, "Color")),
            // GL, not DX: Bevy follows glTF, whose normal maps have +Y up.
            // Loading the DX variant flips every bump into a dent.
            normal: linear(map_path(name, "NormalGL")),
            roughness: linear(map_path(name, "Roughness")),
            occlusion: present(name, "AmbientOcclusion")
                .then(|| linear(map_path(name, "AmbientOcclusion"))),
        };

        library.pending.extend(
            [
                scanned.color.id(),
                scanned.normal.id(),
                scanned.roughness.id(),
            ]
            .into_iter()
            .chain(scanned.occlusion.as_ref().map(|h| h.id())),
        );
        library.sets.insert(name, scanned);
    }

    if library.is_empty() {
        info!("no scanned materials found; using procedural textures throughout");
    } else {
        info!(
            "{}/{} scanned material sets found in assets/{ROOT}",
            library.len(),
            set::ALL.len()
        );
    }
}

/// Builds a mip chain and applies a tiling sampler as each map finishes loading.
fn finish_loaded_maps(
    mut events: MessageReader<AssetEvent<Image>>,
    mut images: ResMut<Assets<Image>>,
    mut library: ResMut<MaterialLibrary>,
) {
    for event in events.read() {
        let AssetEvent::LoadedWithDependencies { id } = event else {
            continue;
        };
        if !library.pending.remove(id) {
            continue;
        }
        let Some(mut image) = images.get_mut(*id) else {
            continue;
        };
        if let Err(reason) = add_mip_chain(&mut image) {
            warn!("no mip chain for a scanned map ({reason}); it will alias");
        }
        image.sampler = ImageSampler::Descriptor(tiling_sampler());
    }
}

fn tiling_sampler() -> bevy::image::ImageSamplerDescriptor {
    bevy::image::ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        // The road and the pavement are seen almost edge-on almost always,
        // which is precisely where trilinear filtering turns to mud.
        anisotropy_clamp: 16,
        ..default()
    }
}

/// Appends every mip level below the loaded one, in place.
fn add_mip_chain(image: &mut Image) -> Result<(), &'static str> {
    if image.texture_descriptor.mip_level_count > 1 {
        return Ok(());
    }
    let format = image.texture_descriptor.format;
    let srgb = format == TextureFormat::Rgba8UnormSrgb;
    if !matches!(
        format,
        TextureFormat::Rgba8Unorm | TextureFormat::Rgba8UnormSrgb
    ) {
        return Err("unexpected texture format");
    }

    let size = image.texture_descriptor.size;
    if size.depth_or_array_layers != 1 {
        return Err("not a plain 2D image");
    }
    let Some(data) = image.data.as_mut() else {
        return Err("pixels were dropped before the render world");
    };

    let (mut width, mut height) = (size.width, size.height);
    let mut level = data.clone();
    let mut levels = 1;
    while width > 1 || height > 1 {
        let (next, w, h) = super::texture::downsample(&level, width, height, srgb);
        data.extend_from_slice(&next);
        level = next;
        width = w;
        height = h;
        levels += 1;
    }
    image.texture_descriptor.mip_level_count = levels;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::render::render_resource::{Extent3d, TextureDimension};

    #[test]
    fn map_paths_match_what_the_fetch_script_writes() {
        assert_eq!(
            map_path(set::ROAD, "Color"),
            "materials/Asphalt031/Asphalt031_2K-JPG_Color.jpg"
        );
    }

    #[test]
    fn a_mip_chain_is_appended_in_place() {
        let mut image = Image::new_uninit(
            Extent3d {
                width: 4,
                height: 4,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.data = Some(vec![128; 4 * 4 * 4]);

        add_mip_chain(&mut image).expect("4x4 should mip");
        assert_eq!(image.texture_descriptor.mip_level_count, 3, "4, 2, then 1");
        // 16 + 4 + 1 texels, four bytes each.
        assert_eq!(image.data.as_ref().unwrap().len(), (16 + 4 + 1) * 4);
    }

    #[test]
    fn mipping_twice_is_a_no_op() {
        let mut image = Image::new_uninit(
            Extent3d {
                width: 2,
                height: 2,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            TextureFormat::Rgba8Unorm,
            RenderAssetUsages::default(),
        );
        image.data = Some(vec![200; 2 * 2 * 4]);

        add_mip_chain(&mut image).unwrap();
        let once = image.data.clone();
        add_mip_chain(&mut image).unwrap();
        assert_eq!(image.data, once, "a second pass must not stack more levels");
    }

    #[test]
    fn every_named_set_is_in_the_all_list() {
        for name in [
            set::ROAD,
            set::PAVEMENT,
            set::CONCRETE,
            set::BRICK,
            set::ROOF,
            set::GRASS,
        ] {
            assert!(set::ALL.contains(&name), "{name} missing from ALL");
        }
    }
}
