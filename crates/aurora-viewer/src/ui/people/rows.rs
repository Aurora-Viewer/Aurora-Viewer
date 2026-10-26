//! The pieces the lists of People share in Aurora's look (the mock-up's
//! rows): a row's face, its round picture and status dot, its two lines of
//! names, the fixed controls laid out from the right edge (what does not
//! fit folds into "+2" or steps aside), the quick actions over the names
//! of the row under the pointer, and the Ctrl / Shift selection.

use crate::theme::Palette;
use crate::ui::context::{self, AvatarList};
use crate::ui::{menu, presence, widgets};
use crate::world::World;
use egui::{Color32, FontId, Pos2, Rect, Sense, Vec2};
use uuid::Uuid;

/// Height of a row, and the gap between two (Near you's).
pub(super) use super::nearby::{RICH_GAP, RICH_H};
/// What the names keep at least, and the gap after them.
pub(super) const NAMES_MIN: f32 = 60.0;
const NAMES_GAP: f32 = 6.0;
/// Room left of a row's right edge.
const PAD: f32 = 8.0;
/// One quick action's button, and the room around them in their pill.
pub(super) const BUTTON: Vec2 = Vec2::new(26.0, 24.0);
const ACTIONS_PAD: f32 = 3.0;
/// Width of the "+2" of what does not fit.
pub(super) const MORE_W: f32 = 24.0;
/// Height of a toolbar (search, buttons).
pub(super) const TOOLBAR_H: f32 = 30.0;

/// The side of a contact set's symbol after a name (`SetMark::Icon`).
const SET_ICON: f32 = 12.0;

/// The name a row shows: the display name as the name preferences make it
/// (a contact set nickname included), as Near you's rows do.
pub(super) fn display_name(world: &World, id: &Uuid) -> String {
    let names = &world.social.avatar_names;
    names
        .get(id)
        .map(|n| n.display(&names.options))
        .unwrap_or_else(|| world.social.name_of(id))
}

/// A semi-bold text format (the mock-up's weight 600).
pub(super) fn semibold(ui: &egui::Ui, size: f32, color: Color32) -> egui::TextFormat {
    crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(ui.ctx()), size, color)
}

/// The face of a row: the accent's wash when selected, raised under the
/// pointer, else the field.
pub(super) fn face(ui: &egui::Ui, p: &Palette, rect: Rect, selected: bool, hot: bool) {
    let fill = if selected {
        p.violet.gamma_multiply(0.18)
    } else if hot {
        p.raised.gamma_multiply(0.45)
    } else {
        p.field
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
}

/// The round picture at the left of a row, with the status dot of a
/// resident when it is known; returns where the names start.
pub(super) fn picture(
    ui: &egui::Ui,
    p: &Palette,
    world: &World,
    rect: Rect,
    pic: Option<&egui::TextureHandle>,
    id: Uuid,
    name: &str,
    dim: bool,
) -> f32 {
    let disc = Rect::from_center_size(Pos2::new(rect.left() + 8.0 + 15.0, rect.center().y), Vec2::splat(30.0));
    widgets::avatar_disc(ui, p, disc, pic, name, dim);
    let status = presence::of(world, &id);
    if status != presence::Status::Unknown {
        presence::paint_dot(ui.painter(), p, disc, status, p.field);
    }
    names_left(rect)
}

/// The names of a row start there (after a 30 px picture).
pub(super) fn names_left(rect: Rect) -> f32 {
    rect.left() + 8.0 + 30.0 + 10.0
}

/// A square tile at the left of a row (a group's insignia, an object): a
/// picture, or `icon` / `initials` on `fill`; returns where the names start.
pub(super) fn tile(
    ui: &egui::Ui,
    p: &Palette,
    rect: Rect,
    pic: Option<&egui::TextureHandle>,
    icon: &str,
    initials: &str,
    fill: Color32,
) -> f32 {
    let tile = Rect::from_center_size(Pos2::new(rect.left() + 8.0 + 15.0, rect.center().y), Vec2::splat(30.0));
    paint_tile(ui, p, tile, pic, icon, initials, fill);
    names_left(rect)
}

/// The tile itself, at any size.
pub(super) fn paint_tile(
    ui: &egui::Ui,
    p: &Palette,
    tile: Rect,
    pic: Option<&egui::TextureHandle>,
    icon: &str,
    initials: &str,
    fill: Color32,
) {
    let radius = ui.visuals().widgets.inactive.corner_radius;
    match pic {
        Some(t) => {
            egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), tile.size()))
                .tint(widgets::PICTURE_TINT)
                .corner_radius(radius)
                .paint_at(ui, tile);
        }
        None => {
            ui.painter().rect_filled(tile, radius, fill);
            if icon.is_empty() {
                let g = ui.painter().layout_job(egui::text::LayoutJob::single_section(
                    initials.to_owned(),
                    semibold(ui, tile.height() * 0.36, p.on_accent),
                ));
                crate::ui::widgets::paint_ink_centered(ui.painter(), g, tile.center(), p.on_accent);
            } else {
                super::radar::icon(ui, icon, tile, tile.height() * 0.45, p.muted);
            }
        }
    }
}

