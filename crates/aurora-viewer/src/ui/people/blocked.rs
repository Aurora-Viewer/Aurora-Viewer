//! People › Blocked in Aurora's look (the People mock-up v2): the block
//! list (LLPanelBlockedList, panel_block_list_sidetray.xml, LLMuteList) as
//! rows, what is blocked of each in pills (everything, or voice, text,
//! particles, object sounds), each pill a click away from changing it as
//! the list's menu does (menu_fs_block_list.xml), "Unblock" on the row
//! under the pointer, and "Block by name" for an object
//! (LLFloaterGetBlockedObjectName). Ctrl / Shift choose several to unblock
//! at once (removeMutes).

use super::rows::{self, RICH_GAP, RICH_H};
use super::{Ctx, PeopleAction, PeopleUi};
use crate::ui::{menu, widgets};
use crate::world::mutes::{Mute, MuteType, flag};
use egui::{FontId, Pos2, Rect, Sense, Vec2};
use uuid::Uuid;

/// What can be blocked apart, in the pills' order.
const PARTS: [(u32, &str); 4] = [
    (flag::VOICE_CHAT, "people-blocked-voice"),
    (flag::TEXT_CHAT, "people-blocked-text"),
    (flag::PARTICLES, "common-particles"),
    (flag::OBJECT_SOUNDS, "people-blocked-sounds"),
];

/// A block list entry's key: its id, or its name for a legacy entry.
fn key(m: &Mute) -> (Uuid, String) {
    (m.id, m.name.clone())
}

/// A row's own id in the selection: the entry's id, or one made of its name.
fn row_id(m: &Mute) -> Uuid {
    if m.id.is_nil() {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        m.name.hash(&mut h);
        let h = h.finish();
        Uuid::from_u64_pair(h, !h)
    } else {
        m.id
    }
}

/// The pills of an entry: everything, or what of it is blocked (a set flag
/// means "not blocked", LLMute flags); a by-name entry blocks everything.
pub(super) fn pills(m: &Mute) -> Vec<(u32, &'static str)> {
    if m.kind == MuteType::ByName || m.flags & flag::ALL == 0 {
        return vec![(flag::ALL, aurora_i18n::tr!("people-blocked-all"))];
    }
    PARTS
        .iter()
        .filter(|(bit, _)| m.flags & bit == 0)
        .map(|(bit, key)| (*bit, aurora_i18n::text(key)))
        .collect()
}

pub(super) fn tab(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi) {
    use crate::world::mutes::LoadState;
    let p = c.p;
    let world = c.world;
    ui.spacing_mut().item_spacing.y = 0.0;
    let status = match world.mutes.state {
        LoadState::Initial | LoadState::Requested(_) => Some(aurora_i18n::tr!("ui-people-loading-block-list")),
        LoadState::Degraded => Some(aurora_i18n::tr!("ui-people-the-server-did-not-respond-using-the-local-list")),
        LoadState::Loaded => None,
    };
    if let Some(s) = status {
        ui.label(egui::RichText::new(s).size(11.5).color(p.muted));
        ui.add_space(4.0);
    }
    let line = Vec2::new(ui.available_width(), rows::TOOLBAR_H);
    ui.allocate_ui_with_layout(line, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let by_name = widgets::action_button(
            ui,
            p,
            "ban",
            aurora_i18n::tr!("ui-people-block-by-name"),
            widgets::ActionKind::Flat,
            Vec2::new(0.0, rows::TOOLBAR_H),
        );
        menu::dropdown(&by_name, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            ui.set_width(240.0);
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("ui-people-exact-object-name"))
                    .size(12.0)
                    .color(p.muted),
            );
            let r = ui.add(egui::TextEdit::singleline(&mut st.block_name).desired_width(f32::INFINITY));
            let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let ok = !st.block_name.trim().is_empty();
            ui.add_space(4.0);
            let block = widgets::action_button_if(
                ui,
                p,
                "ban",
                aurora_i18n::tr!("people-blocked-block"),
                widgets::ActionKind::Primary,
                Vec2::new(0.0, 26.0),
                ok,
            );
            if (block.clicked() || enter) && ok {
                c.actions.push(PeopleAction::BlockByName(st.block_name.trim().to_owned()));
                st.block_name.clear();
                ui.close();
            }
        });
        let w = ui.available_width();
        widgets::search_box(ui, p, &mut st.filter, aurora_i18n::tr!("people-filter-short"), w);
    });
    ui.add_space(7.0);
    let filter = st.filter.trim().to_lowercase();
    let mut entries: Vec<Mute> = world
        .mutes
        .entries()
        .into_iter()
        .filter(|m| filter.is_empty() || m.name.to_lowercase().contains(&filter))
        .collect();
    // by name, BlockPeopleSortOrder's default (LLBlockList::sortByName)
    entries.sort_by_cached_key(|m| m.name.to_lowercase());
    let order: Vec<Uuid> = entries.iter().map(row_id).collect();
    st.sel.retain(|id| order.contains(id));
    egui::ScrollArea::vertical()
        .id_salt("people-blocked-rows")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = RICH_GAP;
            if entries.is_empty() {
                rows::empty(ui, p, aurora_i18n::tr!("ui-people-nobody-is-blocked"));
            }
            for m in &entries {
                row(ui, c, st, m, &entries, &order);
            }
        });
}

