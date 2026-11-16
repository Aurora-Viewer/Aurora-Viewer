//! Which dry volumes the camera stands in (aurora_render::dry_volume).
//!
//! A dry volume is an object marked "Dry interior" in the Build window's
//! Object tab, which adds `[dry]` (`TAG`) to its description: its own box
//! (or cylinder, or sphere; a mesh or sculpt fills its box) is the dry
//! area. Below the water line, inside one, the view is drawn as in air and
//! the water's fog starts at its walls. Aurora's own: the Legacy Viewer has
//! no such test (LLViewerCamera::cameraUnderWater only compares heights).
//! The object stays as it is drawn: a house can be its own dry room.
//! Descriptions are only known
//! once asked (ObjectPropertiesFamily, as a hover asks), so while the camera
//! is under the water the objects whose box it is in are asked, once.
//! Aurora hides the tag where it shows a description (`strip_tag`); the
//! other viewers show it, and have no dry rooms either.
//!
//! The prims that qualify are looked for twice a second (`REFRESH`), only
//! while the camera is below the water line; their placement follows them
//! every frame.

use super::Scene;
use crate::world::World;
use crate::world::objects::Object;
use aurora_net::NetCommand;
use aurora_net::build::ObjectProps;
use aurora_prim::params as pp;
use aurora_render::dry_volume::{DryShape, DryVolume, MAX_DRY_VOLUMES};
use glam::Vec3;
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

const REFRESH: Duration = Duration::from_millis(500);
/// Volumes farther than this from the camera are not sent (their centre,
/// less their half diagonal).
const REACH: f32 = 64.0;
/// An object whose description was asked is not asked again before this.
const ASK_AGAIN: Duration = Duration::from_secs(30);

/// The "Dry interior" tag, at the end of an object's description.
pub const TAG: &str = "[dry]";

/// The description carries the tag (in any case).
pub fn has_tag(description: &str) -> bool {
    description.to_ascii_lowercase().contains(TAG)
}

