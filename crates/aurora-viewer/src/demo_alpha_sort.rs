//! AURORA_DEMO_ALPHA_SORT=front|back|hud|hud-back|hud-zoom: the same unrigged DAE and PNGs supplied
//! for the human's grid comparison (#2575), passed through the normal loader.

use super::*;
use aurora_assets::import_mesh::{self, Model, encode};

const MESH: Uuid = Uuid::from_u128(0xDE40_2575_0000_0000_0000_0000_0000_0001);
const TEX: [Uuid; 3] = [
    Uuid::from_u128(0xDE40_2575_0000_0000_0000_0000_0000_0010),
    Uuid::from_u128(0xDE40_2575_0000_0000_0000_0000_0000_0011),
    Uuid::from_u128(0xDE40_2575_0000_0000_0000_0000_0000_0012),
];
const MODEL: &[u8] = include_bytes!("../assets/demo/alpha-sort/alpha-sort-three-faces.dae");
const PNG: [&[u8]; 3] = [
    include_bytes!("../assets/demo/alpha-sort/0-front-red.png"),
    include_bytes!("../assets/demo/alpha-sort/1-middle-green.png"),
    include_bytes!("../assets/demo/alpha-sort/2-back-blue.png"),
];

pub fn enabled() -> bool {
    std::env::var_os("AURORA_DEMO_ALPHA_SORT").is_some()
}

fn is_hud() -> bool {
    std::env::var("AURORA_DEMO_ALPHA_SORT").is_ok_and(|v| v.starts_with("hud"))
}

fn model() -> Result<Model, aurora_assets::AssetError> {
    import_mesh::collada::parse(MODEL, &aurora_assets::Skeleton::default())?
        .models
        .into_iter()
        .next()
        .ok_or_else(|| aurora_assets::AssetError::invalid("alpha sort fixture has no model"))
}

pub fn point() -> Vec3 {
    Vec3::new(139.0, 131.0, height(139.0, 131.0) + 12.0)
}

pub fn view(frame: u64) -> Option<(Vec3, Vec3)> {
    if !enabled() || is_hud() || frame != 300 {
        return None;
    }
    let y = if std::env::var("AURORA_DEMO_ALPHA_SORT").as_deref() == Ok("back") {
        4.0
    } else {
        -4.0
    };
    Some((point() + Vec3::new(0.0, y, 0.1), point()))
}

pub fn texture(id: &Uuid) -> Option<(Vec<u8>, u32, u32)> {
    let index = TEX.iter().position(|t| t == id)?;
    let rgba = image::load_from_memory(PNG[index]).ok()?.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some((rgba.into_raw(), w, h))
}

pub fn tick(world: &mut crate::world::World, scene: &mut crate::scene::Scene, frame: u64) {
    if !enabled() || frame != 120 {
        return;
    }
    let result = (|| {
        let model = model()?;
        let size = model.size;
        let data = encode::asset(
            &[None, None, None, Some(model)],
            None,
            &import_mesh::process::Physics::default(),
            encode::Options::default(),
        )?;
        let header = aurora_assets::parse_mesh_header(&data)?;
        scene.meshes.insert_local(MESH, Arc::new(data), header, None);
        Ok::<_, aurora_assets::AssetError>(size)
    })();
    let size = match result {
        Ok(size) => size,
        Err(e) => {
            log::warn!("demo alpha sort: {e}");
            return;
        }
    };
    let sculpt = SculptParams {
        texture: MESH,
        sculpt_type: LL_SCULPT_TYPE_MESH,
    };
    let mut entry = (*te([1.0; 4], 0, true, 0.0)).clone();
    for (face, texture) in entry.faces.iter_mut().zip(TEX) {
        face.texture = texture;
    }
    let hud = is_hud();
    let back = std::env::var("AURORA_DEMO_ALPHA_SORT").as_deref() == Ok("hud-back");
    let mut object = prim(
        25750,
        if hud { Vec3::new(0.0, 0.35, 0.03) } else { point() },
        if hud {
            Quat::from_rotation_z(if back {
                std::f32::consts::FRAC_PI_2
            } else {
                -std::f32::consts::FRAC_PI_2
            })
        } else {
            Quat::IDENTITY
        },
        if hud { size * 0.25 } else { size },
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
    let id = object.full_id;
    if hud {
        object.parent_id = 9000;
        object.state = 31u8.rotate_left(4);
        object.owner_id = DEMO_AGENT;
        object.name_values = format!("AttachItemID STRING RW SV {id}");
    }
    world.apply(NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![object],
    });
    if hud {
        // A prior demo run may have saved this synthetic attachment's zoom.
        world.hud_zoom = 1.0;
        let zoom = if std::env::var("AURORA_DEMO_ALPHA_SORT").as_deref() == Ok("hud-zoom") {
            0.5
        } else {
            1.0
        };
        world.hud_scales.set(id, zoom);
    }
    log::info!(
        "demo alpha sort: three nonrigged faces, hud={hud}, size={size:?}, face_sort={}",
        crate::scene::blend_order::face_sort_enabled()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpha_sort_upload_fixture_is_one_unrigged_mesh_with_three_separate_layers() {
        let model = model().expect("fixture imports");
        assert!(model.skin.is_none());
        assert_eq!(model.faces.len(), 3);
        assert_eq!(model.triangles(), 12);
        assert!((model.size - Vec3::new(2.0, 0.6, 2.0)).abs().max_element() < 1e-5);
        let centers: Vec<_> = model
            .faces
            .iter()
            .map(|f| {
                let p: Vec<_> = f.vertices.iter().map(|v| v.position).collect();
                crate::scene::blend_order::FaceBounds::of(&p).center.y
            })
            .collect();
        assert!(centers[0] < centers[1] && centers[1] < centers[2]);
        for id in TEX {
            let (pixels, w, h) = texture(&id).expect("texture");
            assert_eq!((w, h), (512, 512));
            for alpha in [0, 150, 255] {
                assert!(pixels.as_chunks::<4>().0.iter().any(|p| p[3] == alpha));
            }
        }
    }
}
