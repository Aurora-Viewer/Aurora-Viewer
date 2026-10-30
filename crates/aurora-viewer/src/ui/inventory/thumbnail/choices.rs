//! Local-file thumbnail choices. Only an explicit preview click starts an upload.

use super::*;

#[derive(Debug, PartialEq)]
pub(crate) struct FileChoices {
    pub pixels: [Vec<u8>; 2],
    pub crop: image::RgbaImage,
}

enum Source {
    Pixels(Arc<Vec<u8>>),
    Crop(Crop),
}

struct Crop {
    image: Arc<image::RgbaImage>,
    origin: Vec2,
    drag_origin: Vec2,
}

impl Crop {
    fn new(image: image::RgbaImage) -> Self {
        let edge = image.width().min(image.height());
        let origin = egui::vec2(((image.width() - edge) / 2) as f32, ((image.height() - edge) / 2) as f32);
        Self {
            image: Arc::new(image),
            origin,
            drag_origin: origin,
        }
    }

    fn offset(&self) -> [u32; 2] {
        [self.origin.x.round() as u32, self.origin.y.round() as u32]
    }

    fn uv(&self) -> egui::Rect {
        let edge = self.image.width().min(self.image.height()) as f32;
        let size = egui::vec2(self.image.width() as f32, self.image.height() as f32);
        let [x, y] = self.offset();
        egui::Rect::from_min_max(
            egui::pos2(x as f32 / size.x, y as f32 / size.y),
            egui::pos2((x as f32 + edge) / size.x, (y as f32 + edge) / size.y),
        )
    }

    fn pan(&mut self, delta: Vec2, side: f32) {
        let edge = self.image.width().min(self.image.height());
        let limit = egui::vec2((self.image.width() - edge) as f32, (self.image.height() - edge) as f32);
        self.origin = (self.drag_origin - delta * (edge as f32 / side)).clamp(Vec2::ZERO, limit);
    }
}

pub(super) struct Choice {
    texture: egui::TextureHandle,
    source: Source,
}

impl Choice {
    fn input(&self) -> Input {
        match &self.source {
            Source::Pixels(pixels) => Input::Pixels(pixels.clone()),
            Source::Crop(crop) => Input::Crop {
                image: crop.image.clone(),
                origin: crop.offset(),
            },
        }
    }
}

