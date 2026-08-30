//! Procedural city layout.
//!
//! A perturbed axis-aligned grid rather than L-systems or tensor fields. Curved
//! organic road networks look better in screenshots but produce a messy lane
//! graph, and the lane graph is what traffic, pursuit and the minimap all
//! depend on. An irregular grid keeps every block a rectangle — which makes
//! footprints, sidewalks and colliders trivial — while jittered spacing and
//! per-district massing keep it from reading as graph paper.
//!
//! Generation is pure and deterministic: same seed, same city, every run.

use bevy::math::Vec2;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

use super::roadgraph::RoadGraph;
use crate::core::rng::{stream, stream_for};

const ARTERIAL_WIDTH: f32 = 17.0;
const MINOR_WIDTH: f32 = 9.5;
/// Pavement between the kerb and the buildable area.
pub const SIDEWALK_WIDTH: f32 = 3.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum District {
    Downtown,
    Midtown,
    Residential,
    Industrial,
    Park,
}

impl District {
    /// (min height, max height) in metres.
    fn height_range(self) -> (f32, f32) {
        match self {
            District::Downtown => (38.0, 135.0),
            District::Midtown => (16.0, 46.0),
            District::Residential => (6.5, 15.0),
            District::Industrial => (5.5, 13.0),
            District::Park => (0.0, 0.0),
        }
    }

    /// Smallest lot side before subdivision stops. Bigger = fewer, bulkier
    /// buildings, which is what reads as downtown.
    fn min_lot(self) -> f32 {
        match self {
            District::Downtown => 25.0,
            District::Midtown => 18.0,
            District::Residential => 11.0,
            District::Industrial => 23.0,
            District::Park => f32::MAX,
        }
    }

