use super::*;
use aurora_assets::gesture::{ANIM_STOP, MAX_STEPS, Step, WAIT_ANIMATIONS, WAIT_RELEASE, WAIT_TIME};

pub(super) fn step_label(step: &Step) -> String {
    match step {
        Step::Animation { name, flags, .. } => aurora_i18n::tr!(
            "gesture-step-animation",
            action = if flags & ANIM_STOP == 0 {
                aurora_i18n::tr!("gesture-start")
            } else {
                aurora_i18n::tr!("gesture-stop")
            },
            name = name.as_str()
        ),
        Step::Sound { name, .. } => aurora_i18n::tr!("gesture-step-sound", name = name.as_str()),
        Step::Chat { text, .. } => aurora_i18n::tr!("gesture-step-chat", text = text.as_str()),
        Step::Wait { seconds, flags } => aurora_i18n::tr!(
            "gesture-step-wait",
            value = if flags & WAIT_RELEASE != 0 {
                aurora_i18n::tr!("gesture-wait-release").to_owned()
            } else if flags & WAIT_TIME != 0 {
                aurora_i18n::tr!("gesture-seconds", seconds = aurora_i18n::number_text(&format!("{seconds:.1}")))
            } else {
                aurora_i18n::tr!("gesture-wait-animations").to_owned()
            }
        ),
    }
}

/// Why a gesture key cannot play the gesture: a build shortcut keeps it,
/// always or while the build tools are open.
pub(super) fn shortcut_taken(key: u8, mask: u32) -> Option<String> {
    use crate::build::ui::Shortcut;
    let shortcut = keys::shortcut(key, mask);
    if Shortcut::for_gesture_key(key, mask, false).is_some() {
        Some(aurora_i18n::tr!("gesture-shortcut-taken", shortcut = shortcut.as_str()))
    } else if Shortcut::for_gesture_key(key, mask, true).is_some() {
        Some(aurora_i18n::tr!("gesture-shortcut-taken-building", shortcut = shortcut.as_str()))
    } else {
        None
    }
}

fn flag(ui: &mut egui::Ui, p: &Palette, flags: &mut u32, bit: u32, label: &str) {
    let mut value = *flags & bit != 0;
    if widgets::checkbox(ui, p, &mut value, label).changed() {
        if value {
            *flags |= bit;
        } else {
            *flags &= !bit;
        }
    }
}

fn asset_choice(ui: &mut egui::Ui, inv: &Inventory, assets: &Assets, kind: i32, asset: &mut Uuid, name: &mut String) {
    let choices = assets.choices.iter().filter(|(asset_kind, _, _)| *asset_kind == kind);
    egui::ComboBox::from_id_salt("asset")
        .selected_text(if asset.is_nil() {
            aurora_i18n::tr!("common-none-2")
        } else {
            name.as_str()
        })
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            if ui.selectable_value(asset, Uuid::nil(), aurora_i18n::tr!("common-none-2")).changed() {
                name.clear();
            }
            if kind == 20 {
                for (key, id) in crate::gestures::builtins::ANIMATIONS {
                    let label = aurora_i18n::text(key);
                    if ui.selectable_value(asset, *id, label).changed() {
                        *name = label.to_owned();
                    }
                }
            }
            for (_, id, label) in choices {
                if ui.selectable_value(asset, *id, label).changed() {
                    *name = label.clone();
                }
            }
        });
    if let Some(cargo) = egui::DragAndDrop::payload::<crate::ui::inventory::InvDrag>(ui.ctx())
        && let Some(original) = rules::original(inv, cargo.0)
        && let Some(it) = inv.items.get(&original)
        && it.asset_type == kind
        && it.owner_mask & (rules::COPY | rules::MODIFY | rules::TRANSFER) == rules::COPY | rules::MODIFY | rules::TRANSFER
        && ui.rect_contains_pointer(ui.min_rect())
        && ui.input(|i| i.pointer.any_released())
    {
        *asset = it.asset_id;
        *name = it.name.clone();
    }
}

