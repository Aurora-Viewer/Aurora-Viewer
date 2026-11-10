//! Shaded Create thumbnails of the actual LLToolPlacer::addObject volumes.
//! The same parameters and placement rotation drive creation and these GPU
//! triangle meshes; an orthographic camera frames every complete solid.

use aurora_prim::VolumeParams;
use egui::{Color32, Mesh, Pos2, Rect, Vec2};
use glam::{Quat, Vec3};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

struct Triangle {
    points: [Vec3; 3],
    light: [u8; 3],
}

struct Preview {
    triangles: Vec<Triangle>,
    bounds: Rect,
    painted: RefCell<Option<(Rect, Color32, Arc<Mesh>)>>,
}

type Key = (u64, [u32; 4]);
thread_local! {
    static PREVIEWS: RefCell<HashMap<Key, Rc<Preview>>> = RefCell::new(HashMap::new());
}

// Finer than the world's highest LOD: even tiny silhouettes and broad
// highlights must stay smooth when the inspector is enlarged.
const DETAIL: f32 = 8.0;

/// White satin under a soft studio key, sky fill and broad reflection.
/// Spatial falloff also gives flat faces a gentle top-to-bottom gradient.
fn studio_light(point: Vec3, normal: Vec3) -> u8 {
    let key = (Vec3::new(-3.5, 6.0, 2.5) - point).normalize();
    let fill = Vec3::new(3.0, 4.0, 1.0).normalize();
    let half = (key + Vec3::Y).normalize();
    let diffuse = normal.dot(key).max(0.0);
    let reflection = normal.dot(half).max(0.0).powi(12);
    let sky = normal.z * 0.5 + 0.5;
    let height = (point.z + 0.5).clamp(0.0, 1.0);
    let light = 0.60 + 0.24 * diffuse + 0.035 * normal.dot(fill).max(0.0) + 0.035 * sky + 0.08 * height + 0.10 * reflection;
    (light.clamp(0.0, 1.0) * 255.0).round() as u8
}

impl Preview {
    fn new(params: &VolumeParams, rotation: Quat) -> Self {
        let volume = aurora_prim::generate_volume(params, DETAIL);
        // Tall default annular volumes need a higher eye to expose the hole;
        // keep their real proportions and placement rotation unchanged.
        let profile = params.profile.curve_type & aurora_prim::params::LL_PCODE_PROFILE_MASK;
        let circular = params.path.curve_type == aurora_prim::params::LL_PCODE_PATH_CIRCLE;
        let hemisphere = profile == aurora_prim::params::LL_PCODE_PROFILE_CIRCLE_HALF;
        let elevation = if circular && !hemisphere { 1.15 } else { 0.55 };
        // A triangular pyramid seen square-on looks like a flat triangle.
        let yaw = if !circular && profile == aurora_prim::params::LL_PCODE_PROFILE_EQUALTRI {
            -0.7
        } else {
            0.65
        };
        let mut camera = Quat::from_rotation_x(-elevation) * Quat::from_rotation_z(yaw);
        if hemisphere && params.path.end <= 0.5 {
            // Look from the cut side, with the flat cap above the curved body,
            // as in the Legacy Viewer's hemisphere thumbnail.
            camera *= Quat::from_rotation_x(std::f32::consts::PI);
        }
        Self::from_volume(&volume, camera * rotation)
    }

