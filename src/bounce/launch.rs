//! Being hit by a car, and flying beautifully for it.
//!
//! A vehicle that drives into somebody on foot was once nothing but a contact
//! for the solver to sort out, and with a ton and a half arriving at 20 m/s
//! against a capsule that is actively held upright, the solver lost. The
//! capsule ended up *inside* the bodywork, where it was carried along by a car
//! it could not leave, walking did nothing because there was nowhere to walk,
//! and the follow camera's wall cast hit panels 40cm away and put the view
//! inside the car. Being run over meant riding around in one.
//!
//! So a car hitting a person is an event rather than a collision to be
//! resolved: it throws them clear and takes the character controller off them
//! for a moment, so the throw actually lands instead of being braked away by
//! the walk basis in two frames. Nobody is hurt — there is no health in this
//! city — they are simply launched, and they bounce. This applies to
//! *everyone* on foot: the player and every flummi on the pavement alike,
//! because a crowd that scatters through the air when a car arrives is the
//! scene the whole game is for, and for a while only the player got to fly.
//!
//! Behind that sits [`unwedge_from_vehicles`], which is what makes the bug
//! impossible rather than merely unlikely. However the solver or a spawn
//! contrives to put a person inside a car, they are put back beside it on the
//! next frame.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::controller::{Bouncer, Launched};
use crate::core::schedule::GameSet;
use crate::player::interact::{Driving, clear_spot};
use crate::player::on_foot::{CAPSULE_RADIUS, Player};
use crate::vehicle::spawn::Vehicle;
use crate::vehicle::spec::VehicleSpec;

/// Closing speed below which a car is a shove rather than an accident. Walking
/// pace: nudging someone aside in a car park must not launch them across the
/// street.
const IMPACT_SPEED: f32 = 2.2;
/// Fraction of the closing speed handed to the victim as knockback.
const KNOCKBACK: f32 = 0.8;
/// Upward part of the throw, in m/s. Enough to clear a bumper rather than be
/// swept under one.
const LAUNCH_UP: f32 = 3.4;
/// How long the victim stays off their feet. Also the window in which the same
/// car cannot hit them again — without it, a car resting against somebody
/// launches them sixty times a second.
const DOWN_TIME: f32 = 1.35;

/// On someone a car has just knocked off their feet.
///
/// While this is on them the bounce controller leaves them alone and their
/// rotation is unlocked, so they are a plain elastic rigid body: the throw
/// carries, and they cartwheel off the nearest wall instead of landing
/// mid-stride and carrying on as though nothing had happened.
#[derive(Component, Debug)]
pub struct KnockedDown {
    pub left: f32,
}

/// Throws somebody off their feet.
///
/// The one way anybody in this city is ever knocked over, whether it was a car
/// that did it or a neighbour with a score to settle. Kept in one place because
/// the three components have to come off and go back on together: with
/// `LockedAxes` left on, a launched flummi slides along the pavement bolt
/// upright, and without [`KnockedDown`] nothing ever stands it back up.
pub fn launch(commands: &mut Commands, victim: Entity, velocity: &mut LinearVelocity, throw: Vec3) {
    velocity.0 = throw;
    commands
        .entity(victim)
        .insert((KnockedDown { left: DOWN_TIME }, Launched))
        .remove::<LockedAxes>();
}

/// Upward part of any throw, in m/s. Exposed because a ram wants the same arc
/// a bumper gives: over the top rather than along the ground.
pub const THROW_UP: f32 = LAUNCH_UP;

/// How hard a car actually lands a blow on somebody on foot, in m/s. Both
/// arguments are speeds along the line from the car to the victim: how fast the
/// car is bearing down on them, and how fast they are already going the same
/// way.
///
/// Two numbers rather than one closing speed, because the two halves are not
/// symmetric. A parked car never lands a blow however hard somebody runs into
/// it — it has to be *delivered* — while running with the car that is about to
/// hit you genuinely does take the sting out of it.
pub fn wallop_force(driven_at: f32, fleeing: f32) -> f32 {
    if driven_at < IMPACT_SPEED {
        return 0.0;
    }
    (driven_at - fleeing - IMPACT_SPEED).max(0.0)
}

pub struct LaunchPlugin;

impl Plugin for LaunchPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                run_over_anybody,
                recover_from_knockdown,
                unwedge_from_vehicles,
            )
                .chain()
                .in_set(GameSet::Simulation),
        );
    }
}

/// Whether a point in a car's own frame is close enough to its bodywork for a
/// moving bumper to have arrived.
///
/// Pure box arithmetic rather than a spatial query, because it runs for every
/// pedestrian against every moving car every frame and the answer is almost
/// always no. The margin sits slightly proud of the bodywork, so the hit
/// registers as the bumper arrives rather than once it is already through the
/// ribs; the vertical reach is generous because the victim's origin is at
/// their navel and the car's at its floorpan, and a bonnet arriving at a
/// kneecap is still being run over.
pub fn brushes(local: Vec3, half_extents: Vec3) -> bool {
    let reach = CAPSULE_RADIUS + 0.12;
    local.x.abs() < half_extents.x + reach
        && local.z.abs() < half_extents.z + reach
        && local.y.abs() < half_extents.y + 1.2
}

