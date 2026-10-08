//! Second Life mesh assets: header, LOD geometry and skin info.
//!
//! Ported from the Second Life / Firestorm viewer sources (originally LGPL 2.1):
//! - `indra/newview/llmeshrepository.cpp` / `.h` (`LLMeshHeader::fromLLSD`,
//!   `LLMeshRepoThread::headerReceived`, `getActualMeshLOD`)
//! - `indra/llmath/llvolume.cpp` (`LLVolume::unpackVolumeFacesInternal`)
//! - `indra/llprimitive/llmodel.cpp` (`LLMeshSkinInfo::fromLLSD`)
//! - `indra/llcommon/llsdserialize.cpp` (`LLUZipHelper::unzip_llsd`)
//!
//! Copyright (C) 2010-2024, Linden Research, Inc. and the Firestorm project.
//!
//! Conventions:
//! - Positions are in the mesh's normalized object space (as stored; the
//!   object scale is applied by the renderer). Coordinates are SL's
//!   right-handed Z-up frame.
//! - Matrices are converted to glam's column-vector convention. LL stores 16
//!   reals as `mMatrix[row][col]` of a row-vector matrix (`v' = v * M`,
//!   translation in elements 12..14); loading those 16 values with
//!   `Mat4::from_cols_array` yields the transposed, column-vector equivalent,
//!   so `glam_matrix * v` == LL's `v * M`. LL's `bind_pose = bind_shape *
//!   inv_bind` (row-vector) is therefore `inverse_bind[i] * bind_shape` here.
//! - Sculpt mirror/invert flags (applied by LL from `LLVolumeParams`) are not
//!   applied; callers handle them.

use aurora_llsd::{Llsd, LlsdError};
use glam::Mat4;

use crate::AssetError;

/// `MAX_MESH_VERSION` from `llmeshrepository.cpp`.
pub const MAX_MESH_VERSION: i32 = 999;
/// Number of bytes the viewer fetches for the header (`MESH_HEADER_SIZE`).
pub const MESH_HEADER_SIZE: usize = 4096;
/// Upper bound on a decompressed mesh section.
pub const MAX_DECOMPRESSED_SIZE: usize = 32 * 1024 * 1024;

const LOD_KEYS: [&str; 4] = ["lowest_lod", "low_lod", "medium_lod", "high_lod"];

/// Parsed mesh asset header. All ranges are `(offset, size)` with the offset
/// relative to the start of the asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshHeader {
    pub header_size: usize,
    /// 0 = lowest_lod, 1 = low_lod, 2 = medium_lod, 3 = high_lod.
    pub lods: [Option<(usize, usize)>; 4],
    pub skin: Option<(usize, usize)>,
    pub physics_convex: Option<(usize, usize)>,
    pub version: i32,
}

impl MeshHeader {
    /// Port of `LLMeshRepository::getActualMeshLOD`: the requested LOD if
    /// present, else the next lower one, else the next higher one.
    pub fn actual_lod(&self, lod: usize) -> Option<usize> {
        let lod = lod.min(3);
        if self.lods[lod].is_some() {
            return Some(lod);
        }
        (0..lod)
            .rev()
            .find(|&i| self.lods[i].is_some())
            .or_else(|| (lod + 1..4).find(|&i| self.lods[i].is_some()))
    }
}

fn llsd_err(e: LlsdError, what: &'static str) -> AssetError {
    match e {
        LlsdError::Eof => AssetError::Truncated(what),
        other => AssetError::Llsd(other),
    }
}

