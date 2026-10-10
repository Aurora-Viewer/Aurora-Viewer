//! Offline HUD fixtures use the same ObjectUpdate and touch path as the grid.
use super::*;
use std::time::Instant;

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
    o.update_flags = (1 << 8) | (1 << 7) | (1 << 5) | (1 << 2);
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

const MAT_DOUBLE_OPAQUE: Uuid = Uuid::from_u128(8260);
const MAT_DOUBLE_BLEND: Uuid = Uuid::from_u128(8261);
const MAT_DOUBLE_MASK: Uuid = Uuid::from_u128(8262);
const MAT_SINGLE_MASK: Uuid = Uuid::from_u128(8263);

/// Front / back / explicitly double-sided panels, also after a half turn.
/// All other box faces are transparent, as in a folding scripted HUD.
pub fn face_events(turned: bool) -> Vec<NetEvent> {
    let mut objects = Vec::new();
    for row in 0..3 {
        for col in 0..3 {
            let id = 8270 + row * 3 + col;
            let mut o = object(31, false);
            o.local_id = id;
            o.full_id = u(id as u128);
            o.name_values = format!("AttachItemID STRING RW SV {}", o.full_id);
            o.position = Vec3::new(0.0, 0.33 - col as f32 * 0.33, 0.25 - row as f32 * 0.25);
            o.scale = Vec3::new(0.012, 0.24, 0.13);
            o.rotation = Quat::from_rotation_z(if (col != 0) ^ turned { std::f32::consts::PI } else { 0.0 });
            let kind = ["Opaque", "Translucide", "Masqué"][row as usize];
            let side = ["Avant", "Arrière", "Double face"][col as usize];
            o.text = format!("{kind} · {side}");
            o.texture_entry = Some(te([0.0; 4], 0, true, 0.0));
            let face = &mut Arc::make_mut(o.texture_entry.as_mut().expect("fixture texture")).faces[4];
            face.color = [0.35, 0.65, 1.0, if row == 1 { 0.45 } else { 1.0 }];
            face.texture = TEX_TILES;
            let mat = match (row, col) {
                (0, 2) => Some(MAT_DOUBLE_OPAQUE),
                (1, 2) => Some(MAT_DOUBLE_BLEND),
                (2, 2) => Some(MAT_DOUBLE_MASK),
                (2, _) => Some(MAT_SINGLE_MASK),
                _ => None,
            };
            if let Some(mat) = mat {
                o.extra.render_materials = vec![(4, mat)];
            }
            objects.push(o);
        }
    }
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

pub fn face_materials() -> Vec<(Uuid, aurora_assets::PbrMaterial)> {
    use aurora_assets::{AlphaMode, PbrMaterial};
    [
        (MAT_DOUBLE_OPAQUE, AlphaMode::Opaque, true),
        (MAT_DOUBLE_BLEND, AlphaMode::Blend, true),
        (MAT_DOUBLE_MASK, AlphaMode::Mask, true),
        (MAT_SINGLE_MASK, AlphaMode::Mask, false),
    ]
    .into_iter()
    .map(|(id, alpha_mode, double_sided)| {
        (
            id,
            PbrMaterial {
                base_color_texture: Some(TEX_TILES),
                base_color_factor: [0.35, 0.9, 0.65, if alpha_mode == AlphaMode::Blend { 0.45 } else { 1.0 }],
                alpha_mode,
                double_sided,
                ..Default::default()
            },
        )
    })
    .collect()
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

/// A scripted color change must not reset a HUD repositioned in the demo.
pub fn preserve_touch_transform(world: &crate::world::World, event: &mut NetEvent) {
    let NetEvent::ObjectUpdates { objects, .. } = event else { return };
    for update in objects {
        if let Some(idx) = world.objects.index_of_uuid(&update.full_id)
            && crate::scene::Scene::object_transform(world, idx, Instant::now(), 0).is_some_and(|(_, _, hud)| hud)
            && let Some(current) = world.objects.get(idx)
        {
            update.position = current.position;
            update.rotation = current.rotation;
            update.scale = current.scale;
        }
    }
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
    fn folding_panels_have_only_one_visible_face_and_opposite_orientations() {
        let NetEvent::ObjectUpdates { objects, .. } = face_events(false).remove(0) else {
            panic!("fixture update")
        };
        let mesh = aurora_prim::generate_volume(&objects[0].volume, 1.0);
        let f = &mesh.faces[4];
        let [a, b, c] = [f.indices[0], f.indices[1], f.indices[2]].map(|i| Vec3::from_array(f.positions[i as usize]));
        let normal = (b - a).cross(c - a).normalize();
        assert!(normal.dot(-Vec3::X) > 0.99);
        for (i, o) in objects.iter().enumerate() {
            let col = i % 3;
            assert_eq!(
                o.texture_entry.as_ref().unwrap().faces.iter().filter(|f| f.color[3] > 0.0).count(),
                1
            );
            assert!((o.rotation * normal).dot(Vec3::X) * if col == 0 { -1.0 } else { 1.0 } > 0.99);
        }
    }

    #[test]
    fn touch_color_change_keeps_the_edited_hud_transform() {
        let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        for ev in super::super::events().into_iter().chain(events()) {
            world.apply(ev);
        }
        let idx = world.objects.index_of_uuid(&object(31, false).full_id).unwrap();
        let pos = Vec3::new(0.01, -0.12, 0.18);
        let rot = Quat::from_rotation_x(0.3);
        let scale = Vec3::new(0.02, 0.3, 0.15);
        let o = world.objects.get_mut(idx).unwrap();
        o.position = pos;
        o.rotation = rot;
        o.scale = scale;
        let mut replies = reply(&aurora_net::NetCommand::ObjectRelease {
            handle: HANDLE,
            local_id: CHILD,
            surface: Default::default(),
        })
        .unwrap();
        for ev in &mut replies {
            preserve_touch_transform(&world, ev);
        }
        for ev in replies {
            world.apply(ev);
        }
        let o = world.objects.get(idx).unwrap();
        assert_eq!((o.position, o.rotation, o.scale), (pos, rot, scale));
        assert!(o.text.contains("Touché"));
    }

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
