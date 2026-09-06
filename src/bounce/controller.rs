//! Getting about by bouncing.
//!
//! Everybody in this city is a rubber ball with legs, and this is the part that
//! keeps them in the air. It replaces a floating character controller, which
//! could not be made to work here for a reason worth writing down: a floating
//! controller holds the body a fixed distance above the ground with a spring,
//! so the collider never touches anything and never forms a contact manifold.
//! Restitution is a property of a contact. A body that has no contacts cannot
//! bounce, however elastic you declare it to be.
//!
//! So the ground is found with a ray and the rebound is applied by hand. The
//! vertical speed is *assigned* at the bottom of each hop rather than added to,
//! which is what stops the solver's own restitution and this system compounding
//! into a body that climbs out of the world. Whatever the last bounce gave back,
//! the next hop leaves at the same speed — so a flummi crossing a flat street
//! keeps a steady rhythm, and one thrown off a roof still lands like rubber.
//!
//! The ground probe reaches well past the soles on purpose. A bouncing body is
//! airborne for most of its cycle, and a controller that only steers while
//! strictly touching the ground gives the player about three frames of control
//! per second. Reaching down means the lower part of every arc counts as
//! grounded, which is where the steering that matters happens anyway.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::boing::PreviousVelocity;
use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;

/// How far below the soles the ground probe still counts as contact.
const GROUND_REACH: f32 = 0.45;
/// The probe starts a little above the origin so it cannot begin inside a kerb
/// the body is already standing on.
const PROBE_LIFT: f32 = 0.1;
/// Multiplier on the hop when somebody deliberately jumps.
pub const JUMP_SCALE: f32 = 2.6;

/// A body that gets about by bouncing.
///
/// Written by whoever owns it — the input handler for the player, the pavement
/// AI for the crowd — so this module does not need to know which it is looking
/// at, in the same way [`crate::ai::figure::WalkCycle`] does not.
#[derive(Component)]
pub struct Bouncer {
    /// Ground velocity this body is trying to reach, in m/s.
    pub desired: Vec2,
    /// Scales the next hop. 1.0 is travelling; more is a jump.
    pub hop_scale: f32,
    /// Distance from the body origin down to the soles.
    pub stand_height: f32,
    /// Whether the probe found ground under it this tick.
    pub grounded: bool,
    /// Seconds since the last landing, and how long that whole arc lasted.
    /// Together they say where in its hop a figure is, which is what the
    /// squash and stretch is posed from.
    pub since_landing: f32,
    pub last_arc: f32,
    /// Downward speed at the last landing. The boing is pitched off this.
    pub landing_speed: f32,
}

impl Bouncer {
    pub fn new(stand_height: f32) -> Self {
        Self {
            desired: Vec2::ZERO,
            hop_scale: 1.0,
            stand_height,
            grounded: false,
            since_landing: 0.0,
            // Not zero: a figure spawned mid-air would otherwise be posed as if
            // it had just landed, and pop when it actually does.
            last_arc: 0.5,
            landing_speed: 0.0,
        }
    }

    /// How far through its current hop this body is, 0 at the bottom and 1 at
    /// the next landing. Clamped, because an arc can always run long.
    pub fn hop_phase(&self) -> f32 {
        (self.since_landing / self.last_arc.max(0.05)).clamp(0.0, 1.0)
    }
}

/// A body temporarily not in charge of itself: thrown, and tumbling.
///
/// While this is on an entity the controller leaves it alone entirely, so the
/// throw carries and the solver's restitution is the only thing acting on it.
#[derive(Component)]
pub struct Launched;

pub struct BounceControllerPlugin;

impl Plugin for BounceControllerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, bounce_bodies.in_set(GameSet::Simulation));
    }
}

/// Steering a grounded body towards the speed it wants, in m/s².
///
/// A pure function of what the caller already knows, so the feel of the thing
/// is testable without a physics world: at rest it accelerates hardest, and as
/// the gap closes it eases off rather than overshooting and oscillating.
pub fn steer(current: Vec2, desired: Vec2, accel: f32, dt: f32) -> Vec2 {
    let gap = desired - current;
    let step = accel * dt;
    if gap.length() <= step {
        desired
    } else {
        current + gap.normalize() * step
    }
}

