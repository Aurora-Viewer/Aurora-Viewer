//! Preferences › Chat › Spelling: the Legacy Viewer's two floaters in one
//! tab. Spell checking (floater_spellcheck.xml,
//! LLFloaterSpellCheckerSettings): on / off, the main dictionary, the
//! extra ones, Import and Remove. Auto-replace (floater_autoreplace.xml,
//! LLFloaterAutoReplaceSettings): on / off, the lists in their order
//! (up / down, new, delete, import, export) and the keywords of the list
//! chosen (add, change, remove).
//!
//! The lists are not settings: they are saved in their own file as soon
//! as they change (the Legacy "Save Changes" button is not needed), and
//! the page's reset leaves them alone; "Default lists" brings back the
//! bundled ones.

use super::super::cx::{Cx, Nothing, field};
use crate::autoreplace::List;
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use egui::{RichText, Vec2};
use std::sync::mpsc;

/// The page's state between frames.
#[derive(Default)]
pub struct SpellingPage {
    /// The list whose keywords are shown.
    list: usize,
    filter: String,
    new_list: String,
    keyword: String,
    replacement: String,
    /// The keyword clicked in the table, replaced by the one saved
    /// (onSaveEntry removes mPreviousKeyword).
    editing: Option<String>,
    /// The last import / export: (it failed, what to say).
    message: Option<(bool, String)>,
    dialog: Option<mpsc::Receiver<Dialog>>,
}

enum Dialog {
    Imported(Result<List, String>),
    Exported(Result<String, String>),
}

/// The rows heights of the keyword table.
const ROW_H: f32 = 22.0;
const TABLE_H: f32 = 220.0;

pub fn page(cx: &mut Cx) {
    use aurora_i18n::tr;
    let p = cx.p;
    let catalogue = crate::spelling::catalogue();
    cx.group(tr!("ui-options-spelling"), |cx| {
        cx.toggle(
            tr!("ui-options-spelling-on"),
            tr!("ui-options-spelling-on-hint"),
            &field!(spelling.enabled),
        );
        cx.greyed(!cx.s.spelling.enabled);
        let names: Vec<&str> = catalogue.iter().map(|d| d.language.as_str()).collect();
        cx.keywords(&names.join(" ")).row(
            tr!("ui-options-spelling-main"),
            tr!("ui-options-spelling-main-hint"),
            &field!(spelling.main),
            |ui, s| main_dictionary(ui, &p, &mut s.spelling, &catalogue),
        );
        cx.wide().below().row(
            tr!("ui-options-spelling-extra"),
            tr!("ui-options-spelling-extra-hint"),
            &field!(spelling.extra),
            |ui, s| extra_dictionaries(ui, &p, &mut s.spelling, &catalogue),
        );
        cx.wide().below().row_with(
            tr!("ui-options-spelling-imported"),
            tr!("ui-options-spelling-imported-hint"),
            &Nothing,
            |ui, s, st, _| {
                imported(ui, &p, &s.spelling, &catalogue, &mut st.spelling_page);
                false
            },
        );
    });
    cx.group(tr!("ui-options-auto-replace"), |cx| {
        cx.toggle(
            tr!("ui-options-auto-replace-on"),
            tr!("ui-options-auto-replace-on-hint"),
            &field!(auto_replace),
        );
        // the lists stay editable while it is off: they can be made ready
        cx.wide().below().row_with(
            tr!("ui-options-auto-replace-lists"),
            tr!("ui-options-auto-replace-lists-hint"),
            &Nothing,
            |ui, _, st, _| {
                lists(ui, &p, &mut st.spelling_page);
                false
            },
        );
        cx.wide().below().row_with(
            tr!("ui-options-auto-replace-keywords"),
            tr!("ui-options-auto-replace-keywords-hint"),
            &Nothing,
            |ui, _, st, _| {
                keywords(ui, &p, &mut st.spelling_page);
                false
            },
        );
    });
}

/// The main dictionary (those with an affix file), and where it stands.
fn main_dictionary(ui: &mut egui::Ui, p: &Palette, s: &mut crate::spelling::SpellSettings, catalogue: &[crate::spelling::Info]) -> bool {
    use aurora_i18n::tr;
    let shown = catalogue
        .iter()
        .find(|d| d.name == s.main)
        .map_or(s.main.as_str(), |d| d.language.as_str())
        .to_owned();
    let mut changed = false;
    egui::ComboBox::from_id_salt(ui.scope_id().with("main_dictionary"))
        .width(super::super::COMBO_W)
        .selected_text(shown)
        .truncate()
        .show_ui(ui, |ui| {
            for d in catalogue.iter().filter(|d| d.primary) {
                if ui.selectable_label(d.name == s.main, &d.language).clicked() && d.name != s.main {
                    // LLFloaterSpellCheckerSettings: the new main one is no
                    // longer an extra one
                    s.extra.retain(|extra| *extra != d.name);
                    s.main = d.name.clone();
                    changed = true;
                }
            }
        });
    if s.enabled {
        match crate::spelling::status() {
            crate::spelling::Status::Loading => {
                ui.label(RichText::new(tr!("ui-options-spelling-loading")).size(12.0).color(p.muted));
            }
            crate::spelling::Status::Failed(why) => {
                ui.label(RichText::new(tr!("ui-options-spelling-failed")).size(12.0).color(p.danger))
                    .on_hover_text(why);
            }
            _ => {}
        }
    }
    changed
}

