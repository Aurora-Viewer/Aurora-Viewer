//! The flash of a conversation's entry when a message comes in while
//! another conversation is shown: LLMultiFloater::setFloaterFlashing as
//! FSFloaterIMContainer::startFlashingTab calls it (FlashCount 8 times
//! every FlashPeriod 0.5 s, then the tab keeps the flash colour until it
//! is opened). Here a violet wash that rises and falls softly eight times,
//! then stays faint until the conversation is read; each new message
//! starts it again. Preferences › Chat › Flash unread conversations.
//!
//! The Conversations button flashes the same way (`ButtonFlash`).

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
/// The Conversations button's wash, against an entry's.
const BUTTON: f32 = 2.0;
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
        match self.track(id, unread, now) {
            Some(start) if on => breathe(start, now),
            _ => (0.0, false),
        }
    }

    /// `wash` for a small face that wears the Conversations button's mark
    /// (a conversation's thumbnail, `thumbnails.rs`): the button's
    /// strength, held at the top of a rise with Reduce motion (`still`).
    pub(super) fn mark(&mut self, id: Uuid, unread: usize, now: f64, still: bool) -> (f32, bool) {
        let Some(start) = self.track(id, unread, now) else {
            return (0.0, false);
        };
        let (wash, moving) = if still { hold(start, now) } else { breathe(start, now) };
        (BUTTON * wash, moving)
    }

    /// When the flash of `id` with `unread` messages began (None: no
    /// flash, the faint wash only), or None when nothing is unread.
    fn track(&mut self, id: Uuid, unread: usize, now: f64) -> Option<Option<f64>> {
        if unread == 0 {
            self.seen.remove(&id);
            return None;
        }
        // more unread than last frame while the list was on screen: news
        // (a conversation seen for the first time too: an IM that opens it)
        let (count, start) = self.seen.entry(id).or_insert((0, None));
        if unread > *count && self.fresh {
            *start = Some(now);
        }
        *count = unread;
        Some(*start)
    }
}

/// How far into a rise the flash begun at `start` stands, 0 to 1, while
/// its eight rises last.
fn rise(start: Option<f64>, now: f64) -> Option<f32> {
    let t = start.map(|s| now - s).filter(|t| (0.0..PERIOD * COUNT).contains(t))?;
    // a cosine: no edge, the wash breathes
    Some((0.5 - 0.5 * (std::f64::consts::TAU * t / PERIOD).cos()) as f32)
}

/// The wash of a flash begun at `start`, faint once it is over, and
/// whether it still moves.
fn breathe(start: Option<f64>, now: f64) -> (f32, bool) {
    match rise(start, now) {
        Some(r) => (REST + (PEAK - REST) * r, true),
        None => (REST, false),
    }
}

/// `breathe` for Preferences › Accessibility › Reduce motion: the same
/// span at the top of a rise, held still, then faint.
fn hold(start: Option<f64>, now: f64) -> (f32, bool) {
    match rise(start, now) {
        Some(_) => (PEAK, true),
        None => (REST, false),
    }
}

/// The Conversations button's flash (FSNotifyIMFlash): LLButton::setFlashing
/// as FSFloaterIMContainer::checkFlashing calls it for the "chat" command
/// when a message arrives with the window closed, FlashCount rises every
/// FlashPeriod, then the button keeps the flash colour until the window is
/// shown (setVisible clears the flashing sessions). There a minimized window
/// is still visible and nothing flashes; here its bar flashes instead, the
/// same eight rises, then keeps the button's tint while unread messages
/// wait inside: a cue that is hard to miss. The Grid status button flashes
/// the same way on a new entry of the feed (`grid_status.rs`).
#[derive(Debug, Default, Clone, Copy)]
pub struct ButtonFlash {
    /// When the latest message that flashes arrived.
    since: Option<f64>,
    /// A message came while the window was closed: the button stays tinted.
    lit: bool,
    /// The same for a resident's line of nearby chat, out of sight: on its
    /// own, as nearby chat may live in its own window (#910).
    nearby_since: Option<f64>,
    lit_nearby: bool,
    /// Preferences › Chat › "Flash the Conversations button" is on.
    on: bool,
    /// Preferences › Accessibility › Reduce motion: a steady light instead
    /// of the rises.
    still: bool,
}

impl ButtonFlash {
    /// Follow the preference, given every frame: off, nothing flashes nor
    /// stays tinted.
    pub fn set_on(&mut self, on: bool) {
        if !on {
            *self = ButtonFlash::default();
        }
        self.on = on;
    }

