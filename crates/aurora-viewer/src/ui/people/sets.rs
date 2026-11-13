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
use crate::world::contact_sets::{self, ContactSets, DN_REMOVED};
use egui::{Color32, FontId, Pos2, Rect, Sense, Vec2};
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
    let choice = match &c.contacts.set_choice {
        Some(s) if ContactSets::is_internal(s) || c.world.contact_sets.is_valid(s) => s.clone(),
        _ => c
            .world
            .contact_sets
            .ordered_set_names()
            .into_iter()
            .next()
            .unwrap_or_else(|| contact_sets::ALL_SETS.to_owned()),
    };
    c.contacts.set_choice = Some(choice.clone());
    choice
}

/// The people of a set or smart list, and how many are online.
pub(super) struct Members {
    pub ids: Vec<Uuid>,
    pub online: usize,
}

/// Whether a resident shows as online: a friend seen online (the status
/// of the others is not known).
fn is_online(world: &World, id: &Uuid) -> bool {
    world.social.friends.iter().any(|f| f.id == *id && f.online)
}

/// The members of `set` (generateAvatarList) and how many are online.
pub(super) fn members(world: &World, set: &str) -> Members {
    let friends: Vec<Uuid> = world.social.friends.iter().map(|f| f.id).collect();
    let ids = world.contact_sets.members(set, &friends, |id| world.social.is_friend(id));
    let online = ids.iter().filter(|id| is_online(world, id)).count();
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
    if st.sets_online_only {
        ids.retain(|id| is_online(world, id));
    }
    let by_status = world.contact_sets.sorts_by_online(set);
    // FSPanelContactSets::updateAvatarListSorting
    ids.sort_by_cached_key(|id| {
        (
            by_status && !is_online(world, id),
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
        if widgets::segmented(ui, p, &mut which, &[(all, false), (online, false)]) {
            st.sets_online_only = which == 1;
        }
        let w = ui.available_width();
        widgets::clearable_search_box(ui, p, &mut st.filter, aurora_i18n::tr!("ui-contacts-filter-people"), w);
    });
}

/// The second line of a member: their nickname's real name, "not a
/// friend", or their legacy name.
fn second_line(world: &World, id: &Uuid) -> String {
    let legacy = world
        .social
        .avatar_names
        .get(id)
        .map(|n| crate::world::social::format_name(&n.legacy_first, &n.legacy_last))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| world.social.name_of(id));
    if world.contact_sets.pseudonym(id).is_some_and(|a| a != DN_REMOVED) {
        aurora_i18n::tr!("people-sets-nickname-of", name = legacy)
    } else if !world.social.is_friend(id) {
        aurora_i18n::tr!("people-sets-non-friend").to_owned()
    } else {
        legacy
    }
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
