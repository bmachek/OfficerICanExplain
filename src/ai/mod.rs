//! Agents that inhabit the city: traffic now, pedestrians and police to follow.

pub mod pedestrian;
pub mod police;
pub mod steering;
pub mod traffic;

use bevy::prelude::*;

pub struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            traffic::TrafficPlugin,
            pedestrian::PedestrianPlugin,
            police::PolicePlugin,
        ));
    }
}
