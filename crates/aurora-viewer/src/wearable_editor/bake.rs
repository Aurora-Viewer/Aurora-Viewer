//! Self-only texture preview. Layer order, color operations, clothing masks
//! and visibility follow LLTexLayerSet, LLTexLayerParamColor/Alpha and
//! LLImageTGA::decodeAndProcess (indra/llappearance, indra/llimage,
//! originally LGPL 2.1). Decoding and compositing run on the scene workers;
//! neither network appearance nor inventory assets are changed by a preview.
use super::masks;
use crate::scene::avatar::{AvatarLibrary, IMG_DEFAULT_AVATAR, IMG_INVISIBLE};
use aurora_assets::wearable::Wearable;
use aurora_llsd::xml::{self, Element};
use aurora_render::preview::Image;
use rayon::prelude::*;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, LazyLock},
};
use uuid::Uuid;

const TYPES: [&str; 17] = [
    "shape",
    "skin",
    "hair",
    "eyes",
    "shirt",
    "pants",
    "shoes",
    "socks",
    "jacket",
    "gloves",
    "undershirt",
    "underpants",
    "skirt",
    "alpha",
    "tattoo",
    "physics",
    "universal",
];
const TEXTURES: &[(u32, &str, u8)] = &[
    (0, "head_bodypaint", 1),
    (1, "upper_shirt", 4),
    (2, "lower_pants", 5),
    (3, "eyes_iris", 3),
    (4, "hair_grain", 2),
    (5, "upper_bodypaint", 1),
    (6, "lower_bodypaint", 1),
    (7, "lower_shoes", 6),
    (12, "lower_socks", 7),
    (13, "upper_jacket", 8),
    (14, "lower_jacket", 8),
    (15, "upper_gloves", 9),
    (16, "upper_undershirt", 10),
    (17, "lower_underpants", 11),
    (18, "skirt", 12),
    (21, "lower_alpha", 13),
    (22, "upper_alpha", 13),
    (23, "head_alpha", 13),
    (24, "eyes_alpha", 13),
    (25, "hair_alpha", 13),
    (26, "head_tattoo", 14),
    (27, "upper_tattoo", 14),
    (28, "lower_tattoo", 14),
    (29, "head_universal_tattoo", 16),
    (30, "upper_universal_tattoo", 16),
    (31, "lower_universal_tattoo", 16),
    (32, "skirt_tattoo", 16),
    (33, "hair_tattoo", 16),
    (34, "eyes_tattoo", 16),
    (35, "leftarm_tattoo", 16),
    (36, "leftleg_tattoo", 16),
    (37, "aux1_tattoo", 16),
    (38, "aux2_tattoo", 16),
    (39, "aux3_tattoo", 16),
];

fn number(e: &Element, name: &str, default: f32) -> f32 {
    e.attr(name)
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}
fn flag(e: &Element, name: &str) -> bool {
    e.attr(name).is_some_and(|s| s.eq_ignore_ascii_case("true"))
}
fn color(s: &str) -> [f32; 4] {
    let mut values = s.split([',', ' ']).filter(|s| !s.is_empty()).filter_map(|s| s.parse::<f32>().ok());
    std::array::from_fn(|_| values.next().unwrap_or(0.0) / 255.0)
}
fn byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

pub struct Definitions {
    pub regions: Vec<Region>,
    globals: HashMap<String, Vec<ColorParam>>,
    owners: HashMap<i32, u8>,
    images: HashMap<String, Arc<Image>>,
}
pub struct Region {
    pub te: usize,
    pub size: u32,
    clear_alpha: bool,
    layers: Vec<Layer>,
}
struct Layer {
    local: Option<(u32, u8)>,
    image: Option<String>,
    mask: bool,
    alpha_only: bool,
    visibility: bool,
    replace: bool,
    global: Option<String>,
    fixed: Option<[f32; 4]>,
    colors: Vec<ColorParam>,
    alphas: Vec<AlphaParam>,
}
struct ColorParam {
    id: i32,
    default: f32,
    operation: String,
    values: Vec<[f32; 4]>,
}
struct AlphaParam {
    id: i32,
    default: f32,
    multiply: bool,
    skip_zero: bool,
    domain: f32,
    image: Option<String>,
}

