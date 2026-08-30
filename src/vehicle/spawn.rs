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

#[derive(Resource)]
pub struct VehicleAssets {
    body: Handle<Mesh>,
    wheel: Handle<Mesh>,
    tyre: Handle<StandardMaterial>,
    glass: Handle<StandardMaterial>,
}

pub fn build_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> VehicleAssets {
    VehicleAssets {
        body: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        // Bevy cylinders run along Y; the child transform lays it on its side.
        wheel: meshes.add(Cylinder::new(1.0, 0.24)),
        tyre: materials.add(StandardMaterial {
            base_color: Color::srgb(0.06, 0.06, 0.07),
            perceptual_roughness: 0.95,
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
    let paint = materials.add(StandardMaterial {
        base_color: spec.body_color,
        perceptual_roughness: 0.45,
        metallic: 0.25,
        ..default()
    });

    let anchors = spec.wheel_anchors();
    let wheel_radius = spec.wheel_radius;
    let name = spec.display_name;

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
        // Body shell. Kept separate from the collider so the visual can be
        // shaped without changing how the car collides.
        parent.spawn((
            Mesh3d(assets.body.clone()),
            MeshMaterial3d(paint.clone()),
            Transform::from_scale(size),
        ));
        // Cabin, set back and narrower, so the car reads as having a front.
        parent.spawn((
            Mesh3d(assets.body.clone()),
            MeshMaterial3d(assets.glass.clone()),
            Transform::from_xyz(0.0, size.y * 0.52, size.z * 0.06).with_scale(Vec3::new(
                size.x * 0.82,
                size.y * 0.60,
                size.z * 0.44,
            )),
        ));

        for (index, anchor) in anchors.iter().enumerate().take(WHEEL_COUNT) {
            parent.spawn((
                WheelVisual(index),
                Mesh3d(assets.wheel.clone()),
                MeshMaterial3d(assets.tyre.clone()),
                Transform::from_translation(*anchor)
                    .with_scale(Vec3::splat(wheel_radius))
                    .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
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
            // Lay the cylinder on its side, then spin it about its axle.
            transform.rotation = steer
                * Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)
                * Quat::from_rotation_y(-state.wheel_spin);
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
        let spec = class.spec();
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

        let spec = VehicleClass::Sedan.spec();
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
