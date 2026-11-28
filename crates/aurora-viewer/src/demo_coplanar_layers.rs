//! AURORA_DEMO_COPLANAR_LAYERS: a worn layer lying exactly on the skin.
//!
//! One rigged mesh, a plate of eight horizontal strips hanging behind our
//! avatar's back (seen from the start camera), worn sixteen times: for each
//! strip a "skin" copy (the upper-body bake, as a mesh body shows it) and a
//! dark "layer" copy, each showing that strip alone (the other faces at
//! texture-entry alpha 0, like the hidden parts of a worn body). Both copies
//! of a strip have the very same triangles, so the same depths. The pairs
//! arrive in alternating order (skin first on even strips, layer first on odd
//! ones), so their draw records alternate too: without a layer rank the
//! winner alternated from strip to strip, showing bands; with it the dark
//! layer covers every strip.
//!
//! `AURORA_DEMO_COPLANAR_LAYERS=aux`: the add-on of #3461 instead, four
//! strips, each worn once as one object of two faces with the very same
//! triangles: its own dark opaque skin and a bake shown blended over it, as
//! an add-on shows an aux channel. The faces come in alternating order (the
//! dark one first on even strips). The bake must show over the dark face on
//! every strip; a dark strip means the blended bake ranks under the face it
//! lies on again.

use super::*;
use crate::scene::avatar::{AvatarLibrary, BAKES};
use aurora_assets::import_mesh::{self, Face, Model, Skin, Vertex, encode};
use glam::Mat4;

pub const MESH: Uuid = Uuid::from_u128(0xD0D0_C0D1_0000_0000_0000_0000_0000_0001);
/// Local ids of the copies: skin of strip i at FIRST + 2i, layer at FIRST + 2i + 1.
pub const FIRST: u32 = 9780;
const STRIPS: usize = 8;
/// Segments across a strip.
const ACROSS: usize = 6;

pub fn enabled() -> bool {
    std::env::var_os("AURORA_DEMO_COPLANAR_LAYERS").is_some()
}

/// `AURORA_DEMO_COPLANAR_LAYERS=aux`: one object of two faces per strip.
fn aux() -> bool {
    std::env::var("AURORA_DEMO_COPLANAR_LAYERS").is_ok_and(|v| v.trim().eq_ignore_ascii_case("aux"))
}

/// The plate where the uploader had it: 0.5 m wide and 0.8 m high, 0.55 m
/// behind the default skeleton's chest, facing the start camera (which sees
/// our avatar from behind), bound to the chest. Each strip has both
/// windings, seen from either side. `pairs`: half as many strips, each the
/// triangles of two faces (2i and 2i + 1).
fn model(lib: &AvatarLibrary, pairs: bool) -> Option<Model> {
    let chest = *lib.rig.names.get("mChest")?;
    let top = lib.rig.default_world_inv.get(chest)?.inverse().w_axis.truncate() + Vec3::new(-0.55, 0.0, 0.25);
    let strips = if pairs { STRIPS / 2 } else { STRIPS };
    let (wide, high) = (0.5, 0.8 / strips as f32);
    let faces = (0..STRIPS)
        .map(|face| {
            let strip = if pairs { face / 2 } else { face };
            let mut vertices = Vec::new();
            for row in 0..2 {
                for col in 0..=ACROSS {
                    let y = (col as f32 / ACROSS as f32 - 0.5) * wide;
                    let z = -((strip + row) as f32) * high;
                    vertices.push(Vertex {
                        position: (top + Vec3::new(0.0, y, z)).to_array(),
                        normal: Vec3::NEG_X.to_array(),
                        uv: [col as f32 / ACROSS as f32, 1.0 - row as f32],
                        joints: [0; 4],
                        weights: [1.0, 0.0, 0.0, 0.0],
                    });
                }
            }
            let row = (ACROSS + 1) as u32;
            let indices = (0..ACROSS as u32)
                .flat_map(|c| {
                    [
                        c,
                        c + row,
                        c + 1,
                        c + 1,
                        c + row,
                        c + row + 1,
                        c,
                        c + 1,
                        c + row,
                        c + 1,
                        c + row + 1,
                        c + row,
                    ]
                })
                .collect();
            Face {
                vertices,
                indices,
                material: face,
            }
        })
        .collect();
    Some(Model {
        name: "Demo coplanar layers".into(),
        submodel: 0,
        faces,
        skin: Some(Skin {
            names: vec!["mChest".into()],
            inverse_bind: vec![lib.rig.default_world_inv[chest]],
            alternate_bind: Vec::new(),
            bind_shape: Mat4::IDENTITY,
            pelvis_offset: 0.0,
        }),
        center: Vec3::ZERO,
        size: Vec3::ONE,
    })
}

/// The mesh asset in the scene's mesh cache; the size the upload gives the
/// object.
fn install(lib: &AvatarLibrary, scene: &mut crate::scene::Scene, pairs: bool) -> Result<Vec3, aurora_assets::AssetError> {
    let mut model = model(lib, pairs).ok_or_else(|| aurora_assets::AssetError::invalid("no mChest joint"))?;
    model.normalize()?;
    let size = model.size;
    let data = encode::asset(
        &[None, None, None, Some(model)],
        None,
        &import_mesh::process::Physics::default(),
        encode::Options {
            skin: true,
            ..Default::default()
        },
    )?;
    let header = aurora_assets::parse_mesh_header(&data)?;
    let skin = header
        .skin
        .and_then(|range| aurora_assets::mesh::section_bytes(&data, range))
        .map(aurora_assets::decode_skin)
        .transpose()?
        .map(Arc::new);
    scene.meshes.insert_local(MESH, Arc::new(data), header, skin);
    Ok(size)
}

