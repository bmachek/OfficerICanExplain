//! Building vehicle entities and scattering parked ones around the city.

use avian3d::prelude::*;
use bevy::prelude::*;
use rand::RngExt;

use super::controller::{VehicleInput, VehicleState};
use super::spec::{VehicleClass, VehicleSpec, WHEEL_COUNT};
use crate::core::config::GameConfig;
use crate::core::rng::{stream, stream_for};
use crate::player::interact::DrivenBy;
use crate::world::City;

/// Marks a vehicle the player can get into.
#[derive(Component)]
pub struct Vehicle;

/// A vehicle close enough to matter, and therefore simulated.
///
/// Several hundred parked cars are scattered across the city. Simulating all of
/// them means Avian solving hundreds of dynamic bodies and this module casting
/// four suspension rays each, every tick, for cars nobody can see. Distant ones
/// are held static instead, and because they are parked at exactly their
/// settled ride height, switching back to dynamic causes no visible pop.
#[derive(Component)]
pub struct ActiveVehicle;

/// Vehicles within this distance of the camera are simulated.
pub const ACTIVE_RADIUS: f32 = 140.0;

/// Exempts a vehicle from distance-based deactivation.
///
/// Traffic and police manage their own lifetimes and must keep driving out to
/// their own despawn ranges; freezing them the moment they pass the parked-car
/// radius would strand a pursuit the instant it fell behind.
#[derive(Component)]
pub struct AlwaysSimulated;

/// A wheel mesh, positioned each frame from the suspension state.
#[derive(Component)]
pub struct WheelVisual(pub usize);

/// The three lofts for one archetype, as loaded meshes.
struct BodyHandles {
    shell: Handle<Mesh>,
    lower: Handle<Mesh>,
    cabin: Option<Handle<Mesh>>,
}

#[derive(Resource)]
pub struct VehicleAssets {
    /// One entry per archetype, built once and shared by every car of that
    /// kind — which is also what lets Bevy batch a street full of them.
    bodies: Vec<(VehicleClass, BodyHandles)>,
    tyre_mesh: Handle<Mesh>,
    rim_mesh: Handle<Mesh>,
    tyre: Handle<StandardMaterial>,
    rim: Handle<StandardMaterial>,
    glass: Handle<StandardMaterial>,
}

impl VehicleAssets {
    fn body(&self, class: VehicleClass) -> &BodyHandles {
        self.bodies
            .iter()
            .find(|(c, _)| *c == class)
            .map(|(_, meshes)| meshes)
            .expect("every archetype gets meshes at startup")
    }
}

/// Fraction of its own radius that a tyre is wide.
const TYRE_WIDTH: f32 = 0.66;

pub fn build_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> VehicleAssets {
    let bodies = [
        VehicleClass::Sedan,
        VehicleClass::Sports,
        VehicleClass::Truck,
        VehicleClass::Police,
    ]
    .into_iter()
    .map(|class| {
        let built = super::body::build(class, &class.spec());
        // Normal maps need a tangent basis, and mikktspace is the one the
        // shader agrees with.
        let mut add = |mesh| meshes.add(crate::world::buildings::with_tangents(mesh));
        (
            class,
            BodyHandles {
                shell: add(built.shell),
                lower: add(built.lower),
                cabin: built.cabin.map(&mut add),
            },
        )
    })
    .collect();

    VehicleAssets {
        bodies,
        tyre_mesh: meshes.add(crate::world::buildings::with_tangents(
            super::body::tyre_mesh(TYRE_WIDTH),
        )),
        rim_mesh: meshes.add(crate::world::buildings::with_tangents(
            super::body::rim_mesh(TYRE_WIDTH),
        )),
        tyre: materials.add(StandardMaterial {
            base_color: Color::srgb(0.34, 0.34, 0.36),
            base_color_texture: Some(images.add(super::paint::tyre())),
            normal_map_texture: Some(images.add(super::paint::tyre_normal())),
            perceptual_roughness: 0.94,
            ..default()
        }),
        rim: materials.add(StandardMaterial {
            base_color: Color::srgb(0.82, 0.83, 0.86),
            base_color_texture: Some(images.add(super::paint::rim())),
            normal_map_texture: Some(images.add(super::paint::rim_normal())),
            metallic_roughness_texture: Some(images.add(super::paint::rim_surface())),
            perceptual_roughness: 1.0,
            metallic: 1.0,
            ..default()
        }),
        glass: materials.add(StandardMaterial {
            base_color: Color::srgb(0.14, 0.17, 0.22),
            perceptual_roughness: 0.25,
            metallic: 0.4,
            ..default()
        }),
    }
}

