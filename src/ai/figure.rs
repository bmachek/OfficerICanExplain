//! The shape of a person, and the way one walks.
//!
//! A pedestrian was a capsule. A capsule reads as a person at fifty metres and
//! as a bollard at five, and the difference matters because the whole point of
//! pedestrians is that the player gets close to them — witnesses, victims, the
//! crowd that scatters when a car mounts the kerb.
//!
//! So the figure is assembled from parts hung off the same entity the capsule
//! collider is still on. Nothing here is physical: the collider is unchanged,
//! and the limbs are decoration that follows it. That keeps every raycast, every
//! line-of-sight check and every piece of pursuit logic exactly as it was.
//!
//! Limbs pivot at their joint rather than their centre, which is why each one
//! is an entity at the shoulder or hip with the mesh hung *below* it. Rotating a
//! centred capsule swings it about its middle, and a leg that does that is not
//! walking, it is being stirred.
//!
//! Every part also carries its [`Rest`] pose, which is what makes the whole
//! figure squash and stretch as one. The alternative — scaling the body entity
//! — is not available: Avian scales a collider by its transform, so a figure
//! flattening at the bottom of a hop would flatten its own collider with it and
//! sink through the pavement.

use bevy::prelude::*;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

use crate::bounce::controller::Bouncer;

/// Metres walked per full stride cycle — one step of each foot.
const STRIDE: f32 = 1.45;
/// How far a leg swings at a walk, in radians.
const SWING: f32 = 0.62;
/// Arms swing less than legs, and opposite them.
const ARM_SWING: f32 = 0.44;

/// Where a limb hangs from.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Limb {
    LeftArm,
    RightArm,
    LeftLeg,
    RightLeg,
}

impl Limb {
    /// Legs lead, arms follow half a cycle behind, and the two sides oppose.
    fn phase_offset(self) -> f32 {
        use std::f32::consts::PI;
        match self {
            Limb::LeftLeg | Limb::RightArm => 0.0,
            Limb::RightLeg | Limb::LeftArm => PI,
        }
    }

    fn amplitude(self) -> f32 {
        match self {
            Limb::LeftLeg | Limb::RightLeg => SWING,
            Limb::LeftArm | Limb::RightArm => ARM_SWING,
        }
    }
}

/// The head. Marked because it is the one part of a figure that anything else
/// wants to find: it is where the face goes.
#[derive(Component)]
pub struct Head;

/// Where a part of a figure sits when the body is at its natural height.
///
/// Held per part rather than looked up from the part's kind, so that posing a
/// figure is one query over its children instead of a match with a arm for
/// every piece of anatomy.
#[derive(Component, Clone, Copy)]
pub struct Rest(pub Vec3);

/// How far through a stride this figure is, and how fast it is covering ground.
///
/// The speed is written by whoever owns the figure — the pedestrian AI for a
/// crowd, the character controller for the player — so the animation itself
/// does not need to know which it is looking at.
#[derive(Component, Default)]
pub struct WalkCycle {
    pub phase: f32,
    pub speed: f32,
}

#[derive(Resource)]
pub struct FigureAssets {
    torso: Handle<Mesh>,
    head: Handle<Mesh>,
    arm: Handle<Mesh>,
    leg: Handle<Mesh>,
    skin: Vec<Handle<StandardMaterial>>,
    trousers: Vec<Handle<StandardMaterial>>,
}

/// Proportions, in metres, measured from the middle of the collider capsule.
///
/// The capsule is unchanged, so these have to fit inside it: feet on its bottom
/// cap, head under its top. A figure that pokes out of its own collider is one
/// that can be shot through the head without being hit.
mod body {
    pub const FEET: f32 = -0.845;
    pub const HIP: f32 = -0.09;
    pub const SHOULDER: f32 = 0.34;
    pub const SHOULDER_X: f32 = 0.19;
    pub const TORSO_CENTRE: f32 = 0.16;
    pub const TORSO_HEIGHT: f32 = 0.52;
    pub const HEAD_CENTRE: f32 = 0.62;
    pub const HEAD_RADIUS: f32 = 0.13;
    pub const LEG_LENGTH: f32 = HIP - FEET;
    pub const ARM_LENGTH: f32 = 0.60;
}

/// Half the collider capsule's height, which is what the figure has to fit in.
const CAPSULE_HALF: f32 = 0.845;

