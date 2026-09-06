//! Everything in this city is made of rubber.
//!
//! The module owns the elastic half of the simulation: how bodies get about by
//! bouncing, what a bounce looks and sounds like, and what happens to somebody
//! hit hard enough to stop being in charge of themselves for a moment.

pub mod boing;
pub mod controller;
pub mod launch;
pub mod squash;

use bevy::prelude::*;

pub struct BouncePlugin;

impl Plugin for BouncePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            controller::BounceControllerPlugin,
            boing::BoingPlugin,
            launch::LaunchPlugin,
        ));
    }
}
