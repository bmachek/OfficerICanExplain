//! Crash damage and explosions.
//!
//! Damage is taken from sudden changes in velocity rather than from collision
//! events. A crash is defined by how hard the car stops, which is exactly what
//! a velocity delta measures — and it catches every way a car can be wrecked
//! (walls, other cars, landing badly from a jump) through one code path,
//! instead of needing a separate rule per collision pair.
//!
//! Braking is nowhere near this threshold: a hard stop is about 1g, or 0.15
//! m/s of velocity change per tick, while hitting a wall at 70km/h sheds ten
//! times that in a single tick.

use avian3d::prelude::*;
use bevy::prelude::*;

use super::controller::VehicleState;
use super::spawn::{ActiveVehicle, Vehicle};
use super::spec::VehicleSpec;

/// Velocity change in one tick below which an impact does no damage.
const IMPACT_THRESHOLD: f32 = 2.5;
/// Damage per (m/s) of velocity lost beyond the threshold.
const DAMAGE_PER_IMPACT: f32 = 7.0;
/// How far an explosion throws things.
const BLAST_RADIUS: f32 = 9.0;
const BLAST_IMPULSE: f32 = 5_500.0;

#[derive(Component, Debug, Clone)]
pub struct VehicleHealth {
    pub current: f32,
    pub max: f32,
}

impl Default for VehicleHealth {
    fn default() -> Self {
        Self {
            current: 100.0,
            max: 100.0,
        }
    }
}

impl VehicleHealth {
    pub fn fraction(&self) -> f32 {
        (self.current / self.max).clamp(0.0, 1.0)
    }
    /// Smoking, but still driveable.
    pub fn is_critical(&self) -> bool {
        self.fraction() < 0.3
    }
    pub fn is_wrecked(&self) -> bool {
        self.current <= 0.0
    }
}

/// Last tick's velocity, used to spot impacts.
#[derive(Component, Default)]
pub struct PreviousVelocity(pub Vec3);

/// A panel that crash damage is allowed to deform and scuff.
#[derive(Component)]
pub struct BodyPanel;

/// On a panel whose mesh is this car's alone.
///
/// Every car of an archetype shares one mesh, which is what lets a street of
/// them batch into a single draw call. A dent cannot be written into that, so
/// the first real impact copies the mesh for that car only. Damaged cars stop
/// batching with the rest; there are never many, and the alternative is giving
/// five hundred parked cars their own copy against the chance one gets hit.
#[derive(Component)]
pub struct OwnPanels;

/// Fired for every impact hard enough to do damage.
///
/// Separate from [`VehicleDestroyed`] because most crashes are survivable, and
/// a crash the player walks away from still has to be heard.
#[derive(Message, Debug, Clone, Copy)]
pub struct VehicleImpact {
    pub vehicle: Entity,
    pub position: Vec3,
    /// Where the blow arrived from, in the car's own frame. Read off the
    /// velocity change: whichever way the car was shoved, the thing it hit was
    /// on the other side.
    pub from: Vec3,
    /// Velocity lost in the impact, in m/s. A scrape is a couple; hitting a
    /// wall at speed is twenty.
    pub severity: f32,
}

/// Fired when a vehicle is destroyed. M5's wanted system listens to this.
#[derive(Message, Debug, Clone, Copy)]
pub struct VehicleDestroyed {
    pub vehicle: Entity,
    pub position: Vec3,
}

/// A fading explosion flash.
#[derive(Component)]
pub struct Explosion {
    pub life: Timer,
}

