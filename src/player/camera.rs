//! One camera, several modes.
//!
//! Deliberately a single camera entity rather than one per mode: switching
//! `Camera::is_active` between several cameras means every mode has to agree on
//! render target, fog and post-processing, and they drift. One rig that changes
//! behaviour keeps that impossible.

use std::f32::consts::{FRAC_PI_2, PI, TAU};

use avian3d::prelude::*;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;

use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;
use crate::player::input::Action;
use crate::player::interact::Driving;
use crate::player::on_foot::Player;
use crate::vehicle::spawn::{Vehicle, heading_towards};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraMode {
    /// Orbits the player. The normal gameplay view.
    #[default]
    Follow,
    /// Detached free-fly, for inspecting the world.
    Free,
}

#[derive(Component)]
pub struct CameraRig {
    pub yaw: f32,
    pub pitch: f32,
    pub mode: CameraMode,
    /// How far back the camera sits when nothing is in the way.
    pub distance: f32,
    /// Seconds since the player last aimed the view themselves. The automatic
    /// swing waits on this so it never wrestles a hand that is already on the
    /// mouse.
    pub since_look: f32,
}

impl Default for CameraRig {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: -0.22,
            mode: CameraMode::Follow,
            distance: 6.5,
            since_look: 0.0,
        }
    }
}

/// Height above the player's origin that the camera looks at.
const SHOULDER_HEIGHT: f32 = 1.5;
/// Keeps the camera off the surface it collides with.
const WALL_MARGIN: f32 = 0.35;

/// Mouse movement below this many pixels in one frame is jitter, not a look.
/// Testing for exactly zero would let a single drifting pixel switch the
/// automatic swing off for the rest of the session.
const LOOK_DEADZONE: f32 = 0.5;
/// How slowly the view creeps round on foot, against the driving rate. A car
/// is committed to where its nose points; a person sidesteps constantly, and a
/// view that chased every step would be seasick.
const ON_FOOT_EASE: f32 = 0.45;
/// Below this speed there is no direction of travel worth following.
const MIN_TRAVEL_SPEED: f32 = 0.9;

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_camera).add_systems(
            Update,
            (
                toggle_mode,
                // Mouse look only. The OS keeps reporting pointer motion over
                // the window even with nobody at the keyboard, which during a
                // capture silently rotates the rig into the ground and makes
                // screenshots non-reproducible.
                orbit.run_if(|| !crate::core::capture::is_capture_mode()),
                // Same reasoning as `orbit`, and one more: a capture asks for
                // an exact pose with `--eye`, and a camera that steers itself
                // would quietly walk away from it between warm-up frames.
                auto_follow.run_if(|| !crate::core::capture::is_capture_mode()),
                free_fly,
                follow_player,
            )
                .chain()
                .in_set(GameSet::Camera),
        );
    }
}

fn spawn_camera(mut commands: Commands) {
    let rig = CameraRig::default();
    commands.spawn((
        Name::new("Camera"),
        Camera3d::default(),
        // With a minimap camera in the world too, UI needs to be told which
        // view it belongs to; otherwise the HUD silently renders nowhere.
        IsDefaultUiCamera,
        Transform::from_xyz(0.0, 60.0, 120.0).with_rotation(Quat::from_euler(
            EulerRot::YXZ,
            rig.yaw,
            rig.pitch,
            0.0,
        )),
        rig,
    ));
}

fn toggle_mode(actions: Query<&ActionState<Action>>, mut rigs: Query<&mut CameraRig>) {
    let Ok(action_state) = actions.single() else {
        return;
    };
    if !action_state.just_pressed(&Action::ToggleDebugCamera) {
        return;
    }
    for mut rig in &mut rigs {
        rig.mode = match rig.mode {
            CameraMode::Follow => CameraMode::Free,
            CameraMode::Free => CameraMode::Follow,
        };
    }
}

/// Mouse look. In free mode it needs the right button held, so the cursor stays
/// usable for the dev panel; while following, the mouse always steers the view.
fn orbit(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    config: Res<GameConfig>,
    mut rigs: Query<&mut CameraRig>,
) {
    if motion.delta == Vec2::ZERO {
        return;
    }
    let sensitivity = config.camera.mouse_sensitivity;
    for mut rig in &mut rigs {
        if rig.mode == CameraMode::Free && !buttons.pressed(MouseButton::Right) {
            continue;
        }
        rig.yaw -= motion.delta.x * sensitivity;
        rig.pitch =
            (rig.pitch - motion.delta.y * sensitivity).clamp(-FRAC_PI_2 + 0.05, FRAC_PI_2 - 0.05);
    }
}

