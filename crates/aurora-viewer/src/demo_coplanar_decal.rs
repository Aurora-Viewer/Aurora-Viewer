//! AURORA_DEMO_COPLANAR_DECAL=1|mask|blend|layers: blended faces lying
//! exactly on an opaque face of the same mesh (issue #3059), and on one
//! another.
//!
//! One static mesh, a 3 x 2.4 m panel standing 9 m along the camera's view
//! and 2 m below it (1.5 m above it with `layers`),
//! made of two faces with the very same triangles, as a road section is
//! made of a grass base and a dirt track over it: face 0 opaque grass, face
//! 1 two dirt strips on a transparent background. The strips arrive with a
//! hard-edged alpha (drawn as a mask); with `1`, at frame 360 a full update
//! gives face 1 a soft-edged version (drawn blended), as a season script
//! changing the texture does. `mask` and `blend` keep one version.
//! The strips must stay on the panel after the change: the blended pass
//! passes equal depths, as the Legacy Viewer's alpha pass (GL_LEQUAL).
//!
//! `layers`: four faces with the very same triangles, as the layers of a
//! mesh eye are: face 0 the grass, face 1 a blue disc, face 2 a smaller
//! black disc, face 3 two white dots, the three of them blended. Each must
//! lie over the faces before it whatever the order of transparent surfaces
//! (`AURORA_BLEND_ORDER`): the Legacy Viewer draws blended faces at one
//! distance in the order of the faces (`stacked_layer_rank`, scene/mod.rs).

use super::*;
use aurora_assets::import_mesh::{self, Face, Model, Vertex, encode};

pub const MESH: Uuid = Uuid::from_u128(0xD0D0_C0DE_CA10_0000_0000_0000_0000_0001);
pub const TEX_GRASS: Uuid = Uuid::from_u128(0xD0D0_C0DE_CA10_0000_0000_0000_0000_0002);
pub const TEX_STRIPS_HARD: Uuid = Uuid::from_u128(0xD0D0_C0DE_CA10_0000_0000_0000_0000_0003);
pub const TEX_STRIPS_SOFT: Uuid = Uuid::from_u128(0xD0D0_C0DE_CA10_0000_0000_0000_0000_0004);
pub const TEX_DISC: Uuid = Uuid::from_u128(0xD0D0_C0DE_CA10_0000_0000_0000_0000_0005);
pub const TEX_CORE: Uuid = Uuid::from_u128(0xD0D0_C0DE_CA10_0000_0000_0000_0000_0006);
pub const TEX_DOTS: Uuid = Uuid::from_u128(0xD0D0_C0DE_CA10_0000_0000_0000_0000_0007);
const LOCAL_ID: u32 = 9_840;
const ARRIVE_FRAME: u64 = 120;
const SWITCH_FRAME: u64 = 360;
/// Segments across and along the panel.
const ACROSS: usize = 4;
const ALONG: usize = 6;
const SIZE: Vec3 = Vec3::new(3.0, 0.0, 2.4);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    /// Hard strips, then soft ones at SWITCH_FRAME.
    Switch,
    Mask,
    Blend,
    /// Three blended faces on the grass and on one another.
    Layers,
}

impl Mode {
    /// Faces of the panel, all of the same triangles.
    fn faces(self) -> usize {
        if self == Mode::Layers { 4 } else { 2 }
    }

    /// Height of the panel's centre above the camera (m): the track whole
    /// in the start view, which looks down; the layers above our avatar,
    /// which stands in the middle of the view and would hide their centre.
    fn rise(self) -> f32 {
        if self == Mode::Layers { 1.5 } else { -2.0 }
    }
}

fn mode() -> Option<Mode> {
    match std::env::var("AURORA_DEMO_COPLANAR_DECAL").ok()?.trim() {
        "mask" => Some(Mode::Mask),
        "blend" => Some(Mode::Blend),
        "layers" => Some(Mode::Layers),
        "" | "0" => None,
        _ => Some(Mode::Switch),
    }
}

