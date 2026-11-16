//! Features tab ("Features", LLPanelVolume, indra/newview/llpanelvolume.cpp,
//! originally LGPL 2.1): animated mesh, flexible path, light (and its
//! projector texture), reflection probe, physics shape and material.

use super::common::{Edit, check, color_swatch, combo, label, linear_to_srgb, row, small};
use super::{Confirm, Env, FeaturesClip, PickTarget};
use crate::build::edits::{CLICK_ACTION_SIT, CLICK_ACTION_TOUCH, Extra, FLEX_DEFAULT, LIGHT_DEFAULT, LIGHT_IMAGE_DEFAULT, PROBE_DEFAULT};
use crate::build::{BuildSettings, BuildTool};
use crate::theme::Palette;
use crate::ui::widgets::collapsing;
use crate::world::World;
use aurora_prim::extra::{EXTENDED_MESH_ANIMATED, FlexibleParams, ReflectionProbeParams};
use aurora_prim::params::{LL_PCODE_PATH_FLEXIBLE, LL_PCODE_PATH_LINE};

/// LL_MCODE_* in the "Material" combo order (material_codes.h).
fn materials() -> [(&'static str, u8); 7] {
    [
        (aurora_i18n::tr!("build-ui-features-stone"), 0),
        (aurora_i18n::tr!("build-ui-features-metal"), 1),
        (aurora_i18n::tr!("build-ui-features-glass"), 2),
        (aurora_i18n::tr!("build-ui-features-wood"), 3),
        (aurora_i18n::tr!("extra-chair"), 4),
        (aurora_i18n::tr!("build-ui-features-plastic"), 5),
        (aurora_i18n::tr!("build-ui-features-rubber"), 6),
    ]
}
/// LL_MCODE_LIGHT: legacy fullbright material.
const LL_MCODE_LIGHT: u8 = 7;
/// LLMaterialTable friction and restitution of each material code.
const FRICTION: [f32; 7] = [0.8, 0.3, 0.2, 0.6, 0.9, 0.4, 0.9];
const RESTITUTION: [f32; 7] = [0.4, 0.4, 0.7, 0.5, 0.3, 0.7, 0.9];

/// "Physics shape type" (PHYSICS_SHAPE_*): None 1, Prim 0, Convex hull 2.
fn physics_shapes() -> [(&'static str, u8); 3] {
    [
        (aurora_i18n::tr!("common-none"), 1),
        (aurora_i18n::tr!("build-ui-features-prim"), 0),
        (aurora_i18n::tr!("build-ui-features-convex-hull"), 2),
    ]
}

/// Reflection probe flags (LLReflectionProbeParams).
const PROBE_BOX: u8 = 0x01;
const PROBE_DYNAMIC: u8 = 0x02;

const W: f32 = 56.0;

// LLPanelVolume::getState (indra/newview/llpanelvolume.cpp) clears disabled
// flexible, light and probe parameters. Physics values remain readable.
fn spin(
    ui: &mut egui::Ui,
    value: &mut f32,
    increment: f32,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
    width: f32,
    mixed: bool,
) -> Edit {
    if ui.is_enabled() {
        super::common::spin(ui, value, increment, range, decimals, width, mixed)
    } else {
        super::common::empty_spin(ui, width)
    }
}

pub fn show(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, env: &mut Env) {
    let Some(key) = tool.selection.first().copied() else { return };
    let Some(idx) = world.objects.index_of(&key) else { return };
    let Some(o) = world.objects.get(idx) else { return };
    let single = tool.sel_prims(world).len() == 1;
    let editable = tool.can_modify(world, idx) && !crate::build::linking::permanent(world, crate::build::root_of(world, idx));
    let enabled = editable && single;
    let (extra, volume, material, click, flexible) = (o.extra.clone(), o.volume, o.prim_material, o.click_action, o.has_flexible_path());
    let mesh = volume.is_mesh();
    let legacy_canvas =
        super::legacy::active(ui).then(|| egui::Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 460.0)));

    ui.horizontal(|ui| {
        small(
            ui,
            p,
            if single {
                aurora_i18n::tr!("build-ui-features-edit-object-attributes")
            } else {
                aurora_i18n::tr!("build-ui-features-select-a-prim-to-change-its-attributes")
            },
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if super::common::icon_button(
                ui,
                p,
                "clipboard-text-filled",
                aurora_i18n::tr!("build-ui-features-paste-attributes"),
                enabled && tool.ui.features_clipboard.is_some(),
            ) && let Some(c) = tool.ui.features_clipboard
            {
                paste_features(tool, world, idx, c);
            }
            if super::common::icon_button(ui, p, "copy-filled", aurora_i18n::tr!("build-ui-features-copy-attributes"), enabled) {
                let (light_image, keep_light_image) = light_image_copy(world, extra.light_image);
                tool.ui.features_clipboard = Some(FeaturesClip {
                    flexible: flexible.then_some(extra.flexible.unwrap_or(FLEX_DEFAULT)),
                    light: extra.light,
                    light_image,
                    keep_light_image,
                    probe: extra.reflection_probe,
                    material: material & 0x0f,
                    physics: tool.physics.get(&key.local_id).copied(),
                });
            }
        });
    });

    let animated = extra.extended_mesh_flags.unwrap_or(0) & EXTENDED_MESH_ANIMATED != 0;
    if let Some(v) = check(
        ui,
        p,
        animated,
        false,
        aurora_i18n::tr!("build-ui-features-animated-mesh"),
        tool.animated_mesh_editable(world),
    ) {
        let flags = extra.extended_mesh_flags.unwrap_or(0);
        let f = if v {
            flags | EXTENDED_MESH_ANIMATED
        } else {
            flags & !EXTENDED_MESH_ANIMATED
        };
        tool.set_extra(world, idx, Extra::ExtendedMesh(Some(f)));
    }
    let (flexibility, open) = (aurora_i18n::tr!("build-ui-features-flexibility"), flexible);
    feature_group(
        ui,
        p,
        "attributes_flex",
        flexibility,
        open,
        legacy_canvas,
        [0.0, 43.0, 130.0, 205.0],
        |ui| {
            let can_flex = matches!(volume.path.curve_type, LL_PCODE_PATH_LINE | LL_PCODE_PATH_FLEXIBLE) && !mesh;
            if let Some(v) = check(
                ui,
                p,
                flexible,
                false,
                aurora_i18n::tr!("build-ui-features-flexibility"),
                enabled && can_flex,
            ) {
                set_flexible(tool, world, idx, v, click);
            }
            let flex_on = flexible && enabled && !mesh;
            let mut f = extra.flexible.unwrap_or(FLEX_DEFAULT);
            let mut e = Edit::default();
            ui.add_enabled_ui(flex_on, |ui| {
                let mut soft = f.softness as i32;
                row(ui, p, aurora_i18n::tr!("build-ui-features-softness"), 70.0, |ui| {
                    let r = if flex_on {
                        super::common::spin_i(ui, &mut soft, 0..=3, W)
                    } else {
                        super::common::empty_spin(ui, W)
                    };
                    e = e.or(r);
                });
                f.softness = soft as u8;
                for (name, v, range, inc) in [
                    (aurora_i18n::tr!("build-ui-features-gravity"), &mut f.gravity, -10.0..=10.0, 0.5),
                    (
                        aurora_i18n::tr!("build-ui-features-elasticity"),
                        &mut f.air_friction,
                        0.0..=10.0,
                        0.5,
                    ),
                    (aurora_i18n::tr!("common-wind"), &mut f.wind_sensitivity, 0.0..=10.0, 0.5),
                    (aurora_i18n::tr!("build-ui-features-tension"), &mut f.tension, 0.0..=10.0, 0.5),
                ] {
                    row(ui, p, name, 70.0, |ui| e = e.or(spin(ui, v, inc, range, 3, W, false)));
                }
                let mut force = f.user_force.to_array();
                for (i, name) in [
                    aurora_i18n::tr!("build-ui-features-force-x"),
                    aurora_i18n::tr!("build-ui-features-force-y"),
                    aurora_i18n::tr!("build-ui-features-force-z"),
                ]
                .iter()
                .enumerate()
                {
                    row(ui, p, name, 70.0, |ui| {
                        e = e.or(spin(ui, &mut force[i], 0.01, -10.0..=10.0, 3, W, false))
                    });
                }
                f.user_force = glam::Vec3::from_array(force);
            });
            if e.commit && flex_on && Some(f) != extra.flexible {
                tool.set_extra(world, idx, Extra::Flexible(Some(f)));
            }
        },
    );
    let physics = aurora_i18n::tr!("build-ui-features-physics-shape-type");
    let demo_physics = env.demo && std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("physics"));
    feature_group(
        ui,
        p,
        "attributes_physics",
        physics,
        demo_physics,
        legacy_canvas,
        [140.0, 43.0, 135.0, 205.0],
        |ui| {
            label(ui, p, aurora_i18n::tr!("build-ui-features-physics-shape-type"));
            let phys = tool.physics.get(&key.local_id).copied();
            ui.horizontal(|ui| {
                if let Some(t) = combo(ui, "physics_shape", phys.map(|x| x.shape_type), &physics_shapes(), 110.0, editable) {
                    tool.set_physics(world, |x| x.shape_type = t);
                }
                let eye = if s.show_physics_shape { "eye-filled" } else { "eye-off" };
                if super::common::icon_button(
                    ui,
                    p,
                    eye,
                    aurora_i18n::tr!("build-ui-features-show-physics-shape-while-editing"),
                    true,
                ) {
                    s.show_physics_shape = !s.show_physics_shape;
                }
            });
            let mcode = material & 0x0f;
            let mut mats: Vec<(&str, u8)> = materials().to_vec();
            if mcode == LL_MCODE_LIGHT {
                mats.push((aurora_i18n::tr!("build-ui-features-fullbright-legacy"), LL_MCODE_LIGHT));
            }
            if let Some(m) = combo(ui, "material", Some(mcode), &mats, 110.0, enabled && mcode != LL_MCODE_LIGHT)
                && m != LL_MCODE_LIGHT
            {
                tool.set_material(world, m);
                // the panel shows the material's values (not sent)
                let k = m as usize;
                if let Some(x) = tool.physics.get_mut(&key.local_id) {
                    x.gravity_multiplier = 1.0;
                    x.friction = FRICTION[k];
                    x.density = 1000.0;
                    x.restitution = RESTITUTION[k];
                }
            }
            let mut ph = phys.unwrap_or_default();
            let mut e = Edit::default();
            ui.add_enabled_ui(editable && phys.is_some(), |ui| {
                row(ui, p, aurora_i18n::tr!("build-ui-features-gravity"), 70.0, |ui| {
                    e = e.or(super::common::spin(ui, &mut ph.gravity_multiplier, 1.0, -1.0..=28.0, 3, W, false))
                });
                row(ui, p, aurora_i18n::tr!("common-friction"), 70.0, |ui| {
                    e = e.or(super::common::spin(ui, &mut ph.friction, 0.1, 0.0..=255.0, 3, W, false))
                });
                row(ui, p, aurora_i18n::tr!("build-ui-features-density-in-100-kg-m-3"), 70.0, |ui| {
                    e = e.or(super::common::spin(ui, &mut ph.density, 0.1, 1.0..=22587.0, 2, W, false))
                });
                row(ui, p, aurora_i18n::tr!("common-restitution"), 70.0, |ui| {
                    e = e.or(super::common::spin(ui, &mut ph.restitution, 0.01, 0.0..=1.0, 3, W, false))
                });
            });
            if e.commit && Some(ph) != phys {
                tool.set_physics(world, |x| {
                    x.gravity_multiplier = ph.gravity_multiplier;
                    x.friction = ph.friction;
                    x.density = ph.density;
                    x.restitution = ph.restitution;
                });
            }
        },
    );
    let (light, open) = (aurora_i18n::tr!("common-light"), extra.light.is_some());
    feature_group(
        ui,
        p,
        "attributes_light",
        light,
        open,
        legacy_canvas,
        [0.0, 255.0, 275.0, 112.0],
        |ui| {
            light_section(ui, p, tool, world, idx, &extra, enabled, env);
        },
    );
    let (probe, open) = (aurora_i18n::tr!("common-reflection-probe"), extra.reflection_probe.is_some());
    feature_group(
        ui,
        p,
        "attributes_probe",
        probe,
        open,
        legacy_canvas,
        [0.0, 360.0, 275.0, 80.0],
        |ui| {
            probe_section(ui, p, tool, world, idx, &extra, enabled && !mesh);
        },
    );
    if let Some(rect) = legacy_canvas {
        ui.advance_cursor_after_rect(rect);
    }
}

