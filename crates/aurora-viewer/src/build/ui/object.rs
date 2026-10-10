//! Object tab (LLPanelObject, indra/newview/llpanelobject.cpp, Firestorm,
//! originally LGPL 2.1): lock / physical / temporary / phantom, position,
//! size and rotation with their C / P / p buttons, the prim shape (22
//! types, Firestorm's « Working33 » extra paths included) and sculpties.

use super::common::{Edit, check, combo, label, section, small, spin};
use super::{Env, Request, ShapeClip};
use crate::build::{BuildSettings, BuildTool, MAX_PRIM_SCALE, MIN_PRIM_SCALE};
use crate::scene::Scene;
use crate::theme::Palette;
use crate::world::World;
use aurora_net::build::{perm, perm_field};
use aurora_prim::params::*;
use egui::{Color32, RichText};
use glam::{EulerRot, Quat, Vec3};
use std::time::Instant;

/// FLAGS_USE_PHYSICS / TEMPORARY_ON_REZ / PHANTOM (llprimitive.h).
use crate::build::edits::{FLAGS_PHANTOM, FLAGS_TEMPORARY_ON_REZ, FLAGS_USE_PHYSICS};

/// comboBaseType: (label, path, profile). Index 7 is « Sculptie ».
pub const TYPES: [(&str, u8, u8); 22] = [
    ("Boîte", LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE),
    ("Cylindre", LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE),
    ("Prisme", LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_EQUALTRI),
    ("Sphère", LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF),
    ("Tore", LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE),
    ("Tube", LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_SQUARE),
    ("Anneau", LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_EQUALTRI),
    ("Sculptie", LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE),
    ("Line->Half-Circle", LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE_HALF),
    ("Circle->Half-Circle", LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF),
    ("Circle2->Square", LL_PCODE_PATH_CIRCLE2, LL_PCODE_PROFILE_SQUARE),
    ("Circle2->Triangle", LL_PCODE_PATH_CIRCLE2, LL_PCODE_PROFILE_EQUALTRI),
    ("Circle2->Circle", LL_PCODE_PATH_CIRCLE2, LL_PCODE_PROFILE_CIRCLE),
    ("Circle2->Half-Circle", LL_PCODE_PATH_CIRCLE2, LL_PCODE_PROFILE_CIRCLE_HALF),
    ("Test->Square", LL_PCODE_PATH_TEST, LL_PCODE_PROFILE_SQUARE),
    ("Test->Triangle", LL_PCODE_PATH_TEST, LL_PCODE_PROFILE_EQUALTRI),
    ("Test->Circle", LL_PCODE_PATH_TEST, LL_PCODE_PROFILE_CIRCLE),
    ("Test->Half-Circle", LL_PCODE_PATH_TEST, LL_PCODE_PROFILE_CIRCLE_HALF),
    ("33->Circle", LL_PCODE_PATH_CIRCLE_33, LL_PCODE_PROFILE_CIRCLE),
    ("33->Square", LL_PCODE_PATH_CIRCLE_33, LL_PCODE_PROFILE_SQUARE),
    ("33->Triangle", LL_PCODE_PATH_CIRCLE_33, LL_PCODE_PROFILE_ISOTRI),
    ("33->HalfCircle", LL_PCODE_PATH_CIRCLE_33, LL_PCODE_PROFILE_CIRCLE_HALF),
];
const SCULPTED: usize = 7;

/// « Forme du creux » (hole type bits).
const HOLES: [(&str, u8); 4] = [
    ("Défaut", LL_PCODE_HOLE_SAME),
    ("Cercle", LL_PCODE_HOLE_CIRCLE),
    ("Carré", LL_PCODE_HOLE_SQUARE),
    ("Triangle", LL_PCODE_HOLE_TRIANGLE),
];

/// « Type de raccord » (sculpt stitching): Sphère, Tore, Plan, Cylindre.
const SCULPT_TYPES: [(&str, u8); 4] = [("Sphère", 1), ("Tore", 2), ("Plan", 3), ("Cylindre", 4)];

/// Sphere, torus, tube, ring: the "cut" is the path (T) cut there.
fn circular_basic(ty: usize) -> bool {
    matches!(ty, 3..=6)
}

