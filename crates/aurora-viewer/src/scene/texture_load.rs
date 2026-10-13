//! Measure of texture loading (TASKS.md, leaving JPEG2000 after the
//! download): what the decode jobs cost, stage by stage, and one log line
//! per loading episode — from the first disk read, fetch or decode until
//! nothing is left to do — with its duration, its decodes, the bytes
//! downloaded and the disk cache hits. That line is the "time until
//! everything is loaded" compared between builds, and with Firestorm at the
//! same place.

use aurora_assets::DecodeTimings;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// An episode ends once nothing has been fetched, read, decoded or waiting
/// for the GPU for this long (a retry delay or the next wave of objects
/// does not cut it in two).
const IDLE_ENDS: Duration = Duration::from_secs(2);
/// Episodes below both are not worth a line: a few textures appearing as
/// the camera turns.
const WORTH_DECODES: u64 = 200;
const WORTH_TIME: Duration = Duration::from_secs(5);
/// A progress line this often while an episode runs.
const PROGRESS_EVERY: Duration = Duration::from_secs(10);

/// Totals since the start. The decode fields come from the jobs
/// ([`DecodeMeter`]); the others are counted by the main thread.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LoadTotals {
    /// Decodes by discard level actually applied (0 = full resolution).
    pub by_discard: [u64; 6],
    /// Decodes of a texture that was already shown at a coarser level.
    pub again: u64,
    /// Pixels of the decoded levels (before the mips).
    pub pixels: u64,
    pub codec_us: u64,
    pub convert_us: u64,
    /// Expansion to RGBA and alpha classification.
    pub rgba_us: u64,
    pub mips_us: u64,
    /// BC7 encoding and its quality check.
    pub bc7_us: u64,
    /// Decodes whose texture went to the GPU as BC7 blocks.
    pub compressed: u64,
    /// Pick mask and copy into the renderer's staging memory.
    pub stage_us: u64,
    pub fetched_bytes: u64,
    pub replies: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
    /// Textures read back from the transcoded cache: sent to the GPU
    /// without a decode.
    pub transcoded: u64,
    /// Downloaded textures that showed their first pixels, and the time
    /// each waited for them since its first request, in milliseconds.
    pub first_shown: u64,
    pub first_wait_ms: u64,
    /// First requests sent before the size on screen was known: at the
    /// same rank as every other, and for the smallest range.
    pub first_blind: u64,
    /// The same weighted by the area each covers on screen, in pixels: how
    /// long a pixel of a new texture stays without its picture.
    pub first_area: u64,
    pub first_area_wait: u64,
}

impl LoadTotals {
    pub fn decodes(&self) -> u64 {
        self.by_discard.iter().sum()
    }

    fn since(&self, start: &LoadTotals) -> LoadTotals {
        let mut by_discard = self.by_discard;
        for (d, s) in by_discard.iter_mut().zip(start.by_discard) {
            *d -= s;
        }
        LoadTotals {
            by_discard,
            again: self.again - start.again,
            pixels: self.pixels - start.pixels,
            codec_us: self.codec_us - start.codec_us,
            convert_us: self.convert_us - start.convert_us,
            rgba_us: self.rgba_us - start.rgba_us,
            mips_us: self.mips_us - start.mips_us,
            bc7_us: self.bc7_us - start.bc7_us,
            compressed: self.compressed - start.compressed,
            stage_us: self.stage_us - start.stage_us,
            fetched_bytes: self.fetched_bytes - start.fetched_bytes,
            replies: self.replies - start.replies,
            cache_hits: self.cache_hits - start.cache_hits,
            cache_misses: self.cache_misses - start.cache_misses,
            transcoded: self.transcoded - start.transcoded,
            first_shown: self.first_shown - start.first_shown,
            first_wait_ms: self.first_wait_ms - start.first_wait_ms,
            first_blind: self.first_blind - start.first_blind,
            first_area: self.first_area - start.first_area,
            first_area_wait: self.first_area_wait - start.first_area_wait,
        }
    }

    /// A downloaded texture covering `px` pixels on screen showed its first
    /// pixels `wait` after its first request.
    pub fn first_pixels(&mut self, wait: Duration, px: f32) {
        let ms = wait.as_millis() as u64;
        let side = px.clamp(16.0, 2048.0) as u64;
        self.first_shown += 1;
        self.first_wait_ms += ms;
        self.first_area += side * side;
        self.first_area_wait += side * side * ms;
    }

