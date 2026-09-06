//! How a flummi feels, and how it catches that from everyone else.
//!
//! A mood is one number between −1 and +1, and the whole game is the argument
//! about where it sits. Two things move it: what happens to a flummi, and what
//! is happening to the flummis around it. The second is the important one — a
//! city where every citizen sulks privately is a city of forty-five unrelated
//! sulks, whereas one where a mood spreads is a crowd.
//!
//! The disposition doing the reacting is a [`Temperament`], and it is per
//! citizen rather than global. That is what makes the same shove funny twice:
//! it bounces off a serene flummi and starts a feud with the one next to it.
//!
//! Everything that decides anything here is a free function taking numbers and
//! returning a number. The systems are the plumbing around them, and the feel
//! of the city is argued about in the tests rather than in the game.

use bevy::prelude::*;
use rand::RngExt;
use rand_chacha::ChaCha8Rng;

use crate::bounce::boing::Wallop;
use crate::core::config::{GameConfig, MoodConfig};
use crate::core::rng::{stream, stream_for};
use crate::core::schedule::GameSet;
use crate::player::on_foot::Player;

/// How much a friendly bop lifts the mood of somebody with no temper at all.
const BOP_CHEER: f32 = 0.14;
/// How much of that a short fuse takes back. Above 1.25 the sign flips, which
/// is the point: a proper Wutbürger is insulted by being cheered up.
const FUSE_SOURS_A_BOP: f32 = 0.8;
/// Flummis that have to go red inside one second for it to count as a wave.
const WAVE_SIZE: usize = 5;
/// How long the wave is announced for, in seconds.
const WAVE_BANNER: f32 = 2.5;

/// How a flummi feels: −1 furious, 0 indifferent, +1 delighted.
#[derive(Component, Debug)]
pub struct Mood {
    pub value: f32,
    /// Last frame's value.
    ///
    /// Kept on the component rather than derived, because "how many flummis
    /// went red *just now*" is the entire rage-wave reading, and a crossing is
    /// only visible against where the mood was a moment ago.
    pub previous: f32,
}

impl Mood {
    pub fn new(value: f32) -> Self {
        Self {
            value,
            previous: value,
        }
    }

    /// True if this flummi crossed into the red since the last frame. Crossing
    /// rather than being below it: a crowd that is already furious is not a
    /// wave, it is a mob, and announcing it every frame would be noise.
    pub fn flared(&self, line: f32) -> bool {
        self.previous > line && self.value <= line
    }
}

/// What kind of citizen this is. Five named ones exist — see the constructors —
/// and nothing else ever builds one field by field, so that a temperament can
/// be talked about by name in the dev panel and in the tests.
#[derive(Component, Clone, Copy, Debug)]
pub struct Temperament {
    /// Where the mood drifts back to when nothing is happening.
    pub baseline: f32,
    /// How hard a provocation lands. Above 1 it lands harder than it was meant.
    pub fuse: f32,
    /// How fast they get over it, as a rate in reciprocal seconds.
    pub recovery: f32,
    /// How much of the neighbourhood's mood they pick up.
    pub contagion: f32,
    /// How likely they are to go after whoever did it: the roll
    /// `grudge::bears_a_grudge` checks — for offences suffered, and, since
    /// the crowd learned to watch (`schadenfreude`), for ones merely
    /// witnessed.
    pub grudge: f32,
}

impl Temperament {
    /// Nothing touches them. Walks through a riot whistling.
    pub fn serene() -> Self {
        Self {
            baseline: 0.55,
            fuse: 0.20,
            recovery: 0.55,
            contagion: 0.15,
            grudge: 0.02,
        }
    }

    pub fn easygoing() -> Self {
        Self {
            baseline: 0.30,
            fuse: 0.40,
            recovery: 0.40,
            contagion: 0.30,
            grudge: 0.10,
        }
    }

    pub fn ordinary() -> Self {
        Self {
            baseline: 0.05,
            fuse: 0.65,
            recovery: 0.28,
            contagion: 0.45,
            grudge: 0.30,
        }
    }

