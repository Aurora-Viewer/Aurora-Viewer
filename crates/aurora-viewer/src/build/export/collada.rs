//! The Collada 1.4.1 document of a model: DAESaver::saveDAE, addSource,
//! addPolygons, generateEffects and generateImagesSection
//! (indra/newview/daeexport.cpp, Latif Khalifa and Cinder Biscuits, LGPL
//! 2.1), written as text instead of through the Collada DOM. The same
//! elements, ids and options: Z up in metres, one node per prim with its
//! matrix, a `<polylist>` per material, Phong effects with the face colour
//! or its texture.

use super::model::{Model, Prim};
use super::{Settings, TextureFiles};
use std::fmt::Write;

/// The `<asset>` facts.
pub struct Header {
    pub author: String,
    pub tool: String,
    /// ISO 8601, UTC, without a zone (as the Legacy Viewer writes it).
    pub date: String,
}

pub fn write(model: &Model, textures: &TextureFiles, settings: &Settings, header: &Header) -> String {
    let mut x = String::with_capacity(4096 + model.prims.iter().map(|p| p.positions.len() * 96).sum::<usize>());
    x.push_str("<?xml version=\"1.0\" encoding=\"utf-8\"?>\n");
    x.push_str("<COLLADA xmlns=\"http://www.collada.org/2005/11/COLLADASchema\" version=\"1.4.1\">\n");
    let _ = write!(
        x,
        "  <asset>\n    <contributor>\n      <author>{}</author>\n      <authoring_tool>{}</authoring_tool>\n    </contributor>\n    <created>{}</created>\n    <modified>{}</modified>\n    <unit name=\"meter\" meter=\"1\"/>\n    <up_axis>Z_UP</up_axis>\n  </asset>\n",
        escape(&header.author),
        escape(&header.tool),
        escape(&header.date),
        escape(&header.date)
    );

    // generateImagesSection
    let export_textures = settings.textures && !textures.names.is_empty();
    if export_textures {
        x.push_str("  <library_images>\n");
        for name in textures.sorted_names() {
            let id = image_id(name, textures);
            let _ = writeln!(
                x,
                "    <image id=\"{id}\" name=\"{id}\">\n      <init_from>{}</init_from>\n    </image>",
                escape(&uri_escape(&format!("{name}.{}", textures.extension)))
            );
        }
        x.push_str("  </library_images>\n");
    }

    x.push_str("  <library_geometries>\n");
    for prim in &model.prims {
        geometry(&mut x, prim, model, settings);
    }
    x.push_str("  </library_geometries>\n");

    // generateEffects
    x.push_str("  <library_effects>\n");
    for m in &model.materials {
        let _ = writeln!(x, "    <effect id=\"{}-fx\">\n      <profile_COMMON>", escape(&m.name));
        let texture = textures.names.get(&m.texture).filter(|_| export_textures);
        if let Some(name) = texture {
            let id = image_id(name, textures);
            let _ = writeln!(
                x,
                "        <newparam sid=\"{id}-surface\">\n          <surface type=\"2D\">\n            <init_from>{id}</init_from>\n          </surface>\n        </newparam>\n        <newparam sid=\"{id}-sampler\">\n          <sampler2D>\n            <source>{id}-surface</source>\n          </sampler2D>\n        </newparam>"
            );
        }
        x.push_str("        <technique sid=\"common\">\n          <phong>\n            <diffuse>\n");
        match texture {
            Some(name) => {
                let id = image_id(name, textures);
                let _ = writeln!(x, "              <texture texture=\"{id}-sampler\" texcoord=\"{id}\"/>");
                x.push_str("            </diffuse>\n");
            }
            None => {
                let [r, g, b, a] = m.color;
                let _ = writeln!(x, "              <color sid=\"diffuse\">{r:.6} {g:.6} {b:.6} {a:.6}</color>");
                x.push_str("            </diffuse>\n");
                let _ = writeln!(
                    x,
                    "            <transparency>\n              <float>{a:.6}</float>\n            </transparency>"
                );
            }
        }
        x.push_str("          </phong>\n        </technique>\n      </profile_COMMON>\n    </effect>\n");
    }
    x.push_str("  </library_effects>\n");

    x.push_str("  <library_materials>\n");
    for m in &model.materials {
        let name = escape(&m.name);
        let _ = writeln!(
            x,
            "    <material id=\"{name}-material\">\n      <instance_effect url=\"#{name}-fx\"/>\n    </material>"
        );
    }
    x.push_str("  </library_materials>\n");

    x.push_str("  <library_visual_scenes>\n    <visual_scene id=\"Scene\" name=\"Scene\">\n");
    for prim in &model.prims {
        let id = &prim.name;
        let _ = writeln!(x, "      <node type=\"NODE\" id=\"{id}\" name=\"{id}\">");
        // row by row, as LLMatrix4 transposed
        let m = prim.transform;
        let values: Vec<String> = (0..4)
            .flat_map(|row| (0..4).map(move |col| m.col(col)[row]))
            .map(|v| format!("{v}"))
            .collect();
        let _ = writeln!(x, "        <matrix>{}</matrix>", values.join(" "));
        let _ = writeln!(
            x,
            "        <instance_geometry url=\"#{id}-mesh\">\n          <bind_material>\n            <technique_common>"
        );
        for material in materials_of(prim) {
            let name = escape(&model.materials[material].name);
            let _ = writeln!(
                x,
                "              <instance_material symbol=\"{name}-material\" target=\"#{name}-material\"/>"
            );
        }
        x.push_str("            </technique_common>\n          </bind_material>\n        </instance_geometry>\n      </node>\n");
    }
    x.push_str("    </visual_scene>\n  </library_visual_scenes>\n");
    x.push_str("  <scene>\n    <instance_visual_scene url=\"#Scene\"/>\n  </scene>\n</COLLADA>\n");
    x
}

