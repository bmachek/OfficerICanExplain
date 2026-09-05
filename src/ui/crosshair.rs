//! The aiming reticle.
//!
//! Shots are cast from the camera along its own forward axis, so the centre of
//! the screen genuinely is where the bullet goes. A mark drawn there is not
//! decoration — it is the one honest statement the HUD can make about the
//! weapon, and without it the player is aiming at an unlabelled point and
//! guessing which one.
//!
//! It stays hidden until it is wanted. A reticle burned into the middle of the
//! screen while walking down a street is in the way of the game; one that
//! appears when the trigger finger does is information.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;

use crate::combat::health::Health;
use crate::combat::weapons::{Weapon, WeaponFired};
use crate::core::schedule::GameSet;
use crate::player::camera::CameraRig;
use crate::player::input::Action;
use crate::player::interact::Driving;
use crate::player::on_foot::Player;
use crate::vehicle::damage::VehicleHealth;

/// Length of each arm, in pixels.
const ARM: f32 = 9.0;
/// Thickness of each arm. Odd-looking at 2px until you remember the centre gap
/// is what the eye actually reads.
const THICK: f32 = 2.0;
/// Half the empty space in the middle. The gap is the aiming point; filling it
/// in would hide the thing being aimed at.
const GAP: f32 = 5.0;
/// Side of the square the arms are laid out in.
const SPAN: f32 = (GAP + ARM) * 2.0;

/// How long the reticle lingers after a shot, so a pistol tapped once does not
/// flicker it on and off.
const LINGER: f32 = 1.2;

/// Seconds since the last shot.
///
/// Kept here rather than read off `Weapon::since_shot`, which is the firing
/// cooldown: a fresh weapon starts its cooldown already spent — that is what
/// makes the first shot instant — and reading it would open the reticle on the
/// title screen for a second before anybody had touched a trigger.
#[derive(Resource)]
struct SinceShot(f32);

impl Default for SinceShot {
    fn default() -> Self {
        // Starts run out: nothing has been fired yet.
        Self(LINGER)
    }
}

const READY: Color = Color::srgba(0.96, 0.97, 1.0, 0.85);
/// Over something that can be hurt.
const HOT: Color = Color::srgba(1.0, 0.36, 0.28, 0.95);

#[derive(Component)]
struct CrosshairRoot;
#[derive(Component)]
struct CrosshairArm;

pub struct CrosshairPlugin;

impl Plugin for CrosshairPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SinceShot>()
            .add_systems(PostStartup, spawn_crosshair)
            .add_systems(Update, refresh_crosshair.in_set(GameSet::Ui));
    }
}

/// A piece of the reticle: left, top, width, height, in the square's own
/// pixels.
type Rect = (f32, f32, f32, f32);

/// The four arms and the centre pip.
///
/// Built by a function rather than written out so the shape is derived from
/// the three numbers that describe it, and so the arithmetic that keeps the
/// arms square around the aiming point is something a test can look at.
fn layout() -> [Rect; 5] {
    let centre = SPAN * 0.5;
    // Each arm is centred on the axis it straddles, and grows outwards from
    // the gap.
    let near = centre - THICK * 0.5;
    let far = SPAN - ARM;
    [
        (near, 0.0, THICK, ARM),
        (near, far, THICK, ARM),
        (0.0, near, ARM, THICK),
        (far, near, ARM, THICK),
        // The pip in the middle of the gap. Small enough to sight past, and
        // the only part that survives against a busy street.
        (near, near, THICK, THICK),
    ]
}

/// One piece of the reticle, placed in the square by its top-left corner.
fn arm((left, top, width, height): Rect) -> impl Bundle {
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(left),
            top: Val::Px(top),
            width: Val::Px(width),
            height: Val::Px(height),
            ..default()
        },
        BackgroundColor(READY),
        CrosshairArm,
    )
}

