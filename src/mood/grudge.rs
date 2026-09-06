//! Taking it personally.
//!
//! A mood that only moves a number and paints a face is a readout. What makes
//! the crowd worth annoying is that some of them do something about it: a
//! flummi with a short fuse and a long memory picks whoever offended it out of
//! the street, hops after them, and rams them off their feet — which launches
//! the victim, sours *their* mood, and starts the next one. The chain is the
//! joke, and none of it is scripted; it falls out of the same three rules
//! applied to everybody.
//!
//! The mirror of it is the happy half. A delighted flummi does not hunt anybody
//! down, it spins on the spot and bumps into the nearest neighbour, and a
//! gentle bump raises a mood rather than lowering one (see
//! [`super::feeling::jolt`]). So the city has two contagions running through it
//! in opposite directions, built out of the same collision.
//!
//! ## Who gets blamed
//!
//! A provocation names its author, so a grudge from one is straightforward.
//! A [`Wallop`] does not: it is spotted from a sudden change in velocity and
//! has no idea what caused it — a car, a wall, a bad landing. Rather than
//! plumb a culprit through the physics for it, an aggrieved flummi blames
//! *whoever is standing nearest*. That is wrong about half the time, which is
//! the correct amount: being furious at the closest available person is
//! exactly what the temperament is for, and a Wutbürger who blamed the wall
//! would be a much less interesting neighbour.

use avian3d::prelude::*;
use bevy::prelude::*;
use rand::RngExt;

use super::feeling::{Mood, Temperament};
use super::provoke::{Provocation, Rudeness};
use crate::audio::AudioRng;
use crate::bounce::boing::Wallop;
use crate::bounce::controller::{Bouncer, Launched};
use crate::bounce::launch::{KnockedDown, THROW_UP, launch};
use crate::core::config::GameConfig;
use crate::core::schedule::GameSet;

/// Mood below which somebody is cross enough to hold a grudge at all.
const SORE: f32 = -0.35;
/// And above which they are pleased enough to go and celebrate at somebody.
const CHUFFED: f32 = 0.6;
/// How far a Wallop's blame can reach when looking for somebody to pin it on.
const BLAME_RANGE: f32 = 6.0;
/// Velocity lost that is worth blaming anybody for at all. Below this it was
/// the kerb.
const WORTH_BLAMING: f32 = 5.0;

/// How close a pursuer has to get before it counts as a ram.
const RAM_REACH: f32 = 1.5;
/// Fraction of the pursuer's closing speed handed to the victim.
const RAM_FORCE: f32 = 1.4;

/// How long a pirouette lasts, and how fast it spins, in turns per second.
const SPIN_SECONDS: f32 = 1.1;
const SPIN_RATE: f32 = 2.2;
/// How far a delighted flummi will go out of its way to bump a neighbour.
const BUMP_RANGE: f32 = 7.0;
/// Chance per second of a delighted flummi starting one.
const DELIGHT: f32 = 0.5;
/// How fast it wanders over to do it, in m/s.
const BUMP_SPEED: f32 = 2.6;

/// Somebody this flummi is cross with, and how much longer it cares.
#[derive(Component, Debug)]
pub struct Grudge {
    pub against: Entity,
    pub left: f32,
}

/// A flummi too pleased with itself to walk in a straight line.
#[derive(Component, Debug)]
pub struct Pirouette {
    pub left: f32,
    /// Who it is spinning towards, if anybody was near enough to be worth it.
    pub towards: Option<Entity>,
}

impl Pirouette {
    /// A spin with nobody to bump: what a delighted witness does on the spot
    /// when somebody else's afternoon goes wrong entertainingly
    /// (`super::schadenfreude`).
    pub fn solo() -> Self {
        Self {
            left: SPIN_SECONDS,
            towards: None,
        }
    }
}

pub struct GrudgePlugin;

