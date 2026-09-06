//! Crashes, and what a crash is for.
//!
//! This file used to be `damage.rs`: health, dents, scuffed paint, engine
//! smoke and a terminal explosion. All of it went with the crime game. In a
//! city made of rubber a crash costs nobody anything — no health, no panels,
//! no fail state — so what is left of a collision is the *reaction*: both
//! parties are flung apart harder than physics would fling them, because a
//! fender-bender where everyone leaves the scene backwards, airborne and
//! spinning is the joke this game is built around.
//!
//! Impacts are still read off sudden changes in velocity rather than off
//! collision events. A crash is defined by how hard the car stops, which is
//! exactly what a velocity delta measures — and it catches every way a car can
//! be stopped (walls, other cars, landing badly from a jump) through one code
//! path, instead of needing a separate rule per collision pair. It also means
//! a two-car crash fires once per car, so both fly, each away from the other.
//!
//! Braking is nowhere near the threshold: a hard stop is about 1g, or 0.15
//! m/s of velocity change per tick, while hitting a wall at 70km/h sheds ten
//! times that in a single tick.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::controller::VehicleState;
use super::spawn::{ActiveVehicle, Vehicle};
use crate::core::config::BounceConfig;
use crate::core::config::GameConfig;
use crate::player::interact::DrivenBy;

/// Velocity change in one tick below which a knock is the solver's business
/// and not ours.
const IMPACT_THRESHOLD: f32 = 2.5;
/// Severity beyond which the fling stops growing. The rebound is proportional
/// to how hard the car stopped, and a car dropped off a tower stops very hard
/// indeed; without a cap that is a car in orbit, which is past the joke.
const FLING_CAP: f32 = 16.0;

/// Last tick's velocity, used to spot impacts.
#[derive(Component, Default)]
pub struct PreviousVelocity(pub Vec3);

/// Fired for every impact hard enough to be worth reacting to — the crash
/// sound, the indignant honk, and the fling itself all read this.
#[derive(Message, Debug, Clone, Copy)]
pub struct VehicleImpact {
    pub vehicle: Entity,
    pub position: Vec3,
    /// Where the blow arrived from, in the car's own frame. Read off the
    /// velocity change: whichever way the car was shoved, the thing it hit was
    /// on the other side.
    pub from: Vec3,
    /// Velocity lost in the impact, in m/s. A scrape is a couple; hitting a
    /// wall at speed is twenty.
    pub severity: f32,
}

pub fn spot_impacts(
    mut impacts: MessageWriter<VehicleImpact>,
    mut vehicles: Query<
        (
            Entity,
            &LinearVelocity,
            &mut PreviousVelocity,
            &VehicleState,
            &Transform,
        ),
        (With<Vehicle>, With<ActiveVehicle>),
    >,
) {
    for (entity, velocity, mut previous, state, transform) in &mut vehicles {
        let change = velocity.0 - previous.0;
        let delta = change.length();
        previous.0 = velocity.0;

        // Ignore the first tick after activation, when previous velocity is
        // meaningless, and airborne landings on all four wheels.
        if delta > IMPACT_THRESHOLD && state.grounded_wheels() > 0 {
            impacts.write(VehicleImpact {
                vehicle: entity,
                position: transform.translation,
                from: transform.rotation.inverse() * (-change / delta),
                severity: delta - IMPACT_THRESHOLD,
            });
        }
    }
}

