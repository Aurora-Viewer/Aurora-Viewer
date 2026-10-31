//! Object tab (LLPanelObject, indra/newview/llpanelobject.cpp, Firestorm,
//! originally LGPL 2.1): lock / physical / temporary / phantom, position,
//! size and rotation with copy / paste actions, the prim shape (22
//! types, Firestorm's "Working33" extra paths included) and sculpties.

use super::common::{Edit, check, combo, label, row, spin};
use super::{Env, Request, ShapeClip};
use crate::build::{BuildSettings, BuildTool, EditMode, MAX_PRIM_SCALE, MIN_PRIM_SCALE};
use crate::scene::Scene;
use crate::theme::Palette;
use crate::ui::menu;
use crate::world::World;
use aurora_net::build::{perm, perm_field};
use aurora_prim::params::*;
use egui::{Color32, RichText, Vec2};
use glam::{EulerRot, Quat, Vec3};
use std::time::Instant;

/// FLAGS_USE_PHYSICS / TEMPORARY_ON_REZ / PHANTOM (llprimitive.h).
use crate::build::edits::{FLAGS_PHANTOM, FLAGS_TEMPORARY_ON_REZ, FLAGS_USE_PHYSICS};

/// comboBaseType: (label, path, profile). Index 7 is "Sculptie".
pub fn types() -> [(&'static str, u8, u8); 22] {
    [
        (aurora_i18n::tr!("common-box"), LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE),
        (aurora_i18n::tr!("common-cylinder"), LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE),
        (
            aurora_i18n::tr!("build-ui-object-prism"),
            LL_PCODE_PATH_LINE,
            LL_PCODE_PROFILE_EQUALTRI,
        ),
        (
            aurora_i18n::tr!("common-sphere"),
            LL_PCODE_PATH_CIRCLE,
            LL_PCODE_PROFILE_CIRCLE_HALF,
        ),
        (aurora_i18n::tr!("common-torus"), LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE),
        (aurora_i18n::tr!("common-tube"), LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_SQUARE),
        (aurora_i18n::tr!("common-ring"), LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_EQUALTRI),
        (
            aurora_i18n::tr!("build-ui-object-sculptie"),
            LL_PCODE_PATH_CIRCLE,
            LL_PCODE_PROFILE_CIRCLE,
        ),
        (
            aurora_i18n::tr!("build-ui-object-line-half-circle"),
            LL_PCODE_PATH_LINE,
            LL_PCODE_PROFILE_CIRCLE_HALF,
        ),
        (
            aurora_i18n::tr!("build-ui-object-circle-half-circle"),
            LL_PCODE_PATH_CIRCLE,
            LL_PCODE_PROFILE_CIRCLE_HALF,
        ),
        (
            aurora_i18n::tr!("build-ui-object-circle2-square"),
            LL_PCODE_PATH_CIRCLE2,
            LL_PCODE_PROFILE_SQUARE,
        ),
        (
            aurora_i18n::tr!("build-ui-object-circle2-triangle"),
            LL_PCODE_PATH_CIRCLE2,
            LL_PCODE_PROFILE_EQUALTRI,
        ),
        (
            aurora_i18n::tr!("build-ui-object-circle2-circle"),
            LL_PCODE_PATH_CIRCLE2,
            LL_PCODE_PROFILE_CIRCLE,
        ),
        (
            aurora_i18n::tr!("build-ui-object-circle2-half-circle"),
            LL_PCODE_PATH_CIRCLE2,
            LL_PCODE_PROFILE_CIRCLE_HALF,
        ),
        (aurora_i18n::tr!("extra-test-square"), LL_PCODE_PATH_TEST, LL_PCODE_PROFILE_SQUARE),
        (
            aurora_i18n::tr!("extra-test-triangle"),
            LL_PCODE_PATH_TEST,
            LL_PCODE_PROFILE_EQUALTRI,
        ),
        (aurora_i18n::tr!("extra-test-circle"), LL_PCODE_PATH_TEST, LL_PCODE_PROFILE_CIRCLE),
        (
            aurora_i18n::tr!("extra-test-half-circle"),
            LL_PCODE_PATH_TEST,
            LL_PCODE_PROFILE_CIRCLE_HALF,
        ),
        ("33->Circle", LL_PCODE_PATH_CIRCLE_33, LL_PCODE_PROFILE_CIRCLE),
        ("33->Square", LL_PCODE_PATH_CIRCLE_33, LL_PCODE_PROFILE_SQUARE),
        ("33->Triangle", LL_PCODE_PATH_CIRCLE_33, LL_PCODE_PROFILE_ISOTRI),
        ("33->HalfCircle", LL_PCODE_PATH_CIRCLE_33, LL_PCODE_PROFILE_CIRCLE_HALF),
    ]
}
const SCULPTED: usize = 7;

/// "Hollow shape" (hole type bits).
fn holes() -> [(&'static str, u8); 4] {
    [
        (aurora_i18n::tr!("common-default"), LL_PCODE_HOLE_SAME),
        (aurora_i18n::tr!("build-ui-object-circle"), LL_PCODE_HOLE_CIRCLE),
        (aurora_i18n::tr!("build-ui-object-square"), LL_PCODE_HOLE_SQUARE),
        (aurora_i18n::tr!("build-ui-object-triangle"), LL_PCODE_HOLE_TRIANGLE),
    ]
}

/// "Join type" (sculpt stitching): Sphere, Torus, Plane, Cylinder.
fn sculpt_types() -> [(&'static str, u8); 4] {
    [
        (aurora_i18n::tr!("common-sphere"), 1),
        (aurora_i18n::tr!("common-torus"), 2),
        (aurora_i18n::tr!("build-ui-object-plane"), 3),
        (aurora_i18n::tr!("common-cylinder"), 4),
    ]
}

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
    types()
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
    /// "Bevel" / "Hole size".
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
    let (_, mut path, profile) = types()[u.ty.min(types().len() - 1)];
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

/// Icon of the paste entries, the one of the paste buttons.
const PASTE: &str = "clipboard-text-filled";

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
    mixed: [bool; 3],
) -> (Edit, Option<Vec3>, bool) {
    if super::legacy::active(ui) {
        return legacy_vec_block(ui, p, tool, which, title, v, increment, range, decimals, enabled, mixed);
    }
    let mut e = Edit::default();
    let mut paste = None;
    let mut copy = false;
    let spacing = ui.spacing().item_spacing.y;
    ui.spacing_mut().item_spacing.y = 2.0;
    let vector_name = [
        aurora_i18n::tr!("build-compact-position"),
        aurora_i18n::tr!("build-compact-size"),
        aurora_i18n::tr!("build-compact-rotation"),
    ][which];
    // LLPanelObject::onCopyPos/Size/Rot keeps a separate value per vector.
    // Keep each pair on its own caption line, independent of the manipulator.
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(super::common::BODY).color(p.muted));
        super::compact::actions(ui, |ui| {
            if super::compact::icon_button(
                ui,
                p,
                PASTE,
                &aurora_i18n::tr!("build-ui-object-paste", n = vector_name),
                enabled && tool.ui.vec_clipboard[which].is_some(),
            )
            .clicked()
            {
                paste = tool.ui.vec_clipboard[which];
            }
            if super::compact::icon_button(
                ui,
                p,
                "copy-filled",
                &aurora_i18n::tr!("build-ui-object-copy", n = vector_name),
                enabled,
            )
            .clicked()
            {
                copy = true;
            }
        });
    });
    // Three cells cut out of one row: equal widths from edge to edge and one
    // baseline, whatever the length of the values.
    const GAP: f32 = 6.0;
    const HEIGHT: f32 = 28.0;
    const MARK: f32 = 2.0;
    const LETTER: f32 = super::common::SMALL;
    let row = ui.add_enabled_ui(enabled, |ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), HEIGHT), egui::Sense::hover());
        let width = ((rect.width() - 2.0 * GAP) / 3.0).max(40.0);
        let radius = ui.visuals().window_corner_radius;
        for (axis, value) in v.iter_mut().enumerate() {
            let left = rect.left() + axis as f32 * (width + GAP);
            let cell = egui::Rect::from_min_size(egui::pos2(left, rect.top()), egui::vec2(width, HEIGHT));
            let color = crate::build::draw::axis_color(p, axis);
            ui.painter()
                .rect(cell, radius, p.field, egui::Stroke::new(1.0, p.raised), egui::StrokeKind::Inside);
            // axis mark on the left edge, in the color of the manipulator's axis
            ui.painter().rect_filled(
                egui::Rect::from_min_size(cell.min, egui::vec2(MARK, HEIGHT)),
                egui::CornerRadius {
                    nw: radius.nw,
                    sw: radius.sw,
                    ne: 0,
                    se: 0,
                },
                color,
            );
            let inner = egui::Rect::from_min_max(
                egui::pos2(cell.left() + MARK + 5.0, cell.top() + 1.0),
                egui::pos2(cell.right() - 2.0, cell.bottom() - 1.0),
            );
            ui.scope_builder(
                egui::UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    ui.spacing_mut().interact_size.y = 23.0;
                    ui.spacing_mut().button_padding.y = 3.0;
                    // the value sits in the cell itself: no second face under it
                    let widgets = &mut ui.visuals_mut().widgets;
                    widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
                    widgets.inactive.bg_stroke = egui::Stroke::NONE;
                    widgets.hovered.bg_stroke = egui::Stroke::NONE;
                    widgets.active.bg_stroke = egui::Stroke::NONE;
                    // The letter is smaller than the value: the row would
                    // level their tops. Lowered by half the difference of
                    // their capital heights (about 0.7 em), it sits on the
                    // middle of the digits.
                    let value_size = egui::TextStyle::Button.resolve(ui.style()).size;
                    let galley = ui
                        .painter()
                        .layout_no_wrap(["X", "Y", "Z"][axis].to_owned(), egui::FontId::proportional(LETTER), color);
                    let (letter, _) = ui.allocate_exact_size(galley.size(), egui::Sense::hover());
                    let drop = 0.35 * (value_size - LETTER).max(0.0);
                    ui.painter().galley(letter.min + egui::vec2(0.0, drop), galley, color);
                    e = e.or(super::common::spin_with_min_decimals(
                        ui,
                        value,
                        increment,
                        range[axis].clone(),
                        decimals,
                        (inner.right() - letter.right() - 2.0).max(20.0),
                        mixed[axis],
                        if which == 2 { 1 } else { 3 },
                    ));
                },
            );
        }
    });
    // The row menu also accepts vectors from the system clipboard.
    menu::context_menu(&row.response, p, |ui| {
        if menu::item_if(ui, p, "copy-filled", aurora_i18n::tr!("build-compact-copy-vector"), enabled) {
            copy = true;
        }
        let stored = enabled && tool.ui.vec_clipboard[which].is_some();
        if menu::item_if(ui, p, PASTE, aurora_i18n::tr!("build-compact-paste-vector"), stored) {
            paste = tool.ui.vec_clipboard[which];
        }
        if menu::item_if(ui, p, PASTE, aurora_i18n::tr!("build-compact-paste-system"), enabled) {
            tool.ui.requests.push(Request::PasteVector(which));
        }
    });
    ui.add_space(3.0);
    ui.spacing_mut().item_spacing.y = spacing;
    (e, paste, copy)
}