    /// The log line of an episode (`running`: still under way) that took
    /// `took` and left `loaded` of `total` textures loaded.
    pub fn describe(&self, took: Duration, running: bool, (loaded, total): (usize, usize)) -> String {
        let s = |us: u64| us as f64 / 1e6;
        let cpu = self.codec_us + self.convert_us + self.rgba_us + self.mips_us + self.bc7_us + self.stage_us;
        format!(
            "texture load{}: {:.1} s, {loaded}/{total} textures loaded; {} decodes ({} of a texture already shown, {} compressed to BC7; discard 0..5: {:?}), \
             {:.1} Mpx; decode CPU {:.1} s (codec {:.1}, convert {:.1}, rgba {:.1}, mips {:.1}, bc7 {:.1}, staging {:.1}); \
             downloaded {:.1} MB in {} replies; disk cache {} hits / {} misses, {} read as BC7 without a decode; \
             first pixels of {} downloaded textures {:.2} s after their request on average, {:.2} s weighted by their size on screen \
             ({} asked before that size was known)",
            if running { " (running)" } else { "" },
            took.as_secs_f64(),
            self.decodes(),
            self.again,
            self.compressed,
            self.by_discard,
            self.pixels as f64 / 1e6,
            s(cpu),
            s(self.codec_us),
            s(self.convert_us),
            s(self.rgba_us),
            s(self.mips_us),
            s(self.bc7_us),
            s(self.stage_us),
            self.fetched_bytes as f64 / 1e6,
            self.replies,
            self.cache_hits,
            self.cache_misses,
            self.transcoded,
            self.first_shown,
            self.first_wait_ms as f64 / 1e3 / self.first_shown.max(1) as f64,
            self.first_area_wait as f64 / 1e3 / self.first_area.max(1) as f64,
            self.first_blind,
        )
    }
}

/// What the decode jobs spent, summed since the start; shared with them.
#[derive(Default)]
pub struct DecodeMeter {
    by_discard: [AtomicU64; 6],
    pixels: AtomicU64,
    codec_us: AtomicU64,
    convert_us: AtomicU64,
    rgba_us: AtomicU64,
    mips_us: AtomicU64,
    bc7_us: AtomicU64,
    compressed: AtomicU64,
    stage_us: AtomicU64,
}

impl DecodeMeter {
    /// One finished decode job: the level it decoded, the time of each
    /// stage (`rgba`, `mips`, `bc7`, `stage` follow the two stages of the
    /// decoder) and whether the texture leaves as BC7 blocks.
    pub fn add(&self, discard: u8, pixels: u64, decode: DecodeTimings, [rgba, mips, bc7, stage]: [Duration; 4], compressed: bool) {
        let us = |d: Duration| d.as_micros() as u64;
        self.by_discard[usize::from(discard.min(5))].fetch_add(1, Ordering::Relaxed);
        self.pixels.fetch_add(pixels, Ordering::Relaxed);
        self.codec_us.fetch_add(us(decode.codec), Ordering::Relaxed);
        self.convert_us.fetch_add(us(decode.convert), Ordering::Relaxed);
        self.rgba_us.fetch_add(us(rgba), Ordering::Relaxed);
        self.mips_us.fetch_add(us(mips), Ordering::Relaxed);
        self.bc7_us.fetch_add(us(bc7), Ordering::Relaxed);
        self.compressed.fetch_add(u64::from(compressed), Ordering::Relaxed);
        self.stage_us.fetch_add(us(stage), Ordering::Relaxed);
    }

    /// The jobs' part of `totals`.
    pub fn read_into(&self, totals: &mut LoadTotals) {
        for (t, a) in totals.by_discard.iter_mut().zip(&self.by_discard) {
            *t = a.load(Ordering::Relaxed);
        }
        totals.pixels = self.pixels.load(Ordering::Relaxed);
        totals.codec_us = self.codec_us.load(Ordering::Relaxed);
        totals.convert_us = self.convert_us.load(Ordering::Relaxed);
        totals.rgba_us = self.rgba_us.load(Ordering::Relaxed);
        totals.mips_us = self.mips_us.load(Ordering::Relaxed);
        totals.bc7_us = self.bc7_us.load(Ordering::Relaxed);
        totals.compressed = self.compressed.load(Ordering::Relaxed);
        totals.stage_us = self.stage_us.load(Ordering::Relaxed);
    }
}

