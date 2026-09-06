//! Saving and loading.
//!
//! The save is a small RON document rather than a snapshot of the ECS. The
//! world is fully reproducible from its seed, so there is nothing to store
//! about the city itself — only the handful of facts that are *not* derivable:
//! where the player is, and what time it is when they get there.

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;
use serde::{Deserialize, Serialize};

use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;
use crate::player::input::Action;
use crate::player::on_foot::Player;
use crate::world::timeofday::TimeOfDay;

/// Bumped whenever the format changes incompatibly.
pub const SAVE_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SaveGame {
    pub version: u32,
    /// The city regenerates from this; nothing about it is stored.
    pub world_seed: u64,
    pub player: [f32; 3],
    pub hour: f32,
}

impl SaveGame {
    pub fn to_ron(&self) -> Result<String, ron::Error> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default())
    }

    pub fn from_ron(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text)
    }

    /// Whether this save can be loaded by the current build.
    pub fn is_compatible(&self) -> bool {
        self.version == SAVE_VERSION
    }
}

pub fn save_path() -> PathBuf {
    Path::new("saves").join("quicksave.ron")
}

/// Whether a save exists to load, so the menu can grey out "Laden" instead of
/// letting the player click their way to an error.
pub fn save_exists() -> bool {
    save_path().exists()
}

/// Writes the current player position and time of day to [`save_path`].
///
/// Shared by the `F5` quicksave keybind and the pause menu's "Speichern"
/// button, so there is exactly one place that knows the save format.
pub fn write_save(
    config: &GameConfig,
    clock: &TimeOfDay,
    transform: &Transform,
) -> Result<(), String> {
    let save = SaveGame {
        version: SAVE_VERSION,
        world_seed: config.world_seed,
        player: transform.translation.to_array(),
        hour: clock.hours,
    };

    let path = save_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    save.to_ron()
        .map_err(|e| e.to_string())
        .and_then(|text| std::fs::write(&path, text).map_err(|e| e.to_string()))
}

/// Reads [`save_path`] onto `clock` and `transform`. Shared by the `F9`
/// quickload keybind and the pause menu's "Laden" button; callers that care
/// about a missing file rather than a corrupt one should check
/// [`save_exists`] first.
pub fn read_save(clock: &mut TimeOfDay, transform: &mut Transform) -> Result<(), String> {
    let path = save_path();
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let save = SaveGame::from_ron(&text).map_err(|e| e.to_string())?;
    if !save.is_compatible() {
        return Err(format!(
            "Speicherstand ist Version {}, dieser Build liest Version {SAVE_VERSION}",
            save.version
        ));
    }

    transform.translation = Vec3::from_array(save.player);
    clock.hours = save.hour;
    Ok(())
}

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (quick_save, quick_load).in_set(GameSet::Simulation));
    }
}

fn quick_save(
    config: Res<GameConfig>,
    clock: Res<TimeOfDay>,
    players: Query<(&Transform, &ActionState<Action>), With<Player>>,
) {
    let Ok((transform, action_state)) = players.single() else {
        return;
    };
    if !action_state.just_pressed(&Action::QuickSave) {
        return;
    }

    match write_save(&config, &clock, transform) {
        Ok(()) => info!("saved to {}", save_path().display()),
        Err(error) => error!("could not save: {error}"),
    }
}

fn quick_load(
    mut clock: ResMut<TimeOfDay>,
    mut players: Query<(&mut Transform, &ActionState<Action>), With<Player>>,
) {
    let Ok((mut transform, action_state)) = players.single_mut() else {
        return;
    };
    if !action_state.just_pressed(&Action::QuickLoad) {
        return;
    }

    if !save_exists() {
        warn!("no save at {}", save_path().display());
        return;
    }
    match read_save(&mut clock, &mut transform) {
        Ok(()) => info!("loaded from {}", save_path().display()),
        Err(error) => error!("could not load: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SaveGame {
        SaveGame {
            version: SAVE_VERSION,
            world_seed: 0xA17E_5EED,
            player: [12.5, 1.0, -403.25],
            hour: 21.25,
        }
    }

    #[test]
    fn a_save_survives_a_round_trip() {
        let original = sample();
        let text = original.to_ron().expect("serialise");
        let restored = SaveGame::from_ron(&text).expect("deserialise");
        assert_eq!(original, restored);
    }

    #[test]
    fn the_seed_is_stored_so_the_city_can_be_rebuilt() {
        // The world is not serialised; it is regenerated. Losing the seed would
        // mean loading a save into a completely different city.
        let text = sample().to_ron().unwrap();
        assert!(text.contains("world_seed"));
        let restored = SaveGame::from_ron(&text).unwrap();
        assert_eq!(restored.world_seed, 0xA17E_5EED);
    }

    #[test]
    fn a_save_from_another_version_is_rejected() {
        let mut save = sample();
        save.version = SAVE_VERSION + 1;
        assert!(!save.is_compatible());
        assert!(sample().is_compatible());
    }

    #[test]
    fn corrupt_text_is_an_error_not_a_panic() {
        assert!(SaveGame::from_ron("this is not ron at all {{{").is_err());
        assert!(SaveGame::from_ron("").is_err());
    }
}