fn spawn_crosshair(mut commands: Commands) {
    let [top, bottom, left, right, pip] = layout();

    commands.spawn((
        Name::new("Crosshair"),
        // Centred by the layout rather than by a hand-computed offset, so it
        // stays in the middle at every window size — and the middle is where
        // the bullet goes, so being off by a few pixels would be a lie.
        Node {
            position_type: PositionType::Absolute,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            display: Display::None,
            ..default()
        },
        Pickable::IGNORE,
        GlobalZIndex(11),
        CrosshairRoot,
        children![(
            Node {
                width: Val::Px(SPAN),
                height: Val::Px(SPAN),
                ..default()
            },
            children![arm(top), arm(bottom), arm(left), arm(right), arm(pip),],
        )],
    ));
}

fn refresh_crosshair(
    time: Res<Time>,
    spatial: SpatialQuery,
    mut shots: MessageReader<WeaponFired>,
    mut since_shot: ResMut<SinceShot>,
    cameras: Query<&GlobalTransform, With<CameraRig>>,
    players: Query<(Entity, &ActionState<Action>, &Weapon), (With<Player>, Without<Driving>)>,
    targets: Query<(), Or<(With<Health>, With<VehicleHealth>)>>,
    mut root: Query<&mut Node, With<CrosshairRoot>>,
    mut arms: Query<&mut BackgroundColor, With<CrosshairArm>>,
) {
    // Drained every frame whether or not anything is listening, or the queue
    // grows for as long as the player is in a car.
    since_shot.0 = if shots.read().count() > 0 {
        0.0
    } else {
        (since_shot.0 + time.delta_secs()).min(LINGER)
    };

    // Driving fails the query, which is the intent: the shot from a car goes
    // through the boot of the car, and a reticle promising otherwise would be
    // the dishonest kind.
    let mut show = false;
    let mut hot = false;

    if let Ok((player, action_state, weapon)) = players.single() {
        show = weapon.ammo > 0
            && (action_state.pressed(&Action::Aim)
                || action_state.pressed(&Action::Fire)
                || since_shot.0 < LINGER);

        if show
            && let Ok(camera) = cameras.single()
            && let Ok(direction) = Dir3::new(camera.forward().as_vec3())
        {
            let filter = SpatialQueryFilter::from_excluded_entities([player]);
            hot = spatial
                .cast_ray(
                    camera.translation(),
                    direction,
                    weapon.kind.range(),
                    true,
                    &filter,
                )
                .is_some_and(|hit| targets.contains(hit.entity));
        }
    }

    if let Ok(mut node) = root.single_mut() {
        let wanted = if show { Display::Flex } else { Display::None };
        if node.display != wanted {
            node.display = wanted;
        }
    }
    if !show {
        return;
    }
    let ink = if hot { HOT } else { READY };
    for mut color in &mut arms {
        if color.0 != ink {
            color.0 = ink;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_piece_sits_inside_the_reticle() {
        // An arm that hangs outside its square is drawn off-centre, and the
        // centre is the one thing the reticle is claiming to mark.
        for (left, top, width, height) in layout() {
            assert!(
                left >= 0.0 && top >= 0.0 && left + width <= SPAN && top + height <= SPAN,
                "piece at ({left}, {top}) size {width}x{height} escapes a {SPAN}px square"
            );
        }
    }

    #[test]
    fn the_four_arms_are_symmetric_about_the_aiming_point() {
        let [top, bottom, left, right, _] = layout();
        let centre = SPAN * 0.5;
        // Each arm is the same distance from the middle as its opposite. Get
        // this wrong and the reticle points a little away from where the
        // bullet actually goes, which is worse than having no reticle at all.
        assert!(((centre - (top.1 + top.3)) - (bottom.1 - centre)).abs() < 1e-4);
        assert!(((centre - (left.0 + left.2)) - (right.0 - centre)).abs() < 1e-4);
    }

    #[test]
    fn the_arms_stop_short_of_the_target() {
        let [top, _, _, _, pip] = layout();
        let centre = SPAN * 0.5;
        let clearance = centre - (top.1 + top.3);
        assert!(
            clearance >= pip.3,
            "the arms close to within {clearance}px of the middle, leaving nothing to sight through"
        );
    }
}
