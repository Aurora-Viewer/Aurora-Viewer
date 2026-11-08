//! The user's answer to the download of Aurora's media plugins pack
//! (`media/pack.rs`): the consent card (`ui::media_pack`) and the Download
//! buttons, kept in the settings at once.

use super::*;

impl App {
    /// "Download" (true) or "Not now" (false).
    pub(super) fn media_pack_answer(&mut self, download: bool) {
        if let Some(consent) = self.plugin_pack.answer(download, self.net.runtime())
            && consent != self.settings.media.pack_consent
        {
            self.settings.media.pack_consent = consent;
            self.settings.save();
        }
    }

    /// The card, while it is due and no other dialog is open.
    pub(super) fn media_pack_card(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.plugin_pack.asks() || self.updater.open || self.exit_prompt.open {
            return;
        }
        if let Some(download) = ui::media_pack::card(ctx, p, &self.skin.icons) {
            self.media_pack_answer(download);
        }
    }
}
