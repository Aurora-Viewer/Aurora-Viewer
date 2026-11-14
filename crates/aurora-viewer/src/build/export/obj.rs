//! The Wavefront OBJ and MTL files of a model. Aurora's own: the Legacy
//! Viewer writes Collada only. The same model, rights and options as the
//! Collada export (`collada.rs`), in the form OBJ readers expect:
//!
//! - one `o` per prim, its vertices already placed (OBJ has no nodes), from
//!   the first root as in the Collada file;
//! - Y up (OBJ's usual axes: the importers of Blender and the others turn
//!   it back to their own), where the grid and the Collada file are Z up:
//!   (x, y, z) becomes (x, z, -y);
//! - a `usemtl` per material list, the materials in the `.mtl` beside it:
//!   the face colour (`Kd`), its alpha (`d`) and, with the textures, the
//!   texture file (`map_Kd`, and `map_d` for its alpha).

use super::model::Model;
use super::{Settings, TextureFiles};
use glam::{Mat3, Vec3};
use std::fmt::Write;

/// SL (Z up) to OBJ (Y up); `+ 0.0` writes no "-0".
fn y_up(v: Vec3) -> [f32; 3] {
    [v.x + 0.0, v.z + 0.0, -v.y + 0.0]
}

/// The `.obj` text; `mtl` is the material file's name beside it.
pub fn write_obj(model: &Model, mtl: &str, tool: &str) -> String {
    let mut x = String::with_capacity(1024 + model.prims.iter().map(|p| p.positions.len() * 120).sum::<usize>());
    let _ = writeln!(x, "# {tool}");
    let _ = writeln!(x, "# {} prims, {} triangles", model.prims.len(), model.triangles());
    let _ = writeln!(x, "mtllib {mtl}");
    // OBJ numbers its vertices from 1, across the whole file
    let mut base = 1usize;
    for prim in &model.prims {
        let _ = writeln!(x, "o {}", prim.name);
        let m = prim.transform;
        let normal_matrix = Mat3::from_mat4(m).inverse().transpose();
        for p in &prim.positions {
            let [a, b, c] = y_up(m.transform_point3(Vec3::from(*p)));
            let _ = writeln!(x, "v {a} {b} {c}");
        }
        for uv in &prim.uvs {
            let _ = writeln!(x, "vt {} {}", uv[0], uv[1]);
        }
        for n in &prim.normals {
            let n = (normal_matrix * Vec3::from(*n)).normalize_or(Vec3::Z);
            let [a, b, c] = y_up(n);
            let _ = writeln!(x, "vn {a} {b} {c}");
        }
        for group in &prim.groups {
            let _ = writeln!(x, "usemtl {}", model.materials[group.material].name);
            for t in group.indices.as_chunks::<3>().0 {
                let [i, j, k] = t.map(|i| i as usize + base);
                let _ = writeln!(x, "f {i}/{i}/{i} {j}/{j}/{j} {k}/{k}/{k}");
            }
        }
        base += prim.positions.len();
    }
    x
}

/// The `.mtl` text.
pub fn write_mtl(model: &Model, textures: &TextureFiles, settings: &Settings, tool: &str) -> String {
    let mut x = String::new();
    let _ = writeln!(x, "# {tool}");
    for m in &model.materials {
        let [r, g, b, a] = m.color;
        let _ = writeln!(x, "\nnewmtl {}", m.name);
        let _ = writeln!(x, "Ka 0 0 0");
        let _ = writeln!(x, "Ks 0 0 0");
        let _ = writeln!(x, "illum 1");
        if let Some(name) = textures.names.get(&m.texture).filter(|_| settings.textures) {
            // a texture replaces the colour, as in the Collada file
            let file = format!("{name}.{}", textures.extension);
            let _ = writeln!(x, "Kd 1 1 1");
            let _ = writeln!(x, "map_Kd {file}");
            if textures.extension != "j2c" {
                let _ = writeln!(x, "map_d {file}");
            }
        } else {
            let _ = writeln!(x, "Kd {r:.6} {g:.6} {b:.6}");
            let _ = writeln!(x, "d {a:.6}");
        }
    }
    x
}

#[cfg(test)]
mod tests {
    use super::super::model::{SourceFace, SourcePrim, build};
    use super::*;
    use aurora_prim::te::TextureFace;
    use glam::Quat;
    use uuid::Uuid;

    fn model() -> Model {
        let face = SourceFace {
            positions: vec![[0.0, 0.0, 0.0], [0.5, 0.0, 0.0], [0.0, 0.5, 0.0]],
            normals: vec![[0.0, 1.0, 0.0]; 3],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
            indices: vec![0, 1, 2],
            te: TextureFace {
                texture: Uuid::from_u128(1),
                color: [1.0, 0.0, 0.0, 0.5],
                ..Default::default()
            },
        };
        let prim = SourcePrim {
            position: Vec3::new(100.0, 100.0, 20.0),
            rotation: Quat::IDENTITY,
            scale: Vec3::new(2.0, 2.0, 2.0),
            faces: vec![face],
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

    #[test]
    fn vertices_are_placed_from_the_root_y_up_and_numbered_across_prims() {
        let text = write_obj(&model(), "chair.mtl", "Aurora Viewer");
        assert!(text.contains("mtllib chair.mtl"));
        let lines: Vec<&str> = text.lines().collect();
        let v: Vec<&str> = lines.iter().filter(|l| l.starts_with("v ")).copied().collect();
        assert_eq!(v.len(), 6);
        assert_eq!(v[1], "v 1 0 0", "x 0.5 scaled by 2");
        assert_eq!(v[3], "v 0 0 -3", "the second prim, 3 m north: -Z in Y up");
        assert_eq!(v[5], "v 0 0 -4");
        assert!(lines.contains(&"vn 0 0 -1"), "a normal facing north");
        let f: Vec<&str> = lines.iter().filter(|l| l.starts_with("f ")).copied().collect();
        assert_eq!(f, ["f 1/1/1 2/2/2 3/3/3", "f 4/4/4 5/5/5 6/6/6"]);
        assert_eq!(lines.iter().filter(|l| l.starts_with("usemtl Material0")).count(), 2);
    }

    #[test]
    fn materials_carry_their_colour_or_their_texture() {
        let mut files = TextureFiles {
            extension: "png".into(),
            ..Default::default()
        };
        let plain = write_mtl(&model(), &files, &Settings::default(), "Aurora");
        assert!(plain.contains("newmtl Material0\n"));
        assert!(plain.contains("Kd 1.000000 0.000000 0.000000\nd 0.500000"));
        files.names.insert(Uuid::from_u128(1), "Brick".into());
        let textured = write_mtl(&model(), &files, &Settings::default(), "Aurora");
        assert!(textured.contains("map_Kd Brick.png\nmap_d Brick.png"));
        let off = Settings {
            textures: false,
            ..Settings::default()
        };
        assert!(!write_mtl(&model(), &files, &off, "Aurora").contains("map_Kd"));
    }
}