/// The initials of a group's name: its first letter, then that of the next
/// word in capitals ("Loups du Nord": LN), else of the second word.
pub(super) fn initials(name: &str) -> String {
    let letters: Vec<char> = name
        .split_whitespace()
        .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
        .collect();
    let Some(first) = letters.first() else {
        return String::new();
    };
    let second = letters[1..].iter().find(|c| c.is_uppercase()).or(letters.get(1));
    std::iter::once(first).chain(second).flat_map(|c| c.to_uppercase()).collect()
}

/// What follows a resident's name for their contact set: the set's symbol
/// in its colour when it has one, else a dot of its colour (#1104).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum SetMark {
    Dot(Color32),
    Icon(&'static str, Color32),
}

/// The two lines of a row between `left` and `right`: the first in
/// semi-bold, with a dot of `dot` (a contact set's color) and a tag after
/// it when they fit, the second muted. Returns whether the first was cut.
#[allow(clippy::too_many_arguments)]
pub(super) fn names(
    ui: &egui::Ui,
    p: &Palette,
    rect: Rect,
    left: f32,
    right: f32,
    first: &str,
    color: Color32,
    second: &str,
    dot: Option<Color32>,
    tag: Option<(&str, Color32, Color32)>,
) -> bool {
    person_names(ui, p, rect, left, right, first, color, second, dot.map(SetMark::Dot), tag, None)
}

