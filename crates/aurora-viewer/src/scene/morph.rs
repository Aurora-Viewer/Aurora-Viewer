//! Morphs of the system avatar body: the visual parameters that move the
//! vertices of the base meshes (`*.llm`): sex, body fat, the sliders of the
//! face, the shape of the system hair, the eyelashes, the skirt.
//!
//! Port of LLPolyMorphTarget::apply and LLPolyMesh::initializeForMorph
//! (indra/llappearance/llpolymorph.cpp and llpolymesh.cpp, originally LGPL
//! 2.1). In `avatar_lad.xml` each `<mesh>` lists the parameters that drive a
//! morph of its file; a parameter's name is its morph's. Firestorm keeps one
//! copy of the meshes per avatar and adds weight differences to it as the
//! sliders move; here a body part is built once for a set of weights, off
//! the main thread, and avatars with the same weights share it.
//!
//! Not applied: the seven morphs that wait for a mask taken from a baked
//! texture (`<morph_masks>`: loose clothes, long cuffs, facial hair), which
//! Firestorm applies only once that mask is known.

use super::avatar::AvatarPart;
use super::jobs::FaceGeom;
use aurora_assets::LlmMesh;
use aurora_llsd::xml;
use aurora_render::Vertex;
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

/// NORMAL_SOFTEN_FACTOR (llpolymorph.cpp): a morph bends the normals less
/// than it moves the vertices.
const NORMAL_SOFTEN_FACTOR: f32 = 0.65;

/// One morph of a body part and the parameter that weighs it.
#[derive(Debug, Clone)]
pub struct MorphTarget {
    pub param: i32,
    pub(super) vertices: Vec<u32>,
    pub(super) d_positions: Vec<Vec3>,
    pub(super) d_normals: Vec<Vec3>,
    d_uvs: Vec<[f32; 2]>,
}

/// The morph parameters of avatar_lad.xml: (parameter, morph name) by mesh
/// file, without the morphs that wait for a mask.
pub fn mesh_params(lad: &[u8]) -> HashMap<String, Vec<(i32, String)>> {
    let mut out: HashMap<String, Vec<(i32, String)>> = HashMap::new();
    let Ok(root) = xml::parse(lad) else {
        return out;
    };
    // (wherever the parser puts the document's root)
    let (mut meshes, mut masked) = (Vec::new(), HashSet::new());
    let mut stack = vec![&root];
    while let Some(el) = stack.pop() {
        for c in el.elements() {
            match c.name.as_str() {
                "mesh" => meshes.push(c),
                "morph_masks" => masked.extend(c.elements().filter_map(|m| m.attr("morph_name"))),
                _ => stack.push(c),
            }
        }
    }
    for mesh in meshes {
        let Some(file) = mesh.attr("file_name") else {
            continue;
        };
        for p in mesh.elements().filter(|p| p.name == "param" && p.child("param_morph").is_some()) {
            let (Some(id), Some(name)) = (p.attr("id").and_then(|v| v.trim().parse::<i32>().ok()), p.attr("name")) else {
                continue;
            };
            if !masked.contains(name) {
                out.entry(file.to_owned()).or_default().push((id, name.to_owned()));
            }
        }
    }
    out
}

/// The morphs of `mesh` that a parameter drives. A parameter listed twice
/// for a mesh keeps its first morph (the id is the visual parameter).
pub fn targets(mesh: &LlmMesh, params: &[(i32, String)]) -> Vec<MorphTarget> {
    let mut seen = HashSet::new();
    params
        .iter()
        .filter_map(|(param, name)| {
            let m = mesh.morphs.iter().find(|m| &m.name == name)?;
            seen.insert(*param).then(|| MorphTarget {
                param: *param,
                vertices: m.vertex_indices.clone(),
                d_positions: m.d_positions.iter().map(|p| Vec3::from_array(*p)).collect(),
                d_normals: m.d_normals.iter().map(|n| Vec3::from_array(*n)).collect(),
                d_uvs: m.d_uvs.clone(),
            })
        })
        .collect()
}

