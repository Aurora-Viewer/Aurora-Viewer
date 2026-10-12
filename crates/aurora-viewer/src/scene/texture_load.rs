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
    /// Pick mask and copy into the renderer's staging memory.
    pub stage_us: u64,
    pub fetched_bytes: u64,
    pub replies: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
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
            stage_us: self.stage_us - start.stage_us,
            fetched_bytes: self.fetched_bytes - start.fetched_bytes,
            replies: self.replies - start.replies,
            cache_hits: self.cache_hits - start.cache_hits,
            cache_misses: self.cache_misses - start.cache_misses,
        }
    }

    /// The log line of an episode that took `took` and left `loaded` of
    /// `total` textures loaded.
    pub fn describe(&self, took: Duration, (loaded, total): (usize, usize)) -> String {
        let s = |us: u64| us as f64 / 1e6;
        let cpu = self.codec_us + self.convert_us + self.rgba_us + self.mips_us + self.stage_us;
        format!(
            "texture load: {:.1} s, {loaded}/{total} textures loaded; {} decodes ({} of a texture already shown; discard 0..5: {:?}), \
             {:.1} Mpx; decode CPU {:.1} s (codec {:.1}, convert {:.1}, rgba {:.1}, mips {:.1}, staging {:.1}); \
             downloaded {:.1} MB in {} replies; disk cache {} hits / {} misses",
            took.as_secs_f64(),
            self.decodes(),
            self.again,
            self.by_discard,
            self.pixels as f64 / 1e6,
            s(cpu),
            s(self.codec_us),
            s(self.convert_us),
            s(self.rgba_us),
            s(self.mips_us),
            s(self.stage_us),
            self.fetched_bytes as f64 / 1e6,
            self.replies,
            self.cache_hits,
            self.cache_misses,
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
    stage_us: AtomicU64,
}

impl DecodeMeter {
    /// One finished decode job: the level it decoded and the time of each
    /// stage (`rgba`, `mips`, `stage` follow the two stages of the decoder).
    pub fn add(&self, discard: u8, pixels: u64, decode: DecodeTimings, [rgba, mips, stage]: [Duration; 3]) {
        let us = |d: Duration| d.as_micros() as u64;
        self.by_discard[usize::from(discard.min(5))].fetch_add(1, Ordering::Relaxed);
        self.pixels.fetch_add(pixels, Ordering::Relaxed);
        self.codec_us.fetch_add(us(decode.codec), Ordering::Relaxed);
        self.convert_us.fetch_add(us(decode.convert), Ordering::Relaxed);
        self.rgba_us.fetch_add(us(rgba), Ordering::Relaxed);
        self.mips_us.fetch_add(us(mips), Ordering::Relaxed);
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
        totals.stage_us = self.stage_us.load(Ordering::Relaxed);
    }
}

/// Follows the loading episodes, frame after frame.
#[derive(Default)]
pub struct LoadWatch {
    started: Option<(Instant, LoadTotals)>,
    idle_since: Option<Instant>,
}

impl LoadWatch {
    /// `busy`: something is being read, fetched, decoded or waits for the
    /// GPU. Returns a finished episode worth a log line: its duration (up
    /// to the last busy frame) and what it added to the totals. `totals` is
    /// only read when an episode starts or ends.
    pub fn step(&mut self, now: Instant, busy: bool, totals: impl FnOnce() -> LoadTotals) -> Option<(Duration, LoadTotals)> {
        if busy {
            self.idle_since = None;
            if self.started.is_none() {
                self.started = Some((now, totals()));
            }
            return None;
        }
        self.started.as_ref()?;
        let idle = *self.idle_since.get_or_insert(now);
        if now.duration_since(idle) < IDLE_ENDS {
            return None;
        }
        self.idle_since = None;
        let (start, at_start) = self.started.take()?;
        let took = idle.duration_since(start);
        let added = totals().since(&at_start);
        (added.decodes() >= WORTH_DECODES || took >= WORTH_TIME).then_some((took, added))
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

    #[test]
    fn episode_runs_from_the_first_busy_frame_to_the_last() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut w = LoadWatch::default();
        assert_eq!(w.step(at(0), false, || totals(0)), None);
        assert_eq!(w.step(at(100), true, || totals(50)), None);
        assert_eq!(w.step(at(7_000), true, || unreachable!("read at the ends only")), None);
        // idle, but not for long enough yet
        assert_eq!(w.step(at(7_100), false, || unreachable!()), None);
        assert_eq!(w.step(at(9_000), false, || unreachable!()), None);
        let (took, added) = w.step(at(9_100), false, || totals(1050)).expect("episode");
        assert_eq!(took, Duration::from_millis(7_000));
        assert_eq!(added.decodes(), 1000);
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
        let (took, _) = w.step(at(8_000), false, || totals(10)).expect("episode");
        assert_eq!(took, Duration::from_millis(6_000));
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
        assert!(w.step(at(5_400), false, || totals(12 + WORTH_DECODES)).is_some());
    }

    #[test]
    fn meter_and_line() {
        let m = DecodeMeter::default();
        let t = DecodeTimings {
            codec: Duration::from_millis(40),
            convert: Duration::from_millis(3),
        };
        let ms = Duration::from_millis;
        m.add(0, 1 << 20, t, [ms(2), ms(4), ms(1)]);
        m.add(9, 1 << 10, t, [ms(2), ms(4), ms(1)]);
        let mut totals = LoadTotals {
            again: 1,
            fetched_bytes: 2_500_000,
            replies: 3,
            cache_hits: 1,
            cache_misses: 1,
            ..Default::default()
        };
        m.read_into(&mut totals);
        assert_eq!(totals.by_discard, [1, 0, 0, 0, 0, 1]);
        assert_eq!(totals.codec_us, 80_000);
        assert_eq!(
            totals.describe(Duration::from_millis(12_340), (2, 3)),
            "texture load: 12.3 s, 2/3 textures loaded; 2 decodes (1 of a texture already shown; discard 0..5: [1, 0, 0, 0, 0, 1]), \
             1.0 Mpx; decode CPU 0.1 s (codec 0.1, convert 0.0, rgba 0.0, mips 0.0, staging 0.0); \
             downloaded 2.5 MB in 3 replies; disk cache 1 hits / 1 misses"
        );
    }
}
