//! Drawing the Region objects and Characters windows, in the Legacy layout
//! (floater_pathfinding_linksets.xml, floater_pathfinding_characters.xml):
//! the filters (Region objects), the list, its line and its buttons, the
//! actions on the selection, then the edit panel (Region objects).
use super::edit::{self, EditPanel};
use super::list::{Column, List, use_label};
use super::{Action, CHARACTERS_WINDOW, Confirm, Facts, LINKSETS_WINDOW, PathfindingUi};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind, Floater};
use aurora_net::pathfinding::Kind;
use aurora_net::pathfinding::linkset::{Possible, Use};
use egui::{RichText, Sense, Vec2};

const ROW: f32 = 22.0;
const BUTTON: Vec2 = Vec2::new(0.0, 26.0);
/// The use filter's width.
const FILTER_COMBO: f32 = 170.0;

impl PathfindingUi {
    /// Both windows, then any question; `facts` per window (linksets,
    /// characters).
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, facts: [Facts; 2]) -> Vec<Action> {
        let mut actions = Vec::new();
        if self.linksets_open {
            let mut open = true;
            let mut window = Floater::new(
                LINKSETS_WINDOW,
                aurora_i18n::tr!("pathfinding-linksets-title"),
                egui::pos2(160.0, 110.0),
                Vec2::new(1060.0, 560.0),
            );
            window.min_size = Vec2::new(900.0, 420.0);
            window.show(ctx, p, &mut open, |ui| self.linksets_body(ui, p, facts[0], &mut actions));
            if !open {
                self.linksets_open = false;
                self.closed(Kind::Objects);
                actions.push(Action::Closed(Kind::Objects));
            }
        }
        if self.characters_open {
            let mut open = true;
            let mut window = Floater::new(
                CHARACTERS_WINDOW,
                aurora_i18n::tr!("pathfinding-characters-title"),
                egui::pos2(220.0, 160.0),
                Vec2::new(620.0, 400.0),
            );
            window.min_size = Vec2::new(520.0, 300.0);
            window.show(ctx, p, &mut open, |ui| self.characters_body(ui, p, facts[1], &mut actions));
            if !open {
                self.characters_open = false;
                self.closed(Kind::Characters);
                actions.push(Action::Closed(Kind::Characters));
            }
        }
        if let Some(action) = self.confirm_dialog(ctx, p) {
            actions.push(action);
        }
        actions
    }

    fn linksets_body(&mut self, ui: &mut egui::Ui, p: &Palette, facts: Facts, actions: &mut Vec<Action>) {
        filters(ui, p, &mut self.linksets);
        ui.add_space(4.0);
        self.reload_edit();
        let (mut table_ui, footer) = widgets::above_footer(ui);
        table(&mut table_ui, p, &mut self.linksets, LINKSETS_WINDOW);
        footer.show(ui, |ui| {
            list_controls(ui, p, &mut self.linksets, actions);
            widgets::divider(ui, p);
            let mut beacons = self.linkset_beacons;
            self.actions(ui, p, Kind::Objects, facts, &mut beacons, None, actions);
            self.linkset_beacons = beacons;
            widgets::divider(ui, p);
            self.edit_panel(ui, p, facts);
        });
    }

    fn characters_body(&mut self, ui: &mut egui::Ui, p: &Palette, facts: Facts, actions: &mut Vec<Action>) {
        let (mut table_ui, footer) = widgets::above_footer(ui);
        table(&mut table_ui, p, &mut self.characters, CHARACTERS_WINDOW);
        footer.show(ui, |ui| {
            list_controls(ui, p, &mut self.characters, actions);
            widgets::divider(ui, p);
            let mut beacons = self.character_beacons;
            let mut capsule = self.capsule;
            self.actions(ui, p, Kind::Characters, facts, &mut beacons, Some(&mut capsule), actions);
            self.character_beacons = beacons;
            self.capsule = capsule;
        });
    }

    /// "Actions on selected": Show beacon (and Show physics capsule), then
    /// Take, Take copy, Teleport me to it, Return, Delete
    /// (updateStateOnActionControls).
    #[allow(clippy::too_many_arguments)]
    fn actions(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        kind: Kind,
        facts: Facts,
        beacons: &mut bool,
        capsule: Option<&mut bool>,
        actions: &mut Vec<Action>,
    ) {
        let list = self.list(kind);
        let ids = list.selection.ids.clone();
        let any = !ids.is_empty() && !facts.rlva_blocked;
        let single = list.selection.single().filter(|_| !facts.rlva_no_teleport);
        let listed = single.and_then(|id| list.row(&id)).map(|r| r.position);
        ui.horizontal(|ui| {
            let title = match kind {
                Kind::Objects => aurora_i18n::tr!("pathfinding-actions"),
                Kind::Characters => aurora_i18n::tr!("pathfinding-actions-characters"),
            };
            ui.label(RichText::new(title).strong().color(p.ink));
            ui.add_space(16.0);
            ui.add_enabled_ui(any, |ui| {
                widgets::checkbox(ui, p, beacons, aurora_i18n::tr!("pathfinding-show-beacon"))
            });
            if let Some(capsule) = capsule {
                // isEditEnabled: exactly one character; drawn by Aurora, so
                // no pathing library is needed (a Havok call in the Legacy Viewer)
                let one = list.selection.single().is_some();
                if !one {
                    *capsule = false;
                }
                ui.add_enabled_ui(one, |ui| {
                    widgets::checkbox(ui, p, capsule, aurora_i18n::tr!("pathfinding-show-capsule"))
                });
            }
            if !any {
                *beacons = false;
            }
        });
        // the Legacy row: Take and Take copy on the left, Teleport me to it
        // in the middle, Return and Delete on the right
        let button = |ui: &mut egui::Ui, icon: &str, key: &'static str, enabled: bool| {
            widgets::action_button_if(ui, p, icon, aurora_i18n::text(key), ActionKind::Flat, BUTTON, enabled).clicked()
        };
        let [mut take, mut take_copy, mut teleport, mut return_, mut delete] = [false; 5];
        ui.columns(3, |cols| {
            cols[0].horizontal(|ui| {
                take = button(ui, "hand-grab", "pathfinding-take", any && facts.take);
                take_copy = button(ui, "copy-filled", "pathfinding-take-copy", any && facts.take_copy);
            });
            cols[1].vertical_centered(|ui| {
                teleport = button(ui, "location-filled", "pathfinding-teleport", single.is_some());
            });
            cols[2].with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                delete = button(ui, "trash-filled", "pathfinding-delete", any && facts.delete);
                return_ = button(ui, "arrow-back-up", "pathfinding-return", any && facts.return_);
            });
        });
        if take {
            actions.push(Action::Take(ids.clone()));
        }
        if take_copy {
            actions.push(Action::TakeCopy(ids.clone()));
        }
        if teleport && let (Some(id), Some(listed)) = (single, listed) {
            actions.push(Action::Teleport { id, listed });
        }
        if return_ {
            match ids.len() {
                1 => actions.push(Action::Return(ids.clone())),
                _ => self.confirm = Some(Confirm::Return(ids.clone())),
            }
        }
        if delete {
            match ids.len() {
                1 => actions.push(Action::Delete(ids)),
                _ => self.confirm = Some(Confirm::Delete(ids)),
            }
        }
    }

    /// updateEditFieldValues whenever the selection changes.
    fn reload_edit(&mut self) {
        if self.edit_loaded == self.linksets.selection.ids {
            return;
        }
        self.edit_loaded.clone_from(&self.linksets.selection.ids);
        let first = self.linksets.selected_rows().next().and_then(|r| r.linkset()).copied();
        self.edit.load(first.as_ref());
        self.possible = Possible::of(self.linksets.selected_rows().filter_map(|r| r.linkset()));
    }

    /// "Edit pathfinding attributes" (updateStateOnEditFields,
    /// updateStateOnEditLinksetUse).
    fn edit_panel(&mut self, ui: &mut egui::Ui, p: &Palette, facts: Facts) {
        let list = &self.linksets;
        let enabled = !list.selection.is_empty() && !facts.rlva_blocked;
        let can_apply = enabled && !list.is_pending();
        ui.label(RichText::new(aurora_i18n::tr!("pathfinding-edit-title")).strong().color(p.ink));
        let mut apply = false;
        ui.add_enabled_ui(enabled, |ui| {
            // top-aligned: Apply changes lines up with the fields, the
            // suggested uses hang under them as in the Legacy panel
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                use_choice(ui, &mut self.edit, &self.possible);
                ui.add_space(16.0);
                ui.label(aurora_i18n::tr!("pathfinding-walkability"));
                for (field, (letter, hint, tip)) in [
                    ("A", "pathfinding-suggested-a", "pathfinding-walkability-a-tip"),
                    ("B", "pathfinding-suggested-b", "pathfinding-walkability-b-tip"),
                    ("C", "pathfinding-suggested-c", "pathfinding-walkability-c-tip"),
                    ("D", "pathfinding-suggested-d", "pathfinding-walkability-d-tip"),
                ]
                .into_iter()
                .enumerate()
                {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(letter);
                            let response = ui.add(egui::TextEdit::singleline(&mut self.edit.fields[field]).desired_width(44.0));
                            if response.lost_focus() {
                                self.edit.commit(field);
                            }
                            response.on_hover_text(aurora_i18n::text(tip));
                        });
                        ui.label(RichText::new(aurora_i18n::text(hint)).size(11.5).color(p.muted));
                    });
                }
                ui.add_space(8.0);
                apply = widgets::action_button_if(
                    ui,
                    p,
                    "check",
                    aurora_i18n::tr!("pathfinding-apply-changes"),
                    ActionKind::Flat,
                    BUTTON,
                    can_apply,
                )
                .clicked();
            });
        });
        if apply {
            self.ask_apply();
        }
    }

    /// applyEdit: the question first when a warning applies.
    fn ask_apply(&mut self) {
        let list = &self.linksets;
        let warnings = self.edit.warnings(list.selected_rows());
        let apply = self.edit.apply(list.selected_rows());
        match self.edit.use_ {
            Some(requested) if warnings.any() => {
                self.confirm = Some(Confirm::Apply(edit::question(warnings, requested), apply));
            }
            _ => self.confirm = Some(Confirm::Apply(String::new(), apply)),
        }
    }

    /// PathfindingDirty, opened by the rebake icon: Rebake or Close. Some(true)
    /// when Rebake was chosen (rebakeRegionCallback: it then rebakes if a
    /// rebake is needed).
    pub fn dirty_dialog(&mut self, ctx: &egui::Context, p: &Palette) -> Option<bool> {
        if !self.dirty_question {
            return None;
        }
        let mut answer = None;
        let modal = widgets::Modal::new(egui::Id::unique("pathfinding_dirty")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("pathfinding-menu-rebake"))
                    .size(15.0)
                    .strong()
                    .color(p.ink),
            );
            ui.add_space(6.0);
            ui.label(RichText::new(aurora_i18n::tr!("pathfinding-dirty-text")).size(12.5).color(p.muted));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if widgets::action_button(
                    ui,
                    p,
                    "refresh",
                    aurora_i18n::tr!("pathfinding-rebake"),
                    ActionKind::Primary,
                    BUTTON,
                )
                .clicked()
                {
                    answer = Some(true);
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-close"), ActionKind::Flat, BUTTON).clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        if answer.is_some() {
            self.dirty_question = false;
        }
        answer
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context, p: &Palette) -> Option<Action> {
        let confirm = self.confirm.as_ref()?;
        // no warning: sent at once (doApplyEdit)
        if let Confirm::Apply(text, apply) = confirm
            && text.is_empty()
        {
            let action = Action::Apply(apply.clone());
            self.confirm = None;
            return Some(action);
        }
        let (title, text, yes_icon, yes, kind) = match confirm {
            Confirm::Apply(text, _) => (
                aurora_i18n::tr!("pathfinding-edit-title"),
                text.clone(),
                "check",
                aurora_i18n::tr!("common-ok"),
                ActionKind::Primary,
            ),
            Confirm::Return(ids) => (
                aurora_i18n::tr!("pathfinding-return"),
                aurora_i18n::tr!("pathfinding-return-many", count = ids.len() as i64),
                "arrow-back-up",
                aurora_i18n::tr!("pathfinding-return"),
                ActionKind::Primary,
            ),
            Confirm::Delete(ids) => (
                aurora_i18n::tr!("pathfinding-delete"),
                aurora_i18n::tr!("pathfinding-delete-many", count = ids.len() as i64),
                "trash-filled",
                aurora_i18n::tr!("pathfinding-delete"),
                ActionKind::Danger,
            ),
        };
        let mut answer = None;
        let modal = widgets::Modal::new(egui::Id::unique("pathfinding_confirm")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(title).size(15.0).strong().color(p.ink));
            ui.add_space(6.0);
            ui.label(RichText::new(text).size(12.5).color(p.muted));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if widgets::action_button(ui, p, yes_icon, yes, kind, BUTTON).clicked() {
                    answer = Some(true);
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, BUTTON).clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        let yes = answer?;
        let confirm = self.confirm.take()?;
        yes.then_some(match confirm {
            Confirm::Apply(_, apply) => Action::Apply(apply),
            Confirm::Return(ids) => Action::Return(ids),
            Confirm::Delete(ids) => Action::Delete(ids),
        })
    }
}

