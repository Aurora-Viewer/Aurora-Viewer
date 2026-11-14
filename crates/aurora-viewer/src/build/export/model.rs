//! The model an export writes: each prim's faces gathered into one vertex
//! list, its transform from the first root, and the materials of its faces.
//! Port of DAESaver::saveDAE, transformTexCoord, skipFace, getMaterial(s)
//! and getFacesWithMaterial (indra/newview/daeexport.cpp, Latif Khalifa and
//! Cinder Biscuits, LGPL 2.1); the writers (`collada.rs`, `obj.rs`) only
//! format it.

use super::Settings;
use aurora_assets::mesh::SkinInfo;
use aurora_prim::te::{TRANSPARENT_TEXTURE, TextureFace};
use glam::{Mat4, Quat, Vec2, Vec3};
use std::sync::Arc;
use uuid::Uuid;

/// One face of a prim as its volume or mesh gives it: unit-scale positions
/// (the object's scale is in the prim's transform), in texture-entry order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SourceFace {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
    pub te: TextureFace,
    /// A rigged mesh's per-vertex joints (into its skin's joint names) and
    /// weights; empty otherwise.
    pub joints: Vec<[u8; 4]>,
    pub weights: Vec<[f32; 4]>,
}

/// A prim to write.
#[derive(Debug, Clone, PartialEq)]
pub struct SourcePrim {
    /// Render position, rotation and scale (getRenderPosition,
    /// getRenderRotation, getScale).
    pub position: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub faces: Vec<SourceFace>,
    /// A rigged mesh's skin (LLMeshSkinInfo).
    pub skin: Option<Arc<SkinInfo>>,
}

/// One material of the file (MaterialInfo): a face's texture and colour.
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    /// "Material<n>".
    pub name: String,
    pub texture: Uuid,
    /// The face's colour and alpha, as the texture entry holds them.
    pub color: [f32; 4],
}

/// The triangles of a prim drawn with one material (a `<polylist>`).
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub material: usize,
    /// Into the prim's vertex lists.
    pub indices: Vec<u32>,
}

/// A prim's geometry, ready to write.
#[derive(Debug, Clone, PartialEq)]
pub struct Prim {
    /// "prim<n>".
    pub name: String,
    /// Scale, rotation, then the position from the first root.
    pub transform: Mat4,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub groups: Vec<Group>,
    /// A rigged prim: its skin, and each vertex's joints and weights. The
    /// formats that cannot carry rigging leave them out.
    pub skin: Option<Arc<SkinInfo>>,
    pub joints: Vec<[u8; 4]>,
    pub weights: Vec<[f32; 4]>,
}

/// A skin usable for a file: one inverse bind matrix per joint name.
pub fn usable_skin(skin: &Option<Arc<SkinInfo>>) -> Option<&Arc<SkinInfo>> {
    skin.as_ref()
        .filter(|s| !s.joint_names.is_empty() && s.inverse_bind.len() == s.joint_names.len())
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub prims: Vec<Prim>,
    pub materials: Vec<Material>,
}

impl Model {
    pub fn triangles(&self) -> usize {
        self.prims.iter().flat_map(|p| &p.groups).map(|g| g.indices.len() / 3).sum()
    }
}

/// skipFace: with "Skip transparent", a face fully transparent or bearing
/// the transparent texture.
pub fn skip_face(settings: &Settings, te: &TextureFace) -> bool {
    settings.skip_transparent && (te.color[3] < 0.01 || te.texture == TRANSPARENT_TEXTURE)
}

