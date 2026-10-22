//! LLFloaterGesture / LLPreviewGesture behaviour with Aurora's shared widgets.
//! Firestorm indra/newview, originally LGPL 2.1.
mod editor;
#[cfg(test)]
mod tests;
use super::widgets::{self, Floater};
use crate::gestures::{Manager, keys};
use crate::theme::Palette;
use crate::world::inventory::{Inventory, actions as rules};
use aurora_assets::gesture::Gesture;
use egui::{RichText, Vec2};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub only_active: bool,
    pub enabled: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub accounts: BTreeMap<Uuid, Preferences>,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            only_active: false,
            enabled: true,
        }
    }
}

pub enum Action {
    Open(Uuid),
    Play(Uuid),
    Preview(Uuid, Gesture),
    Stop(Uuid),
    Active(Vec<Uuid>, bool),
    Save {
        item: Uuid,
        definition: Gesture,
        description: String,
        close: bool,
    },
    Create,
    Delete(Vec<Uuid>),
    ShowInventory(Uuid),
    AddToOutfit(Vec<Uuid>),
    Refresh,
}

pub struct Editor {
    /// Preview playback belongs to this window, independently of active gestures.
    pub preview: Uuid,
    pub definition: Option<Gesture>,
    pub saved: Option<Gesture>,
    pub description: String,
    pub saved_description: String,
    pub message: String,
    pub saving: Option<Uuid>,
    pub close_after_save: bool,
    pub step: Option<usize>,
    pub library: u8,
    pub open: bool,
}
impl Editor {
    pub fn new(description: String) -> Self {
        Self {
            preview: Uuid::new_v4(),
            definition: None,
            saved: None,
            saved_description: description.clone(),
            description,
            message: String::new(),
            saving: None,
            close_after_save: false,
            step: None,
            library: 0,
            open: true,
        }
    }
    pub fn dirty(&self) -> bool {
        self.definition != self.saved || self.description != self.saved_description
    }
    pub fn loaded(&mut self, definition: Gesture) {
        self.step = (!definition.steps.is_empty()).then_some(0);
        self.saved = Some(definition.clone());
        self.definition = Some(definition);
    }
}

#[derive(Default)]
pub struct GesturesUi {
    pub editors: BTreeMap<Uuid, Editor>,
    editor_order: Vec<Uuid>,
    raise_editor: Option<Uuid>,
    pub message: String,
    selected: BTreeSet<Uuid>,
    anchor: Option<Uuid>,
    filter: String,
    sort: u8,
    descending: bool,
    pub catalog: Vec<Uuid>,
    generation: Option<u64>,
    rows: Vec<Uuid>,
    rows_key: Option<(u64, u64, bool, String, u8, bool)>,
    discard: Option<Uuid>,
    assets: editor::Assets,
}

impl GesturesUi {
    pub fn open(&mut self, inv: &Inventory, item: Uuid) {
        if let Some(it) = inv.items.get(&item) {
            if !self.editors.contains_key(&item) {
                self.editor_order.retain(|id| *id != item);
                self.editor_order.push(item);
            }
            self.editors.entry(item).or_insert_with(|| Editor::new(it.desc.clone())).open = true;
            self.raise_editor = Some(item);
        }
    }
    pub fn refresh_catalog(&mut self, inv: &Inventory) {
        if self.generation == Some(inv.generation) {
            return;
        }
        self.generation = Some(inv.generation);
        self.catalog = inv
            .items
            .values()
            .filter(|it| it.asset_type == 21 && !rules::library(inv, it.id) && !rules::in_type(inv, it.id, 14))
            .map(|it| it.id)
            .collect();
        self.catalog.sort_by_cached_key(|id| inv.items[id].name.to_lowercase());
        self.selected.retain(|id| inv.items.contains_key(id));
    }
}

pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    inv: &Inventory,
    manager: &Manager,
    state: &mut GesturesUi,
    preferences: &mut Preferences,
    open: &mut bool,
) -> Vec<Action> {
    let mut out = Vec::new();
    if *open {
        state.refresh_catalog(inv);
        Floater::new(
            "gestures",
            aurora_i18n::tr!("gesture-title"),
            egui::pos2(70.0, 120.0),
            Vec2::new(460.0, 510.0),
        )
        .show(ctx, p, open, |ui| {
            let check_width = ui.spacing().interact_size.y;
            let usable = (ui.available_width() - check_width - ui.spacing().item_spacing.x * 4.0 - 10.0).max(150.0);
            let key_width = 70.0_f32.min(usable * 0.24);
            let trigger_width = 110.0_f32.min(usable * 0.32);
            let columns = [usable - key_width - trigger_width, trigger_width, key_width];
            ui.horizontal(|ui| {
                ui.add_space(check_width + ui.spacing().item_spacing.x);
                for (column, key) in [(0, "common-name"), (1, "gesture-trigger"), (2, "gesture-key")] {
                    let mut heading = egui::Button::new(aurora_i18n::text(key)).truncate();
                    if state.sort == column
                        && let Some(caret) = crate::ui::icons::global(widgets::sort_caret(!state.descending))
                    {
                        heading = heading.right_text(egui::Image::new((caret.id(), Vec2::splat(10.0))).tint(p.muted));
                    }
                    if ui.add_sized([columns[usize::from(column)], 24.0], heading).clicked() {
                        if state.sort == column {
                            state.descending = !state.descending;
                        } else {
                            state.sort = column;
                            state.descending = false;
                        }
                    }
                }
            });
            state.refresh_rows(inv, manager, preferences);
            let empty = state.rows.is_empty();
            let (mut list, foot) = widgets::above_footer(ui);
            let rows = &state.rows;
            egui::ScrollArea::vertical()
                .max_height(list.available_height())
                .auto_shrink([false, false])
                .show_rows(&mut list, 24.0, rows.len(), |ui, range| {
                    for index in range {
                        let id = rows[index];
                        let it = &inv.items[&id];
                        ui.push_id(id, |ui| {
                            let active = manager.active.contains(&id);
                            let selected = state.selected.contains(&id);
                            let response = ui.add_sized([ui.available_width(), 24.0], egui::Button::selectable(selected, ""));
                            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, &it.name));
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .max_rect(response.rect)
                                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                                |ui| {
                                    let mut active = manager.active.contains(&id);
                                    if widgets::checkbox(ui, p, &mut active, "").changed() {
                                        out.push(Action::Active(vec![id], active));
                                    }
                                    let text = RichText::new(&it.name).color(if selected {
                                        p.on_accent
                                    } else if active {
                                        p.ink
                                    } else {
                                        p.muted
                                    });
                                    let text = if active { text.strong() } else { text };
                                    ui.allocate_ui_with_layout(
                                        Vec2::new(columns[0], 24.0),
                                        egui::Layout::left_to_right(egui::Align::Center),
                                        |ui| {
                                            ui.set_min_size(Vec2::new(columns[0], 24.0));
                                            ui.add(egui::Label::new(text).truncate().selectable(false).sense(egui::Sense::hover()));
                                        },
                                    );
                                    let (trigger, shortcut) = manager.entries.get(&id).map_or_else(
                                        || ("---".to_owned(), "---".to_owned()),
                                        |entry| {
                                            (
                                                entry.definition.trigger.clone(),
                                                keys::shortcut(entry.definition.key, entry.definition.mask),
                                            )
                                        },
                                    );
                                    let color = if selected { p.on_accent } else { p.ink };
                                    for (width, text) in [(columns[1], trigger), (columns[2], shortcut)] {
                                        ui.allocate_ui_with_layout(
                                            Vec2::new(width, 24.0),
                                            egui::Layout::left_to_right(egui::Align::Center),
                                            |ui| {
                                                ui.set_min_size(Vec2::new(width, 24.0));
                                                ui.add(
                                                    egui::Label::new(RichText::new(text).color(color))
                                                        .truncate()
                                                        .selectable(false)
                                                        .sense(egui::Sense::hover()),
                                                );
                                            },
                                        );
                                    }
                                },
                            );
                            if response.clicked() {
                                let modifiers = ui.input(|i| i.modifiers);
                                if modifiers.shift
                                    && let Some(anchor) = state.anchor.and_then(|a| rows.iter().position(|id| *id == a))
                                {
                                    state.selected.extend(rows[anchor.min(index)..=anchor.max(index)].iter().copied());
                                } else if modifiers.command {
                                    if !state.selected.remove(&id) {
                                        state.selected.insert(id);
                                    }
                                    state.anchor = Some(id);
                                } else {
                                    state.selected = BTreeSet::from([id]);
                                    state.anchor = Some(id);
                                }
                            }
                            if response.double_clicked() {
                                out.push(Action::Play(id));
                            }
                            response.context_menu(|ui| {
                                if super::menu::item(ui, p, "pencil-filled", aurora_i18n::tr!("common-open")) {
                                    out.push(Action::Open(id));
                                }
                                if super::menu::item(ui, p, "player-play-filled", aurora_i18n::tr!("common-play")) {
                                    out.push(Action::Play(id));
                                }
                                if super::menu::item(
                                    ui,
                                    p,
                                    "check",
                                    if active {
                                        aurora_i18n::tr!("gesture-deactivate")
                                    } else {
                                        aurora_i18n::tr!("gesture-activate")
                                    },
                                ) {
                                    out.push(Action::Active(vec![id], !active));
                                }
                            });
                        });
                    }
                });
            foot.show(ui, |ui| {
                if empty {
                    ui.label(RichText::new(aurora_i18n::tr!("gesture-empty")).color(p.muted));
                }
                ui.horizontal(|ui| {
                    ui.menu_button(aurora_i18n::tr!("gesture-options"), |ui| {
                        if super::menu::item(ui, p, "circle-plus-filled", aurora_i18n::tr!("ui-inventory-context-new-gesture")) {
                            out.push(Action::Create);
                        }
                        if super::menu::item_if(
                            ui,
                            p,
                            "shirt-filled",
                            aurora_i18n::tr!("gesture-add-outfit"),
                            !state.selected.is_empty(),
                        ) {
                            out.push(Action::AddToOutfit(state.selected.iter().copied().collect()));
                        }
                        if super::menu::item_if(
                            ui,
                            p,
                            "briefcase-filled",
                            aurora_i18n::tr!("gesture-show-inventory"),
                            state.selected.len() == 1,
                        ) && let Some(id) = state.selected.first()
                        {
                            out.push(Action::ShowInventory(*id));
                        }
                        if super::menu::item(ui, p, "refresh", aurora_i18n::tr!("gesture-refresh")) {
                            out.push(Action::Refresh);
                        }
                    });
                    if widgets::icon_button(
                        ui,
                        p,
                        "circle-plus-filled",
                        aurora_i18n::tr!("ui-inventory-context-new-gesture"),
                        true,
                    ) {
                        out.push(Action::Create);
                    }
                    let activate = state.selected.iter().any(|id| !manager.active.contains(id));
                    if widgets::icon_button(
                        ui,
                        p,
                        "check",
                        if activate {
                            aurora_i18n::tr!("gesture-activate")
                        } else {
                            aurora_i18n::tr!("gesture-deactivate")
                        },
                        !state.selected.is_empty(),
                    ) {
                        out.push(Action::Active(state.selected.iter().copied().collect(), activate));
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut state.filter)
                            .hint_text(aurora_i18n::tr!("gesture-filter"))
                            .desired_width(ui.available_width() - 30.0),
                    );
                    if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("common-delete"), !state.selected.is_empty()) {
                        out.push(Action::Delete(state.selected.iter().copied().collect()));
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    widgets::checkbox(ui, p, &mut preferences.only_active, aurora_i18n::tr!("gesture-only-active"));
                    widgets::checkbox(ui, p, &mut preferences.enabled, aurora_i18n::tr!("gesture-enabled"));
                });
                ui.horizontal(|ui| {
                    let one = state.selected.len() == 1;
                    if ui.add_enabled(one, egui::Button::new(aurora_i18n::tr!("gesture-edit"))).clicked()
                        && let Some(id) = state.selected.first()
                    {
                        out.push(Action::Open(*id));
                    }
                    let playing = state.selected.iter().any(|id| manager.is_playing(*id));
                    if ui
                        .add_enabled(
                            !state.selected.is_empty() && preferences.enabled,
                            egui::Button::new(if playing {
                                aurora_i18n::tr!("gesture-stop")
                            } else {
                                aurora_i18n::tr!("common-play")
                            }),
                        )
                        .clicked()
                    {
                        out.extend(
                            state
                                .selected
                                .iter()
                                .map(|id| if playing { Action::Stop(*id) } else { Action::Play(*id) }),
                        );
                    }
                });
                if !state.message.is_empty() {
                    ui.label(RichText::new(&state.message).color(p.warn));
                }
            });
        });
    }
    editor::show(ctx, p, inv, manager, state, &mut out);
    out
}