/// The reference volume panel keeps all groups exposed, with unavailable
/// parameters cleared. Aurora retains its independent folding sections.
#[allow(clippy::too_many_arguments)]
fn feature_group(
    ui: &mut egui::Ui,
    p: &Palette,
    id: &str,
    title: &str,
    open: bool,
    canvas: Option<egui::Rect>,
    bounds: [f32; 4],
    body: impl FnOnce(&mut egui::Ui),
) {
    if let Some(canvas) = canvas {
        let scale = canvas.width() / 275.0;
        let rect = egui::Rect::from_min_size(
            canvas.min + egui::vec2(bounds[0] * scale, bounds[1]),
            egui::vec2(bounds[2] * scale, bounds[3]),
        );
        let mut child = super::legacy::pane(ui, rect, id);
        body(&mut child);
    } else {
        collapsing(ui, p, id, title, open, body);
    }
}

/// LLVOVolume::setIsFlexible + LLPanelVolume::onCommitIsFlexible: the
/// object becomes phantom and non physical, its line path flexible.
fn set_flexible(tool: &mut BuildTool, world: &mut World, idx: usize, on: bool, click: u8) {
    if on && click == CLICK_ACTION_SIT {
        tool.set_click_action(world, CLICK_ACTION_TOUCH);
    }
    if on {
        tool.set_flags(world, Some(false), None, Some(true));
        tool.set_extra(world, idx, Extra::Flexible(Some(FLEX_DEFAULT)));
    } else {
        tool.set_flags(world, None, None, Some(false));
        tool.set_extra(world, idx, Extra::Flexible(None));
    }
    if let Some(mut v) = world.objects.get(idx).map(|o| o.volume) {
        v.path.curve_type = if on { LL_PCODE_PATH_FLEXIBLE } else { LL_PCODE_PATH_LINE };
        tool.set_shape(world, idx, v);
    }
}

