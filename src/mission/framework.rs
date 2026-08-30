//! Mission objectives as a state machine.
//!
//! Missions are a list of objectives evaluated against a snapshot of the world,
//! advanced by a pure function. That split is deliberate: mission logic is
//! exactly the kind of code that accretes special cases, and keeping the rules
//! out of the ECS means every branch is reachable from a unit test instead of
//! only by driving across a city to trigger it.

use bevy::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Objective {
    /// Get to a place, optionally behind the wheel.
    Reach {
        position: Vec3,
        radius: f32,
        in_vehicle: bool,
    },
    /// Stay at or above a wanted level for a while.
    HoldHeat { seconds: f32, min_stars: u8 },
    /// Shake the police completely.
    LoseThePolice,
}

impl Objective {
    /// One line of instruction for the HUD.
    pub fn describe(&self) -> String {
        match self {
            Objective::Reach {
                in_vehicle: true, ..
            } => "Deliver the car to the drop-off".into(),
            Objective::Reach { .. } => "Get to the marker".into(),
            Objective::HoldHeat {
                seconds, min_stars, ..
            } => format!("Keep {min_stars}+ stars for {seconds:.0}s"),
            Objective::LoseThePolice => "Lose the police".into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Mission {
    pub id: &'static str,
    pub name: &'static str,
    pub brief: &'static str,
    pub objectives: Vec<Objective>,
    pub reward: u32,
}

/// Everything the mission rules are allowed to look at.
#[derive(Debug, Clone, Copy)]
pub struct WorldSnapshot {
    pub player: Vec3,
    pub in_vehicle: bool,
    pub stars: u8,
    pub dt: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Still working on the current objective.
    Working,
    /// Objective done, moved to the next.
    Advanced,
    /// Every objective done.
    Complete,
}

#[derive(Resource, Debug, Clone)]
pub struct ActiveMission {
    pub mission: Mission,
    pub index: usize,
    /// Time accumulated against the current objective.
    pub elapsed: f32,
    pub finished: bool,
}

impl ActiveMission {
    pub fn new(mission: Mission) -> Self {
        Self {
            mission,
            index: 0,
            elapsed: 0.0,
            finished: false,
        }
    }

    pub fn current(&self) -> Option<&Objective> {
        self.mission.objectives.get(self.index)
    }

    /// Progress through the objective list, 0..1.
    pub fn fraction(&self) -> f32 {
        if self.mission.objectives.is_empty() {
            return 1.0;
        }
        self.index as f32 / self.mission.objectives.len() as f32
    }

    /// Evaluates the current objective and advances if it is satisfied.
    pub fn advance(&mut self, world: &WorldSnapshot) -> Progress {
        if self.finished {
            return Progress::Complete;
        }
        let Some(objective) = self.current().copied() else {
            self.finished = true;
            return Progress::Complete;
        };

        self.elapsed += world.dt;
        if !satisfied(&objective, world, self.elapsed) {
            return Progress::Working;
        }

        self.index += 1;
        self.elapsed = 0.0;
        if self.index >= self.mission.objectives.len() {
            self.finished = true;
            Progress::Complete
        } else {
            Progress::Advanced
        }
    }
}

/// Whether an objective's condition currently holds.
fn satisfied(objective: &Objective, world: &WorldSnapshot, elapsed: f32) -> bool {
    match objective {
        Objective::Reach {
            position,
            radius,
            in_vehicle,
        } => {
            // Compared on the ground plane: the marker is at street level and
            // the player's origin is not, so a 3D distance never quite closes.
            let flat = world.player.xz().distance(position.xz());
            flat <= *radius && (!*in_vehicle || world.in_vehicle)
        }
        Objective::HoldHeat { seconds, min_stars } => {
            // The timer only counts while the condition holds; dropping below
            // the required heat resets progress rather than failing outright.
            world.stars >= *min_stars && elapsed >= *seconds
        }
        Objective::LoseThePolice => world.stars == 0,
    }
}

/// Resets the hold timer when its condition lapses, so the player must sustain
/// it rather than accumulate it across separate attempts.
pub fn maintain(active: &mut ActiveMission, world: &WorldSnapshot) {
    if let Some(Objective::HoldHeat { min_stars, .. }) = active.current()
        && world.stars < *min_stars
    {
        active.elapsed = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> WorldSnapshot {
        WorldSnapshot {
            player: Vec3::ZERO,
            in_vehicle: false,
            stars: 0,
            dt: 1.0 / 60.0,
        }
    }

    fn mission(objectives: Vec<Objective>) -> ActiveMission {
        ActiveMission::new(Mission {
            id: "test",
            name: "Test",
            brief: "",
            objectives,
            reward: 500,
        })
    }

    #[test]
    fn reaching_a_marker_completes_the_objective() {
        let target = Vec3::new(50.0, 0.0, 0.0);
        let mut active = mission(vec![Objective::Reach {
            position: target,
            radius: 6.0,
            in_vehicle: false,
        }]);

        let mut world = snapshot();
        world.player = Vec3::new(20.0, 0.0, 0.0);
        assert_eq!(active.advance(&world), Progress::Working);

        world.player = Vec3::new(48.0, 0.0, 0.0);
        assert_eq!(active.advance(&world), Progress::Complete);
        assert!(active.finished);
    }

    #[test]
    fn height_does_not_stop_you_reaching_a_marker() {
        // The player's origin sits above the street, and a car's higher still.
        // Measuring in 3D would leave the objective permanently just out of reach.
        let mut active = mission(vec![Objective::Reach {
            position: Vec3::new(10.0, 0.0, 0.0),
            radius: 5.0,
            in_vehicle: true,
        }]);
        let mut world = snapshot();
        world.in_vehicle = true;
        world.player = Vec3::new(10.0, 3.5, 0.0);
        assert_eq!(active.advance(&world), Progress::Complete);
    }

    #[test]
    fn a_delivery_objective_requires_the_car() {
        let mut active = mission(vec![Objective::Reach {
            position: Vec3::ZERO,
            radius: 8.0,
            in_vehicle: true,
        }]);

        let mut world = snapshot();
        world.player = Vec3::ZERO;
        world.in_vehicle = false;
        assert_eq!(
            active.advance(&world),
            Progress::Working,
            "arriving on foot should not count"
        );

        world.in_vehicle = true;
        assert_eq!(active.advance(&world), Progress::Complete);
    }

    #[test]
    fn objectives_run_in_order() {
        let mut active = mission(vec![
            Objective::Reach {
                position: Vec3::ZERO,
                radius: 5.0,
                in_vehicle: false,
            },
            Objective::LoseThePolice,
        ]);

        let mut world = snapshot();
        world.player = Vec3::ZERO;
        world.stars = 3;
        assert_eq!(active.advance(&world), Progress::Advanced);
        assert_eq!(active.index, 1);
        assert!(!active.finished, "second objective still outstanding");

        assert_eq!(active.advance(&world), Progress::Working, "still wanted");
        world.stars = 0;
        assert_eq!(active.advance(&world), Progress::Complete);
    }

    #[test]
    fn holding_heat_needs_the_time_sustained() {
        let mut active = mission(vec![Objective::HoldHeat {
            seconds: 2.0,
            min_stars: 2,
        }]);
        let mut world = snapshot();
        world.stars = 3;
        world.dt = 0.5;

        for _ in 0..3 {
            assert_eq!(active.advance(&world), Progress::Working);
        }
        assert_eq!(active.advance(&world), Progress::Complete);
    }

    #[test]
    fn losing_the_heat_resets_the_hold_timer() {
        let mut active = mission(vec![Objective::HoldHeat {
            seconds: 3.0,
            min_stars: 2,
        }]);
        let mut world = snapshot();
        world.dt = 0.5;
        world.stars = 3;

        for _ in 0..4 {
            active.advance(&world);
        }
        assert!(active.elapsed > 1.5);

        // Cool off: the clock goes back to zero rather than banking progress.
        world.stars = 0;
        maintain(&mut active, &world);
        assert_eq!(active.elapsed, 0.0);
    }

    #[test]
    fn a_finished_mission_stays_finished() {
        let mut active = mission(vec![Objective::LoseThePolice]);
        let world = snapshot();
        assert_eq!(active.advance(&world), Progress::Complete);
        assert_eq!(active.advance(&world), Progress::Complete);
        assert_eq!(active.index, 1, "must not run off the end of the list");
    }

    #[test]
    fn progress_fraction_tracks_the_objective_list() {
        let mut active = mission(vec![
            Objective::LoseThePolice,
            Objective::LoseThePolice,
            Objective::LoseThePolice,
        ]);
        assert_eq!(active.fraction(), 0.0);
        let world = snapshot();
        active.advance(&world);
        assert!((active.fraction() - 1.0 / 3.0).abs() < 1e-5);
    }
}