/// Box, cylinder, prism: top size = 1 - ratio.
fn linear_basic(ty: usize) -> bool {
    ty <= 2
}

/// LLPanelObject::getState type detection.
pub fn detect_type(v: &VolumeParams) -> usize {
    if v.sculpt.is_some() {
        return SCULPTED;
    }
    let profile = v.profile.curve_type & LL_PCODE_PROFILE_MASK;
    let path = v.path.curve_type;
    if path == LL_PCODE_PATH_LINE || path == LL_PCODE_PATH_FLEXIBLE {
        return match profile {
            LL_PCODE_PROFILE_CIRCLE => 1,
            LL_PCODE_PROFILE_SQUARE => 0,
            LL_PCODE_PROFILE_ISOTRI | LL_PCODE_PROFILE_EQUALTRI | LL_PCODE_PROFILE_RIGHTTRI => 2,
            LL_PCODE_PROFILE_CIRCLE_HALF if path == LL_PCODE_PATH_LINE => 8,
            _ => 1,
        };
    }
    if path == LL_PCODE_PATH_CIRCLE {
        return match profile {
            LL_PCODE_PROFILE_CIRCLE if v.path.scale[1] > 0.75 => 3,
            LL_PCODE_PROFILE_CIRCLE => 4,
            LL_PCODE_PROFILE_CIRCLE_HALF => 3,
            LL_PCODE_PROFILE_EQUALTRI => 6,
            LL_PCODE_PROFILE_SQUARE if v.path.scale[1] <= 0.75 => 5,
            _ => 0,
        };
    }
    if path == LL_PCODE_PATH_CIRCLE2 && profile == LL_PCODE_PROFILE_CIRCLE {
        return 3;
    }
    TYPES
        .iter()
        .enumerate()
        .skip(8)
        .find(|(_, (_, pa, pr))| *pa == path && *pr == profile)
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// The shape as the panel shows it (LLPanelObject::getState).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeUi {
    pub ty: usize,
    pub hole: u8,
    pub cut: (f32, f32),
    /// Percent.
    pub hollow: f32,
    pub skew: f32,
    /// Degrees.
    pub twist: (f32, f32),
    /// « Biseautage » / « Taille du trou ».
    pub top_size: [f32; 2],
    pub shear: [f32; 2],
    pub adv_cut: (f32, f32),
    pub taper: [f32; 2],
    pub radius: f32,
    pub revolutions: f32,
}

fn twist_scale(path: u8) -> f32 {
    if path == LL_PCODE_PATH_LINE || path == LL_PCODE_PATH_FLEXIBLE {
        180.0
    } else {
        360.0
    }
}

pub fn ui_from_volume(v: &VolumeParams) -> ShapeUi {
    let ty = detect_type(v);
    let (cut, adv_cut) = if circular_basic(ty) {
        ((v.path.begin, v.path.end), (v.profile.begin, v.profile.end))
    } else {
        ((v.profile.begin, v.profile.end), (v.path.begin, v.path.end))
    };
    let k = twist_scale(v.path.curve_type);
    let top_size = if linear_basic(ty) {
        [1.0 - v.path.scale[0], 1.0 - v.path.scale[1]]
    } else {
        v.path.scale
    };
    ShapeUi {
        ty,
        hole: v.profile.curve_type & LL_PCODE_HOLE_MASK,
        cut,
        hollow: v.profile.hollow * 100.0,
        skew: v.path.skew,
        twist: (v.path.twist_begin * k, v.path.twist_end * k),
        top_size,
        shear: v.path.shear,
        adv_cut,
        taper: v.path.taper,
        radius: v.path.radius_offset,
        revolutions: v.path.revolutions,
    }
}

