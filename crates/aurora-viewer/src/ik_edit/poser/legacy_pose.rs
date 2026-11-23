//! The Legacy Viewer's saved poses: the LLSD XML files its poser writes in
//! `user_settings/poses`, one map per joint. A file becomes a pose
//! `Document`: loaded from Open a file, or converted into the library a whole
//! folder at a time; the original is never changed.
//!
//! Port of FSFloaterPoser::loadPoseFromXml (indra/newview/fsfloaterposer.cpp)
//! and FSPoserAnimator::loadJointRotation, loadJointPosition,
//! loadJointScale and translateRotationToQuaternion
//! (indra/newview/fsposeranimator.cpp), originally LGPL 2.1.
//!
//! Alchemy's released poser is another one, which writes the same map with
//! version 3: BDFloaterPoser::onPoseSave (indra/newview/bdfloaterposer.cpp)
//! and BDAnimator::loadPose (indra/newview/bdanimator.cpp) of its `main`
//! branch, Copyright © 2018 NiranV Dean, originally LGPL 2.1. Its Euler
//! angles are in another order (`alchemy_file`); its positions and scales
//! are whole values, as in the Legacy Viewer's files before version 4.
//! Alchemy's development branch has the Legacy Viewer's poser, whose files
//! read as the Legacy Viewer's.
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
    let alchemy = version <= 3 && alchemy_file(map);
    let mut joints: Vec<(usize, Joint)> = Vec::new();
    for (key, value) in map {
        // the other keys: version, startFromTeePose and the animations of
        // savePosingState, which name no joint
        let Some(&bone) = rig.names.get(key) else { continue };
        // an attachment point: Alchemy's file lists them, but neither poser
        // moves one (no FSPoserJoint of that name; BDAnimator::loadPose
        // gives them no joint state)
        if attachment_point(rig, bone) {
            continue;
        }
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
        // pitch, yaw), whose column form is Rx · Ry · Rz. Alchemy writes
        // getEulerAngles(&x, &z, &y) and reads setEulerAngles(x, z, y): its
        // file holds roll, yaw, pitch
        let (roll, pitch, yaw) = if alchemy {
            (euler.x, euler.z, euler.y)
        } else {
            (euler.x, euler.y, euler.z)
        };
        let rotation = Quat::from_euler(EulerRot::XYZ, roll, pitch, yaw);
        // a whole pelvis position is an offset from the Legacy Viewer's rest
        // at zero (LLAvatarAppearance::buildCharacter), which
        // `Document::apply` adds on the subject's own
        let rest_position = if bone == rig.pelvis { Vec3::ZERO } else { rig.local_pos[bone] };
        let (translation, scale) = match channels {
            Channels::LegacyAbsolute => (
                vector("position").unwrap_or(rest_position),
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

/// Whether a file of version 3 or less comes from Alchemy's poser. Both
/// posers wrote version 3, so the version does not tell them apart:
/// Alchemy's writes its collision volumes and attachment points without
/// `enabled` (BDFloaterPoser::onPoseSave keeps it for the bones), where the
/// Legacy Viewer's wrote it on every joint.
fn alchemy_file(map: &aurora_llsd::Map) -> bool {
    map.values().any(|joint| joint.has("rotation") && !joint.has("enabled"))
}

/// Whether `joint` is one of the attachment points `Rig::add_attachment_points`
/// adds: a point below a bone is in `skin_points`, "Avatar Center" (on
/// mRoot, which the skeleton has not) is a root other than the pelvis.
fn attachment_point(rig: &Rig, joint: usize) -> bool {
    rig.skin_points.contains_key(&rig.canonical_names[joint]) || (rig.parent[joint].is_none() && joint != rig.pelvis)
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

    /// A joint as Alchemy's BDFloaterPoser::onPoseSave writes a collision
    /// volume or an attachment point: no `enabled`.
    fn alchemy_extra(name: &str, position: [f32; 3], scale: [f32; 3]) -> String {
        let array = |v: [f32; 3]| format!("<array><real>{}</real><real>{}</real><real>{}</real></array>", v[0], v[1], v[2]);
        format!(
            "<key>{name}</key><map><key>rotation</key>{}<key>position</key>{}<key>scale</key>{}</map>",
            array([0.0; 3]),
            array(position),
            array(scale)
        )
    }

    /// An excerpt of a pose saved by Alchemy's poser (version 3, no
    /// startFromTeePose): bones with `enabled`, a collision volume and an
    /// attachment point without.
    fn alchemy_pose(elbow: [f32; 3]) -> Vec<u8> {
        file(
            3,
            None,
            &(joint("mElbowRight", true, elbow, [0.0, -0.248, 0.0], [0.996, 1.099, 0.996], "")
                + &joint("mPelvis", true, [0.0; 3], [0.0039, 0.0159, 0.0016], [0.996, 1.0, 0.946], "")
                + &alchemy_extra("L_CLAVICLE", [0.02, 0.0, 0.01844], [0.07, 0.12952, 0.04844])
                + &alchemy_extra("Avatar Center", [0.0; 3], [1.0; 3])),
        )
    }

    #[test]
    fn an_alchemy_file_holds_roll_yaw_pitch() {
        // BDFloaterPoser::onPoseSave: getEulerAngles(&x, &z, &y)
        let rig = rig();
        let bytes = alchemy_pose([0.462, 2.562, -0.0945]);
        let document = document(&bytes, "port11".into(), &rig).expect("pose");
        assert_eq!(document.channels, Channels::LegacyAbsolute);
        let expected = Quat::from_rotation_x(0.462) * Quat::from_rotation_y(-0.0945) * Quat::from_rotation_z(2.562);
        let elbow = find(&document, "mElbowRight").expect("elbow");
        // a dot product: angle_between's approximate acos is too coarse here
        assert!(elbow.rotation().dot(expected).abs() > 1.0 - 1e-6);
        assert!(!elbow.change());
        // the attachment point is left as it is
        assert!(rig.names.contains_key("Avatar Center") && rig.names.contains_key("Chest"));
        assert!(find(&document, "Avatar Center").is_none());
        assert!(attachment_point(&rig, rig.names["Chest"]));
        assert!(!attachment_point(&rig, rig.names["mChest"]));
        assert!(!attachment_point(&rig, rig.names["CHEST"]));
        assert!(!attachment_point(&rig, rig.pelvis));
        assert!(find(&document, "L_CLAVICLE").is_some());
    }

    #[test]
    fn a_legacy_file_of_version_three_keeps_roll_pitch_yaw() {
        // the Legacy Viewer's poser also wrote version 3, `enabled` on
        // every joint
        let rig = rig();
        let bytes = file(
            3,
            None,
            &(joint("mHead", true, [0.3, -0.7, 1.1], [0.0, 0.0, 0.076], [1.0; 3], "")
                + &joint("PELVIS", true, [0.0; 3], [0.0; 3], [0.1, 0.1, 0.1], "")),
        );
        let document = document(&bytes, "Look".into(), &rig).expect("pose");
        let expected = Quat::from_rotation_x(0.3) * Quat::from_rotation_y(-0.7) * Quat::from_rotation_z(1.1);
        assert!(find(&document, "mHead").expect("head").rotation().dot(expected).abs() > 1.0 - 1e-6);
    }

    #[test]
    fn a_whole_pelvis_position_is_an_offset_from_the_subject_rest() {
        // LLAvatarAppearance::buildCharacter puts the pelvis at zero: an
        // Alchemy pose must not sink the avatar into the ground
        let rig = rig();
        let mut session = session(&rig);
        let pelvis = rig.pelvis;
        let elbow = rig.names["mElbowRight"];
        let pose = document(&alchemy_pose([0.0; 3]), "port11".into(), &rig).expect("pose");
        assert!(pose.apply(&rig, &mut session, document::LoadMode::All, Mat4::IDENTITY));
        let offset = Vec3::new(0.0039, 0.0159, 0.0016);
        assert!((session.pose.positions[pelvis] - (rig.local_pos[pelvis] + offset)).length() < 1e-6);
        // the other joints keep the file's whole position
        assert!((session.pose.positions[elbow] - Vec3::new(0.0, -0.248, 0.0)).length() < 1e-6);
        // a file without the pelvis's position leaves it at rest
        let bytes = file(
            3,
            None,
            &joint("mPelvis", true, [0.1, 0.0, 0.0], [0.0; 3], [1.0; 3], "")
                .replace("<key>position</key><array><real>0</real><real>0</real><real>0</real></array>", ""),
        );
        let tilt = document(&bytes, "Tilt".into(), &rig).expect("pose");
        let mut other = self::session(&rig);
        assert!(tilt.apply(&rig, &mut other, document::LoadMode::All, Mat4::IDENTITY));
        assert!((other.pose.positions[pelvis] - rig.local_pos[pelvis]).length() < 1e-6);
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
            hold_rotation: vec![false; rig.len()],
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
