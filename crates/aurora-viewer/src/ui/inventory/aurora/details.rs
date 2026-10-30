//! What Aurora's inventory window says of its selection: a card at the
//! bottom of the narrow and medium widths (picture, name, type and
//! creator, your rights and the next owner's, the main actions) and the
//! details pane of the wide width, which also sets the next owner's rights
//! as the properties do (LLSidepanelItemInfo, LLFloaterProperties). The
//! main action is the one a double-click does (wear, open, teleport…).

use super::super::permissions::{Permissions, Rights};
use super::super::*;
use super::{Layout, rights, rows};
use crate::ui::{sheet, widgets};
use crate::world::appearance::Action as Wear;
use aurora_llsd::llsd_map;
use egui::{Pos2, Rect, Sense, Vec2};
use std::collections::HashMap;

/// The height of the card.
pub const STRIP: f32 = 118.0;
const BUTTON: f32 = 26.0;

/// The main action of an item: what its double-click does, by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Main {
    Wear,
    TakeOff,
    Open,
    Teleport,
    Play,
    Activate(bool),
    Profile,
}

impl Main {
    fn of(inv: &Inventory, id: Uuid, facts: &Facts) -> Option<Self> {
        let original = rules::original(inv, id).and_then(|id| inv.items.get(&id))?;
        Some(match original.asset_type {
            5 | 6 | 13 if facts.worn.contains(&original.id) => {
                // a worn body part can only be replaced
                if original.asset_type == 13 {
                    return None;
                }
                Self::TakeOff
            }
            5 | 6 | 13 => Self::Wear,
            3 => Self::Teleport,
            1 => Self::Play,
            21 => Self::Activate(!facts.active_gestures.contains(&original.id)),
            2 => Self::Profile,
            _ => Self::Open,
        })
    }

    fn label(self) -> (&'static str, &'static str) {
        match self {
            Self::Wear => ("hanger-2-filled", aurora_i18n::tr!("common-wear")),
            Self::TakeOff => ("hanger-2-filled", aurora_i18n::tr!("inventory-take-off")),
            Self::Open => ("folder-open-filled", aurora_i18n::tr!("common-open")),
            Self::Teleport => ("map-pin-filled", aurora_i18n::tr!("common-teleport")),
            Self::Play => ("player-play-filled", aurora_i18n::tr!("common-play")),
            Self::Activate(true) => ("player-play-filled", aurora_i18n::tr!("gesture-activate")),
            Self::Activate(false) => ("player-stop-filled", aurora_i18n::tr!("gesture-deactivate")),
            Self::Profile => ("user-filled", aurora_i18n::tr!("inventory-profile")),
        }
    }

    fn run(
        self,
        inv: &Inventory,
        id: Uuid,
        st: &mut InventoryUi,
        prefs: &InventoryPreferences,
        facts: &Facts,
        actions: &mut Vec<InvAction>,
    ) {
        let original = rules::original(inv, id);
        match self {
            Self::Wear | Self::TakeOff => context::double_click(inv, id, st, prefs, facts, actions),
            Self::Play => {
                if let Some(asset) = original
                    .and_then(|id| inv.items.get(&id))
                    .map(|it| it.asset_id)
                    .filter(|a| !a.is_nil())
                {
                    actions.push(InvAction::Sound(asset));
                }
            }
            Self::Activate(on) => {
                if let Some(original) = original {
                    actions.push(InvAction::Gesture(crate::ui::gestures::Action::Active(vec![original], on)));
                }
            }
            Self::Open | Self::Teleport | Self::Profile => context::open(inv, id, st, actions),
        }
    }
}

/// "Add": an object or clothing not worn, kept with what is worn.
fn can_add(inv: &Inventory, id: Uuid, facts: &Facts, st: &InventoryUi) -> bool {
    rules::original(inv, id).and_then(|id| inv.items.get(&id)).is_some_and(|it| {
        matches!(it.asset_type, 5 | 6)
            && !(it.asset_type == 5 && it.flags & 0xff == 15)
            && !facts.worn.contains(&it.id)
            && !rules::library(inv, it.id)
            && !rules::in_type(inv, it.id, 14)
            && !facts.appearance_busy
            && st.pending.is_none()
    })
}

fn add(inv: &Inventory, id: Uuid, actions: &mut Vec<InvAction>) {
    if let Some(original) = rules::original(inv, id) {
        actions.push(InvAction::Appearance(Wear::WearItems {
            items: vec![original],
            replace: false,
            point: 0,
        }));
    }
}

