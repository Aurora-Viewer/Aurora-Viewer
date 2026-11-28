//! Original fine-movement behavior by this contribution's author, first
//! implemented in a personal Legacy Viewer fork; adapted here for Aurora.
//! Optional: ordinary camera controls retain their existing acceleration.

pub fn control_rate(held: f32, precision: bool) -> f32 {
    if precision {
        if held < crate::app::NUDGE_TIME { 0.01 } else { 0.25 }
    } else {
        0.05 + (held / crate::app::NUDGE_TIME).clamp(0.0, 1.0) * 0.95
    }
}

/// Continuous photo pads and free-camera movement have no click/hold phase.
pub fn precision_scale(precision: bool) -> f32 {
    if precision { 0.25 } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fine_taps_do_not_accelerate_and_holds_remain_slow() {
        for held in [0.0, 0.05, 0.1, 0.249] {
            assert_eq!(control_rate(held, true), 0.01);
        }
        for held in [0.25, 1.0, 60.0] {
            assert_eq!(control_rate(held, true), 0.25);
        }
        assert_eq!(control_rate(0.0, false), 0.05);
        assert_eq!(control_rate(0.125, false), 0.525);
        assert_eq!(control_rate(1.0, false), 1.0);
        assert_eq!(precision_scale(true), 0.25);
        assert_eq!(precision_scale(false), 1.0);
    }
}