/// `names` for a resident: a member of the Aurora team or of Linden Lab
/// (`mark`) gets its tag right after the name, which is cut before the
/// tag is (the tag is how the staff is recognized).
#[allow(clippy::too_many_arguments)]
pub(super) fn person_names(
    ui: &egui::Ui,
    p: &Palette,
    rect: Rect,
    left: f32,
    right: f32,
    first: &str,
    color: Color32,
    second: &str,
    set: Option<SetMark>,
    tag: Option<(&str, Color32, Color32)>,
    mark: Option<crate::world::staff::Mark>,
) -> bool {
    let width = (right - left).max(0.0);
    let staff = crate::ui::staff::width(ui, p, mark, 13.0);
    // the tag keeps its place as long as some of the name still shows
    let staff = if staff > 0.0 && width - staff - 6.0 >= 40.0 { staff } else { 0.0 };
    let mut job = egui::text::LayoutJob::single_section(first.to_owned(), semibold(ui, 13.0, color));
    job.wrap = egui::text::TextWrapping::truncate_at_width(if staff > 0.0 { width - staff - 6.0 } else { width });
    let name_g = ui.painter().layout_job(job);
    let cut = name_g.elided;
    let mut job = egui::text::LayoutJob::single_section(second.to_owned(), egui::TextFormat::simple(FontId::proportional(11.5), p.muted));
    job.wrap = egui::text::TextWrapping::truncate_at_width(width);
    let second_g = ui.painter().layout_job(job);
    let lines = if second.is_empty() { 0.0 } else { second_g.size().y - 2.0 };
    let top = rect.center().y - (name_g.size().y + lines) / 2.0;
    let (name_w, name_h) = (name_g.size().x, name_g.size().y);
    ui.painter().galley(Pos2::new(left, top), name_g, color);
    if !second.is_empty() {
        ui.painter().galley(Pos2::new(left, top + name_h - 2.0), second_g, p.muted);
    }
    // what follows the name, only where it fits
    let mut x = left + name_w + 6.0;
    let y = top + name_h / 2.0;
    if staff > 0.0
        && let Some(r) = crate::ui::staff::paint_at(ui, ui.painter(), p, mark, Pos2::new(x, y), 13.0)
    {
        x = r.right() + 6.0;
    }
    match set.filter(|_| !cut) {
        Some(SetMark::Dot(c)) if x + 9.0 <= right => {
            ui.painter().circle_filled(Pos2::new(x + 4.5, y), 4.5, c);
            x += 9.0 + 6.0;
        }
        Some(SetMark::Icon(icon, c)) if x + SET_ICON <= right => {
            let cell = Rect::from_min_size(Pos2::new(x, y - SET_ICON / 2.0), Vec2::splat(SET_ICON));
            super::radar::icon(ui, icon, cell, SET_ICON, c);
            x += SET_ICON + 6.0;
        }
        _ => {}
    }
    if let Some((text, ink, fill)) = tag.filter(|_| !cut) {
        let g = ui
            .painter()
            .layout_no_wrap(text.to_owned(), FontId::new(10.5, crate::ui::fonts::semibold(ui.ctx())), ink);
        let r = Rect::from_min_size(Pos2::new(x, y - 8.0), Vec2::new(g.size().x + 12.0, 16.0));
        if r.right() <= right {
            ui.painter().rect_filled(r, ui.visuals().window_corner_radius, fill);
            let left = r.center().x - g.size().x / 2.0;
            crate::ui::widgets::paint_row_text(ui.painter(), g, left, r.center().y, ink);
        }
    }
    cut
}

/// Where the parts of a row's right end go, laid from the right edge: the
/// first of `widths` is the rightmost. What would leave the names less
/// than `NAMES_MIN` does not show: it folds into a "+2" when `fold`, else
/// it steps aside (the parts the sheet and the menu also hold).
#[derive(Debug)]
pub(super) struct End {
    /// One place per part shown, in the order of `widths`.
    pub items: Vec<Rect>,
    /// The "+2" of the others, and how many it stands for.
    pub more: Option<(Rect, usize)>,
    /// Where the names stop.
    pub names_right: f32,
}

pub(super) fn end(row: Rect, names_left: f32, widths: &[f32], gap: f32, fold: bool) -> End {
    end_before(row, names_left + NAMES_MIN + NAMES_GAP, widths, gap, fold)
}

/// Keep the fixed right-side parts after a reserved horizontal strip.
pub(super) fn end_before(row: Rect, minimum_left: f32, widths: &[f32], gap: f32, fold: bool) -> End {
    let right = row.right() - PAD;
    let room = right - minimum_left;
    let total = |k: usize| -> f32 { widths[..k].iter().sum::<f32>() + gap * k.saturating_sub(1) as f32 };
    let n = widths.len();
    let (shown, more) = if total(n) <= room {
        (n, false)
    } else if fold {
        let k = (0..n).rev().find(|k| total(*k) + if *k > 0 { gap } else { 0.0 } + MORE_W <= room);
        match k {
            Some(k) => (k, true),
            None => (0, false),
        }
    } else {
        ((0..n).rev().find(|k| total(*k) <= room).unwrap_or(0), false)
    };
    let mut x = right;
    let items: Vec<Rect> = widths[..shown]
        .iter()
        .map(|w| {
            let r = Rect::from_min_max(Pos2::new(x - w, row.top()), Pos2::new(x, row.bottom()));
            x = r.left() - gap;
            r
        })
        .collect();
    let more = more.then(|| {
        let r = Rect::from_min_max(Pos2::new(x - MORE_W, row.top()), Pos2::new(x, row.bottom()));
        x = r.left() - gap;
        (r, n - shown)
    });
    let leftmost = more.map(|(r, _)| r.left()).or(items.last().map(|r| r.left())).unwrap_or(right);
    End {
        items,
        more,
        names_right: leftmost - NAMES_GAP,
    }
}

