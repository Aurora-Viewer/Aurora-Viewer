//! Sky and water editors: Firestorm's LLFloaterFixedEnvironmentSky and
//! LLFloaterFixedEnvironmentWater (newview/llfloaterfixedenvironment.cpp,
//! llfloatereditenvironmentbase.cpp, floater_fixedenvironment.xml) with the
//! panels of llpaneleditsky.cpp (Atmosphere & Lighting, Clouds, Sun & Moon;
//! Firestorm never adds its density panel) and llpaneleditwater.cpp
//! (panel_fs_settings_water.xml). Same controls, ranges and scales; the
//! name, Load and Import on top, Save with its menu and Cancel below.
//! Originally LGPL 2.1.

use super::environment::{azimuth_elevation, color_w, rotation_from, slider_w};

/// Label column of the sliders (the longest French labels fit).
const LABEL_W: f32 = 190.0;

fn color(ui: &mut egui::Ui, p: &Palette, label: &str, value: &mut Vec3, scale: f32) {
    color_w(ui, p, label, value, scale, LABEL_W);
}

fn slider(ui: &mut egui::Ui, p: &Palette, label: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>) -> egui::Response {
    slider_w(ui, p, label, v, range, LABEL_W)
}
use super::texture_picker::TexturePicker;
use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::eep::{self, SkyFrame, WaterFrame};
use crate::world::env_edit::{self, Rights, SettingsDoc};
use crate::world::env_select::{SettingsCatalog, SettingsKind};
use crate::world::inventory::Inventory;
use egui::{RichText, Sense, Vec2};
use glam::{Vec2 as V2, Vec3};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// LLPanelSettingsSky slider scales (llpaneleditsky.cpp).
const SLIDER_SCALE_SUN_AMBIENT: f32 = 3.0;
const SLIDER_SCALE_BLUE_HORIZON_DENSITY: f32 = 2.0;
const SLIDER_SCALE_GLOW_R: f32 = 20.0;
const SLIDER_SCALE_GLOW_B: f32 = -5.0;
const SLIDER_SCALE_DENSITY_MULTIPLIER: f32 = 0.001;

pub const WINDOW: &str = "env_fixed_editor";

/// What the window asks the app to do.
#[derive(Debug, Clone, PartialEq)]
pub enum EditorAction {
    /// The settings changed: show them (updateEditEnvironment).
    Changed,
    /// "Load": a settings item of the same kind.
    Load(Uuid),
    /// "Import": a Windlight XML file.
    Import,
    /// "Save" over the item being edited.
    Save,
    /// "Save As" chosen: the name is asked once the settings are checked
    /// ([`EnvEditorUi::ask_save_as`]).
    SaveAsAsk,
    /// "Save As": a new item with that name.
    SaveAs(String),
    /// "Apply only to myself".
    ApplyLocal,
    /// "Apply To Parcel" / "Apply To Region" (doApplyEnvironment).
    ApplyParcel,
    ApplyRegion,
    /// The window closed (changes discarded or saved).
    Closed,
}

/// What the editors' Save menu may offer (LLFloaterEditEnvironmentBase::
/// canUseInventory, canApplyParcel, canApplyRegion).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EditorAccess {
    /// The region keeps settings in the inventory.
    pub inventory: bool,
    /// The agent may change the parcel's environment.
    pub parcel: bool,
    /// The agent manages the estate.
    pub region: bool,
}

/// An action waiting for "discard the changes?" (checkAndConfirmSettingsLoss).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Confirm {
    Close,
    Load(Uuid),
    Import,
}

/// The texture controls (LLTextureCtrl).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TexSlot {
    Cloud,
    Sun,
    Moon,
    Normal,
}

#[derive(Default)]
pub struct EnvEditorUi {
    pub open: bool,
    pub kind: Option<SettingsKind>,
    /// None while the item's asset loads.
    pub doc: Option<SettingsDoc>,
    /// The inventory item edited (None: a snapshot or an import).
    pub item: Option<Uuid>,
    pub rights: Rights,
    pub dirty: bool,
    /// Item whose asset is on its way.
    pub loading: Option<Uuid>,
    /// Save request in flight.
    pub saving: Option<Uuid>,
    /// Status line: (text, is an error).
    pub message: Option<(String, bool)>,
    tab: usize,
    confirm: Option<Confirm>,
    save_as: Option<String>,
    name: String,
    picker: TexturePicker,
    picking: Option<TexSlot>,
    pub wanted_images: HashSet<Uuid>,
    /// Demo: an item to open once its folder has loaded.
    pub wait_item: Option<Uuid>,
    /// The sun and moon beacons (the sunbeacon / moonbeacon settings, kept
    /// by the app).
    pub beacons: [bool; 2],
}

impl EnvEditorUi {
    /// The settings were saved as `item` (onInventoryCreated / Updated):
    /// the editor follows it, clean.
    pub fn saved(&mut self, item: Uuid, name: &str, rights: Rights) {
        if let Some(doc) = &mut self.doc {
            doc.set_name(name);
        }
        self.name = name.to_owned();
        self.item = Some(item);
        self.rights = rights;
        self.dirty = false;
        self.message = Some((aurora_i18n::tr!("ui-env-editor-saved").into(), false));
    }

