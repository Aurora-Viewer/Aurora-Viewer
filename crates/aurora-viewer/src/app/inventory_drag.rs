//! Behaviour reference: LLToolDragAndDrop::{pick,dad3dRezObjectOnLand,
//! dad3dRezObjectOnObject,dropObject,willObjectAcceptInventory} and
//! FSCommon::getGroupForRezzing (Firestorm indra/newview, LGPL 2.1).
//! Aurora owns the input, geometry queries and command preparation; what
//! goes into an object's contents is decided by `build::contents_drop`.

use super::*;
pub(super) use crate::build::contents_drop::target_allowed;
use crate::build::contents_drop::{self, Target, copyable};
use crate::ui::inventory::InvDrag;
use crate::world::inventory::{Inventory, actions as rules};
use aurora_net::build::BuildCmd;
use aurora_net::inventory::{InvItem, rez::RezInventory};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct InventoryDrag {
    candidates: Vec<usize>,
    ray: Option<(Vec3, Vec3)>,
    checked: Option<Instant>,
    surface: Option<(Vec3, Option<usize>)>,
    /// whether the object under the pointer takes the cargo into its contents
    contents: contents_drop::PlanCheck,
    pub cursor: Option<egui::CursorIcon>,
}

impl InventoryDrag {
    /// The surface under the pointer (`pointer` in pixels), asked again
    /// when the ray moves, every 100 ms and on release.
    fn refresh_surface(
        &mut self,
        scene: &crate::scene::Scene,
        world: &World,
        far: f32,
        ray: (Vec3, Vec3),
        pointer: (f32, f32),
        released: bool,
    ) {
        if released || self.ray != Some(ray) || self.checked.is_none_or(|at| at.elapsed() >= Duration::from_millis(100)) {
            // HUD attachments are avatar targets in Firestorm, never supports
            // for rez. Block the drop rather than tracing through the HUD.
            self.surface = if scene.hud_pick(world, pointer, false).is_some() {
                None
            } else {
                scene.inventory_surface(world, ray, far, &mut self.candidates)
            };
            self.ray = Some(ray);
            self.checked = Some(Instant::now());
        }
    }
}

pub(super) struct InventoryDrop {
    command: BuildCmd,
    consume: Option<Uuid>,
    selected: bool,
}

fn reason_text(reason: &str) -> &'static str {
    match reason {
        "portal-bridge-required" => aurora_i18n::tr!("portal-bridge-required"),
        "portal-invalid" => aurora_i18n::tr!("portal-invalid"),
        "portal-landmark-unavailable" => aurora_i18n::tr!("portal-landmark-unavailable"),
        _ => contents_drop::reason_text(reason),
    }
}

fn portal_support(world: &World, surface: (Vec3, Option<usize>), ready: bool) -> Result<(), &'static str> {
    if let Some(idx) = surface.1 {
        contents_drop::surface_allowed(world, idx)?;
    }
    if !ready {
        return Err("portal-bridge-required");
    }
    let center = surface.0 + Vec3::Z * crate::world::portals::HEIGHT;
    if !center.is_finite()
        || !(0.0..=4096.0).contains(&center.z)
        || world
            .main()
            .is_none_or(|region| !crate::world::portals::fits_region(center, [region.heightmap.size_x, region.heightmap.size_y]))
    {
        return Err("portal-invalid");
    }
    Ok(())
}

#[cfg(test)]
fn original(world: &World, id: Uuid) -> Result<&InvItem, &'static str> {
    cargo(world, id, false)
}
fn cargo(world: &World, id: Uuid, contents: bool) -> Result<&InvItem, &'static str> {
    let inv = &world.inventory;
    if inv.in_folder_type(id, 53) {
        return Err("rez-marketplace");
    }
    let original = inv
        .items
        .get(&id)
        .filter(|it| !matches!(it.asset_type, 24 | 25))
        .map(|it| it.id)
        .or_else(|| rules::original(inv, id))
        .ok_or("rez-incomplete")?;
    let item = inv
        .items
        .get(&original)
        .filter(|it| it.asset_type == 6 || contents)
        .ok_or("rez-object-only")?;
    if inv.in_folder_type(original, 53) {
        return Err("rez-marketplace");
    }
    let avatar = world.objects.index_of_uuid(&world.agent_id).and_then(|idx| world.objects.get(idx));
    let Some(avatar) = avatar.filter(|_| world.movement_complete) else {
        return Err("rez-incomplete");
    };
    if world
        .objects
        .children_of(&avatar.key)
        .iter()
        .filter_map(|&idx| world.objects.get(idx))
        .any(|o| o.attachment_item_id() == Some(original))
    {
        return Err("rez-worn");
    }
    Ok(item)
}

fn remove_on_rez(inv: &Inventory, item: &InvItem, agent: Uuid, on_object: bool) -> bool {
    !copyable(item, agent) || (on_object && inv.in_folder_type(item.id, 14))
}

