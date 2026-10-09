//! GLTF PBR material assets (asset type "material").
//!
//! Ported from the Second Life / Firestorm viewer sources (originally LGPL 2.1):
//! - `indra/newview/llgltfmateriallist.cpp` (`LLGLTFMaterialList::onAssetLoadComplete`)
//! - `indra/newview/llmaterialeditor.cpp` (asset serialization)
//! - `indra/llprimitive/llgltfmaterial.cpp`, `llgltfmaterial_templates.h`
//!   (`LLGLTFMaterial::setFromModel`, `setFromTexture`, `gltf_get_texture_image`)
//! - `indra/llcommon/llsdserialize.cpp` (`LLSDSerialize::deserialize` format detection)
//!
//! Copyright (C) 2022-2024, Linden Research, Inc. and the Firestorm project.
//!
//! Asset container: an LLSD map serialized by `LLSDSerialize::serialize(..., LLSD_BINARY)`,
//! i.e. the text line `"<? LLSD/Binary ?>\n"` followed by binary LLSD:
//! ```text
//! { "version": "1.1" (or "1.0"), "type": "GLTF 2.0", "data": "<glTF 2.0 JSON>" }
//! ```
//! LL reads it with the auto-detecting `LLSDSerialize::deserialize`, so XML and
//! notation encodings are accepted too. The glTF JSON contains one material
//! whose texture images have `uri` = texture asset UUID string.

use aurora_llsd::{Llsd, Map};
use serde_json::Value;
use uuid::Uuid;

use crate::AssetError;

/// `LLGLTFMaterial::ASSET_TYPE`.
pub const ASSET_TYPE: &str = "GLTF 2.0";
/// `LLGLTFMaterial::ACCEPTED_ASSET_VERSIONS`.
pub const ACCEPTED_ASSET_VERSIONS: [&str; 2] = ["1.0", "1.1"];

/// KHR_texture_transform parameters.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureTransform {
    pub offset: [f32; 2],
    pub scale: [f32; 2],
    /// Radians.
    pub rotation: f32,
}

