//! The player: input mapping, on-foot movement, and the camera rig.

pub mod camera;
pub mod input;
pub mod interact;
pub mod on_foot;

use bevy::prelude::*;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            input::InputPlugin,
            camera::CameraPlugin,
            on_foot::OnFootPlugin,
            interact::InteractPlugin,
        ));
    }
}