/// Parse the binary LLSD mesh header at the start of a mesh asset.
///
/// `data` must contain at least the whole header (the viewer fetches the first
/// 4096 bytes). Offsets in the header are relative to its end; the returned
/// ranges are absolute.
pub fn parse_mesh_header(data: &[u8]) -> Result<MeshHeader, AssetError> {
    if data.is_empty() {
        return Err(AssetError::Truncated("mesh header"));
    }
    let (hdr, header_size) = aurora_llsd::from_binary(data).map_err(|e| llsd_err(e, "mesh header"))?;
    if !hdr.is_map() {
        return Err(AssetError::invalid("mesh header is not a map"));
    }
    let version = hdr["version"].as_i32();
    if version > MAX_MESH_VERSION {
        return Err(AssetError::Unsupported(format!("mesh version {version}")));
    }
    if hdr.has("404") {
        return Err(AssetError::invalid("mesh header marked 404"));
    }
    let range = |key: &str| -> Option<(usize, usize)> {
        let block = &hdr[key];
        let offset = block["offset"].as_i32();
        let size = block["size"].as_i32();
        if offset < 0 || size <= 0 {
            return None;
        }
        Some((header_size.checked_add(offset as usize)?, size as usize))
    };
    let lods = [range(LOD_KEYS[0]), range(LOD_KEYS[1]), range(LOD_KEYS[2]), range(LOD_KEYS[3])];
    if lods.iter().all(Option::is_none) {
        // LL marks such headers as 404 in getActualMeshLOD.
        return Err(AssetError::invalid("mesh header has no LOD"));
    }
    Ok(MeshHeader {
        header_size,
        lods,
        skin: range("skin"),
        physics_convex: range("physics_convex"),
        version,
    })
}

/// Inflate a zlib block, refusing output larger than `MAX_DECOMPRESSED_SIZE`.
/// Like `LLUZipHelper::unzip_llsd`, the zlib stream must be complete.
pub(crate) fn inflate_capped(input: &[u8]) -> Result<Vec<u8>, AssetError> {
    use flate2::{Decompress, FlushDecompress, Status};

    if input.is_empty() {
        return Err(AssetError::Truncated("compressed mesh section"));
    }
    let mut d = Decompress::new(true);
    let mut out: Vec<u8> = Vec::with_capacity(input.len().saturating_mul(4).clamp(4096, MAX_DECOMPRESSED_SIZE + 1));
    loop {
        if out.len() == out.capacity() {
            if out.len() > MAX_DECOMPRESSED_SIZE {
                return Err(AssetError::TooLarge("decompressed mesh section"));
            }
            let grow = out.capacity().max(4096).min(MAX_DECOMPRESSED_SIZE + 1 - out.len());
            out.reserve_exact(grow);
        }
        let before_in = d.total_in();
        let before_out = d.total_out();
        let rest = input.get(before_in as usize..).unwrap_or(&[]);
        let status = d
            .decompress_vec(rest, &mut out, FlushDecompress::None)
            .map_err(|e| AssetError::Decompress(e.to_string()))?;
        if status == Status::StreamEnd {
            break;
        }
        if d.total_in() == before_in && d.total_out() == before_out {
            return Err(AssetError::Truncated("zlib stream"));
        }
    }
    if out.len() > MAX_DECOMPRESSED_SIZE {
        return Err(AssetError::TooLarge("decompressed mesh section"));
    }
    Ok(out)
}

/// `LLUZipHelper::unzip_llsd`: inflate then parse binary LLSD.
fn unzip_llsd(section: &[u8]) -> Result<Llsd, AssetError> {
    let raw = inflate_capped(section)?;
    let (v, _) = aurora_llsd::from_binary(&raw).map_err(|e| llsd_err(e, "mesh section llsd"))?;
    Ok(v)
}

/// One submesh (texture entry) of a mesh LOD.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshFace {
    pub positions: Vec<[f32; 3]>,
    /// Empty when the asset has no normals.
    pub normals: Vec<[f32; 3]>,
    /// Empty when the asset has no texture coordinates.
    pub uvs: Vec<[f32; 2]>,
    /// Triangle list.
    pub indices: Vec<u16>,
    /// Per-vertex joint indices into [`SkinInfo::joint_names`]; empty if not rigged.
    pub joints: Vec<[u8; 4]>,
    /// Per-vertex weights matching `joints`, normalized to sum to 1.
    pub weights: Vec<[f32; 4]>,
}