/// The panel in the XZ plane (normal -Y), both windings, `faces` faces
/// sharing every vertex and index; its texture repeated `repeats` times
/// along it.
fn model(faces: usize, repeats: f32) -> Model {
    let mut vertices = Vec::new();
    for row in 0..=ALONG {
        for col in 0..=ACROSS {
            let (u, v) = (col as f32 / ACROSS as f32, row as f32 / ALONG as f32);
            vertices.push(Vertex {
                position: [(u - 0.5) * SIZE.x, 0.0, (v - 0.5) * SIZE.z],
                normal: Vec3::NEG_Y.to_array(),
                uv: [u, v * repeats],
                joints: [0; 4],
                weights: [0.0; 4],
            });
        }
    }
    let row = (ACROSS + 1) as u32;
    let indices: Vec<u32> = (0..ALONG as u32)
        .flat_map(|r| (0..ACROSS as u32).map(move |c| r * row + c))
        .flat_map(|i| {
            [
                i,
                i + row,
                i + 1,
                i + 1,
                i + row,
                i + row + 1,
                i,
                i + 1,
                i + row,
                i + 1,
                i + row + 1,
                i + row,
            ]
        })
        .collect();
    Model {
        name: "Demo coplanar decal".into(),
        submodel: 0,
        faces: (0..faces)
            .map(|material| Face {
                vertices: vertices.clone(),
                indices: indices.clone(),
                material,
            })
            .collect(),
        skin: None,
        center: Vec3::ZERO,
        size: Vec3::ONE,
    }
}

/// The mesh asset in the scene's mesh cache; the size the upload gives the
/// object.
fn install(scene: &mut crate::scene::Scene, mode: Mode) -> Result<Vec3, aurora_assets::AssetError> {
    // the track runs along the panel, its texture repeated twice
    let mut model = model(mode.faces(), if mode == Mode::Layers { 1.0 } else { 2.0 });
    model.normalize()?;
    let size = model.size;
    let data = encode::asset(
        &[None, None, None, Some(model)],
        None,
        &import_mesh::process::Physics::default(),
        encode::Options::default(),
    )?;
    let header = aurora_assets::parse_mesh_header(&data)?;
    scene.meshes.insert_local(MESH, Arc::new(data), header, None);
    Ok(size)
}

/// The panel 9 m along the camera's horizontal look from `pos` and `rise`
/// m above it, its face turned to it, the textures `over` on the faces
/// after the grass.
fn panel(size: Vec3, (pos, look): (Vec3, Vec3), rise: f32, over: &[Uuid]) -> ObjectUpdate {
    let sculpt = SculptParams {
        texture: MESH,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    };
    let mut entry = (*te([1.0; 4], 0, false, 0.0)).clone();
    entry.faces[0].texture = TEX_GRASS;
    for (face, texture) in entry.faces[1..].iter_mut().zip(over) {
        face.texture = *texture;
    }
    prim(
        LOCAL_ID,
        pos + look * 9.0 + Vec3::Z * rise,
        // mesh +Y (away from the face) along the look
        Quat::from_rotation_z(look.y.atan2(look.x) - std::f32::consts::FRAC_PI_2),
        size,
        aurora_prim::VolumeParams {
            sculpt: Some(sculpt),
            ..Default::default()
        },
        Arc::new(entry),
        ExtraParams {
            sculpt: Some(sculpt),
            ..Default::default()
        },
        "",
    )
}

/// The camera's position at the arrival and its horizontal look: the
/// update keeps the panel where it was.
static PLACE: std::sync::Mutex<Option<(Vec3, Vec3)>> = std::sync::Mutex::new(None);

/// The mesh and the panel at ARRIVE_FRAME; the soft strips at SWITCH_FRAME.
pub fn tick(world: &mut crate::world::World, scene: &mut crate::scene::Scene, frame: u64, (eye, forward): (Vec3, Vec3)) {
    let Some(mode) = mode() else { return };
    let strips = match (frame, mode) {
        (ARRIVE_FRAME, Mode::Layers) => TEX_DISC,
        (ARRIVE_FRAME, Mode::Blend) => TEX_STRIPS_SOFT,
        (ARRIVE_FRAME, _) => TEX_STRIPS_HARD,
        (SWITCH_FRAME, Mode::Switch) => TEX_STRIPS_SOFT,
        _ => return,
    };
    let over: &[Uuid] = if mode == Mode::Layers {
        &[TEX_DISC, TEX_CORE, TEX_DOTS]
    } else {
        std::slice::from_ref(&strips)
    };
    // the upload's size, the same on each update
    let size = match install(scene, mode) {
        Ok(size) => size,
        Err(e) => {
            log::warn!("demo coplanar decal: {e}");
            return;
        }
    };
    let place = {
        let mut place = PLACE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *place.get_or_insert_with(|| {
            let look = Vec3::new(forward.x, forward.y, 0.0).try_normalize().unwrap_or(Vec3::X);
            (eye, look)
        })
    };
    log::info!(
        "demo coplanar decal: frame {frame}, {}, camera at {:.1?}",
        match strips {
            TEX_DISC => "three blended layers",
            TEX_STRIPS_SOFT => "soft (blend) strips",
            _ => "hard (mask) strips",
        },
        place.0
    );
    world.apply(NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![panel(size, place, mode.rise(), over)],
    });
}