    /// Chance a lot is left empty (car park, yard, vacant plot).
    fn vacancy(self) -> f32 {
        match self {
            District::Downtown => 0.05,
            District::Midtown => 0.08,
            District::Residential => 0.10,
            District::Industrial => 0.18,
            District::Park => 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    pub fn new(min: Vec2, max: Vec2) -> Self {
        Self { min, max }
    }
    pub fn size(&self) -> Vec2 {
        self.max - self.min
    }
    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }
    pub fn inset(&self, d: f32) -> Self {
        Self {
            min: self.min + Vec2::splat(d),
            max: self.max - Vec2::splat(d),
        }
    }
    pub fn is_valid(&self) -> bool {
        self.max.x > self.min.x && self.max.y > self.min.y
    }
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.min.x < other.max.x
            && other.min.x < self.max.x
            && self.min.y < other.max.y
            && other.min.y < self.max.y
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Street {
    /// Centreline position on the perpendicular axis.
    pub center: f32,
    pub width: f32,
    pub arterial: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Building {
    pub footprint: Rect,
    pub height: f32,
    /// Index into the district's material palette.
    pub palette: u8,
}

#[derive(Debug, Clone)]
pub struct Block {
    /// Kerb-to-kerb extent, sidewalk included.
    pub area: Rect,
    pub district: District,
    pub buildings: Vec<Building>,
}

#[derive(Debug, Clone)]
pub struct CityLayout {
    pub seed: u64,
    pub half_extent: f32,
    /// Streets running along Z, indexed by their X centreline.
    pub x_streets: Vec<Street>,
    /// Streets running along X, indexed by their Z centreline.
    pub z_streets: Vec<Street>,
    pub blocks: Vec<Block>,
    pub graph: RoadGraph,
}

impl CityLayout {
    pub fn building_count(&self) -> usize {
        self.blocks.iter().map(|b| b.buildings.len()).sum()
    }

    /// Order-independent digest, for asserting a seed reproduces a city.
    pub fn digest(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |v: f32| {
            // Quantise so bit-level float noise cannot flip the digest.
            let q = (v * 64.0).round() as i64 as u64;
            h ^= q;
            h = h.wrapping_mul(0x0100_0000_01b3);
        };
        for s in self.x_streets.iter().chain(self.z_streets.iter()) {
            mix(s.center);
            mix(s.width);
        }
        for b in &self.blocks {
            mix(b.area.min.x);
            mix(b.area.min.y);
            for building in &b.buildings {
                mix(building.footprint.min.x);
                mix(building.footprint.min.y);
                mix(building.height);
            }
        }
        h
    }
}

pub fn generate(seed: u64, half_extent: f32) -> CityLayout {
    let mut road_rng = stream_for(seed, stream::ROADS);
    let x_streets = streets(&mut road_rng, half_extent);
    let z_streets = streets(&mut road_rng, half_extent);

    let graph = build_graph(&x_streets, &z_streets);
    let blocks = build_blocks(seed, &x_streets, &z_streets);

    CityLayout {
        seed,
        half_extent,
        x_streets,
        z_streets,
        blocks,
        graph,
    }
}

/// Walks one axis laying down streets, alternating arterials with a run of
/// minor streets and jittering the gap between them.
fn streets(rng: &mut ChaCha8Rng, half_extent: f32) -> Vec<Street> {
    let mut out = Vec::new();
    let mut edge = -half_extent;
    let mut until_arterial = 0u32;

    loop {
        let arterial = until_arterial == 0;
        let width = if arterial {
            ARTERIAL_WIDTH
        } else {
            MINOR_WIDTH
        };
        if edge + width > half_extent {
            break;
        }

        out.push(Street {
            center: edge + width * 0.5,
            width,
            arterial,
        });

        until_arterial = if arterial {
            rng.random_range(3..=4)
        } else {
            until_arterial - 1
        };

        // Arterials front deeper blocks, which is what puts the tall stuff on
        // the main roads instead of scattering it.
        let depth: f32 = if arterial {
            rng.random_range(62.0..92.0)
        } else {
            rng.random_range(48.0..78.0)
        };
        edge += width + depth;
    }

    out
}

fn build_graph(x_streets: &[Street], z_streets: &[Street]) -> RoadGraph {
    let mut graph = RoadGraph::default();

    // A node at every crossing.
    for (xi, xs) in x_streets.iter().enumerate() {
        for (zi, zs) in z_streets.iter().enumerate() {
            graph.add_node(Vec2::new(xs.center, zs.center), (xi as u16, zi as u16));
        }
    }

    // Link along each street to its immediate neighbour.
    for (xi, xs) in x_streets.iter().enumerate() {
        for zi in 0..z_streets.len().saturating_sub(1) {
            let a = graph.node_at_grid((xi as u16, zi as u16));
            let b = graph.node_at_grid((xi as u16, zi as u16 + 1));
            if let (Some(a), Some(b)) = (a, b) {
                graph.connect(a, b, xs.width, xs.arterial);
            }
        }
    }
    for (zi, zs) in z_streets.iter().enumerate() {
        for xi in 0..x_streets.len().saturating_sub(1) {
            let a = graph.node_at_grid((xi as u16, zi as u16));
            let b = graph.node_at_grid((xi as u16 + 1, zi as u16));
            if let (Some(a), Some(b)) = (a, b) {
                graph.connect(a, b, zs.width, zs.arterial);
            }
        }
    }

    graph
}

fn build_blocks(seed: u64, x_streets: &[Street], z_streets: &[Street]) -> Vec<Block> {
    let mut rng = stream_for(seed, stream::BLOCKS);
    let mut building_rng = stream_for(seed, stream::BUILDINGS);
    let mut blocks = Vec::new();

    for xi in 0..x_streets.len().saturating_sub(1) {
        for zi in 0..z_streets.len().saturating_sub(1) {
            let (left, right) = (x_streets[xi], x_streets[xi + 1]);
            let (near, far) = (z_streets[zi], z_streets[zi + 1]);

            let area = Rect::new(
                Vec2::new(
                    left.center + left.width * 0.5,
                    near.center + near.width * 0.5,
                ),
                Vec2::new(
                    right.center - right.width * 0.5,
                    far.center - far.width * 0.5,
                ),
            );
            if !area.is_valid() {
                continue;
            }

            let district = district_for(area.center(), &mut rng);
            let buildings = lay_out_buildings(area, district, &mut building_rng);
            blocks.push(Block {
                area,
                district,
                buildings,
            });
        }
    }

    blocks
}

fn district_for(center: Vec2, rng: &mut ChaCha8Rng) -> District {
    // A few parks anywhere keep the skyline from being uniform.
    if rng.random_range(0.0..1.0) < 0.04 {
        return District::Park;
    }
    let r = center.length();
    match r {
        _ if r < 300.0 => District::Downtown,
        _ if r < 600.0 => District::Midtown,
        _ if r < 850.0 => District::Residential,
        _ => District::Industrial,
    }
}

fn lay_out_buildings(area: Rect, district: District, rng: &mut ChaCha8Rng) -> Vec<Building> {
    if district == District::Park {
        return Vec::new();
    }

    let buildable = area.inset(SIDEWALK_WIDTH);
    if !buildable.is_valid() {
        return Vec::new();
    }

    let mut lots = Vec::new();
    subdivide(buildable, district.min_lot(), rng, 0, &mut lots);

    let (min_h, max_h) = district.height_range();
    let vacancy = district.vacancy();

    lots.into_iter()
        .filter_map(|lot| {
            if rng.random_range(0.0..1.0) < vacancy {
                return None;
            }
            // Setback keeps neighbours from sharing a face, so the massing
            // still reads as separate buildings from street level.
            let footprint = lot.inset(rng.random_range(0.6..2.2));
            if !footprint.is_valid() {
                return None;
            }
            Some(Building {
                footprint,
                height: rng.random_range(min_h..max_h),
                palette: rng.random_range(0..PALETTE_SIZE),
            })
        })
        .collect()
}

/// Number of material variants per district.
pub const PALETTE_SIZE: u8 = 4;

/// Recursively halves a block into lots, always splitting the longer side so
/// lots stay roughly square rather than degenerating into slivers.
fn subdivide(rect: Rect, min_lot: f32, rng: &mut ChaCha8Rng, depth: u32, out: &mut Vec<Rect>) {
    const MAX_DEPTH: u32 = 6;
    let size = rect.size();
    let can_split_x = size.x > min_lot * 2.0;
    let can_split_z = size.y > min_lot * 2.0;

    if depth >= MAX_DEPTH || (!can_split_x && !can_split_z) {
        out.push(rect);
        return;
    }

    let split_x = if can_split_x && can_split_z {
        size.x > size.y
    } else {
        can_split_x
    };
    let t: f32 = rng.random_range(0.4..0.6);

    let (a, b) = if split_x {
        let x = rect.min.x + size.x * t;
        (
            Rect::new(rect.min, Vec2::new(x, rect.max.y)),
            Rect::new(Vec2::new(x, rect.min.y), rect.max),
        )
    } else {
        let z = rect.min.y + size.y * t;
        (
            Rect::new(rect.min, Vec2::new(rect.max.x, z)),
            Rect::new(Vec2::new(rect.min.x, z), rect.max),
        )
    };

    subdivide(a, min_lot, rng, depth + 1, out);
    subdivide(b, min_lot, rng, depth + 1, out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> CityLayout {
        generate(0xA17E_5EED, 1000.0)
    }

    #[test]
    fn same_seed_rebuilds_the_same_city() {
        assert_eq!(generate(7, 800.0).digest(), generate(7, 800.0).digest());
    }

    #[test]
    fn different_seeds_differ() {
        assert_ne!(generate(7, 800.0).digest(), generate(8, 800.0).digest());
    }

    #[test]
    fn produces_a_substantial_city() {
        let city = layout();
        assert!(city.blocks.len() > 100, "blocks: {}", city.blocks.len());
        assert!(
            city.building_count() > 500,
            "buildings: {}",
            city.building_count()
        );
    }

    #[test]
    fn buildings_stay_inside_their_block() {
        for block in &layout().blocks {
            let buildable = block.area.inset(SIDEWALK_WIDTH);
            for b in &block.buildings {
                assert!(
                    b.footprint.min.x >= buildable.min.x - 1e-3
                        && b.footprint.min.y >= buildable.min.y - 1e-3
                        && b.footprint.max.x <= buildable.max.x + 1e-3
                        && b.footprint.max.y <= buildable.max.y + 1e-3,
                    "building escaped its block: {:?} vs {:?}",
                    b.footprint,
                    buildable
                );
            }
        }
    }

    #[test]
    fn buildings_never_overlap_each_other() {
        for block in &layout().blocks {
            for (i, a) in block.buildings.iter().enumerate() {
                for b in &block.buildings[i + 1..] {
                    assert!(
                        !a.footprint.overlaps(&b.footprint),
                        "overlapping footprints {:?} / {:?}",
                        a.footprint,
                        b.footprint
                    );
                }
            }
        }
    }

    #[test]
    fn blocks_never_overlap_the_roads() {
        let city = layout();
        for block in &city.blocks {
            for street in &city.x_streets {
                let (lo, hi) = (
                    street.center - street.width * 0.5,
                    street.center + street.width * 0.5,
                );
                assert!(
                    block.area.max.x <= lo + 1e-3 || block.area.min.x >= hi - 1e-3,
                    "block {:?} overlaps street at x={}",
                    block.area,
                    street.center
                );
            }
        }
    }

    #[test]
    fn every_intersection_is_reachable() {
        let city = layout();
        let graph = &city.graph;
        assert!(graph.node_count() > 100);

        // A grid should be fully connected; a path from the first node to the
        // last is a cheap proxy that also exercises A*.
        let first = super::super::roadgraph::NodeId(0);
        let last = super::super::roadgraph::NodeId(graph.node_count() as u32 - 1);
        let path = graph.path(first, last).expect("no route across the city");
        assert!(path.len() > 2);
        assert_eq!(path[0], first);
        assert_eq!(*path.last().unwrap(), last);

        // Consecutive nodes in the path must actually share an edge.
        for pair in path.windows(2) {
            assert!(
                graph.neighbors(pair[0]).any(|(n, _)| n == pair[1]),
                "path jumps between unconnected nodes"
            );
        }
    }
}
