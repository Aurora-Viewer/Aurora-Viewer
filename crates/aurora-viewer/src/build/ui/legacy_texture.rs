//! FSPanelFace / panel_fs_tools_texture.xml: visible material tabs, maps
//! beside their parameters, channel tabs and three compact UV columns.
use super::*;
use crate::ui::widgets;
use egui::{Rect, vec2};

fn child(ui: &mut egui::Ui, canvas: Rect, x: f32, y: f32, w: f32, h: f32, id: &str) -> egui::Ui {
    super::super::legacy::pane(ui, Rect::from_min_size(canvas.min + vec2(x, y), vec2(w, h)), id)
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, env: &mut Env) {
    egui::Frame::new().inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        let sel = selected(tool, world);
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 46.0).max(120.0);
            ui.allocate_ui_with_layout(vec2(width, 22.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                // Keep mode values shared with Aurora while presenting PBR first.
                let mut tab = match tool.ui.tex_tab {
                    0 => 1,
                    1 => 0,
                    _ => 2,
                };
                if widgets::compact_tabs(
                    ui,
                    p,
                    &mut tab,
                    &[
                        ("PBR", true),
                        (aurora_i18n::tr!("build-ui-texture-blinn-phong"), true),
                        (aurora_i18n::tr!("extra-media"), true),
                    ],
                ) {
                    tool.ui.tex_tab = match tab {
                        0 => 1,
                        1 => 0,
                        _ => 2,
                    };
                }
            });
            copy_actions(ui, p, tool, world, env, &sel);
        });
        if sel.is_empty() {
            small(ui, p, aurora_i18n::tr!("build-ui-texture-no-editable-face-selected"));
            return;
        }
        match tool.ui.tex_tab {
            0 => bp_panel(ui, p, tool, world, s, env, &sel),
            1 => pbr(ui, p, tool, world, env, &sel),
            _ => media(ui, p, tool, world, env, &sel),
        }
        footer(ui, p, tool, world, s, &sel, env);
    });
}

fn copy_actions(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &Env, sel: &[Sel]) {
    if super::super::common::icon_button(
        ui,
        p,
        "copy-filled",
        aurora_i18n::tr!("build-ui-texture-copy-texture-settings-to-clipboard"),
        !sel.is_empty() && sel.iter().all(|x| x.idx == sel[0].idx),
    ) {
        tool.ui.face_clipboard = Some(
            sel.iter()
                .map(|x| FaceClip {
                    face: x.te,
                    legacy: tool.face_legacy(env, world, x.idx, x.face, &x.te),
                    gltf: BuildTool::gltf_asset(world, x.idx, x.face).map(|a| (a, BuildTool::gltf_override(world, x.idx, x.face))),
                })
                .collect(),
        );
    }
    if super::super::common::icon_button(
        ui,
        p,
        "clipboard-text-filled",
        aurora_i18n::tr!("build-ui-texture-paste-texture-settings-from-the-clipboard"),
        !sel.is_empty() && tool.ui.face_clipboard.is_some(),
    ) {
        paste_faces(tool, world, sel);
    }
}

#[allow(clippy::too_many_arguments)]
fn tint(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    env: &mut Env,
    target: super::super::ColorTarget,
    rgb: [f32; 3],
    enabled: bool,
    caption: &str,
) {
    let response = super::super::common::color_swatch_sized(ui, p, rgb, "", vec2(30.0, 60.0), enabled);
    ui.add_sized(
        [ui.available_width(), 14.0],
        egui::Label::new(egui::RichText::new(caption).size(11.0).color(p.muted)).truncate(),
    )
    .on_hover_text(caption);
    tool.color_swatch_response(world, env, target, rgb, response);
}

fn compact_glow(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, sel: &[Sel]) {
    let (value, mixed) = same(sel.iter().map(|x| x.te.glow));
    let mut value = value.unwrap_or(0.0);
    row(ui, p, aurora_i18n::tr!("build-ui-texture-glow"), 35.0, |ui| {
        if spin(ui, &mut value, 0.1, 0.0..=1.0, 3, (ui.available_width() - 1.0).max(50.0), mixed).commit {
            tool.edit_faces(world, |f, _, _| f.glow = value);
        }
    });
}

