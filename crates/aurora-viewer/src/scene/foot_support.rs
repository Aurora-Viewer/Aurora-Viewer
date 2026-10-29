//! Full-detail support geometry for foot placement (#1281).
//!
//! The drawn detail level of an object follows the camera (`lod_for`): a
//! mesh floor or a curved prim changes height between its levels, and feet
//! traced against the drawn level went up and down as the camera zoomed.
//! The objects under grounded avatars keep a reference on their highest
//! level, built and streamed by the ordinary geometry paths, and
//! `Scene::ground_surface` traces it once ready, whatever level is drawn.
//! The drawn level stays the fallback while the highest one loads.
use super::*;

/// Frames between two refreshes of the held set. Loading the geometry takes
/// longer than this; a support under a moving avatar is found in time.
const REFRESH_FRAMES: u32 = 8;
/// Horizontal reach of the feet around the avatar's root, in metres.
const FOOT_REACH: f32 = 0.6;

struct Held {
    id: Uuid,
    key: GeomKey,
    /// Whether the reference is taken: a geometry still waiting for its
    /// inputs (a sculpt map) has no entry to reference yet.
    referenced: bool,
}

#[derive(Default)]
pub(super) struct FootSupport {
    held: HashMap<usize, Held>,
    refreshed: Option<u32>,
}

impl FootSupport {
    /// The full-detail geometry held for object `idx`, if it is still the
    /// object it was held for.
    pub(super) fn key(&self, idx: usize, id: Uuid) -> Option<GeomKey> {
        self.held.get(&idx).filter(|h| h.id == id).map(|h| h.key)
    }
}

/// What the ankle traces of `ground_surface` reach under an avatar standing
/// at `root`, `height` tall: one trace reach above and below ankles from a
/// little under its floor up to its root (a raised or kneeling leg).
fn reach_box(root: Vec3, height: f32) -> (Vec3, Vec3) {
    let reach = crate::settings::MAX_GROUND_FEET_HEIGHT + 0.01;
    let floor = root.z - height.abs() * 0.5;
    (
        Vec3::new(root.x - FOOT_REACH, root.y - FOOT_REACH, floor - 0.25 - reach),
        Vec3::new(root.x + FOOT_REACH, root.y + FOOT_REACH, root.z.max(floor + 0.75) + reach),
    )
}

impl Scene {
    /// Whether `ground_surface` traces object `idx`: solid, unworn volumes.
    pub(super) fn foot_support_candidate(g: &ObjGpu, object: &Object) -> bool {
        object.pcode == aurora_prim::params::LL_PCODE_VOLUME
            && object.update_flags & crate::build::edits::FLAGS_PHANTOM == 0
            && !g.hud
            && !g.rigged
            && g.owner_avatar.is_none()
    }

