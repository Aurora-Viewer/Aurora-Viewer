//! The flash of a conversation's entry when a message comes in while
//! another conversation is shown: LLMultiFloater::setFloaterFlashing as
//! FSFloaterIMContainer::startFlashingTab calls it (FlashCount 8 times
//! every FlashPeriod 0.5 s, then the tab keeps the flash colour until it
//! is opened). Here a violet wash that rises and falls softly eight times,
//! then stays faint until the conversation is read; each new message
//! starts it again. Preferences › Chat › Flash unread conversations.

use std::collections::HashMap;
use uuid::Uuid;

/// One rise and fall (FlashPeriod).
const PERIOD: f64 = 0.5;
/// Rises and falls (FlashCount).
const COUNT: f64 = 8.0;
/// The violet's strength at the top of a rise, and once the flash is over:
/// below the selected entry's 0.22, which it must not be mistaken for.
const PEAK: f32 = 0.17;
const REST: f32 = 0.06;
/// Longer than this between two frames that drew the list, the window was
/// hidden: what is unread then is not news.
const AWAY: f64 = 1.0;

#[derive(Debug, Default)]
pub(super) struct Flashes {
    /// Per conversation with unread messages: their count when last drawn,
    /// and when its flash began (None: no flash, the faint wash only).
    seen: HashMap<Uuid, (usize, Option<f64>)>,
    /// When the list was last drawn, and whether the frame before this one
    /// drew it too (else the window comes back: what it holds unread is
    /// not news, it stays faint).
    last: Option<f64>,
    fresh: bool,
}

impl Flashes {
    /// Call once per frame that draws the list, before its entries.
    pub(super) fn begin(&mut self, now: f64) {
        self.fresh = self.last.is_some_and(|t| now - t <= AWAY);
        if !self.fresh {
            for (_, start) in self.seen.values_mut() {
                *start = None;
            }
        }
        self.last = Some(now);
    }

    /// The violet's strength (0: none) for the entry of `id` with `unread`
    /// messages this frame (0 for the conversation shown), and whether it
    /// is still moving (a repaint is due).
    pub(super) fn wash(&mut self, id: Uuid, unread: usize, now: f64, on: bool) -> (f32, bool) {
        if unread == 0 {
            self.seen.remove(&id);
            return (0.0, false);
        }
        // more unread than last frame while the list was on screen: news
        // (a conversation seen for the first time too: an IM that opens it)
        let (count, start) = self.seen.entry(id).or_insert((0, None));
        if unread > *count && self.fresh {
            *start = Some(now);
        }
        *count = unread;
        if !on {
            return (0.0, false);
        }
        match start.map(|s| now - s).filter(|t| *t < PERIOD * COUNT) {
            Some(t) => {
                // a cosine: no edge, the wash breathes
                let rise = 0.5 - 0.5 * (std::f64::consts::TAU * t / PERIOD).cos();
                (REST + (PEAK - REST) * rise as f32, true)
            }
            None => (REST, false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Uuid = Uuid::from_u128(1);

    /// A new message starts eight soft rises over four seconds, then the
    /// wash stays faint until the conversation is read; another message
    /// starts it again.
    #[test]
    fn a_message_flashes_then_the_entry_stays_tinted_until_read() {
        let mut f = Flashes::default();
        f.begin(0.0);
        assert_eq!(f.wash(A, 0, 0.0, true), (0.0, false));
        f.begin(0.1);
        assert_eq!(f.wash(A, 1, 0.1, true), (REST, true), "starts at rest");
        f.begin(0.35);
        let (top, moving) = f.wash(A, 1, 0.35, true);
        assert!(moving && (top - PEAK).abs() < 1e-3, "{top}");
        f.begin(4.2);
        assert_eq!(f.wash(A, 1, 4.2, true), (REST, false), "over: faint");
        f.begin(4.3);
        assert!(f.wash(A, 2, 4.3, true).1, "another message flashes again");
        f.begin(4.4);
        assert_eq!(f.wash(A, 0, 4.4, true), (0.0, false), "read");
    }

    /// Unread messages found when the window comes back, or with the
    /// option off, do not flash.
    #[test]
    fn no_flash_for_old_news_nor_with_the_option_off() {
        let mut f = Flashes::default();
        f.begin(10.0);
        assert_eq!(f.wash(A, 3, 10.0, true), (REST, false));
        let mut f = Flashes::default();
        f.begin(0.0);
        f.begin(0.1);
        assert_eq!(f.wash(A, 1, 0.1, false), (0.0, false));
    }
}