fn bp_panel(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, env: &mut Env, sel: &[Sel]) {
    let bp: Vec<_> = sel.iter().map(|x| tool.face_legacy(env, world, x.idx, x.face, &x.te)).collect();
    let water = sel.iter().all(|x| is_hide_water(&x.te, bp[0].as_ref()));
    let (diffuse, diffuse_mixed) = same(sel.iter().map(|x| x.te.texture));
    let (normal, normal_mixed) = same(bp.iter().map(|m| m.as_ref().map(|m| m.normal_map).unwrap_or_default()));
    let (spec, spec_mixed) = same(bp.iter().map(|m| m.as_ref().map(|m| m.specular_map).unwrap_or_default()));
    let normal = normal.filter(|id| !id.is_nil());
    let spec = spec.filter(|id| !id.is_nil());
    let (canvas, _) = ui.allocate_exact_size(vec2(ui.available_width(), 224.0), egui::Sense::hover());
    for (y, id, mixed, target, caption) in [
        (
            0.0,
            diffuse,
            diffuse_mixed,
            PickTarget::Diffuse,
            aurora_i18n::tr!("build-ui-texture-diffuse"),
        ),
        (
            74.0,
            normal,
            normal_mixed,
            PickTarget::Normal,
            aurora_i18n::tr!("build-ui-texture-normal"),
        ),
        (
            148.0,
            spec,
            spec_mixed,
            PickTarget::Specular,
            aurora_i18n::tr!("build-ui-texture-specular"),
        ),
    ] {
        let mut map = child(ui, canvas, 0.0, y, 60.0, 76.0, &format!("legacy-map-{target:?}"));
        material_map(&mut map, p, tool, env, id, mixed, caption, target, !water, 60.0);
    }
    let (rgb, _) = same(sel.iter().map(|x| [x.te.color[0], x.te.color[1], x.te.color[2]]));
    let mut diffuse_tint = child(ui, canvas, 64.0, 0.0, 40.0, 76.0, "legacy-diffuse-tint");
    tint(
        &mut diffuse_tint,
        p,
        tool,
        world,
        env,
        super::super::ColorTarget::Diffuse,
        rgb.unwrap_or([1.0; 3]),
        !water,
        aurora_i18n::tr!("build-ui-texture-tint"),
    );
    let color = bp[0].as_ref().map(|m| m.specular_color).unwrap_or([255; 4]);
    let rgb = [color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0];
    let mut spec_tint = child(ui, canvas, 64.0, 148.0, 40.0, 76.0, "legacy-specular-tint");
    tint(
        &mut spec_tint,
        p,
        tool,
        world,
        env,
        super::super::ColorTarget::Specular,
        rgb,
        !water && spec.is_some(),
        aurora_i18n::tr!("common-color"),
    );

    let options_x = 104.0;
    let options_w = canvas.width() - options_x;
    let mut bright = child(ui, canvas, options_x, 0.0, options_w - 96.0, 22.0, "legacy-fullbright");
    let (full, mixed) = same(sel.iter().map(|x| x.te.bump_shiny_fullbright & TEM_FULLBRIGHT != 0));
    if let Some(on) = check(
        &mut bright,
        p,
        full.unwrap_or(false),
        mixed,
        aurora_i18n::tr!("build-ui-texture-fullbright"),
        !water,
    ) {
        tool.edit_faces(world, |f, _, _| {
            f.bump_shiny_fullbright = if on {
                f.bump_shiny_fullbright | TEM_FULLBRIGHT
            } else {
                f.bump_shiny_fullbright & !TEM_FULLBRIGHT
            }
        });
    }
    let mut glow_ui = child(ui, canvas, canvas.width() - 96.0, 0.0, 96.0, 22.0, "legacy-glow");
    compact_glow(&mut glow_ui, p, tool, world, sel);
    let mut properties = child(ui, canvas, options_x, 27.0, options_w, 197.0, "legacy-bp-properties");
    let label_width = (properties.available_width() - 70.0).max(65.0);
    let (alpha, alpha_mixed) = same(sel.iter().map(|x| x.te.color[3]));
    let mut value = (1.0 - alpha.unwrap_or(1.0)) * 100.0;
    row(
        &mut properties,
        p,
        aurora_i18n::tr!("build-ui-texture-transparency"),
        label_width,
        |ui| {
            ui.add_enabled_ui(!water, |ui| {
                if spin(ui, &mut value, 2.0, 0.0..=100.0, 0, 64.0, alpha_mixed).commit {
                    tool.edit_faces(world, |f, _, _| f.color[3] = (100.0 - value) / 100.0);
                }
            });
        },
    );
    let has_alpha = diffuse.and_then(|id| env.scene.textures.has_alpha(&id)).unwrap_or(false);
    let opaque = sel.iter().all(|x| x.te.color[3] >= 1.0);
    let default = if has_alpha { alpha_mode::BLEND } else { alpha_mode::NONE };
    let (mode, mixed) = same(bp.iter().map(|m| m.as_ref().map(|m| m.diffuse_alpha_mode).unwrap_or(default)));
    let mode = if opaque { mode } else { Some(alpha_mode::BLEND) };
    row(&mut properties, p, aurora_i18n::tr!("build-ui-texture-alpha-mode"), 70.0, |ui| {
        if let Some(value) = combo(
            ui,
            "legacy-alpha-mode",
            if mixed { None } else { mode },
            &alpha_modes(),
            ui.available_width(),
            !water && has_alpha && opaque,
        ) {
            tool.edit_legacy(env, world, sel, |m| m.diffuse_alpha_mode = value);
        }
    });
    let (cutoff, cutoff_mixed) = same(bp.iter().map(|m| m.as_ref().map(|m| m.alpha_cutoff).unwrap_or(127)));
    let mut cutoff = cutoff.unwrap_or(127) as f32;
    row(
        &mut properties,
        p,
        aurora_i18n::tr!("build-ui-texture-mask-cutoff"),
        label_width,
        |ui| {
            ui.add_enabled_ui(!water && has_alpha && opaque && mode == Some(alpha_mode::MASK), |ui| {
                if spin(ui, &mut cutoff, 1.0, 0.0..=255.0, 0, 64.0, cutoff_mixed).commit {
                    tool.edit_legacy(env, world, sel, |m| m.alpha_cutoff = cutoff as u8);
                }
            });
        },
    );
    properties.add_space(20.0);
    let (bump, mixed) = same(sel.iter().map(|x| x.te.bump_shiny_fullbright & TEM_BUMP_MASK));
    let mut items: Vec<_> = bumps().iter().enumerate().map(|(i, name)| (*name, i as u8)).collect();
    if normal.is_some() {
        items.push((aurora_i18n::tr!("build-ui-texture-use-texture"), BUMPY_TEXTURE));
    }
    row(&mut properties, p, aurora_i18n::tr!("build-ui-texture-bumpiness"), 65.0, |ui| {
        if let Some(value) = combo(
            ui,
            "legacy-bump",
            if normal.is_some() {
                Some(BUMPY_TEXTURE)
            } else if mixed {
                None
            } else {
                bump
            },
            &items,
            ui.available_width(),
            !water,
        ) && value != BUMPY_TEXTURE
        {
            tool.edit_faces(world, |f, _, _| {
                f.bump_shiny_fullbright = (f.bump_shiny_fullbright & !TEM_BUMP_MASK) | value
            });
            if normal.is_some() {
                tool.edit_legacy(env, world, sel, |m| m.normal_map = Uuid::nil());
            }
        }
    });
    properties.add_space(8.0);
    let (shiny, mixed) = same(sel.iter().map(|x| x.te.bump_shiny_fullbright >> TEM_SHINY_SHIFT));
    let mut items: Vec<_> = shines().iter().enumerate().map(|(i, name)| (*name, i as u8)).collect();
    if spec.is_some() {
        items.push((aurora_i18n::tr!("build-ui-texture-use-texture"), SHINY_TEXTURE));
    }
    row(&mut properties, p, aurora_i18n::tr!("build-ui-texture-shininess"), 65.0, |ui| {
        if let Some(value) = combo(
            ui,
            "legacy-shiny",
            if spec.is_some() {
                Some(SHINY_TEXTURE)
            } else if mixed {
                None
            } else {
                shiny
            },
            &items,
            ui.available_width(),
            !water,
        ) && value != SHINY_TEXTURE
        {
            tool.edit_faces(world, |f, _, _| {
                f.bump_shiny_fullbright = (f.bump_shiny_fullbright & !(3 << TEM_SHINY_SHIFT)) | (value << TEM_SHINY_SHIFT)
            });
            if spec.is_some() {
                tool.edit_legacy(env, world, sel, |m| m.specular_map = Uuid::nil());
            }
        }
    });
    properties.add_space(4.0);
    for (name, environment) in [
        (aurora_i18n::tr!("build-legacy-glossiness"), false),
        (aurora_i18n::tr!("common-environment"), true),
    ] {
        let (value, mixed) = same(bp.iter().map(|m| {
            m.as_ref()
                .map(|m| if environment { m.env_intensity } else { m.specular_exp })
                .unwrap_or(if environment { 0 } else { 51 })
        }));
        let mut value = value.unwrap_or(0) as f32;
        row(&mut properties, p, name, label_width, |ui| {
            ui.add_enabled_ui(!water && spec.is_some(), |ui| {
                if spin(ui, &mut value, 1.0, 0.0..=255.0, 0, 64.0, mixed).commit {
                    tool.edit_legacy(env, world, sel, |m| {
                        if environment {
                            m.env_intensity = value as u8
                        } else {
                            m.specular_exp = value as u8
                        }
                    });
                }
            });
        });
    }
    bp_transforms(ui, p, tool, world, s, env, sel, &bp, normal, spec, !water);
}

