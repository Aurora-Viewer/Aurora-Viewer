//! Compact texture/material library: the human-selected lateral grid (V1)
//! and bottom shelf (H3), backed by #Aurora/Textures Library in inventory.

pub(crate) mod catalogue;
mod previews;
#[cfg(test)]
mod tests;

use super::{
    inventory::InvDrag,
    widgets::{self, ActionKind, Floater},
};
use crate::{
    scene::Scene,
    theme::Palette,
    world::{
        World,
        inventory::{FetchState, Inventory, actions as rules},
    },
};
use aurora_net::inventory::operations::{Mutation, Operation};
use catalogue::{Catalogue, Entry};
use egui::{RichText, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

pub const ICON: &str = "stack-2-filled";
pub const VERTICAL: &str = "texture_library_vertical";
pub const HORIZONTAL: &str = "texture_library_horizontal";
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Presentation {
    #[default]
    Vertical,
    Horizontal,
}
impl Presentation {
    pub fn window(self) -> &'static str {
        if self == Self::Vertical { VERTICAL } else { HORIZONTAL }
    }
    fn flip(&mut self) {
        *self = if *self == Self::Vertical {
            Self::Horizontal
        } else {
            Self::Vertical
        };
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub presentation: Presentation,
    pub favorites: HashSet<Uuid>,
    pub recent: Vec<Uuid>,
}
impl Preferences {
    pub fn record(&mut self, item: Uuid) {
        self.recent.retain(|id| *id != item);
        self.recent.insert(0, item);
        self.recent.truncate(32);
    }
}
#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum Kind {
    #[default]
    All,
    Texture,
    Material,
}
#[derive(Debug, Default, Clone, Copy, PartialEq)]
enum Scope {
    #[default]
    All,
    Favorites,
    Recent,
}
pub enum Action {
    Change(Mutation),
    Reveal(Uuid),
}
#[derive(Default)]
pub struct LibraryUi {
    catalogue: Catalogue,
    search: String,
    folder: Option<Uuid>,
    kind: Kind,
    scope: Scope,
    selected: Option<Uuid>,
    preview: Option<Uuid>,
    preview_view: aurora_render::preview::View,
    previews: previews::Previews,
    manager: bool,
    manager_initialized: bool,
    manager_generation: Option<(Uuid, u64)>,
    manager_folders: Vec<(Uuid, String, String)>,
    manager_search: String,
    manager_selection: HashSet<Uuid>,
    agent: Uuid,
    prepared: bool,
    pub pending: Option<Uuid>,
    pub pending_refresh: Vec<Uuid>,
    pub message: String,
    pub wanted_images: HashSet<Uuid>,
}
impl LibraryUi {
    pub fn demo(&mut self, mode: &str, agent: Uuid) {
        self.reset();
        self.agent = agent;
        if mode == "sources" {
            self.manager = true;
        }
        if mode == "search" {
            self.search = aurora_i18n::tr!("texture-library-demo-stone").into();
            self.kind = Kind::Material;
        }
    }
    pub fn reset(&mut self) {
        *self = Self::default();
    }
    pub fn mutation_result(&mut self, request: Uuid, result: &Result<aurora_net::inventory::operations::Reply, String>) -> bool {
        if self.pending != Some(request) {
            return false;
        }
        self.pending = None;
        self.catalogue.invalidate();
        self.message = result.as_ref().err().cloned().unwrap_or_default();
        true
    }
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        scene: &mut Scene,
        images: &HashMap<Uuid, egui::TextureHandle>,
        prefs: &mut Preferences,
        open: &mut bool,
        protected: &HashSet<Uuid>,
    ) -> Vec<Action> {
        self.previews.poll();
        if self.agent != world.agent_id {
            self.reset();
            self.agent = world.agent_id;
        }
        let mut actions = Vec::new();
        if *open {
            self.prepare_root(&mut world.inventory, &mut actions);
            self.catalogue.prepare(&mut world.inventory);
            if self.folder.is_some_and(|id| !self.catalogue.folders.iter().any(|f| f.id == id)) {
                self.folder = None;
            }
            let horizontal = prefs.presentation == Presentation::Horizontal;
            let screen = ctx.content_rect();
            let size = if horizontal {
                Vec2::new(760.0, 130.0)
            } else {
                Vec2::new(252.0, 590.0)
            };
            let size = Vec2::new(
                size.x.min(screen.width() - 20.0).max(220.0),
                size.y.min(screen.height() - 110.0).max(120.0),
            );
            let position = if horizontal {
                egui::pos2(screen.center().x - size.x / 2.0, screen.bottom() - size.y - 42.0)
            } else {
                egui::pos2(screen.right() - size.x - 12.0, screen.top() + 56.0)
            };
            let mut flip = false;
            let tip = if horizontal {
                aurora_i18n::tr!("texture-library-side")
            } else {
                aurora_i18n::tr!("texture-library-bottom")
            };
            let mut floater = Floater::new(
                prefs.presentation.window(),
                aurora_i18n::tr!("texture-library-title"),
                position,
                size,
            )
            .compact_header()
            .action(
                if horizontal {
                    "layout-sidebar-right-filled"
                } else {
                    "layout-bottombar-filled"
                },
                tip,
                &mut flip,
            );
            floater.min_size = if horizontal {
                Vec2::new(size.x.min(660.0), 130.0)
            } else {
                Vec2::new(252.0, 260.0)
            };
            floater.show(ctx, p, open, |ui| {
                ui.spacing_mut().item_spacing = egui::vec2(4.0, 3.0);
                ui.spacing_mut().interact_size.y = 20.0;
                if self.catalogue.root.is_none() {
                    ui.label(if self.pending.is_some() || !self.prepared {
                        aurora_i18n::tr!("texture-library-preparing")
                    } else {
                        aurora_i18n::tr!("texture-library-empty")
                    });
                    if !self.message.is_empty() {
                        ui.label(RichText::new(&self.message).color(p.warn));
                    }
                    if widgets::flat_button(ui, p, aurora_i18n::tr!("texture-library-retry")).clicked() {
                        self.prepared = false;
                        self.message.clear();
                        world.inventory.request(world.inventory.root);
                        if let Some(id) = catalogue::child(&world.inventory, world.inventory.root, catalogue::AURORA) {
                            world.inventory.request(id);
                        }
                        self.catalogue.invalidate();
                    }
                    return;
                }
                self.controls(ui, p, prefs, horizontal);
                let entries = self.filtered(prefs);
                let empty = entries.is_empty();
                if horizontal {
                    egui::ScrollArea::horizontal()
                        .id_salt("library_strip")
                        .max_height(61.0)
                        .show_viewport(ui, |ui, viewport| {
                            let first = (viewport.min.x / 48.0).floor().max(0.0) as usize;
                            let end = ((viewport.max.x / 48.0).ceil() as usize + 1).min(entries.len());
                            ui.set_min_width((entries.len() as f32 * 48.0 - 4.0).max(0.0));
                            ui.horizontal(|ui| {
                                if first > 0 {
                                    ui.add_space(first as f32 * 48.0 - 4.0);
                                }
                                for index in entries.iter().take(end).skip(first) {
                                    let entry = self.catalogue.entries[*index].clone();
                                    self.card(ui, p, world, scene, images, prefs, &entry, 44.0, false);
                                }
                            });
                        });
                } else {
                    let row_height = 148.0;
                    let rows = entries.len().div_ceil(2);
                    let room = (ui.available_height() - 34.0).max(80.0);
                    egui::ScrollArea::vertical()
                        .id_salt("library_grid")
                        .max_height(room)
                        .show_rows(ui, row_height, rows, |ui, visible| {
                            for row in visible {
                                ui.horizontal(|ui| {
                                    for index in entries.iter().skip(row * 2).take(2) {
                                        let entry = self.catalogue.entries[*index].clone();
                                        self.card(ui, p, world, scene, images, prefs, &entry, 106.0, true);
                                    }
                                });
                            }
                        });
                    ui.horizontal(|ui| {
                        if let Some(entry) = self
                            .selected
                            .and_then(|id| self.catalogue.entries.iter().find(|e| e.id == id))
                            .cloned()
                        {
                            ui.add_sized(
                                [ui.available_width() - 25.0, 21.0],
                                egui::Label::new(RichText::new(&entry.name).size(10.0)).truncate(),
                            )
                            .on_hover_text(&entry.name);
                            if icon_button(ui, p, "arrows-maximize", aurora_i18n::tr!("texture-library-preview")).clicked() {
                                self.preview = Some(entry.id);
                            }
                        } else {
                            ui.add(
                                egui::Label::new(
                                    RichText::new(aurora_i18n::tr!("texture-library-drag-hint"))
                                        .size(10.0)
                                        .color(p.muted),
                                )
                                .truncate(),
                            )
                            .on_hover_text(aurora_i18n::tr!("texture-library-drag-hint"));
                        }
                    });
                }
                if empty {
                    ui.label(
                        RichText::new(if self.catalogue.loading {
                            aurora_i18n::tr!("texture-library-loading")
                        } else if self.catalogue.entries.is_empty() {
                            aurora_i18n::tr!("texture-library-empty")
                        } else {
                            aurora_i18n::tr!("texture-library-no-results")
                        })
                        .size(10.0)
                        .color(p.muted),
                    );
                }
                if !self.catalogue.failed.is_empty() {
                    ui.horizontal(|ui| {
                        let width = (ui.available_width() - 70.0).max(80.0);
                        ui.add_sized(
                            [width, 20.0],
                            egui::Label::new(
                                RichText::new(aurora_i18n::tr!("texture-library-folder-error"))
                                    .size(10.0)
                                    .color(p.warn),
                            )
                            .truncate(),
                        )
                        .on_hover_text(aurora_i18n::tr!("texture-library-folder-error"));
                        if widgets::flat_button(ui, p, aurora_i18n::tr!("texture-library-retry")).clicked() {
                            for id in &self.catalogue.failed {
                                world.inventory.request(*id);
                            }
                            self.catalogue.invalidate();
                        }
                    });
                }
                if !self.message.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&self.message).size(10.0).color(p.warn)).truncate())
                        .on_hover_text(&self.message);
                }
            });
            if flip {
                prefs.presentation.flip();
            }
        }
        if self.manager && *open {
            self.manager(ctx, p, &world.inventory, protected, &mut actions);
        }
        if self.preview.is_some() && *open {
            self.preview(ctx, p, world, scene, images);
        }
        actions
    }
    fn prepare_root(&mut self, inv: &mut Inventory, actions: &mut Vec<Action>) {
        if inv.root.is_nil() || inv.held.is_some() {
            return;
        }
        if inv.folders.get(&inv.root).is_none_or(|f| f.state != FetchState::Fetched) {
            let root = inv.root;
            self.request_root(inv, root);
            return;
        }
        if catalogue::root(inv).is_some() || self.prepared || self.pending.is_some() {
            return;
        }
        let mut change = Mutation::default();
        let aurora = if let Some(id) = catalogue::child(inv, inv.root, catalogue::AURORA) {
            if inv.folders.get(&id).is_none_or(|f| f.state != FetchState::Fetched) {
                self.request_root(inv, id);
                return;
            }
            id
        } else {
            let id = Uuid::new_v4();
            change.operations.push(Operation::Folder {
                id,
                parent: inv.root,
                name: catalogue::AURORA.into(),
                type_default: -1,
            });
            change.refresh.push(inv.root);
            id
        };
        let id = Uuid::new_v4();
        change.operations.push(Operation::Folder {
            id,
            parent: aurora,
            name: catalogue::LIBRARY.into(),
            type_default: -1,
        });
        change.refresh.extend([aurora, id]);
        self.prepared = true;
        actions.push(Action::Change(change));
    }
    fn request_root(&mut self, inv: &mut Inventory, id: Uuid) {
        match inv.folders.get(&id).map(|f| f.state) {
            Some(FetchState::Unknown) => inv.request(id),
            Some(FetchState::Failed) => self.message = aurora_i18n::tr!("texture-library-folder-error").into(),
            _ => {}
        }
    }
    fn filtered(&self, prefs: &Preferences) -> Vec<usize> {
        let query = catalogue::folded(&self.search);
        let mut originals = HashSet::new();
        let mut entries: Vec<_> = self
            .catalogue
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.matches(&query, self.folder))
            .filter(|(_, e)| self.kind == Kind::All || (self.kind == Kind::Material) == e.material)
            .filter(|(_, e)| match self.scope {
                Scope::All => true,
                Scope::Favorites => prefs.favorites.contains(&e.original),
                Scope::Recent => prefs.recent.contains(&e.original),
            })
            .filter(|(_, e)| originals.insert(e.original))
            .map(|(index, _)| index)
            .collect();
        if self.scope == Scope::Recent {
            entries.sort_by_key(|index| prefs.recent.iter().position(|id| *id == self.catalogue.entries[*index].original));
        }
        entries
    }
    fn controls(&mut self, ui: &mut egui::Ui, p: &Palette, prefs: &Preferences, horizontal: bool) {
        let row = |this: &mut Self, ui: &mut egui::Ui| {
            let label = this
                .folder
                .and_then(|id| this.catalogue.folders.iter().find(|f| f.id == id))
                .map(|f| f.label.as_str())
                .unwrap_or(aurora_i18n::tr!("texture-library-all"));
            let width = if horizontal {
                180.0
            } else {
                (ui.available_width() - 26.0).max(140.0)
            };
            let response = ui.add_sized([width, 20.0], egui::Button::new(label).truncate());
            super::menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                if super::menu::item(ui, p, ICON, aurora_i18n::tr!("texture-library-all")) {
                    this.folder = None;
                    this.scope = Scope::All;
                    ui.close();
                }
                egui::ScrollArea::vertical()
                    .max_height(280.0)
                    .show_rows(ui, 22.0, this.catalogue.folders.len(), |ui, rows| {
                        for row in rows {
                            let folder = &this.catalogue.folders[row];
                            if super::menu::item(ui, p, "folder-filled", &folder.label) {
                                this.folder = Some(folder.id);
                                this.scope = Scope::All;
                                ui.close();
                            }
                        }
                    });
            });
            response.on_hover_text(label);
            if icon_button(ui, p, "folder-filled", aurora_i18n::tr!("texture-library-folders")).clicked() {
                this.manager = true;
                this.manager_initialized = false;
                this.manager_search.clear();
                this.manager_selection.clear();
            }
        };
        if horizontal {
            ui.horizontal(|ui| {
                row(self, ui);
                widgets::search_field(ui, &mut self.search, aurora_i18n::tr!("common-search"), 160.0);
                self.filters(ui, p, prefs);
            });
        } else {
            widgets::search_field(ui, &mut self.search, aurora_i18n::tr!("common-search"), ui.available_width());
            ui.horizontal(|ui| row(self, ui));
            ui.horizontal(|ui| self.filters(ui, p, prefs));
        }
    }
    fn filters(&mut self, ui: &mut egui::Ui, p: &Palette, _prefs: &Preferences) {
        for (kind, label) in [
            (Kind::All, aurora_i18n::tr!("common-all")),
            (Kind::Texture, aurora_i18n::tr!("texture-library-textures")),
            (Kind::Material, aurora_i18n::tr!("texture-library-pbr")),
        ] {
            if widgets::action_button(
                ui,
                p,
                "",
                label,
                if self.kind == kind { ActionKind::Primary } else { ActionKind::Flat },
                Vec2::new(if kind == Kind::Texture { 55.0 } else { 34.0 }, 20.0),
            )
            .clicked()
            {
                self.kind = kind;
            }
        }
        if scope_button(
            ui,
            p,
            "star-filled",
            aurora_i18n::tr!("common-favorites"),
            self.scope == Scope::Favorites,
        )
        .clicked()
        {
            self.scope = if self.scope == Scope::Favorites {
                Scope::All
            } else {
                Scope::Favorites
            };
        }
        if scope_button(
            ui,
            p,
            "clock-filled",
            aurora_i18n::tr!("texture-library-recent"),
            self.scope == Scope::Recent,
        )
        .clicked()
        {
            self.scope = if self.scope == Scope::Recent { Scope::All } else { Scope::Recent };
        }
    }
    fn card(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        world: &World,
        scene: &mut Scene,
        images: &HashMap<Uuid, egui::TextureHandle>,
        prefs: &mut Preferences,
        entry: &Entry,
        size: f32,
        details: bool,
    ) {
        ui.push_id((entry.id, details), |ui| {
            ui.vertical(|ui| {
                let visible = ui.is_rect_visible(egui::Rect::from_min_size(ui.next_widget_position(), Vec2::splat(size)));
                let model = if entry.material && visible {
                    self.previews.model(entry.asset, scene, &mut self.wanted_images, ui.ctx())
                } else {
                    None
                };
                let response = if let Some(model) = model {
                    aurora_render::preview::paint_swatch(ui, ui.id().with("material"), model, previews::view(p), Vec2::splat(size))
                } else {
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click_and_drag());
                    let image = if entry.material { entry.thumbnail } else { entry.asset };
                    if visible && !image.is_nil() {
                        self.wanted_images.insert(image);
                    }
                    if let Some(image) = images.get(&image) {
                        ui.painter().image(
                            image.id(),
                            rect,
                            egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                            widgets::PICTURE_TINT,
                        );
                    } else {
                        ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
                        if let Some(icon) = super::icons::global(if entry.material { "cube" } else { "photo-filled" }) {
                            ui.painter().image(
                                icon.id(),
                                egui::Rect::from_center_size(rect.center(), Vec2::splat(23.0)),
                                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                                p.muted_dim,
                            );
                        }
                    }
                    response
                };
                if self.selected == Some(entry.id) {
                    ui.painter().rect_stroke(
                        response.rect,
                        ui.visuals().window_corner_radius,
                        egui::Stroke::new(1.0, p.violet_light),
                        egui::StrokeKind::Inside,
                    );
                }
                if self.catalogue.entries.first().is_some_and(|first| first.id == entry.id) {
                    ui.ctx()
                        .data_mut(|d| d.insert_temp(egui::Id::new("texture_library_first_card"), response.rect.center()));
                }
                response.dnd_set_drag_payload(InvDrag(entry.id));
                if response.clicked() {
                    self.selected = Some(entry.id);
                }
                if response.double_clicked() {
                    self.preview = Some(entry.id);
                    self.preview_view = previews::view(p);
                }
                response.on_hover_text(format!(
                    "{}\n{}\n{}",
                    entry.name,
                    entry.path,
                    aurora_i18n::tr!("texture-library-drag-hint")
                ));
                ui.add_sized(
                    [size, 14.0],
                    egui::Label::new(RichText::new(&entry.name).size(if details { 10.0 } else { 9.0 })).truncate(),
                )
                .on_hover_text(&entry.name);
                if details {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(if entry.material {
                                aurora_i18n::tr!("texture-library-pbr")
                            } else {
                                aurora_i18n::tr!("texture-library-textures")
                            })
                            .size(9.0)
                            .color(p.muted),
                        );
                        let favorite = prefs.favorites.contains(&entry.original);
                        if icon_button(
                            ui,
                            p,
                            if favorite { "star-filled" } else { "star" },
                            aurora_i18n::tr!("common-favorites"),
                        )
                        .clicked()
                        {
                            if favorite {
                                prefs.favorites.remove(&entry.original);
                            } else {
                                prefs.favorites.insert(entry.original);
                            }
                        }
                    });
                }
                let _ = world;
            });
        });
    }
    fn manager(&mut self, ctx: &egui::Context, p: &Palette, inv: &Inventory, protected: &HashSet<Uuid>, actions: &mut Vec<Action>) {
        let Some(root) = self.catalogue.root else { return };
        let links: HashMap<Uuid, Uuid> = inv
            .folders
            .get(&root)
            .into_iter()
            .flat_map(|f| &f.items)
            .filter_map(|id| {
                let item = inv.items.get(id)?;
                (item.asset_type == 25)
                    .then(|| rules::original(inv, *id).map(|target| (target, *id)))
                    .flatten()
            })
            .collect();
        // Initialise on opening; editing an empty selection must stay empty.
        if !self.manager_initialized {
            self.manager_selection = links.keys().copied().collect();
            self.manager_initialized = true;
        }
        if self.manager_generation != Some((root, inv.generation)) {
            self.manager_folders = source_folders(inv, root);
            self.manager_generation = Some((root, inv.generation));
        }
        let mut done = false;
        let modal = widgets::Modal::new(egui::Id::new("texture_library_sources")).show(ctx, |ui| {
            ui.heading(aurora_i18n::tr!("texture-library-folders"));
            ui.set_width(420.0);
            ui.label(aurora_i18n::tr!("texture-library-source-hint"));
            widgets::search_field(
                ui,
                &mut self.manager_search,
                aurora_i18n::tr!("common-search"),
                ui.available_width(),
            );
            let query = catalogue::folded(&self.manager_search);
            let folders: Vec<_> = self
                .manager_folders
                .iter()
                .filter(|(_, _, search)| search.contains(&query))
                .collect();
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .show_rows(ui, 24.0, folders.len(), |ui, rows| {
                    for row in rows {
                        let (id, name, _) = folders[row];
                        let mut selected = self.manager_selection.contains(id);
                        if widgets::checkbox(ui, p, &mut selected, name).changed() {
                            if selected {
                                self.manager_selection.insert(*id);
                            } else {
                                self.manager_selection.remove(id);
                            }
                        }
                    }
                });
            ui.label(
                RichText::new(aurora_i18n::tr!("texture-library-source-links"))
                    .size(11.0)
                    .color(p.muted),
            );
            ui.horizontal(|ui| {
                if widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    done = true;
                }
                if widgets::action_button_if(
                    ui,
                    p,
                    "check",
                    aurora_i18n::tr!("common-save"),
                    ActionKind::Primary,
                    Vec2::new(110.0, 26.0),
                    self.pending.is_none(),
                )
                .clicked()
                {
                    match sources_change(inv, root, &self.manager_selection, protected) {
                        Ok(change) => {
                            if !change.operations.is_empty() {
                                actions.push(Action::Change(change));
                            }
                            done = true;
                        }
                        Err(reason) => self.message = reason,
                    }
                }
                if widgets::flat_button(ui, p, aurora_i18n::tr!("texture-library-inventory")).clicked() {
                    actions.push(Action::Reveal(root));
                    done = true;
                }
            });
            if !self.message.is_empty() {
                ui.label(RichText::new(&self.message).color(p.warn));
            }
        });
        if done || modal.should_close() {
            self.manager = false;
            self.manager_initialized = false;
        }
    }
    fn preview(&mut self, ctx: &egui::Context, p: &Palette, world: &World, scene: &mut Scene, images: &HashMap<Uuid, egui::TextureHandle>) {
        let Some(entry) = self
            .preview
            .and_then(|id| self.catalogue.entries.iter().find(|e| e.id == id))
            .cloned()
        else {
            self.preview = None;
            return;
        };
        let size = Vec2::new(360.0, 420.0);
        let mut open = true;
        Floater::new(
            "texture_library_preview",
            &entry.name,
            ctx.content_rect().center() - size / 2.0,
            size,
        )
        .fixed()
        .show(ctx, p, &mut open, |ui| {
            if entry.material {
                if let Some(model) = self.previews.model(entry.asset, scene, &mut self.wanted_images, ctx) {
                    self.preview_view.background = previews::view(p).background;
                    aurora_render::preview::paint(ui, ui.id().with(entry.asset), model, &mut self.preview_view, Vec2::splat(320.0));
                } else {
                    ui.allocate_space(Vec2::splat(320.0));
                }
            } else {
                self.wanted_images.insert(entry.asset);
                if let Some(image) = images.get(&entry.asset) {
                    ui.add(
                        egui::Image::new(image)
                            .fit_to_exact_size(Vec2::splat(320.0))
                            .maintain_aspect_ratio(true),
                    );
                }
            }
            ui.label(RichText::new(&entry.path).size(11.0).color(p.muted));
            if let Some(item) = world.inventory.items.get(&entry.original) {
                ui.label(
                    RichText::new(if item.owner_mask & rules::COPY != 0 {
                        aurora_i18n::tr!("texture-library-copy")
                    } else {
                        aurora_i18n::tr!("texture-library-no-copy")
                    })
                    .size(11.0)
                    .color(p.muted),
                );
            }
        });
        if !open {
            self.preview = None;
        }
    }
}
fn icon_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str) -> egui::Response {
    widgets::action_button(ui, p, icon, "", ActionKind::Flat, Vec2::splat(20.0)).on_hover_text(tip)
}
fn scope_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, active: bool) -> egui::Response {
    widgets::action_button(
        ui,
        p,
        icon,
        "",
        if active { ActionKind::Primary } else { ActionKind::Flat },
        Vec2::splat(20.0),
    )
    .on_hover_text(tip)
}
fn source_folders(inv: &Inventory, root: Uuid) -> Vec<(Uuid, String, String)> {
    let Some(library) = inv.folders.get(&root) else { return Vec::new() };
    let mut folders: Vec<_> = inv
        .folders
        .values()
        .filter(|f| {
            !f.library
                && f.info.id != inv.root
                && rules::under(inv, f.info.id, inv.root)
                && !rules::under(inv, root, f.info.id)
                && !rules::under(inv, f.info.id, library.info.parent)
                && !inv.in_folder_type(f.info.id, 14)
                && !matches!(f.info.type_default, 46..=50 | 53 | 54)
        })
        .map(|f| {
            let mut path = vec![f.info.name.as_str()];
            let mut parent = f.info.parent;
            for _ in 0..64 {
                if parent == inv.root {
                    break;
                }
                let Some(folder) = inv.folders.get(&parent) else {
                    break;
                };
                path.push(&folder.info.name);
                parent = folder.info.parent;
            }
            path.reverse();
            let name = path.join(" / ");
            (f.info.id, name.clone(), catalogue::folded(&name))
        })
        .collect();
    folders.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)));
    folders
}
fn sources_change(inv: &Inventory, root: Uuid, selected: &HashSet<Uuid>, protected: &HashSet<Uuid>) -> Result<Mutation, String> {
    let folder = inv.folders.get(&root).ok_or(aurora_i18n::tr!("common-folder-not-found"))?;
    let mut existing = HashSet::new();
    let mut remove = Vec::new();
    for id in &folder.items {
        if inv.items.get(id).is_some_and(|it| it.asset_type == 25)
            && let Some(target) = rules::original(inv, *id)
        {
            existing.insert(target);
            if !selected.contains(&target) {
                remove.push(*id);
            }
        }
    }
    let mut change = Mutation::default();
    if !remove.is_empty() {
        let trash = rules::system(inv, 14).ok_or(aurora_i18n::tr!("common-trash-folder-not-found"))?;
        change = rules::move_selection(inv, &remove, trash, protected, &HashSet::new())?;
    }
    let add: Vec<_> = selected.iter().filter(|id| !existing.contains(id)).copied().collect();
    if !add.is_empty() {
        let links = rules::paste(inv, &add, root, false, true, protected, &HashSet::new())?;
        change.operations.extend(links.operations);
        change.refresh.extend(links.refresh);
    }
    // Removing a folder link never removes the source folder or its items.
    Ok(change)
}