fn group_for_rez(world: &World, under_land: bool) -> Uuid {
    world
        .parcel
        .as_ref()
        .filter(|_| under_land)
        .map(|p| p.group_id)
        .filter(|id| !id.is_nil() && world.groups.groups.iter().any(|g| g.id == *id))
        .unwrap_or(world.groups.active)
}

/// The avatar an object is, or is worn by: an attachment counts as its
/// wearer (LLVOAvatar::findAvatarFromAttachment).
fn wearer(world: &World, idx: usize) -> Option<usize> {
    let mut current = idx;
    for _ in 0..64 {
        let o = world.objects.get(current)?;
        if o.is_avatar() {
            return Some(current);
        }
        if o.parent_id == 0 {
            return None;
        }
        current = world.objects.parent_of(o)?;
    }
    None
}

fn prepare(
    world: &World,
    id: Uuid,
    eye: Vec3,
    surface: (Vec3, Option<usize>),
    ctrl: bool,
    selected: bool,
    under_land: bool,
) -> Result<InventoryDrop, &'static str> {
    let (point, target) = surface;
    let item = cargo(world, id, target.is_some())?;
    // Over an object, Ctrl puts an object into its contents; the other
    // items go there without it (dad3dUpdateInventory, dad3dRezScript).
    let contents = target.is_some() && (ctrl || item.asset_type != 6);
    if !contents && item.asset_type != 6 {
        return Err("rez-object-only");
    }
    if !point.is_finite() || !eye.is_finite() {
        return Err("rez-no-surface");
    }
    if let Some(idx) = target {
        target_allowed(world, idx, item, false)?;
        if contents {
            let drop = contents_drop::plan(world, &[id], idx, ctrl, Target::World)?
                .pop()
                .ok_or("rez-contents-denied")?;
            return Ok(InventoryDrop {
                command: drop.command,
                consume: drop.consume,
                selected: false,
            });
        }
    }
    let (handle, offset) = world
        .regions
        .values()
        .find_map(|r| {
            let offset = world.region_offset(r.handle)?;
            let local = point - offset;
            (local.x >= 0.0 && local.y >= 0.0 && local.x < r.heightmap.size_x as f32 && local.y < r.heightmap.size_y as f32)
                .then_some((r.handle, offset))
        })
        .ok_or("rez-no-surface")?;
    let remove = remove_on_rez(&world.inventory, item, world.agent_id, target.is_some());
    let source_folder = if rules::library(&world.inventory, item.id) || world.inventory.in_folder_type(item.id, 14) {
        Uuid::nil()
    } else {
        item.parent
    };
    Ok(InventoryDrop {
        command: BuildCmd::RezInventory(Box::new(RezInventory {
            handle,
            group: group_for_rez(world, under_land),
            item: item.clone(),
            ray_start: eye - offset,
            ray_end: point - offset,
            target: target.and_then(|idx| world.objects.get(idx)).map_or(Uuid::nil(), |o| o.full_id),
            selected,
            remove,
            source_folder,
        })),
        consume: remove.then_some(item.id),
        selected,
    })
}

impl App {
    /// Run inside the UI pass, after all windows: egui clears released cargo
    /// at end-pass. Wants-pointer stays true during a drag from a widget;
    /// only the actual destination area decides whether the world gets it.
    pub(super) fn inventory_drag_input(&mut self, ctx: &egui::Context, renderer: &Renderer) -> Vec<InventoryDrop> {
        self.inventory_drag.cursor = ui::inventory::task_drop::cursor(ctx).or_else(|| ui::inventory::drag::cursor(ctx));
        ui::inventory::drag::badge(ctx, &self.palette);
        ui::places::drag::ghost(ctx, &self.palette);
        let place_drag = egui::DragAndDrop::has_payload_of_type::<ui::places::drag::PlaceDrag>(ctx);
        if ui::inventory::drag::carried_count(ctx).is_none() && !place_drag {
            self.inventory_drag.checked = None;
        }
        let give = self.avatar_give_input(ctx, renderer);
        self.scene.mark_give_target(&self.world, give);
        if give.is_some() || !matches!(self.screen, Screen::World) || self.world.teleporting || self.exit_prompt.open {
            return Vec::new();
        }
        if egui::DragAndDrop::payload::<ui::places::drag::PlaceDrag>(ctx)
            .is_some_and(|cargo| matches!(cargo.target, ui::places::drag::Target::History(_)))
        {
            self.history_portal_input(ctx, renderer);
            return Vec::new();
        }
        if egui::DragAndDrop::has_payload_of_type::<InvDrag>(ctx) || place_drag {
            self.item_drag_input(ctx, renderer).into_iter().collect()
        } else {
            self.selection_drag_input(ctx, renderer)
        }
    }

