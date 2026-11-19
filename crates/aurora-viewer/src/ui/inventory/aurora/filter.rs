//! Dolphin's filter bar (Ctrl+I, Cmd+Option+F on a Mac): a field under the
//! toolbar that lists only the shown folder's entries whose names hold
//! what is typed, apart from the search, which looks through every folder.
//! It stays open from folder to folder, as Dolphin's does; Esc or its
//! button closes it and lists everything again. Aurora difference: the
//! Legacy Viewer filters only through its search (LLInventoryFilter).

use super::super::*;
use crate::ui::widgets;
use egui::Vec2;

/// The bar's height.
pub const HEIGHT: f32 = 30.0;

/// Opens the bar, or gives its field the keyboard again.
pub(in super::super) fn open(st: &mut InventoryUi) {
    st.folder_filter.get_or_insert_with(String::new);
    st.focus_folder_filter = true;
}

/// Closes the bar: the folder lists everything again.
pub(in super::super) fn close(st: &mut InventoryUi) {
    st.folder_filter = None;
    st.focus_folder_filter = false;
}

/// The bar, while it is open.
pub(super) fn bar(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi) {
    let Some(mut text) = st.folder_filter.take() else {
        return;
    };
    let mut closed = false;
    ui.allocate_ui_with_layout(
        Vec2::new(ui.available_width(), HEIGHT),
        egui::Layout::left_to_right(egui::Align::Center),
        |ui| {
            // Dolphin's close button, at the bar's start: the field's own
            // button at its end only clears the text
            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("inventory-filter-close"), true) {
                closed = true;
            }
            let (icon, _) = ui.allocate_exact_size(Vec2::splat(20.0), egui::Sense::hover());
            let tint = if text.is_empty() { p.muted } else { p.violet_light };
            widgets::paint_glyph(ui.painter(), "filter-filled", icon.center(), 14.0, tint);
            let width = ui.available_width().max(80.0);
            let field = widgets::search_field(ui, p, &mut text, aurora_i18n::tr!("inventory-filter-hint"), width);
            #[cfg(test)]
            ui.ctx()
                .data_mut(|data| data.insert_temp(egui::Id::unique("inventory_folder_filter"), field.rect));
            if std::mem::take(&mut st.focus_folder_filter) {
                field.request_focus();
            }
            // egui's field gives the keyboard up on Esc the same frame
            if (field.has_focus() || field.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                closed = true;
            }
        },
    );
    if closed {
        // the list takes the keyboard back
        close(st);
        super::super::clipboard::request_focus(ui, st);
    } else {
        st.folder_filter = Some(text);
    }
}
