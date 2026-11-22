//! "Align planar faces" of the texture tab: the repeats, rotation and
//! offsets that make a planar-mapped face continue the texture of a
//! reference face, so that their edges meet without a seam.
//!
//! Follows LLFace::calcAlignedPlanarTE and getPlanarProjectedParams
//! (indra/newview/llface.cpp, originally LGPL 2.1) for what the user sees.
//! The computation is Aurora's own: each face's world-to-texture mapping is
//! built from the same planar projection the renderer draws
//! (`scene::picking::planar_st`, `planar_st` in object.wgsl) and the
//! texture-entry transform (`media::pick::te_transform`), and the target's
//! transform is solved so that both mappings agree on the target's plane.
//!
//! Deviation, better for the user: the Legacy Viewer measures the offset
//! between the objects' centres and reads the face orientation from the
//! first vertex's tangent, which only lines up coplanar faces of identical
//! prims and leaves a face of a prim without normal map unaligned (no
//! tangents). Here any two planar faces line up: coplanar ones continue the
//! texture across their shared plane, faces at an angle continue it across
//! their shared edge (the target's plane is turned about that edge onto the
//! reference's), whatever the prims' sizes and rotations.

use glam::{Mat2, Mat4, Quat, Vec2, Vec3};

/// Scale, offset and rotation (radians) of a texture layer, in the order
/// of LLTextureEntry (and of a legacy material's normal / specular map).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    pub scale: Vec2,
    pub offset: Vec2,
    pub rotation: f32,
}

/// Where a planar-mapped face lies in the world, and its planar
/// projection before the texture-entry transform.
#[derive(Clone, Copy, Debug)]
pub struct PlanarFace {
    /// The object's position (the projection's origin).
    origin: Vec3,
    /// `st - 0.5 = (u · (w - origin), v · (w - origin))` for a world point
    /// `w` of the face, before the texture-entry transform.
    u: Vec3,
    v: Vec3,
    /// Unit world normal and area-weighted centre of the face.
    normal: Vec3,
    center: Vec3,
}

/// Below this, a direction or a determinant is taken as degenerate.
const EPS: f32 = 1e-6;

/// Two face normals closer than this (sine of their angle) are parallel: no
/// shared edge to turn about.
const PARALLEL: f32 = 1e-4;

impl PlanarFace {
    /// A face from its triangles in unit-volume space (`PickFace`'s
    /// positions) and the drawn model matrix (position, rotation and scale
    /// of the prim). None for a face with no area.
    pub fn new(triangles: impl IntoIterator<Item = [Vec3; 3]>, model: Mat4) -> Option<Self> {
        let (scale, rotation, origin) = model.to_scale_rotation_translation();
        if scale.min_element().abs() < EPS {
            return None;
        }
        let mut unit_normal = Vec3::ZERO;
        let mut world_normal = Vec3::ZERO;
        let mut weighted_center = Vec3::ZERO;
        let mut area = 0.0;
        for [a, b, c] in triangles {
            unit_normal += (b - a).cross(c - a);
            let (a, b, c) = (model.transform_point3(a), model.transform_point3(b), model.transform_point3(c));
            let n = (b - a).cross(c - a);
            let w = n.length();
            world_normal += n;
            weighted_center += (a + b + c) / 3.0 * w;
            area += w;
        }
        if area < EPS {
            return None;
        }
        let unit_normal = unit_normal.try_normalize()?;
        let normal = world_normal.try_normalize()?;
        // planarProjection's axes, picked from the unit-volume normal as in
        // `planar_st`: s along `b`, t along `-(b × n)`, two repeats per metre
        let b = planar_binormal(unit_normal);
        let t = b.cross(unit_normal);
        Some(Self {
            origin,
            u: rotation * b * 2.0,
            v: rotation * t * -2.0,
            normal,
            center: weighted_center / area,
        })
    }

    /// Texture coordinates of world point `w` under `x`, as drawn.
    pub fn st(&self, x: &Xform, w: Vec3) -> Vec2 {
        linear(x) * self.raw(w) + x.offset + Vec2::splat(0.5)
    }

    /// The projection before the texture-entry transform, centred on 0.
    fn raw(&self, w: Vec3) -> Vec2 {
        let d = w - self.origin;
        Vec2::new(self.u.dot(d), self.v.dot(d))
    }
}

/// `planarProjection`'s binormal for a unit-volume normal (`planar_st`).
fn planar_binormal(n: Vec3) -> Vec3 {
    if n.x.abs() >= 0.5 {
        Vec3::new(0.0, if n.x < 0.0 { -1.0 } else { 1.0 }, 0.0)
    } else if n.y > 0.0 {
        -Vec3::X
    } else {
        Vec3::X
    }
}

