//! One explicit ordering for per-frame game logic.
//!
//! Everything gameplay-related joins one of these sets so that ordering is a
//! property of the set graph rather than of `.after()` chains scattered across
//! plugins. Physics is *not* here: Avian owns its own `PhysicsSchedule`, and
//! vehicle forces are applied inside it (see `vehicle::controller`).

use bevy::prelude::*;

use super::states::AppState;

#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum GameSet {
    /// Sample devices into `ActionState`.
    Input,
    /// Agents decide what they want (steering targets, pursuit decisions).
    Ai,
    /// Apply decisions to the world.
    Simulation,
    /// Position cameras after everything they follow has moved.
    Camera,
    /// HUD and menus read final state.
    Ui,
}

pub struct SchedulePlugin;

impl Plugin for SchedulePlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            Update,
            (
                GameSet::Input,
                GameSet::Ai,
                GameSet::Simulation,
                GameSet::Camera,
                GameSet::Ui,
            )
                .chain(),
        )
        .configure_sets(
            Update,
            (GameSet::Ai, GameSet::Simulation, GameSet::Camera).run_if(in_state(AppState::InGame)),
        );
    }
}
