//! Object filters from renderScripted(Beacons/TouchBeacons),
//! renderPhysicalBeacons, renderMOAPBeacons and renderParticleBeacons
//! (indra/newview/pipeline.cpp, originally LGPL 2.1).

use super::objects::Object;
use super::terrain::Heightmap;
use glam::{Mat3, Quat, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Beacons {
    pub lines: bool,
    pub highlights: bool,
    pub info: bool,
    pub width: u8,
    pub scripted: bool,
    pub touch: bool,
    pub physical: bool,
    pub sound: bool,
    pub particles: bool,
    pub media: bool,
    /// "Bounding boxes for lights in scene": the RENDER_DEBUG_LIGHTS mask,
    /// which the Legacy Viewer does not save either.
    #[serde(skip)]
    pub light_boxes: bool,
    /// FSRegionCornerBeacons (default off).
    pub region_corners: bool,
    /// FSShowChatRangeSpheres (default off).
    pub chat_spheres: bool,
}

impl Default for Beacons {
    fn default() -> Self {
        Self {
            lines: false,
            highlights: true,
            info: true,
            width: 1,
            scripted: false,
            touch: false,
            physical: false,
            sound: false,
            particles: false,
            media: false,
            light_boxes: false,
            region_corners: false,
            chat_spheres: false,
        }
    }
}

/// A light's box of renderLights (indra/newview/llspatialpartition.cpp,
/// originally LGPL 2.1): the outline of its drawn bounds, and a box of
/// half-size its radius around their centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightBox {
    pub center: Vec3,
    /// Half extents of the prim's axis-aligned bounds.
    pub half: Vec3,
    /// LLVOVolume::getLightRadius: the radius as sent, not the 1.5x reach
    /// of the deferred light.
    pub radius: f32,
}

impl LightBox {
    pub fn new(center: Vec3, rot: Quat, scale: Vec3, radius: f32) -> Self {
        let m = Mat3::from_quat(rot);
        let h = scale * 0.5;
        let half = m.x_axis.abs() * h.x + m.y_axis.abs() * h.y + m.z_axis.abs() * h.z;
        Self { center, half, radius }
    }
}

/// The twelve edges of an axis-aligned box (drawBoxOutline).
pub fn box_edges(center: Vec3, half: Vec3) -> impl Iterator<Item = [Vec3; 2]> {
    (0..3).flat_map(move |axis| {
        [(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)].into_iter().map(move |(u, v)| {
            let (i, j) = ((axis + 1) % 3, (axis + 2) % 3);
            let mut a = center;
            a[axis] -= half[axis];
            a[i] += u * half[i];
            a[j] += v * half[j];
            let mut b = a;
            b[axis] += 2.0 * half[axis];
            [a, b]
        })
    })
}

/// The region corner beacons (FSRegionCornerBeacons in
/// LLPipeline::renderDebug, indra/newview/pipeline.cpp): SW, SE, NW, NE at
/// land height, in region coordinates.
pub fn region_corners(heightmap: &Heightmap) -> [Vec3; 4] {
    let (w, h) = (heightmap.size_x as f32, heightmap.size_y as f32);
    [(0.0, 0.0), (w, 0.0), (0.0, h), (w, h)].map(|(x, y)| Vec3::new(x, y, heightmap.sample(x, y)))
}

/// Segments of each circle of a chat range sphere.
pub const SPHERE_SEGMENTS: usize = 32;

