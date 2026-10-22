//! The C/M/T letters of Aurora's inventory rows and the "CMT" menu that
//! chooses which rights they show: yours by default, the next owner's (what
//! a resident you give the item to gets), both, or none. The letters are
//! the Legacy Viewer's (LLItemBridge::getLabelSuffix); a right refused is
//! struck through and dimmed, the next owner's come after an arrow.

use super::super::permissions::{Permissions, Rights};
use crate::theme::Palette;
use crate::ui::{menu, sheet};
use aurora_net::build::perm;
use egui::{Color32, Pos2, Sense, Stroke, Vec2};

/// The room of one letter.
const SLOT: f32 = 14.0;
/// The room of the arrow before the next owner's letters.
const ARROW: f32 = 16.0;
const SIZE: f32 = 11.0;

/// The width the letters take.
pub fn width(rights: Rights) -> f32 {
    match rights {
        Rights::Hidden => 0.0,
        Rights::Mine => 3.0 * SLOT,
        Rights::Next => ARROW + 3.0 * SLOT,
        Rights::Both => 6.0 * SLOT + ARROW,
    }
}

/// The colour of a letter: allowed, refused or unknown (a broken link),
/// lighter on a selected row.
fn ink(p: &Palette, allowed: Option<bool>, selected: bool) -> Color32 {
    match allowed {
        Some(true) if selected => p.violet_pale,
        _ => super::super::permissions::letter_ink(p, allowed),
    }
}

fn letters(ui: &egui::Ui, p: &Palette, mask: Option<u32>, left: f32, center_y: f32, selected: bool) -> f32 {
    let mut x = left;
    for (bit, letter) in [(perm::COPY, "C"), (perm::MODIFY, "M"), (perm::TRANSFER, "T")] {
        let allowed = mask.map(|mask| mask & bit != 0);
        let color = ink(p, allowed, selected);
        let galley = ui.painter().layout_job(sheet::semibold(ui, letter, SIZE, color));
        let center = Pos2::new(x + SLOT / 2.0, center_y);
        crate::ui::widgets::paint_ink_centered(ui.painter(), galley.clone(), center, color);
        if allowed == Some(false) {
            let half = galley.size().x / 2.0 + 1.0;
            ui.painter()
                .hline(center.x - half..=center.x + half, center.y, Stroke::new(1.0, color));
        }
        x += SLOT;
    }
    x
}

/// Paint `permissions` as `rights` says, from `left`, centred on `center_y`.
pub fn paint(ui: &egui::Ui, p: &Palette, permissions: Permissions, rights: Rights, left: f32, center_y: f32, selected: bool) {
    let mut x = left;
    if rights.shows_mine() {
        x = letters(ui, p, permissions.owner(), x, center_y, selected);
    }
    if rights.shows_next() {
        let galley = ui
            .painter()
            .layout_no_wrap("→".to_owned(), egui::FontId::proportional(SIZE), p.muted);
        crate::ui::widgets::paint_ink_centered(ui.painter(), galley, Pos2::new(x + ARROW / 2.0, center_y), p.muted);
        letters(ui, p, permissions.next(), x + ARROW, center_y, selected);
    }
}

/// The letters as a widget of their own (the cards of the selection).
pub fn label(ui: &mut egui::Ui, p: &Palette, permissions: Permissions, rights: Rights) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width(rights), 16.0), Sense::hover());
    paint(ui, p, permissions, rights, rect.left(), rect.center().y, false);
    response.on_hover_text(permissions.tooltip())
}

/// The text of the column's header for `rights`.
pub fn heading(rights: Rights) -> &'static str {
    match rights {
        Rights::Next => aurora_i18n::tr!("inventory-column-next-owner"),
        Rights::Both => aurora_i18n::tr!("inventory-column-both-rights"),
        _ => aurora_i18n::tr!("inventory-column-rights"),
    }
}

const CHEVRON: f32 = 12.0;