/// The filter row: applied with Apply or Enter, emptied with Clear.
fn filters(ui: &mut egui::Ui, p: &Palette, list: &mut List) {
    let mut apply = false;
    ui.horizontal(|ui| {
        // the two text fields share what the labels, the dropdown and the
        // buttons leave (100 to 180 px each): Apply and Clear never cover
        // the dropdown, whatever the language
        let (filter_by, name_label, description_label) = (
            aurora_i18n::tr!("pathfinding-filter-by"),
            aurora_i18n::tr!("pathfinding-filter-name"),
            aurora_i18n::tr!("pathfinding-filter-description"),
        );
        let text_width = |text: &str| {
            ui.painter()
                .layout_no_wrap(text.to_owned(), egui::TextStyle::Body.resolve(ui.style()), p.ink)
                .size()
                .x
        };
        let fixed = text_width(filter_by)
            + text_width(name_label)
            + text_width(description_label)
            + FILTER_COMBO
            + widgets::action_button_fit(ui, "filter-filled", aurora_i18n::tr!("pathfinding-filter-apply"))
            + widgets::action_button_fit(ui, "x", aurora_i18n::tr!("pathfinding-filter-clear"))
            + 3.0 * 6.0
            + 9.0 * ui.spacing().item_spacing.x;
        let field = ((ui.available_width() - fixed) / 2.0).clamp(100.0, 180.0);
        ui.label(RichText::new(filter_by).color(p.ink));
        ui.add_space(6.0);
        ui.label(name_label);
        let name = ui.add(
            egui::TextEdit::singleline(&mut list.filter.name)
                .hint_text(aurora_i18n::tr!("pathfinding-filter-name-hint"))
                .desired_width(field),
        );
        ui.add_space(6.0);
        ui.label(description_label);
        let description = ui.add(
            egui::TextEdit::singleline(&mut list.filter.description)
                .hint_text(aurora_i18n::tr!("pathfinding-filter-description-hint"))
                .desired_width(field),
        );
        let entered = |r: &egui::Response| r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        apply |= entered(&name) || entered(&description);
        ui.add_space(6.0);
        let shown = list
            .filter
            .use_
            .map_or_else(|| aurora_i18n::text("pathfinding-filter-use"), use_label);
        egui::ComboBox::from_id_salt("pathfinding_filter_use")
            .selected_text(shown)
            .width(FILTER_COMBO)
            .show_ui(ui, |ui| {
                apply |= ui
                    .selectable_value(&mut list.filter.use_, None, aurora_i18n::tr!("pathfinding-filter-use"))
                    .clicked();
                for use_ in Use::ALL {
                    apply |= ui.selectable_value(&mut list.filter.use_, Some(use_), use_label(use_)).clicked();
                }
            });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::action_button(ui, p, "x", aurora_i18n::tr!("pathfinding-filter-clear"), ActionKind::Flat, BUTTON).clicked() {
                list.clear_filters();
            }
            apply |= widgets::action_button(
                ui,
                p,
                "filter-filled",
                aurora_i18n::tr!("pathfinding-filter-apply"),
                ActionKind::Flat,
                BUTTON,
            )
            .clicked();
        });
    });
    if apply {
        list.apply_filters();
    }
}