/// "Give": the resident picker, for the selection.
fn can_give(inv: &Inventory, ids: &[Uuid], st: &InventoryUi, prefs: &InventoryPreferences, facts: &Facts) -> bool {
    st.pending.is_none()
        && !ids.is_empty()
        && ids
            .iter()
            .all(|id| rules::mutable(inv, *id, &prefs.protected) && rules::offer(inv, *id, &prefs.protected, &facts.worn).is_ok())
}

fn give(st: &mut InventoryUi, ids: Vec<Uuid>) {
    st.share_items = ids;
    st.share_picker.open(&format!("inventory_share_{}", st.ui_id), false);
}

/// The ids the actions act on: the selection, or the row shown.
fn selection(st: &InventoryUi) -> Vec<Uuid> {
    let mut ids: Vec<_> = st.selection.iter().copied().collect();
    ids.sort();
    ids
}

/// The picture of an item or folder: its thumbnail, else its type's icon.
#[allow(clippy::too_many_arguments)]
fn picture(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    id: Uuid,
    rect: Rect,
    images: &HashMap<Uuid, egui::TextureHandle>,
    wanted: &mut HashSet<Uuid>,
) -> egui::Response {
    let thumbnail = thumbnail::image(inv, id);
    if !thumbnail.is_nil() {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
        thumbnail::picture(&mut child, p, thumbnail, rect.width().min(rect.height()), images, wanted);
        return ui.interact(rect, ui.scope_id().with(("aurora_picture", id)), Sense::click());
    }
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.panel);
    let (icon, tint) = inv.items.get(&id).map_or_else(
        || {
            let kind = inv.folders.get(&id).map_or(-1, |f| f.info.type_default);
            (tree::folder_icon(kind, false), p.muted_dim)
        },
        |it| inventory_item_icon(inv, it, p),
    );
    let size = (rect.height() * 0.42).clamp(20.0, 44.0);
    if let Some(texture) = icons.get(icon) {
        ui.painter().image(
            texture.id(),
            Rect::from_center_size(rect.center(), Vec2::splat(size)),
            Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
            tint,
        );
    }
    ui.interact(rect, ui.scope_id().with(("aurora_picture", id)), Sense::click())
}

/// The "more actions" button: the right-click menu of what it is about.
fn more(ui: &mut egui::Ui, p: &Palette, st: &mut InventoryUi, id: Uuid) {
    let response = widgets::action_button(ui, p, "dots-filled", "", widgets::ActionKind::Flat, Vec2::splat(BUTTON))
        .on_hover_text(aurora_i18n::tr!("inventory-more-actions"));
    if response.clicked() {
        st.row_menu = Some((id, response.rect.left_bottom()));
    }
}

/// The name of a resident, or "You".
fn resident(facts: &Facts, id: Uuid) -> String {
    if id == facts.agent && !id.is_nil() {
        aurora_i18n::tr!("inventory-owner-you").to_owned()
    } else if id.is_nil() {
        aurora_i18n::tr!("ui-inventory-properties-creator-unknown-multiple").to_owned()
    } else {
        facts
            .names
            .get(&id)
            .cloned()
            .unwrap_or_else(|| aurora_i18n::tr!("common-loading-2").to_owned())
    }
}

