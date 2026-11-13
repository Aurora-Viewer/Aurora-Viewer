//! The list of sets of People › Contact sets: a column beside the members
//! in a wide window, a picker above them in a narrow one. The sets in
//! their priority order (dragged to reorder it), each with its tile, its
//! counts and the cog of its editor; then the smart lists, the pseudo-sets
//! of the Legacy Viewer's combo (FSPanelContactSets::refreshSetList).

use super::super::{Ctx, PeopleUi};
use super::{Members, count_line, members};
use crate::theme::Palette;
use crate::ui::contacts::SetEdit;
use crate::ui::{context, menu, widgets};
use crate::world::contact_sets;
use egui::{FontId, Pos2, Rect, Sense, Vec2};

/// A set's row in the column, in the picker's list, a smart list's row.
const SET_ROW_H: f32 = 42.0;
const PICKER_ROW_H: f32 = 36.0;
const SMART_ROW_H: f32 = 30.0;
/// The picker's button.
const PICKER_H: f32 = 46.0;

/// The payload of a set dragged to a new place in the priority order.
#[derive(Clone, Debug)]
struct SetDrag(String);

/// The smart lists: (pseudo-set, icon, label key).
fn smart_lists() -> [(&'static str, &'static str, &'static str); 4] {
    [
        (contact_sets::ALL_SETS, "users", "people-sets-smart-all"),
        (contact_sets::NO_SETS, "user-plus", "people-sets-smart-none"),
        (contact_sets::PSEUDONYM, "edit-filled", "people-sets-smart-nicknamed"),
        (contact_sets::EXTRA_AVS, "user", "people-sets-smart-not-friends"),
    ]
}

/// The label of a set or smart list.
pub(super) fn label(choice: &str) -> String {
    smart_lists()
        .iter()
        .find(|(id, _, _)| *id == choice)
        .map_or_else(|| choice.to_owned(), |(_, _, key)| aurora_i18n::text(key).to_owned())
}

/// "YOUR SETS", the options' gear and "New".
fn header(ui: &mut egui::Ui, c: &mut Ctx) {
    let p = c.p;
    let line = Vec2::new(ui.available_width(), 26.0);
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if widgets::icon_button(ui, p, "plus", aurora_i18n::tr!("ui-contacts-add-a-new-contact-set"), true) {
            c.contacts.ask_new_set();
        }
        let gear = super::super::rows::gear(ui, p, aurora_i18n::tr!("people-sets-gear"));
        let world = c.world;
        let contacts = &mut *c.contacts;
        menu::dropdown(&gear, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            if menu::item(ui, p, "settings-filled", aurora_i18n::tr!("people-sets-options")) {
                contacts.open_set_options();
            }
            menu::separator(ui, p);
            if menu::item_tip(
                ui,
                p,
                "file-import",
                aurora_i18n::tr!("contacts-import"),
                contacts.can_import_contact_sets(),
                aurora_i18n::tr!("contacts-import-hint"),
            ) {
                contacts.import_contact_sets(ui.ctx(), world.agent_id);
            }
        });
        let rest = ui.available_rect_before_wrap();
        caption(ui, p, aurora_i18n::tr!("people-sets-your-sets"), rest.left() + 8.0, rest.center().y);
    });
}

/// A small capitals caption.
fn caption(ui: &egui::Ui, p: &Palette, text: &str, left: f32, y: f32) {
    let g = ui.painter().layout_no_wrap(
        text.to_uppercase(),
        crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), 10.5, p.muted).font_id,
        p.muted,
    );
    widgets::paint_row_text(ui.painter(), g, left, y, p.muted);
}

/// The column of a wide window.
pub(super) fn column(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, set: &str) {
    let p = c.p;
    ui.spacing_mut().item_spacing.y = 2.0;
    header(ui, c);
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .id_salt("people-sets-side-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            set_rows(ui, c, st, set, SET_ROW_H);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 30.0), Sense::hover());
            caption(
                ui,
                p,
                aurora_i18n::tr!("people-sets-smart-lists"),
                rect.left() + 8.0,
                rect.bottom() - 8.0,
            );
            for (id, icon, _) in smart_lists() {
                smart_row(ui, c, st, set, id, icon);
            }
            ui.add_space(12.0);
            ui.add(
                egui::Label::new(
                    egui::RichText::new(aurora_i18n::tr!("people-sets-priority-hint"))
                        .size(11.0)
                        .color(p.muted),
                )
                .wrap(),
            );
        });
}

