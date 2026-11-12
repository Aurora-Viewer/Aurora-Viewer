//! Library lifecycle and world drops. Behaviour follows Firestorm
//! LLToolDragAndDrop::{dad3dApplyToObject,handleDropMaterialProtections,
//! dropTextureOneFace,dropMaterialOneFace} (LGPL 2.1): Shift all faces,
//! Ctrl contents, restricted assets retained in the object's inventory.

use super::*;
use crate::build::contents_drop;
use crate::build::texture_copy::{requires_contents, selection_copy_denied};
use crate::ui::texture_library::{self as library, catalogue};
use crate::world::inventory::actions as rules;
use aurora_net::build::BuildCmd;
use uuid::Uuid;

#[derive(Default)]
pub(super) struct DragState {
    candidates: Vec<usize>,
    ray: Option<(Vec3, Vec3)>,
    checked: Option<Instant>,
    target: Option<(usize, u8)>,
    /// What the CPU search stops at, the avatars' boxes aside: bounds what
    /// the faces drawn may designate.
    found: Option<Vec3>,
    /// The faces drawn under the pointer, asked at the pace of the surface
    /// search, and the ray they were asked for.
    drawn: Option<(u64, (Vec3, Vec3))>,
    /// A face of what we wear under the pointer, from the last answer.
    worn: Option<(usize, u8)>,
    pub highlighted: Vec<(usize, u64)>,
    /// A drop waiting for the no-copy confirmation, and its object.
    confirm: Option<(Drop, Uuid)>,
}
impl DragState {
    fn end_worn_search(&mut self, renderer: &mut Renderer) {
        if let Some((ticket, _)) = self.drawn.take() {
            renderer.forget_drawn(ticket);
        }
        self.worn = None;
    }
}

pub(super) struct Drop {
    item: Uuid,
    object: usize,
    face: u8,
    all: bool,
    /// With `all`: every prim of the worn linkset, not only the one hit.
    linkset: bool,
    /// The no-copy confirmation was answered.
    confirmed: bool,
    contents: bool,
    remove_pbr: bool,
}

/// Ctrl+Shift over what we wear: every face of its linkset. Ctrl alone would
/// go into a worn object's contents, which it refuses, so nothing is lost.
/// A **wanted difference** from dropTextureAllFaces, which stops at the prim
/// hit, as the pieces of a worn mesh often have a single face.
fn linkset_drop(worn: bool, ctrl: bool, shift: bool) -> bool {
    worn && ctrl && shift
}
/// The keys of a drop on what we wear, one line each; what is held stands out.
fn worn_drop_keys(ui: &mut egui::Ui, p: &crate::theme::Palette, shift: bool, linkset: bool) {
    let shift_key = || crate::keybinds::Binding {
        input: crate::keybinds::Input::key(winit::keyboard::KeyCode::ShiftLeft),
        ctrl: false,
        shift: false,
        alt: false,
    };
    ui::widgets::keycap::legend(
        ui,
        p,
        &[
            (
                shift_key(),
                aurora_i18n::tr!("texture-library-worn-drop-all-faces"),
                shift && !linkset,
            ),
            (
                crate::keybinds::Binding { ctrl: true, ..shift_key() },
                aurora_i18n::tr!("texture-library-worn-drop-linked"),
                linkset,
            ),
        ],
    );
}
/// The faces of `prim` a drop on `face` covers: all of them with Shift.
fn drop_faces(world: &World, prim: usize, face: u8, all: bool) -> Vec<u8> {
    if !all {
        return vec![face];
    }
    let count = world
        .objects
        .get(prim)
        .and_then(|o| o.te.as_ref())
        .map_or(0, |te| te.faces.len().min(256));
    (0..count).map(|i| i as u8).collect()
}
/// The prims a drop on `idx` covers: its linkset with `linkset_drop`.
fn drop_prims(world: &World, idx: usize, linkset: bool) -> Vec<usize> {
    let root = contents_drop::linkset(world, idx).filter(|_| linkset);
    root.map_or_else(|| vec![idx], |(root, _)| crate::build::family(world, root))
}
/// Whether a drop moves `item` out of the inventory for good: a no-copy item
/// the object does not hold yet (handleDropMaterialProtections). The asset of
/// the library or of the object's contents costs nothing.
fn no_copy_leaves(inv: &crate::world::inventory::Inventory, item: &aurora_net::inventory::InvItem, agent: Uuid, retained: bool) -> bool {
    !retained && !rules::library(inv, item.id) && rules::agent_mask(item, agent) & rules::COPY == 0
}

impl App {
    pub(super) fn record_library_use(&mut self, id: Uuid) {
        if !self.demo && ui::texture_library::catalogue::contains(&self.world.inventory, id) {
            self.settings.texture_library.activate(self.world.agent_id);
            self.settings.texture_library.record(id);
            self.settings.save();
        }
    }
    pub(super) fn record_library_asset(&mut self, asset: Uuid, kind: i32) {
        let inv = &self.world.inventory;
        let matches = |item: &&aurora_net::inventory::InvItem| {
            item.asset_type == kind && item.asset_id == asset && ui::texture_library::catalogue::contains(inv, item.id)
        };
        let item = self
            .inventory_ui
            .preview
            .and_then(|id| inv.items.get(&id))
            .filter(matches)
            .map(|it| it.id)
            .or_else(|| inv.items.values().filter(matches).map(|it| it.id).min());
        if let Some(id) = item {
            self.record_library_use(id);
        }
    }