pub fn spawn_vehicle(
    commands: &mut Commands,
    assets: &VehicleAssets,
    materials: &mut Assets<StandardMaterial>,
    spec: VehicleSpec,
    transform: Transform,
) -> Entity {
    let size = spec.half_extents * 2.0;
    // Car paint is a coloured base under a clear lacquer, and modelling it that
    // way rather than as "shiny metal" is what makes the highlight sit *on* the
    // panel instead of tinting itself the colour of the car.
    let paint = materials.add(StandardMaterial {
        base_color: spec.body_color,
        // Flake in the basecoat, then lacquer over the top. Metallic paint is
        // rougher underneath than solid paint and reads duller for it, which is
        // why the roughness moves with the flake rather than staying put.
        perceptual_roughness: 0.30 + spec.body_metallic * 0.22,
        metallic: spec.body_metallic,
        clearcoat: 1.0,
        clearcoat_perceptual_roughness: 0.08,
        ..default()
    });

    let anchors = spec.wheel_anchors();
    let wheel_radius = spec.wheel_radius;
    let name = spec.display_name;
    let body = assets.body(spec.class);

    let mut entity = commands.spawn((
        Name::new(name),
        Vehicle,
        transform,
        Visibility::default(),
        // Parked cars start static; `activate_nearby_vehicles` promotes them.
        RigidBody::Static,
        Collider::cuboid(size.x, size.y, size.z),
        Mass(spec.mass),
        // Lowering the centre of mass is what stops it rolling over in corners.
        CenterOfMass(spec.center_of_mass),
        VehicleInput::default(),
        VehicleState::default(),
        super::damage::VehicleHealth::default(),
        super::damage::PreviousVelocity::default(),
        spec,
    ));

    entity.with_children(|parent| {
        // The shell is already built at this spec's size, so unlike the box it
        // replaced it needs no scaling. It stays separate from the collider:
        // the car collides as the box it always did, and the bodywork is free
        // to be any shape inside that.
        parent.spawn((
            Mesh3d(body.shell.clone()),
            MeshMaterial3d(paint.clone()),
            Transform::IDENTITY,
        ));
        // The sill, in the same paint. Separate only so the shell above it can
        // arch over the wheels without the body pinching in half.
        parent.spawn((
            Mesh3d(body.lower.clone()),
            MeshMaterial3d(paint.clone()),
            Transform::IDENTITY,
        ));
        if let Some(cabin) = &body.cabin {
            parent.spawn((
                Mesh3d(cabin.clone()),
                MeshMaterial3d(assets.glass.clone()),
                Transform::IDENTITY,
            ));
        }

        for (index, anchor) in anchors.iter().enumerate().take(WHEEL_COUNT) {
            // Wheels are built about the X axis at unit radius, so the whole
            // assembly scales with the spec and needs no laying-on-its-side.
            parent
                .spawn((
                    WheelVisual(index),
                    Mesh3d(assets.tyre_mesh.clone()),
                    MeshMaterial3d(assets.tyre.clone()),
                    Transform::from_translation(*anchor).with_scale(Vec3::splat(wheel_radius)),
                ))
                .with_child((
                    Mesh3d(assets.rim_mesh.clone()),
                    MeshMaterial3d(assets.rim.clone()),
                    // The face belongs on the outside of the car, so the
                    // left-hand wheels wear theirs turned around. A negative
                    // scale would do it too, and would turn every triangle
                    // inside out.
                    Transform::from_rotation(if anchor.x < 0.0 {
                        Quat::from_rotation_y(std::f32::consts::PI)
                    } else {
                        Quat::IDENTITY
                    }),
                ));
        }
    });

    entity.id()
}

