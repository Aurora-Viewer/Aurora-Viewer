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

/// The button of the options shared by every set and the import: the
/// app's "more" dots; inside, the options keep the cog.
const MORE_ICON: &str = "dots-filled";

/// A set's row in the column, in the picker's list, a smart list's row.
const SET_ROW_H: f32 = 42.0;
const PICKER_ROW_H: f32 = 36.0;
const SMART_ROW_H: f32 = 30.0;
/// The picker's button.
const PICKER_H: f32 = 46.0;

/// The smart lists: (pseudo-set, icon, label key). Difference (human
/// decision): the Legacy Viewer's "Non-friends" entry (CS_SET_EXTRA_AVS)
/// is left out; those residents still show in their own sets and in All
/// sets, and the classic Contacts window keeps the entry.
fn smart_lists() -> [(&'static str, &'static str, &'static str); 3] {
    [
        (contact_sets::ALL_SETS, "users", "people-sets-smart-all"),
        (contact_sets::NO_SETS, "user-plus", "people-sets-smart-none"),
        (contact_sets::PSEUDONYM, "edit-filled", "people-sets-smart-nicknamed"),
    ]
}

/// Whether `choice` is one of the smart lists this tab offers.
pub(super) fn is_smart(choice: &str) -> bool {
    smart_lists().iter().any(|(id, _, _)| *id == choice)
}

/// The label of a set or smart list.
pub(super) fn label(choice: &str) -> String {
    smart_lists()
        .iter()
        .find(|(id, _, _)| *id == choice)
        .map_or_else(|| choice.to_owned(), |(_, _, key)| aurora_i18n::text(key).to_owned())
}

/// "YOUR SETS", "New" and the options' "more" button.
fn header(ui: &mut egui::Ui, c: &mut Ctx) {
    let p = c.p;
    let line = Vec2::new(ui.available_width(), 26.0);
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        // the options of all sets and their import, behind "more", at the
        // right edge with "New" to its left
        // flat like "+": a face only under the pointer or while its menu is open
        let (cell, gear) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::click());
        let gear = gear.on_hover_text(aurora_i18n::tr!("people-sets-gear"));
        let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&gear));
        if gear.hovered() || open {
            ui.painter()
                .rect_filled(cell, ui.visuals().widgets.inactive.corner_radius, p.raised);
        }
        super::super::radar::icon(ui, MORE_ICON, cell, 14.0, if gear.hovered() || open { p.ink } else { p.muted });
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
        if widgets::icon_button(ui, p, "plus", aurora_i18n::tr!("ui-contacts-add-a-new-contact-set"), true) {
            c.contacts.ask_new_set();
        }
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
    // the hint rests at the column's bottom; the lists scroll above it
    let mut job = egui::text::LayoutJob::single_section(
        aurora_i18n::tr!("people-sets-priority-hint").to_owned(),
        egui::TextFormat::simple(FontId::proportional(11.0), p.muted),
    );
    job.wrap.max_width = (ui.available_width() - 4.0).max(1.0);
    let hint = ui.painter().layout_job(job);
    let hint_h = hint.size().y;
    egui::ScrollArea::vertical()
        .id_salt("people-sets-side-rows")
        .auto_shrink([false, false])
        .max_height((ui.available_height() - hint_h - 12.0).max(0.0))
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
        });
    let bottom = ui.max_rect().bottom();
    ui.painter()
        .galley(Pos2::new(ui.max_rect().left() + 2.0, bottom - hint_h), hint, p.muted);
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
                let full = ui.available_width();
                let half = (full - 2.0) / 2.0;
                for pair in smart_lists().chunks(2) {
                    // a list left alone on the last line takes the whole width
                    let width = if pair.len() == 1 { full } else { half };
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (id, icon, _) in pair {
                            ui.allocate_ui(Vec2::new(width, SMART_ROW_H), |ui| smart_row(ui, c, st, set, id, icon));
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

/// A set carried in the list: its name, where it stood, and where the
/// pointer holds the card (from the card's top).
#[derive(Clone, Debug)]
struct Carry {
    name: String,
    from: usize,
    grab: f32,
}

/// How the rows slide: each set's offset from its place, eased toward
/// where it should be, and the set carried.
#[derive(Clone, Debug, Default)]
struct Slide {
    offsets: std::collections::HashMap<String, f32>,
    carry: Option<Carry>,
    /// A set released, until the new order shows (the edit is applied
    /// after this frame): drawn where it was dropped meanwhile.
    dropped: Option<Dropped>,
    /// While sets are reordered, each tile shows its place: the number
    /// shown, the one before, and when it changed (ui time).
    numbers: std::collections::HashMap<String, (usize, Option<usize>, f64)>,
}

/// How long a tile's number takes to turn over.
const NUMBER_TURN: f64 = 0.22;

/// A set released at `to`, its card at `card_top`, over `order` (the order
/// when it was released); `frames` it has waited.
#[derive(Clone, Debug)]
struct Dropped {
    carry: Carry,
    to: usize,
    card_top: f32,
    order: Vec<String>,
    frames: u32,
}

/// The place a card whose top is at `card_top` takes among `n` rows, the
/// first at `top0`, one every `pitch`.
fn target_slot(card_top: f32, top0: f32, pitch: f32, n: usize) -> usize {
    if n == 0 || pitch <= 0.0 {
        return 0;
    }
    (((card_top - top0) / pitch).round().max(0.0) as usize).min(n - 1)
}

/// Where the row `i` stands while the set at `from` is carried to `to`:
/// the rows between make way, one step toward the place it left.
fn shifted_slot(i: usize, from: usize, to: usize) -> usize {
    if from < to && i > from && i <= to {
        i - 1
    } else if to < from && i >= to && i < from {
        i + 1
    } else {
        i
    }
}

/// Where each set lands in the order once `from` is put at `to`.
fn new_index(i: usize, from: usize, to: usize) -> usize {
    if i == from { to } else { shifted_slot(i, from, to) }
}

/// A set's row: its tile, its name and counts (or count), and room for the
/// cog when `cog` is shown. Painted with `ui`'s painter (another layer for
/// the card carried).
#[allow(clippy::too_many_arguments)]
fn paint_set_row(
    ui: &egui::Ui,
    c: &Ctx,
    rect: Rect,
    set: &contact_sets::ContactSet,
    face: Option<egui::Color32>,
    cog: bool,
    compact: bool,
    number: Option<crate::ui::contacts::TileNumber>,
) {
    let p = c.p;
    let radius = ui.visuals().widgets.inactive.corner_radius;
    if let Some(face) = face {
        ui.painter().rect_filled(rect, radius, face);
    }
    let side = if compact { 26.0 } else { 28.0 };
    let tile = Rect::from_center_size(Pos2::new(rect.left() + 6.0 + side / 2.0, rect.center().y), Vec2::splat(side));
    match number {
        Some(number) => {
            // a little lift as the number turns over
            let pop = (number.turn.clamp(0.0, 1.0) * std::f32::consts::PI).sin() * 2.0;
            crate::ui::contacts::set_tile_number(ui, p, tile.expand(pop), set, number);
        }
        None => crate::ui::contacts::set_tile(ui, p, tile, set),
    }
    let m: Members = members(c.world, &set.name);
    let text_right = if cog { rect.right() - 31.0 } else { rect.right() - 6.0 };
    if compact {
        let count = m.ids.len().to_string();
        let g = ui.painter().layout_no_wrap(count, FontId::proportional(11.5), p.muted);
        let count_w = g.size().x;
        widgets::paint_row_text(ui.painter(), g, text_right - count_w, rect.center().y, p.muted);
        let mut job = egui::text::LayoutJob::single_section(set.name.clone(), super::super::rows::semibold(ui, 13.0, p.ink));
        job.wrap = egui::text::TextWrapping::truncate_at_width((text_right - count_w - 8.0 - tile.right() - 9.0).max(0.0));
        widgets::paint_row_text(
            ui.painter(),
            ui.painter().layout_job(job),
            tile.right() + 9.0,
            rect.center().y,
            p.ink,
        );
    } else {
        two_lines(
            ui,
            p,
            tile.right() + 9.0,
            text_right,
            rect.center().y,
            &set.name,
            &count_line(&m),
            13.0,
        );
    }
}

/// The sets, highest first: a click shows one, its cog opens its editor,
/// residents dropped join it. A set dragged is carried under the pointer
/// as a lifted card while the others slide out of its way; released, it
/// takes that place in the priority order.
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
        return;
    }
    let compact = height < SET_ROW_H;
    let key = ui.make_persistent_id("people-set-slide");
    let mut slide: Slide = ui.ctx().data(|d| d.get_temp(key)).unwrap_or_default();
    // the places, fixed: what the pointer clicks and drops on
    let rows: Vec<(Rect, egui::Response)> = names
        .iter()
        .map(|_| ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click_and_drag()))
        .collect();
    let top0 = rows[0].0.top();
    let pitch = if rows.len() > 1 { rows[1].0.top() - top0 } else { height };
    let pointer = ui.input(|i| i.pointer.interact_pos());
    let down = ui.input(|i| i.pointer.primary_down());
    if slide.carry.is_none()
        && let Some((i, (rect, _))) = rows.iter().enumerate().find(|(_, (_, r))| r.drag_started())
        // held where it was pressed, not where the drag began to count
        && let Some(at) = ui.input(|i| i.pointer.press_origin()).or(pointer)
    {
        slide.carry = Some(Carry {
            name: names[i].clone(),
            from: i,
            grab: at.y - rect.top(),
        });
    }
    // the carry is dropped if its set went away meanwhile
    if slide.carry.as_ref().is_some_and(|carry| names.get(carry.from) != Some(&carry.name)) {
        slide.carry = None;
    }
    // a drop waiting for the new order: once it shows, the offsets move to
    // the new places, so that every row glides on from where it is drawn
    if let Some(drop) = slide.dropped.as_mut() {
        drop.frames += 1;
        let applied = drop.order != names && names.get(drop.to) == Some(&drop.carry.name);
        if applied {
            for (i, name) in drop.order.iter().enumerate() {
                let j = new_index(i, drop.carry.from, drop.to);
                if let Some(offset) = slide.offsets.get_mut(name) {
                    *offset += (i as f32 - j as f32) * pitch;
                }
            }
        }
        // applied, or not after half a second, or the sets changed otherwise:
        // the rows glide back to their places
        if applied || drop.frames > 30 || drop.order != names {
            slide.dropped = None;
        }
    }
    let n = names.len();
    // the set held (carried, or dropped and waiting): where it stood, the
    // top of its card, the place it takes
    let held: Option<(usize, f32, usize)> = match (&slide.carry, &slide.dropped) {
        (Some(carry), _) => pointer.map(|at| {
            let top = at.y - carry.grab;
            (carry.from, top, target_slot(top, top0, pitch, n))
        }),
        (None, Some(drop)) => Some((drop.carry.from, drop.card_top, drop.to)),
        _ => None,
    };
    // where every row should be, and the ease toward it
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    let ease = 1.0 - (-dt * 22.0).exp();
    let mut moving = false;
    for (i, name) in names.iter().enumerate() {
        let carried = held.is_some_and(|(from, _, _)| from == i);
        let target = match held {
            Some((from, top, _)) if from == i => top - rows[i].0.top(),
            Some((from, _, to)) => (shifted_slot(i, from, to) as f32 - i as f32) * pitch,
            None => 0.0,
        };
        let offset = slide.offsets.entry(name.clone()).or_insert(0.0);
        // the card follows the pointer at once, the others glide
        *offset = if carried { target } else { *offset + (target - *offset) * ease };
        if (target - *offset).abs() > 0.5 {
            moving = true;
        } else if !carried {
            *offset = target;
        }
    }
    slide.offsets.retain(|name, _| names.contains(name));
    // released: the set takes its place once the edit is applied
    if slide.carry.is_some() && !down {
        if let (Some(carry), Some((from, card_top, to))) = (slide.carry.take(), held)
            && to != from
        {
            context::request(
                ui.ctx(),
                context::CtxAction::ContactSet(SetEdit::Reorder {
                    set: carry.name.clone(),
                    index: to,
                }),
            );
            slide.dropped = Some(Dropped {
                carry,
                to,
                card_top,
                order: names.clone(),
                frames: 0,
            });
        }
        moving = true;
    }
    // the places shown on the tiles while a set is held
    let now = ui.input(|i| i.time);
    match held {
        Some((from, _, to)) => {
            for (i, name) in names.iter().enumerate() {
                let place = if i == from { to } else { shifted_slot(i, from, to) } + 1;
                let entry = slide.numbers.entry(name.clone()).or_insert((place, None, now));
                if entry.0 != place {
                    *entry = (place, Some(entry.0), now);
                }
                if now - entry.2 < NUMBER_TURN {
                    moving = true;
                }
            }
        }
        None => slide.numbers.clear(),
    }
    let number_of = |name: &str| {
        slide
            .numbers
            .get(name)
            .map(|(now_shown, before, changed)| crate::ui::contacts::TileNumber {
                now: *now_shown,
                before: *before,
                turn: ((now - changed) / NUMBER_TURN) as f32,
            })
    };
    let carrying = slide
        .carry
        .as_ref()
        .map(|carry| carry.from)
        .or(slide.dropped.as_ref().map(|drop| drop.carry.from));
    if carrying.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    for (i, name) in names.iter().enumerate() {
        let Some(set) = c.world.contact_sets.set(name) else { continue };
        let (rect, resp) = &rows[i];
        let at = rect.translate(Vec2::new(0.0, slide.offsets.get(name).copied().unwrap_or(0.0)));
        let on = name == current;
        if carrying == Some(i) {
            // the card in the hand, lifted over the list
            let layer = egui::LayerId::new(egui::Order::Tooltip, key.with("carried"));
            let mut card = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(at));
            card.set_clip_rect(ui.ctx().content_rect());
            let radius = ui.visuals().widgets.inactive.corner_radius;
            card.painter()
                .rect_filled(at.translate(Vec2::new(0.0, 4.0)).expand(2.0), radius, p.navy.gamma_multiply(0.45));
            card.painter()
                .rect_stroke(at, radius, egui::Stroke::new(1.5, p.violet), egui::StrokeKind::Inside);
            paint_set_row(
                &card,
                c,
                at,
                set,
                Some(p.raised.lerp_to_gamma(p.ink, 0.06)),
                false,
                compact,
                number_of(name),
            );
            continue;
        }
        let hovered = carrying.is_none() && (resp.hovered() || resp.contains_pointer());
        // a resident carried over the set: it lights up to take them
        let taking = egui::DragAndDrop::has_payload_of_type::<crate::ui::call_invite::PersonDrag>(ui.ctx()) && resp.contains_pointer();
        let face = if taking {
            Some(p.violet.gamma_multiply(0.22))
        } else if on {
            Some(p.raised)
        } else if hovered {
            Some(p.raised.gamma_multiply(0.5))
        } else {
            None
        };
        let cog_shown = carrying.is_none() && (on || hovered) && !egui::DragAndDrop::has_any_payload(ui.ctx());
        paint_set_row(ui, c, at, set, face, cog_shown && !taking, compact, number_of(name));
        if taking {
            ui.painter().rect_stroke(
                at,
                ui.visuals().widgets.inactive.corner_radius,
                egui::Stroke::new(1.5, p.violet),
                egui::StrokeKind::Inside,
            );
        }
        if cog_shown {
            let radius = ui.visuals().widgets.inactive.corner_radius;
            let cog = Rect::from_center_size(Pos2::new(rect.right() - 15.0, rect.center().y), Vec2::splat(24.0));
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
        // residents dropped join it (the people chosen with the one dragged)
        crate::ui::contacts::drag::set_target_with(resp, name, &st.sel);
    }
    if moving || slide.carry.is_some() || slide.dropped.is_some() {
        ui.ctx().request_repaint();
    }
    ui.ctx().data_mut(|d| d.insert_temp(key, slide));
}

