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
fn a_new_format_reshapes_a_typed_size_kept_in_proportion() {
    let mut s = Settings {
        resolution: Resolution::Custom,
        custom: [3200, 1800],
        keep_ratio: true,
        ..Settings::default()
    };
    s.set_aspect(Aspect::R1x1, (1600, 900));
    assert_eq!(s.custom, [3200, 3200]);
    s.set_aspect(Aspect::R4x5, (1600, 900));
    assert_eq!(s.custom, [2560, 3200]);
    assert_eq!(photo_size(s.aspect, s.resolution, s.custom, (1600, 900)), (2560, 3200));
    // the size typed freely stays as it is
    s.keep_ratio = false;
    s.set_aspect(Aspect::R16x9, (1600, 900));
    assert_eq!(s.custom, [2560, 3200]);
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
    assert!(same_proportions(s.size(win), win));
    s.resolution = Resolution::K8;
    assert!(same_proportions(s.size(win), win));
    s.aspect = Aspect::R16x9;
    assert!(same_proportions(s.size(win), win));
    s.aspect = Aspect::R4x5;
    assert!(!same_proportions(s.size(win), win));
}

#[test]
fn one_pass_while_the_card_holds_it_tiles_otherwise() {
    let whole = Tile {
        x: 0,
        y: 0,
        width: 7680,
        height: 4320,
    };
    assert_eq!(
        plan((7680, 4320), 16384, 64_000_000, TILE_GUARD),
        vec![TilePass { keep: whole, draw: whole }]
    );
    // just within the limits: still one pass, no guard band
    assert_eq!(
        plan((7680, 4320), 7680, 7680 * 4320, TILE_GUARD),
        vec![TilePass { keep: whole, draw: whole }]
    );
    // a side over the card's limit
    // (2 x 2 tiles of 3840 keep room for their one band each: 3968 drawn)
    let tiles = plan((7680, 4320), 4096, u64::MAX, TILE_GUARD);
    assert_eq!(tiles.len(), 4);
    assert!(tiles.iter().all(|t| t.draw.width <= 4096 && t.draw.height <= 4096));
    // too many pixels for the memory
    let tiles = plan((7680, 4320), 16384, 10_000_000, TILE_GUARD);
    assert!(tiles.len() >= 4);
    assert!(
        tiles
            .iter()
            .all(|t| u64::from(t.draw.width) * u64::from(t.draw.height) <= 10_000_000)
    );
    // the tiles kept cover the photo exactly once
    let mut covered = vec![0u8; 7680 * 4320];
    for t in &tiles {
        let t = t.keep;
        for y in t.y..t.y + t.height {
            for x in t.x..t.x + t.width {
                covered[(y * 7680 + x) as usize] += 1;
            }
        }
    }
    assert!(covered.iter().all(|&c| c == 1));
    // down to the smallest limits, every pass fits
    for (max_side, max_pixels) in [(32, 0), (100, 5_000), (1024, 1_000_000), (16384, 2_000)] {
        let tiles = plan((3200, 1800), max_side, max_pixels, TILE_GUARD);
        let max_pixels = max_pixels.max(32 * 32);
        assert!(
            tiles.iter().all(|t| t.draw.width <= max_side.max(32)
                && t.draw.height <= max_side.max(32)
                && u64::from(t.draw.width) * u64::from(t.draw.height) <= max_pixels),
            "{max_side} {max_pixels}"
        );
    }
}

#[test]
fn a_card_over_its_budget_still_draws_passes_of_the_windows_size() {
    // half of the free memory at 88 bytes a pixel (MSAA 4x)
    assert_eq!(pass_pixels(880_000_000, 88, (1318, 745)), 10_000_000);
    // nothing free (1,818 MB used of a 1,248 MB budget): the window's frame
    assert_eq!(pass_pixels(0, 88, (1318, 745)), 1318 * 745);
    assert_eq!(pass_pixels(0, 0, (1920, 1080)), 1920 * 1080);
    // a photo of the window at twice its size (the photo quality) takes a
    // handful of passes, not the thousands of 32-pixel tiles drawn before
    let tiles = plan((2636, 1490), 16384, pass_pixels(0, 88, (1318, 745)), TILE_GUARD);
    assert!(tiles.len() <= 16, "{}", tiles.len());
    let tiles = plan((3840, 2160), 16384, pass_pixels(0, 88, (1920, 1080)), TILE_GUARD);
    assert!(tiles.len() <= 16, "{}", tiles.len());
    assert!(plan((2636, 1490), 16384, 0, TILE_GUARD).len() > 1000);
}