/// The id of a texture's `<image>`: its file name and format.
fn image_id(name: &str, textures: &TextureFiles) -> String {
    escape(&format!("{name}_{}", textures.extension))
}

/// The materials a prim's lists use, each once.
fn materials_of(prim: &Prim) -> Vec<usize> {
    let mut out = Vec::new();
    for g in &prim.groups {
        if !out.contains(&g.material) {
            out.push(g.material);
        }
    }
    out
}

fn geometry(x: &mut String, prim: &Prim, model: &Model, settings: &Settings) {
    let id = &prim.name;
    let _ = writeln!(x, "    <geometry id=\"{id}-mesh\">\n      <mesh>");
    let flat3 = |v: &[[f32; 3]]| v.iter().flatten().copied().collect::<Vec<f32>>();
    source(x, &format!("{id}-positions"), "XYZ", &flat3(&prim.positions));
    source(x, &format!("{id}-normals"), "XYZ", &flat3(&prim.normals));
    // "Single UV map": every prim's map under one name, for tools that
    // join the prims afterwards
    let map = if settings.single_uv_map {
        "unified-map0".to_owned()
    } else {
        format!("{id}-map0")
    };
    source(x, &map, "ST", &prim.uvs.iter().flatten().copied().collect::<Vec<f32>>());
    let _ = writeln!(
        x,
        "        <vertices id=\"{id}-vertices\">\n          <input semantic=\"POSITION\" source=\"#{id}-positions\"/>\n        </vertices>"
    );
    for group in &prim.groups {
        let material = escape(&model.materials[group.material].name);
        let count = group.indices.len() / 3;
        let _ = writeln!(x, "        <polylist material=\"{material}-material\" count=\"{count}\">");
        let _ = writeln!(x, "          <input semantic=\"VERTEX\" source=\"#{id}-vertices\" offset=\"0\"/>");
        let _ = writeln!(x, "          <input semantic=\"NORMAL\" source=\"#{id}-normals\" offset=\"0\"/>");
        let _ = writeln!(x, "          <input semantic=\"TEXCOORD\" source=\"#{map}\" offset=\"0\"/>");
        x.push_str("          <vcount>");
        x.push_str(&vec!["3"; count].join(" "));
        x.push_str("</vcount>\n          <p>");
        let indices: Vec<String> = group.indices.iter().map(u32::to_string).collect();
        x.push_str(&indices.join(" "));
        x.push_str("</p>\n        </polylist>\n");
    }
    x.push_str("      </mesh>\n    </geometry>\n");
}

