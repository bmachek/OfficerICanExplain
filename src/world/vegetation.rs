//! Things that grow.
//!
//! There was not a single tree in this city. A park was a green rectangle and a
//! residential street was two rows of boxes with a bin between them, and no
//! amount of work on the renderer was going to fix either — a street reads as a
//! street partly because something on it is *soft*, and every surface here was
//! flat, hard and man-made.
//!
//! Three decisions carry the whole module:
//!
//! * **Two entities per tree.** A trunk whose origin is at its foot, and one
//!   crown as its child. The crown is several blobs merged into a single mesh
//!   at build time rather than several children at spawn time, because the
//!   silhouette is what distinguishes a plane from a poplar and merging costs
//!   nothing once, per species, at startup.
//! * **The trunk pivots.** Its mesh is translated so that the origin sits on
//!   the ground, which means the wind can lean the whole tree by writing a
//!   rotation and nothing else — no per-vertex animation, no custom material,
//!   and the crown comes with it because it is a child.
//! * **Only what is on screen sways.** The sway system skips anything the
//!   camera cannot see. Writing `Transform` marks an entity for transform
//!   propagation and a fresh upload of its instance data, so animating every
//!   tree in a nine-hundred-metre radius would cost far more than drawing them.
//!
//! Placement follows the same rule as everything else streamed: it is drawn
//! from the chunk's own RNG stream, so walking out of a chunk and back into it
//! regrows exactly the same trees.

use bevy::camera::visibility::{ViewVisibility, VisibilityRange};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

use super::buildings::{ChunkOf, SIDEWALK_HEIGHT};
use super::citygen::{Block, District};
use super::roadgraph::RoadEdge;
use super::weather::Weather;

/// Metres between street trees along a kerb.
const SPACING: f32 = 17.0;
/// How far in from the kerb line a trunk stands.
const SET_BACK: f32 = 1.15;
/// Fraction of streets that are planted at all.
///
/// Not all of them, and not at random per tree: a street either is an avenue or
/// it is not, and half a row of trees reads as trees that have died.
const AVENUE: f32 = 0.42;
/// How much likelier an arterial road is to be an avenue.
const AVENUE_ARTERIAL: f32 = 1.7;

/// Metres between trees in a park, before jitter.
const PARK_SPACING: f32 = 11.0;
/// How far inside a park's edge planting starts.
const PARK_MARGIN: f32 = 4.5;
/// Chance a park grid cell has a tree in it.
const PARK_DENSITY: f32 = 0.55;

/// How far away foliage stops being drawn, before `lod_scale`.
///
/// Further than street furniture: a plane tree is six metres across and reads
/// as a shape on the street long after a bollard has stopped being a pixel.
pub const RANGE: f32 = 500.0;

/// Wind speed, in metres per second, at which a tree leans as far as it is
/// going to. Above this it thrashes rather than leans, and thrashing is not
/// something a rigid rotation can portray honestly.
const GALE: f32 = 12.0;
/// How far the strongest wind lays the most flexible species over, in radians.
const LEAN: f32 = 0.085;
/// Rotation below which a tree counts as not having moved, in radians.
///
/// A thousandth of a degree. It exists to make still air free rather than to
/// quantise the movement.
const STILL: f32 = 3.0e-5;

/// What a tree is, which is mostly a statement about its silhouette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Species {
    /// Broad, high-crowned, and the standard street tree of half the cities in
    /// the world for the good reason that it tolerates being one.
    Plane,
    /// Rounded and domestic. The residential street's tree.
    Lime,
    /// Columnar. Narrow enough to line a road that has no room for a tree.
    Poplar,
    /// Small and ornamental, for a forecourt or a park path.
    Cherry,
}

impl Species {
    pub const ALL: [Species; 4] = [
        Species::Plane,
        Species::Lime,
        Species::Poplar,
        Species::Cherry,
    ];

    fn index(self) -> usize {
        match self {
            Species::Plane => 0,
            Species::Lime => 1,
            Species::Poplar => 2,
            Species::Cherry => 3,
        }
    }

    /// Trunk radius and clear height to the underside of the crown.
    fn trunk(self) -> (f32, f32) {
        match self {
            Species::Plane => (0.24, 3.6),
            Species::Lime => (0.19, 2.6),
            Species::Poplar => (0.17, 2.4),
            Species::Cherry => (0.13, 1.9),
        }
    }

