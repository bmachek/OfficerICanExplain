//! The player on foot.
//!
//! Movement goes through Tnua rather than a hand-rolled kinematic controller.
//! A floating character controller solves stairs, kerbs and slopes by hovering
//! the body a fixed distance above whatever is beneath it, which is exactly the
//! set of problems that make hand-written controllers snag on geometry — and
//! this city is nothing but kerbs and corners.

use avian3d::prelude::*;
use bevy::prelude::*;
use bevy_tnua::builtins::{
    TnuaBuiltinJump, TnuaBuiltinJumpConfig, TnuaBuiltinWalk, TnuaBuiltinWalkConfig,
};
use bevy_tnua::prelude::*;
use bevy_tnua_avian3d::prelude::*;
use leafwing_input_manager::prelude::ActionState;

use crate::core::schedule::GameSet;
use crate::player::camera::CameraRig;
use crate::player::input::Action;
use crate::world::City;
use crate::world::buildings::SIDEWALK_HEIGHT;

/// What the player can do on foot. The derive generates `PlayerSchemeConfig`,
/// with one field per variant plus the walk basis.
#[derive(TnuaScheme)]
#[scheme(basis = TnuaBuiltinWalk)]
pub enum PlayerScheme {
    Jump(TnuaBuiltinJump),
}

#[derive(Component)]
pub struct Player;

pub const CAPSULE_RADIUS: f32 = 0.38;
/// Length of the cylindrical section; total height is this plus two radii.
pub const CAPSULE_LENGTH: f32 = 1.05;
/// Must exceed the distance from the capsule's centre to its lowest point
/// (`CAPSULE_LENGTH / 2 + CAPSULE_RADIUS`), or the controller fights the floor.
const FLOAT_HEIGHT: f32 = 1.0;

const SPRINT_SPEED: f32 = 7.6;
/// Fraction of top speed used when not sprinting.
const JOG_PACE: f32 = 0.62;

pub struct OnFootPlugin;

impl Plugin for OnFootPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            TnuaControllerPlugin::<PlayerScheme>::new(FixedUpdate),
            TnuaAvian3dPlugin::new(FixedUpdate),
        ))
        .add_systems(PostStartup, spawn_player)
        .add_systems(
            Update,
            drive_player
                .in_set(TnuaUserControlsSystems)
                .in_set(GameSet::Simulation),
        );
    }
}

fn spawn_player(
    mut commands: Commands,
    city: Res<City>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut configs: ResMut<Assets<PlayerSchemeConfig>>,
    figures: Res<crate::ai::figure::FigureAssets>,
) {
    // Start on an actual street rather than at the origin, which is usually
    // inside a downtown block.
    let start = city
        .graph
        .nearest_node(Vec2::ZERO)
        .map(|id| city.graph.node(id).pos)
        .unwrap_or(Vec2::ZERO);

    let mut player = commands.spawn((
        Name::new("Player"),
        Player,
        Transform::from_xyz(start.x, SIDEWALK_HEIGHT + FLOAT_HEIGHT + 0.2, start.y),
        Visibility::default(),
        RigidBody::Dynamic,
        Collider::capsule(CAPSULE_RADIUS, CAPSULE_LENGTH),
        // Tnua corrects tipping, but locking rotation stops the capsule from
        // toppling in the frames before it gets the chance.
        LockedAxes::ROTATION_LOCKED,
        TnuaController::<PlayerScheme>::default(),
        TnuaConfig::<PlayerScheme>(configs.add(PlayerSchemeConfig {
            basis: TnuaBuiltinWalkConfig {
                speed: SPRINT_SPEED,
                float_height: FLOAT_HEIGHT,
                acceleration: 55.0,
                air_acceleration: 18.0,
                // Anything steeper than this is a wall, not a ramp.
                max_slope: std::f32::consts::FRAC_PI_4 * 1.2,
                ..default()
            },
            jump: TnuaBuiltinJumpConfig {
                height: 1.5,
                ..default()
            },
        })),
        // Without a sensor shape the ground check is a single ray, which snags
        // on kerb edges and building corners.
        TnuaAvian3dSensorShape(Collider::cylinder(CAPSULE_RADIUS * 0.94, 0.0)),
        // The player carries the input map; everything else reads ActionState.
        Action::default_input_map(),
        crate::combat::health::Health::new(100.0),
        crate::combat::weapons::Weapon::new(crate::combat::weapons::WeaponKind::Pistol, 90),
    ));

    // The same figure the crowd wears, in a jacket that reads at a distance —
    // in a third-person game the player is on screen more than anything else.
    let coat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.62, 0.20, 0.17),
        perceptual_roughness: 0.82,
        ..default()
    });
    let mut rng = crate::core::rng::stream_for(0, crate::core::rng::stream::PEDESTRIANS);
    crate::ai::figure::dress(&mut player, &figures, coat, &mut rng);
}

