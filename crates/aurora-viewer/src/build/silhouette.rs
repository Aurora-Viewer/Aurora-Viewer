//! Selection outlines of prims, like LLVolume::generateSilhouetteVertices:
//! per face, the edges without a neighbor triangle, plus the edges between
//! a triangle facing the camera and one facing away (the outline seen from
//! here). Mesh and sculpted objects are drawn as wireframes by the renderer,
//! like LL does for meshes.

use super::draw::{Painter3d, rgba};
use crate::scene::Scene;
use crate::world::World;
use egui::Color32;
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// One edge in unit object space.
#[derive(Debug, Clone, Copy)]
struct Edge {
    a: Vec3,
    b: Vec3,
    /// Border edge: normal of its triangle (inner edges: zero).
    n: Vec3,
    /// Its two triangles (the same twice for a border edge).
    tris: (u32, u32),
}

/// Edges of a prim shape, all faces together.
#[derive(Default)]
pub(crate) struct Topology {
    /// Triangle (first vertex, face normal) for the facing test.
    tris: Vec<(Vec3, Vec3)>,
    border: Vec<Edge>,
    inner: Vec<Edge>,
}

#[derive(Default)]
pub struct Cache {
    shapes: HashMap<u64, Arc<Topology>>,
}

/// SilhouetteParentColor (yellow) / SilhouetteChildColor (SL-MidBlue).
pub fn color(root: bool) -> Color32 {
    if root { rgba(1.0, 1.0, 0.0, 0.9) } else { rgba(0.3, 0.6, 0.9, 0.9) }
}

fn topology(params: &aurora_prim::VolumeParams) -> Topology {
    let mesh = aurora_prim::generate_volume(params, aurora_prim::volume::DETAIL_SCALES[2]);
    from_mesh(&mesh)
}

/// LLVOVolume::generateSilhouette uses the current simulated flexible
/// volume. Called on rayon together with that volume's vertex update.
pub(crate) fn from_mesh(mesh: &aurora_prim::VolumeMesh) -> Topology {
    let mut out = Topology::default();
    for face in &mesh.faces {
        // weld by position so texture seams are not borders
        let mut ids: HashMap<[i32; 3], u32> = HashMap::new();
        let weld: Vec<u32> = face
            .positions
            .iter()
            .map(|p| {
                let k = [
                    (p[0] * 1e4).round() as i32,
                    (p[1] * 1e4).round() as i32,
                    (p[2] * 1e4).round() as i32,
                ];
                let n = ids.len() as u32;
                *ids.entry(k).or_insert(n)
            })
            .collect();
        // edge -> (uses, vertex a, vertex b, first triangle, second triangle)
        let mut edges: HashMap<(u32, u32), (u32, usize, usize, u32, u32)> = HashMap::new();
        for tri in face.indices.as_chunks::<3>().0 {
            let v = [tri[0] as usize, tri[1] as usize, tri[2] as usize];
            if v.iter().any(|&i| i >= face.positions.len()) {
                continue;
            }
            let p = v.map(|i| Vec3::from_array(face.positions[i]));
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            if n.length_squared() < 1e-14 {
                continue; // degenerate
            }
            let t = out.tris.len() as u32;
            out.tris.push((p[0], n));
            for k in 0..3 {
                let (i0, i1) = (v[k], v[(k + 1) % 3]);
                let (w0, w1) = (weld[i0], weld[i1]);
                if w0 == w1 {
                    continue;
                }
                let e = edges.entry((w0.min(w1), w0.max(w1))).or_insert((0, i0, i1, t, t));
                if e.0 == 1 {
                    e.4 = t;
                }
                e.0 += 1;
            }
        }
        for (_, (c, i0, i1, t0, t1)) in edges {
            let (a, b) = (Vec3::from_array(face.positions[i0]), Vec3::from_array(face.positions[i1]));
            match c {
                1 => out.border.push(Edge {
                    a,
                    b,
                    n: out.tris[t0 as usize].1.normalize(),
                    tris: (t0, t0),
                }),
                2 => out.inner.push(Edge {
                    a,
                    b,
                    n: Vec3::ZERO,
                    tris: (t0, t1),
                }),
                _ => {}
            }
        }
    }
    out
}

impl Cache {
    fn topology_for(&mut self, params: &aurora_prim::VolumeParams) -> Arc<Topology> {
        let key = params.cache_key();
        if self.shapes.len() > 256 {
            self.shapes.clear();
        }
        self.shapes.entry(key).or_insert_with(|| Arc::new(topology(params))).clone()
    }

