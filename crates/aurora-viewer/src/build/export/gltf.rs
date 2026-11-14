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
//!   their item gives them (never their id).

use super::model::Model;
use super::{Settings, TextureFiles};
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

/// The `.glb` bytes. `images` holds the textures fetched for the file;
/// those missing (not allowed, or not arrived) leave the colour alone.
pub fn write_glb(model: &Model, textures: &TextureFiles, images: &HashMap<Uuid, Embedded>, settings: &Settings, tool: &str) -> Vec<u8> {
    let mut buffer = Buffer::default();
    let mut accessors: Vec<Value> = Vec::new();
    let mut accessor = |buffer: &mut Buffer, data: &[u8], target: u32, value: Value| {
        let view = buffer.view(data, Some(target));
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
    for prim in &model.prims {
        let groups: Vec<_> = prim.groups.iter().filter(|g| !g.indices.is_empty()).collect();
        if prim.positions.is_empty() || groups.is_empty() {
            continue;
        }
        let count = prim.positions.len();
        let (mut min, mut max) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &prim.positions {
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        let position = accessor(
            &mut buffer,
            &floats(&prim.positions),
            ARRAY_BUFFER,
            json!({ "componentType": FLOAT, "count": count, "type": "VEC3", "min": min, "max": max }),
        );
        let normal = accessor(
            &mut buffer,
            &floats(&prim.normals),
            ARRAY_BUFFER,
            json!({ "componentType": FLOAT, "count": prim.normals.len(), "type": "VEC3" }),
        );
        let uvs: Vec<[f32; 2]> = prim.uvs.iter().map(|[u, v]| [*u, 1.0 - v]).collect();
        let uv = accessor(
            &mut buffer,
            &floats(&uvs),
            ARRAY_BUFFER,
            json!({ "componentType": FLOAT, "count": uvs.len(), "type": "VEC2" }),
        );
        let primitives: Vec<Value> = groups
            .iter()
            .map(|g| {
                let bytes: Vec<u8> = g.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
                let indices = accessor(
                    &mut buffer,
                    &bytes,
                    ELEMENT_ARRAY_BUFFER,
                    json!({ "componentType": UNSIGNED_INT, "count": g.indices.len(), "type": "SCALAR" }),
                );
                json!({
                    "attributes": { "POSITION": position, "NORMAL": normal, "TEXCOORD_0": uv },
                    "indices": indices,
                    "material": g.material,
                })
            })
            .collect();
        meshes.push(json!({ "name": prim.name, "primitives": primitives }));
        nodes.push(json!({ "name": prim.name, "mesh": meshes.len() - 1, "matrix": prim.transform.to_cols_array() }));
        let child = nodes.len() - 1;
        if let Some(children) = nodes[0]["children"].as_array_mut() {
            children.push(json!(child));
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
    use glam::{Quat, Vec3};

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
        };
        let prim = SourcePrim {
            position: Vec3::new(100.0, 100.0, 20.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(2.0, 2.0, 2.0),
            faces: vec![face(1, [1.0; 4]), face(2, [1.0, 0.0, 0.0, 0.5])],
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
