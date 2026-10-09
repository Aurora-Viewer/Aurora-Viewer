//! Contents tab (LLPanelContents + LLPanelObjectInventory,
//! indra/newview/llpanelcontents.cpp / llpanelobjectinventory.cpp,
//! originally LGPL 2.1): the object's inventory with its buttons, filter,
//! item menu, and inventory items dropped on it.

use super::common::small;
use crate::build::BuildTool;
use crate::build::contents::AT_LSL_TEXT;
use crate::theme::Palette;
use crate::ui::inventory::{InvDrag, item_icon};
use crate::world::World;
use aurora_net::build::{TaskItem, perm};
use egui::{RichText, Vec2};

/// Item name with the no-copy / no-modify / no-transfer suffixes
/// (LLTaskInvFVBridge::getLabelSuffix).
fn display_name(i: &TaskItem) -> String {
    let mut n = if i.asset_type == AT_LSL_TEXT && i.name.starts_with("New Script") {
        i.name.replacen("New Script", "Nouveau script", 1)
    } else {
        i.name.clone()
    };
    if i.owner_mask & perm::COPY == 0 {
        n.push_str(" (pas de copie)");
    }
    if i.owner_mask & perm::MODIFY == 0 {
        n.push_str(" (pas de modification)");
    }
    if i.owner_mask & perm::TRANSFER == 0 {
        n.push_str(" (pas de transfert)");
    }
    n
}

/// "Contenus (N éléments)".
fn root_label(n: usize) -> String {
    match n {
        0 => "Contenus (Aucun élément)".into(),
        1 => "Contenus (1 élément)".into(),
        n => format!("Contenus ({n} éléments)"),
    }
}