    /// Outline the selected objects ((object index, root color)).
    pub fn draw(&mut self, p: &Painter3d, world: &World, scene: &Scene, prims: &[(usize, bool)]) {
        let now = Instant::now();
        let eye = p.cam.eye;
        for &(idx, root) in prims.iter().take(512) {
            let (Some(o), Some((pos, rot, hud))) = (world.objects.get(idx), Scene::object_transform(world, idx, now, 0)) else {
                continue;
            };
            if hud != p.cam.hud.is_some() || (!hud && o.volume.sculpt.is_some()) {
                continue; // sculpts and meshes: renderer wireframe (Scene::selection_wire)
            }
            let c = color(root);
            let scale = world.visual_scale(idx).max(Vec3::splat(0.001));
            if let Some((center, r, half)) = o.is_tree().then(|| scene.plant_box(world, idx, pos, rot)).flatten() {
                obb(p, center, r, half, c);
                continue;
            }
            if o.is_tree() || (hud && o.volume.sculpt.is_some()) {
                obb(p, pos, rot, scale * 0.5, c);
                continue;
            }
            let cached;
            let topo = if o.is_flexible() {
                let Some(topo) = scene.flexible_outline(idx) else {
                    continue;
                };
                topo
            } else {
                cached = self.topology_for(&o.volume);
                &cached
            };
            let world_pt = |v: Vec3| pos + rot * (v * scale);
            // the camera in the prim's unit space: facing signs are kept by the
            // (orientation preserving) object transform
            let cam = (rot.inverse() * (eye - pos)) / scale;
            let hud_dir = (rot.inverse() * -p.cam.at) / scale;
            let facing: Vec<bool> = topo
                .tris
                .iter()
                .map(|(v, n)| n.dot(if hud { hud_dir } else { cam - *v }) > 0.0)
                .collect();
            let inv_scale = Vec3::ONE / scale;
            for e in &topo.border {
                let (a, b) = (world_pt(e.a), world_pt(e.b));
                let n = rot * (e.n * inv_scale);
                if n.dot(if hud { -p.cam.at } else { eye - (a + b) * 0.5 }) < 0.0 {
                    continue; // back side: hidden (RenderHiddenSelections off)
                }
                p.line(a, b, c, 2.0);
            }
            for e in &topo.inner {
                if facing[e.tris.0 as usize] != facing[e.tris.1 as usize] {
                    p.line(world_pt(e.a), world_pt(e.b), c, 2.0);
                }
            }
        }
    }
}

fn obb(p: &Painter3d, pos: Vec3, rot: Quat, half: Vec3, c: Color32) {
    let corner = |x: f32, y: f32, z: f32| pos + rot * (half * Vec3::new(x, y, z));
    for a in [-1.0f32, 1.0] {
        for b in [-1.0f32, 1.0] {
            p.line(corner(-1.0, a, b), corner(1.0, a, b), c, 1.5);
            p.line(corner(a, -1.0, b), corner(a, 1.0, b), c, 1.5);
            p.line(corner(a, b, -1.0), corner(a, b, 1.0), c, 1.5);
        }
    }
}

thread_local! {
    static PREVIEWS: std::cell::RefCell<HashMap<u64, Arc<Vec<(Vec3, Vec3)>>>> = std::cell::RefCell::new(HashMap::new());
}

/// Outline segments of a shape for the Create palette icons.
pub fn preview_edges(params: &aurora_prim::VolumeParams) -> Arc<Vec<(Vec3, Vec3)>> {
    let key = params.cache_key();
    PREVIEWS.with(|m| {
        m.borrow_mut()
            .entry(key)
            .or_insert_with(|| {
                let t = topology(params);
                // borders, plus the outline seen from the icon's viewpoint
                let view = Vec3::new(0.6, -0.9, 0.7) * 10.0;
                let facing: Vec<bool> = t.tris.iter().map(|(v, n)| n.dot(view - *v) > 0.0).collect();
                let mut out: Vec<(Vec3, Vec3)> = t.border.iter().map(|e| (e.a, e.b)).collect();
                out.extend(
                    t.inner
                        .iter()
                        .filter(|e| facing[e.tris.0 as usize] != facing[e.tris.1 as usize])
                        .map(|e| (e.a, e.b)),
                );
                Arc::new(out)
            })
            .clone()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_outline_is_its_face_borders() {
        let params = super::super::shapes::shapes()[0].params();
        let t = topology(&params);
        // 6 faces with 4 unit border sides each (split in segments)
        let total: f32 = t.border.iter().map(|e| e.a.distance(e.b)).sum();
        assert!((total - 24.0).abs() < 1e-3, "{total}");
    }

    #[test]
    fn closed_torus_has_a_view_outline() {
        let params = super::super::shapes::shapes()[10].params();
        let t = topology(&params);
        let cam = Vec3::new(0.0, -5.0, 2.0);
        let facing: Vec<bool> = t.tris.iter().map(|(v, n)| n.dot(cam - *v) > 0.0).collect();
        let outline = t
            .inner
            .iter()
            .filter(|e| facing[e.tris.0 as usize] != facing[e.tris.1 as usize])
            .count();
        assert!(outline > 20, "{outline}");
    }

    #[test]
    fn flexible_outline_uses_current_bent_vertices_instead_of_the_rest_shape() {
        use aurora_prim::flexible::Flexible;
        let mut params = aurora_prim::VolumeParams::default();
        params.path.curve_type = aurora_prim::params::LL_PCODE_PATH_FLEXIBLE;
        let rot = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let scale = Vec3::new(0.3, 0.5, 4.0);
        let mut sim = Flexible::new(2, scale.z, Vec3::ZERO, rot);
        let flex = super::super::edits::FLEX_DEFAULT;
        for _ in 0..240 {
            sim.update(flex, Vec3::ZERO, rot, 1.0 / 60.0, |_| Vec3::ZERO);
        }
        let mesh = aurora_prim::volume::generate_flexible_volume(&params, 3.0, &sim.path(Vec3::ZERO, rot), scale);
        let bent = from_mesh(&mesh);
        let lowest = |t: &Topology| {
            t.border
                .iter()
                .flat_map(|e| [e.a, e.b])
                .map(|v| (rot * (v * scale)).z)
                .fold(f32::INFINITY, f32::min)
        };
        assert!(lowest(&bent) < lowest(&topology(&params)) - 0.5);
        for e in bent.border.iter().chain(&bent.inner) {
            for v in [e.a, e.b] {
                assert!(mesh.faces.iter().any(|f| f.positions.iter().any(|p| Vec3::from_array(*p) == v)));
            }
            assert!(e.n.is_finite());
            assert!((e.tris.0 as usize) < bent.tris.len() && (e.tris.1 as usize) < bent.tris.len());
        }
    }
}
