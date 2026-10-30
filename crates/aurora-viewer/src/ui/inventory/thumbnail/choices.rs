//! Local-file thumbnail choices. Only an explicit preview click starts an upload.

use super::*;

pub(super) struct Choice {
    texture: egui::TextureHandle,
    pixels: Arc<Vec<u8>>,
}

impl ThumbnailUi {
    pub(crate) fn set_file_choices(&mut self, ctx: &egui::Context, pixels: [Vec<u8>; 3]) {
        self.choices = pixels
            .into_iter()
            .enumerate()
            .map(|(index, pixels)| Choice {
                texture: ctx.load_texture(
                    format!("inventory_file_choice_{index}"),
                    egui::ColorImage::from_rgba_unmultiplied([256, 256], &pixels),
                    egui::TextureOptions::LINEAR,
                ),
                pixels: Arc::new(pixels),
            })
            .collect();
    }

    pub(super) fn show_file_choices(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        ui_id: Uuid,
        name: &str,
        ready: bool,
        actions: &mut Vec<InvAction>,
    ) {
        let Some(item) = self.item.filter(|_| !self.choices.is_empty()) else {
            return;
        };
        let labels = [
            aurora_i18n::tr!("inventory-thumbnail-stretch"),
            aurora_i18n::tr!("inventory-thumbnail-fit"),
            aurora_i18n::tr!("inventory-thumbnail-crop"),
        ];
        let tips = [
            aurora_i18n::tr!("inventory-thumbnail-stretch-help"),
            aurora_i18n::tr!("inventory-thumbnail-fit-help"),
            aurora_i18n::tr!("inventory-thumbnail-crop-help"),
        ];
        let mut chosen = None;
        let mut cancel = false;
        let modal = crate::ui::widgets::Modal::new(egui::Id::unique(("inventory_file_choices", ui_id))).show(ctx, |ui| {
            let side = ((ctx.content_rect().width() - 112.0) / 3.0).clamp(64.0, 176.0);
            ui.set_width((side + 2.0 * ui.spacing().button_padding.x) * 3.0 + 2.0 * ui.spacing().item_spacing.x);
            ui.label(
                RichText::new(aurora_i18n::tr!("inventory-thumbnail-choose-layout"))
                    .strong()
                    .color(p.ink),
            );
            ui.label(RichText::new(name).color(p.muted));
            ui.label(aurora_i18n::tr!("inventory-thumbnail-choose-layout-help"));
            ui.add_space(8.0);
            ui.columns(3, |columns| {
                for (index, ui) in columns.iter_mut().enumerate() {
                    let choice = &self.choices[index];
                    ui.vertical_centered(|ui| {
                        let image = egui::Image::new((choice.texture.id(), Vec2::splat(side)));
                        let response = ui.add_enabled(ready, egui::Button::new(image)).on_hover_text(tips[index]);
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ready, labels[index]));
                        if response.clicked() {
                            chosen = Some(index);
                        }
                        ui.label(labels[index]);
                    });
                }
            });
            ui.add_space(8.0);
            cancel = flat_button(ui, p, aurora_i18n::tr!("common-cancel")).clicked();
        });
        if let Some(index) = chosen {
            let pixels = self.choices[index].pixels.clone();
            self.choices.clear();
            self.request(item, Input::Pixels(pixels), actions);
        } else if cancel || modal.should_close() {
            self.choices.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        state: &mut ThumbnailUi,
        actions: &mut Vec<InvAction>,
        ready: bool,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let theme = crate::theme::Theme::default();
        crate::ui::inventory::headless_output(ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1100.0, 720.0))),
                events,
                ..Default::default()
            },
            |_ui| state.show_file_choices(ctx, &theme.palette(), Uuid::nil(), "Target inventory entry", ready, actions),
        ))
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn thumbnail_preview_click_uploads_only_the_chosen_pixels_for_items_and_folders() {
        for target in [8101, 8000] {
            for chosen in 0..3 {
                let ctx = egui::Context::default();
                crate::theme::Theme::default().apply(&ctx, 1.0);
                let mut state = ThumbnailUi::default();
                let item = Uuid::from_u128(target);
                state.open(item);
                state.set_file_choices(&ctx, std::array::from_fn(|index| vec![index as u8 + 40; 256 * 256 * 4]));
                let expected = state.choices[chosen].pixels.clone();
                let texture = state.choices[chosen].texture.id();
                let textures: Vec<_> = state.choices.iter().map(|choice| choice.texture.id()).collect();
                let mut actions = Vec::new();
                for _ in 0..4 {
                    frame(&ctx, &mut state, &mut actions, true, Vec::new());
                }
                let output = frame(&ctx, &mut state, &mut actions, true, Vec::new());
                assert!(actions.is_empty(), "preparing/opening previews never uploads");
                let rectangles: Vec<_> = textures
                    .iter()
                    .map(|texture| {
                        output
                            .shapes
                            .iter()
                            .find_map(|shape| match &shape.shape {
                                egui::Shape::Mesh(mesh) if mesh.texture_id == *texture => Some(mesh.calc_bounds()),
                                egui::Shape::Rect(rect) if rect.fill_texture_id() == *texture => Some(rect.rect),
                                _ => None,
                            })
                            .expect("preview rectangle")
                    })
                    .collect();
                for pair in rectangles.windows(2) {
                    assert!(pair[0].right() < pair[1].left(), "thumbnail previews must never overlap: {pair:?}");
                    assert_eq!(pair[0].top(), pair[1].top());
                }
                let rect = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.calc_bounds()),
                        egui::Shape::Rect(rect) if rect.fill_texture_id() == texture => Some(rect.rect),
                        _ => None,
                    })
                    .expect("clickable preview image");
                assert!(rect.width() > 150.0 && rect.height() > 150.0, "previews must not be icon-sized");
                for pressed in [true, false] {
                    frame(&ctx, &mut state, &mut actions, true, pointer(rect.center(), pressed));
                }
                assert!(
                    matches!(&actions[..], [InvAction::Thumbnail { item: actual, input: Input::Pixels(pixels), .. }]
                    if *actual == item && Arc::ptr_eq(pixels, &expected))
                );
                assert!(state.pending.is_some());
                assert!(state.choices.is_empty());
                assert_eq!(state.item, Some(item));
            }
        }
    }

    #[test]
    fn cancelling_file_previews_or_losing_edit_permission_never_uploads() {
        for escape in [false, true] {
            let ctx = egui::Context::default();
            crate::theme::Theme::default().apply(&ctx, 1.0);
            let mut state = ThumbnailUi::default();
            state.open(Uuid::from_u128(8000));
            state.set_file_choices(&ctx, std::array::from_fn(|_| vec![128; 256 * 256 * 4]));
            let texture = state.choices[0].texture.id();
            let mut actions = Vec::new();
            for _ in 0..4 {
                frame(&ctx, &mut state, &mut actions, false, Vec::new());
            }
            let output = frame(&ctx, &mut state, &mut actions, false, Vec::new());
            let rect = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.calc_bounds()),
                    egui::Shape::Rect(rect) if rect.fill_texture_id() == texture => Some(rect.rect),
                    _ => None,
                })
                .expect("disabled preview");
            for pressed in [true, false] {
                frame(&ctx, &mut state, &mut actions, false, pointer(rect.center(), pressed));
            }
            assert_eq!(state.choices.len(), 3);
            if escape {
                frame(
                    &ctx,
                    &mut state,
                    &mut actions,
                    false,
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::NONE,
                    }],
                );
            } else {
                let pos = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text() == aurora_i18n::tr!("common-cancel") => {
                            Some(text.pos + egui::vec2(4.0, 4.0))
                        }
                        _ => None,
                    })
                    .expect("Cancel button");
                for pressed in [true, false] {
                    frame(&ctx, &mut state, &mut actions, false, pointer(pos, pressed));
                }
            }
            assert!(state.choices.is_empty());
            assert!(state.pending.is_none());
            assert!(actions.is_empty());
        }
    }
}