/// Every other dictionary, checked when it adds its words.
fn extra_dictionaries(ui: &mut egui::Ui, p: &Palette, s: &mut crate::spelling::SpellSettings, catalogue: &[crate::spelling::Info]) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| {
        for d in catalogue.iter().filter(|d| d.name != s.main) {
            let mut on = s.extra.contains(&d.name);
            if widgets::checkbox(ui, p, &mut on, &d.language).changed() {
                if on {
                    s.extra.push(d.name.clone());
                } else {
                    s.extra.retain(|extra| *extra != d.name);
                }
                changed = true;
            }
        }
    });
    ui.add_space(6.0);
    changed
}

/// The imported dictionaries (each removable while not in use) and Import.
fn imported(
    ui: &mut egui::Ui,
    p: &Palette,
    s: &crate::spelling::SpellSettings,
    catalogue: &[crate::spelling::Info],
    st: &mut SpellingPage,
) {
    use aurora_i18n::tr;
    if let Some(result) = crate::spelling::take_import_result() {
        st.message = Some(match result {
            Ok(language) => (false, aurora_i18n::tr!("ui-options-spelling-import-done", name = language)),
            Err(why) => (true, why),
        });
    }
    ui.vertical(|ui| {
        let mine: Vec<_> = catalogue.iter().filter(|d| d.imported).collect();
        if mine.is_empty() {
            ui.label(RichText::new(tr!("ui-options-spelling-imported-none")).size(12.0).color(p.muted));
        }
        for d in mine {
            ui.horizontal(|ui| {
                // LLSpellChecker::canRemoveDictionary: not while in use
                let in_use = s.enabled && (d.name == s.main || s.extra.contains(&d.name));
                let tip = if in_use {
                    tr!("ui-options-spelling-remove-in-use")
                } else {
                    tr!("ui-options-spelling-remove")
                };
                if widgets::icon_button(ui, p, "trash-filled", tip, !in_use) {
                    crate::spelling::remove(&d.name);
                }
                ui.label(RichText::new(&d.language).size(12.5).color(p.ink));
            });
        }
        ui.add_space(4.0);
        if widgets::action_button(
            ui,
            p,
            "file-import",
            tr!("ui-options-spelling-import"),
            ActionKind::Flat,
            Vec2::new(0.0, 26.0),
        )
        .clicked()
        {
            st.message = None;
            crate::spelling::import();
        }
        message(ui, p, st);
    });
}

fn message(ui: &mut egui::Ui, p: &Palette, st: &SpellingPage) {
    if let Some((failed, text)) = &st.message {
        ui.label(RichText::new(text).size(12.0).color(if *failed { p.danger } else { p.muted }));
    }
}

/// The answer of an import or export dialog, once closed.
fn poll_dialog(st: &mut SpellingPage) {
    let Some(rx) = &st.dialog else {
        return;
    };
    match rx.try_recv() {
        Ok(Dialog::Imported(Ok(mut list))) => {
            st.dialog = None;
            let mut lists = crate::autoreplace::lists().as_ref().clone();
            list.name = crate::autoreplace::unique_name(&lists, &list.name);
            st.message = Some((
                false,
                aurora_i18n::tr!("ui-options-auto-replace-imported", name = list.name.clone()),
            ));
            lists.push(list);
            st.list = lists.len() - 1;
            crate::autoreplace::set_lists(lists);
        }
        Ok(Dialog::Imported(Err(why)) | Dialog::Exported(Err(why))) => {
            st.dialog = None;
            st.message = Some((true, why));
        }
        Ok(Dialog::Exported(Ok(path))) => {
            st.dialog = None;
            st.message = Some((false, aurora_i18n::tr!("ui-options-auto-replace-exported", path = path)));
        }
        Err(mpsc::TryRecvError::Empty) => {}
        Err(mpsc::TryRecvError::Disconnected) => st.dialog = None,
    }
}