fn light_section(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    idx: usize,
    extra: &aurora_prim::extra::ExtraParams,
    enabled: bool,
    env: &mut Env,
) {
    let on = extra.light.is_some() && enabled;
    let l = extra.light.unwrap_or(LIGHT_DEFAULT);
    let img = extra.light_image;
    let legacy = super::legacy::active(ui);
    let swatch_size = if legacy { 32.0 } else { 40.0 };
    ui.horizontal(|ui| {
        if let Some(v) = check(ui, p, extra.light.is_some(), false, aurora_i18n::tr!("common-light"), enabled) {
            tool.set_extra(world, idx, Extra::Light(v.then_some(LIGHT_DEFAULT)));
        }
        ui.add_space(12.0);
        // the swatch shows sRGB; the wire carries linear (setLightSRGBColor)
        let srgb = [linear_to_srgb(l.color[0]), linear_to_srgb(l.color[1]), linear_to_srgb(l.color[2])];
        let swatch = if on {
            color_swatch(
                ui,
                p,
                srgb,
                if legacy { "" } else { aurora_i18n::tr!("common-color") },
                swatch_size,
                on,
            )
        } else {
            super::common::swatch(
                ui,
                p,
                env.images,
                &mut tool.ui.wanted_images,
                None,
                if legacy { "" } else { aurora_i18n::tr!("common-color") },
                swatch_size,
                false,
                false,
            )
        }
        .on_hover_text(aurora_i18n::tr!("common-color"));
        if let Some(object) = world.objects.get(idx) {
            tool.color_swatch_response(world, env, super::ColorTarget::Light(object.full_id), srgb, swatch);
        }
        let r = super::common::swatch(
            ui,
            p,
            env.images,
            &mut tool.ui.wanted_images,
            world
                .objects
                .get(idx)
                .filter(|_| on)
                .and_then(|object| object.visual_light_image())
                .map(|image| image.texture),
            if legacy { "" } else { aurora_i18n::tr!("common-texture") },
            swatch_size,
            false,
            on,
        )
        .on_hover_text(aurora_i18n::tr!("common-texture"));
        super::texture::texture_drop::slot(ui, p, tool, world, r.rect, PickTarget::LightTexture, on);
        let demo_picker = egui::Id::unique("demo_light_texture_picker_opened");
        let demo_open = env.demo
            && on
            && std::env::var("AURORA_DEMO_BUILD_TAB").is_ok_and(|v| v == "features:light-picker")
            && !ui.ctx().data(|d| d.get_temp::<bool>(demo_picker).unwrap_or(false));
        if r.clicked() || demo_open {
            // LLPanelVolume's light texture control allows no texture:
            // clearing it disables only PARAMS_LIGHT_IMAGE, keeping the light.
            tool.ui.picker.open_with_defaults(
                aurora_i18n::tr!("build-ui-features-choose-light-texture"),
                img.map(|i| i.texture).unwrap_or_default(),
                None,
                true,
            );
            tool.ui.pick_target = Some(PickTarget::LightTexture);
            if demo_open {
                ui.ctx().data_mut(|d| d.insert_temp(demo_picker, true));
            }
        }
    });
    let mut nl = l;
    let mut ni = img.unwrap_or(LIGHT_IMAGE_DEFAULT);
    let mut e = Edit::default();
    let mut ei = Edit::default();
    let mut light_values = |ui: &mut egui::Ui| {
        for (name, value, inc, max) in [
            (aurora_i18n::tr!("build-ui-features-intensity"), &mut nl.color[3], 0.1, 1.0),
            (aurora_i18n::tr!("build-ui-features-range"), &mut nl.radius, 0.1, 20.0),
            (aurora_i18n::tr!("build-ui-features-falloff"), &mut nl.falloff, 0.25, 2.0),
        ] {
            row(ui, p, name, 70.0, |ui| e = e.or(spin(ui, value, inc, 0.0..=max, 3, W, false)));
        }
    };
    let mut spot_values = |ui: &mut egui::Ui| {
        ui.add_enabled_ui(img.is_some(), |ui| {
            let mut v = ni.params.to_array();
            for (i, (name, inc, range)) in [
                (aurora_i18n::tr!("build-ui-features-field-of-view"), 0.1, 0.0..=3.0),
                (aurora_i18n::tr!("build-ui-features-center-point"), 0.5, -20.0..=20.0),
                (aurora_i18n::tr!("common-ambient"), 0.05, 0.0..=1.0),
            ]
            .into_iter()
            .enumerate()
            {
                row(ui, p, name, 70.0, |ui| ei = ei.or(spin(ui, &mut v[i], inc, range, 3, W, false)));
            }
            ni.params = glam::Vec3::from_array(v);
        });
    };
    ui.add_enabled_ui(on, |ui| {
        if super::legacy::active(ui) {
            ui.columns(2, |columns| {
                light_values(&mut columns[0]);
                spot_values(&mut columns[1]);
            });
        } else {
            ui.vertical(|ui| {
                light_values(ui);
                spot_values(ui);
            });
        }
    });
    if e.commit && nl != l {
        tool.set_extra(world, idx, Extra::Light(Some(nl)));
    }
    if ei.commit && Some(ni) != img {
        tool.set_extra(world, idx, Extra::LightImage(Some(ni)));
    }
}