// Checked at compile time rather than in a test: these are all constants, so
// there is nothing to run — and a figure that pokes out of its own collider is
// one whose head can be shot at without being hit.
const _: () = {
    assert!(
        body::FEET >= -CAPSULE_HALF,
        "the feet hang below the capsule"
    );
    assert!(
        body::HEAD_CENTRE + body::HEAD_RADIUS <= CAPSULE_HALF,
        "the head stands above the capsule"
    );
    assert!(
        body::TORSO_CENTRE + body::TORSO_HEIGHT * 0.5 < body::HEAD_CENTRE - body::HEAD_RADIUS,
        "the head is inside the chest"
    );
    assert!(
        body::LEG_LENGTH > 0.0 && body::ARM_LENGTH > 0.0,
        "a limb has no length"
    );
};

pub fn build_assets(
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> FigureAssets {
    let cloth = |color: Color| StandardMaterial {
        base_color: color,
        perceptual_roughness: 0.88,
        ..default()
    };

    FigureAssets {
        torso: meshes.add(Cuboid::new(0.36, body::TORSO_HEIGHT, 0.22)),
        head: meshes.add(Sphere::new(body::HEAD_RADIUS)),
        arm: meshes.add(Capsule3d {
            radius: 0.058,
            half_length: body::ARM_LENGTH * 0.5 - 0.058,
        }),
        leg: meshes.add(Capsule3d {
            radius: 0.078,
            half_length: body::LEG_LENGTH * 0.5 - 0.078,
        }),
        skin: [
            Color::srgb(0.76, 0.60, 0.48),
            Color::srgb(0.58, 0.42, 0.32),
            Color::srgb(0.38, 0.26, 0.19),
            Color::srgb(0.86, 0.72, 0.60),
        ]
        .into_iter()
        .map(|color| materials.add(cloth(color)))
        .collect(),
        trousers: [
            Color::srgb(0.16, 0.18, 0.24),
            Color::srgb(0.22, 0.20, 0.18),
            Color::srgb(0.12, 0.13, 0.14),
        ]
        .into_iter()
        .map(|color| materials.add(cloth(color)))
        .collect(),
    }
}

/// Hangs a figure off an entity that already has its collider and behaviour.
pub fn dress(
    entity: &mut EntityCommands,
    assets: &FigureAssets,
    coat: Handle<StandardMaterial>,
    rng: &mut ChaCha8Rng,
) {
    let skin = assets.skin[rng.random_range(0..assets.skin.len())].clone();
    let trousers = assets.trousers[rng.random_range(0..assets.trousers.len())].clone();

    entity.insert(WalkCycle::default());
    entity.with_children(|parent| {
        let torso = Vec3::new(0.0, body::TORSO_CENTRE, 0.0);
        parent.spawn((
            Rest(torso),
            Mesh3d(assets.torso.clone()),
            MeshMaterial3d(coat.clone()),
            Transform::from_translation(torso),
        ));
        let head = Vec3::new(0.0, body::HEAD_CENTRE, 0.0);
        parent.spawn((
            Head,
            Rest(head),
            Mesh3d(assets.head.clone()),
            MeshMaterial3d(skin.clone()),
            Transform::from_translation(head),
        ));

        for (limb, side) in [(Limb::LeftArm, -1.0f32), (Limb::RightArm, 1.0)] {
            let joint = Vec3::new(side * body::SHOULDER_X, body::SHOULDER, 0.0);
            parent
                .spawn((
                    limb,
                    Rest(joint),
                    Transform::from_translation(joint),
                    Visibility::default(),
                ))
                // Hung below the joint, so the parent's rotation swings it from
                // the shoulder rather than spinning it about its own middle.
                .with_child((
                    Mesh3d(assets.arm.clone()),
                    MeshMaterial3d(coat.clone()),
                    Transform::from_xyz(0.0, -body::ARM_LENGTH * 0.5, 0.0),
                ));
        }

        for (limb, side) in [(Limb::LeftLeg, -1.0f32), (Limb::RightLeg, 1.0)] {
            let joint = Vec3::new(side * 0.10, body::HIP, 0.0);
            parent
                .spawn((
                    limb,
                    Rest(joint),
                    Transform::from_translation(joint),
                    Visibility::default(),
                ))
                .with_child((
                    Mesh3d(assets.leg.clone()),
                    MeshMaterial3d(trousers.clone()),
                    Transform::from_xyz(0.0, -body::LEG_LENGTH * 0.5, 0.0),
                ));
        }
    });
}

/// Angle a limb is swung to, at a given point in the stride.
pub fn limb_angle(limb: Limb, phase: f32) -> f32 {
    (phase + limb.phase_offset()).sin() * limb.amplitude()
}

/// Advances every figure's stride, poses its limbs, and squashes it into
/// whatever part of its hop it is in.
///
/// The stride and the hop are independent on purpose. The legs are paced by
/// ground covered and the squash by the bounce, so a flummi sailing across a
/// junction is still running in mid-air — which is both what a cartoon does and
/// what the walk cycle would do anyway if asked.
pub fn animate(
    time: Res<Time>,
    config: Res<crate::core::config::GameConfig>,
    figures: Query<(&mut WalkCycle, Option<&Bouncer>, &Children)>,
    mut parts: Query<(&mut Transform, &Rest, Option<&Limb>)>,
) {
    let dt = time.delta_secs();
    for (mut cycle, bouncer, children) in figures {
        // Driven by distance covered, not by time: someone running has to take
        // faster steps, not longer ones, or they moonwalk.
        cycle.phase = (cycle.phase + cycle.speed / STRIDE * TAU_F32 * dt) % TAU_F32;

        let (vertical, horizontal) = match bouncer {
            Some(bouncer) => {
                crate::bounce::squash::stretch(bouncer.hop_phase(), config.bounce.squash)
            }
            None => (1.0, 1.0),
        };
        let pose = Vec3::new(horizontal, vertical, horizontal);

        for &child in children {
            let Ok((mut transform, rest, limb)) = parts.get_mut(child) else {
                continue;
            };
            // The rest pose scaled by the squash, so parts stay attached to one
            // another as the body flattens instead of pulling apart at the neck.
            transform.translation = rest.0 * pose;
            transform.scale = pose;
            if let Some(limb) = limb {
                transform.rotation = Quat::from_rotation_x(limb_angle(*limb, cycle.phase));
            }
        }
    }
}

const TAU_F32: f32 = std::f32::consts::TAU;

/// Paces the crowd's figures from what the pedestrian AI decided this frame.
pub fn pace_pedestrians(mut walkers: Query<(&super::pedestrian::Pedestrian, &mut WalkCycle)>) {
    for (pedestrian, mut cycle) in &mut walkers {
        cycle.speed = pedestrian.current_speed;
    }
}

/// And the player's, from how fast they are actually moving.
///
/// Read off the body rather than off the input, so being shoved by a car or
/// sliding down a kerb moves the legs too.
pub fn pace_player(
    mut player: Query<
        (&avian3d::prelude::LinearVelocity, &mut WalkCycle),
        With<crate::player::on_foot::Player>,
    >,
) {
    for (velocity, mut cycle) in &mut player {
        cycle.speed = velocity.0.xz().length();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_legs_are_always_out_of_step_with_each_other() {
        // Both legs forward at once is a hop, not a walk.
        for step in 0..64 {
            let phase = TAU_F32 * step as f32 / 64.0;
            let left = limb_angle(Limb::LeftLeg, phase);
            let right = limb_angle(Limb::RightLeg, phase);
            assert!(
                (left + right).abs() < 1e-5,
                "at phase {phase:.2} the legs are at {left:.2} and {right:.2}"
            );
        }
    }

    #[test]
    fn an_arm_swings_opposite_the_leg_on_the_same_side() {
        // Same-side arm and leg swinging together is the walk of someone
        // thinking very hard about walking.
        for step in 0..64 {
            let phase = TAU_F32 * step as f32 / 64.0;
            let arm = limb_angle(Limb::LeftArm, phase);
            let leg = limb_angle(Limb::LeftLeg, phase);
            assert!(
                arm * leg <= 1e-6,
                "at phase {phase:.2} the left arm and leg both went {arm:.2}/{leg:.2}"
            );
        }
    }

    #[test]
    fn arms_swing_less_than_legs() {
        let peak = |limb: Limb| {
            (0..256)
                .map(|i| limb_angle(limb, TAU_F32 * i as f32 / 256.0).abs())
                .fold(0.0, f32::max)
        };
        assert!(peak(Limb::LeftArm) < peak(Limb::LeftLeg));
    }

    #[test]
    fn the_head_rides_above_the_chest_at_every_squash() {
        // The parts are posed by scaling one rest pose, so they can only come
        // apart if the scale is applied to one of them and not the other.
        for step in 0..16 {
            let (vertical, _) = crate::bounce::squash::stretch(step as f32 / 16.0, 0.35);
            let head = body::HEAD_CENTRE * vertical;
            let chest = body::TORSO_CENTRE * vertical;
            assert!(
                head > chest,
                "the head sank into the chest at {vertical:.2}"
            );
        }
    }
}