#[test]
fn a_tile_is_drawn_with_a_guard_band_on_the_sides_it_shares() {
    // 3200 x 1800 with sides of 1024 at most: 5 x 3 tiles of 640 x 600,
    // drawn with up to 128 pixels more on each inner side
    let tiles = plan((3200, 1800), 1024, u64::MAX, TILE_GUARD);
    assert_eq!(tiles.len(), 15);
    let corner = tiles[0];
    assert_eq!(
        corner.keep,
        Tile {
            x: 0,
            y: 0,
            width: 640,
            height: 600
        }
    );
    // the photo's own edges get no band
    assert_eq!(
        corner.draw,
        Tile {
            x: 0,
            y: 0,
            width: 640 + 128,
            // 728, rounded to the alignment
            height: 768
        }
    );
    let middle = tiles[5 + 1];
    assert_eq!((middle.keep.x, middle.keep.y), (640, 600));
    assert_eq!(
        middle.draw,
        Tile {
            x: 512,
            // 472 and 1328, on the alignment
            y: 448,
            width: 128 + 640 + 128,
            height: 1344 - 448
        }
    );
    let last = tiles[14];
    assert_eq!((last.draw.x + last.draw.width, last.draw.y + last.draw.height), (3200, 1800));
    for t in &tiles {
        // what is kept lies in what is drawn, from a multiple of 64
        assert!(t.draw.x <= t.keep.x && t.keep.x + t.keep.width <= t.draw.x + t.draw.width);
        assert!(t.draw.y <= t.keep.y && t.keep.y + t.keep.height <= t.draw.y + t.draw.height);
        assert!(t.draw.x % 64 == 0 && t.draw.y % 64 == 0);
    }
    // an odd tile origin moves the drawn one to the multiple of 64 before,
    // its end to the one after
    let odd = plan((3003, 1800), 1024, u64::MAX, TILE_GUARD)[1];
    assert_eq!((odd.keep.x, odd.draw.x, odd.draw.width), (601, 448, 1344 - 448));
}

