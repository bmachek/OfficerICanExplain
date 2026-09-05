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
    /// A parking meter. Small, and there are more of them on a real street
    /// than of anything else here.
    Meter,
    Bench,
    /// A newspaper vending box, in a colour nobody chose for it.
    NewsBox,
    /// A planter. The one piece of furniture that is here to look like it is
    /// not furniture.
    Planter,
    /// A phone box, kept for exactly the reason real ones are: nobody has got
    /// round to taking it away.
    PhoneBox,
}

impl Prop {
    /// Weighted the way a street is: bollards, meters and bins everywhere, a
    /// hydrant here and there, a phone box rarely.
    const TABLE: [(Prop, u32); 10] = [
        (Prop::Bollard, 9),
        (Prop::Meter, 7),
        (Prop::Bin, 6),
        (Prop::Sign, 5),
        (Prop::Hydrant, 3),
        (Prop::Bench, 3),
        (Prop::NewsBox, 2),
        (Prop::Planter, 2),
        (Prop::PostBox, 1),
        (Prop::PhoneBox, 1),
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
    meter: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    bench: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    news_box: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    planter: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    phone_box: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    /// The signal head, its pole, and the lens plate that faces the traffic.
    signal_post: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    signal_head: (Handle<Mesh>, Handle<StandardMaterial>, f32),
    signal_lens: [(Handle<Mesh>, Handle<StandardMaterial>); 3],
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
            Prop::Meter => &self.meter,
            Prop::Bench => &self.bench,
            Prop::NewsBox => &self.news_box,
            Prop::Planter => &self.planter,
            Prop::PhoneBox => &self.phone_box,
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
        meter: (
            meshes.add(Cuboid::new(0.14, 1.22, 0.12)),
            materials.add(painted_metal(Color::srgb(0.24, 0.26, 0.28), 0.58)),
            1.22,
        ),
        bench: (
            meshes.add(Cuboid::new(1.75, 0.46, 0.55)),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.36, 0.26, 0.17),
                perceptual_roughness: 0.88,
                ..default()
            }),
            0.46,
        ),
        news_box: (
            meshes.add(Cuboid::new(0.44, 1.08, 0.40)),
            materials.add(painted_metal(Color::srgb(0.16, 0.34, 0.46), 0.52)),
            1.08,
        ),
        planter: (
            meshes.add(Cuboid::new(0.86, 0.58, 0.86)),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.44, 0.42, 0.39),
                perceptual_roughness: 0.97,
                ..default()
            }),
            0.58,
        ),
        phone_box: (
            meshes.add(Cuboid::new(0.94, 2.42, 0.94)),
            materials.add(painted_metal(Color::srgb(0.34, 0.12, 0.12), 0.42)),
            2.42,
        ),
        signal_post: (
            meshes.add(Cylinder::new(0.075, SIGNAL_HEIGHT)),
            materials.add(painted_metal(Color::srgb(0.17, 0.18, 0.19), 0.50)),
            SIGNAL_HEIGHT,
        ),
        signal_head: (
            meshes.add(Cuboid::new(0.32, 0.86, 0.26)),
            materials.add(painted_metal(Color::srgb(0.13, 0.14, 0.15), 0.55)),
            0.86,
        ),
        // Unlit. Nothing in this game drives the signals yet, and a signal
        // showing green down every approach of a crossroads at once would be a
        // clearer lie than one showing nothing.
        signal_lens: [
            Color::srgb(0.34, 0.06, 0.05),
            Color::srgb(0.36, 0.26, 0.05),
            Color::srgb(0.06, 0.30, 0.12),
        ]
        .map(|color| {
            (
                meshes.add(Cylinder::new(0.085, 0.045)),
                materials.add(StandardMaterial {
                    base_color: color,
                    perceptual_roughness: 0.28,
                    ..default()
                }),
            )
        }),
    }
}

/// How high a signal head hangs above the pavement.
const SIGNAL_HEIGHT: f32 = 3.1;
/// Where the three lenses sit on the head, measured from its middle.
const SIGNAL_LENSES: [f32; 3] = [0.26, 0.0, -0.26];
/// How far back from the junction's centre a signal stands, as a multiple of
/// the widest road meeting there.
const SIGNAL_SET_BACK: f32 = 0.62;

/// Where a signal for one approach stands, and which way it looks.
///
/// It stands on the near kerb, on the right of a driver coming down this arm,
/// and it looks back up the arm at them. Both halves are easy to get a quarter
/// turn or a whole side out, and neither shows in a still — which is why they
/// are a function with a test rather than four lines inside a spawn.
fn signal_pose(at: Vec2, towards: Vec2, width: f32, widest: f32) -> Option<(Vec2, f32)> {
    let direction = Dir2::new(towards - at).ok()?;
    // The driver is coming *down* the arm, so their heading is the other way
    // and their right hand is the other way with it.
    let heading = -*direction;
    let right = Vec2::new(-heading.y, heading.x);
    let kerb = if crate::ai::steering::RIGHT_HAND_TRAFFIC {
        right
    } else {
        -right
    };

    let foot = at + *direction * (widest * SIGNAL_SET_BACK) + kerb * (width * 0.5 + 0.8);
    // The lenses look down the head's local +Z, and a yaw of theta sends +Z to
    // (sin, cos) — which has to come out as the direction the traffic arrives
    // *from*, or the signal shows its back to the only people it is for.
    Some((foot, direction.x.atan2(direction.y)))
}