/// Paint the "+2" of what folded.
pub(super) fn paint_more(ui: &egui::Ui, p: &Palette, cell: Rect, hidden: usize) {
    let g = ui.painter().layout_job(egui::text::LayoutJob::single_section(
        format!("+{hidden}"),
        semibold(ui, 12.0, p.muted),
    ));
    crate::ui::widgets::paint_ink_centered(ui.painter(), g, cell.center(), p.muted);
}

/// Width of the pill of `n` quick actions.
pub(super) fn actions_width(n: usize) -> f32 {
    n as f32 * (BUTTON.x + ACTIONS_PAD) + ACTIONS_PAD
}

/// Width of a single strip for the chosen button width.
pub(super) fn actions_width_for(count: usize, button_width: f32) -> f32 {
    count as f32 * (button_width + ACTIONS_PAD) + ACTIONS_PAD
}

/// A single horizontal action strip inside the row, without changing its height.
pub(super) fn actions_slot(row: Rect, left: f32, right: f32, count: usize, button: Vec2) -> Rect {
    let width = count as f32 * (button.x + ACTIONS_PAD) + ACTIONS_PAD;
    Rect::from_min_max(Pos2::new((right - width).max(left), row.top()), Pos2::new(right, row.bottom()))
}

/// Keep every button on one line, tightening its hit area only
/// when the usual size does not fit. Icons keep their usual size.
pub(super) fn action_rects(slot: Rect, count: usize, button: Vec2) -> (Rect, Vec<Rect>) {
    let count_f = count.max(1) as f32;
    let button = Vec2::new(
        button.x.min((slot.width() - ACTIONS_PAD * (count_f + 1.0)) / count_f).max(0.0),
        button.y.min(slot.height() - 2.0 * ACTIONS_PAD).max(0.0),
    );
    let size = Vec2::new(count as f32 * (button.x + ACTIONS_PAD) + ACTIONS_PAD, button.y + 2.0 * ACTIONS_PAD);
    let pill = Rect::from_min_size(Pos2::new(slot.right() - size.x, slot.center().y - size.y / 2.0), size);
    let cells = (0..count)
        .map(|i| {
            Rect::from_min_size(
                Pos2::new(
                    pill.left() + ACTIONS_PAD + i as f32 * (button.x + ACTIONS_PAD),
                    pill.top() + ACTIONS_PAD,
                ),
                button,
            )
        })
        .collect();
    (pill, cells)
}

/// The quick actions of the row under the pointer, in their pill at the
/// middle of `slot`: one response per button (`buttons`: icon, tip).
pub(super) fn actions(
    ui: &mut egui::Ui,
    p: &Palette,
    slot: Rect,
    buttons: &[(&str, &str)],
    salt: impl std::hash::Hash + std::fmt::Debug,
) -> Vec<egui::Response> {
    let (pill, cells) = action_rects(slot, buttons.len(), BUTTON);
    ui.painter()
        .rect_filled(pill, ui.visuals().widgets.inactive.corner_radius, p.raised);
    let id = ui.scope_id().with(salt);
    buttons
        .iter()
        .zip(cells)
        .enumerate()
        .map(|(i, ((icon, tip), r))| {
            let resp = ui.interact(r, id.with(i), Sense::click());
            if resp.hovered() {
                ui.painter().rect_filled(r, widgets::inner_radius(ui.visuals()), p.bar);
            }
            super::radar::icon(ui, icon, r, 14.0, p.ink);
            resp.on_hover_text(*tip).on_hover_cursor(egui::CursorIcon::PointingHand)
        })
        .collect()
}

/// A deliberate hold opens contact settings; ordinary clicks keep the
/// Legacy list's selection and double-click actions (LLPanelPeople).
const SHEET_HOLD_SECONDS: f64 = 0.5;

