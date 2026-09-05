//! Turning city layout data into meshes, materials and colliders.
//!
//! Every building, kerb and roof shares ONE unit-cube mesh handle and is sized
//! by its transform; every block top shares one unit quad. Bevy batches
//! entities that share a mesh *and* material handle into a single draw call, so
//! the city costs roughly one draw call per material rather than one per
//! building.
//!
//! Texturing widens that material table, and it is worth being honest about the
//! trade. A facade needs its window grid to match the building's height, and
//! that is a property of the entity, not the material — so buildings are
//! bucketed into four height classes and the table becomes
//! `districts x palette x class`, about eighty materials instead of twenty.
//! Eighty draw calls for an entire city is still nothing; eighty *thousand*
//! would not be, which is why the bucket count stays small and fixed.
//!
//! The cube mesh is built here rather than taken from `Cuboid`, whose UVs are
//! rotated a quarter turn on the ±X faces and flipped on -Z. That is invisible
//! on noise and glaring on a window grid: two walls of every building would
//! have had their floors running vertically.
//!
//! The collider is a unit cube too: Avian scales colliders by the entity's
//! global transform, so the same scale drives visual and physical size and the
//! two can never drift apart.

use avian3d::prelude::*;
use bevy::math::Affine2;
use bevy::prelude::*;
use bevy::render::mesh::{Indices, PrimitiveTopology};

use super::citygen::{Block, Building, District, PALETTE_SIZE};
use bevy::camera::visibility::VisibilityRange;
use bevy::light::NotShadowCaster;

use super::rooftop::{self, RoofKit};
use super::shell::{self, ShellKit};
use super::texture::{self, FacadeClass};

/// Pavement height above the road surface.
pub const SIDEWALK_HEIGHT: f32 = 0.28;

/// Height of the plinth course at the foot of every building, in metres.
///
/// Buildings met the pavement at a bare edge, which is the join a real street
/// never has: there is always a plinth, a step, a stall riser or at minimum a
/// change of material, and its shadow line is what makes a wall look like it is
/// *standing on* the ground rather than pushed into it.
const PLINTH_HEIGHT: f32 = 0.62;
/// How far the plinth stands proud of the wall above it.
const PLINTH_PROUD: f32 = 0.11;
/// How far away the plinth stops being drawn, before `lod_scale`.
///
/// It is eleven centimetres deep. Past a couple of hundred metres that is well
/// under a pixel, and all it contributes is another edge for the anti-aliasing
/// to chew on.
const PLINTH_RANGE: f32 = 260.0;

/// Roughly how wide a paving slab or a patch of grass should be, in metres.
const GROUND_TILE: f32 = 2.6;
/// Tiling factors block tops are quantised to, so they can share materials.
const GROUND_BUCKETS: [f32; 4] = [8.0, 12.0, 17.0, 24.0];

const CLASS_COUNT: usize = FacadeClass::ALL.len();

