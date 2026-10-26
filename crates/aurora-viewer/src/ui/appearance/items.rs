//! LLWearableItemsList::ContextMenu and LLViewerAttachMenu::populateMenus
//! (indra/newview/llwearableitemslist.cpp / llviewerattachmenu.cpp, LGPL 2.1).

use super::{Action, AppearanceUi, model};
use crate::{
    theme::Palette,
    ui::{menu, widgets},
    world::World,
};
use aurora_net::inventory::InvItem;
use egui::{RichText, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// LLPanelWearableOutfitItem::updateItem: clothing follows the confirmed COF;
/// objects are worn when the simulator reports them attached to our avatar.
pub(super) fn worn_items(world: &World) -> HashSet<Uuid> {
    let mut worn = world.worn_attachment_items();
    if let Some(cof) = model::cof(&world.inventory) {
        worn.extend(
            model::folder_links(&world.inventory, cof)
                .into_iter()
                .filter(|link| !link.folder && world.inventory.items.get(&link.target).is_some_and(|it| it.asset_type != 6))
                .map(|link| link.target),
        );
    }
    worn
}

/// LLObjectBridge::getLabelSuffix (llinventorybridge.cpp, LGPL 2.1): use
/// the simulator's current attachment point, never the item's saved flags.
pub(crate) fn worn_labels(world: &World) -> HashMap<Uuid, String> {
    let mut labels: HashMap<_, _> = worn_items(world)
        .into_iter()
        .map(|id| (id, aurora_i18n::tr!("common-worn").into()))
        .collect();
    for (id, point) in world.worn_attachment_points() {
        let name = world
            .avatar_lib
            .attach_names
            .get(&point)
            .map(|name| point_name(name).to_owned())
            .unwrap_or_else(|| aurora_i18n::tr!("attachment-point", point = point));
        let hud = world.avatar_lib.attach_points.get(&point).is_some_and(|ap| ap.hud);
        labels.insert(
            id,
            aurora_i18n::tr!(
                "ui-appearance-items-worn-on",
                value0 = (if hud { "HUD : " } else { "" }).to_string(),
                name = name.to_string()
            ),
        );
    }
    labels
}

pub(super) fn context_menu(
    response: &egui::Response,
    p: &Palette,
    world: &World,
    st: &mut AppearanceUi,
    folder: Uuid,
    it: &InvItem,
    actions: &mut Vec<Action>,
) {
    menu::context_menu(response, p, |ui| {
        let worn = worn_items(world).contains(&it.id);
        let wearable = matches!(it.asset_type, 5 | 6 | 13);
        let editable = matches!(it.asset_type, 5 | 13);
        if worn && editable && menu::item(ui, p, "pencil-filled", aurora_i18n::tr!("common-edit")) {
            crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::EditWorn(it.id));
        }
        if !worn && wearable {
            if menu::item(ui, p, "shirt-filled", aurora_i18n::tr!("common-wear")) {
                actions.push(Action::WearItem {
                    item: it.id,
                    replace: true,
                    point: 0,
                });
            }
            if it.asset_type != 13 && menu::item(ui, p, "plus", aurora_i18n::tr!("common-add")) {
                actions.push(Action::WearItem {
                    item: it.id,
                    replace: false,
                    point: 0,
                });
            }
            if it.asset_type == 6 {
                attach_menu(ui, p, world, it.id, false, actions);
                attach_menu(ui, p, world, it.id, true, actions);
            }
        }
        // "touch" / "edit" of a worn object (updateItemsVisibility); editing
        // worn clothing waits for the appearance editor
        if worn && it.asset_type == 6 {
            let touch = world.touchable_attachment_items().contains(&it.id);
            if menu::item_if(ui, p, "click-filled", aurora_i18n::tr!("ui-context-touch"), touch) {
                crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::TouchWorn(it.id));
            }
            let edit = world.worn_attachment(&it.id).is_some();
            if menu::item_if(ui, p, "pencil-filled", aurora_i18n::tr!("common-edit"), edit) {
                crate::ui::context::request(ui.ctx(), crate::ui::context::CtxAction::EditWorn(it.id));
            }
        }
        if worn
            && it.asset_type != 13
            && menu::item(
                ui,
                p,
                "circle-x-filled",
                if it.asset_type == 6 {
                    aurora_i18n::tr!("common-detach")
                } else {
                    aurora_i18n::tr!("ui-appearance-items-remove")
                },
            )
        {
            actions.push(Action::Remove(it.id));
        }
        if menu::item(ui, p, "info-circle-filled", aurora_i18n::tr!("ui-appearance-items-object-profile")) {
            st.profile_item = Some(it.id);
        }
        menu::separator(ui, p);
        if menu::item_if(
            ui,
            p,
            "star-filled",
            if it.favorite {
                aurora_i18n::tr!("common-remove-from-favorites")
            } else {
                aurora_i18n::tr!("common-add-to-favorites")
            },
            !st.favorite_pending.contains(&it.id),
        ) {
            actions.push(Action::Favorite(it.id));
        }
        if menu::item(ui, p, "external-link-filled", aurora_i18n::tr!("ui-appearance-items-show-original")) {
            actions.push(Action::ShowOriginal(it.id));
        }
        if menu::item_if(
            ui,
            p,
            "trash-filled",
            aurora_i18n::tr!("ui-appearance-items-remove-from-outfit"),
            st.pending.is_none(),
        ) {
            actions.push(Action::DeleteFromOutfit { folder, item: it.id });
        }
    });
}