#[allow(clippy::too_many_arguments)]
fn legacy_vec_block(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    which: usize,
    title: &str,
    values: &mut [f32; 3],
    increment: f32,
    ranges: [std::ops::RangeInclusive<f32>; 3],
    decimals: usize,
    enabled: bool,
    mixed: [bool; 3],
) -> (Edit, Option<Vec3>, bool) {
    let mut edit = Edit::default();
    let mut paste = None;
    let mut copy = false;
    let caption = match which {
        0 => aurora_i18n::tr!("build-ui-object-position-meters"),
        1 => aurora_i18n::tr!("build-ui-object-size-meters"),
        _ => aurora_i18n::tr!("build-ui-object-rotation-degrees"),
    };
    super::common::label(ui, p, caption);
    ui.add_enabled_ui(enabled, |ui| {
        for (axis, (value, range)) in values.iter_mut().zip(ranges).enumerate() {
            let row = ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.add_sized(
                    [11.0, 20.0],
                    egui::Label::new(
                        RichText::new(["X", "Y", "Z"][axis])
                            .size(11.0)
                            .color(crate::build::draw::axis_color(p, axis)),
                    ),
                );
                let width = (ui.available_width() - 23.0).max(45.0);
                edit = edit.or(super::common::spin_with_min_decimals(
                    ui,
                    value,
                    increment,
                    range,
                    decimals,
                    width,
                    mixed[axis],
                    if which == 2 { 1 } else { 3 },
                ));
                let (label, hint, active) = match axis {
                    0 => (
                        aurora_i18n::tr!("build-legacy-vector-copy"),
                        aurora_i18n::tr!("build-ui-object-copy", n = title),
                        true,
                    ),
                    1 => (
                        aurora_i18n::tr!("build-legacy-vector-paste"),
                        aurora_i18n::tr!("build-ui-object-paste", n = title),
                        tool.ui.vec_clipboard[which].is_some(),
                    ),
                    _ => (
                        aurora_i18n::tr!("build-legacy-vector-system"),
                        aurora_i18n::tr!("build-compact-paste-system").to_owned(),
                        true,
                    ),
                };
                if ui
                    .add_enabled(active, egui::Button::new(label).min_size(egui::vec2(20.0, 20.0)))
                    .on_hover_text(hint)
                    .clicked()
                {
                    match axis {
                        0 => copy = true,
                        1 => paste = tool.ui.vec_clipboard[which],
                        _ => tool.ui.requests.push(Request::PasteVector(which)),
                    }
                }
            });
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::unique(("legacy-vector-row", which, axis)), row.response.rect));
            #[cfg(not(test))]
            let _ = row;
        }
    });
    ui.add_space(5.0);
    (edit, paste, copy)
}

