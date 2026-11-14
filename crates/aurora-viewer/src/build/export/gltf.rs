//! The binary glTF 2.0 file (`.glb`) of a model. Aurora's own: the Legacy
//! Viewer's Develop › GLTF › Save As (GLTFSceneManager::saveAs,
//! indra/newview/gltfscenemanager.cpp) only saves the test scene hung on an
//! object, never its prims. The same model, rights and options as the
//! Collada and OBJ files (`collada.rs`, `obj.rs`), in one file:
//!
//! - a root node turning the grid's Z up into glTF's Y up (-90° about X),
//!   then one node per prim with its transform, placed from the first root;
//! - one mesh per prim, one primitive per material list, 32-bit indices;
//! - texture coordinates with V flipped (glTF counts it from the top);
//! - a metallic-roughness material per model material: the face colour as
//!   the base colour (linear), multiplied by its texture as the grid draws
//!   it; BLEND when the colour or the texture has transparency;
//! - the textures embedded as PNG in the file's binary chunk, by the name
//!   their item gives them (never their id);
//! - a rigged mesh with its rigging (Aurora's own: the Legacy Viewer's
//!   Collada export writes none): one skeleton for the file, the joints the
//!   rigged meshes use and their parents in avatar_skeleton.xml's tree, a
//!   weighted joint at its bind transform (the mesh shows in its bind pose,
//!   an A-pose kit stays in its A-pose), the others at their default place
//!   or the mesh's own joint position (LLMeshSkinInfo::mAlternateBindMatrix);
//!   per rigged mesh a skin with the
//!   asset's inverse bind matrices, its vertices placed by its bind shape
//!   matrix (the bind pose of LLSkinningUtil, `inverse_bind × bind_shape`:
//!   glTF has no bind shape), and each vertex's joints and weights. Every
//!   skin also lists the file's other joints, so the tools build one
//!   armature from mPelvis. The mesh is written in its bind pose.

use super::model::Model;
use super::{Settings, TextureFiles};
use aurora_assets::Skeleton;
use aurora_assets::mesh::SkinInfo;
use glam::Vec3;
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

/// A texture to embed: its PNG bytes, and whether any pixel is not opaque.
#[derive(Debug, Clone, PartialEq)]
pub struct Embedded {
    pub png: Vec<u8>,
    pub alpha: bool,
}

const MAGIC: u32 = 0x4654_6C67; // "glTF"
const CHUNK_JSON: u32 = 0x4E4F_534A; // "JSON"
const CHUNK_BIN: u32 = 0x004E_4942; // "BIN\0"
const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_INT: u32 = 5125;
const UNSIGNED_BYTE: u32 = 5121;
/// -90° about X: (x, y, z) becomes (x, z, -y).
const Z_UP_TO_Y_UP: [f32; 4] = [-std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2];

/// The face colour (sRGB) as glTF's linear base colour (llmath.h
/// sRGBtoLinear).
fn srgb_to_linear(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

/// The binary buffer and its views, each view 4-byte aligned.
#[derive(Default)]
struct Buffer {
    bytes: Vec<u8>,
    views: Vec<Value>,
}

impl Buffer {
    fn view(&mut self, data: &[u8], target: Option<u32>) -> usize {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
        let mut view = json!({ "buffer": 0, "byteOffset": self.bytes.len(), "byteLength": data.len() });
        if let Some(target) = target {
            view["target"] = json!(target);
        }
        self.bytes.extend_from_slice(data);
        self.views.push(view);
        self.views.len() - 1
    }
}

fn floats<const N: usize>(values: &[[f32; N]]) -> Vec<u8> {
    values.iter().flatten().flat_map(|v| v.to_le_bytes()).collect()
}

/// The avatar skeleton the rigged meshes' joints are laid out on.
fn skeleton() -> Option<&'static Skeleton> {
    static SKELETON: std::sync::OnceLock<Option<Skeleton>> = std::sync::OnceLock::new();
    SKELETON
        .get_or_init(
            || match Skeleton::parse(include_bytes!("../../../assets/character/avatar_skeleton.xml")) {
                Ok(skeleton) => Some(skeleton),
                Err(e) => {
                    log::warn!("object export: avatar skeleton unreadable, rigging left out: {e}");
                    None
                }
            },
        )
        .as_ref()
}

/// The inverse bind matrix written for a skin's joint. A collision volume's
/// bind carries the volume's scale (fitted mesh, as the uploaders write it);
/// a bone cannot hold a scale at rest in Blender, whose importer then
/// crushes everything weighted to it. Like Blender's own export of the
/// dev kits, the volume is written without it: its node and its inverse
/// bind both lose the scale (the bind pose is the same), and an importer
/// that knows the skeleton puts the volume's scale back, as Aurora's does
/// (import_mesh/gltf_skin.rs) and Avastar's Collada export.
fn written_bind(skeleton: &Skeleton, name: &str, inverse: glam::Mat4) -> glam::Mat4 {
    let volume = skeleton.find(name).is_some_and(|j| skeleton.joints[j].is_collision_volume);
    let bind = inverse.inverse();
    if !volume || !bind.is_finite() {
        return inverse;
    }
    let (_, rotation, translation) = bind.to_scale_rotation_translation();
    glam::Mat4::from_rotation_translation(rotation, translation).inverse()
}

