//! Preferences › Language and updates › Backup: the window that shows what
//! a Legacy Viewer's lists will bring, with the lists to take. The files
//! are read and merged by the app (`app/legacy_lists.rs`).

use super::{OptionsResult, OptionsUi};
use crate::theme::Palette;
use crate::ui::widgets::{ActionKind, Modal, action_button, action_button_if, checkbox, radio};
use crate::world::legacy_lists::{Folder, Parts};
use aurora_i18n::tr;
use egui::{RichText, Vec2};

#[derive(Default)]
pub struct State {
    pub open: bool,
    /// The account's folders and what each holds; None until the search
    /// answers.
    pub found: Option<Vec<Folder>>,
    /// The folder to read.
    pub chosen: usize,
    pub parts: Parts,
    /// A grid whose folders cannot be told apart (`Account::grid_known`).
    pub other_grid: bool,
}

pub(super) fn modal(ctx: &egui::Context, p: &Palette, st: &mut OptionsUi, r: &mut OptionsResult, open: bool) {
    if !(st.legacy_lists.open && open) {
        return;
    }
    let state = &mut st.legacy_lists;
    let modal = Modal::new(egui::Id::unique("options_import_legacy_lists")).show(ctx, |ui| {
        ui.set_width(460.0);
        ui.label(RichText::new(tr!("ui-options-legacy-lists-title")).size(15.0).strong().color(p.ink));
        ui.add_space(6.0);
        ui.label(RichText::new(tr!("ui-options-legacy-lists-text")).size(12.5).color(p.muted));
        ui.add_space(8.0);
        let mut ready = false;
        match &state.found {
            None => {
                ui.label(RichText::new(tr!("ui-options-legacy-lists-searching")).size(12.5).color(p.muted));
            }
            Some(found) if found.is_empty() => {
                ui.label(RichText::new(tr!("ui-options-legacy-lists-none")).size(12.5).color(p.muted));
            }
            Some(found) => {
                state.chosen = state.chosen.min(found.len() - 1);
                for (i, folder) in found.iter().enumerate() {
                    let text = RichText::new(folder.dir.display().to_string()).size(12.5).color(p.ink);
                    if found.len() == 1 {
                        ui.label(text);
                    } else if radio(ui, p, state.chosen == i, text).clicked() {
                        state.chosen = i;
                    }
                }
                if state.other_grid {
                    ui.add_space(6.0);
                    ui.label(RichText::new(tr!("ui-options-legacy-lists-other-grid")).size(12.5).color(p.muted));
                }
                ui.add_space(6.0);
                let folder = &found[state.chosen];
                let counts = folder.counts;
                let parts = &mut state.parts;
                let rows = [
                    (
                        &mut parts.teleports,
                        tr!("ui-options-legacy-lists-teleports", count = counts.teleports),
                        counts.teleports,
                    ),
                    (
                        &mut parts.derender,
                        tr!("ui-options-legacy-lists-derender", count = counts.derender),
                        counts.derender,
                    ),
                    (
                        &mut parts.media,
                        tr!("ui-options-legacy-lists-media", count = counts.media),
                        counts.media,
                    ),
                    (
                        &mut parts.contact_sets,
                        tr!("ui-options-legacy-lists-contact-sets", count = counts.contact_sets),
                        counts.contact_sets,
                    ),
                ];
                for (on, label, count) in rows {
                    ui.add_enabled_ui(count > 0, |ui| {
                        checkbox(ui, p, on, RichText::new(label).size(12.5).color(p.ink));
                    });
                    ready |= *on && count > 0;
                }
                if !folder.unreadable.is_empty() {
                    ui.add_space(6.0);
                    let files = folder.unreadable.join(", ");
                    let text = tr!("ui-options-legacy-lists-failed", files = files);
                    ui.label(RichText::new(text).size(12.5).color(p.muted));
                }
            }
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 26.0);
            if action_button_if(
                ui,
                p,
                "file-import",
                tr!("ui-options-legacy-lists-confirm"),
                ActionKind::Primary,
                size,
                ready,
            )
            .clicked()
            {
                r.import_legacy_lists = Some((state.chosen, state.parts));
                state.open = false;
            }
            if action_button(ui, p, "x", tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                state.open = false;
            }
        });
    });
    if modal.should_close() {
        state.open = false;
    }
}