fn colors(e: &Element) -> Vec<ColorParam> {
    e.elements()
        .filter(|p| p.name == "param")
        .filter_map(|p| {
            let node = p.child("param_color")?;
            Some(ColorParam {
                id: p.attr("id")?.parse().ok()?,
                default: number(p, "value_default", 0.0),
                operation: node.attr("operation").unwrap_or("add").into(),
                values: node.elements().filter_map(|v| v.attr("color").map(color)).collect(),
            })
        })
        .collect()
}

impl Definitions {
    fn parse() -> Result<Self, String> {
        let root = xml::parse(AvatarLibrary::file("avatar_lad.xml").ok_or("missing avatar definition")?).map_err(|e| e.to_string())?;
        let mut out = Self {
            regions: Vec::new(),
            globals: HashMap::new(),
            owners: HashMap::new(),
            images: HashMap::new(),
        };
        let mut pending = vec![&root];
        while let Some(e) = pending.pop() {
            if e.name == "param"
                && let Some(id) = e.attr("id").and_then(|id| id.parse().ok())
                && let Some(kind) = e.attr("wearable").and_then(|name| TYPES.iter().position(|kind| *kind == name))
            {
                out.owners.entry(id).or_insert(kind as u8);
            }
            if let Some(file) = e.attr("tga_file")
                && !out.images.contains_key(file)
            {
                let bytes = masks::file(file).ok_or_else(|| format!("missing avatar mask {file}"))?;
                let image = image::load_from_memory_with_format(bytes, image::ImageFormat::Tga).map_err(|e| e.to_string())?;
                let size = [image.width(), image.height()];
                let rgba = if image.color().channel_count() == 1 {
                    image.to_luma8().into_raw().into_iter().flat_map(|a| [255, 255, 255, a]).collect()
                } else {
                    image.into_rgba8().into_raw()
                };
                out.images.insert(file.into(), Arc::new(Image { size, rgba: rgba.into() }));
            }
            pending.extend(e.elements());
        }
        for e in root.elements().filter(|e| e.name == "global_color") {
            out.globals.insert(e.attr("name").unwrap_or_default().into(), colors(e));
        }
        for set in root.elements().filter(|e| e.name == "layer_set") {
            let te = match set.attr("body_region").unwrap_or_default() {
                "head" => 8,
                "upper_body" => 9,
                "lower_body" => 10,
                "eyes" => 11,
                "skirt" => 19,
                "hair" => 20,
                "leftarm" => 40,
                "leftleg" => 41,
                "aux1" => 42,
                "aux2" => 43,
                "aux3" => 44,
                _ => continue,
            };
            let mut region = Region {
                te,
                size: number(set, "width", 512.0) as u32,
                clear_alpha: set.attr("clear_alpha").is_none_or(|s| !s.eq_ignore_ascii_case("false")),
                layers: Vec::new(),
            };
            for e in set
                .elements()
                .filter(|e| e.name == "layer" && e.attr("render_pass") != Some("bump"))
            {
                let mut layer = Layer {
                    local: None,
                    image: None,
                    mask: false,
                    alpha_only: false,
                    visibility: flag(e, "visibility_mask"),
                    replace: flag(e, "write_all_channels"),
                    global: e.attr("global_color").map(Into::into),
                    fixed: e.attr("fixed_color").map(color),
                    colors: colors(e),
                    alphas: Vec::new(),
                };
                for texture in e.elements().filter(|e| e.name == "texture") {
                    if let Some(file) = texture.attr("tga_file") {
                        layer.image = Some(file.into());
                        layer.mask = flag(texture, "file_is_mask");
                    }
                    if let Some(name) = texture.attr("local_texture") {
                        layer.local = TEXTURES.iter().find(|(_, n, _)| *n == name).map(|(slot, _, kind)| (*slot, *kind));
                        layer.alpha_only = flag(texture, "local_texture_alpha_only");
                    }
                }
                for param in e.elements().filter(|e| e.name == "param") {
                    if let Some(a) = param.child("param_alpha")
                        && let Some(id) = param.attr("id").and_then(|s| s.parse().ok())
                    {
                        layer.alphas.push(AlphaParam {
                            id,
                            default: number(param, "value_default", 0.0),
                            multiply: flag(a, "multiply_blend"),
                            skip_zero: flag(a, "skip_if_zero"),
                            domain: number(a, "domain", 0.0),
                            image: a.attr("tga_file").map(Into::into),
                        });
                    }
                }
                region.layers.push(layer);
            }
            out.regions.push(region);
        }
        Ok(out)
    }

