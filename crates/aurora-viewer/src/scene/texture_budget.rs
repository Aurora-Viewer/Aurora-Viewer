//! Texture memory budget: how much video memory the streamed textures may
//! hold now, and how hard to press on them when they hold more.
//!
//! What Firestorm does (LLViewerTexture::updateClass and
//! LLViewerTextureList::updateImageDecodePriority in
//! indra/newview/llviewertexture.cpp and llviewertexturelist.cpp,
//! LLImageGL::scaleDown in indra/llrender/llimagegl.cpp, originally LGPL
//! 2.1), kept here:
//! - without memory pressure nothing loaded is lowered (its virtual size is a
//!   running maximum);
//! - the target is the video memory budget less 512 MB, at most 80 % of it,
//!   never under 768 MB; the pressure rises only after a second above it and
//!   falls only under 90 % of it;
//! - under pressure textures come down to the size they have on screen now,
//!   then lose up to two more levels, those out of view first;
//! - a loaded texture is lowered on the GPU from the levels it already
//!   holds, a few per frame, never decoded again; avatar bakes, HUDs, the
//!   interface, sculpt maps and the terrain are left alone.
//!
//! What differs, on purpose:
//! - the budget is read from the driver as it is now and the use is what
//!   the driver counts for the viewer (aurora_render::vram), where the
//!   Legacy Viewer doubles the texture and vertex bytes it tracks as an
//!   estimate against the memory size read at startup;
//! - steps of half a level instead of a bias that drifts with the frame
//!   time, and a step is only taken back once the memory the textures would
//!   then hold fits: Firestorm lowers its bias as soon as memory is free,
//!   the textures load again and the memory fills again, in a loop;
//! - textures nothing uses any more are evicted after a few seconds under
//!   pressure instead of waiting their full time;
//! - a step lowers only what brings the pages back under 85 % of the target
//!   ([`lowering_goal`]), the largest textures first, instead of every
//!   texture finer than its size on screen. On the grid, a busy region
//!   13 MB over a 4.2 GB target lost 2.6 GB of textures at step 1, was back
//!   at step 0 five seconds later, and loaded them again as the camera
//!   moved, every few minutes.

use aurora_render::vram::VideoMemory;
use std::time::{Duration, Instant};

const MB: u64 = 1 << 20;

/// Highest pressure step.
pub const MAX_PRESSURE: u8 = 7;

/// Half levels taken from the textures in view and from those out of view
/// at each pressure step, below the size they have on screen. Step 1 only
/// brings every texture down to that size; then those out of view lose a
/// level before those in view lose any, up to two levels each. Firestorm's
/// bias divides the virtual size by up to 16 (two levels), the faces of
/// little importance to the camera from a bias of 1.9 and the others from
/// 2.4.
///
/// Half levels, because a whole one is a lot at once: half the textures
/// change level at each step, not all of them. With whole levels, on the
/// grid, a shop that held 1,037 MB for a target of 1,024 lost a second
/// level in view to end at 812 MB.
const DROPPED: [(u8, u8); MAX_PRESSURE as usize + 1] = [(0, 0), (0, 0), (0, 1), (0, 2), (1, 3), (2, 4), (3, 4), (4, 4)];

/// A step up needs this long above the target (sEvaluationTimer), a step
/// down this long since the last change (what the bias takes to fall by
/// half a level at RenderHighMemMinDiscardDecrement, 0.1 a second).
const RAISE_AFTER: Duration = Duration::from_secs(1);
const LOWER_AFTER: Duration = Duration::from_secs(5);

/// A step up without waiting for the last one to be done, still above the
/// target this long after it. Textures keep arriving in a busy place and
/// the camera keeps moving: each pass lists new reductions, so the step
/// was never done and the pressure stayed at 1, the video memory past the
/// driver's budget until the driver reset (#1561; in demo, a 6,000-texture
/// arrival held step 1 for 14 s at 2.4 GB for a 2 GB budget). The Legacy
/// Viewer raises its bias all the while it is above the target, done or
/// not (LLViewerTexture::updateClass).
const RAISE_UNSETTLED_AFTER: Duration = Duration::from_secs(3);

/// Under pressure, textures nothing uses are evicted after this long.
pub const PRESSURE_EVICT: Duration = Duration::from_secs(5);

/// The textures keep at least this much, whatever else fills the memory.
const MIN_TEXTURE_BYTES: u64 = 256 * MB;

/// What divides the size on screen of the textures at a pressure step: (in
/// view, out of view), a factor of √2 per half level dropped.
pub fn dropped(pressure: u8) -> (f32, f32) {
    let (in_view, away) = DROPPED[pressure.min(MAX_PRESSURE) as usize];
    let divisor = |half_levels: u8| std::f32::consts::SQRT_2.powi(half_levels as i32);
    (divisor(in_view), divisor(away))
}