    /// A selection of several entries, or a folder, dragged over the world:
    /// only an object's contents take it, with Ctrl when it holds objects,
    /// textures, materials or folders (LLToolDragAndDrop::dragOrDrop3D: rez
    /// and folders accept a single entry, contents accept many).
    fn selection_drag_input(&mut self, ctx: &egui::Context, renderer: &Renderer) -> Vec<InventoryDrop> {
        let Some(ids) = ui::inventory::drag::carried_ids(ctx) else {
            return Vec::new();
        };
        if ctx.is_pointer_over_egui() {
            return Vec::new();
        }
        let Some((pos, released, ctrl)) = ctx.input(|i| {
            Some((
                i.pointer.interact_pos()?,
                i.pointer.button_released(egui::PointerButton::Primary),
                i.modifiers.ctrl,
            ))
        }) else {
            return Vec::new();
        };
        let ppp = ctx.pixels_per_point();
        let Some(ray) = renderer.cursor_ray(pos.x * ppp, pos.y * ppp) else {
            return Vec::new();
        };
        self.inventory_drag.refresh_surface(
            &self.scene,
            &self.world,
            self.settings.draw_distance,
            ray,
            (pos.x * ppp, pos.y * ppp),
            released,
        );
        let target = self.inventory_drag.surface.and_then(|(_, idx)| idx);
        let result = match target {
            Some(idx) => self
                .inventory_drag
                .contents
                .check(&self.world, &ids, idx, ctrl, Target::World)
                .map(|_| idx),
            // the land rezzes a single object
            None => Err(if ctrl { "rez-no-surface" } else { "rez-hold-ctrl" }),
        };
        self.inventory_drag.cursor = Some(if result.is_ok() {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::NotAllowed
        });
        let outline = result.ok().and_then(|idx| self.contents_outline(idx));
        self.scene.mark_give_target(&self.world, outline);
        if !released {
            return Vec::new();
        }
        egui::DragAndDrop::clear_payload(ctx);
        self.scene.mark_give_target(&self.world, None);
        // Checked again on release: the cached answer may be a few frames old.
        let result = result.and_then(|idx| contents_drop::plan(&self.world, &ids, idx, ctrl, Target::World));
        log::debug!(
            "inventory selection drop: {} entries, ctrl={ctrl}, {:?}",
            ids.len(),
            result.as_ref().map(Vec::len)
        );
        match result {
            Ok(drops) => {
                self.inventory_ui.release_world_drag_focus(ctx);
                drops
                    .into_iter()
                    .map(|drop| InventoryDrop {
                        command: drop.command,
                        consume: drop.consume,
                        selected: false,
                    })
                    .collect()
            }
            Err(reason) => {
                self.inventory_ui.message = reason_text(reason).into();
                self.world
                    .system_notice(reason_text(reason), crate::world::notice_topics::Topic::Items);
                Vec::new()
            }
        }
    }

    /// The linkset outlined while a drop would go into the prim `idx`'s
    /// contents (LLSelectMgr::highlightObjectAndFamily in dragOrDrop3D).
    fn contents_outline(&self, idx: usize) -> Option<usize> {
        let mut current = idx;
        for _ in 0..64 {
            let o = self.world.objects.get(current)?;
            if o.parent_id == 0 {
                return Some(current);
            }
            current = self.world.objects.parent_of(o)?;
        }
        None
    }

    /// History carries an arrival snapshot, never an inventory item to rez or give.
    fn history_portal_input(&mut self, ctx: &egui::Context, renderer: &Renderer) {
        let Some(drag) = ui::places::drag::history_world_drag(ctx) else {
            return;
        };
        let ppp = ctx.pixels_per_point();
        let Some(ray) = renderer.cursor_ray(drag.position.x * ppp, drag.position.y * ppp) else {
            return;
        };
        self.inventory_drag.refresh_surface(
            &self.scene,
            &self.world,
            self.settings.draw_distance,
            ray,
            (drag.position.x * ppp, drag.position.y * ppp),
            drag.released,
        );
        let ready = self.demo || self.bridge_runtime.query_endpoint().is_some();
        super::portals::landmark::preview(ctx, &self.palette, drag.position, drag.shift, ready);
        let result = self
            .inventory_drag
            .surface
            .ok_or("rez-no-surface")
            .and_then(|surface| portal_support(&self.world, surface, ready).map(|()| surface.0));
        self.inventory_drag.cursor = Some(if result.is_ok() {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::NotAllowed
        });
        if drag.released {
            egui::DragAndDrop::clear_payload(ctx);
            match result {
                Ok(point) => {
                    self.queue_history_portal(drag.global, point, drag.shift);
                    self.inventory_ui.release_world_drag_focus(ctx);
                }
                Err(reason) => self.portal_drop_notice(reason),
            }
        }
    }

