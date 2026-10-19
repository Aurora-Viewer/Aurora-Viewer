//! Approved AO workshop layout using the unchanged Aurora widgets and palette.
use super::*;
use crate::ao::{Animation, StateConfig, inventory::editor::List};

#[derive(Clone, Copy, Debug)]
struct EntryDrag {
    set: Uuid,
    state: State,
    list: List,
    entry: Uuid,
}

pub(super) fn show(
    ui: &mut egui::Ui,
    p: &Palette,
    inv: &Inventory,
    set: &Set,
    runtime: &mut Runtime,
    state: &mut AoUi,
    out: &mut Vec<Action>,
) {
    let category = super::category::Category::of(state.state);
    state.category_states[category.index()] = Some(state.state);
    let mut selected_category = category.index();
    if widgets::tabs(
        ui,
        p,
        &mut selected_category,
        &super::category::Category::ALL.map(|c| (c.label(), true)),
    ) {
        let category = super::category::Category::ALL[selected_category];
        let target = state.category_states[selected_category].unwrap_or_else(|| category.initial_state());
        select_state(state, target);
    }
    let category = super::category::Category::of(state.state);
    let height = ui.available_height().clamp(230.0, 386.0);
    let reveal = state.rail_state != Some(state.state);
    state.rail_state = Some(state.state);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(164.0, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.label(RichText::new(aurora_i18n::tr!("ao-states")).strong());
            egui::ScrollArea::vertical()
                .id_salt("ao_state_rail")
                .max_height(height - 24.0)
                .show(ui, |ui| {
                    for item in category.states() {
                        let response = ui
                            .horizontal(|ui| {
                                let marker_width = if item.swimming() { 16.0 + ui.spacing().item_spacing.x } else { 0.0 };
                                let response = ui.add_sized(
                                    [ui.available_width() - marker_width, 28.0],
                                    egui::Button::selectable(state.state == item, item.label()).wrap(),
                                );
                                if item.swimming() {
                                    super::swimming_marker(ui, p, set, item);
                                }
                                response
                            })
                            .inner;
                        if reveal && state.state == item {
                            response.scroll_to_me(Some(egui::Align::Center));
                        }
                        let pending_focus = state.state_focus == Some(item);
                        if pending_focus {
                            response.request_focus();
                            response.scroll_to_me(Some(egui::Align::Center));
                            if response.has_focus() && ui.is_rect_visible(response.rect) {
                                state.state_focus = None;
                            }
                        }
                        if response.clicked() && state.state != item {
                            select_state(state, item);
                        }
                        // A wrapped target can still be clipped while the
                        // scroll animates. Keep its focus request and accept
                        // the next key rather than losing keyboard navigation.
                        if response.has_focus() || pending_focus {
                            ui.memory_mut(|m| {
                                m.set_focus_lock_filter(
                                    response.id,
                                    egui::EventFilter {
                                        vertical_arrows: true,
                                        ..Default::default()
                                    },
                                );
                            });
                            let next = ui.input_mut(|i| {
                                if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
                                    Some(1)
                                } else if i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
                                    Some(-1)
                                } else {
                                    None
                                }
                            });
                            if let Some(delta) = next {
                                let index = category.states().position(|s| s == item).unwrap_or(0);
                                let index = (index as isize + delta).rem_euclid(category.states().count() as isize) as usize;
                                if let Some(next) = category.states().nth(index) {
                                    select_state(state, next);
                                    state.state_focus = Some(next);
                                }
                            }
                        }
                    }
                });
        });
        let width = ui.available_width();
        ui.allocate_ui_with_layout(Vec2::new(width, height), egui::Layout::top_down(egui::Align::Min), |ui| {
            ui.label(RichText::new(state.state.label()).strong().color(p.violet_pale));
            egui::ScrollArea::vertical()
                .id_salt(("ao_state_details", set.id, state.state, state.container))
                .max_height(height - 24.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if state.state.swimming() {
                        super::swimming_status(ui, p, set, state.state);
                    }
                    let empty = StateConfig::default();
                    let config = set.states.get(&state.state).unwrap_or(&empty);
                    let mut editor = Editor {
                        p,
                        inv,
                        set,
                        runtime,
                        state,
                        out,
                    };
                    editor.details(ui, config);
                });
        });
    });
}

