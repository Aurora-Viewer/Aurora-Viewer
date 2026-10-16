//! Short contact-loss tolerance for enhanced running, never a general fall override.
use super::Frame;
use std::time::{Duration, Instant};

/// Actual script feedback, not an inference from the selected multiplier.
pub fn diagnostic(report: Option<aurora_net::bridge::MotorReport>) -> String {
    let Some(report) = report else {
        return aurora_i18n::tr!("movement-run-awaiting-script").into();
    };
    if report.mode != 1 {
        return aurora_i18n::tr!("movement-run-no-command").into();
    }
    let state = aurora_i18n::text(match report.run_state {
        1 => "movement-run-no-controls",
        2 => "movement-run-airborne",
        3 => "movement-run-locked",
        4 => "movement-run-at-speed",
        5 => "movement-run-thrust",
        6 => "movement-run-invalid-mass",
        7 => "movement-run-expired",
        8 => "movement-run-jumping",
        9 => "movement-run-seated",
        10 => "movement-run-ramping",
        11 => "movement-run-saturated",
        _ => "movement-run-no-command",
    });
    let mut status = aurora_i18n::tr!(
        "movement-run-feedback",
        state = state,
        target = format!("{:.1}", report.target_cm as f32 / 100.0),
        speed = format!("{:.1}", report.speed_cm as f32 / 100.0),
        pushes = report.pushes
    );
    if let Some(regulation) = report.regulation {
        status.push('\n');
        status.push_str(&aurora_i18n::tr!(
            "movement-run-regulation",
            power = format!("{:.1}", regulation.power_milli as f32 / 4000.0),
            ramp = format!("{:.1}", regulation.ramp_cm as f32 / 100.0),
            restarts = regulation.restarts
        ));
    }
    status
}

pub fn on_ground(position: glam::Vec3, height: f32, plane: Option<glam::Vec4>, terrain: Option<f32>, falling: bool) -> bool {
    if falling || !position.is_finite() || !height.is_finite() {
        return false;
    }
    let clearance = match plane {
        Some(plane) if plane.is_finite() && plane.z > 0.1 => (position.dot(plane.truncate()) - plane.w) / plane.z,
        Some(_) => return false, // An explicit cleared plane reports airborne.
        None => match terrain.filter(|height| height.is_finite()) {
            Some(ground) => position.z - ground,
            None => return false,
        },
    };
    (0.0..=height * 0.6 + 0.2).contains(&clearance)
}

#[derive(Default)]
pub struct Guard {
    supported: Option<(Instant, f32)>,
    start: Option<Instant>,
}
impl Guard {
    pub fn update(&mut self, requested: bool, frame: &Frame, now: Instant) -> Option<Instant> {
        if !requested || frame.flying || frame.seated || frame.suspended || frame.locked {
            self.supported = None;
            self.start = None;
            return None;
        }
        if frame.on_ground && frame.velocity.z.abs() <= 1.5 {
            self.supported = Some((now, frame.position.z));
        }
        let supported = self.supported.is_some_and(|(last, height)| {
            now.saturating_duration_since(last) <= Duration::from_millis(250)
                && (height - frame.position.z).abs() <= 0.35
                && frame.velocity.z.abs() <= 1.5
        });
        if supported {
            Some(*self.start.get_or_insert(now))
        } else {
            self.start = None;
            None
        }
    }
}
