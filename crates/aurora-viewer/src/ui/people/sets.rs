//! People › Contact sets in Aurora's look: the contact sets of
//! Conversations › Contacts (FSPanelContactSets, fspanelcontactsets.cpp,
//! panel_fs_contacts_sets.xml, LGGContactSets) laid out as a list of sets
//! beside their members. The sets come in their priority order, each with
//! its tile, its members and how many are online, and the cog of its editor
//! (FSFloaterContactSetConfiguration); the pseudo-sets of the Legacy
//! Viewer's combo (all, no set, with a nickname, non-friends) are the smart
//! lists under them. A window too narrow for two columns, or one with a
//! sheet open beside, shows the sets in a picker above the members instead.
//! The members are People's rows, with chips for their other sets, online
//! first when the set sorts by status (FSPanelContactSets::
//! updateAvatarListSorting); the people chosen with a
//! click, Ctrl or Shift get a bar of actions (add to a set, move, nickname,
//! remove). Non-friends stay in their sets. Nicknames are in the sheet too.
//!
//! Visible differences from the Legacy Viewer, all Aurora's own: the sets
//! are listed by priority rather than by name (the Legacy Viewer has no
//! priority) and dragged to reorder it; Quick sort; the selection bar.

mod sidebar;

use super::rows::{self, RICH_GAP, RICH_H};
use super::{Ctx, PeopleUi};
use crate::theme::Palette;
use crate::ui::contacts::SetEdit;
use crate::ui::context::{self, AvatarList};
use crate::ui::{menu, widgets};
use crate::world::World;
use crate::world::contact_sets::{self, ContactSets};
use egui::{Color32, FontId, Pos2, Rect, Sense, Vec2};
use std::collections::HashSet;
use uuid::Uuid;

/// The width from which the list of sets gets a column of its own.
const SIDEBAR_MIN: f32 = 470.0;
/// Below this width the pane's buttons drop their labels.
const LABELS_MIN: f32 = 640.0;
/// Height of the selection bar.
const BAR_H: f32 = 40.0;
/// A chip of a member's other set.
const CHIP_H: f32 = 16.0;

/// The contact sets tab: the list, and the sheet of the person clicked.
pub(super) fn tab(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi) {
    super::split(ui, c, data, st, "people-sets-list", list, super::person_sheet);
}

/// The set shown: the one chosen while it exists, else the highest set,
/// else all of them (FSPanelContactSets::refreshSetList, in Aurora's order).
fn choice(c: &mut Ctx) -> String {
    let (shown, store) = shown_set(c.contacts.set_choice.as_deref(), &c.world.contact_sets);
    if store {
        c.contacts.set_choice = Some(shown.clone());
    }
    shown
}

/// The set or smart list shown for the choice shared with the classic
/// Contacts window, and whether to store it there. A pseudo-set this tab
/// does not list (Non-friends) falls back here, but stays the classic
/// window's choice.
fn shown_set(chosen: Option<&str>, sets: &ContactSets) -> (String, bool) {
    let fallback = || {
        sets.ordered_set_names()
            .into_iter()
            .next()
            .unwrap_or_else(|| contact_sets::ALL_SETS.to_owned())
    };
    match chosen {
        Some(s) if sidebar::is_smart(s) || sets.is_valid(s) => (s.to_owned(), false),
        Some(s) if ContactSets::is_internal(s) => (fallback(), false),
        _ => (fallback(), true),
    }
}

/// The people of a set or smart list, and how many are online.
pub(super) struct Members {
    pub ids: Vec<Uuid>,
    pub online: usize,
}

/// The residents shown as online: friends seen online (the status of the
/// others is not known). Looked up once per list: every row of the sets'
/// column counts its members each frame, and a scan of the friends per
/// member would grow with members × friends.
fn online_friends(world: &World) -> HashSet<Uuid> {
    world.social.friends.iter().filter(|f| f.online).map(|f| f.id).collect()
}

/// The members of `set` (generateAvatarList) and how many are online.
pub(super) fn members(world: &World, set: &str) -> Members {
    let friends: HashSet<Uuid> = world.social.friends.iter().map(|f| f.id).collect();
    let list: Vec<Uuid> = world.social.friends.iter().map(|f| f.id).collect();
    let ids = world.contact_sets.members(set, &list, |id| friends.contains(id));
    let on = online_friends(world);
    let online = ids.iter().filter(|id| on.contains(id)).count();
    Members { ids, online }
}