/// The joint nodes of the rigged meshes' skins, under the root (node 0):
/// each joint they name and its parents up to mPelvis, in the skeleton's
/// tree. A joint a mesh weights stands at its bind transform, the inverse
/// of the mesh's inverse bind matrix (the first mesh that names it), so
/// every tool shows the mesh in its bind pose: a body kit bound in an
/// A-pose stays in it (on the default T-pose skeleton its arms were pulled
/// 45° away). A parent no mesh weights stands at its default place
/// (LLAvatarBoneInfo: `pos` scaled by the parent's scale, `rot`; the mesh's
/// own joint position when it carries one). Collision volumes go without
/// their scale (`written_bind`). A name the skeleton does not know gets a
/// node of its own under the root. Nodes carry a matrix.
struct Joints {
    /// Each joint name's node (the skeleton's names and aliases, and the
    /// unknown names).
    by_name: HashMap<String, usize>,
    /// The skeleton's joint nodes in order, each with its rest transform in
    /// the grid's frame (the root's turn to Y up left out).
    rest: Vec<(usize, glam::Mat4)>,
}

fn joint_nodes(skeleton: &Skeleton, skins: &[&SkinInfo], nodes: &mut Vec<Value>) -> Joints {
    let count = skeleton.joints.len();
    let mut needed = vec![false; count];
    let mut moved: Vec<Option<Vec3>> = vec![None; count];
    let mut bound: Vec<Option<glam::Mat4>> = vec![None; count];
    let mut unknown: Vec<(String, glam::Mat4)> = Vec::new();
    for skin in skins {
        let positions = (skin.alt_inverse_bind.len() == skin.joint_names.len()).then_some(&skin.alt_inverse_bind);
        for (i, name) in skin.joint_names.iter().enumerate() {
            let position = positions.map(|p| p[i].w_axis.truncate());
            let bind = written_bind(skeleton, name, skin.inverse_bind[i]).inverse();
            match skeleton.find(name) {
                Some(j) => {
                    if moved[j].is_none() {
                        moved[j] = position;
                    }
                    if bound[j].is_none() && bind.is_finite() {
                        bound[j] = Some(bind);
                    }
                    let mut at = Some(j);
                    while let Some(k) = at.filter(|k| !needed[*k]) {
                        needed[k] = true;
                        at = skeleton.joints[k].parent;
                    }
                }
                None if !unknown.iter().any(|(n, _)| n == name) => {
                    let bind = if bind.is_finite() {
                        bind
                    } else {
                        glam::Mat4::from_translation(position.unwrap_or(Vec3::ZERO))
                    };
                    unknown.push((name.clone(), bind));
                }
                None => {}
            }
        }
    }
    let mut node_of: Vec<Option<usize>> = vec![None; count];
    let mut global: Vec<glam::Mat4> = vec![glam::Mat4::IDENTITY; count];
    let mut by_name = HashMap::new();
    let mut rest = Vec::new();
    // pre-order: a parent's node is made before its children's
    for (j, joint) in skeleton.joints.iter().enumerate().filter(|(j, _)| needed[*j]) {
        let parent = joint.parent.map_or(glam::Mat4::IDENTITY, |p| global[p]);
        global[j] = bound[j].unwrap_or_else(|| {
            let parent_scale = joint.parent.map_or(Vec3::ONE, |p| skeleton.joints[p].scale);
            let translation = moved[j].unwrap_or(joint.pos) * parent_scale;
            let rotation = aurora_assets::skeleton::maya_q_xyz(joint.rot_euler_deg);
            let scale = if joint.is_collision_volume { joint.scale } else { Vec3::ONE };
            parent * glam::Mat4::from_scale_rotation_translation(scale, rotation, translation)
        });
        let local = parent.inverse() * global[j];
        nodes.push(json!({ "name": joint.name, "matrix": local.to_cols_array() }));
        let index = nodes.len() - 1;
        node_of[j] = Some(index);
        let parent = joint.parent.and_then(|p| node_of[p]).unwrap_or(0);
        if let Some(children) = nodes[parent]["children"].as_array_mut() {
            children.push(json!(index));
        } else {
            nodes[parent]["children"] = json!([index]);
        }
        by_name.insert(joint.name.clone(), index);
        for alias in &joint.aliases {
            by_name.entry(alias.clone()).or_insert(index);
        }
        rest.push((index, global[j]));
    }
    for (name, bind) in unknown {
        nodes.push(json!({ "name": name, "matrix": bind.to_cols_array() }));
        let index = nodes.len() - 1;
        if let Some(children) = nodes[0]["children"].as_array_mut() {
            children.push(json!(index));
        }
        by_name.insert(name, index);
    }
    Joints { by_name, rest }
}

