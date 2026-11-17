//! The Legacy Viewer's saved poses: the LLSD XML files its poser writes in
//! `user_settings/poses`, one map per joint. Alchemy's poser is the same
//! (FSFloaterPoser in its sources), so its files read the same way. A file
//! becomes a pose `Document`: loaded from Open a file, or converted into the
//! library a whole folder at a time; the original is never changed.
//!
//! Port of FSFloaterPoser::loadPoseFromXml (indra/newview/fsfloaterposer.cpp)
//! and FSPoserAnimator::loadJointRotation, loadJointPosition,
//! loadJointScale and translateRotationToQuaternion
//! (indra/newview/fsposeranimator.cpp), originally LGPL 2.1.
//!
//! What a file of a pose started from a playing animation also records (its
//! `startFromTeePose` false, FSPoserAnimator::savePosingState) is the ids
//! of the animations it started from, to play them again
//! (FSPoserAnimator::loadPosingState). Those are someone's assets: they are
//! neither read nor kept, stricter than the Legacy Viewer. Such a pose's
//! joints are changes, applied on the pose the subject has when it loads.
use super::*;
use document::{Channels, Document, Joint};
use std::path::{Path, PathBuf};

/// Files a folder import reads at most, the folder's other files included.
const MAX_FOLDER_FILES: usize = 4096;

/// The `.xml` extension of FSFloaterPoser's POSE_INTERNAL_FORMAT_FILE_EXT.
pub(super) fn is_pose_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("xml"))
}

/// The pose a Legacy Viewer pose file gives, named `name`.
pub(super) fn document(bytes: &[u8], name: String, rig: &Rig) -> Result<Document, String> {
    let invalid = || aurora_i18n::tr!("poser-legacy-invalid").to_string();
    let llsd = aurora_llsd::from_xml(bytes).map_err(|_| invalid())?;
    let map = llsd.as_map().ok_or_else(invalid)?;
    // loadPoseFromXml: no version is 0, no startFromTeePose a pose from
    // the T-pose
    let version = map.get("version").map_or(0, |v| v.get("value").as_i32());
    let from_tee_pose = map.get("startFromTeePose").is_none_or(|v| v.get("value").as_bool());
    // version 4 made positions and scales a change from the joint's own
    let channels = if version > 3 {
        Channels::LegacyDelta
    } else {
        Channels::LegacyAbsolute
    };
    let mut joints: Vec<(usize, Joint)> = Vec::new();
    for (key, value) in map {
        // the other keys: version, startFromTeePose and the animations of
        // savePosingState, which name no joint
        let Some(&bone) = rig.names.get(key) else { continue };
        if !value.is_map() || joints.iter().any(|(b, _)| *b == bone) {
            continue;
        }
        // a joint left out of the posing keeps what moves it already
        if value.has("enabled") && !value.get("enabled").as_bool() {
            continue;
        }
        let vector = |field: &str| value.has(field).then(|| Vec3::from_array(value.get(field).as_vec3()));
        let euler = vector("rotation").unwrap_or(Vec3::ZERO);
        // translateRotationToQuaternion with SWAP_NOTHING: LLMatrix3(roll,
        // pitch, yaw), whose column form is Rx · Ry · Rz
        let rotation = Quat::from_euler(EulerRot::XYZ, euler.x, euler.y, euler.z);
        let (translation, scale) = match channels {
            Channels::LegacyAbsolute => (
                vector("position").unwrap_or(rig.local_pos[bone]),
                vector("scale").unwrap_or(rig.scale[bone]),
            ),
            _ => (vector("position").unwrap_or(Vec3::ZERO), vector("scale").unwrap_or(Vec3::ZERO)),
        };
        if !rotation.is_finite() || !translation.is_finite() || !scale.is_finite() {
            continue;
        }
        // FSJointPose::setPublicRotation: the joint's rotation is the file's
        // alone when its base is zeroed (a pose from the T-pose), the file's
        // on top of the subject's otherwise (getTargetRotation). A
        // collision volume keeps its base (FSJointPose::zeroBaseRotation).
        let zero_base = if value.has("jointBaseRotationIsZero") {
            value.get("jointBaseRotationIsZero").as_bool()
        } else {
            from_tee_pose
        };
        let collision_volume = !rig.canonical_names[bone].starts_with('m');
        let locked = value.get("worldLocked").as_bool();
        joints.push((
            bone,
            Joint::new(
                rig.canonical_names[bone].clone(),
                rotation,
                translation,
                scale,
                locked,
                !zero_base || collision_volume,
            ),
        ));
    }
    joints.sort_by_key(|(bone, _)| *bone);
    let name: String = name.chars().filter(|c| !c.is_control()).collect();
    let name = if name.trim().is_empty() { "Pose".to_owned() } else { name };
    let name = truncate(name, 400);
    // the poser's own priority: the file has none
    let document = Document::new(name, 4, joints.into_iter().map(|(_, j)| j).collect(), channels);
    if !document.valid(rig) {
        return Err(invalid());
    }
    Ok(document)
}

