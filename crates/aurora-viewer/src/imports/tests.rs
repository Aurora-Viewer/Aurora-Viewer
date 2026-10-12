use super::*;

fn prices() -> Prices {
    Prices {
        texture: Some(10),
        large_texture: Some(40),
        sound: Some(10),
        animation: Some(10),
    }
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
