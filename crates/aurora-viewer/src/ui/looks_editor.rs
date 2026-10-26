//! The photo's looks editor (issue #969), in the viewfinder's filter panel
//! and the photo window's Filters section: the stack of looks (each
//! switched on or off, moved, taken out, chosen), whether a click in the
//! list replaces the chosen look or adds one, the list in three tabs
//! (Legacy, Aurora by section, the user's tables) with a search, and the
//! chosen look's strength and settings. The look pointed at in the list is
//! tried on the live view (`snapshot::looks::tried`).

use super::snapshot::{Action, chip};
use super::widgets;
use crate::snapshot::Settings;
use crate::snapshot::knobs::{self, Kind, Knob};
use crate::snapshot::looks::{self, Entry, MAX_LAYERS, Mode, Tab};
use crate::theme::Palette;
use egui::{RichText, Vec2};

/// The editor's state for the session.
#[derive(Debug, Default)]
pub struct LooksUi {
    pub tab: Tab,
    pub query: String,
    pub mode: Mode,
    /// The layer the settings show and a click replaces.
    pub selected: usize,
    /// The look pointed at in the list, tried on the live view until the
    /// next frame of the interface (Some(None): "No filter").
    pub hover: Option<Option<String>>,
    /// The before / after comparison of the live view: the share of the
    /// photo's width left as it is.
    pub compare: Option<f32>,
    /// The photo window's settings scroll to the filters once
    /// (AURORA_DEMO_PHOTO_LOOK).
    pub reveal: bool,
}

/// What the editor shows besides the looks.
pub struct Options<'a> {
    /// The list's height before it scrolls.
    pub list_height: f32,
    /// The before / after switch (the viewfinder).
    pub compare: bool,
    /// The user's tables, by file name.
    pub luts: &'a [String],
}

fn keep_keys(resp: &egui::Response) {
    if resp.clicked() || resp.drag_stopped() {
        resp.surrender_focus();
    }
}

/// A section's title: small, muted, in capitals.
fn heading(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text.to_uppercase()).size(10.5).color(p.muted));
}