    /// One inventory item dragged over the world: rezzed, put into an
    /// object's contents, or a landmark turned into a portal.
    fn item_drag_input(&mut self, ctx: &egui::Context, renderer: &Renderer) -> Option<InventoryDrop> {
        if egui::DragAndDrop::payload::<InvDrag>(ctx)
            .is_some_and(|drag| crate::ui::texture_library::catalogue::resource(&self.world.inventory, drag.0).is_some())
        {
            return None;
        }
        let drag = ui::inventory::world_drag(ctx)?;
        let (pos, released, ctrl) = (drag.position, drag.released, drag.ctrl);
        let ppp = ctx.pixels_per_point();
        let ray = renderer.cursor_ray(pos.x * ppp, pos.y * ppp)?;
        let started = Instant::now();
        let landmark = !ctrl && super::portals::landmark::cargo(&self.world.inventory, drag.item).is_some();
        let portal_ready = self.demo || self.bridge_runtime.query_endpoint().is_some();
        if landmark {
            super::portals::landmark::preview(ctx, &self.palette, pos, drag.shift, portal_ready);
        }
        let item = if landmark {
            super::portals::landmark::cargo(&self.world.inventory, drag.item).ok_or("portal-landmark-unavailable")
        } else {
            cargo(&self.world, drag.item, true)
        };
        if item.is_ok() {
            self.inventory_drag.refresh_surface(
                &self.scene,
                &self.world,
                self.settings.draw_distance,
                ray,
                (pos.x * ppp, pos.y * ppp),
                released,
            );
        }
        let mut outline = None;
        let result = item.and_then(|item| {
            let surface = self.inventory_drag.surface.ok_or("rez-no-surface")?;
            let contents = !landmark && surface.1.is_some() && (ctrl || item.asset_type != 6);
            // Avoid cloning inventory metadata in the frame loop.
            if let Some(idx) = surface.1 {
                target_allowed(&self.world, idx, item, false)?;
            }
            if landmark {
                if item.asset_id.is_nil() {
                    return Err("portal-landmark-unavailable");
                }
                portal_support(&self.world, surface, portal_ready)?;
                Ok(egui::CursorIcon::Copy)
            } else if let Some(idx) = surface.1.filter(|_| contents) {
                self.inventory_drag
                    .contents
                    .check(&self.world, &[drag.item], idx, ctrl, Target::World)?;
                outline = Some(idx);
                Ok(egui::CursorIcon::Copy)
            } else if item.asset_type != 6 {
                Err("rez-object-only")
            } else if !copyable(item, self.world.agent_id) || self.world.inventory.in_folder_type(item.id, 14) {
                Ok(egui::CursorIcon::Grabbing)
            } else {
                Ok(egui::CursorIcon::Copy)
            }
        });
        self.inventory_drag.cursor = Some(result.unwrap_or(egui::CursorIcon::NotAllowed));
        self.scene
            .mark_give_target(&self.world, outline.and_then(|idx| self.contents_outline(idx)));
        if !released {
            return None;
        }
        egui::DragAndDrop::clear_payload(ctx);
        self.scene.mark_give_target(&self.world, None);
        if landmark {
            match result {
                Ok(_) => {
                    if let Some((point, _)) = self.inventory_drag.surface {
                        self.queue_landmark_portal(drag.item, point, drag.shift);
                        self.inventory_ui.release_world_drag_focus(ctx);
                    }
                }
                Err(reason) => self.portal_drop_notice(reason),
            }
            return None;
        }
        log::debug!(
            "inventory drop target: {:?}, ctrl={ctrl}",
            self.inventory_drag.surface.map(|(point, idx)| (
                point,
                idx.and_then(|idx| self.world.objects.get(idx))
                    .map(|o| (o.key.local_id, o.update_flags))
            ))
        );
        let result = result.and_then(|_| {
            prepare(
                &self.world,
                drag.item,
                self.camera.position,
                self.inventory_drag.surface.ok_or("rez-no-surface")?,
                ctrl,
                self.build.open,
                self.settings.build.rez_under_land_group,
            )
        });
        log::debug!(
            "inventory drop: {:.3} ms, {} candidates",
            started.elapsed().as_secs_f64() * 1000.0,
            self.inventory_drag.candidates.len()
        );
        match result {
            Ok(drop) => Some(drop),
            Err(reason) => {
                self.inventory_ui.message = reason_text(reason).into();
                self.world
                    .system_notice(reason_text(reason), crate::world::notice_topics::Topic::Items);
                None
            }
        }
    }