#[derive(Resource)]
pub struct CityAssets {
    pub unit_cube: Handle<Mesh>,
    /// A 1x1 quad in the XZ plane, laid over each block as its walking surface.
    unit_quad: Handle<Mesh>,
    /// Indexed by `(district_index * PALETTE_SIZE + palette) * CLASS_COUNT + class`.
    building: Vec<Handle<super::facade::FacadeMaterial>>,
    roof: Handle<StandardMaterial>,
    kerb: Handle<StandardMaterial>,
    park_kerb: Handle<StandardMaterial>,
    /// One per entry in [`GROUND_BUCKETS`].
    paving: Vec<Handle<StandardMaterial>>,
    grass: Vec<Handle<StandardMaterial>>,
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

/// Four tones per district, tinting a shared greyscale facade. Blocks of
/// colour are still the art direction; the texture only says where the windows
/// are.
fn palette(district: District) -> [Color; PALETTE_SIZE as usize] {
    match district {
        District::Downtown => [
            Color::srgb(0.42, 0.48, 0.58),
            Color::srgb(0.33, 0.40, 0.51),
            Color::srgb(0.53, 0.58, 0.63),
            Color::srgb(0.28, 0.35, 0.45),
        ],
        District::Midtown => [
            Color::srgb(0.61, 0.58, 0.53),
            Color::srgb(0.50, 0.48, 0.46),
            Color::srgb(0.69, 0.65, 0.58),
            Color::srgb(0.44, 0.43, 0.43),
        ],
        District::Residential => [
            Color::srgb(0.71, 0.58, 0.48),
            Color::srgb(0.77, 0.69, 0.56),
            Color::srgb(0.60, 0.47, 0.39),
            Color::srgb(0.66, 0.61, 0.52),
        ],
        District::Industrial => [
            Color::srgb(0.48, 0.46, 0.42),
            Color::srgb(0.56, 0.45, 0.36),
            Color::srgb(0.39, 0.40, 0.41),
            Color::srgb(0.51, 0.49, 0.44),
        ],
        District::Park => [Color::srgb(0.30, 0.44, 0.26); PALETTE_SIZE as usize],
    }
}

/// A unit cube whose six faces all agree about which way is up.
///
/// Side faces run U along their horizontal axis and V from the bottom edge to
/// the top; the top and bottom map U to X and V to Z. Without that, a facade
/// texture arrives sideways on two walls out of four.
fn unit_cube_mesh() -> Mesh {
    // (position, normal, uv), four vertices per face.
    let faces: [([f32; 3], [f32; 3], [f32; 2]); 24] = [
        // +Z
        ([-0.5, -0.5, 0.5], [0.0, 0.0, 1.0], [0.0, 0.0]),
        ([0.5, -0.5, 0.5], [0.0, 0.0, 1.0], [1.0, 0.0]),
        ([0.5, 0.5, 0.5], [0.0, 0.0, 1.0], [1.0, 1.0]),
        ([-0.5, 0.5, 0.5], [0.0, 0.0, 1.0], [0.0, 1.0]),
        // -Z
        ([0.5, -0.5, -0.5], [0.0, 0.0, -1.0], [0.0, 0.0]),
        ([-0.5, -0.5, -0.5], [0.0, 0.0, -1.0], [1.0, 0.0]),
        ([-0.5, 0.5, -0.5], [0.0, 0.0, -1.0], [1.0, 1.0]),
        ([0.5, 0.5, -0.5], [0.0, 0.0, -1.0], [0.0, 1.0]),
        // +X
        ([0.5, -0.5, 0.5], [1.0, 0.0, 0.0], [0.0, 0.0]),
        ([0.5, -0.5, -0.5], [1.0, 0.0, 0.0], [1.0, 0.0]),
        ([0.5, 0.5, -0.5], [1.0, 0.0, 0.0], [1.0, 1.0]),
        ([0.5, 0.5, 0.5], [1.0, 0.0, 0.0], [0.0, 1.0]),
        // -X
        ([-0.5, -0.5, -0.5], [-1.0, 0.0, 0.0], [0.0, 0.0]),
        ([-0.5, -0.5, 0.5], [-1.0, 0.0, 0.0], [1.0, 0.0]),
        ([-0.5, 0.5, 0.5], [-1.0, 0.0, 0.0], [1.0, 1.0]),
        ([-0.5, 0.5, -0.5], [-1.0, 0.0, 0.0], [0.0, 1.0]),
        // +Y
        ([-0.5, 0.5, 0.5], [0.0, 1.0, 0.0], [0.0, 0.0]),
        ([0.5, 0.5, 0.5], [0.0, 1.0, 0.0], [1.0, 0.0]),
        ([0.5, 0.5, -0.5], [0.0, 1.0, 0.0], [1.0, 1.0]),
        ([-0.5, 0.5, -0.5], [0.0, 1.0, 0.0], [0.0, 1.0]),
        // -Y
        ([-0.5, -0.5, -0.5], [0.0, -1.0, 0.0], [0.0, 0.0]),
        ([0.5, -0.5, -0.5], [0.0, -1.0, 0.0], [1.0, 0.0]),
        ([0.5, -0.5, 0.5], [0.0, -1.0, 0.0], [1.0, 1.0]),
        ([-0.5, -0.5, 0.5], [0.0, -1.0, 0.0], [0.0, 1.0]),
    ];

    let indices: Vec<u32> = (0..6u32)
        .flat_map(|face| {
            let base = face * 4;
            [base, base + 1, base + 2, base + 2, base + 3, base]
        })
        .collect();

    Mesh::new(
        PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        faces.iter().map(|(p, _, _)| *p).collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        faces.iter().map(|(_, n, _)| *n).collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        faces.iter().map(|(_, _, uv)| *uv).collect::<Vec<_>>(),
    )
    .with_inserted_indices(Indices::U32(indices))
}

/// Adds a mikktspace tangent basis, or leaves the mesh alone and says so.
///
/// A missing tangent attribute makes the normal-mapped pipeline fail to build
/// rather than fall back, so a loud warning beats a silently black city.
pub fn with_tangents(mut mesh: Mesh) -> Mesh {
    if let Err(error) = mesh.generate_tangents() {
        warn!("no tangents for a mesh, normal maps will be wrong: {error}");
    }
    mesh
}

/// Nearest tiling factor that puts ground tiles near [`GROUND_TILE`] across a
/// surface `extent` metres wide.
fn ground_bucket(extent: f32) -> usize {
    let wanted = (extent / GROUND_TILE).max(1.0);
    GROUND_BUCKETS
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (*a - wanted).abs().total_cmp(&(*b - wanted).abs()))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

pub fn build_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
    library: &super::material::MaterialLibrary,
    facades_out: &mut Assets<super::facade::FacadeMaterial>,
    wet: &mut super::weather::WetSurfaces,
) -> CityAssets {
    let districts = [
        District::Downtown,
        District::Midtown,
        District::Residential,
        District::Industrial,
        District::Park,
    ];

    // One set of facade maps per height class, shared by every district that
    // has a building of that height.
    let facades: Vec<_> = FacadeClass::ALL
        .iter()
        .map(|&class| {
            let maps = texture::facade(class);
            (
                images.add(maps.base),
                images.add(maps.emissive),
                images.add(maps.surface),
                images.add(maps.normal),
            )
        })
        .collect();

    let mut building = Vec::with_capacity(districts.len() * PALETTE_SIZE as usize * CLASS_COUNT);
    for district in districts {
        for (slot, color) in palette(district).into_iter().enumerate() {
            // The grain is the district's, but how it is dressed — scale, and
            // whether it is turned — belongs to the palette slot, so a street
            // of one district is not a street of one photograph.
            let grain = super::facade::FacadeGrain::for_district(library, district, slot);
            for (base, emissive, surface, normal) in &facades {
                building.push(facades_out.add(super::facade::FacadeMaterial {
                    base: StandardMaterial {
                        base_color: color,
                        base_color_texture: Some(base.clone()),
                        emissive_texture: Some(emissive.clone()),
                        // Dark until dusk; `timeofday::light_windows` drives it.
                        emissive: LinearRgba::BLACK,
                        metallic_roughness_texture: Some(surface.clone()),
                        normal_map_texture: Some(normal.clone()),
                        perceptual_roughness: 1.0,
                        metallic: 1.0,
                        ..default()
                    },
                    extension: grain.clone(),
                }));
            }
        }
    }

    // Painted stand-ins, made whether or not they end up used: the scanned
    // library decides per surface, and a set can be present for the pavement
    // and missing for the grass.
    let paving_texture = images.add(texture::paving());
    let paving_relief = images.add(texture::paving_normal());
    let grass_texture = images.add(texture::grass());

    let mut paving = Vec::with_capacity(GROUND_BUCKETS.len());
    let mut grass = Vec::with_capacity(GROUND_BUCKETS.len());
    for tiling in GROUND_BUCKETS {
        let uv_transform = Affine2::from_scale(Vec2::splat(tiling));

        let mut slabs = StandardMaterial {
            uv_transform,
            ..default()
        };
        match library.get(super::material::set::PAVEMENT) {
            Some(scanned) => scanned.apply(&mut slabs),
            None => {
                slabs.base_color = Color::srgb(0.50, 0.50, 0.52);
                slabs.base_color_texture = Some(paving_texture.clone());
                slabs.normal_map_texture = Some(paving_relief.clone());
                slabs.perceptual_roughness = 0.95;
            }
        }
        let (dry_color, dry_roughness) = (slabs.base_color, slabs.perceptual_roughness);
        let handle = materials.add(slabs);
        // Pavements soak like the road does. Grass does not — wet grass is
        // darker but no glossier, and the shine is the whole point here.
        wet.add(handle.clone(), dry_color, dry_roughness);
        paving.push(handle);

        let mut lawn = StandardMaterial {
            uv_transform,
            ..default()
        };
        match library.get(super::material::set::GRASS) {
            Some(scanned) => scanned.apply(&mut lawn),
            None => {
                lawn.base_color = Color::srgb(0.29, 0.43, 0.24);
                lawn.base_color_texture = Some(grass_texture.clone());
                lawn.perceptual_roughness = 1.0;
            }
        }
        grass.push(materials.add(lawn));
    }

    let mut tar = StandardMaterial {
        // Roofs are only ever seen from a distance, so one tiling suits all.
        uv_transform: Affine2::from_scale(Vec2::splat(6.0)),
        ..default()
    };
    match library.get(super::material::set::ROOF) {
        Some(scanned) => scanned.apply(&mut tar),
        None => {
            tar.base_color = Color::srgb(0.38, 0.38, 0.40);
            tar.base_color_texture = Some(images.add(texture::roof()));
            tar.normal_map_texture = Some(images.add(texture::roof_normal()));
            tar.perceptual_roughness = 0.96;
        }
    }

    CityAssets {
        // Normal mapping needs a tangent basis, and mikktspace is the one the
        // shader agrees with; hand-written tangents are how normal maps end up
        // lit from the wrong side on two faces out of six.
        unit_cube: meshes.add(with_tangents(unit_cube_mesh())),
        unit_quad: meshes.add(with_tangents(
            Plane3d::default().mesh().size(1.0, 1.0).build(),
        )),
        building,
        roof: materials.add(tar),
        kerb: materials.add(StandardMaterial {
            base_color: Color::srgb(0.50, 0.50, 0.51),
            perceptual_roughness: 0.95,
            ..default()
        }),
        park_kerb: materials.add(StandardMaterial {
            base_color: Color::srgb(0.33, 0.31, 0.26),
            perceptual_roughness: 1.0,
            ..default()
        }),
        paving,
        grass,
    }
}

