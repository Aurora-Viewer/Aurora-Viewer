use super::*;
use aurora_net::inventory::InvItem;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn add_item(inv: &mut Inventory, n: u128, ty: i32, inv_type: i32, flags: u32, target: Uuid) -> Row {
    let item = InvItem {
        id: id(n),
        parent: id(3),
        asset_type: ty,
        inv_type,
        flags,
        asset_id: target,
        name: format!("Item {n}"),
        ..Default::default()
    };
    inv.items.insert(item.id, item);
    Row::Item(id(n))
}

fn rows(
    ctx: &egui::Context,
    art: &RowStyle,
    inv: &Inventory,
    row: Row,
    view: ViewLayout,
    selected: bool,
    images: &HashMap<Uuid, egui::TextureHandle>,
    path: Option<&str>,
) -> egui::FullOutput {
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let rect = Rect::from_min_size(
            pos2(10.0, 10.0),
            if matches!(view, ViewLayout::Icons(_)) {
                vec2(165.0, 160.0)
            } else {
                vec2(720.0, 42.0)
            },
        );
        cell(
            ui,
            art,
            inv,
            row,
            None,
            rect,
            view,
            dimensions::columns(rect.width(), path.is_some(), [0; 3]),
            "Example",
            path,
            selected,
            images,
            &mut Vec::new(),
            &mut MaterialTiles::default(),
        );
    });
    out.textures_delta.clear();
    out
}

fn has_image(out: &egui::FullOutput, tex: &egui::TextureHandle, tint: Color32) -> bool {
    out.shapes.iter().any(|s| {
        matches!(&s.shape, egui::Shape::Mesh(m) if m.texture_id == tex.id() &&
        !m.vertices.is_empty() && m.vertices.iter().all(|v|v.color == tint))
    })
}

#[test]
fn organized_special_folder_symbols_use_real_types_and_legacy_colors_on_both_surfaces() {
    let ctx = egui::Context::default();
    let icons = Icons::load(&ctx, None);
    let mut inv = super::super::tests::fixture();
    let palette = crate::theme::Theme::default().palette();
    let mut prefs = InventoryPreferences::default();
    let facts = Facts::default();
    let folder = id(5);
    let custom = Color32::from_rgb(12, 127, 52);
    prefs.folder_colors.insert(folder, [12, 127, 52]);
    let expected = [
        (15, "Inv_Snapshot"),
        (7, "Inv_Notecard"),
        (56, "Inv_Settings"),
        (50, "inbox"),
        (53, "building-store"),
        (16, "archive-filled"),
        (14, "Inv_TrashClosed"),
    ];
    let mut used = HashSet::new();
    for (kind, symbol) in expected {
        let texture = icons.get(symbol).expect(symbol);
        assert!(used.insert(texture.id()), "{symbol} must have a distinct silhouette");
        inv.folders.get_mut(&folder).unwrap().info.type_default = kind;
        for colored in [true, false] {
            prefs.folder_colors_enabled = colored;
            let art = RowStyle {
                palette: &palette,
                icons: &icons,
                prefs: &prefs,
                facts: &facts,
            };
            let tree = super::super::super::tree::folder_art(&prefs, folder, kind, false, &palette);
            let mut legacy = ctx.run_ui(Default::default(), |ui| {
                super::super::super::tree::icon(ui, &icons, tree.0, &palette, Some(tree.1));
            });
            legacy.textures_delta.clear();
            let (legacy_texture, tint) = legacy
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Rect(rect) => rect.brush.as_ref().map(|brush| (brush.fill_texture_id, rect.fill)),
                    _ => None,
                })
                .expect("Legacy tree painted its folder");
            assert_eq!(legacy_texture, icons.get(tree.0).unwrap().id());
            assert_eq!(
                tint,
                if colored {
                    custom
                } else if kind == 14 {
                    palette.ink
                } else {
                    palette.violet_light
                }
            );
            let row = Row::Place(Location::Folder(folder));
            for selected in [false, true] {
                let mut pin = ctx.run_ui(Default::default(), |ui| {
                    super::super::entry(ui, &art, "Pinned folder", art.resolve(&inv, row, selected, selected), selected);
                });
                pin.textures_delta.clear();
                let tint = if colored {
                    custom
                } else if selected {
                    palette.violet_pale
                } else {
                    palette.muted
                };
                assert!(has_image(&pin, texture, tint), "pin {symbol}, {selected}, {colored}");
            }
            for mode in [ViewLayout::Details, ViewLayout::List, ViewLayout::Icons(IconSize::Large)] {
                let out = rows(&ctx, &art, &inv, row, mode, true, &HashMap::new(), None);
                assert!(
                    has_image(&out, texture, if colored { custom } else { palette.violet_pale }),
                    "{mode:?}, {symbol}, {colored}"
                );
            }
        }
    }
    let art = RowStyle {
        palette: &palette,
        icons: &icons,
        prefs: &prefs,
        facts: &facts,
    };
    for (place, symbol) in [
        (Location::Home, "home-filled"),
        (Location::Library, "library-filled"),
        (Location::Group(Group::Appearance), "shirt-filled"),
        (Location::Custom(id(99)), "Inv_FolderClosed"),
    ] {
        assert_eq!(
            art.resolve(&inv, Row::Place(place), false, false),
            Glyph::navigation(symbol, palette.muted)
        );
    }
    // Same name in a personal folder or an imported nested category must not pick up system artwork.
    let info = &mut inv.folders.get_mut(&folder).unwrap().info;
    info.name = "Photos".to_owned();
    info.parent = id(4);
    info.type_default = 15;
    assert_eq!(
        art.resolve(&inv, Row::Place(Location::Folder(folder)), false, false),
        Glyph::navigation("Inv_FolderClosed", palette.muted)
    );
}

