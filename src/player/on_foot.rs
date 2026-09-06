//! The player on foot — or rather, the player in the air, most of the time.
//!
//! Movement went through a floating character controller, which solves stairs,
//! kerbs and slopes by hovering the body a fixed distance above whatever is
//! beneath it. That is exactly the right answer for a city made of kerbs and
//! corners, and exactly the wrong one for a city made of rubber: a body held
//! off the ground by a spring never forms a contact, and restitution is a
//! property of a contact. The player could be declared as elastic as you like
//! and would still land like a sack.
//!
//! So the float is gone and [`crate::bounce::controller`] has the job instead.
//! It costs the free kerb handling — a hop clears a kerb rather than stepping
//! over one — which turns out to be the better trade, because clearing a kerb
//! by bouncing over it is the game.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;

use crate::bounce::controller::{Bouncer, JUMP_SCALE};
use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;
use crate::core::settings::KeyBindings;
use crate::mood::face::FaceLevel;
use crate::mood::feeling::{Mood, Temperament};
use crate::mood::provoke::Provoker;
use crate::mood::voice::Voicebox;
use crate::player::camera::CameraRig;
use crate::player::input::Action;
use crate::world::City;
use crate::world::buildings::SIDEWALK_HEIGHT;

#[derive(Component)]
pub struct Player;

pub const CAPSULE_RADIUS: f32 = 0.38;
/// Length of the cylindrical section; total height is this plus two radii.
pub const CAPSULE_LENGTH: f32 = 1.05;
/// Distance from the capsule's centre to its lowest point.
pub const STAND_HEIGHT: f32 = CAPSULE_LENGTH / 2.0 + CAPSULE_RADIUS;

const SPRINT_SPEED: f32 = 7.6;
/// Fraction of top speed used when not sprinting.
const JOG_PACE: f32 = 0.62;

pub struct OnFootPlugin;

impl Plugin for OnFootPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostStartup, spawn_player).add_systems(
            Update,
            drive_player
                .in_set(GameSet::Simulation)
                // Ahead of the bouncer, which reads what this writes.
                .before(crate::bounce::controller::bounce_bodies),
        );
    }
}

fn spawn_player(
    mut commands: Commands,
    city: Res<City>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    figures: Res<crate::ai::figure::FigureAssets>,
    faces: Res<crate::mood::face::FaceAssets>,
    keybindings: Res<KeyBindings>,
) {
    // Start on an actual street rather than at the origin, which is usually
    // inside a downtown block.
    let start = city
        .graph
        .nearest_node(Vec2::ZERO)
        .map(|id| city.graph.node(id).pos)
        .unwrap_or(Vec2::ZERO);

    let temper = Temperament::ordinary();
    let worn = faces.wear(temper.baseline);

    let mut player = commands.spawn((
        Name::new("Player"),
        Player,
        Transform::from_xyz(start.x, SIDEWALK_HEIGHT + STAND_HEIGHT + 0.2, start.y),
        Visibility::default(),
        RigidBody::Dynamic,
        Collider::capsule(CAPSULE_RADIUS, CAPSULE_LENGTH),
        // Upright while in charge of themselves. `bounce::launch` takes this
        // off when somebody is thrown, which is when tumbling is the point.
        LockedAxes::ROTATION_LOCKED,
        Bouncer::new(STAND_HEIGHT),
        // An ordinary citizen rather than a special case, so that the player's
        // own face sours in a bad-tempered crowd and cheers up in a good one.
        // Being subject to the mood is what makes it a toy rather than a gauge.
        temper,
        Mood::new(temper.baseline),
        FaceLevel(worn.level),
        // Dead centre of the crowd's range: the player's voice is the one the
        // others are heard against.
        Voicebox::new(1.0),
        Provoker::default(),
        // The player carries the input map; everything else reads ActionState.
        Action::input_map(&keybindings),
    ));

    // The same figure the crowd wears, in a jacket that reads at a distance —
    // in a third-person game the player is on screen more than anything else.
    let coat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.62, 0.20, 0.17),
        perceptual_roughness: 0.82,
        ..default()
    });
    let mut rng = crate::core::rng::stream_for(0, crate::core::rng::stream::PEDESTRIANS);
    crate::ai::figure::dress(&mut player, &figures, coat, &worn, &mut rng);
}

