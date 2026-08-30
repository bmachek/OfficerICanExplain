//! Live-tunable gameplay constants.
//!
//! Everything that needs feel-tuning lives here rather than as scattered
//! literals, so the egui dev panel can edit it at runtime instead of forcing a
//! recompile. Sections are added as milestones need them.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Resource, Debug, Clone, Serialize, Deserialize)]
pub struct GameConfig {
    /// Everything about world layout derives from this. Same seed, same city.
    pub world_seed: u64,
    pub world: WorldConfig,
    pub camera: CameraConfig,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CameraConfig {
    /// Free-fly movement speed.
    pub speed: f32,
    pub boost_multiplier: f32,
    pub mouse_sensitivity: f32,
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
            },
            camera: CameraConfig {
                speed: 25.0,
                boost_multiplier: 5.0,
                mouse_sensitivity: 0.002,
            },
        }
    }
}

pub struct ConfigPlugin;

impl Plugin for ConfigPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameConfig>();
    }
}

impl Default for CameraConfig {
    fn default() -> Self {
        GameConfig::default().camera
    }
}