#[derive(Clone, Copy)]
struct SheetPress {
    started: f64,
    opened: bool,
}

#[derive(Default)]
pub(super) struct RowAction {
    pub sheet: bool,
    pub activate: bool,
}

/// Ctrl toggles, Shift extends, right click selects for the menu. A hold
/// belongs to this response only: moving, leaving the row or using a child
/// button cancels it. Its release cannot also start an IM or a drag.
pub(super) fn select(
    ui: &egui::Ui,
    resp: &egui::Response,
    sel: &mut Vec<Uuid>,
    anchor: &mut Option<Uuid>,
    order: &[Uuid],
    id: Uuid,
) -> RowAction {
    let (now, pressed, down, secondary, mods) = ui.input(|i| {
        (
            i.time,
            i.pointer.primary_pressed(),
            i.pointer.primary_down(),
            i.pointer.secondary_down(),
            i.modifiers,
        )
    });
    let key = resp.id.with(("contact-sheet-press", id));
    let mut press = ui.ctx().data_mut(|d| d.get_temp::<SheetPress>(key));
    if pressed {
        // egui can give a child the click and its row the potential drag.
        // Being down on the row therefore does not establish click ownership.
        let hovered = ui.ctx().interaction_snapshot(|i| i.hovered.clone());
        let child_clicked = hovered
            .iter()
            .any(|other| *other != resp.id && ui.ctx().read_response(*other).is_some_and(|r| r.sense.senses_click()));
        press = (resp.is_pointer_button_down_on() && !child_clicked && !mods.command && !mods.shift && !secondary).then_some(SheetPress {
            started: now,
            opened: false,
        });
    }
    let consumed = press.is_some_and(|p| p.opened);
    let mut action = RowAction::default();
    if !consumed {
        if !down || secondary || mods.command || mods.shift || !resp.contains_pointer() || resp.dragged() {
            press = None;
        } else if let Some(p) = &mut press {
            let remaining = SHEET_HOLD_SECONDS - (now - p.started);
            if remaining <= 0.0 {
                p.opened = true;
                action.sheet = true;
                *sel = vec![id];
                *anchor = Some(id);
            } else {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(remaining));
            }
        }
    }
    let consumed = consumed || action.sheet;
    ui.ctx().data_mut(|d| {
        d.remove::<SheetPress>(key);
        if down && let Some(press) = press {
            d.insert_temp(key, press);
        }
    });
    if consumed {
        return action;
    }
    resp.dnd_set_drag_payload(crate::ui::call_invite::PersonDrag(id));
    if resp.secondary_clicked() && !sel.contains(&id) {
        *sel = vec![id];
        *anchor = Some(id);
    }
    if resp.clicked() {
        crate::ui::contacts::click_select(sel, anchor, order, id, mods);
    }
    // egui counts clicks across widgets, so opening Contacts before a
    // double click on its first row can count as three clicks.
    action.activate = resp.double_clicked() || resp.triple_clicked();
    action
}

pub(super) fn contact_tip(resp: egui::Response, detail: Option<&str>) {
    let hint = aurora_i18n::tr!("people-edit-contact-hold");
    resp.on_hover_text(detail.map_or_else(|| hint.to_owned(), |detail| format!("{detail}\n{hint}")));
}

/// The right-click menu of a resident's row: the menu of several when the
/// row is part of a selection of several (menu_fs_radar_multiselect.xml),
/// else the resident's own.
pub(super) fn person_menu(resp: &egui::Response, p: &Palette, world: &World, sel: &[Uuid], id: Uuid) {
    let several = sel.len() > 1 && sel.contains(&id);
    let kind = if world.social.is_friend(&id) {
        AvatarList::Friend
    } else {
        AvatarList::Nearby
    };
    menu::context_menu(resp, p, |ui| {
        if several {
            context::avatars_menu(ui, p, world, sel);
        } else {
            context::avatar_list_menu(ui, p, world, id, kind);
        }
    });
}

