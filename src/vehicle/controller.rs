//! Raycast suspension and arcade tyre model.
//!
//! Each wheel is a downward raycast from its suspension mount. Where it hits,
//! the wheel pushes the body up with a spring-damper, and applies tyre forces
//! in the contact plane: a lateral force that cancels sideways slip, and a
//! longitudinal one for drive and braking. Both are clamped by a friction
//! budget proportional to that wheel's load.
//!
//! That clamp is the whole design. Grip is not a boolean and drift is not a
//! special mode: exceed the friction available at a wheel and it simply stops
//! cancelling slip, so the back steps out on its own. Pulling the handbrake
//! just shrinks the rear budget.
//!
//! Runs in `FixedUpdate`, which is before Avian's step in `FixedPostUpdate`;
//! Avian clears accumulated forces after each step, so they must be re-applied
//! every tick.

use avian3d::dynamics::rigid_body::forces::{
    ForcesItem, ReadRigidBodyForces, WriteRigidBodyForces,
};
use avian3d::prelude::*;
use bevy::prelude::*;

use super::spec::{VehicleSpec, WHEEL_COUNT};

/// Normalised driver input. Written by the player, or by AI in M4/M5.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct VehicleInput {
    /// -1 (reverse / brake) to 1 (throttle).
    pub throttle: f32,
    /// -1 (left) to 1 (right).
    pub steer: f32,
    pub handbrake: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct WheelState {
    pub grounded: bool,
    /// Metres of suspension travel used, 0 at full droop.
    pub compression: f32,
    /// Anchor-to-contact distance; drives where the wheel mesh sits.
    pub ray_length: f32,
    /// Normal force through this wheel, in newtons.
    pub load: f32,
}

#[derive(Component, Debug, Default)]
pub struct VehicleState {
    pub wheels: [WheelState; WHEEL_COUNT],
    /// Current road-wheel angle in radians, smoothed towards the input.
    pub steer_angle: f32,
    /// Signed speed along the vehicle's forward axis, m/s.
    pub forward_speed: f32,
    /// Accumulated wheel rotation, for spinning the wheel meshes.
    pub wheel_spin: f32,
    /// Sideways speed the tyres failed to cancel this tick, in m/s.
    ///
    /// Zero whenever the car has grip: the lateral force is exactly the one
    /// that would kill the slip outright. It only becomes non-zero once a wheel
    /// asks for more than its friction budget, which is precisely the moment
    /// the tyre is sliding rather than rolling — so this is both the physical
    /// residual and the cue the audio uses to decide a tyre is squealing.
    pub slip: f32,
}

impl VehicleState {
    pub fn grounded_wheels(&self) -> usize {
        self.wheels.iter().filter(|w| w.grounded).count()
    }

    pub fn speed_kph(&self) -> f32 {
        self.forward_speed.abs() * 3.6
    }
}

