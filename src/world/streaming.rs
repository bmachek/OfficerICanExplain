//! Chunk streaming.
//!
//! Note what is and is not streamed: the *layout* (streets, blocks, road graph)
//! is generated once and kept whole, because it is only a few thousand
//! rectangles and because traffic, pursuit and the minimap all need to query
//! parts of the city the player cannot currently see. Only the *entities* —
//! meshes and colliders — come and go with camera distance.

use bevy::platform::collections::{HashMap, HashSet};
use bevy::prelude::*;

use super::City;
use super::buildings::{ChunkOf, CityAssets, spawn_block};
use super::markings::{MarkingAssets, spawn_edge};
use super::props::PropAssets;
use super::roadgraph::{EdgeId, NodeId};
use crate::core::config::GameConfig;

pub const CHUNK_SIZE: f32 = 250.0;

/// Chunk containing a world-space XZ position.
pub fn chunk_of(pos: Vec2) -> IVec2 {
    IVec2::new(
        (pos.x / CHUNK_SIZE).floor() as i32,
        (pos.y / CHUNK_SIZE).floor() as i32,
    )
}

pub fn chunk_center(chunk: IVec2) -> Vec2 {
    Vec2::new(
        (chunk.x as f32 + 0.5) * CHUNK_SIZE,
        (chunk.y as f32 + 0.5) * CHUNK_SIZE,
    )
}

/// Which blocks live in which chunk. Built once from the layout.
#[derive(Resource, Default)]
pub struct ChunkIndex {
    blocks: HashMap<IVec2, Vec<usize>>,
    /// Streets, filed by the chunk their midpoint falls in. A street can span
    /// two chunks; filing it by its middle means it is painted exactly once,
    /// and the half that hangs over the edge is a few metres of line beyond a
    /// boundary nobody can see.
    streets: HashMap<IVec2, Vec<EdgeId>>,
    /// Junctions, filed by the chunk they stand in. Separate from the streets
    /// because a junction belongs to one chunk unambiguously and a street does
    /// not, and because what goes up at a junction — signals — is placed once
    /// per junction rather than once per arm.
    junctions: HashMap<IVec2, Vec<NodeId>>,
}

impl ChunkIndex {
    pub fn build(city: &City) -> Self {
        let mut blocks: HashMap<IVec2, Vec<usize>> = HashMap::default();
        for (i, block) in city.blocks.iter().enumerate() {
            blocks
                .entry(chunk_of(block.area.center()))
                .or_default()
                .push(i);
        }

        let graph = &city.graph;
        let mut streets: HashMap<IVec2, Vec<EdgeId>> = HashMap::default();
        for (i, edge) in graph.edges().enumerate() {
            let middle = graph.node(edge.a).pos.midpoint(graph.node(edge.b).pos);
            streets
                .entry(chunk_of(middle))
                .or_default()
                .push(EdgeId(i as u32));
        }

        let mut junctions: HashMap<IVec2, Vec<NodeId>> = HashMap::default();
        for (id, node) in graph.nodes() {
            junctions.entry(chunk_of(node.pos)).or_default().push(id);
        }

        Self {
            blocks,
            streets,
            junctions,
        }
    }

    pub fn chunk_count(&self) -> usize {
        self.blocks.len()
    }

    pub fn blocks_in(&self, chunk: IVec2) -> Option<&[usize]> {
        self.blocks.get(&chunk).map(|v| v.as_slice())
    }

    pub fn streets_in(&self, chunk: IVec2) -> Option<&[EdgeId]> {
        self.streets.get(&chunk).map(|v| v.as_slice())
    }

    pub fn junctions_in(&self, chunk: IVec2) -> Option<&[NodeId]> {
        self.junctions.get(&chunk).map(|v| v.as_slice())
    }

    /// Which chunks should be resident for a camera at `focus`.
    ///
    /// Split out from the spawn/despawn plumbing so the selection rule — the
    /// part where an off-by-one silently strands entities or pops geometry in
    /// the player's face — can be tested directly.
    pub fn desired(&self, focus: Vec2, radius: f32) -> HashSet<IVec2> {
        // Measured against chunk centres, padded by half a chunk diagonal so a
        // chunk loads as soon as any corner of it comes into range.
        let cutoff = radius + CHUNK_SIZE * std::f32::consts::SQRT_2 * 0.5;
        self.blocks
            .keys()
            .chain(self.streets.keys())
            .copied()
            .filter(|&c| chunk_center(c).distance(focus) <= cutoff)
            .collect()
    }
}

#[derive(Resource, Default)]
pub struct ActiveChunks(HashSet<IVec2>);

impl ActiveChunks {
    pub fn len(&self) -> usize {
        self.0.len()
    }
}

/// Throttle: streaming does not need to run every frame, and doing so would
/// scan every spawned entity 120 times a second for no benefit.
#[derive(Resource)]
pub struct StreamTimer(Timer);

impl Default for StreamTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(0.25, TimerMode::Repeating))
    }
}