    /// The blobs the crown is made of: centre, then radius, both relative to
    /// the top of the trunk.
    ///
    /// Three offset blobs for a plane tree, three stacked ones for a poplar.
    /// This is the entire difference between the species as far as anyone
    /// standing on the pavement can tell.
    fn crown(self) -> &'static [(Vec3, f32)] {
        const PLANE: [(Vec3, f32); 3] = [
            (Vec3::new(0.0, 1.9, 0.0), 2.5),
            (Vec3::new(-1.5, 1.1, 0.7), 1.7),
            (Vec3::new(1.4, 1.3, -0.8), 1.8),
        ];
        const LIME: [(Vec3, f32); 2] = [
            (Vec3::new(0.0, 1.7, 0.0), 2.1),
            (Vec3::new(0.5, 0.6, 0.4), 1.5),
        ];
        const POPLAR: [(Vec3, f32); 3] = [
            (Vec3::new(0.0, 1.6, 0.0), 1.2),
            (Vec3::new(0.0, 3.5, 0.0), 1.0),
            (Vec3::new(0.0, 5.2, 0.0), 0.7),
        ];
        const CHERRY: [(Vec3, f32); 2] = [
            (Vec3::new(0.0, 1.0, 0.0), 1.4),
            (Vec3::new(-0.6, 0.5, 0.3), 1.0),
        ];

        match self {
            Species::Plane => &PLANE,
            Species::Lime => &LIME,
            Species::Poplar => &POPLAR,
            Species::Cherry => &CHERRY,
        }
    }

    /// How far a full gale lays it over, as a fraction of [`LEAN`].
    ///
    /// A plane tree with a trunk a quarter of a metre thick barely moves; a
    /// young cherry moves a lot. Getting this the wrong way round is the sort
    /// of thing that reads as wrong without anybody being able to say why.
    fn give(self) -> f32 {
        match self {
            Species::Plane => 0.45,
            Species::Lime => 0.7,
            Species::Poplar => 1.0,
            Species::Cherry => 0.9,
        }
    }

    /// Overall height, ground to the top of the crown.
    fn height(self) -> f32 {
        let (_, clear) = self.trunk();
        clear
            + self
                .crown()
                .iter()
                .map(|(centre, radius)| centre.y + radius)
                .fold(0.0, f32::max)
    }

    /// How stiff it is, up to a constant nobody needs.
    ///
    /// A tree in wind is a cantilever with the load at the top, and a
    /// cantilever's tip deflection goes as its length cubed over the second
    /// moment of its section — which for a round trunk is the fourth power of
    /// its radius. So height matters more than thickness does, which is why a
    /// poplar bends further than a cherry despite the thicker trunk.
    fn stiffness(self) -> f32 {
        let (radius, _) = self.trunk();
        radius.powi(4) / self.height().powi(3)
    }

    fn foliage(self) -> Color {
        match self {
            Species::Plane => Color::srgb(0.24, 0.35, 0.16),
            Species::Lime => Color::srgb(0.29, 0.40, 0.17),
            Species::Poplar => Color::srgb(0.26, 0.37, 0.20),
            Species::Cherry => Color::srgb(0.34, 0.40, 0.22),
        }
    }

    /// Which species line a street, and how often. Poplars where a plane tree
    /// would not fit, and no cherries: they belong in gardens.
    fn on_streets() -> [(Species, u32); 3] {
        [
            (Species::Plane, 5),
            (Species::Lime, 4),
            (Species::Poplar, 2),
        ]
    }

    fn in_parks() -> [(Species, u32); 4] {
        [
            (Species::Plane, 4),
            (Species::Lime, 5),
            (Species::Cherry, 3),
            (Species::Poplar, 2),
        ]
    }

    fn pick(table: &[(Species, u32)], rng: &mut ChaCha8Rng) -> Species {
        let total: u32 = table.iter().map(|(_, weight)| weight).sum();
        let mut ticket = rng.random_range(0..total);
        for &(species, weight) in table {
            if ticket < weight {
                return species;
            }
            ticket -= weight;
        }
        table[0].0
    }
}

