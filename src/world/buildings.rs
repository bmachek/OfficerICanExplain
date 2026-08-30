//! Turning city layout data into meshes and colliders.
//!
//! Every building and pavement slab shares ONE unit-cube mesh handle and is
//! sized by its transform. Bevy batches entities that share a mesh *and*
//! material handle into a single draw call, so the whole city costs roughly one
//! draw call per material — about twenty — instead of one per building.
//!
//! The collider is a unit cube too: Avian scales colliders by the entity's
//! global transform, so the same scale drives visual and physical size and the
//! two can never drift apart.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::citygen::{Block, Building, District, PALETTE_SIZE};

/// Pavement height above the road surface.
pub const SIDEWALK_HEIGHT: f32 = 0.28;

#[derive(Resource)]
pub struct CityAssets {
    pub unit_cube: Handle<Mesh>,
    /// Indexed by `district_index * PALETTE_SIZE + palette`.
    building: Vec<Handle<StandardMaterial>>,
    sidewalk: Handle<StandardMaterial>,
    park: Handle<StandardMaterial>,
}

/// Marks which chunk an entity belongs to, so streaming can despawn it.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub struct ChunkOf(pub IVec2);

fn district_index(district: District) -> usize {
    match district {
        District::Downtown => 0,
        District::Midtown => 1,
        District::Residential => 2,
        District::Industrial => 3,
        District::Park => 4,
    }
}

/// Four tones per district. Flat-shaded blocks of colour are the whole art
/// direction here: it needs no textures or artist, and reads as deliberate.
fn palette(district: District) -> [Color; PALETTE_SIZE as usize] {
    match district {
        District::Downtown => [
            Color::srgb(0.40, 0.46, 0.55),
            Color::srgb(0.31, 0.38, 0.48),
            Color::srgb(0.50, 0.55, 0.60),
            Color::srgb(0.26, 0.33, 0.42),
        ],
        District::Midtown => [
            Color::srgb(0.58, 0.55, 0.50),
            Color::srgb(0.47, 0.45, 0.43),
            Color::srgb(0.65, 0.61, 0.55),
            Color::srgb(0.41, 0.40, 0.40),
        ],
        District::Residential => [
            Color::srgb(0.67, 0.55, 0.45),
            Color::srgb(0.73, 0.65, 0.53),
            Color::srgb(0.57, 0.44, 0.37),
            Color::srgb(0.62, 0.58, 0.49),
        ],
        District::Industrial => [
            Color::srgb(0.45, 0.43, 0.40),
            Color::srgb(0.53, 0.42, 0.34),
            Color::srgb(0.37, 0.38, 0.39),
            Color::srgb(0.48, 0.46, 0.41),
        ],
        District::Park => [Color::srgb(0.28, 0.42, 0.24); PALETTE_SIZE as usize],
    }
}

pub fn build_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> CityAssets {
    let districts = [
        District::Downtown,
        District::Midtown,
        District::Residential,
        District::Industrial,
        District::Park,
    ];

    let mut building = Vec::with_capacity(districts.len() * PALETTE_SIZE as usize);
    for district in districts {
        for color in palette(district) {
            building.push(materials.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: 0.85,
                ..default()
            }));
        }
    }

    CityAssets {
        unit_cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        building,
        sidewalk: materials.add(StandardMaterial {
            base_color: Color::srgb(0.54, 0.54, 0.55),
            perceptual_roughness: 0.95,
            ..default()
        }),
        park: materials.add(StandardMaterial {
            base_color: Color::srgb(0.27, 0.41, 0.23),
            perceptual_roughness: 1.0,
            ..default()
        }),
    }
}

impl CityAssets {
    fn material_for(&self, district: District, palette: u8) -> Handle<StandardMaterial> {
        let i = district_index(district) * PALETTE_SIZE as usize + palette as usize;
        self.building[i.min(self.building.len() - 1)].clone()
    }
}

/// Spawns one block's pavement and buildings, tagged for streaming.
pub fn spawn_block(commands: &mut Commands, assets: &CityAssets, block: &Block, chunk: IVec2) {
    let area = block.area;
    let size = area.size();
    let center = area.center();

    // Pavement slab (or grass, for parks). It gets a collider: at 28cm it is a
    // kerb the player steps up onto, and without one they would stand sunk into
    // it. One static box per block is cheap.
    let surface = if block.district == District::Park {
        assets.park.clone()
    } else {
        assets.sidewalk.clone()
    };
    commands.spawn((
        ChunkOf(chunk),
        Mesh3d(assets.unit_cube.clone()),
        MeshMaterial3d(surface),
        Transform::from_xyz(center.x, SIDEWALK_HEIGHT * 0.5, center.y).with_scale(Vec3::new(
            size.x,
            SIDEWALK_HEIGHT,
            size.y,
        )),
        RigidBody::Static,
        Collider::cuboid(1.0, 1.0, 1.0),
    ));

    for building in &block.buildings {
        spawn_building(commands, assets, block.district, building, chunk);
    }
}

fn spawn_building(
    commands: &mut Commands,
    assets: &CityAssets,
    district: District,
    building: &Building,
    chunk: IVec2,
) {
    let size = building.footprint.size();
    let center = building.footprint.center();
    let height = building.height;

    commands.spawn((
        ChunkOf(chunk),
        Mesh3d(assets.unit_cube.clone()),
        MeshMaterial3d(assets.material_for(district, building.palette)),
        Transform::from_xyz(center.x, height * 0.5 + SIDEWALK_HEIGHT, center.y)
            .with_scale(Vec3::new(size.x, height, size.y)),
        RigidBody::Static,
        // Unit cube: Avian scales it by the transform above.
        Collider::cuboid(1.0, 1.0, 1.0),
    ));
}