/// The width of the "CMT" button.
pub fn button_width(ui: &egui::Ui) -> f32 {
    ui.painter()
        .layout_job(sheet::semibold(ui, "CMT", SIZE, Color32::PLACEHOLDER))
        .size()
        .x
        + CHEVRON
        + 18.0
}

/// The "CMT" button and its menu. Lit while the rows show rights.
pub fn button(ui: &mut egui::Ui, p: &Palette, rights: &mut Rights, height: f32) -> egui::Response {
    let size = Vec2::new(button_width(ui), height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let lit = *rights != Rights::Hidden;
    let hot = response.hovered() || response.has_focus();
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if lit || hot {
        let fill = if hot { p.raised.lerp_to_gamma(p.ink, 0.16) } else { p.raised };
        ui.painter().rect_filled(rect, radius, fill);
    }
    let ink = if lit || hot { p.ink } else { p.muted };
    let text = ui.painter().layout_job(sheet::semibold(ui, "CMT", SIZE, ink));
    let text_left = rect.left() + 8.0;
    crate::ui::widgets::paint_row_text(ui.painter(), text.clone(), text_left, rect.center().y, ink);
    crate::ui::widgets::paint_glyph(
        ui.painter(),
        "chevron-down",
        Pos2::new(text_left + text.size().x + 2.0 + CHEVRON / 2.0, rect.center().y),
        CHEVRON,
        ink,
    );
    if hot {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let tip = aurora_i18n::tr!("inventory-rights-tip");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        menu::section(ui, p, aurora_i18n::tr!("inventory-rights-menu"));
        for (choice, icon, label) in [
            (Rights::Hidden, "eye-off", aurora_i18n::tr!("inventory-rights-hidden")),
            (Rights::Mine, "user-filled", aurora_i18n::tr!("inventory-rights-mine")),
            (Rights::Next, "share", aurora_i18n::tr!("inventory-rights-next")),
            (Rights::Both, "users", aurora_i18n::tr!("inventory-rights-both")),
        ] {
            if menu::check(ui, p, icon, label, *rights == choice, true) {
                *rights = choice;
            }
        }
    });
    response.on_hover_text(tip)
}

/// The rect of a slot, for tests of the letters' layout.
#[cfg(test)]
fn slot(left: f32, index: usize) -> egui::Rect {
    egui::Rect::from_min_size(Pos2::new(left + index as f32 * SLOT, 0.0), Vec2::new(SLOT, 16.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_choice_takes_the_room_of_its_letters() {
        assert_eq!(width(Rights::Hidden), 0.0);
        assert_eq!(width(Rights::Mine), 3.0 * SLOT);
        assert_eq!(width(Rights::Next), width(Rights::Mine) + ARROW);
        assert_eq!(width(Rights::Both), 2.0 * width(Rights::Mine) + ARROW);
        assert_eq!(slot(10.0, 2).left(), 10.0 + 2.0 * SLOT);
    }

    #[test]
    fn mine_is_the_default_and_each_choice_shows_its_masks() {
        assert_eq!(Rights::default(), Rights::Mine);
        assert!(Rights::Mine.shows_mine() && !Rights::Mine.shows_next());
        assert!(!Rights::Next.shows_mine() && Rights::Next.shows_next());
        assert!(Rights::Both.shows_mine() && Rights::Both.shows_next());
        assert!(!Rights::Hidden.shows_mine() && !Rights::Hidden.shows_next());
    }

    #[test]
    fn refused_rights_are_dimmed_and_unknown_ones_are_neither_allowed_nor_refused() {
        let p = crate::theme::Theme::default().palette();
        assert_eq!(ink(&p, Some(true), false), p.muted);
        assert_eq!(ink(&p, Some(true), true), p.violet_pale);
        assert_eq!(ink(&p, Some(false), false), p.muted_dim.gamma_multiply(0.45));
        assert_eq!(ink(&p, None, false), p.muted_dim);
    }
}