pub fn apply_crash_damage(
    mut impacts: MessageWriter<VehicleImpact>,
    mut vehicles: Query<
        (
            Entity,
            &LinearVelocity,
            &mut PreviousVelocity,
            &mut VehicleHealth,
            &VehicleState,
            &Transform,
        ),
        (With<Vehicle>, With<ActiveVehicle>),
    >,
) {
    for (entity, velocity, mut previous, mut health, state, transform) in &mut vehicles {
        let change = velocity.0 - previous.0;
        let delta = change.length();
        previous.0 = velocity.0;

        if health.is_wrecked() {
            continue;
        }
        // Ignore the first tick after activation, when previous velocity is
        // meaningless, and airborne landings on all four wheels.
        if delta > IMPACT_THRESHOLD && state.grounded_wheels() > 0 {
            health.current -= (delta - IMPACT_THRESHOLD) * DAMAGE_PER_IMPACT;
            impacts.write(VehicleImpact {
                vehicle: entity,
                position: transform.translation,
                from: transform.rotation.inverse() * (-change / delta),
                severity: delta - IMPACT_THRESHOLD,
            });
        }
    }
}

pub fn explode_wrecked_vehicles(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut destroyed: MessageWriter<VehicleDestroyed>,
    wrecked: Query<(Entity, &Transform, &VehicleHealth), With<Vehicle>>,
    mut nearby: Query<(&Transform, Forces), With<RigidBody>>,
) {
    for (entity, transform, health) in &wrecked {
        if !health.is_wrecked() {
            continue;
        }
        let center = transform.translation;

        commands.entity(entity).despawn();
        destroyed.write(VehicleDestroyed {
            vehicle: entity,
            position: center,
        });

        // Flash: a bright, short-lived light plus an expanding shell. Cheap,
        // and reads far better than a particle system at this art fidelity.
        commands.spawn((
            Name::new("Explosion"),
            Explosion {
                life: Timer::from_seconds(0.6, TimerMode::Once),
            },
            PointLight {
                color: Color::srgb(1.0, 0.65, 0.25),
                intensity: 4_000_000.0,
                range: 40.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Mesh3d(meshes.add(Sphere::new(1.0))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.55, 0.2),
                emissive: LinearRgba::rgb(12.0, 5.0, 1.5),
                ..default()
            })),
            Transform::from_translation(center),
        ));

        // Shove whatever is close enough to care.
        for (other, mut forces) in &mut nearby {
            let offset = other.translation - center;
            let distance = offset.length();
            if !(f32::EPSILON..=BLAST_RADIUS).contains(&distance) {
                continue;
            }
            let falloff = 1.0 - distance / BLAST_RADIUS;
            // Bias upward so things are thrown clear rather than scraped along.
            let direction = (offset / distance + Vec3::Y * 0.6).normalize_or_zero();
            forces.apply_linear_impulse(direction * BLAST_IMPULSE * falloff);
        }
    }
}