pub fn show_transform(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    s: &mut BuildSettings,
    _mods: crate::build::Mods,
) {
    let Some(key) = tool.selection.first().copied() else { return };
    let Some(idx) = world.objects.index_of(&key) else { return };
    let now = Instant::now();
    let Some((wp, wr, hud)) = Scene::object_transform(world, idx, now, 0) else {
        return;
    };
    // what we wear is shown relative to its attachment point, body or HUD
    // (LLPanelObject::getState, isAttachment)
    let local = crate::build::select_type(world, idx) != crate::build::SelectType::World;
    let reach = if hud { 64.0 } else { crate::build::MAX_ATTACHMENT_DIST };
    let Some(off) = world.region_offset(key.region) else { return };
    let Some(o) = world.objects.get(idx) else { return };
    let (local_pos, local_rot) = (o.position, o.rotation);
    let scale = o.scale;
    let decimals = s.decimal_precision.min(7) as usize;

    let mixed = mixed_vectors(tool, world, s);
    let can_move = crate::build::can_transform(world, idx, s.edit_linked, EditMode::Move);
    let can_scale = crate::build::can_transform(world, idx, s.edit_linked, EditMode::Stretch);
    let (z, y, x) = if local { local_rot } else { wr }.to_euler(EulerRot::ZYX);
    let wrap = |a: f32| {
        let d = (a.to_degrees() / 0.05).round() * 0.05;
        if d == 0.0 {
            0.0
        } else if d < 0.0 {
            d + 360.0
        } else {
            d
        }
    };
    let mut rot = [wrap(x), wrap(y), wrap(z)];
    let mut pos = if local { local_pos } else { wp - off }.to_array();
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
        aurora_i18n::tr!("build-compact-position"),
        &mut pos,
        0.01,
        if local {
            [-reach..=reach, -reach..=reach, -reach..=reach]
        } else {
            [0.0..=region.0, 0.0..=region.1, 0.0..=4096.0]
        },
        decimals,
        can_move,
        mixed[0],
    );
    if copy {
        copy_vector(ui, tool, 0, Vec3::from_array(pos), decimals);
    }
    if let Some(v) = paste {
        let v = if local {
            v.clamp(Vec3::splat(-reach), Vec3::splat(reach))
        } else {
            Vec3::new(v.x.clamp(0.0, region.0), v.y.clamp(0.0, region.1), v.z)
        };
        tool.set_transform(world, s, Some(v), None, None, true);
    } else if e.changed {
        tool.set_transform(world, s, Some(Vec3::from_array(pos)), None, None, e.commit);
    }
    let mut size = scale.to_array();
    let (e, paste, copy) = vec_block(
        ui,
        p,
        tool,
        1,
        aurora_i18n::tr!("build-compact-size"),
        &mut size,
        0.01,
        [
            MIN_PRIM_SCALE..=MAX_PRIM_SCALE,
            MIN_PRIM_SCALE..=MAX_PRIM_SCALE,
            MIN_PRIM_SCALE..=MAX_PRIM_SCALE,
        ],
        decimals,
        can_scale,
        mixed[1],
    );
    if copy {
        copy_vector(ui, tool, 1, Vec3::from_array(size), decimals);
    }
    if let Some(v) = paste {
        tool.set_transform(world, s, None, Some(v), None, true);
    } else if e.changed {
        tool.set_transform(world, s, None, Some(Vec3::from_array(size)), None, e.commit);
    }
    let (e, paste, copy) = vec_block(
        ui,
        p,
        tool,
        2,
        aurora_i18n::tr!("build-compact-rotation"),
        &mut rot,
        1.0,
        [-9999.0..=9999.0, -9999.0..=9999.0, -9999.0..=9999.0],
        decimals,
        can_move,
        mixed[2],
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
}