    pub fn tab_index(&self) -> usize {
        self.tab
    }

    /// "SaveSettingAs": the name prompt, filled with the settings' name.
    pub fn ask_save_as(&mut self) {
        self.save_as = Some(self.name.clone());
    }

    /// The tab shown (sky: Atmosphere & Lighting, Clouds, Sun & Moon).
    pub fn set_tab(&mut self, tab: usize) {
        self.tab = tab.min(2);
    }

    /// Edit `doc` (a snapshot, an import, or a loaded item).
    pub fn start(&mut self, doc: SettingsDoc, item: Option<Uuid>, rights: Rights) {
        self.open = true;
        self.kind = Some(doc.kind());
        self.name = doc.name().to_owned();
        self.doc = Some(doc);
        self.item = item;
        self.rights = rights;
        self.dirty = false;
        self.loading = None;
        self.confirm = None;
        self.save_as = None;
        self.picking = None;
        self.picker.open = false;
    }

    /// An item's asset is being fetched (loadInventoryItem).
    pub fn start_loading(&mut self, kind: SettingsKind, item: Uuid) {
        self.open = true;
        self.kind = Some(kind);
        self.loading = Some(item);
        self.message = None;
        self.confirm = None;
        self.save_as = None;
    }

    pub fn close(&mut self) {
        *self = EnvEditorUi {
            picker: std::mem::take(&mut self.picker),
            ..Default::default()
        };
    }