fn attach_menu(ui: &mut egui::Ui, p: &Palette, world: &World, item: Uuid, hud: bool, actions: &mut Vec<Action>) {
    let mut points: Vec<_> = world.avatar_lib.attach_points.iter().filter(|(_, ap)| ap.hud == hud).collect();
    points.sort_by_cached_key(|(id, _)| {
        world
            .avatar_lib
            .attach_names
            .get(id)
            .map(|name| point_name(name).to_lowercase())
            .unwrap_or_default()
    });
    menu::submenu(
        ui,
        p,
        if hud { "device-desktop-filled" } else { "man-filled" },
        if hud {
            aurora_i18n::tr!("common-attach-to-hud")
        } else {
            aurora_i18n::tr!("common-attach-to")
        },
        !points.is_empty(),
        |ui| {
            egui::ScrollArea::vertical().max_height(500.0).show(ui, |ui| {
                for (id, _) in points {
                    let name = world
                        .avatar_lib
                        .attach_names
                        .get(id)
                        .map(String::as_str)
                        .unwrap_or(aurora_i18n::tr!("common-attachment"));
                    let name = point_name(name);
                    let label = if hud { name.to_owned() } else { format!("{name} ({id})") };
                    if menu::item(ui, p, if hud { "device-desktop-filled" } else { "man-filled" }, &label) {
                        // Firestorm's Attach To always adds, even on an occupied point.
                        actions.push(Action::WearItem {
                            item,
                            replace: false,
                            point: *id,
                        });
                    }
                }
            });
        },
    );
}

pub(crate) fn point_name(name: &str) -> &str {
    match name {
        "Chest" => aurora_i18n::tr!("ui-appearance-items-chest-2"),
        "Skull" => aurora_i18n::tr!("ui-appearance-items-skull"),
        "Left Shoulder" => aurora_i18n::tr!("ui-appearance-items-left-shoulder-2"),
        "Right Shoulder" => aurora_i18n::tr!("ui-appearance-items-right-shoulder-2"),
        "Left Hand" => aurora_i18n::tr!("ui-appearance-items-left-hand-2"),
        "Right Hand" => aurora_i18n::tr!("ui-appearance-items-right-hand"),
        "Left Foot" => aurora_i18n::tr!("ui-appearance-items-left-foot-2"),
        "Right Foot" => aurora_i18n::tr!("ui-appearance-items-right-foot"),
        "Spine" => aurora_i18n::tr!("ui-appearance-items-spine"),
        "Pelvis" => aurora_i18n::tr!("ui-appearance-items-pelvis"),
        "Mouth" => aurora_i18n::tr!("ui-appearance-items-mouth"),
        "Chin" => aurora_i18n::tr!("ui-appearance-items-chin-2"),
        "Left Ear" => aurora_i18n::tr!("ui-appearance-items-left-ear-2"),
        "Right Ear" => aurora_i18n::tr!("ui-appearance-items-right-ear"),
        "Left Eyeball" => aurora_i18n::tr!("ui-appearance-items-left-eyeball"),
        "Right Eyeball" => aurora_i18n::tr!("ui-appearance-items-right-eyeball"),
        "Nose" => aurora_i18n::tr!("ui-appearance-items-nose"),
        "R Upper Arm" => aurora_i18n::tr!("ui-appearance-items-right-upper-arm"),
        "R Forearm" => aurora_i18n::tr!("ui-appearance-items-right-forearm"),
        "L Upper Arm" => aurora_i18n::tr!("ui-appearance-items-left-upper-arm"),
        "L Forearm" => aurora_i18n::tr!("ui-appearance-items-left-forearm"),
        "Right Hip" => aurora_i18n::tr!("ui-appearance-items-right-hip"),
        "R Upper Leg" => aurora_i18n::tr!("ui-appearance-items-right-upper-leg"),
        "R Lower Leg" => aurora_i18n::tr!("ui-appearance-items-right-lower-leg"),
        "Left Hip" => aurora_i18n::tr!("ui-appearance-items-left-hip"),
        "L Upper Leg" => aurora_i18n::tr!("ui-appearance-items-left-upper-leg"),
        "L Lower Leg" => aurora_i18n::tr!("ui-appearance-items-left-lower-leg"),
        "Stomach" => aurora_i18n::tr!("ui-appearance-items-stomach"),
        "Left Pec" => aurora_i18n::tr!("ui-appearance-items-left-pec-2"),
        "Right Pec" => aurora_i18n::tr!("ui-appearance-items-right-pec"),
        "Neck" => aurora_i18n::tr!("ui-appearance-items-neck"),
        "Avatar Center" => aurora_i18n::tr!("ui-appearance-items-avatar-center-2"),
        "Left Ring Finger" => aurora_i18n::tr!("ui-appearance-items-left-ring-finger"),
        "Right Ring Finger" => aurora_i18n::tr!("ui-appearance-items-right-ring-finger"),
        "Tail Base" => aurora_i18n::tr!("ui-appearance-items-tail-base"),
        "Tail Tip" => aurora_i18n::tr!("ui-appearance-items-tail-tip"),
        "Left Wing" => aurora_i18n::tr!("ui-appearance-items-left-wing"),
        "Right Wing" => aurora_i18n::tr!("ui-appearance-items-right-wing"),
        "Jaw" => aurora_i18n::tr!("ui-appearance-items-jaw-2"),
        "Alt Left Ear" => aurora_i18n::tr!("ui-appearance-items-alt-left-ear-2"),
        "Alt Right Ear" => aurora_i18n::tr!("ui-appearance-items-alt-right-ear-2"),
        "Alt Left Eye" => aurora_i18n::tr!("ui-appearance-items-left-eye-alt"),
        "Alt Right Eye" => aurora_i18n::tr!("ui-appearance-items-right-eye-alt"),
        "Tongue" => aurora_i18n::tr!("ui-appearance-items-tongue"),
        "Groin" => aurora_i18n::tr!("ui-appearance-items-groin"),
        "Left Hind Foot" => aurora_i18n::tr!("ui-appearance-items-left-hind-foot-2"),
        "Right Hind Foot" => aurora_i18n::tr!("ui-appearance-items-right-hind-foot"),
        "Center 2" => aurora_i18n::tr!("ui-appearance-items-center-2-2"),
        "Top Right" => aurora_i18n::tr!("ui-appearance-items-top-right"),
        "Top" => aurora_i18n::tr!("ui-appearance-items-top"),
        "Top Left" => aurora_i18n::tr!("ui-appearance-items-top-left"),
        "Center" => aurora_i18n::tr!("ui-appearance-items-center-3"),
        "Bottom Left" => aurora_i18n::tr!("ui-appearance-items-bottom-left-2"),
        "Bottom" => aurora_i18n::tr!("ui-appearance-items-bottom-2"),
        "Bottom Right" => aurora_i18n::tr!("ui-appearance-items-bottom-right-2"),
        _ => name,
    }
}

