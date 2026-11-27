use super::adjust::Adjust;
use super::*;

/// A small picture whose every pixel differs.
fn sample(w: u32, h: u32) -> Picture {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&[(x * 37 % 256) as u8, (y * 53 % 256) as u8, ((x + y) * 11 % 256) as u8, 255]);
        }
    }
    Picture::new(w, h, rgba)
}

fn px(pic: &Picture, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * pic.w + x) * 4) as usize;
    [pic.rgba[i], pic.rgba[i + 1], pic.rgba[i + 2], pic.rgba[i + 3]]
}

#[test]
fn four_quarter_turns_and_two_flips_give_the_picture_back() {
    let pic = sample(7, 4);
    let mut turned = pic.clone();
    for _ in 0..4 {
        turned = geometry::apply(&turned, Edit::RotateRight);
    }
    assert_eq!(turned, pic);
    let one = geometry::apply(&pic, Edit::RotateRight);
    assert_eq!((one.w, one.h), (4, 7), "a quarter turn swaps the sides");
    assert_eq!(geometry::apply(&one, Edit::RotateLeft), pic, "left undoes right");
    for flip in [Edit::FlipHorizontal, Edit::FlipVertical] {
        let once = geometry::apply(&pic, flip);
        assert_ne!(once, pic);
        assert_eq!(geometry::apply(&once, flip), pic, "{flip:?} twice");
    }
    let flipped = geometry::apply(&pic, Edit::FlipHorizontal);
    assert_eq!(px(&flipped, 0, 0), px(&pic, 6, 0));
}

#[test]
fn a_crop_stays_inside_the_picture() {
    let pic = sample(10, 8);
    let crop = Crop {
        x: 0.2,
        y: 0.25,
        w: 0.5,
        h: 0.5,
    };
    let out = geometry::apply(&pic, Edit::Crop(crop));
    assert_eq!((out.w, out.h), (5, 4));
    assert_eq!(px(&out, 0, 0), px(&pic, 2, 2));
    // out of bounds: pulled back in, never empty
    let wild = Crop {
        x: 0.9,
        y: -3.0,
        w: 4.0,
        h: 0.0,
    };
    let c = wild.clamped();
    assert!(c.x >= 0.0 && c.x + c.w <= 1.0 + 1e-6 && c.y >= 0.0 && c.y + c.h <= 1.0 + 1e-6);
    let (x, y, w, h) = wild.pixels(10, 8);
    assert!(w >= 1 && h >= 1 && x + w <= 10 && y + h <= 8);
    assert_eq!(geometry::apply(&pic, Edit::Crop(Crop::FULL)), pic);
}

#[test]
fn a_resize_keeps_the_ratio_it_is_given_and_its_limits() {
    let pic = sample(40, 20);
    let half = geometry::apply(&pic, Edit::Resize { sx: 0.5, sy: 0.5 });
    assert_eq!((half.w, half.h), (20, 10));
    assert_eq!(geometry::resized(40, 20, 0.0, f32::NAN), (40, 20), "nonsense keeps the size");
    assert_eq!(geometry::resized(40, 20, 1e9, 1e-9), (geometry::MAX_SIDE, 1));
    let edits = [
        Edit::Crop(Crop {
            x: 0.0,
            y: 0.0,
            w: 0.5,
            h: 1.0,
        }),
        Edit::RotateRight,
        Edit::Resize { sx: 2.0, sy: 2.0 },
    ];
    assert_eq!(geometry::size_after(40, 20, &edits), (40, 40));
    let mut real = pic.clone();
    for e in edits {
        real = geometry::apply(&real, e);
    }
    assert_eq!((real.w, real.h), (40, 40), "the size shown is the size made");
}

#[test]
fn neutral_light_and_colour_change_nothing() {
    let pic = sample(9, 9);
    assert_eq!(adjust::apply(&pic, &Adjust::default()), pic);
    let tiny = Adjust {
        brightness: 0.0,
        contrast: 0.0,
        saturation: 0.0,
        warmth: 0.0,
        sharpen: -0.5,
    };
    assert_eq!(adjust::apply(&pic, &tiny), pic, "a negative sharpness is none");
}

#[test]
fn brightness_warmth_and_saturation_go_the_right_way() {
    let grey = Picture::new(1, 1, vec![128, 128, 128, 255]);
    let brighter = adjust::apply(
        &grey,
        &Adjust {
            brightness: 0.5,
            ..Adjust::default()
        },
    );
    assert!(px(&brighter, 0, 0)[0] > 128);
    let darker = adjust::apply(
        &grey,
        &Adjust {
            brightness: -0.5,
            ..Adjust::default()
        },
    );
    assert!(px(&darker, 0, 0)[0] < 128);
    let warm = px(
        &adjust::apply(
            &grey,
            &Adjust {
                warmth: 1.0,
                ..Adjust::default()
            },
        ),
        0,
        0,
    );
    assert!(warm[0] > 128 && warm[2] < 128, "warmer: more red, less blue {warm:?}");
    let red = Picture::new(1, 1, vec![200, 40, 40, 255]);
    let grey_red = px(
        &adjust::apply(
            &red,
            &Adjust {
                saturation: -1.0,
                ..Adjust::default()
            },
        ),
        0,
        0,
    );
    assert!(
        grey_red[0] == grey_red[1] && grey_red[1] == grey_red[2],
        "no colour left {grey_red:?}"
    );
    assert_eq!(px(&red, 0, 0)[3], 255, "alpha untouched");
}