    pub fn touchy() -> Self {
        Self {
            baseline: -0.15,
            fuse: 0.95,
            recovery: 0.18,
            contagion: 0.60,
            grudge: 0.60,
        }
    }

    /// The Wutbürger. Starts the day annoyed, takes everything personally, and
    /// never lets it go.
    pub fn ragemonger() -> Self {
        Self {
            baseline: -0.45,
            fuse: 1.40,
            recovery: 0.08,
            contagion: 0.80,
            grudge: 0.95,
        }
    }

    /// For test failures worth reading. Reported off the fuse, because that is
    /// the field the five differ in most and the one they are named for.
    pub fn name(&self) -> &'static str {
        match self.fuse {
            f if f < 0.30 => "serene",
            f if f < 0.55 => "easygoing",
            f if f < 0.80 => "ordinary",
            f if f < 1.15 => "touchy",
            _ => "ragemonger",
        }
    }
}

/// One entry in the city's mix of dispositions.
#[derive(Clone, Copy, Debug)]
pub struct Kind {
    pub name: &'static str,
    pub temper: Temperament,
    /// This kind's share of the crowd, relative to the others.
    pub share: f32,
}

/// The five, and how much of the city each of them is.
///
/// A resource rather than a constant so the dev panel can push them about while
/// the game runs, which is the only way to find out how touchy a city has to be
/// before it is funny rather than exhausting. Newly spawned flummis draw from
/// whatever is in here; the ones already walking about keep the disposition
/// they were born with, and the panel's button to clear the crowd is how you
/// see a change take hold.
///
/// Weighted rather than uniform because a city of one-fifth Wutbürger is a city
/// where the joke never lands — a shove has to bounce off somebody most of the
/// time for it to be funny when it does not.
#[derive(Resource, Clone, Debug)]
pub struct Tempers(pub [Kind; 5]);

impl Default for Tempers {
    fn default() -> Self {
        Self([
            Kind {
                name: "serene",
                temper: Temperament::serene(),
                share: 0.15,
            },
            Kind {
                name: "easygoing",
                temper: Temperament::easygoing(),
                share: 0.25,
            },
            Kind {
                name: "ordinary",
                temper: Temperament::ordinary(),
                share: 0.30,
            },
            Kind {
                name: "touchy",
                temper: Temperament::touchy(),
                share: 0.20,
            },
            Kind {
                name: "ragemonger",
                temper: Temperament::ragemonger(),
                share: 0.10,
            },
        ])
    }
}

impl Tempers {
    /// Draws one citizen's disposition from the mix.
    pub fn draw(&self, rng: &mut ChaCha8Rng) -> Temperament {
        let total: f32 = self.0.iter().map(|kind| kind.share.max(0.0)).sum();
        if total <= 0.0 {
            // Every share dragged to zero in the dev panel. Somebody still has
            // to be spawned, and an ordinary citizen is the least surprising
            // thing to hand back.
            return Temperament::ordinary();
        }
        let mut ticket = rng.random_range(0.0..total);
        for kind in &self.0 {
            ticket -= kind.share.max(0.0);
            if ticket <= 0.0 {
                return kind.temper;
            }
        }
        self.0[self.0.len() - 1].temper
    }
}

/// Its own stream, so that retuning a temper cannot reshuffle the city.
#[derive(Resource)]
pub struct MoodRng(pub ChaCha8Rng);

/// What the HUD reads. Computed once a frame rather than by each widget, since
/// three of them want the same sweep over the same crowd.
#[derive(Resource, Default, Debug)]
pub struct CityMood {
    /// The player's own mood, or 0 before there is a player.
    pub player: f32,
    /// Mean mood of every flummi resident in the city, the player included.
    pub average: f32,
    pub crowd: usize,
    /// Seconds left on the rage-wave banner, and how big the wave was.
    pub wave: f32,
    pub wave_size: usize,
    /// Crossings counted so far in the current second, and what is left of it.
    tally: usize,
    window: f32,
}