/// A resident as a link to their profile.
fn resident_link(ui: &mut egui::Ui, p: &Palette, facts: &Facts, id: Uuid, size: f32, actions: &mut Vec<InvAction>) {
    let name = resident(facts, id);
    if id.is_nil() {
        ui.label(egui::RichText::new(name).size(size).color(p.muted));
        return;
    }
    let link = ui
        .add(egui::Label::new(egui::RichText::new(name).size(size).color(p.violet_light)).sense(Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if link.clicked() {
        actions.push(InvAction::Profile(id));
    }
}

/// The empty card: what selecting does.
fn nothing(ui: &mut egui::Ui, p: &Palette, rect: Rect) {
    let galley = ui.painter().layout(
        aurora_i18n::tr!("inventory-details-none").to_owned(),
        egui::FontId::proportional(12.0),
        p.muted,
        rect.width() - 24.0,
    );
    widgets::paint_ink_centered(ui.painter(), galley, rect.center(), p.muted);
}

/// The card of the selection at the bottom of the narrow and medium
/// widths.
#[allow(clippy::too_many_arguments)]
pub(super) fn strip(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), STRIP), Sense::hover());
    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
    let inner = rect.shrink(10.0);
    let ids = selection(st);
    let shown = st.selected.filter(|id| ids.len() == 1 && ids[0] == *id);
    if ids.is_empty() {
        nothing(ui, p, rect);
        return;
    }
    let picture_rect = Rect::from_min_size(inner.min, Vec2::splat(64.0));
    let text = Rect::from_min_max(Pos2::new(picture_rect.right() + 10.0, inner.top()), inner.max);
    let mut body = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(text)
            .id_salt("aurora_inventory_card")
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    body.spacing_mut().item_spacing = Vec2::new(6.0, 2.0);
    let Some(id) = shown else {
        // several entries: what they are, and their actions
        if let Some(t) = icons.get("Inv_FolderClosed") {
            ui.painter().rect_filled(picture_rect, ui.visuals().window_corner_radius, p.panel);
            ui.painter().image(
                t.id(),
                Rect::from_center_size(picture_rect.center(), Vec2::splat(26.0)),
                Rect::from_min_max(Pos2::ZERO, egui::pos2(1.0, 1.0)),
                p.muted_dim,
            );
        }
        body.label(sheet::semibold(
            &body,
            &aurora_i18n::tr!("inventory-details-many", count = ids.len()),
            13.0,
            p.ink,
        ));
        body.add_space(16.0);
        body.horizontal(|ui| {
            if widgets::action_button_if(
                ui,
                p,
                "share",
                aurora_i18n::tr!("common-share"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, BUTTON),
                can_give(inv, &ids, st, prefs, facts),
            )
            .clicked()
            {
                give(st, ids.clone());
            }
            more(ui, p, st, ids[0]);
        });
        return;
    };
    let clicked = picture(ui, p, icons, inv, id, picture_rect, images, &mut st.wanted_images);
    if clicked.double_clicked() {
        context::double_click(inv, id, st, prefs, facts, actions);
    }
    let name = rules::name(inv, id);
    let name = if id == inv.root || id == inv.lib_root {
        folder_name(inv, id)
    } else {
        name
    };
    body.add(egui::Label::new(sheet::semibold(&body, &name, 13.0, p.ink)).truncate());
    if let Some(folder) = inv.folders.get(&id) {
        body.label(egui::RichText::new(rows::kind_name(inv, id)).size(12.0).color(p.muted));
        body.add_space(8.0);
        body.horizontal(|ui| {
            if widgets::action_button(
                ui,
                p,
                "folder-open-filled",
                aurora_i18n::tr!("common-open"),
                widgets::ActionKind::Primary,
                Vec2::new(0.0, BUTTON),
            )
            .clicked()
            {
                if st.aurora.is_some_and(|f| f.layout == Layout::Compact) {
                    st.plain.set_open(id, true);
                } else {
                    st.browse = folder.info.id;
                    set_tab(st, prefs, 0);
                }
            }
            more(ui, p, st, id);
        });
        return;
    }
    let Some(it) = inv.items.get(&id) else { return };
    let original = rules::original(inv, id).and_then(|id| inv.items.get(&id)).unwrap_or(it);
    body.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(egui::RichText::new(rows::kind_name(inv, id)).size(12.0).color(p.muted));
        ui.label(egui::RichText::new("·").size(12.0).color(p.muted_dim));
        resident_link(ui, p, facts, original.creator, 12.0, actions);
    });
    if let Some(permissions) = Permissions::inventory(inv, id) {
        body.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("inventory-rights-you"))
                    .size(12.0)
                    .color(p.muted),
            );
            rights::label(ui, p, permissions, Rights::Mine);
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new(aurora_i18n::tr!("inventory-rights-next"))
                    .size(12.0)
                    .color(p.muted),
            );
            rights::label(ui, p, permissions.with_next_owner(true), Rights::Next);
        });
    }
    body.add_space(2.0);
    body.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some(main) = Main::of(inv, id, facts) {
            let (icon, label) = main.label();
            let enabled = st.pending.is_none() && !(matches!(main, Main::Wear | Main::TakeOff) && facts.appearance_busy);
            if widgets::action_button_if(ui, p, icon, label, widgets::ActionKind::Primary, Vec2::new(0.0, BUTTON), enabled).clicked() {
                main.run(inv, id, st, prefs, facts, actions);
            }
        }
        if can_add(inv, id, facts, st)
            && widgets::action_button(
                ui,
                p,
                "plus",
                aurora_i18n::tr!("common-add"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, BUTTON),
            )
            .clicked()
        {
            add(inv, id, actions);
        }
        if widgets::action_button(
            ui,
            p,
            "info-circle-filled",
            aurora_i18n::tr!("common-properties"),
            widgets::ActionKind::Flat,
            Vec2::new(0.0, BUTTON),
        )
        .clicked()
        {
            st.property_edit = None;
            st.dialog = Some(EditDialog::Properties(id));
        }
        more(ui, p, st, id);
    });
}

