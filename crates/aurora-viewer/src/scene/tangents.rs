//! Per-vertex tangents of the normal maps, packed into each vertex
//! (aurora_render::pack_tangent), as the Legacy Viewer builds them when a
//! face is loaded:
//! - prims and sculpts: LLVolumeFace::createTangents
//!   (LLCalculateTangentArray, indra/llmath/llvolume.cpp, originally LGPL
//!   2.1), one tangent per vertex, never a new vertex;
//! - meshes: MikkTSpace in LLVolumeFace::cacheOptimize, on the mesh in its
//!   upload proportions (`NormalizedScale`), the vertices split where two
//!   corners of one disagree, then welded again; past the 16-bit index limit,
//!   createTangents instead.
//!
//! Faces whose vertices carry no tangent keep the per-pixel frame of
//! perturb_normal (common.wgsl).

use aurora_render::{Vertex, pack_tangent};
use glam::{Vec2, Vec3};
use std::collections::HashMap;

/// One vertex's tangent in the face's frame, and its handedness (±1).
pub type Tangent = ([f32; 3], f32);

fn normal_of(v: &Vertex) -> Vec3 {
    Vec3::new(v.normal[0] as f32, v.normal[1] as f32, v.normal[2] as f32) / 127.0
}

/// Packs Lengyel's tangents into the vertices of a face
/// (LLCalculateTangentArray): the sum of the faces' dP/ds made orthogonal to
/// the normal, its handedness from the sum of their dP/dt.
pub fn pack_volume_tangents(vertices: &mut [Vertex], indices: &[u16]) {
    let tangents = volume_tangents(vertices_view(vertices), indices);
    for (v, (t, hand)) in vertices.iter_mut().zip(tangents) {
        v.normal[3] = pack_tangent(v.normal, t, hand);
    }
}

fn vertices_view(vertices: &[Vertex]) -> impl Iterator<Item = (Vec3, Vec3, Vec2)> + Clone + '_ {
    vertices
        .iter()
        .map(|v| (Vec3::from_array(v.pos), normal_of(v), Vec2::from_array(v.uv)))
}

fn volume_tangents(vertices: impl Iterator<Item = (Vec3, Vec3, Vec2)> + Clone, indices: &[u16]) -> Vec<Tangent> {
    let count = vertices.clone().count();
    let (p, uv): (Vec<Vec3>, Vec<Vec2>) = vertices.clone().map(|(p, _, uv)| (p, uv)).unzip();
    let mut tan1 = vec![Vec3::ZERO; count];
    let mut tan2 = vec![Vec3::ZERO; count];
    for tri in indices.as_chunks::<3>().0 {
        let [i1, i2, i3] = tri.map(|i| i as usize);
        if i1 >= count || i2 >= count || i3 >= count {
            continue;
        }
        let (e1, e2) = (p[i2] - p[i1], p[i3] - p[i1]);
        let (w1, w2) = (uv[i2] - uv[i1], uv[i3] - uv[i1]);
        let rd = w1.x * w2.y - w2.x * w1.y;
        // LL's "made up large ratio" for UVs without area
        let r = if rd * rd > f32::EPSILON {
            1.0 / rd
        } else if rd > 0.0 {
            1024.0
        } else {
            -1024.0
        };
        let sdir = (e1 * w2.y - e2 * w1.y) * r;
        let tdir = (e2 * w1.x - e1 * w2.x) * r;
        for i in [i1, i2, i3] {
            tan1[i] += sdir;
            tan2[i] += tdir;
        }
    }
    vertices
        .zip(tan1.iter().zip(&tan2))
        .map(|((_, n, _), (&t, &t2))| {
            let tp = t - n * n.dot(t);
            // F_APPROXIMATELY_ZERO
            if tp.length_squared() > 1e-5 {
                let hand = if n.cross(t).dot(t2) < 0.0 { -1.0 } else { 1.0 };
                (tp.normalize().to_array(), hand)
            } else {
                ([0.0, 0.0, 1.0], 1.0)
            }
        })
        .collect()
}