fn parameters(ui: &mut egui::Ui, p: &Palette, inv: &Inventory, assets: &Assets, step: &mut Step) {
    match step {
        Step::Animation { name, asset, flags } => {
            ui.label(aurora_i18n::tr!("gesture-animation"));
            asset_choice(ui, inv, assets, 20, asset, name);
            flag(ui, p, flags, ANIM_STOP, aurora_i18n::tr!("gesture-stop-animation"));
        }
        Step::Sound { name, asset, .. } => {
            ui.label(aurora_i18n::tr!("gesture-sound"));
            asset_choice(ui, inv, assets, 1, asset, name);
        }
        Step::Chat { text, .. } => {
            ui.label(aurora_i18n::tr!("gesture-message"));
            ui.add(egui::TextEdit::singleline(text).char_limit(255).desired_width(ui.available_width()));
        }
        Step::Wait { seconds, flags } => {
            flag(ui, p, flags, WAIT_RELEASE, aurora_i18n::tr!("gesture-wait-release"));
            flag(ui, p, flags, WAIT_ANIMATIONS, aurora_i18n::tr!("gesture-wait-animations"));
            ui.horizontal(|ui| {
                flag(ui, p, flags, WAIT_TIME, aurora_i18n::tr!("gesture-wait-time"));
                ui.add_enabled(
                    *flags & WAIT_TIME != 0,
                    egui::DragValue::new(seconds).range(0.0..=3600.0).speed(0.1),
                );
            });
        }
    }
}

