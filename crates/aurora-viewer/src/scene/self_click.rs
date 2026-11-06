//! Whether a left click lands on our own avatar: the press then steers it
//! (LLToolPie::handleLeftClickPick: our avatar picked, LLToolCamera's mouse
//! steering and ClickOnAvatarKeepsCamera) instead of acting on what the ray
//! meets.
//!
//! Like LLVOAvatar::lineSegmentIntersect (indra/newview/llvoavatar.cpp),
//! our avatar is where the ray meets its collision volumes, posed as last
//! drawn, or a surface the GPU drew of it there: its body, or a prim it
//! wears (an attachment without a touch script leads to its avatar,
//! handleLeftClickPick). A rigged mesh we wear counts only over the volumes
//! (the Legacy Viewer's left pick never sees a rigged face): big hair above
//! our head does not hide the table game behind it. Anything the click
//! found in front of that is not our avatar's: it takes the click.
//!
//! An attachment the Legacy Viewer's transparent pick finds comes before any
//! avatar (LLToolPie::handleMouseDown, priority 1; LLPipeline::
//! lineSegmentIntersectInWorld keeps an attachment over the avatar it
//! overlaps): an invisible clicker worn by another resident standing close to
//! us (a spanker) takes the click even when the ray passes through our body.

use super::*;
use aurora_render::Drawn;

/// How far in front of our avatar a surface the click found still counts
/// as on it (a sleeve over the arm's volume).
const ON_BODY: f32 = 0.05;

/// LLPipeline::lineSegmentIntersectInWorld's ATTACHMENT_OVERRIDE_DIST.
const ATTACHMENT_OVERRIDE: f32 = 0.1;

impl Scene {
    /// Where a left click along `ray` lands on our own avatar, if it does.
    /// `hit`: the point the click search settled on (`action_point`, or the
    /// surface the GPU drew for an Alt click); `drawn`: the faces the GPU
    /// drew under the cursor. A hit on our avatar, or inside it (a prim worn
    /// under a mesh body), is the avatar's; a ray through our collision
    /// volumes with nothing drawn of us there (the body not drawn yet or
    /// alpha-masked) too, except for an Alt click, which focuses what it
    /// shows.
    pub fn own_avatar_click(&self, world: &World, ray: (Vec3, Vec3), hit: Option<Vec3>, drawn: &Drawn, alt: bool) -> Option<Vec3> {
        let (o, d) = ray;
        let volumes = own_volumes(world, ray, Instant::now());
        let (worn, rigged) = self.own_drawn(world, ray, drawn);
        let rigged = rigged.filter(|_| volumes.is_some());
        let (t0, t1) = [volumes, worn.map(|t| (t, t)), rigged.map(|t| (t, t))]
            .into_iter()
            .flatten()
            .reduce(|a, b| (a.0.min(b.0), a.1.max(b.1)))?;
        let t = hit.map(|p| (p - o).dot(d));
        // LLPipeline::lineSegmentIntersectInWorld: an attachment up to
        // ATTACHMENT_OVERRIDE_DIST behind where the avatar is met keeps the
        // pick (a clicker worn by someone pressed against us)
        if let (Some(p), Some(t)) = (hit, t)
            && t <= t0 + ATTACHMENT_OVERRIDE
            && self.worn_clicker_of_another(world, p, ray).is_some()
        {
            return None;
        }
        match t {
            Some(t) if t < t0 - ON_BODY => None,
            Some(t) if t <= t1 + ON_BODY => hit,
            _ if alt => None,
            _ => Some(o + d * t0),
        }
    }

    /// Where along `ray` the GPU drew our avatar under the cursor: its body
    /// or a prim it wears, then a rigged mesh it wears (nearest of each).
    fn own_drawn(&self, world: &World, ray: (Vec3, Vec3), drawn: &Drawn) -> (Option<f32>, Option<f32>) {
        let Some(own) = world.objects.index_of_uuid(&world.agent_id) else {
            return (None, None);
        };
        let (mut worn, mut rigged) = (None::<f32>, None::<f32>);
        for face in [drawn.blend, drawn.opaque].into_iter().flatten() {
            let Some((idx, _)) = self.record_object(face.record) else {
                continue;
            };
            let Some(g) = self.gpu.get(idx).filter(|g| !g.hud && (idx == own || g.owner_avatar == Some(own))) else {
                continue;
            };
            let t = (face.point - ray.0).dot(ray.1);
            let slot = if g.rigged { &mut rigged } else { &mut worn };
            *slot = Some(slot.map_or(t, |s| s.min(t)));
        }
        (worn, rigged)
    }