/// The weights of a part's morphs for an avatar (`weights`: the effective
/// weight of every visual parameter), and what names their set: None when
/// no morph applies, the base mesh then.
pub fn part_weights(part: &AvatarPart, weights: &HashMap<i32, f32>) -> Option<(Vec<f32>, u64)> {
    let w: Vec<f32> = part
        .morphs
        .iter()
        .map(|m| weights.get(&m.param).copied().filter(|w| w.is_finite()).unwrap_or(0.0))
        .collect();
    if w.iter().all(|w| *w == 0.0) {
        return None;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for x in &w {
        x.to_bits().hash(&mut h);
    }
    Some((w, h.finish()))
}

/// The part with each of its morphs applied at its weight (`weights`
/// parallel to `part.morphs`), and its bounds.
pub fn morphed(part: &AvatarPart, weights: &[f32]) -> (FaceGeom, Vec3, Vec3) {
    let mut positions: Vec<Vec3> = part.vertices.iter().map(|v| Vec3::from_array(v.pos)).collect();
    // the "scaled normals" of LLPolyMesh: summed without being normalised
    let mut normals = part.normals.clone();
    let mut uvs: Vec<[f32; 2]> = part.vertices.iter().map(|v| v.uv).collect();
    for (m, &w) in part.morphs.iter().zip(weights) {
        if w == 0.0 || !w.is_finite() {
            continue;
        }
        for (i, &v) in m.vertices.iter().enumerate() {
            let v = v as usize;
            let (Some(p), Some(n), Some(uv)) = (positions.get_mut(v), normals.get_mut(v), uvs.get_mut(v)) else {
                continue;
            };
            *p += m.d_positions[i] * w;
            *n += m.d_normals[i] * (w * NORMAL_SOFTEN_FACTOR);
            uv[0] += m.d_uvs[i][0] * w;
            uv[1] += m.d_uvs[i][1] * w;
        }
    }
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    let vertices = positions
        .iter()
        .zip(&normals)
        .zip(&uvs)
        .map(|((p, n), uv)| {
            min = min.min(*p);
            max = max.max(*p);
            Vertex::new(p.to_array(), n.normalize_or(Vec3::Z).to_array(), *uv)
        })
        .collect();
    let skin = (part.skin.len() == part.vertices.len()).then(|| part.skin.clone());
    (
        FaceGeom {
            vertices,
            indices: part.indices.clone(),
            skin,
        },
        min,
        max,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::avatar::AvatarLibrary;

    fn part<'a>(lib: &'a AvatarLibrary, name: &str) -> &'a AvatarPart {
        lib.parts.iter().find(|p| p.name == name).expect("body part")
    }

    #[test]
    fn every_body_mesh_has_its_morphs_and_the_masked_ones_wait() {
        let lib = AvatarLibrary::load();
        for (name, at_least) in [
            ("head", 90),
            ("upper", 30),
            ("lower", 20),
            ("hair", 45),
            ("eyelashes", 30),
            ("skirt", 10),
        ] {
            let n = part(&lib, name).morphs.len();
            assert!(n >= at_least, "{name}: {n} morphs");
        }
        // the eyes follow their joints only
        assert!(part(&lib, "eye_left").morphs.is_empty());
        let lad = AvatarLibrary::file("avatar_lad.xml").expect("avatar_lad.xml");
        let params = mesh_params(lad);
        let names = |file: &str| params[file].iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>();
        assert!(names("avatar_upper_body.llm").contains(&"Male_Torso"));
        assert!(!names("avatar_upper_body.llm").contains(&"Shirtsleeve_flair"));
        assert!(!names("avatar_head.llm").contains(&"Displace_Hair_Facial"));
    }

    #[test]
    fn the_male_parameter_reshapes_the_torso_and_default_weights_too() {
        let lib = AvatarLibrary::load();
        let upper = part(&lib, "upper");
        let params = &lib.shape_params;
        let default = params.effective_weights(&[]);
        let male = params.effective_weights_for_sex(true);
        let (mw, mkey) = part_weights(upper, &male).expect("male weights");
        assert_ne!(part_weights(upper, &default).map(|(_, k)| k), Some(mkey));
        let dw: Vec<f32> = upper.morphs.iter().map(|m| default.get(&m.param).copied().unwrap_or(0.0)).collect();
        let (dgeom, ..) = morphed(upper, &dw);
        let (mgeom, mmin, mmax) = morphed(upper, &mw);
        assert_eq!(mgeom.vertices.len(), upper.vertices.len());
        assert_eq!(mgeom.indices, upper.indices);
        assert!(mmin.cmple(mmax).all());
        // the default shape is not the bare mesh either: morphs of the head
        // and of the hair have a default weight
        for name in ["head", "hair"] {
            assert!(part_weights(part(&lib, name), &default).is_some(), "{name}");
        }
        let moved = |a: &FaceGeom, b: &[Vertex]| {
            a.vertices
                .iter()
                .zip(b)
                .filter(|(x, y)| Vec3::from_array(x.pos).distance(Vec3::from_array(y.pos)) > 1e-3)
                .count()
        };
        assert!(moved(&mgeom, &dgeom.vertices) > 200, "the male torso differs from the female one");
        // no weight at all: the mesh as it is in its file
        assert!(part_weights(upper, &HashMap::new()).is_none());
        let (bare, ..) = morphed(upper, &vec![0.0; upper.morphs.len()]);
        assert_eq!(moved(&bare, &upper.vertices), 0);
        // the same weights name the same geometry
        assert_eq!(part_weights(upper, &male).map(|(_, k)| k), Some(mkey));
    }

    /// AURORA_DEMO_CLASSIC: two shapes as a simulator sends them, each with
    /// its own geometries.
    #[test]
    fn the_demo_shapes_are_read_back_from_their_appearance_values() {
        let lib = AvatarLibrary::load();
        let mut seen = HashSet::new();
        for ev in crate::demo::classic_events(&lib.shape_params) {
            let aurora_net::NetEvent::Appearance(a) = ev else {
                panic!("an appearance");
            };
            assert_eq!(a.visual_params.len(), lib.shape_params.transmitted_count());
            let weights = lib.shape_params.effective_weights(&a.visual_params);
            let male = lib.shape_params.shape(&a.visual_params, &lib.rig).male;
            assert_eq!(male, a.avatar_id == crate::demo::DEMO_AGENT);
            let keys: Vec<Option<u64>> = lib.parts.iter().map(|p| part_weights(p, &weights).map(|(_, k)| k)).collect();
            assert!(keys.iter().flatten().count() >= 4, "{keys:?}");
            assert!(seen.insert(keys), "each demo avatar has its own shape");
        }
        assert_eq!(seen.len(), 2);
    }

    #[test]
    fn a_morph_moves_its_vertices_by_its_weight_and_softens_the_normals() {
        let lib = AvatarLibrary::load();
        let head = part(&lib, "head");
        let (i, m) = head
            .morphs
            .iter()
            .enumerate()
            .find(|(_, m)| m.d_positions.iter().any(|d| d.length() > 1e-3))
            .expect("a morph that moves vertices");
        let mut w = vec![0.0; head.morphs.len()];
        w[i] = 0.5;
        let (geom, ..) = morphed(head, &w);
        let (k, d) = m
            .d_positions
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.length().total_cmp(&b.1.length()))
            .expect("deltas");
        let v = m.vertices[k] as usize;
        let expected = Vec3::from_array(head.vertices[v].pos) + *d * 0.5;
        assert!(Vec3::from_array(geom.vertices[v].pos).distance(expected) < 1e-5);
        let n = (head.normals[v] + m.d_normals[k] * (0.5 * NORMAL_SOFTEN_FACTOR)).normalize_or(Vec3::Z);
        let got = geom.vertices[v].normal;
        let got = Vec3::new(got[0] as f32, got[1] as f32, got[2] as f32) / 127.0;
        assert!(got.distance(n) < 0.02, "{got:?} != {n:?}");
    }
}