/// MikkTSpace input as LLVolumeFace::cacheOptimize gives it (MikktData):
/// positions scaled back to the mesh's upload proportions, normals by the
/// inverse scale.
struct Mikk<'a> {
    positions: Vec<Vec3>,
    normals: Vec<Vec3>,
    uvs: &'a [[f32; 2]],
    indices: &'a [u16],
    corners: Vec<Tangent>,
}

impl bevy_mikktspace::Geometry for Mikk<'_> {
    fn num_faces(&self) -> usize {
        self.indices.len() / 3
    }

    fn num_vertices_of_face(&self, _face: usize) -> usize {
        3
    }

    fn position(&self, face: usize, vert: usize) -> [f32; 3] {
        self.positions[self.indices[face * 3 + vert] as usize].to_array()
    }

    fn normal(&self, face: usize, vert: usize) -> [f32; 3] {
        self.normals[self.indices[face * 3 + vert] as usize].to_array()
    }

    fn tex_coord(&self, face: usize, vert: usize) -> [f32; 2] {
        self.uvs[self.indices[face * 3 + vert] as usize]
    }

    fn set_tangent(&mut self, tangent: Option<bevy_mikktspace::TangentSpace>, face: usize, vert: usize) {
        // the reference implementation's default for a corner it cannot
        // place (a degenerate face without neighbours)
        let t = tangent.unwrap_or_default().tangent_encoded();
        self.corners[face * 3 + vert] = ([t[0], t[1], t[2]], t[3]);
    }
}

/// MikkTSpace tangents of a mesh face, one per corner (`indices` order),
/// brought back to its normalized frame as LLVolumeFace::cacheOptimize
/// does (times the scale, normalized). `None` when the face has no normals
/// or no texture coordinates, or an index out of range.
pub fn mesh_corner_tangents(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u16],
    scale: Vec3,
) -> Option<Vec<Tangent>> {
    let count = positions.len();
    if normals.len() != count || uvs.len() != count || indices.iter().any(|&i| i as usize >= count) {
        return None;
    }
    let scale = if scale.is_finite() && scale.min_element() > 0.0 {
        scale
    } else {
        Vec3::ONE
    };
    let mut mikk = Mikk {
        positions: positions.iter().map(|p| Vec3::from_array(*p) * scale).collect(),
        normals: normals.iter().map(|n| (Vec3::from_array(*n) / scale).normalize_or_zero()).collect(),
        uvs,
        indices,
        corners: vec![([1.0, 0.0, 0.0], 1.0); indices.len() - indices.len() % 3],
    };
    if let Err(e) = bevy_mikktspace::generate_tangents(&mut mikk) {
        log::debug!("mesh tangents: {e}");
        return None;
    }
    Some(
        mikk.corners
            .into_iter()
            .map(|(t, hand)| ((Vec3::from_array(t) * scale).normalize_or(Vec3::X).to_array(), hand))
            .collect(),
    )
}

/// A face with one tangent per corner: its vertices split where corners of
/// one vertex pack different tangents, and the source vertex of each.
pub struct Welded {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    pub sources: Vec<u16>,
}