/// Bytes of the textures to lower under pressure: what brings the pages
/// down to 85 % of the target, under the 90 % a step down waits for, told in
/// bytes of textures (`live`) rather than of pages, which hold free room on
/// top that compaction gives back.
pub fn lowering_goal(used: u64, live: u64, target: u64) -> u64 {
    let goal = target / 100 * 85;
    (used.saturating_sub(goal) as f64 * live as f64 / used.max(1) as f64) as u64
}

/// Bytes the pages would hold one step below ([`Reading::below`]): the
/// textures' bytes there (`bytes_below`, each between the level it would be
/// lowered to and the one it would load) with the free room the pages have
/// now. From step 1 too: its lowering takes the largest textures under the
/// level step 0 loads, so they load again there. Counted as nothing taken
/// back, as it was, the step down came as soon as the pages were under
/// 90 % of the target, and the pressure went 0 -> 1 -> 0 every 5 to 20 s on
/// the grid with a steady budget, 45 to 900 textures lowered each time.
pub fn below_estimate(bytes_below: u64, used: u64, live: u64) -> u64 {
    (bytes_below as f64 * (used as f64 / live.max(1) as f64)) as u64
}

/// AURORA_TEXTURE_BUDGET_MB: the texture memory target forced to this many
/// megabytes, whatever the setting and the driver say (tests of the budget
/// on any machine).
pub fn forced_target() -> Option<u64> {
    static FORCED: std::sync::OnceLock<Option<u64>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| {
        let mb: u64 = std::env::var("AURORA_TEXTURE_BUDGET_MB").ok()?.trim().parse().ok()?;
        Some(mb.clamp(1, 1 << 20) * MB)
    })
}

/// Bytes the texture pages may hold now: the setting (manual, or the share
/// of the video memory found at startup), less when the rest of what the
/// process holds on the card leaves less under the target of the whole
/// process. The rest is what the driver counts for the process besides the
/// pages: render targets and geometry, and the free room of wgpu's memory
/// blocks, which the driver counts too (see aurora_render::vram).
///
/// Counted from the live resources only, as it was, the viewer could stay
/// past the driver's budget with its pages under their target, the driver
/// counting more than a gigabyte of free room in blocks. That room was
/// mostly left by freed pages, and counting it then would have fed on
/// itself (pages lowered, more room left, a lower target); the pages now
/// hold memory of their own, given back to the driver as they shrink
/// (aurora_render's dedicated.rs), and the room left belongs to the other
/// resources.
pub fn texture_target(setting: u64, pages: u64, video: Option<VideoMemory>) -> u64 {
    let Some(v) = video else {
        return setting;
    };
    // target = max(min(budget - 512, budget * 0.8), MIN_VRAM_BUDGET)
    let process = v.budget.saturating_sub(512 * MB).min(v.budget / 5 * 4).max(768 * MB);
    let others = v.usage.max(v.allocated).saturating_sub(pages);
    setting.min(process.saturating_sub(others)).max(MIN_TEXTURE_BYTES)
}

/// What the pressure is decided on, a few times a second.
#[derive(Clone, Copy, Debug)]
pub struct Reading {
    /// Bytes of the texture pages, and what they may hold.
    pub used: u64,
    pub target: u64,
    /// Everything the current step asks for is done: no reduction waits and
    /// the pages have given back what they could.
    pub settled: bool,
    /// Bytes the pages would hold one step below (the textures load again
    /// what that step gives back).
    pub below: u64,
}

/// The pressure step, see the module.
#[derive(Debug)]
pub struct Pressure {
    step: u8,
    over_since: Option<Instant>,
    changed: Instant,
}

impl Pressure {
    pub fn new(now: Instant) -> Self {
        Pressure {
            step: 0,
            over_since: None,
            changed: now,
        }
    }

    pub fn step(&self) -> u8 {
        self.step
    }

