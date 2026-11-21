//! Preferences › Language and updates › Share: the windows of a share code
//! (`settings/share.rs` says what one holds). "Share…" picks the pages and
//! copies the code; "Import…" takes a pasted code, lists every change page
//! by page, old value then new, and applies the settings ticked only once
//! the user confirms. The last import can be undone until the viewer closes.

use super::{OptionsResult, OptionsUi, Page};
use crate::settings::Settings;
use crate::settings::registry::Locks;
use crate::settings::share::{self, Catalog, Code, LOOKS, Plan, Refused, SHARED_PAGES};
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_i18n::tr;
use egui::{RichText, Vec2};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};

/// The pages of Preferences and their rows' labels, taken when a window
/// opens (in the interface's language at that time).
struct Rows {
    catalog: Catalog,
    labels: HashMap<String, String>,
}

impl Rows {
    fn new() -> Rows {
        let rows = super::setting_pages();
        let labels = rows.iter().map(|(_, path, label)| (path.clone(), label.clone())).collect();
        Rows {
            catalog: Catalog::new(rows.into_iter().map(|(page, path, _)| (page, path))),
            labels,
        }
    }

    /// A setting's label: its row's, or the row of the map it is in, with
    /// the rest of its path.
    fn label(&self, key: &str) -> String {
        if let Some(label) = self.labels.get(key) {
            return label.clone();
        }
        let mut at = key;
        while let Some((head, _)) = at.rsplit_once('.') {
            if let Some(label) = self.labels.get(head) {
                return format!("{label} › {}", &key[head.len() + 1..]);
            }
            at = head;
        }
        key.to_owned()
    }
}

/// The Share window.
struct Export {
    rows: Rows,
    pages: BTreeSet<&'static str>,
    counts: BTreeMap<&'static str, usize>,
    name: String,
    text: String,
    dirty: bool,
    copied: bool,
}

/// The Import window.
struct Import {
    rows: Rows,
    text: String,
    code: Result<Code, Refused>,
    plan: Plan,
    reset_others: bool,
    /// The settings left unticked, by path.
    unticked: BTreeSet<String>,
    dirty: bool,
}

/// The last import, while it can be undone: the values it replaced.
struct Undo {
    before: Vec<(String, Value)>,
    pages: Vec<&'static str>,
}

#[derive(Default)]
pub struct State {
    export: Option<Export>,
    import: Option<Import>,
    undo: Option<Undo>,
    pub status: String,
    /// `AURORA_DEMO_OPTIONS=system:share` (false) or `system:share-import`
    /// (true, with a sample code pasted): the window to open.
    pub demo: Option<bool>,
}

impl State {
    pub fn open_export(&mut self, s: &Settings) {
        let rows = Rows::new();
        let counts = share::counts(s, &rows.catalog);
        // the looks to start with, or every page changed when the looks are
        // all defaults
        let mut pages: BTreeSet<&'static str> = LOOKS.iter().copied().filter(|p| counts.contains_key(p)).collect();
        if pages.is_empty() {
            pages = counts.keys().copied().collect();
        }
        self.status.clear();
        self.export = Some(Export {
            rows,
            pages,
            counts,
            name: String::new(),
            text: String::new(),
            dirty: true,
            copied: false,
        });
    }

    pub fn open_import(&mut self) {
        self.status.clear();
        self.import = Some(Import {
            rows: Rows::new(),
            text: String::new(),
            code: Err(Refused::NotACode),
            plan: Plan::default(),
            reset_others: false,
            unticked: BTreeSet::new(),
            dirty: false,
        });
    }

    pub fn can_undo(&self) -> bool {
        self.undo.is_some()
    }
}

fn page_of(key: &str) -> Option<Page> {
    Page::ALL.into_iter().find(|p| p.key() == key)
}

fn page_name(key: &str) -> String {
    page_of(key).map_or_else(|| key.to_owned(), |page| page.name().to_owned())
}

/// A value as the Import window shows it.
fn shown(value: &Value) -> String {
    match value {
        Value::Bool(true) => tr!("ui-options-share-on").to_owned(),
        Value::Bool(false) => tr!("ui-options-share-off").to_owned(),
        Value::Number(n) if n.is_f64() => {
            let text = format!("{:.3}", n.as_f64().unwrap_or_default());
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        }
        Value::Number(n) => n.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(items) => items.iter().map(shown).collect::<Vec<_>>().join(", "),
        _ => String::new(),
    }
}