/// The comedy rebound, in the car's own frame: what a blow from `from` at
/// `severity` adds to the car's velocity and spin.
///
/// Pure, so the choreography can be argued about in a test. The linear part is
/// straight back the way the blow came plus a hop; the spin is read off where
/// the blow landed — a blow on the flank spins the car about its yaw axis like
/// a struck top, a head-on blow tips it over its nose instead, because a car
/// that leaves a head-on crash somersaulting backwards is funnier than one
/// that leaves it flat.
pub fn fling(from: Vec3, severity: f32, tune: &BounceConfig) -> (Vec3, Vec3) {
    let oomph = severity.min(FLING_CAP);
    let shove = -from * (oomph * tune.crash_rebound) + Vec3::Y * (oomph * tune.crash_pop);
    // `from.cross(Y)` is horizontal and perpendicular to the blow: for a blow
    // from the flank it points along the car, which as a rotation axis is
    // roll — so it is swapped about by taking the yaw for the flank and the
    // pitch for the nose directly from the blow's components instead.
    let spin = Vec3::new(
        // Pitch: nose blows tip the car backwards over its rear axle.
        -from.z * oomph * tune.crash_spin,
        // Yaw: flank blows spin it like a struck top.
        from.x * oomph * tune.crash_spin * 2.0,
        // Roll: a pinch of it off the flank, so a side swipe rocks the car
        // onto two wheels rather than rotating it flat like a record.
        from.x * oomph * tune.crash_spin * 0.5,
    );
    (shove, spin)
}

/// Applies [`fling`] to every car that just crashed.
///
/// Runs after [`spot_impacts`] and writes the flung velocity back into
/// `PreviousVelocity`, which is load-bearing: the fling *is* a sudden change
/// in velocity, and left visible to next frame's delta it would read as
/// another impact, which would fling again, forever, which is a car that
/// vibrates itself over the skyline the first time it touches a bollard.
pub fn fling_apart(
    config: Res<GameConfig>,
    mut impacts: MessageReader<VehicleImpact>,
    mut vehicles: Query<
        (
            &Transform,
            &mut LinearVelocity,
            &mut AngularVelocity,
            &mut PreviousVelocity,
        ),
        With<Vehicle>,
    >,
) {
    for impact in impacts.read() {
        let Ok((transform, mut velocity, mut spin, mut previous)) =
            vehicles.get_mut(impact.vehicle)
        else {
            continue;
        };
        let (shove, twirl) = fling(impact.from, impact.severity, &config.bounce);
        velocity.0 += transform.rotation * shove;
        spin.0 += transform.rotation * twirl;
        previous.0 = velocity.0;
    }
}

/// Seconds the player's car may stay unsettled — tumbling, airborne, or on
/// its roof — before it is stood back up for them.
///
/// The fling is the joke and the wait afterwards is the bill; past a couple
/// of seconds the bill exceeds the joke. Long enough that a proper somersault
/// plays out in full, short enough that nobody reaches for the reset key this
/// game does not have.
const RECOVER_AFTER: f32 = 2.4;

/// How settled a car has to be to count as driveable again: this many wheels
/// down, roughly upright, and not spinning like a coin.
fn is_settled(state: &VehicleState, transform: &Transform, spin: Vec3) -> bool {
    state.grounded_wheels() >= 2 && transform.up().y > 0.6 && spin.length() < 3.0
}

/// Seconds the driven car has spent unsettled.
#[derive(Component, Default)]
pub struct Unsettled(pub f32);

