//! Live-tunable gameplay constants.
//!
//! Everything that needs feel-tuning lives here rather than as scattered
//! literals, so the egui dev panel can edit it at runtime instead of forcing a
//! recompile. Sections are added as milestones need them.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::render::quality::GraphicsSettings;

#[derive(Resource, Debug, Clone, Serialize, Deserialize)]
pub struct GameConfig {
    /// Everything about world layout derives from this. Same seed, same city.
    pub world_seed: u64,
    pub world: WorldConfig,
    /// How much of a rubber ball everything in this city is.
    pub bounce: BounceConfig,
    /// How quick this city's temper is.
    pub mood: MoodConfig,
    pub camera: CameraConfig,
    pub audio: AudioConfig,
    /// What the renderer is allowed to spend. Resolved from a single quality
    /// preset and then walked back to what the GPU actually supports; see
    /// [`crate::render::quality`].
    pub graphics: GraphicsSettings,
    /// The window itself, as opposed to what is rendered into it. Kept out of
    /// [`GraphicsSettings`] because that block is resolved from a quality
    /// preset and restored from saves; how big the window is belongs to the
    /// player and the screen it sits on, not to a preset or a save game.
    ///
    /// `#[serde(default)]` so an options file written before this existed
    /// still parses instead of resetting everything else in it.
    #[serde(default)]
    pub window: WindowConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WindowConfig {
    /// Only meaningful in windowed mode; fullscreen takes the screen's own
    /// size, and the settings screen greys this out accordingly.
    pub resolution: Resolution,
    /// Borderless fullscreen on the screen the window is currently on —
    /// never the "exclusive" kind, which switches the display's video mode
    /// and takes the whole desktop down with it when the game hiccups.
    ///
    /// `#[serde(default)]` so an options file written when the resolution
    /// switcher was the whole of this section still parses.
    #[serde(default)]
    pub fullscreen: bool,
}

/// The window sizes the settings menu offers, in logical pixels — on a HiDPI
/// screen the OS multiplies by its scale factor, the same as the size the
/// window is opened with in `main`.
///
/// A fixed menu rather than a free width and height, so the options file can
/// never ask for a size the menu could not have produced, and so the settings
/// screen is a switch rather than two drag fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Resolution {
    R1280x720,
    /// The size the game has always opened at; staying the default means a
    /// fresh install and a pre-switcher install open the same window.
    #[default]
    R1600x900,
    R1920x1080,
    R2560x1440,
    R3840x2160,
}

impl Resolution {
    pub const ALL: [Self; 5] = [
        Self::R1280x720,
        Self::R1600x900,
        Self::R1920x1080,
        Self::R2560x1440,
        Self::R3840x2160,
    ];

    pub fn size(self) -> (u32, u32) {
        match self {
            Self::R1280x720 => (1280, 720),
            Self::R1600x900 => (1600, 900),
            Self::R1920x1080 => (1920, 1080),
            Self::R2560x1440 => (2560, 1440),
            Self::R3840x2160 => (3840, 2160),
        }
    }