#[allow(clippy::too_many_arguments)]
pub(super) fn pbr_surface(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    env: &mut Env,
    sel: &[Sel],
    mat: &aurora_assets::material::PbrMaterial,
    asset: Option<Uuid>,
    mixed: bool,
    model: Option<std::sync::Arc<aurora_render::preview::Model>>,
    editable: bool,
) {
    let (canvas, _) = ui.allocate_exact_size(vec2(ui.available_width(), 224.0), egui::Sense::hover());
    let mut material = child(ui, canvas, 0.0, 0.0, 60.0, 76.0, "legacy-pbr-material");
    material_asset(&mut material, p, tool, asset, mixed, model, 60.0);
    for (x, y, id, caption, channel) in [
        (64.0, 0.0, mat.base_color_texture, aurora_i18n::tr!("common-color"), 0),
        (0.0, 74.0, mat.normal_texture, aurora_i18n::tr!("build-ui-texture-normal"), 1),
        (64.0, 74.0, mat.emissive_texture, aurora_i18n::tr!("build-ui-texture-emissive"), 3),
        (0.0, 148.0, mat.metallic_roughness_texture, "(O)RM", 2),
    ] {
        let mut map = child(ui, canvas, x, y, 60.0, 76.0, &format!("legacy-pbr-map-{channel}"));
        material_map(
            &mut map,
            p,
            tool,
            env,
            id,
            false,
            caption,
            PickTarget::PbrMap(channel),
            editable,
            60.0,
        );
    }
    for (y, target, color) in [
        (
            0.0,
            super::super::ColorTarget::PbrBase,
            [mat.base_color_factor[0], mat.base_color_factor[1], mat.base_color_factor[2]],
        ),
        (74.0, super::super::ColorTarget::PbrEmissive, mat.emissive_factor),
    ] {
        let mut swatch = child(ui, canvas, 128.0, y, 37.0, 76.0, &format!("legacy-pbr-tint-{y}"));
        tint(
            &mut swatch,
            p,
            tool,
            world,
            env,
            target,
            color.map(super::super::common::linear_to_srgb),
            editable,
            aurora_i18n::tr!("build-ui-texture-tint"),
        );
    }
    let mut params = child(ui, canvas, 165.0, 0.0, canvas.width() - 165.0, 224.0, "legacy-pbr-factors");
    compact_glow(&mut params, p, tool, world, sel);
    params.add_space(4.0);
    if let Some(value) = check(
        &mut params,
        p,
        mat.double_sided,
        false,
        aurora_i18n::tr!("common-double-sided"),
        editable,
    ) {
        tool.edit_gltf(world, sel, |o| o.double_sided = Some(value));
    }
    let mut alpha = mat.base_color_factor[3];
    let label_width = (params.available_width() - 58.0).max(55.0);
    let field_width = (params.available_width() - label_width - 3.0).max(50.0);
    params.add_enabled_ui(editable, |ui| {
        row(ui, p, aurora_i18n::tr!("common-alpha"), label_width, |ui| {
            if spin(ui, &mut alpha, 0.1, 0.0..=1.0, 3, field_width, false).commit {
                let [r, g, b, _] = mat.base_color_factor;
                tool.edit_gltf(world, sel, |o| o.base_color_factor = Some([r, g, b, alpha]));
            }
        });
        row(ui, p, aurora_i18n::tr!("build-ui-texture-mode"), 38.0, |ui| {
            if let Some(value) = combo(
                ui,
                "legacy-pbr-alpha",
                Some(mat.alpha_mode),
                &[
                    (aurora_i18n::tr!("common-opaque"), AlphaMode::Opaque),
                    (aurora_i18n::tr!("build-ui-texture-blend"), AlphaMode::Blend),
                    (aurora_i18n::tr!("build-ui-texture-masking"), AlphaMode::Mask),
                ],
                ui.available_width(),
                true,
            ) {
                tool.edit_gltf(world, sel, |o| o.alpha_mode = Some(value));
            }
        });
        let mut cutoff = mat.alpha_cutoff;
        row(ui, p, aurora_i18n::tr!("build-ui-texture-cut"), label_width, |ui| {
            ui.add_enabled_ui(mat.alpha_mode == AlphaMode::Mask, |ui| {
                if spin(ui, &mut cutoff, 0.1, 0.0..=1.0, 3, field_width, false).commit {
                    tool.edit_gltf(world, sel, |o| o.alpha_cutoff = Some(cutoff));
                }
            });
        });
        ui.add_space(10.0);
        for (name, value, metallic) in [
            (aurora_i18n::tr!("build-ui-texture-metallic"), mat.metallic_factor, true),
            (aurora_i18n::tr!("build-ui-texture-roughness"), mat.roughness_factor, false),
        ] {
            let mut value = value;
            row(ui, p, name, label_width, |ui| {
                if spin(ui, &mut value, 0.1, 0.0..=1.0, 3, field_width, false).commit {
                    tool.edit_gltf(world, sel, |o| {
                        if metallic {
                            o.metallic_factor = Some(value)
                        } else {
                            o.roughness_factor = Some(value)
                        }
                    });
                }
            });
        }
    });
}

