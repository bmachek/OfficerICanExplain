//! Getting in and out of vehicles, and driving them.
//!
//! The player entity is never destroyed on entering a car — its body is
//! disabled and it is carried along by the vehicle. Keeping one persistent
//! player entity means health, wanted level, weapons and money live in one
//! place and do not need migrating between an "on foot" and an "in car" entity
//! every time someone opens a door.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;

use crate::core::schedule::GameSet;
use crate::core::states::{AppState, InGameState};

use crate::player::input::Action;
use crate::player::on_foot::{CAPSULE_LENGTH, CAPSULE_RADIUS, Player};
use crate::vehicle::controller::VehicleInput;
use crate::vehicle::spawn::Vehicle;

/// On the player: the vehicle they are currently driving.
#[derive(Component)]
pub struct Driving(pub Entity);

/// On the vehicle: who is driving it.
#[derive(Component)]
pub struct DrivenBy(pub Entity);

/// How close the player must be to a car to take it.
const ENTER_RANGE: f32 = 4.0;

/// Where to try putting the player down when they leave a car, in the car's own
/// frame, best first. High enough that the road and a 28cm kerb are not
/// themselves obstacles — the capsule's feet sit at about 0.45 at this height.
const EXIT_OFFSETS: [Vec3; 7] = [
    Vec3::new(-1.9, 1.35, 0.0),
    Vec3::new(1.9, 1.35, 0.0),
    Vec3::new(0.0, 1.35, 3.1),
    Vec3::new(0.0, 1.35, -3.1),
    Vec3::new(-2.9, 1.35, 2.6),
    Vec3::new(2.9, 1.35, 2.6),
    // On the roof. Ugly, but it is never *inside* anything.
    Vec3::new(0.0, 3.0, 0.0),
];

/// Finds somewhere to stand that is not inside a car, a wall or a wreck.
///
/// Both ways out of a vehicle used to drop the player at one fixed offset and
/// hope for the best. Beside a kerb that is fine. In the middle of the pile-up
/// that just wrecked the car it puts them *inside* whatever they hit, and every
/// symptom follows from there: the capsule is wedged so walking does nothing,
/// the follow camera's wall-avoidance cast hits bodywork 40cm away and pulls
/// the view inside the other car, and the screen goes black with no way out.
///
/// So try the offsets in order and take the first one that is actually empty.
pub fn clear_spot(spatial: &SpatialQuery, car: &Transform, ignore: [Entity; 2]) -> Vec3 {
    // A shade under the real capsule: brushing a wall should not read as being
    // buried in it.
    let probe = Collider::capsule(CAPSULE_RADIUS * 0.9, CAPSULE_LENGTH);
    let filter = SpatialQueryFilter::from_excluded_entities(ignore);

    for offset in EXIT_OFFSETS {
        let spot = car.transform_point(offset);
        if spatial
            .shape_intersections(&probe, spot, Quat::IDENTITY, &filter)
            .is_empty()
        {
            return spot;
        }
    }

    // Everything within reach is blocked. Straight up: dropping back onto the
    // wreckage is survivable, being sealed inside it is not.
    car.translation + Vec3::Y * 4.5
}

pub struct InteractPlugin;

impl Plugin for InteractPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                enter_or_exit_vehicle,
                eject_from_wrecked_vehicle,
                carry_driver,
            )
                .chain()
                .in_set(GameSet::Simulation),
        )
        // Not in the Update chain with the rest: `RunFixedMainLoop` runs
        // *before* `Update`, so controls written there used to reach
        // `drive_vehicles` one frame late — a full frame of steering latency
        // on top of the physics tick. Writing them just before the fixed loop
        // hands the controller this frame's keys. The state gate mirrors what
        // `GameSet::Simulation` would have provided.
        .add_systems(
            RunFixedMainLoop,
            drive_from_input
                .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop)
                .run_if(in_state(AppState::InGame).and_then(in_state(InGameState::Playing))),
        );
    }
}

fn enter_or_exit_vehicle(
    mut commands: Commands,
    spatial: SpatialQuery,
    players: Query<(Entity, &ActionState<Action>, &Transform, Option<&Driving>), With<Player>>,
    parked: Query<(Entity, &Transform), (With<Vehicle>, Without<DrivenBy>)>,
    vehicles: Query<&Transform, With<Vehicle>>,
) {
    let Ok((player, action_state, player_transform, driving)) = players.single() else {
        return;
    };
    if !action_state.just_pressed(&Action::Interact) {
        return;
    }

    match driving {
        // --- Get out ---
        Some(Driving(vehicle)) => {
            let drop_at = vehicles
                .get(*vehicle)
                .map(|car| clear_spot(&spatial, car, [player, *vehicle]))
                .unwrap_or(player_transform.translation);

            commands
                .entity(player)
                .remove::<Driving>()
                .remove::<RigidBodyDisabled>()
                .remove::<ColliderDisabled>()
                .insert(Visibility::Visible)
                .insert(Transform::from_translation(drop_at))
                // The body was frozen while riding along; whatever it was doing
                // before it went in must not be handed back on the way out.
                .insert((LinearVelocity::ZERO, AngularVelocity::ZERO));

            commands
                .entity(*vehicle)
                .remove::<DrivenBy>()
                // Leave the handbrake on, so an abandoned car does not roll.
                .insert(VehicleInput {
                    handbrake: true,
                    ..default()
                });
        }

        // --- Get in ---
        None => {
            let origin = player_transform.translation;
            let nearest = parked
                .iter()
                .map(|(entity, transform)| (entity, transform.translation.distance(origin)))
                .filter(|(_, distance)| *distance <= ENTER_RANGE)
                .min_by(|a, b| a.1.total_cmp(&b.1));

            let Some((vehicle, _)) = nearest else { return };

            commands
                .entity(player)
                // Disabled rather than despawned: the character keeps existing,
                // it just stops interacting with the world.
                .insert((
                    Driving(vehicle),
                    RigidBodyDisabled,
                    ColliderDisabled,
                    Visibility::Hidden,
                ));
            commands
                .entity(vehicle)
                .insert((DrivenBy(player), VehicleInput::default()));
        }
    }
}

