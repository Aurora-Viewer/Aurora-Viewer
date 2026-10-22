//! The physics layer editor: panel_edit_physics.xml's accordion (one tab
//! open at a time) and LLScrollingPanelParamBase's 0–100 sliders, with the
//! shape editor's surfaces, footer and dialogs.
use super::widgets::{self, ActionKind};
use crate::{
    physics_editor::{self as editor, State},
    theme::Palette,
    world::World,
};
use egui::{RichText, Vec2};

pub enum Action {
    Save(Option<String>),
    Retry,
}

/// `physics_on`: Preferences › Graphics › Avatar physics, without which the
/// preview cannot move.
pub fn show(ctx: &egui::Context, p: &Palette, world: &mut World, st: &mut State, physics_on: bool) -> Vec<Action> {
    if !st.open {
        return Vec::new();
    }
    let mut actions = Vec::new();
    let had_confirmation = st.close_confirmation;
    let had_copy = st.copy_name.is_some();
    let mut changed = false;
    let mut open = true;
    let mut close = false;
    let mut revert = false;
    let mut floater = widgets::Floater::new(
        "physics_editor",
        aurora_i18n::tr!("physics-title"),
        egui::pos2(28.0, 64.0),
        Vec2::new(380.0, 640.0),
    )
    .help(aurora_i18n::tr!("physics-help"));
    floater.min_size = Vec2::new(320.0, 380.0);
    floater.show(ctx, p, &mut open, |ui| {
        let male = editor::male(world);
        if let Some(draft) = &mut st.draft {
            let busy = draft.pending.is_some();
            let modify = draft.can_modify(world);
            let copy = draft.can_copy(world);
            widgets::surface(ui, p, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    widgets::glyph(ui, "shirt-filled", p.muted, 24.0);
                    ui.add(egui::Label::new(RichText::new(&draft.item.name).strong().size(15.0).color(p.ink)).truncate());
                });
                if !physics_on {
                    ui.label(RichText::new(aurora_i18n::tr!("physics-preview-off")).size(12.0).color(p.warn));
                }
                if !modify {
                    ui.label(RichText::new(aurora_i18n::tr!("physics-readonly")).size(12.0).color(p.warn));
                }
            });
            ui.add_space(6.0);
            // the save controls stay at the bottom (LLPanelEditWearable),
            // laid out first so their height is reserved
            let rest = ui.available_rect_before_wrap();
            ui.allocate_rect(rest, egui::Sense::hover());
            let mut footer = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rest)
                    .layout(egui::Layout::bottom_up(egui::Align::Min)),
            );
            if busy {
                footer.label(RichText::new(aurora_i18n::tr!("physics-saving")).color(p.muted));
            }
            footer.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    let size = Vec2::new((ui.available_width() - 16.0) / 3.0, 30.0);
                    if widgets::action_button_if(
                        ui,
                        p,
                        "copy-filled",
                        aurora_i18n::tr!("common-save-as"),
                        ActionKind::Flat,
                        size,
                        copy,
                    )
                    .clicked()
                    {
                        st.copy_name = Some(draft.item.name.clone());
                    }
                    if widgets::action_button_if(ui, p, "restore", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size, true)
                        .clicked()
                    {
                        revert = true;
                        close = true;
                    }
                    if widgets::action_button_if(
                        ui,
                        p,
                        "device-floppy",
                        aurora_i18n::tr!("common-save"),
                        ActionKind::Primary,
                        size,
                        modify && draft.dirty(),
                    )
                    .clicked()
                    {
                        actions.push(Action::Save(None));
                    }
                });
            });
            if !st.message.is_empty() {
                footer.add_space(6.0);
                footer.label(RichText::new(&st.message).size(12.0).color(p.warn));
            }
            footer.add_space(6.0);
            let mut controls = footer.new_child(
                egui::UiBuilder::new()
                    .max_rect(footer.available_rect_before_wrap())
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            let ui = &mut controls;
            let available = ui.available_height().max(0.0);
            egui::ScrollArea::vertical()
                .id_salt("physics_parameters")
                .max_height(available)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for group in editor::visible_groups(male) {
                        let section = widgets::collapsing_forced(
                            ui,
                            p,
                            ("physics_group", group),
                            editor::group_label(group),
                            false,
                            Some(st.group == Some(group)),
                            |ui| {
                                ui.add_enabled_ui(modify && !busy, |ui| {
                                    for param in editor::visible_params(group, male) {
                                        changed |= slider(ui, p, draft, param);
                                    }
                                });
                            },
                        );
                        // single_expansion: opening a tab closes the others
                        if section.header.clicked() {
                            st.group = (st.group != Some(group)).then_some(group);
                        }
                    }
                });
        } else {
            ui.label(
                RichText::new(if st.loading.is_some() {
                    aurora_i18n::tr!("physics-loading")
                } else {
                    &st.message
                })
                .color(p.muted),
            );
            if st.loading.is_none()
                && widgets::action_button(
                    ui,
                    p,
                    "refresh",
                    aurora_i18n::tr!("shape-retry"),
                    ActionKind::Flat,
                    Vec2::new(120.0, 30.0),
                )
                .clicked()
            {
                actions.push(Action::Retry);
            }
        }
    });
    if revert {
        st.revert(world);
    } else if changed {
        st.changed(world);
    }
    if !open || (!had_confirmation && !had_copy && ctx.input(|i| i.key_pressed(egui::Key::Escape))) || close {
        if st.draft.as_ref().is_some_and(|d| d.pending.is_some()) {
            st.open = true;
        } else if st.draft.as_ref().is_some_and(|d| d.dirty()) {
            st.close_confirmation = true;
        } else {
            st.close(world);
        }
    }
    if st.copy_name.is_some() {
        let mut confirmed = false;
        let mut cancel = false;
        let response = widgets::Modal::new(egui::Id::unique("physics_copy_name")).show(ctx, |ui| {
            ui.set_width(350.0);
            ui.label(RichText::new(aurora_i18n::tr!("physics-save-as")).strong().size(17.0));
            if let Some(name) = &mut st.copy_name {
                ui.add(
                    egui::TextEdit::singleline(name)
                        .hint_text(aurora_i18n::tr!("physics-copy-name"))
                        .desired_width(f32::INFINITY),
                );
            }
            ui.horizontal(|ui| {
                if widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked() {
                    cancel = true;
                }
                if ui
                    .add_enabled_ui(st.copy_name.as_ref().is_some_and(|n| !n.trim().is_empty()), |ui| {
                        widgets::primary_button_sized(ui, p, aurora_i18n::tr!("common-save"), Vec2::new(140.0, 30.0))
                    })
                    .inner
                    .clicked()
                {
                    confirmed = true;
                }
            });
        });
        if had_copy && response.should_close() {
            cancel = true;
        }
        if confirmed {
            actions.push(Action::Save(st.copy_name.clone()));
        }
        if cancel {
            st.copy_name = None;
        }
    }
    if st.close_confirmation {
        let mut discard = false;
        let mut cancel = false;
        let mut save = false;
        let response = widgets::Modal::new(egui::Id::unique("physics_unsaved")).show(ctx, |ui| {
            ui.set_width(350.0);
            ui.label(RichText::new(aurora_i18n::tr!("physics-close-title")).strong().size(17.0));
            ui.label(aurora_i18n::tr!("shape-close-message"));
            ui.horizontal(|ui| {
                discard = widgets::flat_button(ui, p, aurora_i18n::tr!("shape-discard")).clicked();
                cancel = widgets::flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked();
                save = widgets::primary_button_sized(ui, p, aurora_i18n::tr!("common-save"), Vec2::new(120.0, 30.0)).clicked();
            });
        });
        if had_confirmation && response.should_close() {
            cancel = true;
        }
        if discard {
            st.close(world);
        }
        if cancel {
            st.close_confirmation = false;
        }
        if save {
            st.close_confirmation = false;
            st.close_after_save = true;
            actions.push(Action::Save(None));
        }
    }
    actions
}

