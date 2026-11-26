//! What the regions send of themselves: the camera's view by default,
//! everything around while the 360° photo or Area Search is open.
//! Port of LLAgent::changeInterestListMode (indra/newview/llagent.cpp) and of
//! its two holders, LLFloater360Capture and FSAreaSearch (indra/newview/
//! llfloater360capture.cpp, fsareasearch.cpp, originally LGPL 2.1): closing
//! one leaves the mode to the other.
//!
//! What then comes is drawn in by the usual budgets: the full syncs of a
//! frame (`FullSyncBudget`), the downloads by size on screen
//! (`scene/fetch_priority.rs`). Applying the messages themselves is eased
//! here for the first seconds ([`event_budget`]).
use super::*;
use aurora_net::InterestListMode;
use std::time::Duration;

/// How long after Area Search asked for everything its rush is eased.
const RUSH: Duration = Duration::from_secs(30);
/// What a frame gives the network's messages during the rush.
const RUSH_BUDGET: Duration = Duration::from_millis(2);
/// A message that waited this long is a queue the eased budget does not
/// empty: back to the usual one, so that moves and chat are never more than
/// about this late (they may be that late during the rush).
const RUSH_LAG: Duration = Duration::from_millis(150);

#[derive(Default)]
pub(super) struct InterestList {
    mode: InterestListMode,
    /// Area Search is why, since when.
    searching: Option<Instant>,
}

/// The mode the open windows ask for.
fn wanted(in_world: bool, pano: bool, area_search: bool) -> InterestListMode {
    if in_world && (pano || area_search) {
        InterestListMode::Capture360
    } else {
        InterestListMode::Default
    }
}

/// `AURORA_AREA_SEARCH_360=0`: Area Search leaves the mode alone, to compare.
fn area_search_asks() -> bool {
    static ASKS: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ASKS.get_or_init(|| std::env::var("AURORA_AREA_SEARCH_360").as_deref() != Ok("0"))
}

/// The time a frame may spend on the network's messages, `usual` outside
/// the rush; `lag` is how long the message just applied had waited. Gaming
/// mode's longer budget (few frames a second) is never shortened.
fn eased(usual: Duration, rush: bool, lag: Duration) -> Duration {
    if rush && lag < RUSH_LAG && usual <= Duration::from_millis(8) {
        usual.min(RUSH_BUDGET)
    } else {
        usual
    }
}

impl InterestList {
    fn rush(&self, now: Instant) -> bool {
        self.searching.is_some_and(|t| now.saturating_duration_since(t) < RUSH)
    }

    /// `covered`: a login or teleport screen hides the world, whose frames
    /// may be long and whose loading must not slow down.
    pub(super) fn event_budget(&self, usual: Duration, lag: Duration, covered: bool, now: Instant) -> Duration {
        eased(usual, !covered && self.rush(now), lag)
    }
}

impl App {
    /// Every frame: the mode the open windows ask for, told to the regions
    /// when it changes.
    pub(super) fn interest_list_frame(&mut self) {
        let in_world = self.in_world();
        let searching = in_world && self.panels.area_search && area_search_asks();
        if searching != self.interest_list.searching.is_some() {
            self.interest_list.searching = searching.then(Instant::now);
        }
        let mode = wanted(in_world, self.pano_open(), searching);
        if mode != self.interest_list.mode {
            self.interest_list.mode = mode;
            log::info!(
                "interest list mode {} asked of the regions (360° photo {}, area search {searching})",
                mode.as_str(),
                self.pano_open()
            );
            self.send(NetCommand::SetInterestListMode(mode));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_one_window_leaves_the_mode_to_the_other() {
        use InterestListMode::{Capture360, Default};
        assert_eq!(wanted(true, false, false), Default);
        assert_eq!(wanted(true, true, false), Capture360);
        assert_eq!(wanted(true, false, true), Capture360);
        assert_eq!(wanted(true, true, true), Capture360);
        assert_eq!(wanted(false, true, true), Default, "nothing is asked of a region we left");
    }

    #[test]
    fn the_rush_is_eased_until_messages_wait() {
        let ms = Duration::from_millis;
        assert_eq!(eased(ms(8), true, ms(0)), RUSH_BUDGET);
        assert_eq!(eased(ms(8), true, RUSH_LAG), ms(8), "a late queue gets the usual budget");
        assert_eq!(eased(ms(8), false, ms(0)), ms(8));
        assert_eq!(eased(ms(60), true, ms(0)), ms(60), "gaming mode keeps its own");
    }

    #[test]
    fn the_rush_ends_by_itself_and_only_area_search_starts_one() {
        let t = Instant::now();
        let list = InterestList {
            mode: InterestListMode::Capture360,
            searching: Some(t),
        };
        assert!(list.rush(t) && list.rush(t + RUSH - Duration::from_millis(1)));
        assert!(!list.rush(t + RUSH));
        let ms = Duration::from_millis;
        assert_eq!(list.event_budget(ms(8), ms(0), false, t), RUSH_BUDGET);
        assert_eq!(list.event_budget(ms(8), ms(0), true, t), ms(8), "behind a loading screen");
        let photo = InterestList {
            mode: InterestListMode::Capture360,
            searching: None,
        };
        assert!(!photo.rush(t));
    }
}