pub fn drive_vehicles(
    time: Res<Time>,
    spatial: SpatialQuery,
    mut vehicles: Query<
        (
            Entity,
            &VehicleSpec,
            &VehicleInput,
            &mut VehicleState,
            &Transform,
            Forces,
        ),
        With<crate::vehicle::spawn::ActiveVehicle>,
    >,
) {
    let dt = time.delta_secs();
    if dt <= f32::EPSILON {
        return;
    }

    for (entity, spec, input, mut state, transform, mut forces) in &mut vehicles {
        let up = *transform.up();
        let forward = *transform.forward();

        let linear = forces.linear_velocity();
        let speed = linear.length();
        let center_of_mass = transform.transform_point(spec.center_of_mass);
        state.forward_speed = linear.dot(forward);

        // Steering tightens with *road* speed, not velocity magnitude: the
        // magnitude includes the vertical bob of deliberately soft springs and
        // the sideways component of a slide, so using it made the lock wobble
        // over bumps and shrink further mid-slide — at exactly the moment the
        // driver is trying to steer out of it.
        let road_speed = state.forward_speed.abs();
        update_steering(spec, input, &mut state, road_speed, dt);

        let anchors = spec.wheel_anchors();
        let max_ray = spec.max_ray_length();
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
        let Ok(down) = Dir3::new(-up) else { continue };

        // --- Pass 1: suspension. Loads from here become the friction budget. ---
        let mut contacts = [Vec3::ZERO; WHEEL_COUNT];
        for i in 0..WHEEL_COUNT {
            let anchor = transform.transform_point(anchors[i]);
            let wheel = &mut state.wheels[i];

            let Some(hit) = spatial.cast_ray(anchor, down, max_ray, true, &filter) else {
                *wheel = WheelState {
                    ray_length: max_ray,
                    ..default()
                };
                continue;
            };

            let compression = max_ray - hit.distance;
            let vertical_speed = forces.velocity_at_point(anchor).dot(up);
            // Damper opposes motion; clamp at zero so suspension can only push,
            // never suck the car down onto the road.
            let load =
                (compression * spec.spring_strength - vertical_speed * spec.damping).max(0.0);

            *wheel = WheelState {
                grounded: true,
                compression,
                ray_length: hit.distance,
                load,
            };
            contacts[i] = anchor + -up * hit.distance;
            forces.apply_force_at_point(up * load, anchor);
        }

        apply_anti_roll(spec, &state, transform, &anchors, &mut forces, up);

        if state.grounded_wheels() == 0 {
            // Airborne: no tyres, no drag from the road. Let it fly.
            state.slip = 0.0;
            continue;
        }

        // --- Pass 2: tyre forces in the contact plane. ---
        // Copied out so the wheel slice can be iterated while forces are applied.
        let steer_angle = state.steer_angle;
        let forward_speed = state.forward_speed;
        let mass_share = spec.wheel_mass_share();

        let mut slip: f32 = 0.0;
        for (index, (wheel, &contact)) in state.wheels.iter().zip(contacts.iter()).enumerate() {
            if !wheel.grounded {
                continue;
            }
            let budget = friction_budget(spec, input, index, wheel.load);

            let (wheel_forward, wheel_right) = wheel_axes(steer_angle, index, forward, up);
            let velocity = forces.velocity_at_point(contact);
            let lateral_speed = velocity.dot(wheel_right);

            // Force that would cancel sideways slip outright this tick, then
            // clipped to what the tyre can actually hold.
            let wanted = -lateral_speed * mass_share / dt;
            let lateral = wanted.clamp(-budget, budget);
            // Whatever the clamp threw away, back in units of speed: how fast
            // this wheel is still sliding after the tyre has done its best.
            slip = slip.max((wanted.abs() - budget).max(0.0) * dt / mass_share);
            let longitudinal = longitudinal_force(
                spec,
                input,
                forward_speed,
                index,
                velocity.dot(wheel_forward),
            )
            .clamp(-budget, budget);

            // Longitudinal force acts at the contact patch, so braking still
            // pitches the nose down — that weight transfer reads as weight.
            forces.apply_force_at_point(wheel_forward * longitudinal, contact);
            // Lateral force is raised towards the centre of mass to shed most
            // of the roll moment. See `VehicleSpec::roll_couple`.
            let roll_axis_height = center_of_mass
                .y
                .lerp(contact.y, spec.roll_couple.clamp(0.0, 1.0));
            let lateral_at = Vec3::new(contact.x, roll_axis_height, contact.z);
            forces.apply_force_at_point(wheel_right * lateral, lateral_at);
        }

        // Body drag and downforce act through the centre of mass, so neither
        // introduces a torque.
        if speed > 0.05 {
            forces.apply_force(-linear / speed * spec.drag * speed * speed);
        }
        forces.apply_force(-up * spec.downforce * speed * speed);

        state.wheel_spin += state.forward_speed / spec.wheel_radius * dt;
        state.slip = slip;
    }
}

fn update_steering(
    spec: &VehicleSpec,
    input: &VehicleInput,
    state: &mut VehicleState,
    speed: f32,
    dt: f32,
) {
    // Steering lock tightens with speed. Without this, a flick of the stick at
    // 150km/h spins the car, which feels broken rather than difficult.
    let speed_ratio = (speed / spec.max_speed).clamp(0.0, 1.0);
    let lock = spec.max_steer * (1.0 - speed_ratio * (1.0 - spec.high_speed_steer));
    // Positive input is "right", which is a negative yaw about +Y.
    let target = -input.steer.clamp(-1.0, 1.0) * lock;

    // Asymmetric on purpose. The first cure for "laggy" was raising the rate
    // wholesale, which overshot into twitchy: digital keys slamming to full
    // lock at 13/s put every car sideways. What actually reads as direct is
    // the *release* — letting go must centre the wheel promptly so a
    // correction is possible at all — while the turn-in can stay calm.
    let rate = if target.abs() < state.steer_angle.abs() {
        spec.steer_rate * 1.6
    } else {
        spec.steer_rate
    };
    let blend = 1.0 - (-rate * dt).exp();
    state.steer_angle += (target - state.steer_angle) * blend;
}