pub(super) fn show(ctx: &egui::Context, p: &Palette, inv: &Inventory, manager: &Manager, state: &mut GesturesUi, out: &mut Vec<Action>) {
    if state.editors.is_empty() {
        return;
    }
    state.assets.refresh(inv);
    let assets = &state.assets;
    state.editor_order.retain(|id| state.editors.contains_key(id));
    for id in &state.editor_order {
        let Some(editor) = state.editors.get_mut(id) else { continue };
        let Some(it) = inv.items.get(id) else {
            editor.open = false;
            continue;
        };
        let mut open = editor.open;
        let title = aurora_i18n::tr!("gesture-editor-title", name = it.name.as_str());
        let editable = !rules::library(inv, *id);
        let modifiable = editable && it.owner_mask & rules::MODIFY != 0;
        let window_id = format!("gesture_{id}");
        let floater = Floater::new(&window_id, &title, egui::pos2(570.0, 130.0), Vec2::new(400.0, 590.0))
            .cascade(crate::ui::widgets::CascadeGroup::Gesture);
        floater.show(ctx, p, &mut open, |ui| {
            let dirty = editor.dirty();
            let Some(g) = &mut editor.definition else {
                ui.label(if editor.message.is_empty() {
                    aurora_i18n::tr!("gesture-loading")
                } else {
                    editor.message.as_str()
                });
                return;
            };
            let (mut body, foot) = widgets::above_footer(ui);
            body.add_enabled_ui(editor.saving.is_none(), |ui| {
                egui::Grid::new("gesture_fields").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    ui.label(aurora_i18n::tr!("common-description"));
                    ui.add_enabled(
                        modifiable,
                        egui::TextEdit::singleline(&mut editor.description)
                            .char_limit(127)
                            .desired_width(ui.available_width()),
                    );
                    ui.end_row();
                    ui.label(aurora_i18n::tr!("gesture-trigger"));
                    ui.add_enabled(editable, egui::TextEdit::singleline(&mut g.trigger).char_limit(255));
                    ui.end_row();
                    ui.label(aurora_i18n::tr!("gesture-replacement"));
                    ui.add_enabled(
                        editable && (!g.trigger.is_empty() || !g.replacement.is_empty()),
                        egui::TextEdit::singleline(&mut g.replacement).char_limit(255),
                    );
                    ui.end_row();
                    ui.label(aurora_i18n::tr!("gesture-shortcut"));
                    ui.add_enabled_ui(editable, |ui| {
                        ui.horizontal(|ui| {
                            egui::ComboBox::from_id_salt("modifier")
                                .selected_text(match g.mask {
                                    1 => "Ctrl",
                                    4 => aurora_i18n::tr!("keybinds-shift"),
                                    _ => "---",
                                })
                                .show_ui(ui, |ui| {
                                    for (mask, name) in [(0, "---"), (4, aurora_i18n::tr!("keybinds-shift")), (1, "Ctrl")] {
                                        ui.selectable_value(&mut g.mask, mask, name);
                                    }
                                });
                            egui::ComboBox::from_id_salt("key")
                                .selected_text(keys::label(g.key))
                                .height(300.0)
                                .show_ui(ui, |ui| {
                                    for key in keys::choices() {
                                        ui.selectable_value(&mut g.key, key, keys::label(key));
                                    }
                                });
                        })
                    });
                    ui.end_row();
                    // The Legacy Viewer's menu shortcuts silently win over
                    // a gesture key: say so when one is chosen (#915).
                    if let Some(taken) = shortcut_taken(g.key, g.mask) {
                        ui.label("");
                        ui.add(egui::Label::new(RichText::new(taken).color(p.warn)).wrap());
                        ui.end_row();
                    }
                });
                widgets::section_header(ui, p, "library-filled", aurora_i18n::tr!("gesture-library"));
                ui.add_enabled_ui(modifiable, |ui| {
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("library")
                            .selected_text(aurora_i18n::text(
                                ["gesture-animation", "gesture-sound", "gesture-chat", "gesture-wait"][usize::from(editor.library)],
                            ))
                            .show_ui(ui, |ui| {
                                for (index, label) in ["gesture-animation", "gesture-sound", "gesture-chat", "gesture-wait"]
                                    .iter()
                                    .enumerate()
                                {
                                    ui.selectable_value(&mut editor.library, index as u8, aurora_i18n::text(label));
                                }
                            });
                        if ui
                            .add_enabled(g.steps.len() < MAX_STEPS, egui::Button::new(aurora_i18n::tr!("gesture-add-step")))
                            .clicked()
                        {
                            g.steps.push(match editor.library {
                                0 => Step::Animation {
                                    name: String::new(),
                                    asset: Uuid::nil(),
                                    flags: 0,
                                },
                                1 => Step::Sound {
                                    name: String::new(),
                                    asset: Uuid::nil(),
                                    flags: 0,
                                },
                                2 => Step::Chat {
                                    text: String::new(),
                                    flags: 0,
                                },
                                _ => Step::Wait {
                                    seconds: 1.0,
                                    flags: WAIT_TIME,
                                },
                            });
                            editor.step = Some(g.steps.len() - 1);
                        }
                    })
                });
                widgets::section_header(ui, p, "list", aurora_i18n::tr!("gesture-steps"));
                let (mut list, foot) = widgets::above_footer(ui);
                egui::ScrollArea::vertical()
                    .max_height(list.available_height())
                    .auto_shrink([false, false])
                    .show(&mut list, |ui| {
                        for (index, step) in g.steps.iter().enumerate() {
                            if ui.selectable_label(editor.step == Some(index), step_label(step)).clicked() {
                                editor.step = Some(index);
                            }
                        }
                    });
                foot.show(ui, |ui| {
                    ui.add_enabled_ui(modifiable, |ui| {
                        ui.horizontal(|ui| {
                            let index = editor.step;
                            if ui
                                .add_enabled(index.is_some_and(|i| i > 0), egui::Button::new(aurora_i18n::tr!("gesture-up")))
                                .clicked()
                                && let Some(i) = index
                            {
                                g.steps.swap(i, i - 1);
                                editor.step = Some(i - 1);
                            }
                            if ui
                                .add_enabled(
                                    index.is_some_and(|i| i + 1 < g.steps.len()),
                                    egui::Button::new(aurora_i18n::tr!("gesture-down")),
                                )
                                .clicked()
                                && let Some(i) = index
                            {
                                g.steps.swap(i, i + 1);
                                editor.step = Some(i + 1);
                            }
                            if ui
                                .add_enabled(index.is_some(), egui::Button::new(aurora_i18n::tr!("common-delete")))
                                .clicked()
                                && let Some(i) = index
                            {
                                g.steps.remove(i);
                                editor.step = (!g.steps.is_empty()).then_some(i.saturating_sub(1));
                            }
                        });
                        if let Some(step) = editor.step.and_then(|i| g.steps.get_mut(i)) {
                            parameters(ui, p, inv, assets, step);
                        }
                    });
                    ui.label(RichText::new(aurora_i18n::tr!("gesture-simultaneous")).small().color(p.muted));
                    ui.horizontal(|ui| {
                        let mut active = manager.active.contains(id);
                        ui.add_enabled_ui(editable, |ui| {
                            if widgets::checkbox(ui, p, &mut active, aurora_i18n::tr!("gesture-active")).changed() {
                                out.push(Action::Active(vec![*id], active));
                            }
                        });
                        if ui
                            .add_enabled(
                                manager.enabled,
                                egui::Button::new(if manager.is_playing(editor.preview) {
                                    aurora_i18n::tr!("gesture-stop")
                                } else {
                                    aurora_i18n::tr!("gesture-preview")
                                }),
                            )
                            .clicked()
                        {
                            out.push(if manager.is_playing(editor.preview) {
                                Action::Stop(editor.preview)
                            } else {
                                Action::Preview(editor.preview, g.clone())
                            });
                        }
                        if ui
                            .add_enabled(editable && dirty, egui::Button::new(aurora_i18n::tr!("gesture-save")))
                            .clicked()
                        {
                            out.push(Action::Save {
                                item: *id,
                                definition: g.clone(),
                                description: editor.description.clone(),
                                close: false,
                            });
                        }
                    });
                });
            });
            foot.show(ui, |ui| {
                if editor.saving.is_some() {
                    ui.label(aurora_i18n::tr!("gesture-saving"));
                }
                if !editor.message.is_empty() {
                    ui.label(RichText::new(&editor.message).color(p.warn));
                }
            });
        });
        if !open && editor.dirty() {
            state.discard = Some(*id);
            editor.open = true;
        } else {
            editor.open = open;
        }
    }
    if let Some(id) = state.raise_editor.take() {
        let window_id = format!("gesture_{id}");
        widgets::open_floater(ctx, &window_id, &mut true);
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(window_id)));
    }
    if let Some(id) = state.discard {
        let response = widgets::Modal::new(egui::Id::unique("gesture_discard")).show(ctx, |ui| {
            ui.set_width(400.0);
            widgets::section_header(ui, p, "device-floppy", aurora_i18n::tr!("ui-inventory-dialogs-save-changes"));
            ui.label(aurora_i18n::tr!("gesture-unsaved"));
            ui.horizontal(|ui| {
                if widgets::default_button(ui.button(aurora_i18n::tr!("gesture-save"))).clicked()
                    && let Some(e) = state.editors.get(&id)
                    && let Some(g) = &e.definition
                {
                    out.push(Action::Save {
                        item: id,
                        definition: g.clone(),
                        description: e.description.clone(),
                        close: true,
                    });
                    state.discard = None;
                }
                if ui.button(aurora_i18n::tr!("gesture-discard")).clicked() {
                    if let Some(editor) = state.editors.remove(&id) {
                        out.push(Action::Stop(editor.preview));
                    }
                    state.discard = None;
                }
                if ui.button(aurora_i18n::tr!("common-cancel")).clicked() {
                    state.discard = None;
                }
            });
        });
        if response.should_close() {
            state.discard = None;
        }
    }
    let closed: Vec<_> = state
        .editors
        .iter()
        .filter_map(|(id, e)| (!e.open && e.saving.is_none()).then_some(*id))
        .collect();
    for id in closed {
        if let Some(editor) = state.editors.remove(&id) {
            out.push(Action::Stop(editor.preview));
        }
    }
}

#[derive(Default)]
pub(super) struct Assets {
    generation: Option<u64>,
    choices: Vec<(i32, Uuid, String)>,
}
impl Assets {
    fn refresh(&mut self, inv: &Inventory) {
        if self.generation == Some(inv.generation) {
            return;
        }
        self.generation = Some(inv.generation);
        self.choices = inv
            .items
            .values()
            .filter(|it| {
                matches!(it.asset_type, 1 | 20)
                    && !it.asset_id.is_nil()
                    && it.owner_mask & (rules::COPY | rules::MODIFY | rules::TRANSFER) == rules::COPY | rules::MODIFY | rules::TRANSFER
                    && !rules::in_type(inv, it.id, 14)
            })
            .map(|it| (it.asset_type, it.asset_id, it.name.clone()))
            .collect();
        self.choices.sort_by_cached_key(|(kind, _, name)| (*kind, name.to_lowercase()));
    }
}