/// The width of "Unblock" on a row.
fn unblock_width(ui: &egui::Ui) -> f32 {
    ui.painter()
        .layout_no_wrap(
            aurora_i18n::tr!("common-unblock").to_owned(),
            FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())),
            egui::Color32::PLACEHOLDER,
        )
        .size()
        .x
        + 20.0
}

fn row(ui: &mut egui::Ui, c: &mut Ctx, st: &mut PeopleUi, m: &Mute, all: &[Mute], order: &[Uuid]) {
    let p = c.p;
    let id = row_id(m);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), RICH_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let selected = st.sel.contains(&id);
    let hot = ui.rect_contains_pointer(rect) || st.row_menu == Some(id) || st.demo_hover == Some(id);
    rows::face(ui, p, rect, selected, hot);
    let name = if m.name.is_empty() {
        aurora_i18n::tr!("common-unnamed")
    } else {
        m.name.as_str()
    };
    let left = match m.kind {
        MuteType::Agent => {
            let disc = Rect::from_center_size(Pos2::new(rect.left() + 8.0 + 15.0, rect.center().y), Vec2::splat(30.0));
            widgets::avatar_disc(ui, p, disc, None, name, false);
            rows::names_left(rect)
        }
        MuteType::Object => rows::tile(ui, p, rect, None, "box", "", p.raised),
        _ => rows::tile(ui, p, rect, None, "ban", "", p.raised),
    };
    // from the right edge: "Unblock" under the pointer, then the pills
    let blocked = pills(m);
    let mut widths: Vec<f32> = Vec::new();
    if hot {
        widths.push(unblock_width(ui));
    }
    // the pills right to left: the last one nearest the edge
    widths.extend(blocked.iter().rev().map(|(_, label)| widgets::filter_pill_width(ui, "", label)));
    let end = rows::end(rect, left, &widths, 6.0, true);
    let mut items = end.items.iter();
    let mut clicked_inside = false;
    if hot && let Some(cell) = items.next() {
        let r = Rect::from_center_size(cell.center(), Vec2::new(cell.width(), 26.0));
        let unblock = ui.put(
            r,
            egui::Button::new(egui::RichText::new(aurora_i18n::tr!("common-unblock")).size(12.5)),
        );
        if unblock.clicked() {
            clicked_inside = true;
            for target in targets(st, m, all) {
                let (tid, tname) = key(&target);
                c.actions.push(PeopleAction::Unblock(tid, tname));
            }
        }
    }
    let mut menu_open = false;
    for ((bit, label), cell) in blocked.iter().rev().zip(items) {
        let r = Rect::from_center_size(cell.center(), Vec2::new(cell.width(), 26.0));
        let pill = widgets::filter_pill_at(ui, p, r, ui.scope_id().with(("people-blocked-pill", id, *bit)), "", label, false);
        if m.kind != MuteType::ByName {
            menu_open |= menu::dropdown(&pill, p, egui::PopupCloseBehavior::CloseOnClick, |ui| parts_menu(ui, c, m));
        }
        clicked_inside |= pill.clicked();
    }
    if let Some((cell, n)) = end.more {
        rows::paint_more(ui, p, cell, n);
    }
    rows::names(ui, p, rect, left, end.names_right, name, p.ink, m.kind.label(), None, None);
    super::keep_row_menu(st, id, menu_open);
    if !clicked_inside && resp.clicked() {
        let mods = ui.input(|i| i.modifiers);
        crate::ui::contacts::click_select(&mut st.sel, &mut st.anchor, order, id, mods);
    }
    if resp.secondary_clicked() && !selected {
        st.sel = vec![id];
        st.anchor = Some(id);
    }
    // menu_fs_block_list.xml; for several, unblock them all
    let several = st.sel.len() > 1 && st.sel.contains(&id);
    let chosen = targets(st, m, all);
    menu::context_menu(&resp, p, |ui| {
        if several {
            if menu::item(ui, p, "circle-check-filled", aurora_i18n::tr!("common-stop-ignoring")) {
                for target in &chosen {
                    let (tid, tname) = key(target);
                    c.actions.push(PeopleAction::Unblock(tid, tname));
                }
            }
            return;
        }
        if menu::item(ui, p, "circle-check-filled", aurora_i18n::tr!("common-stop-ignoring")) {
            c.actions.push(PeopleAction::Unblock(m.id, m.name.clone()));
        }
        if m.kind != MuteType::ByName {
            parts_menu(ui, c, m);
        }
        menu::separator(ui, p);
        if menu::item_if(ui, p, "user-filled", aurora_i18n::tr!("common-profile"), m.kind == MuteType::Agent) {
            c.actions.push(PeopleAction::Profile(m.id));
        }
    });
}