/// "4 people · 2 online" under a set's name.
pub(super) fn count_line(m: &Members) -> String {
    let people = aurora_i18n::tr!("people-sets-people", count = m.ids.len());
    if m.online == 0 {
        people
    } else {
        format!("{people} · {}", aurora_i18n::tr!("people-sets-online", count = m.online))
    }
}

fn list(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi, beside_sheet: bool) {
    let set = choice(c);
    let full = ui.available_rect_before_wrap();
    if beside_sheet || full.width() < SIDEBAR_MIN {
        let right = if beside_sheet { full.right() - 10.0 } else { full.right() };
        let rect = Rect::from_min_max(full.min, Pos2::new(right, full.bottom()));
        let mut pane = super::nearby::list_ui(ui, rect, "people-sets-pane", true);
        sidebar::picker(&mut pane, c, st, &set);
        members_pane(&mut pane, c, data, st, &set, true);
    } else {
        let side_w = (full.width() * 0.34).clamp(178.0, 232.0).round();
        let side = Rect::from_min_max(full.min, Pos2::new(full.left() + side_w, full.bottom()));
        let pane = Rect::from_min_max(Pos2::new(side.right() + 10.0, full.top()), full.max);
        ui.painter().rect_filled(side, 0.0, c.p.bar);
        let mut side_ui = super::nearby::list_ui(ui, side.shrink2(Vec2::new(6.0, 10.0)), "people-sets-side", true);
        sidebar::column(&mut side_ui, c, st, &set);
        let mut pane_ui = super::nearby::list_ui(ui, pane, "people-sets-pane", true);
        members_pane(&mut pane_ui, c, data, st, &set, full.width() < LABELS_MIN);
    }
    ui.allocate_rect(full, Sense::hover());
}

/// The members of `set`: the toolbar, the rows, and the selection's bar.
fn members_pane(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi, set: &str, compact: bool) {
    let p = c.p;
    let world = c.world;
    let mutable = !ContactSets::is_internal(set);
    ui.spacing_mut().item_spacing.y = 0.0;
    ui.add_space(10.0);
    let all = members(world, set);
    // the friends in no set (generateAvatarList's NO_SETS lists friends only)
    let quick_sort = set == contact_sets::NO_SETS && !all.ids.is_empty();
    toolbar(ui, c, st, set, quick_sort, mutable, compact);
    ui.add_space(4.0);
    let filter = st.filter.trim().to_lowercase();
    let mut ids = all.ids.clone();
    ids.retain(|id| super::matches_name(world, id, &rows::display_name(world, id), &filter));
    let online = online_friends(world);
    if st.sets_online_only {
        ids.retain(|id| online.contains(id));
    }
    let by_status = world.contact_sets.sorts_by_online(set);
    // FSPanelContactSets::updateAvatarListSorting
    ids.sort_by_cached_key(|id| {
        (
            by_status && !online.contains(id),
            crate::natural_order::key(&rows::display_name(world, id)),
        )
    });
    st.sel.retain(|id| ids.contains(id));
    let bar = !st.sel.is_empty();
    let rows_h = (ui.available_height() - if bar { BAR_H + 10.0 } else { 4.0 }).max(0.0);
    let area = egui::ScrollArea::vertical()
        .id_salt("people-sets-rows")
        .auto_shrink([false, false])
        .max_height(rows_h)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = RICH_GAP;
            if ids.is_empty() {
                let text = if all.ids.is_empty() {
                    aurora_i18n::tr!("ui-contacts-this-contact-set-is-empty")
                } else {
                    aurora_i18n::tr!("people-sets-no-match")
                };
                rows::empty(ui, p, text);
            }
            for id in &ids {
                row(ui, c, data, st, set, *id, &rows::display_name(world, id), &ids);
            }
        });
    carried_card(ui, c, data, st, set);
    let drop = ui.interact(area.inner_rect, ui.scope_id().with("contact_set_drop"), Sense::hover());
    crate::ui::contacts::drag::set_target_with(&drop, set, &st.sel);
    if bar {
        ui.add_space(6.0);
        selection_bar(ui, c, st, set, compact);
    }
}

/// A button of the pane: its label when there is room, else its icon
/// alone with the label as its hint.
fn pane_button(
    ui: &mut egui::Ui,
    p: &Palette,
    icon: &str,
    label: &str,
    kind: widgets::ActionKind,
    compact: bool,
    enabled: bool,
) -> egui::Response {
    let (text, w) = if compact { ("", rows::TOOLBAR_H) } else { (label, 0.0) };
    let resp = widgets::action_button_if(ui, p, icon, text, kind, Vec2::new(w, rows::TOOLBAR_H), enabled);
    if compact { resp.on_hover_text(label) } else { resp }
}

