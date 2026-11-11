//! An inventory drag resting on a build tab opens that tab.
//!
//! Legacy Viewer: `LLTabContainer::handleDragAndDrop` when
//! `open_tabs_on_drag_and_drop` is set on the tools floater
//! (`floater_tools.xml`), `indra/llui/lltabcontainer.cpp`. The wait is
//! `SCROLL_DELAY_TIME` (0.5 s). That timer belongs to the whole tab
//! container: it starts on the first drag event over the container, is
//! not reset by moving between buttons, and keeps running after the
//! pointer leaves until a later event fires it. Aurora restarts the wait
//! when the pointer leaves the tab, and when the drag ends, so only a
//! rest on that tab opens it. A drag that slides across the strip does
//! not open the last tab on time spent over the others. The strip does
//! not scroll, so the scroll-arrow branch of that function does not apply.

use egui::Rect;

/// `SCROLL_DELAY_TIME` in `lltabcontainer.cpp`.
pub(super) const DELAY_SECS: f64 = 0.5;

/// One section tab the pointer might be over.
pub(super) struct Spot {
    pub tab: usize,
    pub rect: Rect,
    pub enabled: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Hit {
    pub tab: usize,
    pub enabled: bool,
    pub current: bool,
}

/// The tab the drag is resting on, and when that rest started.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Wait {
    tab: Option<usize>,
    since: f64,
}

impl Wait {
    pub(super) fn stop(&mut self) {
        self.tab = None;
    }

    /// The tab to select once the drag has rested on an enabled, hidden
    /// tab for longer than [`DELAY_SECS`]. `now` is egui's time, in seconds.
    pub(super) fn step(&mut self, now: f64, dragging: bool, over: Option<Hit>) -> Option<usize> {
        let candidate = over.filter(|hit| dragging && hit.enabled && !hit.current);
        let Some(hit) = candidate else {
            self.stop();
            return None;
        };
        if self.tab != Some(hit.tab) {
            self.tab = Some(hit.tab);
            self.since = now;
            return None;
        }
        if now - self.since > DELAY_SECS {
            self.stop();
            return Some(hit.tab);
        }
        None
    }
}

fn rect_id(tab: usize) -> egui::Id {
    egui::Id::unique(("build-section-tab", tab))
}

/// Where a section tab was drawn, for the demo pointer.
pub(crate) fn demo_tab_rect(ctx: &egui::Context, tab: usize) -> Option<Rect> {
    ctx.data(|d| d.get_temp(rect_id(tab)))
}

/// Remember the tab rectangles and open the one an inventory drag has
/// rested on. Returns the tab to select.
pub(super) fn apply(ui: &egui::Ui, wait: &mut Wait, spots: &[Spot], current: usize) -> Option<usize> {
    for spot in spots {
        ui.ctx().data_mut(|d| d.insert_temp(rect_id(spot.tab), spot.rect));
    }
    let dragging = crate::ui::inventory::drag::carried_ids(ui.ctx()).is_some();
    let over = spots.iter().find(|spot| ui.rect_contains_pointer(spot.rect)).map(|spot| Hit {
        tab: spot.tab,
        enabled: spot.enabled,
        current: spot.tab == current,
    });
    let now = ui.input(|i| i.time);
    wait.step(now, dragging, over)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(tab: usize, enabled: bool, current: bool) -> Hit {
        Hit { tab, enabled, current }
    }

    #[test]
    fn a_rest_past_half_a_second_opens_the_tab_and_a_pass_does_not() {
        let mut wait = Wait::default();
        let over = Some(hit(4, true, false));
        assert_eq!(wait.step(0.0, true, over), None);
        assert_eq!(wait.step(DELAY_SECS, true, over), None, "exactly the delay does not open");
        assert_eq!(wait.step(DELAY_SECS + 1e-4, true, over), Some(4));
        // Stopped after firing: still on this tab, now current, does not open again.
        assert_eq!(wait.step(2.0, true, Some(hit(4, true, true))), None);
    }

    #[test]
    fn leaving_the_tab_or_ending_the_drag_restarts_the_wait() {
        let mut wait = Wait::default();
        let contents = Some(hit(4, true, false));
        assert_eq!(wait.step(0.0, true, contents), None);
        assert_eq!(wait.step(0.4, true, None), None, "leaving the tab");
        assert_eq!(wait.step(0.9, true, contents), None, "coming back starts over");
        assert_eq!(wait.step(1.4, true, contents), None);
        assert_eq!(wait.step(1.4 + 1e-4, true, contents), Some(4));

        let mut wait = Wait::default();
        assert_eq!(wait.step(0.0, true, contents), None);
        assert_eq!(wait.step(1.0, false, contents), None, "no drag");
        assert_eq!(wait.step(1.1, true, contents), None);
        assert_eq!(wait.step(1.6 + 1e-4, true, contents), Some(4));
    }

    #[test]
    fn sliding_onto_another_tab_does_not_keep_the_first_tab_s_time() {
        let mut wait = Wait::default();
        assert_eq!(wait.step(0.0, true, Some(hit(5, true, false))), None);
        assert_eq!(wait.step(0.4, true, Some(hit(4, true, false))), None);
        assert_eq!(wait.step(0.8, true, Some(hit(4, true, false))), None);
        assert_eq!(wait.step(0.9 + 1e-4, true, Some(hit(4, true, false))), Some(4));
    }

    #[test]
    fn a_disabled_or_current_tab_never_opens() {
        let mut wait = Wait::default();
        let disabled = Some(hit(4, false, false));
        assert_eq!(wait.step(0.0, true, disabled), None);
        assert_eq!(wait.step(1.0, true, disabled), None);
        let current = Some(hit(6, true, true));
        assert_eq!(wait.step(1.0, true, current), None);
        assert_eq!(wait.step(2.0, true, current), None);
    }
}