/// The chat range spheres (drawChatRangeSpheres, indra/newview/
/// llviewerdisplay.cpp) as lines: the equator, the parallels at 45 degrees
/// and two meridians. The Legacy Viewer fills a translucent sphere; lines
/// keep the world readable from inside the 100 m shout sphere, and go
/// through the interface's depth-tested line pass.
pub fn sphere_rings(center: Vec3, radius: f32) -> impl Iterator<Item = [Vec3; 2]> {
    let step = std::f32::consts::TAU / SPHERE_SEGMENTS as f32;
    let parallels = [-1.0f32, 0.0, 1.0].map(|k| k * std::f32::consts::FRAC_PI_4);
    let point = move |ring: usize, t: f32| {
        let (s, c) = t.sin_cos();
        match ring {
            0..3 => {
                let (z, r) = parallels[ring].sin_cos();
                center + radius * Vec3::new(c * r, s * r, z)
            }
            3 => center + radius * Vec3::new(c, 0.0, s),
            _ => center + radius * Vec3::new(0.0, c, s),
        }
    };
    (0..5).flat_map(move |ring| (0..SPHERE_SEGMENTS).map(move |i| [point(ring, i as f32 * step), point(ring, (i + 1) as f32 * step)]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Script,
    Physical,
    Sound,
    Particle,
    Media,
}

impl Beacons {
    pub fn objects_enabled(&self) -> bool {
        (self.lines || self.highlights) && (self.scripted || self.touch || self.physical || self.sound || self.particles || self.media)
    }

    /// Scripts only on unparented objects; physics, media and particles on
    /// child prims and attachments too. Touch requires both simulator flags.
    /// Sound uses the audio engine's sources, independently of visibility.
    pub fn kinds(&self, o: &Object, particle_source: bool) -> impl Iterator<Item = Kind> {
        let script = o.parent_id == 0
            && o.update_flags & (1 << 6) != 0
            && (if self.touch {
                o.update_flags & (1 << 7) != 0
            } else {
                self.scripted
            });
        [
            (script, Kind::Script),
            (self.physical && o.update_flags & 1 != 0, Kind::Physical),
            (self.particles && particle_source, Kind::Particle),
            (
                self.media
                    && o.te
                        .as_ref()
                        .is_some_and(|te| (0..aurora_prim::te::MAX_TES).any(|i| te.face(i).media_flags & 1 != 0)),
                Kind::Media,
            ),
        ]
        .into_iter()
        .filter(move |(on, _)| *on && !o.is_avatar())
        .map(|(_, k)| k)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlays {
    pub property_lines: bool,
    #[serde(skip)]
    pub land_owners: bool,
    #[serde(skip)]
    pub hide_particles: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beacon_defaults_and_saved_filters_follow_the_legacy_settings() {
        let s: Beacons = serde_json::from_str("{}").unwrap();
        assert_eq!(s, Beacons::default());
        assert!(!s.lines && s.highlights && s.info && s.width == 1);
        assert!(!s.objects_enabled());
        let selected = Beacons {
            touch: true,
            lines: true,
            width: 7,
            ..s
        };
        let saved = serde_json::to_string(&selected).unwrap();
        assert_eq!(serde_json::from_str::<Beacons>(&saved).unwrap(), selected);
    }

    #[test]
    fn corners_and_chat_spheres_are_saved_but_light_boxes_are_not() {
        let s = Beacons::default();
        assert!(!s.light_boxes && !s.region_corners && !s.chat_spheres);
        let on = Beacons {
            light_boxes: true,
            region_corners: true,
            chat_spheres: true,
            ..s
        };
        let back: Beacons = serde_json::from_str(&serde_json::to_string(&on).unwrap()).unwrap();
        assert!(back.region_corners && back.chat_spheres);
        assert!(!back.light_boxes, "a debug mask, off at the next start");
    }

    #[test]
    fn light_boxes_hold_the_rotated_bounds_and_the_light_radius() {
        let flat = LightBox::new(Vec3::new(10.0, 20.0, 30.0), Quat::IDENTITY, Vec3::new(2.0, 4.0, 6.0), 5.0);
        assert_eq!(flat.half, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(flat.radius, 5.0);
        let turned = LightBox::new(
            Vec3::ZERO,
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(2.0, 4.0, 6.0),
            5.0,
        );
        assert!(turned.half.abs_diff_eq(Vec3::new(2.0, 1.0, 3.0), 1e-5));
        let edges: Vec<_> = box_edges(Vec3::ZERO, Vec3::new(1.0, 2.0, 3.0)).collect();
        assert_eq!(edges.len(), 12);
        for [a, b] in edges {
            let d = (b - a).abs();
            assert_eq!([d.x, d.y, d.z].iter().filter(|v| **v > 0.0).count(), 1, "each edge along one axis");
            assert!(
                a.abs() == Vec3::new(1.0, 2.0, 3.0) && b.abs() == Vec3::new(1.0, 2.0, 3.0),
                "from corner to corner"
            );
        }
    }

    #[test]
    fn region_corners_stand_on_the_land_of_each_corner() {
        let mut hm = Heightmap::new(256, 256);
        for (i, h) in hm.heights.iter_mut().enumerate() {
            *h = (i % 256) as f32 * 0.1 + (i / 256) as f32;
        }
        let [sw, se, nw, ne] = region_corners(&hm);
        assert_eq!((sw.x, sw.y, sw.z), (0.0, 0.0, 0.0));
        assert_eq!((se.x, se.y), (256.0, 0.0));
        assert!((se.z - 25.5).abs() < 0.01);
        assert_eq!((nw.x, nw.y), (0.0, 256.0));
        assert!((nw.z - 255.0).abs() < 0.01);
        assert_eq!((ne.x, ne.y), (256.0, 256.0));
        assert!((ne.z - 280.5).abs() < 0.01);
    }

    #[test]
    fn chat_sphere_rings_are_closed_circles_on_the_sphere() {
        let c = Vec3::new(5.0, 6.0, 7.0);
        let segments: Vec<_> = sphere_rings(c, 20.0).collect();
        assert_eq!(segments.len(), 5 * SPHERE_SEGMENTS);
        for ring in segments.chunks(SPHERE_SEGMENTS) {
            assert!(ring[0][0].abs_diff_eq(ring[SPHERE_SEGMENTS - 1][1], 1e-3), "each circle closes");
            for [a, b] in ring {
                assert!((a.distance(c) - 20.0).abs() < 1e-3 && (b.distance(c) - 20.0).abs() < 1e-3);
            }
        }
    }

    #[test]
    fn beacon_filters_use_root_scripts_touch_flags_and_per_prim_media() {
        let s = Beacons {
            scripted: true,
            physical: true,
            particles: true,
            media: true,
            ..Default::default()
        };
        let objects: Vec<_> = crate::demo::beacons::objects()
            .into_iter()
            .map(|o| Object::from_update(1, o))
            .collect();
        for (index, expected) in [
            (0, Kind::Script),
            (1, Kind::Script),
            (2, Kind::Physical),
            (4, Kind::Particle),
            (5, Kind::Media),
            (6, Kind::Script),
        ] {
            assert_eq!(
                s.kinds(&objects[index], objects[index].particles.is_some()).collect::<Vec<_>>(),
                [expected]
            );
        }
        assert_eq!(s.kinds(&objects[3], false).count(), 0, "sounds come from the audio engine");
        let touch = Beacons { touch: true, ..s };
        assert_eq!(
            touch.kinds(&objects[0], false).count(),
            0,
            "touch takes precedence over all scripts"
        );
        assert_eq!(touch.kinds(&objects[1], false).collect::<Vec<_>>(), [Kind::Script]);
        let mut child = Object::from_update(1, crate::demo::beacons::objects().remove(1));
        child.parent_id = 68200;
        assert_eq!(touch.kinds(&child, false).count(), 0);
        child.update_flags |= 1;
        assert_eq!(touch.kinds(&child, false).collect::<Vec<_>>(), [Kind::Physical]);
        child.pcode = 47;
        assert_eq!(touch.kinds(&child, false).count(), 0, "avatars are never object highlights");
    }
}