/// A colour setting's swatch, before its numbers.
fn swatch(ui: &mut egui::Ui, value: &Value) {
    let Some(rgba) = value.as_array().filter(|a| a.len() == 4) else {
        return;
    };
    let c: Vec<u8> = rgba.iter().map(|v| v.as_u64().unwrap_or(0).min(255) as u8).collect();
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), egui::Sense::hover());
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let color = egui::Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    ui.painter().rect_filled(rect, radius, color);
}

fn title(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text).size(15.0).strong().color(p.ink));
    ui.add_space(6.0);
}

fn muted(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).wrap());
}

pub(super) fn modals(ctx: &egui::Context, p: &Palette, s: &mut Settings, st: &mut OptionsUi, r: &mut OptionsResult, open: bool) {
    if !open {
        st.share.export = None;
        st.share.import = None;
        return;
    }
    match st.share.demo.take() {
        Some(false) => st.share.open_export(s),
        Some(true) => {
            st.share.open_import();
            if let Some(im) = &mut st.share.import {
                im.text = demo_code(&im.rows.catalog);
                im.dirty = true;
            }
        }
        None => {}
    }
    export_modal(ctx, p, s, st);
    import_modal(ctx, p, s, st, r);
}

/// The code a friend could send: a darker, closer evening look.
fn demo_code(catalog: &Catalog) -> String {
    let mut friend = Settings::default();
    friend.draw_distance = 192.0;
    friend.vsync = !friend.vsync;
    friend.colors.chat_direct = [190, 160, 255, 255];
    friend.camera.fov = 0.9;
    share::export(&friend, catalog, LOOKS, "Cozy evening")
}

fn export_modal(ctx: &egui::Context, p: &Palette, s: &Settings, st: &mut OptionsUi) {
    let Some(ex) = &mut st.share.export else {
        return;
    };
    if ex.dirty {
        let pages: Vec<&str> = ex.pages.iter().copied().collect();
        ex.text = share::export(s, &ex.rows.catalog, &pages, &ex.name);
        ex.dirty = false;
        ex.copied = false;
    }
    let mut close = false;
    let modal = widgets::Modal::new(egui::Id::unique("options_share_export")).show(ctx, |ui| {
        use widgets::{ActionKind, action_button, checkbox, primary_button_sized};
        ui.set_width(560.0);
        title(ui, p, tr!("ui-options-share-title"));
        muted(ui, p, tr!("ui-options-share-intro"));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let size = Vec2::new(0.0, 24.0);
            if action_button(ui, p, "palette-filled", tr!("ui-options-share-looks"), ActionKind::Flat, size).clicked() {
                // the looks the user changed: a page left at its defaults
                // has nothing to say
                ex.pages = LOOKS.iter().copied().filter(|p| ex.counts.contains_key(p)).collect();
                ex.dirty = true;
            }
            if action_button(
                ui,
                p,
                "square-check-filled",
                tr!("ui-options-share-everything"),
                ActionKind::Flat,
                size,
            )
            .clicked()
            {
                ex.pages = ex.counts.keys().copied().collect();
                ex.dirty = true;
            }
            if action_button(ui, p, "x", tr!("ui-options-share-none"), ActionKind::Flat, size).clicked() {
                ex.pages.clear();
                ex.dirty = true;
            }
        });
        ui.add_space(4.0);
        // three columns: every page in sight, without a scroll
        egui::Grid::new("options_share_pages_grid")
            .num_columns(3)
            .spacing(Vec2::new(14.0, 4.0))
            .show(ui, |ui| {
                for (i, page) in SHARED_PAGES.iter().enumerate() {
                    let count = ex.counts.get(page).copied().unwrap_or(0);
                    let mut on = ex.pages.contains(page);
                    let label = tr!("ui-options-share-page", page = page_name(page), count = count);
                    let text = RichText::new(label).color(if count == 0 { p.muted } else { p.ink });
                    if checkbox(ui, p, &mut on, text).changed() {
                        if on {
                            ex.pages.insert(page);
                        } else {
                            ex.pages.remove(page);
                        }
                        ex.dirty = true;
                    }
                    if i % 3 == 2 {
                        ui.end_row();
                    }
                }
            });
        ui.add_space(6.0);
        let name = ui.add(
            egui::TextEdit::singleline(&mut ex.name)
                .hint_text(tr!("ui-options-share-name"))
                .char_limit(share::MAX_NAME)
                .desired_width(f32::INFINITY),
        );
        ex.dirty |= name.changed();
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .id_salt("options_share_text")
            .max_height(150.0)
            .show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut ex.text.as_str())
                        .font(egui::TextStyle::Monospace)
                        .desired_rows(6)
                        .desired_width(f32::INFINITY),
                );
            });
        let shared: usize = ex.pages.iter().map(|p| ex.counts.get(p).copied().unwrap_or(0)).sum();
        ui.add_space(4.0);
        if ex.copied {
            muted(ui, p, &tr!("ui-options-share-copied", count = shared));
        } else {
            muted(ui, p, &tr!("ui-options-share-count", count = shared));
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(!ex.pages.is_empty(), |ui| {
                if primary_button_sized(ui, p, tr!("ui-options-share-copy"), Vec2::new(120.0, 26.0)).clicked() {
                    ui.ctx().copy_text(ex.text.clone());
                    ex.copied = true;
                    log::info!("settings share: code of {shared} settings copied");
                }
            });
            if action_button(ui, p, "x", tr!("common-close"), ActionKind::Flat, Vec2::new(0.0, 26.0)).clicked() {
                close = true;
            }
        });
    });
    if close || modal.should_close() {
        st.share.export = None;
    }
}