pub fn show_flags(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World) {
    let Some(key) = tool.selection.first().copied() else { return };
    let Some(idx) = world.objects.index_of(&key) else { return };
    let Some(o) = world.objects.get(idx) else { return };
    let (flags, is_root, full_id, flexible) = (o.update_flags, o.parent_id == 0, o.full_id, o.extra.flexible.is_some());
    let editable = tool.can_modify(world, idx);
    let self_owned = tool.props.get(&full_id).is_some_and(|pr| pr.owner_id == world.agent_id);
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
    if let Some(v) = check(
        ui,
        p,
        locked.unwrap_or(false),
        locked_mixed,
        aurora_i18n::tr!("build-ui-object-locked"),
        self_owned,
    ) {
        tool.set_perm(world, perm_field::OWNER, !v, perm::MOVE | perm::MODIFY);
    }
    let root_ok = editable && is_root;
    if let Some(v) = check(
        ui,
        p,
        flags & FLAGS_USE_PHYSICS != 0,
        false,
        aurora_i18n::tr!("build-ui-object-physics"),
        root_ok && !flexible,
    ) {
        tool.set_flags(world, Some(v), None, None);
    }
    if let Some(v) = check(
        ui,
        p,
        flags & FLAGS_TEMPORARY_ON_REZ != 0,
        false,
        aurora_i18n::tr!("common-temporary"),
        root_ok,
    ) {
        tool.set_flags(world, None, Some(v), None);
    }
    if let Some(v) = check(
        ui,
        p,
        flags & FLAGS_PHANTOM != 0,
        false,
        aurora_i18n::tr!("build-ui-object-phantom"),
        root_ok && !flexible,
    ) {
        tool.set_flags(world, None, None, Some(v));
    }
}

