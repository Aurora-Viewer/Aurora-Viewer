use super::*;
use std::sync::atomic::AtomicU32;

#[test]
fn presets_give_their_long_side_with_the_proportions() {
    let win = (2560, 1369);
    assert_eq!(photo_size(Aspect::R16x9, Resolution::Uhd, [0, 0], win), (3840, 2160));
    assert_eq!(photo_size(Aspect::R4x5, Resolution::Uhd, [0, 0], win), (3072, 3840));
    assert_eq!(photo_size(Aspect::R9x16, Resolution::Hd, [0, 0], win), (1080, 1920));
    assert_eq!(photo_size(Aspect::R1x1, Resolution::K8, [0, 0], win), (7680, 7680));
    assert_eq!(photo_size(Aspect::R3x2, Resolution::K5, [0, 0], win), (5120, 3412));
    // the window's own size, its proportions
    assert_eq!(photo_size(Aspect::Window, Resolution::Window, [0, 0], (1600, 900)), (1600, 900));
    assert_eq!(photo_size(Aspect::Window, Resolution::Uhd, [0, 0], (1600, 900)), (3840, 2160));
    // the window's long side with other proportions
    assert_eq!(photo_size(Aspect::R1x1, Resolution::Window, [0, 0], (1600, 900)), (1600, 1600));
}

#[test]
fn sizes_stay_even_and_within_firestorms_limits() {
    let (w, h) = photo_size(Aspect::R16x9, Resolution::Custom, [99_999, 7], (800, 600));
    assert_eq!((w, h), (MAX_SIDE, MIN_SIDE));
    let (w, h) = photo_size(Aspect::Window, Resolution::Window, [0, 0], (1001, 667));
    assert_eq!((w % 2, h % 2), (0, 0));
    assert_eq!((w, h), (1000, 666));
}

#[test]
fn typed_sizes_keep_the_proportions_when_asked() {
    let mut s = Settings {
        aspect: Aspect::R16x9,
        resolution: Resolution::Custom,
        ..Default::default()
    };
    s.set_custom(Some(3200), None, (800, 600));
    assert_eq!(s.custom, [3200, 1800]);
    s.set_custom(None, Some(900), (800, 600));
    assert_eq!(s.custom, [1600, 900]);
    s.keep_ratio = false;
    s.set_custom(Some(1000), None, (800, 600));
    assert_eq!(s.custom, [1000, 900]);
    // the window's proportions for "Window"
    s.aspect = Aspect::Window;
    s.keep_ratio = true;
    s.set_custom(Some(1200), None, (800, 600));
    assert_eq!(s.custom, [1200, 900]);
}

#[test]
fn the_interface_needs_the_windows_proportions() {
    let win = (1920, 1080);
    let mut s = Settings::default();
    assert!(s.interface_possible(win));
    s.resolution = Resolution::K8;
    assert!(s.interface_possible(win));
    s.aspect = Aspect::R16x9;
    assert!(s.interface_possible(win));
    s.aspect = Aspect::R4x5;
    assert!(!s.interface_possible(win));
}

#[test]
fn one_pass_while_the_card_holds_it_tiles_otherwise() {
    assert_eq!(
        plan((7680, 4320), 16384, 64_000_000),
        vec![Tile {
            x: 0,
            y: 0,
            width: 7680,
            height: 4320
        }]
    );
    // a side over the card's limit
    let tiles = plan((7680, 4320), 4096, u64::MAX);
    assert_eq!(tiles.len(), 4);
    assert!(tiles.iter().all(|t| t.width <= 4096 && t.height <= 4096));
    // too many pixels for the memory
    let tiles = plan((7680, 4320), 16384, 10_000_000);
    assert!(tiles.len() >= 4);
    assert!(tiles.iter().all(|t| u64::from(t.width) * u64::from(t.height) <= 10_000_000));
    // the tiles cover the photo exactly once
    let mut covered = vec![0u8; 7680 * 4320];
    for t in &tiles {
        for y in t.y..t.y + t.height {
            for x in t.x..t.x + t.width {
                covered[(y * 7680 + x) as usize] += 1;
            }
        }
    }
    assert!(covered.iter().all(|&c| c == 1));
}

#[test]
fn a_tile_projection_maps_its_part_of_the_photo_to_the_whole_target() {
    let size = (1000, 500);
    let whole = Tile {
        x: 0,
        y: 0,
        width: 1000,
        height: 500,
    };
    assert!(tile_matrix(size, whole).abs_diff_eq(Mat4::IDENTITY, 1e-6));
    // the right half, bottom half: the photo's pixel (750, 375) is the
    // tile's centre
    let tile = Tile {
        x: 500,
        y: 250,
        width: 500,
        height: 250,
    };
    let m = tile_matrix(size, tile);
    let ndc = |px: f32, py: f32| Vec4::new(2.0 * px / 1000.0 - 1.0, 1.0 - 2.0 * py / 500.0, 0.5, 1.0);
    let c = m * ndc(750.0, 375.0);
    assert!(c.x.abs() < 1e-5 && c.y.abs() < 1e-5);
    let corner = m * ndc(500.0, 250.0);
    assert!((corner.x + 1.0).abs() < 1e-5 && (corner.y - 1.0).abs() < 1e-5);
    // depth and w untouched (reverse-Z stays)
    assert!((c.z - 0.5).abs() < 1e-6 && (c.w - 1.0).abs() < 1e-6);
}