impl LooksUi {
    /// The whole editor.
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings, opts: Options, out: &mut Vec<Action>) {
        self.selected = self.selected.min(s.looks.len().saturating_sub(1));
        self.stack(ui, p, s);
        if !s.looks.is_empty() {
            let mut mode = usize::from(self.mode == Mode::Add);
            let full = s.looks.len() >= MAX_LAYERS;
            if widgets::segmented(
                ui,
                p,
                &mut mode,
                &[
                    (aurora_i18n::tr!("photo-looks-replace"), true),
                    (aurora_i18n::tr!("photo-looks-add"), !full),
                ],
            ) {
                self.mode = if mode == 1 { Mode::Add } else { Mode::Replace };
            }
        }
        let labels: Vec<(&str, bool)> = Tab::ALL.iter().map(|t| (t.label(), true)).collect();
        let mut tab = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        if widgets::stretched_tabs(ui, p, &mut tab, &labels) {
            self.tab = Tab::ALL[tab];
            self.query.clear();
        }
        // in a row: the field centres itself in the height it is given
        ui.horizontal(|ui| {
            let search =
                widgets::clearable_search_box(ui, p, &mut self.query, aurora_i18n::tr!("photo-looks-search"), ui.available_width());
            keep_keys(&search);
        });
        egui::ScrollArea::vertical()
            .id_salt("looks_list")
            .max_height(opts.list_height)
            .auto_shrink([false, true])
            .show(ui, |ui| self.list(ui, p, s, opts.luts, out));
        widgets::divider(ui, p);
        self.settings(ui, p, s);
        if opts.compare {
            ui.add_enabled_ui(!s.looks.is_empty(), |ui| {
                let mut compare = self.compare.is_some();
                let resp = widgets::switch_labeled(ui, p, &mut compare, aurora_i18n::tr!("photo-look-compare"))
                    .on_hover_text(aurora_i18n::tr!("photo-look-compare-tip"));
                keep_keys(&resp);
                if resp.changed() {
                    self.compare = compare.then_some(0.5);
                }
            });
        }
    }

    /// The stack: a row per look, the chosen one marked.
    fn stack(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings) {
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("photo-looks-stack", count = s.looks.len()))
                    .strong()
                    .color(p.ink),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let none = chip(ui, p, aurora_i18n::tr!("photo-look-none"), None, s.looks.is_empty(), true);
                keep_keys(&none);
                if none.hovered() {
                    self.hover = Some(None);
                }
                if none.clicked() {
                    s.looks.clear();
                    self.selected = 0;
                    self.mode = Mode::Replace;
                }
            });
        });
        if s.looks.is_empty() {
            ui.label(RichText::new(aurora_i18n::tr!("photo-looks-empty")).size(11.5).color(p.muted));
            return;
        }
        let count = s.looks.len();
        let mut moved: Option<(usize, usize)> = None;
        let mut removed = None;
        for (i, layer) in s.looks.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let on = widgets::checkbox(ui, p, &mut layer.on, "").on_hover_text(aurora_i18n::tr!("photo-looks-on-tip"));
                keep_keys(&on);
                let name = looks::label(Some(&layer.id));
                let from = looks::tab_of(&layer.id).label();
                let text = format!("{name}  ·  {} %", layer.strength);
                let tip = format!("{from} · {}", aurora_i18n::tr!("photo-looks-choose-tip"));
                let row = chip(ui, p, &text, None, i == self.selected, true).on_hover_text(tip);
                keep_keys(&row);
                if row.clicked() {
                    self.selected = i;
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("photo-looks-remove"), true) {
                        removed = Some(i);
                    }
                    if widgets::icon_button(ui, p, "arrow-down", aurora_i18n::tr!("photo-looks-down"), i + 1 < count) {
                        moved = Some((i, i + 1));
                    }
                    if widgets::icon_button(ui, p, "arrow-up", aurora_i18n::tr!("photo-looks-up"), i > 0) {
                        moved = Some((i, i - 1));
                    }
                });
            });
        }
        if let Some((from, to)) = moved {
            s.looks.swap(from, to);
            if self.selected == from {
                self.selected = to;
            } else if self.selected == to {
                self.selected = from;
            }
        }
        if let Some(i) = removed {
            s.looks.remove(i);
            if self.selected >= i && self.selected > 0 {
                self.selected -= 1;
            }
            if s.looks.is_empty() {
                self.mode = Mode::Replace;
            }
        }
    }

    /// The tab's looks, or what the search finds in every tab.
    fn list(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings, luts: &[String], out: &mut Vec<Action>) {
        let query = self.query.trim().to_owned();
        if !query.is_empty() {
            let found = looks::search(&query, luts);
            if found.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("photo-looks-no-result")).color(p.muted));
            } else {
                self.chips(ui, p, &found, s);
            }
            return;
        }
        match self.tab {
            Tab::Legacy => self.chips(ui, p, &looks::legacy_list(), s),
            Tab::Aurora => {
                for (section, entries) in looks::aurora_list() {
                    heading(ui, p, looks::section_name(section));
                    self.chips(ui, p, &entries, s);
                }
            }
            Tab::Luts => self.luts(ui, p, s, luts, out),
        }
    }

    /// A wrapped row of looks: pointed at, tried; clicked, chosen.
    fn chips(&mut self, ui: &mut egui::Ui, p: &Palette, entries: &[Entry], s: &mut Settings) {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(4.0, 4.0);
            for e in entries {
                let used = s.looks.iter().any(|l| l.id == e.id);
                let mut resp = chip(ui, p, &e.name, None, used, true);
                keep_keys(&resp);
                if let Some(tip) = e.tip {
                    resp = resp.on_hover_text(tip);
                }
                if resp.hovered() {
                    self.hover = Some(Some(e.id.clone()));
                }
                if resp.clicked() {
                    self.pick(s, &e.id);
                }
            }
        });
    }

    /// A look clicked: it replaces the chosen layer or goes on top.
    pub fn pick(&mut self, s: &mut Settings, id: &str) {
        self.selected = looks::choose(&mut s.looks, id, self.mode, self.selected);
        // a look added: the next click goes on with it
        self.mode = Mode::Replace;
    }

    /// The user's tables: a row each with its bin, then the import.
    fn luts(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings, luts: &[String], out: &mut Vec<Action>) {
        let entries = looks::lut_list(luts);
        if entries.is_empty() {
            ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("photo-lut-empty")).color(p.muted)).wrap());
        }
        for e in &entries {
            ui.horizontal(|ui| {
                let used = s.looks.iter().any(|l| l.id == e.id);
                let resp = chip(ui, p, &e.name, None, used, true);
                keep_keys(&resp);
                if resp.hovered() {
                    self.hover = Some(Some(e.id.clone()));
                }
                if resp.clicked() {
                    self.pick(s, &e.id);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("photo-lut-remove"), true)
                        && let Some(file) = e.id.strip_prefix(looks::LUT)
                    {
                        out.push(Action::RemoveLut(file.to_owned()));
                    }
                });
            });
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let import = widgets::action_button(
                ui,
                p,
                "file-import",
                aurora_i18n::tr!("photo-lut-import"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 28.0),
            );
            keep_keys(&import);
            if import.clicked() {
                out.push(Action::ImportLut);
            }
            let folder = widgets::action_button(
                ui,
                p,
                "folder-open",
                aurora_i18n::tr!("photo-lut-folder"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 28.0),
            );
            keep_keys(&folder);
            if folder.clicked() {
                out.push(Action::OpenLutFolder);
            }
        });
        ui.add(egui::Label::new(RichText::new(aurora_i18n::tr!("photo-lut-hint")).size(11.0).color(p.muted)).wrap());
    }

    /// The chosen layer's strength and settings.
    fn settings(&mut self, ui: &mut egui::Ui, p: &Palette, s: &mut Settings) {
        let Some(layer) = s.looks.get_mut(self.selected) else {
            return;
        };
        let id = layer.id.clone();
        let name = looks::label(Some(&id));
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(aurora_i18n::tr!("photo-looks-settings", name = name.as_ref()))
                    .strong()
                    .color(p.ink),
            );
        });
        slider_row(ui, p, aurora_i18n::tr!("photo-look-strength"), |ui| {
            ui.add(egui::Slider::new(&mut layer.strength, 0..=100).suffix(" %"))
        });
        let Some(aurora) = id.strip_prefix(looks::AURORA) else {
            let note = if id.starts_with(looks::LUT) {
                aurora_i18n::tr!("photo-looks-lut-note")
            } else {
                aurora_i18n::tr!("photo-looks-legacy-note")
            };
            ui.add(egui::Label::new(RichText::new(note).size(11.0).color(p.muted)).wrap());
            return;
        };
        let Some(look) = aurora_render::looks::aurora_look(aurora) else {
            return;
        };
        let list = knobs::knobs(aurora);
        let defaults = knobs::defaults(list, &look.ops);
        let values = s.look_settings.entry(id.clone()).or_insert_with(|| defaults.clone());
        if values.len() != defaults.len() {
            *values = defaults.clone();
        }
        let mut at = 0;
        for knob in list {
            let width = knob.width();
            knob_row(ui, p, knob, &mut values[at..at + width], &id);
            at += width;
        }
        if *values != defaults
            && widgets::action_button(
                ui,
                p,
                "restore",
                aurora_i18n::tr!("photo-looks-reset"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 24.0),
            )
            .clicked()
        {
            *values = defaults;
        }
    }
}