impl MeshFace {
    /// True for `NoGeometry` placeholder faces and faces whose data was unusable.
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty() || self.positions.is_empty()
    }
}

#[inline]
fn u16_at(b: &[u8], i: usize) -> u16 {
    match b.get(2 * i..2 * i + 2) {
        Some(s) => u16::from_le_bytes([s[0], s[1]]),
        None => 0,
    }
}

fn vec3_llsd(v: &Llsd) -> [f32; 3] {
    v.as_vec3()
}

fn vec2_llsd(v: &Llsd) -> [f32; 2] {
    [v.at(0).as_f32(), v.at(1).as_f32()]
}

/// Decode one compressed LOD block (port of `LLVolume::unpackVolumeFacesInternal`).
///
/// Face order matches the texture entry order; `NoGeometry` submeshes are kept as
/// empty faces.
pub fn decode_mesh_lod(section: &[u8]) -> Result<Vec<MeshFace>, AssetError> {
    let mdl = unzip_llsd(section)?;
    let submeshes = match &mdl {
        Llsd::Array(a) => a.as_slice(),
        _ => return Err(AssetError::invalid("mesh LOD is not an array")),
    };
    if submeshes.is_empty() {
        return Err(AssetError::invalid("mesh LOD has no faces"));
    }
    Ok(submeshes.iter().enumerate().map(|(i, s)| decode_face(i, s)).collect())
}

fn decode_face(face_index: usize, sm: &Llsd) -> MeshFace {
    let mut face = MeshFace::default();
    if sm.has("NoGeometry") {
        return face;
    }
    let pos = sm["Position"].as_binary();
    let norm = sm["Normal"].as_binary();
    let tc = sm["TexCoord0"].as_binary();
    let idx = sm["TriangleList"].as_binary();

    let mut num_indices = idx.len() / 2;
    if !num_indices.is_multiple_of(3) {
        log::warn!("mesh face {face_index}: index count {num_indices} not divisible by 3, discarding incomplete triangle");
        num_indices -= num_indices % 3;
    }
    if num_indices < 3 {
        log::debug!("mesh face {face_index}: empty face");
        return face;
    }

    let num_verts = pos.len() / 6;
    if num_verts == 0 {
        log::warn!("mesh face {face_index}: indices without vertices");
        return face;
    }

    // Indices; drop triangles referencing missing vertices (LL would read out of bounds).
    face.indices.reserve(num_indices);
    let mut dropped = 0usize;
    for t in 0..num_indices / 3 {
        let tri = [u16_at(idx, t * 3), u16_at(idx, t * 3 + 1), u16_at(idx, t * 3 + 2)];
        if tri.iter().all(|&v| (v as usize) < num_verts) {
            face.indices.extend_from_slice(&tri);
        } else {
            dropped += 1;
        }
    }
    if dropped > 0 {
        log::warn!("mesh face {face_index}: dropped {dropped} triangles with out-of-range indices");
    }
    if face.indices.is_empty() {
        return face;
    }

    let min_pos = vec3_llsd(&sm["PositionDomain"]["Min"]);
    let max_pos = vec3_llsd(&sm["PositionDomain"]["Max"]);
    let pos_range = [max_pos[0] - min_pos[0], max_pos[1] - min_pos[1], max_pos[2] - min_pos[2]];
    face.positions = (0..num_verts)
        .map(|j| std::array::from_fn(|k| (u16_at(pos, j * 3 + k) as f32 / 65535.0) * pos_range[k] + min_pos[k]))
        .collect();

    if !norm.is_empty() {
        face.normals = (0..num_verts)
            .map(|j| std::array::from_fn(|k| (u16_at(norm, j * 3 + k) as f32 / 65535.0) * 2.0 - 1.0))
            .collect();
    }

    if !tc.is_empty() {
        let min_tc = vec2_llsd(&sm["TexCoord0Domain"]["Min"]);
        let max_tc = vec2_llsd(&sm["TexCoord0Domain"]["Max"]);
        let tc_range = [max_tc[0] - min_tc[0], max_tc[1] - min_tc[1]];
        face.uvs = (0..num_verts)
            .map(|j| {
                [
                    (u16_at(tc, j * 2) as f32 / 65535.0) * tc_range[0] + min_tc[0],
                    (u16_at(tc, j * 2 + 1) as f32 / 65535.0) * tc_range[1] + min_tc[1],
                ]
            })
            .collect();
    }

    if sm.has("Weights") {
        let (joints, weights) = unpack_weights(face_index, sm["Weights"].as_binary(), num_verts);
        face.joints = joints;
        face.weights = weights;
    }
    face
}

