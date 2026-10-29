//! Firestorm's shape editor layout with Aurora's shared surfaces and controls.
use super::widgets::{self, ActionKind};
use crate::{
    shape_editor::{State, catalog},
    theme::Palette,
    world::World,
};
use egui::{RichText, Vec2};

pub enum Action {
    Save(Option<String>),
    Retry,
    CustomizeAnimation(bool),
}

fn groups() -> [&'static str; 9] {
    [
        aurora_i18n::tr!("shape-group-body"),
        aurora_i18n::tr!("shape-group-head"),
        aurora_i18n::tr!("shape-group-eyes"),
        aurora_i18n::tr!("shape-group-ears"),
        aurora_i18n::tr!("shape-group-nose"),
        aurora_i18n::tr!("shape-group-mouth"),
        aurora_i18n::tr!("shape-group-chin"),
        aurora_i18n::tr!("shape-group-torso"),
        aurora_i18n::tr!("shape-group-legs"),
    ]
}

fn category_wheel(
    ctx: &egui::Context,
    group: usize,
    remainder: &mut f32,
    last_frame: &mut Option<u64>,
    response: &egui::Response,
    enabled: bool,
) -> Option<usize> {
    if !enabled || !response.contains_pointer() {
        *remainder = 0.0;
        return None;
    }
    let frame = ctx.cumulative_frame_nr();
    if *last_frame == Some(frame) {
        return None;
    }
    *last_frame = Some(frame);
    let delta = ctx.input_mut(|input| {
        let delta: f32 = input
            .events
            .iter()
            .filter_map(|event| {
                let egui::Event::MouseWheel {
                    unit, delta, modifiers, ..
                } = event
                else {
                    return None;
                };
                (*modifiers == egui::Modifiers::NONE).then_some(
                    (delta.x + delta.y)
                        / match unit {
                            egui::MouseWheelUnit::Point => 55.0,
                            _ => 1.0,
                        },
                )
            })
            .filter(|delta| delta.is_finite())
            .sum::<f32>()
            .clamp(-9.0, 9.0);
        if delta != 0.0 {
            input.smooth_scroll_delta = Vec2::ZERO;
        }
        delta
    });
    *remainder += delta;
    let steps = remainder.trunc() as i32;
    *remainder -= steps as f32;
    if steps != 0 {
        let next = (group as i32 - steps).clamp(0, 8) as usize;
        if next != group {
            ctx.request_repaint();
            return Some(next);
        }
    }
    None
}

