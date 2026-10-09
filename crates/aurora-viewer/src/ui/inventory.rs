//! Inventory window: lazily fetched folder tree, type icons, search.

use super::icons::Icons;
use crate::theme::Palette;
use crate::world::inventory::{FetchState, Inventory};
use egui::{RichText, Vec2};
use std::collections::HashSet;
use uuid::Uuid;

pub enum InvAction {
    TeleportLandmark(Uuid),
}

/// An inventory item being dragged (dropped on an object's contents in
/// the build floater, like LLToolDragAndDrop).
#[derive(Debug, Clone, Copy)]
pub struct InvDrag(pub Uuid);

#[derive(Default)]
pub struct InventoryUi {
    pub search: String,
    pub tab: usize,
    pub expand_all: Option<bool>,
    pub selected: Option<Uuid>,
    reveal: Option<Uuid>,
    reveal_path: HashSet<Uuid>,
}

impl InventoryUi {
    /// show_item_original: clear filters, expand ancestors, select and scroll to the original.
    pub fn show_original(&mut self, inv: &Inventory, id: Uuid) {
        self.search.clear();
        self.tab = 0;
        self.expand_all = None;
        self.selected = Some(id);
        self.reveal = Some(id);
        self.reveal_path.clear();
        let mut parent = inv.items.get(&id).map(|it| it.parent);
        while let Some(id) = parent {
            if !self.reveal_path.insert(id) {
                break;
            }
            let Some(folder) = inv.folders.get(&id) else {
                break;
            };
            if folder.library {
                self.tab = 1;
            }
            parent = (!folder.info.parent.is_nil()).then_some(folder.info.parent);
        }
    }
}

fn folder_icon(type_default: i32, open: bool) -> &'static str {
    match type_default {
        14 => "Inv_TrashClosed",
        8 | 9 => "Inv_FolderOpen",
        _ if open => "Inv_FolderOpen",
        _ => "Inv_FolderClosed",
    }
}

pub fn item_icon(asset_type: i32, inv_type: i32) -> &'static str {
    match (asset_type, inv_type) {
        (_, 15) => "Inv_Snapshot",
        (0, _) => "Inv_Texture",
        (1, _) => "Inv_Sound",
        (2, _) => "Inv_CallingCard",
        (3, _) => "Inv_Landmark",
        (5, _) => "Inv_Clothing",
        (6, _) => "Inv_Object",
        (7, _) => "Inv_Notecard",
        (10, _) => "Inv_Script",
        (13, _) => "Inv_BodyShape",
        (20, _) => "Inv_Animation",
        (21, _) => "Inv_Gesture",
        (24, _) => "Inv_LinkItem",
        (25, _) => "Inv_LinkFolder",
        (49, _) => "Inv_Mesh",
        (56, _) => "Inv_Settings",
        (57, _) => "Inv_Material",
        _ => "Inv_Invalid",
    }
}

fn icon(ui: &mut egui::Ui, icons: &Icons, name: &str, p: &Palette) {
    if let Some(t) = icons.get(name) {
        let tint = if name.starts_with("Inv_Folder") { p.violet_light } else { p.ink };
        ui.add(egui::Image::new(t).fit_to_exact_size(Vec2::splat(16.0)).tint(tint));
    } else {
        ui.add_space(16.0);
    }
}