impl Default for TextureTransform {
    fn default() -> Self {
        Self {
            offset: [0.0, 0.0],
            scale: [1.0, 1.0],
            rotation: 0.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AlphaMode {
    #[default]
    Opaque,
    Blend,
    Mask,
}

/// Index into [`PbrMaterial::transforms`] (`LLGLTFMaterial::TextureInfo`).
pub const TEXTURE_BASE_COLOR: usize = 0;
pub const TEXTURE_NORMAL: usize = 1;
pub const TEXTURE_METALLIC_ROUGHNESS: usize = 2;
pub const TEXTURE_EMISSIVE: usize = 3;

/// A GLTF metallic-roughness material as used by SL (`LLGLTFMaterial`).
/// Colors are linear.
#[derive(Debug, Clone, PartialEq)]
pub struct PbrMaterial {
    pub base_color_factor: [f32; 4],
    pub base_color_texture: Option<Uuid>,
    pub metallic_factor: f32,
    pub roughness_factor: f32,
    /// ORM texture (occlusion in R is ignored by SL; roughness G, metallic B).
    pub metallic_roughness_texture: Option<Uuid>,
    pub normal_texture: Option<Uuid>,
    pub emissive_factor: [f32; 3],
    pub emissive_texture: Option<Uuid>,
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    /// base color, normal, metallic-roughness, emissive.
    pub transforms: [TextureTransform; 4],
}

impl Default for PbrMaterial {
    /// `LLGLTFMaterial::LLGLTFMaterial()` defaults.
    fn default() -> Self {
        Self {
            base_color_factor: [1.0; 4],
            base_color_texture: None,
            metallic_factor: 1.0,
            roughness_factor: 1.0,
            metallic_roughness_texture: None,
            normal_texture: None,
            emissive_factor: [0.0; 3],
            emissive_texture: None,
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
            transforms: [TextureTransform::default(); 4],
        }
    }
}

/// `LLGLTFMaterial::GLTF_OVERRIDE_NULL_UUID`: an override texture that
/// removes the base material's texture.
pub const OVERRIDE_NULL_UUID: Uuid = Uuid::from_u128(u128::MAX);

/// Per-face override of a GLTF material, as sent by the simulator (edits made
/// in the build tools on top of the material asset). Port of the override
/// side of `LLGLTFMaterial` (indra/llprimitive/llgltfmaterial.cpp, originally
/// LGPL 2.1): `None` / nil / identity means "keep the base material's value".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PbrOverride {
    /// Base color, normal, metallic-roughness, emissive: nil keeps the base
    /// texture, [`OVERRIDE_NULL_UUID`] removes it, any other id replaces it.
    pub textures: [Uuid; 4],
    pub base_color_factor: Option<[f32; 4]>,
    pub emissive_factor: Option<[f32; 3]>,
    pub metallic_factor: Option<f32>,
    pub roughness_factor: Option<f32>,
    pub alpha_mode: Option<AlphaMode>,
    pub alpha_cutoff: Option<f32>,
    pub double_sided: Option<bool>,
    /// Applied per component (offset, scale, rotation) when not identity.
    pub transforms: [TextureTransform; 4],
}

impl PbrOverride {
    /// `LLGLTFMaterial::applyOverrideLLSD` on a default material: one `od`
    /// entry of the override message. LL nudges a value equal to the default
    /// by FLT_EPSILON so that `applyOverride` still takes it; a present key
    /// becomes `Some` here instead. The type checks are LL's (`mf`, `rf`,
    /// `ac`, `r` must be reals, `am` an integer, `ds` a boolean).
    pub fn from_llsd(data: &Llsd) -> PbrOverride {
        let mut o = PbrOverride::default();
        let real = |v: &Llsd| matches!(v, Llsd::Real(_)).then(|| v.as_f64() as f32);
        for (i, id) in data.get("tex").as_array().iter().take(4).enumerate() {
            o.textures[i] = id.as_uuid();
        }
        let bc = data.get("bc");
        if !bc.is_undef() {
            o.base_color_factor = Some(std::array::from_fn(|i| bc.at(i).as_f64() as f32));
        }
        let ec = data.get("ec");
        if !ec.is_undef() {
            o.emissive_factor = Some(std::array::from_fn(|i| ec.at(i).as_f64() as f32));
        }
        o.metallic_factor = real(data.get("mf"));
        o.roughness_factor = real(data.get("rf"));
        if let Llsd::Integer(am) = data.get("am") {
            o.alpha_mode = Some(match am {
                1 => AlphaMode::Blend,
                2 => AlphaMode::Mask,
                _ => AlphaMode::Opaque,
            });
        }
        o.alpha_cutoff = real(data.get("ac"));
        if let Llsd::Boolean(ds) = data.get("ds") {
            o.double_sided = Some(*ds);
        }
        for (i, t) in data.get("ti").as_array().iter().take(4).enumerate() {
            let vec2 = |v: &Llsd| [v.at(0).as_f64() as f32, v.at(1).as_f64() as f32];
            if !t.get("o").is_undef() {
                o.transforms[i].offset = vec2(t.get("o"));
            }
            if !t.get("s").is_undef() {
                o.transforms[i].scale = vec2(t.get("s"));
            }
            if let Some(r) = real(t.get("r")) {
                o.transforms[i].rotation = r;
            }
        }
        o
    }
}

impl PbrMaterial {
    /// `LLGLTFMaterial::applyOverride`: the render material of a face is its
    /// base material with the override on top.
    pub fn apply_override(&mut self, o: &PbrOverride) {
        let textures = [
            &mut self.base_color_texture,
            &mut self.normal_texture,
            &mut self.metallic_roughness_texture,
            &mut self.emissive_texture,
        ];
        for (dst, id) in textures.into_iter().zip(o.textures) {
            // applyOverrideUUID
            if id == OVERRIDE_NULL_UUID {
                *dst = None;
            } else if !id.is_nil() {
                *dst = Some(id);
            }
        }
        if let Some(v) = o.base_color_factor {
            self.base_color_factor = v;
        }
        if let Some(v) = o.emissive_factor {
            self.emissive_factor = v;
        }
        if let Some(v) = o.metallic_factor {
            self.metallic_factor = v;
        }
        if let Some(v) = o.roughness_factor {
            self.roughness_factor = v;
        }
        if let Some(v) = o.alpha_mode {
            self.alpha_mode = v;
        }
        if let Some(v) = o.alpha_cutoff {
            self.alpha_cutoff = v;
        }
        if let Some(v) = o.double_sided {
            self.double_sided = v;
        }
        let identity = TextureTransform::default();
        for (dst, t) in self.transforms.iter_mut().zip(&o.transforms) {
            if t.offset != identity.offset {
                dst.offset = t.offset;
            }
            if t.scale != identity.scale {
                dst.scale = t.scale;
            }
            if t.rotation != identity.rotation {
                dst.rotation = t.rotation;
            }
        }
    }
}

/// Parse a material asset (LLSD container in binary, XML or notation form) or a
/// bare glTF JSON document.
pub fn parse_material_asset(data: &[u8]) -> Result<PbrMaterial, AssetError> {
    let body = trim_start(data);
    if body.is_empty() {
        return Err(AssetError::Truncated("empty material asset"));
    }

    if let Some(rest) = strip_header(body, b"LLSD/Binary") {
        let (v, _) = aurora_llsd::from_binary(rest)?;
        return material_from_container(&v);
    }
    if let Some(rest) = strip_header(body, b"LLSD/XML") {
        return material_from_container(&aurora_llsd::from_xml(rest)?);
    }
    if let Some(rest) = strip_header(body, b"llsd/notation") {
        return material_from_container(&aurora_llsd::from_notation(rest)?);
    }
    if body.starts_with(b"<") {
        return material_from_container(&aurora_llsd::from_xml(body)?);
    }
    if body.starts_with(b"{") {
        // JSON: either a bare glTF document or a JSON-compatible notation container.
        if let Ok(json) = serde_json::from_slice::<Value>(body) {
            if let Some(Value::String(gltf)) = json.get("data") {
                let mut m = Map::new();
                for key in ["version", "type"] {
                    if let Some(Value::String(s)) = json.get(key) {
                        m.insert(key.to_owned(), Llsd::String(s.clone()));
                    }
                }
                m.insert("data".to_owned(), Llsd::String(gltf.clone()));
                return material_from_container(&Llsd::Map(m));
            }
            return Ok(material_from_gltf(&json));
        }
        if let Ok(v) = aurora_llsd::from_notation(body) {
            return material_from_container(&v);
        }
        // Headerless binary LLSD map.
        let (v, _) = aurora_llsd::from_binary(body)?;
        return material_from_container(&v);
    }
    Err(AssetError::invalid("unrecognized material asset encoding"))
}

fn trim_start(mut d: &[u8]) -> &[u8] {
    if d.starts_with(&[0xEF, 0xBB, 0xBF]) {
        d = &d[3..];
    }
    let n = d.iter().take_while(|b| b.is_ascii_whitespace()).count();
    &d[n..]
}

/// Match `<? NAME ?>` (case-insensitive) and return the data after the line.
fn strip_header<'a>(d: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    let line_end = d.iter().position(|&b| b == b'\n').unwrap_or(d.len()).min(64);
    let line = d.get(..line_end)?;
    let inner = line.strip_prefix(b"<?")?;
    let inner = trim_start(inner);
    let token_len = inner.iter().position(|&b| b == b' ' || b == b'?').unwrap_or(inner.len());
    if !inner[..token_len].eq_ignore_ascii_case(name) {
        return None;
    }
    let close = d.windows(2).position(|w| w == b"?>")?;
    Some(trim_start(&d[close + 2..]))
}

fn material_from_container(asset: &Llsd) -> Result<PbrMaterial, AssetError> {
    if !asset.is_map() {
        return Err(AssetError::invalid("material asset is not a map"));
    }
    let version = asset["version"].to_string_value();
    if !ACCEPTED_ASSET_VERSIONS.contains(&version.as_str()) {
        return Err(AssetError::Unsupported(format!("material asset version \"{version}\"")));
    }
    let ty = asset["type"].to_string_value();
    if ty != ASSET_TYPE {
        return Err(AssetError::Unsupported(format!("material asset type \"{ty}\"")));
    }
    let Llsd::String(gltf) = &asset["data"] else {
        return Err(AssetError::invalid("material asset has no data string"));
    };
    let json: Value = serde_json::from_str(gltf)?;
    Ok(material_from_gltf(&json))
}

fn json_f32(v: Option<&Value>, default: f32) -> f32 {
    v.and_then(Value::as_f64).map_or(default, |f| f as f32)
}

fn json_array<const N: usize>(v: Option<&Value>, default: [f32; N]) -> [f32; N] {
    let Some(Value::Array(a)) = v else {
        return default;
    };
    if a.len() != N {
        return default;
    }
    let mut out = default;
    for (o, e) in out.iter_mut().zip(a) {
        match e.as_f64() {
            Some(f) => *o = f as f32,
            None => return default,
        }
    }
    out
}

/// `vec2FromJson`: tinygltf only accepts *real* JSON numbers here, so an
/// integer literal falls back to the default exactly like LL.
fn real_vec2(obj: &Value, key: &str, default: [f32; 2]) -> [f32; 2] {
    let Some(Value::Array(a)) = obj.get(key) else {
        return default;
    };
    if a.len() < 2 {
        return default;
    }
    let mut out = [0f32; 2];
    for (o, e) in out.iter_mut().zip(a) {
        match e {
            Value::Number(n) if n.is_f64() => *o = n.as_f64().unwrap_or(0.0) as f32,
            _ => return default,
        }
    }
    out
}

/// `floatFromJson` (real numbers only, see [`real_vec2`]).
fn real_f32(obj: &Value, key: &str, default: f32) -> f32 {
    match obj.get(key) {
        Some(Value::Number(n)) if n.is_f64() => n.as_f64().map_or(default, |f| f as f32),
        _ => default,
    }
}

/// `gltf_get_texture_image` + `LLUUID::set(uri)` + KHR_texture_transform.
fn texture_from_info(model: &Value, info: Option<&Value>) -> (Option<Uuid>, TextureTransform) {
    let mut transform = TextureTransform::default();
    let Some(info) = info.filter(|i| i.is_object()) else {
        return (None, transform);
    };
    let id = info
        .get("index")
        .and_then(Value::as_i64)
        .and_then(|i| usize::try_from(i).ok())
        .and_then(|i| model.get("textures")?.get(i))
        .and_then(|tex| tex.get("source")?.as_i64())
        .and_then(|s| usize::try_from(s).ok())
        .and_then(|s| model.get("images")?.get(s))
        .and_then(|img| img.get("uri")?.as_str())
        .and_then(|uri| Uuid::parse_str(uri.trim()).ok())
        .filter(|u| !u.is_nil());
    if let Some(t) = info.get("extensions").and_then(|e| e.get("KHR_texture_transform"))
        && t.is_object()
    {
        transform.offset = real_vec2(t, "offset", [0.0, 0.0]);
        transform.scale = real_vec2(t, "scale", [1.0, 1.0]);
        transform.rotation = real_f32(t, "rotation", 0.0);
    }
    (id, transform)
}

/// Port of `LLGLTFMaterial::setFromModel(model, 0)` on top of tinygltf defaults.
fn material_from_gltf(model: &Value) -> PbrMaterial {
    let mut m = PbrMaterial::default();
    let Some(mat) = model.get("materials").and_then(|a| a.get(0)) else {
        return m;
    };
    let pbr = mat.get("pbrMetallicRoughness");
    let pbr_get = |k: &str| pbr.and_then(|p| p.get(k));

    let (id, t) = texture_from_info(model, pbr_get("baseColorTexture"));
    m.base_color_texture = id;
    m.transforms[TEXTURE_BASE_COLOR] = t;
    let (id, t) = texture_from_info(model, mat.get("normalTexture"));
    m.normal_texture = id;
    m.transforms[TEXTURE_NORMAL] = t;
    let (id, t) = texture_from_info(model, pbr_get("metallicRoughnessTexture"));
    m.metallic_roughness_texture = id;
    m.transforms[TEXTURE_METALLIC_ROUGHNESS] = t;
    let (id, t) = texture_from_info(model, mat.get("emissiveTexture"));
    m.emissive_texture = id;
    m.transforms[TEXTURE_EMISSIVE] = t;

    m.alpha_mode = match mat.get("alphaMode").and_then(Value::as_str) {
        Some("MASK") => AlphaMode::Mask,
        Some("BLEND") => AlphaMode::Blend,
        _ => AlphaMode::Opaque,
    };
    m.alpha_cutoff = json_f32(mat.get("alphaCutoff"), 0.5).clamp(0.0, 1.0);
    m.base_color_factor = json_array(pbr_get("baseColorFactor"), [1.0; 4]);
    m.emissive_factor = json_array(mat.get("emissiveFactor"), [0.0; 3]);
    m.metallic_factor = json_f32(pbr_get("metallicFactor"), 1.0).clamp(0.0, 1.0);
    m.roughness_factor = json_f32(pbr_get("roughnessFactor"), 1.0).clamp(0.0, 1.0);
    m.double_sided = mat.get("doubleSided").and_then(Value::as_bool).unwrap_or(false);
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use aurora_llsd::llsd_map;

    const BASE: &str = "11111111-2222-3333-4444-555555555555";
    const NORMAL: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

    fn sample_gltf() -> String {
        format!(
            r#"{{
  "asset": {{ "version": "2.0" }},
  "images": [ {{ "uri": "{BASE}" }}, {{ "uri": "{NORMAL}" }}, {{ "uri": "not-a-uuid" }} ],
  "textures": [ {{ "source": 0 }}, {{ "source": 1 }}, {{ "source": 2 }} ],
  "materials": [ {{
    "pbrMetallicRoughness": {{
      "baseColorFactor": [0.5, 0.25, 1.0, 0.75],
      "baseColorTexture": {{
        "index": 0,
        "extensions": {{ "KHR_texture_transform": {{ "offset": [0.5, 0.25], "scale": [2.0, 3.0], "rotation": 1.5 }} }}
      }},
      "metallicFactor": 0.0,
      "roughnessFactor": 2.0,
      "metallicRoughnessTexture": {{ "index": 2 }}
    }},
    "normalTexture": {{
      "index": 1,
      "extensions": {{ "KHR_texture_transform": {{ "scale": [4, 4], "rotation": 0.5 }} }}
    }},
    "emissiveFactor": [0.1, 0.2, 0.3],
    "emissiveTexture": {{ "index": 7 }},
    "alphaMode": "MASK",
    "alphaCutoff": 0.25,
    "doubleSided": true
  }} ]
}}"#
        )
    }

    fn check(m: &PbrMaterial) {
        assert_eq!(m.base_color_factor, [0.5, 0.25, 1.0, 0.75]);
        assert_eq!(m.base_color_texture, Some(Uuid::parse_str(BASE).unwrap()));
        assert_eq!(m.transforms[TEXTURE_BASE_COLOR].offset, [0.5, 0.25]);
        assert_eq!(m.transforms[TEXTURE_BASE_COLOR].scale, [2.0, 3.0]);
        assert_eq!(m.transforms[TEXTURE_BASE_COLOR].rotation, 1.5);
        assert_eq!(m.normal_texture, Some(Uuid::parse_str(NORMAL).unwrap()));
        // Integer scale literal is ignored like tinygltf/LL; real rotation is kept.
        assert_eq!(m.transforms[TEXTURE_NORMAL].scale, [1.0, 1.0]);
        assert_eq!(m.transforms[TEXTURE_NORMAL].rotation, 0.5);
        assert_eq!(m.metallic_roughness_texture, None);
        assert_eq!(m.emissive_texture, None);
        assert_eq!(m.metallic_factor, 0.0);
        assert_eq!(m.roughness_factor, 1.0); // clamped
        assert_eq!(m.emissive_factor, [0.1, 0.2, 0.3]);
        assert_eq!(m.alpha_mode, AlphaMode::Mask);
        assert_eq!(m.alpha_cutoff, 0.25);
        assert!(m.double_sided);
    }

    fn container() -> Llsd {
        llsd_map! { "version" => "1.1", "type" => "GLTF 2.0", "data" => sample_gltf() }
    }

    #[test]
    fn binary_container() {
        let mut asset = b"<? LLSD/Binary ?>\n".to_vec();
        asset.extend(aurora_llsd::to_binary(&container()));
        check(&parse_material_asset(&asset).unwrap());
        // Headerless binary
        check(&parse_material_asset(&aurora_llsd::to_binary(&container())).unwrap());
    }

    #[test]
    fn xml_container() {
        let xml = aurora_llsd::to_xml(&container());
        check(&parse_material_asset(&xml).unwrap());
        let mut hdr = b"<? LLSD/XML ?>\n".to_vec();
        hdr.extend(xml);
        check(&parse_material_asset(&hdr).unwrap());
    }

    #[test]
    fn notation_container() {
        let gltf = sample_gltf().replace('\\', "\\\\").replace('\'', "\\'");
        let n = format!("<? llsd/notation ?>\n{{'version':'1.0','type':s(8)\"GLTF 2.0\",'data':'{gltf}'}}");
        check(&parse_material_asset(n.as_bytes()).unwrap());
        let n = format!(
            "{{\"version\":\"1.1\",\"type\":\"GLTF 2.0\",\"data\":{}}}",
            serde_json::to_string(&sample_gltf()).unwrap()
        );
        check(&parse_material_asset(n.as_bytes()).unwrap());
    }

    #[test]
    fn raw_gltf_and_defaults() {
        check(&parse_material_asset(sample_gltf().as_bytes()).unwrap());
        let m = parse_material_asset(br#"{"asset":{"version":"2.0"}}"#).unwrap();
        assert_eq!(m, PbrMaterial::default());
    }

    #[test]
    fn rejects_bad_assets() {
        assert!(parse_material_asset(b"").is_err());
        assert!(parse_material_asset(b"garbage").is_err());
        let bad = llsd_map! { "version" => "2.0", "type" => "GLTF 2.0", "data" => "{}" };
        assert!(matches!(
            parse_material_asset(&aurora_llsd::to_binary(&bad)),
            Err(AssetError::Unsupported(_))
        ));
        let bad = llsd_map! { "version" => "1.1", "type" => "GLTF 2.0", "data" => "{not json" };
        assert!(parse_material_asset(&aurora_llsd::to_binary(&bad)).is_err());
        assert!(parse_material_asset(b"{'version':'1.1'").is_err());
    }

    #[test]
    fn override_from_llsd_and_apply() {
        let od = aurora_llsd::from_notation(
            b"{'bc':[r1,r0.5,r0.5,r1],'tex':[!,!,!,uffffffff-ffff-ffff-ffff-ffffffffffff],\
              'ti':[{'s':[r8,r4]},{'s':[r8,r4],'o':[r0.25,r0]}],'mf':i0,'rf':r0.25,'am':i2,'ac':r0.5,'ds':1}",
        )
        .unwrap();
        let o = PbrOverride::from_llsd(&od);
        assert_eq!(o.base_color_factor, Some([1.0, 0.5, 0.5, 1.0]));
        assert_eq!(o.textures[TEXTURE_EMISSIVE], OVERRIDE_NULL_UUID);
        // mf must be a real (LL ignores an integer)
        assert_eq!(o.metallic_factor, None);
        assert_eq!(o.roughness_factor, Some(0.25));
        assert_eq!(o.alpha_mode, Some(AlphaMode::Mask));
        // equal to the default but present: LL nudges it so that it applies
        assert_eq!(o.alpha_cutoff, Some(0.5));
        assert_eq!(o.double_sided, Some(true));

        let base_id = Uuid::parse_str(BASE).unwrap();
        let mut m = PbrMaterial {
            base_color_texture: Some(base_id),
            emissive_texture: Some(Uuid::parse_str(NORMAL).unwrap()),
            alpha_cutoff: 0.3,
            ..Default::default()
        };
        m.transforms[TEXTURE_NORMAL].rotation = 1.0;
        m.apply_override(&o);
        assert_eq!(m.base_color_texture, Some(base_id), "nil override keeps the base texture");
        assert_eq!(m.emissive_texture, None, "the null sentinel removes it");
        assert_eq!(m.base_color_factor, [1.0, 0.5, 0.5, 1.0]);
        assert_eq!(m.metallic_factor, 1.0);
        assert_eq!(m.roughness_factor, 0.25);
        assert_eq!(m.alpha_cutoff, 0.5);
        assert_eq!(m.transforms[TEXTURE_BASE_COLOR].scale, [8.0, 4.0]);
        assert_eq!(m.transforms[TEXTURE_NORMAL].offset, [0.25, 0.0]);
        assert_eq!(m.transforms[TEXTURE_NORMAL].rotation, 1.0, "identity components keep the base");
        assert_eq!(m.transforms[TEXTURE_METALLIC_ROUGHNESS], TextureTransform::default());
    }
}
