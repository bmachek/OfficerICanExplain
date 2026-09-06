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

use crate::core::schedule::GameSet;
use crate::core::settings::{KeyBindings, RebindableAction};

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

    /// Same as [`Self::default_input_map`], except the keyboard side of the
    /// rebindable actions comes from `keybindings` instead of the hard-coded
    /// defaults.
    ///
    /// Built by taking the default map and swapping out just the bindings
    /// that differ, rather than clearing each action outright: an action like
    /// `Jump` also carries a gamepad button, and clearing it to rebind the
    /// key would throw that away too.
    pub fn input_map(keybindings: &KeyBindings) -> InputMap<Self> {
        let mut map = Self::default_input_map();
        for rebindable in RebindableAction::ALL {
            let bound = keybindings.key_for(rebindable);
            let default = rebindable.default_key();
            if bound != default {
                let action = rebindable.action();
                map.remove(&action, default);
                map.insert(action, bound);
            }
        }
        map
    }
}

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        // The map itself is attached to the player entity in `on_foot`.
        app.add_plugins(InputManagerPlugin::<Action>::default())
            .add_systems(Update, apply_keybindings.in_set(GameSet::Input));
    }
}

/// Rebuilds the player's input map whenever the settings menu changes a key
/// binding. Whole-map rebuild rather than an incremental patch: it is the
/// same construction `input_map` already does for the initial spawn, so
/// there is exactly one place that knows how a `KeyBindings` becomes an
/// `InputMap`.
fn apply_keybindings(
    keybindings: Res<KeyBindings>,
    mut maps: Query<&mut InputMap<Action>, With<crate::player::on_foot::Player>>,
) {
    if !keybindings.is_changed() {
        return;
    }
    for mut map in &mut maps {
        *map = Action::input_map(&keybindings);
    }
}
