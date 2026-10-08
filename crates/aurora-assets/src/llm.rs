//! Avatar base meshes ("Linden Binary Mesh 1.0", `character/*.llm`).
//!
//! Ported from the Second Life / Firestorm viewer sources (originally LGPL 2.1):
//! - `indra/llappearance/llpolymesh.cpp` (`LLPolyMeshSharedData::loadMesh`)
//! - `indra/llappearance/llpolymorph.cpp` (`LLPolyMorphData::loadBinary`)
//! - `indra/llappearance/llpolyskeletaldistortion.cpp` (`clone_morph_param_*`)
//!
//! Copyright (C) 2001-2024, Linden Research, Inc. and the Firestorm project.
//!
//! File layout (little endian):
//! ```text
//! char[24]  "Linden Binary Mesh 1.0"
//! u8 has_weights, u8 has_detail_texcoords
//! f32[3] position, f32[3] rotation (euler degrees), u8 rotation_order, f32[3] scale
//! -- base meshes only --
//! u16 num_vertices; f32[3]*n coords; f32[3]*n normals; f32[3]*n binormals;
//! f32[2]*n texcoords; [f32[2]*n detail texcoords]; [f32*n weights]
//! -- all meshes --
//! u16 num_faces; u16[3]*n faces
//! -- base meshes only (LOD files also carry an empty trailer) --
//! [u16 num_skin_joints; char[64]*n joint names]
//! { char[64] morph name; i32 n; n * (u32 index, f32[3] d_coord, f32[3] d_normal,
//!   f32[3] d_binormal, f32[2] d_uv) }* char[64] "End Morphs"
//! i32 num_remaps; (i32 src, i32 dst) * n
//! ```
//!
//! LOD meshes (`*_1.llm` .. `*_5.llm`) contain only the face list; the LOD flag
//! comes from `avatar_lad.xml` in LL, here it is detected from the file layout.

use glam::Vec3;

use crate::{AssetError, ByteReader};

const HEADER_BINARY: &[u8] = b"Linden Binary Mesh 1.0";
const HEADER_FIELD_LEN: usize = 24;
const NAME_LEN: usize = 64;
const END_MORPHS: &[u8] = b"End Morphs";
/// LL rejects morph vertex indices above this ("Bad morph index").
const MAX_MORPH_VERTEX_INDEX: u32 = 10000;

/// A morph target: per-vertex deltas for a subset of vertices.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LlmMorph {
    pub name: String,
    pub vertex_indices: Vec<u32>,
    pub d_positions: Vec<[f32; 3]>,
    pub d_normals: Vec<[f32; 3]>,
    pub d_binormals: Vec<[f32; 3]>,
    pub d_uvs: Vec<[f32; 2]>,
}

/// A parsed `.llm` mesh.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LlmMesh {
    pub has_weights: bool,
    pub has_detail_texcoords: bool,
    pub position: Vec3,
    /// Euler angles in degrees, applied in LL `mayaQ` XYZ order.
    pub rotation_euler: Vec3,
    pub scale: Vec3,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub binormals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub detail_uvs: Vec<[f32; 2]>,
    /// LL packed skin weights, one per vertex: the integer part indexes the
    /// mesh's joint render list and the fractional part blends towards the next
    /// entry. The render list is built by `LLAvatarJointMesh::setupJoint`: a
    /// depth-first walk of the skeleton that, for each joint in `joint_names`,
    /// appends the joint, preceded by its (base skeleton) parent unless that
    /// parent is already the last entry.
    pub weights: Vec<f32>,
    pub faces: Vec<[u16; 3]>,
    pub joint_names: Vec<String>,
    /// Morph targets in file order; the physics "driven" morphs LL synthesizes in
    /// `loadMesh` are appended right after their source morph.
    pub morphs: Vec<LlmMorph>,
    /// True for face-only LOD meshes, which share vertex data with the base mesh.
    pub is_lod: bool,
    /// Vertex remap table (`mSharedVerts`): `(source, destination)` pairs used to
    /// weld seam vertices when applying morphs.
    pub vertex_remaps: Vec<(i32, i32)>,
}