fn probe_section(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    idx: usize,
    extra: &aurora_prim::extra::ExtraParams,
    enabled: bool,
) {
    let probe = extra.reflection_probe;
    let on = probe.is_some() && enabled;
    let pr = probe.unwrap_or(PROBE_DEFAULT);
    let mut np = pr;
    let mut send = false;
    let shape_items = [
        (aurora_i18n::tr!("common-sphere"), 0u8),
        (aurora_i18n::tr!("common-box"), PROBE_BOX),
    ];
    ui.horizontal(|ui| {
        if let Some(v) = check(ui, p, probe.is_some(), false, aurora_i18n::tr!("common-reflection-probe"), enabled) {
            if v {
                // ReflectionProbeApplied: confirmed first
                tool.ui.confirm = Some(Confirm::Probe);
            } else {
                tool.set_extra(world, idx, Extra::Probe(None));
            }
        }
        if let Some(b) = combo(ui, "probe_shape", on.then_some(pr.flags & PROBE_BOX), &shape_items, 90.0, on) {
            np.flags = (pr.flags & !PROBE_BOX) | b;
            send = true;
        }
    });
    ui.add_enabled_ui(on, |ui| {
        if let Some(d) = check(
            ui,
            p,
            on && pr.flags & PROBE_DYNAMIC != 0,
            false,
            aurora_i18n::tr!("build-ui-features-dynamic"),
            on,
        ) {
            np.flags = if d { np.flags | PROBE_DYNAMIC } else { np.flags & !PROBE_DYNAMIC };
            send = true;
        }
        let mut e = Edit::default();
        let mut field = |ui: &mut egui::Ui, ambient: bool| {
            let (name, value, max) = if ambient {
                (aurora_i18n::tr!("common-ambient"), &mut np.ambiance, 100.0)
            } else {
                (aurora_i18n::tr!("build-ui-features-coverage"), &mut np.clip_distance, 1024.0)
            };
            row(ui, p, name, 70.0, |ui| spin(ui, value, 0.05, 0.0..=max, 3, W, false))
        };
        if super::legacy::active(ui) {
            ui.columns(2, |columns| {
                e = e.or(field(&mut columns[0], true));
                e = e.or(field(&mut columns[1], false));
            });
        } else {
            ui.vertical(|ui| {
                e = e.or(field(ui, true));
                e = e.or(field(ui, false));
            });
        }
        send |= e.commit;
    });
    if send && np != pr && on {
        tool.set_extra(world, idx, Extra::Probe(Some(np)));
    }
}