impl CityMood {
    /// Folds one frame's crossings into a rolling one-second window, and
    /// announces a wave when enough of them land inside the same second.
    ///
    /// A window rather than a running total, because a wave is a *rate*: five
    /// flummis going red together is a street turning on somebody, and five
    /// going red over a quiet minute is just Tuesday.
    pub fn tick(&mut self, flared: usize, dt: f32) {
        self.tally += flared;
        self.window -= dt;
        if self.window <= 0.0 {
            if self.tally >= WAVE_SIZE {
                self.wave = WAVE_BANNER;
                self.wave_size = self.tally;
            }
            self.tally = 0;
            self.window = 1.0;
        }
        self.wave = (self.wave - dt).max(0.0);
    }
}

/// Everything that moves a mood, so that anything reading one — the face, the
/// HUD — can simply run after the lot of it.
#[derive(SystemSet, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Feeling;

pub struct FeelingPlugin;

impl Plugin for FeelingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CityMood>()
            .init_resource::<Tempers>()
            .add_systems(Startup, seed_the_stream)
            .add_systems(
                Update,
                (drift_moods, spread_moods, feel_wallops, read_the_room)
                    .chain()
                    .in_set(Feeling)
                    .in_set(GameSet::Ai),
            );
    }
}

fn seed_the_stream(mut commands: Commands, config: Res<GameConfig>) {
    commands.insert_resource(MoodRng(stream_for(config.world_seed, stream::MOOD)));
}

// ------------------------------------------------------------- the maths ----

/// Where a mood settles when nothing is happening to it.
///
/// An exponential ease rather than a fixed step, so that recovery is a rate
/// somebody can reason about — a flummi with `recovery` 0.5 closes half the gap
/// to its baseline in about a second and a half, whatever the frame rate — and
/// so that it can never overshoot into a mood nobody asked for.
pub fn drifted(value: f32, temper: &Temperament, dt: f32) -> f32 {
    value + (temper.baseline - value) * (temper.recovery * dt).clamp(0.0, 1.0)
}

/// Where a mood goes when everyone nearby feels differently.
pub fn caught(value: f32, neighbourhood: f32, temper: &Temperament, rate: f32, dt: f32) -> f32 {
    value + (neighbourhood - value) * (temper.contagion * rate * dt).clamp(0.0, 1.0)
}

/// What a knock does to a mood.
///
/// The sign is the joke. Below [`MoodConfig::bop_limit`] a knock is a bop and
/// cheers most people up; above it, it is an insult that lands in proportion to
/// the fuse. And a short enough fuse turns even the bop sour, which is exactly
/// what a Wutbürger is.
pub fn jolt(severity: f32, temper: &Temperament, tune: &MoodConfig) -> f32 {
    if severity < tune.bop_limit {
        return BOP_CHEER * (1.0 - FUSE_SOURS_A_BOP * temper.fuse);
    }
    let span = (tune.outrage_limit - tune.bop_limit).max(0.1);
    let force = ((severity - tune.bop_limit) / span).clamp(0.0, 1.0);
    -force * temper.fuse
}

// ----------------------------------------------------------- the systems ----

fn drift_moods(time: Res<Time>, mut flummis: Query<(&mut Mood, &Temperament)>) {
    let dt = time.delta_secs();
    for (mut mood, temper) in &mut flummis {
        mood.previous = mood.value;
        mood.value = drifted(mood.value, temper, dt).clamp(-1.0, 1.0);
    }
}

/// A mood is caught from the neighbours, not broadcast to them.
///
/// Two passes over a snapshot rather than one pass mutating in place: with
/// forty-five flummis the cost is nothing either way, but reading and writing
/// the same values in one sweep makes the result depend on iteration order,
/// which is exactly the kind of thing that quietly stops the city being
/// reproducible from its seed.
fn spread_moods(
    time: Res<Time>,
    config: Res<GameConfig>,
    mut flummis: Query<(&Transform, &mut Mood, &Temperament)>,
) {
    let dt = time.delta_secs();
    let tune = &config.mood;
    let reach = tune.contagion_radius * tune.contagion_radius;

    let crowd: Vec<(Vec3, f32)> = flummis
        .iter()
        .map(|(transform, mood, _)| (transform.translation, mood.value))
        .collect();
    if crowd.len() < 2 {
        return;
    }

    for (transform, mut mood, temper) in &mut flummis {
        let here = transform.translation;
        let mut sum = 0.0;
        let mut seen = 0usize;
        for (position, value) in &crowd {
            let apart = position.distance_squared(here);
            // Skips itself, since a body is always exactly zero from itself and
            // a flummi cannot catch its own mood.
            if apart > f32::EPSILON && apart < reach {
                sum += value;
                seen += 1;
            }
        }
        if seen == 0 {
            continue;
        }
        let neighbourhood = sum / seen as f32;
        mood.value =
            caught(mood.value, neighbourhood, temper, tune.contagion_rate, dt).clamp(-1.0, 1.0);
    }
}