pub fn show_shape(ui: &mut egui::Ui, p: &Palette, tool: &mut BuildTool, world: &mut World, env: &mut Env) {
    let Some(key) = tool.selection.first().copied() else { return };
    let Some(idx) = world.objects.index_of(&key) else { return };
    let Some(o) = world.objects.get(idx) else { return };
    let (volume, flexible) = (o.volume, o.extra.flexible.is_some());
    if volume.is_mesh() {
        super::mesh::show(ui, p, tool, world, idx, env);
        return;
    }
    let editable = tool.can_modify(world, idx);
    let single = tool.sel_prims(world).len() == 1 || tool.roots(world).len() == 1;
    let mut chosen_type = None;
    let legacy = super::legacy::active(ui);
    ui.horizontal(|ui| {
        let items: Vec<(&str, usize)> = types().iter().enumerate().map(|(i, t)| (t.0, i)).collect();
        if volume.is_mesh() {
            label(ui, p, aurora_i18n::tr!("build-ui-object-mesh"));
        } else {
            chosen_type = combo(
                ui,
                "prim_type",
                Some(ui_from_volume(&volume).ty),
                &items,
                if legacy { (ui.available_width() - 42.0).max(70.0) } else { 128.0 },
                editable && single,
            );
        }
        super::compact::actions(ui, |ui| {
            let mesh = volume.is_mesh();
            if super::common::icon_button(
                ui,
                p,
                "clipboard-text-filled",
                aurora_i18n::tr!("build-ui-object-paste-object-settings-from-the-clipboard"),
                editable && single && !mesh && tool.ui.shape_clipboard.is_some(),
            ) && let Some(c) = tool.ui.shape_clipboard
            {
                paste_params(tool, world, idx, c);
            }
            if super::common::icon_button(
                ui,
                p,
                "copy-filled",
                aurora_i18n::tr!("build-ui-object-copy-object-settings-to-clipboard"),
                editable && single && !mesh,
            ) {
                tool.ui.shape_clipboard = Some(ShapeClip {
                    volume,
                    sculpt: volume.sculpt,
                });
            }
        });
    });
    shape_column(ui, p, tool, world, idx, &volume, chosen_type, editable && single, flexible, env);
}

