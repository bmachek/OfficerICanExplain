//! Health and armour for anything that can be killed.

use bevy::prelude::*;

#[derive(Component, Debug, Clone)]
pub struct Health {
    pub current: f32,
    pub max: f32,
    /// Absorbed before health, and never regenerates on its own.
    pub armor: f32,
}

impl Health {
    pub fn new(max: f32) -> Self {
        Self {
            current: max,
            max,
            armor: 0.0,
        }
    }

    pub fn with_armor(mut self, armor: f32) -> Self {
        self.armor = armor;
        self
    }

    /// Applies damage, armour first. Returns true if this was the killing blow.
    pub fn damage(&mut self, amount: f32) -> bool {
        if amount <= 0.0 || self.is_dead() {
            return false;
        }
        let absorbed = self.armor.min(amount);
        self.armor -= absorbed;
        self.current = (self.current - (amount - absorbed)).max(0.0);
        self.is_dead()
    }

    pub fn heal(&mut self, amount: f32) {
        self.current = (self.current + amount).min(self.max);
    }

    pub fn is_dead(&self) -> bool {
        self.current <= 0.0
    }

    pub fn fraction(&self) -> f32 {
        (self.current / self.max).clamp(0.0, 1.0)
    }
}

/// Raised when something with [`Health`] is killed.
#[derive(Message, Debug, Clone, Copy)]
pub struct Died {
    pub entity: Entity,
    pub position: Vec3,
    /// True when the player caused it.
    pub by_player: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn armor_absorbs_before_health() {
        let mut health = Health::new(100.0).with_armor(50.0);
        health.damage(30.0);
        assert_eq!(health.armor, 20.0);
        assert_eq!(health.current, 100.0, "health should be untouched");
    }

    #[test]
    fn damage_spills_through_depleted_armor() {
        let mut health = Health::new(100.0).with_armor(20.0);
        health.damage(50.0);
        assert_eq!(health.armor, 0.0);
        assert_eq!(health.current, 70.0, "the extra 30 should hit health");
    }

    #[test]
    fn death_is_reported_once() {
        let mut health = Health::new(30.0);
        assert!(!health.damage(10.0));
        assert!(health.damage(25.0), "that should have been fatal");
        assert!(health.is_dead());
        assert_eq!(health.current, 0.0, "health should not go negative");
        assert!(!health.damage(10.0), "already dead, no second report");
    }

    #[test]
    fn healing_stops_at_maximum() {
        let mut health = Health::new(100.0);
        health.damage(60.0);
        health.heal(500.0);
        assert_eq!(health.current, 100.0);
    }
}