/// The `.glb` bytes. `images` holds the textures fetched for the file;
/// those missing (not allowed, or not arrived) leave the colour alone.
pub fn write_glb(model: &Model, textures: &TextureFiles, images: &HashMap<Uuid, Embedded>, settings: &Settings, tool: &str) -> Vec<u8> {
    let mut buffer = Buffer::default();
    let mut accessors: Vec<Value> = Vec::new();
    let mut accessor = |buffer: &mut Buffer, data: &[u8], target: Option<u32>, value: Value| {
        let view = buffer.view(data, target);
        let mut a = value;
        a["bufferView"] = json!(view);
        accessors.push(a);
        accessors.len() - 1
    };

    // textures: one image and one texture per embedded texture, in name order
    let mut texture_index: HashMap<Uuid, (usize, bool)> = HashMap::new();
    let mut gltf_images = Vec::new();
    let mut gltf_textures = Vec::new();
    if settings.textures {
        let mut named: Vec<(&Uuid, &String)> = textures.names.iter().filter(|(id, _)| images.contains_key(id)).collect();
        named.sort_by(|a, b| a.1.cmp(b.1));
        for (id, name) in named {
            let image = &images[id];
            let view = buffer.view(&image.png, None);
            gltf_images.push(json!({ "name": name, "mimeType": "image/png", "bufferView": view }));
            gltf_textures.push(json!({ "sampler": 0, "source": gltf_images.len() - 1 }));
            texture_index.insert(*id, (gltf_textures.len() - 1, image.alpha));
        }
    }

    let materials: Vec<Value> = model
        .materials
        .iter()
        .map(|m| {
            let [r, g, b, a] = m.color;
            let mut pbr = json!({
                "baseColorFactor": [srgb_to_linear(r), srgb_to_linear(g), srgb_to_linear(b), a.clamp(0.0, 1.0)],
                "metallicFactor": 0.0,
                "roughnessFactor": 1.0,
            });
            let mut blend = a < 0.999;
            if let Some((index, alpha)) = texture_index.get(&m.texture) {
                pbr["baseColorTexture"] = json!({ "index": index });
                blend |= *alpha;
            }
            json!({
                "name": m.name,
                "pbrMetallicRoughness": pbr,
                "alphaMode": if blend { "BLEND" } else { "OPAQUE" },
            })
        })
        .collect();

    let mut meshes = Vec::new();
    let mut nodes = vec![json!({ "name": "root", "rotation": Z_UP_TO_Y_UP, "children": [] })];
    let skeleton = skeleton();
    // the rigged prims' skins, in their nodes' order
    let mut skins: Vec<&SkinInfo> = Vec::new();
    for prim in &model.prims {
        let groups: Vec<_> = prim.groups.iter().filter(|g| !g.indices.is_empty()).collect();
        if prim.positions.is_empty() || groups.is_empty() {
            continue;
        }
        let count = prim.positions.len();
        let skin = prim
            .skin
            .as_deref()
            .filter(|_| skeleton.is_some() && prim.joints.len() == count && prim.weights.len() == count);
        // glTF has no bind shape matrix: a rigged mesh's vertices are written
        // with it applied, and its joints keep the asset's own inverse bind
        // matrices (the same pose: inverse_bind × (bind_shape × v)), so the
        // tools that rebuild the bones from them place them where the
        // uploader's skeleton had them
        let (positions, normals): (std::borrow::Cow<[[f32; 3]]>, std::borrow::Cow<[[f32; 3]]>) = match skin {
            Some(skin) => {
                let m = skin.bind_shape;
                let n = glam::Mat3::from_mat4(m).inverse().transpose();
                (
                    prim.positions
                        .iter()
                        .map(|p| m.transform_point3(Vec3::from(*p)).to_array())
                        .collect(),
                    prim.normals
                        .iter()
                        .map(|v| (n * Vec3::from(*v)).normalize_or(Vec3::Z).to_array())
                        .collect(),
                )
            }
            None => (prim.positions.as_slice().into(), prim.normals.as_slice().into()),
        };
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in positions.iter() {
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        let position = accessor(
            &mut buffer,
            &floats(&positions),
            Some(ARRAY_BUFFER),
            json!({ "componentType": FLOAT, "count": count, "type": "VEC3", "min": min, "max": max }),
        );
        let normal = accessor(
            &mut buffer,
            &floats(&normals),
            Some(ARRAY_BUFFER),
            json!({ "componentType": FLOAT, "count": normals.len(), "type": "VEC3" }),
        );
        let uvs: Vec<[f32; 2]> = prim.uvs.iter().map(|[u, v]| [*u, 1.0 - v]).collect();
        let uv = accessor(
            &mut buffer,
            &floats(&uvs),
            Some(ARRAY_BUFFER),
            json!({ "componentType": FLOAT, "count": uvs.len(), "type": "VEC2" }),
        );
        let rig = skin.map(|skin| {
            let joints: Vec<u8> = prim.joints.iter().flatten().copied().collect();
            let joints = accessor(
                &mut buffer,
                &joints,
                Some(ARRAY_BUFFER),
                json!({ "componentType": UNSIGNED_BYTE, "count": count, "type": "VEC4" }),
            );
            let weights = accessor(
                &mut buffer,
                &floats(&prim.weights),
                Some(ARRAY_BUFFER),
                json!({ "componentType": FLOAT, "count": count, "type": "VEC4" }),
            );
            skins.push(skin);
            (joints, weights)
        });
        let primitives: Vec<Value> = groups
            .iter()
            .map(|g| {
                let bytes: Vec<u8> = g.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
                let indices = accessor(
                    &mut buffer,
                    &bytes,
                    Some(ELEMENT_ARRAY_BUFFER),
                    json!({ "componentType": UNSIGNED_INT, "count": g.indices.len(), "type": "SCALAR" }),
                );
                let mut attributes = json!({ "POSITION": position, "NORMAL": normal, "TEXCOORD_0": uv });
                if let Some((joints, weights)) = rig {
                    attributes["JOINTS_0"] = json!(joints);
                    attributes["WEIGHTS_0"] = json!(weights);
                }
                json!({ "attributes": attributes, "indices": indices, "material": g.material })
            })
            .collect();
        meshes.push(json!({ "name": prim.name, "primitives": primitives }));
        // a skinned mesh is placed by its joints: its node has no transform
        nodes.push(if rig.is_some() {
            json!({ "name": prim.name, "mesh": meshes.len() - 1, "skin": skins.len() - 1 })
        } else {
            json!({ "name": prim.name, "mesh": meshes.len() - 1, "matrix": prim.transform.to_cols_array() })
        });
        let child = nodes.len() - 1;
        if let Some(children) = nodes[0]["children"].as_array_mut() {
            children.push(json!(child));
        }
    }

    // Each skin: the joints it weights, with the asset's bind pose, then
    // every other joint of the file's skeleton, with the inverse of its rest
    // transform (no vertex follows them): one armature from mPelvis for all
    // the rigged meshes in tools that build one per skin root (Blender).
    let mut gltf_skins: Vec<Value> = Vec::new();
    if let Some(skeleton) = skeleton.filter(|_| !skins.is_empty()) {
        let joints = joint_nodes(skeleton, &skins, &mut nodes);
        for skin in &skins {
            let mut list: Vec<usize> = Vec::new();
            let mut bind: Vec<[f32; 16]> = Vec::new();
            for (name, inverse) in skin.joint_names.iter().zip(&skin.inverse_bind) {
                let mut node = joints.by_name[name];
                if list.contains(&node) {
                    // a name and its alias both listed: glTF wants each joint
                    // once, so the second gets a node of its own on the first
                    nodes.push(json!({ "name": name }));
                    let own = nodes.len() - 1;
                    if let Some(children) = nodes[node]["children"].as_array_mut() {
                        children.push(json!(own));
                    } else {
                        nodes[node]["children"] = json!([own]);
                    }
                    node = own;
                }
                list.push(node);
                bind.push(written_bind(skeleton, name, *inverse).to_cols_array());
            }
            for (node, rest) in &joints.rest {
                if !list.contains(node) {
                    list.push(*node);
                    bind.push(rest.inverse().to_cols_array());
                }
            }
            let matrices = accessor(
                &mut buffer,
                &floats(&bind),
                None,
                json!({ "componentType": FLOAT, "count": bind.len(), "type": "MAT4" }),
            );
            gltf_skins.push(json!({ "inverseBindMatrices": matrices, "joints": list }));
        }
    }

    while !buffer.bytes.len().is_multiple_of(4) {
        buffer.bytes.push(0);
    }
    let mut doc = json!({
        "asset": { "version": "2.0", "generator": tool },
        "scene": 0,
        "scenes": [{ "nodes": [0] }],
        "nodes": nodes,
    });
    // glTF arrays, when present, are never empty
    let mut set = |key: &str, list: Vec<Value>| {
        if !list.is_empty() {
            doc[key] = Value::Array(list);
        }
    };
    let used = !meshes.is_empty();
    set("meshes", meshes);
    // materials only for primitives that use them
    set("materials", if used { materials } else { Vec::new() });
    set("accessors", accessors);
    set("bufferViews", buffer.views);
    if !gltf_textures.is_empty() {
        // REPEAT both ways (the default), as the grid tiles textures
        set("samplers", vec![json!({ "magFilter": 9729, "minFilter": 9987 })]);
    }
    set("images", gltf_images);
    set("textures", gltf_textures);
    set("skins", gltf_skins);
    if !buffer.bytes.is_empty() {
        doc["buffers"] = json!([{ "byteLength": buffer.bytes.len() }]);
    }
    glb(&doc, buffer.bytes)
}

/// The binary container: header, JSON chunk (space padded), BIN chunk (zero
/// padded).
fn glb(doc: &Value, mut bin: Vec<u8>) -> Vec<u8> {
    let mut json = doc.to_string().into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&MAGIC.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&json);
    if !bin.is_empty() {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(&bin);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::model::{SourceFace, SourcePrim, build};
    use super::*;
    use aurora_prim::te::TextureFace;
    use glam::Quat;

    fn model() -> Model {
        let face = |texture: u128, color: [f32; 4]| SourceFace {
            positions: vec![[0.0, 0.0, 0.0], [0.5, 0.0, 0.0], [0.0, 0.5, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
            te: TextureFace {
                texture: Uuid::from_u128(texture),
                color,
                ..Default::default()
            },
            ..Default::default()
        };
        let prim = SourcePrim {
            position: Vec3::new(100.0, 100.0, 20.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(2.0, 2.0, 2.0),
            faces: vec![face(1, [1.0; 4]), face(2, [1.0, 0.0, 0.0, 0.5])],
            skin: None,
        };
        build(
            &[
                prim.clone(),
                SourcePrim {
                    position: Vec3::new(100.0, 103.0, 20.0),
                    ..prim
                },
            ],
            &Settings::default(),
        )
    }

    /// Reads a `.glb` back: the JSON and the binary chunk.
    fn read(glb: &[u8]) -> (Value, Vec<u8>) {
        let word = |at: usize| u32::from_le_bytes(glb[at..at + 4].try_into().expect("4 bytes"));
        assert_eq!(word(0), MAGIC);
        assert_eq!(word(4), 2);
        assert_eq!(word(8) as usize, glb.len());
        let json_len = word(12) as usize;
        assert_eq!(word(16), CHUNK_JSON);
        assert_eq!(json_len % 4, 0);
        let doc: Value = serde_json::from_slice(&glb[20..20 + json_len]).expect("json chunk");
        let at = 20 + json_len;
        if at == glb.len() {
            return (doc, Vec::new());
        }
        assert_eq!(word(at + 4), CHUNK_BIN);
        let bin = glb[at + 8..at + 8 + word(at) as usize].to_vec();
        (doc, bin)
    }

    fn accessor_floats(doc: &Value, bin: &[u8], accessor: usize) -> Vec<f32> {
        let a = &doc["accessors"][accessor];
        let view = &doc["bufferViews"][a["bufferView"].as_u64().expect("view") as usize];
        let start = view["byteOffset"].as_u64().expect("offset") as usize;
        let len = view["byteLength"].as_u64().expect("length") as usize;
        bin[start..start + len]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect()
    }

    #[test]
    fn a_glb_reads_back_with_one_node_per_prim_under_a_y_up_root() {
        let glb = write_glb(
            &model(),
            &TextureFiles::default(),
            &HashMap::new(),
            &Settings::default(),
            "Aurora Viewer",
        );
        let (doc, bin) = read(&glb);
        assert_eq!(doc["asset"]["version"], "2.0");
        assert_eq!(doc["asset"]["generator"], "Aurora Viewer");
        assert_eq!(doc["buffers"][0]["byteLength"].as_u64(), Some(bin.len() as u64));
        let root = &doc["nodes"][0];
        assert_eq!(root["children"], json!([1, 2]));
        let r = root["rotation"].as_array().expect("rotation");
        assert!((r[0].as_f64().expect("x") + 0.70710677).abs() < 1e-6 && (r[3].as_f64().expect("w") - 0.70710677).abs() < 1e-6);
        // the second prim, 3 m north of the first root, scaled by 2
        let m: Vec<f32> = doc["nodes"][2]["matrix"]
            .as_array()
            .expect("matrix")
            .iter()
            .map(|v| v.as_f64().expect("number") as f32)
            .collect();
        assert_eq!(&m[12..15], &[0.0, 3.0, 0.0]);
        assert_eq!(m[0], 2.0);
        // two materials, so two primitives per mesh, each with its indices
        let mesh = &doc["meshes"][0];
        assert_eq!(mesh["primitives"].as_array().expect("primitives").len(), 2);
        let first = &mesh["primitives"][0];
        let position = first["attributes"]["POSITION"].as_u64().expect("position") as usize;
        assert_eq!(doc["accessors"][position]["count"], 6);
        assert_eq!(doc["accessors"][position]["max"], json!([0.5, 0.5, 0.0]));
        let indices = first["indices"].as_u64().expect("indices") as usize;
        assert_eq!(doc["accessors"][indices]["componentType"], UNSIGNED_INT);
        // V counted from the top: (u, v) = (0, 1) becomes (0, 0)
        let uv = first["attributes"]["TEXCOORD_0"].as_u64().expect("uv") as usize;
        assert_eq!(&accessor_floats(&doc, &bin, uv)[..6], &[0.0, 1.0, 1.0, 1.0, 0.0, 0.0]);
        // every view starts on a 4-byte boundary
        for view in doc["bufferViews"].as_array().expect("views") {
            assert_eq!(view["byteOffset"].as_u64().expect("offset") % 4, 0);
        }
    }

    #[test]
    fn colours_are_linear_and_transparency_blends() {
        let glb = write_glb(&model(), &TextureFiles::default(), &HashMap::new(), &Settings::default(), "Aurora");
        let (doc, _) = read(&glb);
        let white = &doc["materials"][0];
        assert_eq!(white["alphaMode"], "OPAQUE");
        assert_eq!(white["pbrMetallicRoughness"]["baseColorFactor"], json!([1.0, 1.0, 1.0, 1.0]));
        assert!(
            white["pbrMetallicRoughness"].get("baseColorTexture").is_none(),
            "no texture allowed: colour only"
        );
        let red = &doc["materials"][1];
        assert_eq!(red["alphaMode"], "BLEND");
        assert_eq!(red["pbrMetallicRoughness"]["baseColorFactor"], json!([1.0, 0.0, 0.0, 0.5]));
        assert_eq!(red["pbrMetallicRoughness"]["metallicFactor"], 0.0);
        let half = srgb_to_linear(0.5);
        assert!((half - 0.214).abs() < 0.01, "sRGB 0.5 is about 0.214 linear");
        assert!(doc.get("images").is_none() && doc.get("textures").is_none());
    }

    #[test]
    fn allowed_textures_are_embedded_by_name_and_their_alpha_blends() {
        let mut files = TextureFiles {
            extension: "png".into(),
            ..Default::default()
        };
        files.names.insert(Uuid::from_u128(1), "Brick".into());
        // texture 2 is not allowed: never embedded even when its pixels are here
        let images = HashMap::from([
            (
                Uuid::from_u128(1),
                Embedded {
                    png: vec![1, 2, 3, 4, 5],
                    alpha: true,
                },
            ),
            (
                Uuid::from_u128(2),
                Embedded {
                    png: vec![9; 8],
                    alpha: false,
                },
            ),
        ]);
        let glb = write_glb(&model(), &files, &images, &Settings::default(), "Aurora");
        let (doc, bin) = read(&glb);
        assert_eq!(doc["images"].as_array().expect("images").len(), 1);
        assert_eq!(doc["images"][0]["name"], "Brick");
        assert_eq!(doc["images"][0]["mimeType"], "image/png");
        let view = &doc["bufferViews"][doc["images"][0]["bufferView"].as_u64().expect("view") as usize];
        let start = view["byteOffset"].as_u64().expect("offset") as usize;
        assert_eq!(&bin[start..start + 5], &[1, 2, 3, 4, 5]);
        assert!(view.get("target").is_none(), "an image view has no target");
        let brick = &doc["materials"][0];
        assert_eq!(brick["pbrMetallicRoughness"]["baseColorTexture"]["index"], 0);
        assert_eq!(brick["alphaMode"], "BLEND", "the texture has transparency");
        assert!(doc["materials"][1]["pbrMetallicRoughness"].get("baseColorTexture").is_none());
        // the option off: colours only
        let off = Settings {
            textures: false,
            ..Settings::default()
        };
        let (doc, _) = read(&write_glb(&model(), &files, &images, &off, "Aurora"));
        assert!(doc.get("images").is_none());
        assert_eq!(doc["materials"][0]["alphaMode"], "OPAQUE");
    }

    /// A rigged triangle on mChest and the BELLY volume, mChest moved by the
    /// mesh, beside a plain prim.
    fn rigged_model() -> (Model, std::sync::Arc<SkinInfo>) {
        use glam::Mat4;
        let skin = std::sync::Arc::new(SkinInfo {
            joint_names: vec!["mChest".into(), "BELLY".into()],
            inverse_bind: vec![
                Mat4::from_translation(Vec3::new(0.0, 0.0, -1.4)),
                // a collision volume's bind carries its scale (fitted mesh)
                Mat4::from_scale_rotation_translation(Vec3::new(0.09, 0.08, 0.06), glam::Quat::IDENTITY, Vec3::new(0.0, 0.0, 1.1))
                    .inverse(),
            ],
            bind_shape: Mat4::from_scale(Vec3::splat(0.5)),
            alt_inverse_bind: vec![
                Mat4::from_translation(Vec3::new(-0.02, 0.0, 0.25)),
                Mat4::from_translation(Vec3::new(0.0, 0.0, 0.0)),
            ],
            ..Default::default()
        });
        let face = SourceFace {
            positions: vec![[0.0, 0.0, 0.0], [0.5, 0.0, 0.0], [0.0, 0.5, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
            te: TextureFace {
                texture: Uuid::from_u128(1),
                color: [1.0; 4],
                ..Default::default()
            },
            joints: vec![[0, 1, 0, 0], [0, 0, 0, 0], [1, 0, 0, 0]],
            weights: vec![[0.5, 0.5, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
        };
        let rigged = SourcePrim {
            position: Vec3::new(10.0, 10.0, 20.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            faces: vec![face.clone()],
            skin: Some(skin.clone()),
        };
        let plain = SourcePrim {
            faces: vec![SourceFace {
                joints: Vec::new(),
                weights: Vec::new(),
                ..face
            }],
            skin: None,
            ..rigged.clone()
        };
        (build(&[rigged, plain], &Settings::default()), skin)
    }

    fn node_named<'a>(doc: &'a Value, name: &str) -> (usize, &'a Value) {
        doc["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .enumerate()
            .find(|(_, n)| n["name"] == name)
            .unwrap_or_else(|| panic!("no node {name}"))
    }

    fn parent_of(doc: &Value, child: usize) -> Option<usize> {
        doc["nodes"]
            .as_array()
            .expect("nodes")
            .iter()
            .position(|n| n["children"].as_array().is_some_and(|c| c.contains(&json!(child))))
    }

    /// A node's transform from under the root (the root's turn to Y up left
    /// out), from the nodes' matrices.
    fn global(doc: &Value, node: usize) -> glam::Mat4 {
        let local = |n: usize| {
            let cols: Vec<f32> = doc["nodes"][n]["matrix"]
                .as_array()
                .map(|m| m.iter().map(|v| v.as_f64().expect("number") as f32).collect())
                .unwrap_or_else(|| glam::Mat4::IDENTITY.to_cols_array().to_vec());
            glam::Mat4::from_cols_slice(&cols)
        };
        let mut m = local(node);
        let mut at = parent_of(doc, node);
        while let Some(p) = at.filter(|p| *p != 0) {
            m = local(p) * m;
            at = parent_of(doc, p);
        }
        m
    }

    #[test]
    fn a_rigged_mesh_is_written_with_its_skin_joints_and_weights() {
        let (model, skin) = rigged_model();
        let (doc, bin) = read(&write_glb(
            &model,
            &TextureFiles::default(),
            &HashMap::new(),
            &Settings::default(),
            "Aurora",
        ));
        // the rigged prim's node: its skin, no transform; the plain one keeps its matrix
        let (_, rigged) = node_named(&doc, "prim0");
        assert_eq!(rigged["skin"], 0);
        assert!(rigged.get("matrix").is_none());
        assert!(node_named(&doc, "prim1").1.get("matrix").is_some());
        assert_eq!(doc["skins"].as_array().expect("skins").len(), 1);
        // the joints by name, mChest moved by the mesh, BELLY with its scale
        let joints: Vec<&str> = doc["skins"][0]["joints"]
            .as_array()
            .expect("joints")
            .iter()
            .map(|j| doc["nodes"][j.as_u64().expect("joint") as usize]["name"].as_str().expect("name"))
            .collect();
        // the joints it weights first, then the rest of the file's skeleton
        assert_eq!(&joints[..2], &["mChest", "BELLY"]);
        assert_eq!(joints[2], "mPelvis", "the skeleton from its root, for one armature");
        assert!(joints.contains(&"mSpine4") && joints.len() == 8, "{joints:?}");
        let (chest, _) = node_named(&doc, "mChest");
        // mChest's parents up to the root, as avatar_skeleton.xml chains them
        let mut chain = Vec::new();
        let mut at = parent_of(&doc, chest);
        while let Some(node) = at {
            chain.push(doc["nodes"][node]["name"].as_str().expect("name").to_owned());
            at = parent_of(&doc, node);
        }
        assert_eq!(chain, ["mSpine4", "mSpine3", "mTorso", "mSpine2", "mSpine1", "mPelvis", "root"]);
        assert!(
            doc["nodes"].as_array().expect("nodes").iter().all(|n| n["name"] != "mHead"),
            "only the joints used and their parents"
        );
        // per-vertex joints (bytes) and weights
        let attributes = &doc["meshes"][0]["primitives"][0]["attributes"];
        let joints = &doc["accessors"][attributes["JOINTS_0"].as_u64().expect("joints") as usize];
        assert_eq!(joints["componentType"], UNSIGNED_BYTE);
        assert_eq!(joints["type"], "VEC4");
        let view = &doc["bufferViews"][joints["bufferView"].as_u64().expect("view") as usize];
        let start = view["byteOffset"].as_u64().expect("offset") as usize;
        assert_eq!(&bin[start..start + 12], &[0, 1, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0]);
        let weights = accessor_floats(&doc, &bin, attributes["WEIGHTS_0"].as_u64().expect("weights") as usize);
        assert_eq!(&weights[..4], &[0.5, 0.5, 0.0, 0.0]);
        // the bind pose: inverse_bind × bind_shape
        let matrices = accessor_floats(&doc, &bin, doc["skins"][0]["inverseBindMatrices"].as_u64().expect("ibm") as usize);
        assert_eq!(matrices.len(), 16 * 8);
        assert_eq!(
            &matrices[..16],
            &skin.inverse_bind[0].to_cols_array(),
            "the asset's own inverse bind"
        );
        // the bind shape (a 0.5 scale here) applied to the vertices instead
        let position = &doc["accessors"][attributes["POSITION"].as_u64().expect("position") as usize];
        assert_eq!(position["max"], json!([0.25, 0.25, 0.0]));
        let plain = &doc["meshes"][1]["primitives"][0]["attributes"]["POSITION"];
        assert_eq!(
            doc["accessors"][plain.as_u64().expect("position") as usize]["max"],
            json!([0.5, 0.5, 0.0]),
            "a plain prim is not scaled"
        );
        // an unweighted parent: the inverse of its rest place (mPelvis at its default spot)
        let pelvis = skeleton().expect("skeleton").joints[skeleton().expect("skeleton").find("mPelvis").expect("mPelvis")].pos;
        let rest = glam::Mat4::from_cols_array(&matrices[32..48].try_into().expect("16 floats"));
        assert!(rest.w_axis.truncate().abs_diff_eq(-pelvis, 1e-5), "{rest:?}");
        assert!(doc["meshes"][1]["primitives"][0]["attributes"].get("JOINTS_0").is_none());
        // every joint of the skin: its node × its inverse bind is the identity,
        // so the mesh shows in its bind pose (an A-pose kit stays in it)
        for (k, joint) in doc["skins"][0]["joints"].as_array().expect("joints").iter().enumerate() {
            let ibm = glam::Mat4::from_cols_slice(&matrices[k * 16..k * 16 + 16]);
            let pose = global(&doc, joint.as_u64().expect("joint") as usize) * ibm;
            assert!(pose.abs_diff_eq(glam::Mat4::IDENTITY, 1e-4), "joint {k}: {pose:?}");
        }
        // the volume is written without its scale (Blender bones hold none at rest)
        let belly = glam::Mat4::from_cols_slice(&matrices[16..32]);
        assert!(belly.to_scale_rotation_translation().0.abs_diff_eq(Vec3::ONE, 1e-5), "{belly:?}");
        assert!(belly.inverse().w_axis.truncate().abs_diff_eq(Vec3::new(0.0, 0.0, 1.1), 1e-5));
        // a weighted joint stands at its bind transform: mChest at 1.4 m
        assert!(global(&doc, chest).w_axis.truncate().abs_diff_eq(Vec3::new(0.0, 0.0, 1.4), 1e-5));
        // a parent nothing weights at its default place: mPelvis
        let (pelvis_node, _) = node_named(&doc, "mPelvis");
        let pelvis = skeleton().expect("skeleton").joints[skeleton().expect("skeleton").find("mPelvis").expect("mPelvis")].pos;
        assert!(global(&doc, pelvis_node).w_axis.truncate().abs_diff_eq(pelvis, 1e-5));
    }

    #[test]
    fn a_rigged_glb_reads_back_in_the_importer_with_the_same_joints_and_weights() {
        let (model, _) = rigged_model();
        let glb = write_glb(&model, &TextureFiles::default(), &HashMap::new(), &Settings::default(), "Aurora");
        let doc = aurora_assets::import_gltf::Document::parse(&glb, |_| Err(aurora_assets::AssetError::invalid("no external file")))
            .expect("the importer reads the file");
        let scene = aurora_assets::import_mesh::gltf::parse(&doc, skeleton().expect("skeleton")).expect("the importer reads the scene");
        let rigged: Vec<_> = scene.models.iter().filter(|m| m.skin.is_some()).collect();
        assert_eq!(rigged.len(), 1, "{:?}", scene.warnings);
        let skin = rigged[0].skin.as_ref().expect("skin");
        let vertices: Vec<_> = rigged[0].faces.iter().flat_map(|f| &f.vertices).collect();
        assert_eq!(vertices.len(), 3);
        // each vertex's weights by joint name, as written
        let by_name = |v: &aurora_assets::import_mesh::Vertex| {
            // weights as whole percentages, to compare and sort
            let mut w: Vec<(String, u32)> = (0..4)
                .filter(|&k| v.weights[k] > 0.0)
                .map(|k| (skin.names[v.joints[k] as usize].clone(), (v.weights[k] * 100.0).round() as u32))
                .collect();
            w.sort_by(|a, b| a.0.cmp(&b.0));
            w
        };
        let mut seen: Vec<Vec<(String, u32)>> = vertices.iter().map(|v| by_name(v)).collect();
        seen.sort();
        let mut expected = vec![
            vec![("BELLY".to_owned(), 50), ("mChest".to_owned(), 50)],
            vec![("mChest".to_owned(), 100)],
            vec![("BELLY".to_owned(), 100)],
        ];
        expected.sort();
        assert_eq!(seen, expected);
    }

    #[test]
    fn an_empty_model_is_still_a_valid_file() {
        let (doc, bin) = read(&write_glb(
            &Model::default(),
            &TextureFiles::default(),
            &HashMap::new(),
            &Settings::default(),
            "Aurora",
        ));
        assert!(bin.is_empty());
        assert_eq!(doc["nodes"].as_array().expect("nodes").len(), 1);
        assert_eq!(doc["scenes"][0]["nodes"], json!([0]));
    }
}