/// Packed weight format: per vertex up to 4 `(joint u8, weight u16 LE)` pairs,
/// terminated by a 0xFF joint byte unless all 4 influences are present.
fn unpack_weights(face_index: usize, w: &[u8], num_verts: usize) -> (Vec<[u8; 4]>, Vec<[f32; 4]>) {
    const END_INFLUENCES: u8 = 0xFF;
    let mut joints = vec![[0u8; 4]; num_verts];
    let mut weights = vec![[1.0f32, 0.0, 0.0, 0.0]; num_verts];
    let mut idx = 0usize;
    let mut cur_vertex = 0usize;
    while idx < w.len() && cur_vertex < num_verts {
        let mut joint = w[idx];
        idx += 1;
        let mut cur_influence = 0usize;
        let mut wght = [0f32; 4];
        let mut jnt = [0u8; 4];
        while joint != END_INFLUENCES && idx < w.len() {
            let Some(b) = w.get(idx..idx + 2) else {
                idx = w.len();
                break;
            };
            let influence = u16::from_le_bytes([b[0], b[1]]);
            idx += 2;
            wght[cur_influence] = (influence as f32 / 65535.0).clamp(0.001, 0.999);
            jnt[cur_influence] = joint;
            cur_influence += 1;
            if cur_influence >= 4 {
                joint = END_INFLUENCES;
            } else {
                let Some(&j) = w.get(idx) else { break };
                joint = j;
                idx += 1;
            }
        }
        let mut wsum: f32 = wght.iter().sum();
        if wsum <= 0.0 {
            wght = [0.999, 0.0, 0.0, 0.0];
            wsum = 0.999;
        }
        for v in &mut wght {
            *v /= wsum;
        }
        joints[cur_vertex] = jnt;
        weights[cur_vertex] = wght;
        cur_vertex += 1;
    }
    if cur_vertex != num_verts || idx != w.len() {
        log::warn!("mesh face {face_index}: vertex weight count does not match vertex count");
    }
    (joints, weights)
}

/// Skin binding (port of `LLMeshSkinInfo`). See the module docs for matrix conventions.
#[derive(Debug, Clone, PartialEq)]
pub struct SkinInfo {
    pub joint_names: Vec<String>,
    pub inverse_bind: Vec<Mat4>,
    pub bind_shape: Mat4,
    pub alt_inverse_bind: Vec<Mat4>,
    pub pelvis_offset: f32,
    pub lock_scale_if_joint_position: bool,
}

impl Default for SkinInfo {
    fn default() -> Self {
        Self {
            joint_names: Vec::new(),
            inverse_bind: Vec::new(),
            bind_shape: Mat4::IDENTITY,
            alt_inverse_bind: Vec::new(),
            pelvis_offset: 0.0,
            lock_scale_if_joint_position: false,
        }
    }
}

impl SkinInfo {
    /// LL's `mBindPoseMatrix[i]` (bind shape followed by inverse bind), in
    /// column-vector form: `inverse_bind[i] * bind_shape`.
    pub fn bind_pose(&self, i: usize) -> Option<Mat4> {
        self.inverse_bind.get(i).map(|ib| *ib * self.bind_shape)
    }
}

/// 16 LLSD reals (LL row-major, row-vector convention) to a glam matrix.
fn mat4_from_llsd(v: &Llsd) -> Mat4 {
    let mut a = [0f32; 16];
    for (i, e) in a.iter_mut().enumerate() {
        *e = v.at(i).as_f32();
    }
    Mat4::from_cols_array(&a)
}