/// The picker of a narrow window: the set shown, and once clicked the
/// list of sets under it (it pushes the members down).
pub(super) fn picker(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, set: &str) {
    let p = c.p;
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(10.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), PICKER_H), Sense::click());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let face = if resp.hovered() {
        p.raised.lerp_to_gamma(p.ink, 0.05)
    } else {
        p.raised
    };
    ui.painter().rect_filled(rect, radius, face);
    if st.sets_picker_open {
        ui.painter()
            .rect_stroke(rect, radius, egui::Stroke::new(1.5, p.violet), egui::StrokeKind::Inside);
    }
    let tile = Rect::from_center_size(Pos2::new(rect.left() + 8.0 + 15.0, rect.center().y), Vec2::splat(30.0));
    paint_choice_tile(ui, c, tile, set);
    let m = members(c.world, set);
    two_lines(
        ui,
        p,
        tile.right() + 10.0,
        rect.right() - 30.0,
        rect.center().y,
        &label(set),
        &count_line(&m),
        13.5,
    );
    let chevron = if st.sets_picker_open { "chevron-up" } else { "chevron-down" };
    super::super::radar::icon(
        ui,
        chevron,
        Rect::from_center_size(Pos2::new(rect.right() - 16.0, rect.center().y), Vec2::splat(16.0)),
        14.0,
        p.muted,
    );
    if resp.clicked() {
        st.sets_picker_open = !st.sets_picker_open;
    }
    crate::ui::contacts::drag::set_target_with(&resp, set, &st.sel);
    if st.sets_picker_open {
        ui.add_space(6.0);
        egui::Frame::new()
            .fill(p.bar)
            .corner_radius(ui.visuals().window_corner_radius)
            .inner_margin(egui::Margin::same(6))
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                header(ui, c);
                set_rows(ui, c, st, set, PICKER_ROW_H);
                ui.add_space(6.0);
                let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 18.0), Sense::hover());
                caption(ui, p, aurora_i18n::tr!("people-sets-smart-lists"), r.left() + 8.0, r.center().y);
                let half = (ui.available_width() - 2.0) / 2.0;
                for pair in smart_lists().chunks(2) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (id, icon, _) in pair {
                            ui.allocate_ui(Vec2::new(half, SMART_ROW_H), |ui| smart_row(ui, c, st, set, id, icon));
                        }
                    });
                }
            });
    }
}

/// The tile of the set shown, or the icon of a smart list on the raised
/// face.
fn paint_choice_tile(ui: &egui::Ui, c: &Ctx, tile: Rect, choice: &str) {
    match c.world.contact_sets.set(choice) {
        Some(set) => crate::ui::contacts::set_tile(ui, c.p, tile, set),
        None => {
            let radius = ui.visuals().widgets.inactive.corner_radius;
            ui.painter().rect_filled(tile, radius, c.p.raised.lerp_to_gamma(c.p.ink, 0.08));
            let icon = smart_lists()
                .iter()
                .find(|(id, _, _)| *id == choice)
                .map_or("users", |(_, icon, _)| icon);
            super::super::radar::icon(ui, icon, tile, (tile.height() * 0.5).round(), c.p.ink);
        }
    }
}

/// A name and its count line, centred on `y` between `left` and `right`.
#[allow(clippy::too_many_arguments)]
fn two_lines(ui: &egui::Ui, p: &Palette, left: f32, right: f32, y: f32, name: &str, sub: &str, size: f32) {
    let width = (right - left).max(0.0);
    let mut job = egui::text::LayoutJob::single_section(name.to_owned(), super::super::rows::semibold(ui, size, p.ink));
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let name_g = ui.painter().layout_job(job);
    let mut job = egui::text::LayoutJob::single_section(sub.to_owned(), egui::TextFormat::simple(FontId::proportional(11.0), p.muted));
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let sub_g = ui.painter().layout_job(job);
    widgets::paint_row_text(ui.painter(), name_g, left, y - 7.0, p.ink);
    widgets::paint_row_text(ui.painter(), sub_g, left, y + 8.0, p.muted);
}

