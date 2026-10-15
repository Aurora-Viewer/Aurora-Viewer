//! Library lifecycle and world drops. Behaviour follows Firestorm
//! LLToolDragAndDrop::{dad3dApplyToObject,handleDropMaterialProtections,
//! dropTextureOneFace,dropMaterialOneFace} (LGPL 2.1): Shift all faces,
//! Ctrl contents, restricted assets retained in the object's inventory.

use super::*;
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
}
pub(super) struct Drop {
    item: Uuid,
    object: usize,
    face: u8,
    all: bool,
    contents: bool,
    remove_pbr: bool,
}

fn permissions(item: &aurora_net::inventory::InvItem, agent: Uuid) -> u32 {
    if item.owner == agent && !item.group_owned {
        item.owner_mask
    } else {
        item.everyone_mask
    }
}
fn restricted(item: &aurora_net::inventory::InvItem, agent: Uuid) -> bool {
    let mask = permissions(item, agent);
    mask & rules::COPY == 0 || mask & rules::TRANSFER == 0 || item.asset_type == 57 && mask & rules::MODIFY == 0
}
fn requires_contents(inv: &crate::world::inventory::Inventory, item: &aurora_net::inventory::InvItem, agent: Uuid) -> bool {
    !rules::library(inv, item.id) && restricted(item, agent)
}
impl App {
    pub(super) fn texture_library_demo_input(&self, raw: &mut egui::RawInput) {
        if std::env::var("AURORA_DEMO_TEXTURE_LIBRARY").as_deref() != Ok("drop") {
            return;
        }
        let Some(source) = self
            .egui_ctx
            .data(|d| d.get_temp::<egui::Pos2>(egui::Id::new("texture_library_first_card")))
        else {
            return;
        };
        let target = egui::pos2(480.0, 430.0);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        };
        match self.frame_count {
            650 => raw.events.extend([egui::Event::PointerMoved(source), button(source, true)]),
            651 => raw.events.push(egui::Event::PointerMoved(source + egui::vec2(12.0, 0.0))),
            652..=654 => raw.events.push(egui::Event::PointerMoved(target)),
            655 => raw.events.push(button(target, false)),
            _ => {}
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
            self.settings.texture_library.presentation = if mode == "horizontal" {
                library::Presentation::Horizontal
            } else {
                library::Presentation::Vertical
            };
            self.panels.texture_library = mode != "baseline";
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.perf = false;
            self.panels.minimap = false;
            self.panels.inventory = mode == "inventory";
            if mode == "inventory"
                && let Some(root) = catalogue::root(&self.world.inventory)
            {
                self.inventory_ui.show_folder(&self.world.inventory, root);
            }
            self.texture_library.demo(&mode, self.world.agent_id);
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
                library::Action::Reveal(id) => {
                    self.inventory_ui.show_folder(&self.world.inventory, id);
                    ui::widgets::open_floater(ctx, "inventory", &mut self.panels.inventory);
                }
            }
        }
        if self.settings.texture_library != preferences {
            self.settings.save();
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
    pub(super) fn texture_library_drag_input(&mut self, ctx: &egui::Context, renderer: &Renderer) -> Option<Drop> {
        if !matches!(self.screen, Screen::World) || self.world.teleporting || self.exit_prompt.open {
            return None;
        }
        let drag = ui::inventory::world_drag(ctx)?;
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
            let hud = self.scene.hud_edit_pick(&self.world, position);
            let surface = if let Some((idx, _, ray)) = hud {
                self.scene.touch_surface(&self.world, idx, ray).map(|s| (idx, s.face as u8))
            } else {
                self.scene
                    .inventory_surface(&self.world, ray, self.settings.draw_distance, &mut self.library_drag.candidates)
                    .and_then(|(_, idx)| idx)
                    .and_then(|idx| self.scene.touch_surface(&self.world, idx, ray).map(|s| (idx, s.face as u8)))
            };
            self.library_drag.target = surface;
            self.library_drag.ray = Some(ray);
            self.library_drag.checked = Some(Instant::now());
        }
        let target = self.library_drag.target;
        let allowed = target
            .and_then(|(idx, face)| self.world.objects.get(idx).map(|o| (idx, face, o)))
            .is_some_and(|(idx, face, o)| {
                !o.is_avatar()
                    && !o.is_tree()
                    && o.te.as_ref().is_some_and(|te| (face as usize) < te.faces.len())
                    && (if drag.ctrl {
                        self.build.contents_editable(&self.world, idx)
                    } else {
                        self.build.can_modify(&self.world, idx)
                    })
            });
        let protected_asset = requires_contents(&self.world.inventory, item, self.world.agent_id);
        let mut reason = None;
        if !allowed || drag.ctrl && self.world.inventory.items.get(&drag.item).is_some_and(|i| i.asset_type == 24) {
            reason = Some(aurora_i18n::tr!("texture-library-locked"));
        }
        if allowed
            && let Some((idx, _)) = target
            && (protected_asset || drag.ctrl)
        {
            if protected_asset && !drag.ctrl {
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
            && !drag.ctrl
        {
            let mut highlighted = self.build.face_selection(&self.world, &self.settings.build);
            if face < 64 {
                *highlighted.entry(idx).or_default() |= 1u64 << face;
                self.scene.set_face_selection(&mut self.world, highlighted);
            }
        }
        if drag.released {
            egui::DragAndDrop::clear_payload(ctx);
            if let Some(reason) = reason {
                self.world.system_message(reason);
                return None;
            }
            let (object, face) = target?;
            Some(Drop {
                item: drag.item,
                object,
                face,
                all: self.shift,
                contents: drag.ctrl,
                remove_pbr: self.alt,
            })
        } else {
            None
        }
    }
    pub(super) fn apply_texture_library_drop(&mut self, drop: Drop) {
        let Some(item) = catalogue::resource(&self.world.inventory, drop.item).cloned() else {
            return;
        };
        let Some(object) = self.world.objects.get(drop.object) else {
            return;
        };
        let object_id = object.full_id;
        let material = item.asset_type == 57;
        let asset = catalogue::asset(&item);
        if !drop.contents && !self.build.can_modify(&self.world, drop.object) {
            return;
        }
        let retained = self.build.contents.objects.get(&object_id).is_some_and(|inv| {
            inv.items
                .iter()
                .any(|i| i.asset_id == item.asset_id && i.asset_type as i32 == item.asset_type)
        });
        if drop.contents || requires_contents(&self.world.inventory, &item, self.world.agent_id) && !retained {
            if super::inventory_drag::target_allowed(&self.world, drop.object, &item, true).is_err() {
                return;
            }
            let task = crate::build::task_inventory::from_inventory(object_id, &item);
            self.build.send(BuildCmd::UpdateTaskInventory {
                handle: object.key.region,
                local_id: object.key.local_id,
                item: Box::new(task),
            });
            if permissions(&item, self.world.agent_id) & rules::COPY == 0 {
                self.world.inventory.consume_item(item.id);
            }
            self.build.request_contents(&self.world, drop.object, true);
        }
        if !drop.contents {
            let count = self
                .media
                .face_count(&self.world, &self.scene, drop.object)
                .unwrap_or_else(|| self.build.num_faces(&self.world, drop.object));
            self.build.face_counts.insert(object_id, count);
            let faces: Vec<_> = if drop.all {
                (0..count.min(256)).map(|i| i as u8).collect()
            } else {
                vec![drop.face]
            };
            let mut env = crate::build::ui::Env {
                images: &self.ui_images,
                media: &mut self.media,
                scene: &mut self.scene,
                zoom_fraction: self.camera.zoom_fraction(),
                demo: self.demo,
            };
            self.build
                .apply_inventory_surface(&mut self.world, &mut env, drop.object, &faces, asset, material, drop.remove_pbr);
            self.settings.texture_library.record(item.id);
            self.settings.save();
            self.world.system_message(aurora_i18n::tr!("texture-library-applied"));
        }
        self.flush_build();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_net::inventory::InvItem;
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
