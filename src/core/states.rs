//! Top-level application state and the in-game sub-state.

use bevy::prelude::*;

#[derive(States, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum AppState {
    #[default]
    Loading,
    Menu,
    InGame,
}

/// Only exists while [`AppState::InGame`]; the resource is removed otherwise.
#[derive(SubStates, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
#[source(AppState = AppState::InGame)]
pub enum InGameState {
    #[default]
    Playing,
    Paused,
}

pub struct GameStatesPlugin;

impl Plugin for GameStatesPlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>()
            .add_sub_state::<InGameState>()
            // Nothing to stream in yet, so drop straight into the world.
            // M6 replaces this with a real main menu.
            .add_systems(Startup, skip_straight_into_game);
    }
}

fn skip_straight_into_game(mut next: ResMut<NextState<AppState>>) {
    next.set(AppState::InGame);
}