fn c_name(raw: &[u8]) -> String {
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    String::from_utf8_lossy(&raw[..end]).into_owned()
}

/// LOD files hold only `num_faces` + faces, followed either by EOF or by the
/// empty trailer LL's exporter writes (`u16 0`, "End Morphs", `i32 0`).
fn looks_like_lod(data: &[u8], at: usize) -> bool {
    let Some(b) = data.get(at..at + 2) else {
        return false;
    };
    let n = u16::from_le_bytes([b[0], b[1]]) as usize;
    let after = at + 2 + n * 6;
    if after == data.len() {
        return true;
    }
    data.get(after..after + 2) == Some(&[0, 0])
        && data
            .get(after + 2..after + 2 + NAME_LEN)
            .is_some_and(|name| name.starts_with(END_MORPHS) && name.get(END_MORPHS.len()) == Some(&0))
}

fn read_vec3s(r: &mut ByteReader<'_>, n: usize, what: &'static str) -> Result<Vec<[f32; 3]>, AssetError> {
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(r.vec3(what)?);
    }
    Ok(v)
}

fn read_vec2s(r: &mut ByteReader<'_>, n: usize, what: &'static str) -> Result<Vec<[f32; 2]>, AssetError> {
    let mut v = Vec::with_capacity(n);
    for _ in 0..n {
        v.push(r.vec2(what)?);
    }
    Ok(v)
}

/// Port of `LLPolyMorphData::loadBinary`.
fn read_morph(r: &mut ByteReader<'_>, name: String) -> Result<LlmMorph, AssetError> {
    const BYTES_PER_VERTEX: usize = 4 + 12 * 3 + 8;
    let n = r.i32("morph vertex count")?;
    if n < 0 {
        return Err(AssetError::invalid(format!("morph {name}: negative vertex count")));
    }
    let n = n as usize;
    if n.saturating_mul(BYTES_PER_VERTEX) > r.remaining() {
        return Err(AssetError::Truncated("morph vertices"));
    }
    let mut m = LlmMorph {
        name,
        vertex_indices: Vec::with_capacity(n),
        d_positions: Vec::with_capacity(n),
        d_normals: Vec::with_capacity(n),
        d_binormals: Vec::with_capacity(n),
        d_uvs: Vec::with_capacity(n),
    };
    for _ in 0..n {
        let idx = r.u32("morph vertex index")?;
        if idx > MAX_MORPH_VERTEX_INDEX {
            return Err(AssetError::invalid(format!("morph {}: bad vertex index {idx}", m.name)));
        }
        m.vertex_indices.push(idx);
        m.d_positions.push(r.vec3("morph coords")?);
        m.d_normals.push(r.vec3("morph normal")?);
        m.d_binormals.push(r.vec3("morph binormal")?);
        m.d_uvs.push(r.vec2("morph uv")?);
    }
    Ok(m)
}

fn clone_duplicate(src: &LlmMorph, name: &str) -> LlmMorph {
    LlmMorph {
        name: name.to_owned(),
        ..src.clone()
    }
}

fn clone_direction(src: &LlmMorph, dir: [f32; 3], name: &str) -> LlmMorph {
    let n = src.vertex_indices.len();
    LlmMorph {
        name: name.to_owned(),
        vertex_indices: src.vertex_indices.clone(),
        d_positions: vec![dir; n],
        d_normals: vec![[0.0; 3]; n],
        d_binormals: vec![[0.0; 3]; n],
        d_uvs: src.d_uvs.clone(),
    }
}

fn clone_cleavage(src: &LlmMorph, scale: f32, name: &str) -> LlmMorph {
    let mut m = clone_duplicate(src, name);
    let mul = |v: [f32; 3], s: [f32; 3]| [v[0] * s[0], v[1] * s[1], v[2] * s[2]];
    for v in 0..m.vertex_indices.len() {
        let s = if m.d_positions[v][1] < 0.0 {
            [scale, -scale, scale]
        } else {
            [scale; 3]
        };
        m.d_positions[v] = mul(src.d_positions[v], s);
        m.d_normals[v] = mul(src.d_normals[v], s);
        m.d_binormals[v] = mul(src.d_binormals[v], s);
    }
    m
}