    fn mark(&mut self, out: &mut Vec<EditorAction>) {
        self.dirty = true;
        self.message = None;
        if !out.contains(&EditorAction::Changed) {
            out.push(EditorAction::Changed);
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        inv: &Inventory,
        images: &HashMap<Uuid, egui::TextureHandle>,
        access: EditorAccess,
    ) -> Vec<EditorAction> {
        let mut out = Vec::new();
        if !self.open {
            return out;
        }
        let kind = self.kind.unwrap_or(SettingsKind::Sky);
        let title = if kind == SettingsKind::Water {
            aurora_i18n::tr!("ui-env-editor-edit-water")
        } else {
            aurora_i18n::tr!("ui-env-editor-edit-sky")
        };
        let screen = ctx.content_rect();
        let size = Vec2::new(560.0, 600.0);
        let mut open = true;
        let mut floater = Floater::new(WINDOW, title, screen.center() - size * 0.5, size);
        floater.min_size = Vec2::new(500.0, 480.0);
        floater.help(aurora_i18n::tr!("ui-env-editor-help")).show(ctx, p, &mut open, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
            self.top_row(ui, p, inv, kind, &mut out);
            ui.add_space(4.0);
            if self.doc.is_none() || self.loading.is_some() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| match &self.message {
                    Some((text, true)) => ui.label(RichText::new(text).color(p.danger)),
                    _ => ui.label(RichText::new(aurora_i18n::tr!("ui-env-editor-loading")).color(p.muted)),
                });
                return;
            }
            let footer = 64.0;
            let body_h = (ui.available_height() - footer).max(120.0);
            self.panels_into(ui, p, images, body_h, &mut out);
            widgets::divider(ui, p);
            self.footer(ui, p, access, &mut out);
        });
        if !open {
            self.request(Confirm::Close, &mut out);
        }
        self.dialogs(ctx, p, &mut out);
        out
    }

    /// The tabs and panels of the document (the sky tabs or the water
    /// panel), within `height`: the fixed editors' body, and the day cycle
    /// editor's frame panels (LLPanelSettingsSky / Water in its tabs).
    /// Returns whether the document changed.
    pub fn panels(&mut self, ui: &mut egui::Ui, p: &Palette, images: &HashMap<Uuid, egui::TextureHandle>, height: f32) -> bool {
        let mut out = Vec::new();
        self.panels_into(ui, p, images, height, &mut out);
        out.contains(&EditorAction::Changed)
    }

    fn panels_into(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        images: &HashMap<Uuid, egui::TextureHandle>,
        height: f32,
        out: &mut Vec<EditorAction>,
    ) {
        let kind = self.kind.unwrap_or(SettingsKind::Sky);
        if kind == SettingsKind::Sky {
            widgets::tabs(
                ui,
                p,
                &mut self.tab,
                &[
                    (aurora_i18n::tr!("ui-env-editor-atmosphere"), true),
                    (aurora_i18n::tr!("ui-environment-clouds"), true),
                    (aurora_i18n::tr!("ui-env-editor-sun-moon"), true),
                ],
            );
            ui.add_space(4.0);
        }
        egui::ScrollArea::vertical()
            .id_salt(("env_panels", kind == SettingsKind::Water))
            .max_height(height - if kind == SettingsKind::Sky { 34.0 } else { 0.0 })
            .auto_shrink([false, false])
            .show(ui, |ui| {
                // "Show Beacon" of the sun and the moon: a view option,
                // enabled whatever the rights (llpaneleditsky.cpp)
                if kind == SettingsKind::Sky && self.tab == 2 {
                    ui.horizontal(|ui| {
                        widgets::checkbox(ui, p, &mut self.beacons[0], aurora_i18n::tr!("ui-env-editor-sun-beacon"));
                        ui.add_space(16.0);
                        widgets::checkbox(ui, p, &mut self.beacons[1], aurora_i18n::tr!("ui-env-editor-moon-beacon"));
                    });
                    ui.add_space(4.0);
                }
                ui.add_enabled_ui(self.rights.modify, |ui| match kind {
                    SettingsKind::Water => self.water_panel(ui, p, images, out),
                    _ => match self.tab {
                        1 => self.clouds_panel(ui, p, images, out),
                        2 => self.sun_moon_panel(ui, p, images, out),
                        _ => self.atmosphere_panel(ui, p, out),
                    },
                });
            });
    }

    /// Run `what` now, or after "discard the changes?" when dirty.
    fn request(&mut self, what: Confirm, out: &mut Vec<EditorAction>) {
        if self.dirty {
            self.confirm = Some(what);
        } else {
            self.run(what, out);
        }
    }

    fn run(&mut self, what: Confirm, out: &mut Vec<EditorAction>) {
        match what {
            Confirm::Close => {
                self.close();
                out.push(EditorAction::Closed);
            }
            Confirm::Load(item) => out.push(EditorAction::Load(item)),
            Confirm::Import => out.push(EditorAction::Import),
        }
    }

    fn top_row(&mut self, ui: &mut egui::Ui, p: &Palette, inv: &Inventory, kind: SettingsKind, out: &mut Vec<EditorAction>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("common-name")).size(12.0).color(p.ink));
            // a fixed width: a field following the window would grow it
            let w = 280.0;
            let enabled = self.rights.modify && self.doc.is_some();
            let r = ui.add_enabled(enabled, egui::TextEdit::singleline(&mut self.name).desired_width(w).char_limit(63));
            // setCommitOnFocusLost: the name is committed when the field is left
            if (r.lost_focus() || r.changed() && ui.input(|i| i.key_pressed(egui::Key::Enter)))
                && let Some(doc) = &mut self.doc
                && doc.name() != self.name
            {
                doc.set_name(&self.name);
                self.dirty = true;
            }
            let load = widgets::action_button(
                ui,
                p,
                "briefcase-filled",
                aurora_i18n::tr!("ui-env-editor-load"),
                ActionKind::Flat,
                Vec2::new(0.0, 24.0),
            )
            .on_hover_text(aurora_i18n::tr!("ui-env-editor-load-tip"));
            super::menu::dropdown(&load, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                let order = super::inventory::view::picker_order(ui.ctx(), inv);
                let items = SettingsCatalog::items_of(inv, kind, |id| order.rank(id));
                if items.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-env-editor-no-items")).size(12.0).color(p.muted));
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (id, name) in items {
                        let icon = if kind == SettingsKind::Water {
                            "droplet-filled"
                        } else {
                            "sun-filled"
                        };
                        if super::menu::item(ui, p, icon, &name) {
                            self.request(Confirm::Load(id), out);
                        }
                    }
                });
            });
            if widgets::action_button(
                ui,
                p,
                "folder-open-filled",
                aurora_i18n::tr!("ui-env-editor-import"),
                ActionKind::Flat,
                Vec2::new(0.0, 24.0),
            )
            .on_hover_text(aurora_i18n::tr!("ui-env-editor-import-tip"))
            .clicked()
                && self.rights.modify
            {
                self.request(Confirm::Import, out);
            }
        });
    }

    fn footer(&mut self, ui: &mut egui::Ui, p: &Palette, access: EditorAccess, out: &mut Vec<EditorAction>) {
        let can_inventory = access.inventory;
        ui.horizontal(|ui| {
            if let Some((text, error)) = &self.message {
                let c = if *error { p.danger } else { p.muted };
                ui.label(RichText::new(text).size(11.5).color(c));
            } else if self.saving.is_some() {
                ui.label(RichText::new(aurora_i18n::tr!("ui-env-editor-saving")).size(11.5).color(p.muted));
            } else if !can_inventory {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-env-editor-no-inventory"))
                        .size(11.5)
                        .color(p.muted),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let size = Vec2::new(0.0, 26.0);
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    self.request(Confirm::Close, out);
                }
                let idle = self.saving.is_none();
                // the flyout's entries (refresh / updateButtons)
                let save = idle && can_inventory && self.rights.modify && self.rights.save && self.item.is_some();
                let save_as = idle && can_inventory && self.rights.copy && self.rights.save;
                let more = widgets::action_button(ui, p, "caret-down-filled", "", ActionKind::Flat, Vec2::new(26.0, 26.0))
                    .on_hover_text(aurora_i18n::tr!("ui-env-editor-more"));
                super::menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    if super::menu::item_if(ui, p, "device-floppy-filled", aurora_i18n::tr!("common-save"), save) {
                        out.push(EditorAction::Save);
                    }
                    if super::menu::item_if(ui, p, "copy-filled", aurora_i18n::tr!("common-save-as"), save_as) {
                        out.push(EditorAction::SaveAsAsk);
                    }
                    super::menu::separator(ui, p);
                    if super::menu::item(ui, p, "user-filled", aurora_i18n::tr!("common-apply-to-myself-only")) {
                        out.push(EditorAction::ApplyLocal);
                    }
                    if super::menu::item_if(
                        ui,
                        p,
                        "map-pin-filled",
                        aurora_i18n::tr!("ui-env-editor-apply-parcel"),
                        access.parcel,
                    ) {
                        out.push(EditorAction::ApplyParcel);
                    }
                    if super::menu::item_if(ui, p, "world-filled", aurora_i18n::tr!("ui-env-editor-apply-region"), access.region) {
                        out.push(EditorAction::ApplyRegion);
                    }
                });
                if widgets::action_button_if(
                    ui,
                    p,
                    "device-floppy-filled",
                    aurora_i18n::tr!("common-save"),
                    ActionKind::Primary,
                    size,
                    save,
                )
                .clicked()
                {
                    out.push(EditorAction::Save);
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context, p: &Palette, out: &mut Vec<EditorAction>) {
        if let Some(what) = self.confirm {
            let name = self.doc.as_ref().map(|d| d.name().to_owned()).unwrap_or_default();
            let mut answer = None;
            let modal = widgets::Modal::new(egui::Id::unique("env_editor_confirm")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-env-editor-confirm-loss", name = name))
                        .size(13.0)
                        .color(p.ink),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let size = Vec2::new(0.0, 26.0);
                    if widgets::action_button(ui, p, "check", aurora_i18n::tr!("common-yes"), ActionKind::Danger, size).clicked() {
                        answer = Some(true);
                    }
                    if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-no"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if modal.should_close() && answer.is_none() {
                answer = Some(false);
            }
            match answer {
                Some(true) => {
                    self.confirm = None;
                    self.dirty = false;
                    self.run(what, out);
                }
                Some(false) => self.confirm = None,
                None => {}
            }
        }
        if let Some(mut name) = self.save_as.clone() {
            let mut close = false;
            let modal = widgets::Modal::new(egui::Id::unique("env_editor_save_as")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new(aurora_i18n::tr!("common-save-as")).size(17.0).strong().color(p.ink));
                ui.add_space(6.0);
                ui.label(aurora_i18n::tr!("ui-env-editor-save-as-prompt"));
                ui.add_space(8.0);
                ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY).char_limit(63));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let size = Vec2::new(0.0, 26.0);
                    let enter = widgets::enter_pressed(ui);
                    if widgets::action_button(ui, p, "check", aurora_i18n::tr!("common-ok"), ActionKind::Primary, size).clicked() || enter {
                        // onSaveAsCommit: corrected, "Unnamed" when empty
                        out.push(EditorAction::SaveAs(env_edit::correct_item_name(&name)));
                        close = true;
                    }
                    if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        close = true;
                    }
                });
            });
            self.save_as = if close || modal.should_close() { None } else { Some(name) };
        }
    }

    /// The texture picker of a texture control; applies the choice.
    pub fn show_picker(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> bool {
        let chosen = self.picker.show_live(ctx, p, world, images);
        self.wanted_images.extend(self.picker.wanted_images.drain());
        let slot = self.picking;
        if !self.picker.open {
            self.picking = None;
        }
        let (Some(id), Some(slot), Some(doc)) = (chosen, slot, self.doc.as_mut()) else {
            return false;
        };
        let changed = match slot {
            TexSlot::Normal => {
                let before = doc.water();
                let after = WaterFrame { normal_map: id, ..before };
                doc.edit_water(&before, &after)
            }
            _ => {
                let before = doc.sky();
                let mut after = before;
                match slot {
                    TexSlot::Cloud => after.cloud_id = id,
                    TexSlot::Sun => after.sun_id = id,
                    _ => after.moon_id = id,
                }
                doc.edit_sky(&before, &after)
            }
        };
        if changed {
            self.dirty = true;
        }
        changed
    }

    fn texture(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        images: &HashMap<Uuid, egui::TextureHandle>,
        slot: TexSlot,
        label: &str,
        id: Uuid,
    ) {
        ui.vertical(|ui| {
            ui.add(egui::Label::new(RichText::new(label).size(12.0).color(p.ink)).extend());
            let enabled = ui.is_enabled();
            let (rect, resp) = ui.allocate_exact_size(Vec2::splat(76.0), if enabled { Sense::click() } else { Sense::hover() });
            let radius = ui.visuals().window_corner_radius;
            ui.painter().rect_filled(rect, radius, p.field);
            if id.is_nil() {
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    aurora_i18n::tr!("common-none"),
                    egui::FontId::proportional(11.0),
                    p.muted,
                );
            } else if let Some(t) = images.get(&id) {
                egui::Image::new(t).corner_radius(radius).paint_at(ui, rect.shrink(2.0));
            } else {
                self.wanted_images.insert(id);
            }
            if resp.hovered() && enabled {
                ui.painter()
                    .rect_stroke(rect, radius, egui::Stroke::new(1.0, p.violet), egui::StrokeKind::Inside);
            }
            if resp.clicked() {
                self.open_texture(slot, label, id);
            }
        });
    }

    fn open_texture(&mut self, slot: TexSlot, label: &str, id: Uuid) {
        // LLTextureCtrl: the default image and "None" where allowed.
        let (default, none) = match slot {
            TexSlot::Cloud => (Some(eep::DEFAULT_CLOUD_ID), true),
            TexSlot::Sun => (Some(Uuid::nil()), true),
            TexSlot::Moon => (Some(eep::DEFAULT_MOON_ID), true),
            TexSlot::Normal => (Some(eep::DEFAULT_WATER_NORMAL), false),
        };
        self.picker.open_with_defaults(label, id, default, none);
        // Local settings cannot be saved or applied (WLLocalTextureFixedBlock).
        self.picker.allow_local = true;
        self.picking = Some(slot);
    }

    pub(crate) fn demo_cloud_picker(&mut self) {
        if let Some(doc) = &self.doc {
            self.open_texture(TexSlot::Cloud, aurora_i18n::tr!("ui-env-editor-cloud-image"), doc.sky().cloud_id);
        }
    }

    fn atmosphere_panel(&mut self, ui: &mut egui::Ui, p: &Palette, out: &mut Vec<EditorAction>) {
        let Some(doc) = &mut self.doc else { return };
        let before = doc.sky();
        let mut s = before;
        let mut extra_changed = false;
        ui.spacing_mut().slider_width = 200.0;
        egui::Grid::new("env_editor_atmos_colors")
            .num_columns(4)
            .spacing(Vec2::new(8.0, 4.0))
            .show(ui, |ui| {
                color(ui, p, aurora_i18n::tr!("common-ambient"), &mut s.ambient, SLIDER_SCALE_SUN_AMBIENT);
                color(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-environment-blue-horizon"),
                    &mut s.blue_horizon,
                    SLIDER_SCALE_BLUE_HORIZON_DENSITY,
                );
                ui.end_row();
                color(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-environment-blue-density"),
                    &mut s.blue_density,
                    SLIDER_SCALE_BLUE_HORIZON_DENSITY,
                );
                ui.end_row();
            });
        ui.add_space(4.0);
        slider(
            ui,
            p,
            aurora_i18n::tr!("ui-environment-haze-horizon"),
            &mut s.haze_horizon,
            0.0..=5.0,
        );
        slider(
            ui,
            p,
            aurora_i18n::tr!("ui-environment-haze-density"),
            &mut s.haze_density,
            0.0..=5.0,
        );
        for (key, label, range) in [
            (env_edit::MOISTURE_LEVEL, aurora_i18n::tr!("ui-env-editor-moisture"), 0.0..=1.0),
            (
                env_edit::DROPLET_RADIUS,
                aurora_i18n::tr!("ui-env-editor-droplet-radius"),
                5.0..=1000.0,
            ),
            (env_edit::ICE_LEVEL, aurora_i18n::tr!("ui-env-editor-ice-level"), 0.0..=1.0),
        ] {
            let mut v = doc.extra(key);
            if slider(ui, p, label, &mut v, range).changed() {
                doc.set_extra(key, v);
                extra_changed = true;
            }
        }
        // shown divided by SLIDER_SCALE_DENSITY_MULTIPLIER
        let mut density = s.density_multiplier / SLIDER_SCALE_DENSITY_MULTIPLIER;
        if slider(
            ui,
            p,
            aurora_i18n::tr!("ui-env-editor-density-multiplier"),
            &mut density,
            0.0001..=2.0,
        )
        .changed()
        {
            s.density_multiplier = density * SLIDER_SCALE_DENSITY_MULTIPLIER;
        }
        slider(
            ui,
            p,
            aurora_i18n::tr!("ui-env-editor-distance-multiplier"),
            &mut s.distance_multiplier,
            0.05..=1000.0,
        );
        slider(ui, p, aurora_i18n::tr!("ui-env-editor-max-altitude"), &mut s.max_y, 0.0..=10000.0);
        let mut ambiance = s.probe_ambiance;
        if slider(ui, p, aurora_i18n::tr!("ui-environment-probe-ambiance"), &mut ambiance, 0.0..=10.0)
            .on_hover_text(aurora_i18n::tr!(
                "ui-environment-intensity-of-indirect-environment-lighting-at-zero-hdr-scale"
            ))
            .changed()
        {
            s.probe_ambiance = ambiance;
            s.can_auto_adjust = false;
        }
        // updateGammaLabel
        let hdr = s.probe_ambiance != 0.0;
        let r = slider(
            ui,
            p,
            if hdr {
                aurora_i18n::tr!("ui-environment-hdr-scale")
            } else {
                aurora_i18n::tr!("common-brightness")
            },
            &mut s.gamma,
            0.0..=20.0,
        );
        if hdr {
            r.on_hover_text(aurora_i18n::tr!(
                "ui-environment-intensity-of-lighting-effects-such-as-realistic-bright-skies"
            ));
        }
        if doc.edit_sky(&before, &s) || extra_changed {
            self.mark(out);
        }
    }

    fn clouds_panel(&mut self, ui: &mut egui::Ui, p: &Palette, images: &HashMap<Uuid, egui::TextureHandle>, out: &mut Vec<EditorAction>) {
        let Some(doc) = &self.doc else { return };
        let before = doc.sky();
        let mut s = before;
        ui.spacing_mut().slider_width = 200.0;
        egui::Grid::new("env_editor_cloud_color").num_columns(2).show(ui, |ui| {
            color(ui, p, aurora_i18n::tr!("ui-env-editor-cloud-color"), &mut s.cloud_color, 1.0);
            ui.end_row();
        });
        slider(
            ui,
            p,
            aurora_i18n::tr!("ui-environment-cloud-coverage"),
            &mut s.cloud_shadow,
            0.0..=1.0,
        );
        slider(ui, p, aurora_i18n::tr!("ui-environment-cloud-size"), &mut s.cloud_scale, 0.01..=3.0);
        slider(
            ui,
            p,
            aurora_i18n::tr!("ui-env-editor-cloud-variance"),
            &mut s.cloud_variance,
            0.0..=1.0,
        );
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-env-editor-cloud-scroll"))
                        .size(12.0)
                        .color(p.ink),
                );
                xy_pad(ui, p, "cloud_scroll", &mut s.cloud_scroll_rate, 30.0, true);
            });
            ui.add_space(16.0);
            let id = s.cloud_id;
            self.texture(ui, p, images, TexSlot::Cloud, aurora_i18n::tr!("ui-env-editor-cloud-image"), id);
        });
        ui.add_space(4.0);
        for (label, v, d_max) in [
            (aurora_i18n::tr!("ui-env-editor-cloud-density"), &mut s.cloud_pos_density1, 3.0),
            (aurora_i18n::tr!("ui-env-editor-cloud-detail"), &mut s.cloud_pos_density2, 1.0),
        ] {
            ui.label(RichText::new(label).size(12.5).strong().color(p.ink));
            slider(ui, p, aurora_i18n::tr!("ui-env-editor-axis-x"), &mut v.x, 0.0..=1.0);
            slider(ui, p, aurora_i18n::tr!("ui-env-editor-axis-y"), &mut v.y, 0.0..=1.0);
            slider(ui, p, aurora_i18n::tr!("ui-env-editor-axis-d"), &mut v.z, 0.0..=d_max);
        }
        if let Some(doc) = &mut self.doc
            && doc.edit_sky(&before, &s)
        {
            self.mark(out);
        }
    }

    fn sun_moon_panel(&mut self, ui: &mut egui::Ui, p: &Palette, images: &HashMap<Uuid, egui::TextureHandle>, out: &mut Vec<EditorAction>) {
        let Some(doc) = &self.doc else { return };
        let before = doc.sky();
        let mut s = before;
        ui.spacing_mut().slider_width = 180.0;
        widgets::section_header(ui, p, "sun-filled", aurora_i18n::tr!("ui-environment-sun"));
        ui.horizontal_top(|ui| {
            let (mut az, mut el) = azimuth_elevation(s.sun_rotation);
            if sky_dome(ui, p, "sun_dome", &mut az, &mut el, p.amber) {
                s.sun_rotation = rotation_from(az, el);
            }
            ui.vertical(|ui| {
                let a = slider(ui, p, aurora_i18n::tr!("ui-environment-azimuth"), &mut az, 0.0..=359.99).changed();
                let e = slider(ui, p, aurora_i18n::tr!("ui-environment-elevation"), &mut el, -90.0..=90.0).changed();
                if a || e {
                    s.sun_rotation = rotation_from(az, el);
                }
                slider(ui, p, aurora_i18n::tr!("common-scale"), &mut s.sun_scale, 0.25..=20.0);
                // glow: size 40..0.2 shown as 0..1.99, focus scaled by -5
                let mut focus = s.glow.z / SLIDER_SCALE_GLOW_B;
                let mut size = 2.0 - s.glow.x / SLIDER_SCALE_GLOW_R;
                let f = slider(ui, p, aurora_i18n::tr!("ui-environment-glare-sharpness"), &mut focus, -2.0..=2.0).changed();
                let z = slider(ui, p, aurora_i18n::tr!("ui-environment-glow-size"), &mut size, 0.0..=1.99).changed();
                if f || z {
                    s.glow = Vec3::new((2.0 - size) * SLIDER_SCALE_GLOW_R, 0.0, focus * SLIDER_SCALE_GLOW_B);
                }
                slider(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-environment-star-brightness"),
                    &mut s.star_brightness,
                    0.0..=500.0,
                );
            });
        });
        ui.horizontal_top(|ui| {
            egui::Grid::new("env_editor_sun_color").num_columns(2).show(ui, |ui| {
                color(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-env-editor-sun-color"),
                    &mut s.sunlight,
                    SLIDER_SCALE_SUN_AMBIENT,
                );
                ui.end_row();
            });
        });
        let sun_id = s.sun_id;
        self.texture(ui, p, images, TexSlot::Sun, aurora_i18n::tr!("ui-env-editor-sun-image"), sun_id);
        ui.add_space(6.0);
        widgets::section_header(ui, p, "moon-filled", aurora_i18n::tr!("ui-environment-moon"));
        ui.horizontal_top(|ui| {
            let (mut az, mut el) = azimuth_elevation(s.moon_rotation);
            if sky_dome(ui, p, "moon_dome", &mut az, &mut el, p.violet_light) {
                s.moon_rotation = rotation_from(az, el);
            }
            ui.vertical(|ui| {
                let a = slider(ui, p, aurora_i18n::tr!("ui-environment-azimuth"), &mut az, 0.0..=359.99).changed();
                let e = slider(ui, p, aurora_i18n::tr!("ui-environment-elevation"), &mut el, -90.0..=90.0).changed();
                if a || e {
                    s.moon_rotation = rotation_from(az, el);
                }
                slider(ui, p, aurora_i18n::tr!("common-scale"), &mut s.moon_scale, 0.25..=20.0);
                slider(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-env-editor-moon-brightness"),
                    &mut s.moon_brightness,
                    0.0..=1.0,
                );
            });
        });
        let moon_id = s.moon_id;
        self.texture(ui, p, images, TexSlot::Moon, aurora_i18n::tr!("ui-env-editor-moon-image"), moon_id);
        if let Some(doc) = &mut self.doc
            && doc.edit_sky(&before, &s)
        {
            self.mark(out);
        }
    }

    fn water_panel(&mut self, ui: &mut egui::Ui, p: &Palette, images: &HashMap<Uuid, egui::TextureHandle>, out: &mut Vec<EditorAction>) {
        let Some(doc) = &self.doc else { return };
        let before = doc.water();
        let mut w = before;
        let mut extras = [doc.extra(env_edit::SCALE_ABOVE), doc.extra(env_edit::SCALE_BELOW)];
        let mut extra_changed = false;
        ui.spacing_mut().slider_width = 200.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                egui::Grid::new("env_editor_fog").num_columns(2).show(ui, |ui| {
                    color(ui, p, aurora_i18n::tr!("ui-env-editor-fog-color"), &mut w.fog_color, 1.0);
                    ui.end_row();
                });
                slider(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-env-editor-fog-density"),
                    &mut w.fog_density,
                    0.0..=100.0,
                );
                slider(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-env-editor-underwater-mod"),
                    &mut w.underwater_fog_mod,
                    0.0..=20.0,
                );
                slider(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-env-editor-fresnel-scale"),
                    &mut w.fresnel_scale,
                    0.0..=1.0,
                );
                slider(
                    ui,
                    p,
                    aurora_i18n::tr!("ui-env-editor-fresnel-offset"),
                    &mut w.fresnel_offset,
                    0.0..=1.0,
                );
            });
            ui.add_space(10.0);
            let id = w.normal_map;
            self.texture(ui, p, images, TexSlot::Normal, aurora_i18n::tr!("ui-env-editor-normal-map"), id);
        });
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            // shown flipped "so that north and east are +"
            for (label, wave, salt) in [
                (aurora_i18n::tr!("ui-env-editor-large-wave"), &mut w.wave1, "wave1"),
                (aurora_i18n::tr!("ui-env-editor-small-wave"), &mut w.wave2, "wave2"),
            ] {
                ui.vertical(|ui| {
                    ui.label(RichText::new(label).size(12.0).color(p.ink));
                    let mut shown = -*wave;
                    if xy_pad(ui, p, salt, &mut shown, 20.0, false) {
                        *wave = -shown;
                    }
                });
                ui.add_space(12.0);
            }
        });
        ui.label(
            RichText::new(aurora_i18n::tr!("ui-env-editor-wavelet-scale"))
                .size(12.5)
                .strong()
                .color(p.ink),
        );
        slider(ui, p, aurora_i18n::tr!("ui-env-editor-axis-x"), &mut w.normal_scale.x, 0.0..=10.0);
        slider(ui, p, aurora_i18n::tr!("ui-env-editor-axis-y"), &mut w.normal_scale.y, 0.0..=10.0);
        slider(ui, p, aurora_i18n::tr!("ui-env-editor-axis-z"), &mut w.normal_scale.z, 0.0..=10.0);
        for (i, label) in [
            aurora_i18n::tr!("ui-env-editor-refract-above"),
            aurora_i18n::tr!("ui-env-editor-refract-below"),
        ]
        .into_iter()
        .enumerate()
        {
            if slider(ui, p, label, &mut extras[i], 0.0..=3.0).changed() {
                extra_changed = true;
            }
        }
        slider(ui, p, aurora_i18n::tr!("ui-env-editor-blur"), &mut w.blur_multiplier, 0.0..=0.5);
        if let Some(doc) = &mut self.doc {
            if extra_changed {
                doc.set_extra(env_edit::SCALE_ABOVE, extras[0]);
                doc.set_extra(env_edit::SCALE_BELOW, extras[1]);
            }
            if doc.edit_water(&before, &w) || extra_changed {
                self.mark(out);
            }
        }
    }
}