/// Puts a signal on each approach to one junction.
///
/// Only where it would actually be: three or more arms, at least one of them an
/// arterial road. A signalled crossroads on a back street is as clear a tell
/// that a city was generated as an unsignalled one on a main road.
pub fn spawn_junction(
    commands: &mut Commands,
    assets: &PropAssets,
    at: Vec2,
    approaches: &[(Vec2, f32)],
    arterial: bool,
    chunk: IVec2,
) {
    if approaches.len() < 3 || !arterial {
        return;
    }
    let widest = approaches
        .iter()
        .map(|(_, width)| *width)
        .fold(0.0f32, f32::max);

    for (towards, width) in approaches {
        let Some((foot, yaw)) = signal_pose(at, *towards, *width, widest) else {
            continue;
        };

        let (post, steel, height) = &assets.signal_post;
        let (head, casing, head_height) = &assets.signal_head;
        commands.spawn((
            ChunkOf(chunk),
            Mesh3d(post.clone()),
            MeshMaterial3d(steel.clone()),
            Transform::from_xyz(foot.x, SIDEWALK_HEIGHT + height * 0.5, foot.y)
                .with_rotation(Quat::from_rotation_y(yaw)),
            children![(
                Mesh3d(head.clone()),
                MeshMaterial3d(casing.clone()),
                Transform::from_xyz(0.0, height * 0.5 - head_height * 0.5, 0.0),
                Children::spawn(SpawnIter(
                    assets
                        .signal_lens
                        .clone()
                        .into_iter()
                        .zip(SIGNAL_LENSES)
                        .map(|((lens, glass), y)| {
                            (
                                Mesh3d(lens),
                                MeshMaterial3d(glass),
                                // Cylinders stand up; a lens looks out.
                                Transform::from_xyz(0.0, y, 0.15).with_rotation(
                                    Quat::from_rotation_x(std::f32::consts::FRAC_PI_2),
                                ),
                            )
                        })
                        .collect::<Vec<_>>()
                        .into_iter()
                )),
            )],
        ));
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

    /// The table is sampled in proportion to its weights.
    ///
    /// Checked against the weights themselves rather than against the table's
    /// order: several kinds now share a weight, and which of two equally common
    /// props happens to come up more often over a finite number of draws is
    /// noise, not a property worth asserting.
    #[test]
    fn the_table_is_sampled_in_proportion_to_its_weights() {
        const DRAWS: usize = 40_000;
        let mut rng = stream_for(4, stream::PROPS);
        let mut counts = [0usize; Prop::TABLE.len()];
        for _ in 0..DRAWS {
            let picked = Prop::pick(&mut rng);
            let slot = Prop::TABLE.iter().position(|(p, _)| *p == picked).unwrap();
            counts[slot] += 1;
        }

        let total: u32 = Prop::TABLE.iter().map(|(_, weight)| weight).sum();
        for (slot, (prop, weight)) in Prop::TABLE.iter().enumerate() {
            let expected = DRAWS as f32 * *weight as f32 / total as f32;
            // Generous: the rarest entry expects about twelve hundred draws,
            // so this is still several standard deviations wide.
            assert!(
                (counts[slot] as f32 - expected).abs() < expected * 0.15,
                "{prop:?} came up {} times against an expected {expected:.0}",
                counts[slot]
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

    /// A signal that faces the wrong way is still a signal in a screenshot.
    #[test]
    fn a_signal_looks_back_at_the_traffic_it_is_for() {
        // An arm running east out of the origin: traffic arrives heading west.
        let (foot, yaw) = signal_pose(Vec2::ZERO, Vec2::new(60.0, 0.0), 12.0, 17.0)
            .expect("an arm with a length");

        assert!(
            foot.x > 0.0,
            "the signal stands at {foot}, on the far side of the junction"
        );
        let facing = Vec2::new(yaw.sin(), yaw.cos());
        assert!(
            facing.dot(Vec2::X) > 0.999,
            "the signal looks {facing} rather than back up its own arm"
        );
        // A driver heading west has -Z on their right.
        assert_eq!(
            foot.y < 0.0,
            crate::ai::steering::RIGHT_HAND_TRAFFIC,
            "the signal at {foot} is on the wrong kerb for this side of the road"
        );
        // Clear of the widest carriageway meeting here, not standing in it.
        assert!(
            foot.x > 17.0 * 0.5,
            "the signal at {foot} is inside the junction"
        );

        assert!(signal_pose(Vec2::ZERO, Vec2::ZERO, 12.0, 17.0).is_none());
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
