//! LLPanelEditWearable: native texture, tint and visual-parameter controls
//! shared by skin, hair, eyes and the system clothing layers.
use super::widgets::{self, ActionKind};
use crate::{
    theme::Palette,
    wearable_editor::{self as editor, State},
    world::World,
};
use egui::{RichText, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

pub enum Action {
    Save(Option<String>),
}

fn group_label(group: &str) -> &'static str {
    match group {
        "skin_color" => aurora_i18n::tr!("wearable-group-skin-color"),
        "skin_facedetail" => aurora_i18n::tr!("wearable-group-skin-face"),
        "skin_makeup" => aurora_i18n::tr!("wearable-group-skin-makeup"),
        "skin_bodydetail" => aurora_i18n::tr!("wearable-group-skin-body"),
        "hair_color" => aurora_i18n::tr!("wearable-group-hair-color"),
        "hair_style" => aurora_i18n::tr!("wearable-group-hair-style"),
        "hair_eyebrows" => aurora_i18n::tr!("wearable-group-hair-eyebrows"),
        "hair_facial" => aurora_i18n::tr!("wearable-group-hair-facial"),
        _ => aurora_i18n::tr!("wearable-parameters"),
    }
}

pub fn show(
    ctx: &egui::Context,
    p: &Palette,
    world: &mut World,
    st: &mut State,
    images: &HashMap<Uuid, egui::TextureHandle>,
) -> Vec<Action> {
    if !st.open {
        return Vec::new();
    }
    let mut actions = Vec::new();
    let mut open = true;
    let mut changed = false;
    let mut cancel = false;
    let mut choose = None;
    let mut floater = widgets::Floater::new(
        "wearable_editor",
        aurora_i18n::tr!("wearable-title"),
        egui::pos2(28.0, 64.0),
        Vec2::new(420.0, 700.0),
    );
    floater.min_size = Vec2::new(340.0, 380.0);
    floater.show(ctx, p, &mut open, |ui| {
        let male = crate::physics_editor::male(world);
        let Some(draft) = &mut st.draft else {
            if st.loading.is_some() {
                ui.label(aurora_i18n::tr!("wearable-loading"));
            }
            if !st.message.is_empty() {
                ui.label(RichText::new(&st.message).color(p.warn));
            }
            return;
        };
        let busy = draft.pending.is_some();
        let modify = draft.can_modify(world);
        let copy = draft.can_copy(world) && (modify || !draft.dirty());
        ui.label(RichText::new(&draft.item.name).strong().size(16.0));
        if !modify {
            ui.label(RichText::new(aurora_i18n::tr!("wearable-readonly")).color(p.warn));
        }
        if !st.message.is_empty() {
            ui.label(RichText::new(&st.message).color(p.warn));
        }
        if st.preview.error {
            ui.label(RichText::new(aurora_i18n::tr!("wearable-preview-unavailable")).color(p.warn));
        }
        let rest = ui.available_rect_before_wrap();
        ui.allocate_rect(rest, egui::Sense::hover());
        let mut footer = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rest)
                .layout(egui::Layout::bottom_up(egui::Align::Min)),
        );
        if busy {
            footer.label(aurora_i18n::tr!("wearable-saving"));
        }
        footer.add_enabled_ui(!busy, |ui| {
            ui.horizontal(|ui| {
                let size = Vec2::new((ui.available_width() - ui.spacing().item_spacing.x * 2.0) / 3.0, 30.0);
                let save = widgets::action_button_if(
                    ui,
                    p,
                    "copy-filled",
                    aurora_i18n::tr!("common-save-as"),
                    ActionKind::Flat,
                    size,
                    copy,
                );
                if save.clicked() {
                    st.copy_name = Some(draft.item.name.clone());
                }
                if widgets::action_button_if(ui, p, "restore", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size, true).clicked() {
                    cancel = true;
                }
                let save = widgets::action_button_if(
                    ui,
                    p,
                    "device-floppy",
                    aurora_i18n::tr!("common-save"),
                    ActionKind::Primary,
                    size,
                    modify && draft.dirty(),
                );
                if cfg!(test) {
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(egui::Id::unique("wearable_save_button"), save.rect));
                }
                if save.clicked() {
                    actions.push(Action::Save(None));
                }
            });
        });
        footer.add_space(8.0);
        let mut controls = footer.new_child(
            egui::UiBuilder::new()
                .max_rect(footer.available_rect_before_wrap())
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(&mut controls, |ui| {
            ui.add_enabled_ui(modify && !busy, |ui| {
                if let Some(channels) = editor::color_params(draft.edited.kind) {
                    let mut color: [u8; 4] = [0, 0, 0, 255];
                    for (value, id) in color.iter_mut().zip(channels) {
                        *value = (draft.edited.params.get(&id).copied().unwrap_or(1.0).clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                    ui.horizontal(|ui| {
                        ui.label(aurora_i18n::tr!("wearable-tint"));
                        if widgets::color_button(ui, p, "wearable_tint", &mut color, false).changed() {
                            for (value, id) in color.into_iter().zip(channels) {
                                draft.edited.params.insert(id, f32::from(value) / 255.0);
                            }
                            changed = true;
                        }
                    });
                }
                widgets::section_header(ui, p, "photo-filled", aurora_i18n::tr!("wearable-textures"));
                for &slot in editor::slots(draft.edited.kind) {
                    let texture = draft.edited.textures.get(&slot).copied().unwrap_or_default();
                    if !texture.is_nil() {
                        st.wanted_images.insert(texture);
                    }
                    ui.horizontal(|ui| {
                        if let Some(image) = images.get(&texture) {
                            if ui
                                .add(egui::Button::image(egui::Image::new(image).fit_to_exact_size(Vec2::splat(48.0))))
                                .clicked()
                            {
                                choose = Some((slot, texture));
                            }
                        } else if widgets::icon_button(ui, p, "photo-filled", aurora_i18n::tr!("wearable-choose-texture"), true) {
                            choose = Some((slot, texture));
                        }
                        if ui.button(editor::slot_label(slot)).clicked() {
                            choose = Some((slot, texture));
                        }
                        if draft.edited.kind == 13 {
                            let mut invisible = texture == crate::scene::avatar::IMG_INVISIBLE;
                            if widgets::checkbox(ui, p, &mut invisible, aurora_i18n::tr!("wearable-invisible")).changed() {
                                if invisible {
                                    st.alpha_previous.insert(slot, texture);
                                }
                                draft.edited.textures.insert(
                                    slot,
                                    if invisible {
                                        crate::scene::avatar::IMG_INVISIBLE
                                    } else {
                                        st.alpha_previous.remove(&slot).unwrap_or(aurora_prim::te::BLANK_TEXTURE)
                                    },
                                );
                                changed = true;
                            }
                        }
                    });
                }
                let sex = if male { 2 } else { 1 };
                let mut groups = Vec::new();
                for param in editor::catalog::PARAMS
                    .iter()
                    .filter(|param| param.kind == draft.edited.kind && param.sex & sex != 0)
                {
                    if !groups.contains(&param.group) {
                        groups.push(param.group);
                    }
                }
                for group in groups {
                    let response = widgets::collapsing_forced(
                        ui,
                        p,
                        ("wearable_group", group),
                        group_label(group),
                        false,
                        Some(st.group == group),
                        |ui| {
                            let mut params: Vec<_> = editor::catalog::PARAMS
                                .iter()
                                .filter(|param| param.kind == draft.edited.kind && param.group == group && param.sex & sex != 0)
                                .collect();
                            params.sort_by(|a, b| a.order.total_cmp(&b.order).then(a.id.cmp(&b.id)));
                            for param in params {
                                let value = draft
                                    .edited
                                    .params
                                    .get(&param.id)
                                    .copied()
                                    .unwrap_or(param.default)
                                    .clamp(param.min, param.max);
                                let mut percent = if param.max > param.min {
                                    (value - param.min) / (param.max - param.min) * 100.0
                                } else {
                                    0.0
                                };
                                ui.label(param.label());
                                if ui.add(egui::Slider::new(&mut percent, 0.0..=100.0).fixed_decimals(0)).changed() {
                                    draft
                                        .edited
                                        .params
                                        .insert(param.id, param.min + percent / 100.0 * (param.max - param.min));
                                    changed = true;
                                }
                            }
                        },
                    );
                    if response.header.clicked() {
                        st.group = if st.group == group { String::new() } else { group.into() };
                    }
                }
            });
        });
    });
    if let Some((slot, texture)) = choose {
        st.picker_slot = Some(slot);
        let kind = st.draft.as_ref().map_or(0, |draft| draft.edited.kind);
        st.picker.open_with_defaults(
            aurora_i18n::tr!("wearable-choose-texture"),
            texture,
            None,
            matches!(kind, 1 | 13 | 14 | 16),
        );
        st.picker.allow_local = false;
    }
    if let Some(texture) = st.picker.show(ctx, p, world, images)
        && let Some(slot) = st.picker_slot.take()
    {
        changed |= st.set_texture(world, slot, texture);
    }
    st.wanted_images.extend(st.picker.wanted_images.drain());
    if changed {
        st.changed(world);
    }
    if cancel {
        st.close(world);
        return actions;
    }
    if !open {
        if st.draft.as_ref().is_some_and(|draft| draft.pending.is_some()) {
            return actions;
        }
        if st.draft.as_ref().is_some_and(|draft| draft.dirty()) {
            st.close_confirmation = true;
        } else {
            st.close(world);
        }
    }
    if let Some(name) = &mut st.copy_name {
        let mut accepted = false;
        let mut dismissed = false;
        widgets::Modal::new(egui::Id::unique("wearable_save_as")).show(ctx, |ui| {
            ui.label(aurora_i18n::tr!("common-save-as"));
            ui.text_edit_singleline(name);
            ui.horizontal(|ui| {
                accepted = ui
                    .add_enabled(!name.trim().is_empty(), egui::Button::new(aurora_i18n::tr!("common-save")))
                    .clicked();
                dismissed = ui.button(aurora_i18n::tr!("common-cancel")).clicked();
            });
        });
        if accepted {
            actions.push(Action::Save(st.copy_name.take()));
        }
        if dismissed {
            st.copy_name = None;
        }
    }
    if st.close_confirmation {
        let mut discard = false;
        widgets::Modal::new(egui::Id::unique("wearable_unsaved")).show(ctx, |ui| {
            ui.label(aurora_i18n::tr!("wearable-unsaved"));
            ui.horizontal(|ui| {
                if ui.button(aurora_i18n::tr!("common-save")).clicked() {
                    st.close_after_save = true;
                    st.close_confirmation = false;
                    actions.push(Action::Save(None));
                }
                if ui.button(aurora_i18n::tr!("wearable-discard")).clicked() {
                    discard = true;
                }
                if ui.button(aurora_i18n::tr!("common-cancel")).clicked() {
                    st.close_confirmation = false;
                }
            });
        });
        if discard {
            st.close(world);
        }
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(ctx: &egui::Context, world: &mut World, state: &mut State, events: Vec<egui::Event>) -> Vec<Action> {
        let mut actions = Vec::new();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1280.0, 900.0))),
                events,
                ..Default::default()
            },
            |ui| {
                actions = show(ui.ctx(), &crate::theme::Theme::default().palette(), world, state, &HashMap::new());
            },
        );
        output.textures_delta.clear();
        actions
    }

    #[test]
    fn all_wearable_editors_keep_the_save_button_reachable_and_click_only_when_modifiable() {
        for scale in [1.0, 1.5] {
            for kind in (1..=14).chain(std::iter::once(16)) {
                let (mut world, mut state, id) = crate::wearable_editor::tests::fixture(kind);
                state
                    .draft
                    .as_mut()
                    .expect("draft")
                    .edited
                    .textures
                    .insert(editor::slots(kind)[0], Uuid::from_u128(777));
                let ctx = egui::Context::default();
                ctx.set_zoom_factor(scale);
                for _ in 0..3 {
                    assert!(frame(&ctx, &mut world, &mut state, Vec::new()).is_empty());
                }
                let rect = ctx
                    .data_mut(|data| data.get_temp::<egui::Rect>(egui::Id::unique("wearable_save_button")))
                    .expect("save button");
                assert!(
                    ctx.content_rect().contains_rect(rect),
                    "save outside the screen at scale {scale}, type {kind}"
                );
                let click = |pressed| {
                    vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: Default::default(),
                        },
                    ]
                };
                assert!(frame(&ctx, &mut world, &mut state, click(true)).is_empty());
                assert!(matches!(
                    &frame(&ctx, &mut world, &mut state, click(false))[..],
                    [Action::Save(None)]
                ));
                world.inventory.items.get_mut(&id).expect("item").owner_mask &= !crate::world::inventory::actions::MODIFY;
                let _ = frame(&ctx, &mut world, &mut state, Vec::new());
                assert!(frame(&ctx, &mut world, &mut state, click(true)).is_empty());
                assert!(frame(&ctx, &mut world, &mut state, click(false)).is_empty());
            }
        }
    }
}