    /// LLToolDragAndDrop::pick: inventory over another resident, or over
    /// what they wear (HUDs excepted), is given to them
    /// (dad3dGiveInventory, dad3dGiveInventoryObject,
    /// dad3dGiveInventoryCategory), the nearest surface deciding, so a wall
    /// in front hides them. The drop opens the Share confirmation of the
    /// conversations and profiles, for every entry carried; the Legacy
    /// Viewer gives a single entry, at once (#991). While they are the
    /// target, their avatar's object index, outlined by the scene. Our own
    /// avatar is left to the rez rules.
    fn avatar_give_input(&mut self, ctx: &egui::Context, renderer: &Renderer) -> Option<usize> {
        let ids = ui::inventory::drag::carried_ids(ctx)?;
        if !matches!(self.screen, Screen::World) || self.world.teleporting || self.exit_prompt.open || ctx.is_pointer_over_egui() {
            return None;
        }
        // the texture library's cards are not inventory to give
        if egui::DragAndDrop::payload::<InvDrag>(ctx)
            .is_some_and(|drag| crate::ui::texture_library::catalogue::resource(&self.world.inventory, drag.0).is_some())
        {
            return None;
        }
        let (pos, released) = ctx.input(|i| Some((i.pointer.interact_pos()?, i.pointer.button_released(egui::PointerButton::Primary))))?;
        let ppp = ctx.pixels_per_point();
        let ray = renderer.cursor_ray(pos.x * ppp, pos.y * ppp)?;
        self.inventory_drag.refresh_surface(
            &self.scene,
            &self.world,
            self.settings.draw_distance,
            ray,
            (pos.x * ppp, pos.y * ppp),
            released,
        );
        let avatar = wearer(&self.world, self.inventory_drag.surface?.1?)?;
        let to = self.world.objects.get(avatar)?.full_id;
        if to == self.world.agent_id {
            return None;
        }
        // RLVa @share, as the give itself (`apply_inventory_action`)
        let allowed = self.world.rlva.allows("share", &to.to_string());
        self.inventory_drag.cursor = Some(if allowed {
            egui::CursorIcon::Copy
        } else {
            egui::CursorIcon::NotAllowed
        });
        if released {
            egui::DragAndDrop::clear_payload(ctx);
            if allowed {
                self.inventory_ui.dialog = Some(ui::inventory::EditDialog::Share(ids, to));
                self.inventory_ui.release_world_drag_focus(ctx);
            } else {
                self.world
                    .system_notice(aurora_i18n::tr!("rlva-blocked"), crate::world::notice_topics::Topic::Warning);
            }
        }
        Some(avatar)
    }