pub fn show(ctx: &egui::Context, p: &Palette, world: &mut World, st: &mut State, mut customize_animation: bool) -> (Vec<Action>, Vec<i32>) {
    if !st.open {
        return (Vec::new(), Vec::new());
    }
    let mut actions = Vec::new();
    let had_confirmation = st.close_confirmation;
    let had_copy = st.copy_name.is_some();
    let mut visible = Vec::new();
    let mut changed = false;
    st.editing = false;
    let mut open = true;
    let mut close = false;
    let mut reset = false;
    let mut floater = widgets::Floater::new(
        "shape_editor",
        aurora_i18n::tr!("shape-title"),
        egui::pos2(28.0, 64.0),
        Vec2::new(440.0, 680.0),
    )
    .help(aurora_i18n::tr!("shape-help"));
    floater.min_size = Vec2::new(350.0, 430.0);
    floater.show(ctx, p, &mut open, |ui| {
        if let Some(draft) = &mut st.draft {
            let busy = draft.pending.is_some();
            let modify = draft.can_modify(world);
            let copy = draft.can_copy(world);
            let male = draft.edited.params.get(&80).copied().unwrap_or(0.0) > 0.5;
            widgets::surface(ui, p, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    widgets::glyph(ui, "man-filled", p.muted, 24.0);
                    ui.add(egui::Label::new(RichText::new(&draft.item.name).strong().size(15.0).color(p.ink)).truncate());
                });
                ui.add_enabled_ui(modify && !busy, |ui| {
                    let mut selection = male;
                    ui.horizontal(|ui| {
                        widgets::radio_value(ui, p, &mut selection, true, aurora_i18n::tr!("shape-male"));
                        widgets::radio_value(ui, p, &mut selection, false, aurora_i18n::tr!("shape-female"));
                    });
                    if selection != male {
                        draft.edited.params.insert(80, f32::from(selection));
                        changed = true;
                    }
                });
                let weights = world
                    .avatar_lib
                    .shape_params
                    .weights_with(world.server_visual_params(), &draft.edited.params);
                let shape = world.avatar_lib.shape_params.shape_weights(weights, &world.avatar_lib.rig);
                let (_, height) = crate::scene::shape::body_size(&world.avatar_lib.rig, &shape.local_pos, &shape.scale);
                // LLPanelEditWearable::updateTypeSpecificControls adds 0.195 m
                // to the server collision height for the displayed stature.
                let height = height + 0.195;
                ui.horizontal(|ui| {
                    let value = aurora_i18n::number((height * if st.feet { 3.28084 } else { 1.0 }) as f64, 2);
                    ui.label(RichText::new(aurora_i18n::tr!("shape-height", value = value)).color(p.muted));
                    if widgets::flat_button(
                        ui,
                        p,
                        if st.feet {
                            aurora_i18n::tr!("shape-feet")
                        } else {
                            aurora_i18n::tr!("shape-meters")
                        },
                    )
                    .clicked()
                    {
                        st.feet = !st.feet;
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::switch_labeled(ui, p, &mut customize_animation, aurora_i18n::tr!("shape-t-pose"))
                            .on_hover_text(aurora_i18n::tr!("ui-options-customize-animation-hint"))
                            .changed()
                        {
                            actions.push(Action::CustomizeAnimation(customize_animation));
                        }
                    });
                });
            });
            ui.add_space(6.0);
            let names = groups();
            let category_row = ui.horizontal(|ui| {
                if widgets::icon_button(ui, p, "arrow-left", aurora_i18n::tr!("common-previous"), st.group > 0) {
                    st.group -= 1;
                    st.filter.clear();
                }
                let width = (ui.available_width() - 28.0).max(180.0);
                let count = (width / 72.0).floor().clamp(3.0, 9.0) as usize;
                let start = st.group.saturating_sub(count / 2).min(9 - count);
                let mut selected = st.group - start;
                let tabs: Vec<_> = names[start..start + count].iter().map(|name| (*name, true)).collect();
                ui.scope(|ui| {
                    ui.set_width(width);
                    if widgets::tabs(ui, p, &mut selected, &tabs) {
                        st.filter.clear();
                    }
                });
                st.group = start + selected;
                if widgets::icon_button(ui, p, "arrow-right", aurora_i18n::tr!("common-next"), st.group < 8) {
                    st.group += 1;
                    st.filter.clear();
                }
            });
            if let Some(group) = category_wheel(
                ctx,
                st.group,
                &mut st.category_wheel,
                &mut st.category_wheel_frame,
                &category_row.response,
                !had_confirmation && !had_copy,
            ) {
                st.group = group;
                st.filter.clear();
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let width = (ui.available_width() - 32.0 - ui.spacing().item_spacing.x).max(80.0);
                widgets::search_field(ui, &mut st.filter, aurora_i18n::tr!("shape-search"), width);
                if widgets::icon_toggle(ui, p, "photo-filled", aurora_i18n::tr!("shape-hints"), st.hints) {
                    st.hints = !st.hints;
                }
            });
            if !modify {
                ui.label(RichText::new(aurora_i18n::tr!("shape-readonly")).color(p.warn));
            }
            let sex = if draft.edited.params.get(&80).copied().unwrap_or(0.0) > 0.5 {
                2
            } else {
                1
            };
            let query = st.filter.trim().to_lowercase();
            let mut params: Vec<_> = catalog::PARAMS
                .iter()
                .filter(|param| {
                    param.sex & sex != 0
                        && (if query.is_empty() {
                            param.group == st.group
                        } else {
                            param.labels().0.to_lowercase().contains(&query)
                        })
                })
                .collect();
            params.sort_by(|a, b| a.group.cmp(&b.group).then(a.order.total_cmp(&b.order)).then(a.id.cmp(&b.id)));
            // LLPanelEditWearable keeps its save controls at the bottom.
            // Lay them out first using their actual height: an unreserved
            // pending/wrapped label otherwise grows the floater every frame.
            let rest = ui.available_rect_before_wrap();
            ui.allocate_rect(rest, egui::Sense::hover());
            let mut footer = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(rest)
                    .layout(egui::Layout::bottom_up(egui::Align::Min)),
            );
            if busy {
                footer.label(RichText::new(aurora_i18n::tr!("shape-saving")).color(p.muted));
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
                        reset = true;
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
            // Keep a complete row (both hints, captions and value) reachable
            // at the minimum viewport. Fixed row geometry makes virtualisation
            // accurate even while asynchronous previews replace placeholders.
            let image_size = ((widgets::scrolled_width(ui) - 8.0) * 0.5)
                .min(128.0)
                .min((available - 80.0).clamp(64.0, 128.0));
            let row_height = if st.hints { image_size + 80.0 } else { 67.0 };
            let mut hint_views: Vec<_> = Vec::new();
            egui::ScrollArea::vertical()
                .id_salt(("shape_parameters", st.group, st.hints))
                .max_height(available)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, params.len(), |ui, range| {
                    if params.is_empty() {
                        ui.label(RichText::new(aurora_i18n::tr!("shape-no-results")).color(p.muted));
                    }
                    ui.add_enabled_ui(modify && !busy, |ui| {
                        for index in range {
                            let param = params[index];
                            let (label, min_label, max_label) = param.labels();
                            visible.push(param.id);
                            ui.push_id(param.id, |parent| {
                                let (rect, _) =
                                    parent.allocate_exact_size(Vec2::new(parent.available_width(), row_height), egui::Sense::hover());
                                let mut row = parent.new_child(
                                    egui::UiBuilder::new()
                                        .max_rect(rect)
                                        .layout(egui::Layout::top_down(egui::Align::Min)),
                                );
                                row.set_clip_rect(rect.intersect(parent.clip_rect()));
                                row.spacing_mut().item_spacing.y = 3.0;
                                let ui = &mut row;
                                ui.label(RichText::new(label).strong().color(p.ink));
                                if st.hints {
                                    ui.horizontal(|ui| {
                                        for (max, text) in [(false, min_label), (true, max_label)] {
                                            ui.vertical(|ui| {
                                                let size = Vec2::splat(image_size);
                                                let response = if let Some(model) = st.models.get(&(param.id, max)) {
                                                    let view = aurora_render::preview::View {
                                                        yaw: param.angle.to_radians(),
                                                        pitch: 0.0,
                                                        distance: param.distance * if param.group == 0 { 1.6 } else { 1.32 },
                                                        background: p.field.to_normalized_gamma_f32(),
                                                        ambient: [0.42; 3],
                                                        ..Default::default()
                                                    };
                                                    aurora_render::preview::paint_hint(
                                                        ui,
                                                        ui.scope_id().with(max),
                                                        model.clone(),
                                                        view,
                                                        size,
                                                    )
                                                } else {
                                                    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
                                                    ui.painter().rect_filled(rect, ui.visuals().window_corner_radius, p.field);
                                                    ui.painter().text(
                                                        rect.center(),
                                                        egui::Align2::CENTER_CENTER,
                                                        aurora_i18n::tr!("shape-preview-loading"),
                                                        egui::FontId::proportional(11.0),
                                                        p.muted,
                                                    );
                                                    response
                                                };
                                                ui.painter().rect_stroke(
                                                    response.rect,
                                                    ui.visuals().window_corner_radius,
                                                    egui::Stroke::new(1.0, if response.hovered() { p.violet_light } else { p.raised }),
                                                    egui::StrokeKind::Inside,
                                                );
                                                if response.clicked() {
                                                    let step = (param.max - param.min) * 0.01;
                                                    let value = draft.value(param) + if max { step } else { -step };
                                                    draft.set(param, value);
                                                    changed = true;
                                                }
                                                ui.add(egui::Label::new(RichText::new(text).size(11.0).color(p.muted)).truncate());
                                            });
                                        }
                                    });
                                }
                                let old = draft.value(param);
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
                                        draft.set(param, param.min + (param.max - param.min) * percent / 100.0);
                                        changed = true;
                                    }
                                    st.editing |= a.dragged() || b.dragged() || b.has_focus();
                                });
                                ui.add_space(9.0);
                            });
                        }
                    });
                });
            // Keep offscreen geometry bounded to the visible rows.
            hint_views.extend(visible.iter().copied());
            st.models.retain(|(id, _), _| hint_views.contains(id));
        } else {
            ui.label(
                RichText::new(if st.loading.is_some() {
                    aurora_i18n::tr!("shape-loading")
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
    if reset && let Some(draft) = &mut st.draft {
        draft.edited = draft.original.clone();
    }
    if changed || reset {
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
        let response = widgets::Modal::new(egui::Id::unique("shape_copy_name")).show(ctx, |ui| {
            ui.set_width(350.0);
            ui.label(RichText::new(aurora_i18n::tr!("shape-save-as")).strong().size(17.0));
            if let Some(name) = &mut st.copy_name {
                ui.add(
                    egui::TextEdit::singleline(name)
                        .hint_text(aurora_i18n::tr!("shape-copy-name"))
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
        let response = widgets::Modal::new(egui::Id::unique("shape_unsaved")).show(ctx, |ui| {
            ui.set_width(350.0);
            ui.label(RichText::new(aurora_i18n::tr!("shape-close-title")).strong().size(17.0));
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
    (actions, visible)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run_headless(ctx: &egui::Context, input: egui::RawInput, draw: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
        let mut output = ctx.run_ui(input, draw);
        // These tests inspect layout/input without a renderer. Explicitly
        // discard font/icon uploads as required by egui's debug assertions.
        output.textures_delta.clear();
        output
    }

    fn draw(ctx: &egui::Context, world: &mut World, state: &mut State, events: Vec<egui::Event>) -> Vec<Action> {
        let mut actions = Vec::new();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0))),
            events,
            ..Default::default()
        };
        let _ = run_headless(ctx, input, |ui| {
            actions = show(ui.ctx(), &crate::theme::Theme::default().palette(), world, state, true).0;
        });
        actions
    }
    fn escape() -> egui::Event {
        egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }
    #[test]
    fn wheel_over_categories_navigates_both_ways_and_respects_bounds_and_modals() {
        let (mut world, mut state) = crate::shape_editor::tests::fixture();
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 800.0))),
            ..Default::default()
        };
        let mut output = Default::default();
        for _ in 0..3 {
            output = run_headless(&ctx, input.clone(), |ui| {
                let _ = show(ui.ctx(), &crate::theme::Theme::default().palette(), &mut world, &mut state, true);
            });
        }
        let at = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == groups()[1] => Some(text.pos + text.galley.size() * 0.5),
                _ => None,
            })
            .expect("head category");
        let wheel = |unit, value| egui::Event::MouseWheel {
            unit,
            delta: Vec2::new(0.0, value),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        draw(
            &ctx,
            &mut world,
            &mut state,
            vec![egui::Event::PointerMoved(at), wheel(egui::MouseWheelUnit::Line, -1.0)],
        );
        assert_eq!(state.group, 1);
        draw(&ctx, &mut world, &mut state, vec![wheel(egui::MouseWheelUnit::Line, 1.0)]);
        assert_eq!(state.group, 0);
        draw(&ctx, &mut world, &mut state, vec![wheel(egui::MouseWheelUnit::Point, -30.0)]);
        assert_eq!(state.group, 0);
        draw(&ctx, &mut world, &mut state, vec![wheel(egui::MouseWheelUnit::Point, -30.0)]);
        assert_eq!(state.group, 1);
        state.group = 8;
        draw(&ctx, &mut world, &mut state, vec![wheel(egui::MouseWheelUnit::Line, -1.0)]);
        assert_eq!(state.group, 8);
        draw(
            &ctx,
            &mut world,
            &mut state,
            vec![
                egui::Event::PointerMoved(egui::pos2(700.0, 650.0)),
                wheel(egui::MouseWheelUnit::Line, 1.0),
            ],
        );
        assert_eq!(state.group, 8);
        state.copy_name = Some("Copy".into());
        draw(
            &ctx,
            &mut world,
            &mut state,
            vec![egui::Event::PointerMoved(at), wheel(egui::MouseWheelUnit::Line, 1.0)],
        );
        assert_eq!(state.group, 8);
    }

    #[test]
    fn saving_and_wrapped_feedback_keep_the_editor_size_and_footer_visible() {
        for (screen, hints) in [
            (Vec2::new(1280.0, 1000.0), true),
            (Vec2::new(800.0, 560.0), true),
            (Vec2::new(800.0, 560.0), false),
        ] {
            let (mut world, mut state) = crate::shape_editor::tests::fixture();
            state.hints = hints;
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, screen)),
                ..Default::default()
            };
            let mut frame = |state: &mut State| {
                run_headless(&ctx, input.clone(), |ui| {
                    let _ = show(ui.ctx(), &crate::theme::Theme::default().palette(), &mut world, state, true);
                })
            };
            for _ in 0..8 {
                frame(&mut state);
            }
            let initial = egui::AreaState::load(&ctx, egui::Id::unique("shape_editor"))
                .and_then(|s| s.size)
                .expect("editor size");
            // Both save commands share the asynchronous pending state. Keep
            // each pending for many frames, as a real grid upload may be.
            for copy in [false, true] {
                state.draft.as_mut().expect("draft").pending = Some(uuid::Uuid::from_u128(91));
                state.copy_name = copy.then(|| "Copy".into());
                state.message.clear();
                for _ in 0..40 {
                    frame(&mut state);
                    let size = egui::AreaState::load(&ctx, egui::Id::unique("shape_editor"))
                        .and_then(|s| s.size)
                        .expect("editor size");
                    assert!((size - initial).length() < 0.5, "pending copy={copy}: {initial:?} -> {size:?}");
                }
                state.draft.as_mut().expect("draft").pending = None;
                state.copy_name = None;
            }
            for message in [
                aurora_i18n::tr!("shape-saved").to_owned(),
                aurora_i18n::tr!("shape-save-interrupted").repeat(3),
                String::new(),
            ] {
                state.message = message;
                for _ in 0..8 {
                    let output = frame(&mut state);
                    let size = egui::AreaState::load(&ctx, egui::Id::unique("shape_editor"))
                        .and_then(|s| s.size)
                        .expect("editor size");
                    assert!((size - initial).length() < 0.5, "feedback: {initial:?} -> {size:?}");
                    assert!(
                        output.shapes.iter().any(|shape| {
                            matches!(&shape.shape, egui::Shape::Text(text)
                            if text.galley.text() == aurora_i18n::tr!("common-save-as")
                            && shape.clip_rect.contains_rect(egui::Rect::from_min_size(text.pos, text.galley.size())))
                        }),
                        "footer must remain visible"
                    );
                }
            }
        }
    }

    #[test]
    fn drawing_the_editor_does_not_round_or_edit_the_original_shape() {
        let (mut world, mut state) = crate::shape_editor::tests::fixture();
        let ctx = egui::Context::default();
        let original = state.draft.as_ref().expect("draft").edited.params.clone();
        for _ in 0..8 {
            assert!(draw(&ctx, &mut world, &mut state, Vec::new()).is_empty());
        }
        assert_eq!(state.draft.as_ref().expect("draft").edited.params, original);
        assert!(!state.draft.as_ref().expect("draft").dirty());
    }

    #[test]
    fn compact_viewport_keeps_the_first_numeric_value_visible_with_hints() {
        let (mut world, mut state) = crate::shape_editor::tests::fixture();
        state.group = 1;
        let param = catalog::PARAMS
            .iter()
            .filter(|p| p.group == 1 && p.sex & 1 != 0)
            .min_by(|a, b| a.order.total_cmp(&b.order))
            .expect("first head control");
        let weight = state.draft.as_ref().expect("draft").value(param);
        let label = aurora_i18n::number(((weight - param.min) / (param.max - param.min) * 100.0) as f64, 1);
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(800.0, 560.0))),
            ..Default::default()
        };
        let mut output = Default::default();
        for _ in 0..4 {
            output = run_headless(&ctx, input.clone(), |ui| {
                let _ = show(ui.ctx(), &crate::theme::Theme::default().palette(), &mut world, &mut state, true);
            });
        }
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text()==label
            && shape.clip_rect.contains_rect(egui::Rect::from_min_size(text.pos,text.galley.size())))),
            "first value must be visible above the footer"
        );
    }
    #[test]
    fn escape_on_a_dirty_shape_opens_confirmation_and_escape_again_keeps_the_draft() {
        let (mut world, mut state) = crate::shape_editor::tests::fixture();
        let ctx = egui::Context::default();
        state.draft.as_mut().expect("draft").edited.params.insert(33, 2.0);
        state.changed(&mut world);
        draw(&ctx, &mut world, &mut state, Vec::new());
        draw(&ctx, &mut world, &mut state, vec![escape()]);
        assert!(state.close_confirmation && state.open);
        draw(&ctx, &mut world, &mut state, Vec::new());
        draw(&ctx, &mut world, &mut state, vec![escape()]);
        assert!(!state.close_confirmation && state.open);
        assert!(state.draft.as_ref().expect("draft").dirty());
    }
    #[test]
    fn escape_cancels_save_as_without_closing_the_editor() {
        let (mut world, mut state) = crate::shape_editor::tests::fixture();
        let ctx = egui::Context::default();
        state.copy_name = Some("Copy".into());
        draw(&ctx, &mut world, &mut state, Vec::new());
        draw(&ctx, &mut world, &mut state, vec![escape()]);
        assert!(state.open && state.copy_name.is_none());
    }
}
