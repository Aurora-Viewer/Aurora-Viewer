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
    if !imports.open {
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
                widgets::picture(
                    ui,
                    p,
                    preview.map(|(_, _, texture)| texture),
                    Vec2::splat(square),
                    "photo-filled",
                    "",
                );
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
                aurora_render::preview::paint_with_pose(
                    &mut area,
                    egui::Id::unique(("large_import_preview", id)),
                    model.clone(),
                    &mut entry.view,
                    size,
                    pose,
                );
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