fn feel_wallops(
    config: Res<GameConfig>,
    mut wallops: MessageReader<Wallop>,
    mut flummis: Query<(&mut Mood, &Temperament)>,
) {
    for wallop in wallops.read() {
        let Ok((mut mood, temper)) = flummis.get_mut(wallop.entity) else {
            continue;
        };
        mood.value = (mood.value + jolt(wallop.severity, temper, &config.mood)).clamp(-1.0, 1.0);
    }
}

/// Reads the city's temperature for the HUD, and spots a rage wave going
/// through it.
fn read_the_room(
    time: Res<Time>,
    config: Res<GameConfig>,
    mut city: ResMut<CityMood>,
    flummis: Query<(&Mood, Option<&Player>)>,
) {
    let dt = time.delta_secs();
    let line = config.mood.rage_line;

    let mut sum = 0.0;
    let mut crowd = 0usize;
    let mut flared = 0usize;
    for (mood, player) in &flummis {
        sum += mood.value;
        crowd += 1;
        if mood.flared(line) {
            flared += 1;
        }
        if player.is_some() {
            city.player = mood.value;
        }
    }
    city.crowd = crowd;
    city.average = if crowd == 0 { 0.0 } else { sum / crowd as f32 };

    city.tick(flared, dt);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tune() -> MoodConfig {
        GameConfig::default().mood
    }

    #[test]
    fn a_mood_left_alone_returns_to_the_temperament_it_came_from() {
        let temper = Temperament::ordinary();
        let mut value = -1.0;
        for _ in 0..1800 {
            value = drifted(value, &temper, 1.0 / 60.0);
        }
        assert!(
            (value - temper.baseline).abs() < 0.02,
            "half a minute of quiet left them at {value:.2} rather than {:.2}",
            temper.baseline
        );
    }

    #[test]
    fn a_wutburger_takes_far_longer_to_calm_down_than_a_peaceable_soul() {
        let steps = |temper: Temperament| {
            let mut value = -1.0;
            let mut ticks = 0;
            while value < temper.baseline - 0.05 && ticks < 100_000 {
                value = drifted(value, &temper, 1.0 / 60.0);
                ticks += 1;
            }
            ticks
        };
        assert!(
            steps(Temperament::ragemonger()) > steps(Temperament::serene()) * 3,
            "the Wutbürger got over it too easily"
        );
    }

    #[test]
    fn a_friendly_bop_cheers_up_a_peaceable_flummi_and_insults_a_wutburger() {
        // The same nudge, at the same speed, in both directions. This is the
        // joke the whole game is built on, so it is a test rather than a hope.
        let bop = tune().bop_limit * 0.5;
        assert!(jolt(bop, &Temperament::serene(), &tune()) > 0.05);
        assert!(jolt(bop, &Temperament::ragemonger(), &tune()) < 0.0);
    }

    #[test]
    fn being_run_over_lands_harder_on_a_short_fuse() {
        let flattened = tune().outrage_limit * 2.0;
        let calm = jolt(flattened, &Temperament::serene(), &tune());
        let furious = jolt(flattened, &Temperament::ragemonger(), &tune());
        assert!(calm < 0.0 && furious < calm);
        assert!(
            furious.abs() <= Temperament::ragemonger().fuse,
            "one knock swung a mood by more than a whole fuse"
        );
    }

    #[test]
    fn a_knock_can_only_get_so_insulting() {
        // Otherwise being hit by a truck at 90 would be arbitrarily worse than
        // being hit by one at 40, and every collision with a vehicle would end
        // in the same pinned-to-the-floor mood.
        let temper = Temperament::ordinary();
        assert_eq!(
            jolt(tune().outrage_limit, &temper, &tune()),
            jolt(tune().outrage_limit * 10.0, &temper, &tune())
        );
    }

    #[test]
    fn a_mood_is_pulled_towards_the_neighbourhood_without_passing_it() {
        let temper = Temperament::touchy();
        let mut value = -1.0;
        for _ in 0..600 {
            value = caught(value, 0.8, &temper, 0.9, 1.0 / 60.0);
            assert!(value <= 0.8, "overshot the crowd it was catching from");
        }
        assert!(
            value > 0.5,
            "ten seconds in a happy crowd left them at {value:.2}"
        );
    }

    #[test]
    fn the_serene_barely_notice_the_mob_around_them() {
        let mob = -1.0;
        let calm = caught(0.5, mob, &Temperament::serene(), 0.9, 0.1);
        let touchy = caught(0.5, mob, &Temperament::touchy(), 0.9, 0.1);
        assert!(touchy < calm);
    }

    #[test]
    fn the_crowd_is_mostly_bearable() {
        let mut rng = stream_for(0, stream::MOOD);
        let tempers = Tempers::default();
        let drawn: Vec<Temperament> = (0..1000).map(|_| tempers.draw(&mut rng)).collect();
        let menaces = drawn.iter().filter(|t| t.fuse > 1.0).count();
        assert!(
            (50..200).contains(&menaces),
            "{menaces} in a thousand were Wutbürger; the joke needs them rare"
        );
        assert!(
            drawn.iter().any(|t| t.name() == "serene"),
            "nobody in a thousand was at peace"
        );
    }

    #[test]
    fn every_temperament_answers_to_its_own_name() {
        // The dev panel labels its sliders from `Kind::name` and tests report
        // from `Temperament::name`, which is derived. If the two ever disagree
        // a slider is quietly editing something other than what it says.
        for kind in Tempers::default().0 {
            assert_eq!(
                kind.temper.name(),
                kind.name,
                "the table calls it {} and the fuse says {}",
                kind.name,
                kind.temper.name()
            );
        }
    }

    #[test]
    fn a_table_with_nothing_left_in_it_still_produces_a_citizen() {
        // Every share can be dragged to zero in the dev panel, and the crowd
        // still has to be filled.
        let mut empty = Tempers::default();
        for kind in &mut empty.0 {
            kind.share = 0.0;
        }
        let mut rng = stream_for(0, stream::MOOD);
        assert_eq!(empty.draw(&mut rng).name(), "ordinary");
    }

    #[test]
    fn enough_flummis_going_red_at_once_is_announced_as_a_wave() {
        let mut city = CityMood::default();
        for _ in 0..30 {
            city.tick(1, 1.0 / 30.0);
        }
        // Two ticks past the end of the window: one to close it, one for the
        // banner to be up.
        city.tick(0, 1.0 / 30.0);
        assert!(
            city.wave > 0.0,
            "thirty crossings in a second went unremarked"
        );
        assert!(city.wave_size >= WAVE_SIZE);
    }

    #[test]
    fn the_same_crossings_spread_thin_are_not_a_wave() {
        let mut city = CityMood::default();
        // One flummi going red every second for half a minute. A grumbling
        // city, not a riot, and the HUD must not shout about it.
        for _ in 0..30 {
            city.tick(1, 0.5);
            city.tick(0, 0.5);
        }
        assert_eq!(city.wave, 0.0, "a slow trickle was announced as a wave");
    }

    #[test]
    fn a_flare_is_a_crossing_rather_than_a_state() {
        let line = tune().rage_line;
        let crossing = Mood {
            value: line - 0.01,
            previous: line + 0.01,
        };
        let already_furious = Mood {
            value: -0.9,
            previous: -0.9,
        };
        assert!(crossing.flared(line));
        assert!(
            !already_furious.flared(line),
            "a crowd that is already angry would announce itself every frame"
        );
    }
}
