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

use bevy::prelude::*;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

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

/// The upper body, which rises and falls with the stride.
#[derive(Component)]
pub struct Torso;

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
    hand: Handle<Mesh>,
    hair: Handle<Mesh>,
    shoe: Handle<Mesh>,
    skin: Vec<Handle<StandardMaterial>>,
    trousers: Vec<Handle<StandardMaterial>>,
    hair_colours: Vec<Handle<StandardMaterial>>,
    leather: Handle<StandardMaterial>,
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
    pub const HAND_RADIUS: f32 = 0.062;
    /// A cap rather than a hairstyle: at the distance a pedestrian is normally
    /// seen, the only thing hair does is stop the head reading as a bare ball.
    pub const HAIR_RADIUS: f32 = 0.134;
    pub const HAIR_FLATTEN: f32 = 0.74;
    pub const HAIR_RISE: f32 = 0.026;
    pub const SHOE_HEIGHT: f32 = 0.062;
    pub const SHOE_LENGTH: f32 = 0.245;
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
    assert!(
        body::HEAD_CENTRE + body::HAIR_RISE + body::HAIR_RADIUS * body::HAIR_FLATTEN
            <= CAPSULE_HALF,
        "the hair stands above the capsule"
    );
    assert!(
        body::FEET + body::SHOE_HEIGHT * 0.5 >= -CAPSULE_HALF,
        "the shoes sink through the capsule"
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
    // Skin is not cloth. At 0.88 a face is as matte as a wool coat and takes no
    // highlight at all, which is most of why a figure reads as a mannequin —
    // real skin is closer to a half-gloss, with an oily sheen on the forehead
    // and nose that a sphere cannot describe but a roughness can hint at.
    let skin = |color: Color| StandardMaterial {
        base_color: color,
        perceptual_roughness: 0.52,
        // Slightly above the dielectric default, because skin is wet.
        reflectance: 0.55,
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
        hand: meshes.add(Sphere::new(body::HAND_RADIUS)),
        hair: meshes.add(Sphere::new(body::HAIR_RADIUS)),
        shoe: meshes.add(Cuboid::new(0.105, body::SHOE_HEIGHT, body::SHOE_LENGTH)),
        skin: [
            Color::srgb(0.76, 0.60, 0.48),
            Color::srgb(0.58, 0.42, 0.32),
            Color::srgb(0.38, 0.26, 0.19),
            Color::srgb(0.86, 0.72, 0.60),
        ]
        .into_iter()
        .map(|color| materials.add(skin(color)))
        .collect(),
        hair_colours: [
            Color::srgb(0.07, 0.06, 0.06),
            Color::srgb(0.19, 0.13, 0.09),
            Color::srgb(0.35, 0.26, 0.16),
            Color::srgb(0.52, 0.49, 0.47),
        ]
        .into_iter()
        // Hair is matte and dark, and it is the darkness that does the work:
        // what a cap of it buys is a head that ends in a shape instead of
        // fading into whatever is behind it.
        .map(|color| materials.add(cloth(color)))
        .collect(),
        leather: materials.add(StandardMaterial {
            base_color: Color::srgb(0.09, 0.08, 0.08),
            perceptual_roughness: 0.64,
            ..default()
        }),
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
    let hair = assets.hair_colours[rng.random_range(0..assets.hair_colours.len())].clone();

    entity.insert(WalkCycle::default());
    entity.with_children(|parent| {
        parent.spawn((
            Torso,
            Mesh3d(assets.torso.clone()),
            MeshMaterial3d(coat.clone()),
            Transform::from_xyz(0.0, body::TORSO_CENTRE, 0.0),
        ));
        parent.spawn((
            Torso,
            Mesh3d(assets.head.clone()),
            MeshMaterial3d(skin.clone()),
            Transform::from_xyz(0.0, body::HEAD_CENTRE, 0.0),
        ));
        // Sat a little high and a little back, so it reads as hair rather than
        // as a helmet: the forehead stays bare and the crown does not.
        parent.spawn((
            Torso,
            Mesh3d(assets.hair.clone()),
            MeshMaterial3d(hair.clone()),
            Transform::from_xyz(0.0, body::HEAD_CENTRE + body::HAIR_RISE, 0.018)
                .with_scale(Vec3::new(1.0, body::HAIR_FLATTEN, 1.0)),
        ));

        for (limb, side) in [(Limb::LeftArm, -1.0f32), (Limb::RightArm, 1.0)] {
            parent
                .spawn((
                    limb,
                    Transform::from_xyz(side * body::SHOULDER_X, body::SHOULDER, 0.0),
                    Visibility::default(),
                ))
                // Hung below the joint, so the parent's rotation swings it from
                // the shoulder rather than spinning it about its own middle.
                .with_children(|joint| {
                    joint.spawn((
                        Mesh3d(assets.arm.clone()),
                        MeshMaterial3d(coat.clone()),
                        Transform::from_xyz(0.0, -body::ARM_LENGTH * 0.5, 0.0),
                    ));
                    // A sleeve that ends in nothing is the other half of why
                    // a figure reads as a shop dummy. The hand is one sphere
                    // and it swings with the arm because it hangs off the
                    // same joint.
                    joint.spawn((
                        Mesh3d(assets.hand.clone()),
                        MeshMaterial3d(skin.clone()),
                        Transform::from_xyz(0.0, -body::ARM_LENGTH, 0.0),
                    ));
                });
        }

        for (limb, side) in [(Limb::LeftLeg, -1.0f32), (Limb::RightLeg, 1.0)] {
            parent
                .spawn((
                    limb,
                    Transform::from_xyz(side * 0.10, body::HIP, 0.0),
                    Visibility::default(),
                ))
                .with_children(|joint| {
                    joint.spawn((
                        Mesh3d(assets.leg.clone()),
                        MeshMaterial3d(trousers.clone()),
                        Transform::from_xyz(0.0, -body::LEG_LENGTH * 0.5, 0.0),
                    ));
                    // Toes forward, and the sole exactly on the capsule's
                    // bottom cap — which is where the ground check puts the
                    // figure, so this is the one part that has to be right or
                    // everybody walks on their ankles.
                    joint.spawn((
                        Mesh3d(assets.shoe.clone()),
                        MeshMaterial3d(assets.leather.clone()),
                        Transform::from_xyz(
                            0.0,
                            -body::LEG_LENGTH + body::SHOE_HEIGHT * 0.5,
                            -body::SHOE_LENGTH * 0.22,
                        ),
                    ));
                });
        }
    });
}

/// Angle a limb is swung to, at a given point in the stride.
pub fn limb_angle(limb: Limb, phase: f32) -> f32 {
    (phase + limb.phase_offset()).sin() * limb.amplitude()
}

/// How much the body rises at a given point in the stride.
///
/// Twice per cycle, because the body lifts over each leg in turn — a bob at the
/// stride frequency is a limp.
pub fn bob(phase: f32) -> f32 {
    (phase * 2.0).cos() * 0.022
}

/// Advances every figure's stride and poses its limbs.
pub fn animate(
    time: Res<Time>,
    figures: Query<(&mut WalkCycle, &Children)>,
    mut limbs: Query<(&Limb, &mut Transform), Without<Torso>>,
    mut torsos: Query<&mut Transform, (With<Torso>, Without<Limb>)>,
) {
    let dt = time.delta_secs();
    for (mut cycle, children) in figures {
        // Driven by distance covered, not by time: someone running has to take
        // faster steps, not longer ones, or they moonwalk.
        cycle.phase = (cycle.phase + cycle.speed / STRIDE * TAU_F32 * dt) % TAU_F32;

        for &child in children {
            if let Ok((limb, mut transform)) = limbs.get_mut(child) {
                transform.rotation = Quat::from_rotation_x(limb_angle(*limb, cycle.phase));
            } else if let Ok(mut transform) = torsos.get_mut(child) {
                let rest = if transform.translation.y > body::HEAD_CENTRE - 0.01 {
                    body::HEAD_CENTRE
                } else {
                    body::TORSO_CENTRE
                };
                transform.translation.y = rest + bob(cycle.phase);
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
    fn the_body_bobs_twice_per_stride() {
        // Once per stride is a limp: the body lifts over each leg in turn.
        let mut peaks = 0;
        for i in 0..1024 {
            let phase = TAU_F32 * i as f32 / 1024.0;
            let step = TAU_F32 / 1024.0;
            if bob(phase) > bob(phase - step) && bob(phase) >= bob(phase + step) {
                peaks += 1;
            }
        }
        assert_eq!(peaks, 2, "counted {peaks} rises per stride");
    }
}