/// Signed shortest way round from `from` to `to`, in radians.
///
/// Worth its own function: the naive `to - from` takes the long way whenever
/// the two straddle the wrap at ±π, and the view would whip a full circle at
/// the exact moment somebody is mid-corner.
pub fn angle_delta(from: f32, to: f32) -> f32 {
    let delta = (to - from).rem_euclid(TAU);
    if delta > PI { delta - TAU } else { delta }
}

/// One step of the swing: where the yaw lands after easing towards `target`.
///
/// Exponential rather than a fixed step so it is framerate independent, and so
/// the last few degrees arrive gently instead of stopping dead.
pub fn swing_towards(yaw: f32, target: f32, rate: f32, delta_seconds: f32) -> f32 {
    if rate <= 0.0 {
        return yaw;
    }
    let blend = 1.0 - (-rate * delta_seconds).exp();
    yaw + angle_delta(yaw, target) * blend
}

/// Swings the view in behind the direction of travel.
///
/// A camera that only ever points where the mouse last left it makes driving
/// two jobs at once: one hand on the car and one keeping the car on screen.
/// Every game in this genre answers it the same way — the view drifts round
/// behind you as you move, and gets out of the way the moment you take hold of
/// it yourself.
fn auto_follow(
    time: Res<Time>,
    config: Res<GameConfig>,
    motion: Res<AccumulatedMouseMotion>,
    players: Query<
        (
            &LinearVelocity,
            Option<&Driving>,
            Option<&ActionState<Action>>,
        ),
        With<Player>,
    >,
    vehicles: Query<(&Transform, &LinearVelocity), (With<Vehicle>, Without<Player>)>,
    mut rigs: Query<&mut CameraRig>,
) {
    let Ok((velocity, driving, action_state)) = players.single() else {
        return;
    };

    // Aiming counts as looking: someone holding a weapon on a target is
    // pointing the camera on purpose, and their own footwork must not drag it
    // off the thing they are pointing it at.
    let aiming = action_state.is_some_and(|state| state.pressed(&Action::Aim));
    let handled = aiming || motion.delta.length() > LOOK_DEADZONE;

    // What "behind" means. A car points where its nose points even while it is
    // reversing, so backing off a kerb does not whip the view round the boot
    // and back. On foot there is no reliable heading — the capsule's rotation
    // is locked so the body never turns — so travel is read off the velocity.
    let (heading, speed, ease) = match driving.and_then(|d| vehicles.get(d.0).ok()) {
        Some((car, car_velocity)) => (
            car.forward().as_vec3(),
            car_velocity.0.with_y(0.0).length(),
            1.0,
        ),
        None => {
            let flat = velocity.0.with_y(0.0);
            (flat, flat.length(), ON_FOOT_EASE)
        }
    };
    let target = Dir3::new(heading.with_y(0.0))
        .ok()
        .filter(|_| speed >= MIN_TRAVEL_SPEED)
        .map(|direction| heading_towards(Vec2::new(direction.x, direction.z)));

    for mut rig in &mut rigs {
        rig.since_look = if handled {
            0.0
        } else {
            rig.since_look + time.delta_secs()
        };

        if rig.mode != CameraMode::Follow || rig.since_look < config.camera.auto_follow_delay {
            continue;
        }
        let Some(target) = target else { continue };
        rig.yaw = swing_towards(
            rig.yaw,
            target,
            config.camera.auto_follow * ease,
            time.delta_secs(),
        );
    }
}

/// WASD to move, Q/E for down/up, Shift to boost. Free mode only.
fn free_fly(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    config: Res<GameConfig>,
    mut rigs: Query<(&mut Transform, &CameraRig)>,
) {
    let mut local = Vec3::ZERO;
    for (key, axis) in [
        (KeyCode::KeyW, -Vec3::Z),
        (KeyCode::KeyS, Vec3::Z),
        (KeyCode::KeyA, -Vec3::X),
        (KeyCode::KeyD, Vec3::X),
        (KeyCode::KeyE, Vec3::Y),
        (KeyCode::KeyQ, -Vec3::Y),
    ] {
        if keys.pressed(key) {
            local += axis;
        }
    }

    let mut speed = config.camera.speed;
    if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        speed *= config.camera.boost_multiplier;
    }
    let step = local.normalize_or_zero() * speed * time.delta_secs();

    for (mut transform, rig) in &mut rigs {
        if rig.mode != CameraMode::Free {
            continue;
        }
        transform.rotation = Quat::from_euler(EulerRot::YXZ, rig.yaw, rig.pitch, 0.0);
        if step == Vec3::ZERO {
            continue;
        }
        // Vertical stays world-relative so Q/E never drifts with pitch.
        let horizontal = transform.rotation * Vec3::new(step.x, 0.0, step.z);
        transform.translation += horizontal + Vec3::Y * step.y;
    }
}