/// Turns a car arriving at anybody on foot into a throw.
///
/// Anybody: the same rule for the player and the crowd, in one system, so the
/// comedy is symmetric — a taxi that scatters six flummis across a junction
/// obeys exactly the physics that launched you yesterday.
fn run_over_anybody(
    mut commands: Commands,
    vehicles: Query<(&Transform, &LinearVelocity, &VehicleSpec), With<Vehicle>>,
    mut victims: Query<
        (Entity, &Transform, &mut LinearVelocity),
        (
            With<Bouncer>,
            Without<Vehicle>,
            Without<Driving>,
            Without<KnockedDown>,
        ),
    >,
) {
    // Almost every car in the city is parked, and a parked car never lands a
    // blow (see `wallop_force`), so only the handful actually moving are worth
    // testing the crowd against.
    let moving: Vec<_> = vehicles
        .iter()
        .filter(|(_, velocity, _)| velocity.length_squared() > IMPACT_SPEED * IMPACT_SPEED)
        .collect();
    if moving.is_empty() {
        return;
    }

    for (victim, transform, mut velocity) in &mut victims {
        for (car, car_velocity, spec) in &moving {
            let local = car.rotation.inverse() * (transform.translation - car.translation);
            if !brushes(local, spec.half_extents) {
                continue;
            }

            // Measured along the line from the car to the victim, so a car
            // sliding past someone's shoulder is a scrape and only one driving
            // *at* them counts. Flattened: a car coming down off a kerb is not
            // running anybody over from above.
            let offset = (transform.translation - car.translation).with_y(0.0);
            // Dead centre of the car there is no such line; fall back to the
            // way it is travelling, and if it is not travelling either then
            // this is a wedge rather than an impact and
            // `unwedge_from_vehicles` has the player's case.
            let Ok(away) = Dir3::new(offset).or_else(|_| Dir3::new(car_velocity.0.with_y(0.0)))
            else {
                continue;
            };
            let driven_at = car_velocity.0.dot(*away);
            let fleeing = velocity.0.dot(*away);
            let wallop = wallop_force(driven_at, fleeing);
            if wallop <= 0.0 {
                continue;
            }
            let closing = driven_at - fleeing;

            // Over the wing rather than under the wheels.
            launch(
                &mut commands,
                victim,
                &mut velocity,
                *away * (closing * KNOCKBACK) + Vec3::Y * LAUNCH_UP,
            );

            info!("run over at {closing:.1} m/s, launched with {wallop:.1} m/s of wallop");
            // One car per victim. Being hit by two at once is still one
            // accident.
            break;
        }
    }
}

/// Hands control back once the victim has stopped rolling, and stands them up.
fn recover_from_knockdown(
    mut commands: Commands,
    time: Res<Time>,
    mut victims: Query<(Entity, &mut KnockedDown)>,
) {
    for (entity, mut knocked) in &mut victims {
        knocked.left -= time.delta_secs();
        if knocked.left > 0.0 {
            continue;
        }
        commands
            .entity(entity)
            .remove::<KnockedDown>()
            .remove::<Launched>()
            // Back on their feet, in both senses: a flummi that stayed free to
            // rotate would spend the rest of the game lying on its face.
            .insert(LockedAxes::ROTATION_LOCKED);
    }
}