#[test]
fn organized_icon_parity_across_layouts_and_the_named_skin_override() {
    let ctx = egui::Context::default();
    let dir = std::env::temp_dir().join(format!("aurora-explorer-icons-{}", Uuid::new_v4()));
    std::fs::create_dir(&dir).unwrap();
    let marker = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="6" y="2" width="10" height="12"/></svg>"#;
    std::fs::write(dir.join("Inv_FolderClosed.svg"), marker).unwrap();
    std::fs::write(dir.join("Inv_Object_Multi_Link.svg"), marker).unwrap();
    let icons = Icons::load(&ctx, Some(&dir));
    std::fs::remove_dir_all(dir).unwrap();
    let p = crate::theme::Theme::default().palette();
    let mut inv = super::super::tests::fixture();
    let object = add_item(&mut inv, 100, 6, 6, 0x20_0000, id(2000));
    let linked = add_item(&mut inv, 101, 24, 6, 0, id(100));
    let broken = add_item(&mut inv, 102, 24, 6, 0, id(9999));
    let shirt = add_item(&mut inv, 103, 5, 18, 4, id(2001));
    let folder_link = add_item(&mut inv, 104, 25, 8, 0, id(3));
    inv.folders.get_mut(&id(5)).unwrap().info.type_default = 14;
    inv.folders.get_mut(&id(7)).unwrap().info.type_default = 100;
    let mut prefs = InventoryPreferences::default();
    let custom = Color32::from_rgb(51, 143, 173);
    prefs.folder_colors_enabled = true;
    prefs.folder_colors.insert(id(3), [51, 143, 173]);
    let facts = Facts::default();
    let art = RowStyle {
        palette: &p,
        icons: &icons,
        prefs: &prefs,
        facts: &facts,
    };
    for mode in [ViewLayout::Details, ViewLayout::List, ViewLayout::Icons(IconSize::Medium)] {
        for selected in [false, true] {
            let folder_tint = if selected { p.violet_pale } else { p.muted };
            for (row, name, tint) in [
                (object, "Inv_Object_Multi", p.amber),
                (linked, "Inv_Object_Multi_Link", p.amber),
                (broken, "Inv_LinkItem", p.muted),
                (shirt, "Inv_Shirt", p.rose),
                (folder_link, "Inv_FolderClosed_Link", p.violet_light),
                (Row::Place(Location::Folder(id(3))), "Inv_FolderClosed", custom),
                (Row::Place(Location::Folder(id(4))), "Inv_FolderClosed", folder_tint),
                (Row::Place(Location::Folder(id(5))), "Inv_TrashClosed", folder_tint),
                (Row::Place(Location::Folder(id(7))), "Inv_SuitcaseClosed", folder_tint),
            ] {
                let out = rows(&ctx, &art, &inv, row, mode, selected, &HashMap::new(), None);
                assert!(
                    has_image(&out, icons.get(name).unwrap(), tint),
                    "{mode:?}, selected={selected}, {name}"
                );
            }
        }
    }
    assert_ne!(
        icons.get("Inv_FolderClosed").unwrap().id(),
        icons.get("folder-filled").unwrap().id()
    );
    assert_eq!(
        art.resolve(&inv, Row::Place(Location::Folder(id(3))), true, false),
        Glyph::navigation("Inv_FolderOpen", custom)
    );
}