/// Gather `prims` (the first is the root the file is centred on).
pub fn build(prims: &[SourcePrim], settings: &Settings) -> Model {
    let mut model = Model::default();
    let offset = prims.first().map_or(Vec3::ZERO, |p| -p.position);
    for (n, prim) in prims.iter().enumerate() {
        let mut out = Prim {
            name: format!("prim{n}"),
            transform: Mat4::from_scale_rotation_translation(prim.scale, prim.rotation, prim.position + offset),
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            groups: Vec::new(),
            skin: usable_skin(&prim.skin).cloned(),
            joints: Vec::new(),
            weights: Vec::new(),
        };
        let rigged = out.skin.is_some();
        // (face's material, its indices into the prim's lists)
        let mut faces: Vec<(usize, Vec<u32>)> = Vec::new();
        for face in &prim.faces {
            if skip_face(settings, &face.te) || face.positions.is_empty() {
                continue;
            }
            let base = out.positions.len() as u32;
            let count = face.positions.len();
            out.positions.extend_from_slice(&face.positions);
            out.normals
                .extend((0..count).map(|i| face.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0])));
            let uv = |i: usize| face.uvs.get(i).copied().unwrap_or([0.0, 0.0]);
            if settings.texture_params {
                out.uvs.extend((0..count).map(|i| {
                    let normal = face.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]);
                    transform_tex_coord(uv(i), face.positions[i], normal, &face.te, prim.scale)
                }));
            } else {
                out.uvs.extend((0..count).map(uv));
            }
            if rigged {
                // a vertex without weights follows the first joint whole
                out.joints.extend((0..count).map(|i| face.joints.get(i).copied().unwrap_or([0; 4])));
                out.weights
                    .extend((0..count).map(|i| face.weights.get(i).copied().unwrap_or([1.0, 0.0, 0.0, 0.0])));
            }
            let indices = face
                .indices
                .iter()
                .filter(|&&i| (i as usize) < count)
                .map(|&i| base + i)
                .collect::<Vec<_>>();
            // whole triangles only
            let indices = indices[..indices.len() / 3 * 3].to_vec();
            faces.push((material(&mut model.materials, &face.te, settings), indices));
        }
        if settings.consolidate {
            // one list per material, in the order its first face came
            for (material, indices) in faces {
                match out.groups.iter_mut().find(|g| g.material == material) {
                    Some(group) => group.indices.extend(indices),
                    None => out.groups.push(Group { material, indices }),
                }
            }
        } else {
            out.groups
                .extend(faces.into_iter().map(|(material, indices)| Group { material, indices }));
        }
        model.prims.push(out);
    }
    model
}

/// getMaterial: with "Consolidate textures", faces of the same texture and
/// colour share one material; without, each face has its own.
fn material(materials: &mut Vec<Material>, te: &TextureFace, settings: &Settings) -> usize {
    if settings.consolidate
        && let Some(n) = materials.iter().position(|m| m.texture == te.texture && m.color == te.color)
    {
        return n;
    }
    materials.push(Material {
        name: format!("Material{}", materials.len()),
        texture: te.texture,
        color: te.color,
    });
    materials.len() - 1
}