/// LLPanelObject::getVolumeParams: the panel's values as volume
/// parameters. `prev_ty`: the type the values were shown for (the top size
/// is converted with it), `flexible`: a line path stays flexible.
pub fn volume_from_ui(u: &ShapeUi, prev_ty: usize, flexible: bool) -> VolumeParams {
    let mut v = VolumeParams::default();
    let (_, mut path, profile) = TYPES[u.ty.min(TYPES.len() - 1)];
    if path == LL_PCODE_PATH_LINE && flexible {
        path = LL_PCODE_PATH_FLEXIBLE;
    }
    v.profile.curve_type = profile | u.hole;
    v.path.curve_type = path;
    if u.ty == SCULPTED {
        // forced values of a sculpted prim
        v.profile.begin = 0.0;
        v.profile.end = 1.0;
        v.path.begin = 0.0;
        v.path.end = 1.0;
        v.path.scale = [1.0, 0.5];
        v.path.revolutions = 1.0;
        v.constrain();
        return v;
    }
    // cut begin is at least OBJECT_MIN_CUT_INC below the end
    let fix = |(b, e): (f32, f32)| (b.min(e - 0.02).max(0.0), e);
    let (cut, adv) = (fix(u.cut), fix(u.adv_cut));
    if circular_basic(u.ty) {
        (v.path.begin, v.path.end) = cut;
        (v.profile.begin, v.profile.end) = adv;
    } else {
        (v.profile.begin, v.profile.end) = cut;
        (v.path.begin, v.path.end) = adv;
    }
    v.profile.hollow = (u.hollow / 100.0).clamp(0.0, 0.99);
    let k = twist_scale(path);
    v.path.twist_begin = u.twist.0 / k;
    v.path.twist_end = u.twist.1 / k;
    v.path.scale = if linear_basic(prev_ty) {
        [1.0 - u.top_size[0], 1.0 - u.top_size[1]]
    } else {
        u.top_size
    };
    v.path.skew = u.skew;
    v.path.taper = u.taper;
    v.path.radius_offset = u.radius;
    v.path.revolutions = u.revolutions;
    v.path.shear = u.shear;
    v.constrain();
    v
}

/// Small isometric drawing of a shape's outline (Create palette).
pub fn shape_preview(ui: &egui::Ui, rect: egui::Rect, params: &VolumeParams, rot: Quat, color: Color32) {
    let edges = crate::build::silhouette::preview_edges(params);
    let view = Quat::from_rotation_x(-0.6) * Quat::from_rotation_z(0.75) * rot;
    let s = rect.width().min(rect.height()) * 0.62;
    let proj = |v: Vec3| {
        let q = view * v;
        egui::pos2(rect.center().x + q.x * s, rect.center().y - q.z * s)
    };
    for (a, b) in edges.iter() {
        ui.painter().line_segment([proj(*a), proj(*b)], egui::Stroke::new(1.0, color));
    }
}

/// "<x, y, z>" with the decimal precision (LLPanelObject::onCopyPos...).
pub fn vector_text(v: Vec3, decimals: usize) -> String {
    format!("<{:.*}, {:.*}, {:.*}>", decimals, v.x, decimals, v.y, decimals, v.z)
}

/// sscanf("<%f, %f, %f>"): exactly three numbers.
pub fn parse_vector(t: &str) -> Option<Vec3> {
    let t = t.trim().trim_start_matches('<').trim_end_matches('>');
    let parts: Vec<f32> = t.split(',').map(|x| x.trim().parse::<f32>()).collect::<Result<_, _>>().ok()?;
    (parts.len() == 3).then(|| Vec3::new(parts[0], parts[1], parts[2]))
}

/// The three numbers of a vector with their C / P / p buttons.
#[allow(clippy::too_many_arguments)]
fn vec_block(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    which: usize,
    title: &str,
    v: &mut [f32; 3],
    increment: f32,
    range: [std::ops::RangeInclusive<f32>; 3],
    decimals: usize,
    enabled: bool,
) -> (Edit, Option<Vec3>, bool) {
    let colors = [
        Color32::from_rgb(240, 110, 110),
        Color32::from_rgb(110, 220, 110),
        Color32::from_rgb(120, 150, 245),
    ];
    let names = ["la position", "la taille", "la rotation"];
    let mut e = Edit::default();
    let mut paste = None;
    let mut copy = false;
    label(ui, p, title);
    ui.add_enabled_ui(enabled, |ui| {
        for (i, x) in v.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.label(RichText::new(["X", "Y", "Z"][i]).size(12.0).strong().color(colors[i]));
                e = e.or(spin(ui, x, increment, range[i].clone(), decimals, 92.0, false));
                let n = names[which];
                match i {
                    0 => {
                        if ui.small_button("C").on_hover_text(format!("Copie {n}")).clicked() {
                            copy = true;
                        }
                    }
                    1 => {
                        let has = tool.ui.vec_clipboard[which].is_some();
                        if ui
                            .add_enabled(has, egui::Button::new("P").small())
                            .on_hover_text(format!("Colle {n}"))
                            .clicked()
                        {
                            paste = tool.ui.vec_clipboard[which];
                        }
                    }
                    _ => {
                        if ui
                            .small_button("p")
                            .on_hover_text(format!("Essaie de coller {n} depuis le presse-papiers"))
                            .clicked()
                        {
                            tool.ui.requests.push(Request::PasteVector(which));
                        }
                    }
                }
            });
        }
    });
    (e, paste, copy)
}