    pub fn owned(&self, wearable: &Wearable) -> BTreeMap<i32, f32> {
        wearable
            .params
            .iter()
            .filter(|(id, _)| self.owners.get(id) == Some(&wearable.kind))
            .map(|(id, value)| (*id, *value))
            .collect()
    }
    pub fn affected(&self, region: &Region, kind: u8) -> bool {
        region.layers.iter().any(|layer| {
            layer.local.is_some_and(|(_, k)| k == kind)
                || layer.colors.iter().any(|p| self.owners.get(&p.id) == Some(&kind))
                || layer.alphas.iter().any(|p| self.owners.get(&p.id) == Some(&kind))
                || layer
                    .global
                    .as_ref()
                    .and_then(|name| self.globals.get(name))
                    .is_some_and(|params| params.iter().any(|p| self.owners.get(&p.id) == Some(&kind)))
        })
    }
}
pub fn definitions() -> Result<&'static Definitions, String> {
    static DEFINITIONS: LazyLock<Result<Definitions, String>> = LazyLock::new(Definitions::parse);
    DEFINITIONS.as_ref().map_err(Clone::clone)
}

fn net_color(params: &[ColorParam], weights: &HashMap<i32, f32>, mut value: [f32; 4]) -> [f32; 4] {
    for param in params {
        if param.values.is_empty() {
            continue;
        }
        let weight = weights.get(&param.id).copied().unwrap_or(param.default);
        let index = (weight * (param.values.len() - 1) as f32).clamp(0.0, (param.values.len() - 1) as f32);
        let a = param.values[index.floor() as usize];
        let b = param.values[index.ceil() as usize];
        for channel in 0..4 {
            let c = a[channel] + (b[channel] - a[channel]) * index.fract();
            value[channel] = match param.operation.as_str() {
                "multiply" => value[channel] * c,
                "blend" => value[channel] + (c - value[channel]) * weight,
                _ => value[channel] + c,
            };
        }
    }
    value.map(|v| v.clamp(0.0, 1.0))
}

fn sample(image: &Image, x: u32, y: u32, size: u32) -> [f32; 4] {
    let sx = ((x as f32 + 0.5) * image.size[0] as f32 / size as f32 - 0.5).clamp(0.0, (image.size[0] - 1) as f32);
    let sy = ((y as f32 + 0.5) * image.size[1] as f32 / size as f32 - 0.5).clamp(0.0, (image.size[1] - 1) as f32);
    let (x0, y0) = (sx.floor() as u32, sy.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(image.size[0] - 1), (y0 + 1).min(image.size[1] - 1));
    std::array::from_fn(|c| {
        let get = |x: u32, y: u32| image.rgba[((y * image.size[0] + x) * 4) as usize + c] as f32 / 255.0;
        let a = get(x0, y0) + (get(x1, y0) - get(x0, y0)) * sx.fract();
        let b = get(x0, y1) + (get(x1, y1) - get(x0, y1)) * sx.fract();
        a + (b - a) * sy.fract()
    })
}