    /// The attachment of another avatar at `point` (the surface a left click
    /// found along `ray`) that has something to do with the click: a touch
    /// script or a click action, on the prim or its root (`interaction::
    /// target`). Ours are left to the press, which touches them before our
    /// avatar; one without an action stops nothing.
    pub fn worn_clicker_of_another(&self, world: &World, point: Vec3, ray: (Vec3, Vec3)) -> Option<usize> {
        let idx = self.interaction_at_ray(world, point, ray, true)?;
        let g = self.gpu.get(idx)?;
        let own = world.objects.index_of_uuid(&world.agent_id);
        let other = !g.hud && !g.is_avatar && g.owner_avatar.is_some() && g.owner_avatar != own;
        (other && crate::interaction::target(world, idx, &HashMap::new()).is_some_and(|t| t.action != crate::interaction::Action::Disabled))
            .then_some(idx)
    }
}

/// Where `ray` enters and leaves the collision volumes of our avatar (the
/// unit spheres of their world matrices: LLVOAvatar::lineSegmentIntersect,
/// `mCollisionVolumes`), with its shape and its pose as last drawn (the rest
/// pose before one is drawn), as `Scene::joint_world` places joints.
fn own_volumes(world: &World, ray: (Vec3, Vec3), now: Instant) -> Option<(f32, f32)> {
    let lib = &world.avatar_lib;
    let skeleton = lib.skeleton.as_ref()?;
    let idx = world.objects.index_of_uuid(&world.agent_id)?;
    let (position, rotation, _) = Scene::object_transform(world, idx, now, 0)?;
    let pose = world.avatar_poses.get(&world.agent_id);
    let placed = Mat4::from_rotation_translation(rotation, position) * Mat4::from_translation(-lib.pelvis);
    let rig = &lib.rig;
    let mut span: Option<(f32, f32)> = None;
    for (j, joint) in skeleton.joints.iter().enumerate().take(rig.len()) {
        if !joint.is_collision_volume {
            continue;
        }
        let rest = rig.default_world_inv[j].inverse();
        let posed = pose.and_then(|p| p.get(j)).map_or(rest, |delta| *delta * rest);
        let inv = (placed * posed).inverse();
        if !inv.is_finite() {
            continue;
        }
        let o = inv.transform_point3(ray.0);
        let d = inv.transform_vector3(ray.1);
        // |o + t d| = 1
        let a = d.length_squared();
        let b = o.dot(d);
        let c = o.length_squared() - 1.0;
        let disc = b * b - a * c;
        if a < 1e-12 || disc < 0.0 {
            continue;
        }
        let root = disc.sqrt();
        let (near, far) = ((-b - root) / a, (-b + root) / a);
        if far < 0.0 {
            continue;
        }
        let near = near.max(0.0);
        span = Some(span.map_or((near, far), |(n, f)| (n.min(near), f.max(far))));
    }
    span
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The world of `picking::tests::scene_with_two_objects`, with our
    /// avatar standing at the origin of the click, facing +X, and another
    /// resident close in front of us. Returns (world, scene, front, back,
    /// other avatar).
    fn two_residents() -> (World, Scene, usize, usize, usize) {
        let (mut world, scene, front, back) = super::super::picking::tests::scene_with_two_objects();
        let own = world.objects.index_of_uuid(&world.agent_id).expect("our avatar");
        let other = world
            .objects
            .avatars()
            .map(|(idx, _)| idx)
            .find(|&idx| idx != own)
            .expect("another resident");
        let o = world.objects.get_mut(own).unwrap();
        o.position = Vec3::new(0.0, 0.0, 50.0);
        o.rotation = Quat::IDENTITY;
        o.scale = Vec3::new(0.45, 0.6, 1.9);
        world.agent.position = Vec3::new(0.0, 0.0, 50.0);
        world.agent.yaw = 0.0;
        (world, scene, front, back, other)
    }

    /// #2509 / #2179: a fully transparent touch prim (a spanker, every face
    /// at alpha 0, no draw record) worn by a resident standing in front of
    /// us. The camera behind us looks over our head at it, through our box
    /// and close to our axis: the click touches the prim, with its face and
    /// UV, instead of turning our avatar. Without an action it takes
    /// nothing from our avatar; ours are left to the press (`own_attachment`).
    #[test]
    fn a_transparent_clicker_worn_by_another_resident_takes_the_click_from_our_avatar() {
        let (mut world, mut scene, clicker, _, other) = two_residents();
        let o = world.objects.get_mut(clicker).unwrap();
        o.position = Vec3::new(1.0, 0.0, 50.98);
        o.scale = Vec3::splat(0.2);
        o.click_action = crate::interaction::code::TOUCH;
        o.update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        for f in &mut Arc::make_mut(o.te.as_mut().unwrap()).faces {
            f.color[3] = 0.0;
        }
        let g = &mut scene.gpu[clicker];
        g.owner_avatar = Some(other);
        g.built_faces = 1;
        g.faces.clear();
        g.center = Vec3::new(1.0, 0.0, 50.98);
        g.radius = 0.2;
        scene.pick.set(clicker, super::super::hover::PickEntry::of(&scene.gpu[clicker]));
        // just over our head, through our box and within 0.2 m of its axis;
        // the other resident's body is drawn 30 cm behind the clicker
        let ray = (Vec3::new(-2.0, 0.0, 50.98), Vec3::X);
        let body = Vec3::new(1.3, 0.0, 50.98);
        let on_clicker = Vec3::new(0.9, 0.0, 50.98);
        let hit = scene.action_point(&world, Some(ray), Some(body), false, 100.0).expect("a point");
        assert!(hit.distance(on_clicker) < 1e-4, "the clicker in front of its wearer: {hit:?}");
        assert_eq!(scene.interaction_at_ray(&world, hit, ray, true), Some(clicker));
        assert_eq!(scene.worn_clicker_of_another(&world, hit, ray), Some(clicker));
        assert_eq!(
            scene.own_avatar_click(&world, ray, Some(hit), &Drawn::default(), false),
            None,
            "not our avatar"
        );
        let touch = scene.left_touch_surface(&world, clicker, ray).expect("a touch surface");
        assert_eq!(touch.face, 0);
        assert!(touch.uv.is_finite() && touch.uv.x >= 0.0 && touch.uv.y >= 0.0, "{:?}", touch.uv);

        // without an action it takes nothing from our avatar
        world.objects.get_mut(clicker).unwrap().update_flags = 0;
        assert_eq!(scene.worn_clicker_of_another(&world, on_clicker, ray), None);

        // one of ours keeps the press's own rule (touched before our avatar)
        world.objects.get_mut(clicker).unwrap().update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        scene.gpu[clicker].owner_avatar = world.objects.index_of_uuid(&world.agent_id);
        assert_eq!(scene.interaction_at_ray(&world, on_clicker, ray, true), Some(clicker));
        assert_eq!(scene.worn_clicker_of_another(&world, on_clicker, ray), None);
    }

    /// Place object `idx` (a cube of `size`) at `at`, where the searches find it.
    fn place(world: &mut World, scene: &mut Scene, idx: usize, at: Vec3, size: f32) {
        let o = world.objects.get_mut(idx).unwrap();
        o.position = at;
        o.scale = Vec3::splat(size);
        scene.gpu[idx].center = at;
        scene.gpu[idx].radius = size;
        scene.pick.set(idx, super::super::hover::PickEntry::of(&scene.gpu[idx]));
    }

    fn drawn_at(record: u32, point: Vec3) -> aurora_render::DrawnFace {
        aurora_render::DrawnFace {
            record,
            point,
            st: glam::Vec2::splat(0.5),
            normal: -Vec3::X,
            binormal: Vec3::Z,
        }
    }

    /// A seated card player's click on the pile just above their head on
    /// the screen (a Discord Hand & Foot report): the camera behind and
    /// above us, the ray passing just over our head down to the pile ahead.
    /// The box that stood for our avatar took it, close to its axis; now our
    /// avatar is its collision volumes and what the GPU drew of it, and the
    /// pile gets the click, even through our big rigged hair. A click on our
    /// head or body is still our avatar's; a prim in front of our body takes
    /// it; another resident's clicker over our body too, within the Legacy
    /// Viewer's 10 cm (ATTACHMENT_OVERRIDE_DIST), and only with an action.
    #[test]
    fn a_click_just_over_our_head_reaches_the_object_ahead() {
        let (mut world, mut scene, object, other_prim, other) = two_residents();
        let own = world.objects.index_of_uuid(&world.agent_id).unwrap();
        let now = Instant::now();
        // our drawn body (record 9) and a rigged mesh we wear (hair, record 8)
        scene.gpu.resize_with(scene.gpu.len().max(own + 1), Default::default);
        let mut face = scene.gpu[object].faces[0];
        scene.gpu[own].is_avatar = true;
        face.record = 9;
        scene.gpu[own].faces.push(face);
        scene.gpu[other_prim].faces[0].record = 8;
        scene.gpu[other_prim].rigged = true;
        scene.gpu[other_prim].owner_avatar = Some(own);
        world.objects.get_mut(other_prim).unwrap().position = Vec3::new(0.0, 30.0, 50.0);
        let head = scene.joint_world(&world, &world.agent_id, "HEAD", now).expect("the head volume");
        let top = own_volumes(&world, (head + Vec3::Z * 5.0, -Vec3::Z), now)
            .map(|(t, _)| head.z + 5.0 - t)
            .expect("our head");
        assert!(top > head.z && top < 51.0, "the top of our head at {top}");
        let none = Drawn::default();

        // the pile ahead, the ray 10 cm over our head (old box: our avatar)
        let eye = Vec3::new(-1.5, 0.0, top + 0.6);
        let dir = (Vec3::new(0.0, 0.0, top + 0.1) - eye).normalize();
        let ray = (eye, dir);
        place(&mut world, &mut scene, object, eye + dir * (2.5 / dir.x), 0.2);
        let o = world.objects.get_mut(object).unwrap();
        o.click_action = crate::interaction::code::TOUCH;
        o.update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        let (hit, picked) = scene.selection_pick(&world, Some(ray), Some(eye + dir * 10.0), false, false, 100.0);
        assert_eq!(picked, Some(object));
        let hit = hit.unwrap();
        let pile = Drawn {
            opaque: Some(drawn_at(0, hit)),
            blend: None,
        };
        assert_eq!(scene.own_avatar_click(&world, ray, Some(hit), &pile, false), None, "the pile's");
        assert_eq!(scene.interaction_at_ray(&world, hit, ray, true), Some(object));
        // our rigged hair over our head does not hide it
        let hair = Drawn {
            blend: Some(drawn_at(8, eye + dir * (1.5 / dir.x))),
            ..pile
        };
        assert_eq!(
            scene.own_avatar_click(&world, ray, Some(hit), &hair, false),
            None,
            "through our hair"
        );

        // our head: nothing drawn yet, then drawn, then an Alt click behind it
        let ray = (eye, (head - eye).normalize());
        let at = scene.own_avatar_click(&world, ray, None, &none, false).expect("our head");
        assert!(at.distance(head) < 0.2, "{at:?} on our head at {head:?}");
        let far = ray.0 + ray.1 * 20.0;
        assert!(scene.own_avatar_click(&world, ray, Some(far), &none, false).is_some());
        assert_eq!(scene.own_avatar_click(&world, ray, Some(far), &none, true), None);
        let skin = Drawn {
            opaque: Some(drawn_at(9, at - ray.1 * 0.02)),
            blend: None,
        };
        assert_eq!(
            scene.own_avatar_click(&world, ray, Some(at - ray.1 * 0.02), &skin, true),
            Some(at - ray.1 * 0.02)
        );
        // our rigged hair over our head counts, over the volumes
        let over = Drawn {
            blend: Some(drawn_at(8, at - ray.1 * 0.1)),
            ..skin
        };
        assert!(scene.own_avatar_click(&world, ray, Some(at - ray.1 * 0.1), &over, false).is_some());

        // our body, and a prim in front of it
        let ray = (Vec3::new(-2.0, 0.0, 50.0), Vec3::X);
        let (entry, _) = own_volumes(&world, ray, now).expect("our pelvis");
        let body = ray.0 + ray.1 * entry;
        assert!(scene.own_avatar_click(&world, ray, Some(body), &none, false).is_some());
        place(&mut world, &mut scene, object, body - Vec3::X * 0.5, 0.2);
        let front = body - Vec3::X * 0.6;
        assert_eq!(scene.interaction_at_ray(&world, front, ray, true), Some(object));
        assert_eq!(scene.own_avatar_click(&world, ray, Some(front), &none, false), None);

        // another resident's clicker over our body (a hug)
        scene.gpu[object].owner_avatar = Some(other);
        for (at, ours) in [(0.05, false), (0.3, true)] {
            place(&mut world, &mut scene, object, body + Vec3::X * (at + 0.1), 0.2);
            let p = body + Vec3::X * at;
            assert_eq!(scene.worn_clicker_of_another(&world, p, ray), Some(object));
            assert_eq!(
                scene.own_avatar_click(&world, ray, Some(p), &none, false).is_some(),
                ours,
                "{at} m in"
            );
        }
        world.objects.get_mut(object).unwrap().update_flags = 0;
        place(&mut world, &mut scene, object, body + Vec3::X * 0.15, 0.2);
        let p = body + Vec3::X * 0.05;
        assert!(
            scene.own_avatar_click(&world, ray, Some(p), &none, false).is_some(),
            "without an action"
        );
    }
}
