//! AURORA_DEMO_BUMP=sphere: a low-poly mesh ball (12 × 8 quads, smooth
//! normals, its UVs warped as an unwrapping tool leaves them) in front of
//! the normal-mapped panel, with a normal map baked the way a modelling
//! tool bakes one: from a perfectly round, dimpled ball, through the mesh's
//! own MikkTSpace tangents (the ones the viewer builds, scene/tangents.rs).
//! On the vertices' tangents (AURORA_TANGENTS=vertex) the ball is round and
//! evenly dimpled, its triangles invisible; on a per-pixel frame (`pixel`,
//! `derivative`) each triangle turns the baked relief its own way and the
//! facets show.

use super::*;
use aurora_assets::import_mesh::{self, Face, Model, encode};

/// The ball's mesh asset, its baked normal map and its material.
pub const MESH: Uuid = Uuid::from_u128(0xDE40_7A90_0000_0000_0000_0000_0000_0001);
pub const TEX: Uuid = Uuid::from_u128(0xDE40_7E10_0000_0000_0000_0000_0000_0015);
pub const MAT: Uuid = Uuid::from_u128(0xDE40_3A70_0000_0000_0000_0000_0000_0006);
const OBJECT: u32 = 9725;
const SEGMENTS: usize = 12;
const RINGS: usize = 8;
/// Side of the baked map, in texels.
const SIZE: usize = 512;

pub fn enabled() -> bool {
    std::env::var("AURORA_DEMO_BUMP").as_deref() == Ok("sphere")
}

/// Unwrapped coordinates of the ball's (longitude, latitude) grid point,
/// warped so that no two neighbouring triangles map the texture alike.
fn warp(u: f32, v: f32) -> [f32; 2] {
    let tau = std::f32::consts::TAU;
    [
        0.06 + 0.88 * (u + 0.07 * (tau * 2.0 * v).sin()),
        0.06 + 0.88 * (v + 0.05 * (tau * 3.0 * u).sin()),
    ]
}

fn model() -> Model {
    let mut vertices = Vec::new();
    for r in 0..=RINGS {
        for s in 0..=SEGMENTS {
            let (u, v) = (s as f32 / SEGMENTS as f32, r as f32 / RINGS as f32);
            let (theta, phi) = (v * std::f32::consts::PI, u * std::f32::consts::TAU);
            let n = Vec3::new(theta.sin() * phi.cos(), theta.sin() * phi.sin(), -theta.cos());
            vertices.push(import_mesh::Vertex {
                position: (n * 0.5).to_array(),
                normal: n.to_array(),
                uv: warp(u, v),
                joints: [0; 4],
                weights: [0.0; 4],
            });
        }
    }
    let at = |r: usize, s: usize| (r * (SEGMENTS + 1) + s) as u32;
    let mut indices = Vec::new();
    for r in 0..RINGS {
        for s in 0..SEGMENTS {
            let (a, b, c, d) = (at(r, s), at(r, s + 1), at(r + 1, s + 1), at(r + 1, s));
            if r > 0 {
                indices.extend([a, b, c]);
            }
            if r + 1 < RINGS {
                indices.extend([a, c, d]);
            }
        }
    }
    Model {
        name: "Baked ball".into(),
        submodel: 0,
        faces: vec![Face {
            vertices,
            indices,
            material: 0,
        }],
        skin: None,
        center: Vec3::ZERO,
        size: Vec3::ONE,
    }
}

struct Asset {
    data: Arc<Vec<u8>>,
    header: aurora_assets::MeshHeader,
}

fn asset() -> Option<&'static Asset> {
    static ASSET: std::sync::OnceLock<Option<Asset>> = std::sync::OnceLock::new();
    ASSET
        .get_or_init(|| {
            let built = (|| {
                let mut model = model();
                model.normalize()?;
                let data = encode::asset(
                    &[None, None, None, Some(model)],
                    None,
                    &import_mesh::process::Physics::default(),
                    encode::Options::default(),
                )?;
                let header = aurora_assets::parse_mesh_header(&data)?;
                Ok::<_, aurora_assets::AssetError>(Asset {
                    data: Arc::new(data),
                    header,
                })
            })();
            built.map_err(|e| log::warn!("demo baked ball: {e}")).ok()
        })
        .as_ref()
}

/// The perfect ball the map is baked from: its normal at `p` (normalized
/// mesh space), dimpled by h = sin(ax) sin(ay) sin(az).
fn baked_normal(p: Vec3) -> Vec3 {
    let n = p.normalize_or(Vec3::Z);
    let a = std::f32::consts::TAU * 2.5;
    let (s, c) = ((p * a).to_array().map(f32::sin), (p * a).to_array().map(f32::cos));
    let grad = Vec3::new(c[0] * s[1] * s[2], s[0] * c[1] * s[2], s[0] * s[1] * c[2]) * a;
    (n - (grad - n * n.dot(grad)) * 0.035).normalize()
}

