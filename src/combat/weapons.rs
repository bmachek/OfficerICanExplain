//! Hitscan weapons.
//!
//! Bullets are raycasts, not projectiles. At the ranges and speeds this game
//! operates at, a simulated projectile is indistinguishable from an instant
//! ray, and rays cost nothing and cannot tunnel through a thin wall.
//!
//! Firing always raises a `Gunfire` crime whether or not it hits, and gunfire
//! needs no witness — the noise is the witness.

use avian3d::prelude::*;
use bevy::prelude::*;
use leafwing_input_manager::prelude::ActionState;

use super::health::{Died, Health};
use crate::core::schedule::GameSet;
use crate::crime::events::{CrimeKind, CrimeReported};
use crate::player::camera::CameraRig;
use crate::player::input::Action;
use crate::player::on_foot::Player;
use crate::vehicle::damage::VehicleHealth;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeaponKind {
    Pistol,
    Smg,
}

impl WeaponKind {
    pub fn damage(self) -> f32 {
        match self {
            WeaponKind::Pistol => 34.0,
            WeaponKind::Smg => 18.0,
        }
    }
    pub fn range(self) -> f32 {
        match self {
            WeaponKind::Pistol => 90.0,
            WeaponKind::Smg => 65.0,
        }
    }
    /// Seconds between shots.
    pub fn cooldown(self) -> f32 {
        match self {
            WeaponKind::Pistol => 0.32,
            WeaponKind::Smg => 0.085,
        }
    }
    /// Whether holding the trigger keeps firing.
    pub fn is_automatic(self) -> bool {
        matches!(self, WeaponKind::Smg)
    }
    pub fn name(self) -> &'static str {
        match self {
            WeaponKind::Pistol => "Pistol",
            WeaponKind::Smg => "SMG",
        }
    }
}

#[derive(Component, Debug)]
pub struct Weapon {
    pub kind: WeaponKind,
    pub ammo: u32,
    pub since_shot: f32,
}

impl Weapon {
    pub fn new(kind: WeaponKind, ammo: u32) -> Self {
        Self {
            kind,
            ammo,
            since_shot: kind.cooldown(),
        }
    }

    /// Whether the trigger state should produce a shot this frame.
    pub fn wants_to_fire(&self, held: bool, pressed_now: bool) -> bool {
        if self.ammo == 0 || self.since_shot < self.kind.cooldown() {
            return false;
        }
        if self.kind.is_automatic() {
            held
        } else {
            pressed_now
        }
    }
}

/// A fading bullet trail.
#[derive(Component)]
pub struct Tracer(pub Timer);

pub struct WeaponsPlugin;

impl Plugin for WeaponsPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Died>().add_systems(
            Update,
            (fire_weapons, fade_tracers).in_set(GameSet::Simulation),
        );
    }
}

fn fire_weapons(
    mut commands: Commands,
    time: Res<Time>,
    spatial: SpatialQuery,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut crimes: MessageWriter<CrimeReported>,
    mut deaths: MessageWriter<Died>,
    cameras: Query<(&GlobalTransform, &CameraRig)>,
    mut shooters: Query<(Entity, &ActionState<Action>, &mut Weapon), With<Player>>,
    mut targets: Query<(&mut Health, &GlobalTransform)>,
    mut vehicles: Query<&mut VehicleHealth>,
) {
    let Ok((entity, action_state, mut weapon)) = shooters.single_mut() else {
        return;
    };
    weapon.since_shot += time.delta_secs();

    let held = action_state.pressed(&Action::Fire);
    let pressed_now = action_state.just_pressed(&Action::Fire);
    if !weapon.wants_to_fire(held, pressed_now) {
        return;
    }
    let Ok((camera, _)) = cameras.single() else {
        return;
    };

    weapon.since_shot = 0.0;
    weapon.ammo = weapon.ammo.saturating_sub(1);

    // Aim down the camera: what the player sees is what they hit.
    let origin = camera.translation();
    let Ok(direction) = Dir3::new(camera.forward().as_vec3()) else {
        return;
    };
    let range = weapon.kind.range();
    let filter = SpatialQueryFilter::from_excluded_entities([entity]);

    let mut endpoint = origin + direction * range;
    if let Some(hit) = spatial.cast_ray(origin, direction, range, true, &filter) {
        endpoint = origin + direction * hit.distance;

        if let Ok(mut vehicle) = vehicles.get_mut(hit.entity) {
            vehicle.current -= weapon.kind.damage();
        } else if let Ok((mut health, transform)) = targets.get_mut(hit.entity)
            && health.damage(weapon.kind.damage())
        {
            deaths.write(Died {
                entity: hit.entity,
                position: transform.translation(),
                by_player: true,
            });
        }
    }

    spawn_tracer(&mut commands, &mut meshes, &mut materials, origin, endpoint);

    // Gunfire is its own crime, hit or miss.
    crimes.write(CrimeReported {
        kind: CrimeKind::Gunfire,
        position: origin,
    });
}

fn spawn_tracer(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    from: Vec3,
    to: Vec3,
) {
    let delta = to - from;
    let length = delta.length();
    if length < 0.1 {
        return;
    }

    commands.spawn((
        Tracer(Timer::from_seconds(0.07, TimerMode::Once)),
        Mesh3d(meshes.add(Cylinder::new(0.02, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.9, 0.5),
            emissive: LinearRgba::rgb(9.0, 7.0, 2.5),
            ..default()
        })),
        Transform::from_translation(from + delta * 0.5)
            // Cylinders run along Y; point it down the shot.
            .with_rotation(Quat::from_rotation_arc(Vec3::Y, delta / length))
            .with_scale(Vec3::new(1.0, length, 1.0)),
    ));
}

fn fade_tracers(
    mut commands: Commands,
    time: Res<Time>,
    mut tracers: Query<(Entity, &mut Tracer)>,
) {
    for (entity, mut tracer) in &mut tracers {
        if tracer.0.tick(time.delta()).is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pistol_fires_once_per_press() {
        let mut weapon = Weapon::new(WeaponKind::Pistol, 12);
        assert!(weapon.wants_to_fire(true, true), "the press should fire");

        weapon.since_shot = 0.0;
        assert!(
            !weapon.wants_to_fire(true, false),
            "holding must not auto-fire a pistol"
        );
    }

    #[test]
    fn an_smg_keeps_firing_while_held() {
        let mut weapon = Weapon::new(WeaponKind::Smg, 30);
        assert!(weapon.wants_to_fire(true, true));
        weapon.since_shot = WeaponKind::Smg.cooldown();
        assert!(weapon.wants_to_fire(true, false), "held should keep firing");
    }

    #[test]
    fn the_cooldown_is_respected() {
        let mut weapon = Weapon::new(WeaponKind::Smg, 30);
        weapon.since_shot = WeaponKind::Smg.cooldown() * 0.5;
        assert!(!weapon.wants_to_fire(true, true));
    }

    #[test]
    fn an_empty_weapon_does_nothing() {
        let weapon = Weapon::new(WeaponKind::Pistol, 0);
        assert!(!weapon.wants_to_fire(true, true));
    }

    #[test]
    fn an_smg_puts_out_more_damage_per_second_than_a_pistol() {
        let dps = |kind: WeaponKind| kind.damage() / kind.cooldown();
        assert!(
            dps(WeaponKind::Smg) > dps(WeaponKind::Pistol),
            "the trade for lower per-shot damage should be rate"
        );
        assert!(
            WeaponKind::Pistol.range() > WeaponKind::Smg.range(),
            "and the pistol should reach further"
        );
    }
}