/// Search, all or online only, Quick sort for the friends in no set, and
/// "Add people" (residents chosen in the picker).
fn toolbar(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, set: &str, quick_sort: bool, mutable: bool, compact: bool) {
    let p = c.p;
    let line = Vec2::new(ui.available_width(), rows::TOOLBAR_H);
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let add = pane_button(
            ui,
            p,
            "user-plus",
            aurora_i18n::tr!("people-sets-add-people"),
            widgets::ActionKind::Flat,
            compact,
            mutable,
        );
        if add.clicked() {
            c.contacts.pick_for_set(set);
        }
        if quick_sort {
            let sort = pane_button(
                ui,
                p,
                "sparkles-filled",
                aurora_i18n::tr!("people-sets-quick-sort"),
                widgets::ActionKind::Primary,
                compact,
                true,
            )
            .on_hover_text(aurora_i18n::tr!("people-sets-quick-sort-tip"));
            if sort.clicked() {
                c.contacts.open_quick_sort(c.world);
            }
        }
        let mut which = usize::from(st.sets_online_only);
        let (all, online) = (aurora_i18n::tr!("common-all"), aurora_i18n::tr!("people-sets-online-only"));
        if widgets::segmented_tall(ui, p, &mut which, &[(all, false), (online, false)], rows::TOOLBAR_H) {
            st.sets_online_only = which == 1;
        }
        let w = ui.available_width();
        widgets::clearable_search_box(ui, p, &mut st.filter, aurora_i18n::tr!("ui-contacts-filter-people"), w);
    });
}

/// The second line of a member: their legacy name (the friend's heart
/// beside the name already tells friends apart).
fn second_line(world: &World, id: &Uuid) -> String {
    world
        .social
        .avatar_names
        .get(id)
        .map(|n| crate::world::social::format_name(&n.legacy_first, &n.legacy_last))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| world.social.name_of(id))
}

/// The member's other sets, highest first: (name, colour).
fn other_sets(world: &World, id: &Uuid, set: &str) -> Vec<(String, Color32)> {
    world
        .contact_sets
        .ordered_set_names()
        .into_iter()
        .filter_map(|s| {
            let cs = world.contact_sets.set(&s)?;
            (s != set && cs.friends.contains(id)).then(|| {
                let color = crate::interface_colors::on_panel(crate::ui::colors::set_color(cs.color));
                (s, color)
            })
        })
        .collect()
}

/// A chip's text and width.
fn chip_layout(ui: &egui::Ui, p: &Palette, name: &str) -> (std::sync::Arc<egui::Galley>, f32) {
    let g = ui.painter().layout_no_wrap(name.to_owned(), FontId::proportional(10.5), p.ink);
    let w = 16.0 + g.size().x + 6.0;
    (g, w)
}

fn paint_chip(ui: &egui::Ui, p: &Palette, rect: Rect, g: std::sync::Arc<egui::Galley>, color: Color32) {
    ui.painter()
        .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised.gamma_multiply(0.55));
    ui.painter()
        .circle_filled(Pos2::new(rect.left() + 9.0, rect.center().y), 3.0, color);
    widgets::paint_row_text(ui.painter(), g, rect.left() + 16.0, rect.center().y, p.ink);
}

/// The chips of `chips` that fit in `width` beside a second line kept 40
/// points wide at least, in their order.
fn fitting_chips(widths: &[f32], width: f32) -> usize {
    let mut room = width - 40.0;
    let mut n = 0;
    for w in widths {
        if w + 4.0 > room {
            break;
        }
        room -= w + 4.0;
        n += 1;
    }
    n
}