/// A file dialog on its own thread, its answer read by [`poll_dialog`].
fn dialog(st: &mut SpellingPage, run: impl FnOnce() -> Option<Dialog> + Send + 'static) {
    if st.dialog.is_some() {
        return;
    }
    let (tx, rx) = mpsc::channel();
    st.dialog = Some(rx);
    st.message = None;
    let spawned = std::thread::Builder::new().name("aurora-autoreplace-dialog".into()).spawn(move || {
        if let Some(answer) = run() {
            let _ = tx.send(answer);
        }
    });
    if let Err(e) = spawned {
        log::warn!("autoreplace: no dialog thread: {e}");
        st.dialog = None;
    }
}

/// The lists in their order: the one chosen shows its keywords below.
fn lists(ui: &mut egui::Ui, p: &Palette, st: &mut SpellingPage) {
    use aurora_i18n::tr;
    poll_dialog(st);
    let current = crate::autoreplace::lists();
    // Copied only when something changes: the bundled lists alone hold
    // some 4,000 entries, too many to copy every frame (#1363).
    let mut edited: Option<Vec<List>> = None;
    st.list = st.list.min(current.len().saturating_sub(1));
    ui.vertical(|ui| {
        if current.is_empty() {
            ui.label(RichText::new(tr!("ui-options-auto-replace-no-list")).size(12.0).color(p.muted));
        }
        let count = current.len();
        let mut action = None;
        for (i, list) in current.iter().enumerate() {
            ui.horizontal(|ui| {
                if widgets::icon_button(ui, p, "arrow-up", tr!("ui-options-auto-replace-up"), i > 0) {
                    action = Some((i, Move::Up));
                }
                if widgets::icon_button(ui, p, "arrow-down", tr!("ui-options-auto-replace-down"), i + 1 < count) {
                    action = Some((i, Move::Down));
                }
                if widgets::icon_button(ui, p, "download-filled", tr!("ui-options-auto-replace-export"), st.dialog.is_none()) {
                    action = Some((i, Move::Export));
                }
                if widgets::icon_button(ui, p, "trash-filled", tr!("ui-options-auto-replace-delete-list"), true) {
                    action = Some((i, Move::Delete));
                }
                let label = format!("{}  ({})", list.name, list.replacements.len());
                if widgets::radio(ui, p, st.list == i, label).clicked() {
                    st.list = i;
                }
            });
        }
        match action {
            Some((i, Move::Up)) => {
                let lists = edited.get_or_insert_with(|| current.as_ref().clone());
                lists.swap(i, i - 1);
                st.list = if st.list == i {
                    i - 1
                } else if st.list == i - 1 {
                    i
                } else {
                    st.list
                };
            }
            Some((i, Move::Down)) => {
                let lists = edited.get_or_insert_with(|| current.as_ref().clone());
                lists.swap(i, i + 1);
                st.list = if st.list == i {
                    i + 1
                } else if st.list == i + 1 {
                    i
                } else {
                    st.list
                };
            }
            Some((i, Move::Delete)) => {
                edited.get_or_insert_with(|| current.as_ref().clone()).remove(i);
                if st.list > i {
                    st.list -= 1;
                }
            }
            Some((i, Move::Export)) => {
                let list = current[i].clone();
                let title = tr!("ui-options-auto-replace-export");
                dialog(st, move || {
                    let path = rfd::FileDialog::new()
                        .set_title(title)
                        .set_file_name(crate::autoreplace::export_name(&list.name))
                        .add_filter("XML", &["xml"])
                        .save_file()?;
                    let written = crate::atomic_file::write(&path, crate::autoreplace::exported_xml(&list).as_bytes())
                        .map(|()| path.display().to_string())
                        .map_err(|e| format!("{}: {e}", path.display()));
                    Some(Dialog::Exported(written))
                });
            }
            None => {}
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            let typed = ui.add(
                egui::TextEdit::singleline(&mut st.new_list)
                    .desired_width(200.0)
                    .hint_text(tr!("ui-options-auto-replace-new-list-hint")),
            );
            let entered = typed.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let name = st.new_list.trim().to_owned();
            let add = widgets::action_button_if(
                ui,
                p,
                "circle-plus-filled",
                tr!("ui-options-auto-replace-new-list"),
                ActionKind::Flat,
                Vec2::new(0.0, 26.0),
                !name.is_empty(),
            )
            .clicked();
            if (add || entered) && !name.is_empty() {
                // LLAutoReplaceSettings::addList: an empty list at the end
                let lists = edited.get_or_insert_with(|| current.as_ref().clone());
                let name = crate::autoreplace::unique_name(lists, &name);
                lists.push(List {
                    name,
                    replacements: Default::default(),
                });
                st.list = lists.len() - 1;
                st.new_list.clear();
            }
            if widgets::action_button_if(
                ui,
                p,
                "file-import",
                tr!("ui-options-auto-replace-import"),
                ActionKind::Flat,
                Vec2::new(0.0, 26.0),
                st.dialog.is_none(),
            )
            .clicked()
            {
                let title = tr!("ui-options-auto-replace-import");
                dialog(st, move || {
                    let path = rfd::FileDialog::new().set_title(title).add_filter("XML", &["xml"]).pick_file()?;
                    let list = std::fs::read(&path)
                        .map_err(|e| format!("{}: {e}", path.display()))
                        .and_then(|bytes| crate::autoreplace::parse_exported(&bytes).map_err(|e| format!("{}: {e}", path.display())));
                    Some(Dialog::Imported(list))
                });
            }
            if !crate::autoreplace::is_default(&current)
                && widgets::action_button(
                    ui,
                    p,
                    "arrow-back-up",
                    tr!("ui-options-auto-replace-defaults"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 26.0),
                )
                .clicked()
            {
                edited = Some(crate::autoreplace::defaults().as_ref().clone());
                st.list = 0;
            }
        });
        message(ui, p, st);
    });
    if let Some(lists) = edited {
        crate::autoreplace::set_lists(lists);
    }
}