    fn from_volume(volume: &aurora_prim::VolumeMesh, view: Quat) -> Self {
        // In this frame +Y points toward the eye. Use the same camera for
        // projection, visibility, depth and lighting.
        let mut triangles = Vec::new();
        let mut bounds = Rect::NOTHING;
        for face in &volume.faces {
            let positions: Vec<Vec3> = face.positions.iter().map(|v| view * Vec3::from_array(*v)).collect();
            for point in &positions {
                bounds.extend_with(Pos2::new(point.x, -point.z));
            }
            for indices in face.indices.as_chunks::<3>().0 {
                let indices = indices.map(usize::from);
                let points = indices.map(|i| positions[i]);
                let normal = (points[1] - points[0]).cross(points[2] - points[0]);
                if normal.y <= 1e-8 {
                    continue;
                }
                let light = indices.map(|i| studio_light(positions[i], view * Vec3::from_array(face.normals[i])));
                triangles.push(Triangle { points, light });
            }
        }
        // Opaque triangles hide the back of the shape, including the inside of
        // torus holes. Draw far to near in the same orthographic camera frame.
        triangles.sort_by(|a, b| {
            let depth = |t: &Triangle| t.points.iter().map(|v| v.y).sum::<f32>();
            depth(a).total_cmp(&depth(b))
        });
        Self {
            triangles,
            bounds,
            painted: RefCell::new(None),
        }
    }

    fn paint(&self, ui: &egui::Ui, rect: Rect, color: Color32) {
        let mut painted = self.painted.borrow_mut();
        if painted
            .as_ref()
            .is_none_or(|(last_rect, last_colors, _)| *last_rect != rect || *last_colors != color)
        {
            *painted = Some((rect, color, Arc::new(self.mesh(rect, color))));
        }
        if let Some((_, _, mesh)) = painted.as_ref() {
            aurora_render::preview::paint_antialiased_mesh(
                ui,
                ui.scope_id().with(("primitive-thumbnail", self as *const Self as usize)),
                rect,
                mesh.clone(),
            );
        }
    }

    fn mesh(&self, rect: Rect, color: Color32) -> Mesh {
        let mut mesh = Mesh::default();
        if !self.bounds.is_positive() {
            return mesh;
        }
        let scale = (rect.width() / self.bounds.width()).min(rect.height() / self.bounds.height()) * 0.88;
        let center = self.bounds.center();
        // Shade an opaque material independently of the button background:
        // selecting a white solid must not tint its shadows violet.
        let shades: [Color32; 256] = std::array::from_fn(|i| {
            let shade = |channel: u8| (u16::from(channel) * i as u16 / 255) as u8;
            Color32::from_rgba_unmultiplied(shade(color.r()), shade(color.g()), shade(color.b()), color.a())
        });
        mesh.reserve_triangles(self.triangles.len());
        mesh.reserve_vertices(self.triangles.len() * 3);
        for triangle in &self.triangles {
            let base = mesh.vertices.len() as u32;
            for (point, light) in triangle.points.iter().zip(triangle.light) {
                let pos = rect.center() + (Vec2::new(point.x, -point.z) - center.to_vec2()) * scale;
                mesh.colored_vertex(pos, shades[light as usize]);
            }
            mesh.add_triangle(base, base + 1, base + 2);
        }
        mesh
    }
}

/// Generic tree and grass volumes, as the Legacy palette represents plants
/// before a species is chosen. These share the solids' camera and lighting.
fn plant(tree: bool) -> Preview {
    let shapes = crate::build::shapes::shapes();
    let mut volume = aurora_prim::VolumeMesh::default();
    let mut part = |shape: usize, scale: Vec3, center: Vec3, lean: f32| {
        let mut mesh = aurora_prim::generate_volume(&shapes[shape].params(), DETAIL);
        for face in &mut mesh.faces {
            for (position, normal) in face.positions.iter_mut().zip(&mut face.normals) {
                let mut v = Vec3::from_array(*position) * scale;
                let mut n = Vec3::from_array(*normal) / scale;
                let height = v.z + scale.z * 0.5;
                v.x += lean * height * height;
                n.z -= 2.0 * lean * height * n.x;
                *position = (v + center).to_array();
                *normal = n.normalize_or_zero().to_array();
            }
        }
        volume.faces.extend(mesh.faces);
    };
    if tree {
        part(4, Vec3::new(0.12, 0.12, 0.7), Vec3::new(0.0, 0.0, -0.15), 0.0);
        for (scale, center) in [
            (0.5, Vec3::new(-0.2, 0.0, 0.15)),
            (0.55, Vec3::new(0.16, 0.0, 0.22)),
            (0.48, Vec3::new(0.0, 0.0, 0.4)),
        ] {
            part(8, Vec3::splat(scale), center, 0.0);
        }
    } else {
        for i in 0..7 {
            let x = (i as f32 - 3.0) * 0.085;
            let height = 0.6 + 0.13 * (i % 3) as f32;
            part(
                2,
                Vec3::new(0.085, 0.035, height),
                Vec3::new(x, (i % 2) as f32 * 0.12, height * 0.5 - 0.45),
                (i as f32 - 3.0) * 0.16,
            );
        }
    }
    Preview::from_volume(&volume, Quat::from_rotation_x(-0.55) * Quat::from_rotation_z(0.65))
}

