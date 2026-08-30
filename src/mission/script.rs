//! The mission chain.
//!
//! Objectives are placed against the generated city rather than hard-coded
//! coordinates, so a different world seed still produces a playable chain.

use bevy::prelude::*;

use super::framework::{Mission, Objective};
use crate::world::City;

/// Picks a road junction roughly `distance` metres from `origin`.
fn junction_near(city: &City, origin: Vec2, distance: f32) -> Vec3 {
    let best = city
        .graph
        .nodes()
        .min_by(|(_, a), (_, b)| {
            let error = |p: Vec2| (p.distance(origin) - distance).abs();
            error(a.pos).total_cmp(&error(b.pos))
        })
        .map(|(_, node)| node.pos)
        .unwrap_or(origin);
    Vec3::new(best.x, 0.0, best.y)
}

pub fn chain(city: &City) -> Vec<Mission> {
    let start = Vec2::ZERO;

    vec![
        Mission {
            id: "cold_open",
            name: "Cold Open",
            brief: "Find a car and get it to the lock-up. Try not to be seen taking it.",
            objectives: vec![Objective::Reach {
                position: junction_near(city, start, 260.0),
                radius: 9.0,
                in_vehicle: true,
            }],
            reward: 750,
        },
        Mission {
            id: "making_noise",
            name: "Making Noise",
            brief: "Draw them out and keep them busy. Then disappear.",
            objectives: vec![
                Objective::HoldHeat {
                    seconds: 40.0,
                    min_stars: 2,
                },
                Objective::LoseThePolice,
            ],
            reward: 2_000,
        },
    ]
}
