//! Cross-cutting foundations: states, schedule structure, tunables, determinism.

pub mod assets;
pub mod capture;
pub mod config;
pub mod rng;
pub mod schedule;
pub mod states;

use bevy::prelude::*;

pub struct CorePlugin;

impl Plugin for CorePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            states::GameStatesPlugin,
            schedule::SchedulePlugin,
            config::ConfigPlugin,
            capture::CapturePlugin,
        ));
    }
}