fn alpha_table(domain: f32, weight: f32) -> [u8; 256] {
    std::array::from_fn(|i| {
        if domain > 0.0 {
            (255.0 * ((i as f32 / 255.0 - (1.0 - domain) * (1.0 - weight).clamp(0.0, 1.0)) / domain)).clamp(0.0, 255.0) as u8
        } else if i >= (255.0 * (1.0 - weight).clamp(0.0, 1.0)) as usize {
            255
        } else {
            0
        }
    })
}

pub struct Worn {
    pub value: Wearable,
    pub weights: HashMap<i32, f32>,
}
pub fn compose(
    defs: &Definitions,
    region: &Region,
    size: u32,
    worn: &[Worn],
    global: &HashMap<i32, f32>,
    images: &HashMap<Uuid, Arc<Image>>,
) -> Result<Vec<u8>, String> {
    let mut output = vec![0u8; (size * size * 4) as usize];
    // Publishing a bake UUID also enables the system skirt mesh. An empty
    // universal's optional skirt slot must therefore remain transparent;
    // the compositor's usual opaque-black clear would create a black skirt.
    if region.te == 19
        && !worn.iter().any(|wearable| {
            [18, 32].into_iter().any(|slot| {
                wearable
                    .value
                    .textures
                    .get(&slot)
                    .is_some_and(|id| !id.is_nil() && *id != IMG_DEFAULT_AVATAR && *id != IMG_INVISIBLE)
            })
        })
    {
        return Ok(output);
    }
    output.par_chunks_exact_mut(4).for_each(|p| p[3] = 255);
    for layer in region.layers.iter().filter(|layer| !layer.visibility) {
        let instances: Vec<Option<&Worn>> = if let Some((_, kind)) = layer.local {
            worn.iter().filter(|w| w.value.kind == kind).map(Some).collect()
        } else {
            vec![None]
        };
        for instance in instances {
            let weights = instance.map_or(global, |w| &w.weights);
            let local = layer
                .local
                .and_then(|(slot, _)| instance.and_then(|w| w.value.textures.get(&slot)))
                .copied();
            if local.is_some_and(|id| id.is_nil() || id == IMG_DEFAULT_AVATAR) {
                continue;
            }
            let tex = if let Some(id) = local {
                Some(images.get(&id).ok_or_else(|| format!("preview texture unavailable: {id}"))?)
            } else {
                layer.image.as_ref().and_then(|name| defs.images.get(name))
            };
            let global_color = layer
                .global
                .as_ref()
                .and_then(|name| defs.globals.get(name))
                .map(|params| net_color(params, global, [0.0; 4]));
            let base = global_color
                .or(layer.fixed.filter(|c| c[3] != 0.0))
                .unwrap_or(if layer.colors.is_empty() { [1.0; 4] } else { [0.0; 4] });
            let tint = net_color(&layer.colors, weights, base);
            let masks: Vec<_> = layer
                .alphas
                .iter()
                .filter_map(|param| {
                    let weight = weights.get(&param.id).copied().unwrap_or(param.default);
                    if param.skip_zero && weight.abs() < f32::EPSILON {
                        return None;
                    }
                    let image = param.image.as_ref().and_then(|name| defs.images.get(name));
                    Some((param.multiply, image, alpha_table(param.domain, weight), weight))
                })
                .collect();
            let has_alpha = !layer.alphas.is_empty();
            let color_source = tex.is_some() && !layer.alpha_only
                || layer.local.is_none() && (layer.fixed.is_some() || layer.global.is_some() || !layer.colors.is_empty());
            output.par_chunks_exact_mut((size * 4) as usize).enumerate().for_each(|(y, row)| {
                for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    let source = tex.map_or([1.0; 4], |image| sample(image, x as u32, y as u32, size));
                    let mut alpha = if has_alpha {
                        if masks.first().is_some_and(|(multiply, _, _, _)| *multiply) {
                            pixel[3] as f32 / 255.0
                        } else {
                            0.0
                        }
                    } else {
                        source[3] * tint[3]
                    };
                    if has_alpha {
                        for (multiply, image, lut, weight) in &masks {
                            let mask = image.map_or(*weight, |image| {
                                // Process the source mask before bilinear filtering.
                                let processed = sample_lut(image, x as u32, y as u32, size, lut);
                                processed / 255.0
                            });
                            alpha = if *multiply { alpha * mask } else { (alpha + mask).min(1.0) };
                        }
                        alpha *= source[3] * tint[3];
                        pixel[3] = byte(alpha);
                    }
                    if !color_source {
                        continue;
                    }
                    let rgba = if layer.mask {
                        [tint[0], tint[1], tint[2], source[3] * tint[3]]
                    } else {
                        std::array::from_fn(|c| source[c] * tint[c])
                    };
                    if layer.replace {
                        for c in 0..4 {
                            pixel[c] = byte(rgba[c]);
                        }
                    } else {
                        for c in 0..4 {
                            pixel[c] = byte(rgba[c] * alpha + pixel[c] as f32 / 255.0 * (1.0 - alpha));
                        }
                    }
                }
            });
        }
    }
    let visibility: Vec<_> = region.layers.iter().filter(|layer| layer.visibility).collect();
    if region.clear_alpha || !visibility.is_empty() {
        output.par_chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    for layer in visibility {
        let mut masks = Vec::new();
        if let Some((slot, kind)) = layer.local {
            for w in worn.iter().filter(|w| w.value.kind == kind) {
                if let Some(id) = w.value.textures.get(&slot).filter(|id| !id.is_nil() && **id != IMG_DEFAULT_AVATAR) {
                    if *id == IMG_INVISIBLE {
                        output.fill(0);
                        return Ok(output);
                    }
                    masks.push(images.get(id).ok_or_else(|| format!("preview alpha unavailable: {id}"))?.clone());
                }
            }
        } else if let Some(image) = layer.image.as_ref().and_then(|name| defs.images.get(name)) {
            masks.push(image.clone());
        }
        for image in masks {
            output.par_chunks_exact_mut((size * 4) as usize).enumerate().for_each(|(y, row)| {
                for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                    pixel[3] = byte(pixel[3] as f32 / 255.0 * sample(&image, x as u32, y as u32, size)[3]);
                }
            });
        }
    }
    Ok(output)
}