/// Decode a compressed skin block (port of `LLMeshSkinInfo::fromLLSD`).
pub fn decode_skin(section: &[u8]) -> Result<SkinInfo, AssetError> {
    let skin = unzip_llsd(section)?;
    if !skin.is_map() {
        return Err(AssetError::invalid("mesh skin is not a map"));
    }
    let mut info = SkinInfo::default();

    if skin.has("joint_names") {
        info.joint_names = skin["joint_names"].as_array().iter().map(Llsd::to_string_value).collect();
    }

    if skin.has("inverse_bind_matrix") {
        info.inverse_bind = skin["inverse_bind_matrix"].as_array().iter().map(mat4_from_llsd).collect();
        if info.joint_names.len() != info.inverse_bind.len() {
            log::warn!("mesh skin: joints vs bind matrix count mismatch, dropping joint bindings");
            info.joint_names.clear();
            info.inverse_bind.clear();
        }
    }

    if skin.has("bind_shape_matrix") {
        info.bind_shape = mat4_from_llsd(&skin["bind_shape_matrix"]);
    }

    if skin.has("alt_inverse_bind_matrix") {
        info.alt_inverse_bind = skin["alt_inverse_bind_matrix"].as_array().iter().map(mat4_from_llsd).collect();
    }

    if skin.has("pelvis_offset") {
        info.pelvis_offset = skin["pelvis_offset"].as_f32();
    }

    info.lock_scale_if_joint_position = skin["lock_scale_if_joint_position"].as_bool();
    Ok(info)
}