/// A setting's row: its name on the left, its control on the right.
fn slider_row(ui: &mut egui::Ui, p: &Palette, label: &str, control: impl FnOnce(&mut egui::Ui) -> egui::Response) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).color(p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 70.0).clamp(60.0, 170.0);
            let resp = control(ui);
            keep_keys(&resp);
        });
    });
}

/// One setting of an Aurora look.
fn knob_row(ui: &mut egui::Ui, p: &Palette, knob: &Knob, values: &mut [f32], id: &str) {
    let label = aurora_i18n::text(knob.label);
    match knob.kind {
        Kind::Range { min, max } => {
            let mut percent = ((values[0] - min) / (max - min) * 100.0).clamp(0.0, 100.0);
            slider_row(ui, p, label, |ui| {
                let resp = ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).fixed_decimals(0).suffix(" %"));
                if resp.changed() {
                    values[0] = min + percent / 100.0 * (max - min);
                }
                resp
            });
        }
        Kind::Meters { min, max } => slider_row(ui, p, label, |ui| {
            ui.add(egui::Slider::new(&mut values[0], min..=max).fixed_decimals(0).suffix(" m"))
        }),
        Kind::Ratio { min, max } => slider_row(ui, p, label, |ui| {
            ui.add(
                egui::Slider::new(&mut values[0], min..=max)
                    .fixed_decimals(2)
                    .suffix(":1")
                    .custom_parser(aurora_i18n::parse_number),
            )
        }),
        Kind::Hue => slider_row(ui, p, label, |ui| {
            ui.add(egui::Slider::new(&mut values[0], 0.0..=360.0).fixed_decimals(0).suffix("°"))
        }),
        Kind::Seed => slider_row(ui, p, label, |ui| {
            let mut seed = values[0].round() as i32;
            let resp = ui.add(egui::Slider::new(&mut seed, 0..=99));
            values[0] = seed as f32;
            resp
        }),
        Kind::Color => {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).color(p.ink));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // the look's colours are in the photo's gamma-encoded levels
                    let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                    let mut rgba = [byte(values[0]), byte(values[1]), byte(values[2]), 255];
                    let resp = widgets::color_button(ui, p, (id, knob.label, knob.op), &mut rgba, false);
                    if resp.changed() {
                        for (v, b) in values.iter_mut().zip(rgba) {
                            *v = f32::from(b) / 255.0;
                        }
                    }
                });
            });
        }
    }
}