fn sample_lut(image: &Image, x: u32, y: u32, size: u32, lut: &[u8; 256]) -> f32 {
    let sx = ((x as f32 + 0.5) * image.size[0] as f32 / size as f32 - 0.5).clamp(0.0, (image.size[0] - 1) as f32);
    let sy = ((y as f32 + 0.5) * image.size[1] as f32 / size as f32 - 0.5).clamp(0.0, (image.size[1] - 1) as f32);
    let (x0, y0) = (sx.floor() as u32, sy.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(image.size[0] - 1), (y0 + 1).min(image.size[1] - 1));
    let get = |x: u32, y: u32| lut[image.rgba[((y * image.size[0] + x) * 4 + 3) as usize] as usize] as f32;
    let a = get(x0, y0) + (get(x1, y0) - get(x0, y0)) * sx.fract();
    let b = get(x0, y1) + (get(x1, y1) - get(x0, y1)) * sx.fract();
    a + (b - a) * sy.fract()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bundled_masks_cover_every_layer_and_editable_wearable() {
        let defs = definitions().expect("bundled definitions");
        assert_eq!(defs.regions.len(), 11);
        for kind in (1..=14).chain([16]) {
            assert!(defs.regions.iter().any(|r| defs.affected(r, kind)), "kind {kind}");
            let mut actual: Vec<_> = defs
                .regions
                .iter()
                .filter(|region| defs.affected(region, kind))
                .map(|region| region.te)
                .collect();
            actual.sort_unstable();
            let mut expected = super::super::affected_bakes(kind).to_vec();
            expected.sort_unstable();
            assert_eq!(actual, expected, "dependency regions for {kind}");
        }
    }
    #[test]
    fn alpha_domains_match_reference_thresholds_and_processing_order() {
        assert_eq!(alpha_table(0.0, 0.5)[126], 0);
        assert_eq!(alpha_table(0.0, 0.5)[127], 255);
        assert_eq!(alpha_table(0.5, 0.5)[0], 0);
        assert_eq!(alpha_table(0.5, 0.5)[255], 255);
        let image = Image {
            size: [2, 1],
            rgba: vec![255, 255, 255, 0, 255, 255, 255, 255].into(),
        };
        assert!((sample_lut(&image, 1, 0, 3, &alpha_table(0.0, 0.5)) - 127.5).abs() < 0.1);
    }

    #[test]
    fn an_empty_universal_does_not_create_a_visible_system_skirt() {
        let defs = definitions().expect("definitions");
        let region = defs.regions.iter().find(|region| region.te == 19).expect("skirt region");
        let asset = crate::world::inventory::wearable::create(16, "Empty universal", Uuid::nil()).expect("asset");
        let value = Wearable::parse(&asset).expect("wearable");
        let pixels = compose(
            defs,
            region,
            8,
            &[Worn {
                value,
                weights: HashMap::new(),
            }],
            &HashMap::new(),
            &HashMap::new(),
        )
        .expect("empty skirt");
        assert!(pixels.iter().all(|channel| *channel == 0));
    }

    fn solid(rgba: [u8; 4]) -> Arc<Image> {
        Arc::new(Image {
            size: [1, 1],
            rgba: rgba.to_vec().into(),
        })
    }

    #[test]
    fn composite_preserves_layer_order_tint_and_visibility_without_touching_assets() {
        let defs = definitions().expect("definitions");
        let region = defs.regions.iter().find(|region| region.te == 42).expect("aux1 region");
        let red = Uuid::from_u128(101);
        let blue = Uuid::from_u128(102);
        let asset = crate::world::inventory::wearable::create(16, "Layers", Uuid::nil()).expect("wearable");
        let mut first = Wearable::parse(&asset).expect("first");
        first.textures.insert(37, red);
        let mut second = first.clone();
        second.textures.insert(37, blue);
        let library = crate::wearable_editor::tests::fixture(16).0.avatar_lib;
        let weights = library
            .shape_params
            .edited_weights(&[], &BTreeMap::from([(1238, 1.0), (1239, 1.0), (1240, 1.0)]));
        let worn = vec![
            Worn {
                value: first.clone(),
                weights: weights.clone(),
            },
            Worn {
                value: second.clone(),
                weights: weights.clone(),
            },
        ];
        let images = HashMap::from([(red, solid([255, 0, 0, 255])), (blue, solid([0, 0, 255, 255]))]);
        let pixels = compose(defs, region, 3, &worn, &weights, &images).expect("ordered layers");
        assert!(pixels.as_chunks::<4>().0.iter().all(|pixel| *pixel == [0, 0, 255, 255]));
        let mut tinted = worn;
        tinted[1].weights = library
            .shape_params
            .edited_weights(&[], &BTreeMap::from([(1238, 1.0), (1239, 1.0), (1240, 0.5)]));
        let pixels = compose(defs, region, 3, &tinted, &weights, &images).expect("tint");
        assert_eq!(&pixels[..3], &[0, 0, 128]);
        assert_eq!(tinted[0].value.textures, first.textures);
        assert_eq!(tinted[1].value.textures, second.textures);
        let head = defs.regions.iter().find(|region| region.te == 8).expect("head region");
        let asset = crate::world::inventory::wearable::create(13, "Hidden", Uuid::nil()).expect("alpha");
        let mut alpha = Wearable::parse(&asset).expect("alpha asset");
        alpha.textures.insert(23, IMG_INVISIBLE);
        let pixels = compose(
            defs,
            head,
            3,
            &[Worn {
                value: alpha,
                weights: HashMap::new(),
            }],
            &HashMap::new(),
            &images,
        )
        .expect("invisible alpha");
        assert!(pixels.iter().all(|value| *value == 0));
    }
}
