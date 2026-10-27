//! Camera presets (issue #1164): a saved camera position grown into a
//! whole shot. The Legacy Viewer's camera presets (LLFloaterSaveCameraPreset,
//! LLPresetsManager::getControlNames, indra/newview, originally LGPL 2.1)
//! keep the camera and its focus in the avatar's frame; Aurora's also choose
//! what holds the camera (the avatar, a fixed place in the world, or an
//! object it follows) and may keep the photo mode's lens (focal length,
//! aperture and focus, exposure), its looks and the light (the local
//! environment and the time of day). Every part is optional: a preset can
//! hold only a look and a light, and leave the camera where it is.
//!
//! A preset is a `CameraShot` (`settings.rs`), so the positions saved before
//! stay presets anchored on the avatar. What the app captures and applies is
//! `app::camera_presets`.

use super::CameraShot;
use crate::dof::PhotoDof;
use crate::snapshot::looks::{Layer, MAX_LAYERS};
use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

/// What holds a preset's camera.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Anchor {
    /// The avatar: the shot's `camera_offset` and `focus_offset`, in its
    /// frame, wherever it goes (the Legacy Viewer's camera preset).
    Avatar,
    /// A fixed place: the camera and its focus in global metres (the
    /// region's corner plus the position in it), the same from any region.
    World { camera: [f64; 3], focus: [f64; 3] },
    /// An object (a linkset or an avatar): the camera and its focus in its
    /// frame; the camera follows it as it moves and turns (TrackFocusObject).
    Object {
        id: Uuid,
        name: String,
        camera: [f32; 3],
        focus: [f32; 3],
    },
}

/// The anchor chosen when a preset is saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnchorKind {
    #[default]
    Avatar,
    World,
    Object,
}

impl Anchor {
    pub fn kind(&self) -> AnchorKind {
        match self {
            Anchor::Avatar => AnchorKind::Avatar,
            Anchor::World { .. } => AnchorKind::World,
            Anchor::Object { .. } => AnchorKind::Object,
        }
    }
}

/// The photo mode's lens. The field of view is the view's (the Legacy
/// Viewer's preset keeps CameraAngle); while the viewfinder is open it is
/// the photo's, given back when it closes. The exposure is the photo's
/// (`snapshot::light`): None, Preferences' own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lens {
    pub fov: f32,
    pub dof: PhotoDof,
    pub exposure: Option<f32>,
}

/// The photo's looks (`snapshot::looks`): the stack and the settings of
/// the looks in it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Looks {
    pub layers: Vec<Layer>,
    pub settings: BTreeMap<String, Vec<f32>>,
}

/// The light: the local environment (`EnvSelector::saved_local`, in LLSD
/// notation; None, the shared one) and the time of day preset (0: none).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Light {
    pub environment: Option<String>,
    pub time_of_day: u8,
}

/// What a preset keeps, chosen when it is saved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Include {
    /// The camera's position and what holds it; None: left out.
    pub anchor: Option<AnchorKind>,
    pub lens: bool,
    pub looks: bool,
    pub light: bool,
}

impl Default for Include {
    fn default() -> Self {
        Include {
            anchor: Some(AnchorKind::Avatar),
            lens: true,
            looks: true,
            light: true,
        }
    }
}

impl Include {
    /// Something to save.
    pub fn any(&self) -> bool {
        self.anchor.is_some() || self.lens || self.looks || self.light
    }
}

/// Render-space `p` in global metres, the main region's corner at `origin`.
pub fn to_global(origin: (u32, u32), p: Vec3) -> [f64; 3] {
    [origin.0 as f64 + p.x as f64, origin.1 as f64 + p.y as f64, p.z as f64]
}

/// Global metres back in render space.
pub fn from_global(origin: (u32, u32), g: [f64; 3]) -> Vec3 {
    Vec3::new((g[0] - origin.0 as f64) as f32, (g[1] - origin.1 as f64) as f32, g[2] as f32)
}

/// `p` in the frame of an object at `position` turned by `rotation`.
pub fn into_frame(position: Vec3, rotation: Quat, p: Vec3) -> [f32; 3] {
    (rotation.inverse() * (p - position)).to_array()
}

/// A point of an object's frame back in render space.
pub fn out_of_frame(position: Vec3, rotation: Quat, local: [f32; 3]) -> Vec3 {
    position + rotation * Vec3::from(local)
}

