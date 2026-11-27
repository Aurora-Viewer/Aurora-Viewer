//! Textures whose texels the renderer could not stage: video memory had no
//! staging buffer for their write (aurora-render's write_staging.rs), so the
//! slot kept what it held. Their owners send them again after a wait, which
//! doubles while the memory stays full, instead of the device being lost.

use super::App;
use aurora_render::Renderer;
use std::time::{Duration, Instant};

/// The first wait after a refusal: the moment of memory pressure is often
/// over (another program let go, the region's arrival settled).
const FIRST_WAIT: Duration = Duration::from_secs(2);
/// The longest wait while the refusals go on.
const LONGEST_WAIT: Duration = Duration::from_secs(30);
/// Without a refusal for this long, the next one waits `FIRST_WAIT` again.
const QUIET: Duration = Duration::from_secs(60);

/// The slots waiting to be sent again, and when.
pub(super) struct LostTextures {
    slots: Vec<u32>,
    due: Option<Instant>,
    wait: Duration,
    last_lost: Option<Instant>,
}

impl Default for LostTextures {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            due: None,
            wait: FIRST_WAIT,
            last_lost: None,
        }
    }
}

impl LostTextures {
    /// The slots lost this frame.
    fn note(&mut self, lost: Vec<u32>, now: Instant) {
        if lost.is_empty() {
            return;
        }
        if self.last_lost.is_some_and(|t| now.saturating_duration_since(t) > QUIET) {
            self.wait = FIRST_WAIT;
        }
        self.last_lost = Some(now);
        for slot in lost {
            if !self.slots.contains(&slot) {
                self.slots.push(slot);
            }
        }
        if self.due.is_none() {
            self.due = Some(now + self.wait);
            self.wait = (self.wait * 2).min(LONGEST_WAIT);
        }
    }

    /// The slots to send again now: none before their wait is over.
    fn take_due(&mut self, now: Instant) -> Vec<u32> {
        if self.due.is_some_and(|t| now >= t) {
            self.due = None;
            std::mem::take(&mut self.slots)
        } else {
            Vec::new()
        }
    }

    /// After a GPU recovery: the slots were the lost renderer's.
    pub(super) fn clear(&mut self) {
        *self = Self::default();
    }
}

impl App {
    /// Every frame: the slots the renderer could not stage are noted, and
    /// once their wait is over each owner sends its texture again (the
    /// streamer's textures, the scene's own, media, the Director's cards).
    pub(super) fn send_lost_textures(&mut self, renderer: &mut Renderer) {
        let now = Instant::now();
        self.lost_textures.note(renderer.take_lost_textures(), now);
        let due = self.lost_textures.take_due(now);
        if due.is_empty() {
            return;
        }
        let mut left = self.scene.send_lost_textures(&due, renderer);
        // (AURORA_DEMO_STREAM's checkers have no data the streamer could
        // decode again: the scenario decodes them)
        if let Some(demo) = &mut self.stream_demo {
            demo.textures_lost();
        }
        left.retain(|&slot| !(renderer.send_texture_again(slot) || self.media.texture_lost(slot) || self.director_card_lost(slot)));
        log::info!(
            "textures: {} sent again after video memory refused their staging buffer, {} no longer held{}",
            due.len() - left.len(),
            left.len(),
            if left.is_empty() {
                String::new()
            } else {
                format!(" (slots {:?})", &left[..left.len().min(8)])
            }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lost_textures_wait_longer_while_memory_stays_full() {
        let mut l = LostTextures::default();
        let t0 = Instant::now();
        l.note(vec![7, 8, 7], t0);
        // not before the first wait
        assert!(l.take_due(t0 + Duration::from_secs(1)).is_empty());
        assert_eq!(l.take_due(t0 + FIRST_WAIT), [7, 8]);
        assert!(l.take_due(t0 + FIRST_WAIT).is_empty(), "sent once");
        // refused again: twice the wait, then up to the longest
        let t1 = t0 + FIRST_WAIT;
        l.note(vec![8], t1);
        assert!(l.take_due(t1 + FIRST_WAIT).is_empty());
        assert_eq!(l.take_due(t1 + FIRST_WAIT * 2), [8]);
        let mut t = t1 + FIRST_WAIT * 2;
        for _ in 0..10 {
            l.note(vec![8], t);
            t += LONGEST_WAIT;
            assert_eq!(l.take_due(t), [8]);
        }
        // a slot lost while one waits goes with it
        l.note(vec![8], t);
        l.note(vec![9], t + Duration::from_secs(1));
        assert_eq!(l.take_due(t + LONGEST_WAIT), [8, 9]);
        // after a quiet minute, the first wait again
        let later = t + LONGEST_WAIT + QUIET + Duration::from_secs(1);
        l.note(vec![3], later);
        assert_eq!(l.take_due(later + FIRST_WAIT), [3]);
    }
}
