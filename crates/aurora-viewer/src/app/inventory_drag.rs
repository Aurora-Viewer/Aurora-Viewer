//! Behaviour reference: LLToolDragAndDrop::{pick,dad3dRezObjectOnLand,
//! dad3dRezObjectOnObject,dropObject,willObjectAcceptInventory} and
//! FSCommon::getGroupForRezzing (Firestorm indra/newview, LGPL 2.1).
//! Aurora owns the input, geometry queries and command preparation.

use super::*;
use crate::ui::inventory::InvDrag;
use crate::world::inventory::{Inventory, actions as rules};
use aurora_net::build::{BuildCmd, TaskItem};
use aurora_net::inventory::{InvItem, rez::RezInventory};
use uuid::Uuid;

#[derive(Default)]
pub(super) struct InventoryDrag {
    candidates: Vec<usize>,
    ray: Option<(Vec3, Vec3)>,
    checked: Option<Instant>,
    surface: Option<(Vec3, Option<usize>)>,
    pub cursor: Option<egui::CursorIcon>,
}

pub(super) struct InventoryDrop {
    command: BuildCmd,
    consume: Option<Uuid>,
    selected: bool,
}

fn reason_text(reason: &str) -> &'static str {
    match reason {
        "rez-marketplace" => aurora_i18n::tr!("rez-marketplace"),
        "rez-incomplete" => aurora_i18n::tr!("rez-incomplete"),
        "rez-object-only" => aurora_i18n::tr!("rez-object-only"),
        "rez-worn" => aurora_i18n::tr!("rez-worn"),
        "rez-contents-denied" => aurora_i18n::tr!("rez-contents-denied"),
        "rez-transfer-denied" => aurora_i18n::tr!("rez-transfer-denied"),
        _ => aurora_i18n::tr!("rez-no-surface"),
    }
}

fn original(world: &World, id: Uuid) -> Result<&InvItem, &'static str> {
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
    let item = inv.items.get(&original).filter(|it| it.asset_type == 6).ok_or("rez-object-only")?;
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