#[test]
fn sharpening_raises_the_contrast_of_an_edge() {
    let mut rgba = Vec::new();
    for _y in 0..3 {
        for x in 0..4 {
            let v = if x < 2 { 80 } else { 170 };
            rgba.extend_from_slice(&[v, v, v, 255]);
        }
    }
    let edge = Picture::new(4, 3, rgba);
    let sharp = adjust::apply(
        &edge,
        &Adjust {
            sharpen: 1.0,
            ..Adjust::default()
        },
    );
    assert!(px(&sharp, 1, 1)[0] < 80 && px(&sharp, 2, 1)[0] > 170);
}

#[test]
fn undo_and_redo_play_the_same_recipe_again() {
    let pic = sample(12, 6);
    let mut history = History::default();
    let mut recipe = Recipe::default();
    history.push(recipe.clone());
    recipe.edits.push(Edit::RotateRight);
    history.push(recipe.clone());
    recipe.adjust.brightness = 0.3;
    let last = render(&pic, &recipe);
    let back = history.undo(&recipe).expect("one step back");
    assert_eq!(back.edits, vec![Edit::RotateRight]);
    assert!(back.adjust.is_neutral());
    let again = history.redo(&back).expect("redo");
    assert_eq!(render(&pic, &again), last, "the same bytes");
    assert!(history.redo(&again).is_none());
    // a new change forgets what was undone
    let back = history.undo(&again).expect("back");
    history.push(back.clone());
    assert!(history.undone.is_empty());
}

#[test]
fn a_flat_card_is_two_triangles_and_relief_follows_the_brightness() {
    let pic = demo_image(64, 32);
    let flat = relief::card(&pic, 0.0);
    let part = &flat.parts[0];
    assert_eq!((part.vertices.len(), part.indices.len()), (4, 6));
    assert!(part.vertices.iter().all(|v| v.position[0] == 0.0));
    let (nx, ny) = relief::cells(64, 32);
    assert_eq!((nx, ny), (relief::CELLS, relief::CELLS / 2));
    let raised = relief::card(&pic, 1.0);
    let part = &raised.parts[0];
    assert_eq!(part.vertices.len() as u32, (nx + 1) * (ny + 1));
    assert_eq!(part.indices.len() as u32, nx * ny * 6);
    assert!(part.indices.iter().all(|&i| (i as usize) < part.vertices.len()));
    for v in part.vertices.iter() {
        let n = glam::Vec3::from_array(v.normal);
        assert!((n.length() - 1.0).abs() < 1e-3, "unit normal");
        assert!(n.x > 0.0, "facing the viewer");
    }
    // the highest point is on the demo's bright disc (62%, 45% of the picture)
    let top = part
        .vertices
        .iter()
        .max_by(|a, b| a.position[0].total_cmp(&b.position[0]))
        .expect("vertices");
    assert!((top.uv[0] - 0.62).abs() < 0.12 && (top.uv[1] - 0.45).abs() < 0.2, "{:?}", top.uv);
    assert!(top.position[0] > 0.2);
    // the card keeps the picture's shape: twice as wide as high
    let ys = part.vertices.iter().map(|v| v.position[1]);
    let zs = part.vertices.iter().map(|v| v.position[2]);
    let width = ys.clone().fold(f32::MIN, f32::max) - ys.fold(f32::MAX, f32::min);
    let height = zs.clone().fold(f32::MIN, f32::max) - zs.fold(f32::MAX, f32::min);
    assert!((width / height - 2.0).abs() < 1e-3);
}

#[test]
fn a_saved_png_opens_again_the_same_and_a_huge_header_is_refused() {
    let dir = std::env::temp_dir().join(format!("aurora-picture-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let pic = sample(13, 7);
    let (path, format) = io::save_target(&dir.join("round"));
    assert_eq!(format, crate::snapshot::FileFormat::Png);
    let job = crate::snapshot::save::Job {
        width: pic.w,
        height: pic.h,
        rgba: pic.rgba.to_vec(),
        format,
        jpeg_quality: 90,
        path: path.clone(),
        alpha: true,
    };
    crate::snapshot::save::save(job, &std::sync::atomic::AtomicU32::new(0)).expect("saved");
    assert_eq!(io::load(&path).expect("opens"), pic);
    // a PNG header that claims 100000×100000 pixels
    let mut huge = Vec::new();
    huge.extend_from_slice(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR");
    huge.extend_from_slice(&100_000u32.to_be_bytes());
    huge.extend_from_slice(&100_000u32.to_be_bytes());
    huge.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    let bad = dir.join("huge.png");
    std::fs::write(&bad, huge).expect("written");
    assert!(io::load(&bad).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn save_names_and_formats() {
    use crate::snapshot::FileFormat;
    use std::path::Path;
    assert_eq!(io::save_target(Path::new("a/b.JPG")).1, FileFormat::Jpeg);
    assert_eq!(io::save_target(Path::new("a/b.bmp")).1, FileFormat::Bmp);
    assert_eq!(io::save_target(Path::new("a/b.tiff")).0, Path::new("a/b.png"));
    assert_eq!(io::suggested_name(Path::new("c/holiday.jpg")), "holiday-edited.png");
    let small = io::working_copy(&sample(40, 10), 20);
    assert_eq!((small.w, small.h), (20, 5));
}
