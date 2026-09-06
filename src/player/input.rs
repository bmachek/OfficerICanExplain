//! Device-agnostic action mapping.
//!
//! Gameplay never reads keys directly; it reads `ActionState<Action>`. That
//! keeps keyboard and gamepad on one path and makes rebinding a data change.
//!
//! `Move` is deliberately shared between on-foot and driving: on foot it is a
//! direction, in a car its Y is throttle/brake and its X is steering. Same for
//! `Jump`/`Handbrake` sharing Space — context decides which one reads it, which
//! is exactly how the games this borrows from behave.
//!
//! [`Action::Taunt`] and [`Action::Cheer`] sit on the two mouse buttons that
//! used to fire and aim a weapon. The bindings are unchanged on purpose: the
//! trigger finger already knows where they are, and a game about being rude to
//! strangers wants its rudeness under the same thumb a shooter puts a gun.

use bevy::prelude::*;
use leafwing_input_manager::prelude::*;

#[derive(Actionlike, PartialEq, Eq, Clone, Copy, Hash, Debug, Reflect)]
pub enum Action {
    #[actionlike(DualAxis)]
    Move,
    #[actionlike(DualAxis)]
    Look,
    Jump,
    Sprint,
    /// Enter/exit vehicle, pick things up.
    Interact,
    /// Blow a raspberry at everybody nearby.
    Taunt,
    /// Whistle at them instead.
    Cheer,
    Handbrake,
    Pause,
    /// Opens the full-screen map.
    Map,
    QuickSave,
    QuickLoad,
    /// Toggles the free-fly debug camera.
    ToggleDebugCamera,
}

impl Action {
    pub fn default_input_map() -> InputMap<Self> {
        let mut map = InputMap::default();

        // Keyboard and mouse.
        map.insert_dual_axis(Self::Move, VirtualDPad::wasd());
        map.insert_dual_axis(Self::Look, MouseMove::default());
        map.insert(Self::Jump, KeyCode::Space);
        map.insert(Self::Sprint, KeyCode::ShiftLeft);
        map.insert(Self::Interact, KeyCode::KeyF);
        map.insert(Self::Taunt, MouseButton::Left);
        map.insert(Self::Cheer, MouseButton::Right);
        map.insert(Self::Handbrake, KeyCode::Space);
        map.insert(Self::Pause, KeyCode::Escape);
        map.insert(Self::Map, KeyCode::KeyM);
        map.insert(Self::QuickSave, KeyCode::F5);
        map.insert(Self::QuickLoad, KeyCode::F9);
        map.insert(Self::ToggleDebugCamera, KeyCode::F1);

        // Gamepad.
        map.insert_dual_axis(Self::Move, GamepadStick::LEFT);
        map.insert_dual_axis(Self::Look, GamepadStick::RIGHT);
        map.insert(Self::Jump, GamepadButton::South);
        map.insert(Self::Sprint, GamepadButton::LeftThumb);
        map.insert(Self::Interact, GamepadButton::North);
        map.insert(Self::Taunt, GamepadButton::RightTrigger2);
        map.insert(Self::Cheer, GamepadButton::LeftTrigger2);
        map.insert(Self::Handbrake, GamepadButton::East);
        map.insert(Self::Pause, GamepadButton::Start);
        map.insert(Self::Map, GamepadButton::Select);

        map
    }
}

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        // The map itself is attached to the player entity in `on_foot`.
        app.add_plugins(InputManagerPlugin::<Action>::default());
    }
}