pub fn fade_explosions(
    mut commands: Commands,
    time: Res<Time>,
    mut explosions: Query<(Entity, &mut Explosion, &mut Transform, &mut PointLight)>,
) {
    for (entity, mut explosion, mut transform, mut light) in &mut explosions {
        explosion.life.tick(time.delta());
        let remaining = explosion.life.fraction_remaining();

        // Expands as it dims.
        transform.scale = Vec3::splat(1.5 + 5.0 * (1.0 - remaining));
        light.intensity = 4_000_000.0 * remaining * remaining;

        if explosion.life.is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

/// Deepest a single impact can push a panel in, in metres.
const MAX_DENT: f32 = 0.34;
/// How far a dent spreads across the bodywork.
const DENT_RADIUS: f32 = 1.15;
/// Velocity loss that produces a full-depth dent.
const DENT_AT: f32 = 16.0;

/// Beats a dent into whichever panels took the blow.
pub fn dent_bodywork(
    mut commands: Commands,
    mut impacts: MessageReader<VehicleImpact>,
    mut meshes: ResMut<Assets<Mesh>>,
    vehicles: Query<(&Children, Has<OwnPanels>), With<Vehicle>>,
    mut panels: Query<&mut Mesh3d, With<BodyPanel>>,
) {
    for impact in impacts.read() {
        let Ok((children, owns_panels)) = vehicles.get(impact.vehicle) else {
            continue;
        };
        let depth = (impact.severity / DENT_AT).clamp(0.0, 1.0) * MAX_DENT;
        if depth < 0.02 {
            continue;
        }

        if !owns_panels {
            commands.entity(impact.vehicle).insert(OwnPanels);
        }

        for &child in children {
            let Ok(mut panel) = panels.get_mut(child) else {
                continue;
            };
            // Copy-on-write: the first hit takes this car off the shared mesh.
            if !owns_panels {
                let Some(own) = meshes.get(&panel.0).cloned() else {
                    continue;
                };
                panel.0 = meshes.add(own);
            }
            if let Some(mut mesh) = meshes.get_mut(&panel.0) {
                super::body::dent(&mut mesh, impact.from, depth, DENT_RADIUS);
            }
        }
    }
}

/// Takes the shine off a car as it comes apart.
///
/// Paint is per-car already, so this needs no copy-on-write. Everything is
/// recomputed from the spec rather than nudged from the current value: a
/// material edited in place drifts, and a car that has been repaired would
/// stay dull.
pub fn scuff_paint(
    mut materials: ResMut<Assets<StandardMaterial>>,
    vehicles: Query<(&VehicleHealth, &VehicleSpec, &Children), Changed<VehicleHealth>>,
    panels: Query<&MeshMaterial3d<StandardMaterial>, With<BodyPanel>>,
) {
    for (health, spec, children) in &vehicles {
        let hurt = 1.0 - health.fraction();
        for &child in children {
            let Ok(handle) = panels.get(child) else {
                continue;
            };
            let Some(mut paint) = materials.get_mut(&handle.0) else {
                continue;
            };

            // Lacquer goes first, then the flake stops reading, then the
            // colour cooks off towards soot.
            paint.clearcoat = (1.0 - hurt * 1.4).clamp(0.0, 1.0);
            paint.metallic = spec.body_metallic * (1.0 - hurt * 0.8).max(0.0);
            paint.perceptual_roughness =
                (0.30 + spec.body_metallic * 0.22 + hurt * 0.55).clamp(0.0, 1.0);

            let clean = LinearRgba::from(spec.body_color);
            let soot = LinearRgba::rgb(0.035, 0.032, 0.030);
            let burn = (hurt - 0.35).max(0.0) / 0.65;
            paint.base_color = Color::LinearRgba(LinearRgba::rgb(
                clean.red.lerp(soot.red, burn),
                clean.green.lerp(soot.green, burn),
                clean.blue.lerp(soot.blue, burn),
            ));
        }
    }
}

/// A puff of smoke from a dying engine.
#[derive(Component)]
pub struct Smoke {
    pub life: Timer,
}

/// How often a critical car coughs out a puff, and what a puff is made of.
///
/// The mesh is shared; the material cannot be, because each puff fades on its
/// own schedule and alpha lives in the material.
#[derive(Resource)]
pub struct SmokeAssets {
    timer: Timer,
    puff: Handle<Mesh>,
}

impl SmokeAssets {
    pub fn new(meshes: &mut Assets<Mesh>) -> Self {
        Self {
            timer: Timer::from_seconds(0.16, TimerMode::Repeating),
            puff: meshes.add(Sphere::new(0.28)),
        }
    }
}

/// Smoke off the bonnet of anything running on its last thirty percent.
///
/// Puffs are unlit, unshadowed, alpha-blended spheres that rise and swell.
/// A particle system would buy softer edges for a great deal more machinery,
/// and at the distance a smoking car is read from — far enough away that it is
/// a cue rather than a subject — the edges are not what sells it.
pub fn smoke_from_dying_engines(
    mut commands: Commands,
    time: Res<Time>,
    mut smoke: ResMut<SmokeAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    vehicles: Query<(&Transform, &VehicleSpec, &VehicleHealth), With<ActiveVehicle>>,
) {
    if !smoke.timer.tick(time.delta()).just_finished() {
        return;
    }
    for (transform, spec, health) in &vehicles {
        if !health.is_critical() || health.is_wrecked() {
            continue;
        }
        // Out of the bonnet, not the middle of the roof.
        let vent = transform.transform_point(Vec3::new(
            0.0,
            spec.half_extents.y * 0.4,
            -spec.half_extents.z * 0.6,
        ));
        commands.spawn((
            Name::new("Smoke"),
            Smoke {
                life: Timer::from_seconds(1.6, TimerMode::Once),
            },
            Mesh3d(smoke.puff.clone()),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgba(0.10, 0.10, 0.11, 0.55),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            })),
            Transform::from_translation(vent),
        ));
    }
}