/// Physics morphs synthesized by `LLPolyMeshSharedData::loadMesh`.
fn push_with_clones(morphs: &mut Vec<LlmMorph>, m: LlmMorph) {
    let mut extra = Vec::new();
    match m.name.as_str() {
        "Breast_Female_Cleavage" => {
            extra.push(clone_cleavage(&m, 0.75, "Breast_Physics_LeftRight_Driven"));
            extra.push(clone_duplicate(&m, "Breast_Physics_InOut_Driven"));
        }
        "Breast_Gravity" => extra.push(clone_duplicate(&m, "Breast_Physics_UpDown_Driven")),
        "Big_Belly_Torso" => extra.push(clone_direction(&m, [0.0, 0.0, 0.05], "Belly_Physics_Torso_UpDown_Driven")),
        "Big_Belly_Legs" => extra.push(clone_direction(&m, [0.0, 0.0, 0.05], "Belly_Physics_Legs_UpDown_Driven")),
        "skirt_belly" => extra.push(clone_direction(&m, [0.0, 0.0, 0.05], "Belly_Physics_Skirt_UpDown_Driven")),
        "Small_Butt" => {
            extra.push(clone_direction(&m, [0.0, 0.0, 0.05], "Butt_Physics_UpDown_Driven"));
            extra.push(clone_direction(&m, [0.0, 0.03, 0.0], "Butt_Physics_LeftRight_Driven"));
        }
        _ => {}
    }
    morphs.push(m);
    morphs.extend(extra);
}