/// Show a dash only on axes whose selected objects have different values.
/// Whole linksets compare roots; linked editing compares the selected parts.
fn mixed_vectors(tool: &BuildTool, world: &World, s: &BuildSettings) -> [[bool; 3]; 3] {
    let indices = if s.edit_linked { tool.sel_prims(world) } else { tool.roots(world) };
    let mut first = None;
    let mut mixed = [[false; 3]; 3];
    let now = Instant::now();
    for idx in indices {
        let Some(o) = world.objects.get(idx) else { continue };
        let Some((pos, rot, _)) = Scene::object_transform(world, idx, now, 0) else {
            continue;
        };
        let local = crate::build::select_type(world, idx) != crate::build::SelectType::World;
        let Some(off) = world.region_offset(o.key.region) else { continue };
        let (z, y, x) = if local { o.rotation } else { rot }.to_euler(EulerRot::ZYX);
        let values = [if local { o.position } else { pos - off }.to_array(), o.scale.to_array(), [x, y, z]];
        let reference = first.get_or_insert(values);
        for (row, flags) in mixed.iter_mut().enumerate() {
            for (axis, flag) in flags.iter_mut().enumerate() {
                *flag |= (reference[row][axis] - values[row][axis]).abs() > 0.0001;
            }
        }
    }
    mixed
}

/// "C": the vector in our clipboard and in the system's as "<x, y, z>".
fn copy_vector(ui: &egui::Ui, tool: &mut BuildTool, which: usize, v: Vec3, decimals: usize) {
    tool.ui.vec_clipboard[which] = Some(v);
    ui.ctx().copy_text(vector_text(v, decimals));
}

/// "Paste" of the shape parameters (LLPanelObject::onPasteParams).
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
fn shape_pair(
    ui: &mut egui::Ui,
    p: &Palette,
    title: &str,
    values: [&mut f32; 2],
    axes: [&str; 2],
    ranges: [std::ops::RangeInclusive<f32>; 2],
    increment: f32,
    decimals: usize,
) -> Edit {
    let mut edit = Edit::default();
    if super::legacy::active(ui) {
        super::common::label(ui, p, title);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let width = ((ui.available_width() - 32.0) * 0.5).max(40.0);
            for ((value, axis), range) in values.into_iter().zip(axes).zip(ranges) {
                ui.add_sized(
                    [12.0, 20.0],
                    egui::Label::new(RichText::new(axis.chars().next().unwrap_or(' ').to_string()).size(11.0)),
                )
                .on_hover_text(axis);
                edit = edit.or(spin(ui, value, increment, range, decimals, width, false));
            }
        });
        return edit;
    }
    ui.horizontal(|ui| {
        let label_width = (ui.available_width() - 182.0).max(70.0);
        ui.allocate_ui_with_layout(
            Vec2::new(label_width, 26.0),
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                ui.set_min_width(label_width);
                ui.add(egui::Label::new(RichText::new(title).size(super::common::BODY).color(p.muted)).truncate())
                    .on_hover_text(title);
            },
        );
        for ((value, axis), range) in values.into_iter().zip(axes).zip(ranges) {
            ui.allocate_ui_with_layout(Vec2::new(30.0, 26.0), egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(axis).size(super::common::SMALL).color(p.muted));
            });
            edit = edit.or(spin(ui, value, increment, range, decimals, 49.0, false));
        }
    });
    edit
}

