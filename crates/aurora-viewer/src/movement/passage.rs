//! Match physical outcomes to the requested action; receipt alone is not success.
use aurora_net::bridge::PassageReport;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Feedback {
    pending: Option<(u32, Instant)>,
    previous: Option<PassageReport>,
}
impl Feedback {
    pub fn start(&mut self, action: u32, now: Instant) {
        self.pending = Some((action, now + Duration::from_secs(5)));
        self.previous = None;
    }
    pub fn update(&mut self, report: Option<PassageReport>, now: Instant) -> Option<String> {
        let (action, deadline) = self.pending?;
        if let Some(report) = report.filter(|report| report.action == action && (now < deadline || report.finished())) {
            if report.finished() {
                self.pending = None;
            }
            if self.previous != Some(report) {
                self.previous = Some(report);
                let state = aurora_i18n::text(match report.state {
                    1 => "movement-pass-accepted",
                    2 => "movement-pass-driving",
                    3 => "movement-pass-reached",
                    4 => "movement-pass-not-reached",
                    5 => "movement-pass-cancelled",
                    6 => "movement-pass-locked",
                    _ => "movement-run-invalid-mass",
                });
                return Some(aurora_i18n::tr!(
                    "movement-pass-feedback",
                    state = state,
                    progress = format!("{:.2}", report.progress_cm as f32 / 100.0),
                    remaining = format!("{:.2}", report.remaining_cm as f32 / 100.0),
                    speed = format!("{:.1}", report.speed_cm as f32 / 100.0)
                ));
            }
        }
        if now >= deadline {
            self.pending = None;
            return Some(aurora_i18n::tr!("movement-pass-no-result").into());
        }
        None
    }
}
