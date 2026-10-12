use super::*;

fn prices() -> Prices {
    Prices {
        texture: Some(10),
        large_texture: Some(40),
        sound: Some(10),
        animation: Some(10),
    }
}

#[test]
fn mesh_import_factor_obeys_prim_size_and_linkset_span() {
    use aurora_assets::import_mesh::{Instance, Scene};
    use glam::{Mat4, Vec3};
    let instance = |x| Instance {
        model: 0,
        name: "Part".into(),
        materials: Vec::new(),
        transform: Mat4::from_scale_rotation_translation(Vec3::splat(2.0), glam::Quat::IDENTITY, Vec3::X * x),
    };
    let mut draft = mesh::Draft::new(
        Scene {
            instances: vec![instance(0.0)],
            ..Default::default()
        },
        HashMap::new(),
    );
    draft.options.scale = 2.0;
    assert_eq!(draft.dimensions_and_limit(), (Vec3::splat(4.0), 32.0));
    Arc::make_mut(&mut draft.scene).instances.push(instance(239.0));
    let (dimensions, limit) = draft.dimensions_and_limit();
    assert_eq!(dimensions, Vec3::new(482.0, 4.0, 4.0));
    assert!((limit - 240.0 / 241.0).abs() < 0.00001);
}

#[test]
fn instance_edits_retire_the_quote_and_keep_prepared_geometry() {
    let mut draft = mesh::Draft::new(Default::default(), HashMap::new());
    let prepared = Arc::new(mesh::Prepared {
        lods: Vec::new(),
        physics: Vec::new(),
        hulls: Vec::new(),
        degenerate_triangles: 0,
    });
    draft.prepared = Some(prepared.clone());
    let token = Uuid::new_v4();
    draft.quote = Some(aurora_net::inventory::mesh_upload::Quote {
        token,
        cost: 10,
        data: aurora_llsd::Llsd::Undef,
    });
    let before = draft.options.clone();
    draft.options.scale = 0.5;
    draft.options.pelvis = 0.2;
    assert!(draft.finish_edit(&before, false));
    assert!(Arc::ptr_eq(draft.prepared.as_ref().expect("geometry"), &prepared));
    assert!(draft.quote.is_none());
    assert_eq!(draft.discarded_quotes, [token]);
    let before = draft.options.clone();
    draft.options.normals = true;
    assert!(draft.finish_edit(&before, false));
    assert!(draft.prepared.is_none());
}

#[test]
fn restoring_a_failed_draft_keeps_material_edits_without_sending_or_clearing_the_warning() {
    let mut imports = Imports::default();
    let material = PbrMaterial {
        roughness_factor: 0.123,
        base_color_factor: [0.2, 0.4, 0.6, 1.0],
        ..Default::default()
    };
    let mut entry = Entry::new(
        "material.gltf".into(),
        Ok(Source::Material {
            value: material.clone(),
            images: Box::new([None, None, None, None]),
        }),
        Default::default(),
    );
    entry.status = Status::Error("Unknown result".into());
    let id = entry.id;
    imports.entries.push(entry);
    imports.message = "Verify inventory before retrying".into();
    imports.restore_draft(id);
    let Some(Source::Material { value, .. }) = &imports.entries[0].source else {
        panic!("material")
    };
    assert_eq!(*value, material);
    assert!(matches!(imports.entries[0].status, Status::Ready));
    assert!(imports.current.is_none() && imports.queue.is_empty());
    assert_eq!(imports.message, "Verify inventory before retrying");
}
fn image(w: u32, h: u32) -> Arc<DecodedImage> {
    Arc::new(DecodedImage {
        width: w,
        height: h,
        components: 3,
        data: vec![128; w as usize * h as usize * 3],
        discard: 0,
    })
}
#[test]
fn shared_material_maps_are_charged_once_with_equivalent_effective_encoding() {
    let pixels = image(4, 4);
    let map = LocalImage {
        key: Uuid::new_v4(),
        name: "Shared".into(),
        pixels,
        asset: None,
        empty_alpha: false,
    };
    let source = Source::Material {
        value: PbrMaterial::default(),
        images: Box::new([Some(map), None, None, None]),
    };
    let mut imports = Imports {
        choice: Choice::Material,
        ..Default::default()
    };
    imports
        .entries
        .push(Entry::new("a.gltf".into(), Ok(source.clone()), Default::default()));
    let mut second = Entry::new("b.gltf".into(), Ok(source), Default::default());
    second.options.max_width = 2048;
    second.options.max_height = 2048;
    imports.entries.push(second);
    assert_eq!(imports.total(&prices()), Some(10));
    if let Some(Source::Material { images, .. }) = &mut imports.entries[1].source {
        images[0].as_mut().expect("map").asset = Some(Uuid::new_v4());
    }
    imports.entries[0].selected = false;
    assert_eq!(imports.total(&prices()), Some(0));
}
#[test]
fn bulk_scaling_uses_area_pricing_and_failed_selected_rows_block_payment() {
    let mut imports = Imports {
        choice: Choice::Bulk,
        ..Default::default()
    };
    let options = import_texture::Options {
        max_width: 2048,
        max_height: 2048,
        ..Default::default()
    };
    imports
        .entries
        .push(Entry::new("large.png".into(), Ok(Source::Texture(image(2048, 1024))), options));
    assert_eq!(imports.total(&prices()), Some(40));
    imports.scale_bulk_1k = true;
    assert_eq!(imports.total(&prices()), Some(10));
    let mut failed = Entry::new("broken.png".into(), Err("Invalid".into()), options);
    failed.selected = true;
    imports.entries.push(failed);
    assert_eq!(imports.total(&prices()), None);
}
#[test]
fn cancelling_future_uploads_retains_the_in_flight_receipt() {
    let mut imports = Imports::default();
    let entry = Entry::new("texture.png".into(), Ok(Source::Texture(image(4, 4))), Default::default());
    let step = Step {
        key: entry.id,
        entry: Some(entry.id),
        kind: Kind::Texture,
        folder: Uuid::new_v4(),
        name: entry.name.clone(),
        description: String::new(),
        source: entry.source.clone().expect("source"),
        options: entry.options,
        cost: 10,
        permissions: Permissions::default(),
    };
    let request = Uuid::new_v4();
    imports.current = Some((request, step));
    imports.entries.push(entry);
    imports.entries[0].status = Status::Uploading;
    imports.cancel_remaining();
    let item = aurora_net::inventory::InvItem {
        id: Uuid::new_v4(),
        asset_id: Uuid::new_v4(),
        ..Default::default()
    };
    assert!(
        imports
            .uploaded(
                Uuid::new_v4(),
                Ok(Receipt {
                    item: item.clone(),
                    cost: 10
                })
            )
            .is_none()
    );
    assert!(imports.current.is_some());
    assert_eq!(
        imports
            .uploaded(
                request,
                Ok(Receipt {
                    item: item.clone(),
                    cost: 10
                })
            )
            .expect("receipt")
            .id,
        item.id
    );
    assert!(matches!(imports.entries[0].status, Status::Done(_)));
    assert_eq!(imports.spent, 10);
}