impl ThumbnailUi {
    pub(crate) fn set_file_choices(&mut self, ctx: &egui::Context, choices: FileChoices) {
        self.choices = choices
            .pixels
            .into_iter()
            .enumerate()
            .map(|(index, pixels)| Choice {
                texture: ctx.load_texture(
                    format!("inventory_file_choice_{index}"),
                    egui::ColorImage::from_rgba_unmultiplied([256, 256], &pixels),
                    egui::TextureOptions::LINEAR,
                ),
                source: Source::Pixels(Arc::new(pixels)),
            })
            .collect();
        let crop = Crop::new(choices.crop);
        let texture = ctx.load_texture(
            "inventory_crop_source",
            egui::ColorImage::from_rgba_unmultiplied([crop.image.width() as usize, crop.image.height() as usize], crop.image.as_raw()),
            egui::TextureOptions::LINEAR,
        );
        self.choices.push(Choice {
            texture,
            source: Source::Crop(crop),
        });
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
                    let choice = &mut self.choices[index];
                    ui.vertical_centered(|ui| {
                        let mut image = egui::Image::new((choice.texture.id(), Vec2::splat(side)));
                        let sense = if let Source::Crop(crop) = &choice.source {
                            image = image.uv(crop.uv());
                            egui::Sense::click_and_drag()
                        } else {
                            egui::Sense::click()
                        };
                        let response = ui
                            .add_enabled(ready, egui::Button::new(image).sense(sense))
                            .on_hover_text(tips[index]);
                        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ready, labels[index]));
                        if let Source::Crop(crop) = &mut choice.source {
                            if ready && response.drag_started_by(egui::PointerButton::Primary) {
                                crop.drag_origin = crop.origin;
                            }
                            if ready
                                && response.dragged_by(egui::PointerButton::Primary)
                                && let Some(delta) = response.total_drag_delta()
                            {
                                crop.pan(delta, side);
                                ctx.request_repaint();
                            }
                            if ready {
                                response.clone().on_hover_cursor(if response.dragged() {
                                    egui::CursorIcon::Grabbing
                                } else {
                                    egui::CursorIcon::Grab
                                });
                            }
                        }
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
            let input = self.choices[index].input();
            self.choices.clear();
            self.request(item, input, actions);
        } else if cancel || modal.should_close() {
            self.choices.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choices(width: u32, height: u32) -> FileChoices {
        FileChoices {
            pixels: std::array::from_fn(|index| vec![index as u8 + 40; 256 * 256 * 4]),
            crop: image::RgbaImage::from_pixel(width, height, image::Rgba([42; 4])),
        }
    }

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
                state.set_file_choices(&ctx, choices(256, 256));
                let expected = vec![chosen as u8 + 40; 256 * 256 * 4];
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
                let [InvAction::Thumbnail { item: actual, input, .. }] = &actions[..] else {
                    panic!("one selected upload")
                };
                assert_eq!(*actual, item);
                match input {
                    Input::Pixels(pixels) if chosen < 2 => assert_eq!(pixels.as_ref(), &expected),
                    Input::Crop { image, origin } if chosen == 2 => {
                        assert_eq!(image.as_raw(), &expected);
                        assert_eq!(*origin, [0, 0]);
                    }
                    _ => panic!("chosen image source"),
                }
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
            state.set_file_choices(&ctx, choices(256, 256));
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

    #[test]
    fn dragging_the_crop_changes_its_viewport_without_uploading_and_stays_in_bounds() {
        for target in [8101, 8000] {
            for (width, height) in [(256, 640), (640, 256)] {
                let ctx = egui::Context::default();
                crate::theme::Theme::default().apply(&ctx, 1.0);
                let mut state = ThumbnailUi::default();
                let item = Uuid::from_u128(target);
                state.open(item);
                state.set_file_choices(&ctx, choices(width, height));
                let texture = state.choices[2].texture.id();
                let mut actions = Vec::new();
                for _ in 0..4 {
                    frame(&ctx, &mut state, &mut actions, true, Vec::new());
                }
                let output = frame(&ctx, &mut state, &mut actions, true, Vec::new());
                let rect = output
                    .shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect) if rect.fill_texture_id() == texture => Some(rect.rect),
                        egui::Shape::Mesh(mesh) if mesh.texture_id == texture => Some(mesh.calc_bounds()),
                        _ => None,
                    })
                    .expect("crop image");
                let center = rect.center();
                let edge = width.min(height);
                let limit = [width - edge, height - edge];
                let expected = [(limit[0] / 2).saturating_sub(edge / 4), (limit[1] / 2).saturating_sub(edge / 4)];
                let deltas = [rect.size() / 4.0, Vec2::splat(4000.0), Vec2::splat(-4000.0)];
                let offsets = [expected, [0, 0], limit];
                for (delta, offset) in deltas.into_iter().zip(offsets) {
                    frame(&ctx, &mut state, &mut actions, true, pointer(center, true));
                    for fraction in [0.5, 1.0] {
                        frame(
                            &ctx,
                            &mut state,
                            &mut actions,
                            true,
                            vec![egui::Event::PointerMoved(center + delta * fraction)],
                        );
                    }
                    frame(&ctx, &mut state, &mut actions, true, pointer(center + delta, false));
                    assert!(actions.is_empty(), "releasing a drag must not upload");
                    assert_eq!(state.choices.len(), 3);
                    let Source::Crop(crop) = &state.choices[2].source else {
                        panic!("crop")
                    };
                    assert_eq!(crop.offset(), offset);
                    let uv = crop.uv();
                    assert_eq!(
                        uv.min,
                        egui::pos2(offset[0] as f32 / width as f32, offset[1] as f32 / height as f32)
                    );
                    assert!((uv.size() - egui::vec2(edge as f32 / width as f32, edge as f32 / height as f32)).length() < 1e-6);
                    assert!(state.pending.is_none());
                }
                for pressed in [true, false] {
                    frame(&ctx, &mut state, &mut actions, true, pointer(center, pressed));
                }
                assert!(
                    matches!(&actions[..], [InvAction::Thumbnail { item: actual, input: Input::Crop { origin, .. }, .. }]
                    if *actual == item && *origin == limit)
                );
            }
        }
    }
}