impl BuildTool {
    /// After the ReflectionProbeApplied confirmation (LLPanelVolume::
    /// onCommitIsReflectionProbe): probe on, phantom, box or sphere shape.
    pub fn make_probe(&mut self, world: &mut World, idx: usize) {
        let Some(v) = world.objects.get(idx).map(|o| o.volume) else {
            return;
        };
        let boxed = v.path.curve_type == LL_PCODE_PATH_LINE;
        let pr = ReflectionProbeParams {
            flags: if boxed { PROBE_BOX } else { 0 },
            ..PROBE_DEFAULT
        };
        self.set_extra(world, idx, Extra::Probe(Some(pr)));
        self.set_flags(world, None, None, Some(true));
        // Firestorm keeps a box, otherwise a sphere (half circle on a circle path)
        let raw = if boxed {
            crate::build::shapes::shapes()[0].raw()
        } else {
            crate::build::shapes::shapes()[8].raw()
        };
        self.set_shape(world, idx, raw.to_params());
        // fully transparent
        self.edit_faces(world, |f, _, _| f.color[3] = 0.0);
    }
}

/// The projector of "Copy" (LLPanelVolume::onCopyFeatures): kept only when
/// the agent may copy its map; otherwise nothing, and the paste leaves the
/// target's projector alone (the second value).
pub(super) fn light_image_copy(
    world: &World,
    image: Option<aurora_prim::extra::LightImageParams>,
) -> (Option<aurora_prim::extra::LightImageParams>, bool) {
    match image {
        Some(i) if !crate::build::texture_copy::can_copy(&world.inventory, world.agent_id, i.texture) => (None, true),
        other => (other, false),
    }
}

