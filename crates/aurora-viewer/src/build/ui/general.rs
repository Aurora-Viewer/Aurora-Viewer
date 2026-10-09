//! General tab (LLPanelPermissions, indra/newview/llpanelpermissions.cpp,
//! originally LGPL 2.1): name, description, creator / owner / previous
//! owner, group, share / deed, click action, sale, object key, search,
//! permissions of everyone and of the next owner.

use super::common::{check, combo, label, row, small, spin_i, text};
use super::{Confirm, LABEL_W, Request};
use crate::build::edits::{CLICK_ACTION_BUY, CLICK_ACTION_TOUCH, FLAGS_INCLUDE_IN_SEARCH};
use crate::build::{BuildSettings, BuildTool};
use crate::theme::Palette;
use crate::world::World;
use aurora_net::build::{ObjectProps, perm, perm_field};
use egui::RichText;
use uuid::Uuid;

/// Group powers (roles_constants.h).
const GP_OBJECT_DEED: u64 = 1 << 36;
const GP_OBJECT_MANIPULATE: u64 = 1 << 38;
const GP_OBJECT_SET_SALE: u64 = 1 << 39;
/// FS_NOT / ORIGINAL / COPY / CONTENTS.
const SALE_COPY: u8 = 2;

/// "clickaction" combo, in its display order (CLICK_ACTION_* codes).
const CLICK_ACTIONS: [(&str, u8); 8] = [
    ("Toucher (par défaut)", 0),
    ("S'asseoir sur l'objet", 1),
    ("Acheter l'objet", 2),
    ("Payer l'objet", 3),
    ("Ouvrir", 4),
    ("Zoomer", 7),
    ("Ignorer l'objet", 9),
    ("Aucun", 8),
];

/// "sale type" combo: Copie, Contenus, Original.
const SALE_TYPES: [(&str, u8); 3] = [("Copie", 2), ("Contenus", 3), ("Original", 1)];

/// LLSelectMgr::selectGetPerm over the roots: (all set, all clear) bits.
fn perm_state(list: &[&ObjectProps], mask: impl Fn(&ObjectProps) -> u32) -> (u32, u32) {
    let mut on = u32::MAX;
    let mut off = u32::MAX;
    for p in list {
        let m = mask(p);
        on &= m;
        off &= !m;
    }
    (on, off)
}

/// (value, tentative) of one bit.
fn bit(state: (u32, u32), b: u32) -> (bool, bool) {
    if state.0 & b == b {
        (true, false)
    } else if state.1 & b == b {
        (false, false)
    } else {
        (true, true)
    }
}

/// A clickable name (agent or group SLURL of the panel).
fn name_link(ui: &mut egui::Ui, p: &Palette, name: &str) -> bool {
    ui.add(egui::Label::new(RichText::new(name).size(12.0).color(p.violet_light)).sense(egui::Sense::click()))
        .on_hover_text("Ouvrir le profil")
        .clicked()
}