/// Set the next owner's rights of `it` to `next`, as the properties save
/// them (with the group's and everyone's as they are).
fn set_next_owner(inv: &Inventory, it: &aurora_net::inventory::InvItem, next: u32, st: &mut InventoryUi, actions: &mut Vec<InvAction>) {
    let body = llsd_map! {
        "permissions" => llsd_map! {
            "next_owner_mask" => next as i32,
            "group_mask" => it.group_mask as i32,
            "everyone_mask" => it.everyone_mask as i32
        }
    };
    match rules::patch(inv, it.id, body) {
        Ok(change) => actions.push(InvAction::Edit(change)),
        Err(reason) => st.message = reason,
    }
}

/// The next owner's mask after `bit` is set `on`, with the properties'
/// rules: no copy means transfer, settings always copy.
fn next_mask(it: &aurora_net::inventory::InvItem, bit: u32, on: bool) -> u32 {
    let mut next = if on { it.next_owner_mask | bit } else { it.next_owner_mask & !bit };
    if it.asset_type == 56 {
        next |= rules::COPY;
    }
    if next & rules::COPY == 0 {
        next |= rules::TRANSFER;
    }
    next
}

/// Whether the next owner's `bit` can be changed (LLSidepanelItemInfo's
/// checks, as the properties make them).
fn next_editable(inv: &Inventory, it: &aurora_net::inventory::InvItem, bit: u32, st: &InventoryUi, prefs: &InventoryPreferences) -> bool {
    rules::mutable(inv, it.id, &prefs.protected)
        && !matches!(it.asset_type, 2 | 24 | 25)
        && st.pending.is_none()
        && it.base_mask & bit != 0
        && !(bit == rules::COPY && it.asset_type == 56)
        && (bit != rules::TRANSFER || it.next_owner_mask & rules::COPY != 0)
}

fn fact(ui: &mut egui::Ui, p: &Palette, label: &str, value: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.set_min_height(22.0);
        ui.label(egui::RichText::new(label).size(12.5).color(p.muted));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), value);
    });
}

fn heading(ui: &mut egui::Ui, p: &Palette, text: &str) {
    crate::ui::menu::section(ui, p, text);
}