/// "Paste" of the features (LLPanelVolume::onPasteFeatures).
/// LLPanelVolume::onPasteFeatures: copied flexibility makes a line or
/// flexible path flexible (LLVOVolume::setIsFlexible), then takes the copied
/// parameters; a paste without it turns a flexible path back into a line.
/// The path, not the block, decides, as for the checkbox.
fn paste_flexible(tool: &mut BuildTool, world: &mut World, idx: usize, clip: Option<FlexibleParams>) {
    let Some(o) = world.objects.get(idx) else { return };
    let (was, click) = (o.has_flexible_path(), o.click_action);
    let can = matches!(o.volume.path.curve_type, LL_PCODE_PATH_LINE | LL_PCODE_PATH_FLEXIBLE);
    match clip {
        Some(f) if can => {
            if !was {
                set_flexible(tool, world, idx, true, click);
            }
            if world.objects.get(idx).is_some_and(|o| o.extra.flexible != Some(f)) {
                tool.set_extra(world, idx, Extra::Flexible(Some(f)));
            }
        }
        _ if was => set_flexible(tool, world, idx, false, click),
        _ => {}
    }
}

fn paste_features(tool: &mut BuildTool, world: &mut World, idx: usize, c: FeaturesClip) {
    paste_flexible(tool, world, idx, c.flexible);
    let current = world.objects.get(idx).map(|o| o.extra.clone()).unwrap_or_default();
    if current.light != c.light {
        tool.set_extra(world, idx, Extra::Light(c.light));
    }
    if !c.keep_light_image && current.light_image != c.light_image {
        tool.set_extra(world, idx, Extra::LightImage(c.light_image));
    }
    if current.reflection_probe != c.probe {
        tool.set_extra(world, idx, Extra::Probe(c.probe));
    }
    tool.set_material(world, c.material);
    if let Some(ph) = c.physics {
        tool.set_physics(world, |x| *x = ph);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_projector_the_agent_cannot_copy_leaves_the_target_its_own() {
        let mut w = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let map = uuid::Uuid::from_u128(0x11a1);
        let image = Some(aurora_prim::extra::LightImageParams {
            texture: map,
            ..crate::build::edits::LIGHT_IMAGE_DEFAULT
        });
        assert_eq!(light_image_copy(&w, image), (None, true));
        assert_eq!(
            light_image_copy(&w, None),
            (None, false),
            "no projector: the paste removes the target's"
        );
        let id = uuid::Uuid::from_u128(0x11a2);
        let agent = w.agent_id;
        w.inventory.items.insert(
            id,
            aurora_net::inventory::InvItem {
                id,
                asset_id: map,
                owner: agent,
                owner_mask: crate::world::inventory::actions::COPY,
                ..Default::default()
            },
        );
        assert_eq!(light_image_copy(&w, image), (image, false));
    }

    /// A demo box the agent may edit, selected, on a line path.
    fn editable_box() -> (World, BuildTool, usize) {
        let mut world = World::new(std::sync::Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        world.agent_id = crate::demo::DEMO_AGENT;
        for event in crate::demo::events() {
            if matches!(event, aurora_net::NetEvent::ObjectUpdates { .. }) {
                world.apply(event);
            }
        }
        let idx = world
            .objects
            .iter()
            .find(|(_, o)| o.key.local_id == 118)
            .map(|(idx, _)| idx)
            .unwrap();
        let mut tool = BuildTool::default();
        let o = world.objects.get_mut(idx).unwrap();
        o.owner_id = world.agent_id;
        o.update_flags |= crate::ui::context::flags::OBJECT_MODIFY;
        o.volume.path.curve_type = LL_PCODE_PATH_LINE;
        o.volume.sculpt = None;
        o.extra = Default::default();
        tool.selection.push(o.key);
        (world, tool, idx)
    }

    #[test]
    fn pasted_flexibility_follows_the_path_like_the_legacy_viewer() {
        let own = FlexibleParams {
            softness: 3,
            gravity: -1.0,
            ..FLEX_DEFAULT
        };
        let path = |w: &World, idx: usize| w.objects.get(idx).unwrap().volume.path.curve_type;

        // Copied flexibility makes a line path flexible with the copied block.
        let (mut w, mut tool, idx) = editable_box();
        paste_flexible(&mut tool, &mut w, idx, Some(own));
        assert_eq!(path(&w, idx), LL_PCODE_PATH_FLEXIBLE);
        assert_eq!(w.objects.get(idx).unwrap().extra.flexible, Some(own));

        // A paste without it turns the path back into a line, block or not.
        paste_flexible(&mut tool, &mut w, idx, None);
        assert_eq!(path(&w, idx), LL_PCODE_PATH_LINE);
        assert_eq!(w.objects.get(idx).unwrap().extra.flexible, None);
        let o = w.objects.get_mut(idx).unwrap();
        o.volume.path.curve_type = LL_PCODE_PATH_FLEXIBLE;
        paste_flexible(&mut tool, &mut w, idx, None);
        assert_eq!(path(&w, idx), LL_PCODE_PATH_LINE, "a flexible path that came without the block");

        // A flexible sculpt keeps its path and takes the copied block.
        let o = w.objects.get_mut(idx).unwrap();
        o.volume.path.curve_type = LL_PCODE_PATH_FLEXIBLE;
        o.volume.sculpt = Some(aurora_prim::params::SculptParams::default());
        o.extra.flexible = Some(FLEX_DEFAULT);
        paste_flexible(&mut tool, &mut w, idx, Some(own));
        assert_eq!(path(&w, idx), LL_PCODE_PATH_FLEXIBLE);
        assert_eq!(w.objects.get(idx).unwrap().extra.flexible, Some(own));

        // Another path cannot be flexible: nothing changes.
        let o = w.objects.get_mut(idx).unwrap();
        o.volume.sculpt = None;
        o.volume.path.curve_type = aurora_prim::params::LL_PCODE_PATH_CIRCLE;
        o.extra.flexible = None;
        let sent = tool.out.len();
        paste_flexible(&mut tool, &mut w, idx, Some(own));
        assert_eq!(path(&w, idx), aurora_prim::params::LL_PCODE_PATH_CIRCLE);
        assert_eq!(tool.out.len(), sent);
    }
}
