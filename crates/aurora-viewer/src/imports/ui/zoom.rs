//! Large previews share the draft camera and its current render settings.
use super::*;

pub(super) fn show(ctx: &egui::Context, p: &Palette, imports: &mut Imports, actions: &mut Vec<Action>) {
    if imports.demo_started
        && let Some(target) = ctx.data_mut(|data| data.get_temp::<(Uuid, Option<usize>)>(egui::Id::unique("import_demo_zoom")))
        && imports.entries.iter().any(|entry| entry.id == target.0)
    {
        ctx.data_mut(|data| data.remove_temp::<(Uuid, Option<usize>)>(egui::Id::unique("import_demo_zoom")));
        imports.preview_zoom = Some(target);
        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("import_large_preview")));
    }
    // The shared loupe also belongs to independent type windows. The unified
    // queue can be closed while a Mesh or Material inspector is still open.
    if !imports.any_window_open() {
        imports.preview_zoom = None;
    }
    // One enlarged map at a time; changing/closing the loupe frees its GPU handle.
    for entry in &mut imports.entries {
        for (slot, cached) in entry.material_large_previews.iter_mut().enumerate() {
            if imports.preview_zoom != Some((entry.id, Some(slot))) {
                *cached = None;
            }
        }
    }
    let Some((id, slot)) = imports.preview_zoom else { return };
    let Some(entry) = imports.entries.iter_mut().find(|entry| entry.id == id) else {
        imports.preview_zoom = None;
        return;
    };
    let mut open = true;
    let mut window = Floater::new(
        "import_large_preview",
        aurora_i18n::tr!("import-preview-enlarged", name = entry.name.clone()),
        egui::pos2(180.0, 100.0),
        Vec2::new(760.0, 590.0),
    );
    window.min_size = Vec2::new(420.0, 360.0);
    window.show(ctx, p, &mut open, |ui| {
        // the preview in exactly the room the line or the controls under it
        // leave
        if let Some(slot) = slot.filter(|slot| *slot < 4) {
            let key = match &entry.source {
                Some(Source::Material { images, .. }) => images[slot].as_ref().map(|image| image.key),
                _ => None,
            };
            let preview = entry.material_large_previews[slot]
                .as_ref()
                .filter(|(stored, generation, _)| Some(*stored) == key && *generation == entry.preview_generation);
            let pending = entry.material_large_pending[slot];
            let (mut area, foot) = widgets::above_footer(ui);
            let square = area.available_size().min_elem();
            area.vertical_centered(|ui| {
                let response = widgets::picture(
                    ui,
                    p,
                    preview.map(|(_, _, texture)| texture),
                    Vec2::splat(square),
                    "photo-filled",
                    "",
                );
                if matches!(&entry.source, Some(Source::Material { images, .. }) if images[slot].as_ref().is_some_and(|image| image.asset.is_none())) {
                    widgets::local_texture::paint(ui, p, response.rect);
                }
            });
            if preview.is_none() && key.is_some() && !entry.material_large_pending[slot] {
                actions.push(Action::LargeMap(id, slot));
            }
            foot.show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.horizontal(|ui| {
                        if pending {
                            ui.spinner();
                        }
                        ui.label(map_label(slot));
                    });
                });
            });
        } else {
            let (mut area, foot) = widgets::above_footer(ui);
            let size = area.available_size();
            if let Some(model) = &entry.model {
                let pose = if matches!(entry.source, Some(Source::Mesh(_))) {
                    entry.mesh_player.pose.clone()
                } else {
                    entry.player.pose.clone()
                };
                let response = aurora_render::preview::paint_with_pose(
                    &mut area,
                    egui::Id::unique(("large_import_preview", id)),
                    model.clone(),
                    &mut entry.view,
                    size,
                    pose,
                );
                if matches!(entry.source, Some(Source::Texture(_) | Source::Material { .. })) {
                    widgets::local_texture::paint(&area, p, response.rect);
                }
            } else {
                widgets::picture(&mut area, p, None, size, "cube-3d-sphere", "");
            }
            foot.show(ui, |ui| {
                camera_hint(ui, p);
                preview_camera_controls(ui, p, &mut entry.view);
            });
        }
    });
    if !open || ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        imports.preview_zoom = None;
        entry.material_large_previews = std::array::from_fn(|_| None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_zoom_loupe_opens_from_independent_mesh_and_local_windows() {
        let world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for choice in [Choice::Mesh, Choice::Local] {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            let mut entry = Entry::new(
                "preview.dae".into(),
                Ok(Source::Mesh(Box::new(crate::imports::tests::mesh_fixture()))),
                Default::default(),
            );
            entry.local = choice == Choice::Local;
            let id = entry.id;
            let title = aurora_i18n::tr!("import-preview-enlarged", name = entry.name.clone());
            let mut imports = Imports {
                entries: vec![entry],
                ..Default::default()
            };
            imports.open_type_window(&ctx, choice);
            let frame = |imports: &mut Imports, events| {
                let mut actions = Vec::new();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1440.0, 1000.0))),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        actions = crate::imports::ui::show(
                            ui.ctx(),
                            &theme.palette(),
                            imports,
                            &mut Preferences::default(),
                            &world,
                            &Default::default(),
                        );
                    },
                );
                output.textures_delta.clear();
                (output, actions)
            };
            for _ in 0..4 {
                frame(&mut imports, vec![]);
            }
            assert!(
                !imports.open && imports.window_open(choice),
                "only the independent importer is open"
            );
            let (rect, clip) = ctx
                .data(|data| data.get_temp::<(egui::Rect, egui::Rect)>(egui::Id::unique(("import_mesh_zoom_action", id))))
                .expect("loupe hit target");
            assert!(rect.width() >= 18.0 && rect.height() >= 18.0 && clip.contains_rect(rect));
            let mut targets = Vec::new();
            for pressed in [true, false] {
                let (_, actions) = frame(
                    &mut imports,
                    vec![
                        egui::Event::PointerMoved(rect.center()),
                        egui::Event::PointerButton {
                            pos: rect.center(),
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
                for action in actions {
                    if let Action::Zoom(target, slot) = action {
                        targets.push((target, slot));
                        imports.preview_zoom = Some((target, slot));
                        ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique("import_large_preview")));
                    }
                }
            }
            assert_eq!(targets, vec![(id, None)]);
            let mut visible = false;
            for _ in 0..4 {
                let (output, _) = frame(&mut imports, vec![]);
                assert_eq!(
                    imports.preview_zoom,
                    Some((id, None)),
                    "the unified queue must not close another window's preview"
                );
                visible |= output
                    .shapes
                    .iter()
                    .any(|shape| matches!(&shape.shape, egui::epaint::Shape::Text(text) if text.galley.text() == title));
            }
            assert!(visible, "the separate enlarged-preview window is painted");
            imports.windows[choice.window_index().unwrap()].open = false;
            frame(&mut imports, vec![]);
            assert!(
                imports.preview_zoom.is_none(),
                "closing the last importer still closes its enlarged preview"
            );
        }
    }

    #[test]
    fn import_zoom_material_map_survives_with_only_its_type_window_and_releases_on_close() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let image = crate::imports::tests::image(4, 4);
        let key = Uuid::new_v4();
        let mut images = Box::new(std::array::from_fn(|_| None));
        images[0] = Some(LocalImage {
            path: None,
            key,
            name: "Base".into(),
            pixels: image,
            asset: None,
            empty_alpha: false,
        });
        let entry = Entry::new(
            "material.gltf".into(),
            Ok(Source::Material {
                value: Default::default(),
                images,
            }),
            Default::default(),
        );
        let id = entry.id;
        let mut imports = Imports {
            entries: vec![entry],
            ..Default::default()
        };
        imports.open_type_window(&ctx, Choice::Material);
        imports.preview_zoom = Some((id, Some(0)));
        imports.entries[0].material_large_previews[0] = Some((
            key,
            0,
            ctx.load_texture(
                "large map",
                egui::ColorImage::filled([4, 4], egui::Color32::TRANSPARENT),
                Default::default(),
            ),
        ));
        let frame = |imports: &mut Imports, events| {
            let mut actions = Vec::new();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1200.0, 900.0))),
                    events,
                    ..Default::default()
                },
                |ui| {
                    show(ui.ctx(), &theme.palette(), imports, &mut actions);
                },
            );
            output.textures_delta.clear();
            actions
        };
        for _ in 0..3 {
            assert!(
                frame(&mut imports, vec![]).is_empty(),
                "the ready map needs no new large-image request"
            );
            assert_eq!(imports.preview_zoom, Some((id, Some(0))));
            assert!(imports.entries[0].material_large_previews[0].is_some());
        }
        frame(
            &mut imports,
            vec![egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(imports.window_open(Choice::Material) && imports.preview_zoom.is_none());
        assert!(imports.entries[0].material_large_previews[0].is_none());
        imports.preview_zoom = Some((id, Some(0)));
        imports.entries[0].material_large_previews[0] = Some((
            key,
            0,
            ctx.load_texture(
                "reopened map",
                egui::ColorImage::filled([4, 4], egui::Color32::TRANSPARENT),
                Default::default(),
            ),
        ));
        imports.windows[Choice::Material.window_index().unwrap()].open = false;
        frame(&mut imports, vec![]);
        assert!(!imports.any_window_open() && imports.preview_zoom.is_none());
        assert!(imports.entries[0].material_large_previews[0].is_none());
    }
}