/// Yaw that points a vehicle's forward axis along `direction` (an XZ vector).
///
/// Worth its own function and test: Bevy's forward is -Z, and `Vec2::to_angle`
/// measures from +X, so the naive conversion silently parks every car facing
/// across its street instead of along it.
pub fn heading_towards(direction: Vec2) -> f32 {
    // Rotating -Z by yaw gives (-sin yaw, -cos yaw), so invert that.
    (-direction.x).atan2(-direction.y)
}

/// Height the body origin sits at once the springs balance the car's weight.
/// Spawning at exactly this height means a car promoted from static to dynamic
/// neither drops nor pops.
pub fn resting_height(spec: &VehicleSpec) -> f32 {
    let load_per_wheel = spec.mass * 9.81 / WHEEL_COUNT as f32;
    let compression = load_per_wheel / spec.spring_strength;
    (spec.max_ray_length() - compression) - spec.axle_height
}

/// Promotes nearby vehicles to dynamic and demotes distant ones.
pub fn activate_nearby_vehicles(
    mut commands: Commands,
    cameras: Query<&GlobalTransform, With<crate::player::camera::CameraRig>>,
    vehicles: Query<
        (
            Entity,
            &Transform,
            Has<ActiveVehicle>,
            Has<DrivenBy>,
            Has<AlwaysSimulated>,
        ),
        With<Vehicle>,
    >,
) {
    let Ok(camera) = cameras.single() else { return };
    let focus = camera.translation();

    for (entity, transform, active, driven, always) in &vehicles {
        let wanted = driven || always || transform.translation.distance(focus) < ACTIVE_RADIUS;
        match (wanted, active) {
            (true, false) => {
                commands.entity(entity).insert((
                    ActiveVehicle,
                    RigidBody::Dynamic,
                    // A sleeping car ignores its own suspension.
                    SleepingDisabled,
                ));
            }
            (false, true) => {
                commands
                    .entity(entity)
                    .remove::<ActiveVehicle>()
                    .remove::<SleepingDisabled>()
                    .insert(RigidBody::Static);
            }
            _ => {}
        }
    }
}

/// Positions each wheel mesh from its suspension state.
pub fn update_wheel_visuals(
    vehicles: Query<(&VehicleState, &VehicleSpec, &Children)>,
    mut wheels: Query<(&WheelVisual, &mut Transform)>,
) {
    for (state, spec, children) in &vehicles {
        let anchors = spec.wheel_anchors();
        for child in children.iter() {
            let Ok((wheel, mut transform)) = wheels.get_mut(child) else {
                continue;
            };
            let index = wheel.0;
            let wheel_state = &state.wheels[index];

            // The mesh hangs below its anchor by however much suspension is extended.
            let drop = wheel_state.ray_length - spec.wheel_radius;
            transform.translation = anchors[index] - Vec3::Y * drop;

            let steer = if VehicleSpec::is_front(index) {
                Quat::from_rotation_y(state.steer_angle)
            } else {
                Quat::IDENTITY
            };
            // The wheel's axle is its own X axis, so rolling is a rotation
            // about that. Negative: driving forwards is -Z, and the contact
            // patch has to travel backwards relative to the car.
            transform.rotation = steer * Quat::from_rotation_x(-state.wheel_spin);
        }
    }
}

