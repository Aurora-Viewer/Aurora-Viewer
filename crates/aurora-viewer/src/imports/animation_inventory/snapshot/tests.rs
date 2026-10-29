use super::*;

fn fixture() -> Snapshot {
    Snapshot {
        joints: Vec::new(),
        images: vec![Picture {
            size: [1, 1],
            rgba: STANDARD.encode([120, 70, 210, 255]),
        }],
        parts: vec![Surface {
            vertices: vec![([0.0; 3], [0.0, 0.0, 1.0], [0.0; 2], [0; 4], [1.0, 0.0, 0.0, 0.0]); 3],
            indices: vec![0, 1, 2],
            textures: [Some(0), None, None, None],
            color: [1.0; 4],
            emissive: [0.0; 3],
            metallic: 0.0,
            roughness: 1.0,
            alpha_cutoff: 0.5,
            alpha_mode: 0,
            normal_scale: 1.0,
            double_sided: false,
            uv: Material::default().uv,
            binding: vec![("mTail1".into(), Mat4::IDENTITY.to_cols_array())],
        }],
    }
}

fn bytes(snapshot: &Snapshot) -> Vec<u8> {
    let mut zip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    serde_json::to_writer(&mut zip, snapshot).unwrap();
    let mut bytes = MAGIC.to_vec();
    bytes.extend(zip.finish().unwrap());
    bytes
}

#[test]
fn inventory_snapshot_validates_palette_indices_triangles_textures_and_named_bones() {
    let avatars = crate::scene::avatar::AvatarLibrary::load();
    let decode = |snapshot: &Snapshot| super::decode("test.apmodel".into(), &bytes(snapshot), &avatars);
    let loaded = decode(&fixture()).expect("valid named skin and pixels");
    assert_eq!(
        loaded.geometry.parts[0].skin.as_ref().unwrap().joints,
        vec![avatars.rig.names["mTail1"]]
    );
    assert_eq!(
        loaded.geometry.parts[0].textures[0].as_ref().unwrap().rgba.as_ref(),
        &[120, 70, 210, 255]
    );
    let mut value = fixture();
    value.parts[0].indices[2] = 3;
    assert!(decode(&value).is_err());
    let mut value = fixture();
    value.parts[0].vertices[0].3[0] = 1;
    assert!(decode(&value).is_err());
    let mut value = fixture();
    value.parts[0].vertices[0].4[0] = -1.0;
    assert!(decode(&value).is_err());
    let mut value = fixture();
    value.parts[0].binding[0].0 = "missing-bone".into();
    assert!(decode(&value).is_err());
    let mut value = fixture();
    value.images[0].size = [u32::MAX; 2];
    assert!(decode(&value).is_err());
    let mut value = fixture();
    value.parts[0].textures[1] = Some(4);
    assert!(decode(&value).is_err());
    let mut value = fixture();
    value.parts[0].indices.pop();
    assert!(decode(&value).is_err());
}
