//! Weapons, health, and the consequences of using them.

pub mod health;
pub mod weapons;

use bevy::prelude::*;

pub struct CombatPlugin;

impl Plugin for CombatPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(weapons::WeaponsPlugin);
    }
}