/// Splits `vertices` where the corners sharing one carry different
/// tangents (a mirrored UV island), as MikkTSpace's per-corner output is
/// welded again in cacheOptimize. `None` past the 16-bit index limit.
pub fn weld(vertices: &[Vertex], indices: &[u16], corners: &[Tangent]) -> Option<Welded> {
    let mut out = Welded {
        vertices: Vec::with_capacity(vertices.len()),
        indices: Vec::with_capacity(indices.len()),
        sources: Vec::with_capacity(vertices.len()),
    };
    // the first copy of each vertex with the tangent it was made for (the
    // corners of one smoothing group carry the very same one), the other
    // copies by packed tangent
    let mut first: Vec<Option<(Tangent, i8, u16)>> = vec![None; vertices.len()];
    let mut others = HashMap::<(u16, i8), u16>::new();
    for (&src, &corner) in indices.iter().zip(corners) {
        let base = vertices.get(src as usize)?;
        let slot = first[src as usize];
        let index = match slot {
            Some((t, _, index)) if t == corner => index,
            _ => {
                let code = pack_tangent(base.normal, corner.0, corner.1);
                match slot {
                    Some((_, c, index)) if c == code => index,
                    _ => match others.get(&(src, code)) {
                        Some(&index) => index,
                        None => {
                            let index = u16::try_from(out.vertices.len()).ok()?;
                            let mut v = *base;
                            v.normal[3] = code;
                            out.vertices.push(v);
                            out.sources.push(src);
                            if slot.is_none() {
                                first[src as usize] = Some((corner, code, index));
                            } else {
                                others.insert((src, code), index);
                            }
                            index
                        }
                    },
                }
            }
        };
        out.indices.push(index);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_render::unpack_tangent;

    /// A quad in the XY plane, its texture along +X / +Y; `mirror` flips
    /// the texture horizontally on its second triangle's own vertices.
    fn quad() -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u16>) {
        (
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0.0, 0.0, 1.0]; 4],
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    fn vertices(p: &[[f32; 3]], n: &[[f32; 3]], uv: &[[f32; 2]]) -> Vec<Vertex> {
        p.iter().zip(n).zip(uv).map(|((p, n), uv)| Vertex::new(*p, *n, *uv)).collect()
    }

    #[test]
    fn packed_tangents_come_back_within_a_step() {
        let normals = [
            Vec3::Z,
            -Vec3::Z,
            Vec3::X,
            Vec3::new(0.3, -0.5, 0.81),
            Vec3::new(-0.7, 0.1, -0.7),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        for n in normals {
            let v = Vertex::new([0.0; 3], n.normalize().to_array(), [0.0; 2]);
            let packed = normal_of(&v).normalize();
            let (b, _) = packed.any_orthonormal_pair();
            for k in 0..360 {
                let a = (k as f32).to_radians();
                let t = glam::Quat::from_axis_angle(packed, a) * b;
                for hand in [1.0, -1.0] {
                    let code = pack_tangent(v.normal, t.to_array(), hand);
                    assert_ne!(code, 0);
                    let mut w = v;
                    w.normal[3] = code;
                    let (back, h) = unpack_tangent(w.normal).expect("a tangent");
                    assert_eq!(h, hand);
                    assert!(back.dot(packed).abs() < 1e-5, "across the normal");
                    let off = back.dot(t).clamp(-1.0, 1.0).acos().to_degrees();
                    assert!(off <= 1.42, "{n} at {k}°: {off}° off");
                }
            }
        }
        assert_eq!(unpack_tangent([0, 0, 127, 0]), None);
    }

    #[test]
    fn volume_tangents_follow_s_and_keep_the_mirror() {
        let (p, n, mut uv, idx) = quad();
        let mut v = vertices(&p, &n, &uv);
        pack_volume_tangents(&mut v, &idx);
        for v in &v {
            let (t, hand) = unpack_tangent(v.normal).expect("a tangent");
            assert!(t.dot(Vec3::X) > 0.999, "{t}");
            assert_eq!(hand, 1.0);
        }
        // texture mirrored along t: same tangent, the other hand
        for uv in &mut uv {
            uv[1] = 1.0 - uv[1];
        }
        let mut v = vertices(&p, &n, &uv);
        pack_volume_tangents(&mut v, &idx);
        for v in &v {
            let (t, hand) = unpack_tangent(v.normal).expect("a tangent");
            assert!(t.dot(Vec3::X) > 0.999, "{t}");
            assert_eq!(hand, -1.0);
        }
    }

    #[test]
    fn mesh_tangents_match_lengyel_on_a_plane_and_split_a_mirror() {
        let (p, n, uv, idx) = quad();
        let corners = mesh_corner_tangents(&p, &n, &uv, &idx, Vec3::ONE).expect("tangents");
        let base = vertices(&p, &n, &uv);
        let welded = weld(&base, &idx, &corners).expect("welded");
        assert_eq!(welded.vertices.len(), 4, "no split on one island");
        for v in &welded.vertices {
            let (t, hand) = unpack_tangent(v.normal).expect("a tangent");
            assert!(t.dot(Vec3::X) > 0.999, "{t}");
            assert_eq!(hand, 1.0);
        }
        // two quads sharing an edge, the second's texture mirrored along s:
        // the shared vertices split, the handedness flips
        let p = vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [1.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
        ];
        let n = vec![[0.0, 0.0, 1.0]; 6];
        let uv = vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.0, 0.0], [0.0, 1.0]];
        let idx = vec![0, 1, 2, 0, 2, 3, 1, 4, 5, 1, 5, 2];
        let corners = mesh_corner_tangents(&p, &n, &uv, &idx, Vec3::ONE).expect("tangents");
        let base = vertices(&p, &n, &uv);
        let welded = weld(&base, &idx, &corners).expect("welded");
        assert_eq!(welded.vertices.len(), 8, "the shared edge splits");
        let hands: Vec<f32> = welded
            .indices
            .iter()
            .map(|&i| unpack_tangent(welded.vertices[i as usize].normal).expect("t").1)
            .collect();
        assert!(
            hands[..6].iter().all(|&h| h == 1.0) && hands[6..].iter().all(|&h| h == -1.0),
            "{hands:?}"
        );
    }

    #[test]
    fn mesh_tangents_are_found_in_the_upload_proportions() {
        // a plane stretched 4:1 by its normalization, its texture sheared:
        // dP/ds turns with the proportions, as cacheOptimize sees it
        let p = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0]];
        let n = vec![[0.0, 0.0, 1.0]; 4];
        let uv = vec![[0.0, 0.0], [1.0, 1.0], [1.0, 2.0], [0.0, 1.0]];
        let idx = vec![0, 1, 2, 0, 2, 3];
        let unit = mesh_corner_tangents(&p, &n, &uv, &idx, Vec3::ONE).expect("tangents");
        let wide = mesh_corner_tangents(&p, &n, &uv, &idx, Vec3::new(4.0, 1.0, 1.0)).expect("tangents");
        // s = x, t = x + y: dP/ds (t held) = (1, -1)
        let t = Vec3::from_array(unit[0].0);
        assert!(t.dot(Vec3::new(1.0, -1.0, 0.0).normalize()) > 0.999, "{t}");
        // in the upload proportions X = 4x: dP/ds = (4, -1), brought back
        // to the normalized frame times the scale: (16, -1)
        let t = Vec3::from_array(wide[0].0);
        assert!(t.dot(Vec3::new(16.0, -1.0, 0.0).normalize()) > 0.999, "{t}");
    }

    #[test]
    fn weld_refuses_past_sixteen_bits() {
        let n = 70_000usize;
        let base: Vec<Vertex> = (0..n.min(65_536))
            .map(|_| Vertex::new([0.0; 3], [0.0, 0.0, 1.0], [0.0; 2]))
            .collect();
        // every corner of vertex 0 with its own tangent would need 70000 vertices
        let indices = vec![0u16; n];
        let corners: Vec<Tangent> = (0..n)
            .map(|k| ([(k as f32).cos(), (k as f32).sin(), 0.0], if k % 2 == 0 { 1.0 } else { -1.0 }))
            .collect();
        let welded = weld(&base, &indices, &corners).expect("at most 254 codes per vertex");
        assert!(welded.vertices.len() <= 254);
        let many: Vec<Vertex> = vec![Vertex::new([0.0; 3], [0.0, 0.0, 1.0], [0.0; 2]); 65_536];
        let indices: Vec<u16> = (0..65_536u32).flat_map(|i| [i as u16, i as u16]).collect();
        let corners: Vec<Tangent> = (0..indices.len())
            .map(|k| ([1.0, 0.0, 0.0], if k % 2 == 0 { 1.0 } else { -1.0 }))
            .collect();
        assert!(weld(&many, &indices, &corners).is_none(), "131072 vertices do not fit");
    }
}