    pub(super) fn apply_inventory_drop(&mut self, drop: InventoryDrop) {
        if drop.selected {
            self.build.deselect_all(&self.world);
            self.build.expect_created(&self.world);
        }
        let rez = matches!(drop.command, BuildCmd::RezInventory(_));
        if let BuildCmd::RezInventory(item) = &drop.command {
            self.record_library_use(item.item.id);
        }
        if let BuildCmd::UpdateTaskInventory { item, .. } | BuildCmd::RezScript { item, .. } = &drop.command {
            self.record_library_use(item.item_id);
            self.build.contents_changed(item.parent_id);
            if self.build.open {
                self.build.tab = 4;
            }
        }
        log::debug!(
            "inventory drop queued: {}, consumed={}",
            if rez { "rez" } else { "contents" },
            drop.consume.is_some()
        );
        self.send(NetCommand::Build(drop.command));
        if let Some(id) = drop.consume {
            self.world.inventory.consume_item(id);
        }
        if rez {
            self.world.ui_sounds.push(UiSound::ObjectRezIn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world() -> World {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
            w.apply(ev);
        }
        crate::world::inventory::demo::seed(&mut w.inventory, w.agent_id);
        w
    }

    fn item(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn object(w: &World) -> usize {
        w.objects.index_of_uuid(&crate::demo::action_id(970)).expect("demo target")
    }

    fn rez(drop: InventoryDrop) -> RezInventory {
        let BuildCmd::RezInventory(rez) = drop.command else {
            panic!("expected RezObject")
        };
        *rez
    }

    #[test]
    fn copy_and_nocopy_land_and_trash_object_rules() {
        let mut w = world();
        let point = Vec3::new(140.0, 130.0, 32.0);
        let eye = point + Vec3::new(3.0, 0.0, 4.0);
        let copy = prepare(&w, item(8100), eye, (point, None), false, false, true).unwrap();
        assert!(copy.consume.is_none());
        let copy = rez(copy);
        assert!(!copy.remove && !copy.selected);
        assert_eq!(copy.source_folder, item(8000));
        // Ctrl on terrain still rezzes; Shift has no effect on rez permissions.
        let protected = prepare(&w, item(8109), eye, (point, None), true, true, true).unwrap();
        assert_eq!(protected.consume, Some(item(8109)));
        assert!(rez(protected).remove);
        let trash = rules::system(&w.inventory, 14).expect("trash");
        w.inventory.items.get_mut(&item(8100)).unwrap().parent = trash;
        let land = rez(prepare(&w, item(8100), eye, (point, None), false, false, true).unwrap());
        assert!(!land.remove);
        assert!(land.source_folder.is_nil());
        let object = rez(prepare(&w, item(8100), eye, (point, Some(object(&w))), false, false, true).unwrap());
        assert!(object.remove);
        assert!(object.source_folder.is_nil());
    }

    #[test]
    fn library_object_cards_rez_the_actual_inventory_item_with_hidden_assets_and_group_flags() {
        let mut w = world();
        let mut scene = crate::scene::Scene::new(std::env::temp_dir(), w.avatar_lib.clone());
        crate::demo::texture_library::seed(&mut w.inventory, &mut scene, w.agent_id, false);
        let mut catalogue = crate::ui::texture_library::catalogue::Catalogue::default();
        catalogue.prepare(&mut w.inventory);
        let point = Vec3::new(140.0, 130.0, 32.0);
        let eye = point + Vec3::Z * 4.0;
        let objects: Vec<_> = catalogue.entries.iter().filter(|entry| entry.kind.object()).collect();
        assert_eq!(objects.len(), 4);
        for entry in objects {
            assert!(crate::ui::texture_library::catalogue::resource(&w.inventory, entry.id).is_none());
            for target in [None, Some(object(&w))] {
                let drop = prepare(&w, entry.id, eye, (point, target), false, false, true).expect("library rez");
                let item = &w.inventory.items[&entry.id];
                assert_eq!(drop.consume, (item.owner_mask & rules::COPY == 0).then_some(item.id));
                let request = rez(drop);
                assert_eq!(request.item.id, entry.id);
                assert_eq!(request.item.flags, item.flags);
                assert_eq!(request.item.parent, item.parent);
                assert_eq!(request.item.owner_mask, item.owner_mask);
                assert_eq!(request.source_folder, item.parent);
                assert!(request.item.asset_id.is_nil());
                assert_eq!(request.ray_end, point);
                assert_eq!(
                    request.target,
                    target.map_or(Uuid::nil(), |idx| w.objects.get(idx).unwrap().full_id)
                );
            }
        }
    }

    #[test]
    fn consume_blocks_second_rez_and_server_update_restores_original() {
        let mut w = world();
        let original = w.inventory.items[&item(8109)].clone();
        let generation = w.inventory.generation;
        w.inventory.consume_item(original.id);
        assert!(prepare(&w, original.id, Vec3::ZERO, (Vec3::ONE, None), false, false, true).is_err());
        assert!(!w.inventory.folders[&original.parent].items.contains(&original.id));
        assert!(w.inventory.items.contains_key(&item(8100)));
        assert_eq!(w.inventory.generation, generation + 1);
        w.inventory.add_items(vec![original.clone()]);
        assert_eq!(w.inventory.items[&original.id].owner_mask, original.owner_mask);
        assert!(w.inventory.folders[&original.parent].items.contains(&original.id));
    }

    #[test]
    fn worn_marketplace_broken_links_and_nonobjects_are_refused() {
        let mut w = world();
        assert_eq!(original(&w, item(8102)).unwrap_err(), "rez-object-only");
        w.inventory.items.get_mut(&item(8100)).unwrap().parent = item(8002);
        assert_eq!(original(&w, item(8100)).unwrap_err(), "rez-marketplace");
        w.inventory.items.get_mut(&item(8100)).unwrap().parent = item(8000);
        let attach = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: item(8100),
            owner_id: w.agent_id,
            point: 1,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: rules::TRANSFER,
            name: "Synthetic attachment".into(),
            desc: String::new(),
        });
        w.apply(NetEvent::ObjectUpdates {
            handle: w.main_region.unwrap(),
            objects: vec![attach],
        });
        assert_eq!(original(&w, item(8100)).unwrap_err(), "rez-worn");
        w.inventory.add_items(vec![InvItem {
            id: item(8999),
            parent: item(8000),
            asset_type: 24,
            asset_id: item(8998),
            ..Default::default()
        }]);
        assert_eq!(original(&w, item(8999)).unwrap_err(), "rez-incomplete");
    }

    /// A drop on what a resident wears is a drop on them; a free object is
    /// nobody's (#991).
    #[test]
    fn an_attachment_stands_for_its_wearer_and_a_free_object_for_nobody() {
        let mut w = world();
        let attach = crate::demo::outfit_attachment(&aurora_net::AttachRequest {
            item_id: item(8100),
            owner_id: w.agent_id,
            point: 1,
            add: true,
            flags: 0,
            group_mask: 0,
            everyone_mask: 0,
            next_owner_mask: rules::TRANSFER,
            name: "Synthetic attachment".into(),
            desc: String::new(),
        });
        let attach_id = attach.full_id;
        w.apply(NetEvent::ObjectUpdates {
            handle: w.main_region.unwrap(),
            objects: vec![attach],
        });
        let avatar = w.objects.index_of_uuid(&w.agent_id).expect("our avatar");
        let worn = w.objects.index_of_uuid(&attach_id).expect("the attachment");
        assert_eq!(wearer(&w, worn), Some(avatar));
        assert_eq!(wearer(&w, avatar), Some(avatar));
        assert_eq!(wearer(&w, object(&w)), None);
    }

    #[test]
    fn ctrl_inserts_into_clicked_prim_with_real_permissions_and_transfer_rules() {
        let mut w = world();
        let target = object(&w);
        let point = w.objects.get(target).unwrap().position;
        let o = w.objects.get_mut(target).unwrap();
        o.update_flags |= crate::ui::context::flags::OBJECT_MODIFY | crate::ui::context::flags::OBJECT_YOU_OWNER;
        o.owner_id = w.agent_id;
        w.inventory.items.get_mut(&item(8110)).unwrap().created_at = 1;
        let drop = prepare(&w, item(8110), Vec3::ZERO, (point, Some(target)), true, true, true).unwrap();
        assert!(!drop.selected && drop.consume.is_none());
        let BuildCmd::UpdateTaskInventory { item: task, local_id, .. } = drop.command else {
            panic!("expected content insertion")
        };
        assert_eq!(local_id, w.objects.get(target).unwrap().key.local_id);
        let original = &w.inventory.items[&item(8110)];
        assert_eq!(task.owner_mask, original.owner_mask);
        assert_eq!(task.base_mask, original.base_mask);
        assert_eq!(task.next_owner_mask, original.next_owner_mask);
        assert!(task.creation_date > 1);
        assert_eq!(original.created_at, 1);
        w.objects.get_mut(target).unwrap().owner_id = item(8990);
        w.objects.get_mut(target).unwrap().update_flags &= !crate::ui::context::flags::OBJECT_YOU_OWNER;
        assert_eq!(
            prepare(&w, item(8110), Vec3::ZERO, (point, Some(target)), true, false, true).err(),
            Some("rez-transfer-denied")
        );
        w.objects.get_mut(target).unwrap().update_flags = 0;
        assert_eq!(
            prepare(&w, item(8100), Vec3::ZERO, (point, Some(target)), true, false, true).err(),
            Some("rez-contents-denied")
        );
        // No permission to edit the support is needed for ordinary rez.
        assert!(prepare(&w, item(8100), Vec3::ZERO, (point, Some(target)), false, false, true).is_ok());
    }

    #[test]
    fn a_single_item_other_than_an_object_goes_into_the_contents_without_ctrl() {
        let mut w = world();
        let target = object(&w);
        let point = w.objects.get(target).unwrap().position;
        let o = w.objects.get_mut(target).unwrap();
        o.update_flags |= crate::ui::context::flags::OBJECT_MODIFY | crate::ui::context::flags::OBJECT_YOU_OWNER;
        o.owner_id = w.agent_id;
        // dad3dUpdateInventory / dad3dRezScript: no Ctrl needed, scripts run.
        let drop = prepare(&w, item(8103), Vec3::ZERO, (point, Some(target)), false, false, true).unwrap();
        assert!(matches!(drop.command, BuildCmd::UpdateTaskInventory { ref item, .. } if item.item_id == Uuid::from_u128(8103)));
        let drop = prepare(&w, item(8102), Vec3::ZERO, (point, Some(target)), false, false, true).unwrap();
        assert!(matches!(drop.command, BuildCmd::RezScript { enabled: true, .. }));
        // An object still rezzes, and nothing but an object goes on the land.
        assert!(matches!(
            prepare(&w, item(8100), Vec3::ZERO, (point, Some(target)), false, false, true)
                .unwrap()
                .command,
            BuildCmd::RezInventory(_)
        ));
        assert_eq!(
            prepare(&w, item(8103), Vec3::ZERO, (point, None), false, false, true).err(),
            Some("rez-object-only")
        );
    }

    #[test]
    fn shared_contents_drop_inherits_root_rights_and_retains_transfer_restrictions() {
        use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_YOU_OWNER};
        let mut w = world();
        let root = object(&w);
        let child = w.objects.index_of_uuid(&crate::demo::action_id(971)).unwrap();
        let root_id = w.objects.get(root).unwrap().key.local_id;
        let o = w.objects.get_mut(root).unwrap();
        o.owner_id = item(8990);
        o.update_flags = OBJECT_MODIFY;
        let o = w.objects.get_mut(child).unwrap();
        o.parent_id = root_id;
        o.owner_id = item(8990);
        o.update_flags = 0;
        assert!(target_allowed(&w, child, &w.inventory.items[&item(8100)], true).is_ok());
        assert_eq!(
            target_allowed(&w, child, &w.inventory.items[&item(8110)], true),
            Err("rez-transfer-denied")
        );
        w.objects.get_mut(root).unwrap().update_flags = 0;
        w.objects.get_mut(child).unwrap().update_flags = OBJECT_MODIFY | OBJECT_YOU_OWNER;
        assert_eq!(
            target_allowed(&w, child, &w.inventory.items[&item(8100)], true),
            Err("rez-contents-denied")
        );
    }

