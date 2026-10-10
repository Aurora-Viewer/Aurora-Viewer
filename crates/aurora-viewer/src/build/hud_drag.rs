//! Direct HUD translation. The Alt gesture is an Aurora shortcut; the HUD
//! coordinates and final position update follow LLManipTranslate::handleMouseUp
//! and LLSelectMgr::sendMultipleUpdate (Firestorm, originally LGPL 2.1).
use super::{BuildCmd, ObjKey, TransformUpdate, World, root_of};
use crate::scene::Scene;
use aurora_render::HudView;
use glam::{Quat, Vec3};
use std::time::Instant;
use uuid::Uuid;

/// FLAGS_OBJECT_MOVE: moving a whole HUD does not require modify permission.
const OBJECT_MOVE: u32 = 1 << 8;

pub struct HudDrag {
    key: ObjKey,
    id: Uuid,
    point: u8,
    start_position: Vec3,
    /// Keep the grabbed point under the pointer, including after resize / zoom.
    grab_offset: Vec3,
    depth: f32,
}

fn movable_root(world: &World, idx: usize) -> Option<usize> {
    let root = root_of(world, idx);
    let o = world.objects.get(root)?;
    let parent = world.objects.get(world.objects.parent_of(o)?)?;
    (parent.is_avatar()
        && parent.full_id == world.agent_id
        && (31..=38).contains(&o.attachment_point())
        && o.update_flags & OBJECT_MOVE != 0)
        .then_some(root)
}

impl HudDrag {
    pub fn can_start(world: &World, idx: usize) -> bool {
        movable_root(world, idx).is_some()
    }

    pub fn start(world: &World, idx: usize, view: HudView, cursor: (f32, f32)) -> Option<Self> {
        let root = movable_root(world, idx)?;
        let o = world.objects.get(root)?;
        let (position, _, hud) = Scene::object_transform(world, root, Instant::now(), 0)?;
        if !hud || !position.is_finite() || !o.position.is_finite() {
            return None;
        }
        let offset = position - view.ray(cursor.0, cursor.1).0;
        Some(Self {
            key: o.key,
            id: o.full_id,
            point: o.attachment_point(),
            start_position: o.position,
            grab_offset: Vec3::new(0.0, offset.y, offset.z),
            depth: position.x,
        })
    }

    fn index(&self, world: &World) -> Option<usize> {
        let idx = world.objects.index_of(&self.key)?;
        let o = world.objects.get(idx)?;
        (o.full_id == self.id && o.attachment_point() == self.point && movable_root(world, idx) == Some(idx)).then_some(idx)
    }

    /// Local echo only; no simulator update until the button is released.
    /// False means the HUD was detached, replaced or can no longer be moved.
    pub fn update(&self, world: &mut World, view: HudView, cursor: (f32, f32)) -> bool {
        let Some(idx) = self.index(world) else { return false };
        let mut desired = view.ray(cursor.0, cursor.1).0 + self.grab_offset;
        desired.x = self.depth;
        let (mut anchor, rotation) = world
            .avatar_lib
            .attach_points
            .get(&self.point)
            .map(|a| (a.position, a.rotation))
            .unwrap_or((Vec3::ZERO, Quat::IDENTITY));
        anchor.y *= world.hud_aspect;
        let position = rotation.inverse() * (desired - anchor);
        if !position.is_finite() {
            return false;
        }
        let Some(o) = world.objects.get_mut(idx) else { return false };
        o.position = position;
        o.velocity = Vec3::ZERO;
        o.acceleration = Vec3::ZERO;
        o.angular_velocity = Vec3::ZERO;
        o.render.needs_records = true;
        true
    }

