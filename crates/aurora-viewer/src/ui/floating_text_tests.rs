use super::*;
use crate::scene::{Scene, avatar::AvatarLibrary};
use crate::ui::{fonts, hud};
use crate::world::World;
use egui::{Shape, epaint::TextShape};

fn context(font: &str, scale: f32) -> egui::Context {
    let ctx = egui::Context::default();
    fonts::apply(&ctx, font);
    fonts::set_scale(&ctx, scale);
    ctx
}

fn input(native: f32) -> egui::RawInput {
    let mut raw = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1000.0) / native)),
        ..Default::default()
    };
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native);
    raw
}

// Read the glyph transform from egui's installed definitions, independently
// of the viewer's metrics cache. This is the offset actually put in its UVs.
fn glyph_shift(ctx: &egui::Context, size: f32) -> f32 {
    ctx.fonts(|fonts| {
        let defs = fonts.definitions();
        let name = &defs.families[&egui::FontFamily::Proportional][0];
        let tweak = &defs.font_data[name].tweak;
        (size * tweak.scale * tweak.y_offset_factor + tweak.y_offset).round_ui()
    })
}

fn text_shapes(mut output: egui::FullOutput) -> Vec<TextShape> {
    // Headless fixtures inspect geometry without uploading textures to a
    // renderer. Explicitly discard those deltas, as egui requires on drop.
    output.textures_delta.clear();
    output
        .shapes
        .into_iter()
        .filter_map(|shape| match shape.shape {
            Shape::Text(text) => Some(text),
            _ => None,
        })
        .collect()
}

fn baselines(shape: &TextShape, shift: f32) -> Vec<f32> {
    shape
        .galley
        .rows
        .iter()
        .map(|row| shape.pos.y + row.pos.y + row.glyphs.first().expect("fixture row has a character").pos.y + shift)
        .collect()
}

fn block(text: &str, font: &str, scale: f32, native: f32, zoom: f32, color: Color32) -> (Vec<TextShape>, f32) {
    let ctx = context(font, scale);
    let mut shift = 0.0;
    let output = ctx.run_ui(input(native), |ui| {
        shift = glyph_shift(ui.ctx(), 12.0);
        paint(
            &ui.ctx().layer_painter(egui::LayerId::background()),
            text.to_owned(),
            egui::pos2(800.0, 500.0),
            12.0,
            color,
            zoom,
            None,
        );
    });
    (text_shapes(output), shift)
}

#[test]
fn a_single_line_uses_the_legacy_baseline_not_the_bottom_of_its_box() {
    // LLHUDText updateSize/renderText leaves 12 after subtracting its rows;
    // hud_render_text draws at y=1. Baseline: 500 - (12 + 1) = 487.
    for font in fonts::BUILTINS {
        for scale in [0.8, 1.0, 1.5, 2.0] {
            for native in [1.0, 1.25, 1.5, 2.0] {
                let (shapes, shift) = block("Hg", font.name, scale, native, 1.0, Color32::WHITE);
                let text = shapes.iter().find(|text| text.override_text_color.is_none()).expect("text painted");
                assert!(
                    (baselines(text, shift)[0] - 487.0).abs() * native <= 0.501,
                    "{} size={scale} DPI={native}",
                    font.name
                );
            }
        }
    }
}

