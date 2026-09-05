//! Agents that inhabit the city: traffic, and the crowd on the pavement.

pub mod figure;
pub mod pedestrian;
pub mod steering;
pub mod traffic;

use bevy::prelude::*;

pub struct AiPlugin;

impl Plugin for AiPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((traffic::TrafficPlugin, pedestrian::PedestrianPlugin));
    }
}