#[test]
fn the_bloom_widens_the_guard_band_and_the_tiles_fall_on_its_levels() {
    // an 8K photo with sides of 4096 at most and the bloom's band
    let guard = 384;
    let tiles = plan((7680, 4320), 4096, u64::MAX, guard);
    assert_eq!(tiles.len(), 6);
    for t in &tiles {
        let (k, d) = (t.keep, t.draw);
        assert!(d.width <= 4096 && d.height <= 4096);
        // the whole band on every side shared with another tile
        assert!(k.x == 0 || k.x - d.x >= guard);
        assert!(k.y == 0 || k.y - d.y >= guard);
        assert!(k.x + k.width == 7680 || d.x + d.width - (k.x + k.width) >= guard);
        assert!(k.y + k.height == 4320 || d.y + d.height - (k.y + k.height) >= guard);
        // the edges on the bloom's last level, but the photo's own
        assert!(d.x % 64 == 0 && d.y % 64 == 0);
        assert!(d.x + d.width == 7680 || d.width % 64 == 0);
        assert!(d.y + d.height == 4320 || d.height % 64 == 0);
    }
    // a band wider than a quarter of what a pass draws is narrowed
    let tiles = plan((3200, 1800), 1024, u64::MAX, 384);
    assert!(tiles.iter().all(|t| t.draw.width <= 1024 && t.draw.height <= 1024));
    let middle = tiles.iter().find(|t| t.keep.x > 0 && t.keep.y > 0).expect("an inner tile");
    assert!(middle.keep.x - middle.draw.x >= 256 && middle.keep.x - middle.draw.x < 256 + 64);
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
fn tiles_stitch_into_the_photo_without_their_guard_band() {
    let mut photo = vec![0u8; 4 * 3 * 4];
    let pass = TilePass {
        keep: Tile {
            x: 2,
            y: 1,
            width: 2,
            height: 2,
        },
        draw: Tile {
            x: 1,
            y: 0,
            width: 3,
            height: 3,
        },
    };
    // the drawn pixels hold their column + 10 * their row in the photo
    let drawn: Vec<u8> = (0..3).flat_map(|y| (1..4).flat_map(move |x| [x + 10 * y; 4])).collect();
    stitch(&mut photo, 4, pass, &drawn);
    for y in 0..3u8 {
        for x in 0..4u8 {
            let v = photo[usize::from(y * 4 + x) * 4];
            let expect = if x >= 2 && y >= 1 { x + 10 * y } else { 0 };
            assert_eq!(v, expect, "({x}, {y})");
        }
    }
}

#[test]
fn every_tile_of_a_photo_shows_the_instant_of_its_first() {
    let t0 = std::time::Instant::now();
    let at = |ms: u64, clock: f32| Hold {
        at: t0 + std::time::Duration::from_millis(ms),
        clock,
        later: false,
    };
    let mut kept = None;
    assert_eq!(Hold::for_part(1, 0, &mut kept, at(0, 1.0)), None, "one pass: the live world");
    assert_eq!(kept, None);
    // the first part is drawn from the live world, its instant kept
    let first = Hold::for_part(4, 0, &mut kept, at(16, 2.0));
    assert_eq!(first, Some(at(16, 2.0)));
    // the later parts show it, still
    for (next, ms) in [(1, 33), (2, 50), (3, 66)] {
        let hold = Hold::for_part(4, next, &mut kept, at(ms, 9.0));
        assert_eq!(
            hold,
            Some(Hold {
                later: true,
                ..at(16, 2.0)
            }),
            "part {next}"
        );
    }
    // a first part drawn again (its frame lost) takes its own instant
    assert_eq!(Hold::for_part(4, 0, &mut kept, at(80, 3.0)), Some(at(80, 3.0)));
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
fn local_fields_are_the_zone_s_own() {
    use chrono::{FixedOffset, TimeZone};
    // the Unix epoch seen from Tokyo, then from New York (the day before)
    let tokyo = FixedOffset::east_opt(9 * 3600).unwrap().timestamp_opt(0, 0).unwrap();
    assert_eq!(civil(&tokyo), (1970, 1, 1, 9, 0, 0));
    let new_york = FixedOffset::west_opt(5 * 3600).unwrap().timestamp_opt(0, 0).unwrap();
    assert_eq!(civil(&new_york), (1969, 12, 31, 19, 0, 0));
    let leap = FixedOffset::east_opt(5 * 3600 + 45 * 60)
        .unwrap()
        .with_ymd_and_hms(2028, 2, 29, 23, 59, 58);
    assert_eq!(civil(&leap.unwrap()), (2028, 2, 29, 23, 59, 58));
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
                alpha: false,
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
            alpha: false,
        },
        &AtomicU32::new(0),
    );
    assert!(r.is_err());
    assert!(!path.exists());
}

