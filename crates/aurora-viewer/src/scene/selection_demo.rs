//! Offline coverage fixture: a single mesh material face made of three
//! separate curved islands with holes. All LODs retain the same TE index.

use super::{GeomKey, Scene, jobs};
use aurora_render::{Renderer, Vertex};
use glam::{Mat4, Quat, Vec3};

impl Scene {
    /// Four different real geometries for native mesh-inspector captures.
    pub fn install_demo_mesh_inspector(&mut self, renderer: &mut Renderer) {
        let id = crate::demo::FACE_SELECTION_MESH;
        self.meshes.insert_demo_static(id);
        let mut params = crate::build::shapes::shapes()[10].params();
        params.path.scale = [0.45, 0.20];
        for (lod, detail) in [0.25, 0.5, 1.0, 2.0].into_iter().enumerate() {
            let volume = aurora_prim::generate_volume(&params, detail);
            let (faces, min, max) = jobs::volume_to_faces(&volume);
            self.upload_geom(
                renderer,
                GeomKey::Mesh { id, lod: lod as u8 },
                jobs::PreparedGeom::new(faces, min, max, true, None),
            );
        }
    }

    pub fn install_demo_selection_mesh(&mut self, renderer: &mut Renderer) {
        let id = crate::demo::FACE_SELECTION_MESH;
        self.meshes.insert_demo_static(id);
        let mut params = crate::build::shapes::shapes()[10].params();
        params.path.scale = [0.45, 0.20];
        let torus = aurora_prim::generate_volume(&params, 2.0);
        let mut face = jobs::FaceGeom {
            vertices: Vec::new(),
            indices: Vec::new(),
            skin: None,
        };
        for (position, angle) in [
            (Vec3::new(0.0, -0.25, 0.18), -0.3),
            (Vec3::new(0.0, 0.25, 0.18), 0.3),
            (Vec3::new(0.0, 0.0, -0.20), 0.0),
        ] {
            let transform = Mat4::from_scale_rotation_translation(Vec3::new(0.36, 0.48, 0.48), Quat::from_rotation_y(angle), position);
            let normals = transform.inverse().transpose();
            for source in &torus.faces {
                let base = face.vertices.len() as u16;
                for ((p, n), uv) in source.positions.iter().zip(&source.normals).zip(&source.uvs) {
                    let p = transform.transform_point3(Vec3::from_array(*p));
                    let n = normals.transform_vector3(Vec3::from_array(*n)).normalize_or_zero();
                    face.vertices.push(Vertex::new(p.to_array(), n.to_array(), *uv));
                }
                face.indices.extend(source.indices.iter().map(|i| base + *i));
            }
        }
        for lod in 0..4 {
            let faces = vec![Some(jobs::FaceGeom {
                vertices: face.vertices.clone(),
                indices: face.indices.clone(),
                skin: None,
            })];
            self.upload_geom(
                renderer,
                GeomKey::Mesh { id, lod },
                jobs::PreparedGeom::new(faces, Vec3::splat(-0.5), Vec3::splat(0.5), true, None),
            );
        }
    }
}