#[test]
fn organized_thumbnail_type_and_shortcut_marks_follow_actual_images_and_permissions() {
    let ctx = egui::Context::default();
    let icons = Icons::load(&ctx, None);
    let p = crate::theme::Theme::default().palette();
    let prefs = InventoryPreferences::default();
    let facts = Facts::default();
    let art = RowStyle {
        palette: &p,
        icons: &icons,
        prefs: &prefs,
        facts: &facts,
    };
    let mut inv = super::super::tests::fixture();
    let asset = id(1818);
    let object = add_item(&mut inv, 120, 6, 6, 0, id(982));
    inv.items.get_mut(&id(120)).unwrap().thumbnail = asset;
    let link = add_item(&mut inv, 121, 24, 6, 0, id(120));
    let texture = add_item(&mut inv, 122, 0, 0, 0, asset);
    let thumb = ctx.load_texture(
        "qa fixture image",
        egui::ColorImage::filled([80, 40], Color32::LIGHT_BLUE),
        egui::TextureOptions::LINEAR,
    );
    let images = HashMap::from([(asset, thumb.clone())]);
    let large = ViewLayout::Icons(IconSize::Large);
    for (row, badge, tint) in [(object, "Inv_Object", p.amber), (link, "Inv_Object_Link", p.amber)] {
        let out = rows(&ctx, &art, &inv, row, large, false, &images, None);
        assert!(has_image(&out, &thumb, Color32::WHITE));
        assert!(has_image(&out, icons.get(badge).unwrap(), tint), "{badge}");
    }
    let plain = rows(&ctx, &art, &inv, texture, large, false, &images, None);
    assert!(has_image(&plain, &thumb, Color32::WHITE));
    assert!(!has_image(&plain, icons.get("Inv_Texture").unwrap(), p.indigo_light));
    let small = rows(&ctx, &art, &inv, link, ViewLayout::Icons(IconSize::Small), true, &images, None);
    assert!(has_image(&small, icons.get("Inv_LinkItem").unwrap(), p.ink));
    let empty = Icons::default();
    let without_icons = RowStyle { icons: &empty, ..art };
    let fallback = rows(&ctx, &without_icons, &inv, link, large, true, &images, None);
    assert!(has_image(&fallback, &thumb, Color32::WHITE));
    assert!(has_image(&fallback, &Icons::tabler(&ctx, "link-filled"), p.ink));
    let without_image = rows(&ctx, &without_icons, &inv, link, large, true, &HashMap::new(), None);
    assert!(has_image(&without_image, &Icons::tabler(&ctx, "link-filled"), p.ink));
    inv.rlva_preview_mask = 1;
    let hidden = rows(&ctx, &art, &inv, link, large, true, &images, None);
    assert!(!has_image(&hidden, &thumb, Color32::WHITE));
    assert!(has_image(&hidden, icons.get("Inv_Object_Link").unwrap(), p.amber));
}