impl Plugin for GrudgePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                (take_offence, blame_the_nearest, take_delight),
                (pursue, spin, drift_towards_company),
                settle_scores,
            )
                .chain()
                .in_set(GameSet::Ai)
                // Both of these override where the crowd decided to walk, so
                // they have to write `Bouncer::desired` after the crowd does.
                .after(crate::ai::pedestrian::Walking)
                .after(super::provoke::Provoking),
        );
    }
}

/// Whether somebody in this mood, with this disposition, bears a grudge.
///
/// The roll is against `grudge` rather than against the fuse, because they are
/// genuinely different traits: plenty of people are easy to annoy and let it go
/// immediately, and the ones who are hard to annoy and never forget are the
/// funnier neighbours to have.
pub fn bears_a_grudge(mood: f32, temper: &Temperament, roll: f32) -> bool {
    mood <= SORE && roll < temper.grudge
}

fn take_offence(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut rng: ResMut<AudioRng>,
    mut provocations: MessageReader<Provocation>,
    flummis: Query<(Entity, &Transform, &Mood, &Temperament), Without<Launched>>,
) {
    for provocation in provocations.read() {
        if provocation.kind != Rudeness::Taunt {
            continue;
        }
        for (entity, transform, mood, temper) in &flummis {
            if entity == provocation.by {
                continue;
            }
            // Only somebody who was actually within earshot of it. Skipping
            // this makes every raspberry an event the whole city takes
            // personally, which is a riot rather than a provocation.
            if transform.translation.distance(provocation.at) > config.mood.taunt_radius {
                continue;
            }
            if !bears_a_grudge(mood.value, temper, rng.random::<f32>()) {
                continue;
            }
            commands.entity(entity).insert(Grudge {
                against: provocation.by,
                left: config.mood.grudge_seconds,
            });
        }
    }
}

/// A hard knock, and somebody to pin it on. See the module note: the blame is
/// meant to be unreliable.
fn blame_the_nearest(
    mut commands: Commands,
    config: Res<GameConfig>,
    mut rng: ResMut<AudioRng>,
    mut wallops: MessageReader<Wallop>,
    flummis: Query<(Entity, &Transform, &Mood, &Temperament), Without<Launched>>,
) {
    for wallop in wallops.read() {
        if wallop.severity < WORTH_BLAMING {
            continue;
        }
        let Ok((victim, transform, mood, temper)) = flummis.get(wallop.entity) else {
            continue;
        };
        if !bears_a_grudge(mood.value, temper, rng.random::<f32>()) {
            continue;
        }

        let here = transform.translation;
        let culprit = flummis
            .iter()
            .filter(|(other, ..)| *other != victim)
            .map(|(other, at, ..)| (at.translation.distance(here), other))
            .filter(|(apart, _)| *apart < BLAME_RANGE)
            .min_by(|a, b| a.0.total_cmp(&b.0));
        let Some((_, culprit)) = culprit else {
            continue;
        };

        commands.entity(victim).insert(Grudge {
            against: culprit,
            left: config.mood.grudge_seconds,
        });
    }
}

/// The happy half: somebody spins on the spot and goes to bump a neighbour.
fn take_delight(
    mut commands: Commands,
    time: Res<Time>,
    mut rng: ResMut<AudioRng>,
    flummis: Query<
        (Entity, &Transform, &Mood),
        (
            Without<Launched>,
            Without<Pirouette>,
            Without<Grudge>,
            Without<crate::player::on_foot::Player>,
        ),
    >,
) {
    let dt = time.delta_secs();
    // Positions first: the neighbour being bumped into is looked up out of the
    // same query, and a delighted flummi with nobody about should still spin.
    let crowd: Vec<(Entity, Vec3)> = flummis
        .iter()
        .map(|(entity, transform, _)| (entity, transform.translation))
        .collect();

    for (entity, transform, mood) in &flummis {
        if mood.value < CHUFFED || rng.random::<f32>() > DELIGHT * dt {
            continue;
        }
        let here = transform.translation;
        let towards = crowd
            .iter()
            .filter(|(other, _)| *other != entity)
            .map(|(other, at)| (at.distance(here), *other))
            .filter(|(apart, _)| *apart < BUMP_RANGE)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, other)| other);
        commands.entity(entity).insert(Pirouette {
            left: SPIN_SECONDS,
            towards,
        });
    }
}