/// LLXYVector: a square pad with an arrow from its centre, and the two
/// values below; `log` spreads small values like its logarithmic mode
/// (offset = ln(|v| + 1) / ln(max)). Returns true when changed.
fn xy_pad(ui: &mut egui::Ui, p: &Palette, salt: &str, v: &mut V2, range: f32, log: bool) -> bool {
    let to_pad = |x: f32| {
        if log {
            x.signum() * (x.abs() + 1.0).ln() / range.ln()
        } else {
            x / range
        }
    };
    let from_pad = |t: f32| {
        if log { t.signum() * (range.powf(t.abs()) - 1.0) } else { t * range }
    };
    let mut changed = false;
    let enabled = ui.is_enabled();
    let side = 120.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, ui.visuals().window_corner_radius, p.field);
    let c = rect.center();
    painter.line_segment([egui::pos2(rect.left(), c.y), egui::pos2(rect.right(), c.y)], (1.0, p.raised));
    painter.line_segment([egui::pos2(c.x, rect.top()), egui::pos2(c.x, rect.bottom())], (1.0, p.raised));
    if (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let d = (pos - c) / (side * 0.5);
        // screen y grows downwards: up is +
        *v = V2::new(from_pad(d.x.clamp(-1.0, 1.0)), from_pad(-d.y.clamp(-1.0, 1.0)));
        changed = true;
    }
    let tip = c + Vec2::new(to_pad(v.x), -to_pad(v.y)) * side * 0.5;
    painter.arrow(c, tip - c, (2.0, if enabled { p.violet } else { p.muted }));
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.x = 50.0;
        ui.push_id(salt, |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut v.x)
                        .range(-range..=range)
                        .speed(0.01)
                        .max_decimals(2)
                        .prefix("X "),
                )
                .changed();
            changed |= ui
                .add(
                    egui::DragValue::new(&mut v.y)
                        .range(-range..=range)
                        .speed(0.01)
                        .max_decimals(2)
                        .prefix("Y "),
                )
                .changed();
        });
    });
    changed
}