/// Orbits the player, pulling in when a wall would otherwise be between them.
fn follow_player(
    time: Res<Time>,
    spatial: SpatialQuery,
    players: Query<(&GlobalTransform, Option<&Driving>), With<Player>>,
    vehicles: Query<&GlobalTransform, Without<Player>>,
    mut rigs: Query<(&mut Transform, &CameraRig, Entity)>,
) {
    let Ok((player, driving)) = players.single() else {
        return;
    };

    // Sit further back and higher behind a car; the same framing that works for
    // a person on foot is uselessly tight at 100km/h.
    let (focus, pull_back) = match driving.and_then(|d| vehicles.get(d.0).ok()) {
        Some(vehicle) => (vehicle.translation() + Vec3::Y * 1.1, 1.9),
        None => (player.translation(), 1.0),
    };
    let target = focus + Vec3::Y * SHOULDER_HEIGHT;

    for (mut transform, rig, camera_entity) in &mut rigs {
        if rig.mode != CameraMode::Follow {
            continue;
        }

        let rotation = Quat::from_euler(EulerRot::YXZ, rig.yaw, rig.pitch, 0.0);
        let offset = rotation * Vec3::Z;

        // Cast from the player outwards; if something is in the way, sit just
        // in front of it rather than letting the wall swallow the view.
        let filter = SpatialQueryFilter::default().with_excluded_entities([camera_entity]);
        let wanted = rig.distance * pull_back;
        let mut distance = wanted;
        if let Ok(direction) = Dir3::new(offset)
            && let Some(hit) = spatial.cast_ray(target, direction, wanted, true, &filter)
        {
            distance = (hit.distance - WALL_MARGIN).max(0.6);
        }

        let desired = target + offset * distance;
        // Exponential smoothing, framerate independent.
        let blend = 1.0 - (-18.0 * time.delta_secs()).exp();
        transform.translation = transform.translation.lerp(desired, blend);
        transform.rotation = rotation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_swing_takes_the_short_way_round() {
        // Just past the wrap in each direction. Naive subtraction sends the
        // camera the long way round both times.
        assert!(
            angle_delta(PI - 0.1, -PI + 0.1) > 0.0,
            "crossing +π forwards should keep going forwards"
        );
        assert!(
            angle_delta(-PI + 0.1, PI - 0.1) < 0.0,
            "and crossing it backwards should keep going backwards"
        );
        assert!(
            angle_delta(PI - 0.1, -PI + 0.1).abs() < 0.3,
            "by 0.2 rad, not by 6"
        );
    }

    #[test]
    fn the_swing_closes_on_the_heading_without_overshooting() {
        let target = 2.4;
        let mut yaw: f32 = -1.1;
        for _ in 0..240 {
            let before = angle_delta(yaw, target).abs();
            yaw = swing_towards(yaw, target, 3.0, 1.0 / 60.0);
            assert!(
                angle_delta(yaw, target).abs() <= before + 1e-6,
                "the swing moved away from the heading"
            );
        }
        assert!(
            angle_delta(yaw, target).abs() < 0.01,
            "four seconds should have arrived; sitting at {yaw}"
        );
    }

    #[test]
    fn a_zero_rate_leaves_the_view_alone() {
        // The config dial goes to zero, and zero has to mean off rather than
        // "swing instantly", which is what an unguarded exponential does.
        assert_eq!(swing_towards(0.7, -2.0, 0.0, 1.0 / 60.0), 0.7);
    }

    #[test]
    fn the_target_yaw_points_the_view_along_the_travel() {
        for direction in [
            Vec3::NEG_Z,
            Vec3::Z,
            Vec3::X,
            Vec3::new(0.6, 0.0, -0.8),
            Vec3::new(-0.28, 0.0, 0.96),
        ] {
            let yaw = heading_towards(Vec2::new(direction.x, direction.z));
            let looking = Quat::from_rotation_y(yaw) * Vec3::NEG_Z;
            assert!(
                looking.distance(direction.normalize()) < 1e-4,
                "yaw for {direction:?} left the camera looking {looking:?}"
            );
        }
    }
}