    /// The step for this reading. Up: at once from none (textures only come
    /// down to their size on screen), then one step each time the memory is
    /// still above the target a second after the step was fully applied,
    /// or [`RAISE_UNSETTLED_AFTER`] after it if it never is. Down: under
    /// 90 % of the target for a while, and only if the textures would still
    /// fit there one step below.
    pub fn update(&mut self, now: Instant, r: Reading) -> u8 {
        let free = r.target / 10 * 9;
        if r.used > r.target {
            let since = *self.over_since.get_or_insert(now);
            let at_step = now.duration_since(self.changed);
            let waited = now.duration_since(since) >= RAISE_AFTER && at_step >= RAISE_AFTER;
            let done = r.settled || at_step >= RAISE_UNSETTLED_AFTER;
            if self.step == 0 || (self.step < MAX_PRESSURE && done && waited) {
                self.step += 1;
                self.changed = now;
            }
        } else {
            self.over_since = None;
            if self.step > 0 && r.used < free && r.below < free && now.duration_since(self.changed) >= LOWER_AFTER {
                self.step -= 1;
                self.changed = now;
            }
        }
        self.step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1 << 30;

    /// The driver's budget and the live resources of the process, which the
    /// driver counts as they are.
    fn video(budget: u64, allocated: u64) -> Option<VideoMemory> {
        Some(VideoMemory {
            budget,
            usage: allocated,
            allocated,
        })
    }

    #[test]
    fn target_is_the_setting_while_the_card_has_room() {
        // 24 GB card, 16.8 GB for textures, 1 GB of render targets
        assert_eq!(texture_target(16 * GB, 4 * GB, video(23 * GB, 5 * GB)), 16 * GB);
        // no figures from the driver: the setting alone
        assert_eq!(texture_target(3 * GB, 9 * GB, None), 3 * GB);
    }

    #[test]
    fn target_leaves_room_for_the_rest_of_the_process() {
        // 8 GB card: the process may hold 80 % of the 7.5 GB it is given;
        // with 1.5 GB of render targets and geometry, textures get the rest
        let t = texture_target(6 * GB, 4 * GB, video(7 * GB + GB / 2, 5 * GB + GB / 2));
        assert_eq!(t, 6 * GB - GB / 2 - GB);
        // another program takes the card: the budget given to us falls,
        // the textures follow
        let t = texture_target(6 * GB, 4 * GB, video(4 * GB, 5 * GB + GB / 2));
        assert_eq!(t, (4 * GB / 5 * 4) - (GB + GB / 2));
        // small budgets: 512 MB are left free rather than 20 %, never
        // under 768 MB for the process nor 256 MB for the textures
        assert_eq!(texture_target(6 * GB, 0, video(2 * GB, 0)), 2 * GB - 512 * MB);
        assert_eq!(texture_target(6 * GB, 100 * MB, video(GB, 900 * MB)), 256 * MB);
    }

    #[test]
    fn lowering_gives_back_only_what_is_over_the_goal() {
        // the grid case: 13 MB over a 4,167 MB target, pages 5 % larger
        // than the textures in them; 85 % of the target is 3,541 MB
        let lower = lowering_goal(4180 * MB, 3971 * MB, 4167 * MB);
        assert!(lower > 600 * MB && lower < 620 * MB, "{}", lower / MB);
        // under the goal: nothing
        assert_eq!(lowering_goal(3000 * MB, 2900 * MB, 4167 * MB), 0);
        assert_eq!(lowering_goal(0, 0, 4167 * MB), 0);
    }

    #[test]
    fn room_the_driver_counts_beyond_the_resources_leaves_less() {
        // 8 GB card: 3 GB of pages, 1.5 GB of render targets and geometry,
        // and 1 GB of free room in wgpu's blocks the driver counts too
        let v = |usage| {
            Some(VideoMemory {
                budget: 7 * GB + GB / 2,
                usage,
                allocated: 4 * GB + GB / 2,
            })
        };
        assert_eq!(texture_target(6 * GB, 3 * GB, v(5 * GB + GB / 2)), 6 * GB - GB / 2 - GB - GB);
        // the driver behind the counters (a buffer just created): the live
        // resources at least
        assert_eq!(texture_target(6 * GB, 3 * GB, v(4 * GB)), 6 * GB - GB / 2 - GB);
    }

    #[test]
    fn steps_drop_levels_out_of_view_first() {
        let close = |(a, b): (f32, f32), (c, d): (f32, f32)| (a - c).abs() < 1e-4 && (b - d).abs() < 1e-4;
        let r2 = std::f32::consts::SQRT_2;
        assert!(close(dropped(0), (1.0, 1.0)));
        // down to the size on screen, nothing more
        assert!(close(dropped(1), (1.0, 1.0)));
        // out of view: half a level, a level, before anything in view
        assert!(close(dropped(2), (1.0, r2)));
        assert!(close(dropped(3), (1.0, 2.0)));
        assert!(close(dropped(4), (r2, 2.0 * r2)));
        // two levels at most, as Firestorm's bias
        assert!(close(dropped(MAX_PRESSURE), (4.0, 4.0)));
        assert!(close(dropped(99), dropped(MAX_PRESSURE)));
        // never more in view than out of view, never less than the step
        // before, half a level at a time
        for s in 1..=MAX_PRESSURE {
            let ((v, a), (pv, pa)) = (dropped(s), dropped(s - 1));
            assert!(v <= a + 1e-4 && v >= pv && a >= pa, "step {s}");
            assert!(v / pv < r2 + 1e-4 && a / pa < r2 + 1e-4, "step {s}");
        }
    }

    fn reading(used: u64, settled: bool, below: u64) -> Reading {
        Reading {
            used,
            target: 1000,
            settled,
            below,
        }
    }

    #[test]
    fn pressure_rises_while_reductions_never_end() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut p = Pressure::new(t0);
        assert_eq!(p.update(t0, reading(1200, false, 1200)), 1);
        // a busy place: new reductions at every pass, never settled
        assert_eq!(p.update(t0 + ms(2900), reading(1200, false, 1200)), 1);
        assert_eq!(p.update(t0 + ms(3000), reading(1200, false, 1200)), 2);
        assert_eq!(p.update(t0 + ms(5900), reading(1200, false, 1200)), 2);
        assert_eq!(p.update(t0 + ms(6000), reading(1200, false, 1200)), 3);
        // settled again: back to a second
        assert_eq!(p.update(t0 + ms(7000), reading(1200, true, 1200)), 4);
    }