    #[test]
    fn library_source_and_neighbor_region_coordinates() {
        let mut w = world();
        let source = w.inventory.lib_root;
        let it = w.inventory.items.get_mut(&item(8100)).unwrap();
        it.parent = source;
        it.owner = w.inventory.lib_owner;
        it.everyone_mask = rules::COPY;
        let main = w.main_region.unwrap();
        let (x, y) = aurora_net::handle_to_origin(main);
        let handle = ((x as u64 + 256) << 32) | y as u64;
        w.regions.insert(
            handle,
            crate::world::Region {
                handle,
                name: "Synthetic neighbor".into(),
                info: None,
                heightmap: crate::world::terrain::Heightmap::new(256, 256),
                wind: Default::default(),
                dirty_chunks: HashSet::new(),
                terrain_revision: 0,
                terrain_surface: Default::default(),
                caps: Arc::new(Default::default()),
                voice_server: String::new(),
                avatar_hover_height: false,
                lsl_syntax_id: None,
                materials_limits: Default::default(),
                max_tasks: 0,
                grid_status: Default::default(),
            },
        );
        let r = rez(prepare(
            &w,
            item(8100),
            Vec3::new(245.0, 20.0, 40.0),
            (Vec3::new(270.0, 30.0, 32.0), None),
            false,
            false,
            true,
        )
        .unwrap());
        assert_eq!(r.handle, handle);
        assert_eq!(r.ray_start, Vec3::new(-11.0, 20.0, 40.0));
        assert_eq!(r.ray_end, Vec3::new(14.0, 30.0, 32.0));
        assert!(r.source_folder.is_nil() && !r.remove);
        assert_eq!(w.inventory.items[&item(8100)].parent, source);
    }