/// The gear of a list's options, as tall as its toolbar.
pub(super) fn gear(ui: &mut egui::Ui, p: &Palette, tip: &str) -> egui::Response {
    let gear = match crate::ui::icons::global("settings-filled") {
        Some(t) => egui::Button::image(egui::Image::from_texture(egui::load::SizedTexture::new(t.id(), Vec2::splat(14.0))).tint(p.ink)),
        None => egui::Button::new(""),
    };
    ui.add(gear.min_size(Vec2::splat(TOOLBAR_H - 2.0))).on_hover_text(tip)
}

/// A line of muted text where a list is empty.
pub(super) fn empty(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(8.0);
    ui.label(egui::RichText::new(text).size(12.0).color(p.muted));
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PressHarness {
        frames: crate::ui::widgets::elide_tests::Frames,
        sel: Vec<Uuid>,
        anchor: Option<Uuid>,
        sheets: usize,
        activations: usize,
        rects: Vec<Rect>,
        modifiers: egui::Modifiers,
    }

    impl PressHarness {
        fn frame(&mut self, events: Vec<egui::Event>) {
            let order = [Uuid::from_u128(1), Uuid::from_u128(2)];
            self.rects.clear();
            self.frames.run(events, |ui| {
                ui.input_mut(|i| i.modifiers = self.modifiers);
                for id in order {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(300.0, RICH_H), Sense::click_and_drag());
                    self.rects.push(rect);
                    let child = Rect::from_min_max(Pos2::new(rect.right() - 30.0, rect.top()), rect.max);
                    ui.interact(child, resp.id.with("quick-action"), Sense::click());
                    let action = select(ui, &resp, &mut self.sel, &mut self.anchor, &order, id);
                    self.sheets += usize::from(action.sheet);
                    self.activations += usize::from(action.activate);
                }
            });
        }

        fn new() -> Self {
            let mut h = Self {
                frames: crate::ui::widgets::elide_tests::Frames::new(),
                sel: Vec::new(),
                anchor: None,
                sheets: 0,
                activations: 0,
                rects: Vec::new(),
                modifiers: egui::Modifiers::NONE,
            };
            h.frame(Vec::new());
            h.frame(Vec::new());
            h
        }

        fn button(&mut self, at: Pos2, button: egui::PointerButton, pressed: bool) {
            self.frame(vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button,
                    pressed,
                    modifiers: self.modifiers,
                },
            ]);
        }

        fn wait(&mut self, frames: usize) {
            for _ in 0..frames {
                self.frame(Vec::new());
            }
        }
    }

    #[test]
    fn contact_hold_opens_once_and_consumes_release_and_drag() {
        let mut h = PressHarness::new();
        let at = h.rects[0].center();
        h.button(at, egui::PointerButton::Primary, true);
        h.wait(20);
        assert_eq!(h.sheets, 0, "a short press does not open settings");
        h.wait(15);
        assert_eq!(h.sheets, 1, "settings open while the button is still down");
        assert_eq!(h.sel, vec![Uuid::from_u128(1)]);
        h.wait(60);
        assert_eq!(h.sheets, 1, "the same hold cannot reopen settings");
        assert!(!egui::DragAndDrop::has_any_payload(&h.frames.ctx), "a held sheet is not a drag");
        h.button(at, egui::PointerButton::Primary, false);
        assert_eq!(h.activations, 0, "the release cannot start an IM");
    }

    #[test]
    fn contact_hold_cancels_for_drag_child_button_and_right_click() {
        for kind in ["drag", "leave", "child", "right", "short"] {
            let mut h = PressHarness::new();
            let mut at = h.rects[0].center();
            if kind == "child" {
                at.x = h.rects[0].right() - 15.0;
            }
            let button = if kind == "right" {
                egui::PointerButton::Secondary
            } else {
                egui::PointerButton::Primary
            };
            h.button(at, button, true);
            if kind == "short" {
                h.button(at, button, false);
            } else if kind == "drag" || kind == "leave" {
                let to = if kind == "drag" {
                    at + Vec2::new(15.0, 0.0)
                } else {
                    h.rects[1].center()
                };
                h.frame(vec![egui::Event::PointerMoved(to)]);
                assert!(egui::DragAndDrop::has_payload_of_type::<crate::ui::call_invite::PersonDrag>(
                    &h.frames.ctx
                ));
                h.frame(vec![egui::Event::PointerMoved(at)]);
            }
            h.wait(40);
            assert_eq!(h.sheets, 0, "{kind} cannot open settings");
            h.button(at, button, false);
        }
    }

    #[test]
    fn contact_hold_preserves_modified_selection() {
        let mut h = PressHarness::new();
        h.sel = vec![Uuid::from_u128(1)];
        h.anchor = Some(Uuid::from_u128(1));
        h.modifiers = egui::Modifiers::CTRL | egui::Modifiers::COMMAND;
        let at = h.rects[1].center();
        h.button(at, egui::PointerButton::Primary, true);
        h.wait(35);
        assert_eq!(h.sheets, 0);
        h.button(at, egui::PointerButton::Primary, false);
        assert_eq!(h.sel, vec![Uuid::from_u128(1), Uuid::from_u128(2)]);
        h.modifiers = egui::Modifiers::SHIFT;
        let at = h.rects[0].center();
        h.button(at, egui::PointerButton::Primary, true);
        h.wait(35);
        h.button(at, egui::PointerButton::Primary, false);
        assert_eq!(h.sheets, 0);
        assert_eq!(h.sel.len(), 2, "Shift extends the selection");
    }

    /// From the narrowest list (a floater is 220 points wide at least) to a
    /// wide one, with up to five parts of any width, folded or not: nothing
    /// of the right end overlaps, it stays in the row, the names keep their
    /// least width whenever a part shows, and every part is shown or counted.
    #[test]
    fn the_right_end_never_covers_anything() {
        let overlap = |a: &Rect, b: &Rect| a.left() < b.right() - 0.01 && b.left() < a.right() - 0.01;
        let sets: [&[f32]; 5] = [
            &[],
            &[13.0],
            &[40.0, 60.0],
            &[44.0, 52.0, 52.0, 70.0],
            &[88.0, 13.0, 13.0, 13.0, 116.0],
        ];
        for widths in sets {
            for fold in [false, true] {
                for width in (160..=640).step_by(4) {
                    let row = Rect::from_min_size(Pos2::ZERO, Vec2::new(width as f32, RICH_H));
                    let left = names_left(row);
                    let e = end(row, left, widths, 5.0, fold);
                    let mut parts = e.items.clone();
                    parts.extend(e.more.map(|(r, _)| r));
                    for (i, a) in parts.iter().enumerate() {
                        assert!(a.left() >= row.left() && a.right() <= row.right(), "{width}: {a:?} leaves the row");
                        for b in &parts[i + 1..] {
                            assert!(!overlap(a, b), "{width} {widths:?}: {a:?} over {b:?}");
                        }
                    }
                    for (r, w) in e.items.iter().zip(widths) {
                        assert!((r.width() - w).abs() < 0.01, "each part keeps its width");
                    }
                    if !parts.is_empty() {
                        assert!(e.names_right - left >= NAMES_MIN - 0.01, "{width}: the names keep their room");
                        assert!(parts.iter().all(|r| r.left() >= e.names_right), "the names stop before the parts");
                    }
                    let counted = e.items.len() + e.more.map_or(0, |(_, n)| n);
                    if fold && e.more.is_some() {
                        assert_eq!(counted, widths.len());
                    }
                    if !fold {
                        assert!(e.more.is_none());
                    }
                }
                // a usual list shows everything
                let row = Rect::from_min_size(Pos2::ZERO, Vec2::new(500.0, RICH_H));
                assert_eq!(end(row, names_left(row), widths, 5.0, fold).items.len(), widths.len());
            }
        }
    }

    #[test]
    fn group_initials() {
        assert_eq!(initials("Aurora Builders"), "AB");
        assert_eq!(initials("Loups du Nord"), "LN");
        assert_eq!(initials("Marché de la Place"), "MP");
        assert_eq!(initials("club des lucioles"), "CD");
        assert_eq!(initials("  marché"), "M");
        assert_eq!(initials(""), "");
    }
}