    #[test]
    fn pressure_starts_at_once_and_rises_a_step_at_a_time() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut p = Pressure::new(t0);
        assert_eq!(p.update(t0, reading(900, true, 900)), 0);
        // above the target: textures come down to their size on screen
        assert_eq!(p.update(t0 + s(1), reading(1200, true, 1200)), 1);
        // not again before a second, nor while reductions are still waiting
        assert_eq!(p.update(t0 + Duration::from_millis(1500), reading(1200, true, 1200)), 1);
        assert_eq!(p.update(t0 + s(3), reading(1200, false, 1200)), 1);
        assert_eq!(p.update(t0 + s(4), reading(1100, true, 1200)), 2);
        assert_eq!(p.update(t0 + Duration::from_millis(4500), reading(1100, true, 1200)), 2);
        assert_eq!(p.update(t0 + s(6), reading(1100, true, 1200)), 3);
        assert_eq!(p.update(t0 + s(8), reading(1100, true, 1200)), 4);
        // up to the last step, no further
        for i in 0..10 {
            p.update(t0 + s(10 + 2 * i), reading(1100, true, 1200));
        }
        assert_eq!(p.step(), MAX_PRESSURE);
    }

    #[test]
    fn a_dip_under_the_target_restarts_the_wait() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut p = Pressure::new(t0);
        assert_eq!(p.update(t0, reading(1200, true, 0)), 1);
        assert_eq!(p.update(t0 + ms(1900), reading(950, true, 2000)), 1);
        // above again: a full second from here
        assert_eq!(p.update(t0 + ms(2000), reading(1200, true, 0)), 1);
        assert_eq!(p.update(t0 + ms(2900), reading(1200, true, 0)), 1);
        assert_eq!(p.update(t0 + ms(3100), reading(1200, true, 0)), 2);
    }

    #[test]
    fn pressure_falls_only_when_the_step_below_fits() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut p = Pressure::new(t0);
        p.update(t0, reading(1200, true, 0));
        p.update(t0 + s(2), reading(1200, true, 0));
        assert_eq!(p.step(), 2);
        // between 90 % and the target: held
        assert_eq!(p.update(t0 + s(30), reading(950, true, 100)), 2);
        // free, but the textures would fill the memory again one step below
        assert_eq!(p.update(t0 + s(31), reading(600, true, 1300)), 2);
        assert_eq!(p.update(t0 + s(32), reading(600, true, 950)), 2);
        // they fit: one step, then the next after the same wait
        assert_eq!(p.update(t0 + s(33), reading(600, true, 800)), 1);
        assert_eq!(p.update(t0 + s(35), reading(800, true, 800)), 1);
        assert_eq!(p.update(t0 + s(38), reading(800, true, 800)), 0);
        assert_eq!(p.update(t0 + s(60), reading(800, true, 800)), 0);
    }

    #[test]
    fn step_one_waits_while_the_textures_would_load_back_over_the_target() {
        // run D on the grid, scaled to a target of 1000: step 1 brought the
        // pages to 869 (under the 900 a step down waits for), but the
        // textures lowered there want 1000 again at step 0
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut p = Pressure::new(t0);
        assert_eq!(p.update(t0, reading(1007, true, 1007)), 1);
        let below = below_estimate(1000, 869, 869);
        assert_eq!(below, 1000);
        for t in [6, 20, 60] {
            assert_eq!(p.update(t0 + s(t), reading(869, true, below)), 1, "at {t} s");
        }
        // the scene shrinks (the camera turned away): it fits one step down
        assert_eq!(p.update(t0 + s(70), reading(800, true, below_estimate(850, 800, 800))), 0);
        // the free room of the pages counts as it is now
        assert_eq!(below_estimate(850, 1000, 800), 1062);
    }
}