    pub fn release(self, world: &World) -> Option<BuildCmd> {
        let o = world.objects.get(self.index(world)?)?;
        if o.position.abs_diff_eq(self.start_position, 1e-6) {
            return None;
        }
        Some(BuildCmd::Transform {
            handle: self.key.region,
            updates: vec![TransformUpdate {
                local_id: self.key.local_id,
                position: Some(o.position),
                rotation: None,
                scale: None,
                linked: true,
                uniform: false,
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::sync::Arc;

    fn fixture() -> (World, usize, usize) {
        let mut world = World::new(Arc::new(AvatarLibrary::load()));
        for ev in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
            world.apply(ev);
        }
        let root = world.objects.index_of_uuid(&crate::demo::hud::object(31, false).full_id).unwrap();
        let child = world
            .objects
            .iter()
            .find(|(_, o)| o.key.local_id == crate::demo::hud::CHILD)
            .unwrap()
            .0;
        (world, root, child)
    }

    #[test]
    fn child_drag_moves_linkset_and_saves_only_root_position_on_release() {
        let (mut world, root, child) = fixture();
        let now = Instant::now();
        let o = world.objects.get(root).unwrap();
        let (key, position, rotation, scale) = (o.key, o.position, o.rotation, o.scale);
        let child_position = world.objects.get(child).unwrap().position;
        let child_hud = Scene::object_transform(&world, child, now, 0).unwrap().0;
        let view = HudView::new([1280, 720], 1.0, -1.0, 1.0);
        let drag = HudDrag::start(&world, child, view, (500.0, 350.0)).unwrap();
        assert!(drag.update(&mut world, view, (644.0, 278.0)));
        let delta = Vec3::new(0.0, -0.2, 0.1);
        let o = world.objects.get(root).unwrap();
        assert!(o.position.abs_diff_eq(position + delta, 1e-5));
        assert_eq!(o.rotation, rotation);
        assert_eq!(o.scale, scale);
        assert_eq!(world.objects.get(child).unwrap().position, child_position);
        assert!(
            Scene::object_transform(&world, child, now, 0)
                .unwrap()
                .0
                .abs_diff_eq(child_hud + delta, 1e-5)
        );
        let BuildCmd::Transform { handle, updates } = drag.release(&world).unwrap() else {
            panic!("expected transform")
        };
        assert_eq!(handle, key.region);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].local_id, key.local_id);
        assert_eq!(
            updates[0].kind(),
            aurora_net::build::upd::POSITION | aurora_net::build::upd::LINKED_SETS
        );
        assert!(updates[0].position.unwrap().abs_diff_eq(position + delta, 1e-5));
        assert_eq!(updates[0].data().len(), 12);
    }

    #[test]
    fn keeps_grab_point_at_zoom_and_resize_with_rotated_attachment_anchor() {
        let mut lib = AvatarLibrary::load();
        lib.attach_points.get_mut(&32).unwrap().rotation = Quat::from_rotation_x(0.7);
        let mut world = World::new(Arc::new(lib));
        for ev in crate::demo::events().into_iter().chain(crate::demo::hud::events()) {
            world.apply(ev);
        }
        world.hud_aspect = 1280.0 / 720.0;
        let root = world.objects.index_of_uuid(&crate::demo::hud::object(32, false).full_id).unwrap();
        let view = HudView::new([1280, 720], 0.5, -1.0, 1.0);
        let pos = Scene::object_transform(&world, root, Instant::now(), 0).unwrap().0;
        let cursor = (
            view.width * 0.5 - pos.y * view.height * view.zoom + 10.0,
            view.height * 0.5 - pos.z * view.height * view.zoom,
        );
        let drag = HudDrag::start(&world, root, view, cursor).unwrap();
        assert!(drag.update(&mut world, view, (cursor.0 + 72.0, cursor.1 - 36.0)));
        let moved = Scene::object_transform(&world, root, Instant::now(), 0).unwrap().0;
        assert!(moved.abs_diff_eq(pos + Vec3::new(0.0, -0.2, 0.1), 1e-5));
        world.hud_aspect = 1.0;
        let resized = HudView::new([900, 900], 1.0, -2.0, 3.0);
        assert!(drag.update(&mut world, resized, (600.0, 300.0)));
        let moved = Scene::object_transform(&world, root, Instant::now(), 0).unwrap().0;
        assert!((moved.y - (resized.ray(600.0, 300.0).0.y + drag.grab_offset.y)).abs() < 1e-5);
        assert!((moved.z - (resized.ray(600.0, 300.0).0.z + drag.grab_offset.z)).abs() < 1e-5);
        assert!((moved.x - pos.x).abs() < 1e-5);
    }

    #[test]
    fn no_modify_hud_is_movable_but_locked_foreign_and_world_objects_are_not() {
        let (mut world, root, child) = fixture();
        world.objects.get_mut(root).unwrap().update_flags &= !(1 << 2);
        assert!(HudDrag::can_start(&world, child));
        world.objects.get_mut(root).unwrap().update_flags &= !OBJECT_MOVE;
        assert!(!HudDrag::can_start(&world, child));
        let foreign = world.objects.iter().find(|(_, o)| o.key.local_id == 8241).unwrap().0;
        world.objects.get_mut(foreign).unwrap().update_flags |= OBJECT_MOVE;
        assert!(!HudDrag::can_start(&world, foreign));
        let avatar = world.objects.index_of_uuid(&world.agent_id).unwrap();
        assert!(!HudDrag::can_start(&world, avatar));
        world.objects.get_mut(root).unwrap().parent_id = 0;
        assert!(!HudDrag::can_start(&world, child));
    }

    #[test]
    fn click_without_motion_does_not_save_and_detachment_cancels_drag() {
        let (mut world, root, child) = fixture();
        let view = HudView::new([1280, 720], 1.0, -1.0, 1.0);
        let drag = HudDrag::start(&world, child, view, (500.0, 350.0)).unwrap();
        assert!(drag.update(&mut world, view, (500.0, 350.0)));
        assert!(drag.release(&world).is_none());
        let drag = HudDrag::start(&world, child, view, (500.0, 350.0)).unwrap();
        assert!(drag.update(&mut world, view, (600.0, 350.0)));
        world.objects.get_mut(root).unwrap().parent_id = 0;
        assert!(!drag.update(&mut world, view, (700.0, 350.0)));
        assert!(drag.release(&world).is_none());
    }

    #[test]
    fn stale_server_echo_does_not_change_the_drag_origin() {
        let (mut world, root, child) = fixture();
        let view = HudView::new([1280, 720], 1.0, -1.0, 1.0);
        let drag = HudDrag::start(&world, child, view, (500.0, 350.0)).unwrap();
        assert!(drag.update(&mut world, view, (600.0, 350.0)));
        let position = world.objects.get(root).unwrap().position;
        world.objects.get_mut(root).unwrap().position = Vec3::splat(10.0);
        assert!(drag.update(&mut world, view, (600.0, 350.0)));
        assert!(world.objects.get(root).unwrap().position.abs_diff_eq(position, 1e-6));
        world.objects.get_mut(root).unwrap().full_id = Uuid::nil();
        assert!(!drag.update(&mut world, view, (700.0, 350.0)));
        assert!(drag.release(&world).is_none());
    }
}
