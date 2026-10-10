//! Which media face is under the cursor and where on its texture
//! (LLViewerObject::lineSegmentIntersect + LLPickInfo::mUVCoords): ray against
//! the triangles of the object's faces (prim volume or the cached mesh), then
//! the texture-entry transform of the hit face (LLFace texture matrix).

use glam::{Mat4, Vec2, Vec3};

/// A face's triangles in object space (unit scale), with LL texture coords.
pub struct FaceTris<'a> {
    pub face: usize,
    pub positions: &'a [[f32; 3]],
    pub uvs: &'a [[f32; 2]],
    pub indices: &'a [u16],
}

#[derive(Debug, Clone, Copy)]
pub struct FaceHit {
    pub face: usize,
    /// World-space distance along the ray.
    pub t: f32,
    /// Interpolated vertex texture coordinates (before the TE transform).
    pub uv: Vec2,
    pub normal: Vec3,
    pub binormal: Vec3,
}

/// Nearest intersection of a world ray with the faces (two-sided).
pub fn ray_faces<'a>(faces: impl Iterator<Item = FaceTris<'a>>, model: Mat4, origin: Vec3, dir: Vec3) -> Option<FaceHit> {
    ray_faces_with_sidedness(faces, model, origin, dir, |_| true)
}

/// As `ray_faces`, with each face's back-face policy. HUDs use the same
/// winding as the rasterizer; invisible backs must not intercept buttons.
pub fn ray_faces_with_sidedness<'a>(
    faces: impl Iterator<Item = FaceTris<'a>>,
    model: Mat4,
    origin: Vec3,
    dir: Vec3,
    two_sided: impl Fn(usize) -> bool,
) -> Option<FaceHit> {
    let inv = model.inverse();
    let o = inv.transform_point3(origin);
    let d = inv.transform_vector3(dir);
    let mut best: Option<FaceHit> = None;
    for f in faces {
        let two = two_sided(f.face);
        for tri in f.indices.as_chunks::<3>().0 {
            let (i0, i1, i2) = (tri[0] as usize, tri[1] as usize, tri[2] as usize);
            let (Some(p0), Some(p1), Some(p2)) = (f.positions.get(i0), f.positions.get(i1), f.positions.get(i2)) else {
                continue;
            };
            let (p0, p1, p2) = (Vec3::from_array(*p0), Vec3::from_array(*p1), Vec3::from_array(*p2));
            // Möller–Trumbore
            let e1 = p1 - p0;
            let e2 = p2 - p0;
            let pv = d.cross(e2);
            let det = e1.dot(pv);
            if det.abs() < 1e-12 || (!two && det < 0.0) {
                continue;
            }
            let inv_det = 1.0 / det;
            let tv = o - p0;
            let u = tv.dot(pv) * inv_det;
            if !(0.0..=1.0).contains(&u) {
                continue;
            }
            let qv = tv.cross(e1);
            let v = d.dot(qv) * inv_det;
            if v < 0.0 || u + v > 1.0 {
                continue;
            }
            let t = e2.dot(qv) * inv_det;
            if t <= 0.0 || best.is_some_and(|b| t >= b.t) {
                continue;
            }
            let uv_of = |i: usize| f.uvs.get(i).map(|a| Vec2::from_array(*a)).unwrap_or(Vec2::ZERO);
            let uv = uv_of(i0) * (1.0 - u - v) + uv_of(i1) * u + uv_of(i2) * v;
            let normal = inv.transpose().transform_vector3(e1.cross(e2)).normalize_or_zero();
            let duv1 = uv_of(i1) - uv_of(i0);
            let duv2 = uv_of(i2) - uv_of(i0);
            let uv_det = duv1.x * duv2.y - duv1.y * duv2.x;
            let binormal = if uv_det.abs() > 1e-8 {
                model.transform_vector3((e2 * duv1.x - e1 * duv2.x) / uv_det).normalize_or_zero()
            } else {
                Vec3::ZERO
            };
            best = Some(FaceHit {
                face: f.face,
                t,
                uv,
                normal,
                binormal,
            });
        }
    }
    // the object-space t equals the world-space t: the direction was
    // transformed without renormalizing
    best
}

