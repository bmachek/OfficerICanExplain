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
use crate::crime::events::{CrimeKind, CrimeReported};

use crate::player::input::Action;
use crate::player::on_foot::Player;
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
/// Where the player is put down when they get out.
const EXIT_OFFSET: Vec3 = Vec3::new(-1.9, 0.6, 0.0);

pub struct InteractPlugin;

impl Plugin for InteractPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                enter_or_exit_vehicle,
                report_vehicle_theft,
                eject_from_wrecked_vehicle,
                carry_driver,
                drive_from_input,
            )
                .chain()
                .in_set(GameSet::Simulation),
        );
    }
}

fn enter_or_exit_vehicle(
    mut commands: Commands,
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
                .map(|t| t.transform_point(EXIT_OFFSET))
                .unwrap_or(player_transform.translation);

            commands
                .entity(player)
                .remove::<Driving>()
                .remove::<RigidBodyDisabled>()
                .remove::<ColliderDisabled>()
                .insert(Visibility::Visible)
                .insert(Transform::from_translation(drop_at));

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

/// Reports the theft of any car that has just acquired a driver.
///
/// Keyed on the world changing rather than on the button press: a stolen car is
/// a stolen car however the player came to be sitting in it, and hanging the
/// crime off the input handler meant any other entry path silently skipped it.
fn report_vehicle_theft(
    mut crimes: MessageWriter<CrimeReported>,
    stolen: Query<&Transform, Added<DrivenBy>>,
) {
    for transform in &stolen {
        debug!("theft reported at {:?}", transform.translation);
        crimes.write(CrimeReported {
            kind: CrimeKind::VehicleTheft,
            position: transform.translation,
        });
    }
}

/// Puts the player back on their feet if the car they were driving stops
/// existing.
///
/// A wrecked vehicle despawns, but the driver keeps their `Driving` link and
/// stays hidden with their body disabled — invisible, uncontrollable, and
/// frozen wherever the wreck happened to be, often inside a building.
fn eject_from_wrecked_vehicle(
    mut commands: Commands,
    vehicles: Query<(), With<Vehicle>>,
    players: Query<(Entity, &Driving, &Transform), With<Player>>,
) {
    for (player, driving, transform) in &players {
        if vehicles.get(driving.0).is_ok() {
            continue;
        }
        commands
            .entity(player)
            .remove::<Driving>()
            .remove::<RigidBodyDisabled>()
            .remove::<ColliderDisabled>()
            .insert(Visibility::Visible)
            // Lift clear of the wreckage so they do not start inside it.
            .insert(Transform::from_translation(
                transform.translation + Vec3::Y * 1.5,
            ));
        info!("bailed out of a wrecked vehicle");
    }
}