/// One parameter: its name, a 0–100 slider and the value
/// (LLScrollingPanelParamBase::weightToPercent). True when it changed.
fn slider(ui: &mut egui::Ui, p: &Palette, draft: &mut crate::shape_editor::Draft, param: &editor::Param) -> bool {
    let mut changed = false;
    ui.push_id(param.id, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.label(RichText::new(param.label()).strong().color(p.ink));
        let old = editor::value(draft, param);
        let mut percent = (old - param.min) / (param.max - param.min) * 100.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = (ui.available_width() - 70.0).max(100.0);
            let a = ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).show_value(false));
            let b = ui.add_sized(
                Vec2::new(62.0, 22.0),
                egui::DragValue::new(&mut percent)
                    .range(0.0..=100.0)
                    .speed(0.1)
                    .custom_formatter(|value, _| aurora_i18n::number(value, 1))
                    .custom_parser(aurora_i18n::parse_number),
            );
            if a.changed() || b.changed() {
                editor::set(draft, param, param.min + (param.max - param.min) * percent / 100.0);
                changed = true;
            }
        });
        ui.add_space(2.0);
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics_editor::tests::fixture;

    fn frame(ctx: &egui::Context, world: &mut World, state: &mut State, events: Vec<egui::Event>) -> egui::FullOutput {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0))),
            events,
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let _ = show(ui.ctx(), &crate::theme::Theme::default().palette(), world, state, true);
        });
        // no renderer: drop the font and icon uploads (egui debug assertion)
        output.textures_delta.clear();
        output
    }

    fn texts(output: &egui::FullOutput) -> Vec<String> {
        output
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_open_tab_shows_its_sliders_and_drawing_changes_nothing() {
        let (mut world, mut state, _) = fixture();
        let ctx = egui::Context::default();
        crate::theme::Theme::default().apply(&ctx, 1.0);
        let original = state.draft.as_ref().expect("draft").edited.params.clone();
        let mut output = Default::default();
        for _ in 0..6 {
            output = frame(&ctx, &mut world, &mut state, Vec::new());
        }
        let shown = texts(&output);
        for group in 0..7 {
            assert!(
                shown.iter().any(|t| t == editor::group_label(group)),
                "{}",
                editor::group_label(group)
            );
        }
        assert!(shown.iter().any(|t| t == aurora_i18n::tr!("physics-max-effect")), "first tab open");
        assert!(
            !shown.iter().any(|t| t == aurora_i18n::tr!("physics-breast-mass")),
            "advanced closed"
        );
        assert_eq!(state.draft.as_ref().expect("draft").edited.params, original);
        // the advanced tab instead
        state.group = Some(6);
        for _ in 0..30 {
            output = frame(&ctx, &mut world, &mut state, Vec::new());
        }
        let shown = texts(&output);
        assert!(shown.iter().any(|t| t == aurora_i18n::tr!("physics-breast-mass")));
        assert!(!state.draft.as_ref().expect("draft").dirty());
    }

    #[test]
    fn escape_on_a_dirty_layer_asks_before_closing() {
        let (mut world, mut state, _) = fixture();
        let ctx = egui::Context::default();
        let breast = editor::param_by_id(10003).expect("param");
        editor::set(state.draft.as_mut().expect("draft"), breast, 1.0);
        state.changed(&mut world);
        let escape = egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        };
        frame(&ctx, &mut world, &mut state, Vec::new());
        frame(&ctx, &mut world, &mut state, vec![escape]);
        assert!(state.open && state.close_confirmation);
        assert!(state.draft.as_ref().expect("draft").dirty(), "the draft stays");
    }
}