    /// Hold the highest level of the objects under the avatars `grounded`
    /// (object indices) and release the others.
    pub(super) fn update_foot_support(&mut self, world: &World, grounded: &[usize], now: Instant) {
        if grounded.is_empty() {
            self.release_foot_support();
            return;
        }
        if self
            .foot_support
            .refreshed
            .is_some_and(|at| self.frame.wrapping_sub(at) < REFRESH_FRAMES)
        {
            return;
        }
        self.foot_support.refreshed = Some(self.frame);
        // (object → its id, its highest level, whether it carries our avatar)
        let mut wanted: HashMap<usize, (Uuid, GeomKey, bool)> = HashMap::new();
        let mut candidates = Vec::new();
        for &avatar in grounded {
            let (Some(object), Some((root, _, _))) = (world.objects.get(avatar), Self::object_transform(world, avatar, now, 0)) else {
                continue;
            };
            let own = object.full_id == world.agent_id;
            let (min, max) = reach_box(root, object.scale.z);
            self.pick.box_candidates(min, max, &mut candidates);
            for &idx in &candidates {
                let (Some(o), Some(g)) = (world.objects.get(idx), self.gpu.get(idx)) else {
                    continue;
                };
                if Self::foot_support_candidate(g, o)
                    && let Some(key) = Self::object_geom_key(o, 3)
                {
                    wanted.entry(idx).or_insert((o.full_id, key, false)).2 |= own;
                }
            }
        }
        let mut held = std::mem::take(&mut self.foot_support.held);
        held.retain(|idx, h| {
            let keep = wanted.get(idx).is_some_and(|&(id, key, _)| id == h.id && key == h.key);
            if !keep && h.referenced {
                self.geom_unref(h.key);
            }
            keep
        });
        for (idx, (id, key, own)) in wanted {
            let h = held.entry(idx).or_insert(Held {
                id,
                key,
                referenced: false,
            });
            // (a cleared cache entry loses its reference: take it again)
            if h.referenced && !self.geoms.contains_key(&h.key) {
                h.referenced = false;
            }
            if !h.referenced
                && let Some(o) = world.objects.get(idx)
            {
                // (under our avatar: streamed first, like what it wears)
                self.request_geom(h.key, o, MeshNeed { px: 256.0, own });
                if self.geoms.contains_key(&h.key) {
                    self.geom_ref(h.key);
                    h.referenced = true;
                }
            }
        }
        self.foot_support.held = held;
    }