/// Texture-entry transform about the face center (the shader's ll_uv
/// without the final V flip): LL texture coordinates (t up).
pub fn te_transform(uv: Vec2, scale: [f32; 2], offset: [f32; 2], rotation: f32) -> Vec2 {
    let (s, t) = (uv.x - 0.5, uv.y - 0.5);
    let (sa, ca) = rotation.sin_cos();
    let (s, t) = (s * ca + t * sa, -s * sa + t * ca);
    Vec2::new(s * scale[0] + offset[0] + 0.5, t * scale[1] + offset[1] + 0.5)
}

/// Media pixel under texture coordinates (LLViewerMediaImpl::scaleTextureCoords):
/// coordinates wrap into [0, 1); y counts from the top of the media.
pub fn media_pixel(st: Vec2, texture: (i32, i32), media: (i32, i32)) -> (i32, i32) {
    let wrap = |v: f32| {
        let f = v.fract();
        if f < 0.0 { f + 1.0 } else { f }
    };
    let (u, v) = (wrap(st.x), wrap(st.y));
    let x = (u * texture.0 as f32).round() as i32;
    let y = ((1.0 - v) * texture.1 as f32).round() as i32 - (texture.1 - media.1);
    (x, y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidedness_follows_triangle_winding_after_rotation_and_scale() {
        let positions = [[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.5, 0.5, 0.0], [-0.5, 0.5, 0.0]];
        let indices = [0, 1, 2, 0, 2, 3];
        let faces = || {
            std::iter::once(FaceTris {
                face: 2,
                positions: &positions,
                uvs: &[],
                indices: &indices,
            })
        };
        for angle in [0.0, 0.7, std::f32::consts::PI] {
            let model = Mat4::from_scale_rotation_translation(
                Vec3::new(2.0, 0.5, 1.5),
                glam::Quat::from_rotation_y(angle),
                Vec3::new(3.0, 4.0, 5.0),
            );
            let normal = model.transform_vector3(Vec3::Z).normalize();
            let center = model.transform_point3(Vec3::ZERO);
            for side in [-1.0, 1.0] {
                let origin = center + normal * side * 2.0;
                let direction = -normal * side;
                let single = ray_faces_with_sidedness(faces(), model, origin, direction, |_| false);
                assert_eq!(single.is_some(), side > 0.0);
                let double = ray_faces_with_sidedness(faces(), model, origin, direction, |face| face == 2).unwrap();
                assert!((double.t - 2.0).abs() < 1e-5);
            }
        }
    }

    #[test]
    fn backward_face_does_not_hide_a_visible_face_further_away() {
        let positions = [[-1.0, -1.0, 0.0], [1.0, -1.0, 0.0], [0.0, 1.0, 0.0]];
        let further = positions.map(|p| [p[0], p[1], 1.0]);
        let faces = [
            FaceTris {
                face: 0,
                positions: &positions,
                uvs: &[],
                indices: &[0, 1, 2],
            },
            FaceTris {
                face: 1,
                positions: &further,
                uvs: &[],
                indices: &[0, 2, 1],
            },
        ];
        let hit = ray_faces_with_sidedness(faces.into_iter(), Mat4::IDENTITY, -Vec3::Z, Vec3::Z, |_| false).unwrap();
        assert_eq!(hit.face, 1);
        assert_eq!(hit.t, 2.0);
    }

    #[test]
    fn hits_quad() {
        let pos = [[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.5, 0.5, 0.0], [-0.5, 0.5, 0.0]];
        let uvs = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let idx = [0u16, 1, 2, 0, 2, 3];
        let model = Mat4::from_scale(Vec3::new(2.0, 2.0, 1.0));
        let hit = ray_faces(
            std::iter::once(FaceTris {
                face: 3,
                positions: &pos,
                uvs: &uvs,
                indices: &idx,
            }),
            model,
            Vec3::new(0.5, 0.5, 5.0),
            Vec3::new(0.0, 0.0, -1.0),
        )
        .unwrap();
        assert_eq!(hit.face, 3);
        assert!((hit.t - 5.0).abs() < 1e-4);
        assert!((hit.uv - Vec2::new(0.75, 0.75)).length() < 1e-4);
        assert_eq!(media_pixel(Vec2::new(0.25, 0.75), (1024, 1024), (1024, 1024)), (256, 256));
        // media smaller than its texture sits at the bottom (GL rows)
        assert_eq!(media_pixel(Vec2::new(0.0, 0.5), (1024, 1024), (512, 512)), (0, 0));
        let st = te_transform(Vec2::new(0.25, 0.5), [1.0, 1.0], [0.0, 0.0], 0.0);
        assert!((st - Vec2::new(0.25, 0.5)).length() < 1e-5);
    }
}
