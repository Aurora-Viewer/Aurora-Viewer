use super::*;
use crate::scene::{avatar::AvatarLibrary, ik::Mode};

#[test]
fn symmetry_is_an_involution_in_rest_frames_for_every_paired_bone_and_contact() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let mut mirrors = Mirrors::default();
    mirrors.initialize(rig);
    let rotation = Quat::from_euler(glam::EulerRot::ZYX, 0.7, -0.5, 0.25);
    for bone in 0..rig.len() {
        let goal = Goal {
            follow: None,
            weight: 1.0,
            bone,
            root: crate::scene::ik::default_root(rig, bone),
            position: Vec3::new(0.1, 0.3, 1.5),
            rotation: Some(rotation),
            contact: Vec3::new(0.02, 0.055, -0.015),
            pole: Some(Vec3::new(-0.1, 0.6, 1.2)),
            mode: Mode::Ik,
        };
        let Some(other) = mirrors.goal(&goal) else {
            continue;
        };
        let roundtrip = mirrors.goal(&other).expect("paired counterpart");
        assert_eq!(roundtrip.bone, bone);
        assert_eq!(roundtrip.root, goal.root);
        assert!(roundtrip.position.distance(goal.position) < 1e-6);
        assert!(roundtrip.contact.distance(goal.contact) < 1e-6);
        assert_eq!(roundtrip.pole, goal.pole);
        assert!(roundtrip.rotation.unwrap().dot(rotation).abs() > 0.99999);
        let contact = other.rotation.unwrap() * other.contact;
        assert!(contact.distance(reflect(rotation * goal.contact)) < 1e-5);
    }
    assert!(mirrors.counterpart(rig.pelvis).is_none());
}

#[test]
fn limb_metadata_uses_complete_shoulder_and_hip_chains_and_finite_default_poles() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let frame = Frame {
        model: glam::Mat4::IDENTITY,
        joints: rig.world_with(&rig.local_rot, Vec3::ZERO, &rig.local_pos, &rig.scale),
        at: std::time::Instant::now(),
    };
    let controls = limbs(rig);
    assert_eq!(controls.len(), 4);
    for limb in controls {
        assert_eq!(
            crate::scene::ik::chain(rig, limb.end, limb.root).unwrap(),
            [limb.root, limb.middle, limb.end]
        );
        let pole = limb.pole(&frame);
        assert!(pole.is_finite());
        assert!(pole.distance(frame.joints[limb.middle].w_axis.truncate()) > 0.1);
    }
}
