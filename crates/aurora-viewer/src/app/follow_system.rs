//! The interface's colours follow the system (`crate::system_theme`): the
//! frame loop's side of it.

use super::*;

impl App {
    /// The interface's colours again when the followed system changed. The
    /// skin's theme does not change with them, so the crash record stands.
    pub(super) fn follow_system_theme(&mut self) {
        let following = crate::system_theme::following(&self.settings);
        crate::system_theme::set_active(following);
        if !following {
            return;
        }
        crate::system_theme::start(false);
        crate::system_theme::tick();
        if !crate::system_theme::take_changed() {
            return;
        }
        let colors = wanted_interface_colors(&self.settings);
        if colors == crate::interface_colors::current() {
            return;
        }
        log::info!("system theme: {:?}", crate::system_theme::current());
        crate::interface_colors::set(colors);
        self.palette = self.skin.theme.palette();
        self.cursors.set_palette(&self.palette);
        self.skin.theme.apply(&self.egui_ctx, text_scale(&self.settings));
        apply_chat_text_scale(&self.egui_ctx, &self.settings);
    }
}