fn truncate(mut name: String, bytes: usize) -> String {
    if name.len() > bytes {
        let mut end = bytes;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    name
}

/// The name a pose file gives its pose: its file name without `.xml`, as
/// the Legacy Viewer's pose list shows it.
pub(super) fn pose_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// What a folder import did.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Imported {
    pub added: usize,
    /// A library pose of that name was already there: it is kept.
    pub kept: usize,
    pub unreadable: usize,
}

/// Every pose file of `from` (not its subfolders, such as the hand presets)
/// converted into the library `root`.
pub(super) fn import_folder(from: &Path, root: &Path, rig: &Rig) -> Result<Imported, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(from)
        .map_err(|e| e.to_string())?
        .take(MAX_FOLDER_FILES)
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && is_pose_file(p))
        .collect();
    files.sort();
    std::fs::create_dir_all(root).map_err(|e| e.to_string())?;
    let mut imported = Imported::default();
    for path in files {
        let converted = storage::read(&path).and_then(|bytes| document(&bytes, pose_name(&path), rig));
        let document = match converted {
            Ok(document) => document,
            Err(error) => {
                log::warn!("poser: {} not imported: {error}", path.display());
                imported.unreadable += 1;
                continue;
            }
        };
        let target = root.join(format!("{}.aurora-pose.json", clip::files::basename(&document.name)));
        if target.exists() {
            imported.kept += 1;
            continue;
        }
        let bytes = serde_json::to_vec_pretty(&document).map_err(|e| e.to_string())?;
        storage::write_atomic(root, &target, &bytes)?;
        imported.added += 1;
    }
    Ok(imported)
}

/// The first Legacy Viewer pose folder of this computer, where the folder
/// dialog of an import opens: `<viewer>/user_settings/poses`
/// (LL_PATH_USER_SETTINGS and POSE_SAVE_SUBDIRECTORY).
pub(super) fn default_folder() -> Option<PathBuf> {
    first_pose_folder(&crate::legacy_profile::app_folders())
}