/// A member's two lines: the name (the friend's heart and the staff tag
/// after it), then their second line and the chips of their other sets,
/// as many as fit. Returns whether the name was cut.
#[allow(clippy::too_many_arguments)]
fn member_names(
    ui: &egui::Ui,
    p: &Palette,
    rect: Rect,
    left: f32,
    right: f32,
    name: &str,
    color: Color32,
    second: &str,
    chips: &[(String, Color32)],
    friend: bool,
    mark: Option<crate::world::staff::Mark>,
) -> bool {
    let heart = friend.then_some(rows::SetMark::Icon("heart-filled", p.success));
    // the first line alone, in the top half of the row
    let first = Rect::from_center_size(Pos2::new(rect.center().x, rect.center().y - 7.0), Vec2::new(rect.width(), 18.0));
    let cut = rows::person_names(ui, p, first, left, right, name, color, "", heart, None, mark);
    let y = rect.center().y + 8.0;
    let width = (right - left).max(0.0);
    let layouts: Vec<_> = chips.iter().map(|(n, c)| (chip_layout(ui, p, n), *c)).collect();
    let n = fitting_chips(&layouts.iter().map(|((_, w), _)| *w).collect::<Vec<_>>(), width);
    let chips_w: f32 = layouts.iter().take(n).map(|((_, w), _)| w + 4.0).sum();
    let mut job = egui::text::LayoutJob::single_section(second.to_owned(), egui::TextFormat::simple(FontId::proportional(11.5), p.muted));
    job.wrap = egui::text::TextWrapping::truncate_at_width((width - chips_w - 2.0).max(0.0));
    let g = ui.painter().layout_job(job);
    let mut x = left + g.size().x + if second.is_empty() { 0.0 } else { 6.0 };
    widgets::paint_row_text(ui.painter(), g, left, y, p.muted);
    for ((g, w), c) in layouts.into_iter().take(n) {
        let chip = Rect::from_min_size(Pos2::new(x, y - CHIP_H / 2.0), Vec2::new(w, CHIP_H));
        paint_chip(ui, p, chip, g, c);
        x += w + 4.0;
    }
    cut
}

/// One member: picture and status, names and chips, and the quick actions
/// under the pointer (take out of the set, the menu; moving is the
/// selection bar's).
#[allow(clippy::too_many_arguments)]
fn row(ui: &mut egui::Ui, c: &mut Ctx, data: &super::RadarData, st: &mut PeopleUi, set: &str, id: Uuid, name: &str, order: &[Uuid]) {
    let p = c.p;
    let world = c.world;
    let (rect, resp) = rows::allocate(ui, id, Vec2::new(ui.available_width(), RICH_H));
    widgets::reveal_search_selection(
        ui,
        &resp,
        st.anchor
            .filter(|id| st.sel.contains(id))
            .or_else(|| st.sel.last().copied())
            .or(st.sheet)
            == Some(id),
    );
    if !ui.is_rect_visible(rect) {
        rows::person_menu(&resp, p, world, &st.sel, id);
        return;
    }
    if !data.pics.contains_key(&id) {
        st.want_pic(id, ui.input(|i| i.time));
    }
    let selected = st.sel.contains(&id) || st.sheet == Some(id);
    let hot = (ui.rect_contains_pointer(rect) && !resp.dragged()) || st.row_menu == Some(id) || st.demo_hover == Some(id);
    rows::face(ui, p, rect, selected, hot);
    let left = rows::picture(ui, p, world, rect, data.pics.get(&id), id, name, false);
    let mutable = !ContactSets::is_internal(set);
    // under the pointer: take out, the menu
    let buttons: Option<Vec<(&str, &str)>> = hot.then(|| {
        let mut b = Vec::new();
        if mutable {
            b.push((
                "circle-minus",
                aurora_i18n::tr!("ui-contacts-remove-this-resident-from-the-selected-contact-set"),
            ));
        }
        b.push(("dots-filled", aurora_i18n::tr!("ui-chat-more")));
        b
    });
    let widths: Vec<f32> = match &buttons {
        Some(b) => vec![rows::actions_width(b.len())],
        None => Vec::new(),
    };
    let end = rows::end(rect, left, &widths, 8.0, false);
    if let (Some(buttons), Some(slot)) = (&buttons, end.items.first()) {
        hover_actions(ui, c, st, set, id, *slot, buttons);
    }
    let color = crate::ui::colors::resident_name(world, &id, p.ink);
    let chips = other_sets(world, &id, set);
    let cut = member_names(
        ui,
        p,
        rect,
        left,
        end.names_right,
        name,
        color,
        &second_line(world, &id),
        &chips,
        world.social.is_friend(&id),
        crate::world::staff::mark(world, &id),
    );
    let action = rows::select(ui, &resp, &mut st.sel, &mut st.anchor, order, id);
    // dragged toward a set: the row is carried as a card (`carried_card`)
    if resp.drag_started()
        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
    {
        let carry = MemberCarry {
            id,
            origin,
            grab: origin - rect.min,
            width: rect.width(),
        };
        ui.ctx().data_mut(|d| d.insert_temp(member_carry_key(), carry));
    }
    // the rows being carried wait, dimmed, in their places
    if let Some(drag) = egui::DragAndDrop::payload::<crate::ui::call_invite::PersonDrag>(ui.ctx())
        && (drag.0 == id || (st.sel.contains(&drag.0) && st.sel.contains(&id)))
    {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.panel.gamma_multiply(0.6));
    }
    if action.sheet {
        super::nearby::open_sheet(st, id);
    }
    if action.activate {
        c.actions.push(super::PeopleAction::OpenIm(id));
    }
    rows::person_menu(&resp, p, world, &st.sel, id);
    let tip = (cut && resp.hover_pos().is_some_and(|at| at.x <= end.names_right)).then(|| name.to_owned());
    rows::contact_tip(resp, tip.as_deref());
}