#[derive(Clone, Copy)]
enum Move {
    Up,
    Down,
    Export,
    Delete,
}

/// The keywords of the chosen list: a filter, the table (a click on a row
/// puts it in the fields below to change it, as the Legacy list did) and
/// the fields to add or change one.
fn keywords(ui: &mut egui::Ui, p: &Palette, st: &mut SpellingPage) {
    use aurora_i18n::tr;
    let current = crate::autoreplace::lists();
    let Some(list) = current.get(st.list) else {
        ui.label(RichText::new(tr!("ui-options-auto-replace-no-list")).size(12.0).color(p.muted));
        return;
    };
    let mut edited: Option<List> = None;
    ui.vertical(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut st.filter)
                .desired_width(260.0)
                .hint_text(tr!("ui-options-auto-replace-filter-hint")),
        );
        let filter = st.filter.trim().to_lowercase();
        let shown: Vec<(&String, &String)> = list
            .replacements
            .iter()
            .filter(|(k, v)| filter.is_empty() || k.to_lowercase().contains(&filter) || v.to_lowercase().contains(&filter))
            .collect();
        if shown.is_empty() {
            ui.label(RichText::new(tr!("ui-options-auto-replace-no-keyword")).size(12.0).color(p.muted));
        } else {
            let width = ui.available_width().min(520.0);
            egui::ScrollArea::vertical()
                .id_salt(("autoreplace_keywords", st.list))
                .max_height(TABLE_H)
                .auto_shrink([false, true])
                .show_rows(ui, ROW_H, shown.len(), |ui, rows| {
                    ui.set_width(width);
                    for (keyword, replacement) in &shown[rows] {
                        ui.horizontal(|ui| {
                            ui.set_height(ROW_H);
                            if widgets::icon_button(ui, p, "trash-filled", tr!("ui-options-auto-replace-remove"), true) {
                                let mut changed = list.clone();
                                changed.replacements.remove(*keyword);
                                edited = Some(changed);
                            }
                            let row = ui.add(
                                egui::Label::new(RichText::new(format!("{keyword}  →  {replacement}")).size(12.5).color(p.ink))
                                    .truncate()
                                    .sense(egui::Sense::click()),
                            );
                            if row.clicked() {
                                st.keyword = (*keyword).clone();
                                st.replacement = (*replacement).clone();
                                st.editing = Some((*keyword).clone());
                            }
                        });
                    }
                });
        }
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut st.keyword)
                    .desired_width(140.0)
                    .hint_text(tr!("ui-options-auto-replace-keyword-hint")),
            );
            ui.add(
                egui::TextEdit::singleline(&mut st.replacement)
                    .desired_width(240.0)
                    .hint_text(tr!("ui-options-auto-replace-replacement-hint")),
            );
            // LLAutoReplaceSettings::addEntryToList: one word of keyword
            // characters, a replacement
            let keyword = st.keyword.trim().to_owned();
            let valid = crate::autoreplace::valid_keyword(&keyword) && !st.replacement.trim().is_empty();
            let known = list.replacements.contains_key(&keyword);
            let label = if known {
                tr!("ui-options-auto-replace-save-keyword")
            } else {
                tr!("ui-options-auto-replace-add-keyword")
            };
            if widgets::action_button_if(ui, p, "circle-plus-filled", label, ActionKind::Flat, Vec2::new(0.0, 26.0), valid).clicked() {
                let mut changed = list.clone();
                if let Some(previous) = st.editing.take() {
                    changed.replacements.remove(&previous);
                }
                changed.replacements.insert(keyword, st.replacement.trim().to_owned());
                edited = Some(changed);
                st.keyword.clear();
                st.replacement.clear();
            }
        });
    });
    if let Some(list) = edited {
        let mut lists = current.as_ref().clone();
        if let Some(slot) = lists.get_mut(st.list) {
            *slot = list;
        }
        crate::autoreplace::set_lists(lists);
    }
}