/// A tree that the wind can get at. Sits on the trunk, whose origin is its foot.
#[derive(Component)]
pub struct Sways {
    /// Which way it was planted, kept so the sway can be composed on top of it.
    yaw: f32,
    /// How far this individual leans in a full gale, in radians.
    give: f32,
    /// Where in its own cycle it is, so a row of trees does not move as one.
    phase: f32,
}

#[derive(Resource)]
pub struct FoliageKit {
    /// Trunk mesh and bark material, per species. The mesh's origin is its foot.
    trunk: Vec<(Handle<Mesh>, Handle<StandardMaterial>)>,
    /// The whole crown as one mesh, and how high above the foot it hangs.
    crown: Vec<(Handle<Mesh>, Handle<StandardMaterial>, f32)>,
    hedge: (Handle<Mesh>, Handle<StandardMaterial>),
}

pub fn build_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> FoliageKit {
    let bark = materials.add(StandardMaterial {
        base_color: Color::srgb(0.27, 0.24, 0.21),
        perceptual_roughness: 0.94,
        ..default()
    });

    let mut trunk = Vec::with_capacity(Species::ALL.len());
    let mut crown = Vec::with_capacity(Species::ALL.len());
    for species in Species::ALL {
        let (radius, clear) = species.trunk();
        // The trunk runs a little into the crown so there is no gap to see
        // daylight through, and is translated so the entity's origin is the
        // point it grows out of — which is what lets the wind rotate it.
        let height = clear + 0.8;
        trunk.push((
            meshes.add(
                Cylinder::new(radius, height)
                    .mesh()
                    .resolution(7)
                    .build()
                    .translated_by(Vec3::Y * height * 0.5),
            ),
            bark.clone(),
        ));

        crown.push((
            meshes.add(crown_mesh(species)),
            materials.add(StandardMaterial {
                base_color: species.foliage(),
                perceptual_roughness: 0.98,
                ..default()
            }),
            clear,
        ));
    }

    FoliageKit {
        trunk,
        crown,
        hedge: (
            meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.20, 0.31, 0.15),
                perceptual_roughness: 1.0,
                ..default()
            }),
        ),
    }
}

/// Merges a species' blobs into one mesh.
///
/// Low subdivision on purpose. A crown is read as a silhouette against the sky
/// and as a shadow on the pavement; smoothing it costs triangles on every tree
/// in the city and buys a rounder edge nobody looks at.
fn crown_mesh(species: Species) -> Mesh {
    let mut blobs = species.crown().iter();
    let (first, radius) = blobs.next().expect("every species has a crown");
    let mut mesh = ball(*radius).translated_by(*first);

    for (centre, radius) in blobs {
        if let Err(error) = mesh.merge(&ball(*radius).translated_by(*centre)) {
            warn!("a {species:?} lost part of its crown: {error}");
        }
    }
    mesh
}

fn ball(radius: f32) -> Mesh {
    Sphere::new(radius)
        .mesh()
        .ico(1)
        .unwrap_or_else(|_| Sphere::new(radius).mesh().uv(7, 5))
}

/// Plants one tree, and hands back the trunk so a caller can add to it.
fn plant(
    commands: &mut Commands,
    kit: &FoliageKit,
    chunk: IVec2,
    at: Vec2,
    ground: f32,
    species: Species,
    rng: &mut ChaCha8Rng,
    range: f32,
) {
    let (trunk, bark) = &kit.trunk[species.index()];
    let (crown, leaves, clear) = &kit.crown[species.index()];

    // One scale for the whole tree, so proportions stay the species' own.
    let size = rng.random_range(0.82..1.24);
    let yaw = rng.random_range(0.0..std::f32::consts::TAU);
    // A visibility range is *not* inherited: a crown without one of its own
    // would go on hanging in the air after its trunk stopped being drawn.
    let draw = VisibilityRange {
        start_margin: 0.0..0.0,
        end_margin: (range.max(1.0) * 0.9)..range.max(1.0),
        use_aabb: false,
    };

    commands.spawn((
        ChunkOf(chunk),
        Sways {
            yaw,
            give: species.give(),
            phase: rng.random_range(0.0..std::f32::consts::TAU),
        },
        Mesh3d(trunk.clone()),
        MeshMaterial3d(bark.clone()),
        Transform::from_xyz(at.x, ground, at.y)
            .with_rotation(Quat::from_rotation_y(yaw))
            .with_scale(Vec3::splat(size)),
        draw.clone(),
        children![(
            Mesh3d(crown.clone()),
            MeshMaterial3d(leaves.clone()),
            Transform::from_xyz(0.0, *clear, 0.0),
            draw,
        )],
    ));
}