fn drive_player(
    config: Res<GameConfig>,
    rigs: Query<&CameraRig>,
    mut players: Query<
        (&ActionState<Action>, &mut Bouncer, &mut Transform),
        (With<Player>, Without<crate::player::interact::Driving>),
    >,
) {
    let Ok((action_state, mut bouncer, mut transform)) = players.single_mut() else {
        return;
    };

    // Movement is camera-relative: pushing forward means "away from the
    // camera", which is what every third-person game trains players to expect.
    let yaw = rigs.single().map(|rig| rig.yaw).unwrap_or(0.0);
    let frame = Quat::from_rotation_y(yaw);
    let input = action_state.clamped_axis_pair(&Action::Move);
    let direction = (frame * Vec3::NEG_Z * input.y + frame * Vec3::X * input.x).normalize_or_zero();

    let pace = if action_state.pressed(&Action::Sprint) {
        SPRINT_SPEED
    } else {
        SPRINT_SPEED * JOG_PACE
    };
    bouncer.desired = direction.xz() * pace;

    // Turn to face travel, and hold the last heading when idle. Written here
    // rather than left to the solver because rotation is locked: nothing else
    // is going to turn the body, and a figure that walks sideways looks like a
    // bug rather than like a joke.
    if let Ok(facing) = Dir2::new(direction.xz()) {
        transform.rotation = Quat::from_rotation_y(crate::vehicle::spawn::heading_towards(*facing));
    }

    // The resting hop is set every frame — the controller spends the scale on
    // each landing, the same contract `ai::pedestrian` uses for the crowd. See
    // `BounceConfig::player_hop_scale` for why the player of all people
    // bounces least.
    bouncer.hop_scale = config.bounce.player_hop_scale;

    // Only off the ground. Held down, this would otherwise be a pogo stick with
    // no ceiling: every landing would take the bigger hop, and each one lands
    // faster than the last. Absolute rather than scaled by the resting hop, so
    // dialling the walk-bounce down does not also cost jump height.
    if action_state.pressed(&Action::Jump) && bouncer.grounded {
        bouncer.hop_scale = JUMP_SCALE;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::AssetPlugin;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    /// Steps physics without a window, so "does the character bounce on the
    /// ground or sink through it" is a test rather than something we squint at
    /// in a screenshot.
    fn harness(spawn_height: f32) -> (App, Entity) {
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
        // Real elapsed time in a test is effectively zero, so drive the clock.
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 64.0,
        )));
        app.init_resource::<crate::core::config::GameConfig>();
        app.add_systems(Update, crate::bounce::controller::bounce_bodies);

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
                Bouncer::new(STAND_HEIGHT),
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

    /// Highest and lowest the body gets over a span of ticks.
    fn envelope(app: &mut App, player: Entity, ticks: usize) -> (f32, f32) {
        let mut low = f32::MAX;
        let mut high = f32::MIN;
        for _ in 0..ticks {
            app.update();
            let y = height_of(app, player);
            low = low.min(y);
            high = high.max(y);
        }
        (low, high)
    }

    #[test]
    fn a_dropped_player_lands_on_the_ground_rather_than_through_it() {
        let (mut app, player) = harness(6.0);
        settle(&mut app, 240);
        let (low, _) = envelope(&mut app, player, 120);
        assert!(
            low > STAND_HEIGHT - 0.2,
            "sank to {low}, below the soles at {STAND_HEIGHT}"
        );
    }

    #[test]
    fn a_player_standing_still_keeps_bouncing() {
        // The whole conceit of the game. A flummi that settles is a person.
        let (mut app, player) = harness(2.0);
        settle(&mut app, 240);
        let (low, high) = envelope(&mut app, player, 120);
        assert!(
            high - low > 0.1,
            "only moved {:.3}m over two seconds; that is standing, not bouncing",
            high - low
        );
    }

    #[test]
    fn the_bounce_holds_its_height_instead_of_dying_away() {
        // Restitution alone would damp out within a second or two. The hop is
        // assigned rather than added precisely so that it does not.
        let (mut app, player) = harness(2.0);
        settle(&mut app, 240);
        let (_, early) = envelope(&mut app, player, 90);
        settle(&mut app, 300);
        let (_, late) = envelope(&mut app, player, 90);
        assert!(
            (late - early).abs() < 0.15,
            "bounce apex drifted from {early:.2} to {late:.2}"
        );
    }

    /// Feeds the bouncer directly, the way `drive_player` does.
    fn walk(app: &mut App, player: Entity, direction: Vec2, ticks: usize) {
        for _ in 0..ticks {
            app.world_mut().get_mut::<Bouncer>(player).unwrap().desired = direction * SPRINT_SPEED;
            app.update();
        }
    }

    #[test]
    fn travelling_moves_the_character_at_roughly_the_asked_for_speed() {
        let (mut app, player) = harness(2.0);
        settle(&mut app, 180);

        let start = app.world().get::<Transform>(player).unwrap().translation;
        let ticks = 128;
        walk(&mut app, player, Vec2::NEG_Y, ticks);
        let end = app.world().get::<Transform>(player).unwrap().translation;

        let travelled = (end - start).with_y(0.0).length();
        let seconds = ticks as f32 / 64.0;
        // Allow for the acceleration ramp at the start, and for the reduced
        // authority a bouncing body has while it is off the ground.
        let expected = SPRINT_SPEED * seconds;
        assert!(
            travelled > expected * 0.7,
            "covered only {travelled:.2}m in {seconds:.2}s, expected near {expected:.2}m"
        );
        assert!(
            (end.z - start.z) < -1.0,
            "moved the wrong way along Z: {start:?} -> {end:?}"
        );
    }

    #[test]
    fn a_player_left_alone_stays_where_they_are() {
        // Bouncing on the spot must not wander. A body that drifts while nobody
        // is touching it walks itself into the traffic over a minute.
        let (mut app, player) = harness(2.0);
        settle(&mut app, 240);
        let before = app.world().get::<Transform>(player).unwrap().translation;
        settle(&mut app, 240);
        let after = app.world().get::<Transform>(player).unwrap().translation;
        assert!(
            (after.xz() - before.xz()).length() < 0.3,
            "drifted from {before:?} to {after:?} while standing still"
        );
    }
}
