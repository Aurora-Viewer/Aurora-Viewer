//! Appearance floater, following LLSidepanelAppearance, LLPanelOutfitsInventory,
//! LLOutfitGallery and LLPanelOutfitEdit (Firestorm indra/newview, LGPL 2.1).

use super::{icons, widgets};
use crate::{
    theme::Palette,
    world::{World, appearance as model},
};
use aurora_net::inventory::InvItem;
use egui::{RichText, Vec2};
use model::Action;
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Default)]
pub struct AppearanceUi {
    pub tab: usize,
    pub editing: bool,
    edit_tab: usize,
    search: String,
    selected_outfit: Option<Uuid>,
    selected_item: Option<Uuid>,
    pub save_as: Option<String>,
    select_name: bool,
    adding: bool,
    add_search: String,
    reverse_sort: bool,
    pub pending: Option<(Uuid, bool)>,
    pub saved_new: bool,
    pub message: String,
    requested_items: HashSet<Uuid>,
}

impl AppearanceUi {
    pub fn open(&mut self, tab: usize, editing: bool) {
        self.tab = tab.min(2);
        self.editing = editing;
        self.adding = false;
    }

    pub fn begin_save_as(&mut self, inv: &crate::world::inventory::Inventory) {
        let name = model::base(inv)
            .and_then(|id| inv.folders.get(&id))
            .map(|f| format!("{} (nouv.)", f.info.name))
            .unwrap_or_else(|| "Nouvelle tenue".into());
        self.save_as = Some(name.chars().take(63).collect());
        self.select_name = true;
    }

    pub fn prepare(
        &mut self,
        inv: &mut crate::world::inventory::Inventory,
        agent: Uuid,
        worn: &HashSet<Uuid>,
    ) -> Vec<aurora_net::NetCommand> {
        model::prepare(inv, agent, worn)
            .into_iter()
            .filter_map(|cmd| {
                if let aurora_net::NetCommand::FetchItems { items, owner } = cmd {
                    let items: Vec<_> = items.into_iter().filter(|id| self.requested_items.insert(*id)).collect();
                    (!items.is_empty()).then_some(aurora_net::NetCommand::FetchItems { items, owner })
                } else {
                    Some(cmd)
                }
            })
            .collect()
    }

    pub fn refresh(&mut self, inv: &mut crate::world::inventory::Inventory) {
        self.requested_items.clear();
        for f in inv.folders.values_mut().filter(|f| matches!(f.info.type_default, 46..=48)) {
            f.state = crate::world::inventory::FetchState::Unknown;
        }
    }
}

fn icon(ui: &mut egui::Ui, name: &str, size: f32, color: egui::Color32) {
    if let Some(t) = icons::global(name) {
        ui.add(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(size)).tint(color));
    } else {
        ui.add_space(size);
    }
}

fn icon_button(ui: &mut egui::Ui, p: &Palette, name: &str, tip: &str) -> egui::Response {
    let button = if let Some(t) = icons::global(name) {
        egui::Button::image(egui::Image::new(&t).fit_to_exact_size(Vec2::splat(16.0)).tint(p.ink))
    } else {
        egui::Button::new("·")
    };
    ui.scope(|ui| {
        ui.spacing_mut().button_padding = Vec2::splat(4.0);
        ui.add_sized(Vec2::splat(28.0), button.fill(p.raised))
    })
    .inner
    .on_hover_text(tip)
}

fn icon_menu(ui: &mut egui::Ui, p: &Palette, name: &str, tip: &str, body: impl FnOnce(&mut egui::Ui)) {
    let response = icon_button(ui, p, name, tip);
    egui::Popup::menu(&response).show(body);
}