/// LLVirtualTrackball, seen from above: the zenith in the middle, the
/// horizon on the inner ring, the nadir on the edge; east to the right,
/// azimuth clockwise. Drag to place the sun or the moon.
fn sky_dome(ui: &mut egui::Ui, p: &Palette, salt: &str, az: &mut f32, el: &mut f32, body: egui::Color32) -> bool {
    let enabled = ui.is_enabled();
    let side = 128.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), if enabled { Sense::click_and_drag() } else { Sense::hover() });
    let r = side * 0.5 - 6.0;
    let c = rect.center();
    let painter = ui.painter_at(rect);
    painter.circle_filled(c, r, p.field);
    painter.circle_stroke(c, r * 0.5, (1.0, p.raised));
    painter.circle_stroke(c, r, (1.0, p.raised));
    let mut changed = false;
    if (resp.dragged() || resp.clicked())
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let d = pos - c;
        let dist = (d.length() / r).clamp(0.0, 1.0);
        *el = 90.0 - dist * 180.0;
        let mut a = d.y.atan2(d.x).to_degrees();
        if a < 0.0 {
            a += 360.0;
        }
        *az = a.min(359.99);
        changed = true;
    }
    let dist = (90.0 - *el) / 180.0 * r;
    let a = az.to_radians();
    let at = c + Vec2::new(a.cos(), a.sin()) * dist;
    painter.circle_filled(at, 6.0, if enabled { body } else { p.muted });
    resp.on_hover_text(aurora_i18n::tr!("ui-env-editor-dome-tip"));
    let _ = salt;
    changed
}

