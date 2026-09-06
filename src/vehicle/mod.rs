//! Vehicles: arcade physics, spawning, and the cars themselves.

pub mod body;
pub mod controller;
pub mod damage;
pub mod lights;
pub mod paint;
pub mod plate;
pub mod spawn;
pub mod spec;
pub mod trim;

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
                    // Between the impact and the explosion: a car that is
                    // about to be wrecked should still take its last dent.
                    damage::dent_bodywork,
                    damage::scuff_paint,
                    damage::explode_wrecked_vehicles,
                    damage::fade_explosions,
                    damage::smoke_from_dying_engines,
                    damage::fade_smoke,
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
    mut images: ResMut<Assets<Image>>,
) {
    commands.insert_resource(damage::SmokeAssets::new(&mut meshes));
    commands.insert_resource(spawn::build_assets(
        &mut meshes,
        &mut materials,
        &mut images,
    ));
    commands.insert_resource(lights::build_assets(&mut meshes, &mut materials));
}