fn shape_scalar(
    ui: &mut egui::Ui,
    p: &Palette,
    title: &str,
    value: &mut f32,
    increment: f32,
    range: std::ops::RangeInclusive<f32>,
    decimals: usize,
) -> Edit {
    if super::legacy::active(ui) {
        super::common::label(ui, p, title);
        return spin(ui, value, increment, range, decimals, ui.available_width().max(50.0), false);
    }
    let width = (ui.available_width() - 63.0).max(70.0);
    row(ui, p, title, width, |ui| spin(ui, value, increment, range, decimals, 58.0, false))
}

fn shape_column(
    ui: &mut egui::Ui,
    p: &Palette,
    tool: &mut BuildTool,
    world: &mut World,
    idx: usize,
    volume: &VolumeParams,
    chosen_type: Option<usize>,
    enabled: bool,
    flexible: bool,
    env: &mut Env,
) {
    ui.spacing_mut().button_padding = Vec2::new(3.0, 3.0);
    if volume.is_mesh() {
        super::mesh::show(ui, p, tool, world, idx, env);
        return;
    }
    let ui_now = ui_from_volume(volume);
    let legacy = super::legacy::active(ui);
    let mut u = ui_now;
    let mut e = Edit {
        changed: chosen_type.is_some(),
        commit: chosen_type.is_some(),
    };
    if let Some(ty) = chosen_type {
        u.ty = ty;
    }
    let ty = u.ty;
    if ty == SCULPTED {
        sculpt_section(ui, p, tool, world, idx, enabled, env);
    } else {
        let ends = [aurora_i18n::tr!("build-compact-begin"), aurora_i18n::tr!("build-compact-end")];
        ui.add_enabled_ui(enabled, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            e = e.or(shape_pair(
                ui,
                p,
                aurora_i18n::tr!("build-compact-path-cut"),
                [&mut u.cut.0, &mut u.cut.1],
                ends,
                [0.0..=0.98, 0.02..=1.0],
                0.025,
                3,
            ));
            e = e.or(shape_scalar(
                ui,
                p,
                aurora_i18n::tr!("build-ui-object-hollow"),
                &mut u.hollow,
                5.0,
                0.0..=95.0,
                1,
            ));
            let mut hollow_shape = |ui: &mut egui::Ui| {
                if let Some(h) = combo(ui, "hole_type", Some(u.hole), &holes(), 120.0, u.hollow > 0.0) {
                    u.hole = h;
                    e = e.or(Edit {
                        changed: true,
                        commit: true,
                    });
                }
            };
            if legacy {
                super::common::label(ui, p, aurora_i18n::tr!("build-ui-object-hollow-shape"));
                hollow_shape(ui);
            } else {
                let width = (ui.available_width() - 125.0).max(70.0);
                row(ui, p, aurora_i18n::tr!("build-ui-object-hollow-shape"), width, hollow_shape);
            }
            if !legacy || matches!(ty, 4..=6) {
                e = e.or(shape_scalar(
                    ui,
                    p,
                    aurora_i18n::tr!("build-ui-object-bias"),
                    &mut u.skew,
                    0.05,
                    -0.95..=0.95,
                    2,
                ));
            }
            let (tw, ti) = if circular_basic(ty) { (360.0, 18.0) } else { (180.0, 9.0) };
            e = e.or(shape_pair(
                ui,
                p,
                aurora_i18n::tr!("build-compact-twist"),
                [&mut u.twist.0, &mut u.twist.1],
                ends,
                [-tw..=tw, -tw..=tw],
                ti,
                0,
            ));
            let hole_label = matches!(ty, 4..=6);
            let (rx, ry) = match ty {
                3 => (0.0..=1.0, 0.0..=1.0),
                4..=6 => (0.05..=1.0, 0.05..=0.5),
                0..=2 => (-1.0..=1.0, -1.0..=1.0),
                _ => (0.0..=2.0, 0.0..=2.0),
            };
            let [x, y] = &mut u.top_size;
            e = e.or(shape_pair(
                ui,
                p,
                if hole_label {
                    aurora_i18n::tr!("build-ui-object-hole-size")
                } else {
                    aurora_i18n::tr!("build-ui-object-bevel")
                },
                [x, y],
                ["X", "Y"],
                [rx, ry],
                0.05,
                3,
            ));
            let [x, y] = &mut u.shear;
            e = e.or(shape_pair(
                ui,
                p,
                aurora_i18n::tr!("build-ui-object-tilt"),
                [x, y],
                ["X", "Y"],
                [-0.5..=0.5, -0.5..=0.5],
                0.05,
                3,
            ));
            let adv = match ty {
                3 => aurora_i18n::tr!("build-compact-hollow-cut"),
                0..=2 => aurora_i18n::tr!("build-compact-slice"),
                _ => aurora_i18n::tr!("build-compact-profile-cut"),
            };
            e = e.or(shape_pair(
                ui,
                p,
                adv,
                [&mut u.adv_cut.0, &mut u.adv_cut.1],
                ends,
                [0.0..=0.98, 0.02..=1.0],
                0.02,
                3,
            ));
            if ty != 0 {
                let [x, y] = &mut u.taper;
                e = e.or(shape_pair(
                    ui,
                    p,
                    aurora_i18n::tr!("build-ui-object-bevel"),
                    [x, y],
                    ["X", "Y"],
                    [-1.0..=1.0, -1.0..=1.0],
                    0.05,
                    3,
                ));
            }
            if !legacy || matches!(ty, 4..=6) {
                e = e.or(shape_scalar(
                    ui,
                    p,
                    aurora_i18n::tr!("build-ui-object-radius"),
                    &mut u.radius,
                    0.05,
                    -1.0..=1.0,
                    3,
                ));
                e = e.or(shape_scalar(
                    ui,
                    p,
                    aurora_i18n::tr!("build-ui-object-revolutions"),
                    &mut u.revolutions,
                    0.1,
                    1.0..=4.0,
                    2,
                ));
            }
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
    let width = (ui.available_width() - 69.0).max(70.0);
    let r = row(ui, p, aurora_i18n::tr!("build-ui-object-sculpt-texture"), width, |ui| {
        super::common::swatch(
            ui,
            p,
            env.images,
            &mut tool.ui.wanted_images,
            Some(sc.texture),
            "",
            64.0,
            false,
            enabled,
        )
    });
    if r.clicked() {
        tool.ui
            .picker
            .open(aurora_i18n::tr!("build-ui-object-choose-sculpt-map"), sc.texture);
        tool.ui.pick_target = Some(super::PickTarget::Sculpt);
    }
    let base = sc.sculpt_type & LL_SCULPT_TYPE_MASK;
    let shown = if base == 0 { 3 } else { base };
    let mut new = sc;
    let width = (ui.available_width() - 115.0).max(70.0);
    if let Some(t) = row(ui, p, aurora_i18n::tr!("build-ui-object-join-type"), width, |ui| {
        combo(ui, "sculpt_type", Some(shown), &sculpt_types(), 110.0, enabled)
    }) {
        new.sculpt_type = (sc.sculpt_type & !LL_SCULPT_TYPE_MASK) | t;
    }
    if let Some(v) = check(
        ui,
        p,
        sc.sculpt_type & LL_SCULPT_FLAG_MIRROR != 0,
        false,
        aurora_i18n::tr!("build-ui-object-mirror"),
        enabled,
    ) {
        new.sculpt_type = if v {
            new.sculpt_type | LL_SCULPT_FLAG_MIRROR
        } else {
            new.sculpt_type & !LL_SCULPT_FLAG_MIRROR
        };
    }
    if let Some(v) = check(
        ui,
        p,
        sc.sculpt_type & LL_SCULPT_FLAG_INVERT != 0,
        false,
        aurora_i18n::tr!("status-inside-out"),
        enabled,
    ) {
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
    /// "p": a vector pasted from the system clipboard (`which`: position,
    /// size, rotation), clamped like "P".
    pub fn paste_vector(&mut self, world: &mut World, s: &BuildSettings, which: usize, text: &str) {
        let Some(v) = parse_vector(text) else {
            self.status = aurora_i18n::tr!("build-ui-object-the-clipboard-does-not-contain-a-vector-x-y").into();
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
