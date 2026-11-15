//! Static file assemblies, centered on a support prim. Deliberately extends
//! LLLocalMeshFile::applyToVObject (indra/newview/vjlocalmesh.cpp): preserve
//! every instance's placement, with one uniform fit instead of stretching
//! one selected model. Uploads and rigged local previews keep their own paths.

use crate::imports::mesh;
use aurora_assets::{AssetError, import_mesh::Model};
use glam::Vec3;
use rayon::prelude::*;

pub fn supported(draft: &mesh::Draft) -> bool {
    let faces = draft.scene.instances.iter().try_fold(0usize, |count, instance| {
        let model = draft.scene.models.get(instance.model)?;
        model.skin.is_none().then_some(())?;
        let determinant = instance.transform.determinant();
        (instance.transform.is_finite() && determinant.is_finite() && determinant != 0.0).then_some(())?;
        count.checked_add(model.faces.len())
    });
    faces.is_some_and(|count| count > 0 && count <= aurora_prim::te::MAX_TES)
}

/// Bake once on the import worker. Stable face slots across LODs keep the
/// anchor's face appearances and picking attached to the same pieces.
pub fn assemble(draft: &mesh::Draft, prepared: &mesh::Prepared) -> Result<([Option<Model>; 4], Vec3), AssetError> {
    let mut lods: [Option<Model>; 4] = std::array::from_fn(|_| None);
    lods.par_iter_mut()
        .enumerate()
        .try_for_each(|(level, lod)| -> Result<(), AssetError> {
            let mut faces = Vec::new();
            for instance in &draft.scene.instances {
                let model = prepared
                    .lods
                    .get(instance.model)
                    .and_then(|lods| lods[level].as_ref())
                    .ok_or_else(|| AssetError::invalid("local scene LOD missing"))?;
                let transform = instance.transform;
                let determinant = transform.determinant();
                if !transform.is_finite() || !determinant.is_finite() || determinant == 0.0 {
                    return Err(AssetError::invalid("local scene transform"));
                }
                let normal = transform.inverse().transpose();
                if !normal.is_finite() {
                    return Err(AssetError::invalid("local scene inverse transform"));
                }
                for face in &model.faces {
                    if faces.len() >= aurora_prim::te::MAX_TES {
                        return Err(AssetError::TooLarge("local scene textureable faces"));
                    }
                    let mut face = face.clone();
                    for vertex in &mut face.vertices {
                        vertex.position = transform.transform_point3(Vec3::from_array(vertex.position)).to_array();
                        vertex.normal = normal
                            .transform_vector3(Vec3::from_array(vertex.normal))
                            .normalize_or_zero()
                            .to_array();
                    }
                    if determinant < 0.0 {
                        for triangle in face.indices.as_chunks_mut::<3>().0 {
                            triangle.swap(1, 2);
                        }
                    }
                    face.validate()?;
                    faces.push(face);
                }
            }
            *lod = Some(Model {
                name: "Local assembly".into(),
                submodel: 0,
                faces,
                skin: None,
                center: Vec3::ZERO,
                size: Vec3::ONE,
            });
            Ok(())
        })?;
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    // Include every LOD so a coarser file extending past High also fits.
    for model in lods.iter().flatten() {
        for face in &model.faces {
            for index in &face.indices {
                let point = Vec3::from_array(face.vertices[*index as usize].position);
                min = min.min(point);
                max = max.max(point);
            }
        }
    }
    let center = min * 0.5 + max * 0.5;
    let size = (max - min).max(Vec3::splat(1e-6));
    if !center.is_finite() || !size.is_finite() || min.x == f32::MAX {
        return Err(AssetError::invalid("local scene bounds"));
    }
    for model in lods.iter_mut().flatten() {
        model.center = center;
        model.size = size;
        for face in &mut model.faces {
            for vertex in &mut face.vertices {
                vertex.position = ((Vec3::from_array(vertex.position) - center) / size).to_array();
                vertex.normal = (Vec3::from_array(vertex.normal) * size).normalize_or_zero().to_array();
            }
        }
    }
    Ok((lods, size))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imports::{Entry, Imports, Source, demo};
    use aurora_assets::import_mesh::{collada, encode};
    use glam::{Mat4, Quat};
    use std::{collections::HashMap, sync::Arc};
    use uuid::Uuid;

    fn fixture() -> mesh::Draft {
        let scene = collada::parse(demo::multiple_dae().as_bytes(), &aurora_assets::Skeleton::default()).expect("instances");
        let mut draft = mesh::Draft::new(scene, HashMap::new());
        for lod in &mut draft.lods[..3] {
            lod.source = mesh::LodSource::Above;
        }
        draft
    }

    fn verify(draft: &mesh::Draft) {
        let prepared = mesh::prepare(draft).expect("prepare");
        let (lods, size) = assemble(draft, &prepared).expect("assemble");
        let bytes = encode::local_asset(&lods).expect("encode assembly");
        let header = aurora_assets::parse_mesh_header(&bytes).expect("header");
        for (level, model) in lods.iter().enumerate() {
            let model = model.as_ref().expect("all LODs");
            let section = aurora_assets::mesh::section_bytes(&bytes, header.lods[level].expect("LOD")).expect("section");
            let decoded = aurora_assets::decode_mesh_lod(section).expect("runtime decoder");
            assert_eq!(decoded.len(), model.faces.len());
            let mut slot = 0;
            for instance in &draft.scene.instances {
                let source = prepared.lods[instance.model][level].as_ref().expect("source LOD");
                for face in &source.faces {
                    let built = &model.faces[slot];
                    let encoded = &decoded[slot];
                    assert_eq!(built.vertices.len(), face.vertices.len());
                    assert_eq!(encoded.indices.len(), face.indices.len());
                    for ((original, built), decoded) in face.vertices.iter().zip(&built.vertices).zip(&encoded.positions) {
                        let expected = instance.transform.transform_point3(Vec3::from_array(original.position));
                        let actual = Vec3::from_array(built.position) * size + model.center;
                        assert!(actual.abs_diff_eq(expected, 1e-4), "instance placements survive normalization");
                        let actual = Vec3::from_array(*decoded) * size + model.center;
                        assert!(actual.abs_diff_eq(expected, size.max_element() * 4e-5), "placements survive codec");
                        assert_eq!(built.uv, original.uv);
                        let expected_normal = instance
                            .transform
                            .inverse()
                            .transpose()
                            .transform_vector3(Vec3::from_array(original.normal))
                            .normalize_or_zero();
                        let actual_normal = (Vec3::from_array(built.normal) / size).normalize_or_zero();
                        assert!(actual_normal.abs_diff_eq(expected_normal, 1e-4));
                    }
                    let mut indices = face.indices.clone();
                    if instance.transform.determinant() < 0.0 {
                        for triangle in indices.as_chunks_mut::<3>().0 {
                            triangle.swap(1, 2);
                        }
                    }
                    assert_eq!(built.indices, indices, "mirrors retain front-face winding");
                    slot += 1;
                }
            }
        }
    }

    #[test]
    fn local_assembly_keeps_repeated_instances_rotations_mirrors_and_all_lods() {
        let mut draft = fixture();
        let scene = Arc::make_mut(&mut draft.scene);
        scene.instances[0].transform =
            Mat4::from_scale_rotation_translation(Vec3::new(-2.0, 0.5, 3.0), Quat::from_rotation_z(0.6), Vec3::new(-3.0, 1.0, 2.0));
        verify(&draft);
        let mut entry = Entry::new("assembly.dae".into(), Ok(Source::Mesh(Box::new(draft))), Default::default());
        assert!(entry.local_scene, "new static files default to every piece");
        let entry_id = entry.id;
        let object = Uuid::new_v4();
        entry.local = true;
        let mut imports = Imports {
            entries: vec![entry],
            ..Default::default()
        };
        imports.target_local(entry_id, object, &[], false);
        assert!(imports.local_targets[&object].scene);
        imports.entries[0].local_scene = false;
        imports.target_local(entry_id, object, &[], true);
        assert!(!imports.local_targets[&object].scene);
        assert!(imports.local_targets[&object].use_scale);
    }

    #[test]
    fn local_assembly_keeps_flat_geometry_finite_and_rejects_invalid_transforms() {
        let mut draft = fixture();
        let scene = Arc::make_mut(&mut draft.scene);
        for vertex in &mut scene.models[0].faces[0].vertices {
            vertex.position[2] = 0.0;
        }
        verify(&draft);
        Arc::make_mut(&mut draft.scene).instances[0].transform = Mat4::ZERO;
        assert!(assemble(&draft, &mesh::prepare(&draft).expect("prepare")).is_err());
    }

    #[test]
    fn local_assembly_preserves_old_saved_targets_and_bounds_the_face_slots() {
        let old: crate::imports::local::Target = serde_json::from_value(serde_json::json!({
            "entry": Uuid::nil(), "model": 2, "use_scale": true
        }))
        .expect("old target");
        assert!(!old.scene && old.use_scale);
        let mut entry = Entry::new("reloaded.dae".into(), Ok(Source::Mesh(Box::new(fixture()))), Default::default());
        entry.local_mesh_ids = vec![Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        entry.local_scene_size = Some(Vec3::ONE);
        assert!(
            old.mesh(&entry).is_none(),
            "an obsolete model index cannot alias the appended assembly"
        );
        let mut draft = fixture();
        let scene = Arc::make_mut(&mut draft.scene);
        scene.instances = vec![scene.instances[0].clone(); aurora_prim::te::MAX_TES];
        assert!(supported(&draft));
        verify(&draft);
        let scene = Arc::make_mut(&mut draft.scene);
        scene.instances.push(scene.instances[0].clone());
        assert!(!supported(&draft));
        assert!(assemble(&draft, &mesh::prepare(&draft).expect("prepare")).is_err());
        assert!(!supported(&mesh::Draft::new(
            collada::parse(demo::tail_dae().as_bytes(), &aurora_assets::Skeleton::default()).expect("rigged"),
            HashMap::new()
        )));
    }

    #[test]
    #[ignore = "manual local-file diagnostic; set AURORA_DEMO_IMPORT_FILE"]
    fn local_assembly_external_dae_keeps_every_piece_through_the_runtime_codec() {
        let path = std::env::var_os("AURORA_DEMO_IMPORT_FILE").expect("diagnostic file");
        let bytes = std::fs::read(path).expect("read supplied plain DAE");
        let scene = collada::parse(&bytes, &aurora_assets::Skeleton::default()).expect("parse supplied DAE");
        let draft = mesh::Draft::new(scene, HashMap::new());
        assert!(supported(&draft));
        eprintln!(
            "Local assembly: {} pieces, {} source triangles",
            draft.scene.instances.len(),
            draft.scene.models.iter().map(Model::triangles).sum::<usize>()
        );
        verify(&draft);
    }
}
