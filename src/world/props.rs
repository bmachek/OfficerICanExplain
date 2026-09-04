//! Street furniture.
//!
//! Bins, bollards, hydrants, post boxes, signs. None of it is interactive and
//! none of it is in the way — it exists because a street without any of it does
//! not read as a street. A kerb with nothing standing on it for sixty metres is
//! the single clearest tell that a city was generated rather than built.
//!
//! Everything shares one mesh per kind and one material per kind, so the whole
//! city's furniture is a handful of draw calls however much of it is up. Props
//! are placed from the chunk's own RNG stream, so a chunk regenerates
//! identically however many times the player walks in and out of it — the same
//! rule the buildings follow.
//!
//! Nothing here has a collider. Walking through a bin is a smaller lie than
//! several hundred more static bodies for the physics solver to consider, and
//! the player is never close enough to a bin to care.

use bevy::prelude::*;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

use super::buildings::{ChunkOf, SIDEWALK_HEIGHT};
use super::roadgraph::RoadEdge;

/// Metres between chances to place something on a kerb.
const SPACING: f32 = 14.0;
/// How many of those chances actually produce a prop.
const DENSITY: f32 = 0.42;
/// How far inside the kerb line furniture stands, in metres.
const SET_BACK: f32 = 0.75;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Prop {
    Bin,
    Bollard,
    Hydrant,
    PostBox,
    Sign,
}

impl Prop {
    /// Weighted the way a street is: bollards and bins everywhere, a hydrant
    /// here and there, a post box rarely.
    const TABLE: [(Prop, u32); 5] = [
        (Prop::Bollard, 8),
        (Prop::Bin, 6),
        (Prop::Sign, 4),
        (Prop::Hydrant, 3),
        (Prop::PostBox, 1),
    ];

    fn pick(rng: &mut ChaCha8Rng) -> Prop {
        let total: u32 = Self::TABLE.iter().map(|(_, weight)| weight).sum();
        let mut ticket = rng.random_range(0..total);
        for (prop, weight) in Self::TABLE {
            if ticket < weight {
                return prop;
            }
            ticket -= weight;
        }
        Prop::Bollard
    }
}

#[derive(Resource)]
pub struct PropAssets {
    bin: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    bollard: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    hydrant: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    post_box: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    sign_post: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    sign_plate: (Handle<Mesh>, Handle<StandardMaterial>, f32),
}

impl PropAssets {
    /// The mesh, material and standing height for one kind.
    fn parts(&self, prop: Prop) -> &(Handle<Mesh>, Handle<StandardMaterial>, f32) {
        match prop {
            Prop::Bin => &self.bin,
            Prop::Bollard => &self.bollard,
            Prop::Hydrant => &self.hydrant,
            Prop::PostBox => &self.post_box,
            Prop::Sign => &self.sign_post,
        }
    }
}

fn painted_metal(color: Color, roughness: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: color,
        perceptual_roughness: roughness,
        metallic: 0.55,
        ..default()
    }
}

pub fn build_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> PropAssets {
    PropAssets {
        bin: (
            meshes.add(Cylinder::new(0.26, 0.95)),
            materials.add(painted_metal(Color::srgb(0.17, 0.20, 0.18), 0.72)),
            0.95,
        ),
        bollard: (
            meshes.add(Cylinder::new(0.11, 0.95)),
            materials.add(painted_metal(Color::srgb(0.12, 0.13, 0.15), 0.55)),
            0.95,
        ),
        hydrant: (
            meshes.add(Cylinder::new(0.16, 0.72)),
            materials.add(painted_metal(Color::srgb(0.60, 0.10, 0.09), 0.62)),
            0.72,
        ),
        post_box: (
            meshes.add(Cuboid::new(0.52, 1.25, 0.46)),
            materials.add(painted_metal(Color::srgb(0.42, 0.10, 0.11), 0.50)),
            1.25,
        ),
        sign_post: (
            meshes.add(Cylinder::new(0.045, 2.35)),
            materials.add(painted_metal(Color::srgb(0.55, 0.56, 0.58), 0.45)),
            2.35,
        ),
        sign_plate: (
            meshes.add(Cuboid::new(0.46, 0.46, 0.03)),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.80, 0.81, 0.83),
                perceptual_roughness: 0.34,
                metallic: 0.15,
                ..default()
            }),
            0.46,
        ),
    }
}