fn copyable(item: &InvItem, agent: Uuid) -> bool {
    // allowCopyBy(agent), without an active-group override, like Firestorm.
    let mask = if item.owner == agent && !item.group_owned {
        item.owner_mask
    } else {
        item.everyone_mask
    };
    mask & rules::COPY != 0
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

fn target_allowed(world: &World, idx: usize, item: &InvItem, ctrl: bool) -> Result<(), &'static str> {
    let o = world.objects.get(idx).ok_or("rez-no-surface")?;
    if o.is_avatar() || o.is_tree() {
        return Err("rez-no-surface");
    }
    // Attachments are avatar targets in Firestorm. Giving / wearing through
    // world drag is a separate operation; never rez through an avatar.
    let mut current = idx;
    let mut rooted = false;
    for _ in 0..64 {
        let p = world.objects.get(current).ok_or("rez-no-surface")?;
        if p.is_avatar() {
            return Err("rez-no-surface");
        }
        if p.parent_id == 0 {
            rooted = true;
            break;
        }
        current = world.objects.parent_of(p).ok_or("rez-no-surface")?;
    }
    if !rooted {
        return Err("rez-no-surface");
    }
    if ctrl {
        use crate::ui::context::flags::{OBJECT_MODIFY, OBJECT_YOU_OWNER};
        const ALLOW_INVENTORY_DROP: u32 = 1 << 16;
        if o.pcode != aurora_prim::params::LL_PCODE_VOLUME || o.update_flags & (OBJECT_MODIFY | ALLOW_INVENTORY_DROP) == 0 {
            return Err("rez-contents-denied");
        }
        let owned = o.owner_id == world.agent_id || o.update_flags & OBJECT_YOU_OWNER != 0;
        let transfer = if item.owner == world.agent_id && !item.group_owned {
            item.owner_mask
        } else {
            item.everyone_mask
        };
        if !(owned && item.owner == world.agent_id) && transfer & rules::TRANSFER == 0 {
            return Err("rez-transfer-denied");
        }
    }
    Ok(())
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
    let item = original(world, id)?;
    let (point, target) = surface;
    if !point.is_finite() || !eye.is_finite() {
        return Err("rez-no-surface");
    }
    if let Some(idx) = target {
        target_allowed(world, idx, item, ctrl)?;
    }
    if ctrl && let Some(o) = target.and_then(|idx| world.objects.get(idx)) {
        // willObjectAcceptInventory explicitly refuses giving links.
        if world.inventory.items.get(&id).is_some_and(|it| it.asset_type == 24) {
            return Err("rez-contents-denied");
        }
        let task = TaskItem {
            item_id: item.id,
            parent_id: o.full_id,
            creator_id: item.creator,
            owner_id: item.owner,
            group_id: item.group_id,
            group_owned: item.group_owned,
            base_mask: item.base_mask,
            owner_mask: item.owner_mask,
            group_mask: item.group_mask,
            everyone_mask: item.everyone_mask,
            next_owner_mask: item.next_owner_mask,
            asset_id: item.asset_id,
            asset_type: item.asset_type as i8,
            inv_type: item.inv_type as i8,
            flags: item.flags,
            sale_type: item.sale_type,
            sale_price: item.sale_price,
            name: item.name.clone(),
            description: item.desc.clone(),
            // dropInventory dates the new contents entry at drop time.
            creation_date: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(item.created_at as i32, |d| d.as_secs() as i32),
            ..Default::default()
        };
        return Ok(InventoryDrop {
            command: BuildCmd::UpdateTaskInventory {
                handle: o.key.region,
                local_id: o.key.local_id,
                item: Box::new(task),
            },
            consume: (!copyable(item, world.agent_id)).then_some(item.id),
            selected: false,
        });
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
    pub(super) fn inventory_drag_input(&mut self, ctx: &egui::Context, renderer: &Renderer) -> Option<InventoryDrop> {
        self.inventory_drag.cursor = ui::inventory::task_drop::cursor(ctx);
        if !egui::DragAndDrop::has_payload_of_type::<InvDrag>(ctx) {
            self.inventory_drag.checked = None;
            return None;
        }
        if !matches!(self.screen, Screen::World) || self.world.teleporting || self.exit_prompt.open {
            return None;
        }
        let drag = ui::inventory::world_drag(ctx)?;
        let (pos, released, ctrl) = (drag.position, drag.released, drag.ctrl);
        let ppp = ctx.pixels_per_point();
        let ray = renderer.cursor_ray(pos.x * ppp, pos.y * ppp)?;
        let started = Instant::now();
        let item = original(&self.world, drag.item);
        if item.is_ok()
            && (released
                || self.inventory_drag.ray != Some(ray)
                || self
                    .inventory_drag
                    .checked
                    .is_none_or(|at| at.elapsed() >= Duration::from_millis(100)))
        {
            // HUD attachments are avatar targets in Firestorm, never supports
            // for rez. Block the drop rather than tracing through the HUD.
            self.inventory_drag.surface = if self.scene.hud_pick(&self.world, (pos.x * ppp, pos.y * ppp), false).is_some() {
                None
            } else {
                self.scene
                    .inventory_surface(&self.world, ray, self.settings.draw_distance, &mut self.inventory_drag.candidates)
            };
            self.inventory_drag.ray = Some(ray);
            self.inventory_drag.checked = Some(started);
        }
        let result = item.and_then(|item| {
            let surface = self.inventory_drag.surface.ok_or("rez-no-surface")?;
            // Avoid cloning inventory metadata in the frame loop.
            if let Some(idx) = surface.1 {
                target_allowed(&self.world, idx, item, ctrl)?;
            }
            if ctrl && surface.1.is_some() {
                if self.world.inventory.items.get(&drag.item).is_some_and(|it| it.asset_type == 24) {
                    return Err("rez-contents-denied");
                }
                Ok(egui::CursorIcon::Copy)
            } else if !copyable(item, self.world.agent_id) || self.world.inventory.in_folder_type(item.id, 14) {
                Ok(egui::CursorIcon::Grabbing)
            } else {
                Ok(egui::CursorIcon::Copy)
            }
        });
        self.inventory_drag.cursor = Some(result.unwrap_or(egui::CursorIcon::NotAllowed));
        if !released {
            return None;
        }
        egui::DragAndDrop::clear_payload(ctx);
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
                self.world.system_message(reason_text(reason));
                None
            }
        }
    }

    pub(super) fn apply_inventory_drop(&mut self, drop: InventoryDrop) {
        if drop.selected {
            self.build.deselect_all(&self.world);
            self.build.expect_created(&self.world);
        }
        let rez = matches!(drop.command, BuildCmd::RezInventory(_));
        if let BuildCmd::UpdateTaskInventory { item, .. } = &drop.command {
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

    #[test]
    fn ctrl_inserts_into_clicked_prim_with_real_permissions_and_transfer_rules() {
        let mut w = world();
        let target = object(&w);
        let point = w.objects.get(target).unwrap().position;
        let o = w.objects.get_mut(target).unwrap();
        o.update_flags |= crate::ui::context::flags::OBJECT_MODIFY;
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
                caps: Arc::new(Default::default()),
                voice_server: String::new(),
                avatar_hover_height: false,
                materials_limits: Default::default(),
                max_tasks: 0,
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