#[test]
fn tiles_stitch_into_the_photo() {
    let mut photo = vec![0u8; 4 * 3 * 4];
    let tile = Tile {
        x: 2,
        y: 1,
        width: 2,
        height: 2,
    };
    stitch(&mut photo, 4, tile, &[1; 16]);
    for y in 0..3 {
        for x in 0..4 {
            let v = photo[(y * 4 + x) * 4];
            assert_eq!(v, u8::from(x >= 2 && y >= 1), "({x}, {y})");
        }
    }
}

#[test]
fn focal_length_and_field_of_view_agree() {
    // 50 mm on full frame: 27 degrees vertically
    assert!((fov_of_focal(50.0).to_degrees() - 26.99).abs() < 0.05);
    for mm in [14.0, 24.0, 35.0, 85.0, 135.0] {
        assert!((focal_of_fov(fov_of_focal(mm)) - mm).abs() < 1e-3);
    }
    // the viewer's default (60 degrees) is about 21 mm
    assert!((focal_of_fov(std::f32::consts::FRAC_PI_3) - 20.78).abs() < 0.05);
}

#[test]
fn file_names_follow_firestorm() {
    assert_eq!(file_name("Snapshot", None, 1, FileFormat::Png), "Snapshot_001.png");
    assert_eq!(
        file_name("Snapshot", Some("2026-10-14_093005"), 12, FileFormat::Jpeg),
        "Snapshot_2026-10-14_093005_012.jpg"
    );
    assert_eq!(file_name("a/b:c", None, 3, FileFormat::Bmp), "a_b_c_003.bmp");
    assert_eq!(FileFormat::of_path(Path::new("x/Photo.JPEG")), Some(FileFormat::Jpeg));
    assert_eq!(FileFormat::of_path(Path::new("x/Photo.tga")), None);
    let stamp = local_stamp();
    assert_eq!(stamp.len(), "2026-10-14_093005".len());
    assert_eq!(&stamp[4..5], "-");
    assert_eq!(&stamp[10..11], "_");
}

#[test]
fn free_names_skip_the_taken_ones() {
    let dir = std::env::temp_dir().join(format!("aurora-photo-names-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let first = free_path(&dir, "Snapshot", None, FileFormat::Png);
    assert!(first.ends_with("Snapshot_001.png"));
    std::fs::write(&first, b"x").unwrap();
    assert!(free_path(&dir, "Snapshot", None, FileFormat::Png).ends_with("Snapshot_002.png"));
    // another format does not take the number
    assert!(free_path(&dir, "Snapshot", None, FileFormat::Jpeg).ends_with("Snapshot_001.jpg"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn saved_photos_read_back_in_every_format() {
    let dir = std::env::temp_dir().join(format!("aurora-photo-save-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (w, h) = (300u32, 170u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 128, 255]);
        }
    }
    for format in FileFormat::ALL {
        let progress = AtomicU32::new(0);
        let path = dir.join(format!("p.{}", format.extension()));
        let saved = save::save(
            save::Job {
                width: w,
                height: h,
                rgba: rgba.clone(),
                format,
                jpeg_quality: 95,
                path: path.clone(),
            },
            &progress,
        )
        .unwrap();
        assert_eq!(saved.path, path);
        assert_eq!(progress.load(std::sync::atomic::Ordering::Relaxed), 10_000);
        assert!(!dir.join(format!("p.{}.part", format.extension())).exists());
        let back = image::open(&path).unwrap().to_rgb8();
        assert_eq!((back.width(), back.height()), (w, h));
        let px = back.get_pixel(200, 100);
        let tolerance = if format == FileFormat::Jpeg { 6 } else { 0 };
        assert!(
            px[0].abs_diff(200) <= tolerance && px[1].abs_diff(100) <= tolerance,
            "{format:?} {px:?}"
        );
        assert!(saved.thumb.0 <= save::THUMB_SIDE && saved.thumb.1 <= save::THUMB_SIDE);
        assert_eq!(saved.thumb.2.len(), (saved.thumb.0 * saved.thumb.1 * 4) as usize);
    }
    let recent = gallery::recent(&dir, 10);
    assert_eq!(recent.len(), 3);
    assert!(gallery::load_thumbnail(&recent[0]).is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_wrong_buffer_is_refused_without_a_file() {
    let dir = std::env::temp_dir().join(format!("aurora-photo-bad-{}", std::process::id()));
    let path = dir.join("bad.png");
    let r = save::save(
        save::Job {
            width: 10,
            height: 10,
            rgba: vec![0; 12],
            format: FileFormat::Png,
            jpeg_quality: 75,
            path: path.clone(),
        },
        &AtomicU32::new(0),
    );
    assert!(r.is_err());
    assert!(!path.exists());
}

#[test]
fn guides_draw_inside_the_frame() {
    for g in Guide::ALL {
        let lines = g.lines();
        assert!(!lines.is_empty());
        assert!(lines.iter().flatten().flatten().all(|v| (0.0..=1.0).contains(v)));
    }
    assert!(Guide::None.lines().is_empty());
}

#[test]
fn loaded_settings_are_brought_back_into_range() {
    let mut s = Settings {
        custom: [5, 100_000],
        jpeg_quality: 250,
        base_name: "  ".into(),
        ..Default::default()
    };
    s.sanitize();
    assert_eq!(s.custom, [MIN_SIDE, MAX_SIDE]);
    assert_eq!(s.jpeg_quality, 100);
    assert_eq!(s.base_name, "Snapshot");
}
