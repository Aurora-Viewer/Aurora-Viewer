//! AURORA_DEMO_RT_SWITCH=<frame>[:<frame>]: ray tracing turned on while the
//! world shows, as Preferences › Graphics would: the sun's shadows at the
//! first frame, then at the second the local lights' shadows, the indirect
//! light and the reflections. The ray tracer and what the effects need are
//! made on a worker thread (aurora-render's renderer.rs `RtBuild`): the
//! frames go on meanwhile, and the log gives the longest frame before and
//! after each step.

use std::time::Instant;

/// Frames watched before the first step, and after each.
const WATCHED: u64 = 240;

/// What a step turns on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Shadows,
    Effects,
}

pub struct RtSwitch {
    steps: Vec<(u64, Step)>,
    last: Option<Instant>,
    /// The longest frame of the watch going on (ms), and the frame its
    /// report is due at.
    longest: f32,
    report: Option<(u64, Step)>,
    /// The frame and the time of the first step, until the ray tracer
    /// draws.
    waiting: Option<(u64, Instant)>,
}

/// None when the variable is unset or unreadable (logged).
pub fn from_env() -> Option<RtSwitch> {
    let v = std::env::var("AURORA_DEMO_RT_SWITCH").ok()?;
    let steps = parse(&v);
    if steps.is_empty() {
        log::warn!("AURORA_DEMO_RT_SWITCH: `{v}` not read (expected <frame> or <frame>:<frame>)");
        return None;
    }
    Some(RtSwitch {
        steps,
        last: None,
        longest: 0.0,
        report: None,
        waiting: None,
    })
}

/// The steps in their order: one frame, or two that follow each other far
/// enough for the first one's report.
fn parse(v: &str) -> Vec<(u64, Step)> {
    let mut frames = v.split(':').map(|f| f.trim().parse::<u64>().ok());
    match (frames.next().flatten(), frames.next(), frames.next()) {
        (Some(a), None, _) => vec![(a, Step::Shadows)],
        (Some(a), Some(Some(b)), None) if b > a + WATCHED => vec![(a, Step::Shadows), (b, Step::Effects)],
        _ => Vec::new(),
    }
}

impl RtSwitch {
    /// Once per frame: `tracing` says the ray tracer drew the last frame
    /// (`Renderer::rt_tracing`). The step to apply now, if any.
    pub fn tick(&mut self, frame: u64, now: Instant, tracing: bool) -> Option<Step> {
        let spent = self.last.replace(now).map_or(0.0, |t| now.duration_since(t).as_secs_f32() * 1000.0);
        let next = self.steps.first().copied();
        // (the frames before the first step, then those after each)
        let watched = self.report.is_some() || next.is_some_and(|(at, step)| step == Step::Shadows && frame + WATCHED > at);
        if watched {
            self.longest = self.longest.max(spent);
        }
        if let Some((since, at)) = self.waiting.filter(|_| tracing) {
            self.waiting = None;
            log::info!(
                "demo rt switch: the sun's shadows traced {} frames ({} ms) after they were turned on",
                frame - since,
                now.duration_since(at).as_millis()
            );
        }
        if let Some((due, step)) = self.report.filter(|(due, _)| frame >= *due) {
            self.report = None;
            let what = match step {
                Step::Shadows => "the sun's shadows",
                Step::Effects => "the effects",
            };
            log::info!(
                "demo rt switch: the longest of the {WATCHED} frames after {what} were turned on took {:.1} ms (frame {due})",
                self.longest
            );
            self.longest = 0.0;
        }
        let (at, step) = next.filter(|(at, _)| frame >= *at)?;
        self.steps.remove(0);
        match step {
            Step::Shadows => log::info!(
                "demo rt switch: the sun's shadows turned on at frame {frame}; the longest of the {WATCHED} frames before took {:.1} ms",
                self.longest
            ),
            Step::Effects => {
                log::info!("demo rt switch: the lights' shadows, the indirect light and the reflections turned on at frame {frame}")
            }
        }
        self.longest = 0.0;
        self.report = Some((at.max(frame) + WATCHED, step));
        if step == Step::Shadows && !tracing {
            self.waiting = Some((frame, now));
        }
        Some(step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn reads_one_frame_or_two_far_enough_apart() {
        assert_eq!(parse("700"), vec![(700, Step::Shadows)]);
        assert_eq!(parse(" 700 : 1000 "), vec![(700, Step::Shadows), (1000, Step::Effects)]);
        // the second step before the first one's report, or unreadable
        assert!(parse("700:800").is_empty());
        assert!(parse("700:x").is_empty());
        assert!(parse("700:1000:1300").is_empty());
        assert!(parse("").is_empty());
    }

    #[test]
    fn each_step_comes_once_at_its_frame() {
        let mut d = RtSwitch {
            steps: parse("700:1000"),
            last: None,
            longest: 0.0,
            report: None,
            waiting: None,
        };
        let start = Instant::now();
        let mut steps = Vec::new();
        for frame in 0..1300u64 {
            // one long frame after the first step
            let ms = frame * 4 + if frame > 705 { 500 } else { 0 };
            if let Some(step) = d.tick(frame, start + Duration::from_millis(ms), frame > 720) {
                steps.push((frame, step));
            }
            if frame == 705 {
                assert!((d.longest - 4.0).abs() < 0.01, "{}", d.longest);
            }
            if frame == 706 {
                assert!((d.longest - 504.0).abs() < 0.01, "{}", d.longest);
                assert!(d.waiting.is_some());
            }
        }
        assert_eq!(steps, vec![(700, Step::Shadows), (1000, Step::Effects)]);
        assert!(d.waiting.is_none() && d.report.is_none());
    }
}