/// What [`LoadWatch`] has to say about the episode under way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadReport {
    /// From the first busy frame to the first idle one (to now while it runs).
    pub took: Duration,
    /// What the episode added to the totals.
    pub added: LoadTotals,
    /// The episode is over; else this is a progress line.
    pub done: bool,
}

/// Follows the loading episodes, frame after frame.
#[derive(Default)]
pub struct LoadWatch {
    started: Option<(Instant, LoadTotals)>,
    idle_since: Option<Instant>,
    /// Last progress line of the episode under way (its start before any).
    reported: Option<Instant>,
}

impl LoadWatch {
    /// `busy`: something is being read, fetched, decoded or waits for the
    /// GPU. Returns the line to log, if any: the progress of the episode
    /// under way every [`PROGRESS_EVERY`] (a busy region never comes to
    /// rest: a few downloads are always in flight), or a finished episode
    /// worth a line. `totals` is only read for those and when an episode
    /// starts.
    pub fn step(&mut self, now: Instant, busy: bool, totals: impl FnOnce() -> LoadTotals) -> Option<LoadReport> {
        if busy {
            self.idle_since = None;
            let Some((start, at_start)) = &self.started else {
                self.started = Some((now, totals()));
                self.reported = Some(now);
                return None;
            };
            if self.reported.is_some_and(|t| now.duration_since(t) < PROGRESS_EVERY) {
                return None;
            }
            self.reported = Some(now);
            return Some(LoadReport {
                took: now.duration_since(*start),
                added: totals().since(at_start),
                done: false,
            });
        }
        self.started.as_ref()?;
        let idle = *self.idle_since.get_or_insert(now);
        if now.duration_since(idle) < IDLE_ENDS {
            return None;
        }
        self.end(idle, totals)
    }

    /// Ends the episode under way at `now`, if there is one: the arrival in
    /// another region starts a new one, with its own counts.
    pub fn cut(&mut self, now: Instant, totals: impl FnOnce() -> LoadTotals) -> Option<LoadReport> {
        let last_busy = self.idle_since.unwrap_or(now);
        self.end(last_busy, totals)
    }

    fn end(&mut self, last_busy: Instant, totals: impl FnOnce() -> LoadTotals) -> Option<LoadReport> {
        self.idle_since = None;
        self.reported = None;
        let (start, at_start) = self.started.take()?;
        let took = last_busy.duration_since(start);
        let added = totals().since(&at_start);
        (added.decodes() >= WORTH_DECODES || took >= WORTH_TIME).then_some(LoadReport { took, added, done: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn totals(decodes: u64) -> LoadTotals {
        LoadTotals {
            by_discard: [decodes, 0, 0, 0, 0, 0],
            ..Default::default()
        }
    }

    fn done(took_ms: u64, decodes: u64) -> Option<LoadReport> {
        Some(LoadReport {
            took: Duration::from_millis(took_ms),
            added: totals(decodes),
            done: true,
        })
    }

    #[test]
    fn episode_runs_from_the_first_busy_frame_to_the_last() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut w = LoadWatch::default();
        assert_eq!(w.step(at(0), false, || totals(0)), None);
        assert_eq!(w.step(at(100), true, || totals(50)), None);
        assert_eq!(w.step(at(7_000), true, || unreachable!("read for a line only")), None);
        // idle, but not for long enough yet
        assert_eq!(w.step(at(7_100), false, || unreachable!()), None);
        assert_eq!(w.step(at(9_000), false, || unreachable!()), None);
        assert_eq!(w.step(at(9_100), false, || totals(1050)), done(7_000, 1000));
        // over: nothing more until the next busy frame
        assert_eq!(w.step(at(20_000), false, || unreachable!()), None);
    }

    #[test]
    fn a_short_pause_does_not_cut_an_episode() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut w = LoadWatch::default();
        w.step(at(0), true, || totals(0));
        assert_eq!(w.step(at(1_000), false, || unreachable!()), None);
        assert_eq!(w.step(at(2_500), true, || unreachable!()), None);
        assert_eq!(w.step(at(6_000), false, || unreachable!()), None);
        assert_eq!(w.step(at(8_000), false, || totals(10)), done(6_000, 10));
    }

    #[test]
    fn small_episodes_are_not_reported() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut w = LoadWatch::default();
        w.step(at(0), true, || totals(0));
        w.step(at(500), false, || unreachable!());
        assert_eq!(w.step(at(2_500), false, || totals(12)), None);
        // many decodes in a short time are
        w.step(at(3_000), true, || totals(12));
        w.step(at(3_400), false, || unreachable!());
        assert_eq!(w.step(at(5_400), false, || totals(12 + WORTH_DECODES)), done(400, WORTH_DECODES));
    }