/// The entries a row's action applies to: the selection when the row is
/// part of it, else the row alone.
fn targets(st: &PeopleUi, m: &Mute, all: &[Mute]) -> Vec<Mute> {
    let id = row_id(m);
    if st.sel.contains(&id) {
        all.iter().filter(|x| st.sel.contains(&row_id(x))).cloned().collect()
    } else {
        vec![m.clone()]
    }
}

/// What of an entry is blocked, each a toggle (LLMute flags).
fn parts_menu(ui: &mut egui::Ui, c: &mut Ctx, m: &Mute) {
    let p = c.p;
    for (bit, icon, label) in [
        (flag::VOICE_CHAT, "microphone-off", aurora_i18n::tr!("ui-people-block-voice")),
        (flag::TEXT_CHAT, "message-off", aurora_i18n::tr!("ui-people-block-text")),
        (flag::PARTICLES, "sparkles-filled", aurora_i18n::tr!("common-block-particles")),
        (flag::OBJECT_SOUNDS, "volume-off", aurora_i18n::tr!("ui-people-block-object-sounds")),
    ] {
        // a set flag means "not blocked" (LLMute flags)
        let mut on = m.flags & bit == 0;
        if menu::toggle(ui, p, icon, label, &mut on) {
            c.actions.push(PeopleAction::BlockFlag {
                id: m.id,
                name: m.name.clone(),
                kind: m.kind,
                flag: bit,
                on,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pills_say_what_is_blocked() {
        let m = |kind, flags| Mute {
            id: Uuid::from_u128(1),
            name: "x".into(),
            kind,
            flags,
        };
        // everything
        assert_eq!(pills(&m(MuteType::Agent, 0)).len(), 1);
        assert_eq!(pills(&m(MuteType::Agent, 0))[0].0, flag::ALL);
        assert_eq!(pills(&m(MuteType::ByName, 0))[0].0, flag::ALL);
        // voice and text blocked: the particles and sounds flags are set
        let parts: Vec<u32> = pills(&m(MuteType::Agent, flag::PARTICLES | flag::OBJECT_SOUNDS))
            .iter()
            .map(|(b, _)| *b)
            .collect();
        assert_eq!(parts, vec![flag::VOICE_CHAT, flag::TEXT_CHAT]);
        // the sounds of an object only
        let parts: Vec<u32> = pills(&m(MuteType::Object, flag::ALL & !flag::OBJECT_SOUNDS))
            .iter()
            .map(|(b, _)| *b)
            .collect();
        assert_eq!(parts, vec![flag::OBJECT_SOUNDS]);
    }
}
