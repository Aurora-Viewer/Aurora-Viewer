//! One live lens-flare window for the World menu and the Photo Viewfinder.

use std::ops::RangeInclusive;

use aurora_i18n::tr;
use aurora_render::lens_flare::{LensFlare, MAX_THRESHOLD, Quality};

use super::widgets;
use crate::settings::Settings as ViewerSettings;
use crate::settings::lens_flare::{BuiltIn, Selection};
use crate::theme::Palette;

pub(crate) const WINDOW: &str = "screen_space_lens_flare";

#[derive(Default)]
pub struct Ui {
    open: bool,
    advanced: bool,
    edit: Option<Edit>,
    name: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Edit {
    SaveAs,
    Rename,
    Delete,
}

impl Ui {
    pub fn open(&mut self, ctx: &egui::Context) {
        widgets::open_floater(ctx, WINDOW, &mut self.open);
    }

    pub fn open_advanced(&mut self, ctx: &egui::Context) {
        self.advanced = true;
        self.open(ctx);
    }

    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, settings: &mut ViewerSettings) -> bool {
        if !self.open {
            return false;
        }
        let mut open = self.open;
        let screen = ctx.content_rect();
        let mut floater = widgets::Floater::new(
            WINDOW,
            tr!("lens-flare-title"),
            egui::pos2(screen.left() + 24.0, screen.top() + 90.0),
            egui::vec2(420.0, 360.0),
        )
        .steady_size();
        floater.min_size = egui::vec2(340.0, 200.0);
        let changed = floater
            .show(ctx, p, &mut open, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("flare_controls")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.body(ui, p, settings))
                    .inner
            })
            .unwrap_or(false);
        self.open = open;
        if !open {
            self.edit = None;
        }
        changed
    }

    fn body(&mut self, ui: &mut egui::Ui, p: &Palette, viewer: &mut ViewerSettings) -> bool {
        let ViewerSettings {
            lens_flare: settings,
            bloom,
            bloom_strength,
            ..
        } = viewer;
        let before = settings.current;
        let mut changed = widgets::switch_labeled(ui, p, &mut settings.enabled, tr!("lens-flare-enable")).changed();
        let mut selected = None;
        ui.horizontal_wrapped(|ui| {
            ui.label(tr!("lens-flare-preset"));
            egui::ComboBox::from_id_salt("preset")
                .selected_text(settings.selection.label())
                .width(158.0)
                .truncate()
                .show_ui(ui, |ui| {
                    for built_in in BuiltIn::ALL {
                        let selection = Selection::BuiltIn(built_in);
                        if ui.selectable_label(settings.selection == selection, built_in.label()).clicked() {
                            selected = Some(selection);
                        }
                    }
                    for name in settings.named.keys() {
                        let selection = Selection::Named(name.clone());
                        if ui.selectable_label(settings.selection == selection, name).clicked() {
                            selected = Some(selection);
                        }
                    }
                });
            if settings.modified() {
                ui.label(egui::RichText::new(tr!("lens-flare-modified")).color(p.muted));
            }
        });
        if let Some(selection) = selected.filter(|selection| *selection != settings.selection) {
            settings.select(selection);
            self.edit = None;
            changed = true;
        }
        slider(ui, tr!("lens-flare-intensity"), &mut settings.current.intensity, 0.0..=2.0);
        quality_combo(ui, &mut settings.current.quality);
        widgets::switch_labeled(ui, p, &mut self.advanced, tr!("lens-flare-advanced"));
        if self.advanced {
            changed |= bloom_controls(ui, p, bloom, bloom_strength);
            advanced(ui, p, &mut settings.current);
        }
        widgets::divider(ui, p);
        ui.horizontal_wrapped(|ui| {
            if ui.button(tr!("common-save-as")).clicked() {
                self.edit = Some(Edit::SaveAs);
                self.name.clear();
            }
            if matches!(&settings.selection, Selection::Named(_)) {
                if ui
                    .add_enabled(settings.modified(), egui::Button::new(tr!("lens-flare-update")))
                    .clicked()
                {
                    settings.update();
                    changed = true;
                }
                if ui.button(tr!("common-rename")).clicked() {
                    self.edit = Some(Edit::Rename);
                    self.name = settings.selection.label().to_owned();
                }
                if ui.button(tr!("common-delete")).clicked() {
                    self.edit = Some(Edit::Delete);
                }
            }
            if settings.modified() && ui.button(tr!("lens-flare-revert")).clicked() {
                settings.revert();
            }
        });
        if let Some(edit) = self.edit {
            ui.horizontal_wrapped(|ui| {
                let (label, enabled) = if edit == Edit::Delete {
                    ui.label(tr!("lens-flare-delete-confirm"));
                    (tr!("common-delete"), true)
                } else {
                    ui.label(tr!("lens-flare-name"));
                    ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(140.0).char_limit(64));
                    let label = if edit == Edit::Rename {
                        tr!("common-rename")
                    } else {
                        tr!("common-save-as")
                    };
                    (label, settings.available_name(&self.name))
                };
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    match edit {
                        Edit::SaveAs => settings.save_as(&self.name),
                        Edit::Rename => settings.rename(&self.name),
                        Edit::Delete => settings.delete(),
                    }
                    self.edit = None;
                    changed = true;
                }
                if ui.button(tr!("common-cancel")).clicked() {
                    self.edit = None;
                }
            });
        }
        changed || before != settings.current
    }
}