/// The baked normal map (RGBA8, top row first): for each texel, the
/// perfect ball's normal in the tangent frame the shader interpolates there
/// (materialF.glsl: T as interpolated, B = sign × cross(N, T), the sign of
/// the triangle's first vertex), as MikkTSpace bakers write it.
pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    if *id != TEX {
        return None;
    }
    let asset = asset()?;
    let range = asset.header.lods[3]?;
    let faces = aurora_assets::decode_mesh_lod(aurora_assets::mesh::section_bytes(&asset.data, range)?).ok()?;
    let (faces, _, _) = crate::scene::jobs::mesh_to_faces(&faces, None, None);
    let face = faces.into_iter().next()??;
    let mut texels = vec![None::<Vec3>; SIZE * SIZE];
    let normal = |v: &aurora_render::Vertex| Vec3::new(v.normal[0] as f32, v.normal[1] as f32, v.normal[2] as f32) / 127.0;
    for tri in face.indices.as_chunks::<3>().0 {
        let v = [0, 1, 2].map(|k| face.vertices[tri[k] as usize]);
        let Some((_, sign)) = aurora_render::unpack_tangent(v[0].normal) else {
            continue;
        };
        let t = v.map(|v| aurora_render::unpack_tangent(v.normal).map_or(Vec3::ZERO, |(t, _)| t));
        // texel space: s across, t up the image
        let q = v.map(|v| glam::Vec2::new(v.uv[0] * SIZE as f32, (1.0 - v.uv[1]) * SIZE as f32));
        let area = (q[1] - q[0]).perp_dot(q[2] - q[0]);
        if area.abs() < 1e-6 {
            continue;
        }
        let lo = q[0].min(q[1]).min(q[2]).floor().max(glam::Vec2::ZERO);
        let hi = q[0].max(q[1]).max(q[2]).ceil().min(glam::Vec2::splat(SIZE as f32 - 1.0));
        for y in lo.y as usize..=hi.y as usize {
            for x in lo.x as usize..=hi.x as usize {
                let c = glam::Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let w1 = (c - q[0]).perp_dot(q[2] - q[0]) / area;
                let w2 = (q[1] - q[0]).perp_dot(c - q[0]) / area;
                let w0 = 1.0 - w1 - w2;
                if w0 < -0.02 || w1 < -0.02 || w2 < -0.02 {
                    continue;
                }
                let p = Vec3::from_array(v[0].pos) * w0 + Vec3::from_array(v[1].pos) * w1 + Vec3::from_array(v[2].pos) * w2;
                let n = (normal(&v[0]) * w0 + normal(&v[1]) * w1 + normal(&v[2]) * w2).normalize_or(Vec3::Z);
                let tt = t[0] * w0 + t[1] * w1 + t[2] * w2;
                let frame = glam::Mat3::from_cols(tt, n.cross(tt) * sign, n);
                if frame.determinant().abs() < 1e-6 {
                    continue;
                }
                texels[y * SIZE + x] = Some((frame.inverse() * baked_normal(p)).normalize_or(Vec3::Z));
            }
        }
    }
    // a few texels of padding around the islands, for filtering and mips
    for _ in 0..8 {
        let before = texels.clone();
        for y in 0..SIZE {
            for x in 0..SIZE {
                if before[y * SIZE + x].is_some() {
                    continue;
                }
                let mut sum = Vec3::ZERO;
                for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if (0..SIZE as i32).contains(&nx) && (0..SIZE as i32).contains(&ny) {
                        sum += before[ny as usize * SIZE + nx as usize].unwrap_or(Vec3::ZERO);
                    }
                }
                if sum != Vec3::ZERO {
                    texels[y * SIZE + x] = Some(sum.normalize());
                }
            }
        }
    }
    let e = |v: f32| ((v * 0.5 + 0.5) * 255.0).round().clamp(0.0, 255.0) as u8;
    let pixels = texels
        .iter()
        .flat_map(|n| {
            let n = n.unwrap_or(Vec3::Z);
            [e(n.x), e(n.y), e(n.z), 255]
        })
        .collect();
    Some((pixels, SIZE as u32, SIZE as u32))
}

/// Its legacy material: the baked map, a soft shine.
pub fn material() -> crate::scene::legacy_mat::LegacyMaterial {
    crate::scene::legacy_mat::LegacyMaterial {
        normal_map: TEX,
        normal_st: [1.0, 1.0, 0.0, 0.0],
        normal_rot: 0.0,
        specular_map: Uuid::nil(),
        specular_st: [1.0, 1.0, 0.0, 0.0],
        specular_rot: 0.0,
        specular_color: [255, 255, 255, 255],
        specular_exp: 60,
        env_intensity: 0,
        diffuse_alpha_mode: crate::scene::legacy_mat::alpha_mode::NONE,
        alpha_cutoff: 0,
    }
}

/// The ball, in front of the normal-mapped panel.
fn object() -> ObjectUpdate {
    let sculpt = SculptParams {
        texture: MESH,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    };
    let mut t = (*te([0.8, 0.78, 0.75, 1.0], 0, false, 0.0)).clone();
    for f in &mut t.faces {
        f.material_id = MAT;
    }
    prim(
        OBJECT,
        Vec3::new(134.3, 128.9, floor_at(134.3, 128.9) + 1.3),
        Quat::IDENTITY,
        Vec3::splat(1.1),
        aurora_prim::VolumeParams {
            sculpt: Some(sculpt),
            ..Default::default()
        },
        Arc::new(t),
        ExtraParams {
            sculpt: Some(sculpt),
            ..Default::default()
        },
        "",
    )
}

/// The ball's mesh, then the ball, a little after the arrival.
pub fn tick(world: &mut crate::world::World, scene: &mut crate::scene::Scene, frame: u64) {
    if frame != 120 || !enabled() {
        return;
    }
    let Some(asset) = asset() else {
        return;
    };
    scene.meshes.insert_local(MESH, asset.data.clone(), asset.header.clone(), None);
    world.apply(NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![object()],
    });
}