/// The description without the tag, as Aurora shows it.
pub fn strip_tag(description: &str) -> String {
    let lower = description.to_ascii_lowercase();
    let mut out = String::with_capacity(description.len());
    let mut at = 0;
    // ASCII lowering keeps byte offsets, so `lower`'s matches index `description`
    while let Some(found) = lower[at..].find(TAG) {
        out.push_str(&description[at..at + found]);
        at += found + TAG.len();
    }
    out.push_str(&description[at..]);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `description` with the tag at its end (`on`) or without it.
pub fn with_tag(description: &str, on: bool) -> String {
    let base = strip_tag(description);
    match (on, base.is_empty()) {
        (false, _) => base,
        (true, true) => TAG.to_owned(),
        (true, false) => format!("{base} {TAG}"),
    }
}

#[derive(Default)]
pub struct DryVolumes {
    /// Object indices that are dry rooms.
    candidates: Vec<usize>,
    refreshed: Option<Instant>,
    /// The objects whose description is known: tagged or not.
    tagged: HashMap<Uuid, bool>,
    /// When each object's description was last asked.
    asked: HashMap<Uuid, Instant>,
}

impl DryVolumes {
    /// A description arrived (ObjectProperties / ObjectPropertiesFamily).
    pub fn on_properties(&mut self, props: &[ObjectProps]) {
        for p in props {
            self.tagged.insert(p.object_id, has_tag(&p.description));
        }
    }

    /// A dry room: "Dry interior" is ticked.
    pub fn is_dry(&self, o: &Object) -> bool {
        self.tagged.get(&o.full_id) == Some(&true) && is_room_object(o)
    }

    /// Mark an object as known tagged or not, before its properties come
    /// back (the Build window's own edit, the offline demo).
    pub fn set_tagged(&mut self, id: Uuid, tagged: bool) {
        // a room newly marked joins at the next frame, not the next scan
        if self.tagged.insert(id, tagged) != Some(tagged) && tagged {
            self.refreshed = None;
        }
    }

    /// The volumes to send this frame: empty unless the camera is below
    /// `water` and inside one of them, which then comes first. `ask`
    /// receives the description requests of the objects around the camera.
    pub fn update(
        &mut self,
        world: &World,
        camera: Vec3,
        water: f32,
        enabled: bool,
        now: Instant,
        ask: &mut Vec<NetCommand>,
    ) -> Vec<DryVolume> {
        if !enabled || camera.z >= water {
            self.candidates.clear();
            self.refreshed = None;
            return Vec::new();
        }
        if self.refreshed.is_none_or(|at| now.saturating_duration_since(at) >= REFRESH) {
            self.refresh(world, camera, now, ask);
        }
        let mut near: Vec<(f32, DryVolume)> = self
            .candidates
            .iter()
            .filter_map(|&idx| {
                let o = world.objects.get(idx).filter(|o| self.is_dry(o))?;
                let (pos, rot, hud) = Scene::object_transform(world, idx, now, 0)?;
                if hud {
                    return None;
                }
                let scale = o.visual_scale_at(now);
                let gap = pos.distance(camera) - scale.length() * 0.5;
                (gap <= REACH).then_some(())?;
                Some((gap, DryVolume::new(pos, rot, scale, shape_of(o))?))
            })
            .collect();
        order(&mut near, camera)
    }

    /// The dry rooms of the scene, and the description asked of each object
    /// whose box holds the camera and whose description is not known yet.
    fn refresh(&mut self, world: &World, camera: Vec3, now: Instant, ask: &mut Vec<NetCommand>) {
        self.refreshed = Some(now);
        let mut candidates = Vec::new();
        for (idx, o) in world.objects.iter() {
            if self.is_dry(o) {
                candidates.push(idx);
                continue;
            }
            if !is_room_object(o) || self.tagged.contains_key(&o.full_id) {
                continue;
            }
            if self
                .asked
                .get(&o.full_id)
                .is_some_and(|at| now.saturating_duration_since(*at) < ASK_AGAIN)
            {
                continue;
            }
            let Some((pos, rot, false)) = Scene::object_transform(world, idx, now, 0) else {
                continue;
            };
            let scale = o.visual_scale_at(now);
            // the cheap test first: most objects are far from the camera
            if pos.distance(camera) > scale.length() * 0.5 {
                continue;
            }
            if DryVolume::new(pos, rot, scale, shape_of(o)).is_some_and(|v| v.contains(camera)) {
                self.asked.insert(o.full_id, now);
                ask.push(NetCommand::RequestObjectProperties {
                    handle: o.key.region,
                    object: o.full_id,
                });
            }
        }
        self.candidates = candidates;
    }
}

/// An object that can hold a room: an in-world prim, mesh or sculpt, not
/// worn, not an avatar or a tree.
fn is_room_object(o: &Object) -> bool {
    o.pcode == pp::LL_PCODE_VOLUME && !o.is_attachment()
}

/// The volumes nearest first, the one the camera is in at the front; none
/// when the camera is in none of them.
fn order(near: &mut [(f32, DryVolume)], camera: Vec3) -> Vec<DryVolume> {
    let Some(inside) = near.iter().position(|(_, v)| v.contains(camera)) else {
        return Vec::new();
    };
    near.swap(0, inside);
    near[1..].sort_by(|a, b| a.0.total_cmp(&b.0));
    near.iter().take(MAX_DRY_VOLUMES).map(|(_, v)| *v).collect()
}

/// The solid the prim's shape is closest to (hollows and cuts ignored). A
/// mesh or a sculpt fills its bounding box.
pub fn shape_of(o: &Object) -> DryShape {
    let volume = o.visual_volume();
    if volume.sculpt.is_some() {
        return DryShape::Box;
    }
    let profile = volume.profile.curve_type & pp::LL_PCODE_PROFILE_MASK;
    let path = volume.path.curve_type;
    match (path, profile) {
        (pp::LL_PCODE_PATH_LINE | pp::LL_PCODE_PATH_FLEXIBLE, pp::LL_PCODE_PROFILE_CIRCLE) => DryShape::Cylinder,
        (pp::LL_PCODE_PATH_CIRCLE | pp::LL_PCODE_PATH_CIRCLE2, pp::LL_PCODE_PROFILE_CIRCLE_HALF) => DryShape::Sphere,
        _ => DryShape::Box,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn cube(at: Vec3, size: f32) -> DryVolume {
        DryVolume::new(at, Quat::IDENTITY, Vec3::splat(size), DryShape::Box).unwrap()
    }

    #[test]
    fn nothing_is_sent_outside_every_volume() {
        let mut near = vec![(1.0, cube(Vec3::new(5.0, 0.0, 0.0), 2.0))];
        assert!(order(&mut near, Vec3::ZERO).is_empty());
    }

    #[test]
    fn the_volume_around_the_camera_comes_first() {
        let far = cube(Vec3::new(30.0, 0.0, 0.0), 2.0);
        let next = cube(Vec3::new(6.0, 0.0, 0.0), 4.0);
        let around = cube(Vec3::ZERO, 8.0);
        let mut near = vec![(29.0, far), (2.0, next), (-4.0, around)];
        let sent = order(&mut near, Vec3::ZERO);
        assert_eq!(sent, vec![around, next, far]);
    }

    #[test]
    fn at_most_the_shader_limit_is_sent() {
        let mut near: Vec<_> = (0..20)
            .map(|i| (i as f32, cube(Vec3::new(i as f32 * 3.0, 0.0, 0.0), 2.0)))
            .collect();
        assert_eq!(order(&mut near, Vec3::ZERO).len(), MAX_DRY_VOLUMES);
    }

    #[test]
    fn the_tag_is_found_in_any_case() {
        assert!(has_tag("[dry]"));
        assert!(has_tag("Reef cottage [DRY]"));
        assert!(!has_tag("Dry cottage"));
        assert!(!has_tag("[dry room]"));
    }

    #[test]
    fn the_tag_is_hidden_and_put_back_at_the_end() {
        assert_eq!(strip_tag("Reef cottage [dry]"), "Reef cottage");
        assert_eq!(strip_tag("[Dry] Reef  cottage"), "Reef cottage");
        assert_eq!(strip_tag("[dry]"), "");
        assert_eq!(with_tag("Reef cottage", true), "Reef cottage [dry]");
        assert_eq!(with_tag("", true), "[dry]");
        assert_eq!(with_tag("[dry] Reef cottage", true), "Reef cottage [dry]");
        assert_eq!(with_tag("Reef cottage [dry]", false), "Reef cottage");
    }

    #[test]
    fn a_room_marked_from_the_build_window_is_scanned_at_once() {
        let mut dry = DryVolumes::default();
        let id = Uuid::from_u128(8);
        dry.refreshed = Some(Instant::now());
        dry.set_tagged(id, true);
        assert!(dry.refreshed.is_none(), "the next frame scans for the new room");
        dry.refreshed = Some(Instant::now());
        dry.set_tagged(id, true);
        assert!(dry.refreshed.is_some(), "marking it again changes nothing");
        dry.set_tagged(id, false);
        assert_eq!(dry.tagged.get(&id), Some(&false));
    }

    #[test]
    fn properties_teach_which_objects_are_tagged() {
        let mut dry = DryVolumes::default();
        let id = Uuid::from_u128(7);
        dry.on_properties(&[ObjectProps {
            object_id: id,
            description: "House [dry]".into(),
            ..Default::default()
        }]);
        assert_eq!(dry.tagged.get(&id), Some(&true));
        dry.on_properties(&[ObjectProps {
            object_id: id,
            description: "House".into(),
            ..Default::default()
        }]);
        assert_eq!(dry.tagged.get(&id), Some(&false));
    }
}