/// One worn copy showing strip `strip` alone: the skin (the upper-body
/// bake) or the dark layer.
fn copy(strip: usize, layer: bool, size: Vec3, wearer: u32) -> ObjectUpdate {
    let mut entry = (*te([0.0, 0.0, 0.0, 0.0], 0, false, 0.0)).clone();
    let shown = &mut entry.faces[strip];
    if layer {
        shown.color = [0.08, 0.08, 0.1, 1.0];
    } else {
        shown.texture = BAKES[1].2;
        shown.color = [1.0, 0.8, 0.7, 1.0];
    }
    worn(FIRST + 2 * strip as u32 + u32::from(layer), entry, size, wearer)
}

/// The worn object `local` showing `entry`.
fn worn(local: u32, entry: TextureEntry, size: Vec3, wearer: u32) -> ObjectUpdate {
    let sculpt = SculptParams {
        texture: MESH,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    };
    let mut o = prim(
        local,
        Vec3::ZERO,
        Quat::IDENTITY,
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
    );
    o.parent_id = wearer;
    o.state = 0x10; // chest
    o
}

/// The add-on of strip `pair` (`aux`): its dark opaque skin and, on the same
/// triangles, the upper-body bake at 85 % (drawn blended, as an aux
/// channel's transparent background makes it). The dark face first on even
/// strips.
fn add_on(pair: usize, size: Vec3, wearer: u32) -> ObjectUpdate {
    let mut entry = (*te([0.0, 0.0, 0.0, 0.0], 0, false, 0.0)).clone();
    let (dark, bake) = if pair.is_multiple_of(2) {
        (2 * pair, 2 * pair + 1)
    } else {
        (2 * pair + 1, 2 * pair)
    };
    entry.faces[dark].color = [0.08, 0.08, 0.1, 1.0];
    entry.faces[bake].texture = BAKES[1].2;
    entry.faces[bake].color = [1.0, 0.8, 0.7, 0.85];
    worn(FIRST + pair as u32, entry, size, wearer)
}

/// Skin first on even strips, layer first on odd ones; or the add-ons.
fn objects(size: Vec3, wearer: u32, pairs: bool) -> Vec<ObjectUpdate> {
    if pairs {
        return (0..STRIPS / 2).map(|pair| add_on(pair, size, wearer)).collect();
    }
    (0..STRIPS)
        .flat_map(|strip| {
            let layer_first = strip % 2 == 1;
            [copy(strip, layer_first, size, wearer), copy(strip, !layer_first, size, wearer)]
        })
        .collect()
}

/// The mesh, then the copies, a little after the arrival.
pub fn tick(world: &mut crate::world::World, scene: &mut crate::scene::Scene, frame: u64) {
    if frame != 120 || !enabled() {
        return;
    }
    let pairs = aux();
    let size = match install(&world.avatar_lib, scene, pairs) {
        Ok(size) => size,
        Err(e) => {
            log::warn!("demo coplanar layers: {e}");
            return;
        }
    };
    let Some(wearer) = world
        .objects
        .index_of_uuid(&world.agent_id)
        .and_then(|i| world.objects.get(i))
        .map(|o| o.key.local_id)
    else {
        log::warn!("demo coplanar layers: no avatar to wear them");
        return;
    };
    log::info!("demo coplanar layers: {STRIPS} faces {size:.3?}, worn by {wearer}, add-ons {pairs}");
    world.apply(NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: objects(size, wearer, pairs),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_copy_shows_its_strip_alone_in_alternating_order() {
        let objects = objects(Vec3::ONE, 1, false);
        assert_eq!(objects.len(), 2 * STRIPS);
        for (i, o) in objects.iter().enumerate() {
            let strip = i / 2;
            let entry = o.texture_entry.as_ref().expect("texture entry");
            for (face, f) in entry.faces.iter().enumerate().take(STRIPS) {
                assert_eq!(f.color[3] > 0.0, face == strip, "copy {i} face {face}");
            }
            let layer = o.local_id % 2 == 1;
            assert_eq!(layer, (i % 2 == 0) == (strip % 2 == 1), "copy {i}");
            assert_eq!(entry.faces[strip].texture == BAKES[1].2, !layer);
        }
    }

    #[test]
    fn each_add_on_shows_a_blended_bake_over_its_own_dark_face() {
        let objects = objects(Vec3::ONE, 1, true);
        assert_eq!(objects.len(), STRIPS / 2);
        for (pair, o) in objects.iter().enumerate() {
            let entry = o.texture_entry.as_ref().expect("texture entry");
            let shown: Vec<usize> = (0..STRIPS).filter(|&f| entry.faces[f].color[3] > 0.0).collect();
            assert_eq!(shown, [2 * pair, 2 * pair + 1], "add-on {pair}");
            let bake = *shown.iter().find(|&&f| entry.faces[f].texture == BAKES[1].2).expect("bake face");
            assert!(entry.faces[bake].color[3] < 1.0, "drawn blended");
            // the dark face first on even strips
            assert_eq!(bake == 2 * pair + 1, pair.is_multiple_of(2), "add-on {pair}");
        }
    }
}
