//! Object › Save as › Collada / OBJ / glTF: the export window
//! (ColladaExportFloater, floater_export_collada.xml,
//! indra/newview/daeexport.cpp, Latif Khalifa and Cinder Biscuits, LGPL
//! 2.1) in the Aurora skin. The object's name and how many of its prims and
//! textures may be written, the options, the textures that will be saved,
//! then Save as. The format (Collada, OBJ or glTF) is chosen here too: the
//! Legacy Viewer has Collada only. The work is `app/object_export.rs`.

use super::common::{self, combo, label, section};
use crate::build::export::{Format, ImageFormat, Settings};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind, Floater};
use egui::{RichText, Vec2};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// The window's id.
pub const WINDOW: &str = "object_export";
/// Width of a texture preview (the Legacy Viewer's 100 px tiles).
const TILE: f32 = 84.0;

/// What the window asks of the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Save as: ask for the file, then write it.
    Save,
}

/// Where the selection stands, worked out by the app.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Summary {
    /// Prims with a volume in the selection, and those we may export.
    pub total: usize,
    pub allowed: usize,
    /// Some prims still wait for their properties or their shape.
    pub waiting: bool,
    /// Textures of the exportable prims, and those we may save, with the
    /// file name each gets (the previews' tooltips).
    pub textures_total: usize,
    pub textures: Vec<(Uuid, String)>,
}

/// An export in progress.
#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    /// The file dialog is open.
    Choosing,
    /// The prims' shapes are read and the file written.
    Writing,
    /// The textures are being saved: how many are left of how many.
    Textures { left: usize, total: usize },
}

#[derive(Default)]
pub struct ExportWindow {
    pub open: bool,
    /// The root's name (ObjectProperties).
    pub name: String,
    pub summary: Summary,
    pub progress: Option<Progress>,
    /// What the last Save as did, when it did not close the window.
    pub notice: Option<String>,
    pub wanted_images: HashSet<Uuid>,
}