    /// Follow Reduce motion, given every frame after `set_on`.
    pub fn set_still(&mut self, still: bool) {
        self.still = still;
    }

    /// The wash of the flash begun at `since`, risen and fallen or held
    /// still.
    fn wash(&self, since: Option<f64>, now: f64) -> (f32, bool) {
        let (wash, moving) = if self.still { hold(since, now) } else { breathe(since, now) };
        (BUTTON * wash, moving)
    }

    /// A message that flashes arrived (`World::message_flashes_conversations`),
    /// the window `closed` or else minimized.
    pub fn message(&mut self, now: f64, closed: bool) {
        self.since = Some(now);
        self.lit |= closed;
    }

    /// A resident's line of nearby chat, out of sight (`nearby_flashes`).
    pub fn nearby(&mut self, now: f64) {
        self.nearby_since = Some(now);
        self.lit_nearby = true;
    }

    /// The window is shown, minimized or not: the button goes out for the
    /// conversations.
    pub fn shown(&mut self) {
        self.lit = false;
    }

    /// Nearby chat is on screen (the Conversations window, or its own
    /// window): the button goes out for it.
    pub fn nearby_seen(&mut self) {
        self.lit_nearby = false;
    }

    /// The violet's strength on the button (0: none), and whether it moves:
    /// twice an entry's wash on so small a face, still below the 0.40 of
    /// an open window's button.
    pub fn button(&self, now: f64) -> (f32, bool) {
        let of = |lit: bool, since| if lit { self.wash(since, now) } else { (0.0, false) };
        let (a, b) = (of(self.lit, self.since), of(self.lit_nearby, self.nearby_since));
        (a.0.max(b.0), a.1 || b.1)
    }

    /// The violet of the minimized bar while unread messages wait inside,
    /// and whether it moves: the button's flash when one just arrived, its
    /// tint otherwise.
    pub fn bar(&self, now: f64) -> (f32, bool) {
        if !self.on {
            return (0.0, false);
        }
        self.wash(self.since, now)
    }
}

/// A resident's line of nearby chat flashes the Conversations button
/// (FSNotifyNearbyChatFlash, from on_new_message in llimview.cpp) while
/// nearby chat is out of sight (`nearby_out_of_sight`), never in do not
/// disturb, and not when the button's right-click menu (or Preferences ›
/// Chat) leaves nearby chat out. The Legacy Viewer flashes for objects'
/// lines too; here only residents' lines do, or a chatty script would never
/// let the button rest.
pub fn nearby_flashes(button_on: bool, nearby_on: bool, out_of_sight: bool, dnd: bool) -> bool {
    button_on && nearby_on && out_of_sight && !dnd
}

/// Nearby chat is out of sight: in the Conversations window (`apart`
/// false), that window closed (minimized it still shows, as in the Legacy
/// Viewer); in its own window (#910), that window put away or minimized.
pub fn nearby_out_of_sight(apart: bool, conversations_open: bool, nearby_window_shown: bool) -> bool {
    if apart { !nearby_window_shown } else { !conversations_open }
}

/// What the Conversations button's right-click menu asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonMenu {
    /// Open the Conversations window, as a click on the button would.
    Open,
    /// Whether nearby chat flashes the button from now on.
    NearbyFlash(bool),
}