#[test]
fn a_cut_out_photo_keeps_its_alpha_in_the_png_and_its_colour_when_reduced() {
    let dir = std::env::temp_dir().join(format!("aurora-photo-cutout-{}", std::process::id()));
    let path = dir.join("cutout.png");
    // left half a red subject, a half covered column, the rest empty
    let (w, h) = (4u32, 2u32);
    let mut rgba = Vec::new();
    for _ in 0..h {
        rgba.extend_from_slice(&[255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 128, 0, 0, 0, 0]);
    }
    save::save(
        save::Job {
            width: w,
            height: h,
            rgba: rgba.clone(),
            format: FileFormat::Png,
            jpeg_quality: 75,
            path: path.clone(),
            alpha: true,
        },
        &AtomicU32::new(0),
    )
    .unwrap();
    let back = image::open(&path).unwrap().to_rgba8();
    assert_eq!(back.get_pixel(2, 1).0, [255, 0, 0, 128]);
    assert_eq!(back.get_pixel(3, 0).0, [0, 0, 0, 0]);
    // halved: the empty pixels take nothing from the subject's colour
    let (_, _, half) = save::downscale(w, h, &rgba, 2);
    assert_eq!(&half[..4], &[255, 0, 0, 255]);
    assert_eq!(&half[4..8], &[255, 0, 0, 64]);
    let _ = std::fs::remove_dir_all(&dir);
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

#[test]
fn inventory_photos_become_power_of_two_textures_like_firestorm() {
    use inventory::{TextureSize, capture_size, texture_size};
    let win = (1600, 900);
    // the square presets of panel_snapshot_inventory.xml
    assert_eq!(capture_size(Aspect::R1x1, TextureSize::S512, win), (512, 512));
    assert_eq!(texture_size((512, 512)), (512, 512));
    assert_eq!(texture_size(capture_size(Aspect::R1x1, TextureSize::S2048, win)), (2048, 2048));
    // other proportions: each side biased down unless over 1.75 times the
    // lower power of two (LLImageRaw::biasedDimToPowerOfTwo)
    assert_eq!(capture_size(Aspect::R16x9, TextureSize::S1024, win), (1024, 576));
    assert_eq!(texture_size((1024, 576)), (1024, 512));
    assert_eq!(texture_size((1600, 900)), (1024, 1024));
    // "Current Window": its size, never over 2048
    assert_eq!(capture_size(Aspect::Window, TextureSize::Window, win), (1600, 900));
    assert_eq!(capture_size(Aspect::Window, TextureSize::Window, (3840, 2160)), (2048, 1152));
    assert_eq!(texture_size((2048, 1152)), (2048, 1024));
}

#[test]
fn inventory_photos_are_named_after_the_place() {
    use inventory::{item_description, item_name, location};
    let place = location("Place", "Region", glam::Vec3::new(128.4, 64.6, 25.0), "General");
    assert_eq!(place, "Place, Region (128, 65, 25) - General");
    assert_eq!(location(" ", "Region", glam::Vec3::ZERO, ""), "Region (0, 0, 0)");
    assert_eq!(item_name(&place), "Snapshot : Place, Region (128, 65, 25) - General");
    assert_eq!(
        item_description("Resident", &place),
        "Taken by Resident at Place, Region (128, 65, 25) - General"
    );
    // the server's limits, cut on a character
    let long = location(&"é".repeat(80), "Region", glam::Vec3::ZERO, "");
    assert!(item_name(&long).len() <= 63);
    assert!(item_description("Resident", &long).len() <= 127);
    assert!(item_name(&long).is_char_boundary(item_name(&long).len()));
}

#[test]
fn inventory_photos_encode_to_a_texture_of_their_size() {
    let (w, h) = (300u32, 170u32);
    let rgba: Vec<u8> = (0..w * h)
        .flat_map(|i| [if i / w < h / 2 { 220 } else { 30 }, if i % w < w / 2 { 40 } else { 200 }, 90, 255])
        .collect();
    let texture = inventory::texture_size((w, h));
    assert_eq!(texture, (256, 128));
    let data = inventory::encode(w, h, &rgba, texture).expect("encode");
    let info = aurora_assets::j2k_info(&data).expect("J2K");
    assert_eq!((info.width, info.height), texture);
    let decoded = aurora_assets::decode_j2k(&data, 0).expect("decoded photo");
    for (x, y, expected) in [
        (16, 16, [220, 40, 90]),
        (240, 16, [220, 200, 90]),
        (16, 112, [30, 40, 90]),
        (240, 112, [30, 200, 90]),
    ] {
        let offset = (y * texture.0 as usize + x) * 3;
        for (actual, expected) in decoded.data[offset..offset + 3].iter().zip(expected) {
            assert!(actual.abs_diff(expected) <= 15, "photo orientation at ({x}, {y})");
        }
    }
    assert!(inventory::encode(w, h, &rgba[..10], texture).is_err());
}

#[test]
fn profile_posts_escape_their_caption_like_lluri() {
    assert_eq!(profile::escape("Coucher de soleil é&=?"), "Coucher%20de%20soleil%20%C3%A9%26%3D%3F");
    assert_eq!(profile::escape("a-b_c.d~e"), "a-b_c.d~e");
    // getProfileURL(""): lowercase, ends with a slash
    assert_eq!(
        profile::profile_url("https://My.SecondLife.com").as_deref(),
        Some("https://my.secondlife.com/")
    );
    assert_eq!(profile::profile_url("  "), None);
    assert_eq!(
        profile::grid_web_profile(&crate::settings::GridChoice::SecondLife, "").as_deref(),
        Some("https://my.secondlife.com/")
    );
    assert_eq!(profile::grid_web_profile(&crate::settings::GridChoice::Custom, ""), None);
}

#[test]
fn profile_photos_take_firestorms_sizes() {
    use profile::{ProfileSize, capture_size};
    assert_eq!(capture_size(Aspect::R4x3, ProfileSize::S640, (1600, 900)), (640, 480));
    assert_eq!(capture_size(Aspect::R4x3, ProfileSize::S1024, (1600, 900)), (1024, 768));
    assert_eq!(capture_size(Aspect::Window, ProfileSize::S800, (1600, 900)), (800, 450));
    assert_eq!(capture_size(Aspect::Window, ProfileSize::Window, (1600, 900)), (1600, 900));
}

#[test]
fn the_profile_form_keeps_firestorms_fields_and_order() {
    let data = serde_json::json!({"key": "k", "AWSAccessKeyId": "id", "acl": "public-read", "Content-Type": "image/png",
        "policy": "p", "signature": "s", "success_action_redirect": "https://x/done"});
    let body = profile::form(&data, b"PNGDATA");
    let text = String::from_utf8_lossy(&body);
    let names: Vec<&str> = text.split("; name=\"").skip(1).filter_map(|s| s.split('"').next()).collect();
    assert_eq!(names, profile::demo::FIELDS);
    // buildPostData's boundary
    let delimiter = format!("--{}", "----------------------------0123abcdefab");
    assert!(text.starts_with(&format!("{delimiter}\r\n")));
    assert!(text.contains("filename=\"snapshot.png\"\r\nContent-Type: image/png\r\n\r\nPNGDATA\r\n"));
    assert!(text.ends_with(&format!("\r\n{delimiter}--\r\n")));
}

/// The whole post against the demo's web profile: the session cookie for
/// the OpenID cookie, the configuration with the caption, the form and its
/// 303, as LLWebProfile::uploadImageCoro.
#[test]
fn a_photo_is_posted_to_the_web_profile() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let server = profile::demo::start(false).expect("demo profile");
    let web = profile::Web {
        profile: server.url.clone(),
        openid: profile::demo::OPENID.into(),
        user_agent: "test".into(),
    };
    let rgba: Vec<u8> = (0..64 * 48).flat_map(|i| [(i % 256) as u8, 40, 90, 255]).collect();
    let png = profile::encode_png(64, 48, &rgba).expect("PNG");
    assert!(profile::encode_png(64, 48, &rgba[..8]).is_err());
    rt.block_on(async {
        let auth = profile::auth_cookie(&web).await.expect("auth cookie");
        assert_eq!(auth, "_my_session=demo-auth");
        profile::post(&web, &auth, png.clone(), "Sunset & sea", true).await.expect("posted");
        // without the OpenID cookie the profile gives no session
        let stranger = profile::Web {
            openid: "other=1".into(),
            ..web.clone()
        };
        assert_eq!(profile::auth_cookie(&stranger).await, Err(profile::Error::NoCookie));
    });
    let posts = server.posts.lock();
    let post = posts.last().expect("a post");
    assert_eq!(post.caption, "Sunset & sea");
    assert!(post.add_location && post.finished);
    assert_eq!(post.fields, profile::demo::FIELDS);
    assert_eq!(post.png, png);
    // both cookies, like buildDefaultHeaders + the auth cookie
    assert!(post.cookies.iter().any(|c| c.contains(profile::demo::OPENID)));
    assert!(post.cookies.iter().any(|c| c.contains("_my_session=demo-auth")));
}

#[test]
fn a_refused_configuration_stops_the_post() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let server = profile::demo::start(true).expect("demo profile");
    let web = profile::Web {
        profile: server.url.clone(),
        openid: profile::demo::OPENID.into(),
        user_agent: "test".into(),
    };
    let result = rt.block_on(profile::post(&web, "_my_session=demo-auth", vec![1, 2, 3], "", false));
    assert_eq!(result, Err(profile::Error::Config(500)));
}