    pub(super) fn texture_library_demo_input(&self, raw: &mut egui::RawInput) {
        let Ok(mode) = std::env::var("AURORA_DEMO_TEXTURE_LIBRARY") else {
            return;
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        if mode.starts_with("face-") {
            let Some(idx) = self.world.objects.index_of_uuid(&crate::demo::action_id(970)) else {
                return;
            };
            let Some(target) = self.world.objects.get(idx).and_then(|o| self.build.cam.project(o.position)) else {
                return;
            };
            let item = if mode.contains("material") {
                crate::demo::texture_library::preview_items()[2]
            } else {
                crate::demo::texture_library::first_texture()
            };
            let modifiers = egui::Modifiers {
                shift: mode.contains("all"),
                ctrl: mode.contains("contents"),
                ..Default::default()
            };
            raw.events.push(egui::Event::ModifiersChanged(modifiers));
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            };
            match self.frame_count {
                650 => {
                    egui::DragAndDrop::set_payload(&self.egui_ctx, ui::inventory::InvDrag(item));
                    raw.events.extend([egui::Event::PointerMoved(target), button(target, true)]);
                }
                651..=729 => raw.events.push(egui::Event::PointerMoved(target)),
                730 if mode.contains("drop") => raw.events.push(button(target, false)),
                730 if mode.contains("cancel") => raw.events.push(egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }),
                730 if mode.contains("ui") => {
                    if let Some(point) = self
                        .egui_ctx
                        .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("texture_library_first_card")))
                    {
                        raw.events.push(egui::Event::PointerMoved(point));
                    }
                }
                _ => {}
            }
            if matches!(self.frame_count, 700 | 750) {
                log::info!("demo face drop: mode={mode}, highlighted={:?}", self.library_drag.highlighted);
            }
            return;
        }
        if mode.starts_with("types") {
            if let Some(point) = self
                .egui_ctx
                .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("library_type_selector")))
            {
                match self.frame_count {
                    650 => raw.events.extend([egui::Event::PointerMoved(point), button(point, true)]),
                    651 => raw.events.push(button(point, false)),
                    _ => {}
                }
            }
            return;
        }
        if mode == "preview-sizes" {
            let (item, start, delta) = if self.frame_count < 690 {
                (crate::demo::texture_library::preview_items()[0], 640, egui::vec2(-80.0, -130.0))
            } else {
                (crate::demo::texture_library::preview_items()[2], 710, egui::vec2(-20.0, -100.0))
            };
            if let Some(rect) = self
                .egui_ctx
                .memory(|m| m.area_rect(egui::Id::unique(library::preview_window_id(item))))
            {
                let source = rect.right_bottom() - egui::Vec2::splat(2.0);
                let origin_key = egui::Id::unique(("library_demo_resize_origin", item));
                if self.frame_count == start {
                    self.egui_ctx.data_mut(|d| d.insert_temp(origin_key, source));
                    raw.events.push(egui::Event::PointerMoved(source));
                }
                if let Some(origin) = self.egui_ctx.data(|d| d.get_temp::<egui::Pos2>(origin_key)) {
                    let target = origin + delta;
                    match self.frame_count.saturating_sub(start) {
                        1 if self.frame_count >= start => raw.events.push(button(origin, true)),
                        2..=5 if self.frame_count >= start => raw.events.push(egui::Event::PointerMoved(target)),
                        6 if self.frame_count >= start => raw.events.push(button(target, false)),
                        _ => {}
                    }
                }
            }
            return;
        }
        if mode == "preview-controls-drop" {
            // The texture inspector's image dragged onto the material's empty metallic-roughness map (#1279).
            let [texture, _, material, _] = crate::demo::texture_library::preview_items();
            let source = self
                .egui_ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("resource_texture_preview", texture))));
            let target = self.egui_ctx.data(|d| {
                d.get_temp::<(Option<Uuid>, Option<egui::TextureId>, egui::Rect)>(egui::Id::unique((
                    "resource_material_map",
                    material,
                    2usize,
                )))
            });
            if let (Some(source), Some((_, _, target))) = (source, target) {
                let (source, target) = (source.center(), target.center());
                match self.frame_count {
                    760 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    761 => raw.events.push(egui::Event::PointerMoved(source + egui::vec2(12.0, 0.0))),
                    762..=764 => raw.events.push(egui::Event::PointerMoved(target)),
                    765 => raw.events.push(button(target, false)),
                    _ => {}
                }
            }
            return;
        }
        if mode.starts_with("material-preview") {
            let item = self
                .egui_ctx
                .data(|d| d.get_temp::<Uuid>(egui::Id::unique("texture_library_first_item")))
                .unwrap_or_default();
            if let Some(source) = self
                .egui_ctx
                .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("texture_library_first_card")))
            {
                match self.frame_count {
                    650 | 662 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    651 | 663 => raw.events.push(button(source, false)),
                    _ => {}
                }
            }
            if mode != "material-preview"
                && let Some(rect) = self
                    .egui_ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("resource_material_scene", item))))
            {
                let source = rect.center();
                let rotated = source + egui::vec2(45.0, 18.0);
                match self.frame_count {
                    700 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    701..=703 => raw.events.push(egui::Event::PointerMoved(rotated)),
                    704 => raw.events.push(button(rotated, false)),
                    _ => {}
                }
            }
            if mode == "material-preview-drag"
                && let Some(rect) = self
                    .egui_ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("resource_material_handle", item))))
            {
                let source = rect.center();
                let target = egui::pos2(480.0, 430.0);
                match self.frame_count {
                    760 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    761 => raw.events.push(egui::Event::PointerMoved(source + egui::vec2(12.0, 0.0))),
                    762..=764 => raw.events.push(egui::Event::PointerMoved(target)),
                    765 => raw.events.push(button(target, false)),
                    _ => {}
                }
            }
            return;
        }
        let inventory = matches!(mode.as_str(), "preview" | "preview-drag" | "inventory-drag");
        if !inventory
            && !matches!(
                mode.as_str(),
                "drop" | "apply" | "apply-pbr" | "objects-drop" | "objects-nocopy-drop"
            )
        {
            return;
        }
        let item = if mode.starts_with("objects") {
            crate::demo::texture_library::first_object()
        } else {
            crate::demo::texture_library::first_texture()
        };
        let source = if inventory {
            self.egui_ctx
                .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique(("clipboard_demo_row", self.inventory_ui.ui_id, item))))
        } else {
            self.egui_ctx
                .data(|d| d.get_temp::<egui::Pos2>(egui::Id::unique("texture_library_first_card")))
        };
        if let Some(source) = source {
            if matches!(mode.as_str(), "apply" | "apply-pbr") {
                match self.frame_count {
                    650 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    651 => raw.events.push(button(source, false)),
                    _ => {}
                }
                return;
            }
            if matches!(mode.as_str(), "preview" | "preview-drag") {
                match self.frame_count {
                    650 | 662 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                    651 | 663 => raw.events.push(button(source, false)),
                    _ => {}
                }
            }
        }
        let source = if mode == "preview-drag" {
            self.egui_ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::unique(("resource_texture_preview", item))))
                .map(|r| r.center())
        } else {
            source
        };
        let Some(source) = source else { return };
        let start = if inventory { 720 } else { 650 };
        let target = if mode.starts_with("objects") {
            let point = self.world.agent.position + Vec3::new(5.0, 2.0, 0.0);
            let Some(target) = self
                .world
                .ground_height(point)
                .and_then(|z| self.build.cam.project(Vec3::new(point.x, point.y, z)))
            else {
                return;
            };
            target
        } else {
            egui::pos2(480.0, 430.0)
        };
        if mode != "preview" {
            match self.frame_count.saturating_sub(start) {
                0 if self.frame_count >= start => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
                1 => raw.events.push(egui::Event::PointerMoved(source + egui::vec2(12.0, 0.0))),
                2..=4 => raw.events.push(egui::Event::PointerMoved(target)),
                5 => raw.events.push(button(target, false)),
                _ => {}
            }
        }
    }

    pub(super) fn show_texture_library(&mut self, ctx: &egui::Context, p: &Palette) {
        if self.demo
            && !self.library_demo_seeded
            && self.frame_count >= 600
            && !self.world.inventory.root.is_nil()
            && let Ok(mode) = std::env::var("AURORA_DEMO_TEXTURE_LIBRARY")
        {
            crate::demo::texture_library::seed(&mut self.world.inventory, &mut self.scene, self.world.agent_id, mode == "empty");
            let objects: Vec<_> = self
                .world
                .objects
                .iter()
                .filter(|(_, o)| !o.is_avatar() && !o.is_tree())
                .map(|(idx, _)| idx)
                .collect();
            for idx in objects {
                if let Some(object) = self.world.objects.get_mut(idx) {
                    object.owner_id = self.world.agent_id;
                    object.update_flags |= crate::ui::context::flags::OBJECT_MODIFY | crate::ui::context::flags::OBJECT_YOU_OWNER;
                }
            }
            self.library_demo_seeded = true;
            self.settings.texture_library.presentation =
                if matches!(mode.as_str(), "horizontal" | "objects-horizontal" | "types-horizontal") {
                    library::Presentation::Horizontal
                } else {
                    library::Presentation::Vertical
                };
            self.panels.texture_library = !matches!(mode.as_str(), "baseline" | "preview" | "preview-drag" | "inventory-drag")
                && !mode.starts_with("cascade")
                && !mode.starts_with("panorama");
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.perf = false;
            self.panels.minimap = false;
            self.panels.ao = false;
            if matches!(mode.as_str(), "objects-drop" | "objects-nocopy-drop") && std::env::var_os("AURORA_DEMO_CAM").is_none() {
                self.camera.demo_orbit(0.0, 0.6, Some(9.0), &self.settings.camera);
            }
            let inventory_drag = crate::demo::texture_library::inventory_drag_enabled();
            self.panels.inventory = matches!(mode.as_str(), "inventory" | "objects-inventory") || inventory_drag;
            if mode == "objects-inventory" {
                self.inventory_ui
                    .show_original(&self.world.inventory, crate::demo::texture_library::first_object());
            }
            if inventory_drag {
                let item = crate::demo::texture_library::first_texture();
                self.inventory_ui.show_original(&self.world.inventory, item);
                ui::widgets::place_floater(
                    ctx,
                    &library::preview_window_id(item),
                    egui::pos2(20.0, 60.0),
                    egui::vec2(360.0, 280.0),
                );
            } else if mode == "inventory"
                && let Some(root) = catalogue::child(&self.world.inventory, self.world.inventory.root, catalogue::AURORA)
            {
                self.inventory_ui.show_folder(&self.world.inventory, root);
            }
            self.settings.texture_library.texture_preview_size = None;
            self.settings.texture_library.material_preview_size = None;
            self.texture_library.demo(&mode, self.world.agent_id);
            if mode.starts_with("face-") || mode == "preview-controls-local-apply" {
                if let Some(idx) = self.world.objects.index_of_uuid(&crate::demo::action_id(970)) {
                    let position = self.world.agent.position + Vec3::new(0.0, -2.0, 1.0);
                    if let Some(object) = self.world.objects.get_mut(idx) {
                        object.position = position;
                        object.scale = Vec3::splat(1.5);
                        object.rotation = glam::Quat::IDENTITY;
                        if mode.contains("transparent")
                            && let Some(te) = object.te.as_mut()
                        {
                            for face in &mut Arc::make_mut(te).faces {
                                face.color[3] = 0.0;
                            }
                        }
                        object.material_dirty = true;
                        object.render.needs_records = true;
                    }
                }
                self.camera.demo_orbit(0.0, 0.15, Some(8.0), &self.settings.camera);
            }
            if ui::inventory::drag::demo::enabled() {
                self.panels.texture_library = false;
                self.panels.inventory = false;
                ui::inventory::drag::demo::setup(ctx, &mut self.world.inventory, &mut self.inventory_ui);
            }
            if mode.starts_with("panorama") {
                crate::demo::texture_library::seed_panorama(&mut self.world.inventory, self.world.agent_id);
            }
            if mode.starts_with("cascade") {
                let kind = match mode.as_str() {
                    "cascade-notecard" => 7,
                    "cascade-script" => 10,
                    _ => 0,
                };
                crate::demo::texture_library::seed_cascade(&mut self.world.inventory, self.world.agent_id, kind);
            }
        }
        if self.demo
            && self.library_demo_seeded
            && let Ok(mode) = std::env::var("AURORA_DEMO_TEXTURE_LIBRARY")
            && mode.starts_with("cascade")
            && (620..=720).contains(&self.frame_count)
            && (self.frame_count - 620).is_multiple_of(20)
        {
            let kind = match mode.as_str() {
                "cascade-material" => 57,
                "cascade-notecard" => 7,
                "cascade-script" => 10,
                _ => 0,
            };
            let items = crate::demo::texture_library::cascade_items(kind);
            self.apply_inventory_action(crate::ui::inventory::InvAction::Preview(
                items[((self.frame_count - 620) / 20) as usize],
            ));
        }
        if self.demo
            && self.library_demo_seeded
            && std::env::var("AURORA_DEMO_TEXTURE_LIBRARY").is_ok_and(|mode| mode.starts_with("multi-preview"))
        {
            let items = crate::demo::texture_library::preview_items();
            for (index, item) in items.into_iter().enumerate() {
                // Reapply after egui has measured the new windows: its initial
                // size estimate otherwise clamps the second row too high.
                if self.frame_count == 620 + index as u64 * 20 || self.frame_count == 700 {
                    ui::widgets::place_floater(
                        ctx,
                        &library::preview_window_id(item),
                        egui::pos2(20.0 + (index % 2) as f32 * 340.0, 60.0 + (index / 2) as f32 * 345.0),
                        egui::vec2(300.0, 290.0),
                    );
                    self.texture_library.open_preview(ctx, &self.world, item);
                }
            }
            if self.frame_count == 720 {
                self.texture_library.open_preview(ctx, &self.world, items[0]);
            }
            if self.frame_count >= 760 && std::env::var("AURORA_DEMO_TEXTURE_LIBRARY").as_deref() == Ok("multi-preview-closed") {
                self.panels.texture_library = false;
            }
        }
        // `panorama`: the Aurora 360° upload opens as a panorama; `-switch`:
        // a plain 2:1 texture, its 360° switch turned on; `-full`: enlarged;
        // `-nocopy`: a marked item the agent may not copy (Copy UUID and
        // Save As stay off)
        if self.demo
            && self.library_demo_seeded
            && let Ok(mode) = std::env::var("AURORA_DEMO_TEXTURE_LIBRARY")
            && mode.starts_with("panorama")
        {
            let items = crate::demo::texture_library::panorama_items();
            let item = match mode.as_str() {
                "panorama-switch" => items[1],
                "panorama-nocopy" => items[2],
                _ => items[0],
            };
            if self.frame_count == 620 {
                ui::widgets::place_floater(
                    ctx,
                    &library::preview_window_id(item),
                    egui::pos2(20.0, 60.0),
                    egui::vec2(720.0, 480.0),
                );
                self.texture_library.open_preview(ctx, &self.world, item);
            }
            if self.frame_count == 700 {
                self.texture_library.demo_panorama(&mode);
            }
        }
        if self.demo && self.library_demo_seeded && std::env::var("AURORA_DEMO_TEXTURE_LIBRARY").as_deref() == Ok("preview-sizes") {
            let items = crate::demo::texture_library::preview_items();
            for (item, frame) in items.into_iter().zip([620, 670, 690, 740]) {
                if self.frame_count == frame {
                    self.texture_library.open_preview(ctx, &self.world, item);
                }
            }
            if self.frame_count == 780 {
                for (index, item) in items.into_iter().enumerate() {
                    ui::widgets::move_floater(
                        ctx,
                        &library::preview_window_id(item),
                        egui::pos2(20.0 + (index % 2) as f32 * 380.0, 60.0 + (index / 2) as f32 * 345.0),
                    );
                }
                log::info!(
                    "demo preview window sizes: texture={:?}, material={:?}",
                    self.settings.texture_library.texture_preview_size,
                    self.settings.texture_library.material_preview_size
                );
            }
        }
        if self.demo
            && self.library_demo_seeded
            && let Ok(mode) = std::env::var("AURORA_DEMO_TEXTURE_LIBRARY")
            && mode.starts_with("preview-controls")
            && matches!(self.frame_count, 620 | 700)
        {
            self.panels.texture_library = false;
            self.settings.texture_library.texture_display = Default::default();
            let items = crate::demo::texture_library::preview_items();
            for (index, item) in [items[0], items[2]].into_iter().enumerate() {
                self.texture_library.open_preview(ctx, &self.world, item);
                ui::widgets::place_floater(
                    ctx,
                    &library::preview_window_id(item),
                    egui::pos2(24.0 + index as f32 * 660.0, 60.0),
                    if mode.ends_with("-narrow") {
                        egui::vec2(280.0, 520.0)
                    } else {
                        egui::vec2(620.0, 720.0)
                    },
                );
            }
            if self.frame_count == 700 {
                self.texture_library.demo_preview_controls(&mode, &mut self.world);
            }
        }
        if self.demo && std::env::var("AURORA_DEMO_TEXTURE_LIBRARY").as_deref() == Ok("preview-controls-local-apply") {
            let [texture, _, item, _] = crate::demo::texture_library::preview_items();
            if self.frame_count == 740
                && let Some(material) = self.texture_library.demo_material_draft(item)
            {
                self.resource_material_save(item, material, None);
                self.texture_library.close_texture(texture);
                ui::widgets::move_floater(ctx, &library::preview_window_id(item), egui::pos2(24.0, 60.0));
            }
            if self.frame_count == 760
                && let Some(object) = self.world.objects.index_of_uuid(&crate::demo::action_id(970))
            {
                self.apply_texture_library_drop(Drop {
                    item,
                    object,
                    face: 0,
                    all: true,
                    linkset: false,
                    confirmed: false,
                    contents: false,
                    remove_pbr: false,
                });
                let asset = self.world.inventory.items.get(&item).map(|item| item.asset_id);
                let maps = asset
                    .and_then(|asset| self.scene.materials.peek(&asset))
                    .map(|material| crate::imports::maps::ids(&material));
                let applied = self.world.objects.get(object).and_then(|object| object.visual_material(0));
                log::info!("demo saved local material: asset={asset:?}, maps={maps:?}, applied={applied:?}");
            }
        }
        let preferences = self.settings.texture_library.clone();
        let actions = self.texture_library.show(
            ctx,
            p,
            &mut self.world,
            &mut self.scene,
            &self.ui_images,
            &mut self.settings.texture_library,
            &mut self.panels.texture_library,
        );
        self.panels.texture_library_presentation = self.settings.texture_library.presentation;
        for action in actions {
            match action {
                library::Action::Change(mut change) => {
                    if self.texture_library.pending.is_some() {
                        continue;
                    }
                    change.defer_folder_contents = false;
                    let request = Uuid::new_v4();
                    self.texture_library.pending = Some(request);
                    self.texture_library.pending_refresh.clone_from(&change.refresh);
                    self.send(NetCommand::EditInventory { request, change });
                }
                library::Action::Apply(id) => self.apply_texture_library_selection(id),
                library::Action::Open(id) => {
                    if let Some(item) = self.world.inventory.items.get(&id) {
                        let action = if item.asset_type == 3 {
                            ui::inventory::InvAction::AboutLandmark(id, item.asset_id)
                        } else {
                            ui::inventory::InvAction::Preview(id)
                        };
                        self.apply_inventory_action(action);
                    }
                }
                library::Action::Reveal(id) => {
                    self.inventory_ui.show_folder(&self.world.inventory, id);
                    ui::widgets::open_floater(ctx, "inventory", &mut self.panels.inventory);
                }
                library::Action::SaveMaterial { item, material, name } => self.resource_material_save(item, material, name),
                library::Action::RefreshTexture(asset) => {
                    self.ui_images.remove(&asset);
                    self.scene.textures.refresh(&asset);
                    self.texture_library.refresh_texture(&self.world, asset);
                }
                library::Action::ExportResult(message) => self.world.system_message(message),
                library::Action::ExportTexture { item, png } => {
                    let full = rules::COPY | rules::MODIFY | rules::TRANSFER;
                    if self
                        .world
                        .inventory
                        .items
                        .get(&item)
                        .is_some_and(|it| rules::held(&self.world.inventory, it) & full == full)
                    {
                        self.texture_library.export_texture(item, png);
                    }
                }
                library::Action::Profile(resident) => self.apply_inventory_action(ui::inventory::InvAction::Profile(resident)),
                library::Action::MaterialMapFile { item, slot } => {
                    if !self.world.inventory.items.get(&item).is_some_and(|it| {
                        it.owner == self.world.agent_id
                            && !it.group_owned
                            && rules::agent_mask(it, self.world.agent_id) & rules::MODIFY != 0
                            && !rules::library(&self.world.inventory, item)
                    }) {
                        continue;
                    }
                    if let Some(entry) = self
                        .imports
                        .choose_material_map_upload(&self.scene.jobs, ctx, self.settings.imports.texture())
                    {
                        self.texture_library.track_map_upload(entry, item, slot);
                    }
                }
                library::Action::Discard(item) => {
                    if let Some(trash) = rules::system(&self.world.inventory, 14)
                        && let Ok(change) = rules::move_selection(
                            &self.world.inventory,
                            &[item],
                            trash,
                            &self.settings.inventory.protected,
                            &HashSet::new(),
                        )
                        && self.texture_library.pending.is_none()
                    {
                        let request = Uuid::new_v4();
                        self.texture_library.pending = Some(request);
                        self.texture_library.pending_refresh.clone_from(&change.refresh);
                        self.texture_library.close_texture(item);
                        self.send(NetCommand::EditInventory { request, change });
                    }
                }
            }
        }
        if self.settings.texture_library != preferences {
            self.settings.save();
        }
    }
    fn resource_material_save(&mut self, id: Uuid, material: aurora_assets::PbrMaterial, name: Option<String>) {
        // LLMaterialEditor::getEncodedAsset / finishInventoryUpload
        // (indra/newview/llmaterialeditor.cpp) retain local bitmap IDs. Our
        // local registry restores stable IDs, so the saved asset resolves the
        // same local textures when used on a face, without uploading pixels.
        let Some(item) = self.world.inventory.items.get(&id).cloned() else {
            return;
        };
        let own = item.owner == self.world.agent_id && !item.group_owned;
        let editable =
            own && rules::agent_mask(&item, self.world.agent_id) & rules::MODIFY != 0 && !rules::library(&self.world.inventory, id);
        let copy = name.is_some();
        if item.asset_type != 57
            || if copy {
                rules::held(&self.world.inventory, &item) & rules::COPY == 0
            } else {
                !editable
            }
        {
            return;
        }
        if !editable && self.texture_library.material_draft_dirty(id) {
            return;
        }
        let name = match name.map(|name| rules::clean_name(&name)).transpose() {
            Ok(name) => name,
            Err(reason) => {
                self.texture_library.material_message(id, reason);
                return;
            }
        };
        let request = Uuid::new_v4();
        if !self.texture_library.begin_material_save(id, request) {
            return;
        }
        if let Some(name) = name {
            self.send(NetCommand::CopyInventoryItem {
                request,
                copy: aurora_net::inventory::operations::CopyItem {
                    item: id,
                    owner: item.owner,
                    parent: if rules::library(&self.world.inventory, id) || rules::in_type(&self.world.inventory, id, 53) {
                        rules::system(&self.world.inventory, 57).unwrap_or(self.world.inventory.root)
                    } else {
                        item.parent
                    },
                    name,
                },
            });
        } else {
            match material.encode_asset() {
                Ok(data) => self.send(NetCommand::SaveInventoryContent {
                    request,
                    item: id,
                    asset_type: 57,
                    lua: false,
                    data,
                }),
                Err(reason) => self.resource_material_saved(request, Err(reason.to_string())),
            }
        }
    }
    pub(super) fn resource_material_copied(&mut self, request: Uuid, result: Result<aurora_net::inventory::InvItem, String>) {
        match result {
            Ok(item) => {
                self.world.inventory.add_items(vec![item.clone()]);
                if let Some(material) = self.texture_library.material_copy_ready(request, item.id) {
                    match material.encode_asset() {
                        Ok(data) => self.send(NetCommand::SaveInventoryContent {
                            request,
                            item: item.id,
                            asset_type: 57,
                            lua: false,
                            data,
                        }),
                        Err(reason) => self.resource_material_saved(request, Err(reason.to_string())),
                    }
                } else {
                    self.resource_material_saved(request, Ok(item));
                }
            }
            Err(reason) => self.resource_material_saved(request, Err(reason)),
        }
    }
    pub(super) fn resource_material_saved(&mut self, request: Uuid, result: Result<aurora_net::inventory::InvItem, String>) {
        let material = self.texture_library.material_saved(request, &result);
        if let Ok(item) = result {
            if let Some(material) = material {
                self.scene.materials.insert(item.asset_id, material);
            }
            self.world.inventory.add_items(vec![item]);
        }
    }
    pub(super) fn texture_library_result(
        &mut self,
        request: Uuid,
        result: &Result<aurora_net::inventory::operations::Reply, String>,
    ) -> bool {
        if !self.texture_library.mutation_result(request, result) {
            return false;
        }
        match result {
            Ok(reply) => {
                self.world.inventory.remove(&reply.removed);
                self.world.inventory.apply(reply.contents.clone());
                self.world.inventory.created_folders(reply.folders.clone());
            }
            Err(reason) => {
                log::warn!("texture library inventory update failed: {reason}");
                // A compound mutation can partly succeed. Read parents back
                // before retrying so a created folder is never duplicated.
                for id in std::mem::take(&mut self.texture_library.pending_refresh) {
                    if let Some(folder) = self.world.inventory.folders.get_mut(&id) {
                        folder.state = crate::world::inventory::FetchState::Unknown;
                        self.world.inventory.request(id);
                    }
                }
                self.world.inventory.generation += 1;
            }
        }
        self.texture_library.pending_refresh.clear();
        true
    }
    /// The face of what we wear that the faces drawn designate, if any
    /// (scene/drawn.rs): the CPU search leaves worn meshes to them.
    fn worn_drop_face(&self, drawn: &aurora_render::Drawn, ray: (Vec3, Vec3)) -> Option<(usize, u8)> {
        let shown = self.scene.designated(
            &self.world,
            crate::scene::drawn::Gesture::Drop,
            drawn,
            Some(ray),
            self.library_drag.found,
            None,
            self.build.open,
        )?;
        let wearer = super::inventory_drag::wearer(&self.world, shown.idx)?;
        let own = self.world.objects.get(wearer)?.full_id == self.world.agent_id;
        own.then_some((shown.idx, u8::try_from(shown.face?).ok()?))
    }

    pub(super) fn texture_library_drag_input(&mut self, ctx: &egui::Context, renderer: &mut Renderer) -> Option<Drop> {
        self.library_drag.highlighted.clear();
        if !matches!(self.screen, Screen::World) || self.world.teleporting || self.exit_prompt.open {
            return None;
        }
        let Some(drag) = ui::inventory::world_drag(ctx) else {
            self.library_drag.end_worn_search(renderer);
            return None;
        };
        // over another resident or what they wear, the texture is given
        // (LLToolDragAndDrop::pick turns their attachments into DT_AVATAR)
        if self.inventory_drag.giving {
            self.library_drag.end_worn_search(renderer);
            return None;
        }
        let item = catalogue::resource(&self.world.inventory, drag.item)?;
        let asset = catalogue::asset(item);
        if asset.is_nil() {
            return None;
        }
        let position = (drag.position.x * ctx.pixels_per_point(), drag.position.y * ctx.pixels_per_point());
        let ray = renderer.cursor_ray(position.0, position.1)?;
        let refresh = drag.released
            || self.library_drag.ray != Some(ray)
            || self
                .library_drag
                .checked
                .is_none_or(|at| at.elapsed() >= Duration::from_millis(100));
        if refresh {
            // Unlike rez supports, texture/material targets include transparent
            // faces. Keep the exact face from the same query as the object.
            let hud = self
                .settings
                .show_huds
                .then_some(self.scene.lists.hud_view)
                .flatten()
                .and_then(|view| {
                    self.scene.resource_drop_face_at_ray(
                        &self.world,
                        view.ray(position.0, position.1),
                        true,
                        f32::INFINITY,
                        &mut self.library_drag.candidates,
                    )
                });
            let surface = hud.or_else(|| {
                self.scene.resource_drop_face_at_ray(
                    &self.world,
                    ray,
                    false,
                    self.settings.draw_distance,
                    &mut self.library_drag.candidates,
                )
            });
            self.library_drag.target = surface.map(|(idx, face)| (idx, face as u8));
            self.library_drag.ray = Some(ray);
            self.library_drag.checked = Some(Instant::now());
            if hud.is_some() {
                self.library_drag.end_worn_search(renderer);
            } else {
                // What stands before a worn face: land above all, which the
                // faces drawn do not tell. An avatar's box holds what it wears.
                let found = self
                    .scene
                    .inventory_surface(&self.world, ray, self.settings.draw_distance, &mut self.library_drag.candidates);
                let avatar = found
                    .and_then(|(_, idx)| self.world.objects.get(idx?))
                    .is_some_and(|o| o.is_avatar());
                self.library_drag.found = found.filter(|_| !avatar).map(|(point, _)| point);
                if self.library_drag.drawn.is_none() {
                    let ticket = renderer.request_drawn(position.0, position.1, aurora_render::DrawnMode::Interaction);
                    self.library_drag.drawn = Some((ticket, ray));
                }
            }
        }
        if let Some((ticket, asked)) = self.library_drag.drawn
            && let Some(drawn) = renderer.take_drawn(ticket)
        {
            self.library_drag.drawn = None;
            self.library_drag.worn = self.worn_drop_face(&drawn, asked);
        }
        // On release too: the last answer, a few frames old, is what was shown.
        let target = self.library_drag.worn.or(self.library_drag.target);
        let worn = self.library_drag.worn.is_some();
        let linkset = linkset_drop(worn, drag.ctrl, drag.shift);
        let ctrl = drag.ctrl && !linkset;
        let allowed = target
            .and_then(|(idx, face)| self.world.objects.get(idx).map(|o| (idx, face, o)))
            .is_some_and(|(idx, face, o)| {
                !o.is_avatar()
                    && !o.is_tree()
                    && o.te.as_ref().is_some_and(|te| (face as usize) < te.faces.len())
                    && (if ctrl {
                        // A world drop follows willObjectAcceptInventory;
                        // the Contents panel's New script rule is narrower.
                        super::inventory_drag::target_allowed(&self.world, idx, item, true).is_ok()
                    } else {
                        self.build.can_modify(&self.world, idx)
                    })
            });
        let protected_asset = requires_contents(&self.world.inventory, item, self.world.agent_id);
        let mut reason = None;
        if !allowed || ctrl && self.world.inventory.items.get(&drag.item).is_some_and(|i| i.asset_type == 24) {
            let locked = !ctrl
                && target
                    .and_then(|(idx, _)| self.world.objects.get(idx).map(|o| (idx, o)))
                    .is_some_and(|(idx, o)| !o.is_avatar() && !o.is_tree() && !self.build.can_modify(&self.world, idx));
            reason = Some(if locked {
                aurora_i18n::tr!("build-ui-texture-drop-object")
            } else {
                aurora_i18n::tr!("texture-library-locked")
            });
        }
        let worn_refused = !ctrl && allowed && target.is_some_and(|(idx, _)| contents_drop::worn_refused(&self.world, idx, item));
        // dad3dApplyToObject: a texture without copy and transfer has nowhere
        // to go when every face it covers carries a material
        let pbr_refused = !ctrl
            && allowed
            && item.asset_type != 57
            && target.is_some_and(|(idx, face)| {
                let override_ok = crate::build::texture_copy::map_slot_takes(item, self.world.agent_id);
                drop_prims(&self.world, idx, linkset).into_iter().all(|prim| {
                    let faces = drop_faces(&self.world, prim, face, drag.shift);
                    !crate::build::BuildTool::surface_takes_texture(&self.world, prim, &faces, override_ok, self.alt)
                })
            });
        if worn_refused {
            reason = Some(contents_drop::worn_refusal_text(item, self.world.agent_id));
        } else if pbr_refused {
            reason = Some(aurora_i18n::tr!("build-ui-texture-drop-restricted"));
        } else if allowed
            && let Some((idx, _)) = target
            && (protected_asset || ctrl)
        {
            if protected_asset && !ctrl {
                let object = self.world.objects.get(idx)?.full_id;
                self.build.request_contents(&self.world, idx, false);
                if self
                    .build
                    .contents
                    .objects
                    .get(&object)
                    .is_none_or(|inventory| inventory.loading || inventory.requested.is_none())
                {
                    reason = Some(aurora_i18n::tr!("texture-library-wait-inventory"));
                } else {
                    let retained = self.build.contents.objects.get(&object).is_some_and(|inv| {
                        inv.items
                            .iter()
                            .any(|i| i.asset_id == item.asset_id && i.asset_type as i32 == item.asset_type)
                    });
                    if !retained && super::inventory_drag::target_allowed(&self.world, idx, item, true).is_err() {
                        reason = Some(aurora_i18n::tr!("texture-library-locked"));
                    }
                }
            } else if super::inventory_drag::target_allowed(&self.world, idx, item, true).is_err() {
                reason = Some(aurora_i18n::tr!("texture-library-locked"));
            }
        }
        self.inventory_drag.cursor = Some(if reason.is_none() {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::NotAllowed
        });
        if reason.is_none()
            && let Some((idx, face)) = target
            && !ctrl
            && !drag.released
            && face < 64
        {
            for prim in drop_prims(&self.world, idx, linkset) {
                let te = self.world.objects.get(prim).and_then(|o| o.te.as_ref());
                let mask = if drag.shift {
                    te.map_or(0, |te| (0..te.faces.len().min(64)).fold(0, |mask, i| mask | 1u64 << i))
                } else {
                    1u64 << face
                };
                self.library_drag.highlighted.push((prim, mask));
            }
        }
        if !drag.released {
            // a refusal says why first; over what we wear, the keys come next
            if reason.is_some() || worn && !ctrl {
                let p = &self.palette;
                egui::Tooltip::always_open(
                    ctx.clone(),
                    egui::LayerId::background(),
                    egui::Id::unique("texture-world-drop-refused"),
                    // clear of the cursor and of the badge it carries
                    egui::PopupAnchor::Position(drag.position + egui::vec2(16.0, 14.0)),
                )
                .show(|ui| match reason {
                    Some(reason) => {
                        ui.label(reason);
                    }
                    None => worn_drop_keys(ui, p, drag.shift, linkset),
                });
            }
        } else {
            egui::DragAndDrop::clear_payload(ctx);
            self.library_drag.end_worn_search(renderer);
            if reason.is_some() {
                return None;
            }
            let (object, face) = target?;
            return Some(Drop {
                item: drag.item,
                object,
                face,
                all: drag.shift,
                linkset,
                confirmed: false,
                contents: ctrl,
                remove_pbr: self.alt,
            });
        }
        None
    }
    /// Whether the contents of `object` already hold the asset of `item`.
    fn contents_retain(&self, object: Uuid, item: &aurora_net::inventory::InvItem) -> bool {
        self.build.contents.objects.get(&object).is_some_and(|inv| {
            inv.items
                .iter()
                .any(|i| i.asset_id == item.asset_id && i.asset_type as i32 == item.asset_type)
        })
    }

    /// LLToolDragAndDrop::dad3dApplyToObject's "ApplyInventoryToObject": a
    /// no-copy item is only applied once the user agrees to move it. Gives
    /// the drop back when they do; the object is looked up again, as the
    /// dialog can stay open for long.
    pub(super) fn no_copy_drop_prompt(&mut self, ctx: &egui::Context) -> Option<Drop> {
        self.library_drag.confirm.as_ref()?;
        let p = &self.palette;
        let mut answer = ctx.input(|i| i.key_pressed(egui::Key::Escape)).then_some(false);
        ui::widgets::Modal::new(egui::Id::unique("texture-no-copy-drop")).show(ctx, |ui| {
            ui.set_max_width(380.0);
            ui.label(aurora_i18n::tr!("texture-library-no-copy-drop"));
            ui.add_space(6.0);
            ui.label(aurora_i18n::tr!("texture-library-no-copy-drop-question"));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                if ui.button(aurora_i18n::tr!("common-cancel")).clicked() {
                    answer = Some(false);
                }
                if ui::widgets::action_button(
                    ui,
                    p,
                    "check",
                    aurora_i18n::tr!("texture-library-no-copy-drop-move"),
                    ui::widgets::ActionKind::Primary,
                    egui::vec2(0.0, 28.0),
                )
                .clicked()
                {
                    answer = Some(true);
                }
            });
        });
        let accepted = answer?;
        let (mut drop, object) = self.library_drag.confirm.take()?;
        drop.object = self.world.objects.index_of_uuid(&object).filter(|_| accepted)?;
        drop.confirmed = true;
        Some(drop)
    }

    pub(super) fn apply_texture_library_drop(&mut self, drop: Drop) {
        let Some(item) = catalogue::resource(&self.world.inventory, drop.item).cloned() else {
            return;
        };
        if !drop.confirmed && !drop.contents {
            let Some(object) = self.world.objects.get(drop.object).map(|o| o.full_id) else {
                return;
            };
            let retained = self.contents_retain(object, &item);
            if no_copy_leaves(&self.world.inventory, &item, self.world.agent_id, retained) {
                self.library_drag.confirm = Some((drop, object));
                return;
            }
        }
        let mut applied = false;
        for idx in drop_prims(&self.world, drop.object, drop.all && drop.linkset) {
            let Some(object) = self.world.objects.get(idx) else { continue };
            let object_id = object.full_id;
            let count = self
                .media
                .face_count(&self.world, &self.scene, idx)
                .unwrap_or_else(|| self.build.num_faces(&self.world, idx));
            self.build.face_counts.insert(object_id, count);
            let faces: Vec<_> = if drop.all {
                (0..count.min(256)).map(|i| i as u8).collect()
            } else {
                vec![drop.face]
            };
            applied |= self.apply_inventory_resource(&item, idx, &faces, drop.contents, drop.remove_pbr);
        }
        if applied && !drop.contents {
            self.texture_resource_applied(item.id);
        }
        self.flush_build();
    }

    fn apply_texture_library_selection(&mut self, id: Uuid) {
        if !self.build.open || self.build.tool != crate::build::Tool::Edit {
            return;
        }
        let Some(item) = catalogue::resource(&self.world.inventory, id).cloned() else {
            return;
        };
        // The expanded texture entry is not a geometry face count (notably
        // on meshes). Refresh bounds before collecting the selected faces.
        for idx in self.build.sel_prims(&self.world) {
            if let Some(object) = self.world.objects.get(idx) {
                let count = self
                    .media
                    .face_count(&self.world, &self.scene, idx)
                    .unwrap_or_else(|| self.build.num_faces(&self.world, idx));
                self.build.face_counts.insert(object.full_id, count);
            }
        }
        let targets = self.build.inventory_surface_selection(&self.world);
        if targets.is_empty() {
            return;
        }
        // LLSelectMgr::selectionSetImage refuses duplicating a no-copy item.
        if selection_copy_denied(&item, self.world.agent_id, targets.len()) {
            self.world.system_notice(
                aurora_i18n::tr!("texture-library-no-copy-multiple"),
                crate::world::notice_topics::Topic::Items,
            );
            return;
        }
        // Before the contents: what an attachment holds does not change its rule.
        if targets.iter().any(|(idx, _)| contents_drop::worn_refused(&self.world, *idx, &item)) {
            self.world.system_notice(
                contents_drop::worn_refusal_text(&item, self.world.agent_id),
                crate::world::notice_topics::Topic::Items,
            );
            return;
        }
        let mut waiting = false;
        if requires_contents(&self.world.inventory, &item, self.world.agent_id) {
            for (idx, _) in &targets {
                let Some(object) = self.world.objects.get(*idx) else { continue };
                let object_id = object.full_id;
                self.build.request_contents(&self.world, *idx, false);
                let inventory = self.build.contents.objects.get(&object_id);
                if inventory.is_none_or(|inv| inv.loading || inv.requested.is_none()) {
                    waiting = true;
                } else if !inventory.is_some_and(|inv| {
                    inv.items
                        .iter()
                        .any(|i| i.asset_id == item.asset_id && i.asset_type as i32 == item.asset_type)
                }) && super::inventory_drag::target_allowed(&self.world, *idx, &item, true).is_err()
                {
                    self.world.system_notice(
                        aurora_i18n::tr!("texture-library-locked"),
                        crate::world::notice_topics::Topic::Items,
                    );
                    return;
                }
            }
        }
        if waiting {
            self.world.system_notice(
                aurora_i18n::tr!("texture-library-wait-selection"),
                crate::world::notice_topics::Topic::Items,
            );
            return;
        }
        let mut applied = false;
        for (idx, faces) in targets {
            applied |= self.apply_inventory_resource(&item, idx, &faces, false, false);
        }
        if applied {
            self.texture_resource_applied(item.id);
        }
        self.flush_build();
    }

    /// The same inventory retention and surface edit path is used by preview
    /// drags, inventory drags and clicks on the library's current selection.
    fn apply_inventory_resource(
        &mut self,
        item: &aurora_net::inventory::InvItem,
        idx: usize,
        faces: &[u8],
        contents: bool,
        remove_pbr: bool,
    ) -> bool {
        let Some(object) = self.world.objects.get(idx) else { return false };
        let object_id = object.full_id;
        let asset = catalogue::asset(item);
        if asset.is_nil() || !contents && !self.build.can_modify(&self.world, idx) {
            return false;
        }
        if !contents && contents_drop::worn_refused(&self.world, idx, item) {
            return false;
        }
        let override_ok = item.asset_type == 57 || crate::build::texture_copy::map_slot_takes(item, self.world.agent_id);
        if !contents && !crate::build::BuildTool::surface_takes_texture(&self.world, idx, faces, override_ok, remove_pbr) {
            return false;
        }
        let retained = self.contents_retain(object_id, item);
        if contents || requires_contents(&self.world.inventory, item, self.world.agent_id) && !retained {
            if super::inventory_drag::target_allowed(&self.world, idx, item, true).is_err() {
                return false;
            }
            let task = crate::build::task_inventory::from_inventory(object_id, item);
            self.build.send(BuildCmd::UpdateTaskInventory {
                handle: object.key.region,
                local_id: object.key.local_id,
                item: Box::new(task),
            });
            if rules::agent_mask(item, self.world.agent_id) & rules::COPY == 0 {
                self.world.inventory.consume_item(item.id);
            }
            self.build.request_contents(&self.world, idx, true);
        }
        if !contents {
            let mut env = crate::build::ui::Env {
                images: &self.ui_images,
                media: &mut self.media,
                scene: &mut self.scene,
                zoom_fraction: self.camera.zoom_fraction(),
                property_lines: &mut self.settings.world_overlays.property_lines,
                demo: self.demo,
            };
            self.build.apply_inventory_surface_with(
                &mut self.world,
                &mut env,
                idx,
                faces,
                asset,
                item.asset_type == 57,
                remove_pbr,
                override_ok,
            );
        }
        true
    }

    fn texture_resource_applied(&mut self, id: Uuid) {
        self.settings.texture_library.activate(self.world.agent_id);
        self.settings.texture_library.record(id);
        self.settings.save();
        self.world.system_notice(
            aurora_i18n::tr!("texture-library-applied"),
            crate::world::notice_topics::Topic::Items,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build::texture_copy::restricted;
    use aurora_net::inventory::InvItem;
    #[test]
    fn only_a_no_copy_item_the_object_lacks_asks_before_it_moves() {
        let agent = Uuid::from_u128(0xa6);
        let inv = crate::world::inventory::Inventory::default();
        let item = |owner_mask| InvItem {
            owner: agent,
            owner_mask,
            ..Default::default()
        };
        let no_copy = item(rules::TRANSFER);
        assert!(no_copy_leaves(&inv, &no_copy, agent, false));
        assert!(!no_copy_leaves(&inv, &no_copy, agent, true), "already in the object");
        assert!(!no_copy_leaves(&inv, &item(rules::COPY | rules::TRANSFER), agent, false));
    }
    /// The copy right is the agent's: the everyone mask of a group-owned
    /// item or of someone else's, whatever their owner mask; a Library item
    /// never leaves.
    #[test]
    fn a_no_copy_move_reads_the_agents_mask_and_spares_the_library() {
        let agent = Uuid::from_u128(0xa6);
        let mut inv = crate::world::inventory::Inventory::default();
        let full = rules::FULL;
        let group = InvItem {
            owner: Uuid::from_u128(0x6a),
            group_owned: true,
            owner_mask: full,
            everyone_mask: rules::TRANSFER,
            ..Default::default()
        };
        assert!(no_copy_leaves(&inv, &group, agent, false), "group-owned: the everyone mask");
        assert!(!no_copy_leaves(
            &inv,
            &InvItem {
                everyone_mask: full,
                ..group.clone()
            },
            agent,
            false
        ));
        // a group-owned item keeps the everyone mask even when its owner field is the agent
        let own_group = InvItem { owner: agent, ..group };
        assert!(no_copy_leaves(&inv, &own_group, agent, false));
        let others = InvItem {
            owner: Uuid::from_u128(0xb7),
            owner_mask: full,
            everyone_mask: rules::MODIFY | rules::TRANSFER,
            ..Default::default()
        };
        assert!(no_copy_leaves(&inv, &others, agent, false), "someone else's: the everyone mask");
        assert!(!no_copy_leaves(&inv, &others, agent, true), "already in the object");
        let library = Uuid::from_u128(0x11b);
        inv.folders.insert(
            library,
            crate::world::inventory::Folder {
                info: aurora_net::inventory::InvFolder {
                    id: library,
                    name: "Library".into(),
                    type_default: -1,
                    ..Default::default()
                },
                children: Vec::new(),
                items: Vec::new(),
                state: crate::world::inventory::FetchState::Fetched,
                library: true,
            },
        );
        let from_library = InvItem {
            id: Uuid::from_u128(0x11c),
            parent: library,
            everyone_mask: rules::TRANSFER,
            ..others
        };
        inv.items.insert(from_library.id, from_library.clone());
        assert!(
            !no_copy_leaves(&inv, &from_library, agent, false),
            "the Library's asset costs nothing"
        );
    }
    #[test]
    fn only_ctrl_shift_over_what_is_worn_takes_the_whole_linkset() {
        assert!(linkset_drop(true, true, true));
        for (worn, ctrl, shift) in [
            (true, false, false),
            (true, false, true),
            (true, true, false),
            (false, true, true),
            (false, false, true),
        ] {
            assert!(!linkset_drop(worn, ctrl, shift), "{worn} {ctrl} {shift}");
        }
    }
    #[test]
    fn texture_library_never_duplicates_a_no_copy_resource_across_selected_objects() {
        let agent = Uuid::from_u128(90);
        for kind in [0, 57] {
            let mut item = InvItem {
                owner: agent,
                asset_type: kind,
                owner_mask: rules::TRANSFER,
                ..Default::default()
            };
            assert!(!selection_copy_denied(&item, agent, 1));
            assert!(selection_copy_denied(&item, agent, 2));
            item.owner_mask |= rules::COPY;
            assert!(!selection_copy_denied(&item, agent, 2));
            item.owner = Uuid::from_u128(91);
            assert!(selection_copy_denied(&item, agent, 2));
        }
    }
    #[test]
    fn texture_library_retains_restricted_assets_with_the_effective_permission_mask() {
        let agent = Uuid::from_u128(90);
        let mut item = InvItem {
            owner: agent,
            asset_type: 0,
            owner_mask: rules::COPY | rules::TRANSFER,
            ..Default::default()
        };
        assert!(!restricted(&item, agent), "textures do not require Modify");
        item.asset_type = 57;
        assert!(restricted(&item, agent), "no-modify materials must enter contents");
        item.owner_mask |= rules::MODIFY;
        assert!(!restricted(&item, agent));
        item.owner_mask &= !rules::COPY;
        assert!(restricted(&item, agent));
        item.owner_mask |= rules::COPY;
        item.owner_mask &= !rules::TRANSFER;
        assert!(restricted(&item, agent));
        item.owner_mask = rules::COPY | rules::TRANSFER | rules::MODIFY;
        item.owner = Uuid::from_u128(91);
        assert!(restricted(&item, agent), "foreign assets use everyone permissions");
        item.everyone_mask = rules::COPY | rules::TRANSFER | rules::MODIFY;
        assert!(!restricted(&item, agent));
    }
}