fn item_row(ui: &mut egui::Ui, p: &Palette, icons: &Icons, inv: &Inventory, id: &Uuid, st: &mut InventoryUi, actions: &mut Vec<InvAction>) {
    let Some(it) = inv.items.get(id) else {
        return;
    };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        icon(ui, icons, item_icon(it.asset_type, it.inv_type), p);
        let r = ui
            .dnd_drag_source(egui::Id::new(("inv_drag", it.id)), InvDrag(it.id), |ui| {
                ui.selectable_label(st.selected == Some(it.id), RichText::new(&it.name).size(13.0).color(p.ink))
            })
            .inner;
        let r = if it.desc.is_empty() { r } else { r.on_hover_text(&it.desc) };
        if r.clicked() {
            st.selected = Some(it.id);
        }
        if st.reveal == Some(it.id) {
            r.scroll_to_me(Some(egui::Align::Center));
            st.reveal = None;
            st.reveal_path.clear();
        }
        if it.asset_type == 3 && !it.asset_id.is_nil() {
            if r.double_clicked() {
                actions.push(InvAction::TeleportLandmark(it.asset_id));
            }
            // the landmark part of menu_inventory.xml
            super::menu::context_menu(&r, p, |ui| {
                if super::menu::item(ui, p, "navigation-arrow", "Se téléporter") {
                    actions.push(InvAction::TeleportLandmark(it.asset_id));
                }
                super::menu::todo(ui, p, "info", "À propos du repère");
                super::menu::todo(ui, p, "map-trifold", "Afficher sur la carte");
                super::menu::separator(ui, p);
                super::menu::todo(ui, p, "copy", "Copier le SLurl");
                super::menu::todo(ui, p, "pencil-simple", "Renommer");
                super::menu::todo(ui, p, "trash", "Supprimer");
            });
        }
    });
}

fn folder_tree(
    ui: &mut egui::Ui,
    p: &Palette,
    icons: &Icons,
    inv: &mut Inventory,
    id: Uuid,
    depth: usize,
    expand: Option<bool>,
    state: &mut InventoryUi,
    actions: &mut Vec<InvAction>,
) {
    if depth > 24 {
        return;
    }
    let Some(f) = inv.folders.get(&id) else {
        return;
    };
    let name = f.info.name.clone();
    let tdef = f.info.type_default;
    let fetch_state = f.state;
    let children = f.children.clone();
    let items = f.items.clone();
    let id_salt = egui::Id::new(("inv", id));
    let mut st = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id_salt, depth == 0);
    if state.reveal_path.contains(&id) {
        st.set_open(true);
    }
    if let Some(e) = expand {
        // "Développer" only opens folders already fetched, to avoid fetching everything.
        if !e || fetch_state == FetchState::Fetched || depth == 0 {
            st.set_open(e || depth == 0);
        }
    }
    let open = st.is_open();
    let header = ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (r, resp) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::click());
        let c = r.center();
        let pts = if open {
            vec![c + egui::vec2(-4.0, -2.0), c + egui::vec2(4.0, -2.0), c + egui::vec2(0.0, 3.0)]
        } else {
            vec![c + egui::vec2(-2.0, -4.0), c + egui::vec2(3.0, 0.0), c + egui::vec2(-2.0, 4.0)]
        };
        ui.painter().add(egui::Shape::convex_polygon(pts, p.muted, egui::Stroke::NONE));
        if resp.clicked() {
            st.toggle(ui);
        }
        icon(ui, icons, folder_icon(tdef, open), p);
        let label = match fetch_state {
            FetchState::Fetching => format!("{name}  (chargement…)"),
            FetchState::Failed => format!("{name}  (échec)"),
            _ => name.clone(),
        };
        if ui
            .add(egui::Label::new(RichText::new(label).size(13.0).color(p.ink)).sense(egui::Sense::click()))
            .clicked()
        {
            st.toggle(ui);
        }
    });
    let _ = header;
    if st.is_open() && matches!(fetch_state, FetchState::Unknown | FetchState::Failed) {
        inv.request(id);
    }
    st.store(ui.ctx());
    if st.is_open() {
        ui.indent(id_salt, |ui| {
            for c in children {
                folder_tree(ui, p, icons, inv, c, depth + 1, expand, state, actions);
            }
            for i in &items {
                item_row(ui, p, icons, inv, i, state, actions);
            }
            if items.is_empty() && fetch_state == FetchState::Fetched && inv.folders.get(&id).is_some_and(|f| f.children.is_empty()) {
                ui.label(RichText::new("(vide)").size(12.0).color(p.muted_dim));
            }
        });
    }
}

