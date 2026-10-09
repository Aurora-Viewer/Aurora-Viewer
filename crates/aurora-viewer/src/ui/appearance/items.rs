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
use std::collections::HashSet;
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
        if !worn && wearable {
            if menu::item(ui, p, "t-shirt", "Porter") {
                actions.push(Action::WearItem {
                    item: it.id,
                    replace: true,
                    point: 0,
                });
            }
            if it.asset_type != 13 && menu::item(ui, p, "plus", "Ajouter") {
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
        if worn && it.asset_type != 13 && menu::item(ui, p, "x-circle", if it.asset_type == 6 { "Détacher" } else { "Retirer" }) {
            actions.push(Action::Remove(it.id));
        }
        if menu::item(ui, p, "info", "Profil de l’objet") {
            st.profile_item = Some(it.id);
        }
        menu::separator(ui, p);
        if menu::item_if(
            ui,
            p,
            "star",
            if it.favorite {
                "Supprimer des favoris"
            } else {
                "Ajouter aux favoris"
            },
            !st.favorite_pending.contains(&it.id),
        ) {
            actions.push(Action::Favorite(it.id));
        }
        if menu::item(ui, p, "arrow-square-out", "Afficher l’original") {
            actions.push(Action::ShowOriginal(it.id));
        }
        if menu::item_if(ui, p, "trash", "Supprimer de la tenue", st.pending.is_none()) {
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
        if hud { "monitor" } else { "person" },
        if hud { "Attacher au HUD" } else { "Attacher à" },
        !points.is_empty(),
        |ui| {
            egui::ScrollArea::vertical().max_height(500.0).show(ui, |ui| {
                for (id, _) in points {
                    let name = world.avatar_lib.attach_names.get(id).map(String::as_str).unwrap_or("Attachment");
                    let name = point_name(name);
                    let label = if hud { name.to_owned() } else { format!("{name} ({id})") };
                    if menu::item(ui, p, if hud { "monitor" } else { "person" }, &label) {
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
        "Chest" => "Poitrine",
        "Skull" => "Crâne",
        "Left Shoulder" => "Épaule gauche",
        "Right Shoulder" => "Épaule droite",
        "Left Hand" => "Main gauche",
        "Right Hand" => "Main droite",
        "Left Foot" => "Pied gauche",
        "Right Foot" => "Pied droit",
        "Spine" => "Colonne",
        "Pelvis" => "Bassin",
        "Mouth" => "Bouche",
        "Chin" => "Menton",
        "Left Ear" => "Oreille gauche",
        "Right Ear" => "Oreille droite",
        "Left Eyeball" => "Globe oculaire gauche",
        "Right Eyeball" => "Globe oculaire droit",
        "Nose" => "Nez",
        "R Upper Arm" => "Bras droit",
        "R Forearm" => "Avant-bras droit",
        "L Upper Arm" => "Bras gauche",
        "L Forearm" => "Avant-bras gauche",
        "Right Hip" => "Hanche droite",
        "R Upper Leg" => "Cuisse droite",
        "R Lower Leg" => "Jambe droite",
        "Left Hip" => "Hanche gauche",
        "L Upper Leg" => "Cuisse gauche",
        "L Lower Leg" => "Jambe gauche",
        "Stomach" => "Estomac",
        "Left Pec" => "Pectoral gauche",
        "Right Pec" => "Pectoral droit",
        "Neck" => "Cou",
        "Avatar Center" => "Centre de l’avatar",
        "Left Ring Finger" => "Annulaire gauche",
        "Right Ring Finger" => "Annulaire droit",
        "Tail Base" => "Base de la queue",
        "Tail Tip" => "Extrémité de la queue",
        "Left Wing" => "Aile gauche",
        "Right Wing" => "Aile droite",
        "Jaw" => "Mâchoire",
        "Alt Left Ear" => "Oreille gauche (Alt)",
        "Alt Right Ear" => "Oreille droite (Alt)",
        "Alt Left Eye" => "Œil gauche (Alt)",
        "Alt Right Eye" => "Œil droit (Alt)",
        "Tongue" => "Langue",
        "Groin" => "Aine",
        "Left Hind Foot" => "Pied arrière gauche",
        "Right Hind Foot" => "Pied arrière droit",
        "Center 2" => "Centre 2",
        "Top Right" => "En haut à droite",
        "Top" => "En haut",
        "Top Left" => "En haut à gauche",
        "Center" => "Centre",
        "Bottom Left" => "En bas à gauche",
        "Bottom" => "En bas",
        "Bottom Right" => "En bas à droite",
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
        "Profil de l’objet",
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
        for (label, id) in [("Créateur", item.creator), ("Propriétaire", item.owner)] {
            ui.horizontal(|ui| {
                ui.label(label);
                if !id.is_nil()
                    && ui
                        .link(world.person_name(&id).unwrap_or_else(|| "Afficher le profil".into()))
                        .clicked()
                {
                    crate::ui::profile::request_open(ctx, id);
                }
            });
        }
        ui.horizontal(|ui| {
            ui.label("Permissions du prochain propriétaire :");
        });
        ui.label(format!(
            "{} · {} · {}",
            if item.next_owner_mask & 0x8000 != 0 {
                "Copie"
            } else {
                "Sans copie"
            },
            if item.next_owner_mask & 0x4000 != 0 {
                "Modification"
            } else {
                "Sans modification"
            },
            if item.next_owner_mask & 0x2000 != 0 {
                "Transfert"
            } else {
                "Sans transfert"
            }
        ));
        ui.add_space(8.0);
        if widgets::flat_button(ui, p, "Afficher l’original").clicked() {
            actions.push(Action::ShowOriginal(item.id));
        }
    });
    if !open {
        st.profile_item = None;
    }
}