/// The details pane of the wide width.
#[allow(clippy::too_many_arguments)]
pub(super) fn pane(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &Inventory,
    st: &mut InventoryUi,
    prefs: &mut InventoryPreferences,
    facts: &Facts,
    images: &HashMap<Uuid, egui::TextureHandle>,
    actions: &mut Vec<InvAction>,
) {
    let rect = ui.max_rect();
    ui.painter()
        .rect_filled(rect, ui.visuals().window_corner_radius, p.field.gamma_multiply(0.35));
    let mut column = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rect.shrink(12.0))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    let ui = &mut column;
    ui.spacing_mut().item_spacing.y = 6.0;
    let ids = selection(st);
    let Some(id) = st.selected.filter(|id| ids.len() == 1 && ids[0] == *id) else {
        if ids.is_empty() {
            nothing(ui, p, rect);
        } else {
            ui.label(sheet::semibold(
                ui,
                &aurora_i18n::tr!("inventory-details-many", count = ids.len()),
                15.0,
                p.ink,
            ));
            ui.horizontal(|ui| {
                if widgets::action_button_if(
                    ui,
                    p,
                    "share",
                    aurora_i18n::tr!("common-share"),
                    widgets::ActionKind::Flat,
                    Vec2::new(0.0, 28.0),
                    can_give(inv, &ids, st, prefs, facts),
                )
                .clicked()
                {
                    give(st, ids.clone());
                }
                more(ui, p, st, ids[0]);
            });
        }
        return;
    };
    // The picture: a click changes it.
    let width = ui.available_width();
    let (picture_rect, _) = ui.allocate_exact_size(Vec2::new(width, 176.0), Sense::hover());
    let writable = st.pending.is_none()
        && rules::mutable(inv, id, &prefs.protected)
        && !inv.items.get(&id).is_some_and(|it| matches!(it.asset_type, 24 | 25));
    let response = picture(ui, p, icons, inv, id, picture_rect, images, &mut st.wanted_images);
    if writable {
        let response = response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(aurora_i18n::tr!("inventory-details-picture"));
        if response.clicked() {
            st.thumbnail.open(id);
        }
    }
    // The name, and its renaming.
    let name = if id == inv.root || id == inv.lib_root {
        folder_name(inv, id)
    } else {
        rules::name(inv, id)
    };
    ui.horizontal(|ui| {
        let rename = st.pending.is_none() && rules::renameable(inv, id, &prefs.protected);
        let room = ui.available_width() - if rename { 30.0 } else { 0.0 };
        ui.allocate_ui_with_layout(Vec2::new(room, 22.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(sheet::semibold(ui, &name, 15.0, p.ink)).wrap());
        });
        if rename && widgets::icon_button(ui, p, "pencil-filled", aurora_i18n::tr!("common-rename"), true) {
            st.begin_rename(inv, id);
        }
    });
    let place = rules::parent(inv, id).map(|parent| folder_name(inv, parent));
    let mut line = rows::kind_name(inv, id);
    if let Some(place) = place {
        line = aurora_i18n::tr!("inventory-details-in", kind = line, folder = place);
    }
    ui.label(egui::RichText::new(line).size(12.5).color(p.muted));
    if let Some(folder) = inv.folders.get(&id) {
        ui.horizontal(|ui| {
            if widgets::action_button(
                ui,
                p,
                "folder-open-filled",
                aurora_i18n::tr!("common-open"),
                widgets::ActionKind::Primary,
                Vec2::new(0.0, 28.0),
            )
            .clicked()
            {
                st.browse = folder.info.id;
                set_tab(st, prefs, 0);
            }
            if can_give(inv, &[id], st, prefs, facts)
                && widgets::action_button(ui, p, "share", "", widgets::ActionKind::Flat, Vec2::splat(28.0))
                    .on_hover_text(aurora_i18n::tr!("common-share"))
                    .clicked()
            {
                give(st, vec![id]);
            }
            more(ui, p, st, id);
        });
        widgets::divider(ui, p);
        let date = rows::short_date(st.view.folder_date(id), view::now());
        if !date.is_empty() {
            fact(ui, p, aurora_i18n::tr!("inventory-column-modified"), |ui| {
                ui.label(egui::RichText::new(date).size(12.5).color(p.ink));
            });
        }
        return;
    }
    let Some(it) = inv.items.get(&id) else { return };
    let original = rules::original(inv, id).and_then(|id| inv.items.get(&id)).unwrap_or(it);
    // The actions.
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if let Some(main) = Main::of(inv, id, facts) {
            let (icon, label) = main.label();
            let enabled = st.pending.is_none() && !(matches!(main, Main::Wear | Main::TakeOff) && facts.appearance_busy);
            let width = (ui.available_width() - 2.0 * (28.0 + 6.0) - if can_add(inv, id, facts, st) { 80.0 } else { 0.0 }).max(90.0);
            if widgets::action_button_if(ui, p, icon, label, widgets::ActionKind::Primary, Vec2::new(width, 28.0), enabled).clicked() {
                main.run(inv, id, st, prefs, facts, actions);
            }
        }
        if can_add(inv, id, facts, st)
            && widgets::action_button(
                ui,
                p,
                "",
                aurora_i18n::tr!("common-add"),
                widgets::ActionKind::Flat,
                Vec2::new(74.0, 28.0),
            )
            .clicked()
        {
            add(inv, id, actions);
        }
        if widgets::action_button_if(
            ui,
            p,
            "share",
            "",
            widgets::ActionKind::Flat,
            Vec2::splat(28.0),
            can_give(inv, &[id], st, prefs, facts),
        )
        .on_hover_text(aurora_i18n::tr!("common-share"))
        .clicked()
        {
            give(st, vec![id]);
        }
        more(ui, p, st, id);
    });
    widgets::divider(ui, p);
    // The facts.
    fact(ui, p, aurora_i18n::tr!("inventory-details-creator"), |ui| {
        resident_link(ui, p, facts, original.creator, 12.5, actions);
    });
    fact(ui, p, aurora_i18n::tr!("inventory-details-owner"), |ui| {
        let owner = resident(facts, original.owner);
        ui.label(egui::RichText::new(owner).size(12.5).color(p.ink));
    });
    fact(ui, p, aurora_i18n::tr!("inventory-details-acquired"), |ui| {
        ui.label(egui::RichText::new(properties::acquired(it.created_at)).size(12.5).color(p.ink));
    });
    if let Some(permissions) = Permissions::inventory(inv, id) {
        fact(ui, p, aurora_i18n::tr!("inventory-details-your-rights"), |ui| {
            rights::label(ui, p, permissions, Rights::Mine);
        });
    }
    widgets::divider(ui, p);
    // The next owner's rights: what a resident it is given to gets.
    heading(ui, p, aurora_i18n::tr!("inventory-details-next-owner"));
    for (bit, label) in [
        (rules::COPY, aurora_i18n::tr!("common-copy-2")),
        (rules::MODIFY, aurora_i18n::tr!("common-edit")),
        (rules::TRANSFER, aurora_i18n::tr!("common-transfer")),
    ] {
        let editable = next_editable(inv, original, bit, st, prefs) && original.id == it.id;
        ui.horizontal(|ui| {
            ui.set_min_height(22.0);
            ui.label(egui::RichText::new(label).size(12.5).color(if editable { p.ink } else { p.muted }));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let mut on = original.next_owner_mask & bit != 0;
                let changed = ui.add_enabled_ui(editable, |ui| widgets::switch(ui, p, &mut on)).inner.changed();
                if changed && editable {
                    let next = next_mask(original, bit, on);
                    set_next_owner(inv, original, next, st, actions);
                }
            });
        });
    }
    if !it.desc.is_empty() {
        widgets::divider(ui, p);
        ui.label(egui::RichText::new(&it.desc).size(12.5).color(p.muted));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::InvItem;

    fn item(asset_type: i32) -> InvItem {
        InvItem {
            id: Uuid::from_u128(asset_type as u128 + 100),
            asset_type,
            next_owner_mask: rules::COPY | rules::MODIFY | rules::TRANSFER,
            base_mask: rules::COPY | rules::MODIFY | rules::TRANSFER,
            ..Default::default()
        }
    }

    #[test]
    fn the_main_action_is_the_double_click_of_the_type() {
        let mut inv = Inventory::default();
        for t in [1, 2, 3, 5, 6, 7, 13, 21] {
            let it = item(t);
            inv.items.insert(it.id, it);
        }
        let id = |t: i32| Uuid::from_u128(t as u128 + 100);
        let mut facts = Facts::default();
        assert_eq!(Main::of(&inv, id(5), &facts), Some(Main::Wear));
        assert_eq!(Main::of(&inv, id(3), &facts), Some(Main::Teleport));
        assert_eq!(Main::of(&inv, id(1), &facts), Some(Main::Play));
        assert_eq!(Main::of(&inv, id(7), &facts), Some(Main::Open));
        assert_eq!(Main::of(&inv, id(2), &facts), Some(Main::Profile));
        assert_eq!(Main::of(&inv, id(21), &facts), Some(Main::Activate(true)));
        facts.worn.extend([id(6), id(13)]);
        facts.active_gestures.insert(id(21));
        assert_eq!(Main::of(&inv, id(6), &facts), Some(Main::TakeOff));
        assert_eq!(Main::of(&inv, id(13), &facts), None, "a worn body part is replaced, not taken off");
        assert_eq!(Main::of(&inv, id(21), &facts), Some(Main::Activate(false)));
    }

    #[test]
    fn the_next_owner_switches_keep_the_properties_rules() {
        let mut it = item(6);
        // no copy for the next owner means transfer
        let next = next_mask(&it, rules::COPY, false);
        assert_eq!(next & rules::COPY, 0);
        assert_ne!(next & rules::TRANSFER, 0);
        it.next_owner_mask = rules::MODIFY | rules::TRANSFER;
        assert_eq!(next_mask(&it, rules::MODIFY, false), rules::TRANSFER);
        // settings always copy
        let settings = item(56);
        assert_ne!(next_mask(&settings, rules::COPY, false) & rules::COPY, 0);
    }
}