/// Where a set dragged over the row at `index` would go once released:
/// before it in its top half, after it in its bottom half; `from` is where
/// it stands now (`move_priority_to` takes the place once it is out).
fn drop_index(from: usize, index: usize, lower_half: bool) -> usize {
    let to = index + usize::from(lower_half);
    if from < to { to - 1 } else { to }
}

/// The sets, highest first: a click shows one, its cog opens its editor,
/// a drag moves it in the priority order, residents dropped join it.
fn set_rows(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, current: &str, height: f32) {
    let p = c.p;
    let names = c.world.contact_sets.ordered_set_names();
    if names.is_empty() {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 34.0), Sense::hover());
        let g = ui.painter().layout_no_wrap(
            aurora_i18n::tr!("people-sets-no-sets").to_owned(),
            FontId::proportional(12.0),
            p.muted,
        );
        widgets::paint_row_text(ui.painter(), g, rect.left() + 8.0, rect.center().y, p.muted);
    }
    let dragged = egui::DragAndDrop::payload::<SetDrag>(ui.ctx()).map(|d| d.0.clone());
    let compact = height < SET_ROW_H;
    for (i, name) in names.iter().enumerate() {
        let Some(set) = c.world.contact_sets.set(name) else { continue };
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click_and_drag());
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        let on = name == current;
        let radius = ui.visuals().widgets.inactive.corner_radius;
        let hovered = resp.hovered() || resp.contains_pointer();
        if on {
            ui.painter().rect_filled(rect, radius, p.raised);
        } else if hovered {
            ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
        }
        let side = if compact { 26.0 } else { 28.0 };
        let tile = Rect::from_center_size(Pos2::new(rect.left() + 6.0 + side / 2.0, rect.center().y), Vec2::splat(side));
        crate::ui::contacts::set_tile(ui, p, tile, set);
        let cog = Rect::from_center_size(Pos2::new(rect.right() - 15.0, rect.center().y), Vec2::splat(24.0));
        let m: Members = members(c.world, name);
        let text_right = if on || hovered { cog.left() - 4.0 } else { rect.right() - 6.0 };
        if compact {
            let count = m.ids.len().to_string();
            let g = ui.painter().layout_no_wrap(count, FontId::proportional(11.5), p.muted);
            let count_w = g.size().x;
            widgets::paint_row_text(ui.painter(), g, text_right - count_w, rect.center().y, p.muted);
            let mut job = egui::text::LayoutJob::single_section(name.clone(), super::super::rows::semibold(ui, 13.0, p.ink));
            job.wrap = egui::text::TextWrapping::truncate_at_width((text_right - count_w - 8.0 - tile.right() - 9.0).max(0.0));
            widgets::paint_row_text(
                ui.painter(),
                ui.painter().layout_job(job),
                tile.right() + 9.0,
                rect.center().y,
                p.ink,
            );
        } else {
            two_lines(ui, p, tile.right() + 9.0, text_right, rect.center().y, name, &count_line(&m), 13.0);
        }
        if on || hovered {
            let cog_resp = ui
                .interact(cog, ui.scope_id().with(("people-set-cog", name)), Sense::click())
                .on_hover_text(aurora_i18n::tr!("people-sets-edit"));
            let face = if cog_resp.hovered() {
                p.raised.lerp_to_gamma(p.ink, 0.12)
            } else {
                egui::Color32::TRANSPARENT
            };
            ui.painter().rect_filled(cog, radius, face);
            super::super::radar::icon(ui, "settings-filled", cog, 15.0, if cog_resp.hovered() { p.ink } else { p.muted });
            if cog_resp.clicked() {
                c.contacts.configure_set(name);
            }
        }
        if resp.clicked() && !on {
            c.contacts.set_choice = Some(name.clone());
            st.sel.clear();
            st.sets_picker_open = false;
        }
        // a drag moves the set in the priority order
        resp.dnd_set_drag_payload(SetDrag(name.clone()));
        if let Some(from) = dragged.as_ref().and_then(|d| names.iter().position(|n| n == d))
            && let Some(at) = resp.hover_pos().filter(|_| resp.dnd_hover_payload::<SetDrag>().is_some())
        {
            let lower = at.y > rect.center().y;
            let y = if lower { rect.bottom() + 1.0 } else { rect.top() - 1.0 };
            ui.painter().hline(rect.x_range(), y, egui::Stroke::new(2.0, p.violet));
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if resp.dnd_release_payload::<SetDrag>().is_some() {
                context::request(
                    ui.ctx(),
                    context::CtxAction::ContactSet(SetEdit::Reorder {
                        set: names[from].clone(),
                        index: drop_index(from, i, lower),
                    }),
                );
            }
        }
        // residents dropped join it (the people chosen with the one dragged)
        crate::ui::contacts::drag::set_target_with(&resp, name, &st.sel);
    }
}