/// The use dropdown; entries the selection cannot take are greyed.
fn use_choice(ui: &mut egui::Ui, edit: &mut EditPanel, possible: &Possible) {
    let shown = edit.use_.map_or_else(|| aurora_i18n::text("pathfinding-choose-use"), use_label);
    egui::ComboBox::from_id_salt("pathfinding_edit_use")
        .selected_text(shown)
        .width(180.0)
        .show_ui(ui, |ui| {
            for use_ in Use::ALL {
                ui.add_enabled_ui(possible.allows(use_), |ui| {
                    ui.selectable_value(&mut edit.use_, Some(use_), use_label(use_));
                });
            }
        });
}

/// The line under the list, with Refresh list, Select all, Select none
/// (updateStateOnListControls: none of them while a request runs).
fn list_controls(ui: &mut egui::Ui, p: &Palette, list: &mut List, actions: &mut Vec<Action>) {
    let idle = !list.is_pending();
    let has_rows = list.visible_len() > 0;
    ui.horizontal(|ui| {
        let status = list.status();
        let color = match list.messaging {
            super::Messaging::GetError | super::Messaging::SetError => p.danger,
            _ => p.ink,
        };
        ui.label(RichText::new(status).color(color));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut button = |icon: &str, key: &'static str, enabled: bool| {
                widgets::action_button_if(ui, p, icon, aurora_i18n::text(key), ActionKind::Flat, BUTTON, enabled).clicked()
            };
            if button("deselect", "pathfinding-select-none", idle && has_rows) {
                list.selection.clear();
            }
            if button("select-all", "pathfinding-select-all", idle && has_rows) {
                let all = list.visible_ids();
                list.selection.select_all(all);
            }
            if button("refresh", "pathfinding-refresh", idle) {
                actions.push(Action::Refresh(list.kind()));
            }
        });
    });
}