/// Stands the player's car back on its wheels once a crash has had its fun.
///
/// Only the *driven* car: an empty traffic car on its roof is scenery, and
/// scenery is allowed to lie there being funny. The reset keeps position and
/// heading — it is the tumble that is cancelled, not the journey — and lifts
/// the body slightly so it settles onto its suspension instead of clipping
/// the road it fell on.
pub fn recover_driven_vehicle(
    time: Res<Time>,
    mut vehicles: Query<
        (
            &mut Transform,
            &mut LinearVelocity,
            &mut AngularVelocity,
            &mut PreviousVelocity,
            &VehicleState,
            &mut Unsettled,
        ),
        (With<Vehicle>, With<DrivenBy>),
    >,
) {
    for (mut transform, mut velocity, mut spin, mut previous, state, mut unsettled) in &mut vehicles
    {
        if is_settled(state, &transform, spin.0) {
            unsettled.0 = 0.0;
            continue;
        }
        unsettled.0 += time.delta_secs();
        if unsettled.0 < RECOVER_AFTER {
            continue;
        }

        let (yaw, _, _) = transform.rotation.to_euler(EulerRot::YXZ);
        transform.rotation = Quat::from_rotation_y(yaw);
        transform.translation.y += 0.6;
        velocity.0 = Vec3::ZERO;
        spin.0 = Vec3::ZERO;
        // The stop must not read as another crash next frame.
        previous.0 = Vec3::ZERO;
        unsettled.0 = 0.0;
        info!("stood the car back up after a crash");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::GameConfig;

    #[test]
    fn a_hard_stop_is_not_a_crash() {
        // 1g of braking at 64Hz is about 0.15 m/s per tick. If the threshold
        // ever drops near that, braking would launch the car.
        let braking_delta_per_tick = 9.81 / 64.0;
        assert!(
            IMPACT_THRESHOLD > braking_delta_per_tick * 10.0,
            "impact threshold is close enough to braking to trigger on it"
        );
    }

    #[test]
    fn a_car_on_its_wheels_is_settled_and_a_car_on_its_roof_is_not() {
        let mut state = VehicleState::default();
        for wheel in &mut state.wheels {
            wheel.grounded = true;
        }
        let upright = Transform::IDENTITY;
        assert!(is_settled(&state, &upright, Vec3::ZERO));

        let rolled = Transform::from_rotation(Quat::from_rotation_z(std::f32::consts::PI));
        assert!(
            !is_settled(&state, &rolled, Vec3::ZERO),
            "a car on its roof is not driveable"
        );
        assert!(
            !is_settled(&state, &upright, Vec3::new(0.0, 8.0, 0.0)),
            "and neither is one spinning like a coin"
        );

        let mut airborne = VehicleState::default();
        for wheel in &mut airborne.wheels {
            wheel.grounded = false;
        }
        assert!(!is_settled(&airborne, &upright, Vec3::ZERO));
    }

    #[test]
    fn a_head_on_crash_throws_the_car_up_and_backwards() {
        let tune = GameConfig::default().bounce;
        // The blow arrives from dead ahead: -Z, the way the car faces.
        let (shove, spin) = fling(Vec3::NEG_Z, 10.0, &tune);
        assert!(shove.z > 1.0, "not thrown backwards: {shove:?}");
        assert!(
            shove.y > 0.5,
            "a crash that keeps its wheels down: {shove:?}"
        );
        assert!(
            spin.x > 0.0 && spin.y.abs() < 1e-4,
            "a head-on should tip, not pirouette: {spin:?}"
        );
    }

    #[test]
    fn a_side_swipe_spins_the_car_like_a_top() {
        let tune = GameConfig::default().bounce;
        let (shove, spin) = fling(Vec3::X, 10.0, &tune);
        assert!(shove.x < -1.0, "not shoved away from the blow: {shove:?}");
        assert!(
            spin.y.abs() > spin.x.abs(),
            "a flank blow should mostly yaw: {spin:?}"
        );
    }

    #[test]
    fn harder_crashes_fling_further_until_the_cap() {
        let tune = GameConfig::default().bounce;
        let soft = fling(Vec3::NEG_Z, 4.0, &tune).0.length();
        let hard = fling(Vec3::NEG_Z, 12.0, &tune).0.length();
        let absurd = fling(Vec3::NEG_Z, 400.0, &tune).0.length();
        assert!(hard > soft * 2.0, "the fling should scale with the crash");
        assert!(
            absurd <= fling(Vec3::NEG_Z, FLING_CAP, &tune).0.length() + 1e-4,
            "a fall off a tower must not put the car in orbit"
        );
    }

    #[test]
    fn the_fling_never_outruns_the_crash_that_caused_it() {
        // The rebound has to stay under the speed the car arrived with, or
        // crashing into a wall becomes a way to *gain* speed and the traffic
        // accelerates itself into a boiling pot. The severity is already the
        // speed lost, so the check is that the multipliers sum under one.
        let tune = GameConfig::default().bounce;
        assert!(
            tune.crash_rebound + tune.crash_pop < 1.0,
            "rebound {} + pop {} returns more speed than the crash took",
            tune.crash_rebound,
            tune.crash_pop
        );
    }
}