/// Plants both kerbs of one street, if this street is an avenue at all.
pub fn spawn_edge(
    commands: &mut Commands,
    kit: &FoliageKit,
    rng: &mut ChaCha8Rng,
    edge: &RoadEdge,
    from: Vec2,
    to: Vec2,
    chunk: IVec2,
    range: f32,
) {
    let chance = AVENUE * if edge.arterial { AVENUE_ARTERIAL } else { 1.0 };
    if rng.random_range(0.0..1.0) > chance {
        return;
    }

    let Ok(direction) = Dir2::new(to - from) else {
        return;
    };
    let normal = Vec2::new(-direction.y, direction.x);
    let offset = edge.width * 0.5 + SET_BACK;

    // One species for the street. A row of trees is planted at once, by one
    // council, from one nursery; mixing them per tree is the tell.
    let species = Species::pick(&Species::on_streets(), rng);

    let slots = (edge.length / SPACING).floor() as i32;
    for i in 1..slots {
        for side in [-1.0f32, 1.0] {
            // A gap where a crossing or a driveway would be.
            if rng.random_range(0.0..1.0) > 0.86 {
                continue;
            }
            let jitter = rng.random_range(-1.1..1.1);
            let at = from + *direction * (i as f32 * SPACING + jitter) + normal * offset * side;
            plant(
                commands,
                kit,
                chunk,
                at,
                SIDEWALK_HEIGHT,
                species,
                rng,
                range,
            );
        }
    }
}

/// Plants a park, which is the only block in the city that is not paved.
///
/// A park was a green rectangle. What makes it a park rather than a lawn is
/// that it has things standing in it at different distances, so that walking
/// through it changes what you can see — hence a scattered canopy rather than
/// an avenue, and a hedge along the edge to give it a boundary.
pub fn spawn_park(
    commands: &mut Commands,
    kit: &FoliageKit,
    rng: &mut ChaCha8Rng,
    block: &Block,
    chunk: IVec2,
    range: f32,
) {
    if block.district != District::Park {
        return;
    }
    let planted = block.area.inset(PARK_MARGIN);
    if !planted.is_valid() {
        return;
    }

    let size = planted.size();
    let (across, down) = (
        (size.x / PARK_SPACING).floor() as i32,
        (size.y / PARK_SPACING).floor() as i32,
    );
    // A grid with jitter inside each cell, for the same reason the roof kit
    // uses one: it cannot put two trees in the same place however the dice
    // fall, and it finishes in a fixed number of steps.
    for row in 0..down {
        for column in 0..across {
            if rng.random_range(0.0..1.0) > PARK_DENSITY {
                continue;
            }
            let cell = Vec2::new(
                planted.min.x + (column as f32 + 0.5) * PARK_SPACING,
                planted.min.y + (row as f32 + 0.5) * PARK_SPACING,
            );
            let jitter = Vec2::new(
                rng.random_range(-PARK_SPACING * 0.35..PARK_SPACING * 0.35),
                rng.random_range(-PARK_SPACING * 0.35..PARK_SPACING * 0.35),
            );
            let species = Species::pick(&Species::in_parks(), rng);
            plant(
                commands,
                kit,
                chunk,
                cell + jitter,
                SIDEWALK_HEIGHT,
                species,
                rng,
                range,
            );
        }
    }

    hedge(commands, kit, block, chunk, range);
}