impl CityAssets {
    fn material_for(
        &self,
        district: District,
        palette: u8,
        class: FacadeClass,
    ) -> Handle<super::facade::FacadeMaterial> {
        let slot = district_index(district) * PALETTE_SIZE as usize + palette as usize;
        let i = slot * CLASS_COUNT + class.index();
        self.building[i.min(self.building.len() - 1)].clone()
    }

    /// Every facade material, for the day/night cycle to light up.
    pub fn building_materials(&self) -> &[Handle<super::facade::FacadeMaterial>] {
        &self.building
    }
}

/// Spawns one block's pavement and buildings, tagged for streaming.
/// Everything spawning a block needs beyond the block itself.
///
/// A struct rather than five more positional arguments: the roofs need the
/// world seed to be reproducible and the level-of-detail scale to know how far
/// to draw, and threading those through as bare parameters was already the
/// point at which the call became unreadable.
pub struct BlockContext<'a> {
    pub assets: &'a CityAssets,
    pub roofs: &'a RoofKit,
    pub shells: &'a ShellKit,
    pub seed: u64,
    pub lod_scale: f32,
}

pub fn spawn_block(commands: &mut Commands, ctx: &BlockContext, block: &Block, chunk: IVec2) {
    let assets = ctx.assets;
    let area = block.area;
    let size = area.size();
    let center = area.center();
    let park = block.district == District::Park;

    // The kerb slab gets the collider: at 28cm it is a step the player walks up
    // onto, and without one they would stand sunk into it. One static box per
    // block is cheap.
    commands.spawn((
        ChunkOf(chunk),
        Mesh3d(assets.unit_cube.clone()),
        MeshMaterial3d(if park {
            assets.park_kerb.clone()
        } else {
            assets.kerb.clone()
        }),
        Transform::from_xyz(center.x, SIDEWALK_HEIGHT * 0.5, center.y).with_scale(Vec3::new(
            size.x,
            SIDEWALK_HEIGHT,
            size.y,
        )),
        RigidBody::Static,
        Collider::cuboid(1.0, 1.0, 1.0),
    ));

    // The walking surface is a separate quad rather than the top of the slab,
    // so paving can tile at a metre or two while the kerb face beside it stays
    // plain concrete instead of a stack of squashed slabs.
    let bucket = ground_bucket((size.x + size.y) * 0.5);
    commands.spawn((
        ChunkOf(chunk),
        Mesh3d(assets.unit_quad.clone()),
        MeshMaterial3d(if park {
            assets.grass[bucket].clone()
        } else {
            assets.paving[bucket].clone()
        }),
        // A few millimetres proud of the slab, which is enough to settle the
        // depth test without being visible from standing height.
        Transform::from_xyz(center.x, SIDEWALK_HEIGHT + 0.004, center.y)
            .with_scale(Vec3::new(size.x, 1.0, size.y)),
    ));

    for building in &block.buildings {
        spawn_building(commands, ctx, block.district, building, chunk);
    }
}

