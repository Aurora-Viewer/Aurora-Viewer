//! panel_tools_texture.xml: tint/transparency/glow across the top, channel
//! radios beside the selected texture, then full-width vertical UV fields.
use super::*;
use crate::ui::widgets;
use egui::{Rect, RichText, vec2};

fn child(ui: &mut egui::Ui, canvas: Rect, x: f32, y: f32, w: f32, h: f32, id: &str) -> egui::Ui {
    let scale = canvas.width() / 275.0;
    super::super::legacy::pane(ui, Rect::from_min_size(canvas.min + vec2(x * scale, y), vec2(w * scale, h)), id)
}

pub(super) fn show(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, env: &mut Env) {
    egui::Frame::new().inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        let sel = selected(tool, world);
        if sel.is_empty() {
            small(ui, p, aurora_i18n::tr!("build-ui-texture-no-editable-face-selected"));
            return;
        }
        let bp: Vec<_> = sel.iter().map(|x| tool.face_legacy(env, world, x.idx, x.face, &x.te)).collect();
        let water = sel.iter().all(|x| is_hide_water(&x.te, bp[0].as_ref()));
        let (canvas, _) = ui.allocate_exact_size(vec2(ui.available_width(), 111.0), egui::Sense::hover());
        let (rgb, mixed) = same(sel.iter().map(|x| [x.te.color[0], x.te.color[1], x.te.color[2]]));
        let mut tint = child(ui, canvas, 0.0, 0.0, 64.0, 44.0, "legacy-tint");
        super::super::common::label(&mut tint, p, aurora_i18n::tr!("common-color"));
        let color = rgb.unwrap_or([1.0; 3]);
        let response = super::super::common::color_swatch_sized(&mut tint, p, color, "", vec2(64.0, 22.0), !water && tool.ui.tex_tab == 0);
        tool.color_swatch_response(
            world,
            env,
            super::super::ColorTarget::Diffuse,
            color,
            response.on_hover_text(if mixed {
                aurora_i18n::tr!("status-multiple")
            } else {
                aurora_i18n::tr!("common-color")
            }),
        );
        let mut trans = child(ui, canvas, 80.0, 0.0, 85.0, 44.0, "legacy-transparency");
        super::super::common::label(&mut trans, p, aurora_i18n::tr!("build-ui-texture-transparency"));
        let (alpha, alpha_mixed) = same(sel.iter().map(|x| x.te.color[3]));
        let mut value = (1.0 - alpha.unwrap_or(1.0)) * 100.0;
        trans.add_enabled_ui(!water && tool.ui.tex_tab == 0, |ui| {
            if spin(ui, &mut value, 2.0, 0.0..=100.0, 0, 80.0, alpha_mixed).commit {
                tool.edit_faces(world, |f, _, _| f.color[3] = (100.0 - value) / 100.0);
            }
        });
        let mut glow_ui = child(ui, canvas, 175.0, 0.0, 70.0, 44.0, "legacy-glow");
        super::super::common::label(&mut glow_ui, p, aurora_i18n::tr!("build-ui-texture-glow"));
        let (glow, mixed) = same(sel.iter().map(|x| x.te.glow));
        let mut glow = glow.unwrap_or(0.0);
        if spin(&mut glow_ui, &mut glow, 0.1, 0.0..=1.0, 3, 70.0, mixed).commit {
            tool.edit_faces(world, |f, _, _| f.glow = glow);
        }
        let mut copy = child(ui, canvas, 251.0, 0.0, 24.0, 50.0, "legacy-face-copy");
        copy_actions(&mut copy, p, tool, world, env, &sel);
        let mut bright = child(ui, canvas, 0.0, 46.0, 112.0, 20.0, "legacy-fullbright");
        let (full, mixed) = same(sel.iter().map(|x| x.te.bump_shiny_fullbright & TEM_FULLBRIGHT != 0));
        if let Some(on) = check(
            &mut bright,
            p,
            full.unwrap_or(false),
            mixed,
            aurora_i18n::tr!("build-ui-texture-fullbright"),
            !water && tool.ui.tex_tab == 0,
        ) {
            tool.edit_faces(world, |f, _, _| {
                f.bump_shiny_fullbright = if on {
                    f.bump_shiny_fullbright | TEM_FULLBRIGHT
                } else {
                    f.bump_shiny_fullbright & !TEM_FULLBRIGHT
                };
            });
        }
        let mut hide = child(ui, canvas, 0.0, 66.0, 112.0, 20.0, "legacy-hide-water");
        if let Some(on) = check(&mut hide, p, water, false, aurora_i18n::tr!("build-ui-texture-occlude-water"), true) {
            if on {
                tool.ui.confirm = Some(Confirm::HideWater);
            } else {
                tool.edit_faces(world, |f, _, _| f.texture = DEFAULT_TEXTURE);
            }
        }
        let mut model = child(ui, canvas, 0.0, 87.0, 112.0, 23.0, "legacy-material-type");
        if let Some(mode) = combo(
            &mut model,
            "legacy-material-mode",
            Some(tool.ui.tex_tab),
            &[
                (aurora_i18n::tr!("build-legacy-textures"), 0),
                ("PBR", 1),
                (aurora_i18n::tr!("extra-media"), 2),
            ],
            110.0,
            true,
        ) {
            tool.ui.tex_tab = mode;
        }
        if tool.ui.tex_tab == 0 {
            let mut channels = child(ui, canvas, 120.0, 46.0, 155.0, 64.0, "legacy-map-channels");
            for (channel, label) in [
                (0, aurora_i18n::tr!("build-ui-texture-diffuse")),
                (1, aurora_i18n::tr!("build-ui-texture-bumpiness")),
                (2, aurora_i18n::tr!("common-brightness")),
            ] {
                widgets::radio_value(&mut channels, p, &mut tool.ui.bp_channel, channel, RichText::new(label).size(11.0));
            }
            bp_panel(ui, p, tool, world, s, env, &sel, &bp, !water);
        } else if tool.ui.tex_tab == 1 {
            pbr(ui, p, tool, world, env, &sel);
        } else {
            media(ui, p, tool, world, env, &sel);
        }
        if tool.ui.tex_tab != 2 {
            tail(ui, p, tool, world, s, &sel);
        }
    });
}

