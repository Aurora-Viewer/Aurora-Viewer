//! Object filters from renderScripted(Beacons/TouchBeacons),
//! renderPhysicalBeacons, renderMOAPBeacons and renderParticleBeacons
//! (indra/newview/pipeline.cpp, originally LGPL 2.1).

use super::objects::Object;
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
        }
    }
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