/// Save `shot` in `list`: a preset of the same name is replaced where it is,
/// a new one goes last (LLFloaterSaveCameraPreset's replace).
pub fn store(list: &mut Vec<CameraShot>, shot: CameraShot) {
    match list.iter_mut().find(|s| s.name == shot.name) {
        Some(old) => *old = shot,
        None => list.push(shot),
    }
}

/// Keep only the looks' settings of the looks in `layers`.
pub fn looks_of(layers: &[Layer], settings: &BTreeMap<String, Vec<f32>>) -> Looks {
    Looks {
        layers: layers.to_vec(),
        settings: settings
            .iter()
            .filter(|(id, _)| layers.iter().any(|l| &l.id == *id))
            .map(|(id, v)| (id.clone(), v.clone()))
            .collect(),
    }
}

fn finite3<T: Copy + Into<f64>>(v: [T; 3]) -> bool {
    v.iter().all(|x| (*x).into().is_finite())
}

/// Presets read from a file: values in their ranges, a part that cannot be
/// used dropped (an anchor with a broken position, a light whose
/// environment does not parse).
pub fn sanitize(list: &mut Vec<CameraShot>) {
    list.retain(|s| !s.name.trim().is_empty());
    for shot in list {
        if !finite3(shot.camera_offset) || !finite3(shot.focus_offset) {
            shot.camera_offset = super::CameraSettings::default().camera_offset;
            shot.focus_offset = super::CameraSettings::default().focus_offset;
        }
        let broken = match &shot.anchor {
            Some(Anchor::World { camera, focus }) => !finite3(*camera) || !finite3(*focus),
            Some(Anchor::Object { id, camera, focus, .. }) => id.is_nil() || !finite3(*camera) || !finite3(*focus),
            Some(Anchor::Avatar) | None => false,
        };
        if broken {
            shot.anchor = None;
        }
        if let Some(lens) = &mut shot.lens {
            lens.fov = if lens.fov.is_finite() {
                lens.fov.clamp(5.0f32.to_radians(), 175.0f32.to_radians())
            } else {
                super::DEFAULT_FOV
            };
            lens.dof.sanitize();
            lens.exposure = lens
                .exposure
                .filter(|e| e.is_finite())
                .map(|e| e.clamp(*crate::snapshot::light::EXPOSURE.start(), *crate::snapshot::light::EXPOSURE.end()));
        }
        if let Some(looks) = &mut shot.looks {
            looks.layers.truncate(MAX_LAYERS);
            for l in &mut looks.layers {
                l.strength = l.strength.min(100);
            }
        }
        if let Some(light) = &mut shot.light {
            light.time_of_day = light.time_of_day.min(crate::world::env::TIME_OF_DAY_PRESETS);
            if light
                .environment
                .as_ref()
                .is_some_and(|e| aurora_llsd::from_notation(e.as_bytes()).map_or(true, |d| !d.is_map()))
            {
                shot.light = None;
            }
        }
        if !shot.has_any() {
            shot.anchor = Some(Anchor::Avatar);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shot(name: &str) -> CameraShot {
        CameraShot::at_avatar(name.to_owned(), [-3.0, 0.0, 0.75], [1.0, 0.0, 1.0])
    }

    #[test]
    fn positions_saved_before_presets_are_presets_on_the_avatar() {
        let old = r#"{"name":"Close up","camera_offset":[-1.0,0.0,0.5],"focus_offset":[0.5,0.0,0.8]}"#;
        let s: CameraShot = serde_json::from_str(old).unwrap();
        assert_eq!(s.anchor, Some(Anchor::Avatar));
        assert!(s.lens.is_none() && s.looks.is_none() && s.light.is_none());
        assert_eq!(s.camera_offset, [-1.0, 0.0, 0.5]);
    }

    #[test]
    fn a_preset_keeps_every_part_through_the_settings_file() {
        let mut s = shot("Product");
        s.anchor = Some(Anchor::Object {
            id: Uuid::from_u128(7),
            name: "Chair".into(),
            camera: [2.0, 0.5, 1.0],
            focus: [0.0, 0.0, 0.4],
        });
        s.lens = Some(Lens {
            fov: 0.5,
            dof: PhotoDof::default(),
            exposure: Some(1.5),
        });
        s.looks = Some(Looks {
            layers: vec![Layer::new("Sepia")],
            settings: BTreeMap::new(),
        });
        s.light = Some(Light {
            environment: None,
            time_of_day: 3,
        });
        let text = serde_json::to_string(&s).unwrap();
        let back: CameraShot = serde_json::from_str(&text).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn a_preset_without_a_position_reads_back_without_one() {
        let mut s = shot("Look only");
        s.anchor = None;
        s.light = Some(Light {
            environment: None,
            time_of_day: 0,
        });
        let back: CameraShot = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.anchor, None);
    }

    #[test]
    fn global_positions_hold_across_regions() {
        let p = Vec3::new(12.5, 200.25, 31.0);
        let g = to_global((256_000, 512_256), p);
        assert_eq!(g, [256_012.5, 512_456.25, 31.0]);
        // seen from the region to the east, the same place is 256 m west
        let east = from_global((256_256, 512_256), g);
        assert!((east - Vec3::new(12.5 - 256.0, 200.25, 31.0)).length() < 1e-3);
        assert!((from_global((256_000, 512_256), g) - p).length() < 1e-3);
    }

    #[test]
    fn an_object_frame_follows_the_object() {
        let at = Vec3::new(10.0, 20.0, 5.0);
        let turned = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let camera = Vec3::new(10.0, 23.0, 6.0);
        let local = into_frame(at, turned, camera);
        assert!((Vec3::from(local) - Vec3::new(3.0, 0.0, 1.0)).length() < 1e-4);
        assert!((out_of_frame(at, turned, local) - camera).length() < 1e-4);
        // the object moved and turned back: the camera goes with it
        let moved = out_of_frame(Vec3::new(50.0, 0.0, 5.0), Quat::IDENTITY, local);
        assert!((moved - Vec3::new(53.0, 0.0, 6.0)).length() < 1e-4);
    }

    #[test]
    fn saving_a_known_name_replaces_it_in_place() {
        let mut list = vec![shot("A"), shot("B")];
        let mut b = shot("B");
        b.camera_offset = [1.0, 2.0, 3.0];
        store(&mut list, b.clone());
        store(&mut list, shot("C"));
        assert_eq!(list.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["A", "B", "C"]);
        assert_eq!(list[1], b);
    }

    #[test]
    fn a_preset_keeps_only_the_settings_of_its_looks() {
        let mut settings = BTreeMap::new();
        settings.insert("aurora:film".to_owned(), vec![0.5]);
        settings.insert("aurora:vignette".to_owned(), vec![0.2]);
        let looks = looks_of(&[Layer::new("aurora:film")], &settings);
        assert_eq!(looks.settings.keys().collect::<Vec<_>>(), ["aurora:film"]);
    }

    #[test]
    fn broken_parts_of_an_edited_file_are_dropped() {
        let mut bad = shot("Bad");
        bad.anchor = Some(Anchor::World {
            camera: [f64::NAN, 0.0, 0.0],
            focus: [0.0; 3],
        });
        bad.lens = Some(Lens {
            fov: 40.0,
            dof: PhotoDof::default(),
            exposure: Some(f32::INFINITY),
        });
        bad.light = Some(Light {
            environment: Some("not llsd {".into()),
            time_of_day: 99,
        });
        let mut list = vec![bad, shot("  ")];
        sanitize(&mut list);
        assert_eq!(list.len(), 1, "a preset without a name goes");
        let s = &list[0];
        // nothing left to hold the camera but the lens: no anchor
        assert_eq!(s.anchor, None);
        let lens = s.lens.as_ref().unwrap();
        assert_eq!(lens.fov, 175.0f32.to_radians());
        assert_eq!(lens.exposure, None);
        assert!(s.light.is_none());
    }

    #[test]
    fn a_preset_left_with_nothing_falls_back_to_its_avatar_position() {
        let mut s = shot("Empty");
        s.anchor = Some(Anchor::Object {
            id: Uuid::nil(),
            name: String::new(),
            camera: [0.0; 3],
            focus: [0.0; 3],
        });
        let mut list = vec![s];
        sanitize(&mut list);
        assert_eq!(list[0].anchor, Some(Anchor::Avatar));
    }

    #[test]
    fn a_saved_local_environment_survives_the_notation() {
        let env = aurora_llsd::llsd_map! { "sky_id" => Uuid::from_u128(42) };
        let mut s = shot("Sunset");
        s.light = Some(Light {
            environment: Some(aurora_llsd::to_notation(&env)),
            time_of_day: 0,
        });
        let mut list = vec![s];
        sanitize(&mut list);
        let text = list[0].light.as_ref().and_then(|l| l.environment.clone()).unwrap();
        let back = aurora_llsd::from_notation(text.as_bytes()).unwrap();
        assert_eq!(back["sky_id"].as_uuid(), Uuid::from_u128(42));
    }
}