fn copy_actions(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &Env, sel: &[Sel]) {
    if super::super::common::icon_button(
        ui,
        p,
        "copy-filled",
        aurora_i18n::tr!("build-ui-texture-copy-texture-settings-to-clipboard"),
        sel.iter().all(|x| x.idx == sel[0].idx),
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
        tool.ui.face_clipboard.is_some(),
    ) {
        paste_faces(tool, world, sel);
    }
}

#[allow(clippy::too_many_arguments)]
fn bp_panel(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    env: &mut Env,
    sel: &[Sel],
    bp: &[Option<LegacyMaterial>],
    editable: bool,
) {
    let (diffuse, diffuse_mixed) = same(sel.iter().map(|x| x.te.texture));
    let (normal, normal_mixed) = same(bp.iter().map(|m| m.as_ref().map(|m| m.normal_map).unwrap_or_default()));
    let (spec, spec_mixed) = same(bp.iter().map(|m| m.as_ref().map(|m| m.specular_map).unwrap_or_default()));
    let normal = normal.filter(|id| !id.is_nil());
    let spec = spec.filter(|id| !id.is_nil());
    let (canvas, _) = ui.allocate_exact_size(vec2(ui.available_width(), 113.0), egui::Sense::hover());
    let (id, mixed, target) = match tool.ui.bp_channel {
        1 => (normal, normal_mixed, PickTarget::Normal),
        2 => (spec, spec_mixed, PickTarget::Specular),
        _ => (diffuse, diffuse_mixed, PickTarget::Diffuse),
    };
    let mut preview = child(ui, canvas, 0.0, 0.0, 80.0, 110.0, "legacy-selected-map");
    material_map(
        &mut preview,
        p,
        tool,
        env,
        id,
        mixed,
        aurora_i18n::tr!("common-texture"),
        target,
        editable,
        76.0,
    );
    let mut properties = child(ui, canvas, 94.0, 0.0, 181.0, 110.0, "legacy-selected-map-properties");
    match tool.ui.bp_channel {
        1 => {
            super::super::common::label(&mut properties, p, aurora_i18n::tr!("build-ui-texture-bumpiness"));
            let (value, mixed) = same(sel.iter().map(|x| x.te.bump_shiny_fullbright & TEM_BUMP_MASK));
            let mut items: Vec<_> = bumps().iter().enumerate().map(|(i, name)| (*name, i as u8)).collect();
            if normal.is_some() {
                items.push((aurora_i18n::tr!("build-ui-texture-use-texture"), BUMPY_TEXTURE));
            }
            if let Some(value) = combo(
                &mut properties,
                "legacy-bump",
                if normal.is_some() {
                    Some(BUMPY_TEXTURE)
                } else if mixed {
                    None
                } else {
                    value
                },
                &items,
                150.0,
                editable,
            ) && value != BUMPY_TEXTURE
            {
                tool.edit_faces(world, |f, _, _| {
                    f.bump_shiny_fullbright = (f.bump_shiny_fullbright & !TEM_BUMP_MASK) | value
                });
                if normal.is_some() {
                    tool.edit_legacy(env, world, sel, |m| m.normal_map = Uuid::nil());
                }
            }
        }
        2 => {
            let (value, mixed) = same(sel.iter().map(|x| x.te.bump_shiny_fullbright >> TEM_SHINY_SHIFT));
            let mut items: Vec<_> = shines().iter().enumerate().map(|(i, name)| (*name, i as u8)).collect();
            if spec.is_some() {
                items.push((aurora_i18n::tr!("build-ui-texture-use-texture"), SHINY_TEXTURE));
            }
            row(&mut properties, p, aurora_i18n::tr!("common-brightness"), 80.0, |ui| {
                if let Some(value) = combo(
                    ui,
                    "legacy-shiny",
                    if spec.is_some() {
                        Some(SHINY_TEXTURE)
                    } else if mixed {
                        None
                    } else {
                        value
                    },
                    &items,
                    85.0,
                    editable,
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
            for (name, environment) in [
                (aurora_i18n::tr!("build-ui-texture-shininess"), false),
                (aurora_i18n::tr!("common-environment"), true),
            ] {
                let (value, mixed) = same(bp.iter().map(|m| {
                    m.as_ref()
                        .map(|m| if environment { m.env_intensity } else { m.specular_exp })
                        .unwrap_or(if environment { 0 } else { 51 })
                }));
                let mut value = value.unwrap_or(0) as f32;
                properties.add_enabled_ui(editable && spec.is_some(), |ui| {
                    row(ui, p, name, 80.0, |ui| {
                        if spin(ui, &mut value, 1.0, 0.0..=255.0, 0, 64.0, mixed).commit {
                            tool.edit_legacy(env, world, sel, |m| {
                                if environment {
                                    m.env_intensity = value as u8;
                                } else {
                                    m.specular_exp = value as u8;
                                }
                            });
                        }
                    });
                });
            }
            let color = bp[0].as_ref().map(|m| m.specular_color).unwrap_or([255; 4]);
            let rgb = [color[0] as f32 / 255.0, color[1] as f32 / 255.0, color[2] as f32 / 255.0];
            row(&mut properties, p, aurora_i18n::tr!("common-color"), 80.0, |ui| {
                let r = super::super::common::color_swatch_sized(ui, p, rgb, "", vec2(64.0, 22.0), editable && spec.is_some());
                tool.color_swatch_response(world, env, super::super::ColorTarget::Specular, rgb, r);
            });
        }
        _ => {
            super::super::common::label(&mut properties, p, aurora_i18n::tr!("build-ui-texture-alpha-mode"));
            let has_alpha = diffuse.and_then(|id| env.scene.textures.has_alpha(&id)).unwrap_or(false);
            let opaque = sel.iter().all(|x| x.te.color[3] >= 1.0);
            let default = if has_alpha { alpha_mode::BLEND } else { alpha_mode::NONE };
            let (mode, mixed) = same(bp.iter().map(|m| m.as_ref().map(|m| m.diffuse_alpha_mode).unwrap_or(default)));
            let mode = if opaque { mode } else { Some(alpha_mode::BLEND) };
            if let Some(mode) = combo(
                &mut properties,
                "legacy-alpha-mode",
                if mixed { None } else { mode },
                &alpha_modes(),
                150.0,
                editable && has_alpha && opaque,
            ) {
                tool.edit_legacy(env, world, sel, |m| m.diffuse_alpha_mode = mode);
            }
            if mode == Some(alpha_mode::MASK) {
                super::super::common::label(&mut properties, p, aurora_i18n::tr!("build-ui-texture-mask-cutoff"));
                let (value, mixed) = same(bp.iter().map(|m| m.as_ref().map(|m| m.alpha_cutoff).unwrap_or(127)));
                let mut value = value.unwrap_or(127) as f32;
                if spin(&mut properties, &mut value, 1.0, 0.0..=255.0, 0, 80.0, mixed).commit {
                    tool.edit_legacy(env, world, sel, |m| m.alpha_cutoff = value as u8);
                }
            }
        }
    }
    mapping(ui, p, tool, world, sel);
    bp_transforms(ui, p, tool, world, s, env, sel, bp, normal, spec, editable);
}

pub(super) fn mapping(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, sel: &[Sel]) {
    let (value, mixed) = same(sel.iter().map(|x| x.te.media_flags & TEM_TEX_GEN_MASK));
    row(
        ui,
        p,
        aurora_i18n::tr!("build-legacy-mapping"),
        (ui.available_width() - 130.0).max(90.0),
        |ui| {
            if let Some(value) = combo(
                ui,
                "legacy-texgen",
                if mixed { None } else { value },
                &[
                    (aurora_i18n::tr!("common-default"), 0u8),
                    (aurora_i18n::tr!("build-ui-texture-planar"), TEX_GEN_PLANAR),
                ],
                125.0,
                true,
            ) {
                tool.edit_faces(world, |f, _, _| f.media_flags = (f.media_flags & !TEM_TEX_GEN_MASK) | value);
            }
        },
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn uv_controls(
    ui: &mut egui::Ui,
    p: &Palette,
    values: &mut [f32; 5],
    mixed: [bool; 5],
    decimals: usize,
    enabled: bool,
    repeats: impl FnOnce(&mut egui::Ui),
) -> Edit {
    let mut edit = Edit::default();
    let mut repeats = Some(repeats);
    for (index, label) in [
        (0, aurora_i18n::tr!("build-legacy-scale-horizontal")),
        (1, aurora_i18n::tr!("build-legacy-scale-vertical")),
        (4, aurora_i18n::tr!("build-ui-object-rotation-degrees")),
        (2, aurora_i18n::tr!("build-legacy-offset-horizontal")),
        (3, aurora_i18n::tr!("build-legacy-offset-vertical")),
    ] {
        if index == 4
            && let Some(repeats) = repeats.take()
        {
            repeats(ui);
        }
        let width = (ui.available_width() - 100.0).max(90.0);
        row(ui, p, label, width, |ui| {
            ui.add_enabled_ui(enabled, |ui| {
                let (inc, range) = match index {
                    0 | 1 => (0.1, -10000.0..=10000.0),
                    4 => (1.0, -360.0..=360.0),
                    _ => (0.01, -1.0..=1.0),
                };
                edit = edit.or(spin(ui, &mut values[index], inc, range, decimals, 78.0, mixed[index]));
                if index < 2 && super::super::common::icon_button(ui, p, "arrows-exchange", aurora_i18n::tr!("common-rotate"), true) {
                    values[index] = -values[index];
                    edit.changed = true;
                    edit.commit = true;
                }
            });
        });
    }
    edit
}

fn tail(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, sel: &[Sel]) {
    let planar = sel.iter().all(|x| x.te.media_flags & TEM_TEX_GEN_MASK == TEX_GEN_PLANAR);
    let mut align = tool.ui.planar_align;
    if ui
        .add_enabled_ui(planar, |ui| {
            widgets::checkbox(
                ui,
                p,
                &mut align,
                RichText::new(aurora_i18n::tr!("build-ui-texture-align-planar-faces")).size(11.0),
            )
        })
        .inner
        .changed()
    {
        tool.ui.planar_align = align;
    }
    if let Some(value) = check(
        ui,
        p,
        s.sync_materials,
        false,
        aurora_i18n::tr!("build-ui-texture-synchronize-materials"),
        true,
    ) {
        s.sync_materials = value;
    }
    let objects = sel.iter().map(|x| x.idx).collect::<std::collections::BTreeSet<_>>().len();
    if ui
        .add_enabled(planar && align && objects > 1, egui::Button::new(aurora_i18n::tr!("common-align")))
        .clicked()
    {
        tool.align_planar_faces(world, sel.iter().map(|x| (x.idx, x.face)).collect());
    }
}