    pub fn label(self) -> String {
        let (width, height) = self.size();
        format!("{width} × {height}")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorldConfig {
    /// Half-extent of the city, in metres.
    pub half_extent: f32,
    /// Chunks within this distance of the camera are spawned.
    pub stream_radius: f32,
    /// Real seconds for a full 24h cycle. 0 freezes the clock.
    pub day_length_seconds: f32,
    pub start_hour: f32,
    /// How wet the ground is when the world opens, 0 to 1.
    ///
    /// A starting value rather than a dial. Weather runs on its own from here —
    /// see [`crate::world::weather::Weather`] for the live values — and it runs
    /// on the same clock as the sun, so `day_length_seconds` at zero holds the
    /// whole sky still. That is what a screenshot needs.
    pub start_wetness: f32,
    /// And how much of the sky is under cloud when it opens, 0 to 1.
    pub start_cover: f32,
}

/// The elastic half of the simulation.
///
/// Two unrelated things are tuned from here, and they are kept together because
/// they have to be tuned against each other. `restitution` and `threshold`
/// belong to the solver: they decide how a body that is *not* in charge of
/// itself rebounds. `hop_speed` and the accelerations belong to the character
/// controller: they decide how a body that *is* in charge of itself gets about,
/// which in this city means hopping. A player who bounces off a wall harder
/// than they can hop is a player who has lost control of the game.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BounceConfig {
    /// Fraction of closing speed returned by a collision, 0 to 1.
    pub restitution: f32,
    /// Closing speed, in m/s, below which the solver stops bothering to bounce.
    ///
    /// Avian's default is 1.0, which is most of a hop: without lowering this,
    /// every small knock is absorbed and the city reads as rubber only when
    /// something arrives at speed.
    pub threshold: f32,
    /// Upward speed, in m/s, taken on at the bottom of every hop.
    pub hop_speed: f32,
    /// How hard a grounded body pulls itself towards the speed it wants.
    pub ground_accel: f32,
    /// And in the air, where there is nothing to push against. Much lower, so
    /// that a hop commits you to where it is going.
    pub air_accel: f32,
    /// How far a figure squashes at the bottom of a hop, as a fraction of its
    /// height. Zero is a rigid body on a pogo stick; too much is a puddle.
    pub squash: f32,
    /// Extra speed a crashed car takes away from whatever it hit, as a
    /// fraction of the speed it lost arriving. On top of the solver's own
    /// restitution, because the solver's idea of elastic is physically
    /// defensible and therefore not funny.
    ///
    /// The three `crash_*` dials are `#[serde(default)]` so an options file
    /// written before they existed still parses instead of resetting
    /// everything else in it.
    #[serde(default = "default_crash_rebound")]
    pub crash_rebound: f32,
    /// Upward part of that rebound, as a fraction of the speed lost. A car
    /// that only bounces back is a billiard ball; one that also leaves the
    /// ground is a joke.
    #[serde(default = "default_crash_pop")]
    pub crash_pop: f32,
    /// Spin handed to a crashed car, in rad/s per m/s of speed lost. Off-axis
    /// blows pirouette; head-on ones tip.
    #[serde(default = "default_crash_spin")]
    pub crash_spin: f32,
    /// The player's resting hop, as a fraction of `hop_speed`. Deliberately
    /// below the whole crowd's range: the camera rides the player's hop
    /// almost 1:1, so a scale that looks lively on a citizen across the
    /// street reads as seasickness from the navel the view is bolted to.
    /// The others do the bouncing; the player mostly watches them do it.
    #[serde(default = "default_player_hop_scale")]
    pub player_hop_scale: f32,
    /// Ceiling on how high delight scales a citizen's hop. This is where the
    /// bounce the player gave up went.
    #[serde(default = "default_npc_spring_max")]
    pub npc_spring_max: f32,
}

fn default_apology_range() -> f32 {
    GameConfig::default().mood.apology_range
}
fn default_apology_balm() -> f32 {
    GameConfig::default().mood.apology_balm
}

fn default_player_hop_scale() -> f32 {
    GameConfig::default().bounce.player_hop_scale
}
fn default_npc_spring_max() -> f32 {
    GameConfig::default().bounce.npc_spring_max
}
fn default_crash_rebound() -> f32 {
    GameConfig::default().bounce.crash_rebound
}
fn default_crash_pop() -> f32 {
    GameConfig::default().bounce.crash_pop
}
fn default_crash_spin() -> f32 {
    GameConfig::default().bounce.crash_spin
}

/// How the city feels, and how fast it changes its mind.
///
/// Only the numbers that are shared between subsystems live here. A single
/// flummi's disposition is its [`crate::mood::feeling::Temperament`], because that
/// varies from one citizen to the next and a global dial cannot express "most
/// people are fine, one in ten is a menace".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoodConfig {
    /// How far a mood carries to the neighbours, in metres.
    ///
    /// Roughly the width of a street. Wider and the whole city moves as one
    /// block, which is a single mood rather than a crowd of them.
    pub contagion_radius: f32,
    /// How fast a flummi is pulled towards the mood around it, per second at
    /// full susceptibility.
    pub contagion_rate: f32,
    /// Velocity lost in a knock, in m/s, up to which it reads as a friendly
    /// bop rather than as an insult. The joke lives on this line: the same
    /// nudge delights one flummi and starts a feud with the next.
    pub bop_limit: f32,
    /// And the loss that makes the worst impression anybody can make. Harder
    /// knocks than this exist; they are no more insulting.
    pub outrage_limit: f32,
    /// Mood below which a flummi counts as having gone red. What the rage-wave
    /// readout in the HUD counts crossings of.
    pub rage_line: f32,
    /// How far a raspberry carries as an insult, in metres. Deliberately
    /// shorter than a whistle: it should be possible to be rude to one person
    /// without starting a riot, and to cheer up a whole street at once.
    pub taunt_radius: f32,
    pub cheer_radius: f32,
    /// How much mood a taunt takes off somebody standing right next to it, at
    /// a fuse of 1. Further away it is less; see
    /// [`crate::mood::provoke::carry`].
    pub taunt_bite: f32,
    /// And how much a whistle gives back.
    pub cheer_warmth: f32,
    /// Seconds between one flummi's provocations. Long enough that the button
    /// is a decision rather than a drum roll.
    pub provoke_rest: f32,
    /// How far an apology can be thrown, in metres. Shorter than a cheer:
    /// making peace means walking up to somebody, not shouting sorry across
    /// a junction.
    ///
    /// The two `apology_*` dials are `#[serde(default)]` so an options file
    /// written before they existed still parses.
    #[serde(default = "default_apology_range")]
    pub apology_range: f32,
    /// Mood restored to whoever the flower reaches. Deliberately the biggest
    /// single lift in the game: an apology accepted has to actually settle
    /// the matter, or the flower is a decoration on a feud.
    #[serde(default = "default_apology_balm")]
    pub apology_balm: f32,
    /// How long somebody stays after whoever offended them, in seconds.
    pub grudge_seconds: f32,
    /// Ground speed of a flummi with a score to settle, in m/s. Faster than
    /// walking and slower than sprinting: being chased has to be survivable.
    pub grudge_speed: f32,
}