pub fn show(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings) {
    // the roots, or the selected prims when no root is (LLPanelPermissions::refresh)
    let mut targets: Vec<usize> = tool
        .sel_prims(world)
        .into_iter()
        .filter(|&i| world.objects.get(i).is_some_and(|o| o.parent_id == 0))
        .collect();
    let root_selected = !targets.is_empty();
    if !root_selected {
        targets = tool.sel_prims(world);
    }
    let ids: Vec<Uuid> = targets.iter().filter_map(|&i| world.objects.get(i).map(|o| o.full_id)).collect();
    let props: Vec<ObjectProps> = ids.iter().filter_map(|id| tool.props.get(id).cloned()).collect();
    if props.len() != ids.len() || props.is_empty() {
        small(ui, p, "Propriétés en attente du simulateur…");
        return;
    }
    let refs: Vec<&ObjectProps> = props.iter().collect();
    let first = &props[0];
    let count = props.len();
    let me = world.agent_id;
    let same = |f: &dyn Fn(&ObjectProps) -> Uuid| props.iter().all(|x| f(x) == f(first));
    let owners_identical = same(&|x| x.owner_id);
    let group_owned = first.owner_id == first.group_id && !first.group_id.is_nil();
    let group_power = |power: u64| world.groups.group(&first.group_id).is_some_and(|g| g.powers & power != 0);
    let self_owned = owners_identical && first.owner_id == me;
    let can_manipulate = self_owned || (owners_identical && group_owned && group_power(GP_OBJECT_MANIPULATE));
    let can_sell = self_owned || (owners_identical && group_owned && group_power(GP_OBJECT_SET_SALE));
    let modify = props.iter().all(|x| x.owner_mask & perm::MODIFY != 0) && can_manipulate;

    // ---- name, description
    for (field, title, limit, description) in [("name", "Nom :", 63, false), ("desc", "Description :", 127, true)] {
        let current = if count == 1 {
            if description {
                first.description.clone()
            } else {
                first.name.clone()
            }
        } else {
            String::new()
        };
        let mut v = tool.ui.edits.get(field).cloned().unwrap_or_else(|| current.clone());
        row(ui, p, title, LABEL_W - 30.0, |ui| {
            let r = ui.add_enabled(
                first.owner_mask & perm::MODIFY != 0,
                egui::TextEdit::singleline(&mut v)
                    .desired_width(ui.available_width())
                    .char_limit(limit)
                    .hint_text(if count > 1 { "Sélection multiple" } else { "" }),
            );
            if description {
                r.clone().on_hover_text("127 octets au plus.");
            } else {
                r.clone().on_hover_text("63 caractères au plus, ASCII imprimable, sans « | ».");
            }
            if r.changed() {
                // validateASCIIPrintableNoPipe
                v.retain(|c| (' '..='~').contains(&c) && c != '|');
                tool.ui.edits.insert(field, v.clone());
            }
            if r.lost_focus() {
                tool.ui.edits.remove(field);
                if v != current && (description || !v.trim().is_empty()) {
                    tool.set_name(world, &v, description);
                }
            }
        });
    }
    ui.add_space(4.0);

    // ---- creator, owner, previous owner, group
    for id in [first.creator_id, first.owner_id, first.last_owner_id] {
        if !id.is_nil() {
            world.social.want_name(id);
        }
    }
    row(ui, p, "Créateur :", LABEL_W, |ui| {
        if !same(&|x| x.creator_id) {
            text(ui, p, "(multiple)");
        } else if first.creator_id.is_nil() {
            text(ui, p, "(personne)");
        } else if name_link(ui, p, &world.social.name_of(&first.creator_id)) {
            tool.ui.requests.push(Request::Profile(first.creator_id));
        }
    });
    row(ui, p, "Propriétaire :", LABEL_W, |ui| {
        if !owners_identical {
            text(ui, p, "(multiple)");
        } else if group_owned {
            let name = world
                .groups
                .group(&first.group_id)
                .map(|g| g.name.clone())
                .unwrap_or_else(|| "Groupe".into());
            if name_link(ui, p, &name) {
                tool.ui.requests.push(Request::GroupProfile(first.group_id));
            }
        } else if name_link(ui, p, &world.social.name_of(&first.owner_id)) {
            tool.ui.requests.push(Request::Profile(first.owner_id));
        }
    });
    row(ui, p, "Propriétaire précédent :", LABEL_W, |ui| {
        if same(&|x| x.last_owner_id) && !first.last_owner_id.is_nil() && name_link(ui, p, &world.social.name_of(&first.last_owner_id)) {
            tool.ui.requests.push(Request::Profile(first.last_owner_id));
        }
    });
    row(ui, p, "Groupe :", LABEL_W, |ui| {
        if same(&|x| x.group_id) && !first.group_id.is_nil() {
            let name = world
                .groups
                .group(&first.group_id)
                .map(|g| g.name.clone())
                .unwrap_or_else(|| first.group_id.to_string()[..8].to_owned());
            if name_link(ui, p, &name) {
                tool.ui.requests.push(Request::GroupProfile(first.group_id));
            }
        } else {
            text(ui, p, "");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let can_set_group = root_selected && owners_identical && first.owner_id == me;
            if super::common::icon_button(ui, p, "wrench", "Définir le groupe…", can_set_group) {
                tool.ui.group_picker_open = true;
            }
        });
    });

    // ---- share with group, deed
    let group = perm_state(&refs, |x| x.group_mask);
    let owner = perm_state(&refs, |x| x.owner_mask);
    let base = perm_state(&refs, |x| x.base_mask);
    let share_bits = perm::COPY | perm::MODIFY | perm::MOVE;
    let (share, share_tentative) = if group.0 & share_bits == share_bits {
        (true, false)
    } else if group.1 & share_bits == share_bits {
        (false, false)
    } else {
        (true, true)
    };
    let can_transfer = owner.0 & perm::TRANSFER != 0;
    ui.horizontal(|ui| {
        ui.add_space(LABEL_W);
        if let Some(v) = check(ui, share, share_tentative, "Partager", can_manipulate) {
            tool.set_perm(world, perm_field::GROUP, v, share_bits);
        }
        let deed_ok = !first.group_id.is_nil()
            && group_power(GP_OBJECT_DEED)
            && can_transfer
            && !group_owned
            && (share && !share_tentative || group.0 & perm::MOVE != 0);
        if ui
            .add_enabled(deed_ok, egui::Button::new(RichText::new("Céder").size(12.0)))
            .on_hover_text("Céder cet objet au groupe")
            .clicked()
        {
            tool.ui.confirm = Some(Confirm::Deed(first.group_id));
        }
    });

    // ---- click action
    let prims = tool.sel_prims(world);
    let (click, _) = super::common::same(prims.iter().filter_map(|&i| world.objects.get(i).map(|o| o.click_action)));
    // PLAY / OPEN_MEDIA show as « Toucher »
    let click = click.map(|c| if CLICK_ACTIONS.iter().any(|(_, v)| *v == c) { c } else { 0 });
    row(ui, p, "Cliquer pour :", LABEL_W, |ui| {
        if let Some(a) = combo(ui, "click_action", click, &CLICK_ACTIONS, 150.0, modify) {
            if a == CLICK_ACTION_BUY && !props.iter().all(|x| x.sale_type != 0) {
                // CantSetBuyObject
                tool.status = "Cet objet n'est pas à vendre : impossible de choisir « Acheter l'objet ».".into();
            } else {
                tool.set_click_action(world, a);
            }
        }
    });

    // ---- sale
    let for_sale_n = props.iter().filter(|x| x.sale_type != 0).count();
    let sale_mixed = for_sale_n != 0 && for_sale_n != count;
    let price_mixed = !props.iter().all(|x| x.sale_price == first.sale_price);
    let sale_type_now = props.iter().find(|x| x.sale_type != 0).map(|x| x.sale_type).unwrap_or(SALE_COPY);
    // attachments are never in the build selection here
    let sale_ok = can_sell && owner.0 & perm::TRANSFER != 0;
    let (mut for_sale, mut sale_type, mut price) = tool
        .ui
        .sale_draft
        .unwrap_or((for_sale_n > 0, sale_type_now, first.sale_price.max(0)));
    let mut draft_changed = false;
    let mut commit_now = false;
    egui::Frame::new()
        .stroke(egui::Stroke::new(
            1.0,
            if tool.ui.sale_draft.is_some() { p.violet } else { p.raised },
        ))
        .corner_radius(2)
        .inner_margin(4)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                if let Some(v) = check(ui, for_sale, sale_mixed, "À vendre :", sale_ok && (can_transfer || for_sale_n > 0)) {
                    for_sale = v;
                    draft_changed = true;
                    // unticking commits at once (onCommitSaleInfo)
                    commit_now = !v;
                }
                if let Some(t) = combo(
                    ui,
                    "sale_type",
                    Some(sale_type),
                    &SALE_TYPES,
                    110.0,
                    sale_ok && for_sale && !price_mixed,
                ) {
                    sale_type = t;
                    draft_changed = true;
                }
            });
            ui.horizontal(|ui| {
                label(ui, p, "L$");
                ui.add_enabled_ui(sale_ok && for_sale && !sale_mixed, |ui| {
                    let e = spin_i(ui, &mut price, 0..=999_999_999, 100.0);
                    if e.changed {
                        draft_changed = true;
                    }
                    // a typed price commits when « À vendre » is ticked
                    if e.commit && for_sale {
                        commit_now = true;
                    }
                });
                if ui
                    .add_enabled(
                        tool.ui.sale_draft.is_some() || draft_changed,
                        egui::Button::new(RichText::new("Appliquer").size(12.0)),
                    )
                    .clicked()
                {
                    commit_now = true;
                }
            });
        });
    if draft_changed {
        tool.ui.sale_draft = Some((for_sale, sale_type, price));
    }
    if commit_now {
        // LLPanelPermissions::setAllSaleInfo
        let st = if for_sale && price >= 0 { sale_type } else { 0 };
        let was_for_sale = for_sale_n > 0;
        tool.set_sale(world, st, price.max(0));
        tool.ui.sale_draft = None;
        // FIRE-5273: Buy / Touch follow the sale state when they were Buy or Touch
        let all_touch_or_buy = prims.iter().all(|&i| {
            world
                .objects
                .get(i)
                .is_some_and(|o| o.click_action == CLICK_ACTION_BUY || o.click_action == CLICK_ACTION_TOUCH)
        });
        if modify && all_touch_or_buy && was_for_sale != (st != 0) {
            tool.set_click_action(world, if st != 0 { CLICK_ACTION_BUY } else { CLICK_ACTION_TOUCH });
        }
    }

    // ---- key, search
    ui.horizontal(|ui| {
        if ui
            .button(RichText::new("Copier l'UUID").size(12.0))
            .on_hover_text("Copier la clé des objets (Maj : de toutes les parties)")
            .clicked()
        {
            let shift = ui.input(|i| i.modifiers.shift);
            let keys = tool.copy_keys(world, s, shift);
            ui.ctx().copy_text(keys);
        }
        let roots = tool.roots(world);
        let (search, search_mixed) = super::common::same(
            roots
                .iter()
                .filter_map(|&r| world.objects.get(r).map(|o| o.update_flags & FLAGS_INCLUDE_IN_SEARCH != 0)),
        );
        if let Some(v) = check(ui, search.unwrap_or(false), search_mixed, "Afficher dans la recherche", can_sell) {
            tool.set_include_in_search(world, v);
        }
    });

    // ---- modify info (text modify info 1..6)
    let info = if !can_manipulate && !root_selected {
        "Sélectionnez l'objet en entier"
    } else {
        match (modify, count == 1) {
            (true, true) => "Vous pouvez modifier cet objet",
            (true, false) => "Vous pouvez modifier ces objets",
            (false, true) => "Vous ne pouvez pas modifier cet objet",
            (false, false) => "Vous ne pouvez pas modifier ces objets",
        }
    };
    ui.label(RichText::new(info).size(12.0).color(if modify { p.violet_light } else { p.muted }));

    // ---- everyone / next owner
    let everyone = perm_state(&refs, |x| x.everyone_mask);
    let next = perm_state(&refs, |x| x.next_owner_mask);
    label(ui, p, "N'importe qui :");
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let (v, t) = bit(everyone, perm::MOVE);
        if let Some(n) = check(ui, v, t, "Bouger", can_manipulate && owner.0 & perm::MOVE != 0) {
            tool.set_perm(world, perm_field::EVERYONE, n, perm::MOVE);
        }
        let (v, t) = bit(everyone, perm::COPY);
        let copy_ok = owner.0 & perm::COPY != 0 && owner.0 & perm::TRANSFER != 0;
        if let Some(n) = check(ui, v, t || !copy_ok, "Copier", can_manipulate && copy_ok) {
            tool.set_perm(world, perm_field::EVERYONE, n, perm::COPY);
        }
    });
    label(ui, p, "Le prochain propriétaire :");
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let next_ok = can_sell && owner.0 & perm::TRANSFER != 0;
        let (v, t) = bit(next, perm::MODIFY);
        if let Some(n) = check(ui, v, t, "Modifier", next_ok && base.0 & perm::MODIFY != 0) {
            tool.set_perm(world, perm_field::NEXT_OWNER, n, perm::MODIFY);
        }
        let (v, t) = bit(next, perm::COPY);
        let can_copy = owner.0 & perm::COPY != 0;
        if let Some(n) = check(ui, v, t || !can_copy, "Copier", next_ok && base.0 & perm::COPY != 0) {
            tool.set_perm(world, perm_field::NEXT_OWNER, n, perm::COPY);
        }
        let (v, t) = bit(next, perm::TRANSFER);
        // enabled by the next owner's copy bit (sic, LLPanelPermissions)
        if let Some(n) = check(ui, v, t || !can_transfer, "Transférer", next_ok && next.0 & perm::COPY != 0) {
            tool.set_perm(world, perm_field::NEXT_OWNER, n, perm::TRANSFER);
        }
    });
    row(ui, p, "Att. de rche. de chemin :", LABEL_W + 30.0, |ui| {
        text(ui, p, "Aucun");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn perm_bits_mixed() {
        let a = ObjectProps {
            everyone_mask: perm::MOVE,
            ..Default::default()
        };
        let b = ObjectProps::default();
        let st = perm_state(&[&a, &b], |x| x.everyone_mask);
        assert_eq!(bit(st, perm::MOVE), (true, true));
        assert_eq!(bit(st, perm::COPY), (false, false));
        let st = perm_state(&[&a, &a], |x| x.everyone_mask);
        assert_eq!(bit(st, perm::MOVE), (true, false));
    }
}
