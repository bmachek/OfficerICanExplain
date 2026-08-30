//! Missions, markers, and money.

pub mod framework;
pub mod script;

use bevy::prelude::*;

use crate::core::schedule::GameSet;
use crate::crime::wanted::Wanted;
use crate::player::interact::Driving;
use crate::player::on_foot::Player;
use crate::world::City;
use framework::{ActiveMission, Objective, Progress, WorldSnapshot, maintain};

/// The player's cash.
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct Money(pub u32);

/// Which missions in the chain are done.
#[derive(Resource, Debug, Default)]
pub struct Campaign {
    pub completed: Vec<String>,
    pub next: usize,
}

/// The glowing cylinder marking a `Reach` objective.
#[derive(Component)]
pub struct MissionMarker;

pub struct MissionPlugin;

impl Plugin for MissionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Money>()
            .init_resource::<Campaign>()
            .add_systems(PostStartup, begin_chain)
            .add_systems(
                Update,
                (run_active_mission, place_marker)
                    .chain()
                    .in_set(GameSet::Simulation),
            );
    }
}

fn begin_chain(mut commands: Commands, city: Res<City>) {
    if let Some(first) = script::chain(&city).into_iter().next() {
        info!("mission available: {} - {}", first.name, first.brief);
        commands.insert_resource(ActiveMission::new(first));
    }
}

fn run_active_mission(
    mut commands: Commands,
    time: Res<Time>,
    city: Res<City>,
    wanted: Res<Wanted>,
    mut money: ResMut<Money>,
    mut campaign: ResMut<Campaign>,
    active: Option<ResMut<ActiveMission>>,
    players: Query<(&Transform, Option<&Driving>), With<Player>>,
) {
    let Some(mut active) = active else { return };
    let Ok((transform, driving)) = players.single() else {
        return;
    };

    let world = WorldSnapshot {
        player: transform.translation,
        in_vehicle: driving.is_some(),
        stars: wanted.stars(),
        dt: time.delta_secs(),
    };

    maintain(&mut active, &world);
    if active.advance(&world) != Progress::Complete {
        return;
    }

    // Paid, banked, and on to the next job.
    let reward = active.mission.reward;
    money.0 += reward;
    campaign.completed.push(active.mission.id.to_string());
    campaign.next += 1;
    info!(
        "mission complete: {} (+${reward}, balance ${})",
        active.mission.name, money.0
    );

    let chain = script::chain(&city);
    match chain.into_iter().nth(campaign.next) {
        Some(next) => {
            info!("mission available: {} - {}", next.name, next.brief);
            commands.insert_resource(ActiveMission::new(next));
        }
        None => {
            info!("all missions complete");
            commands.remove_resource::<ActiveMission>();
        }
    }
}

/// Keeps a single marker entity parked on the current `Reach` objective.
fn place_marker(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    active: Option<Res<ActiveMission>>,
    mut markers: Query<&mut Transform, With<MissionMarker>>,
) {
    let target = active.as_ref().and_then(|active| match active.current() {
        Some(Objective::Reach { position, .. }) => Some(*position),
        _ => None,
    });

    match (target, markers.iter_mut().next()) {
        (Some(position), Some(mut transform)) => {
            transform.translation = position + Vec3::Y * 4.0;
        }
        (Some(position), None) => {
            commands.spawn((
                Name::new("Mission Marker"),
                MissionMarker,
                Mesh3d(meshes.add(Cylinder::new(4.5, 8.0))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgba(1.0, 0.82, 0.2, 0.30),
                    emissive: LinearRgba::rgb(2.2, 1.7, 0.3),
                    alpha_mode: AlphaMode::Blend,
                    // A marker you can see through and drive into.
                    double_sided: true,
                    cull_mode: None,
                    ..default()
                })),
                Transform::from_translation(position + Vec3::Y * 4.0),
            ));
        }
        (None, Some(mut transform)) => {
            // Park it out of sight rather than churning the entity.
            transform.translation = Vec3::new(0.0, -500.0, 0.0);
        }
        (None, None) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use framework::{Mission, Objective};
    use std::time::Duration;

    /// Exercises the wiring between the tested rules and the world: reward
    /// payment, campaign bookkeeping, and hand-off to the next job.
    fn harness(objective: Objective) -> (App, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )));
        app.init_resource::<Money>()
            .init_resource::<Campaign>()
            .init_resource::<Wanted>()
            .insert_resource(City(crate::world::citygen::generate(7, 500.0)))
            .insert_resource(ActiveMission::new(Mission {
                id: "test_job",
                name: "Test Job",
                brief: "",
                objectives: vec![objective],
                reward: 1_250,
            }))
            .add_systems(Update, run_active_mission);

        let player = app
            .world_mut()
            .spawn((Player, Transform::from_xyz(0.0, 1.0, 0.0)))
            .id();

        app.finish();
        app.cleanup();
        (app, player)
    }

    #[test]
    fn completing_a_job_pays_out_and_records_it() {
        let target = Vec3::new(40.0, 0.0, 0.0);
        let (mut app, player) = harness(Objective::Reach {
            position: target,
            radius: 8.0,
            in_vehicle: false,
        });

        app.update();
        assert_eq!(app.world().resource::<Money>().0, 0, "not there yet");

        // Walk onto the marker.
        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = target;
        app.update();

        assert_eq!(app.world().resource::<Money>().0, 1_250, "reward not paid");
        let campaign = app.world().resource::<Campaign>();
        assert_eq!(campaign.completed, vec!["test_job".to_string()]);
        assert_eq!(campaign.next, 1);
    }

    #[test]
    fn finishing_the_chain_hands_off_to_the_next_job() {
        // The test mission is a one-off, so completing it should pull the real
        // chain's second entry in rather than leaving the player with nothing.
        let target = Vec3::new(25.0, 0.0, 0.0);
        let (mut app, player) = harness(Objective::Reach {
            position: target,
            radius: 8.0,
            in_vehicle: false,
        });

        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = target;
        app.update();
        // Commands from the system apply on the next frame.
        app.update();

        let active = app.world().get_resource::<ActiveMission>();
        assert!(
            active.is_some(),
            "a second mission should have been offered"
        );
        assert_eq!(active.unwrap().mission.id, "making_noise");
    }

    #[test]
    fn a_delivery_is_not_paid_out_on_foot() {
        let target = Vec3::new(30.0, 0.0, 0.0);
        let (mut app, player) = harness(Objective::Reach {
            position: target,
            radius: 8.0,
            in_vehicle: true,
        });

        app.world_mut()
            .get_mut::<Transform>(player)
            .unwrap()
            .translation = target;
        app.update();
        assert_eq!(
            app.world().resource::<Money>().0,
            0,
            "arriving without the car should not pay"
        );
    }
}