#[test]
fn line_steps_match_freetype_ascent_plus_descent_minus_one() {
    // Independently measured FreeType ascent/descent of the bundled faces
    // at 12/18/24 pixels (Pillow getmetrics); LLFontGL sums them minus one.
    for (font, advances) in [
        (fonts::INTER, [14.0, 22.0, 29.0]),
        (fonts::NOTO, [16.0, 25.0, 33.0]),
        (fonts::ROBOTO, [14.0, 21.0, 28.0]),
        (fonts::OPEN_DYSLEXIC, [22.0, 33.0, 44.0]),
    ] {
        for (scale, advance) in [1.0, 1.5, 2.0].into_iter().zip(advances) {
            for zoom in [1.0, 0.5] {
                for native in [1.0, 1.25, 1.5, 2.0] {
                    let (shapes, shift) = block("One\nTwo\nThree", font, scale, native, zoom, Color32::WHITE);
                    let text = shapes.iter().find(|text| text.override_text_color.is_none()).expect("text painted");
                    let actual = baselines(text, shift);
                    let last = 500.0 - 12.0 * zoom - 1.0;
                    let expected = [last - 2.0 * advance * zoom, last - advance * zoom, last];
                    for (actual, expected) in actual.into_iter().zip(expected) {
                        assert!(
                            (actual - expected).abs() * native <= 0.501,
                            "{font} size={scale}, zoom={zoom}, DPI={native}: {actual} vs {expected}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn three_independently_coloured_hud_prims_match_reference_coordinates() {
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    for event in crate::demo::events().into_iter().chain(crate::demo::floating_text::hud_events()) {
        world.apply(event);
    }
    let mut scene = Scene::new(std::path::PathBuf::new(), lib);
    let own = world.objects.index_of_uuid(&world.agent_id);
    for id in crate::demo::floating_text::TEXTS {
        let idx = world
            .objects
            .iter()
            .find(|(_, object)| object.key.local_id == id)
            .expect("text prim")
            .0;
        scene.gpu.resize_with(idx + 1, Default::default);
        scene.gpu[idx].hud = true;
        scene.gpu[idx].owner_avatar = own;
    }
    for (zoom, raw_anchors, lift) in [(1.0, [422.92, 445.42, 467.92], 13.0), (0.5, [468.46, 479.71, 490.96], 7.0)] {
        scene.lists.hud_view = Some(aurora_render::HudView::new([1600, 1000], zoom, -1.0, 1.0).in_area([32.0, 64.0, 1568.0, 964.0]));
        for native in [1.0, 1.25, 1.5, 2.0] {
            let ctx = context(fonts::INTER, 1.0);
            let mut shift = 0.0;
            let output = ctx.run_ui(input(native), |ui| {
                shift = glyph_shift(ui.ctx(), 12.0);
                hud::draw_attachments(ui.ctx(), &world, &scene);
            });
            let shapes = text_shapes(output);
            let visible: Vec<_> = shapes.iter().filter(|text| text.override_text_color.is_none()).collect();
            assert_eq!(visible.len(), 3);
            for (shape, anchor) in visible.into_iter().zip(raw_anchors) {
                let actual = baselines(shape, shift)[0];
                let expected = anchor / native - lift;
                assert!(
                    (actual - expected).abs() * native <= 0.501,
                    "zoom={zoom}, DPI={native}: {actual} vs {expected}"
                );
            }
        }
    }
}

#[test]
fn world_objects_use_the_same_baselines_without_changing_avatar_tags() {
    use glam::{Mat4, Vec3, Vec4};
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    for event in crate::demo::events() {
        world.apply(event);
    }
    let texted: Vec<_> = world.objects.texted().map(|(idx, _)| idx).collect();
    for idx in texted {
        world.objects.get_mut(idx).expect("fixture object").text.clear();
    }
    for mut event in crate::demo::floating_text::hud_events() {
        if let aurora_net::NetEvent::ObjectUpdates { objects, .. } = &mut event {
            objects.truncate(1);
            let cube = &mut objects[0];
            cube.parent_id = 0;
            cube.state = 0;
            cube.position = Vec3::ZERO;
            cube.scale = Vec3::splat(0.5);
            cube.text = "One\nTwo\nThree".into();
            cube.text_color = [255; 4];
        }
        world.apply(event);
    }
    let scene = Scene::new(std::path::PathBuf::new(), lib);
    let ctx = context(fonts::INTER, 1.0);
    let settings = crate::settings::Settings::default();
    let mut shift = 0.0;
    let output = ctx.run_ui(input(1.0), |ui| {
        shift = glyph_shift(ui.ctx(), 12.0);
        let proj = hud::Projector {
            // Project Z vertically. A 0.5 m cube's anchor is Z=0.3,
            // hence screen Y=350, last baseline=337. The radius pull
            // changes depth only with the eye at that same height.
            view_proj: Mat4::from_cols(Vec4::X, Vec4::Z, Vec4::Y, Vec4::W),
            width: 1600.0,
            height: 1000.0,
            ppp: 1.0,
        };
        let hits = hud::draw(
            ui.ctx(),
            &crate::theme::Theme::default().palette(),
            &crate::ui::icons::Icons::default(),
            &world,
            &scene,
            &proj,
            Vec3::new(0.0, -4.0, 0.3),
            false,
            hud::AvatarTags::Hidden,
            &settings,
            &mut hud::NameTagVisibility::default(),
            &std::collections::HashMap::new(),
            &settings.audio,
            &mut crate::ui::voice_tag::VoiceTags::default(),
            None,
            None,
            None,
            None,
            None,
        );
        assert!(hits.is_empty());
    });
    let shapes = text_shapes(output);
    let visible: Vec<_> = shapes.iter().filter(|text| text.override_text_color.is_none()).collect();
    assert_eq!(visible.len(), 1);
    for (actual, expected) in baselines(visible[0], shift).into_iter().zip([309.0, 323.0, 337.0]) {
        assert!((actual - expected).abs() <= 0.501, "{actual} vs {expected}");
    }
}

#[test]
fn crlf_empty_tokens_and_space_rows_follow_the_reference_tokenizer() {
    let (shapes, shift) = block("\r\n \n\nOne\r\nTwo\n", fonts::INTER, 1.0, 1.0, 1.0, Color32::WHITE);
    let text = shapes.iter().find(|text| text.override_text_color.is_none()).expect("text painted");
    assert_eq!(text.galley.job.text, " \nOne\nTwo");
    for (actual, expected) in baselines(text, shift).into_iter().zip([459.0, 473.0, 487.0]) {
        assert!((actual - expected).abs() <= 0.501, "{actual} vs {expected}");
    }
    assert!(block("\r\n\n", fonts::INTER, 1.0, 1.0, 1.0, Color32::WHITE).0.is_empty());
}

#[test]
fn each_row_centres_on_its_prim_and_wraps_at_the_reference_width() {
    let long = "WideWord ".repeat(28);
    let (shapes, _) = block(&format!("Short\n{long}"), fonts::INTER, 1.0, 1.0, 1.0, Color32::WHITE);
    let text = shapes.iter().find(|text| text.override_text_color.is_none()).expect("text painted");
    assert!(text.galley.rows.len() > 2);
    for row in &text.galley.rows {
        assert!(row.size.x <= 1000.01);
        assert!((text.pos.x + row.pos.x + row.size.x * 0.5 - 800.0).abs() < 1.01);
    }
}

#[test]
fn shadows_are_one_physical_pixel_and_depend_on_luminance() {
    for native in [1.0, 1.5] {
        let (shapes, _) = block("Text", fonts::INTER, 1.0, native, 1.0, Color32::WHITE);
        assert_eq!(shapes.len(), 2);
        assert_eq!(shapes[0].override_text_color, Some(Color32::BLACK));
        assert!(((shapes[0].pos.x - shapes[1].pos.x) * native - 1.0).abs() < 0.001);
        assert!(((shapes[0].pos.y - shapes[1].pos.y) * native - 1.0).abs() < 0.001);
    }
    let dark = Color32::from_gray(40);
    assert_eq!(block("Text", fonts::INTER, 1.0, 1.0, 1.0, dark).0.len(), 1);
    assert!(
        block("Text", fonts::INTER, 1.0, 1.0, 1.0, Color32::from_white_alpha(2))
            .0
            .is_empty()
    );
}

#[test]
fn tessellated_glyphs_stay_on_physical_pixels_without_resizing_the_font() {
    // Inspect the actual triangles, not only the ideal type baselines.
    // egui snaps a TextShape's origin but trusts its row offsets to have
    // remained pixel-aligned. Fractional offsets blur a raster font atlas.
    let sample = "Visitor Gray\nFloating text Hg";
    for font in fonts::BUILTINS {
        for scale in [1.0, 1.5] {
            for native in [1.0, 1.25, 1.5, 2.0] {
                for zoom in [1.0, 0.5, 0.63] {
                    let ctx = context(font.name, scale);
                    let mut reference = None;
                    let output = ctx.run_ui(input(native), |ui| {
                        paint(
                            &ui.ctx().layer_painter(egui::LayerId::background()),
                            sample.into(),
                            egui::pos2(800.23, 500.61),
                            12.0,
                            Color32::WHITE,
                            zoom,
                            None,
                        );
                        reference = Some(crate::ui::bidi::no_wrap(
                            ui.ctx(),
                            sample.into(),
                            egui::FontId::proportional(12.0),
                            Color32::WHITE,
                        ));
                    });
                    let shapes = text_shapes(output);
                    let text = shapes.iter().find(|text| text.override_text_color.is_none()).expect("text painted");
                    let reference = reference.expect("reference layout");
                    for (row, original) in text.galley.rows.iter().zip(&reference.rows) {
                        assert_eq!(row.glyphs.len(), original.glyphs.len());
                        for (glyph, before) in row.glyphs.iter().zip(&original.glyphs) {
                            assert_eq!(glyph.advance_width, before.advance_width, "font width unchanged");
                            assert_eq!(glyph.uv_rect.size, before.uv_rect.size, "raster size unchanged");
                        }
                    }
                    let primitives = ctx.tessellate(
                        shapes
                            .into_iter()
                            .map(|text| egui::epaint::ClippedShape::new(Rect::EVERYTHING, Shape::Text(text)))
                            .collect(),
                        native,
                    );
                    let mut vertices = 0;
                    for primitive in primitives {
                        let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                            continue;
                        };
                        for vertex in &mesh.vertices {
                            for coordinate in [vertex.pos.x, vertex.pos.y] {
                                let pixel = coordinate * native;
                                assert!(
                                    (pixel - pixel.round()).abs() < 0.003,
                                    "{} size={scale}, DPI={native}, zoom={zoom}: vertex at {pixel}",
                                    font.name
                                );
                            }
                        }
                        vertices += mesh.vertices.len();
                    }
                    assert!(vertices > 0);
                }
            }
        }
    }
}

/// The world cube of the baseline test, its prim, and what `hud::draw`
/// paints of it with `settings`, the pointer on `hovered`.
fn world_cube_frame(settings: &crate::settings::Settings, hover: bool) -> Vec<Shape> {
    use glam::{Mat4, Vec3, Vec4};
    let lib = Arc::new(AvatarLibrary::load());
    let mut world = World::new(lib.clone());
    for event in crate::demo::events() {
        world.apply(event);
    }
    let texted: Vec<_> = world.objects.texted().map(|(idx, _)| idx).collect();
    for idx in texted {
        world.objects.get_mut(idx).expect("fixture object").text.clear();
    }
    for mut event in crate::demo::floating_text::hud_events() {
        if let aurora_net::NetEvent::ObjectUpdates { objects, .. } = &mut event {
            objects.truncate(1);
            let cube = &mut objects[0];
            cube.parent_id = 0;
            cube.state = 0;
            cube.position = Vec3::ZERO;
            cube.scale = Vec3::splat(0.5);
            cube.text = "One\nTwo".into();
            cube.text_color = [255; 4];
        }
        world.apply(event);
    }
    let cube = world.objects.texted().map(|(idx, _)| idx).next().expect("the cube");
    let scene = Scene::new(std::path::PathBuf::new(), lib);
    let ctx = context(fonts::INTER, 1.0);
    let mut output = ctx.run_ui(input(1.0), |ui| {
        let proj = hud::Projector {
            view_proj: Mat4::from_cols(Vec4::X, Vec4::Z, Vec4::Y, Vec4::W),
            width: 1600.0,
            height: 1000.0,
            ppp: 1.0,
        };
        hud::draw(
            ui.ctx(),
            &crate::theme::Theme::default().palette(),
            &crate::ui::icons::Icons::default(),
            &world,
            &scene,
            &proj,
            // about 4 m from the text
            Vec3::new(0.0, -4.0, 0.3),
            false,
            hud::AvatarTags::Hidden,
            settings,
            &mut hud::NameTagVisibility::default(),
            &std::collections::HashMap::new(),
            &settings.audio,
            &mut crate::ui::voice_tag::VoiceTags::default(),
            None,
            None,
            None,
            None,
            hover.then_some(cube),
        );
    });
    output.textures_delta.clear();
    output.shapes.into_iter().map(|s| s.shape).collect()
}

fn texts_and_cards(shapes: &[Shape]) -> (usize, Vec<Color32>) {
    let texts = shapes
        .iter()
        .filter(|s| matches!(s, Shape::Text(t) if t.override_text_color.is_none()))
        .count();
    let cards = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Rect(r) => Some(r.fill),
            _ => None,
        })
        .collect();
    (texts, cards)
}

#[test]
fn the_fade_distance_follows_the_preferences_and_the_highlighted_text_ignores_it() {
    let mut settings = crate::settings::Settings::default();
    assert_eq!(texts_and_cards(&world_cube_frame(&settings, false)), (1, vec![]), "8 m by default");
    settings.floating_text_fade_distance = 1.0;
    settings.floating_text_fade_range = 1.0;
    assert_eq!(texts_and_cards(&world_cube_frame(&settings, false)).0, 0, "gone past 2 m");
    assert_eq!(texts_and_cards(&world_cube_frame(&settings, true)).0, 0, "hovered, no highlight");
    settings.floating_text_hover_highlight = true;
    assert_eq!(
        texts_and_cards(&world_cube_frame(&settings, true)).0,
        1,
        "highlighted at any distance"
    );
}

#[test]
fn the_background_is_painted_behind_the_text_as_asked() {
    use crate::settings::FloatingTextBackground as Bg;
    let mut settings = crate::settings::Settings {
        floating_text_background: Bg::All,
        ..Default::default()
    };
    // a white text: black at FSHudTextBackgroundOpacity (0.75)
    assert_eq!(
        texts_and_cards(&world_cube_frame(&settings, false)),
        (1, vec![Color32::from_black_alpha(191)])
    );
    settings.floating_text_background = Bg::Hovered;
    assert!(texts_and_cards(&world_cube_frame(&settings, false)).1.is_empty());
    settings.floating_text_hover_highlight = true;
    assert_eq!(
        texts_and_cards(&world_cube_frame(&settings, true)).1,
        vec![Color32::from_black_alpha(191)]
    );
}
