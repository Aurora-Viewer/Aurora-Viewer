//! AURORA_SYNC_SLOW=<ms>: one log line for each frame whose scene sync
//! took longer than `ms` (8 if not a number), with the time of each of its
//! steps and what the full syncs of the frame were. The `perf summary`
//! line only gives the slowest frame of each second by part (`s_sync_apply`
//! 12 ms at KB with nothing rebuilt): this line names the step and the
//! objects. Costs nothing when the variable is not set.

use std::time::Instant;

/// What the full syncs of a frame were: avatars, objects listed for a new
/// geometry or faces, new objects, objects that waited in the backlog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Full {
    Avatar,
    Listed,
    Arrival,
    Backlog,
}

const FULLS: [(Full, &str); 4] = [
    (Full::Avatar, "avatars"),
    (Full::Listed, "listed"),
    (Full::Arrival, "arrivals"),
    (Full::Backlog, "backlog"),
];

pub struct SlowSync {
    limit_ms: f32,
    started: Instant,
    last: Instant,
    steps: Vec<(&'static str, f32)>,
    /// Count and milliseconds of each kind of full sync.
    fulls: [(u32, f32); 4],
    moves: u32,
    /// The slowest full sync of the frame: milliseconds and object index.
    slowest: (f32, usize),
}

impl SlowSync {
    /// The limit, when AURORA_SYNC_SLOW asks for the line.
    pub fn limit_from_env() -> Option<f32> {
        let v = std::env::var("AURORA_SYNC_SLOW").ok()?;
        if v == "0" {
            return None;
        }
        Some(v.trim().parse().ok().filter(|&ms: &f32| ms > 0.0).unwrap_or(8.0))
    }

    pub fn new(limit_ms: f32, started: Instant) -> SlowSync {
        SlowSync {
            limit_ms,
            started,
            last: started,
            steps: Vec::with_capacity(16),
            fulls: [(0, 0.0); 4],
            moves: 0,
            slowest: (0.0, usize::MAX),
        }
    }

    /// The step that ends now.
    pub fn step(&mut self, name: &'static str) {
        let now = Instant::now();
        let ms = (now - self.last).as_secs_f32() * 1000.0;
        self.last = now;
        // (a step met twice, as the arrivals, adds up)
        match self.steps.iter_mut().find(|(n, _)| *n == name) {
            Some((_, sum)) => *sum += ms,
            None => self.steps.push((name, ms)),
        }
    }

    pub fn moved(&mut self) {
        self.moves += 1;
    }

    /// A full sync of `idx` that started at `started`.
    pub fn full(&mut self, kind: Full, idx: usize, started: Instant) {
        let ms = started.elapsed().as_secs_f32() * 1000.0;
        let f = &mut self.fulls[kind as usize];
        f.0 += 1;
        f.1 += ms;
        if ms > self.slowest.0 {
            self.slowest = (ms, idx);
        }
    }

    /// `count` full syncs timed together from `started` (the new objects,
    /// served a few per clock reading).
    pub fn fulls(&mut self, kind: Full, count: usize, started: Instant) {
        let f = &mut self.fulls[kind as usize];
        f.0 += count as u32;
        f.1 += started.elapsed().as_secs_f32() * 1000.0;
    }

    /// The line, when the frame was slow; `name` names an object index.
    pub fn line(&self, name: impl Fn(usize) -> String) -> Option<String> {
        let total = self.started.elapsed().as_secs_f32() * 1000.0;
        if total < self.limit_ms {
            return None;
        }
        let mut out = format!("slow sync: {total:.2} ms |");
        for (n, ms) in &self.steps {
            out.push_str(&format!(" {n}={ms:.2}"));
        }
        out.push_str(&format!(" | moves={} full:", self.moves));
        for (kind, n) in FULLS {
            let (count, ms) = self.fulls[kind as usize];
            out.push_str(&format!(" {n}={count}/{ms:.2}ms"));
        }
        if self.slowest.1 != usize::MAX {
            out.push_str(&format!(" | slowest {:.2} ms: {}", self.slowest.0, name(self.slowest.1)));
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_fast_frame_gives_no_line() {
        let s = SlowSync::new(1000.0, Instant::now());
        assert!(s.line(|_| String::new()).is_none());
    }

    #[test]
    fn the_line_names_the_steps_and_the_slowest_object() {
        let start = Instant::now() - Duration::from_millis(20);
        let mut s = SlowSync::new(5.0, start);
        s.step("list");
        s.step("arrivals");
        s.step("arrivals");
        s.moved();
        s.full(Full::Avatar, 3, Instant::now() - Duration::from_millis(2));
        s.full(Full::Backlog, 7, Instant::now() - Duration::from_millis(4));
        let l = s.line(|i| format!("object {i}")).expect("slow");
        assert!(l.starts_with("slow sync: "), "{l}");
        assert!(l.contains(" list="), "{l}");
        assert_eq!(l.matches("arrivals=").count(), 2, "{l}"); // the step and the count
        assert!(l.contains("moves=1 full: avatars=1/"), "{l}");
        assert!(l.contains(" backlog=1/"), "{l}");
        assert!(l.contains("ms: object 7"), "{l}");
    }
}