/// Points somebody with a score to settle at whoever they are settling it with.
fn pursue(
    mut commands: Commands,
    time: Res<Time>,
    config: Res<GameConfig>,
    targets: Query<&Transform>,
    mut pursuers: Query<(Entity, &Transform, &mut Bouncer, &mut Grudge), Without<Launched>>,
) {
    let dt = time.delta_secs();
    for (entity, transform, mut bouncer, mut grudge) in &mut pursuers {
        grudge.left -= dt;
        let Ok(target) = targets.get(grudge.against) else {
            // Whoever it was has left the city. Nothing to settle.
            commands.entity(entity).remove::<Grudge>();
            continue;
        };
        if grudge.left <= 0.0 {
            commands.entity(entity).remove::<Grudge>();
            continue;
        }

        let towards = (target.translation - transform.translation).with_y(0.0);
        bouncer.desired = towards.normalize_or_zero().xz() * config.mood.grudge_speed;
    }
}

/// Spins the delighted on the spot.
///
/// Split from [`drift_towards_company`] purely because of what a query may
/// borrow: turning a dancer needs `&mut Transform`, and finding the neighbour
/// it is dancing at needs to read *every* `Transform` including the dancers'.
/// One system cannot hold both, and the split is a better answer than a
/// `ParamSet` here because the two halves genuinely are two jobs.
fn spin(
    mut commands: Commands,
    time: Res<Time>,
    mut dancers: Query<(Entity, &mut Transform, &mut Pirouette), Without<Launched>>,
) {
    let dt = time.delta_secs();
    for (entity, mut transform, mut spin) in &mut dancers {
        spin.left -= dt;
        if spin.left <= 0.0 {
            commands.entity(entity).remove::<Pirouette>();
            continue;
        }
        // Written straight onto the rotation, which nothing else is competing
        // for: the crowd's own facing is set by `walk_pavements` earlier in the
        // same set, and rotation is locked so the solver will not touch it.
        transform.rotate_y(std::f32::consts::TAU * SPIN_RATE * dt);
    }
}

/// And steers them gently into company.
fn drift_towards_company(
    targets: Query<&Transform>,
    mut dancers: Query<(&Transform, &mut Bouncer, &Pirouette), Without<Launched>>,
) {
    for (transform, mut bouncer, spin) in &mut dancers {
        let Some(friend) = spin.towards else { continue };
        let Ok(at) = targets.get(friend) else {
            continue;
        };
        let towards = (at.translation - transform.translation).with_y(0.0);
        // Slower than a chase. The difference between being bumped into by
        // somebody in a good mood and being rammed by somebody in a bad one has
        // to be legible from across the street.
        bouncer.desired = towards.normalize_or_zero().xz() * BUMP_SPEED;
    }
}

