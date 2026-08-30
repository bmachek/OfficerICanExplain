//! Shared steering maths for anything that drives or walks a route.
//!
//! Kept as free functions over plain vectors rather than systems over
//! components, so the geometry that decides whether a car turns the right way
//! can be tested directly instead of inferred from watching traffic.

use bevy::prelude::*;

/// Which side of the road traffic drives on. Right-hand rule.
pub const RIGHT_HAND_TRAFFIC: bool = true;

/// Unit normal pointing to the right of `direction` in the XZ plane.
///
/// Derived from `cross(forward, up)`, which is how Bevy defines a transform's
/// right axis: for forward `(dx, 0, dz)` and up `+Y` that is `(-dz, 0, dx)`.
/// Getting this backwards is invisible in a static screenshot and puts every
/// car in the oncoming lane, so it has its own test.
pub fn right_of(direction: Vec2) -> Vec2 {
    Vec2::new(-direction.y, direction.x)
}

/// A point in the correct travel lane along the segment `a -> b`.
///
/// `t` runs 0..1 along the segment; the result is offset sideways so vehicles
/// keep to their own half of the carriageway rather than driving the centreline
/// head-on into oncoming traffic.
pub fn lane_point(a: Vec2, b: Vec2, width: f32, t: f32) -> Vec2 {
    let Ok(direction) = Dir2::new(b - a) else {
        return a;
    };
    let side = right_of(*direction) * (width * 0.25);
    let centre = a.lerp(b, t);
    if RIGHT_HAND_TRAFFIC {
        centre + side
    } else {
        centre - side
    }
}

/// Flattened forward and right axes of a transform, in the XZ plane.
pub fn ground_axes(transform: &Transform) -> (Vec2, Vec2) {
    let forward = transform.forward();
    let right = transform.right();
    (
        Vec2::new(forward.x, forward.z).normalize_or_zero(),
        Vec2::new(right.x, right.z).normalize_or_zero(),
    )
}

/// Steering input in -1..1 that turns towards `to_target`.
///
/// Positive is right, matching `VehicleInput::steer`.
pub fn steer_towards(forward: Vec2, right: Vec2, to_target: Vec2) -> f32 {
    let Ok(direction) = Dir2::new(to_target) else {
        return 0.0;
    };
    let lateral = direction.dot(right);
    let ahead = direction.dot(forward);

    if ahead <= 0.0 {
        // Target is behind us; commit to full lock on the side it lies on
        // rather than letting a near-zero lateral component dither.
        if lateral >= 0.0 { 1.0 } else { -1.0 }
    } else {
        (lateral * 2.5).clamp(-1.0, 1.0)
    }
}

/// Throttle in -1..1 to converge on `desired` from `current` speed (m/s).
pub fn throttle_for_speed(current: f32, desired: f32) -> f32 {
    let error = desired - current;
    // Deadband stops the AI oscillating on and off the power at cruise.
    if error.abs() < 0.4 {
        0.0
    } else {
        (error * 0.35).clamp(-1.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn right_of_matches_bevys_right_axis() {
        // Facing +Z, a transform's right axis points towards -X.
        let forward = Vec3::new(0.0, 0.0, 1.0);
        let expected = forward.cross(Vec3::Y).normalize();
        let got = right_of(Vec2::new(0.0, 1.0));
        assert!(
            (got - Vec2::new(expected.x, expected.z)).length() < 1e-5,
            "right_of gave {got:?}, cross product says {expected:?}"
        );
    }

    #[test]
    fn lane_point_keeps_right_of_the_centreline() {
        let a = Vec2::ZERO;
        let b = Vec2::new(0.0, 100.0); // heading towards +Z
        let point = lane_point(a, b, 10.0, 0.5);

        // Travelling +Z, "right" is -X.
        assert!(point.x < 0.0, "lane point {point:?} is on the wrong side");
        assert!((point.y - 50.0).abs() < 1e-4);
        assert!(
            (point.x.abs() - 2.5).abs() < 1e-4,
            "offset should be width/4"
        );
    }

    #[test]
    fn opposing_directions_use_opposite_lanes() {
        let a = Vec2::ZERO;
        let b = Vec2::new(100.0, 0.0);
        let outbound = lane_point(a, b, 12.0, 0.5);
        let inbound = lane_point(b, a, 12.0, 0.5);
        assert!(
            (outbound.y - inbound.y).abs() > 1.0,
            "traffic in both directions landed in the same lane: {outbound:?} / {inbound:?}"
        );
    }

    #[test]
    fn steering_turns_the_shorter_way() {
        let forward = Vec2::new(0.0, -1.0);
        let right = Vec2::new(1.0, 0.0);

        assert!(
            steer_towards(forward, right, Vec2::new(10.0, -10.0)) > 0.0,
            "right"
        );
        assert!(
            steer_towards(forward, right, Vec2::new(-10.0, -10.0)) < 0.0,
            "left"
        );
        assert!(
            steer_towards(forward, right, Vec2::new(0.0, -10.0)).abs() < 1e-3,
            "straight ahead needs no steering"
        );
    }

    #[test]
    fn a_target_behind_gets_full_lock() {
        let forward = Vec2::new(0.0, -1.0);
        let right = Vec2::new(1.0, 0.0);
        assert_eq!(steer_towards(forward, right, Vec2::new(1.0, 10.0)), 1.0);
        assert_eq!(steer_towards(forward, right, Vec2::new(-1.0, 10.0)), -1.0);
    }

    #[test]
    fn throttle_closes_on_the_target_speed() {
        assert!(
            throttle_for_speed(0.0, 12.0) > 0.5,
            "should accelerate hard"
        );
        assert!(throttle_for_speed(20.0, 12.0) < 0.0, "should back off");
        assert_eq!(throttle_for_speed(12.0, 12.0), 0.0, "cruise is hands-off");
        assert!(throttle_for_speed(11.8, 12.0).abs() < 1e-6, "deadband");
    }
}