pub fn bounce_bodies(
    time: Res<Time>,
    config: Res<GameConfig>,
    spatial: SpatialQuery,
    bodies: Query<
        (
            Entity,
            &Transform,
            &mut LinearVelocity,
            &mut Bouncer,
            Option<&mut PreviousVelocity>,
        ),
        Without<Launched>,
    >,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let tune = &config.bounce;

    for (entity, transform, mut velocity, mut bouncer, previous) in bodies {
        // Excluding the body itself is load-bearing. An unfiltered ray started
        // above the origin hits the body's own collider first, which reads as
        // ground a body-height up — and it climbs, a metre and a half a frame.
        let from = transform.translation + Vec3::Y * PROBE_LIFT;
        let reach = bouncer.stand_height + PROBE_LIFT + GROUND_REACH;
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
        bouncer.grounded = spatial
            .cast_ray(from, Dir3::NEG_Y, reach, true, &filter)
            .is_some();

        let accel = if bouncer.grounded {
            tune.ground_accel
        } else {
            tune.air_accel
        };
        let steered = steer(velocity.0.xz(), bouncer.desired, accel, dt);
        velocity.x = steered.x;
        velocity.z = steered.y;

        bouncer.since_landing += dt;

        // The bottom of an arc: on the ground, and no longer rising.
        if bouncer.grounded && velocity.y <= 0.0 {
            bouncer.landing_speed = -velocity.y;
            bouncer.last_arc = bouncer.since_landing;
            bouncer.since_landing = 0.0;
            let hop = tune.hop_speed * bouncer.hop_scale;
            // The rebound is assigned by this controller, not done to the body
            // by the world, so it is booked into the wallop detector's memory
            // as well. Without this every hop reads as a ~2×hop_speed knock and
            // every jump as being hit by a car: the street boings on every
            // step, moods shift with nobody touching anybody, and a player who
            // jumps makes themselves furious. Only what the *solver* changes —
            // collisions, being thrown — is left for `spot_wallops` to see.
            if let Some(mut previous) = previous {
                previous.0.y += hop - velocity.y;
            }
            velocity.y = hop;
            // A jump is asked for once and spent once; holding the key down
            // must not turn into a pogo stick to the roofline.
            bouncer.hop_scale = 1.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steering_closes_the_gap_without_overshooting_it() {
        // One step larger than the gap must land exactly on the target, not
        // sail past it and come back — that is an oscillation, and it reads as
        // a body that cannot decide where it is going.
        let landed = steer(Vec2::ZERO, Vec2::new(3.0, 0.0), 100.0, 1.0);
        assert_eq!(landed, Vec2::new(3.0, 0.0));
    }

    #[test]
    fn steering_takes_a_bounded_step_when_the_gap_is_wide() {
        let step = steer(Vec2::ZERO, Vec2::new(40.0, 0.0), 10.0, 0.1);
        assert!(
            (step.length() - 1.0).abs() < 1e-4,
            "took a {} m/s step where the budget was 1.0",
            step.length()
        );
    }

    #[test]
    fn a_body_already_at_speed_is_left_alone() {
        let held = Vec2::new(0.0, 6.0);
        assert_eq!(steer(held, held, 42.0, 0.016), held);
    }

    #[test]
    fn a_hop_is_a_fraction_of_a_second_rather_than_a_moon_jump() {
        // Time to fall back from the top of one hop, under Earth gravity. Long
        // hops read as low gravity, which is a different joke from rubber.
        let hop = GameConfig::default().bounce.hop_speed;
        let arc = 2.0 * hop / 9.81;
        assert!(
            (0.25..0.85).contains(&arc),
            "a hop lasting {arc:.2}s is not a bounce"
        );
    }

    #[test]
    fn a_jump_clears_more_than_a_kerb_and_less_than_a_storey() {
        let hop = GameConfig::default().bounce.hop_speed * JUMP_SCALE;
        let apex = hop * hop / (2.0 * 9.81);
        assert!(apex > 1.0, "a jump of {apex:.2}m clears nothing");
        assert!(apex < 4.0, "a jump of {apex:.2}m is a helicopter");
    }
}