pub fn show(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, s: &mut BuildSettings, env: &mut Env) {
    let Some(key) = tool.selection.first().copied() else { return };
    let Some(idx) = world.objects.index_of(&key) else { return };
    let now = Instant::now();
    let Some((wp, wr, hud)) = Scene::object_transform(world, idx, now, 0) else {
        return;
    };
    let Some(off) = world.region_offset(key.region) else { return };
    let Some(o) = world.objects.get(idx) else { return };
    let (local_pos, local_rot) = (o.position, o.rotation);
    let (flags, scale, volume, is_root, full_id) = (o.update_flags, o.scale, o.volume, o.parent_id == 0, o.full_id);
    let flexible = o.extra.flexible.is_some();
    let props = tool.props.get(&full_id).cloned();
    let editable = tool.can_modify(world, idx);
    let self_owned = props.as_ref().is_some_and(|pr| pr.owner_id == world.agent_id);
    let single = tool.sel_prims(world).len() == 1 || tool.roots(world).len() == 1;
    let decimals = s.decimal_precision.min(7) as usize;

    // ---- lock, physics, temporary, phantom; copy / paste of the shape
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 1.0;
            // lock = owner's MOVE + MODIFY cleared (LLPanelObject::onCommitLock)
            let roots = tool.roots(world);
            let (locked, locked_mixed) = super::common::same(roots.iter().map(|&r| {
                world
                    .objects
                    .get(r)
                    .and_then(|o| tool.props.get(&o.full_id))
                    .is_some_and(|pr| pr.owner_mask & perm::MOVE == 0)
            }));
            if let Some(v) = check(ui, locked.unwrap_or(false), locked_mixed, "Verrouillé", self_owned) {
                tool.set_perm(world, perm_field::OWNER, !v, perm::MOVE | perm::MODIFY);
            }
            let root_ok = editable && is_root;
            if let Some(v) = check(ui, flags & FLAGS_USE_PHYSICS != 0, false, "Physique", root_ok && !flexible) {
                tool.set_flags(world, Some(v), None, None);
            }
            if let Some(v) = check(ui, flags & FLAGS_TEMPORARY_ON_REZ != 0, false, "Temporaire", root_ok) {
                tool.set_flags(world, None, Some(v), None);
            }
            if let Some(v) = check(ui, flags & FLAGS_PHANTOM != 0, false, "Fantôme", root_ok && !flexible) {
                tool.set_flags(world, None, None, Some(v));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            let mesh = volume.is_mesh();
            if super::common::icon_button(
                ui,
                p,
                "clipboard-text",
                "Colle les paramètres de l'objet à partir du presse-papiers",
                editable && single && !mesh && tool.ui.shape_clipboard.is_some(),
            ) && let Some(c) = tool.ui.shape_clipboard
            {
                paste_params(tool, world, idx, c);
            }
            if super::common::icon_button(
                ui,
                p,
                "copy",
                "Copie les paramètres de l'objet dans le presse-papiers",
                editable && single && !mesh,
            ) {
                tool.ui.shape_clipboard = Some(ShapeClip {
                    volume,
                    sculpt: volume.sculpt,
                });
            }
        });
    });
    ui.add_space(4.0);

    // ---- position, size, rotation | shape
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        let can_move = props.as_ref().is_none_or(|pr| pr.owner_mask & perm::MOVE != 0) && (hud || editable);
        let mut pos = if hud { local_pos } else { wp - off }.to_array();
        let region = world
            .regions
            .get(&key.region)
            .map(|r| (r.heightmap.size_x as f32, r.heightmap.size_y as f32))
            .unwrap_or((256.0, 256.0));
        let (e, paste, copy) = vec_block(
            ui,
            p,
            tool,
            0,
            "Position (mètres)",
            &mut pos,
            0.01,
            if hud {
                [-64.0..=64.0, -64.0..=64.0, -64.0..=64.0]
            } else {
                [0.0..=region.0, 0.0..=region.1, 0.0..=4096.0]
            },
            decimals,
            can_move,
        );
        if copy {
            copy_vector(ui, tool, 0, Vec3::from_array(pos), decimals);
        }
        if let Some(v) = paste {
            let v = if hud {
                v.clamp(Vec3::splat(-64.0), Vec3::splat(64.0))
            } else {
                Vec3::new(v.x.clamp(0.0, region.0), v.y.clamp(0.0, region.1), v.z)
            };
            tool.set_transform(world, s, Some(v), None, None, true);
        } else if e.changed {
            tool.set_transform(world, s, Some(Vec3::from_array(pos)), None, None, e.commit);
        }
        ui.add_space(4.0);
        let mut size = scale.to_array();
        let (e, paste, copy) = vec_block(
            ui,
            p,
            tool,
            1,
            "Taille (mètres)",
            &mut size,
            0.01,
            [
                MIN_PRIM_SCALE..=MAX_PRIM_SCALE,
                MIN_PRIM_SCALE..=MAX_PRIM_SCALE,
                MIN_PRIM_SCALE..=MAX_PRIM_SCALE,
            ],
            decimals,
            editable,
        );
        if copy {
            copy_vector(ui, tool, 1, Vec3::from_array(size), decimals);
        }
        if let Some(v) = paste {
            tool.set_transform(world, s, None, Some(v), None, true);
        } else if e.changed {
            tool.set_transform(world, s, None, Some(Vec3::from_array(size)), None, e.commit);
        }
        ui.add_space(4.0);
        // Euler degrees rounded to 0.05, wrapped into 0..360
        let (z, y, x) = if hud { local_rot } else { wr }.to_euler(EulerRot::ZYX);
        let wrap = |a: f32| {
            let d = (a.to_degrees() / 0.05).round() * 0.05;
            if d < 0.0 { d + 360.0 } else { d }
        };
        let mut rot = [wrap(x), wrap(y), wrap(z)];
        let (e, paste, copy) = vec_block(
            ui,
            p,
            tool,
            2,
            "Rotation (degrés)",
            &mut rot,
            1.0,
            [-9999.0..=9999.0, -9999.0..=9999.0, -9999.0..=9999.0],
            decimals,
            can_move,
        );
        if copy {
            copy_vector(ui, tool, 2, Vec3::from_array(rot), decimals);
        }
        let to_q = |r: [f32; 3]| Quat::from_euler(EulerRot::ZYX, r[2].to_radians(), r[1].to_radians(), r[0].to_radians());
        if let Some(v) = paste {
            tool.set_transform(world, s, None, None, Some(to_q(v.to_array())), true);
        } else if e.changed {
            tool.set_transform(world, s, None, None, Some(to_q(rot)), e.commit);
        }

        let ui = &mut cols[1];
        shape_column(ui, p, tool, world, idx, &volume, editable && single, flexible, env);
    });
}

