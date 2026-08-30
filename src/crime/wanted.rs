//! The wanted level.
//!
//! One rule carries this whole system: **heat only decays while no officer can
//! see you.** Escaping is therefore an act — breaking line of sight and staying
//! broken — rather than a timer that runs down while you sit in a firefight.
//! Everything else here is bookkeeping around that.
//!
//! The logic is a plain struct with plain methods so the escalation and cooling
//! rules can be tested directly, without spawning a city and a police force.

use bevy::prelude::*;

use avian3d::prelude::*;

use super::events::{CrimeKind, CrimeReported};

/// Anyone who can report a crime: pedestrians and police.
///
/// Kept as a marker here rather than querying the AI types directly, so the
/// crime system does not need to know what kinds of agent exist.
#[derive(Component)]
pub struct Witness;

/// How far a witness can be and still notice.
const WITNESS_RANGE: f32 = 42.0;

/// Heat required for each star. Index 0 is one star.
const STAR_THRESHOLDS: [f32; 5] = [12.0, 45.0, 100.0, 190.0, 320.0];

/// How long the player must stay out of sight before heat starts to fall.
pub const COOLDOWN_DELAY: f32 = 7.0;
/// Heat lost per second once cooling.
const COOLDOWN_RATE: f32 = 9.0;
/// Ceiling, so a rampage cannot bank hours of pursuit.
const MAX_HEAT: f32 = 420.0;

#[derive(Resource, Debug, Clone)]
pub struct Wanted {
    heat: f32,
    /// Seconds since any officer last had eyes on the player.
    pub since_seen: f32,
    /// Where the police think the player is.
    pub last_known: Option<Vec3>,
}

impl Default for Wanted {
    fn default() -> Self {
        Self {
            heat: 0.0,
            since_seen: COOLDOWN_DELAY,
            last_known: None,
        }
    }
}

impl Wanted {
    pub fn heat(&self) -> f32 {
        self.heat
    }

    /// Current wanted level, 0 to 5.
    pub fn stars(&self) -> u8 {
        STAR_THRESHOLDS
            .iter()
            .filter(|threshold| self.heat >= **threshold)
            .count() as u8
    }

    pub fn is_wanted(&self) -> bool {
        self.stars() > 0
    }

    /// Progress towards the next star, 0..1. Drives the HUD's pulsing star.
    pub fn progress_to_next_star(&self) -> f32 {
        let stars = self.stars() as usize;
        if stars >= STAR_THRESHOLDS.len() {
            return 1.0;
        }
        let floor = if stars == 0 {
            0.0
        } else {
            STAR_THRESHOLDS[stars - 1]
        };
        let ceiling = STAR_THRESHOLDS[stars];
        ((self.heat - floor) / (ceiling - floor)).clamp(0.0, 1.0)
    }

    /// Pins a crime on the player, committed at `position`.
    ///
    /// The position matters as much as the heat: it becomes the police's last
    /// known location. Without it, units dispatched to a crime nobody actually
    /// witnessed have nowhere to drive to, and sit at the kerb forever.
    pub fn report(&mut self, kind: CrimeKind, position: Vec3) {
        self.heat = (self.heat + kind.heat()).min(MAX_HEAT);
        // Being caught in the act also resets any cooling in progress.
        self.since_seen = 0.0;
        self.last_known = Some(position);
    }

    /// Advances one frame. `seen` is true if any officer currently has the
    /// player in sight.
    pub fn tick(&mut self, dt: f32, seen: bool, position: Vec3) {
        if seen {
            self.since_seen = 0.0;
            self.last_known = Some(position);
            return;
        }

        self.since_seen += dt;
        if self.since_seen >= COOLDOWN_DELAY {
            self.heat = (self.heat - COOLDOWN_RATE * dt).max(0.0);
            if self.heat <= 0.0 {
                self.last_known = None;
            }
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Restores heat from a save. Starts out of sight, so a loaded game does
    /// not immediately assume the police are already looking at you.
    pub fn restore(&mut self, heat: f32) {
        self.heat = heat.max(0.0);
        self.since_seen = COOLDOWN_DELAY;
        self.last_known = None;
    }
}

pub struct WantedPlugin;

impl Plugin for WantedPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Wanted>()
            .add_message::<CrimeReported>()
            .add_systems(
                Update,
                absorb_crime_reports.in_set(crate::core::schedule::GameSet::Simulation),
            );
    }
}

fn absorb_crime_reports(
    mut reports: MessageReader<CrimeReported>,
    mut wanted: ResMut<Wanted>,
    spatial: SpatialQuery,
    witnesses: Query<(Entity, &GlobalTransform), With<Witness>>,
) {
    for report in reports.read() {
        debug!(
            "crime received: {:?} at {:?} ({} witnesses in world)",
            report.kind,
            report.position,
            witnesses.iter().len()
        );
        if report.kind.needs_witness() && !anyone_saw(&spatial, &witnesses, report.position) {
            debug!("  ...unwitnessed, no heat");
            // Nobody around: a quiet theft costs nothing. This is the rule that
            // makes an empty street feel different from a busy one.
            continue;
        }
        wanted.report(report.kind, report.position);
        debug!(
            "  ...heat now {:.1} ({} stars)",
            wanted.heat(),
            wanted.stars()
        );
    }
}

