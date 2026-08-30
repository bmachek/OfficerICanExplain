//! Device-agnostic action mapping.
//!
//! Gameplay never reads keys directly; it reads `ActionState<Action>`. That
//! keeps keyboard and gamepad on one path and makes rebinding a data change.
//!
//! `Move` is deliberately shared between on-foot and driving: on foot it is a
//! direction, in a car its Y is throttle/brake and its X is steering. Same for
//! `Jump`/`Handbrake` sharing Space — context decides which one reads it, which
//! is exactly how the games this borrows from behave.

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
    Fire,
    Aim,
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
        map.insert(Self::Fire, MouseButton::Left);
        map.insert(Self::Aim, MouseButton::Right);
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
        map.insert(Self::Fire, GamepadButton::RightTrigger2);
        map.insert(Self::Aim, GamepadButton::LeftTrigger2);
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
