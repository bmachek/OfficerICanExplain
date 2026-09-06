//! Persisted player preferences: rebindable keys, mouse-look direction, audio
//! and graphics — everything the pause menu's settings screens change.
//!
//! `GameConfig` already derives `Serialize`/`Deserialize` (see its module
//! docs), so most of this is just writing it to `saves/options.ron` instead
//! of only ever holding live defaults. Key bindings are the exception: an
//! `InputMap` stores its bindings as trait objects and cannot round-trip
//! through serde on its own, so the handful of keys a player can rebind live
//! in their own small map instead, keyed by [`RebindableAction`] rather than
//! by `Action` so the menu never has to reject an axis or a mouse button as
//! "not rebindable" — the type says so up front.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::core::config::GameConfig;
use crate::player::input::Action;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RebindableAction {
    Jump,
    Sprint,
    Interact,
    Handbrake,
    Pause,
    Map,
    QuickSave,
    QuickLoad,
    ToggleDebugCamera,
}

impl RebindableAction {
    pub const ALL: [Self; 9] = [
        Self::Jump,
        Self::Sprint,
        Self::Interact,
        Self::Handbrake,
        Self::Pause,
        Self::Map,
        Self::QuickSave,
        Self::QuickLoad,
        Self::ToggleDebugCamera,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Jump => "Springen",
            Self::Sprint => "Sprinten",
            Self::Interact => "Interagieren / Einsteigen",
            Self::Handbrake => "Handbremse",
            Self::Pause => "Menü",
            Self::Map => "Karte",
            Self::QuickSave => "Schnellspeichern",
            Self::QuickLoad => "Schnellladen",
            Self::ToggleDebugCamera => "Freie Kamera (Debug)",
        }
    }

    pub fn action(self) -> Action {
        match self {
            Self::Jump => Action::Jump,
            Self::Sprint => Action::Sprint,
            Self::Interact => Action::Interact,
            Self::Handbrake => Action::Handbrake,
            Self::Pause => Action::Pause,
            Self::Map => Action::Map,
            Self::QuickSave => Action::QuickSave,
            Self::QuickLoad => Action::QuickLoad,
            Self::ToggleDebugCamera => Action::ToggleDebugCamera,
        }
    }

    /// The keyboard binding `Action::default_input_map` gives this action.
    /// [`Action::input_map`] needs this to know exactly which binding to
    /// replace rather than clearing the action's bindings outright, which
    /// would also throw away its gamepad button.
    pub fn default_key(self) -> KeyCode {
        match self {
            Self::Jump => KeyCode::Space,
            Self::Sprint => KeyCode::ShiftLeft,
            Self::Interact => KeyCode::KeyF,
            Self::Handbrake => KeyCode::Space,
            Self::Pause => KeyCode::Escape,
            Self::Map => KeyCode::KeyM,
            Self::QuickSave => KeyCode::F5,
            Self::QuickLoad => KeyCode::F9,
            Self::ToggleDebugCamera => KeyCode::F1,
        }
    }
}

/// Live key bindings for the rebindable actions.
#[derive(Resource, Debug, Clone, Serialize, Deserialize)]
pub struct KeyBindings(pub HashMap<RebindableAction, KeyCode>);

impl Default for KeyBindings {
    fn default() -> Self {
        Self(
            RebindableAction::ALL
                .into_iter()
                .map(|action| (action, action.default_key()))
                .collect(),
        )
    }
}

impl KeyBindings {
    pub fn key_for(&self, action: RebindableAction) -> KeyCode {
        self.0
            .get(&action)
            .copied()
            .unwrap_or_else(|| action.default_key())
    }
}

/// The whole of what the settings screens change, together in one file so
/// loading and saving are each a single call.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Options {
    config: GameConfig,
    keybindings: KeyBindings,
}

fn options_path() -> PathBuf {
    Path::new("saves").join("options.ron")
}

/// Reads `saves/options.ron` onto `config` and `keybindings`, if it exists
/// and parses. Silent on any failure beyond a log line: a missing or corrupt
/// options file must never stop the game from starting with defaults.
fn load(config: &mut GameConfig, keybindings: &mut KeyBindings) {
    let Ok(text) = std::fs::read_to_string(options_path()) else {
        return;
    };
    match ron::from_str::<Options>(&text) {
        Ok(options) => {
            *config = options.config;
            *keybindings = options.keybindings;
        }
        Err(error) => warn!("saves/options.ron is corrupt ({error}); using defaults"),
    }
}

/// Writes `config` and `keybindings` to `saves/options.ron`.
pub fn save(config: &GameConfig, keybindings: &KeyBindings) {
    let options = Options {
        config: config.clone(),
        keybindings: keybindings.clone(),
    };
    let text = match ron::ser::to_string_pretty(&options, ron::ser::PrettyConfig::default()) {
        Ok(text) => text,
        Err(error) => {
            error!("could not serialise settings: {error}");
            return;
        }
    };
    let path = options_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(error) = std::fs::write(&path, text) {
        error!("could not save settings: {error}");
    }
}

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        let mut config = GameConfig::default();
        let mut keybindings = KeyBindings::default();
        load(&mut config, &mut keybindings);
        app.insert_resource(config).insert_resource(keybindings);
    }
}