fn anyone_saw(
    spatial: &SpatialQuery,
    witnesses: &Query<(Entity, &GlobalTransform), With<Witness>>,
    scene: Vec3,
) -> bool {
    witnesses.iter().any(|(entity, transform)| {
        let eye = transform.translation() + Vec3::Y * 0.9;
        let offset = scene - eye;
        let distance = offset.length();
        if distance > WITNESS_RANGE {
            return false;
        }
        let Ok(direction) = Dir3::new(offset) else {
            return true;
        };
        let filter = SpatialQueryFilter::from_excluded_entities([entity]);
        match spatial.cast_ray(eye, direction, distance, true, &filter) {
            Some(hit) => distance - hit.distance < 2.5,
            None => true,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stepped(wanted: &mut Wanted, seconds: f32, seen: bool) {
        let dt = 1.0 / 60.0;
        for _ in 0..((seconds / dt) as usize) {
            wanted.tick(dt, seen, Vec3::ZERO);
        }
    }

    #[test]
    fn a_clean_player_is_not_wanted() {
        let wanted = Wanted::default();
        assert_eq!(wanted.stars(), 0);
        assert!(!wanted.is_wanted());
    }

    #[test]
    fn crimes_escalate_the_star_level() {
        let mut wanted = Wanted::default();
        wanted.report(CrimeKind::VehicleTheft, Vec3::ZERO);
        assert_eq!(wanted.stars(), 1, "petty theft is one star");

        wanted.report(CrimeKind::KilledCivilian, Vec3::ZERO);
        assert_eq!(wanted.stars(), 2);

        for _ in 0..3 {
            wanted.report(CrimeKind::KilledOfficer, Vec3::ZERO);
        }
        assert!(wanted.stars() >= 4, "a cop-killing spree should be serious");
    }

    #[test]
    fn stars_never_exceed_five() {
        let mut wanted = Wanted::default();
        for _ in 0..40 {
            wanted.report(CrimeKind::KilledOfficer, Vec3::ZERO);
        }
        assert_eq!(wanted.stars(), 5);
        assert!(wanted.heat() <= MAX_HEAT);
    }

    #[test]
    fn heat_does_not_fall_while_you_are_being_watched() {
        let mut wanted = Wanted::default();
        wanted.report(CrimeKind::KilledCivilian, Vec3::ZERO);
        let before = wanted.heat();

        // A full minute in plain sight.
        stepped(&mut wanted, 60.0, true);
        assert_eq!(
            wanted.heat(),
            before,
            "cooling must not happen under observation"
        );
    }

    #[test]
    fn breaking_line_of_sight_eventually_clears_it() {
        let mut wanted = Wanted::default();
        wanted.report(CrimeKind::VehicleTheft, Vec3::ZERO);
        assert_eq!(wanted.stars(), 1);

        // Hidden, but not yet long enough.
        stepped(&mut wanted, COOLDOWN_DELAY - 1.0, false);
        assert_eq!(wanted.stars(), 1, "cooling started too early");

        stepped(&mut wanted, 30.0, false);
        assert_eq!(wanted.stars(), 0, "hiding should eventually shake them");
        assert!(wanted.last_known.is_none());
    }

    #[test]
    fn being_spotted_again_restarts_the_clock() {
        let mut wanted = Wanted::default();
        wanted.report(CrimeKind::KilledCivilian, Vec3::ZERO);

        stepped(&mut wanted, COOLDOWN_DELAY + 2.0, false);
        let cooled = wanted.heat();
        assert!(cooled < CrimeKind::KilledCivilian.heat());

        // Spotted: cooling stops dead.
        stepped(&mut wanted, 5.0, true);
        assert_eq!(wanted.heat(), cooled);
        assert_eq!(wanted.since_seen, 0.0);
    }

    #[test]
    fn a_reported_crime_gives_the_police_somewhere_to_go() {
        // Without this, units dispatched to an unwitnessed crime have no
        // destination and never leave the kerb.
        let mut wanted = Wanted::default();
        let scene = Vec3::new(30.0, 0.0, -12.0);
        wanted.report(CrimeKind::Gunfire, scene);
        assert_eq!(wanted.last_known, Some(scene));
    }

    #[test]
    fn police_remember_where_they_last_saw_you() {
        let mut wanted = Wanted::default();
        wanted.report(CrimeKind::Gunfire, Vec3::ZERO);
        wanted.tick(0.016, true, Vec3::new(10.0, 0.0, -20.0));
        assert_eq!(wanted.last_known, Some(Vec3::new(10.0, 0.0, -20.0)));

        // They keep believing it after losing sight.
        stepped(&mut wanted, 3.0, false);
        assert_eq!(wanted.last_known, Some(Vec3::new(10.0, 0.0, -20.0)));
    }

    #[test]
    fn progress_runs_between_stars() {
        let mut wanted = Wanted::default();
        assert_eq!(wanted.progress_to_next_star(), 0.0);
        wanted.report(CrimeKind::VehicleTheft, Vec3::ZERO);
        let progress = wanted.progress_to_next_star();
        assert!(
            (0.0..1.0).contains(&progress),
            "progress was {progress} at {} heat",
            wanted.heat()
        );
    }
}