/// « C »: the vector in our clipboard and in the system's as "<x, y, z>".
fn copy_vector(ui: &egui::Ui, tool: &mut BuildTool, which: usize, v: Vec3, decimals: usize) {
    tool.ui.vec_clipboard[which] = Some(v);
    ui.ctx().copy_text(vector_text(v, decimals));
}

/// « Coller » of the shape parameters (LLPanelObject::onPasteParams).
fn paste_params(tool: &mut BuildTool, world: &mut World, idx: usize, c: ShapeClip) {
    let current = world.objects.get(idx).map(|o| o.extra.sculpt);
    match (c.sculpt, current) {
        (Some(sc), _) => tool.set_extra(world, idx, crate::build::edits::Extra::Sculpt(Some(sc))),
        (None, Some(Some(_))) => tool.set_extra(world, idx, crate::build::edits::Extra::Sculpt(None)),
        _ => {}
    }
    let flexible = world.objects.get(idx).is_some_and(|o| o.extra.flexible.is_some());
    let mut v = c.volume;
    v.sculpt = c.sculpt;
    // a line path follows the object's flexible state
    if flexible && v.path.curve_type == LL_PCODE_PATH_LINE {
        v.path.curve_type = LL_PCODE_PATH_FLEXIBLE;
    } else if !flexible && v.path.curve_type == LL_PCODE_PATH_FLEXIBLE {
        v.path.curve_type = LL_PCODE_PATH_LINE;
    }
    tool.set_shape(world, idx, v);
    tool.costs_stale();
}

