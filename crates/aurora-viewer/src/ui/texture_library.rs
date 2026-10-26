//! Compact inventory library: lateral grid and bottom shelf backed by the
//! actual subtree under #Aurora/Aurora Library.

pub(crate) mod catalogue;
mod material_editor;
pub(crate) mod previews;
pub(crate) mod texture_view;
mod windows;
pub(crate) use windows::window_id as preview_window_id;
#[cfg(test)]
mod size_tests;
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
use catalogue::{Catalogue, Entry, ResourceKind};
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
    recent_agent: Uuid,
    recent_by_agent: HashMap<Uuid, Vec<Uuid>>,
    pub texture_preview_size: Option<[f32; 2]>,
    pub material_preview_size: Option<[f32; 2]>,
    pub texture_display: texture_view::Display,
}
impl Preferences {
    pub fn activate(&mut self, agent: Uuid) {
        if agent.is_nil() || self.recent_agent == agent {
            return;
        }
        if !self.recent_agent.is_nil() {
            self.recent_by_agent.insert(self.recent_agent, std::mem::take(&mut self.recent));
            self.recent = self.recent_by_agent.remove(&agent).unwrap_or_default();
        }
        self.recent_agent = agent;
    }
    fn preview_size(&self, material: bool) -> Option<Vec2> {
        let size = if material {
            self.material_preview_size
        } else {
            self.texture_preview_size
        }?;
        size.iter().all(|v| v.is_finite() && *v > 0.0).then(|| Vec2::from(size))
    }
    fn record_preview_size(&mut self, material: bool, size: Vec2) {
        let target = if material {
            &mut self.material_preview_size
        } else {
            &mut self.texture_preview_size
        };
        *target = Some(size.into());
    }
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
    Object,
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
    Apply(Uuid),
    Open(Uuid),
    SaveMaterial {
        item: Uuid,
        material: aurora_assets::PbrMaterial,
        name: Option<String>,
    },
    RefreshTexture(Uuid),
    ExportTexture {
        item: Uuid,
        png: bool,
    },
    ExportResult(String),
    Profile(Uuid),
    Discard(Uuid),
    MaterialMapFile {
        item: Uuid,
        slot: usize,
    },
}
#[derive(Default)]
pub struct LibraryUi {
    export_folder: Option<crossbeam_channel::Receiver<Option<std::path::PathBuf>>>,
    export_items: Vec<(Uuid, bool)>,
    export_queue: std::collections::VecDeque<(Uuid, std::path::PathBuf, bool)>,
    export_jobs: Vec<windows::BatchExport>,
    export_error: Option<String>,
    catalogue: Catalogue,
    search: String,
    folder: Option<Uuid>,
    kind: Kind,
    types: Option<HashSet<ResourceKind>>,
    scope: Scope,
    selected: Option<Uuid>,
    windows: Vec<windows::PreviewWindow>,
    focus_preview: Option<Uuid>,
    previews: previews::Previews,
    agent: Uuid,
    prepared: bool,
    pub pending: Option<Uuid>,
    pub pending_refresh: Vec<Uuid>,
    pub message: String,
    pub wanted_images: HashSet<Uuid>,
    map_uploads: HashMap<Uuid, (Uuid, usize)>,
}
impl LibraryUi {
    pub fn demo(&mut self, mode: &str, agent: Uuid) {
        self.reset();
        self.agent = agent;
        if matches!(mode, "apply" | "drop") {
            self.kind = Kind::Texture;
        }
        if mode == "apply-pbr" || mode.starts_with("material-preview") {
            self.kind = Kind::Material;
        }
        if mode == "search" {
            self.search = aurora_i18n::tr!("texture-library-demo-stone").into();
            self.kind = Kind::Material;
        }
        if mode.starts_with("objects") {
            self.kind = Kind::Object;
            if mode == "objects-nocopy-drop" {
                self.search = "4 ".into();
            }
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
    ) -> Vec<Action> {
        self.previews.poll();
        if self.agent != world.agent_id {
            self.reset();
            self.agent = world.agent_id;
        }
        let mut actions = Vec::new();
        if *open {
            prefs.activate(world.agent_id);
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
                if self.catalogue.roots.is_empty() {
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
                if let Some(root) = self.controls(ui, p, prefs, horizontal) {
                    actions.push(Action::Reveal(root));
                }
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
                                    if let Some(action) = self.card(
                                        ui,
                                        p,
                                        world,
                                        scene,
                                        images,
                                        prefs,
                                        &entry,
                                        44.0,
                                        false,
                                        entries.first() == Some(index),
                                    ) {
                                        actions.push(action);
                                    }
                                }
                            });
                        });
                } else {
                    let row_height = 148.0;
                    let rows = entries.len().div_ceil(2);
                    // the cards in exactly the room the selection and the
                    // warnings leave
                    let (mut list, foot) = widgets::above_footer(ui);
                    egui::ScrollArea::vertical()
                        .id_salt("library_grid")
                        .max_height(list.available_height())
                        .show_rows(&mut list, row_height, rows, |ui, visible| {
                            for row in visible {
                                ui.horizontal(|ui| {
                                    for index in entries.iter().skip(row * 2).take(2) {
                                        let entry = self.catalogue.entries[*index].clone();
                                        if let Some(action) = self.card(
                                            ui,
                                            p,
                                            world,
                                            scene,
                                            images,
                                            prefs,
                                            &entry,
                                            106.0,
                                            true,
                                            entries.first() == Some(index),
                                        ) {
                                            actions.push(action);
                                        }
                                    }
                                });
                            }
                        });
                    foot.show(ui, |ui| {
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
                                if !entry.kind.object()
                                    && icon_button(ui, p, "arrows-maximize", aurora_i18n::tr!("texture-library-preview")).clicked()
                                {
                                    if entry.kind.surface() {
                                        self.open_preview(ctx, world, entry.id);
                                    } else {
                                        actions.push(Action::Open(entry.id));
                                    }
                                }
                            } else {
                                let hint = if self.kind == Kind::Object {
                                    aurora_i18n::tr!("texture-library-object-drag-hint")
                                } else {
                                    aurora_i18n::tr!("texture-library-click-hint")
                                };
                                ui.add(egui::Label::new(RichText::new(hint).size(10.0).color(p.muted)).truncate())
                                    .on_hover_text(hint);
                            }
                        });
                        self.warnings(ui, p, world, empty);
                    });
                }
                if horizontal {
                    self.warnings(ui, p, world, empty);
                }
            });
            if flip {
                prefs.presentation.flip();
            }
        }
        self.show_previews(ctx, p, world, scene, images, prefs, &mut actions);
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
        if self.prepared || self.pending.is_some() {
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
        let library = if let Some(id) = catalogue::child(inv, aurora, catalogue::LIBRARY) {
            if inv.folders.get(&id).is_none_or(|f| f.state != FetchState::Fetched) {
                self.request_root(inv, id);
                return;
            }
            id
        } else {
            let id = Uuid::new_v4();
            change.operations.push(Operation::Folder {
                id,
                parent: aurora,
                name: catalogue::LIBRARY.into(),
                type_default: -1,
            });
            change.refresh.push(id);
            id
        };
        // Preserve old item IDs, favorites and subtrees while adopting the new
        // convention. Parent changes use the existing reliable inventory move.
        let mut adopted = HashSet::new();
        for (legacy, name) in [(catalogue::LEGACY_TEXTURES, "Textures"), (catalogue::OBJECTS_LIBRARY, "Objects")] {
            if let Some(id) = catalogue::child(inv, aurora, legacy) {
                let rename = catalogue::child(inv, library, name).is_none();
                let mut body = aurora_llsd::llsd_map! { "parent_id" => library };
                if rename {
                    body.insert("name", aurora_llsd::Llsd::String(name.into()));
                    adopted.insert(name);
                }
                change.operations.push(Operation::Patch { id, folder: true, body });
                change.refresh.push(id);
            }
        }
        for name in catalogue::DEFAULT_FOLDERS {
            if adopted.contains(name) || catalogue::child(inv, library, name).is_some() {
                continue;
            }
            let id = Uuid::new_v4();
            change.operations.push(Operation::Folder {
                id,
                parent: library,
                name: name.into(),
                type_default: -1,
            });
            change.refresh.push(id);
        }
        change.refresh.push(library);
        if change.operations.is_empty() {
            return;
        }
        change.refresh.push(aurora);
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
            .filter(|(_, e)| {
                self.types.as_ref().map_or_else(
                    || match self.kind {
                        Kind::All => true,
                        Kind::Texture => e.kind == ResourceKind::Texture,
                        Kind::Material => e.kind == ResourceKind::Material,
                        Kind::Object => e.kind.object(),
                    },
                    |types| types.contains(&e.kind.category()),
                )
            })
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
    /// What waits under the cards: nothing found, folders that failed to
    /// load, the last message, with their retries.
    fn warnings(&mut self, ui: &mut egui::Ui, p: &Palette, world: &mut World, empty: bool) {
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
            // the button first, from the right: the note takes what it
            // leaves (a guessed button width widened the window at every
            // frame)
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::flat_button(ui, p, aurora_i18n::tr!("texture-library-retry")).clicked() {
                    for id in &self.catalogue.failed {
                        world.inventory.request(*id);
                    }
                    self.catalogue.invalidate();
                }
                ui.add_sized(
                    [ui.available_width(), 20.0],
                    egui::Label::new(
                        RichText::new(aurora_i18n::tr!("texture-library-folder-error"))
                            .size(10.0)
                            .color(p.warn),
                    )
                    .truncate(),
                )
                .on_hover_text(aurora_i18n::tr!("texture-library-folder-error"));
            });
        }
        if !self.message.is_empty() {
            ui.add(egui::Label::new(RichText::new(&self.message).size(10.0).color(p.warn)).truncate())
                .on_hover_text(&self.message);
            if self.pending.is_none() && widgets::flat_button(ui, p, aurora_i18n::tr!("texture-library-retry")).clicked() {
                self.prepared = false;
                self.message.clear();
            }
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui, p: &Palette, prefs: &Preferences, horizontal: bool) -> Option<Uuid> {
        let mut reveal = None;
        let mut row = |this: &mut Self, ui: &mut egui::Ui| {
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
            let manage = icon_button(ui, p, "folder-filled", aurora_i18n::tr!("texture-library-manage"))
                .on_hover_text(aurora_i18n::tr!("texture-library-content-hint"));
            if manage.clicked() {
                reveal = this.catalogue.parent;
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
        reveal
    }
    fn filters(&mut self, ui: &mut egui::Ui, p: &Palette, _prefs: &Preferences) {
        let selected: HashSet<_> = self.types.clone().unwrap_or_else(|| match self.kind {
            Kind::All => ResourceKind::TYPES.into_iter().collect(),
            Kind::Texture => HashSet::from([ResourceKind::Texture]),
            Kind::Material => HashSet::from([ResourceKind::Material]),
            Kind::Object => HashSet::from([ResourceKind::Object]),
        });
        let label = if selected.len() == ResourceKind::TYPES.len() {
            aurora_i18n::tr!("texture-library-types-all").to_string()
        } else if selected.len() == 1 {
            selected.iter().next().map(|kind| kind.label()).unwrap_or_default().to_string()
        } else {
            aurora_i18n::tr!("texture-library-types-count", count = selected.len()).to_string()
        };
        let response = widgets::action_button(ui, p, "chevron-down", &label, ActionKind::Flat, Vec2::new(158.0, 20.0));
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique("library_type_selector"), response.rect.center()));
        let menu_open = super::menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
            if super::menu::item(ui, p, "check", aurora_i18n::tr!("texture-library-types-all")) {
                self.types = None;
                self.kind = Kind::All;
            }
            if super::menu::item(ui, p, "x", aurora_i18n::tr!("texture-library-types-none")) {
                self.types = Some(HashSet::new());
            }
            super::menu::separator(ui, p);
            for kind in ResourceKind::TYPES {
                let mut checked = selected.contains(&kind);
                let before = checked;
                super::menu::toggle(ui, p, kind.icon(), kind.label(), &mut checked);
                if checked != before {
                    let types = self.types.get_or_insert_with(|| selected.clone());
                    if checked {
                        types.insert(kind);
                    } else {
                        types.remove(&kind);
                    }
                }
            }
        });
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::unique("library_type_menu_open"), menu_open));
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
        first: bool,
    ) -> Option<Action> {
        let mut action = None;
        ui.push_id((entry.id, details), |ui| {
            ui.vertical(|ui| {
                let visible = ui.is_rect_visible(egui::Rect::from_min_size(ui.next_widget_position(), Vec2::splat(size)));
                let model = if entry.kind == ResourceKind::Material && visible {
                    self.previews.model(entry.asset, scene, &mut self.wanted_images, ui.ctx())
                } else {
                    None
                };
                let response = if let Some(model) = model {
                    aurora_render::preview::paint_swatch(ui, ui.scope_id().with("material"), model, previews::view(p), Vec2::splat(size))
                } else {
                    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::click_and_drag());
                    let image = entry.image();
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
                        if let Some(icon) = super::icons::global(entry.kind.icon()) {
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
                if first {
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::unique("texture_library_first_card"), response.rect.center());
                        d.insert_temp(egui::Id::unique("texture_library_first_item"), entry.id);
                    });
                }
                response.dnd_set_drag_payload(InvDrag(entry.id));
                if response.clicked() {
                    self.selected = Some(entry.id);
                    if entry.kind.surface() && !response.double_clicked() {
                        action = Some(Action::Apply(entry.id));
                    }
                }
                if response.double_clicked() && !entry.kind.object() {
                    if entry.kind.surface() {
                        self.open_preview(ui.ctx(), world, entry.id);
                    } else {
                        action = Some(Action::Open(entry.id));
                    }
                }
                response.on_hover_text(if entry.kind.object() {
                    format!(
                        "{}\n{}\n{}",
                        entry.name,
                        entry.path,
                        aurora_i18n::tr!("texture-library-object-drag-hint")
                    )
                } else if !entry.kind.surface() {
                    format!("{}\n{}\n{}", entry.name, entry.path, aurora_i18n::tr!("texture-library-open-hint"))
                } else {
                    format!(
                        "{}\n{}\n{}\n{}",
                        entry.name,
                        entry.path,
                        aurora_i18n::tr!("texture-library-click-hint"),
                        aurora_i18n::tr!("texture-library-drag-hint")
                    )
                });
                ui.add_sized(
                    [size, 14.0],
                    egui::Label::new(RichText::new(&entry.name).size(if details { 10.0 } else { 9.0 })).truncate(),
                )
                .on_hover_text(&entry.name);
                if details {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(entry.kind.label()).size(9.0).color(p.muted));
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
            });
        });
        action
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