/// How wide a member's two lines are when nothing is cut: the name with
/// its heart and staff tag, or the second line with every chip.
fn member_content_width(
    ui: &egui::Ui,
    p: &Palette,
    world: &World,
    id: &Uuid,
    name: &str,
    second: &str,
    chips: &[(String, Color32)],
) -> f32 {
    let name_w = ui
        .painter()
        .layout_no_wrap(name.to_owned(), rows::semibold(ui, 13.0, p.ink).font_id, p.ink)
        .size()
        .x;
    let heart = if world.social.is_friend(id) { 6.0 + 12.0 } else { 0.0 };
    let staff = crate::ui::staff::width(ui, p, crate::world::staff::mark(world, id), 13.0);
    let staff = if staff > 0.0 { staff + 6.0 } else { 0.0 };
    let second_w = ui
        .painter()
        .layout_no_wrap(second.to_owned(), FontId::proportional(11.5), p.muted)
        .size()
        .x;
    let chips_w: f32 = chips.iter().map(|(n, _)| chip_layout(ui, p, n).1 + 4.0).sum();
    // the second line keeps its 40 points beside the chips (`fitting_chips`)
    let second_line_w = second_w.max(40.0) + if chips.is_empty() { 0.0 } else { 6.0 + chips_w };
    (name_w + heart + staff).max(second_line_w) + 2.0
}

/// A member being dragged toward a set: who, where the pointer holds the
/// row (from its corner), and the row's width.
#[derive(Clone, Copy, Debug)]
struct MemberCarry {
    id: Uuid,
    /// Where the press began: the drag this card belongs to.
    origin: Pos2,
    grab: Vec2,
    width: f32,
}

fn member_carry_key() -> egui::Id {
    egui::Id::unique("people-set-member-carry")
}

/// The card of the drag under way, when it started in this tab (a drag of
/// the same resident from the Friends tab or the classic window has no card
/// here, nor this tab's selection).
fn carry_now(ctx: &egui::Context) -> Option<MemberCarry> {
    let drag = egui::DragAndDrop::payload::<crate::ui::call_invite::PersonDrag>(ctx)?;
    let carry: MemberCarry = ctx.data(|d| d.get_temp(member_carry_key()))?;
    let origin = ctx.input(|i| i.pointer.press_origin())?;
    (carry.id == drag.0 && carry.origin == origin).then_some(carry)
}

/// The people a drop on a set brings: this tab's chosen people when the
/// drag started here, else only the one dragged.
pub(super) fn dropped_with<'a>(ctx: &egui::Context, sel: &'a [Uuid]) -> &'a [Uuid] {
    if carry_now(ctx).is_some() { sel } else { &[] }
}