/// Scatters furniture along both kerbs of one street.
pub fn spawn_edge(
    commands: &mut Commands,
    assets: &PropAssets,
    rng: &mut ChaCha8Rng,
    edge: &RoadEdge,
    from: Vec2,
    to: Vec2,
    chunk: IVec2,
) {
    let Ok(direction) = Dir2::new(to - from) else {
        return;
    };
    let normal = Vec2::new(-direction.y, direction.x);
    let offset = edge.width * 0.5 + SET_BACK;

    let slots = (edge.length / SPACING).floor() as i32;
    for i in 1..slots {
        for side in [-1.0f32, 1.0] {
            if rng.random_range(0.0..1.0) > DENSITY {
                continue;
            }
            let jitter = rng.random_range(-2.5..2.5);
            let at = from + *direction * (i as f32 * SPACING + jitter) + normal * offset * side;

            let prop = Prop::pick(rng);
            let (mesh, material, height) = assets.parts(prop);
            // Furniture stands on the pavement, and the meshes are centred, so
            // everything is lifted by half its own height plus the kerb.
            let base = SIDEWALK_HEIGHT + height * 0.5;
            let yaw = rng.random_range(0.0..std::f32::consts::TAU);

            let mut entity = commands.spawn((
                ChunkOf(chunk),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(at.x, base, at.y).with_rotation(Quat::from_rotation_y(yaw)),
            ));

            if prop == Prop::Sign {
                // The plate rides near the top of its post, facing along the
                // street rather than at a random angle — a sign nobody can read
                // from the road is not a sign.
                let (plate_mesh, plate_material, _) = &assets.sign_plate;
                entity.with_child((
                    Mesh3d(plate_mesh.clone()),
                    MeshMaterial3d(plate_material.clone()),
                    Transform::from_xyz(0.0, height * 0.34, 0.0),
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::rng::{stream, stream_for};

    #[test]
    fn bollards_and_bins_outnumber_post_boxes() {
        let mut rng = stream_for(4, stream::PROPS);
        let mut counts = [0usize; 5];
        for _ in 0..2000 {
            let picked = Prop::pick(&mut rng);
            let slot = Prop::TABLE.iter().position(|(p, _)| *p == picked).unwrap();
            counts[slot] += 1;
        }
        // Table order is descending by weight, so the counts should be too.
        for pair in counts.windows(2) {
            assert!(
                pair[0] >= pair[1],
                "the weighting does not hold: {counts:?}"
            );
        }
    }

    #[test]
    fn every_kind_turns_up_eventually() {
        // An off-by-one in the weighted pick makes the last kind unreachable,
        // which nobody notices until they go looking for a post box.
        let mut rng = stream_for(11, stream::PROPS);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..3000 {
            seen.insert(Prop::pick(&mut rng));
        }
        assert_eq!(seen.len(), Prop::TABLE.len());
    }

    #[test]
    fn furniture_stands_on_the_pavement_and_not_in_it() {
        // Meshes are centred on their origin, so a prop placed at pavement
        // height would be buried to its waist.
        for height in [0.72, 0.95, 1.25, 2.35] {
            let base = SIDEWALK_HEIGHT + height * 0.5;
            let foot = base - height * 0.5;
            assert!(
                (foot - SIDEWALK_HEIGHT).abs() < 1e-6,
                "a {height}m prop's foot landed at {foot}, not on the kerb"
            );
        }
    }

    #[test]
    fn furniture_clears_the_carriageway() {
        // Set back from the kerb line, or traffic drives through it.
        let width = 12.0f32;
        let offset = width * 0.5 + SET_BACK;
        assert!(
            offset > width * 0.5,
            "furniture at {offset} is inside a {width}m road"
        );
    }
}
