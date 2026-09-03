//! Vehicles: arcade physics, spawning, and the cars themselves.

pub mod controller;
pub mod damage;
pub mod lights;
pub mod spawn;
pub mod spec;

use bevy::prelude::*;

use crate::core::schedule::GameSet;

pub struct VehiclePlugin;

impl Plugin for VehiclePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(lights::VehicleLightsPlugin)
            .add_message::<damage::VehicleDestroyed>()
            .add_message::<damage::VehicleImpact>()
            .add_systems(Startup, setup_assets)
            .add_systems(PostStartup, spawn::spawn_parked_vehicles)
            // Forces must be applied before Avian steps in `FixedPostUpdate`,
            // and re-applied every tick because Avian clears them after.
            .add_systems(FixedUpdate, controller::drive_vehicles)
            .add_systems(
                Update,
                (
                    damage::apply_crash_damage,
                    damage::explode_wrecked_vehicles,
                    damage::fade_explosions,
                )
                    .chain()
                    .in_set(GameSet::Simulation),
            )
            .add_systems(
                Update,
                (spawn::activate_nearby_vehicles, spawn::update_wheel_visuals)
                    .in_set(GameSet::Simulation),
            );
    }
}

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(spawn::build_assets(&mut meshes, &mut materials));
    commands.insert_resource(lights::build_assets(&mut meshes, &mut materials));
}