#[allow(clippy::too_many_arguments)]
fn shape_column(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    idx: usize,
    volume: &VolumeParams,
    enabled: bool,
    flexible: bool,
    env: &mut Env,
) {
    if volume.is_mesh() {
        section(ui, p, "Mesh");
        small(ui, p, "La forme d'un mesh ne se modifie pas ici.");
        if let Some(n) = tool.face_counts.get(&world.objects.get(idx).map(|o| o.full_id).unwrap_or_default()) {
            small(ui, p, &format!("{n} face(s)"));
        }
        return;
    }
    let ui_now = ui_from_volume(volume);
    let mut u = ui_now;
    let mut e = Edit::default();
    let items: Vec<(&str, usize)> = TYPES.iter().enumerate().map(|(i, t)| (t.0, i)).collect();
    if let Some(t) = combo(ui, "prim_type", Some(u.ty), &items, 128.0, enabled) {
        u.ty = t;
        e = Edit {
            changed: true,
            commit: true,
        };
    }
    let ty = u.ty;
    if ty == SCULPTED {
        sculpt_section(ui, p, tool, world, idx, enabled, env);
    } else {
        let w = 58.0;
        let pair = |ui: &mut egui::Ui,
                    title: &str,
                    a: &mut f32,
                    b: &mut f32,
                    inc: f32,
                    ra: std::ops::RangeInclusive<f32>,
                    rb: std::ops::RangeInclusive<f32>,
                    d: usize| {
            label(ui, p, title);
            let mut e = Edit::default();
            ui.horizontal(|ui| {
                ui.label(RichText::new("D").size(11.0));
                e = e.or(spin(ui, a, inc, ra, d, w, false));
                ui.label(RichText::new("F").size(11.0));
                e = e.or(spin(ui, b, inc, rb, d, w, false));
            });
            e
        };
        ui.add_enabled_ui(enabled, |ui| {
            e = e.or(pair(
                ui,
                "Découpe du tracé (déb./fin)",
                &mut u.cut.0,
                &mut u.cut.1,
                0.025,
                0.0..=0.98,
                0.02..=1.0,
                3,
            ));
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    label(ui, p, "Creux");
                    e = e.or(spin(ui, &mut u.hollow, 5.0, 0.0..=95.0, 1, w, false));
                });
                ui.vertical(|ui| {
                    label(ui, p, "Biais");
                    e = e.or(spin(ui, &mut u.skew, 0.05, -0.95..=0.95, 2, w, false));
                });
            });
            label(ui, p, "Forme du creux");
            if let Some(h) = combo(ui, "hole_type", Some(u.hole), &HOLES, 120.0, u.hollow > 0.0) {
                u.hole = h;
                e = e.or(Edit {
                    changed: true,
                    commit: true,
                });
            }
            let (tw, ti) = if circular_basic(ty) { (360.0, 18.0) } else { (180.0, 9.0) };
            e = e.or(pair(
                ui,
                "Vrille (début/fin)",
                &mut u.twist.0,
                &mut u.twist.1,
                ti,
                -tw..=tw,
                -tw..=tw,
                0,
            ));
            let hole_label = matches!(ty, 4..=6);
            let (rx, ry) = match ty {
                3 => (0.0..=1.0, 0.0..=1.0),
                4..=6 => (0.05..=1.0, 0.05..=0.5),
                0..=2 => (-1.0..=1.0, -1.0..=1.0),
                _ => (0.0..=2.0, 0.0..=2.0),
            };
            label(ui, p, if hole_label { "Taille du trou" } else { "Biseautage" });
            ui.horizontal(|ui| {
                ui.label(RichText::new("X").size(11.0));
                e = e.or(spin(ui, &mut u.top_size[0], 0.05, rx, 3, w, false));
                ui.label(RichText::new("Y").size(11.0));
                e = e.or(spin(ui, &mut u.top_size[1], 0.05, ry, 3, w, false));
            });
            label(ui, p, "Inclinaison");
            ui.horizontal(|ui| {
                ui.label(RichText::new("X").size(11.0));
                e = e.or(spin(ui, &mut u.shear[0], 0.05, -0.5..=0.5, 3, w, false));
                ui.label(RichText::new("Y").size(11.0));
                e = e.or(spin(ui, &mut u.shear[1], 0.05, -0.5..=0.5, 3, w, false));
            });
            let adv = match ty {
                3 => "Creux (début/fin)",
                0..=2 => "Tranche (début/fin)",
                _ => "Découpe du profilé (début/fin)",
            };
            e = e.or(pair(ui, adv, &mut u.adv_cut.0, &mut u.adv_cut.1, 0.02, 0.0..=0.98, 0.02..=1.0, 3));
            // « Biseautage » of the path (taper), hidden for the box
            if ty != 0 {
                label(ui, p, "Biseautage");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("X").size(11.0));
                    e = e.or(spin(ui, &mut u.taper[0], 0.05, -1.0..=1.0, 3, w, false));
                    ui.label(RichText::new("Y").size(11.0));
                    e = e.or(spin(ui, &mut u.taper[1], 0.05, -1.0..=1.0, 3, w, false));
                });
            }
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    label(ui, p, "Rayon");
                    e = e.or(spin(ui, &mut u.radius, 0.05, -1.0..=1.0, 3, w, false));
                });
                ui.vertical(|ui| {
                    label(ui, p, "Révolutions");
                    e = e.or(spin(ui, &mut u.revolutions, 0.1, 1.0..=4.0, 2, w, false));
                });
            });
        });
    }
    if e.changed && u != ui_now {
        let v = volume_from_ui(&u, ui_now.ty, flexible);
        // Sculptie: the sculpt parameter goes on (LLPanelObject::onCommitParametric)
        let was_sculpt = volume.sculpt.is_some();
        if u.ty == SCULPTED && !was_sculpt {
            tool.set_extra(
                world,
                idx,
                crate::build::edits::Extra::Sculpt(Some(crate::build::edits::SCULPT_DEFAULT)),
            );
        } else if u.ty != SCULPTED && was_sculpt {
            tool.set_extra(world, idx, crate::build::edits::Extra::Sculpt(None));
        }
        if e.commit {
            tool.set_shape(world, idx, v);
            tool.costs_stale();
        } else if let Some(o) = world.objects.get_mut(idx) {
            let mut v = v;
            v.sculpt = o.volume.sculpt;
            o.volume = v;
            o.shape_dirty = true;
        }
    }
}