#[test]
fn organized_search_results_paint_their_location_and_original_activity_and_favorite() {
    let ctx = egui::Context::default();
    let icons = Icons::load(&ctx, None);
    let palette = crate::theme::Theme::default().palette();
    let prefs = InventoryPreferences::default();
    let mut inv = super::super::tests::fixture();
    add_item(&mut inv, 130, 5, 18, 4, id(991));
    inv.items.get_mut(&id(130)).unwrap().favorite = true;
    let link = add_item(&mut inv, 131, 24, 18, 0, id(130));
    let mut facts = Facts::default();
    facts.worn.insert(id(130));
    facts.worn_labels.insert(id(130), "Worn status".into());
    let art = RowStyle {
        palette: &palette,
        icons: &icons,
        prefs: &prefs,
        facts: &facts,
    };
    let path = "Home / Archive";
    for mode in [ViewLayout::Details, ViewLayout::List, ViewLayout::Icons(IconSize::Large)] {
        let out = rows(&ctx, &art, &inv, link, mode, false, &HashMap::new(), Some(path));
        assert!(has_image(&out, icons.get("star-filled").unwrap(), palette.amber));
        let painted = |s: &str| {
            out.shapes
                .iter()
                .any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.job.text == s && shape.clip_rect.is_positive()))
        };
        assert!(painted(path), "{mode:?}");
        if !matches!(mode, ViewLayout::Icons(_)) {
            assert!(painted("Worn status"), "{mode:?}");
        }
    }
}