pub fn show(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World) {
    let Some(idx) = tool.contents_target(world) else { return };
    let Some(object_id) = world.objects.get(idx).map(|o| o.full_id) else {
        return;
    };
    tool.request_contents(world, idx, false);
    let editable = tool.contents_editable(world, idx);
    let single = tool.roots(world).len() == 1 || tool.sel_prims(world).len() == 1;

    ui.horizontal(|ui| {
        if ui
            .add_enabled(editable && single, egui::Button::new(RichText::new("Nouveau script").size(12.0)))
            .clicked()
        {
            tool.new_script(world, idx);
        }
        if ui
            .button(RichText::new("Droits").size(12.0))
            .on_hover_text("Changer les droits du contenu")
            .clicked()
        {
            tool.ui.bulk_perms_open = !tool.ui.bulk_perms_open;
        }
        ui.vertical(|ui| {
            if ui.small_button("Rafraîchir").clicked() {
                tool.request_contents(world, idx, true);
            }
            if ui
                .add_enabled(
                    editable && single,
                    egui::Button::new(RichText::new("Réinit. scripts").size(11.0)).small(),
                )
                .clicked()
            {
                tool.reset_scripts(world);
            }
        });
    });
    ui.add(
        egui::TextEdit::singleline(&mut tool.ui.contents_filter)
            .hint_text("Saisir le texte du filtre")
            .desired_width(ui.available_width()),
    );
    let inv = tool.contents.objects.get(&object_id).cloned().unwrap_or_default();
    let items: Vec<TaskItem> = inv.items.iter().filter(|i| !i.is_folder).cloned().collect();
    let filter = tool.ui.contents_filter.trim().to_uppercase();
    // the list is also where inventory items are dropped
    let frame = egui::Frame::new().fill(p.field).corner_radius(2).inner_margin(4);
    let (_, dropped) = ui.dnd_drop_zone::<InvDrag, _>(frame, |ui| {
        ui.set_min_size(Vec2::new(ui.available_width(), 180.0));
        let open_icon = if inv.loading && items.is_empty() { "folder" } else { "folder-open" };
        ui.horizontal(|ui| {
            if let Some(t) = crate::ui::icons::global(open_icon) {
                ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(15.0)).tint(p.violet_light));
            }
            let title = if inv.loading && inv.items.is_empty() {
                "chargement des contenus en cours...".to_owned()
            } else {
                root_label(items.len())
            };
            ui.label(RichText::new(title).size(12.0).color(p.ink))
                .on_hover_text("Contenu de l'objet");
        });
        let mut list: Vec<&TaskItem> = items
            .iter()
            .filter(|i| filter.is_empty() || display_name(i).to_uppercase().contains(&filter))
            .collect();
        list.sort_by_key(|i| i.name.to_lowercase());
        for item in list {
            item_row(ui, p, tool, world, idx, object_id, item, editable);
        }
    });
    if let Some(d) = dropped
        && let Some(item) = world.inventory.items.get(&d.0).cloned()
    {
        if editable {
            let ctrl = ui.input(|i| i.modifiers.ctrl);
            tool.drop_inventory_item(world, idx, &item, ctrl);
        } else {
            tool.status = "Vous ne pouvez pas modifier le contenu de cet objet.".into();
        }
    }
    if !editable {
        small(
            ui,
            p,
            "Glissez des éléments de l'inventaire ici pour les ajouter (objet modifiable).",
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn item_row(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    idx: usize,
    object_id: uuid::Uuid,
    item: &TaskItem,
    editable: bool,
) {
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        if let Some(t) = crate::ui::icons::global(item_icon(item.asset_type as i32, item.inv_type as i32)) {
            ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(15.0)).tint(p.ink));
        }
        // renaming in place
        if let Some((id, text)) = tool.ui.rename.as_mut()
            && *id == item.item_id
        {
            let r = ui.add(egui::TextEdit::singleline(text).desired_width(160.0).char_limit(63));
            r.request_focus();
            if r.lost_focus() {
                let (id, text) = tool.ui.rename.take().unwrap_or_default();
                if ui.input(|i| !i.key_pressed(egui::Key::Escape)) && !text.trim().is_empty() {
                    tool.rename_item(world, idx, id, text.trim());
                }
            }
            return;
        }
        let running = tool.contents.running.get(&(object_id, item.item_id)).copied();
        let mut name = display_name(item);
        if item.asset_type == AT_LSL_TEXT && running.is_some_and(|r| !r.0) {
            name.push_str(" (arrêté)");
        }
        let r = ui.add(egui::Label::new(RichText::new(name).size(12.0).color(p.ink)).sense(egui::Sense::click()));
        let r = if item.description.is_empty() {
            r
        } else {
            r.on_hover_text(&item.description)
        };
        let item_modify = item.owner_mask & perm::MODIFY != 0;
        let can_take = item.owner_mask & perm::COPY != 0 && item.owner_mask & perm::TRANSFER != 0
            || tool.props.get(&object_id).is_some_and(|pr| pr.owner_id == world.agent_id);
        r.context_menu(|ui| {
            if ui
                .add_enabled(false, egui::Button::new("Ouvrir"))
                .on_disabled_hover_text("L'ouverture des éléments d'un objet n'est pas encore disponible.")
                .clicked()
            {
                ui.close();
            }
            if ui.button("Propriétés").clicked() {
                tool.ui.item_properties = Some(item.clone());
                ui.close();
            }
            if ui.add_enabled(editable && item_modify, egui::Button::new("Renommer")).clicked() {
                tool.ui.rename = Some((item.item_id, item.name.clone()));
                ui.close();
            }
            if ui.add_enabled(editable, egui::Button::new("Supprimer")).clicked() {
                tool.remove_item(world, idx, item.item_id);
                ui.close();
            }
            if ui
                .add_enabled(can_take, egui::Button::new("Copier dans l'inventaire"))
                .on_hover_text("Comme glisser l'élément vers l'inventaire")
                .clicked()
            {
                tool.copy_item_to_inventory(world, idx, item, uuid::Uuid::nil());
                ui.close();
            }
            if item.asset_type == AT_LSL_TEXT {
                ui.separator();
                let on = running.map(|r| r.0).unwrap_or(true);
                if ui
                    .add_enabled(editable, egui::Button::new(if on { "Arrêter" } else { "Exécuter" }))
                    .clicked()
                {
                    tool.set_script_running(world, idx, item.item_id, !on);
                    ui.close();
                }
            }
        });
        if r.double_clicked() && editable && item_modify {
            tool.ui.rename = Some((item.item_id, item.name.clone()));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels() {
        assert_eq!(root_label(0), "Contenus (Aucun élément)");
        assert_eq!(root_label(3), "Contenus (3 éléments)");
        let i = TaskItem {
            name: "New Script".into(),
            asset_type: AT_LSL_TEXT,
            owner_mask: perm::COPY | perm::MODIFY,
            ..Default::default()
        };
        assert_eq!(display_name(&i), "Nouveau script (pas de transfert)");
    }
}