fn row(ui: &mut egui::Ui, p: &Palette, it: &InvItem, desc: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 25.0), egui::Sense::click());
    if selected || response.hovered() {
        ui.painter()
            .rect_filled(rect, 2.0, if selected { p.violet.gamma_multiply(0.25) } else { p.raised });
    }
    let (name, color) = match it.asset_type {
        6 => ("cube", p.amber),
        13 => ("person", p.violet_light),
        5 => ("t-shirt", p.rose),
        21 => ("hand-waving", p.teal),
        _ => ("link", p.muted),
    };
    if let Some(t) = icons::global(name) {
        ui.painter().image(
            t.id(),
            egui::Rect::from_center_size(egui::pos2(rect.left() + 11.0, rect.center().y), Vec2::splat(16.0)),
            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            color,
        );
    }
    let text = if desc.is_empty() {
        it.name.clone()
    } else {
        format!("{} {desc}", it.name)
    };
    let galley = ui.painter().layout(
        text.clone(),
        egui::FontId::proportional(12.0),
        p.ink,
        (rect.width() - 30.0).max(20.0),
    );
    let text_rect = egui::Rect::from_min_max(egui::pos2(rect.left() + 25.0, rect.top()), rect.right_bottom());
    ui.painter().with_clip_rect(text_rect).galley(
        egui::pos2(rect.left() + 25.0, rect.center().y - galley.size().y.min(16.0) * 0.5),
        galley,
        p.ink,
    );
    response.on_hover_text(text)
}

fn item_menu(response: &egui::Response, it: &InvItem, actions: &mut Vec<Action>) {
    response.context_menu(|ui| {
        if it.asset_type != 13 && ui.button(if it.asset_type == 6 { "Détacher" } else { "Enlever" }).clicked() {
            actions.push(Action::Remove(it.id));
            ui.close();
        }
    });
}

fn gallery(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut AppearanceUi, outfits: &[Uuid], actions: &mut Vec<Action>) {
    let width = ((ui.available_width() - 10.0) * 0.5).max(90.0);
    for pair in outfits.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for id in pair {
                let Some(folder) = world.inventory.folders.get(id) else {
                    continue;
                };
                ui.vertical(|ui| {
                    ui.set_width(width);
                    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, width * 0.88), egui::Sense::click());
                    let selected = st.selected_outfit == Some(*id);
                    ui.painter().rect_filled(rect, 3.0, p.field);
                    ui.painter().rect_stroke(
                        rect,
                        3.0,
                        egui::Stroke::new(1.0, if selected { p.violet } else { p.raised }),
                        egui::StrokeKind::Inside,
                    );
                    if let Some(t) = icons::global("coat-hanger") {
                        ui.painter().image(
                            t.id(),
                            egui::Rect::from_center_size(rect.center(), Vec2::splat(width * 0.55)),
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            p.violet_light,
                        );
                    }
                    if response.clicked() {
                        st.selected_outfit = Some(*id);
                    }
                    if response.double_clicked() {
                        actions.push(Action::Wear(*id, false));
                    }
                    response.context_menu(|ui| {
                        if ui.button("Porter").clicked() {
                            actions.push(Action::Wear(*id, false));
                            ui.close();
                        }
                        if ui.button("Ajouter à la tenue").clicked() {
                            actions.push(Action::Wear(*id, true));
                            ui.close();
                        }
                    });
                    let current = model::base(&world.inventory) == Some(*id);
                    let text = if current {
                        format!("{} (portée)", folder.info.name)
                    } else {
                        folder.info.name.clone()
                    };
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(if current { p.violet_light } else { p.ink })).truncate());
                });
            }
        });
        ui.add_space(12.0);
    }
}

