//! Offline HUD fixtures use the same ObjectUpdate and touch path as the grid.
use super::*;

pub const FIRST: u32 = 8231;
pub const CHILD: u32 = 8240;

pub fn object(point: u8, active: bool) -> ObjectUpdate {
    let anchor = match point {
        32 => Vec3::new(0.0, -0.5, 0.5),
        33 => Vec3::new(0.0, 0.0, 0.5),
        34 => Vec3::new(0.0, 0.5, 0.5),
        36 => Vec3::new(0.0, 0.5, -0.5),
        37 => Vec3::new(0.0, 0.0, -0.5),
        38 => Vec3::new(0.0, -0.5, -0.5),
        _ => Vec3::ZERO,
    };
    let pos = if point == 31 {
        Vec3::new(0.0, 0.18, 0.04)
    } else if point == 35 {
        Vec3::new(0.0, -0.18, 0.04)
    } else {
        Vec3::new(0.0, -anchor.y.signum() * 0.16, -anchor.z.signum() * 0.14)
    };
    let color = if active { [0.3, 0.92, 0.65, 1.0] } else { [0.75, 0.65, 1.0, 1.0] };
    let mut texture = te(color, 0, true, 0.0);
    for face in &mut Arc::make_mut(&mut texture).faces {
        face.texture = TEX_TILES;
    }
    let mut o = prim(
        8200 + point as u32,
        pos,
        Quat::IDENTITY,
        Vec3::new(0.012, 0.24, 0.1),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        texture,
        ExtraParams::default(),
        &format!("HUD {point}{}", if active { " · Touché" } else { " · Cliquer" }),
    );
    o.parent_id = 9000;
    o.state = point.rotate_left(4);
    o.owner_id = DEMO_AGENT;
    o.update_flags = (1 << 7) | (1 << 5) | (1 << 2);
    o.name_values = format!("AttachItemID STRING RW SV {}", o.full_id);
    o
}

pub fn events() -> Vec<NetEvent> {
    let mut objects: Vec<_> = (31..=38).map(|point| object(point, false)).collect();
    let mut child = object(31, false);
    child.local_id = CHILD;
    child.full_id = u(CHILD as u128);
    child.parent_id = FIRST;
    child.state = 0;
    child.position = Vec3::new(-0.025, 0.0, -0.012);
    child.scale = Vec3::new(0.015, 0.12, 0.055);
    child.rotation = Quat::from_rotation_x(0.12);
    child.texture_entry = Some(te([0.3, 0.9, 0.8, 0.65], 0, false, 0.0));
    child.text.clear();
    objects.push(child);
    let mut other = object(35, false);
    other.local_id = 8241;
    other.full_id = u(8241);
    other.parent_id = 9001;
    other.position = Vec3::ZERO;
    other.scale = Vec3::new(0.1, 1.0, 1.0);
    other.owner_id = DEMO_LOUP;
    objects.push(other);
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

pub fn reply(cmd: &aurora_net::NetCommand) -> Option<Vec<NetEvent>> {
    let aurora_net::NetCommand::ObjectRelease { local_id, .. } = cmd else {
        return None;
    };
    let point = if *local_id == CHILD {
        31
    } else {
        u8::try_from(local_id.checked_sub(8200)?).ok()?
    };
    (31..=38).contains(&point).then(|| {
        vec![NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: vec![object(point, true)],
        }]
    })
}

pub fn seed_inventory(world: &mut crate::world::World) {
    use crate::world::appearance::{self, Action};
    let inv = &mut world.inventory;
    appearance::seed_demo(inv, world.agent_id);
    let Some(cof) = appearance::cof(inv) else { return };
    let objects: Vec<_> = appearance::folder_links(inv, cof)
        .iter()
        .filter(|link| inv.items.get(&link.target).is_some_and(|item| item.asset_type == 6))
        .map(|link| link.target)
        .collect();
    if let Ok((change, _)) = appearance::plan(inv, Action::RemoveItems(objects), &Default::default()) {
        let _ = appearance::demo_mutate(inv, world.agent_id, &change);
    }
    let Some(prototype) = inv.items.values().find(|item| item.asset_type == 6).cloned() else {
        return;
    };
    let items: Vec<_> = (31..=38)
        .map(|point| {
            let mut item = prototype.clone();
            item.id = object(point, false).full_id;
            item.name = format!("HUD de démo {point}");
            item.flags = point as u32;
            item
        })
        .collect();
    let ids: Vec<_> = items.iter().map(|item| item.id).collect();
    inv.add_items(items);
    if let Ok((change, _)) = appearance::plan(
        inv,
        Action::WearItems {
            items: ids,
            replace: false,
            point: 0,
        },
        &Default::default(),
    ) {
        let _ = appearance::demo_mutate(inv, world.agent_id, &change);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hud_detach_removes_only_its_current_outfit_link_and_keeps_inventory() {
        use crate::world::{World, appearance};
        let mut world = World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in super::super::events().into_iter().chain(events()) {
            world.apply(ev);
        }
        seed_inventory(&mut world);
        let cof = appearance::cof(&world.inventory).unwrap();
        let id = object(31, false).full_id;
        assert_eq!(world.worn_attachment_items().len(), 8);
        assert!(appearance::folder_links(&world.inventory, cof).iter().any(|link| link.target == id));
        let (change, sync) = appearance::plan(&world.inventory, appearance::Action::Remove(id), &world.worn_attachment_points()).unwrap();
        assert!(sync);
        appearance::demo_mutate(&mut world.inventory, world.agent_id, &change).unwrap();
        assert!(world.inventory.items.contains_key(&id));
        assert!(!appearance::folder_links(&world.inventory, cof).iter().any(|link| link.target == id));
        let commands = appearance::sync_commands(&world.inventory, world.agent_id, &world.worn_attachment_items());
        assert!(matches!(&commands[0], aurora_net::NetCommand::DetachAttachments(items) if items == &[id]));
        assert!(!commands.iter().any(|c| matches!(c, aurora_net::NetCommand::RezAttachments(_))));
    }
}