/// Couples the wheels on each axle so the body resists rolling into a corner.
fn apply_anti_roll(
    spec: &VehicleSpec,
    state: &VehicleState,
    transform: &Transform,
    anchors: &[Vec3; WHEEL_COUNT],
    forces: &mut ForcesItem,
    up: Vec3,
) {
    for [left, right] in [[0, 1], [2, 3]] {
        let difference = state.wheels[left].compression - state.wheels[right].compression;
        if difference.abs() < f32::EPSILON {
            continue;
        }
        let force = difference * spec.anti_roll;
        if state.wheels[left].grounded {
            forces.apply_force_at_point(-up * force, transform.transform_point(anchors[left]));
        }
        if state.wheels[right].grounded {
            forces.apply_force_at_point(up * force, transform.transform_point(anchors[right]));
        }
    }
}

/// How much force this tyre can put down before it slides.
fn friction_budget(spec: &VehicleSpec, input: &VehicleInput, wheel: usize, load: f32) -> f32 {
    let grip = if VehicleSpec::is_front(wheel) {
        spec.front_grip
    } else if input.handbrake {
        // Locking the rears is the entire drift mechanic.
        spec.rear_grip * spec.handbrake_grip
    } else {
        spec.rear_grip
    };
    grip * load
}

/// Forward and right axes for a wheel, flattened into the contact plane.
fn wheel_axes(steer_angle: f32, wheel: usize, forward: Vec3, up: Vec3) -> (Vec3, Vec3) {
    let steered = if VehicleSpec::is_front(wheel) {
        Quat::from_axis_angle(up, steer_angle) * forward
    } else {
        forward
    };
    // Project onto the ground plane so forces never fight the suspension.
    let flat = (steered - up * steered.dot(up)).normalize_or_zero();
    (flat, up.cross(flat))
}