/// The member dragged, carried as a lifted card under the pointer, with
/// how many go along when the people chosen are dragged together.
fn carried_card(ui: &mut egui::Ui, c: &Ctx, data: &super::RadarData, st: &PeopleUi, set: &str) {
    let p = c.p;
    let world = c.world;
    let (Some(carry), Some(at)) = (carry_now(ui.ctx()), ui.input(|i| i.pointer.interact_pos())) else {
        return;
    };
    let id = carry.id;
    let name = rows::display_name(world, &id);
    let second = second_line(world, &id);
    let chips = other_sets(world, &id, set);
    // as wide as what it shows, never wider than the row: the same lines,
    // nothing cut
    let left_pad = rows::names_left(Rect::from_min_size(Pos2::ZERO, Vec2::new(carry.width, RICH_H)));
    let width = (left_pad + member_content_width(ui, p, world, &id, &name, &second, &chips) + 12.0).min(carry.width);
    // held where it was grabbed, or by its right end when the card is
    // narrower than the place it was grabbed at
    let grab = Vec2::new(carry.grab.x.min(width - 12.0), carry.grab.y);
    let rect = Rect::from_min_size(at - grab, Vec2::new(width, RICH_H));
    let layer = egui::LayerId::new(egui::Order::Tooltip, member_carry_key());
    let mut card = ui.new_child(egui::UiBuilder::new().layer_id(layer).max_rect(rect));
    card.set_clip_rect(ui.ctx().content_rect());
    let radius = card.visuals().widgets.inactive.corner_radius;
    card.painter()
        .rect_filled(rect.translate(Vec2::new(0.0, 5.0)).expand(2.0), radius, p.navy.gamma_multiply(0.45));
    card.painter().rect_filled(rect, radius, p.raised);
    card.painter()
        .rect_stroke(rect, radius, egui::Stroke::new(1.5, p.violet), egui::StrokeKind::Inside);
    let left = rows::picture(&card, p, world, rect, data.pics.get(&id), id, &name, false);
    let color = crate::ui::colors::resident_name(world, &id, p.ink);
    member_names(
        &card,
        p,
        rect,
        left,
        rect.right() - 10.0,
        &name,
        color,
        &second,
        &chips,
        world.social.is_friend(&id),
        crate::world::staff::mark(world, &id),
    );
    // the others chosen go along: their number on the card's corner
    let along = if st.sel.contains(&id) { st.sel.len() } else { 1 };
    if along > 1 {
        let g = card.painter().layout_no_wrap(
            aurora_i18n::integer(along as i64),
            crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(card.ctx()), 11.5, p.on_accent).font_id,
            p.on_accent,
        );
        let w = (g.size().x + 12.0).max(20.0);
        let pill = Rect::from_center_size(Pos2::new(rect.right() - 4.0, rect.top() + 2.0), Vec2::new(w, 20.0));
        card.painter().rect_filled(pill, radius, p.violet);
        let gw = g.size().x;
        widgets::paint_row_text(card.painter(), g, pill.center().x - gw / 2.0, pill.center().y, p.on_accent);
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    ui.ctx().request_repaint();
}

/// The residents a row's action applies to: the selection when the row is
/// part of it, else the row alone.
fn targets(st: &PeopleUi, id: Uuid) -> Vec<Uuid> {
    if st.sel.contains(&id) { st.sel.clone() } else { vec![id] }
}

/// The sets of a "Move to" or "Add to set" menu, highest first, `from`
/// left out; `add` adds to the set chosen (and offers a new set), else
/// moves out of `from` into it.
fn sets_menu(ui: &mut egui::Ui, c: &mut Ctx, from: &str, ids: &[Uuid], add: bool) {
    let p = c.p;
    let others: Vec<String> = c.world.contact_sets.ordered_set_names().into_iter().filter(|s| s != from).collect();
    if others.is_empty() {
        menu::item_if(ui, p, "users-group", aurora_i18n::tr!("ui-contacts-no-other-contact-sets"), false);
    }
    for to in &others {
        if menu::item(ui, p, "users-group", to) {
            let edit = if add {
                SetEdit::Add {
                    set: to.clone(),
                    ids: ids.to_vec(),
                }
            } else {
                SetEdit::Move {
                    from: from.to_owned(),
                    to: to.clone(),
                    ids: ids.to_vec(),
                }
            };
            context::request(ui.ctx(), context::CtxAction::ContactSet(edit));
        }
    }
    if add {
        menu::separator(ui, p);
        if menu::item(ui, p, "plus", aurora_i18n::tr!("people-sets-new-set-with")) {
            c.contacts.ask_new_set_for(ids.to_vec());
        }
    }
}

fn hover_actions(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, set: &str, id: Uuid, slot: Rect, buttons: &[(&str, &str)]) {
    let p = c.p;
    let world = c.world;
    let mut menu_open = false;
    for ((icon, _), resp) in buttons.iter().zip(rows::actions(ui, p, slot, buttons, ("people-set-act", id))) {
        match *icon {
            "circle-minus" if resp.clicked() => c.contacts.ask_remove_from_set(set, targets(st, id)),
            "dots-filled" => {
                let kind = if world.social.is_friend(&id) {
                    AvatarList::Friend
                } else {
                    AvatarList::Name
                };
                menu_open |= menu::dropdown(&resp, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    context::avatar_list_menu(ui, p, world, id, kind);
                });
            }
            _ => {}
        }
    }
    super::keep_row_menu(st, id, menu_open);
}