/// Scatters parked cars along the kerbs so there is always something to steal.
pub fn spawn_parked_vehicles(
    mut commands: Commands,
    config: Res<GameConfig>,
    city: Res<City>,
    assets: Res<VehicleAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mut rng = stream_for(config.world_seed, stream::VEHICLE_SPAWNS);
    let mut spawned = 0;

    for edge in city.graph.edges() {
        // Not every segment, or the city reads as a car park — but often
        // enough that there is always one within a short walk, because a crime
        // sandbox where you cannot find a car is not a crime sandbox.
        if rng.random_range(0.0..1.0) > 0.62 {
            continue;
        }

        let a = city.graph.node(edge.a).pos;
        let b = city.graph.node(edge.b).pos;
        let Ok(direction) = Dir2::new(b - a) else {
            continue;
        };
        let normal = Vec2::new(-direction.y, direction.x);

        // Somewhere along the segment, parked against one kerb.
        let along: f32 = rng.random_range(0.25..0.75);
        let side = if rng.random_range(0.0..1.0) < 0.5 {
            1.0
        } else {
            -1.0
        };
        let offset = edge.width * 0.5 - 1.6;
        let position = a + *direction * (edge.length * along) + normal * offset * side;

        let class = VehicleClass::CIVILIAN[rng.random_range(0..VehicleClass::CIVILIAN.len())];
        let mut spec = class.spec();
        (spec.body_color, spec.body_metallic) = super::paint::street_paint(&mut rng);
        // Nose along the street, facing the way traffic on that side runs.
        let facing = if side > 0.0 { *direction } else { -*direction };
        let heading = heading_towards(facing);
        let transform = Transform::from_xyz(position.x, resting_height(&spec), position.y)
            .with_rotation(Quat::from_rotation_y(heading));

        spawn_vehicle(&mut commands, &assets, &mut materials, spec, transform);
        spawned += 1;
    }

    // Guarantee one at the player's start. Relying on the random scatter to
    // put a car within sight of the spawn is a coin flip, and the first thing
    // anyone does is look for something to drive.
    if let Some(start) = city.graph.nearest_node(Vec2::ZERO) {
        let node = city.graph.node(start);
        let along = city.graph.neighbors(start).next().and_then(|(next, edge)| {
            let to = city.graph.node(next).pos - node.pos;
            Dir2::new(to).ok().map(|d| (d, city.graph.edge(edge).width))
        });
        let Some((direction, width)) = along else {
            return;
        };

        let mut spec = VehicleClass::Sedan.spec();
        (spec.body_color, spec.body_metallic) = super::paint::street_paint(&mut rng);
        let heading = heading_towards(*direction);
        // On the carriageway, a little down the street from the junction.
        let normal = Vec2::new(-direction.y, direction.x);
        let position = node.pos + *direction * 12.0 + normal * (width * 0.25);
        let transform = Transform::from_xyz(position.x, resting_height(&spec), position.y)
            .with_rotation(Quat::from_rotation_y(heading));
        spawn_vehicle(&mut commands, &assets, &mut materials, spec, transform);
        spawned += 1;
    }

    info!("{spawned} vehicles parked around the city");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The forward axis a vehicle ends up with for a given heading.
    fn forward_for(heading: f32) -> Vec2 {
        let f = Quat::from_rotation_y(heading) * Vec3::NEG_Z;
        Vec2::new(f.x, f.z)
    }

    #[test]
    fn heading_points_the_car_along_the_street() {
        for direction in [
            Vec2::new(1.0, 0.0),
            Vec2::new(-1.0, 0.0),
            Vec2::new(0.0, 1.0),
            Vec2::new(0.0, -1.0),
            Vec2::new(0.6, 0.8),
            Vec2::new(-0.28, 0.96),
        ] {
            let forward = forward_for(heading_towards(direction));
            assert!(
                forward.distance(direction.normalize()) < 1e-4,
                "heading for {direction:?} produced forward {forward:?}"
            );
        }
    }

    #[test]
    fn resting_height_keeps_the_wheels_on_the_ground() {
        for class in [
            VehicleClass::Sedan,
            VehicleClass::Sports,
            VehicleClass::Truck,
        ] {
            let spec = class.spec();
            let height = resting_height(&spec);
            // The body must clear the road, and the wheels must still reach it.
            assert!(
                height > spec.half_extents.y,
                "{} would spawn with its belly in the road",
                spec.display_name
            );
            // The invariant that matters: the suspension ray from each wheel
            // anchor must reach exactly the road surface. The wheel hangs below
            // its anchor by the remaining travel, not by its radius.
            let load_per_wheel = spec.mass * 9.81 / WHEEL_COUNT as f32;
            let compression = load_per_wheel / spec.spring_strength;
            let anchor = height + spec.axle_height;
            let ray_length = spec.max_ray_length() - compression;
            assert!(
                (anchor - ray_length).abs() < 1e-4,
                "{}: wheel contact lands at {:.3}m instead of the road",
                spec.display_name,
                anchor - ray_length
            );
        }
    }
}