/// A smart list's row: its icon, its name, its count; "Not in a set"
/// counts in the accent's badge while friends wait in it.
fn smart_row(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, current: &str, id: &str, icon: &str) {
    let p = c.p;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), SMART_ROW_H), Sense::click());
    let on = id == current;
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if on {
        ui.painter().rect_filled(rect, radius, p.raised);
    } else if resp.hovered() {
        ui.painter().rect_filled(rect, radius, p.raised.gamma_multiply(0.5));
    }
    let cell = Rect::from_center_size(Pos2::new(rect.left() + 6.0 + 14.0, rect.center().y), Vec2::splat(16.0));
    super::super::radar::icon(ui, icon, cell, 15.0, if on { p.ink } else { p.muted });
    let count = members(c.world, id).ids.len();
    let count_text = aurora_i18n::integer(count as i64);
    let badge = id == contact_sets::NO_SETS && count > 0;
    let g = ui.painter().layout_no_wrap(
        count_text,
        if badge {
            super::super::rows::semibold(ui, 11.0, p.on_accent).font_id
        } else {
            FontId::proportional(11.5)
        },
        if badge { p.on_accent } else { p.muted },
    );
    let count_w = g.size().x;
    let right = rect.right() - 8.0;
    if badge {
        let pill = Rect::from_min_max(
            Pos2::new(right - count_w - 10.0, rect.center().y - 9.0),
            Pos2::new(right, rect.center().y + 9.0),
        );
        ui.painter().rect_filled(pill, radius, p.violet);
        widgets::paint_row_text(ui.painter(), g, pill.left() + 5.0, rect.center().y, p.on_accent);
    } else {
        widgets::paint_row_text(ui.painter(), g, right - count_w, rect.center().y, p.muted);
    }
    let left = cell.right() + 9.0;
    let mut job = egui::text::LayoutJob::single_section(label(id), egui::TextFormat::simple(FontId::proportional(12.5), p.ink));
    job.wrap = egui::text::TextWrapping::truncate_at_width((right - count_w - 18.0 - left).max(0.0));
    widgets::paint_row_text(ui.painter(), ui.painter().layout_job(job), left, rect.center().y, p.ink);
    if resp.clicked() && !on {
        c.contacts.set_choice = Some(id.to_owned());
        st.sel.clear();
        st.sets_picker_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_set_dropped_goes_before_or_after_the_row_under_it() {
        // order A B C D, A (0) dragged
        assert_eq!(drop_index(0, 2, false), 1, "above C: between B and C");
        assert_eq!(drop_index(0, 2, true), 2, "below C: between C and D");
        assert_eq!(drop_index(0, 0, true), 0, "below itself: stays");
        // D (3) dragged up
        assert_eq!(drop_index(3, 0, false), 0, "above A: first");
        assert_eq!(drop_index(3, 1, true), 2, "below B");
        assert_eq!(drop_index(3, 3, false), 3, "above itself: stays");
    }

    #[test]
    fn every_smart_list_is_a_pseudo_set_with_its_own_label() {
        for (id, _, key) in smart_lists() {
            assert!(contact_sets::ContactSets::is_internal(id));
            assert_eq!(label(id), aurora_i18n::text(key));
        }
        assert_eq!(label("Family"), "Family");
    }
}