    pub(super) fn release_foot_support(&mut self) {
        for (_, h) in std::mem::take(&mut self.foot_support.held) {
            if h.referenced {
                self.geom_unref(h.key);
            }
        }
        self.foot_support.refreshed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::super::picking::PickFace;
    use super::*;
    use crate::scene::anim::grounding::Frame;

    fn ready(scene: &Scene, key: GeomKey) -> Arc<GpuGeom> {
        match &scene.geoms[&key].state {
            GeomState::Ready(geometry) => geometry.clone(),
            _ => panic!("ready geometry"),
        }
    }

    /// A floor whose top is 50.5 at the drawn level, 50.4 at the highest,
    /// and an avatar standing on it.
    fn floor_levels() -> (World, Scene, usize, usize, GeomKey) {
        let (mut world, mut scene, front, _) = super::super::picking::tests::scene_with_two_objects();
        let object = world.objects.get_mut(front).expect("floor");
        object.update_flags = 0;
        // (local -X faces up)
        object.rotation = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        scene.gpu[front].center = Vec3::new(3.0, 0.0, 50.0);
        scene.gpu[front].radius = 1.0;
        scene.pick.set(front, hover::PickEntry::of(&scene.gpu[front]));
        let high = Scene::object_geom_key(world.objects.get(front).expect("floor"), 3).expect("prim");
        let drawn = scene.gpu[front].geom.expect("drawn");
        assert_ne!(high, drawn);
        let geometry = ready(&scene, drawn);
        let x = -0.4;
        let geometry = GpuGeom {
            faces: geometry.faces.clone(),
            min: geometry.min,
            max: geometry.max,
            joint_bounds: Vec::new(),
            pick_faces: vec![Some(PickFace::new(
                vec![[x, -0.5, -0.5], [x, 0.5, -0.5], [x, 0.5, 0.5], [x, -0.5, 0.5]],
                vec![[0.0; 2]; 4],
                vec![0, 2, 1, 0, 3, 2],
            ))],
        };
        scene.geom_insert(high, GeomState::Ready(Arc::new(geometry)));
        let avatar = world
            .objects
            .iter()
            .find(|(_, o)| o.is_avatar() && o.full_id != world.agent_id)
            .map(|(i, _)| i)
            .expect("other avatar");
        let o = world.objects.get_mut(avatar).expect("avatar");
        o.position = Vec3::new(3.0, 0.0, 51.4);
        o.velocity = Vec3::ZERO;
        o.scale.z = 1.8;
        (world, scene, front, avatar, high)
    }

    #[test]
    fn feet_trace_the_highest_level_whatever_level_the_camera_draws() {
        let (world, mut scene, front, avatar, high) = floor_levels();
        let frame = Frame {
            to_world: Mat4::IDENTITY,
            contact: None,
            velocity_z: 0.0,
            sole_height: None,
            max_height: crate::settings::DEFAULT_GROUND_FEET_HEIGHT,
            standing_anchor: false,
        };
        let ankle = Vec3::new(3.0, 0.0, 50.6);
        let now = Instant::now();
        let top = |scene: &Scene| scene.ground_surface(&world, frame, ankle, now).expect("floor").position.z;
        assert!((top(&scene) - 50.5).abs() < 1e-3, "the drawn level until one is held");
        scene.update_foot_support(&world, &[avatar], now);
        assert_eq!(scene.geoms[&high].refs, 1);
        assert!((top(&scene) - 50.4).abs() < 1e-3, "the highest level once held");
        // the camera moves: another level is drawn, the feet stay
        let GeomKey::Prim { hash, .. } = scene.gpu[front].geom.expect("drawn") else {
            panic!("prim");
        };
        let coarser = GeomKey::Prim { hash, lod: 1 };
        let drawn = ready(&scene, scene.gpu[front].geom.expect("drawn"));
        scene.geom_insert(coarser, GeomState::Ready(drawn));
        scene.gpu[front].geom = Some(coarser);
        assert!((top(&scene) - 50.4).abs() < 1e-3);
        // a refresh within the period changes nothing, nor one after it
        scene.update_foot_support(&world, &[avatar], now);
        scene.frame += REFRESH_FRAMES;
        scene.update_foot_support(&world, &[avatar], now);
        assert_eq!(scene.geoms[&high].refs, 1);
        // an avatar no longer grounded releases it
        scene.update_foot_support(&world, &[], now);
        assert_eq!(scene.geoms[&high].refs, 0);
        assert!(scene.foot_support.held.is_empty());
    }

    #[test]
    fn a_floor_out_of_reach_or_another_object_in_its_slot_is_not_held() {
        let (mut world, mut scene, front, avatar, high) = floor_levels();
        let now = Instant::now();
        world.objects.get_mut(avatar).expect("avatar").position.z = 56.0;
        scene.update_foot_support(&world, &[avatar], now);
        assert_eq!(scene.geoms[&high].refs, 0, "a floor far below the feet");
        world.objects.get_mut(avatar).expect("avatar").position.z = 51.4;
        scene.frame += REFRESH_FRAMES;
        scene.update_foot_support(&world, &[avatar], now);
        assert_eq!(scene.geoms[&high].refs, 1);
        let id = world.objects.get(front).expect("floor").full_id;
        assert_eq!(scene.foot_support.key(front, id), Some(high));
        assert_eq!(scene.foot_support.key(front, Uuid::new_v4()), None, "a reused object slot");
        scene.release_foot_support();
        assert_eq!(scene.geoms[&high].refs, 0);
    }

    #[test]
    fn the_reach_box_covers_every_ankle_trace_of_a_standing_avatar() {
        let root = Vec3::new(10.0, 20.0, 31.0);
        let (min, max) = reach_box(root, 2.0);
        // ankles about the floor (root - half height) up to a raised knee,
        // traced one reach above and below
        let reach = crate::settings::MAX_GROUND_FEET_HEIGHT;
        for ankle in [
            Vec3::new(10.3, 20.2, 29.8),
            Vec3::new(9.6, 19.5, 30.05),
            Vec3::new(10.0, 20.4, 30.7),
        ] {
            assert!(ankle.x >= min.x && ankle.x <= max.x && ankle.y >= min.y && ankle.y <= max.y);
            assert!(ankle.z - reach >= min.z, "{ankle} {min}");
            assert!(ankle.z + reach <= max.z, "{ankle} {max}");
        }
    }
}