impl ExportWindow {
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        settings: &mut Settings,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> Option<Action> {
        if !self.open {
            return None;
        }
        let mut open = true;
        let mut action = None;
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("object-export-title"),
            egui::pos2(120.0, 120.0),
            Vec2::new(430.0, 660.0),
        );
        floater.min_size = Vec2::new(300.0, 300.0);
        floater
            .title_note(self.name.clone())
            .show(ctx, p, &mut open, |ui| action = self.body(ui, p, settings, images));
        // the window stays until the file is written (setCanClose)
        if !open && self.progress.is_none() {
            self.open = false;
        }
        action
    }

    fn body(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        settings: &mut Settings,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> Option<Action> {
        common::typography(ui);
        let busy = self.progress.is_some();
        let s = self.summary.clone();
        egui::ScrollArea::vertical()
            .auto_shrink([false, true])
            .max_height((ui.available_height() - 52.0).max(80.0))
            .show(ui, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    section(ui, p, aurora_i18n::tr!("object-export-format"));
                    ui.horizontal(|ui| {
                        widgets::radio_value(
                            ui,
                            p,
                            &mut settings.format,
                            Format::Collada,
                            aurora_i18n::tr!("object-export-collada"),
                        );
                        ui.add_space(12.0);
                        widgets::radio_value(ui, p, &mut settings.format, Format::Obj, aurora_i18n::tr!("object-export-obj"));
                        ui.add_space(12.0);
                        widgets::radio_value(ui, p, &mut settings.format, Format::Gltf, aurora_i18n::tr!("object-export-gltf"));
                    });
                    ui.add_space(8.0);
                    section(ui, p, aurora_i18n::tr!("object-export-object-info"));
                    common::text(ui, p, &aurora_i18n::tr!("object-export-prims", count = s.allowed, total = s.total));
                    common::text(
                        ui,
                        p,
                        &aurora_i18n::tr!("object-export-textures", count = s.textures.len(), total = s.textures_total),
                    );
                    if s.waiting {
                        common::small(ui, p, aurora_i18n::tr!("object-export-waiting"));
                    } else if s.allowed == 0 {
                        ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("object-export-nothing")).size(11.0).color(p.muted)).wrap());
                    }
                    ui.add_space(8.0);
                    section(ui, p, aurora_i18n::tr!("object-export-options"));
                    let textures = !s.textures.is_empty();
                    ui.add_enabled_ui(textures, |ui| {
                        widgets::checkbox(ui, p, &mut settings.textures, aurora_i18n::tr!("object-export-save-textures"));
                    });
                    widgets::checkbox(ui, p, &mut settings.consolidate, aurora_i18n::tr!("object-export-consolidate"));
                    widgets::checkbox(
                        ui,
                        p,
                        &mut settings.skip_transparent,
                        aurora_i18n::tr!("object-export-skip-transparent"),
                    );
                    widgets::checkbox(
                        ui,
                        p,
                        &mut settings.texture_params,
                        aurora_i18n::tr!("object-export-texture-params"),
                    );
                    ui.add_enabled_ui(settings.format == Format::Collada, |ui| {
                        widgets::checkbox(ui, p, &mut settings.single_uv_map, aurora_i18n::tr!("object-export-single-uv"))
                            .on_hover_text(aurora_i18n::tr!("object-export-single-uv-tip"));
                    });
                    // a glTF file holds its textures as PNG
                    let gltf = settings.format == Format::Gltf;
                    let row = ui.horizontal(|ui| {
                        label(ui, p, aurora_i18n::tr!("object-export-texture-format"));
                        let formats: Vec<_> = ImageFormat::ALL.iter().map(|f| (f.label(), *f)).collect();
                        let shown = if gltf { ImageFormat::Png } else { settings.texture_format };
                        if let Some(f) = combo(
                            ui,
                            "object_export_image_format",
                            Some(shown),
                            &formats,
                            120.0,
                            textures && settings.textures && !gltf,
                        ) {
                            settings.texture_format = f;
                        }
                    });
                    if gltf {
                        row.response.on_hover_text(aurora_i18n::tr!("object-export-gltf-png"));
                    }
                    if textures && settings.textures {
                        ui.add_space(8.0);
                        section(ui, p, aurora_i18n::tr!("object-export-texture-list"));
                        self.previews(ui, p, images);
                    }
                });
            });
        widgets::divider(ui, p);
        let mut action = None;
        ui.horizontal(|ui| {
            match &self.progress {
                Some(Progress::Textures { left, total }) => {
                    let done = total.saturating_sub(*left);
                    let fraction = if *total == 0 { 1.0 } else { done as f32 / *total as f32 };
                    let text = aurora_i18n::tr!("object-export-saving-textures", count = *left);
                    widgets::progress(ui, p, "object_export_progress", fraction, 220.0, &text, None);
                }
                Some(Progress::Writing) => {
                    common::small(ui, p, aurora_i18n::tr!("object-export-writing"));
                }
                Some(Progress::Choosing) | None => {
                    if let Some(notice) = &self.notice {
                        ui.add(egui::Label::new(RichText::new(notice).size(11.0).color(p.muted)).wrap());
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let save = widgets::action_button_if(
                    ui,
                    p,
                    "device-floppy-filled",
                    aurora_i18n::tr!("object-export-save"),
                    ActionKind::Primary,
                    Vec2::new(0.0, 28.0),
                    !busy && self.summary.allowed > 0,
                );
                if save.clicked() {
                    action = Some(Action::Save);
                }
            });
        });
        action
    }

    /// addTexturePreview: the textures that will be saved, by their file
    /// name (never their id).
    fn previews(&mut self, ui: &mut egui::Ui, p: &Palette, images: &HashMap<Uuid, egui::TextureHandle>) {
        let textures = self.summary.textures.clone();
        ui.horizontal_wrapped(|ui| {
            for (id, name) in &textures {
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(TILE), egui::Sense::hover());
                match images.get(id) {
                    Some(texture) => {
                        egui::Image::new(texture)
                            .corner_radius(ui.visuals().widgets.inactive.corner_radius)
                            .paint_at(ui, rect);
                    }
                    None => {
                        self.wanted_images.insert(*id);
                        ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.field);
                    }
                }
                response.on_hover_text(name.as_str());
            }
        });
    }
}