/// Sculpt texture, stitching, mirror, inside out (LLPanelObject::sendSculpt).
fn sculpt_section(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, idx: usize, enabled: bool, env: &mut Env) {
    let Some(sc) = world.objects.get(idx).and_then(|o| o.extra.sculpt) else {
        return;
    };
    label(ui, p, "Texture de la sculptie");
    let r = super::common::swatch(
        ui,
        p,
        env.images,
        &mut tool.ui.wanted_images,
        Some(sc.texture),
        "",
        64.0,
        false,
        enabled,
    );
    if r.clicked() {
        tool.ui.picker.open("Choisir : texture de la sculptie", sc.texture);
        tool.ui.pick_target = Some(super::PickTarget::Sculpt);
    }
    let base = sc.sculpt_type & LL_SCULPT_TYPE_MASK;
    let shown = if base == 0 { 3 } else { base };
    let mut new = sc;
    label(ui, p, "Type de raccord");
    if let Some(t) = combo(ui, "sculpt_type", Some(shown), &SCULPT_TYPES, 110.0, enabled) {
        new.sculpt_type = (sc.sculpt_type & !LL_SCULPT_TYPE_MASK) | t;
    }
    if let Some(v) = check(ui, sc.sculpt_type & LL_SCULPT_FLAG_MIRROR != 0, false, "Mirroir", enabled) {
        new.sculpt_type = if v {
            new.sculpt_type | LL_SCULPT_FLAG_MIRROR
        } else {
            new.sculpt_type & !LL_SCULPT_FLAG_MIRROR
        };
    }
    if let Some(v) = check(ui, sc.sculpt_type & LL_SCULPT_FLAG_INVERT != 0, false, "A l'envers", enabled) {
        new.sculpt_type = if v {
            new.sculpt_type | LL_SCULPT_FLAG_INVERT
        } else {
            new.sculpt_type & !LL_SCULPT_FLAG_INVERT
        };
    }
    if new != sc {
        tool.set_extra(world, idx, crate::build::edits::Extra::Sculpt(Some(new)));
    }
}

