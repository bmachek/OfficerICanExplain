//! Officer, I Can Explain — an original open-world crime sandbox.
//!
//! Every asset is generated procedurally at runtime; no third-party art,
//! trademarks or IP are used anywhere in this project.

// Milestones land incrementally, so parts of the foundation are written and
// tested a milestone or two before anything consumes them: the road graph's A*
// and nearest-node queries are for traffic and pursuit (M4/M5), the per-chunk
// RNG streams are for vehicle and pedestrian spawning, and the `arterial` flag
// drives roadblock placement. They are covered by tests today, so warning about
// them on every build would only train us to skim past the warning list — which
// is exactly where genuinely dead code hides.
// TODO: remove once M5 lands and these all have callers.
#![allow(dead_code)]
// Bevy query types are long by construction — the filters *are* the meaning.
// Hiding them behind type aliases moves the information away from where it is
// read. Bevy's own codebase allows this lint for the same reason.
#![allow(clippy::type_complexity)]
// Likewise for arity: a system's parameters *are* its dependency list, declared
// so the scheduler can parallelise. Splitting a system to satisfy an argument
// count would scatter one piece of logic across two, for no benefit.
#![allow(clippy::too_many_arguments)]

mod ai;
mod audio;
mod bounce;
mod core;
mod player;
mod render;
mod save;
mod ui;
mod vehicle;
mod world;

use bevy::asset::AssetPlugin;
use bevy::prelude::*;
use bevy::window::PresentMode;

/// Shown in the window title bar and on the title screen.
pub const GAME_TITLE: &str = "Officer, I Can Explain";

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: GAME_TITLE.into(),
                        resolution: (1600, 900).into(),
                        present_mode: PresentMode::AutoVsync,
                        ..default()
                    }),
                    ..default()
                })
                // Told explicitly, because Bevy would otherwise look for
                // `assets/` beside the binary. See `core::assets`.
                .set(AssetPlugin {
                    file_path: crate::core::assets::root().to_string_lossy().into_owned(),
                    ..default()
                }),
        )
        // Needed before `CorePlugin`, whose capture overrides may set it.
        .init_resource::<crate::ui::minimap::MapOpen>()
        .add_plugins((
            crate::core::CorePlugin,
            crate::ai::AiPlugin,
            crate::audio::AudioPlugin,
            crate::bounce::BouncePlugin,
            crate::save::SavePlugin,
            crate::player::PlayerPlugin,
            crate::render::RenderPlugin,
            crate::world::WorldPlugin,
            crate::vehicle::VehiclePlugin,
            crate::ui::UiPlugin,
        ))
        .run();
}