fn select_state(state: &mut AoUi, item: State) {
    state.state = item;
    state.state_focus = None;
    state.container = None;
    state.cycle_edit = None;
    state.picker = None;
    state.combination = None;
}

struct Editor<'a> {
    p: &'a Palette,
    inv: &'a Inventory,
    set: &'a Set,
    runtime: &'a mut Runtime,
    state: &'a mut AoUi,
    out: &'a mut Vec<Action>,
}

impl Editor<'_> {
    fn picker(&mut self) {
        self.state.picker = Some(false);
        self.state.folder = crate::world::inventory::actions::system(self.inv, 20).unwrap_or(self.inv.root);
    }

    fn details(&mut self, ui: &mut egui::Ui, c: &StateConfig) {
        let group = self
            .state
            .container
            .and_then(|id| c.steps.iter().enumerate().find(|(_, s)| s.group && s.id == id));
        let track = self.state.container.and_then(|id| c.tracks.iter().find(|t| t.id == id));
        if let Some((_, group)) = group {
            self.container_header(ui, group.id, &group.name, false);
            ui.label(RichText::new(aurora_i18n::tr!("ao-group-explanation")).color(self.p.muted));
            if ui
                .add_enabled(!self.state.busy(), egui::Button::new(aurora_i18n::tr!("ao-add-group")))
                .clicked()
            {
                self.picker();
            }
            let selected = c
                .steps
                .get(self.runtime.index(self.state.state, c.folder))
                .is_some_and(|s| s.id == group.id);
            self.members(ui, c, group.id, &group.members, selected, false);
            ui.label(RichText::new(aurora_i18n::tr!("ao-group-count", count = group.members.len())).color(self.p.violet_pale));
            ui.label(RichText::new(aurora_i18n::tr!("ao-group-cycle-help", state = self.state.state.label())).color(self.p.muted));
            self.dissolve(ui, group.id, false);
        } else if let Some(track) = track {
            self.container_header(ui, track.id, &track.name, true);
            ui.label(RichText::new(aurora_i18n::tr!("ao-track-explanation", state = self.state.state.label())).color(self.p.muted));
            if ui
                .add_enabled(!self.state.busy(), egui::Button::new(aurora_i18n::tr!("ao-add-track")))
                .clicked()
            {
                self.picker();
            }
            self.members(ui, c, track.id, &track.members, false, true);
            self.cycle(ui, c, Some(track));
            self.dissolve(ui, track.id, true);
        } else {
            self.state.container = None;
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(!self.state.busy(), egui::Button::new(aurora_i18n::tr!("ao-add")))
                    .clicked()
                {
                    self.picker();
                }
                if ui
                    .add_enabled(!self.state.busy(), egui::Button::new(aurora_i18n::tr!("ao-combine")))
                    .clicked()
                {
                    self.state.picker = None;
                    self.state.combination = Some(super::combinations::Dialog::new(self.set.id, self.state.state));
                }
            });
            self.list_area(ui, |this, ui| {
                for (index, step) in c.steps.iter().enumerate() {
                    let metadata = step.group.then(|| aurora_i18n::tr!("ao-group-summary", count = step.members.len()));
                    if this.row(
                        ui,
                        step.id,
                        &step.name,
                        metadata.as_deref(),
                        List::Main,
                        this.runtime.index(this.state.state, c.folder) == index,
                    ) {
                        this.runtime.choose(this.state.state, c.folder, index);
                        if step.group {
                            this.state.container = Some(step.id);
                            this.state.cycle_edit = None;
                        }
                    }
                }
                if !c.tracks.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("ao-parallel-tracks")).small().color(this.p.muted));
                    for track in &c.tracks {
                        let metadata = aurora_i18n::tr!("ao-track-summary", count = track.members.len());
                        if this.row(ui, track.id, &track.name, Some(&metadata), List::Tracks, false) {
                            this.state.container = Some(track.id);
                            this.state.cycle_edit = None;
                        }
                    }
                }
                if c.steps.is_empty() && c.tracks.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("ao-drop")).color(this.p.muted));
                }
            });
            self.cycle(ui, c, None);
        }
    }

    fn container_header(&mut self, ui: &mut egui::Ui, id: Uuid, name: &str, track: bool) {
        if widgets::flat_button(ui, self.p, &aurora_i18n::tr!("ao-back-state", state = self.state.state.label())).clicked() {
            self.state.container = None;
            self.state.cycle_edit = None;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(name).strong());
            if ui
                .add_enabled(!self.state.busy(), egui::Button::new(aurora_i18n::tr!("common-rename")))
                .clicked()
            {
                self.state.combination = Some(super::combinations::Dialog::rename(self.set.id, self.state.state, id, name, track));
            }
        });
        ui.label(
            RichText::new(aurora_i18n::text(if track { "ao-track-kind" } else { "ao-group-kind" }))
                .small()
                .color(self.p.violet_pale),
        );
    }

    fn dissolve(&mut self, ui: &mut egui::Ui, id: Uuid, track: bool) {
        if ui
            .add_enabled(
                !self.state.busy(),
                egui::Button::new(aurora_i18n::text(if track { "ao-dissolve-track" } else { "ao-dissolve-group" })),
            )
            .clicked()
        {
            match inventory::editor::dissolve(self.inv, self.set, self.state.state, id) {
                Ok(change) => self.out.push(Action::EditContainer(
                    change,
                    EditorTarget {
                        set: self.set.id,
                        state: self.state.state,
                        container: None,
                    },
                )),
                Err(reason) => self.state.message = reason,
            }
        }
    }

    fn list_area(&mut self, ui: &mut egui::Ui, body: impl FnOnce(&mut Self, &mut egui::Ui)) {
        let response = egui::Frame::new()
            .fill(self.p.field)
            .inner_margin(6.0)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("ao_entries", self.state.container))
                    .max_height(216.0)
                    .show(ui, |ui| body(self, ui));
            })
            .response;
        if let Some(payload) = response.dnd_release_payload::<super::super::inventory::InvDrag>()
            && !self.state.busy()
        {
            push(
                self.out,
                self.state,
                inventory::add(self.inv, self.set, self.state.state, payload.0, self.state.container),
            );
        }
    }

    fn members(&mut self, ui: &mut egui::Ui, c: &StateConfig, owner: Uuid, members: &[Animation], simultaneous: bool, track: bool) {
        self.list_area(ui, |this, ui| {
            for (index, member) in members.iter().enumerate() {
                let selected = simultaneous || (track && this.runtime.index(this.state.state, owner) == index);
                if this.row(ui, member.link, &member.name, None, List::Members(owner), selected) {
                    if track {
                        this.runtime.choose(this.state.state, owner, index);
                    } else if let Some(index) = c.steps.iter().position(|s| s.id == owner) {
                        this.runtime.choose(this.state.state, c.folder, index);
                    }
                }
            }
            if members.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("ao-combine-empty")).color(this.p.muted));
            }
        });
    }

    fn row(&mut self, ui: &mut egui::Ui, id: Uuid, name: &str, metadata: Option<&str>, list: List, selected: bool) -> bool {
        ui.push_id(id, |ui| {
            ui.horizontal(|ui| {
                let label = metadata.map_or_else(|| name.to_owned(), |m| format!("{name}\n{m}"));
                let response = ui
                    .add_enabled(
                        !self.state.busy(),
                        egui::Button::selectable(selected, label)
                            .sense(egui::Sense::click_and_drag())
                            .min_size(Vec2::new((ui.available_width() - 28.0).max(40.0), 28.0)),
                    )
                    .on_hover_text(aurora_i18n::tr!("ao-entry-help"));
                if !self.state.busy() {
                    response.dnd_set_drag_payload(EntryDrag {
                        set: self.set.id,
                        state: self.state.state,
                        list,
                        entry: id,
                    });
                    if let Some(drag) = egui::DragAndDrop::payload::<EntryDrag>(ui.ctx())
                        && drag.set == self.set.id
                        && drag.state == self.state.state
                        && drag.list == list
                        && drag.entry != id
                        && response.contains_pointer()
                        && let Some(pointer) = ui.ctx().pointer_hover_pos()
                    {
                        let after = pointer.y >= response.rect.center().y;
                        let y = if after { response.rect.bottom() } else { response.rect.top() };
                        ui.painter()
                            .hline(response.rect.x_range(), y, egui::Stroke::new(2.0, self.p.violet_light));
                        if response.dnd_release_payload::<EntryDrag>().is_some() {
                            match inventory::editor::reorder(self.inv, self.set, self.state.state, list, drag.entry, id, after) {
                                Ok(change) if !change.operations.is_empty() => self.out.push(Action::Edit(change, None)),
                                Ok(_) => {}
                                Err(reason) => self.state.message = reason,
                            }
                        }
                    }
                }
                if widgets::icon_button(ui, self.p, "trash-filled", aurora_i18n::tr!("common-remove"), !self.state.busy()) {
                    if matches!(list, List::Main) {
                        self.state.remove = Some(id);
                    } else {
                        push(self.out, self.state, inventory::remove(self.inv, id));
                    }
                }
                response.clicked()
            })
            .inner
        })
        .inner
    }

    fn cycle(&mut self, ui: &mut egui::Ui, c: &StateConfig, track: Option<&crate::ao::Track>) {
        let (id, cycle) = track.map_or((c.folder, &c.cycle), |t| (t.id, &t.cycle));
        if id.is_nil() {
            return;
        }
        widgets::divider(ui, self.p);
        ui.label(
            RichText::new(if track.is_some() {
                aurora_i18n::tr!("ao-track-cycle").to_owned()
            } else {
                aurora_i18n::tr!("ao-state-cycle", state = self.state.state.label())
            })
            .small()
            .color(self.p.muted),
        );
        if self.state.cycle_edit.as_ref().is_none_or(|(old, _)| *old != id) {
            self.state.cycle_edit = Some((id, cycle.clone()));
        }
        ui.add_enabled_ui(!self.state.busy(), |ui| {
            let Some((_, edit)) = self.state.cycle_edit.as_mut() else { return };
            ui.horizontal_wrapped(|ui| {
                toggle(ui, self.p, "ao-auto-cycle", &mut edit.cycle);
                toggle(ui, self.p, "ao-random", &mut edit.random);
            });
            let apply = ui
                .horizontal(|ui| {
                    ui.label(aurora_i18n::tr!("ao-cycle-interval"));
                    ui.add(egui::DragValue::new(&mut edit.seconds).range(0.0..=86400.0).suffix(" s").speed(1.0));
                    widgets::flat_button(ui, self.p, aurora_i18n::tr!("common-apply")).clicked()
                })
                .inner;
            if apply {
                let change = if let Some(track) = track {
                    let mut track = track.clone();
                    track.cycle = edit.clone();
                    inventory::editor::save_track(self.inv, &track)
                } else {
                    let mut c = c.clone();
                    c.cycle = edit.clone();
                    inventory::save_cycle(self.inv, self.state.state, &c)
                };
                push(self.out, self.state, change);
            }
        });
    }
}