    #[test]
    fn a_region_that_never_rests_gets_progress_lines() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let running = |took_ms: u64, decodes: u64| {
            Some(LoadReport {
                took: Duration::from_millis(took_ms),
                added: totals(decodes),
                done: false,
            })
        };
        let mut w = LoadWatch::default();
        w.step(at(0), true, || totals(100));
        assert_eq!(w.step(at(9_900), true, || unreachable!()), None);
        assert_eq!(w.step(at(10_000), true, || totals(600)), running(10_000, 500));
        assert_eq!(w.step(at(19_000), true, || unreachable!()), None);
        assert_eq!(w.step(at(20_500), true, || totals(900)), running(20_500, 800));
        // the next episode counts its own ten seconds
        assert_eq!(w.step(at(23_000), false, || totals(900)), None);
        assert_eq!(w.step(at(25_000), false, || totals(900)), done(23_000, 800));
        w.step(at(26_000), true, || totals(900));
        assert_eq!(w.step(at(31_000), true, || unreachable!()), None);
    }

    #[test]
    fn an_arrival_ends_the_episode_under_way() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut w = LoadWatch::default();
        assert_eq!(w.cut(at(0), || unreachable!()), None);
        w.step(at(0), true, || totals(0));
        w.step(at(4_000), true, || unreachable!());
        assert_eq!(w.cut(at(6_000), || totals(700)), done(6_000, 700));
        // idle when the cut comes: up to the first idle frame
        w.step(at(7_000), true, || totals(700));
        w.step(at(13_000), false, || unreachable!());
        assert_eq!(w.cut(at(14_000), || totals(705)), done(6_000, 5));
        assert_eq!(w.cut(at(15_000), || unreachable!()), None);
    }

    #[test]
    fn meter_and_line() {
        let m = DecodeMeter::default();
        let t = DecodeTimings {
            codec: Duration::from_millis(40),
            convert: Duration::from_millis(3),
        };
        let ms = Duration::from_millis;
        m.add(0, 1 << 20, t, [ms(2), ms(4), ms(30), ms(1)], true);
        m.add(9, 1 << 10, t, [ms(2), ms(4), ms(0), ms(1)], false);
        let mut totals = LoadTotals {
            again: 1,
            fetched_bytes: 2_500_000,
            replies: 3,
            cache_hits: 1,
            cache_misses: 1,
            transcoded: 7,
            ..Default::default()
        };
        m.read_into(&mut totals);
        // a small texture shown at once, a large one after two round trips
        totals.first_pixels(Duration::from_millis(200), 100.0);
        totals.first_pixels(Duration::from_millis(600), 300.0);
        assert_eq!(totals.by_discard, [1, 0, 0, 0, 0, 1]);
        assert_eq!(totals.codec_us, 80_000);
        assert_eq!(
            totals.describe(Duration::from_millis(12_340), false, (2, 3)),
            "texture load: 12.3 s, 2/3 textures loaded; 2 decodes (1 of a texture already shown, 1 compressed to BC7; discard 0..5: [1, 0, 0, 0, 0, 1]), \
             1.0 Mpx; decode CPU 0.1 s (codec 0.1, convert 0.0, rgba 0.0, mips 0.0, bc7 0.0, staging 0.0); \
             downloaded 2.5 MB in 3 replies; disk cache 1 hits / 1 misses, 7 read as BC7 without a decode; \
             first pixels of 2 downloaded textures 0.40 s after their request on average, 0.56 s weighted by their size on screen \
             (0 asked before that size was known)"
        );
        assert!(
            totals
                .describe(Duration::ZERO, true, (0, 0))
                .starts_with("texture load (running): 0.0 s, ")
        );
    }
}