/// The mixer. Three numbers rather than one, because the background bed and
/// the things that happen in front of it want independent control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    /// Scales everything below it.
    pub master: f32,
    /// Weapons, crashes, engines, sirens: anything an event causes.
    pub effects: f32,
    /// The city's background rumble.
    pub ambience: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraConfig {
    /// Free-fly movement speed.
    pub speed: f32,
    pub boost_multiplier: f32,
    pub mouse_sensitivity: f32,
    /// Flips vertical mouse look. Off by default; some players' hands only
    /// work the other way, and that preference is old enough to predate this
    /// genre.
    pub invert_look_y: bool,
    /// How hard the view swings itself in behind the direction of travel, as
    /// an exponential rate in reciprocal seconds. 0 turns it off entirely.
    pub auto_follow: f32,
    /// Seconds of hands off the mouse before that swing starts. Long enough
    /// that looking somewhere deliberately is never fought, short enough that
    /// a corner taken two-handed does not lose the car off the side of the
    /// screen.
    pub auto_follow_delay: f32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            world_seed: 0xA17E_5EED,
            world: WorldConfig {
                half_extent: 1000.0,
                stream_radius: 900.0,
                day_length_seconds: 600.0,
                start_hour: 9.5,
                start_wetness: 0.0,
                // A fair day with a little cloud in it. Where the weather drifts
                // from here is the seed's business.
                start_cover: 0.18,
            },
            bounce: BounceConfig {
                // Not 1.0 — a perfectly elastic city never settles — and no
                // longer 0.86 either: at that height walls and kerbs threw
                // the player around harder than the player could steer, and
                // a crashed car pinballed for ten seconds before it could be
                // driven again. The comedy send-offs are applied by hand
                // (`vehicle::impact::fling`, `world::mayhem::send_off`), so
                // the solver's own rebound can afford to be modest.
                restitution: 0.62,
                threshold: 0.05,
                hop_speed: 2.8,
                ground_accel: 42.0,
                air_accel: 22.0,
                squash: 0.35,
                // Together these mean a solid 12 m/s crash throws the car
                // back at ~7 m/s, hops it half a metre and turns it most of
                // the way round — enough that both parties leave the scene.
                crash_rebound: 0.6,
                crash_pop: 0.28,
                crash_spin: 0.35,
                player_hop_scale: 0.6,
                npc_spring_max: 1.5,
            },
            mood: MoodConfig {
                contagion_radius: 9.0,
                contagion_rate: 0.9,
                bop_limit: 6.5,
                outrage_limit: 18.0,
                rage_line: -0.5,
                taunt_radius: 11.0,
                cheer_radius: 15.0,
                taunt_bite: 0.55,
                cheer_warmth: 0.34,
                provoke_rest: 0.8,
                apology_range: 13.0,
                apology_balm: 0.55,
                grudge_seconds: 7.0,
                grudge_speed: 5.2,
            },
            camera: CameraConfig {
                speed: 25.0,
                boost_multiplier: 5.0,
                mouse_sensitivity: 0.002,
                invert_look_y: false,
                auto_follow: 3.0,
                auto_follow_delay: 0.7,
            },
            audio: AudioConfig {
                master: 0.7,
                effects: 1.0,
                ambience: 0.5,
            },
            graphics: GraphicsSettings::default(),
            window: WindowConfig::default(),
        }
    }
}