/// The list: a sortable header, then only the visible rows drawn.
fn table(ui: &mut egui::Ui, p: &Palette, list: &mut List, id: &str) {
    let kind = list.kind();
    let columns = Column::of(kind);
    let minimum: f32 = columns.iter().map(|c| c.min_width()).sum();
    let weights: f32 = columns.iter().map(|c| c.weight()).sum();
    let table_width = ui.available_width().max(minimum + 16.0);
    egui::ScrollArea::horizontal()
        .id_salt((id, "horizontal"))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.set_width(table_width);
            let rows_id = ui.scope_id().with((id, "rows"));
            let width = widgets::header_width(ui, rows_id);
            let extra = (width - minimum).max(0.0);
            let widths: Vec<f32> = columns.iter().map(|c| c.min_width() + extra * c.weight() / weights).collect();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                for (&column, &w) in columns.iter().zip(&widths) {
                    let label = column.label(kind);
                    let text = match list.sort {
                        Some(sort) if sort == column => format!("{label} {}", if list.descending { "↓" } else { "↑" }),
                        _ => label.to_owned(),
                    };
                    if ui
                        .add_sized([w - 2.0, 24.0], egui::Button::new(text).truncate())
                        .on_hover_text(label)
                        .clicked()
                    {
                        list.sort_by(column);
                    }
                    ui.add_space(2.0);
                }
            });
            let mut clicked = None;
            let scroll = egui::ScrollArea::vertical()
                .id_salt((id, "rows"))
                .max_height(ui.available_height())
                .auto_shrink([false, false])
                .show_rows(ui, ROW, list.visible_len(), |ui, range| {
                    for n in range {
                        let Some(row) = list.visible_row(n) else { continue };
                        let row_id = row.id;
                        let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
                        let selected = list.selection.contains(&row_id);
                        if selected || response.hovered() {
                            let color = if selected { p.violet.gamma_multiply(0.28) } else { p.raised };
                            ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, color);
                        }
                        let mut x = rect.left();
                        for (&column, &w) in columns.iter().zip(&widths) {
                            let mut job = egui::text::LayoutJob::simple_singleline(
                                list.cell(n, column).to_owned(),
                                egui::FontId::proportional(12.5),
                                p.ink,
                            );
                            job.wrap = egui::text::TextWrapping::truncate_at_width(w - 8.0);
                            let galley = ui.painter().layout_job(job);
                            let at = widgets::row_text_origin(ui.painter(), &galley, x + 4.0, rect.center().y);
                            ui.painter().galley(at, galley, p.ink);
                            x += w;
                        }
                        if response.clicked() {
                            clicked = Some((row_id, ui.input(|i| i.modifiers)));
                        }
                    }
                });
            widgets::remember_scroll_bar(ui, rows_id, &scroll);
            if let Some((row_id, m)) = clicked {
                let order = list.visible_ids();
                list.selection.click(row_id, m.command, m.shift, move || order.clone());
            }
        });
}