fn slider(ui: &mut egui::Ui, label: &str, value: &mut f32, range: RangeInclusive<f32>) -> egui::Response {
    labeled_slider(
        ui,
        label,
        egui::Slider::new(value, range)
            .custom_parser(aurora_i18n::parse_number)
            .max_decimals(2),
    )
}

fn labeled_slider(ui: &mut egui::Ui, label: &str, control: egui::Slider<'_>) -> egui::Response {
    ui.horizontal(|ui| {
        ui.label(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.add(control))
            .inner
    })
    .inner
}

fn quality_combo(ui: &mut egui::Ui, quality: &mut Quality) {
    ui.horizontal(|ui| {
        ui.label(tr!("lens-flare-quality"));
        egui::ComboBox::from_id_salt("quality")
            .selected_text(quality_label(*quality))
            .show_ui(ui, |ui| {
                for level in [Quality::Performance, Quality::Balanced, Quality::High] {
                    ui.selectable_value(quality, level, quality_label(level));
                }
            });
    });
}

fn quality_label(quality: Quality) -> &'static str {
    match quality {
        Quality::Performance => tr!("lens-flare-performance"),
        Quality::Balanced => tr!("lens-flare-balanced"),
        Quality::High => tr!("lens-flare-high"),
    }
}

fn bloom_controls(ui: &mut egui::Ui, p: &Palette, enabled: &mut bool, strength: &mut f32) -> bool {
    let toggle = widgets::switch_labeled(ui, p, enabled, tr!("ui-options-bloom")).on_hover_text(tr!("ui-options-bloom-help"));
    let slider = ui
        .add_enabled_ui(*enabled, |ui| {
            labeled_slider(
                ui,
                tr!("ui-options-bloom-strength"),
                super::options::bloom_strength_slider(strength),
            )
        })
        .inner
        .on_hover_text(tr!("ui-options-bloom-strength-help"));
    widgets::divider(ui, p);
    toggle.changed() || slider.changed()
}

fn advanced(ui: &mut egui::Ui, p: &Palette, flare: &mut LensFlare) {
    labeled_slider(
        ui,
        tr!("lens-flare-threshold"),
        egui::Slider::new(&mut flare.threshold, 0.0..=MAX_THRESHOLD)
            .logarithmic(true)
            .smallest_positive(0.05)
            .custom_parser(aurora_i18n::parse_number)
            .max_decimals(2),
    )
    .on_hover_text(tr!("lens-flare-threshold-help"));
    slider(ui, tr!("lens-flare-threshold-smoothness"), &mut flare.knee, 0.0..=1.0)
        .on_hover_text(tr!("lens-flare-threshold-smoothness-help"));
    slider(ui, tr!("lens-flare-blur"), &mut flare.blur, 0.0..=1.0).on_hover_text(tr!("lens-flare-blur-help"));
    ui.horizontal(|ui| {
        ui.label(tr!("lens-flare-tint"));
        let [r, g, b] = flare.tint.map(|v| (v * 255.0).round() as u8);
        let mut rgba = [r, g, b, 255];
        if widgets::color_button(ui, p, "lens_flare_tint", &mut rgba, false).changed() {
            let [r, g, b, _] = rgba.map(|v| f32::from(v) / 255.0);
            flare.tint = [r, g, b];
        }
    });

    widgets::section_header(ui, p, "circles-filled", tr!("lens-flare-ghosts"));
    slider(ui, tr!("lens-flare-strength"), &mut flare.ghosts, 0.0..=1.0);
    ui.horizontal(|ui| {
        ui.label(tr!("lens-flare-count"));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Slider::new(&mut flare.ghost_count, 1..=6));
        });
    });
    slider(ui, tr!("lens-flare-spread"), &mut flare.ghost_spread, 0.0..=1.0);
    slider(ui, tr!("lens-flare-softness"), &mut flare.ghost_softness, 0.0..=1.0);

    widgets::section_header(ui, p, "circle-dashed", tr!("lens-flare-halo"));
    slider(ui, tr!("lens-flare-strength"), &mut flare.halo, 0.0..=1.0);
    slider(ui, tr!("lens-flare-radius"), &mut flare.halo_radius, 0.0..=1.0);
    slider(ui, tr!("lens-flare-softness"), &mut flare.halo_softness, 0.0..=1.0);

    widgets::section_header(ui, p, "line", tr!("lens-flare-streaks"));
    slider(ui, tr!("lens-flare-strength"), &mut flare.streaks, 0.0..=1.0);
    slider(ui, tr!("lens-flare-length"), &mut flare.streak_length, 0.0..=1.0);
    slider(ui, tr!("lens-flare-taper"), &mut flare.streak_taper, 0.0..=1.0);
    slider(ui, tr!("lens-flare-angle"), &mut flare.streak_angle, 0.0..=180.0);

    widgets::section_header(ui, p, "sparkles-filled", tr!("lens-flare-starburst"));
    slider(ui, tr!("lens-flare-strength"), &mut flare.starburst, 0.0..=1.0);
    slider(ui, tr!("lens-flare-length"), &mut flare.star_length, 0.0..=1.0);
    slider(ui, tr!("lens-flare-angle"), &mut flare.star_angle, 0.0..=180.0);
    ui.horizontal(|ui| {
        ui.label(tr!("lens-flare-rays"));
        for n in [4, 6, 8] {
            widgets::radio_value(ui, p, &mut flare.star_rays, n, n.to_string());
        }
    });
    slider(ui, tr!("lens-flare-chromatic"), &mut flare.chromatic, 0.0..=1.0);
}