/// Read helper for callers that hold the whole asset: the bytes of a section.
pub fn section_bytes(asset: &[u8], range: (usize, usize)) -> Option<&[u8]> {
    asset.get(range.0..range.0.checked_add(range.1)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::{Map, llsd_map, to_binary};
    use flate2::{Compression, write::ZlibEncoder};
    use std::io::Write;

    fn zip(v: &Llsd) -> Vec<u8> {
        let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
        e.write_all(&to_binary(v)).unwrap();
        e.finish().unwrap()
    }

    fn u16s(v: &[u16]) -> Llsd {
        Llsd::Binary(v.iter().flat_map(|x| x.to_le_bytes()).collect())
    }

    fn arr(v: &[f64]) -> Llsd {
        Llsd::Array(v.iter().map(|&x| Llsd::Real(x)).collect())
    }

    fn domain(min: &[f64], max: &[f64]) -> Llsd {
        llsd_map! { "Min" => arr(min), "Max" => arr(max) }
    }

    #[test]
    fn header_offsets_are_absolute() {
        let hdr = llsd_map! {
            "version" => 1,
            "creator" => uuid::Uuid::nil(),
            "lowest_lod" => llsd_map!{ "offset" => 0, "size" => 100 },
            "low_lod" => llsd_map!{ "offset" => 100, "size" => 0 },
            "high_lod" => llsd_map!{ "offset" => 300, "size" => 400 },
            "skin" => llsd_map!{ "offset" => 700, "size" => 50 },
            "physics_convex" => llsd_map!{ "offset" => 750, "size" => 20 },
        };
        let bin = to_binary(&hdr);
        let mut asset = bin.clone();
        asset.extend(std::iter::repeat_n(0u8, 800));
        let h = parse_mesh_header(&asset).unwrap();
        let hs = bin.len();
        assert_eq!(h.header_size, hs);
        assert_eq!(h.version, 1);
        assert_eq!(h.lods, [Some((hs, 100)), None, None, Some((hs + 300, 400))]);
        assert_eq!(h.skin, Some((hs + 700, 50)));
        assert_eq!(h.physics_convex, Some((hs + 750, 20)));
        assert_eq!(h.actual_lod(2), Some(0));
        assert_eq!(h.actual_lod(3), Some(3));

        // Legacy textual header prefix is skipped and counted.
        let mut legacy = b"<? LLSD/Binary ?>\n".to_vec();
        legacy.extend_from_slice(&bin);
        assert_eq!(parse_mesh_header(&legacy).unwrap().header_size, hs + 18);

        assert!(parse_mesh_header(&bin[..bin.len() / 2]).is_err());
        assert!(parse_mesh_header(b"garbage!").is_err());
        let v = llsd_map! { "version" => 1000, "high_lod" => llsd_map!{ "offset" => 0, "size" => 1 } };
        assert!(matches!(parse_mesh_header(&to_binary(&v)), Err(AssetError::Unsupported(_))));
        let v = llsd_map! { "version" => 1 };
        assert!(parse_mesh_header(&to_binary(&v)).is_err());
    }

    #[test]
    fn decode_lod_roundtrip() {
        // Face 0: a rigged triangle; face 1: NoGeometry; face 2: unrigged quad without normals.
        let mut weights = Vec::new();
        // v0: one influence (joint 2, full weight), terminated
        weights.extend_from_slice(&[2, 0xFF, 0xFF, 0xFF]);
        // v1: two influences, terminated
        weights.extend_from_slice(&[0, 0x00, 0x80, 1, 0x00, 0x80, 0xFF]);
        // v2: four influences, no terminator
        for j in 0..4u8 {
            weights.extend_from_slice(&[j, 0x00, 0x40]);
        }
        let f0 = llsd_map! {
            "Position" => u16s(&[0, 0, 0, 65535, 0, 0, 0, 65535, 65535]),
            "PositionDomain" => domain(&[-1.0, -2.0, -3.0], &[1.0, 2.0, 3.0]),
            "Normal" => u16s(&[32768, 32768, 65535, 0, 32768, 32768, 65535, 65535, 65535]),
            "TexCoord0" => u16s(&[0, 0, 65535, 0, 0, 65535]),
            "TexCoord0Domain" => domain(&[0.0, 0.0], &[2.0, 1.0]),
            "TriangleList" => u16s(&[0, 1, 2, 2]),
            "Weights" => Llsd::Binary(weights),
        };
        let f1 = llsd_map! { "NoGeometry" => true };
        let f2 = llsd_map! {
            "Position" => u16s(&[0, 0, 0, 65535, 0, 0, 65535, 65535, 0, 0, 65535, 0]),
            "PositionDomain" => domain(&[0.0, 0.0, 0.0], &[1.0, 1.0, 1.0]),
            "TriangleList" => u16s(&[0, 1, 2, 0, 2, 3, 0, 2, 9]),
        };
        let lod = Llsd::Array(vec![f0, f1, f2]);
        let faces = decode_mesh_lod(&zip(&lod)).unwrap();
        assert_eq!(faces.len(), 3);

        let f = &faces[0];
        assert_eq!(f.indices, vec![0, 1, 2]);
        assert_eq!(f.positions[0], [-1.0, -2.0, -3.0]);
        assert_eq!(f.positions[1], [1.0, -2.0, -3.0]);
        assert_eq!(f.positions[2], [-1.0, 2.0, 3.0]);
        assert!((f.normals[0][2] - 1.0).abs() < 1e-6);
        assert!((f.normals[1][0] + 1.0).abs() < 1e-6);
        assert_eq!(f.uvs[1], [2.0, 0.0]);
        assert_eq!(f.uvs[2], [0.0, 1.0]);
        assert_eq!(f.joints[0][0], 2);
        assert!((f.weights[0][0] - 1.0).abs() < 1e-6);
        assert_eq!(&f.joints[1][..2], &[0, 1]);
        assert!((f.weights[1][0] - 0.5).abs() < 1e-4 && (f.weights[1][1] - 0.5).abs() < 1e-4);
        assert_eq!(f.joints[2], [0, 1, 2, 3]);
        for w in &f.weights {
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }

        assert!(faces[1].is_empty());
        assert!(faces[1].positions.is_empty());

        let f = &faces[2];
        assert!(f.normals.is_empty() && f.uvs.is_empty() && f.joints.is_empty());
        assert_eq!(f.indices, vec![0, 1, 2, 0, 2, 3]); // triangle with index 9 dropped
        assert_eq!(f.positions[2], [1.0, 1.0, 0.0]);
    }

    #[test]
    fn lod_rejects_garbage() {
        assert!(decode_mesh_lod(&[]).is_err());
        assert!(decode_mesh_lod(b"not zlib at all").is_err());
        let z = zip(&Llsd::Array(vec![]));
        assert!(decode_mesh_lod(&z).is_err());
        let z = zip(&llsd_map! { "a" => 1 });
        assert!(decode_mesh_lod(&z).is_err());
        // truncated zlib stream
        let z = zip(&Llsd::Array(vec![llsd_map! { "NoGeometry" => true }]));
        assert!(decode_mesh_lod(&z[..z.len() - 4]).is_err());
        // garbage inside a valid face must not panic
        let f = llsd_map! {
            "Position" => Llsd::Binary(vec![1, 2, 3]),
            "TriangleList" => u16s(&[0, 1, 2]),
            "Weights" => Llsd::Binary(vec![5, 1]),
        };
        let faces = decode_mesh_lod(&zip(&Llsd::Array(vec![f]))).unwrap();
        assert!(faces[0].is_empty());
    }

    #[test]
    fn decompression_bomb_is_capped() {
        let big = vec![0u8; MAX_DECOMPRESSED_SIZE + 1024];
        let mut e = ZlibEncoder::new(Vec::new(), Compression::fast());
        e.write_all(&big).unwrap();
        let z = e.finish().unwrap();
        assert!(matches!(decode_mesh_lod(&z), Err(AssetError::TooLarge(_))));
    }

    #[test]
    fn skin_roundtrip() {
        let ident = [1.0, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.];
        let mut trans = ident;
        trans[12] = 1.0;
        trans[13] = 2.0;
        trans[14] = 3.0;
        let mut m = Map::new();
        m.insert("joint_names".into(), Llsd::Array(vec!["mPelvis".into(), "mTorso".into()]));
        m.insert("inverse_bind_matrix".into(), Llsd::Array(vec![arr(&ident), arr(&trans)]));
        m.insert("bind_shape_matrix".into(), arr(&trans));
        m.insert("alt_inverse_bind_matrix".into(), Llsd::Array(vec![arr(&ident)]));
        m.insert("pelvis_offset".into(), Llsd::Real(0.25));
        m.insert("lock_scale_if_joint_position".into(), Llsd::Boolean(true));
        let s = decode_skin(&zip(&Llsd::Map(m))).unwrap();
        assert_eq!(s.joint_names, vec!["mPelvis", "mTorso"]);
        assert_eq!(s.inverse_bind.len(), 2);
        assert_eq!(s.inverse_bind[0], Mat4::IDENTITY);
        // Translation lands in the w axis (column-vector form).
        assert_eq!(s.bind_shape.w_axis.truncate(), glam::Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(s.bind_shape.transform_point3(glam::Vec3::ZERO), glam::Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(s.alt_inverse_bind.len(), 1);
        assert_eq!(s.pelvis_offset, 0.25);
        assert!(s.lock_scale_if_joint_position);
        assert_eq!(
            s.bind_pose(1).unwrap().transform_point3(glam::Vec3::ZERO),
            glam::Vec3::new(2.0, 4.0, 6.0)
        );

        // Count mismatch drops bindings like LL.
        let m = llsd_map! {
            "joint_names" => Llsd::Array(vec!["a".into()]),
            "inverse_bind_matrix" => Llsd::Array(vec![]),
        };
        let s = decode_skin(&zip(&m)).unwrap();
        assert!(s.joint_names.is_empty() && s.inverse_bind.is_empty());
        assert_eq!(s.bind_shape, Mat4::IDENTITY);

        assert!(decode_skin(&zip(&Llsd::Array(vec![]))).is_err());
        assert!(decode_skin(b"\x78\x9c garbage").is_err());
    }
}
