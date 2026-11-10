//! AURORA_DEMO_COPLANAR_DECAL=1|mask|blend: a blended face lying exactly on
//! an opaque face of the same mesh (issue #3059).
//!
//! One static mesh, a 3 x 2.4 m panel standing 9 m along the camera's view
//! and 2 m below it,
//! made of two faces with the very same triangles, as a road section is
//! made of a grass base and a dirt track over it: face 0 opaque grass, face
//! 1 two dirt strips on a transparent background. The strips arrive with a
//! hard-edged alpha (drawn as a mask); with `1`, at frame 360 a full update
//! gives face 1 a soft-edged version (drawn blended), as a season script
//! changing the texture does. `mask` and `blend` keep one version.
//! The strips must stay on the panel after the change: the blended pass
//! passes equal depths, as the Legacy Viewer's alpha pass (GL_LEQUAL).

use super::*;
use aurora_assets::import_mesh::{self, Face, Model, Vertex, encode};

pub const MESH: Uuid = Uuid::from_u128(0xD0D0_C0DE_CA10_0000_0000_0000_0000_0001);
pub const TEX_GRASS: Uuid = Uuid::from_u128(0xD0D0_C0DE_CA10_0000_0000_0000_0000_0002);
pub const TEX_STRIPS_HARD: Uuid = Uuid::from_u128(0xD0D0_C0DE_CA10_0000_0000_0000_0000_0003);
pub const TEX_STRIPS_SOFT: Uuid = Uuid::from_u128(0xD0D0_C0DE_CA10_0000_0000_0000_0000_0004);
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
}

fn mode() -> Option<Mode> {
    match std::env::var("AURORA_DEMO_COPLANAR_DECAL").ok()?.trim() {
        "mask" => Some(Mode::Mask),
        "blend" => Some(Mode::Blend),
        "" | "0" => None,
        _ => Some(Mode::Switch),
    }
}

/// The panel in the XZ plane (normal -Y), both windings, two faces sharing
/// every vertex and index.
fn model() -> Model {
    let mut vertices = Vec::new();
    for row in 0..=ALONG {
        for col in 0..=ACROSS {
            let (u, v) = (col as f32 / ACROSS as f32, row as f32 / ALONG as f32);
            vertices.push(Vertex {
                position: [(u - 0.5) * SIZE.x, 0.0, (v - 0.5) * SIZE.z],
                normal: Vec3::NEG_Y.to_array(),
                // the track runs along the panel, its texture repeated twice
                uv: [u, v * 2.0],
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
        faces: (0..2)
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
fn install(scene: &mut crate::scene::Scene) -> Result<Vec3, aurora_assets::AssetError> {
    let mut model = model();
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

/// The panel 9 m along the camera's horizontal look from `pos` and 2 m
/// below it (whole in the start view, which looks down), its face
/// turned to it, the strips' texture `strips` on face 1.
fn panel(size: Vec3, (pos, look): (Vec3, Vec3), strips: Uuid) -> ObjectUpdate {
    let sculpt = SculptParams {
        texture: MESH,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    };
    let mut entry = (*te([1.0; 4], 0, false, 0.0)).clone();
    entry.faces[0].texture = TEX_GRASS;
    entry.faces[1].texture = strips;
    prim(
        LOCAL_ID,
        pos + look * 9.0 - Vec3::Z * 2.0,
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
        (ARRIVE_FRAME, Mode::Blend) => TEX_STRIPS_SOFT,
        (ARRIVE_FRAME, _) => TEX_STRIPS_HARD,
        (SWITCH_FRAME, Mode::Switch) => TEX_STRIPS_SOFT,
        _ => return,
    };
    // the upload's size, the same on each update
    let size = match install(scene) {
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
        "demo coplanar decal: frame {frame}, {} strips, camera at {:.1?}",
        if strips == TEX_STRIPS_SOFT { "soft (blend)" } else { "hard (mask)" },
        place.0
    );
    world.apply(NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![panel(size, place, strips)],
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

/// Pixels and dimensions of the demo's textures (RGBA8).
pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    let soft = match *id {
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
    fn both_faces_have_the_same_triangles() {
        let m = model();
        assert_eq!(m.faces.len(), 2);
        assert_eq!(m.faces[0].indices, m.faces[1].indices);
        let positions = |f: &Face| f.vertices.iter().map(|v| v.position).collect::<Vec<_>>();
        assert_eq!(positions(&m.faces[0]), positions(&m.faces[1]));
    }
}