pub(super) fn paint_plant(ui: &egui::Ui, rect: Rect, tree: bool, color: Color32) {
    thread_local! {
        static PLANTS: [Preview; 2] = [plant(true), plant(false)];
    }
    PLANTS.with(|plants| plants[usize::from(!tree)].paint(ui, rect, color));
}

pub(super) fn paint(ui: &egui::Ui, rect: Rect, params: &VolumeParams, rotation: Quat, color: Color32) {
    let key = (params.cache_key(), rotation.to_array().map(f32::to_bits));
    let preview = PREVIEWS.with(|cache| {
        cache
            .borrow_mut()
            .entry(key)
            .or_insert_with(|| Rc::new(Preview::new(params, rotation)))
            .clone()
    });
    preview.paint(ui, rect, color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_creation_volume_is_visible_and_fits_both_palette_sizes() {
        for shape in crate::build::shapes::shapes() {
            let preview = Preview::new(&shape.params(), shape.rotation());
            assert!(!preview.triangles.is_empty(), "{}", shape.name);
            for size in [22.0, 48.0] {
                let rect = Rect::from_min_size(Pos2::new(7.0, 11.0), Vec2::splat(size));
                let p = crate::theme::Theme::default().palette();
                let mesh = preview.mesh(rect, p.ink);
                assert!(mesh.is_valid());
                assert!(
                    mesh.vertices.iter().all(|v| rect.shrink(size * 0.05).contains(v.pos)),
                    "{}",
                    shape.name
                );
                assert!(mesh.vertices.iter().all(|v| v.color.a() == 255));
            }
        }
    }

    #[test]
    fn tetrahedron_shows_more_than_one_planar_face() {
        let shape = &crate::build::shapes::shapes()[3];
        let preview = Preview::new(&shape.params(), shape.rotation());
        let normal = |t: &Triangle| (t.points[1] - t.points[0]).cross(t.points[2] - t.points[0]).normalize();
        let first = normal(&preview.triangles[0]);
        assert!(preview.triangles.iter().any(|t| normal(t).dot(first) < 0.9));
    }

    #[test]
    fn sphere_is_a_solid_and_each_annular_volume_keeps_its_hole() {
        let shapes = crate::build::shapes::shapes();
        let covers_center = |index: usize| {
            let shape = &shapes[index];
            let preview = Preview::new(&shape.params(), shape.rotation());
            preview.triangles.iter().any(|t| {
                let p = t.points.map(|v| Vec2::new(v.x, -v.z));
                let sides = std::array::from_fn::<_, 3, _>(|i| (p[(i + 1) % 3] - p[i]).rot90().dot(-p[i]));
                sides.iter().all(|s| *s >= -1e-7) || sides.iter().all(|s| *s <= 1e-7)
            })
        };
        assert!(covers_center(8));
        for (index, shape) in shapes.iter().enumerate().skip(10) {
            assert!(!covers_center(index), "{}", shape.name);
        }
    }
}