/// Keeps the driver's transform pinned to their vehicle.
fn carry_driver(
    vehicles: Query<&Transform, (With<Vehicle>, Without<Player>)>,
    mut players: Query<(&Driving, &mut Transform), With<Player>>,
) {
    for (driving, mut transform) in &mut players {
        if let Ok(vehicle) = vehicles.get(driving.0) {
            *transform = *vehicle;
        }
    }
}

/// Translates player input into vehicle controls while driving.
fn drive_from_input(
    players: Query<(&ActionState<Action>, &Driving), With<Player>>,
    mut vehicles: Query<&mut VehicleInput>,
) {
    let Ok((action_state, driving)) = players.single() else {
        return;
    };
    let Ok(mut input) = vehicles.get_mut(driving.0) else {
        return;
    };

    // Same stick as walking: forward is throttle, back is brake then reverse,
    // sideways is steering.
    let movement = action_state.clamped_axis_pair(&Action::Move);
    input.throttle = movement.y;
    input.steer = movement.x;
    // Space is jump on foot and handbrake in a car; context decides which.
    input.handbrake = action_state.pressed(&Action::Handbrake);
}

/// Puts the player back on their feet if the car they were driving stops
/// existing.
///
/// A wrecked vehicle despawns, but the driver keeps their `Driving` link and
/// stays hidden with their body disabled — invisible, uncontrollable, and
/// frozen wherever the wreck happened to be, often inside a building.
fn eject_from_wrecked_vehicle(
    mut commands: Commands,
    spatial: SpatialQuery,
    vehicles: Query<(), With<Vehicle>>,
    players: Query<(Entity, &Driving, &Transform), With<Player>>,
) {
    for (player, driving, transform) in &players {
        if vehicles.get(driving.0).is_ok() {
            continue;
        }
        // `carry_driver` kept this pinned to the car, so it is the wreck's last
        // pose — which is the frame the escape offsets are measured in.
        let drop_at = clear_spot(&spatial, transform, [player, driving.0]);

        commands
            .entity(player)
            .remove::<Driving>()
            .remove::<RigidBodyDisabled>()
            .remove::<ColliderDisabled>()
            .insert(Visibility::Visible)
            .insert(Transform::from_translation(drop_at))
            .insert((LinearVelocity::ZERO, AngularVelocity::ZERO));
        info!("bailed out of a wrecked vehicle at {drop_at:?}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::world::buildings::SIDEWALK_HEIGHT;

    /// Height of the capsule's lowest point when its centre is at `y`.
    fn feet(y: f32) -> f32 {
        y - (CAPSULE_LENGTH * 0.5 + CAPSULE_RADIUS)
    }

    #[test]
    fn every_exit_offset_starts_above_the_kerb() {
        // If a candidate spot has the capsule already inside the pavement, the
        // intersection test rejects it before it is ever tried and the player
        // ends up taking the roof — or the straight-up fallback — every time.
        for offset in EXIT_OFFSETS {
            assert!(
                feet(offset.y) > SIDEWALK_HEIGHT,
                "exit offset {offset:?} starts {:.2}m inside the pavement",
                SIDEWALK_HEIGHT - feet(offset.y)
            );
        }
    }

    #[test]
    fn the_first_choice_is_the_drivers_door() {
        let first = EXIT_OFFSETS[0];
        assert!(first.x < 0.0, "should step out to the left");
        assert_eq!(first.z, 0.0, "and level with the seat, not fore or aft");
    }

    #[test]
    fn the_offsets_cover_both_sides_and_both_ends() {
        let side = |sign: f32| EXIT_OFFSETS.iter().any(|o| o.x * sign > 1.0);
        assert!(
            side(-1.0) && side(1.0),
            "boxed in on one side, the other side has to be an option"
        );
        assert!(
            EXIT_OFFSETS.iter().any(|o| o.z > 1.0) && EXIT_OFFSETS.iter().any(|o| o.z < -1.0),
            "and so do fore and aft"
        );
    }
}