pub fn show(ctx: &egui::Context, p: &Palette, icons: &Icons, inv: &mut Inventory, st: &mut InventoryUi, open: &mut bool) -> Vec<InvAction> {
    let mut actions = Vec::new();
    let screen = ctx.content_rect();
    super::widgets::Floater::new(
        "inventory",
        "Inventaire",
        egui::pos2(screen.right() - 390.0, 56.0),
        Vec2::new(370.0, 420.0),
    )
    .help("Double-clic sur un repère pour s'y téléporter")
    .show(ctx, p, open, |ui| {
        super::widgets::search_field(ui, &mut st.search, "Filtrer l'inventaire", ui.available_width());
        ui.add_space(3.0);
        ui.horizontal(|ui| {
            if super::widgets::flat_button(ui, p, "Réduire").clicked() {
                st.expand_all = Some(false);
            }
            if super::widgets::flat_button(ui, p, "Développer").clicked() {
                st.expand_all = Some(true);
            }
        });
        ui.add_space(3.0);
        super::widgets::tabs(
            ui,
            p,
            &mut st.tab,
            &[("Inventaire", true), ("Bibliothèque", true), ("Récent", false), ("Porté", false)],
        );
        ui.add_space(3.0);
        let list_h = (ui.available_height() - 48.0).max(80.0);
        egui::Frame::new().fill(p.field).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .max_height(list_h)
                .min_scrolled_height(list_h)
                .show(ui, |ui| {
                    let q = st.search.trim().to_lowercase();
                    if !q.is_empty() {
                        let mut hits: Vec<Uuid> = inv
                            .items
                            .values()
                            .filter(|i| i.name.to_lowercase().contains(&q))
                            .map(|i| i.id)
                            .collect();
                        hits.sort_by_key(|i| inv.items.get(i).map(|x| x.name.to_lowercase()));
                        let total = hits.len();
                        for id in hits.iter().take(500) {
                            item_row(ui, p, icons, inv, id, st, &mut actions);
                        }
                        if total > 500 {
                            ui.label(RichText::new(format!("… {} autres", total - 500)).color(p.muted));
                        }
                        if total == 0 {
                            ui.label(
                                RichText::new("Aucun résultat parmi les dossiers déjà chargés.")
                                    .size(12.0)
                                    .color(p.muted),
                            );
                        }
                        return;
                    }
                    let root = if st.tab == 1 { inv.lib_root } else { inv.root };
                    if root.is_nil() {
                        ui.label(RichText::new("Inventaire non disponible.").color(p.muted));
                    } else {
                        let expand = st.expand_all.take();
                        folder_tree(ui, p, icons, inv, root, 0, expand, st, &mut actions);
                    }
                });
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} objets", inv.item_count())).size(12.0).color(p.muted));
        });
    });
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn showing_an_original_clears_filters_and_expands_its_ancestors() {
        let mut inv = Inventory::default();
        crate::world::appearance::seed_demo(&mut inv, Uuid::from_u128(1));
        let original = Uuid::from_u128(715);
        let mut parent = inv.folders[&Uuid::from_u128(704)].clone();
        parent.info.parent = inv.root;
        inv.items.get_mut(&original).expect("original").parent = parent.info.id;
        let parent_id = parent.info.id;
        inv.folders.insert(parent_id, parent);
        let mut st = InventoryUi {
            search: "ancien filtre".into(),
            tab: 1,
            expand_all: Some(false),
            ..Default::default()
        };
        st.show_original(&inv, original);
        assert!(st.search.is_empty());
        assert_eq!(st.tab, 0);
        assert_eq!(st.selected, Some(original));
        assert_eq!(st.reveal, Some(original));
        assert!(st.reveal_path.contains(&parent_id));
        assert!(st.expand_all.is_none());
    }
}
