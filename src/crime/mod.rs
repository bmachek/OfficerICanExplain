//! Crime and the police response to it.

pub mod events;
pub mod wanted;

use bevy::prelude::*;

use crate::combat::health::{Died, Health};
use crate::core::schedule::GameSet;
use crate::player::on_foot::Player;
use crate::vehicle::damage::VehicleDestroyed;
use crate::world::City;
use events::{CrimeKind, CrimeReported};
use wanted::Wanted;

/// A wrecked car counts against the player only if it happened close enough to
/// them to plausibly be their doing. Proper attribution would need to track who
/// last touched every vehicle; proximity gets the common cases (you blew it up,
/// you rammed it) without that bookkeeping.
const ATTRIBUTION_RANGE: f32 = 35.0;

pub struct CrimePlugin;

impl Plugin for CrimePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(wanted::WantedPlugin).add_systems(
            Update,
            (consequences, handle_player_death).in_set(GameSet::Simulation),
        );
    }
}

fn consequences(
    mut commands: Commands,
    mut deaths: MessageReader<Died>,
    mut wrecks: MessageReader<VehicleDestroyed>,
    mut crimes: MessageWriter<CrimeReported>,
    players: Query<&Transform, With<Player>>,
) {
    for death in deaths.read() {
        if death.by_player {
            crimes.write(CrimeReported {
                kind: CrimeKind::KilledCivilian,
                position: death.position,
            });
        }
        // The body is not simulated further; M6 can add a corpse if wanted.
        commands.entity(death.entity).try_despawn();
    }

    let player = players.single().ok().map(|t| t.translation);
    for wreck in wrecks.read() {
        let attributable = player
            .map(|p| p.distance(wreck.position) < ATTRIBUTION_RANGE)
            .unwrap_or(false);
        if attributable {
            crimes.write(CrimeReported {
                kind: CrimeKind::DestroyedVehicle,
                position: wreck.position,
            });
        }
    }
}

/// Wasted. Patch the player up, wipe the heat, and put them back on the street.
fn handle_player_death(
    city: Res<City>,
    mut wanted: ResMut<Wanted>,
    mut players: Query<(&mut Health, &mut Transform), With<Player>>,
) {
    let Ok((mut health, mut transform)) = players.single_mut() else {
        return;
    };
    if !health.is_dead() {
        return;
    }

    health.current = health.max;
    health.armor = 0.0;
    wanted.clear();

    // Back on the nearest street, the way waking up outside a hospital works.
    if let Some(node) = city.graph.nearest_node(transform.translation.xz()) {
        let position = city.graph.node(node).pos;
        transform.translation = Vec3::new(position.x, 2.0, position.y);
    }
    info!("wasted - respawned with a clean record");
}