/// Parse a "Linden Binary Mesh 1.0" file (port of `LLPolyMeshSharedData::loadMesh`).
pub fn parse_llm(data: &[u8]) -> Result<LlmMesh, AssetError> {
    if !data.starts_with(HEADER_BINARY) {
        return Err(AssetError::invalid("not a Linden Binary Mesh 1.0 file"));
    }
    let mut r = ByteReader::new(data);
    r.seek(HEADER_FIELD_LEN)?;
    let mut mesh = LlmMesh {
        has_weights: r.u8("has_weights")? > 0,
        has_detail_texcoords: r.u8("has_detail_texcoords")? > 0,
        position: Vec3::from(r.vec3("position")?),
        rotation_euler: Vec3::from(r.vec3("rotation")?),
        ..Default::default()
    };
    // Rotation order is read but LL forces XYZ.
    let _rotation_order = r.u8("rotation order")?;
    mesh.scale = Vec3::from(r.vec3("scale")?);

    mesh.is_lod = looks_like_lod(data, r.pos());

    let mut num_vertices = 0usize;
    if !mesh.is_lod {
        num_vertices = r.u16("num_vertices")? as usize;
        let per_vertex = 12 * 3 + 8 + if mesh.has_detail_texcoords { 8 } else { 0 } + if mesh.has_weights { 4 } else { 0 };
        if num_vertices * per_vertex > r.remaining() {
            return Err(AssetError::Truncated("llm vertex data"));
        }
        mesh.positions = read_vec3s(&mut r, num_vertices, "coords")?;
        mesh.normals = read_vec3s(&mut r, num_vertices, "normals")?;
        mesh.binormals = read_vec3s(&mut r, num_vertices, "binormals")?;
        mesh.uvs = read_vec2s(&mut r, num_vertices, "texcoords")?;
        if mesh.has_detail_texcoords {
            mesh.detail_uvs = read_vec2s(&mut r, num_vertices, "detail texcoords")?;
        }
        if mesh.has_weights {
            mesh.weights = (0..num_vertices).map(|_| r.f32("weights")).collect::<Result<_, _>>()?;
        }
    }

    let num_faces = r.u16("num_faces")? as usize;
    if num_faces * 6 > r.remaining() {
        return Err(AssetError::Truncated("llm faces"));
    }
    mesh.faces.reserve(num_faces);
    for _ in 0..num_faces {
        let f = [r.u16("face")?, r.u16("face")?, r.u16("face")?];
        if !mesh.is_lod && f.iter().any(|&i| i as usize >= num_vertices) {
            return Err(AssetError::invalid("llm face index out of range"));
        }
        mesh.faces.push(f);
    }

    if mesh.is_lod {
        return Ok(mesh);
    }

    if mesh.has_weights {
        let n = r.u16("num_skin_joints")? as usize;
        if n * NAME_LEN > r.remaining() {
            return Err(AssetError::Truncated("llm joint names"));
        }
        for _ in 0..n {
            mesh.joint_names.push(c_name(r.take(NAME_LEN, "joint name")?));
        }
    }

    // Morph section: absent in some files, terminated by "End Morphs".
    while r.remaining() >= NAME_LEN {
        let raw = r.take(NAME_LEN, "morph name")?;
        let name = c_name(raw);
        if name.as_bytes() == END_MORPHS {
            break;
        }
        match read_morph(&mut r, name) {
            Ok(m) => push_with_clones(&mut mesh.morphs, m),
            Err(e) => {
                // LL keeps going from a misaligned position; we stop instead.
                log::warn!("llm: failed to load morph: {e}");
                return Ok(mesh);
            }
        }
    }

    if r.remaining() >= 4 {
        let n = r.i32("num_remaps")?.max(0) as usize;
        for _ in 0..n {
            let (Ok(src), Ok(dst)) = (r.i32("remap src"), r.i32("remap dst")) else {
                log::warn!("llm: truncated vertex remap data");
                break;
            };
            mesh.vertex_remaps.push((src, dst));
        }
    }

    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const CHARACTER_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../aurora-viewer/assets/character");

    #[test]
    fn parse_all_character_meshes() {
        let dir = Path::new(CHARACTER_DIR);
        let Ok(entries) = std::fs::read_dir(dir) else {
            eprintln!("skipping: {CHARACTER_DIR} not found");
            return;
        };
        let mut count = 0;
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) != Some("llm") {
                continue;
            }
            let data = std::fs::read(&path).unwrap();
            let mesh = parse_llm(&data).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            // avatar_lad.xml declares `reference=` LODs for every numbered mesh except
            // the eyes, whose LODs are complete meshes.
            let expect_lod = stem.rsplit('_').next().is_some_and(|s| s.parse::<u32>().is_ok()) && !stem.starts_with("avatar_eye_");
            assert_eq!(mesh.is_lod, expect_lod, "{stem}");
            assert!(!mesh.faces.is_empty(), "{stem}");
            if !mesh.is_lod {
                let n = mesh.positions.len();
                assert!(n > 0, "{stem}");
                assert_eq!(mesh.normals.len(), n);
                assert_eq!(mesh.binormals.len(), n);
                assert_eq!(mesh.uvs.len(), n);
                assert_eq!(mesh.weights.len(), if mesh.has_weights { n } else { 0 });
                assert!(mesh.faces.iter().flatten().all(|&i| (i as usize) < n));
                if mesh.has_weights {
                    assert!(!mesh.joint_names.is_empty(), "{stem}");
                    assert!(mesh.weights.iter().all(|w| w.is_finite() && *w >= 0.0), "{stem}");
                }
                for m in &mesh.morphs {
                    assert!(m.vertex_indices.iter().all(|&i| (i as usize) < n), "{stem}/{}", m.name);
                }
            }
            eprintln!(
                "{stem}: lod={} verts={} faces={} joints={:?} morphs={} remaps={} detail={}",
                mesh.is_lod,
                mesh.positions.len(),
                mesh.faces.len(),
                mesh.joint_names,
                mesh.morphs.len(),
                mesh.vertex_remaps.len(),
                mesh.has_detail_texcoords
            );
            if stem == "avatar_upper_body" {
                assert!(mesh.morphs.iter().any(|m| m.name == "Breast_Physics_LeftRight_Driven"));
            }
            count += 1;
        }
        assert!(count > 0);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_llm(b"").is_err());
        assert!(parse_llm(b"Linden Binary Mesh 1.0").is_err());
        let mut d = b"Linden Binary Mesh 1.0\0\0".to_vec();
        d.extend_from_slice(&[0u8; 39]);
        d.extend_from_slice(&[0xFF, 0xFF]); // 65535 vertices, no data
        assert!(parse_llm(&d).is_err());
    }
}