/// A clipped hedge along each side of a park, in segments with gaps for the
/// ways in.
fn hedge(commands: &mut Commands, kit: &FoliageKit, block: &Block, chunk: IVec2, range: f32) {
    const HEIGHT: f32 = 0.95;
    const DEPTH: f32 = 0.7;
    /// How far in from the kerb the hedge is planted.
    const INSET: f32 = 1.6;
    /// The width of the way in at the middle of each side.
    const GATE: f32 = 6.0;

    let (mesh, leaves) = &kit.hedge;
    let area = block.area.inset(INSET);
    if !area.is_valid() {
        return;
    }
    let size = area.size();
    let centre = area.center();
    let draw = range.max(1.0);

    // Four sides, each in two runs with a gap between them.
    for (along_x, sign) in [(true, -1.0f32), (true, 1.0), (false, -1.0), (false, 1.0)] {
        let span = if along_x { size.x } else { size.y };
        let run = (span - GATE) * 0.5;
        if run <= 0.5 {
            continue;
        }
        for end in [-1.0f32, 1.0] {
            let offset = end * (GATE * 0.5 + run * 0.5);
            let (at, scale) = if along_x {
                (
                    Vec2::new(centre.x + offset, centre.y + sign * size.y * 0.5),
                    Vec3::new(run, HEIGHT, DEPTH),
                )
            } else {
                (
                    Vec2::new(centre.x + sign * size.x * 0.5, centre.y + offset),
                    Vec3::new(DEPTH, HEIGHT, run),
                )
            };
            commands.spawn((
                ChunkOf(chunk),
                Mesh3d(mesh.clone()),
                MeshMaterial3d(leaves.clone()),
                Transform::from_xyz(at.x, SIDEWALK_HEIGHT + HEIGHT * 0.5, at.y).with_scale(scale),
                VisibilityRange {
                    start_margin: 0.0..0.0,
                    end_margin: (draw * 0.9)..draw,
                    use_aabb: false,
                },
                // Knee-high and up against a kerb. Its own shadow is a line on
                // the ground that the ambient occlusion already draws.
                NotShadowCaster,
            ));
        }
    }
}

// ------------------------------------------------------------------ wind ----

/// The axis a tree turns about to lean downwind.
///
/// Rotating by a small angle about this axis moves the crown along the wind:
/// the tilt direction of `Y` under `axis` is `axis × Y`, which for an axis in
/// the ground plane comes out perpendicular to the axis — so the axis itself
/// has to be perpendicular to the wind, not along it. Getting this a quarter
/// turn out is invisible on a still day and unmissable in a gale.
pub fn lean_axis(wind: Vec2) -> Option<Vec3> {
    Dir2::new(wind)
        .ok()
        .map(|wind| Vec3::new(wind.y, 0.0, -wind.x))
}

/// How far a tree of unit give leans, in radians, at this wind speed.
pub fn lean(speed: f32) -> f32 {
    LEAN * (speed / GALE).clamp(0.0, 1.0)
}

/// Leans everything the camera can see, and nothing it cannot.
///
/// The visibility check is not an optimisation of the maths — the maths is four
/// sines. It is an optimisation of the *write*: touching `Transform` queues an
/// entity for transform propagation and for a fresh upload of its instance
/// data, and there are thousands of these standing in chunks nobody is looking
/// at.
fn sway(
    time: Res<Time>,
    weather: Res<Weather>,
    mut trees: Query<(&mut Transform, &Sways, &ViewVisibility)>,
) {
    let Some(axis) = lean_axis(weather.wind) else {
        return;
    };
    let reach = lean(weather.wind_speed());
    let now = time.elapsed_secs();

    for (mut transform, tree, visible) in &mut trees {
        if !visible.get() {
            continue;
        }
        // Two frequencies rather than one: a single sine is a metronome, and a
        // street of metronomes in step is worse than no movement at all.
        let gust =
            (now * 0.9 + tree.phase).sin() * 0.6 + (now * 2.3 + tree.phase * 1.7).sin() * 0.4;
        // Always downwind, never upwind — a gust adds to a lean, it does not
        // reverse it — so the cycle runs from a third of the lean to all of it.
        let angle = reach * tree.give * (0.66 + 0.34 * gust);
        let wanted = Quat::from_axis_angle(axis, angle) * Quat::from_rotation_y(tree.yaw);

        // `Mut` marks a component changed the instant it is dereferenced
        // mutably, and a changed `Transform` is a propagation and an instance
        // upload whether or not the value differs. So the current rotation is
        // read past the bypass and written only when the tree has actually
        // moved — which on a still day is never.
        if transform
            .bypass_change_detection()
            .rotation
            .angle_between(wanted)
            > STILL
        {
            transform.rotation = wanted;
        }
    }
}

pub struct VegetationPlugin;

