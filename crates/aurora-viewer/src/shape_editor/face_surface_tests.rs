use super::*;
use crate::scene::avatar::AvatarLibrary;
use crate::shape_editor::handles::base_rest_joints;

/// A mesh bound on `joints`, so that its vertices are given where they are
/// drawn on them.
fn skin(lib: &AvatarLibrary, joints: &[Mat4], names: &[&str]) -> SkinInfo {
    SkinInfo {
        joint_names: names.iter().map(|n| n.to_string()).collect(),
        inverse_bind: names.iter().map(|n| joints[lib.rig.names[*n]].inverse()).collect(),
        ..SkinInfo::default()
    }
}

fn face(vertices: &[(Vec3, [u8; 4], [f32; 4])]) -> MeshFace {
    MeshFace {
        positions: vertices.iter().map(|(p, ..)| p.to_array()).collect(),
        joints: vertices.iter().map(|(_, j, _)| *j).collect(),
        weights: vertices.iter().map(|(.., w)| *w).collect(),
        indices: vec![0, 1, 2],
        ..MeshFace::default()
    }
}

/// A dot on the upper lip bone, where it is drawn, and its way out from the
/// head's centre.
fn lip_dot(lib: &AvatarLibrary, joints: &[Mat4]) -> (Anchor, Vec3, Vec3) {
    let anchor = Anchor::on(lib.rig.names["mFaceLipUpperCenter"], Vec3::new(0.005, 0.0, 0.0));
    let dot = anchor.at(joints).expect("lip bone");
    let centre = joints[lib.rig.names["HEAD"]].w_axis.truncate();
    (anchor, dot, (dot - centre).normalize())
}

/// A lip dot goes to the front of the lip on its line, though a Bento head
/// blends the lip across several bones: not to the inside of the mouth, nor
/// to skin beside its line.
#[test]
fn a_dot_goes_to_the_outermost_skin_on_its_line() {
    let lib = AvatarLibrary::load();
    let joints = base_rest_joints(&lib, false);
    let (anchor, dot, out) = lip_dot(&lib, &joints);
    let aside = out.any_orthonormal_vector();
    let front = dot + out * 0.02;
    let bound = skin(&lib, &joints, &["mFaceLipUpperCenter", "mFaceLipUpperLeft", "mHead"]);
    let faces = [face(&[
        // the lip, shared by two bones
        (front, [0, 1, 0, 0], [0.5, 0.5, 0.0, 0.0]),
        // inside the mouth
        (dot - out * 0.01, [0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]),
        // the nose, further out but beside the line
        (dot + out * 0.04 + aside * 0.02, [1, 2, 0, 0], [0.6, 0.4, 0.0, 0.0]),
    ])];
    let lifted = surface_anchors(&lib.rig, &joints, &[(&bound, &faces)], &[anchor]);
    let placed = joints[anchor.joint].transform_point3(lifted[&key(&anchor)]);
    assert!(placed.distance(front) < 1e-4, "{placed} {front}");
}

/// A dot with no skin on its line keeps its place, and a mesh mostly led by
/// the skull (a hair with a strand weighted to a lip) adds no surface.
#[test]
fn a_dot_without_face_skin_on_its_line_stays() {
    let lib = AvatarLibrary::load();
    let joints = base_rest_joints(&lib, false);
    let (anchor, dot, out) = lip_dot(&lib, &joints);
    let bound = skin(&lib, &joints, &["mHead", "mFaceLipUpperCenter"]);
    let mut hair: Vec<(Vec3, [u8; 4], [f32; 4])> = (0..9)
        .map(|i| {
            (
                dot + out * 0.01 + Vec3::Z * (0.1 + i as f32 * 0.01),
                [0, 0, 0, 0],
                [1.0, 0.0, 0.0, 0.0],
            )
        })
        .collect();
    hair.push((dot + out * 0.01, [1, 0, 0, 0], [1.0, 0.0, 0.0, 0.0]));
    let faces = [face(&hair)];
    assert!(surface_anchors(&lib.rig, &joints, &[(&bound, &faces)], &[anchor]).is_empty());
    // a face mesh, but nothing in front of the dot
    let bound = skin(&lib, &joints, &["mFaceLipUpperCenter"]);
    let faces = [face(&[(dot + Vec3::Z * 0.05, [0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0])])];
    assert!(surface_anchors(&lib.rig, &joints, &[(&bound, &faces)], &[anchor]).is_empty());
}

/// The surface follows the mesh's own binding: a head bound with its lip
/// bone elsewhere (joint positions) is read where it is drawn.
#[test]
fn the_surface_follows_the_mesh_binding() {
    let lib = AvatarLibrary::load();
    let joints = base_rest_joints(&lib, false);
    let (anchor, dot, out) = lip_dot(&lib, &joints);
    let joint = anchor.joint;
    let shift = Vec3::new(0.01, 0.0, 0.005);
    let mut bound = skin(&lib, &joints, &["mFaceLipUpperCenter"]);
    // bound with the lip bone further forward and up: a vertex given there
    // is drawn `shift` back, on the dot's line
    bound.inverse_bind[0] = (Mat4::from_translation(shift) * joints[joint]).inverse();
    let drawn = dot + out * 0.015;
    let faces = [face(&[(drawn + shift, [0, 0, 0, 0], [1.0, 0.0, 0.0, 0.0])])];
    let lifted = surface_anchors(&lib.rig, &joints, &[(&bound, &faces)], &[anchor]);
    let placed = joints[joint].transform_point3(lifted[&key(&anchor)]);
    assert!(placed.distance(drawn) < 1e-4, "{placed} {drawn}");
}