fn outfit_list(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut AppearanceUi, outfits: &[Uuid], actions: &mut Vec<Action>) {
    for id in outfits {
        let Some(f) = world.inventory.folders.get(id) else {
            continue;
        };
        let label = if model::base(&world.inventory) == Some(*id) {
            format!("{} (portée)", f.info.name)
        } else {
            f.info.name.clone()
        };
        egui::CollapsingHeader::new(RichText::new(label).color(p.ink))
            .id_salt(("outfit", id))
            .show(ui, |ui| {
                if ui
                    .selectable_label(st.selected_outfit == Some(*id), "Sélectionner cette tenue")
                    .clicked()
                {
                    st.selected_outfit = Some(*id);
                }
                let mut links = model::folder_links(&world.inventory, *id);
                links.sort_by_key(|l| l.name.to_lowercase());
                for link in links.iter().filter(|l| !l.folder) {
                    if let Some(it) = world.inventory.items.get(&link.target) {
                        row(ui, p, it, "", false);
                    } else {
                        ui.label(RichText::new(&link.name).size(12.0).color(p.muted));
                    }
                }
                if links.is_empty() {
                    ui.label(
                        RichText::new(if model::loaded(&world.inventory, *id) {
                            "Tenue vide"
                        } else {
                            "Chargement…"
                        })
                        .color(p.muted),
                    );
                }
                ui.horizontal(|ui| {
                    if widgets::flat_button(ui, p, "Porter").clicked() {
                        actions.push(Action::Wear(*id, false));
                    }
                    if widgets::flat_button(ui, p, "Ajouter").clicked() {
                        actions.push(Action::Wear(*id, true));
                    }
                });
            });
    }
}