fn locks(st: &OptionsUi) -> Locks {
    Locks {
        camera_preset: st.rlva_camera_preset_locked,
        camera_fov: st.rlva_camera_fov_locked,
        sendim: st.rlva_sendim_locked,
        // RLVa's own page is never shared
        restricted: false,
    }
}

fn import_modal(ctx: &egui::Context, p: &Palette, s: &mut Settings, st: &mut OptionsUi, r: &mut OptionsResult) {
    let locks = locks(st);
    let Some(im) = &mut st.share.import else {
        return;
    };
    if im.dirty {
        im.code = Code::parse(&im.text);
        im.plan = match &im.code {
            Ok(code) => Plan::new(code, s, &im.rows.catalog, &locks, im.reset_others),
            Err(_) => Plan::default(),
        };
        im.dirty = false;
    }
    let mut apply = false;
    let mut close = false;
    let modal = widgets::Modal::new(egui::Id::unique("options_share_import")).show(ctx, |ui| {
        use widgets::{ActionKind, action_button, checkbox, primary_button_sized};
        ui.set_width(520.0);
        title(ui, p, tr!("ui-options-share-import-title"));
        muted(ui, p, tr!("ui-options-share-import-intro"));
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .id_salt("options_share_paste")
            .max_height(110.0)
            .show(ui, |ui| {
                let field = ui.add(
                    egui::TextEdit::multiline(&mut im.text)
                        .id(egui::Id::unique("options_share_paste_field"))
                        .font(egui::TextStyle::Monospace)
                        .hint_text(share::HEADER)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY),
                );
                im.dirty |= field.changed();
            });
        ui.add_space(6.0);
        match &im.code {
            Err(_) if im.text.trim().is_empty() => {}
            Err(Refused::NotACode) => muted(ui, p, tr!("ui-options-share-not-a-code")),
            Err(Refused::TooLong) => muted(ui, p, tr!("ui-options-share-too-long")),
            Ok(code) => {
                if !code.name.is_empty() {
                    ui.label(RichText::new(&code.name).size(13.5).strong().color(p.ink));
                }
                let left_out = im.plan.refused + code.skipped;
                if left_out > 0 {
                    muted(ui, p, &tr!("ui-options-share-left-out", count = left_out));
                }
                if checkbox(ui, p, &mut im.reset_others, tr!("ui-options-share-reset-others"))
                    .on_hover_text(tr!("ui-options-share-reset-others-help"))
                    .changed()
                {
                    im.dirty = true;
                }
                ui.add_space(4.0);
                if im.plan.changes.is_empty() {
                    muted(ui, p, tr!("ui-options-share-nothing"));
                } else {
                    plan_list(ui, p, im);
                }
            }
        }
        let count = im.plan.changes.iter().filter(|c| !im.unticked.contains(&c.key)).count();
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.add_enabled_ui(count > 0, |ui| {
                let label = tr!("ui-options-share-apply", count = count);
                if primary_button_sized(ui, p, &label, Vec2::new(150.0, 26.0)).clicked() {
                    apply = true;
                }
            });
            if action_button(ui, p, "x", tr!("common-cancel"), ActionKind::Flat, Vec2::new(0.0, 26.0)).clicked() {
                close = true;
            }
        });
    });
    if apply && let Some(im) = st.share.import.take() {
        let ticked = |c: &share::Change| !im.unticked.contains(&c.key);
        let mut pages: Vec<&'static str> = im.plan.changes.iter().filter(|c| ticked(c)).map(|c| c.page).collect();
        pages.dedup();
        let before = im
            .plan
            .changes
            .iter()
            .filter(|c| ticked(c))
            .map(|c| (c.key.clone(), c.from.clone()))
            .collect();
        let applied = im.plan.apply_if(s, ticked);
        for page in pages.iter().filter_map(|key| page_of(key)) {
            st.after_reset(page, s, r);
        }
        let name = im.code.as_ref().map(|c| c.name.clone()).unwrap_or_default();
        log::info!("settings share: {applied} settings imported on {} pages", pages.len());
        st.share.status = if name.is_empty() {
            tr!("ui-options-share-applied", count = applied)
        } else {
            tr!("ui-options-share-applied-named", count = applied, name = name)
        };
        st.share.undo = Some(Undo { before, pages });
    } else if close || modal.should_close() {
        st.share.import = None;
    }
}