    #[test]
    fn rez_uses_agent_parcel_group_only_for_members_when_enabled() {
        let mut w = world();
        for ev in crate::demo::demo_reply(&NetCommand::RequestGroups) {
            w.apply(ev);
        }
        w.groups.active = item(8800);
        let parcel_group = w.groups.groups[0].id;
        w.parcel = Some(Arc::new(aurora_net::ParcelInfo {
            group_id: parcel_group,
            ..Default::default()
        }));
        assert_eq!(group_for_rez(&w, true), parcel_group);
        assert_eq!(group_for_rez(&w, false), item(8800));
        Arc::make_mut(w.parcel.as_mut().unwrap()).group_id = item(8899);
        assert_eq!(group_for_rez(&w, true), item(8800));
        assert!(crate::build::BuildSettings::default().rez_under_land_group);
    }
}

#[cfg(test)]
mod library_contents_tests {
    use super::*;
    #[test]
    fn non_object_library_assets_can_only_drop_into_permitted_contents() {
        let mut w = World::new(Arc::new(AvatarLibrary::load()));
        for event in crate::demo::events().into_iter().chain(crate::demo::action_events()) {
            w.apply(event);
        }
        crate::world::inventory::demo::seed(&mut w.inventory, w.agent_id);
        let target = w.objects.index_of_uuid(&crate::demo::action_id(970)).unwrap();
        let o = w.objects.get_mut(target).unwrap();
        o.owner_id = w.agent_id;
        o.update_flags |= crate::ui::context::flags::OBJECT_MODIFY;
        let point = o.position;
        for kind in [1, 3, 7, 10, 20] {
            let id = Uuid::from_u128(50000 + kind as u128);
            w.inventory.add_items(vec![InvItem {
                id,
                parent: w.inventory.root,
                asset_type: kind,
                owner: w.agent_id,
                owner_mask: rules::COPY | rules::TRANSFER,
                ..Default::default()
            }]);
            assert!(prepare(&w, id, Vec3::ZERO, (point, None), true, false, true).is_err());
            let drop = prepare(&w, id, Vec3::ZERO, (point, Some(target)), true, false, true).unwrap();
            let task = match drop.command {
                BuildCmd::RezScript { enabled, item, .. } if kind == 10 => {
                    assert!(!enabled);
                    item
                }
                BuildCmd::UpdateTaskInventory { item, .. } if kind != 10 => item,
                _ => panic!("wrong inventory drop command"),
            };
            assert_eq!(task.item_id, id);
            assert_eq!(i32::from(task.asset_type), kind);
            assert!(drop.consume.is_none());
            w.inventory.items.get_mut(&id).unwrap().owner_mask &= !rules::COPY;
            let drop = prepare(&w, id, Vec3::ZERO, (point, Some(target)), true, false, true).unwrap();
            assert_eq!(drop.consume, Some(id));
            // AllowInventoryDrop accepts ordinary cargo without Modify, but
            // RezScript still requires the linkset root's effective Modify.
            w.objects.get_mut(target).unwrap().update_flags = 1 << 16;
            let drop = prepare(&w, id, Vec3::ZERO, (point, Some(target)), true, false, true);
            if kind == 10 {
                assert_eq!(drop.err(), Some("rez-contents-denied"));
            } else {
                assert!(drop.is_ok());
            }
            w.objects.get_mut(target).unwrap().update_flags |= crate::ui::context::flags::OBJECT_MODIFY;
        }
    }
}