pub fn fade_smoke(
    mut commands: Commands,
    time: Res<Time>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut puffs: Query<(
        Entity,
        &mut Smoke,
        &mut Transform,
        &MeshMaterial3d<StandardMaterial>,
    )>,
) {
    for (entity, mut smoke, mut transform, handle) in &mut puffs {
        smoke.life.tick(time.delta());
        let remaining = smoke.life.fraction_remaining();

        transform.translation.y += 1.4 * time.delta_secs();
        transform.scale = Vec3::splat(1.0 + 2.2 * (1.0 - remaining));
        if let Some(mut material) = materials.get_mut(&handle.0) {
            material.base_color.set_alpha(0.5 * remaining * remaining);
        }

        if smoke.life.is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_reports_its_state() {
        let mut health = VehicleHealth::default();
        assert!(!health.is_critical() && !health.is_wrecked());

        health.current = 20.0;
        assert!(health.is_critical() && !health.is_wrecked());

        health.current = 0.0;
        assert!(health.is_wrecked());
        assert_eq!(health.fraction(), 0.0);
    }

    #[test]
    fn a_hard_stop_is_not_a_crash() {
        // 1g of braking at 64Hz is about 0.15 m/s per tick. If the threshold
        // ever drops near that, braking would destroy the car.
        let braking_delta_per_tick = 9.81 / 64.0;
        assert!(
            IMPACT_THRESHOLD > braking_delta_per_tick * 10.0,
            "impact threshold is close enough to braking to trigger on it"
        );
    }

    /// Total damage from shedding `speed` m/s spread evenly over `ticks`.
    /// A real impact is not instantaneous — the solver bleeds the velocity off
    /// over a few ticks — so damage has to be summed the same way.
    fn crash_damage(speed: f32, ticks: u32) -> f32 {
        let per_tick = speed / ticks as f32;
        if per_tick <= IMPACT_THRESHOLD {
            return 0.0;
        }
        (per_tick - IMPACT_THRESHOLD) * DAMAGE_PER_IMPACT * ticks as f32
    }

    #[test]
    fn a_serious_impact_wrecks_a_car() {
        let max = VehicleHealth::default().max;
        // 90km/h head-on, however many ticks the solver takes to stop it.
        for ticks in 2..=4 {
            let damage = crash_damage(25.0, ticks);
            assert!(
                damage >= max,
                "a 90km/h head-on over {ticks} ticks dealt only {damage}"
            );
        }
    }

    #[test]
    fn a_moderate_shunt_hurts_without_destroying() {
        // 40km/h into something solid: expensive, not fatal.
        let damage = crash_damage(11.0, 3);
        let max = VehicleHealth::default().max;
        assert!(
            damage > max * 0.15 && damage < max,
            "a 40km/h shunt dealt {damage} of {max}"
        );
    }

    #[test]
    fn a_minor_knock_is_survivable() {
        let delta = 4.0;
        let damage = (delta - IMPACT_THRESHOLD) * DAMAGE_PER_IMPACT;
        assert!(
            damage < VehicleHealth::default().max * 0.25,
            "a light bump took {damage} health"
        );
    }
}
