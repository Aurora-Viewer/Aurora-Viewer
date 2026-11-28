//! Read-only import of Aperture-compatible per-account quick camera slots.
//! Aperture Viewer: https://github.com/ApertureViewer/Aperture-Viewer
//! LLAgentCamera::storeCameraPosition / loadCameraPosition and
//! floater_phototools_camera.xml (indra/newview, originally LGPL 2.1) keep
//! global camera/focus coordinates, an optional focus object, roll, and the
//! separate name of each of eight slots in settings_per_account.xml.

use super::{CameraSettings, CameraShot, presets::Anchor};
use aurora_llsd::Llsd;
use std::path::Path;
use uuid::Uuid;

const MAX_FILE: u64 = 16 * 1024 * 1024;

pub struct Imported {
    pub slots: Vec<CameraShot>,
    /// Numbered slots with data that could not be used.
    pub invalid: Vec<u8>,
}

fn real(value: &Llsd) -> Option<f64> {
    match value {
        Llsd::Real(v) if v.is_finite() => Some(*v),
        Llsd::Integer(v) => Some(f64::from(*v)),
        _ => None,
    }
}

fn point(value: &Llsd) -> Option<[f64; 3]> {
    let values = value.as_array();
    if values.len() != 3 {
        return None;
    }
    Some([real(&values[0])?, real(&values[1])?, real(&values[2])?])
}

fn value<'a>(root: &'a Llsd, key: &str) -> &'a Llsd {
    root.get(key).get("Value")
}

pub fn parse(bytes: &[u8]) -> Result<Imported, String> {
    let root = aurora_llsd::from_xml(bytes).map_err(|e| format!("invalid LLSD XML: {e}"))?;
    if !root.is_map() {
        return Err("account settings are not an LLSD map".into());
    }
    let mut imported = Imported {
        slots: Vec::new(),
        invalid: Vec::new(),
    };
    for number in 1..=8u8 {
        let suffix = format!("{number:02}");
        let pos = value(&root, &format!("APStoredCameraPos_{suffix}"));
        let focus = value(&root, &format!("APStoredCameraFocus_{suffix}"));
        let roll = value(&root, &format!("APStoredCameraRoll_{suffix}"));
        let object = value(&root, &format!("APStoredCameraFocusObjectId_{suffix}"));
        if pos.is_undef() && focus.is_undef() && roll.is_undef() && object.is_undef() {
            continue;
        }
        let id = match object {
            Llsd::Undef => Some(None),
            Llsd::String(s) if s.is_empty() => Some(None),
            Llsd::String(s) => Uuid::parse_str(s).ok().map(|id| (!id.is_nil()).then_some(id)),
            _ => None,
        };
        let parsed = (point(pos), point(focus), real(roll).or_else(|| roll.is_undef().then_some(0.0)), id);
        let (Some(camera), Some(at), Some(roll), Some(id)) = parsed else {
            imported.invalid.push(number);
            continue;
        };
        // loadCameraPosition treats the all-zero values as an empty slot,
        // even if a name was typed into its separate text field.
        if camera == [0.0; 3] && at == [0.0; 3] && roll == 0.0 && id.is_none() {
            continue;
        }
        let name = match value(&root, &format!("saved_name_camera_view{suffix}")) {
            Llsd::String(s) if !s.trim().is_empty() => s.trim().chars().take(64).collect(),
            _ => format!("View {number}"),
        };
        let defaults = CameraSettings::default();
        let mut shot = CameraShot::at_avatar(name, defaults.camera_offset, defaults.focus_offset);
        shot.quick_slot = Some(number - 1);
        let roll = roll as f32;
        if !roll.is_finite() {
            imported.invalid.push(number);
            continue;
        }
        shot.roll = Some(roll);
        shot.quick_focus_object = id;
        shot.anchor = Some(Anchor::World { camera, focus: at });
        imported.slots.push(shot);
    }
    Ok(imported)
}

pub fn read(path: &Path) -> Result<Imported, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > MAX_FILE {
        return Err("account settings file exceeds 16 MiB".into());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_FILE {
        return Err("account settings file exceeds 16 MiB".into());
    }
    parse(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(root: &mut Llsd, key: &str, item: Llsd) {
        let mut map = Llsd::new_map();
        map.insert("Value", item);
        root.insert(key, map);
    }

    #[test]
    fn imports_named_world_slot_and_optional_focus_object() {
        let mut root = Llsd::new_map();
        entry(
            &mut root,
            "APStoredCameraPos_08",
            Llsd::Array(vec![Llsd::Real(1000.0), Llsd::Real(2000.0), Llsd::Real(30.0)]),
        );
        entry(
            &mut root,
            "APStoredCameraFocus_08",
            Llsd::Array(vec![Llsd::Real(1001.0), Llsd::Real(2002.0), Llsd::Real(31.0)]),
        );
        entry(&mut root, "APStoredCameraRoll_08", Llsd::Real(0.4));
        entry(
            &mut root,
            "APStoredCameraFocusObjectId_08",
            Llsd::String(Uuid::from_u128(9).to_string()),
        );
        entry(&mut root, "saved_name_camera_view08", Llsd::String("A cinema screen".into()));
        let imported = parse(&aurora_llsd::to_xml(&root)).unwrap();
        assert!(imported.invalid.is_empty());
        assert_eq!(imported.slots.len(), 1);
        let shot = &imported.slots[0];
        assert_eq!(shot.quick_slot, Some(7));
        assert_eq!(shot.name, "A cinema screen");
        assert_eq!(shot.roll, Some(0.4));
        assert_eq!(shot.quick_focus_object, Some(Uuid::from_u128(9)));
        assert!(shot.quick_fov.is_none());
        assert_eq!(
            shot.anchor,
            Some(Anchor::World {
                camera: [1000.0, 2000.0, 30.0],
                focus: [1001.0, 2002.0, 31.0]
            })
        );
    }

    #[test]
    fn skips_empty_and_broken_slots_without_exposing_other_settings() {
        let mut root = Llsd::new_map();
        entry(&mut root, "APStoredCameraPos_01", Llsd::Array(vec![Llsd::Real(0.0); 3]));
        entry(&mut root, "APStoredCameraFocus_01", Llsd::Array(vec![Llsd::Real(0.0); 3]));
        entry(&mut root, "APStoredCameraRoll_01", Llsd::Real(0.0));
        entry(&mut root, "APStoredCameraPos_02", Llsd::Array(vec![Llsd::Real(1.0); 2]));
        entry(&mut root, "APStoredCameraFocus_02", Llsd::Array(vec![Llsd::Real(4.0); 3]));
        entry(&mut root, "APStoredCameraRoll_02", Llsd::Real(0.0));
        entry(&mut root, "Unrelated", Llsd::String("private".into()));
        let imported = parse(&aurora_llsd::to_xml(&root)).unwrap();
        assert!(imported.slots.is_empty());
        assert_eq!(imported.invalid, [2]);
    }
}