impl Default for CameraConfig {
    fn default() -> Self {
        GameConfig::default().camera
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_resolution_is_the_size_main_opens_the_window_with() {
        assert_eq!(Resolution::default().size(), (1600, 900));
    }

    #[test]
    fn the_resolution_menu_lists_every_size_once_smallest_first() {
        let sizes: Vec<_> = Resolution::ALL.iter().map(|r| r.size()).collect();
        let mut sorted = sizes.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sizes, sorted);
    }

    #[test]
    fn a_window_section_from_before_the_fullscreen_switch_still_parses() {
        let parsed: WindowConfig =
            ron::from_str("(resolution:R1920x1080)").expect("old window section should parse");
        assert_eq!(parsed.resolution, Resolution::R1920x1080);
        assert!(!parsed.fullscreen);
    }

    #[test]
    fn an_options_file_without_a_window_section_still_parses() {
        // What `saves/options.ron` looked like before the resolution switcher:
        // the whole config, minus the `window` field.
        let mut old = GameConfig::default();
        old.audio.master = 0.42;
        let mut text = ron::ser::to_string(&old).unwrap();
        let start = text.find("window:").unwrap();
        let end = text.rfind(')').unwrap();
        text.replace_range(start..end, "");
        let parsed: GameConfig = ron::from_str(&text).expect("old options should parse");
        assert_eq!(parsed.audio.master, 0.42);
        assert_eq!(parsed.window.resolution, Resolution::default());
    }

    #[test]
    fn an_options_file_without_the_hop_dials_still_parses() {
        // What a bounce section looked like before `player_hop_scale` and
        // `npc_spring_max` landed: the fields simply absent.
        let mut old = ron::ser::to_string(&GameConfig::default()).unwrap();
        for field in ["player_hop_scale", "npc_spring_max"] {
            let start = old.find(field).unwrap();
            let rest = &old[start..];
            // The section's last field ends at `)` rather than `,`.
            let comma = rest.find(',');
            let paren = rest.find(')').unwrap();
            let end = match comma {
                Some(comma) if comma < paren => start + comma + 1,
                _ => start + paren,
            };
            old.replace_range(start..end, "");
        }
        let parsed: GameConfig = ron::from_str(&old).expect("old options should parse");
        let fresh = GameConfig::default().bounce;
        assert_eq!(parsed.bounce.player_hop_scale, fresh.player_hop_scale);
        assert_eq!(parsed.bounce.npc_spring_max, fresh.npc_spring_max);
    }
}