impl GesturesUi {
    fn refresh_rows(&mut self, inv: &Inventory, manager: &Manager, preferences: &Preferences) {
        let key = (
            inv.generation,
            manager.revision,
            preferences.only_active,
            self.filter.clone(),
            self.sort,
            self.descending,
        );
        if self.rows_key.as_ref() == Some(&key) {
            return;
        }
        self.rows_key = Some(key);
        let filter = self.filter.to_lowercase();
        let mut rows: Vec<_> = self
            .catalog
            .iter()
            .chain(manager.active.iter())
            .copied()
            .filter(|id| {
                (!preferences.only_active || manager.active.contains(id))
                    && inv.items.get(id).is_some_and(|it| {
                        filter.is_empty()
                            || it.name.to_lowercase().contains(&filter)
                            || manager
                                .entries
                                .get(id)
                                .is_some_and(|e| e.definition.trigger.to_lowercase().contains(&filter))
                    })
            })
            .collect();
        rows.sort();
        rows.dedup();
        rows.sort_by_cached_key(|id| {
            let it = &inv.items[id];
            let gesture = manager.entries.get(id);
            let primary = match self.sort {
                1 => gesture.map_or_else(String::new, |e| e.definition.trigger.to_lowercase()),
                2 => gesture.map_or_else(String::new, |e| keys::shortcut(e.definition.key, e.definition.mask)),
                _ => it.name.to_lowercase(),
            };
            (primary, it.name.to_lowercase(), *id)
        });
        if self.descending {
            rows.reverse();
        }

        self.selected.retain(|id| rows.contains(id));
        if self.selected.is_empty()
            && let Some(id) = rows.first()
        {
            self.selected.insert(*id);
            self.anchor = Some(*id);
        }
        self.rows = rows;
    }
}
