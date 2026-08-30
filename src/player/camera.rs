//! One camera, several modes.
//!
//! Deliberately a single camera entity rather than one per mode: switching
//! `Camera::is_active` between several cameras means every mode has to agree on
//! render target, fog and post-processing, and they drift. One rig that changes
//! behaviour keeps that impossible.

use std::f32::consts::FRAC_PI_2;

use avian3d::prelude::*;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;

use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;
use crate::player::input::Action;
use crate::player::interact::Driving;
use crate::player::on_foot::Player;

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
}

impl Default for CameraRig {
    fn default() -> Self {
        Self {
            yaw: 0.0,
            pitch: -0.22,
            mode: CameraMode::Follow,
            distance: 6.5,
        }
    }
}

/// Height above the player's origin that the camera looks at.
const SHOULDER_HEIGHT: f32 = 1.5;
/// Keeps the camera off the surface it collides with.
const WALL_MARGIN: f32 = 0.35;

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