/// The bar of the people chosen: how many, add them to a set, move them
/// out of this one, their nickname, take them out; the cross forgets the
/// choice. Labelled when there is room, icons with their hint otherwise.
fn selection_bar(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, set: &str, compact: bool) {
    let p = c.p;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), BAR_H), Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.bar);
    let mutable = !ContactSets::is_internal(set);
    let ids = st.sel.clone();
    let mut bar = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(5.0, 5.0)))
            .layout(egui::Layout::right_to_left(egui::Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = 4.0;
    if widgets::icon_button(&mut bar, p, "x", aurora_i18n::tr!("people-sets-clear-selection"), true) {
        st.sel.clear();
        st.anchor = None;
    }
    if mutable {
        let remove = pane_button(
            &mut bar,
            p,
            "circle-minus",
            aurora_i18n::tr!("people-sets-remove"),
            widgets::ActionKind::FlatDanger,
            compact,
            true,
        );
        if remove.clicked() {
            c.contacts.ask_remove_from_set(set, ids.clone());
        }
    }
    let nick = pane_button(
        &mut bar,
        p,
        "edit-filled",
        aurora_i18n::tr!("ui-contacts-nickname"),
        widgets::ActionKind::Flat,
        compact,
        true,
    );
    if nick.clicked() {
        c.contacts.ask_nickname(c.world, ids.clone());
    }
    if mutable {
        let to = pane_button(
            &mut bar,
            p,
            "arrows-exchange",
            aurora_i18n::tr!("people-sets-move-to"),
            widgets::ActionKind::Flat,
            compact,
            true,
        );
        menu::dropdown(&to, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
            sets_menu(ui, c, set, &ids, false)
        });
    }
    // the main action keeps its label
    let add = pane_button(
        &mut bar,
        p,
        "plus",
        aurora_i18n::tr!("people-sets-add-to-set"),
        widgets::ActionKind::Flat,
        false,
        true,
    );
    menu::dropdown(&add, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
        sets_menu(ui, c, set, &ids, true)
    });
    let text = aurora_i18n::tr!("people-sets-selected", count = ids.len());
    let g = bar.painter().layout_no_wrap(
        text,
        crate::ui::fonts::semibold_format(&crate::ui::fonts::semibold(bar.ctx()), 12.5, p.ink).font_id,
        p.ink,
    );
    if g.size().x + 8.0 <= bar.available_width() {
        widgets::paint_row_text(bar.painter(), g, rect.left() + 12.0, rect.center().y, p.ink);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The theme's segmented control: its lit face slides from the choice
    /// before to the new one over a few frames, then rests on it.
    /// A drop on a set brings this tab's chosen people only for a drag
    /// that started in this tab, not for the same resident dragged from
    /// elsewhere afterwards.
    #[test]
    fn a_drop_brings_the_chosen_only_for_a_drag_started_here() {
        let [a, b] = [1, 2].map(Uuid::from_u128);
        let sel = vec![a, b];
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let here = Pos2::new(40.0, 40.0);
        let press = |at: Pos2| {
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        let mut brought = Vec::new();
        frames.run(press(here), |ui| {
            egui::DragAndDrop::set_payload(ui.ctx(), crate::ui::call_invite::PersonDrag(a));
            let carry = MemberCarry {
                id: a,
                origin: here,
                grab: Vec2::ZERO,
                width: 100.0,
            };
            ui.ctx().data_mut(|d| d.insert_temp(member_carry_key(), carry));
            brought = dropped_with(ui.ctx(), &sel).to_vec();
        });
        assert_eq!(brought, sel, "dragged from this tab: the chosen come along");
        frames.run(
            vec![egui::Event::PointerButton {
                pos: here,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            |_| {},
        );
        // the same resident dragged from another window: the old card stays
        frames.run(press(Pos2::new(300.0, 200.0)), |ui| {
            egui::DragAndDrop::set_payload(ui.ctx(), crate::ui::call_invite::PersonDrag(a));
            brought = dropped_with(ui.ctx(), &sel).to_vec();
            assert!(carry_now(ui.ctx()).is_none(), "no card for a drag started elsewhere");
        });
        assert!(brought.is_empty(), "only the one dragged");
    }

    #[test]
    fn the_classic_windows_non_friends_choice_falls_back_here_and_stays_there() {
        let mut sets = ContactSets::default();
        assert_eq!(shown_set(None, &sets), (contact_sets::ALL_SETS.to_owned(), true));
        sets.add_set("Family");
        assert_eq!(shown_set(None, &sets), ("Family".to_owned(), true));
        assert_eq!(shown_set(Some("Gone"), &sets), ("Family".to_owned(), true), "a deleted set");
        assert_eq!(shown_set(Some("Family"), &sets), ("Family".to_owned(), false));
        assert_eq!(
            shown_set(Some(contact_sets::NO_SETS), &sets),
            (contact_sets::NO_SETS.to_owned(), false)
        );
        assert_eq!(
            shown_set(Some(contact_sets::EXTRA_AVS), &sets),
            ("Family".to_owned(), false),
            "shown as the first set here, left as it is for the classic window"
        );
    }

    #[test]
    fn a_segmented_choice_slides_its_face_to_the_new_choice() {
        let p = crate::theme::Theme::default().palette();
        let mut frames = crate::ui::widgets::elide_tests::Frames::new();
        let options = [("One", false), ("Two", false), ("Three", false)];
        let lit_left = |frames: &mut crate::ui::widgets::elide_tests::Frames, chosen: usize| {
            let shapes = frames.shapes(Vec::new(), |ui| {
                let mut i = chosen;
                widgets::segmented(ui, &p, &mut i, &options);
            });
            shapes
                .iter()
                .filter_map(|shape| match shape {
                    egui::Shape::Rect(r) if r.fill == p.raised => Some(r.rect.left()),
                    _ => None,
                })
                .next()
                .expect("the chosen segment is lit")
        };
        let first = lit_left(&mut frames, 0);
        let _ = lit_left(&mut frames, 0);
        // the frame of the click starts from where it was, the next moves
        assert_eq!(lit_left(&mut frames, 2), first);
        let moving = lit_left(&mut frames, 2);
        let mut last = moving;
        for _ in 0..60 {
            last = lit_left(&mut frames, 2);
        }
        assert!(moving > first && moving < last, "on its way: {first} < {moving} < {last}");
    }

    #[test]
    fn chips_fit_while_the_second_line_keeps_its_room() {
        // 40 kept for the second line, 4 between chips
        assert_eq!(fitting_chips(&[50.0, 50.0], 200.0), 2);
        assert_eq!(fitting_chips(&[50.0, 50.0], 120.0), 1);
        assert_eq!(fitting_chips(&[50.0, 50.0], 80.0), 0);
        // a chip that does not fit hides the ones after it, keeping the order
        assert_eq!(fitting_chips(&[50.0, 200.0, 20.0], 160.0), 1);
        assert_eq!(fitting_chips(&[], 300.0), 0);
    }

    fn world() -> World {
        World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()))
    }

    fn friend(world: &mut World, n: u128, online: bool) -> Uuid {
        let id = Uuid::from_u128(n);
        world.social.friends.push(crate::world::social::Friend {
            id,
            online,
            rights_given: 1,
            rights_has: 1,
        });
        id
    }

    #[test]
    fn a_sets_counts_are_its_members_and_those_online() {
        let mut world = world();
        let on = friend(&mut world, 1, true);
        let off = friend(&mut world, 2, false);
        let stranger = Uuid::from_u128(3);
        world.contact_sets.add_set("S");
        let social = &world.social;
        world.contact_sets.add_to_set(&[on, off, stranger], "S", |id| social.is_friend(id));
        let m = members(&world, "S");
        assert_eq!((m.ids.len(), m.online), (3, 1));
        assert_eq!(
            count_line(&m),
            format!(
                "{} · {}",
                aurora_i18n::tr!("people-sets-people", count = 3),
                aurora_i18n::tr!("people-sets-online", count = 1)
            )
        );
    }

    #[test]
    fn a_members_chips_are_their_other_sets_highest_first() {
        let mut world = world();
        let id = friend(&mut world, 1, true);
        for name in ["A", "B", "C"] {
            world.contact_sets.add_set(name);
            world.contact_sets.add_to_set(&[id], name, |_| true);
        }
        world.contact_sets.move_priority_to("C", 0);
        let names: Vec<String> = other_sets(&world, &id, "A").into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["C", "B"]);
        // a smart list shows every set
        let names: Vec<String> = other_sets(&world, &id, contact_sets::ALL_SETS)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, ["C", "A", "B"]);
    }
}