fn worn_list(ui: &mut egui::Ui, p: &Palette, world: &World, st: &mut AppearanceUi, actions: &mut Vec<Action>, edit: bool) {
    let inv = &world.inventory;
    let mut links = model::cof(inv).map(|id| model::folder_links(inv, id)).unwrap_or_default();
    let worn = world.worn_attachment_items();
    for id in &worn {
        if !links.iter().any(|l| l.target == *id)
            && let Some(it) = inv.items.get(id)
        {
            links.push(aurora_net::outfits::OutfitLink {
                target: *id,
                name: it.name.clone(),
                desc: it.desc.clone(),
                folder: false,
                inv_type: it.inv_type,
            });
        }
    }
    links.sort_by(|a, b| a.desc.cmp(&b.desc).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    let q = st.search.trim().to_lowercase();
    let mut present = HashSet::new();
    for link in links
        .iter()
        .filter(|l| !l.folder && (edit || q.is_empty() || l.name.to_lowercase().contains(&q)))
    {
        let Some(it) = inv.items.get(&link.target) else {
            ui.label(RichText::new(format!("{} (chargement…)", link.name)).size(12.0).color(p.muted));
            continue;
        };
        if edit
            && !match st.edit_tab {
                0 => it.asset_type == 5,
                1 => it.asset_type == 6,
                _ => it.asset_type == 13,
            }
        {
            continue;
        }
        present.insert(it.flags & 0xff);
        ui.push_id(it.id, |ui| {
            let resp = row(
                ui,
                p,
                it,
                if it.asset_type == 6 && !worn.contains(&it.id) {
                    "(attachement en cours)"
                } else {
                    ""
                },
                st.selected_item == Some(it.id),
            );
            if resp.clicked() {
                st.selected_item = Some(it.id);
            }
            item_menu(&resp, it, actions);
            if edit && st.selected_item == Some(it.id) {
                ui.horizontal(|ui| {
                    if it.asset_type != 13
                        && icon_button(ui, p, "x-circle", if it.asset_type == 6 { "Détacher" } else { "Enlever" }).clicked()
                    {
                        actions.push(Action::Remove(it.id));
                    }
                    if it.asset_type == 5 {
                        if icon_button(ui, p, "caret-up", "Monter le calque").clicked() {
                            actions.push(Action::MoveLayer(it.id, true));
                        }
                        if icon_button(ui, p, "caret-down", "Descendre le calque").clicked() {
                            actions.push(Action::MoveLayer(it.id, false));
                        }
                    }
                });
            }
        });
    }
    if edit && st.edit_tab != 1 {
        for (kind, name) in model::WEARABLES
            .iter()
            .enumerate()
            .filter(|(k, _)| if st.edit_tab == 0 { *k >= 4 } else { *k < 4 })
        {
            if !present.contains(&(kind as u32)) {
                ui.horizontal(|ui| {
                    icon(ui, if st.edit_tab == 0 { "t-shirt" } else { "person" }, 16.0, p.muted_dim);
                    let ending = match kind {
                        0 | 1 | 4 | 8 | 12 => "e",
                        6 | 7 | 15 => "es",
                        2 | 3 | 9 => "s",
                        _ => "",
                    };
                    ui.label(RichText::new(format!("{name} non porté{ending}")).size(12.0).color(p.muted_dim));
                });
            }
        }
    }
}

fn temporary_attachments(ui: &mut egui::Ui, p: &Palette, world: &World) {
    egui::CollapsingHeader::new("Éléments temporaires")
        .id_salt("temporary_attachments")
        .show(ui, |ui| {
            let temporary = world
                .objects
                .index_of_uuid(&world.agent_id)
                .and_then(|idx| world.objects.get(idx))
                .map(|me| world.objects.children_of(&me.key))
                .unwrap_or_default();
            let mut count = 0;
            for idx in temporary {
                if let Some(o) = world.objects.get(*idx)
                    && o.attachment_item_id().is_none_or(|id| id.is_nil())
                {
                    count += 1;
                    ui.label(
                        RichText::new(format!("Objet temporaire ({})", o.attachment_point()))
                            .size(12.0)
                            .color(p.muted),
                    );
                }
            }
            if count == 0 {
                ui.label(RichText::new("Aucun élément temporaire").size(12.0).color(p.muted_dim));
            }
        });
}

fn add_list(ui: &mut egui::Ui, p: &Palette, world: &mut World, st: &mut AppearanceUi, actions: &mut Vec<Action>) {
    widgets::search_field(ui, &mut st.add_search, "Filtrer les éléments à porter", ui.available_width());
    let folders: Vec<_> = world
        .inventory
        .folders
        .values()
        .filter(|f| !f.library && matches!(f.info.type_default, 5 | 6 | 13))
        .map(|f| f.info.id)
        .collect();
    for id in folders {
        if st.add_search.trim().is_empty() {
            add_folder(ui, p, &mut world.inventory, id, st, actions, 0);
        } else {
            world.inventory.request(id);
        }
    }
    let q = st.add_search.trim().to_lowercase();
    let worn: HashSet<_> = model::cof(&world.inventory)
        .map(|id| model::folder_links(&world.inventory, id))
        .unwrap_or_default()
        .into_iter()
        .map(|l| l.target)
        .collect();
    let mut items: Vec<_> = world
        .inventory
        .items
        .values()
        .filter(|it| {
            matches!(it.asset_type, 5 | 6 | 13)
                && !worn.contains(&it.id)
                && it.name.to_lowercase().contains(&q)
                && (!q.is_empty() || it.parent == world.inventory.root)
                && world.inventory.folders.get(&it.parent).is_some_and(|f| !f.library)
        })
        .collect();
    items.sort_by_key(|it| it.name.to_lowercase());
    for it in items {
        let response = row(ui, p, it, "", st.selected_item == Some(it.id));
        if response.clicked() {
            st.selected_item = Some(it.id);
        }
        if response.double_clicked() {
            actions.push(Action::Add(it.id));
            st.adding = false;
        }
    }
}

/// Lazy folder traversal keeps nested inventory available without fetching it all.
fn add_folder(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &mut crate::world::inventory::Inventory,
    id: Uuid,
    st: &mut AppearanceUi,
    actions: &mut Vec<Action>,
    depth: usize,
) {
    if depth > 24 {
        return;
    }
    let Some(f) = inv.folders.get(&id) else {
        return;
    };
    let (name, children, items) = (f.info.name.clone(), f.children.clone(), f.items.clone());
    egui::CollapsingHeader::new(name)
        .id_salt(("add_outfit_folder", id))
        .default_open(depth == 0)
        .show(ui, |ui| {
            inv.request(id);
            for child in children {
                add_folder(ui, p, inv, child, st, actions, depth + 1);
            }
            let worn: HashSet<_> = model::cof(inv)
                .map(|id| model::folder_links(inv, id))
                .unwrap_or_default()
                .into_iter()
                .map(|l| l.target)
                .collect();
            for id in items {
                let Some(it) = inv.items.get(&id) else {
                    continue;
                };
                let it = if it.asset_type == 24 {
                    inv.items.get(&it.asset_id).unwrap_or(it)
                } else {
                    it
                };
                if !matches!(it.asset_type, 5 | 6 | 13) || worn.contains(&it.id) {
                    continue;
                }
                let response = row(ui, p, it, "", st.selected_item == Some(it.id));
                if response.clicked() {
                    st.selected_item = Some(it.id);
                }
                if response.double_clicked() {
                    actions.push(Action::Add(it.id));
                    st.adding = false;
                }
            }
        });
}

pub fn show(ctx: &egui::Context, p: &Palette, world: &mut World, st: &mut AppearanceUi, open: &mut bool, complexity: u32) -> Vec<Action> {
    let mut actions = Vec::new();
    let inv = &world.inventory;
    let base = model::base(inv);
    let name = base
        .and_then(|id| inv.folders.get(&id))
        .map(|f| f.info.name.clone())
        .unwrap_or_else(|| "Tenue actuelle".into());
    let dirty = model::dirty(inv);
    let count = world.worn_attachment_items().len();
    let my = model::system_folder(inv, model::FT_MY_OUTFITS);
    let mut outfits: Vec<_> = my
        .and_then(|id| inv.folders.get(&id))
        .map(|f| f.children.clone())
        .unwrap_or_default();
    outfits.retain(|id| {
        inv.folders.get(id).is_some_and(|f| {
            f.info.type_default == model::FT_OUTFIT && f.info.name.to_lowercase().contains(&st.search.trim().to_lowercase())
        })
    });
    outfits.sort_by_key(|id| inv.folders.get(id).map(|f| f.info.name.to_lowercase()));
    if st.reverse_sort {
        outfits.reverse();
    }
    let mut floater = widgets::Floater::new("appearance", "Apparence", egui::pos2(18.0, 60.0), Vec2::new(370.0, 560.0))
        .fixed()
        .help("Choisissez une tenue, gérez les éléments portés ou enregistrez votre tenue actuelle.");
    floater.min_size = Vec2::new(350.0, 360.0);
    floater.show(ctx, p, open, |ui| {
        ui.set_width(370.0);
        if st.editing {
            ui.horizontal(|ui| {
                if icon_button(ui, p, "arrow-left", "Revenir aux tenues").clicked() {
                    st.editing = false;
                    st.adding = false;
                }
                ui.label(RichText::new("Modifier l’habillement").size(20.0).strong().color(p.ink));
            });
        }
        ui.horizontal(|ui| {
            let name_width = (ui.available_width() - 30.0 - 28.0 - ui.spacing().item_spacing.x * 2.0).max(100.0);
            icon(ui, "t-shirt", 30.0, p.muted);
            ui.vertical(|ui| {
                ui.set_width(name_width);
                ui.label(
                    RichText::new(if st.pending.is_some() {
                        "Mise à jour de la tenue…"
                    } else if dirty {
                        "Modifications non enregistrées"
                    } else {
                        "Tenue actuelle"
                    })
                    .size(11.0)
                    .color(if dirty { p.warn } else { p.muted }),
                );
                ui.add(egui::Label::new(RichText::new(&name).size(16.0).strong().color(p.ink)).truncate());
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !st.editing && icon_button(ui, p, "wrench", "Modifier cette tenue").clicked() {
                    st.editing = true;
                }
            });
        });
        ui.add_space(8.0);
        if !st.editing {
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 28.0 - ui.spacing().item_spacing.x).max(100.0);
                ui.add_sized(
                    Vec2::new(width, 28.0),
                    egui::TextEdit::singleline(&mut st.search).hint_text("Filtrer les tenues"),
                );
                icon_menu(ui, p, "list", "Trier les tenues", |ui| {
                    if ui.selectable_label(!st.reverse_sort, "Nom : A → Z").clicked() {
                        st.reverse_sort = false;
                        ui.close();
                    }
                    if ui.selectable_label(st.reverse_sort, "Nom : Z → A").clicked() {
                        st.reverse_sort = true;
                        ui.close();
                    }
                });
            });
            let worn = format!("Portés ({count}/38 att.)");
            widgets::tabs(ui, p, &mut st.tab, &[("Galerie des tenues", true), ("Tenues", true), (&worn, true)]);
        } else {
            widgets::tabs(
                ui,
                p,
                &mut st.edit_tab,
                &[("Vêtements", true), ("Éléments attachés", true), ("Corps", true)],
            );
        }
        ui.add_space(5.0);
        let height = if st.editing || st.tab == 2 { 350.0 } else { 380.0 };
        egui::Frame::new().fill(p.field).inner_margin(4).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical()
                .id_salt("appearance_list")
                .max_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_enabled_ui(st.pending.is_none(), |ui| {
                        if st.editing && st.adding {
                            add_list(ui, p, world, st, &mut actions);
                        } else if st.editing {
                            worn_list(ui, p, world, st, &mut actions, true);
                        } else {
                            match st.tab {
                                0 => gallery(ui, p, world, st, &outfits, &mut actions),
                                1 => outfit_list(ui, p, world, st, &outfits, &mut actions),
                                _ => {
                                    ui.label(RichText::new("Éléments à porter").color(p.muted));
                                    worn_list(ui, p, world, st, &mut actions, false);
                                }
                            }
                        }
                        if !st.editing && st.tab < 2 && outfits.is_empty() {
                            ui.label(RichText::new("Aucune tenue à afficher").color(p.muted));
                        }
                    });
                });
        });
        if !st.editing && st.tab == 2 {
            temporary_attachments(ui, p, world);
        }
        if st.editing {
            ui.horizontal(|ui| {
                if widgets::flat_button(
                    ui,
                    p,
                    if st.adding {
                        "Retour aux éléments portés"
                    } else {
                        "Ajouter plus…"
                    },
                )
                .clicked()
                {
                    st.adding = !st.adding;
                    st.selected_item = None;
                }
                if st.adding
                    && ui
                        .add_enabled(st.selected_item.is_some() && st.pending.is_none(), egui::Button::new("Porter"))
                        .clicked()
                    && let Some(id) = st.selected_item
                {
                    actions.push(Action::Add(id));
                    st.adding = false;
                }
            });
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            icon_menu(ui, p, "gear-six", "Options de la tenue", |ui| {
                if ui.button("Actualiser").clicked() {
                    st.refresh(&mut world.inventory);
                    ui.close();
                }
                if ui
                    .add_enabled(
                        base.is_some() && st.pending.is_none(),
                        egui::Button::new("Rétablir la tenue enregistrée"),
                    )
                    .clicked()
                {
                    actions.push(Action::Revert);
                    ui.close();
                }
            });
            if !st.editing && st.tab < 2 {
                ui.label(RichText::new(format!("{} tenues", outfits.len())).size(11.0).color(p.muted));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("Complexité : {}", super::hud::group_digits(complexity)))
                        .size(11.0)
                        .color(p.muted),
                );
            });
        });
        if !st.message.is_empty() {
            ui.add(egui::Label::new(RichText::new(&st.message).size(11.0).color(p.warn)).wrap());
        }
        ui.horizontal(|ui| {
            let available = st.pending.is_none() && model::cof(&world.inventory).is_some_and(|id| model::complete(&world.inventory, id));
            if ui
                .add_enabled(
                    available && base.is_some() && dirty,
                    egui::Button::new("Enregistrer").fill(p.raised),
                )
                .clicked()
            {
                actions.push(Action::Save(None));
            }
            if ui
                .add_enabled(available, egui::Button::new("Enregistrer sous…").fill(p.raised))
                .clicked()
            {
                st.begin_save_as(&world.inventory);
            }
            if st.editing {
                if widgets::flat_button(ui, p, "Annuler").clicked() {
                    if dirty && base.is_some() {
                        actions.push(Action::Revert);
                    }
                    st.editing = false;
                    st.adding = false;
                }
            } else if st.tab < 2
                && ui
                    .add_enabled(
                        available && st.selected_outfit.is_some(),
                        egui::Button::new("Porter").fill(p.violet),
                    )
                    .clicked()
                && let Some(id) = st.selected_outfit
            {
                actions.push(Action::Wear(id, false));
            }
        });
    });
    if let Some(mut name) = st.save_as.clone() {
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("outfit_save_as")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new("Enregistrer sous").size(17.0).strong().color(p.ink));
            ui.add_space(6.0);
            ui.label("Enregistrer ce que je porte comme nouvelle tenue :");
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY).char_limit(63));
            if st.select_name {
                r.request_focus();
                if let Some(mut state) = egui::TextEdit::load_state(ctx, r.id) {
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(name.chars().count()),
                    )));
                    state.store(ctx, r.id);
                }
                st.select_name = false;
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let valid = !name.trim().is_empty() && st.pending.is_none();
                if ui
                    .add_enabled(valid, egui::Button::new("OK").min_size(Vec2::new(80.0, 24.0)).fill(p.violet))
                    .clicked()
                    || (valid && enter)
                {
                    actions.push(Action::Save(Some(name.trim().into())));
                    close = true;
                }
                if widgets::flat_button_sized(ui, p, "Annuler", Vec2::new(80.0, 24.0)).clicked() {
                    close = true;
                }
            });
        });
        st.save_as = if close || response.should_close() { None } else { Some(name) };
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(ctx: &egui::Context, world: &mut World, st: &mut AppearanceUi, key: Option<egui::Key>) -> Vec<Action> {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 900.0))),
            ..Default::default()
        };
        if let Some(key) = key {
            input.events.push(egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            });
        }
        let mut actions = Vec::new();
        let _ = ctx.run_ui(input, |ui| {
            actions = show(ui.ctx(), &crate::theme::Theme::default().palette(), world, st, &mut true, 41_748);
        });
        actions
    }

    #[test]
    fn floater_stays_compact_and_save_as_rejects_empty_names() {
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let _icons = icons::Icons::load(&ctx, None);
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        model::seed_demo(&mut world.inventory, Uuid::from_u128(1));
        let mut st = AppearanceUi::default();
        for _ in 0..80 {
            draw(&ctx, &mut world, &mut st, None);
        }
        let rect = ctx.memory(|m| m.area_rect(egui::Id::new("appearance"))).expect("floater");
        assert!(rect.width() < 500.0 && rect.height() < 700.0, "{rect:?}");
        st.save_as = Some("   ".into());
        draw(&ctx, &mut world, &mut st, None);
        assert!(draw(&ctx, &mut world, &mut st, Some(egui::Key::Enter)).is_empty());
        assert!(st.save_as.is_some());
        st.save_as = Some(" Tenue de test ".into());
        let actions = draw(&ctx, &mut world, &mut st, Some(egui::Key::Enter));
        assert!(matches!(&actions[..],[Action::Save(Some(name))] if name=="Tenue de test"));
        assert!(st.save_as.is_none());
    }
}