/// transformTexCoord: the face's repeats, offset and rotation applied to
/// its texture coordinates ("Apply texture params"), planar mapping
/// computed from the position as the renderer does.
pub fn transform_tex_coord(uv: [f32; 2], position: [f32; 3], normal: [f32; 3], te: &TextureFace, scale: Vec3) -> [f32; 2] {
    let mut coord = Vec2::from(uv);
    if te.planar() {
        let normal = Vec3::from(normal);
        let d = normal.dot(Vec3::X);
        let binormal = if d >= 0.5 || d <= -0.5 {
            if normal.x < 0.0 { -Vec3::Y } else { Vec3::Y }
        } else if normal.y > 0.0 {
            -Vec3::X
        } else {
            Vec3::X
        };
        let tangent = binormal.cross(normal);
        let scaled = Vec3::from(position) * scale;
        coord.x = 1.0 + (binormal.dot(scaled) * 2.0 - 0.5);
        coord.y = -(tangent.dot(scaled) * 2.0 - 0.5);
    }
    let (sin, cos) = te.rotation.sin_cos();
    let t = coord - Vec2::splat(0.5);
    [
        (t.x * cos + t.y * sin) * te.scale_s + te.offset_s + 0.5,
        (-t.x * sin + t.y * cos) * te.scale_t + te.offset_t + 0.5,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(te: TextureFace) -> SourceFace {
        SourceFace {
            positions: vec![[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.5, 0.5, 0.0], [-0.5, 0.5, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            uvs: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
            indices: vec![0, 1, 2, 0, 2, 3],
            te,
            ..Default::default()
        }
    }

    fn prim(position: Vec3, faces: Vec<SourceFace>) -> SourcePrim {
        SourcePrim {
            position,
            rotation: Quat::IDENTITY,
            scale: Vec3::new(2.0, 1.0, 1.0),
            faces,
            skin: None,
        }
    }

    fn tex(n: u128, color: [f32; 4]) -> TextureFace {
        TextureFace {
            texture: Uuid::from_u128(n),
            color,
            ..Default::default()
        }
    }

    #[test]
    fn prims_are_placed_from_the_first_root_with_their_scale() {
        let prims = [
            prim(Vec3::new(10.0, 20.0, 30.0), vec![quad(tex(1, [1.0; 4]))]),
            prim(Vec3::new(11.0, 20.0, 30.0), vec![quad(tex(1, [1.0; 4]))]),
        ];
        let model = build(&prims, &Settings::default());
        assert_eq!(model.prims.len(), 2);
        assert_eq!(model.prims[0].name, "prim0");
        assert_eq!(model.prims[0].transform.transform_point3(Vec3::ZERO), Vec3::ZERO);
        assert_eq!(
            model.prims[1].transform.transform_point3(Vec3::new(0.5, 0.0, 0.0)),
            Vec3::new(2.0, 0.0, 0.0)
        );
        assert_eq!(model.triangles(), 4);
    }

    #[test]
    fn consolidate_shares_materials_of_the_same_texture_and_colour() {
        let faces = vec![
            quad(tex(1, [1.0; 4])),
            quad(tex(2, [1.0; 4])),
            quad(tex(1, [1.0; 4])),
            quad(tex(1, [1.0, 0.0, 0.0, 1.0])),
        ];
        let prims = [prim(Vec3::ZERO, faces.clone()), prim(Vec3::ZERO, faces.clone())];
        let model = build(&prims, &Settings::default());
        assert_eq!(model.materials.len(), 3, "texture 1 white, texture 2, texture 1 red");
        assert_eq!(model.prims[0].groups.len(), 3);
        assert_eq!(model.prims[0].groups[0].indices, vec![0, 1, 2, 0, 2, 3, 8, 9, 10, 8, 10, 11]);
        assert_eq!(model.prims[1].groups[0].material, 0, "shared across prims");

        let apart = Settings {
            consolidate: false,
            ..Settings::default()
        };
        let model = build(&prims, &apart);
        assert_eq!(model.materials.len(), 8, "one per face");
        assert_eq!(model.materials[7].name, "Material7");
        assert_eq!(model.prims[1].groups[0].indices, vec![0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn skip_transparent_drops_invisible_faces_and_their_vertices() {
        let faces = vec![
            quad(tex(1, [1.0, 1.0, 1.0, 0.0])),
            quad(TextureFace {
                texture: TRANSPARENT_TEXTURE,
                ..Default::default()
            }),
            quad(tex(2, [1.0; 4])),
        ];
        let model = build(&[prim(Vec3::ZERO, faces.clone())], &Settings::default());
        assert_eq!(model.prims[0].positions.len(), 4);
        assert_eq!(model.prims[0].groups[0].indices, vec![0, 1, 2, 0, 2, 3]);
        assert_eq!(model.materials.len(), 1);

        let keep = Settings {
            skip_transparent: false,
            ..Settings::default()
        };
        assert_eq!(build(&[prim(Vec3::ZERO, faces)], &keep).prims[0].positions.len(), 12);
    }

    #[test]
    fn broken_indices_never_reach_the_file() {
        let mut face = quad(tex(1, [1.0; 4]));
        face.indices = vec![0, 1, 2, 0, 9, 3, 1];
        let model = build(&[prim(Vec3::ZERO, vec![face])], &Settings::default());
        assert_eq!(model.prims[0].groups[0].indices, vec![0, 1, 2, 0, 3, 1]);
    }

    #[test]
    fn a_rigged_prim_keeps_its_skin_and_each_vertex_weights() {
        let skin = Arc::new(SkinInfo {
            joint_names: vec!["mPelvis".into(), "mChest".into()],
            inverse_bind: vec![Mat4::IDENTITY; 2],
            ..Default::default()
        });
        let mut weighted = quad(tex(1, [1.0; 4]));
        weighted.joints = vec![[1, 0, 0, 0]; 4];
        weighted.weights = vec![[0.75, 0.25, 0.0, 0.0]; 4];
        let mut rigged = prim(Vec3::ZERO, vec![weighted, quad(tex(1, [1.0; 4]))]);
        rigged.skin = Some(skin.clone());
        let model = build(&[rigged.clone()], &Settings::default());
        let p = &model.prims[0];
        assert_eq!(p.skin.as_ref(), Some(&skin));
        assert_eq!(p.joints.len(), 8);
        assert_eq!(p.weights[0], [0.75, 0.25, 0.0, 0.0]);
        assert_eq!(
            (p.joints[4], p.weights[4]),
            ([0; 4], [1.0, 0.0, 0.0, 0.0]),
            "no weights: the first joint"
        );
        // a skin whose matrices do not match its joints is left out
        rigged.skin = Some(Arc::new(SkinInfo {
            joint_names: vec!["mPelvis".into()],
            ..Default::default()
        }));
        let model = build(&[rigged], &Settings::default());
        assert!(model.prims[0].skin.is_none() && model.prims[0].joints.is_empty());
        assert!(
            build(&[prim(Vec3::ZERO, vec![quad(tex(1, [1.0; 4]))])], &Settings::default()).prims[0]
                .skin
                .is_none()
        );
    }

    #[test]
    fn texture_params_apply_repeats_offset_and_rotation() {
        let te = TextureFace {
            scale_s: 2.0,
            scale_t: 3.0,
            offset_s: 0.25,
            offset_t: -0.5,
            ..Default::default()
        };
        let out = transform_tex_coord([1.0, 1.0], [0.0; 3], [0.0, 0.0, 1.0], &te, Vec3::ONE);
        assert_eq!(out, [0.5 * 2.0 + 0.25 + 0.5, 0.5 * 3.0 - 0.5 + 0.5]);
        let turned = TextureFace {
            rotation: std::f32::consts::FRAC_PI_2,
            ..Default::default()
        };
        let out = transform_tex_coord([1.0, 0.5], [0.0; 3], [0.0, 0.0, 1.0], &turned, Vec3::ONE);
        assert!((out[0] - 0.5).abs() < 1e-6 && (out[1] - 0.0).abs() < 1e-6, "{out:?}");

        let model = build(
            &[prim(Vec3::ZERO, vec![quad(te)])],
            &Settings {
                texture_params: false,
                ..Settings::default()
            },
        );
        assert_eq!(model.prims[0].uvs[2], [1.0, 1.0], "raw coordinates without the option");
    }

    #[test]
    fn planar_faces_map_from_their_position() {
        let te = TextureFace {
            media_flags: 0b10,
            ..Default::default()
        };
        assert!(te.planar());
        // a face looking up: binormal X, tangent -Y; the position is scaled
        let out = transform_tex_coord([0.0, 0.0], [0.25, 0.1, 0.5], [0.0, 0.0, 1.0], &te, Vec3::new(2.0, 1.0, 1.0));
        let u = 1.0 + (0.5 * 2.0 - 0.5);
        let v = -(-0.1 * 2.0 - 0.5);
        assert!((out[0] - u).abs() < 1e-6 && (out[1] - v).abs() < 1e-6, "{out:?}");
    }
}
