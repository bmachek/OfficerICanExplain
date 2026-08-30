//! The world: procedural city, physics, streaming, and the day/night cycle.

pub mod buildings;
pub mod citygen;
pub mod roadgraph;
pub mod streaming;
pub mod streetlights;
pub mod timeofday;

use avian3d::prelude::*;
use bevy::prelude::*;

use crate::core::config::GameConfig;

/// The generated city. Held whole rather than streamed — see `streaming`.
#[derive(Resource, Deref)]
pub struct City(pub citygen::CityLayout);

pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            PhysicsPlugins::default(),
            timeofday::TimeOfDayPlugin,
            streetlights::StreetLightPlugin,
        ))
        .init_resource::<streaming::ActiveChunks>()
        .init_resource::<streaming::StreamTimer>()
        .add_systems(Startup, (generate_city, setup_ground))
        .add_systems(Update, streaming::update_streaming);
    }
}

fn generate_city(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let started = std::time::Instant::now();
    let layout = citygen::generate(config.world_seed, config.world.half_extent);

    info!(
        "city generated in {:.1}ms: {} blocks, {} buildings, {} intersections, {} roads",
        started.elapsed().as_secs_f32() * 1000.0,
        layout.blocks.len(),
        layout.building_count(),
        layout.graph.node_count(),
        layout.graph.edge_count(),
    );

    let city = City(layout);
    commands.insert_resource(streaming::ChunkIndex::build(&city));
    commands.insert_resource(city);
    commands.insert_resource(buildings::build_assets(&mut meshes, &mut materials));
}

/// The road surface. Streets are not meshed individually: the ground *is* the
/// asphalt, and the raised pavement slabs on each block carve the street grid
/// out of it as negative space. One quad instead of thousands of road polys.
fn setup_ground(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let size = config.world.half_extent * 2.0 + 200.0;
    commands.spawn((
        Name::new("Road surface"),
        Mesh3d(meshes.add(Plane3d::default().mesh().size(size, size))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.16, 0.16, 0.175),
            perceptual_roughness: 0.98,
            ..default()
        })),
    ));
    commands.spawn((
        Name::new("Ground collider"),
        RigidBody::Static,
        Collider::cuboid(size, 2.0, size),
        Transform::from_xyz(0.0, -1.0, 0.0),
    ));
}