/// The rows of the Conversations button's right-click menu, `nearby_on`
/// whether nearby chat flashes it (Preferences › Chat): opening the window,
/// and leaving nearby chat out of the flash while IMs, group chats and
/// conferences keep it. Shared by the chat island's button and the
/// toolbars' Conversations button.
pub fn button_menu(ui: &mut egui::Ui, p: &crate::theme::Palette, nearby_on: bool) -> Option<ButtonMenu> {
    use crate::ui::menu;
    let mut out = None;
    if menu::item(ui, p, "message-filled", aurora_i18n::tr!("ui-bars-conversations-open")) {
        out = Some(ButtonMenu::Open);
    }
    menu::separator(ui, p);
    if menu::check(
        ui,
        p,
        "bell-off",
        aurora_i18n::tr!("ui-bars-conversations-no-nearby-flash"),
        !nearby_on,
        true,
    ) {
        out = Some(ButtonMenu::NearbyFlash(!nearby_on));
    }
    out
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

    /// A message with the window closed flashes the button eight times,
    /// then leaves it tinted until the window is shown; minimized, only the
    /// bar flashes, then stays tinted like the button.
    #[test]
    fn the_button_flashes_while_closed_and_the_minimized_bar_flashes() {
        let mut b = ButtonFlash::default();
        assert_eq!(b.bar(0.0), (0.0, false), "the preference off");
        b.set_on(true);
        assert_eq!(b.button(0.0), (0.0, false));
        assert_eq!(b.bar(0.0), (BUTTON * REST, false), "unread before: tinted");
        b.message(1.0, true);
        let (top, moving) = b.button(1.25);
        assert!(moving && (top - BUTTON * PEAK).abs() < 1e-3, "{top}");
        assert_eq!(b.button(5.1), (BUTTON * REST, false), "over: tinted");
        b.shown();
        assert_eq!(b.button(5.2), (0.0, false), "shown: out");
        // minimized: the bar flashes, the button stays out
        b.message(10.0, false);
        assert_eq!(b.button(10.25), (0.0, false));
        let (top, moving) = b.bar(10.25);
        assert!(moving && (top - BUTTON * PEAK).abs() < 1e-3, "{top}");
        assert_eq!(b.bar(14.1), (BUTTON * REST, false), "over: tinted");
    }

    /// Nearby chat flashes the button only while the window is closed, with
    /// both preferences on, nearby chat not shown elsewhere and no do not
    /// disturb; turning nearby chat off leaves IMs alone (they go through
    /// `World::message_flashes_conversations`).
    #[test]
    fn nearby_chat_flashes_the_closed_button_unless_left_out() {
        assert!(nearby_flashes(true, true, true, false));
        assert!(!nearby_flashes(true, false, true, false), "left out by the menu");
        assert!(!nearby_flashes(false, true, true, false), "the button's flash off");
        assert!(!nearby_flashes(true, true, false, false), "nearby chat in sight");
        assert!(!nearby_flashes(true, true, true, true), "do not disturb");
        // in the Conversations window: out of sight only while it is closed
        assert!(nearby_out_of_sight(false, false, false));
        assert!(!nearby_out_of_sight(false, true, false), "open or minimized");
        // in its own window (#910): out of sight while that one is put away,
        // whatever the Conversations window does
        assert!(nearby_out_of_sight(true, true, false));
        assert!(!nearby_out_of_sight(true, false, true));
    }

    /// Nearby chat and the conversations light the button each on its own:
    /// seeing nearby chat (its own window shown) leaves an IM's tint, and
    /// opening the Conversations window leaves nearby chat's while it lives
    /// apart; the minimized bar only flashes for the conversations.
    #[test]
    fn nearby_chat_and_the_conversations_light_the_button_apart() {
        let mut b = ButtonFlash::default();
        b.set_on(true);
        b.nearby(1.0);
        assert!(b.button(1.25).1, "nearby chat flashes the button");
        assert_eq!(b.bar(1.25), (BUTTON * REST, false), "not the minimized bar");
        b.message(2.0, true);
        b.nearby_seen();
        assert!(b.button(2.25).0 > BUTTON * REST, "the IM keeps it lit");
        b.shown();
        assert_eq!(b.button(2.3), (0.0, false));
        b.nearby(3.0);
        b.shown();
        assert_eq!(b.button(7.5), (BUTTON * REST, false), "nearby chat's tint stays");
        b.nearby_seen();
        assert_eq!(b.button(7.6), (0.0, false));
    }

    /// Nearby chat flashes by default (FSNotifyNearbyChatFlash), for a
    /// profile saved before the option too, and the menu's choice is kept.
    #[test]
    fn the_nearby_chat_choice_is_kept() {
        use crate::settings::Settings;
        assert!(Settings::default().flash_conversations_nearby);
        let old: Settings = serde_json::from_str("{}").expect("an old profile");
        assert!(old.flash_conversations_nearby);
        let s = Settings {
            flash_conversations_nearby: false,
            ..Default::default()
        };
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).expect("saved")).expect("read back");
        assert!(!back.flash_conversations_nearby);
    }

    /// Reduce motion: the button lights at the top of a rise and holds it
    /// for the flash's span, then stays tinted, like the moving flash.
    #[test]
    fn reduce_motion_holds_the_flash_still() {
        let mut b = ButtonFlash::default();
        b.set_on(true);
        b.set_still(true);
        b.message(1.0, true);
        for t in [1.0, 1.25, 1.5, 3.9] {
            assert_eq!(b.button(t), (BUTTON * PEAK, true), "{t}");
        }
        assert_eq!(b.button(5.1), (BUTTON * REST, false), "over: tinted");
        b.message(10.0, false);
        assert_eq!(b.bar(10.5), (BUTTON * PEAK, true), "the minimized bar too");
    }
}