/// The ram itself.
///
/// Only the angry half lands a blow. A pirouetting flummi simply walks into its
/// neighbour and lets the solver deal with it, which is a small knock, which
/// [`super::feeling::jolt`] reads as a friendly bop and *raises* both moods.
/// That is the whole of the happy contagion: no code, just the same collision
/// with a different number on it.
/// Decided and then applied, in two passes over a [`ParamSet`], because both
/// halves want `LinearVelocity` — the rammer's to see how fast it arrived, the
/// victim's to throw them with. The obvious fix is to make the two queries
/// disjoint with a filter, and it is a trap: the only filter available is
/// `Without<Grudge>` on the victim, which quietly means two angry flummis can
/// never ram each other. That is precisely the collision the whole chain is
/// made of.
fn settle_scores(
    mut commands: Commands,
    knocked: Query<(), With<KnockedDown>>,
    positions: Query<&Transform>,
    mut bodies: ParamSet<(
        Query<(Entity, &Transform, &LinearVelocity, &Grudge), Without<Launched>>,
        Query<&mut LinearVelocity>,
    )>,
) {
    let mut landed: Vec<(Entity, Entity, Vec3)> = Vec::new();
    for (rammer, transform, velocity, grudge) in &bodies.p0() {
        let Ok(target) = positions.get(grudge.against) else {
            continue;
        };
        // Somebody already flat on their back cannot be knocked flatter, and
        // trying makes a pursuer hover over them until the grudge runs out.
        if knocked.contains(grudge.against) {
            continue;
        }
        let apart = (target.translation - transform.translation).with_y(0.0);
        if apart.length() > RAM_REACH {
            continue;
        }
        // Whatever speed the pursuer arrived with, along the line between them,
        // handed on with interest. A rammer that has been slowed to a crawl by
        // the crowd should not still launch anybody across a junction.
        let Ok(away) = Dir3::new(apart) else { continue };
        let closing = velocity.0.dot(*away).max(1.0);
        landed.push((
            rammer,
            grudge.against,
            *away * (closing * RAM_FORCE) + Vec3::Y * THROW_UP,
        ));
    }

    let mut velocities = bodies.p1();
    for (rammer, victim, throw) in landed {
        let Ok(mut velocity) = velocities.get_mut(victim) else {
            continue;
        };
        launch(&mut commands, victim, &mut velocity, throw);
        // Satisfied. Without this the pursuer stays glued to whoever it just
        // launched and rams them again the moment they land.
        commands.entity(rammer).remove::<Grudge>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nobody_in_a_decent_mood_holds_a_grudge() {
        // Whatever the disposition. A Wutbürger who is currently delighted has
        // nothing to be furious about, and chasing somebody down while wearing
        // a grin would read as a bug rather than as a joke.
        for kind in super::super::feeling::Tempers::default().0 {
            assert!(!bears_a_grudge(0.5, &kind.temper, 0.0), "{}", kind.name);
        }
    }

    #[test]
    fn the_short_memoried_let_it_go_and_the_long_memoried_do_not() {
        // The same roll, the same mood, opposite answers: this is the field
        // doing the work, rather than the mood.
        let roll = 0.5;
        assert!(!bears_a_grudge(-1.0, &Temperament::serene(), roll));
        assert!(bears_a_grudge(-1.0, &Temperament::ragemonger(), roll));
    }

    #[test]
    fn a_grudge_is_a_memory_rather_than_a_temper() {
        // `touchy` has a shorter fuse than `ordinary` and only twice the
        // grudge, so a roll between them separates the two traits. If this ever
        // stops holding, one of the five has become a duplicate of another.
        let ordinary = Temperament::ordinary();
        let touchy = Temperament::touchy();
        assert!(touchy.fuse > ordinary.fuse);
        assert!(touchy.grudge > ordinary.grudge);
        let roll = (ordinary.grudge + touchy.grudge) * 0.5;
        assert!(!bears_a_grudge(-1.0, &ordinary, roll));
        assert!(bears_a_grudge(-1.0, &touchy, roll));
    }

    #[test]
    fn a_ram_is_faster_than_a_bump_and_slower_than_being_run_over() {
        // The three ways of being knocked about have to be tellable apart at a
        // glance, which means keeping them in order.
        let ram = crate::core::config::GameConfig::default().mood.grudge_speed;
        assert!(ram > BUMP_SPEED, "a ram is no faster than a friendly bump");
        assert!(
            ram < crate::ai::pedestrian::FLEE_SPEED + 1.0,
            "being chased has to be survivable on foot"
        );
    }
}
