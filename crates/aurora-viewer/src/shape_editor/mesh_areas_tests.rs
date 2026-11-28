use super::*;
use crate::scene::avatar::AvatarLibrary;
use crate::shape_editor::handles::{AREA_ARMS, AREA_HEAD_NECK, AREA_LEGS, AREA_TORSO, derive};

fn class(areas: &MeshAreas, joint: usize) -> u32 {
    (areas.classes[joint >> 3] >> ((joint & 7) * 4)) & 15
}

fn joint(lib: &AvatarLibrary, name: &str) -> usize {
    lib.rig.names[name]
}

/// The body's skin picks the area its bones are in: a feral's hind limbs
/// are its legs, a fitted body's volumes their bone's area, the system
/// body's root slot the torso.
#[test]
fn the_body_skin_belongs_to_the_area_of_its_bones() {
    let lib = AvatarLibrary::load();
    let layout = derive(&lib, false, Part::Body, false);
    let shown = [AREA_HEAD_NECK, AREA_TORSO, AREA_ARMS, AREA_LEGS];
    let areas = MeshAreas::new(&lib.rig, &layout, &shown);
    let area = |name: &str| areas.area(class(&areas, joint(&lib, name)));
    assert_eq!(area("mHead"), Some(AREA_HEAD_NECK));
    assert_eq!(area("mFaceNoseBase"), Some(AREA_HEAD_NECK));
    assert_eq!(area("mChest"), Some(AREA_TORSO));
    assert_eq!(area("BELLY"), Some(AREA_TORSO));
    assert_eq!(area("mElbowLeft"), Some(AREA_ARMS));
    assert_eq!(area("L_LOWER_ARM"), Some(AREA_ARMS));
    assert_eq!(area("mKneeRight"), Some(AREA_LEGS));
    assert_eq!(area("mHindLimb2Left"), Some(AREA_LEGS));
    assert_eq!(area("BUTT"), Some(AREA_LEGS));
    assert_eq!(areas.area(class(&areas, lib.rig.len())), Some(AREA_TORSO));
    // an area's tint lights its own joints and no other's
    let arms = areas.mask(AREA_ARMS);
    let lit = |name: &str| {
        let j = joint(&lib, name);
        arms[j >> 5] & (1 << (j & 31)) != 0
    };
    assert!(lit("mWristLeft") && lit("mShoulderRight"));
    assert!(!lit("mChest") && !lit("mKneeLeft"));
}

/// Skin of an area with no dot shown is no class: nothing to pick there.
#[test]
fn an_area_without_dots_is_not_picked() {
    let lib = AvatarLibrary::load();
    let layout = derive(&lib, false, Part::Body, false);
    let areas = MeshAreas::new(&lib.rig, &layout, &[AREA_TORSO, AREA_LEGS]);
    assert_eq!(class(&areas, joint(&lib, "mElbowLeft")), 0);
    assert_eq!(areas.mask(AREA_ARMS), [0; 8]);
    assert_eq!(areas.area(class(&areas, joint(&lib, "mKneeLeft"))), Some(AREA_LEGS));
}

/// Under a mesh head, each face bone is the group of its dots: the lips
/// the mouth's, the nose bones the nose's, the skull the head's; the body
/// is no face area.
#[test]
fn a_mesh_head_skin_picks_the_group_of_its_face_bones() {
    let lib = AvatarLibrary::load();
    let layout = derive(&lib, false, Part::Head, false);
    let shown: Vec<usize> = layout.handles.iter().map(|h| h.area).collect();
    let areas = MeshAreas::new(&lib.rig, &layout, &shown);
    let area = |name: &str| areas.area(class(&areas, joint(&lib, name)));
    // the shape editor's groups (`catalog::Param::group`)
    let group = |name: &str| {
        ["body", "head", "eyes", "ears", "nose", "mouth", "chin"]
            .iter()
            .position(|g| *g == name)
            .expect("group")
    };
    assert_eq!(area("mHead"), Some(group("head")));
    assert_eq!(area("mFaceLipCornerLeft"), Some(group("mouth")));
    assert_eq!(area("mFaceNoseBase"), Some(group("nose")));
    assert_eq!(area("mEyeLeft"), Some(group("eyes")));
    // the lids and eye corners are the eyes', the brows the forehead's (Outer
    // Eye Corner moves the outer brow bone)
    for name in [
        "mFaceEyeLidUpperLeft",
        "mFaceEyeLidLowerRight",
        "mFaceEyecornerInnerLeft",
        "mFaceEyeAltRight",
    ] {
        assert_eq!(area(name), Some(group("eyes")), "{name}");
    }
    for name in ["mFaceEyebrowOuterLeft", "mFaceEyebrowCenterRight", "mFaceEyebrowInnerLeft"] {
        assert_eq!(area(name), Some(group("head")), "{name}");
    }
    assert_eq!(area("mChest"), None);
    // every face bone belongs to some group
    for (j, name) in lib.rig.canonical_names.iter().enumerate() {
        if name.starts_with("mFace") {
            assert!(class(&areas, j) != 0, "{name}");
        }
    }
}