fn first_pose_folder(apps: &[PathBuf]) -> Option<PathBuf> {
    apps.iter().map(|app| app.join("user_settings").join("poses")).find(|p| p.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig() -> Arc<Rig> {
        static RIG: std::sync::OnceLock<Arc<Rig>> = std::sync::OnceLock::new();
        RIG.get_or_init(|| crate::scene::avatar::AvatarLibrary::load().rig.clone()).clone()
    }

    /// A folder of its own under the tests' user data.
    fn temp_dir() -> PathBuf {
        crate::settings::account_dir(&Uuid::new_v4())
    }

    /// A file as FSFloaterPoser::savePoseToXml writes it (LLSD pretty XML).
    fn file(version: i32, tee: Option<bool>, joints: &str) -> Vec<u8> {
        let tee = tee.map_or(String::new(), |t| {
            format!(
                "<key>startFromTeePose</key><map><key>value</key><boolean>{}</boolean></map>",
                u8::from(t)
            )
        });
        format!(
            "<?xml version=\"1.0\" ?><llsd><map><key>version</key><map><key>value</key><integer>{version}</integer></map>{tee}{joints}</map></llsd>"
        )
        .into_bytes()
    }

    fn joint(name: &str, enabled: bool, rotation: [f32; 3], position: [f32; 3], scale: [f32; 3], extra: &str) -> String {
        let array = |v: [f32; 3]| format!("<array><real>{}</real><real>{}</real><real>{}</real></array>", v[0], v[1], v[2]);
        format!(
            "<key>{name}</key><map><key>enabled</key><boolean>{}</boolean><key>rotation</key>{}<key>position</key>{}<key>scale</key>{}{extra}</map>",
            u8::from(enabled),
            array(rotation),
            array(position),
            array(scale)
        )
    }

    fn find<'a>(document: &'a Document, name: &str) -> Option<&'a Joint> {
        document.joints.iter().find(|j| j.name == name)
    }

    #[test]
    fn a_tee_pose_file_gives_complete_rotations() {
        let rig = rig();
        let bytes = file(8, Some(true), &joint("mElbowLeft", true, [0.5, 0.0, 0.0], [0.0; 3], [0.0; 3], ""));
        let document = document(&bytes, "Wave".into(), &rig).expect("pose");
        assert_eq!(document.name, "Wave");
        assert_eq!(document.channels, Channels::LegacyDelta);
        let elbow = find(&document, "mElbowLeft").expect("elbow");
        assert!(!elbow.change());
        assert!(elbow.rotation().angle_between(Quat::from_rotation_x(0.5)) < 1e-5);
    }

    #[test]
    fn the_rotation_follows_the_legacy_matrix_order() {
        // LLMatrix3::setRot(roll, pitch, yaw): Rx · Ry · Rz in column form
        let rig = rig();
        let bytes = file(8, Some(true), &joint("mHead", true, [0.3, -0.7, 1.1], [0.0; 3], [0.0; 3], ""));
        let document = document(&bytes, "Look".into(), &rig).expect("pose");
        let expected = Quat::from_rotation_x(0.3) * Quat::from_rotation_y(-0.7) * Quat::from_rotation_z(1.1);
        assert!(find(&document, "mHead").expect("head").rotation().angle_between(expected) < 1e-5);
    }

    #[test]
    fn a_pose_from_an_animation_gives_changes() {
        let rig = rig();
        let bytes = file(
            8,
            Some(false),
            &(joint("mHead", true, [0.2, 0.0, 0.0], [0.0; 3], [0.0; 3], "")
                + &joint(
                    "mNeck",
                    true,
                    [0.1, 0.0, 0.0],
                    [0.0; 3],
                    [0.0; 3],
                    "<key>jointBaseRotationIsZero</key><boolean>1</boolean>",
                )
                + "<key>3f6a8e0c-0000-4000-8000-000000000000</key><map><key>animationId</key><string>3f6a8e0c-0000-4000-8000-000000000001</string></map>"),
        );
        let document = document(&bytes, "Lean".into(), &rig).expect("pose");
        assert!(find(&document, "mHead").expect("head").change());
        // a joint zeroed by the user is complete again
        assert!(!find(&document, "mNeck").expect("neck").change());
        // the animations it started from are not kept
        assert_eq!(document.joints.len(), 2);
        assert!(!serde_json::to_string(&document).expect("json").contains("3f6a8e0c"));
    }

    fn session(rig: &Rig) -> data::Session {
        data::Session::new(data::Pose {
            rotations: rig.local_rot.clone(),
            positions: rig.local_pos.clone(),
            scales: rig.scale.clone(),
            locks: std::collections::HashMap::new(),
            origin: rig.local_rot.clone(),
        })
    }

    #[test]
    fn a_loaded_file_poses_the_joints_and_adds_its_scales() {
        let rig = rig();
        let mut session = session(&rig);
        let chest = rig.names["mChest"];
        let head = rig.names["mHead"];
        let turned = Quat::from_rotation_z(0.4);
        session.pose = Arc::new({
            let mut pose = (*session.pose).clone();
            pose.rotations[chest] = turned;
            pose.origin[chest] = turned;
            pose
        });
        let bytes = file(
            8,
            Some(false),
            &(joint("mHead", true, [0.0, 0.3, 0.0], [0.0; 3], [0.25, 0.0, 0.0], "")
                + &joint("mChest", true, [0.2, 0.0, 0.0], [0.0; 3], [0.0; 3], "")),
        );
        let document = document(&bytes, "Lean".into(), &rig).expect("pose");
        assert!(document.apply(&rig, &mut session, document::LoadMode::All, Mat4::IDENTITY));
        // a change goes on top of what the joint had (getTargetRotation)
        let expected = (turned * Quat::from_rotation_x(0.2)).normalize();
        assert!(session.pose.rotations[chest].angle_between(expected) < 1e-5);
        assert!(session.pose.rotations[head].angle_between(rig.local_rot[head] * Quat::from_rotation_y(0.3)) < 1e-5);
        // FSJointState::getTargetScale: the file's scale is added
        let scale = session.pose.scales[head];
        assert!((scale - (rig.scale[head] + Vec3::new(0.25, 0.0, 0.0))).length() < 1e-6);
        // the rotations-only load leaves the scale
        let mut other = self::session(&rig);
        assert!(document.apply(&rig, &mut other, document::LoadMode::Rotations, Mat4::IDENTITY));
        assert_eq!(other.pose.scales[head], rig.scale[head]);
    }

    #[test]
    fn disabled_joints_and_unknown_keys_are_left_out() {
        let rig = rig();
        let bytes = file(
            8,
            Some(true),
            &(joint("mHead", false, [0.2, 0.0, 0.0], [0.0; 3], [0.0; 3], "")
                + &joint("mNotAJoint", true, [0.2, 0.0, 0.0], [0.0; 3], [0.0; 3], "")
                + &joint("mChest", true, [0.2, 0.0, 0.0], [0.0; 3], [0.0; 3], "")),
        );
        let document = document(&bytes, "Chest".into(), &rig).expect("pose");
        assert_eq!(document.joints.iter().map(|j| j.name.as_str()).collect::<Vec<_>>(), ["mChest"]);
    }

    #[test]
    fn collision_volumes_keep_their_base() {
        let rig = rig();
        let Some(volume) = rig.canonical_names.iter().find(|n| !n.starts_with('m')).cloned() else {
            return;
        };
        let bytes = file(8, Some(true), &joint(&volume, true, [0.1, 0.0, 0.0], [0.0; 3], [0.0; 3], ""));
        let document = document(&bytes, "Volume".into(), &rig).expect("pose");
        assert!(find(&document, &volume).expect("volume").change());
    }

    #[test]
    fn versions_before_four_give_whole_positions_and_scales() {
        let rig = rig();
        let bytes = file(3, None, &joint("mHead", true, [0.0; 3], [0.0, 0.0, 0.1], [1.0, 1.0, 1.0], ""));
        let document = document(&bytes, "Old".into(), &rig).expect("pose");
        assert_eq!(document.channels, Channels::LegacyAbsolute);
        let head = find(&document, "mHead").expect("head");
        assert_eq!(head.translation(), Vec3::new(0.0, 0.0, 0.1));
        // no startFromTeePose: a pose from the T-pose
        assert!(!head.change());
    }

    #[test]
    fn the_world_lock_is_kept() {
        let rig = rig();
        let bytes = file(
            8,
            Some(true),
            &joint(
                "mWristLeft",
                true,
                [0.0; 3],
                [0.0; 3],
                [0.0; 3],
                "<key>worldLocked</key><boolean>1</boolean>",
            ),
        );
        let document = document(&bytes, "Lock".into(), &rig).expect("pose");
        assert!(find(&document, "mWristLeft").expect("wrist").locked());
    }

    #[test]
    fn broken_files_are_refused() {
        let rig = rig();
        for bytes in [
            b"not xml".to_vec(),
            b"<llsd><array/></llsd>".to_vec(),
            file(8, Some(true), ""),
            file(8, Some(true), &joint("mHead", true, [f32::NAN, 0.0, 0.0], [0.0; 3], [0.0; 3], "")),
            file(8, Some(true), &joint("mHead", true, [0.0; 3], [500.0, 0.0, 0.0], [0.0; 3], "")),
        ] {
            assert!(document(&bytes, "Broken".into(), &rig).is_err());
        }
    }

    #[test]
    fn a_folder_imports_once_and_keeps_existing_poses() {
        let rig = rig();
        let dir = temp_dir();
        let from = dir.join("poses");
        let root = dir.join("library");
        std::fs::create_dir_all(from.join("hand_presets")).expect("folders");
        let pose = file(8, Some(true), &joint("mHead", true, [0.2, 0.0, 0.0], [0.0; 3], [0.0; 3], ""));
        std::fs::write(from.join("Sit.xml"), &pose).expect("write");
        std::fs::write(from.join("Stand.XML"), &pose).expect("write");
        std::fs::write(from.join("hand_presets").join("Fist.xml"), &pose).expect("write");
        std::fs::write(from.join("broken.xml"), b"<llsd>").expect("write");
        std::fs::write(from.join("notes.txt"), b"hello").expect("write");
        assert_eq!(
            import_folder(&from, &root, &rig).expect("import"),
            Imported {
                added: 2,
                kept: 0,
                unreadable: 1
            }
        );
        assert!(root.join("Sit.aurora-pose.json").is_file());
        assert!(root.join("Stand.aurora-pose.json").is_file());
        assert!(!root.join("Fist.aurora-pose.json").exists());
        // the pose reads back as a library pose
        let saved: Document = serde_json::from_slice(&std::fs::read(root.join("Sit.aurora-pose.json")).expect("read")).expect("json");
        assert!(saved.valid(&rig));
        assert_eq!(saved.channels, Channels::LegacyDelta);
        // a second import changes nothing
        assert_eq!(
            import_folder(&from, &root, &rig).expect("import"),
            Imported {
                added: 0,
                kept: 2,
                unreadable: 1
            }
        );
        // the originals are untouched
        assert_eq!(std::fs::read(from.join("Sit.xml")).expect("read"), pose);
    }

    #[test]
    fn the_dialog_opens_in_the_first_viewer_with_poses() {
        let dir = temp_dir();
        let first = dir.join("Firestorm_x64");
        let second = dir.join("Alchemy");
        std::fs::create_dir_all(second.join("user_settings").join("poses")).expect("folders");
        std::fs::create_dir_all(first.join("user_settings")).expect("folders");
        assert_eq!(
            first_pose_folder(&[first, second.clone()]),
            Some(second.join("user_settings").join("poses"))
        );
        assert_eq!(first_pose_folder(&[]), None);
    }
}
