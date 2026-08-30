//! What counts as a crime, and how badly the police want you for it.

use bevy::prelude::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrimeKind {
    /// Taking a car that is not yours.
    VehicleTheft,
    /// Discharging a weapon in public.
    Gunfire,
    /// Running someone over and driving on.
    HitAndRun,
    KilledCivilian,
    DestroyedVehicle,
    AssaultedOfficer,
    KilledOfficer,
}

impl CrimeKind {
    /// Heat added when this is pinned on the player.
    pub fn heat(self) -> f32 {
        match self {
            CrimeKind::VehicleTheft => 14.0,
            CrimeKind::Gunfire => 12.0,
            CrimeKind::HitAndRun => 22.0,
            CrimeKind::KilledCivilian => 45.0,
            CrimeKind::DestroyedVehicle => 26.0,
            CrimeKind::AssaultedOfficer => 60.0,
            CrimeKind::KilledOfficer => 130.0,
        }
    }

    /// Whether somebody has to see it for it to count.
    ///
    /// Stealing a car on an empty street is free; shooting one is not, because
    /// the noise carries. This distinction is what makes the city feel like it
    /// is watching rather than omniscient.
    pub fn needs_witness(self) -> bool {
        match self {
            CrimeKind::VehicleTheft | CrimeKind::HitAndRun => true,
            CrimeKind::Gunfire
            | CrimeKind::KilledCivilian
            | CrimeKind::DestroyedVehicle
            | CrimeKind::AssaultedOfficer
            | CrimeKind::KilledOfficer => false,
        }
    }
}

/// Raised by gameplay systems whenever the player does something illegal.
#[derive(Message, Debug, Clone, Copy)]
pub struct CrimeReported {
    pub kind: CrimeKind,
    pub position: Vec3,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_is_ordered_sensibly() {
        assert!(CrimeKind::KilledOfficer.heat() > CrimeKind::AssaultedOfficer.heat());
        assert!(CrimeKind::AssaultedOfficer.heat() > CrimeKind::KilledCivilian.heat());
        assert!(CrimeKind::KilledCivilian.heat() > CrimeKind::HitAndRun.heat());
        assert!(CrimeKind::HitAndRun.heat() > CrimeKind::VehicleTheft.heat());
    }

    #[test]
    fn loud_crimes_need_no_witness() {
        assert!(!CrimeKind::Gunfire.needs_witness());
        assert!(!CrimeKind::KilledOfficer.needs_witness());
        assert!(CrimeKind::VehicleTheft.needs_witness());
    }
}