/// Rotation then scale of the texture-entry transform, as a matrix on
/// `st - 0.5` (`te_transform`, LLFace's `xform`).
fn linear(x: &Xform) -> Mat2 {
    let (sa, ca) = x.rotation.sin_cos();
    Mat2::from_cols(
        Vec2::new(ca * x.scale.x, -sa * x.scale.y),
        Vec2::new(sa * x.scale.x, ca * x.scale.y),
    )
}

/// The transform that makes `target` continue the texture of `reference`
/// (drawn with `reference_xform`). None when the target's projection or the
/// result is degenerate (the face is then left as it is, like
/// calcAlignedPlanarTE's `false`).
pub fn aligned(reference: &PlanarFace, reference_xform: &Xform, target: &PlanarFace) -> Option<Xform> {
    // Carry the target's plane onto the reference's: turned about their
    // shared edge when they meet at an angle, as it is when they are
    // parallel (the reference's projection then reads it directly).
    let axis = target.normal.cross(reference.normal);
    let (turn, pivot) = if axis.length() > PARALLEL {
        let c = target.normal.dot(reference.normal);
        let (dt, dr) = (target.normal.dot(target.center), reference.normal.dot(reference.center));
        // the point of the shared edge nearest the world origin
        let pivot = ((dt - dr * c) * target.normal + (dr - dt * c) * reference.normal) / (1.0 - c * c);
        (Quat::from_rotation_arc(target.normal, reference.normal), pivot)
    } else {
        (Quat::IDENTITY, Vec3::ZERO)
    };
    let carried = |w: Vec3| pivot + turn * (w - pivot);

    // Linear part on the target's plane: D·R·G = L_r·P_r·turn, in the basis
    // (e1, e2) of the plane.
    let (e1, e2) = target.normal.any_orthonormal_pair();
    let g = Mat2::from_cols(
        Vec2::new(target.u.dot(e1), target.v.dot(e1)),
        Vec2::new(target.u.dot(e2), target.v.dot(e2)),
    );
    if g.determinant().abs() < EPS {
        return None;
    }
    let lr = linear(reference_xform);
    let h = Mat2::from_cols(
        lr * Vec2::new(reference.u.dot(turn * e1), reference.v.dot(turn * e1)),
        lr * Vec2::new(reference.u.dot(turn * e2), reference.v.dot(turn * e2)),
    );
    let k = h * g.inverse();
    // k = [[ms·c, ms·s], [-mt·s, mt·c]]: the rotation and the s repeats from
    // its first row, the t repeats (signed: a mirrored face) from the second
    let (k00, k01, k10, k11) = (k.x_axis.x, k.y_axis.x, k.x_axis.y, k.y_axis.y);
    let rotation = k01.atan2(k00);
    let (sa, ca) = rotation.sin_cos();
    let scale = Vec2::new(k00.hypot(k01), -k10 * sa + k11 * ca);
    if scale.x < EPS || scale.y.abs() < EPS || !scale.is_finite() || !rotation.is_finite() {
        return None;
    }
    let mut x = Xform {
        scale,
        offset: Vec2::ZERO,
        rotation,
    };
    // Offset: both mappings agree at the target's centre.
    let want = reference.st(reference_xform, carried(target.center));
    let offset = want - target.st(&x, target.center);
    if !offset.is_finite() {
        return None;
    }
    // whole repeats change nothing: keep it in (-1, 1) like calcAlignedPlanarTE
    x.offset = offset - offset.trunc();
    Some(x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::pick::te_transform;
    use crate::scene::picking::planar_st;

    /// A box prim's face at `+axis`, as two unit-volume triangles wound
    /// counter-clockwise seen from outside.
    fn box_face(axis: Vec3) -> Vec<[Vec3; 3]> {
        let n = axis.normalize();
        let (a, b) = n.any_orthonormal_pair();
        // a × b = n, so that (p10 - p00) × (p11 - p00) points along n
        let (a, b) = if a.cross(b).dot(n) > 0.0 { (a, b) } else { (b, a) };
        let c = n * 0.5;
        let p = |x: f32, y: f32| c + a * x + b * y;
        let (p00, p10, p11, p01) = (p(-0.5, -0.5), p(0.5, -0.5), p(0.5, 0.5), p(-0.5, 0.5));
        vec![[p00, p10, p11], [p00, p11, p01]]
    }

    fn model(pos: Vec3, rot: Quat, scale: Vec3) -> Mat4 {
        Mat4::from_scale_rotation_translation(scale, rot, pos)
    }

    /// The renderer's coordinates of a world point of a face: `planar_st`
    /// (object.wgsl) then the texture-entry transform.
    fn drawn(model: Mat4, unit_normal: Vec3, x: &Xform, w: Vec3) -> Vec2 {
        let (_, rot, pos) = model.to_scale_rotation_translation();
        // object space times the prim's scale, as `planar_st` takes it
        let st = planar_st(rot.inverse() * (w - pos), unit_normal);
        te_transform(st, x.scale.into(), x.offset.into(), x.rotation)
    }

    /// Same texel modulo whole repeats.
    fn same_texel(a: Vec2, b: Vec2) -> bool {
        let d = a - b;
        let off = (d - d.round()).abs();
        off.max_element() < 1e-3
    }

    fn sample_points(face: &[[Vec3; 3]], model: Mat4) -> Vec<Vec3> {
        let mut out = Vec::new();
        for [a, b, c] in face {
            for (u, v) in [(0.2, 0.2), (0.6, 0.1), (0.1, 0.7), (0.33, 0.33)] {
                let p = *a + (*b - *a) * u + (*c - *a) * v;
                out.push(model.transform_point3(p));
            }
        }
        out
    }

    #[test]
    fn the_projection_matches_the_renderer() {
        let face = box_face(Vec3::Z);
        let m = model(Vec3::new(10.0, 20.0, 30.0), Quat::from_rotation_z(0.7), Vec3::new(2.0, 3.0, 0.5));
        let f = PlanarFace::new(face.clone(), m).expect("face");
        let x = Xform {
            scale: Vec2::new(1.5, -0.75),
            offset: Vec2::new(0.25, -0.4),
            rotation: 0.9,
        };
        for w in sample_points(&face, m) {
            assert!((f.st(&x, w) - drawn(m, Vec3::Z, &x, w)).length() < 1e-4);
        }
    }

    #[test]
    fn coplanar_tops_of_different_prims_continue_the_texture() {
        let face = box_face(Vec3::Z);
        // reference: a 1 m box; target: a turned and stretched one beside it,
        // its top at the same height
        let mr = model(Vec3::new(0.0, 0.0, 1.0), Quat::IDENTITY, Vec3::ONE);
        let mt = model(Vec3::new(1.7, 0.3, 1.25), Quat::from_rotation_z(0.6), Vec3::new(2.0, 1.2, 0.5));
        let r = PlanarFace::new(face.clone(), mr).expect("reference");
        let t = PlanarFace::new(face.clone(), mt).expect("target");
        let rx = Xform {
            scale: Vec2::new(0.8, 1.3),
            offset: Vec2::new(0.1, 0.35),
            rotation: 0.4,
        };
        let tx = aligned(&r, &rx, &t).expect("aligned");
        for w in sample_points(&face, mt) {
            assert!(
                same_texel(drawn(mt, Vec3::Z, &tx, w), drawn(mr, Vec3::Z, &rx, w)),
                "{w}: {} vs {}",
                drawn(mt, Vec3::Z, &tx, w),
                drawn(mr, Vec3::Z, &rx, w)
            );
        }
        assert!(tx.offset.abs().max_element() < 1.0);
    }

    #[test]
    fn side_faces_of_two_prims_in_a_wall_line_up() {
        let face = box_face(Vec3::NEG_Y);
        let mr = model(Vec3::new(0.0, 0.0, 0.0), Quat::IDENTITY, Vec3::new(1.0, 0.2, 2.0));
        let mt = model(Vec3::new(2.5, 0.0, 0.5), Quat::IDENTITY, Vec3::new(4.0, 0.2, 3.0));
        let r = PlanarFace::new(face.clone(), mr).expect("reference");
        let t = PlanarFace::new(face.clone(), mt).expect("target");
        let rx = Xform {
            scale: Vec2::new(1.0, 1.0),
            offset: Vec2::ZERO,
            rotation: 0.0,
        };
        let tx = aligned(&r, &rx, &t).expect("aligned");
        for w in sample_points(&face, mt) {
            assert!(same_texel(drawn(mt, Vec3::NEG_Y, &tx, w), drawn(mr, Vec3::NEG_Y, &rx, w)));
        }
    }

    #[test]
    fn faces_at_an_angle_meet_on_their_shared_edge() {
        // top and +X side of one stretched box share the edge x = 1, z = 0.5
        let m = model(Vec3::ZERO, Quat::from_rotation_z(0.3), Vec3::new(2.0, 1.0, 1.0));
        let top = box_face(Vec3::Z);
        let side = box_face(Vec3::X);
        let r = PlanarFace::new(top, m).expect("top");
        let t = PlanarFace::new(side, m).expect("side");
        let rx = Xform {
            scale: Vec2::new(1.25, 0.75),
            offset: Vec2::new(0.3, 0.1),
            rotation: 0.5,
        };
        let tx = aligned(&r, &rx, &t).expect("aligned");
        for y in [-0.4, -0.1, 0.2, 0.45] {
            let w = m.transform_point3(Vec3::new(0.5, y, 0.5));
            assert!(same_texel(drawn(m, Vec3::X, &tx, w), drawn(m, Vec3::Z, &rx, w)), "edge point {y}");
        }
        // unfolded, not squashed: the same repeats per metre across the edge
        let step = |n: Vec3, x: &Xform, a: Vec3, b: Vec3| (drawn(m, n, x, b) - drawn(m, n, x, a)).length();
        let down = step(
            Vec3::X,
            &tx,
            m.transform_point3(Vec3::new(0.5, 0.0, 0.5)),
            m.transform_point3(Vec3::new(0.5, 0.0, 0.25)),
        );
        let across = step(
            Vec3::Z,
            &rx,
            m.transform_point3(Vec3::new(0.5, 0.0, 0.5)),
            m.transform_point3(Vec3::new(0.375, 0.0, 0.5)),
        );
        assert!((down - across).abs() < 1e-3, "{down} vs {across}");
    }

    #[test]
    fn a_face_aligned_to_itself_keeps_its_transform_modulo_repeats() {
        let face = box_face(Vec3::NEG_X);
        let m = model(Vec3::new(5.0, 5.0, 5.0), Quat::from_rotation_y(0.2), Vec3::new(1.0, 3.0, 2.0));
        let f = PlanarFace::new(face.clone(), m).expect("face");
        let x = Xform {
            scale: Vec2::new(2.0, 3.0),
            offset: Vec2::new(0.2, -0.3),
            rotation: -0.8,
        };
        let a = aligned(&f, &x, &f).expect("aligned");
        assert!((a.scale - x.scale).length() < 1e-4);
        assert!((a.rotation - x.rotation).abs() < 1e-4);
        assert!(same_texel(a.offset, x.offset));
    }

    /// On the prim generator's own box (the triangles `PickFace` carries):
    /// every face of a stretched, turned box lines up with the top of
    /// another one, checked with the generator's vertex normals.
    #[test]
    fn generated_box_faces_line_up_as_drawn() {
        let params = crate::build::shapes::shapes()[0].params();
        let mesh = aurora_prim::volume::generate_volume(&params, 1.0);
        let triangles = |f: &aurora_prim::volume::VolumeFace| -> Vec<[Vec3; 3]> {
            let p = |i: u16| Vec3::from_array(f.positions[i as usize]);
            f.indices.as_chunks::<3>().0.iter().map(|t| [p(t[0]), p(t[1]), p(t[2])]).collect()
        };
        let top = mesh.faces.iter().find(|f| f.normals[0][2] > 0.9).expect("a top face");
        let mr = model(Vec3::new(0.0, 0.0, 2.0), Quat::IDENTITY, Vec3::new(1.0, 1.0, 1.0));
        let r = PlanarFace::new(triangles(top), mr).expect("reference");
        let rx = Xform {
            scale: Vec2::new(1.4, 0.9),
            offset: Vec2::new(-0.2, 0.45),
            rotation: 0.25,
        };
        let mt = model(Vec3::new(1.2, -0.4, 1.9), Quat::from_rotation_z(-0.8), Vec3::new(1.6, 0.7, 1.2));
        for (i, f) in mesh.faces.iter().enumerate() {
            let t = PlanarFace::new(triangles(f), mt).expect("face");
            let tx = aligned(&r, &rx, &t).expect("aligned");
            let n = Vec3::from_array(f.normals[0]);
            let flat = triangles(f);
            for w in sample_points(&flat, mt) {
                // the reference's texture, carried onto this face's plane
                // the way `aligned` turns it (identity for the top)
                let axis = t.normal.cross(r.normal);
                let carried = if axis.length() > PARALLEL {
                    let c = t.normal.dot(r.normal);
                    let (dt, dr) = (t.normal.dot(t.center), r.normal.dot(r.center));
                    let pivot = ((dt - dr * c) * t.normal + (dr - dt * c) * r.normal) / (1.0 - c * c);
                    pivot + Quat::from_rotation_arc(t.normal, r.normal) * (w - pivot)
                } else {
                    w
                };
                assert!(
                    same_texel(drawn(mt, n, &tx, w), drawn(mr, Vec3::Z, &rx, carried)),
                    "face {i} at {w}"
                );
            }
        }
    }

    #[test]
    fn a_face_with_no_area_is_refused() {
        let flat = vec![[Vec3::ZERO, Vec3::X, Vec3::X * 2.0]];
        assert!(PlanarFace::new(flat, Mat4::IDENTITY).is_none());
        assert!(PlanarFace::new(box_face(Vec3::Z), Mat4::from_scale(Vec3::new(1.0, 0.0, 1.0))).is_none());
    }
}
