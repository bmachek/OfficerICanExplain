//! HUD, minimap, menus, and developer tooling.

pub mod crosshair;
pub mod debug;
pub mod hud;
pub mod minimap;

use bevy::prelude::*;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            minimap::MinimapPlugin,
            hud::HudPlugin,
            crosshair::CrosshairPlugin,
        ));

        // Screenshots should show the game, not the tuning panel.
        if !crate::core::capture::is_capture_mode() {
            app.add_plugins(debug::DebugUiPlugin);
        }
    }
}