pub fn update_streaming(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<GameConfig>,
    city: Res<City>,
    index: Res<ChunkIndex>,
    assets: Res<CityAssets>,
    paint: Res<MarkingAssets>,
    props: Res<PropAssets>,
    foliage: Res<crate::world::vegetation::FoliageKit>,
    roofs: Res<crate::world::rooftop::RoofKit>,
    shells: Res<crate::world::shell::ShellKit>,
    mut active: ResMut<ActiveChunks>,
    mut timer: ResMut<StreamTimer>,
    cameras: Query<&GlobalTransform, With<crate::player::camera::CameraRig>>,
    spawned: Query<(Entity, &ChunkOf)>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }
    let Ok(camera) = cameras.single() else { return };

    let focus = camera.translation().xz();
    let desired = index.desired(focus, config.world.stream_radius);

    let arriving: Vec<IVec2> = desired.difference(&active.0).copied().collect();
    let ctx = crate::world::buildings::BlockContext {
        assets: &assets,
        roofs: &roofs,
        shells: &shells,
        seed: config.world_seed,
        lod_scale: config.graphics.lod_scale,
    };
    let foliage_range = config
        .graphics
        .lod_distance(crate::world::vegetation::RANGE);
    for chunk in arriving {
        // One stream per chunk and per subsystem, so a chunk's furniture is
        // identical every time it is walked back into rather than reshuffling,
        // and so planting a tree cannot move a bin.
        let mut planting = crate::core::rng::stream_for_chunk(
            config.world_seed,
            crate::core::rng::stream::VEGETATION,
            (chunk.x, chunk.y),
        );

        if let Some(block_indices) = index.blocks_in(chunk) {
            for &i in block_indices {
                let block = &city.blocks[i];
                spawn_block(&mut commands, &ctx, block, chunk);
                super::vegetation::spawn_park(
                    &mut commands,
                    &foliage,
                    &mut planting,
                    block,
                    chunk,
                    foliage_range,
                );
            }
        }
        if let Some(streets) = index.streets_in(chunk) {
            let mut rng = crate::core::rng::stream_for_chunk(
                config.world_seed,
                crate::core::rng::stream::PROPS,
                (chunk.x, chunk.y),
            );
            for &id in streets {
                let edge = city.graph.edge(id);
                let (from, to) = (city.graph.node(edge.a).pos, city.graph.node(edge.b).pos);
                spawn_edge(&mut commands, &paint, edge, from, to, chunk);
                super::props::spawn_edge(&mut commands, &props, &mut rng, edge, from, to, chunk);
                super::vegetation::spawn_edge(
                    &mut commands,
                    &foliage,
                    &mut planting,
                    edge,
                    from,
                    to,
                    chunk,
                    foliage_range,
                );
            }
        }
        if let Some(junctions) = index.junctions_in(chunk) {
            for &id in junctions {
                let node = city.graph.node(id);
                let arms: Vec<(Vec2, f32)> = node
                    .edges
                    .iter()
                    .map(|&edge| {
                        let edge = city.graph.edge(edge);
                        let other = if edge.a == id { edge.b } else { edge.a };
                        (city.graph.node(other).pos, edge.width)
                    })
                    .collect();
                let arterial = node
                    .edges
                    .iter()
                    .any(|&edge| city.graph.edge(edge).arterial);
                super::props::spawn_junction(
                    &mut commands,
                    &props,
                    node.pos,
                    &arms,
                    arterial,
                    chunk,
                );
            }
        }
    }

    let leaving: HashSet<IVec2> = active.0.difference(&desired).copied().collect();
    if !leaving.is_empty() {
        for (entity, chunk) in &spawned {
            if leaving.contains(&chunk.0) {
                commands.entity(entity).despawn();
            }
        }
    }

    active.0 = desired;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::{City, citygen};

    fn index() -> (City, ChunkIndex) {
        let city = City(citygen::generate(0xA17E_5EED, 1000.0));
        let index = ChunkIndex::build(&city);
        (city, index)
    }

    #[test]
    fn chunk_math_is_consistent() {
        for pos in [
            Vec2::new(0.0, 0.0),
            Vec2::new(-513.0, 742.0),
            Vec2::new(249.9, -0.1),
        ] {
            let c = chunk_of(pos);
            let center = chunk_center(c);
            assert!(
                (center - pos).abs().max_element() <= CHUNK_SIZE,
                "{pos:?} mapped to chunk {c:?} centred at {center:?}"
            );
            assert_eq!(chunk_of(center), c, "chunk centre must map back to itself");
        }
    }

    #[test]
    fn every_block_lands_in_a_chunk() {
        let (city, index) = index();
        let total: usize = index.blocks.values().map(|v| v.len()).sum();
        assert_eq!(total, city.blocks.len(), "blocks lost during indexing");
        assert!(index.chunk_count() > 10);
    }

    #[test]
    fn distant_chunks_are_not_resident() {
        let (_city, index) = index();
        let near = index.desired(Vec2::ZERO, 400.0);
        assert!(!near.is_empty(), "nothing loaded at the city centre");

        // A chunk on the far edge must not be resident from the centre.
        let far_corner = chunk_of(Vec2::new(950.0, 950.0));
        assert!(
            !near.contains(&far_corner),
            "far corner should not be resident with a 400m radius"
        );
    }

    #[test]
    fn moving_away_evicts_the_old_neighbourhood() {
        let (_city, index) = index();
        let radius = 300.0;
        let here = index.desired(Vec2::new(-800.0, -800.0), radius);
        let there = index.desired(Vec2::new(800.0, 800.0), radius);

        assert!(!here.is_empty() && !there.is_empty());
        assert!(
            here.is_disjoint(&there),
            "opposite corners of a 2km city must share no resident chunks"
        );
    }

    #[test]
    fn residency_grows_monotonically_with_radius() {
        let (_city, index) = index();
        let focus = Vec2::new(120.0, -60.0);
        let small = index.desired(focus, 300.0);
        let large = index.desired(focus, 900.0);
        assert!(
            small.is_subset(&large),
            "a larger radius must be a superset of a smaller one"
        );
        assert!(large.len() > small.len());
    }
}