/// The frames a document shows, for the environment's edit slot.
pub fn frames(doc: &SettingsDoc) -> (Option<SkyFrame>, Option<WaterFrame>) {
    match doc.kind() {
        SettingsKind::Water => (None, Some(doc.water())),
        _ => (Some(doc.sky()), None),
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    #[test]
    fn live_picker_updates_an_environment_slot_repeatedly_and_cancel_preserves_other_edits() {
        let mut ui = crate::ui::texture_picker::tests::PickerUi::new();
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let original = Uuid::from_u128(77);
        let sky = SkyFrame {
            cloud_id: original,
            ..Default::default()
        };
        let mut editor = EnvEditorUi::default();
        editor.start(SettingsDoc::snapshot_sky(&sky, "Test"), None, Rights::default());
        editor.demo_cloud_picker();
        ui.settle(|ctx, p| {
            editor.show_picker(ctx, p, &mut world, &HashMap::new());
        });
        ui.click(aurora_i18n::tr!("common-blank"), |ctx, p| {
            editor.show_picker(ctx, p, &mut world, &HashMap::new());
        });
        assert_eq!(editor.doc.as_ref().unwrap().sky().cloud_id, aurora_prim::te::BLANK_TEXTURE);
        assert!(editor.picker.open);
        ui.click(aurora_i18n::tr!("common-transparent"), |ctx, p| {
            editor.show_picker(ctx, p, &mut world, &HashMap::new());
        });
        assert_eq!(editor.doc.as_ref().unwrap().sky().cloud_id, aurora_prim::te::TRANSPARENT_TEXTURE);
        let doc = editor.doc.as_mut().unwrap();
        let before = doc.sky();
        let sun = Uuid::from_u128(88);
        doc.edit_sky(&before, &SkyFrame { sun_id: sun, ..before });
        ui.click(aurora_i18n::tr!("common-cancel"), |ctx, p| {
            editor.show_picker(ctx, p, &mut world, &HashMap::new());
        });
        let after = editor.doc.as_ref().unwrap().sky();
        assert_eq!(after.cloud_id, original);
        assert_eq!(after.sun_id, sun);
        assert_eq!(editor.picking, None);
    }

    /// The editor keeps its size, with the note of a region without
    /// inventory settings beside its buttons.
    #[test]
    fn the_window_keeps_its_size() {
        let inv = Inventory::default();
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[Vec2::new(560.0, 600.0), Vec2::new(500.0, 480.0)],
            aurora_i18n::tr!("common-cancel"),
            || {
                let mut ui = EnvEditorUi::default();
                ui.start(SettingsDoc::snapshot_sky(&SkyFrame::default(), "Test"), None, Rights::default());
                ui
            },
            |ui, ctx, p| {
                ui.show(ctx, p, &inv, &HashMap::new(), EditorAccess::default());
            },
        );
    }
}