/// The net under all of it: nobody stays inside a car.
///
/// This is what makes riding around inside a police cruiser impossible rather
/// than unlikely. It does not care how the player got in there — solver
/// penetration, a wreck landing on them, a car spawning where they stand — it
/// only cares that they are, and puts them back beside it.
fn unwedge_from_vehicles(
    mut commands: Commands,
    spatial: SpatialQuery,
    // The name is only for the log, so it is optional: a vehicle that somehow
    // has none must still not be allowed to keep the player.
    vehicles: Query<(Option<&Name>, &Transform), (With<Vehicle>, Without<Player>)>,
    players: Query<(Entity, &Transform), (With<Player>, Without<Driving>)>,
) {
    let Ok((player, transform)) = players.single() else {
        return;
    };

    // A ball at the navel rather than the whole capsule. Squeezing past a
    // parked car puts a shoulder through its flank and must not teleport
    // anybody; being *inside* the bodywork always puts this much of the body
    // in it.
    let core = Collider::sphere(CAPSULE_RADIUS * 0.5);
    let filter = SpatialQueryFilter::from_excluded_entities([player]);
    let inside = spatial.shape_intersections(&core, transform.translation, Quat::IDENTITY, &filter);

    for entity in inside {
        let Ok((name, car)) = vehicles.get(entity) else {
            continue;
        };
        let out = clear_spot(&spatial, car, [player, entity]);
        commands
            .entity(player)
            .insert(Transform::from_translation(out))
            .insert((LinearVelocity::ZERO, AngularVelocity::ZERO));
        let name = name.map(Name::as_str).unwrap_or("vehicle");
        warn!("pulled the player out of a {name} at {out:?}");
        return;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::on_foot::CAPSULE_LENGTH;
    use bevy::asset::AssetPlugin;

    #[test]
    fn a_gentle_nudge_launches_nobody() {
        assert_eq!(
            wallop_force(IMPACT_SPEED * 0.5, 0.0),
            0.0,
            "rolling into somebody at walking pace is not an accident"
        );
        assert_eq!(
            wallop_force(-14.0, 0.0),
            0.0,
            "and neither is a car driving away from them"
        );
    }

    #[test]
    fn walking_into_a_parked_car_is_the_pedestrians_problem() {
        // The car is stationary and the victim is sprinting straight at it,
        // which is a closing speed of 7.6 m/s and must still be worth nothing:
        // the blow has to be delivered by the car.
        assert_eq!(wallop_force(0.0, -7.6), 0.0);
    }

    #[test]
    fn running_with_the_car_takes_the_sting_out() {
        let stood_still = wallop_force(12.0, 0.0);
        let running = wallop_force(12.0, 6.0);
        assert!(
            running < stood_still,
            "being clipped while already moving that way should throw you less far"
        );
        assert!(running > 0.0, "but it should still throw you");
    }

    #[test]
    fn the_bumper_reaches_just_past_the_bodywork_and_no_further() {
        let half = Vec3::new(0.95, 0.7, 2.2);
        assert!(
            brushes(Vec3::new(1.0, 0.0, 0.0), half),
            "a shoulder against the flank is a hit"
        );
        assert!(
            brushes(Vec3::new(0.0, 0.9, -2.3), half),
            "a bonnet at the kneecap is still being run over"
        );
        assert!(
            !brushes(Vec3::new(2.0, 0.0, 0.0), half),
            "a metre of daylight is not a collision"
        );
        assert!(
            !brushes(Vec3::new(0.0, 3.0, 0.0), half),
            "somebody bouncing high over the roof is not under the wheels"
        );
    }

    #[test]
    fn the_throw_climbs_with_the_speed_of_the_car() {
        let slow = wallop_force(8.0, 0.0);
        let fast = wallop_force(20.0, 0.0);
        assert!(slow > 0.0, "a car at 8 m/s picks somebody up");
        assert!(
            fast > slow * 2.0,
            "and one at 20 m/s sends them a great deal further"
        );
    }

    /// Physics without a window, so "can the player end up inside a car" is a
    /// test rather than something we notice while playing.
    fn harness() -> (App, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            PhysicsPlugins::default(),
        ));
        // Avian's collider cache reads `AssetEvent<Mesh>`; `AssetPlugin` alone
        // does not register the Mesh asset type outside a render app.
        app.init_asset::<Mesh>();
        app.add_systems(Update, unwedge_from_vehicles);

        // A car-sized box, and the player standing in the middle of it.
        let car = app
            .world_mut()
            .spawn((
                Name::new("Test Car"),
                Vehicle,
                RigidBody::Static,
                Collider::cuboid(1.9, 1.4, 4.4),
                Transform::from_xyz(0.0, 0.9, 0.0),
            ))
            .id();
        let player = app
            .world_mut()
            .spawn((
                Player,
                RigidBody::Dynamic,
                Collider::capsule(CAPSULE_RADIUS, CAPSULE_LENGTH),
                LockedAxes::ROTATION_LOCKED,
                Transform::from_xyz(0.0, 0.9, 0.0),
            ))
            .id();

        // `run()` does this for us; a bare `update()` loop does not. Avian
        // registers its diagnostics resources in `Plugin::finish`, and its
        // systems hard-require them.
        app.finish();
        app.cleanup();

        (app, player, car)
    }

    #[test]
    fn a_player_inside_a_car_is_put_back_beside_it() {
        let (mut app, player, _) = harness();
        // Two ticks: one to get the colliders into the broad phase, one for the
        // system to see the overlap and act on it.
        app.update();
        app.update();

        let at = app.world().get::<Transform>(player).unwrap().translation;
        assert!(
            at.xz().length() > 1.5,
            "still inside the bodywork at {at:?}"
        );
    }

    #[test]
    fn standing_beside_a_car_is_left_alone() {
        let (mut app, player, _) = harness();
        // Off the driver's flank, clear of the box by a comfortable margin.
        let beside = Vec3::new(-3.0, 0.9, 0.0);
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = beside;
        app.update();
        app.update();

        let at = app.world().get::<Transform>(player).unwrap().translation;
        assert!(
            (at.xz() - beside.xz()).length() < 0.5,
            "somebody walking past a parked car was teleported from {beside:?} to {at:?}"
        );
    }
}