impl Plugin for VegetationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, sway);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;

    #[test]
    fn every_species_has_a_trunk_to_hang_its_crown_on() {
        for species in Species::ALL {
            let (radius, clear) = species.trunk();
            assert!(radius > 0.0 && clear > 1.5, "{species:?} is not a tree");
            assert!(
                !species.crown().is_empty(),
                "{species:?} has nothing on top of it"
            );
            // The lowest blob has to reach down to the top of the trunk, or
            // there is a gap you can see the sky through.
            let lowest = species
                .crown()
                .iter()
                .map(|(centre, radius)| centre.y - radius)
                .fold(f32::MAX, f32::min);
            assert!(
                lowest < 0.8,
                "{species:?}'s crown floats {lowest}m above its trunk"
            );
        }
    }

    /// A stiffer tree moves less. The only property of the wind model anybody
    /// would notice being wrong — and the reason it is checked against the
    /// cantilever stiffness rather than against trunk thickness is that a
    /// poplar has a thicker trunk than a cherry and still bends further,
    /// because it is twice the height.
    #[test]
    fn the_stiffer_tree_is_the_one_that_gives_less() {
        let mut by_stiffness = Species::ALL;
        by_stiffness.sort_by(|a, b| b.stiffness().total_cmp(&a.stiffness()));
        for pair in by_stiffness.windows(2) {
            assert!(
                pair[0].give() <= pair[1].give(),
                "{:?} is stiffer than {:?} but gives more ({} against {})",
                pair[0],
                pair[1],
                pair[0].give(),
                pair[1].give()
            );
        }
        // And the ordering has to be a real one rather than four equal values.
        assert!(
            by_stiffness[0].give() < by_stiffness[Species::ALL.len() - 1].give(),
            "every species gives the same amount"
        );
    }

    /// The lean has to go *with* the wind. A quarter turn out is a rotation
    /// that still looks like movement, which is why it needs a test and not an
    /// eye.
    #[test]
    fn trees_lean_downwind() {
        for wind in [
            Vec2::new(6.0, 0.0),
            Vec2::new(0.0, -4.0),
            Vec2::new(-3.0, 3.0),
        ] {
            let axis = lean_axis(wind).expect("a wind with a direction");
            let leaned = Quat::from_axis_angle(axis, lean(wind.length())) * Vec3::Y;
            let drift = Vec2::new(leaned.x, leaned.z);
            assert!(drift.length() > 1e-4, "no lean at all in a {wind} wind");
            assert!(
                drift.normalize().dot(wind.normalize()) > 0.999,
                "a {wind} wind leant the tree towards {drift}"
            );
        }
        assert!(
            lean_axis(Vec2::ZERO).is_none(),
            "still air has no direction"
        );
    }

    #[test]
    fn the_lean_saturates_rather_than_running_away() {
        assert_eq!(lean(0.0), 0.0);
        assert!(lean(GALE * 0.5) < lean(GALE));
        assert_eq!(lean(GALE), lean(GALE * 10.0));
        assert!(lean(GALE) <= LEAN);
    }

    #[test]
    fn street_planting_avoids_the_ornamental_and_park_planting_does_not() {
        let streets: Vec<_> = Species::on_streets().iter().map(|(s, _)| *s).collect();
        assert!(!streets.contains(&Species::Cherry));
        let parks: Vec<_> = Species::in_parks().iter().map(|(s, _)| *s).collect();
        for species in Species::ALL {
            assert!(parks.contains(&species), "{species:?} grows nowhere");
        }
    }

    #[test]
    fn the_weighted_pick_can_reach_every_entry() {
        let mut rng = ChaCha8Rng::seed_from_u64(9);
        let table = Species::in_parks();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..3000 {
            seen.insert(Species::pick(&table, &mut rng));
        }
        assert_eq!(seen.len(), table.len());
    }

    /// Two chunks must not plant the same trees, and one chunk must plant the
    /// same trees every time it is walked back into.
    #[test]
    fn planting_is_per_chunk_and_repeatable() {
        let draw = |chunk: (i32, i32)| {
            let mut rng = crate::core::rng::stream_for_chunk(
                0xBEEF,
                crate::core::rng::stream::VEGETATION,
                chunk,
            );
            (0..8)
                .map(|_| rng.random_range(0.0..1.0f32))
                .collect::<Vec<_>>()
        };
        assert_eq!(draw((3, -4)), draw((3, -4)));
        assert_ne!(draw((3, -4)), draw((-4, 3)));
        assert_ne!(draw((0, 0)), draw((0, 1)));
    }
}