/// addSource: a float array and its accessor.
fn source(x: &mut String, id: &str, params: &str, values: &[f32]) {
    let stride = params.len();
    let numbers: Vec<String> = values.iter().map(|v| format!("{v}")).collect();
    let _ = writeln!(
        x,
        "        <source id=\"{id}\">\n          <float_array id=\"{id}-array\" count=\"{}\">{}</float_array>\n          <technique_common>\n            <accessor source=\"#{id}-array\" count=\"{}\" stride=\"{stride}\">",
        values.len(),
        numbers.join(" "),
        values.len() / stride.max(1)
    );
    for p in params.chars() {
        let _ = writeln!(x, "              <param name=\"{p}\" type=\"float\"/>");
    }
    x.push_str("            </accessor>\n          </technique_common>\n        </source>\n");
}

/// XML text and attribute escaping.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

/// LLURI::escape of an image file name (cdom::nativePathToUri): every
/// byte but the unreserved ones in `%XX`.
pub fn uri_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::model::{SourceFace, SourcePrim, build};
    use super::*;
    use aurora_prim::te::TextureFace;
    use glam::{Quat, Vec3};
    use uuid::Uuid;

    fn model(settings: &Settings) -> Model {
        let face = |n: u128| SourceFace {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
            te: TextureFace {
                texture: Uuid::from_u128(n),
                color: [1.0, 0.5, 0.25, 1.0],
                ..Default::default()
            },
        };
        let prim = SourcePrim {
            position: Vec3::new(1.0, 2.0, 3.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            faces: vec![face(1), face(2)],
        };
        build(
            &[
                prim.clone(),
                SourcePrim {
                    position: Vec3::new(2.0, 2.0, 3.0),
                    ..prim
                },
            ],
            settings,
        )
    }

    fn header() -> Header {
        Header {
            author: "Ann & Bob".into(),
            tool: "Aurora Viewer 1.0".into(),
            date: "2026-11-14T10:00:00".into(),
        }
    }

    fn files() -> TextureFiles {
        let mut files = TextureFiles {
            extension: "png".into(),
            ..Default::default()
        };
        files.names.insert(Uuid::from_u128(1), "My wood".into());
        files
    }

    #[test]
    fn the_document_parses_and_references_what_it_defines() {
        let settings = Settings::default();
        let text = write(&model(&settings), &files(), &settings, &header());
        let doc = roxmltree::Document::parse(&text).expect("well-formed Collada");
        let ids: std::collections::HashSet<&str> = doc.descendants().filter_map(|n| n.attribute("id")).collect();
        for n in doc.descendants() {
            for attr in ["url", "source", "target"] {
                if let Some(r) = n.attribute(attr) {
                    assert!(ids.contains(r.trim_start_matches('#')), "{attr}={r} defined");
                }
            }
        }
        assert!(text.contains("<author>Ann &amp; Bob</author>"));
        assert!(text.contains("<up_axis>Z_UP</up_axis>"));
        assert!(text.contains("<init_from>My%20wood.png</init_from>"), "file names are URIs");
        assert!(text.contains("<texture texture=\"My wood_png-sampler\""));
        assert!(
            text.contains("<color sid=\"diffuse\">1.000000 0.500000 0.250000 1.000000</color>"),
            "the untextured face keeps its colour"
        );
        // second prim one metre east of the first root
        assert!(text.contains("<matrix>1 0 0 1 0 1 0 0 0 0 1 0 0 0 0 1</matrix>"));
        assert_eq!(doc.descendants().filter(|n| n.has_tag_name("polylist")).count(), 4);
        assert_eq!(
            doc.descendants()
                .filter(|n| n.has_tag_name("source") && n.attribute("id") == Some("unified-map0"))
                .count(),
            2,
            "the single UV map, as the Legacy Viewer names it"
        );
    }

    #[test]
    fn without_textures_every_material_is_a_colour() {
        let settings = Settings {
            textures: false,
            single_uv_map: false,
            ..Settings::default()
        };
        let text = write(&model(&settings), &files(), &settings, &header());
        roxmltree::Document::parse(&text).expect("well-formed");
        assert!(!text.contains("library_images"));
        assert!(!text.contains("<texture "));
        assert!(text.contains("prim1-map0"));
    }

    #[test]
    fn escapes() {
        assert_eq!(escape("a<b>\"c\"&'d'\u{1}"), "a&lt;b&gt;&quot;c&quot;&amp;&apos;d&apos;");
        assert_eq!(uri_escape("Wood (old)é.png"), "Wood%20%28old%29%C3%A9.png");
    }
}
