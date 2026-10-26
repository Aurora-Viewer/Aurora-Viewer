//! Help › Grid status and the toolbar's Grid status button: the feed of the
//! grid followed while the button stands on a toolbar or the window is open
//! (LLFloaterGridStatus::checkGridStatusRSS), the button's flash on a new
//! entry, the window and its demo (`grid_status.rs`).
use super::*;
use crate::grid_status::{DEMO_FEED, SECOND_LIFE_PAGE};
use crate::settings::GridChoice;

impl App {
    /// Each frame, after the bars (the button's flash shows from the next).
    pub(super) fn grid_status_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if self.demo {
            self.demo_grid_status();
        }
        let (feed, page) = if self.demo {
            let has_feed = crate::demo::grid_status::has_feed();
            (
                has_feed.then(|| DEMO_FEED.to_owned()),
                has_feed.then(|| SECOND_LIFE_PAGE.to_owned()),
            )
        } else {
            let second_life = matches!(self.settings.grid, GridChoice::SecondLife | GridChoice::SecondLifeBeta);
            crate::grid_status::sources(second_life, self.world.grid_status_extras())
        };
        let open = self.panels.grid_status;
        // the Legacy Viewer loads the page at each opening
        if open && !self.grid_status_was_open {
            self.grid_status.refresh();
        }
        if !open && self.grid_status_was_open {
            self.grid_status.fresh.clear();
        }
        self.grid_status_was_open = open;
        let wanted = open || (self.in_world() && self.grid_status_listed);
        let news = self
            .grid_status
            .poll(self.net.runtime(), feed.as_deref(), page.as_deref(), wanted, Instant::now());
        let now = ctx.input(|i| i.time);
        let flash = &mut self.grid_status.flash;
        flash.set_on(true);
        flash.set_still(self.settings.reduce_motion);
        if news {
            flash.message(now, !open);
        }
        if open {
            flash.shown();
        }
        let (wash, moving) = flash.button(now);
        self.toolbar_ui.grid_status_flash = wash;
        if moving {
            ctx.request_repaint();
        }
        match ui::grid_status::show(ctx, p, &self.grid_status, &mut self.panels.grid_status) {
            Some(ui::grid_status::Action::Refresh) => self.grid_status.refresh(),
            Some(ui::grid_status::Action::Open(url)) => ctx.open_url(egui::OpenUrl::new_tab(url)),
            None => {}
        }
    }

    /// AURORA_DEMO_GRID_STATUS: the window opened, or the new incident.
    fn demo_grid_status(&mut self) {
        use crate::demo::grid_status as demo;
        if demo::mode().is_none() {
            return;
        }
        if self.frame_count == demo::OPEN_FRAME && demo::opens() {
            ui::widgets::open_floater(&self.egui_ctx, ui::grid_status::WINDOW, &mut self.panels.grid_status);
        }
        if self.frame_count == demo::NEWS_FRAME && demo::mode() == Some("new") {
            demo::announce();
            self.grid_status.refresh();
        }
    }
}