/// The changes, page by page: a tick per page (all its settings), then a
/// tick per setting, old → new.
fn plan_list(ui: &mut egui::Ui, p: &Palette, im: &mut Import) {
    egui::ScrollArea::vertical()
        .id_salt("options_share_plan")
        .max_height(280.0)
        .show(ui, |ui| {
            for page in im.plan.pages() {
                let keys: Vec<String> = im.plan.changes.iter().filter(|c| c.page == page).map(|c| c.key.clone()).collect();
                let off = keys.iter().filter(|k| im.unticked.contains(*k)).count();
                let mut on = off < keys.len();
                let label = tr!("ui-options-share-page", page = page_name(page), count = keys.len());
                let mixed = off > 0 && off < keys.len();
                if widgets::checkbox_mixed(ui, p, &mut on, RichText::new(label).strong(), mixed).changed() {
                    for key in &keys {
                        if on {
                            im.unticked.remove(key);
                        } else {
                            im.unticked.insert(key.clone());
                        }
                    }
                }
                ui.indent(("options_share_plan_page", page), |ui| {
                    for change in im.plan.changes.iter().filter(|c| c.page == page) {
                        let mut on = !im.unticked.contains(&change.key);
                        let color = if on { p.ink } else { p.muted };
                        ui.horizontal_wrapped(|ui| {
                            let label = RichText::new(im.rows.label(&change.key)).size(12.0).color(color);
                            if widgets::checkbox(ui, p, &mut on, label).on_hover_text(&change.key).changed() {
                                if on {
                                    im.unticked.remove(&change.key);
                                } else {
                                    im.unticked.insert(change.key.clone());
                                }
                            }
                            ui.label(RichText::new(shown(&change.from)).size(12.0).color(p.muted));
                            ui.label(RichText::new("→").size(12.0).color(p.muted));
                            if change.key.starts_with("colors.") {
                                swatch(ui, &change.to);
                            }
                            let to = if change.reset {
                                tr!("ui-options-share-to-default", value = shown(&change.to))
                            } else {
                                shown(&change.to)
                            };
                            ui.label(RichText::new(to).size(12.0).strong().color(color));
                        });
                    }
                });
                ui.add_space(4.0);
            }
        });
}

/// "Undo the last import": the settings it changed back to what they were;
/// what the user changed since on other settings stays.
pub(super) fn undo(s: &mut Settings, st: &mut OptionsUi, r: &mut OptionsResult) {
    let Some(undo) = st.share.undo.take() else {
        return;
    };
    // a restriction that started after the import (RLVa's camera limits)
    // holds its settings: the undo leaves them as they are
    let locks = locks(st);
    let values: Vec<(&str, Value)> = undo
        .before
        .iter()
        .filter(|(k, _)| !locks.locked(k))
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    crate::settings::registry::set_many(s, &values);
    for page in undo.pages.iter().filter_map(|key| page_of(key)) {
        st.after_reset(page, s, r);
    }
    st.share.status = tr!("ui-options-share-undone").to_owned();
    log::info!("settings share: last import undone");
}