/// Alpha of the strips' texture at `x` across it (0..1): two strips, hard
/// or with soft edges.
fn strip_alpha(x: f32, soft: bool) -> u8 {
    let d = [0.3f32, 0.7].iter().map(|c| (x - c).abs()).fold(f32::MAX, f32::min);
    let a = if soft {
        1.0 - ((d - 0.05) / 0.08).clamp(0.0, 1.0)
    } else {
        f32::from(u8::from(d < 0.09))
    };
    (a * 255.0).round() as u8
}

/// Alpha of a soft-edged disc of radius `r` at the distance `d` from its
/// centre (texture widths).
fn disc_alpha(d: f32, r: f32) -> f32 {
    1.0 - ((d - r) / 0.04).clamp(0.0, 1.0)
}

/// A texel of a layer of `layers`: a blue disc, a black one at its centre,
/// two white dots on that one.
fn layer_texel(id: Uuid, u: f32, v: f32) -> [u8; 4] {
    let from = |cu: f32, cv: f32| ((u - cu).powi(2) + (v - cv).powi(2)).sqrt();
    let (rgb, a) = match id {
        TEX_DISC => ([60, 120, 230], disc_alpha(from(0.5, 0.5), 0.36)),
        TEX_CORE => ([8, 8, 10], disc_alpha(from(0.5, 0.5), 0.2)),
        _ => (
            [255, 255, 255],
            disc_alpha(from(0.42, 0.42), 0.05).max(disc_alpha(from(0.58, 0.47), 0.025)),
        ),
    };
    [rgb[0], rgb[1], rgb[2], (a * 255.0).round() as u8]
}

/// Pixels and dimensions of the demo's textures (RGBA8).
pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    let soft = match *id {
        TEX_DISC | TEX_CORE | TEX_DOTS => {
            let mut rgba = Vec::with_capacity(128 * 128 * 4);
            for y in 0..128 {
                for x in 0..128 {
                    rgba.extend_from_slice(&layer_texel(*id, (x as f32 + 0.5) / 128.0, (y as f32 + 0.5) / 128.0));
                }
            }
            return Some((rgba, 128, 128));
        }
        TEX_GRASS => {
            let mut rgba = Vec::with_capacity(64 * 64 * 4);
            for y in 0..64u32 {
                for x in 0..64u32 {
                    let n = ((x * 7 + y * 13) % 11) as u8 * 3;
                    rgba.extend_from_slice(&[70 + n, 110 + n, 40, 255]);
                }
            }
            return Some((rgba, 64, 64));
        }
        TEX_STRIPS_HARD => false,
        TEX_STRIPS_SOFT => true,
        _ => return None,
    };
    let mut rgba = Vec::with_capacity(64 * 64 * 4);
    for _y in 0..64 {
        for x in 0..64 {
            rgba.extend_from_slice(&[205, 185, 150, strip_alpha((x as f32 + 0.5) / 64.0, soft)]);
        }
    }
    Some((rgba, 64, 64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::jobs::{AlphaKind, classify_alpha};

    #[test]
    fn the_hard_strips_are_a_mask_and_the_soft_ones_blend() {
        let kind = |id| classify_alpha(&texture(&id).expect("demo texture").0);
        assert_eq!(kind(TEX_GRASS), AlphaKind::Opaque);
        assert_eq!(kind(TEX_STRIPS_HARD), AlphaKind::Mask);
        assert_eq!(kind(TEX_STRIPS_SOFT), AlphaKind::Blend);
    }

    #[test]
    fn the_layers_are_blended_and_opaque_at_their_centre() {
        for (id, at) in [(TEX_DISC, (0.5, 0.5)), (TEX_CORE, (0.5, 0.5)), (TEX_DOTS, (0.42, 0.42))] {
            assert_eq!(classify_alpha(&texture(&id).expect("demo texture").0), AlphaKind::Blend, "{id}");
            assert_eq!(layer_texel(id, at.0, at.1)[3], 255, "{id}");
        }
        // the dots lie on the black disc, which lies on the blue one
        assert_eq!(layer_texel(TEX_CORE, 0.58, 0.47)[3], 255);
        assert_eq!(layer_texel(TEX_DISC, 0.2, 0.5)[3], 255);
        assert_eq!(layer_texel(TEX_CORE, 0.2, 0.5)[3], 0);
    }

    #[test]
    fn every_face_has_the_same_triangles() {
        let positions = |f: &Face| f.vertices.iter().map(|v| v.position).collect::<Vec<_>>();
        for mode in [Mode::Switch, Mode::Layers] {
            let m = model(mode.faces(), 1.0);
            assert_eq!(m.faces.len(), mode.faces());
            for face in &m.faces[1..] {
                assert_eq!(face.indices, m.faces[0].indices);
                assert_eq!(positions(face), positions(&m.faces[0]));
            }
        }
    }
}