#[test]
fn organized_previews_prefer_assigned_images_and_only_fetch_real_material_base_color() {
    use crate::scene::meshes::MaterialStreamer;
    let ctx = egui::Context::default();
    let mut inv = super::super::tests::fixture();
    let texture = id(3100);
    let thumbnail = id(3101);
    let asset = id(3102);
    let object_asset = id(3103);
    let material = add_item(&mut inv, 110, 57, 26, 0, asset);
    let linked = add_item(&mut inv, 111, 24, 26, 0, id(110));
    let object = add_item(&mut inv, 112, 6, 6, 0, object_asset);
    let snapshot = add_item(&mut inv, 113, 0, 15, 0, texture);
    let dir = std::env::temp_dir().join(format!("inventory-swatch-test-{}", Uuid::new_v4()));
    let mut streamer = MaterialStreamer::new(dir.clone());
    let mut sample = aurora_assets::PbrMaterial {
        base_color_texture: Some(texture),
        normal_texture: Some(id(3150)),
        metallic_roughness_texture: Some(id(3151)),
        emissive_texture: Some(id(3152)),
        base_color_factor: [0.5, 0.0, 1.0, 1.0],
        ..Default::default()
    };
    streamer.insert(asset, sample.clone());
    let image = ctx.load_texture(
        "material or snapshot map",
        egui::ColorImage::filled([9, 6], Color32::WHITE),
        Default::default(),
    );
    let assigned = ctx.load_texture(
        "assigned thumbnail",
        egui::ColorImage::filled([8, 8], Color32::WHITE),
        Default::default(),
    );
    let images = HashMap::from([(texture, image.clone()), (thumbnail, assigned.clone())]);
    let mut tiles = MaterialTiles::default();
    let draw = |inv: &Inventory, tiles: &mut MaterialTiles, row| {
        let mut wanted = Vec::new();
        let mut seen = false;
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(60.0, 60.0));
            seen = picture(ui, inv, row, rect, &images, &mut wanted, tiles).is_some();
        });
        out.textures_delta.clear();
        (seen, wanted, out)
    };
    assert!(!draw(&inv, &mut tiles, material).0);
    tiles.update(inv.root, &mut streamer);
    let (seen, wanted, out) = draw(&inv, &mut tiles, linked);
    assert!(seen && has_image(&out, &image, Color32::from_rgb(188, 0, 255)));
    assert_eq!(wanted, [(texture, 60)]);
    assert!(has_image(&draw(&inv, &mut tiles, snapshot).2, &image, Color32::WHITE));
    let object_result = draw(&inv, &mut tiles, object);
    assert!(!object_result.0 && object_result.1.is_empty());
    for i in [110, 112] {
        inv.items.get_mut(&id(i)).unwrap().thumbnail = thumbnail;
    }
    for row in [linked, object] {
        let (seen, wanted, out) = draw(&inv, &mut tiles, row);
        assert!(seen && has_image(&out, &assigned, Color32::WHITE));
        assert_eq!(wanted, [(thumbnail, 60)]);
    }
    inv.items.get_mut(&id(110)).unwrap().thumbnail = Uuid::nil();
    sample.base_color_texture = None;
    streamer.insert(asset, sample);
    tiles.update(inv.root, &mut streamer);
    let (solid, wanted, _) = draw(&inv, &mut tiles, linked);
    assert!(solid && wanted.is_empty());
    inv.rlva_preview_mask = 1;
    for row in [linked, object, snapshot] {
        let (visible, wanted, _) = draw(&inv, &mut tiles, row);
        assert!(!visible && wanted.is_empty());
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn organized_tabler_artwork_keeps_exact_paint_rects_and_palette_tints() {
    let ctx = egui::Context::default();
    let p = crate::theme::Theme::default().palette();
    let rect = Rect::from_min_size(pos2(10.0, 20.0), vec2(18.0, 14.0));
    for name in [
        "x",
        "plus",
        "arrow-left",
        "arrow-right",
        "arrow-up",
        "arrow-down",
        "chevron-right",
        "chevron-down",
        "dots-filled",
        "link-filled",
        "help-circle-filled",
    ] {
        assert!(crate::ui::icons::embedded(name), "{name} must be a shipped Tabler resource");
        let texture = Icons::tabler(&ctx, name);
        assert_eq!(texture.id(), Icons::tabler(&ctx, name).id());
        for tint in [p.ink, p.muted_dim, p.violet_light, p.violet_pale, p.warn] {
            let out = ctx.run_ui(Default::default(), |ui| item_art::paint_tabler(ui.painter(), name, rect, tint));
            assert!(has_image(&out, &texture, tint), "{name}");
            assert!(
                out.shapes
                    .iter()
                    .any(|s| matches!(&s.shape, egui::Shape::Mesh(m) if m.texture_id == texture.id() && m.calc_bounds() == rect))
            );
        }
    }
    let out = ctx.run_ui(Default::default(), |ui| {
        Glyph::navigation("missing", p.warn).paint(ui.painter(), None, p.ink, rect)
    });
    assert!(has_image(&out, &Icons::tabler(&ctx, "help-circle-filled"), p.warn));
}

#[test]
fn organized_tabler_replacement_buttons_keep_text_glyph_hit_targets() {
    let ctx = egui::Context::default();
    for (old, name, small) in [
        ("…", "dots-filled", false),
        ("×", "x", true),
        ("✂", "cut", false),
        ("⧉", "copy-filled", false),
        ("✎", "pencil-filled", false),
        ("▤", "clipboard-text-filled", false),
    ] {
        let mut before = Rect::NOTHING;
        let mut after = Rect::NOTHING;
        let _ = ctx.run_ui(Default::default(), |ui| {
            let b = egui::Button::new(old);
            before = ui.add(if small { b.small() } else { b }).rect;
        });
        let _ = ctx.run_ui(Default::default(), |ui| {
            let b = egui::Button::new(item_art::glyph_image(ui, old, name, false, crate::ui::widgets::PICTURE_TINT))
                .image_tint_follows_text_color(true);
            after = ui.add(if small { b.small() } else { b }).rect;
        });
        assert_eq!(before, after, "{old}");
    }
}