fn drive_player(
    rigs: Query<&CameraRig>,
    mut players: Query<
        (&ActionState<Action>, &mut TnuaController<PlayerScheme>),
        (With<Player>, Without<crate::player::interact::Driving>),
    >,
) {
    let Ok((action_state, mut controller)) = players.single_mut() else {
        return;
    };
    // Tnua requires this every frame, before the basis and any actions.
    controller.initiate_action_feeding();

    // Movement is camera-relative: pushing forward means "away from the
    // camera", which is what every third-person game trains players to expect.
    let yaw = rigs.single().map(|rig| rig.yaw).unwrap_or(0.0);
    let frame = Quat::from_rotation_y(yaw);
    let input = action_state.clamped_axis_pair(&Action::Move);
    let direction = (frame * Vec3::NEG_Z * input.y + frame * Vec3::X * input.x).normalize_or_zero();

    let pace = if action_state.pressed(&Action::Sprint) {
        1.0
    } else {
        JOG_PACE
    };

    controller.basis = TnuaBuiltinWalk {
        desired_motion: direction * pace,
        // Turn to face travel; `None` when idle so the body holds its heading.
        desired_forward: Dir3::new(direction).ok(),
    };

    if action_state.pressed(&Action::Jump) {
        controller.action(PlayerScheme::Jump(TnuaBuiltinJump {
            allow_in_air: false,
            ..default()
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    /// Steps physics without a window, so "does the character stand on the
    /// ground or sink through it" is a test rather than something we squint at
    /// in a screenshot.
    fn harness(spawn_height: f32) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            PhysicsPlugins::default(),
            TnuaControllerPlugin::<PlayerScheme>::new(FixedUpdate),
            TnuaAvian3dPlugin::new(FixedUpdate),
        ));
        // Avian's collider cache reads `AssetEvent<Mesh>`; `AssetPlugin` alone
        // does not register the Mesh asset type outside a render app.
        app.init_asset::<Mesh>();
        // Real elapsed time in a test is effectively zero, so drive the clock.
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 64.0,
        )));

        let config = app
            .world_mut()
            .resource_mut::<Assets<PlayerSchemeConfig>>()
            .add(PlayerSchemeConfig {
                basis: TnuaBuiltinWalkConfig {
                    speed: SPRINT_SPEED,
                    float_height: FLOAT_HEIGHT,
                    ..default()
                },
                jump: TnuaBuiltinJumpConfig::default(),
            });

        // Ground: top face at y = 0.
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(200.0, 2.0, 200.0),
            Transform::from_xyz(0.0, -1.0, 0.0),
        ));

        let player = app
            .world_mut()
            .spawn((
                Player,
                Transform::from_xyz(0.0, spawn_height, 0.0),
                RigidBody::Dynamic,
                Collider::capsule(CAPSULE_RADIUS, CAPSULE_LENGTH),
                LockedAxes::ROTATION_LOCKED,
                TnuaController::<PlayerScheme>::default(),
                TnuaConfig::<PlayerScheme>(config),
                TnuaAvian3dSensorShape(Collider::cylinder(CAPSULE_RADIUS * 0.94, 0.0)),
            ))
            .id();

        // `run()` does this for us; a bare `update()` loop does not. Avian
        // registers its diagnostics resources in `Plugin::finish`, and its
        // systems hard-require them.
        app.finish();
        app.cleanup();

        (app, player)
    }

    fn settle(app: &mut App, ticks: usize) {
        for _ in 0..ticks {
            app.update();
        }
    }

    fn height_of(app: &App, player: Entity) -> f32 {
        app.world().get::<Transform>(player).unwrap().translation.y
    }

    #[test]
    fn float_height_clears_the_capsule() {
        // If this fails the controller is trying to hover lower than the body's
        // own lowest point, and it will grind into the floor forever.
        let lowest_point = CAPSULE_LENGTH / 2.0 + CAPSULE_RADIUS;
        assert!(
            FLOAT_HEIGHT > lowest_point,
            "float height {FLOAT_HEIGHT} must exceed capsule reach {lowest_point}"
        );
    }

    #[test]
    fn player_settles_on_the_ground_rather_than_falling_through() {
        let (mut app, player) = harness(6.0);
        settle(&mut app, 240);

        let y = height_of(&app, player);
        assert!(
            (y - FLOAT_HEIGHT).abs() < 0.25,
            "expected to hover near {FLOAT_HEIGHT}, settled at {y}"
        );
    }

    /// Feeds the walk basis directly, the way `drive_player` does.
    fn walk(app: &mut App, player: Entity, direction: Vec3, ticks: usize) {
        for _ in 0..ticks {
            let mut controller = app
                .world_mut()
                .get_mut::<TnuaController<PlayerScheme>>(player)
                .unwrap();
            controller.initiate_action_feeding();
            controller.basis = TnuaBuiltinWalk {
                desired_motion: direction,
                ..default()
            };
            app.update();
        }
    }

    #[test]
    fn walking_moves_the_character_at_roughly_the_configured_speed() {
        let (mut app, player) = harness(2.0);
        settle(&mut app, 180);

        let start = app.world().get::<Transform>(player).unwrap().translation;
        let ticks = 128;
        walk(&mut app, player, Vec3::NEG_Z, ticks);
        let end = app.world().get::<Transform>(player).unwrap().translation;

        let travelled = (end - start).with_y(0.0).length();
        let seconds = ticks as f32 / 64.0;
        // Allow for the acceleration ramp at the start.
        let expected = SPRINT_SPEED * seconds;
        assert!(
            travelled > expected * 0.7,
            "walked only {travelled:.2}m in {seconds:.2}s, expected near {expected:.2}m"
        );
        assert!(
            (end.z - start.z) < -1.0,
            "moved the wrong way along Z: {start:?} -> {end:?}"
        );
    }

    #[test]
    fn player_stays_put_once_settled() {
        let (mut app, player) = harness(2.0);
        settle(&mut app, 240);
        let before = height_of(&app, player);
        settle(&mut app, 120);
        let after = height_of(&app, player);

        assert!(
            (after - before).abs() < 0.02,
            "character drifted from {before} to {after} while standing still"
        );
    }
}