/// A smart list's row: its icon, its name, its count.
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
    let g = ui
        .painter()
        .layout_no_wrap(aurora_i18n::integer(count as i64), FontId::proportional(11.5), p.muted);
    let count_w = g.size().x;
    let right = rect.right() - 8.0;
    widgets::paint_row_text(ui.painter(), g, right - count_w, rect.center().y, p.muted);
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
    fn a_carried_set_takes_the_place_under_it_and_the_others_make_way() {
        // four rows, 40 apart from 100
        assert_eq!(target_slot(100.0, 100.0, 40.0, 4), 0);
        assert_eq!(target_slot(119.0, 100.0, 40.0, 4), 0, "under half a row: stays");
        assert_eq!(target_slot(121.0, 100.0, 40.0, 4), 1);
        assert_eq!(target_slot(10.0, 100.0, 40.0, 4), 0, "above the list: first");
        assert_eq!(target_slot(900.0, 100.0, 40.0, 4), 3, "below it: last");
        // A (0) carried to 2: B and C step up, D stays
        assert_eq!([1, 2, 3].map(|i| shifted_slot(i, 0, 2)), [0, 1, 3]);
        // D (3) carried to 1: B and C step down, A stays
        assert_eq!([0, 1, 2].map(|i| shifted_slot(i, 3, 1)), [0, 2, 3]);
        // the new order once released matches the model's
        let mut sets = contact_sets::ContactSets::default();
        for name in ["A", "B", "C", "D"] {
            sets.add_set(name);
        }
        let before = sets.ordered_set_names();
        sets.move_priority_to("D", 1);
        let after = sets.ordered_set_names();
        for (i, name) in before.iter().enumerate() {
            assert_eq!(after[new_index(i, 3, 1)], *name);
        }
    }

    /// Where the set `name` is drawn this frame: its place plus its offset.
    fn drawn_top(ctx: &egui::Context, key: egui::Id, name: &str, slot_top: f32) -> f32 {
        let slide: Slide = ctx.data(|d| d.get_temp(key)).unwrap_or_default();
        slot_top + slide.offsets.get(name).copied().unwrap_or(0.0)
    }

    /// A drag down the real list, frame by frame: the card follows the
    /// pointer, and from the frame it is released to the frame the new
    /// order shows it does not jump (it waits where it was dropped, then
    /// glides from there).
    #[test]
    fn a_dropped_set_stays_where_it_was_let_go_until_it_glides_into_place() {
        let p = crate::theme::Theme::default().palette();
        let mut world = crate::world::World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for name in ["A", "B", "C", "D"] {
            world.contact_sets.add_set(name);
        }
        let mut contacts = crate::ui::contacts::ContactsUi::default();
        let mut friends = super::super::super::FriendsSettings::default();
        let images = std::collections::HashMap::new();
        let mut actions = Vec::new();
        let mut st = PeopleUi::default();
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let key_cell = std::cell::Cell::new(egui::Id::NULL);
        let tops_cell = std::cell::RefCell::new(Vec::<f32>::new());
        let mut frame = |frames: &mut crate::ui::widgets::elide_tests::Frames, world: &crate::world::World, events: Vec<egui::Event>| {
            frames.run(events, |ui| {
                ui.set_width(220.0);
                key_cell.set(ui.make_persistent_id("people-set-slide"));
                let top = ui.cursor().top();
                tops_cell.replace((0..4).map(|i| top + i as f32 * (SET_ROW_H + ui.spacing().item_spacing.y)).collect());
                let mut c = Ctx {
                    p: &p,
                    world,
                    contacts: &mut contacts,
                    friends: &mut friends,
                    images: &images,
                    actions: &mut actions,
                    embedded_contacts: false,
                    contact_sheet_on_click: false,
                };
                set_rows(ui, &mut c, &mut st, "A", SET_ROW_H);
            });
        };
        let button = |at: Pos2, pressed: bool| egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&mut frames, &world, Vec::new());
        let tops = tops_cell.borrow().clone();
        let key = key_cell.get();
        let start = Pos2::new(60.0, tops[0] + 20.0);
        frame(&mut frames, &world, vec![egui::Event::PointerMoved(start), button(start, true)]);
        // down past C, a little at a time, as a hand does
        let mut at = start;
        for _ in 0..12 {
            at.y += 8.0;
            frame(&mut frames, &world, vec![egui::Event::PointerMoved(at)]);
        }
        let card = drawn_top(&frames.ctx, key, "A", tops[0]);
        assert!(
            (card - (at.y - 20.0)).abs() < 1.0,
            "the card is under the pointer: {card} vs {}",
            at.y - 20.0
        );
        // every tile shows the place it would have: A third, B and C up one
        let slide: Slide = frames.ctx.data(|d| d.get_temp(key)).unwrap_or_default();
        let places: Vec<usize> = ["A", "B", "C", "D"].iter().map(|n| slide.numbers[*n].0).collect();
        assert_eq!(places, [3, 1, 2, 4], "the numbers follow the card");
        assert_eq!(slide.numbers["A"].1, Some(2), "A's number turned over from 2");
        // let go: the edit waits for the app, the card stays put
        frame(&mut frames, &world, vec![button(at, false)]);
        assert!((drawn_top(&frames.ctx, key, "A", tops[0]) - card).abs() < 1.0, "jumped on release");
        frame(&mut frames, &world, Vec::new());
        assert!(
            (drawn_top(&frames.ctx, key, "A", tops[0]) - card).abs() < 1.0,
            "jumped before the new order"
        );
        for request in crate::ui::context::take_requests(&frames.ctx) {
            if let crate::ui::context::CtxAction::ContactSet(edit) = request {
                crate::ui::contacts::apply_set_edit(&mut world, edit);
            }
        }
        let order = world.contact_sets.ordered_set_names();
        let slot = order.iter().position(|n| n == "A").expect("A is still a set");
        assert!(slot >= 2, "A went down past C: {order:?}");
        // the first frame of the new order: drawn where it was, then gliding
        frame(&mut frames, &world, Vec::new());
        let first = drawn_top(&frames.ctx, key, "A", tops[slot]);
        assert!((first - card).abs() < 4.0, "jumped when the order changed: {first} vs {card}");
        for _ in 0..60 {
            frame(&mut frames, &world, Vec::new());
        }
        assert!(
            (drawn_top(&frames.ctx, key, "A", tops[slot]) - tops[slot]).abs() < 0.5,
            "settled in its place"
        );
        let slide: Slide = frames.ctx.data(|d| d.get_temp(key)).unwrap_or_default();
        assert!(slide.numbers.is_empty(), "the badges come back once it has landed");
    }

    #[test]
    fn every_smart_list_is_a_pseudo_set_with_its_own_label() {
        for (id, _, key) in smart_lists() {
            assert!(contact_sets::ContactSets::is_internal(id));
            assert_eq!(label(id), aurora_i18n::text(key));
            assert!(is_smart(id));
        }
        assert_eq!(label("Family"), "Family");
        // the Legacy Viewer's "Non-friends" is not offered here
        assert!(!is_smart(contact_sets::EXTRA_AVS));
        assert!(!is_smart("Family"));
    }
}
