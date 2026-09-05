//! Squash and stretch.
//!
//! A rigid body tracing a parabola is a physics demo. What makes it read as
//! rubber is what the *shape* does either side of the contact: flattened at the
//! bottom, drawn out along the direction of travel on the way up and down, back
//! to itself at the top. That is the oldest trick in animation and it is worth
//! more here than any amount of solver tuning, because the bounce the eye
//! believes is the one it can see the body preparing for.
//!
//! Kept as one pure function rather than a system so the curve can be argued
//! about in a test. It is applied by [`crate::ai::figure::animate`], which is
//! where the rest of a figure's pose already lives.

/// How a figure is stretched at a point in its hop.
///
/// `phase` runs 0 at the moment of landing to 1 at the next one, and `amount`
/// is the depth of the squash as a fraction of height. Returns a vertical scale
/// and the horizontal scale that conserves the figure's bulk — a body that
/// squashes without spreading has not been squashed, it has shrunk.
pub fn stretch(phase: f32, amount: f32) -> (f32, f32) {
    // A cosine over the whole arc: hardest at the two ends, where the body is
    // near the ground, and neutral in the middle. Squaring it tightens the
    // squash into the moment of contact rather than smearing it over the arc,
    // which is the difference between a bounce and a wobble.
    let ground = (phase * std::f32::consts::TAU).cos().max(0.0).powi(2);
    // Above the halfway mark of the squash curve the body is instead drawn out
    // along its travel, at a third of the depth: stretch reads much stronger
    // than squash, so matching them makes a figure look like elastic.
    let squash = ground * amount;
    let stretch = (1.0 - ground) * amount * 0.33;
    let vertical = 1.0 - squash + stretch;
    // Constant volume, near enough: a real one would be 1/sqrt(vertical), and
    // the difference over this range is smaller than the line width of a leg.
    let horizontal = 1.0 + squash * 0.75 - stretch * 0.5;
    (vertical, horizontal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_figure_is_flattest_at_the_moment_it_lands() {
        let (landing, _) = stretch(0.0, 0.35);
        let (midair, _) = stretch(0.5, 0.35);
        assert!(
            landing < midair,
            "landed at {landing:.2} against {midair:.2} in the air"
        );
    }

    #[test]
    fn squashing_spreads_rather_than_shrinks() {
        let (vertical, horizontal) = stretch(0.0, 0.35);
        assert!(vertical < 1.0, "not squashed at all");
        assert!(
            horizontal > 1.0,
            "squashed to {vertical:.2} tall without spreading past {horizontal:.2} wide"
        );
    }

    #[test]
    fn the_top_of_the_arc_is_drawn_out_rather_than_flattened() {
        let (vertical, horizontal) = stretch(0.5, 0.35);
        assert!(vertical > 1.0, "not stretched at the apex");
        assert!(horizontal < 1.0, "stretched without narrowing");
    }

    #[test]
    fn the_pose_closes_on_itself() {
        // The end of one hop is the start of the next. A mismatch here is a pop
        // once per bounce, several times a second, on every figure on screen.
        let (start, _) = stretch(0.0, 0.35);
        let (end, _) = stretch(1.0, 0.35);
        assert!((start - end).abs() < 1e-5, "{start} against {end}");
    }

    #[test]
    fn nothing_moves_when_the_effect_is_turned_off() {
        for step in 0..16 {
            let (vertical, horizontal) = stretch(step as f32 / 16.0, 0.0);
            assert_eq!((vertical, horizontal), (1.0, 1.0));
        }
    }
}