impl BuildTool {
    /// « p »: a vector pasted from the system clipboard (`which`: position,
    /// size, rotation), clamped like « P ».
    pub fn paste_vector(&mut self, world: &mut World, s: &BuildSettings, which: usize, text: &str) {
        let Some(v) = parse_vector(text) else {
            self.status = "Le presse-papiers ne contient pas de vecteur <x, y, z>.".into();
            return;
        };
        match which {
            0 => {
                let Some(key) = self.selection.first() else { return };
                let (w, h) = world
                    .regions
                    .get(&key.region)
                    .map(|r| (r.heightmap.size_x as f32, r.heightmap.size_y as f32))
                    .unwrap_or((256.0, 256.0));
                self.set_transform(
                    world,
                    s,
                    Some(Vec3::new(v.x.clamp(0.0, w), v.y.clamp(0.0, h), v.z)),
                    None,
                    None,
                    true,
                );
            }
            1 => self.set_transform(world, s, None, Some(v), None, true),
            _ => {
                let q = Quat::from_euler(EulerRot::ZYX, v.z.to_radians(), v.y.to_radians(), v.x.to_radians());
                self.set_transform(world, s, None, None, Some(q), true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn box_params() -> VolumeParams {
        RawShape {
            path_curve: LL_PCODE_PATH_LINE,
            profile_curve: LL_PCODE_PROFILE_SQUARE,
            path_scale_x: 100,
            path_scale_y: 100,
            ..Default::default()
        }
        .to_params()
    }

    #[test]
    fn box_round_trips() {
        let v = box_params();
        let u = ui_from_volume(&v);
        assert_eq!(u.ty, 0);
        assert!((u.top_size[0]).abs() < 1e-5);
        let back = volume_from_ui(&u, u.ty, false);
        assert_eq!(back.to_raw(), v.to_raw());
    }

    #[test]
    fn box_to_sphere_swaps_cuts() {
        let mut u = ui_from_volume(&box_params());
        u.cut = (0.25, 0.75);
        u.ty = 3;
        let v = volume_from_ui(&u, 0, false);
        assert_eq!(v.path.curve_type, LL_PCODE_PATH_CIRCLE);
        assert!((v.path.begin - 0.25).abs() < 1e-5);
        assert!((v.profile.begin).abs() < 1e-5);
        assert_eq!(detect_type(&v), 3);
    }

    #[test]
    fn extra_types_detected() {
        let mut v = box_params();
        v.path.curve_type = LL_PCODE_PATH_CIRCLE_33;
        v.profile.curve_type = LL_PCODE_PROFILE_ISOTRI;
        assert_eq!(detect_type(&v), 20);
        v.path.curve_type = LL_PCODE_PATH_TEST;
        v.profile.curve_type = LL_PCODE_PROFILE_CIRCLE_HALF;
        assert_eq!(detect_type(&v), 17);
    }

    #[test]
    fn vectors_parse() {
        assert_eq!(parse_vector("<1.5, 2, -3>"), Some(Vec3::new(1.5, 2.0, -3.0)));
        assert_eq!(parse_vector("1, 2"), None);
        assert_eq!(vector_text(Vec3::new(1.0, 2.0, 3.0), 2), "<1.00, 2.00, 3.00>");
    }
}