fn longitudinal_force(
    spec: &VehicleSpec,
    input: &VehicleInput,
    forward_speed: f32,
    wheel: usize,
    rolling_speed: f32,
) -> f32 {
    let rear = !VehicleSpec::is_front(wheel);
    let mut force = 0.0;

    if input.throttle > 0.0 && rear {
        // Rear-wheel drive, split across the two driven wheels. Torque tapers
        // off near top speed instead of stopping dead.
        let headroom = 1.0 - (forward_speed / spec.max_speed).clamp(0.0, 1.0);
        force += input.throttle * spec.engine_force * headroom * 0.5;
    } else if input.throttle < 0.0 {
        if forward_speed > 0.5 {
            // Still rolling forwards, so this is braking, on all four wheels.
            force += input.throttle * spec.brake_force * 0.25;
        } else if rear {
            force += input.throttle * spec.reverse_force * 0.5;
        }
    }

    if input.handbrake && rear {
        force -= rolling_speed.signum() * spec.brake_force * 0.15;
    }

    force
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vehicle::spec::VehicleClass;
    use bevy::asset::AssetPlugin;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    const TICK: f64 = 1.0 / 64.0;

    /// Ride height the springs should settle at, derived from the spec alone:
    /// compression where spring force balances the car's weight.
    fn expected_ride_height(spec: &VehicleSpec) -> f32 {
        let load = spec.mass * 9.81 / WHEEL_COUNT as f32;
        let compression = load / spec.spring_strength;
        let ray = spec.max_ray_length() - compression;
        ray - spec.axle_height
    }

    fn harness(class: VehicleClass, drop_height: f32) -> (App, Entity, VehicleSpec) {
        let spec = class.spec();
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            PhysicsPlugins::default(),
        ));
        app.init_asset::<Mesh>();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            TICK,
        )));
        app.add_systems(FixedUpdate, drive_vehicles);

        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(400.0, 2.0, 400.0),
            Transform::from_xyz(0.0, -1.0, 0.0),
        ));

        let size = spec.half_extents * 2.0;
        let car = app
            .world_mut()
            .spawn((
                Transform::from_xyz(0.0, drop_height, 0.0),
                RigidBody::Dynamic,
                Collider::cuboid(size.x, size.y, size.z),
                Mass(spec.mass),
                CenterOfMass(spec.center_of_mass),
                SleepingDisabled,
                crate::vehicle::spawn::ActiveVehicle,
                VehicleInput::default(),
                VehicleState::default(),
                spec.clone(),
            ))
            .id();

        // Avian registers diagnostics resources in `Plugin::finish`, which a
        // bare `update()` loop never triggers.
        app.finish();
        app.cleanup();

        (app, car, spec)
    }

    fn step(app: &mut App, ticks: usize) {
        for _ in 0..ticks {
            app.update();
        }
    }

    fn set_input(app: &mut App, car: Entity, input: VehicleInput) {
        *app.world_mut().get_mut::<VehicleInput>(car).unwrap() = input;
    }

    fn transform_of(app: &App, car: Entity) -> Transform {
        *app.world().get::<Transform>(car).unwrap()
    }

    #[test]
    fn every_vehicle_settles_at_its_designed_ride_height() {
        for class in VehicleClass::ALL {
            let (mut app, car, spec) = harness(class, 3.0);
            step(&mut app, 400);

            let y = transform_of(&app, car).translation.y;
            let expected = expected_ride_height(&spec);
            assert!(
                (y - expected).abs() < 0.12,
                "{}: settled at {y:.3}m, expected about {expected:.3}m",
                spec.display_name
            );
        }
    }

    /// Highest and lowest the body gets over a span of ticks.
    fn ride_envelope(app: &mut App, car: Entity, ticks: usize) -> (f32, f32) {
        let mut low = f32::MAX;
        let mut high = f32::MIN;
        for _ in 0..ticks {
            step(app, 1);
            let y = transform_of(app, car).translation.y;
            low = low.min(y);
            high = high.max(y);
        }
        (low, high)
    }

    #[test]
    fn a_car_that_lands_bounces_before_it_settles() {
        // Both halves matter. The bounce is the joke; the settling is what
        // keeps a parked street from shimmering, and an underdamped spring
        // with no floor to it does exactly that forever.
        let (mut app, car, _) = harness(VehicleClass::Sedan, 2.5);
        step(&mut app, 30);
        let (low, high) = ride_envelope(&mut app, car, 80);
        assert!(
            high - low > 0.02,
            "landed and absorbed all of it: only {:.4}m of travel",
            high - low
        );

        step(&mut app, 700);
        let (low, high) = ride_envelope(&mut app, car, 120);
        assert!(
            high - low < 0.02,
            "still pogoing {:.4}m a full ten seconds after landing",
            high - low
        );
    }

    #[test]
    fn all_four_wheels_find_the_ground() {
        // Long enough for the springs to stop ringing. Underdamped suspension
        // carries more or less than the car's weight on every half-cycle, so
        // sampling the load mid-bounce measures the bounce, not the load.
        let (mut app, car, _) = harness(VehicleClass::Sedan, 2.0);
        step(&mut app, 900);
        let state = app.world().get::<VehicleState>(car).unwrap();
        assert_eq!(state.grounded_wheels(), WHEEL_COUNT);
        // Load should roughly add up to the car's weight.
        let total: f32 = state.wheels.iter().map(|w| w.load).sum();
        let weight = 1400.0 * 9.81;
        assert!(
            (total - weight).abs() / weight < 0.2,
            "suspension carries {total:.0}N but the car weighs {weight:.0}N"
        );
    }

    #[test]
    fn throttle_drives_the_car_forwards() {
        let (mut app, car, _) = harness(VehicleClass::Sedan, 1.5);
        step(&mut app, 200);
        let start = transform_of(&app, car).translation;

        set_input(
            &mut app,
            car,
            VehicleInput {
                throttle: 1.0,
                ..default()
            },
        );
        step(&mut app, 240);

        let end = transform_of(&app, car).translation;
        // Forward in Bevy is -Z.
        assert!(
            end.z < start.z - 5.0,
            "car did not accelerate forwards: {start:?} -> {end:?}"
        );
        let speed = app.world().get::<VehicleState>(car).unwrap().forward_speed;
        assert!(speed > 3.0, "forward speed only {speed:.2} m/s");
    }

    #[test]
    fn braking_pulls_a_moving_car_up() {
        let (mut app, car, _) = harness(VehicleClass::Sedan, 1.5);
        step(&mut app, 200);
        set_input(
            &mut app,
            car,
            VehicleInput {
                throttle: 1.0,
                ..default()
            },
        );
        step(&mut app, 300);
        let fast = app.world().get::<VehicleState>(car).unwrap().forward_speed;

        set_input(
            &mut app,
            car,
            VehicleInput {
                throttle: -1.0,
                ..default()
            },
        );
        step(&mut app, 120);
        let slow = app.world().get::<VehicleState>(car).unwrap().forward_speed;

        assert!(
            slow < fast * 0.6,
            "braking barely helped: {fast:.2} -> {slow:.2} m/s"
        );
    }

    /// Angle between where the car points and where it is actually going.
    /// Near zero means gripping; large means sliding.
    fn slip_angle(app: &App, car: Entity) -> f32 {
        let transform = transform_of(app, car);
        let velocity = app.world().get::<LinearVelocity>(car).unwrap().0;
        let flat = Vec3::new(velocity.x, 0.0, velocity.z);
        if flat.length() < 1.0 {
            return 0.0;
        }
        let heading = *transform.forward();
        flat.normalize().angle_between(heading)
    }

    fn corner(app: &mut App, car: Entity, handbrake: bool) -> f32 {
        step(app, 200);
        set_input(
            app,
            car,
            VehicleInput {
                throttle: 1.0,
                ..default()
            },
        );
        step(app, 260);
        set_input(
            app,
            car,
            VehicleInput {
                throttle: 1.0,
                steer: 1.0,
                handbrake,
            },
        );
        step(app, 90);
        slip_angle(app, car)
    }

    /// Peak reported slip while cornering, which is what the tyre audio reads.
    fn peak_slip(app: &mut App, car: Entity, handbrake: bool) -> f32 {
        step(app, 200);
        set_input(
            app,
            car,
            VehicleInput {
                throttle: 1.0,
                ..default()
            },
        );
        step(app, 260);
        set_input(
            app,
            car,
            VehicleInput {
                throttle: 1.0,
                steer: 1.0,
                handbrake,
            },
        );
        let mut peak = 0.0f32;
        for _ in 0..90 {
            step(app, 1);
            peak = peak.max(app.world().get::<VehicleState>(car).unwrap().slip);
        }
        peak
    }

    #[test]
    fn reported_slip_tracks_whether_the_tyres_are_actually_sliding() {
        let (mut app, car, _) = harness(VehicleClass::Sedan, 1.5);
        // Straight and level, the lateral force cancels slip outright.
        step(&mut app, 200);
        assert_eq!(app.world().get::<VehicleState>(car).unwrap().slip, 0.0);

        let gripping = peak_slip(&mut app, car, false);
        let (mut app, car, _) = harness(VehicleClass::Sedan, 1.5);
        let sliding = peak_slip(&mut app, car, true);

        assert!(
            sliding > gripping,
            "the handbrake should report more slip: {gripping:.3} vs {sliding:.3} m/s"
        );
    }

    #[test]
    fn the_handbrake_breaks_rear_traction() {
        // The drift is not a special mode: yanking the handbrake shrinks the
        // rear friction budget, so the back simply stops holding on.
        let (mut app, car, _) = harness(VehicleClass::Sedan, 1.5);
        let gripping = corner(&mut app, car, false);

        let (mut app, car, _) = harness(VehicleClass::Sedan, 1.5);
        let sliding = corner(&mut app, car, true);

        assert!(
            sliding > gripping * 1.5,
            "handbrake barely changed the slip angle: {gripping:.3} rad gripping \
             vs {sliding:.3} rad sliding"
        );
    }

    #[test]
    fn steering_turns_the_car_and_it_stays_upright() {
        let (mut app, car, _) = harness(VehicleClass::Sedan, 1.5);
        step(&mut app, 200);
        let start_yaw = transform_of(&app, car).rotation.to_euler(EulerRot::YXZ).0;

        set_input(
            &mut app,
            car,
            VehicleInput {
                throttle: 1.0,
                steer: 1.0,
                handbrake: false,
            },
        );
        step(&mut app, 400);

        let transform = transform_of(&app, car);
        let end_yaw = transform.rotation.to_euler(EulerRot::YXZ).0;
        assert!(
            (end_yaw - start_yaw).abs() > 0.3,
            "car barely turned: yaw {start_yaw:.3} -> {end_yaw:.3}"
        );

        // Cornering must not roll it over; up should still point up.
        let up = transform.up().dot(Vec3::Y);
        assert!(up > 0.8, "car tipped while cornering, up.y = {up:.3}");
    }
}
