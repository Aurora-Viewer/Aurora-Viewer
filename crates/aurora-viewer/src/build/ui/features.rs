//! Features tab ("Features", LLPanelVolume, indra/newview/llpanelvolume.cpp,
//! originally LGPL 2.1): animated mesh, flexible path, light (and its
//! projector texture), reflection probe, physics shape and material.

use super::common::{Edit, check, color_swatch, combo, label, linear_to_srgb, row, small, spin};
use super::{Confirm, Env, FeaturesClip, PickTarget};
use crate::build::edits::{CLICK_ACTION_SIT, CLICK_ACTION_TOUCH, Extra, FLEX_DEFAULT, LIGHT_DEFAULT, LIGHT_IMAGE_DEFAULT, PROBE_DEFAULT};
use crate::build::{BuildSettings, BuildTool};
use crate::theme::Palette;
use crate::ui::widgets::collapsing;
use crate::world::World;
use aurora_prim::extra::{EXTENDED_MESH_ANIMATED, ReflectionProbeParams};
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

pub fn show(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, env: &mut Env) {
    let Some(key) = tool.selection.first().copied() else { return };
    let Some(idx) = world.objects.index_of(&key) else { return };
    let Some(o) = world.objects.get(idx) else { return };
    let single = tool.sel_prims(world).len() == 1;
    let editable = tool.can_modify(world, idx);
    let enabled = editable && single;
    let (extra, volume, material, is_root, click) = (o.extra.clone(), o.volume, o.prim_material, o.parent_id == 0, o.click_action);
    let mesh = volume.is_mesh();

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
                tool.ui.features_clipboard = Some(FeaturesClip {
                    flexible: extra.flexible,
                    light: extra.light,
                    light_image: extra.light_image,
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
        enabled && is_root && mesh,
    ) {
        let flags = extra.extended_mesh_flags.unwrap_or(0);
        let f = if v {
            flags | EXTENDED_MESH_ANIMATED
        } else {
            flags & !EXTENDED_MESH_ANIMATED
        };
        tool.set_extra(world, idx, Extra::ExtendedMesh(Some(f)));
    }
    let (flexibility, open) = (aurora_i18n::tr!("build-ui-features-flexibility"), extra.flexible.is_some());
    collapsing(ui, p, "attributes_flex", flexibility, open, |ui| {
        let can_flex = matches!(volume.path.curve_type, LL_PCODE_PATH_LINE | LL_PCODE_PATH_FLEXIBLE) && !mesh;
        if let Some(v) = check(
            ui,
            p,
            extra.flexible.is_some(),
            false,
            aurora_i18n::tr!("build-ui-features-flexibility"),
            enabled && can_flex,
        ) {
            set_flexible(tool, world, idx, v, click);
        }
        let flex_on = extra.flexible.is_some() && enabled;
        let mut f = extra.flexible.unwrap_or(FLEX_DEFAULT);
        let mut e = Edit::default();
        ui.add_enabled_ui(flex_on, |ui| {
            let mut soft = f.softness as i32;
            row(ui, p, aurora_i18n::tr!("build-ui-features-softness"), 70.0, |ui| {
                let r = super::common::spin_i(ui, &mut soft, 0..=3, W);
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
    });
    let physics = aurora_i18n::tr!("build-ui-features-physics-shape-type");
    let demo_physics = env.demo && std::env::var("AURORA_DEMO_BUILD").is_ok_and(|v| v.starts_with("physics"));
    collapsing(ui, p, "attributes_physics", physics, demo_physics, |ui| {
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
                e = e.or(spin(ui, &mut ph.gravity_multiplier, 1.0, -1.0..=28.0, 3, W, false))
            });
            row(ui, p, aurora_i18n::tr!("common-friction"), 70.0, |ui| {
                e = e.or(spin(ui, &mut ph.friction, 0.1, 0.0..=255.0, 3, W, false))
            });
            row(ui, p, aurora_i18n::tr!("build-ui-features-density-in-100-kg-m-3"), 70.0, |ui| {
                e = e.or(spin(ui, &mut ph.density, 0.1, 1.0..=22587.0, 2, W, false))
            });
            row(ui, p, aurora_i18n::tr!("common-restitution"), 70.0, |ui| {
                e = e.or(spin(ui, &mut ph.restitution, 0.01, 0.0..=1.0, 3, W, false))
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
    });
    let (light, open) = (aurora_i18n::tr!("common-light"), extra.light.is_some());
    collapsing(ui, p, "attributes_light", light, open, |ui| {
        light_section(ui, p, tool, world, idx, &extra, enabled, env);
    });
    let (probe, open) = (aurora_i18n::tr!("common-reflection-probe"), extra.reflection_probe.is_some());
    collapsing(ui, p, "attributes_probe", probe, open, |ui| {
        probe_section(ui, p, tool, world, idx, &extra, enabled && !mesh);
    });
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
    ui.horizontal(|ui| {
        if let Some(v) = check(ui, p, extra.light.is_some(), false, aurora_i18n::tr!("common-light"), enabled) {
            tool.set_extra(world, idx, Extra::Light(v.then_some(LIGHT_DEFAULT)));
        }
        ui.add_space(12.0);
        // the swatch shows sRGB; the wire carries linear (setLightSRGBColor)
        let srgb = [linear_to_srgb(l.color[0]), linear_to_srgb(l.color[1]), linear_to_srgb(l.color[2])];
        let swatch = color_swatch(ui, p, srgb, aurora_i18n::tr!("common-color"), 40.0, on);
        if let Some(object) = world.objects.get(idx) {
            tool.color_swatch_response(world, env, super::ColorTarget::Light(object.full_id), srgb, swatch);
        }
        let r = super::common::swatch(
            ui,
            p,
            env.images,
            &mut tool.ui.wanted_images,
            img.map(|i| i.texture),
            aurora_i18n::tr!("common-texture"),
            40.0,
            false,
            on,
        );
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
    ui.add_enabled_ui(on, |ui| {
        ui.vertical(|ui| {
            row(ui, p, aurora_i18n::tr!("build-ui-features-intensity"), 70.0, |ui| {
                e = e.or(spin(ui, &mut nl.color[3], 0.1, 0.0..=1.0, 3, W, false))
            });
            row(ui, p, aurora_i18n::tr!("build-ui-features-range"), 70.0, |ui| {
                e = e.or(spin(ui, &mut nl.radius, 0.1, 0.0..=20.0, 3, W, false))
            });
            row(ui, p, aurora_i18n::tr!("build-ui-features-falloff"), 70.0, |ui| {
                e = e.or(spin(ui, &mut nl.falloff, 0.25, 0.0..=2.0, 3, W, false))
            });
            // spot light parameters: only with a projector texture
            ui.add_enabled_ui(img.is_some(), |ui| {
                let mut v = ni.params.to_array();
                row(ui, p, aurora_i18n::tr!("build-ui-features-field-of-view"), 70.0, |ui| {
                    ei = ei.or(spin(ui, &mut v[0], 0.1, 0.0..=3.0, 3, W, false))
                });
                row(ui, p, aurora_i18n::tr!("build-ui-features-center-point"), 70.0, |ui| {
                    ei = ei.or(spin(ui, &mut v[1], 0.5, -20.0..=20.0, 3, W, false))
                });
                row(ui, p, aurora_i18n::tr!("common-ambient"), 70.0, |ui| {
                    ei = ei.or(spin(ui, &mut v[2], 0.05, 0.0..=1.0, 3, W, false))
                });
                ni.params = glam::Vec3::from_array(v);
            });
        });
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
        if let Some(b) = combo(ui, "probe_shape", Some(pr.flags & PROBE_BOX), &shape_items, 90.0, on) {
            np.flags = (pr.flags & !PROBE_BOX) | b;
            send = true;
        }
    });
    ui.add_enabled_ui(on, |ui| {
        if let Some(d) = check(
            ui,
            p,
            pr.flags & PROBE_DYNAMIC != 0,
            false,
            aurora_i18n::tr!("build-ui-features-dynamic"),
            on,
        ) {
            np.flags = if d { np.flags | PROBE_DYNAMIC } else { np.flags & !PROBE_DYNAMIC };
            send = true;
        }
        let mut e = Edit::default();
        ui.vertical(|ui| {
            row(ui, p, aurora_i18n::tr!("common-ambient"), 70.0, |ui| {
                e = e.or(spin(ui, &mut np.ambiance, 0.05, 0.0..=100.0, 3, W, false))
            });
            row(ui, p, aurora_i18n::tr!("build-ui-features-coverage"), 70.0, |ui| {
                e = e.or(spin(ui, &mut np.clip_distance, 0.05, 0.0..=1024.0, 3, W, false))
            });
        });
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

/// "Paste" of the features (LLPanelVolume::onPasteFeatures).
fn paste_features(tool: &mut BuildTool, world: &mut World, idx: usize, c: FeaturesClip) {
    let current = world.objects.get(idx).map(|o| o.extra.clone()).unwrap_or_default();
    if current.flexible != c.flexible {
        tool.set_extra(world, idx, Extra::Flexible(c.flexible));
    }
    if current.light != c.light {
        tool.set_extra(world, idx, Extra::Light(c.light));
    }
    if current.light_image != c.light_image {
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
