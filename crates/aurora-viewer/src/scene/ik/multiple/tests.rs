use super::*;
use crate::scene::avatar::AvatarLibrary;

#[test]
fn multiple_targets_blend_independent_material_points_and_rotations_without_row_order_bias() {
    let lib = AvatarLibrary::load();
    let rig = &lib.rig;
    let bone = rig.names["mWristRight"];
    let pose = LocalPose {
        rotations: rig.local_rot.clone(),
        positions: rig.local_pos.clone(),
    };
    let q = Quat::from_rotation_z(0.4);
    let origin = Vec3::new(0.2, -0.4, 1.1);
    let mut a = Goal {
        weight: 0.25,
        bone,
        root: rig.names["mShoulderRight"],
        position: origin,
        rotation: Some(q),
        contact: Vec3::new(0.1, 0.02, 0.0),
        pole: None,
        mode: Mode::Ik,
    };
    a.position += q * (a.contact * rig.scale[bone]);
    let mut b = a.clone();
    b.weight = 0.75;
    b.rotation = Some(-q);
    b.contact = Vec3::new(-0.05, 0.0, 0.06);
    b.position = origin + Vec3::X * 0.2 + q * (b.contact * rig.scale[bone]);
    let expected = origin + Vec3::X * 0.15;
    for peers in [[(&a, 1.0), (&b, 1.0)], [(&b, 1.0), (&a, 1.0)]] {
        let merged = combine(rig, &pose, Vec3::ZERO, &rig.scale, &peers).unwrap();
        assert!(merged.position.abs_diff_eq(expected, 1e-6));
        assert_eq!(merged.contact, Vec3::ZERO);
        assert!(merged.rotation.unwrap().dot(q).abs() > 0.999999);
        assert_eq!(merged.weight, 1.0);
    }
    let merged = combine(rig, &pose, Vec3::ZERO, &rig.scale, &[(&a, 0.5), (&b, 0.0)]).unwrap();
    assert!((merged.weight - 0.125).abs() < 1e-6);
    assert!(merged.position.abs_diff_eq(origin, 1e-6));
}