pub(super) fn profile(ctx: &egui::Context, p: &Palette, world: &World, st: &mut AppearanceUi, actions: &mut Vec<Action>) {
    let Some(item) = st.profile_item.and_then(|id| world.inventory.items.get(&id)) else {
        return;
    };
    let mut open = true;
    widgets::Floater::new(
        "appearance_item_profile",
        aurora_i18n::tr!("ui-appearance-items-object-profile"),
        egui::pos2(430.0, 90.0),
        Vec2::new(370.0, 260.0),
    )
    .fixed()
    .show(ctx, p, &mut open, |ui| {
        ui.set_width(370.0);
        ui.add(egui::Label::new(RichText::new(&item.name).size(17.0).strong()).wrap());
        if !item.desc.is_empty() {
            ui.add(egui::Label::new(&item.desc).wrap());
        }
        ui.add_space(8.0);
        for (label, id) in [
            (aurora_i18n::tr!("common-creator"), item.creator),
            (aurora_i18n::tr!("common-owner"), item.owner),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                if !id.is_nil()
                    && ui
                        .link(
                            world
                                .person_name(&id)
                                .unwrap_or_else(|| aurora_i18n::tr!("ui-appearance-items-show-profile").into()),
                        )
                        .clicked()
                {
                    crate::ui::profile::request_open(ctx, id);
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("ui-appearance-items-next-owner-permissions"));
        });
        ui.label(format!(
            "{} · {} · {}",
            if item.next_owner_mask & 0x8000 != 0 {
                aurora_i18n::tr!("common-copy")
            } else {
                aurora_i18n::tr!("ui-appearance-items-no-copy")
            },
            if item.next_owner_mask & 0x4000 != 0 {
                aurora_i18n::tr!("ui-appearance-items-modify")
            } else {
                aurora_i18n::tr!("ui-appearance-items-no-modify")
            },
            if item.next_owner_mask & 0x2000 != 0 {
                aurora_i18n::tr!("ui-appearance-items-transfer")
            } else {
                aurora_i18n::tr!("ui-appearance-items-no-transfer")
            }
        ));
        ui.add_space(8.0);
        if widgets::flat_button(ui, p, aurora_i18n::tr!("ui-appearance-items-show-original")).clicked() {
            actions.push(Action::ShowOriginal(item.id));
        }
    });
    if !open {
        st.profile_item = None;
    }
}