fn spawn_building(
    commands: &mut Commands,
    ctx: &BlockContext,
    district: District,
    building: &Building,
    chunk: IVec2,
) {
    let assets = ctx.assets;
    let size = building.footprint.size();
    let center = building.footprint.center();
    let height = building.height;
    let class = FacadeClass::for_height(height);

    // One seed for everything about this building's roof, derived from where it
    // stands. Chunks regenerate on re-entry, so anything keyed on spawn order
    // would give the same building a different roof each time.
    let seed = rooftop::seed_for(ctx.seed, building.footprint);
    let parapet = rooftop::parapet(seed, class);

    // The wall, at three levels of detail. All three carry the same transform
    // and the same material, and `use_aabb: false` measures from the entity's
    // origin, so all three measure the same distance and hand over to one
    // another on precisely the same metre — which is what Bevy needs before it
    // will dither one into the next instead of blinking between them.
    let material = assets.material_for(district, building.palette, class);
    let wall = Transform::from_xyz(center.x, height * 0.5 + SIDEWALK_HEIGHT, center.y)
        .with_scale(Vec3::new(size.x, height, size.y));
    let (near, far) = shell::ranges(ctx.lod_scale);
    // Which balconies and which awnings, from the building's own seed rather
    // than from a counter, for the same reason its roof is.
    let variant = (seed >> 19) as u32;

    commands.spawn((
        ChunkOf(chunk),
        Mesh3d(ctx.shells.get(class, shell::Detail::Full, variant)),
        MeshMaterial3d(material.clone()),
        wall,
        VisibilityRange {
            start_margin: 0.0..0.0,
            end_margin: shell::handover(near),
            use_aabb: false,
        },
    ));
    commands.spawn((
        ChunkOf(chunk),
        Mesh3d(ctx.shells.get(class, shell::Detail::Coarse, variant)),
        MeshMaterial3d(material.clone()),
        wall,
        VisibilityRange {
            start_margin: shell::handover(near),
            end_margin: shell::handover(far),
            use_aabb: false,
        },
    ));
    // The plain box, and with it the collider — which is deliberately on the
    // level of detail that is never culled by *distance*, only by being close.
    // A visibility range hides a mesh and does not touch its collider, so the
    // building stays solid at every distance; putting it anywhere else would
    // work today and break the first time these ranges are reordered.
    commands.spawn((
        ChunkOf(chunk),
        Mesh3d(assets.unit_cube.clone()),
        MeshMaterial3d(material),
        wall,
        VisibilityRange {
            start_margin: shell::handover(far),
            end_margin: f32::INFINITY..f32::INFINITY,
            use_aabb: false,
        },
        RigidBody::Static,
        // Unit cube: Avian scales it by the transform above.
        Collider::cuboid(1.0, 1.0, 1.0),
    ));

    // A capping slab, slightly oversailing the walls. It hides the windowed top
    // face of the cube, and the overhang reads as a parapet from street level —
    // which is most of what stops a box looking like a box. Visual only: the
    // wall collider already reaches this high.
    //
    // Its proportions come from the building's own seed rather than from a
    // constant. That costs nothing — the slab was already an entity with its
    // own transform — and it is the only variation in the roofline that still
    // reads from a kilometre up, where the clutter below is sub-pixel.
    commands.spawn((
        ChunkOf(chunk),
        Mesh3d(assets.unit_cube.clone()),
        MeshMaterial3d(assets.roof.clone()),
        Transform::from_xyz(
            center.x,
            height + SIDEWALK_HEIGHT + parapet.thickness * 0.5,
            center.y,
        )
        .with_scale(Vec3::new(
            size.x + parapet.overhang * 2.0,
            parapet.thickness,
            size.y + parapet.overhang * 2.0,
        )),
    ));

    // The plinth course. Shares the kerb material on purpose — the base of a
    // building and the kerb in front of it are the two things at street level
    // that take the most abuse, and in most cities they are the same stone.
    let plinth_draw = (PLINTH_RANGE * ctx.lod_scale).max(1.0);
    commands.spawn((
        ChunkOf(chunk),
        Mesh3d(assets.unit_cube.clone()),
        MeshMaterial3d(assets.kerb.clone()),
        Transform::from_xyz(center.x, SIDEWALK_HEIGHT + PLINTH_HEIGHT * 0.5, center.y).with_scale(
            Vec3::new(
                size.x + PLINTH_PROUD * 2.0,
                PLINTH_HEIGHT,
                size.y + PLINTH_PROUD * 2.0,
            ),
        ),
        VisibilityRange {
            start_margin: 0.0..0.0,
            end_margin: (plinth_draw * 0.9)..plinth_draw,
            use_aabb: false,
        },
        // The wall behind it casts the same shadow from the same place. A
        // second caster eleven centimetres in front buys nothing and costs a
        // pass over every building in the city.
        NotShadowCaster,
    ));

    // And what accumulated on the deck. Sits on top of the slab, so nothing is
    // buried in it and nothing floats over it.
    rooftop::spawn(
        commands,
        ctx.roofs,
        ChunkOf(chunk),
        center,
        height + SIDEWALK_HEIGHT + parapet.thickness,
        &rooftop::plan(seed, building.footprint, class),
        ctx.lod_scale,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cube_faces_all_agree_about_up() {
        let mesh = unit_cube_mesh();
        let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION).unwrap();
        let uvs = mesh.attribute(Mesh::ATTRIBUTE_UV_0).unwrap();
        let (positions, uvs) = match (positions, uvs) {
            (
                bevy::render::mesh::VertexAttributeValues::Float32x3(p),
                bevy::render::mesh::VertexAttributeValues::Float32x2(u),
            ) => (p, u),
            _ => panic!("unexpected attribute formats"),
        };

        // The four side faces come first. On every one of them, V must climb
        // with Y, or facades land sideways or upside down.
        for vertex in 0..16 {
            let y = positions[vertex][1];
            let v = uvs[vertex][1];
            assert_eq!(
                y > 0.0,
                v > 0.5,
                "side vertex {vertex} has V running against Y"
            );
        }
    }

    #[test]
    fn ground_buckets_pick_the_nearest_tiling() {
        // A 20m block wants about 8 tiles across; a 60m block wants 23.
        assert_eq!(ground_bucket(20.0), 0);
        assert_eq!(ground_bucket(62.0), GROUND_BUCKETS.len() - 1);
        // Anything degenerate still has to land in range.
        for extent in [0.0, 0.5, 5.0, 400.0] {
            assert!(ground_bucket(extent) < GROUND_BUCKETS.len());
        }
    }

    #[test]
    fn every_district_palette_and_class_addresses_a_distinct_material() {
        let districts = [
            District::Downtown,
            District::Midtown,
            District::Residential,
            District::Industrial,
            District::Park,
        ];
        let mut seen = std::collections::HashSet::new();
        for district in districts {
            for palette in 0..PALETTE_SIZE {
                for class in FacadeClass::ALL {
                    let slot = district_index(district) * PALETTE_SIZE as usize + palette as usize;
                    assert!(
                        seen.insert(slot * CLASS_COUNT + class.index()),
                        "material index collision at {district:?}/{palette}/{class:?}"
                    );
                }
            }
        }
        assert_eq!(
            seen.len(),
            districts.len() * PALETTE_SIZE as usize * CLASS_COUNT
        );
    }
}
