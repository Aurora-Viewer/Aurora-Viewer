//! Whether a left click lands on our own avatar: the press then steers it
//! (LLToolPie::handleLeftClickPick: our avatar picked, LLToolCamera's mouse
//! steering and ClickOnAvatarKeepsCamera) instead of acting on what the ray
//! meets.
//!
//! An attachment the Legacy Viewer's transparent pick finds comes before any
//! avatar (LLToolPie::handleMouseDown, priority 1; LLPipeline::
//! lineSegmentIntersectInWorld keeps an attachment over the avatar it
//! overlaps): an invisible clicker worn by another resident standing close to
//! us (a spanker) takes the click even when the ray passes through our box.

use super::*;

impl Scene {
    /// Where a left click along `ray` lands on our own avatar, if it does.
    /// `hit`: the point the click search settled on (`action_point`, or the
    /// surface the GPU drew for an Alt click). A surface inside our avatar's
    /// box counts as our avatar; so does, when the ray goes through the box
    /// without touching anything (body not drawn yet), its pass close to the
    /// body's axis, except for an Alt click.
    pub fn own_avatar_click(&self, world: &World, ray: (Vec3, Vec3), hit: Option<Vec3>, alt: bool) -> Option<Vec3> {
        if hit.is_some_and(|p| self.worn_clicker_of_another(world, p, ray).is_some()) {
            return None;
        }
        let (o, d) = ray;
        let (t0, t1, axis) = own_avatar_ray(world, o, d)?;
        match hit.map(|p| (p - o).dot(d)) {
            Some(t) if t < t0 - 0.05 => None,
            Some(t) if t <= t1 + 0.05 => hit,
            _ if alt => None,
            _ => axis.map(|t| o + d * t),
        }
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

/// Where a ray enters and leaves our avatar's box (the size the simulator
/// gives the avatar object, a little wider for the arms), and where along it
/// the ray passes within 0.2 m of the body's vertical axis (the click finds
/// the avatar before its body and clothes are drawn).
fn own_avatar_ray(world: &World, origin: Vec3, dir: Vec3) -> Option<(f32, f32, Option<f32>)> {
    let a = &world.agent;
    let size = world
        .objects
        .index_of_uuid(&world.agent_id)
        .and_then(|i| world.objects.get(i))
        .map(|o| o.scale)
        .filter(|s| s.z > 0.5 && s.is_finite())
        .unwrap_or(Vec3::new(0.45, 0.6, 1.9));
    let half = Vec3::new((size.x * 0.5).max(0.3) + 0.15, (size.y * 0.5).max(0.3) + 0.15, size.z * 0.5 + 0.1);
    // box space: centered on the agent, turned with its body
    let inv = a.body_rotation().inverse();
    let o = inv * (origin - a.position);
    let d = inv * dir;
    let (mut t0, mut t1) = (0.0f32, f32::INFINITY);
    for k in 0..3 {
        if d[k].abs() < 1e-8 {
            if o[k].abs() > half[k] {
                return None;
            }
            continue;
        }
        let (a, b) = ((-half[k] - o[k]) / d[k], (half[k] - o[k]) / d[k]);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    if t0 > t1 {
        return None;
    }
    // closest approach to the vertical axis, inside the box
    let dxy = d.truncate();
    let t = if dxy.length_squared() > 1e-8 {
        -o.truncate().dot(dxy) / dxy.length_squared()
    } else {
        t0
    };
    let t = t.clamp(t0, t1);
    let q = o + d * t;
    let axis = (q.truncate().length() < 0.2).then_some(t);
    Some((t0, t1, axis))
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
        assert_eq!(scene.own_avatar_click(&world, ray, Some(hit), false), None, "not our avatar");
        let touch = scene.left_touch_surface(&world, clicker, ray).expect("a touch surface");
        assert_eq!(touch.face, 0);
        assert!(touch.uv.is_finite() && touch.uv.x >= 0.0 && touch.uv.y >= 0.0, "{:?}", touch.uv);

        // without an action it takes nothing from our avatar
        world.objects.get_mut(clicker).unwrap().update_flags = 0;
        assert_eq!(scene.worn_clicker_of_another(&world, on_clicker, ray), None);
        assert!(scene.own_avatar_click(&world, ray, Some(on_clicker), false).is_some());

        // one of ours keeps the press's own rule (touched before our avatar)
        world.objects.get_mut(clicker).unwrap().update_flags = crate::ui::context::flags::HANDLE_TOUCH;
        scene.gpu[clicker].owner_avatar = world.objects.index_of_uuid(&world.agent_id);
        assert_eq!(scene.interaction_at_ray(&world, on_clicker, ray, true), Some(clicker));
        assert_eq!(scene.worn_clicker_of_another(&world, on_clicker, ray), None);
    }
}
