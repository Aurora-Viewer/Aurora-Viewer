//! BVH Y-up/inches with a neutral reference frame and two static pose frames.
//! Inverse of import_animation::parse_bvh's makeTranslation basis conversion.
use super::*;
use glam::Mat3;
use std::fmt::Write;

pub(super) fn encode(rig: &Rig, pose: &SubjectPose, unlock_pelvis: bool) -> Result<String, String> {
    if !pose.valid(rig) {
        return Err(aurora_i18n::tr!("anim-edit-invalid").to_string());
    }
    let basis = Quat::from_mat3(&Mat3::from_cols(Vec3::Y, Vec3::Z, Vec3::X));
    let mut out = String::from("HIERARCHY\n");
    let mut order = Vec::new();
    fn joint(rig: &Rig, pose: &SubjectPose, bone: usize, depth: usize, basis: Quat, out: &mut String, order: &mut Vec<usize>) {
        let root = bone == rig.pelvis;
        let indent = "  ".repeat(depth);
        let name = if root { "hip" } else { &rig.canonical_names[bone] };
        let _ = writeln!(out, "{indent}{} {name}\n{indent}{{", if root { "ROOT" } else { "JOINT" });
        let offset = if root {
            Vec3::ZERO
        } else {
            let scale = rig.parent[bone].map_or(rig.parent_scale[bone], |p| pose.scales[p]);
            basis.inverse() * (pose.positions[bone] * scale) / 0.0254
        };
        let _ = writeln!(out, "{indent}  OFFSET {:.6} {:.6} {:.6}", offset.x, offset.y, offset.z);
        let _ = writeln!(
            out,
            "{indent}  CHANNELS {}{}Zrotation Xrotation Yrotation",
            if root { 6 } else { 3 },
            if root { " Xposition Yposition Zposition " } else { " " }
        );
        order.push(bone);
        let children: Vec<_> = rig
            .order
            .iter()
            .copied()
            .filter(|b| rig.parent[*b] == Some(bone) && rig.canonical_names[*b].starts_with('m'))
            .collect();
        if children.is_empty() {
            let _ = writeln!(out, "{indent}  End Site\n{indent}  {{\n{indent}    OFFSET 0 0.1 0\n{indent}  }}");
        }
        for child in children {
            joint(rig, pose, child, depth + 1, basis, out, order);
        }
        let _ = writeln!(out, "{indent}}}");
    }
    joint(rig, pose, rig.pelvis, 0, basis, &mut out, &mut order);
    out.push_str("MOTION\nFrames: 3\nFrame Time: 1.0\n");
    for frame in 0..3 {
        for &bone in &order {
            if bone == rig.pelvis {
                let p = if unlock_pelvis && frame > 0 {
                    basis.inverse() * (pose.positions[bone] - rig.local_pos[bone]) / 0.0254
                } else {
                    Vec3::ZERO
                };
                let _ = write!(out, "{:.6} {:.6} {:.6} ", p.x, p.y, p.z);
            }
            let q = if frame == 0 || bone == rig.pelvis && !unlock_pelvis {
                Quat::IDENTITY
            } else {
                basis.inverse() * pose.rotations[bone] * basis
            };
            // Declared Z/X/Y channels are applied successively: Hamilton Y*X*Z.
            let (y, x, z) = q.to_euler(EulerRot::YXZ);
            let _ = write!(out, "{:.6} {:.6} {:.6} ", z.to_degrees(), x.to_degrees(), y.to_degrees());
        }
        out.push('\n');
    }
    Ok(out)
}