fn footer(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, sel: &[Sel], env: &Env) {
    let (canvas, _) = ui.allocate_exact_size(vec2(ui.available_width(), 68.0), egui::Sense::hover());
    let mut left = child(ui, canvas, 0.0, 0.0, canvas.width() - 96.0, 68.0, "legacy-face-parameters");
    left.spacing_mut().interact_size.y = 16.0;
    repeats_meter(&mut left, p, tool, world, sel);
    if let Some(value) = check(
        &mut left,
        p,
        s.sync_materials,
        false,
        aurora_i18n::tr!("build-ui-texture-synchronize-materials"),
        true,
    ) {
        s.sync_materials = value;
    }
    let planar = sel.iter().all(|x| x.te.media_flags & TEM_TEX_GEN_MASK == TEX_GEN_PLANAR);
    if let Some(value) = check(
        &mut left,
        p,
        tool.ui.planar_align,
        false,
        aurora_i18n::tr!("build-ui-texture-align-planar-faces"),
        planar,
    ) {
        tool.ui.planar_align = value;
    }
    let mut right = child(ui, canvas, canvas.width() - 90.0, 0.0, 90.0, 68.0, "legacy-face-mapping");
    right.spacing_mut().interact_size.y = 16.0;
    let water = sel
        .iter()
        .all(|x| is_hide_water(&x.te, tool.face_legacy(env, world, x.idx, x.face, &x.te).as_ref()));
    let has_pbr = sel.iter().any(|x| BuildTool::gltf_asset(world, x.idx, x.face).is_some());
    if let Some(value) = check(
        &mut right,
        p,
        water,
        false,
        aurora_i18n::tr!("build-ui-texture-occlude-water"),
        !has_pbr && tool.ui.tex_tab != 2,
    ) {
        if value {
            tool.ui.confirm = Some(Confirm::HideWater);
        } else {
            tool.edit_faces(world, |f, _, _| f.texture = DEFAULT_TEXTURE);
        }
    }
    let (value, mixed) = same(sel.iter().map(|x| x.te.media_flags & TEM_TEX_GEN_MASK));
    if let Some(value) = combo(
        &mut right,
        "legacy-texgen",
        if mixed { None } else { value },
        &[
            (aurora_i18n::tr!("common-default"), 0u8),
            (aurora_i18n::tr!("build-ui-texture-planar"), TEX_GEN_PLANAR),
        ],
        88.0,
        true,
    ) {
        tool.edit_faces(world, |f, _, _| f.media_flags = (f.media_flags & !TEM_TEX_GEN_MASK) | value);
    }
    let objects = sel.iter().map(|x| x.idx).collect::<std::collections::BTreeSet<_>>().len();
    if right
        .add_enabled(
            planar && tool.ui.planar_align && objects > 1,
            egui::Button::new(aurora_i18n::tr!("common-align")),
        )
        .clicked()
    {
        tool.align_planar_faces(world, sel.iter().map(|x| (x.idx, x.face)).collect());
    }
}
