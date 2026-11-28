//! Avatar Studio shape handles (#5022): the dots drawn over the avatar and
//! the bar that takes the shape editor's window's place while they show.
//! The dots follow the poser's joint markers (`ik_edit/poser/overlay.rs`):
//! background areas the camera's drag goes around, sized by the edit
//! handle size of Preferences › Build. Aurora's own: the Legacy Viewer has
//! no direct shape manipulation.

use super::{hud::Projector, widgets};
use crate::{
    shape_editor::{
        State, catalog, face_surface,
        handles::{self, Body, Drag, Grip, Pose},
        mesh_areas::MeshAreas,
    },
    theme::Palette,
    world::World,
};
use egui::{RichText, Sense, Stroke, Vec2};
use glam::Vec3;

/// What the bar asks of the shape editor.
#[derive(Default)]
pub(super) struct Bar {
    pub reset: bool,
    pub close: bool,
    pub save: bool,
    /// Undo (false) or redo (true).
    pub history: Option<bool>,
}

/// The bar of the handles view, above the bottom bar: the switch back to the
/// sliders, Body or Head, undo and redo, and Save as, Cancel and Save as in
/// the shape editor's footer (Save as opens its own name dialog).
pub(super) fn bar(ctx: &egui::Context, p: &Palette, world: &World, st: &mut State) -> Bar {
    let mut out = Bar::default();
    let Some(draft) = &st.draft else {
        return out;
    };
    let busy = draft.pending.is_some();
    let modify = draft.can_modify(world);
    let copy = draft.can_copy(world);
    let mut save_as = false;
    let (can_undo, can_redo, dirty) = (draft.history.can_undo(), draft.history.can_redo(), draft.dirty());
    let name = draft.item.name.clone();
    egui::Area::new(egui::Id::unique("shape_handles_bar"))
        // above the bottom bar: the top of the screen holds the Studio's bar
        // and the avatar's head and name tag
        .anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -(super::bars::islands::SLOT + 30.0)))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            widgets::island(ui, p, |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(super::bars::islands::SLOT);
                    widgets::glyph(ui, "man-filled", p.muted, 20.0);
                    ui.add(egui::Label::new(RichText::new(&name).strong().color(p.ink)).truncate());
                    ui.add_space(6.0);
                    widgets::switch_labeled(ui, p, &mut st.handles, aurora_i18n::tr!("shape-handles"))
                        .on_hover_text(aurora_i18n::tr!("shape-handles-hint"));
                    ui.add_space(6.0);
                    // the body's handles or the head's, the camera following
                    let parts = [handles::Part::Body, handles::Part::Head];
                    let mut part = parts.iter().position(|p| *p == st.handles_part).unwrap_or(0);
                    ui.scope(|ui| {
                        ui.set_width(150.0);
                        let labels = [
                            (aurora_i18n::tr!("shape-group-body"), true),
                            (aurora_i18n::tr!("shape-group-head"), true),
                        ];
                        if widgets::tabs(ui, p, &mut part, &labels) && parts[part] != st.handles_part {
                            st.handles_part = parts[part];
                            st.part_framing = true;
                            st.handle_entry = None;
                            st.handles_area = None;
                        }
                    });
                    // the area shown, and the way back to all of them
                    if let Some(area) = st.handles_area
                        && widgets::action_button(
                            ui,
                            p,
                            "arrow-left",
                            area_name(area),
                            widgets::ActionKind::Flat,
                            Vec2::new(0.0, super::bars::islands::SLOT - 6.0),
                        )
                        .on_hover_text(aurora_i18n::tr!("shape-handles-areas-back"))
                        .clicked()
                    {
                        st.handles_area = None;
                        st.handle_entry = None;
                    }
                    ui.add_space(6.0);
                    let editable = modify && !busy;
                    if widgets::icon_button(ui, p, "arrow-back-up", aurora_i18n::tr!("shape-undo"), editable && can_undo) {
                        out.history = Some(false);
                    }
                    if widgets::icon_button(ui, p, "arrow-forward-up", aurora_i18n::tr!("shape-redo"), editable && can_redo) {
                        out.history = Some(true);
                    }
                    ui.add_space(6.0);
                    let size = Vec2::new(0.0, super::bars::islands::SLOT - 6.0);
                    // the footer's Save as: a copy under a new name, worn
                    // (needs copy rights, as there)
                    save_as = widgets::action_button_if(
                        ui,
                        p,
                        "copy-filled",
                        aurora_i18n::tr!("common-save-as"),
                        widgets::ActionKind::Flat,
                        size,
                        copy && !busy,
                    )
                    .clicked();
                    if widgets::action_button_if(
                        ui,
                        p,
                        "restore",
                        aurora_i18n::tr!("common-cancel"),
                        widgets::ActionKind::Flat,
                        size,
                        !busy,
                    )
                    .clicked()
                    {
                        out.reset = true;
                        out.close = true;
                    }
                    if widgets::action_button_if(
                        ui,
                        p,
                        "device-floppy",
                        aurora_i18n::tr!("common-save"),
                        widgets::ActionKind::Primary,
                        size,
                        editable && dirty,
                    )
                    .clicked()
                    {
                        out.save = true;
                    }
                });
                let note = if busy {
                    Some((aurora_i18n::tr!("shape-saving").to_owned(), p.muted))
                } else if !modify {
                    Some((aurora_i18n::tr!("shape-readonly").to_owned(), p.warn))
                } else if st.handles_blocked {
                    Some((aurora_i18n::tr!("shape-handles-posing").to_owned(), p.muted))
                } else if !st.message.is_empty() {
                    Some((st.message.clone(), p.warn))
                } else if st.handles && st.handles_area.is_none() {
                    let hint = match st.handles_part {
                        handles::Part::Body => aurora_i18n::tr!("shape-handles-areas-body"),
                        handles::Part::Head => aurora_i18n::tr!("shape-handles-areas"),
                    };
                    Some((hint.to_owned(), p.muted))
                } else {
                    None
                };
                if let Some((text, color)) = note {
                    ui.label(RichText::new(text).size(12.0).color(color));
                }
            });
        });
    if save_as {
        // the shape editor's Save as dialog, named after the item
        st.copy_name = Some(name);
    }
    out
}

/// The skeleton of the avatar as drawn: its joints and where the camera is.
pub(crate) struct Scene<'a> {
    pub proj: &'a Projector,
    pub eye: Vec3,
    /// Preferences › Build's edit handle size (1 = 100 %).
    pub scale: f32,
}

fn screen(proj: &Projector, p: Vec3) -> Option<glam::Vec2> {
    proj.project(p).map(|(at, _)| glam::Vec2::new(at.x, at.y))
}

fn range(id: i32) -> Option<(f32, f32)> {
    catalog::PARAMS.iter().find(|p| p.id == id).map(|p| (p.min, p.max))
}

/// One dot to draw this frame.
struct Dot {
    grip: Grip,
    drawn: Vec3,
    at: egui::Pos2,
    /// How much of its sliders' motion shows on screen (0: seen along it).
    visible: f32,
    /// The way its strongest slider moves it on screen (unit), for the arrow.
    arrow: Option<egui::Vec2>,
}

/// Samples of a track: enough for a bending slider to read as a curve.
const TRACK_SAMPLES: usize = 12;

/// The other dots' opacity while one is held or typed into: still there to
/// see where they are, out of the way of the part being shaped.
const BEHIND_OPACITY: f32 = 0.18;

/// The glow of an area: at rest and under the pointer (the strength of the
/// violet at its centre). The area shown has none: only what a click would
/// open glows.
const AREA_IDLE: f32 = 0.30;
const AREA_HOVER: f32 = 0.60;

/// A dot's size on screen (Preferences › Build's edit handle size).
fn dot_size(scene: &Scene) -> f32 {
    14.0 * scene.scale.clamp(0.5, 3.0)
}

/// The name of an area: the shape editor's tab of its sliders on the head,
/// the part of the body otherwise.
fn area_name(area: usize) -> &'static str {
    match area {
        handles::AREA_HEAD_NECK => aurora_i18n::tr!("shape-area-head-neck"),
        handles::AREA_TORSO => aurora_i18n::tr!("shape-group-torso"),
        handles::AREA_ARMS => aurora_i18n::tr!("shape-area-arms"),
        handles::AREA_LEGS => aurora_i18n::tr!("shape-group-legs"),
        _ => super::shape::groups().get(area).copied().unwrap_or_default(),
    }
}

/// One glow of an area on screen: its centre, its reach and its area.
type Spot = (egui::Pos2, f32, usize);

/// How far apart two glows of an area may be and still join into one patch
/// (times the sum of their reach): the face's dots sit close together; the
/// body's volumes are glowed by their thickness, so a thigh and a shin need
/// the gap of a knee.
const LINK_FACE: f32 = 0.8;
const LINK_BODY: f32 = 2.0;
/// The area under a point: the one of the nearest glow that reaches it, a
/// glow reaching as far as glows join (`link`, past 1).
fn area_at(at: egui::Pos2, spots: &[Spot], link: f32) -> Option<usize> {
    let reach = link.max(1.0);
    spots
        .iter()
        .map(|(spot, radius, area)| (spot.distance(at) / (radius * reach).max(1.0), *area))
        .filter(|(d, _)| *d < 1.0)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, area)| area)
}

/// The body's glows: one over each collision volume, as large as it shows on
/// screen, so that an area's run together along the arm, the leg or the
/// torso.
fn volume_spots(lib: &crate::scene::avatar::AvatarLibrary, joints: &[glam::Mat4], scene: &Scene) -> Vec<Spot> {
    let Some(skeleton) = &lib.skeleton else {
        return Vec::new();
    };
    let project = |at: Vec3| scene.proj.project(at).map(|(pos, _)| pos);
    skeleton
        .joints
        .iter()
        .enumerate()
        .take(lib.rig.len())
        .filter(|(_, info)| info.is_collision_volume)
        .filter_map(|(j, _)| {
            let m = joints.get(j)?;
            let centre = m.w_axis.truncate();
            let at = project(centre)?;
            // its thickness as seen: the middle of its axes' reach on screen
            // (an arm's width, not its length, which its patch's own
            // length covers)
            let mut axes: Vec<f32> = [m.x_axis, m.y_axis, m.z_axis]
                .iter()
                .filter_map(|axis| project(centre + axis.truncate()))
                .map(|end| end.distance(at))
                .collect();
            axes.sort_by(|a, b| a.total_cmp(b));
            let reach = axes.get(1).or(axes.last()).copied().unwrap_or(0.0);
            let area = handles::area_of(&lib.rig, handles::Part::Body, handles::Anchor::on(j, Vec3::ZERO), 0);
            Some((at, (reach * 1.15).max(20.0 * scene.scale.clamp(0.5, 3.0)), area))
        })
        .collect()
}

/// The patches an area's glows make on screen: its spots that touch, joined
/// (both ears, each arm), each the indices of its spots and their area.
fn patches(spots: &[Spot], link: f32) -> Vec<(usize, Vec<usize>)> {
    let mut patch: Vec<usize> = (0..spots.len()).collect();
    fn root(patch: &mut [usize], mut i: usize) -> usize {
        while patch[i] != i {
            patch[i] = patch[patch[i]];
            i = patch[i];
        }
        i
    }
    for i in 0..spots.len() {
        for j in i + 1..spots.len() {
            let ((a, ra, area_a), (b, rb, area_b)) = (spots[i], spots[j]);
            if area_a == area_b && a.distance(b) < (ra + rb) * link {
                let (x, y) = (root(&mut patch, i), root(&mut patch, j));
                patch[x] = y;
            }
        }
    }
    let mut out: Vec<(usize, usize, Vec<usize>)> = Vec::new();
    for (i, (_, _, area)) in spots.iter().enumerate() {
        let r = root(&mut patch, i);
        match out.iter_mut().find(|(root, ..)| *root == r) {
            Some((.., members)) => members.push(i),
            None => out.push((r, *area, vec![i])),
        }
    }
    out.into_iter().map(|(_, area, members)| (area, members)).collect()
}

/// One soft glow over a patch: an ellipse along the patch's length (an arm,
/// the row of the eyes), strongest in its middle and fading to nothing at
/// its edge, drawn once so that dense dots never stack into a solid blot.
fn paint_patch(painter: &egui::Painter, spots: &[Spot], members: &[usize], color: egui::Color32) {
    const SEGMENTS: u32 = 32;
    let points: Vec<(egui::Pos2, f32)> = members.iter().filter_map(|i| spots.get(*i)).map(|(at, r, _)| (*at, *r)).collect();
    let Some(n) = (!points.is_empty()).then_some(points.len() as f32) else {
        return;
    };
    let centre = points.iter().fold(egui::Vec2::ZERO, |sum, (at, _)| sum + at.to_vec2()) / n;
    // the patch's long way: the main axis of its spots' spread
    let (mut xx, mut xy, mut yy) = (0.0, 0.0, 0.0);
    for (at, _) in &points {
        let d = at.to_vec2() - centre;
        xx += d.x * d.x;
        xy += d.x * d.y;
        yy += d.y * d.y;
    }
    let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
    let (u, v) = (egui::vec2(angle.cos(), angle.sin()), egui::vec2(-angle.sin(), angle.cos()));
    let reach = |axis: egui::Vec2| {
        points
            .iter()
            .map(|(at, r)| (at.to_vec2() - centre).dot(axis).abs() + r)
            .fold(0.0, f32::max)
    };
    let (along, across) = (reach(u), reach(v));
    let centre = centre.to_pos2();
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(centre, color);
    // a ring part of the way out keeps the middle even, then the fade
    for (ring, (scale, strength)) in [(0.6, 0.8), (1.0, 0.0)].into_iter().enumerate() {
        for i in 0..SEGMENTS {
            let a = std::f32::consts::TAU * i as f32 / SEGMENTS as f32;
            let at = centre + u * (along * scale * a.cos()) + v * (across * scale * a.sin());
            mesh.colored_vertex(at, color.gamma_multiply(strength));
            let (this, next) = (1 + ring as u32 * SEGMENTS + i, 1 + ring as u32 * SEGMENTS + (i + 1) % SEGMENTS);
            if ring == 0 {
                mesh.add_triangle(0, this, next);
            } else {
                mesh.add_triangle(this - SEGMENTS, this, next);
                mesh.add_triangle(this - SEGMENTS, next, next - SEGMENTS);
            }
        }
    }
    painter.add(mesh);
}

/// The areas before one is picked (lit by the caller on the meshes, or as
/// glows): the one under the pointer named, and a click shows its dots.
fn area_overview(ctx: &egui::Context, p: &Palette, st: &mut State, hovered: Option<usize>) {
    let Some(area) = hovered else {
        return;
    };
    area_tip(ctx, p, area);
    if ctx.input(|i| i.pointer.primary_clicked()) {
        st.handles_area = Some(area);
        st.handle_entry = None;
    }
}

/// The pointing hand and the name of the area under the pointer.
fn area_tip(ctx: &egui::Context, p: &Palette, area: usize) {
    ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
    if let Some(at) = ctx.input(|i| i.pointer.hover_pos()) {
        egui::Tooltip::always_open(
            ctx.clone(),
            egui::LayerId::background(),
            // one per area: a tooltip keeps the size it first had
            egui::Id::unique(("shape-area-tip", area)),
            egui::PopupAnchor::Position(at + egui::vec2(16.0, 14.0)),
        )
        .show(|ui| {
            ui.label(RichText::new(area_name(area)).strong().color(p.ink));
        });
    }
}

/// The handle as a double-headed arrow along its path (a dot seen along its
/// motion stays round), or a resize handle's grow mark, four short arrows
/// out of the dot: what it does shows before it is touched.
fn paint_handle(painter: &egui::Painter, p: &Palette, center: egui::Pos2, size: f32, arrow: Option<egui::Vec2>, resize: bool, lit: bool) {
    let color = if lit { p.violet_light } else { p.violet };
    if resize {
        let (inner, outer) = (size * 0.36, size * if lit { 0.95 } else { 0.8 });
        let tick = Stroke::new(if lit { 2.0 } else { 1.6 }, color);
        let outline = Stroke::new(tick.width + 2.0, p.on_accent);
        for stroke in [outline, tick] {
            for corner in [
                egui::vec2(1.0, 1.0),
                egui::vec2(-1.0, 1.0),
                egui::vec2(1.0, -1.0),
                egui::vec2(-1.0, -1.0),
            ] {
                let way = corner.normalized();
                painter.arrow(center + way * inner, way * (outer - inner), stroke);
            }
        }
        painter.circle_filled(center, size * 0.22, color);
        painter.circle_stroke(center, size * 0.22, Stroke::new(1.5, p.on_accent));
        return;
    }
    match arrow {
        Some(dir) => {
            let reach = size * if lit { 0.95 } else { 0.8 };
            let shaft = Stroke::new(if lit { 2.5 } else { 2.0 }, color);
            // a light outline under the shaft keeps it readable on any skin
            let outline = Stroke::new(shaft.width + 2.0, p.on_accent);
            for stroke in [outline, shaft] {
                painter.arrow(center, dir * reach, stroke);
                painter.arrow(center, -dir * reach, stroke);
            }
            painter.circle_filled(center, size * 0.22, color);
            painter.circle_stroke(center, size * 0.22, Stroke::new(1.5, p.on_accent));
        }
        None => {
            let radius = size * if lit { 0.42 } else { 0.32 };
            painter.circle_filled(center, radius, color);
            painter.circle_stroke(center, radius, Stroke::new(1.5, p.on_accent));
        }
    }
}

/// A slider's end label at one end of its track, on a small chip.
fn paint_end_label(painter: &egui::Painter, ctx: &egui::Context, p: &Palette, end: egui::Pos2, outward: egui::Vec2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_owned(), egui::FontId::proportional(11.0), p.ink);
    let size = galley.size();
    // beyond the end, along the track, clear of the line
    let center = end + outward * (8.0 + 0.5 * size.x.max(size.y));
    let rect = egui::Rect::from_center_size(center, size).expand(3.0);
    painter.rect_filled(rect, ctx.global_style().visuals.widgets.inactive.corner_radius, p.panel);
    painter.galley(rect.min + egui::vec2(3.0, 3.0), galley, p.ink);
}

/// Draw the handles and drag them. Whether the pointer is the handles'
/// (over a dot, or dragging one): the Studio's camera leaves it alone.
pub(crate) fn overlay(ctx: &egui::Context, p: &Palette, world: &mut World, st: &mut State, scene: &Scene) -> bool {
    let shown = st.handles_view() && !st.handles_blocked;
    let editable = st
        .draft
        .as_ref()
        .is_some_and(|draft| draft.pending.is_none() && draft.can_modify(world));
    if !shown || !editable {
        end_drag(st);
        st.drop_mesh_part();
        return false;
    }
    let Some(frame) = crate::ik_edit::Frame::new(world) else {
        end_drag(st);
        st.drop_mesh_part();
        return false;
    };
    let lib = world.avatar_lib.clone();
    let male = st
        .draft
        .as_ref()
        .is_some_and(|d| d.edited.params.get(&80).copied().unwrap_or(0.0) > 0.5);
    let layout = st.handle_layout(&lib, male).clone();
    let Some(pose) = Pose::from_joints(&lib.rig, frame.model, &frame.joints) else {
        end_drag(st);
        st.drop_mesh_part();
        return false;
    };
    // where the dots are drawn: a head vertex carries the morphs of the
    // values shown
    let drawn_body = Body::new(&lib, world.server_visual_params(), pose);
    let shown_values = st.draft.as_ref().map(|d| d.edited.params.clone()).unwrap_or_default();
    // once for every dot (only the system head's vertices need them)
    let shown_weights = if layout.system_head {
        drawn_body.weights(&shown_values)
    } else {
        Default::default()
    };
    // under a mesh head, each face dot on the head's own surface
    let surface = st
        .face_surface
        .as_ref()
        .filter(|_| layout.part == handles::Part::Head && !layout.system_head)
        .map(|(_, lifted)| lifted.clone());
    let to_camera = |at: Vec3| (scene.eye - at).normalize_or_zero();
    let mut dots = Vec::new();
    // the areas with dots, whichever side of the avatar faces the camera
    let mut areas_with_dots: Vec<usize> = Vec::new();
    for (index, handle) in layout.handles.iter().enumerate() {
        let params: Vec<i32> = handles::shown_params(&layout, handle, st.worn.system(layout.part), &st.worn.rigged).collect();
        if params.is_empty() {
            continue;
        }
        if !areas_with_dots.contains(&handle.area) {
            areas_with_dots.push(handle.area);
        }
        for mirrored in [false, true] {
            if mirrored && handle.mirror.is_none() {
                continue;
            }
            let anchor = handle.anchor(mirrored);
            let anchor = match surface.as_ref().and_then(|lifted| lifted.get(&face_surface::key(&anchor))) {
                Some(local) => handles::Anchor { local: *local, ..anchor },
                None => anchor,
            };
            let grip = Grip {
                handle: index,
                mirrored,
                anchor,
                params: params.clone(),
                resize: handle.resize,
            };
            let dragged = st.handle_drag.as_ref().is_some_and(|(d, _)| d.grip.same_dot(&grip));
            let (Some(drawn), Some(normal)) = (
                drawn_body.drawn_with(&shown_weights, anchor, &frame.joints),
                drawn_body.drawn_normal(anchor, &frame.joints),
            ) else {
                continue;
            };
            let view = to_camera(drawn);
            // the far side of the body: hidden, unless held
            if normal.dot(view) < -0.15 && !dragged {
                continue;
            }
            let Some((at, _)) = scene.proj.project(drawn) else {
                continue;
            };
            let rotation = frame.joints[anchor.joint].to_scale_rotation_translation().1;
            // the strongest shown slider's way on screen
            let (visible, direction) = handle
                .params
                .iter()
                .zip(&handle.directions)
                .filter(|(id, _)| params.contains(id))
                .map(|(_, local)| {
                    let local = if mirrored { Vec3::new(local.x, -local.y, local.z) } else { *local };
                    let direction = rotation * local;
                    (direction.reject_from_normalized(view).length(), direction)
                })
                .fold((0.0, Vec3::ZERO), |best, next| if next.0 > best.0 { next } else { best });
            let arrow = scene
                .proj
                .project(drawn + direction * 0.05)
                .map(|(tip, _)| tip - at)
                .filter(|_| visible >= 0.3)
                .and_then(|v| (v.length() > 1e-3).then(|| v.normalized()));
            dots.push(Dot {
                grip,
                drawn,
                at,
                visible,
                arrow,
            });
        }
    }
    let mut owned = st.handle_drag.is_some();
    // the areas first (a glow over each part of the avatar), then the dots
    // of the one picked
    let area_of_dot = |d: &Dot| layout.handles.get(d.grip.handle).map(|h| h.area);
    let link = match layout.part {
        handles::Part::Head => LINK_FACE,
        handles::Part::Body => LINK_BODY,
    };
    let spots: Vec<Spot> = match layout.part {
        // the face: around its dots, which run together over each part
        handles::Part::Head => dots
            .iter()
            .filter_map(|d| Some((d.at, 24.0 * scene.scale.clamp(0.5, 3.0), area_of_dot(d)?)))
            .collect(),
        // the body: over its collision volumes, the part a fitted mesh follows
        handles::Part::Body => volume_spots(&lib, &frame.joints, scene)
            .into_iter()
            .filter(|s| areas_with_dots.contains(&s.2))
            .collect(),
    };
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("shape-handle-areas")));
    // the areas on the avatar's own meshes, as the renderer draws and picks
    // them from their skinning (mesh_areas.rs): whatever the body's shape,
    // a feral's or a furry's included. Not on the system head (its face is
    // morphs on one bone), nor while its meshes are not drawn (an
    // impostor): the glows over the dots and volumes then.
    let on_meshes = !layout.system_head && st.meshes_drawn != Some(false);
    if on_meshes {
        let key = (layout.male, layout.part, layout.system_head, areas_with_dots.clone());
        if st.mesh_areas.as_ref().is_none_or(|(made_for, _)| *made_for != key) {
            st.mesh_areas = Some((key, MeshAreas::new(&lib.rig, &layout, &areas_with_dots)));
        }
    }
    let pointer = ctx
        .input(|i| i.pointer.hover_pos())
        .filter(|at| !widgets::pointer_over_ui(ctx, *at));
    st.mesh_tint = None;
    st.mesh_pick_wanted = on_meshes && pointer.is_some() && st.handle_drag.is_none();
    // the pick's newest answer: it comes a frame or two behind the pointer,
    // and holding it while the pointer moves keeps the area lit (waiting
    // for an answer read right under the pointer left it dark through
    // every move)
    let picked = pointer
        .and(st.mesh_pick)
        .and_then(|(class, _)| st.mesh_areas.as_ref()?.1.area(class))
        .filter(|_| on_meshes);
    let area_under = |at: egui::Pos2| if on_meshes { picked } else { area_at(at, &spots, link) };
    let Some(area) = st.handles_area else {
        end_drag(st);
        st.handle_hover = None;
        // the areas cover the whole avatar: they never hold the pointer, so
        // that a drag over them still turns the Studio camera and the wheel
        // still zooms; only a click (no drag) picks an area
        let hovered = pointer.and_then(area_under).or(st.demo_hover_area);
        if on_meshes {
            st.mesh_tint = hovered.and_then(|a| Some(st.mesh_areas.as_ref()?.1.mask(a)));
        } else {
            for (area, members) in patches(&spots, link) {
                let strength = if hovered == Some(area) { AREA_HOVER } else { AREA_IDLE };
                paint_patch(&painter, &spots, &members, p.violet_light.gamma_multiply(strength));
            }
        }
        area_overview(ctx, p, st, hovered);
        st.handles_pointer = false;
        return false;
    };
    // the dots of the area shown come first: off them, another area under
    // the pointer lights as in the overview, a click away
    let on_dot = |at: egui::Pos2| {
        dots.iter()
            .any(|d| area_of_dot(d) == Some(area) && d.at.distance(at) < dot_size(scene))
    };
    let free = st.handle_drag.is_none() && st.handle_entry.is_none() && st.handle_hover.is_none();
    let other = pointer
        .filter(|at| free && !on_dot(*at))
        .and_then(area_under)
        .filter(|a| *a != area);
    if on_meshes {
        st.mesh_tint = other.and_then(|a| Some(st.mesh_areas.as_ref()?.1.mask(a)));
    } else {
        // only the area a click would open glows, not the one shown
        for (_, members) in patches(&spots, link).into_iter().filter(|(a, _)| other == Some(*a)) {
            paint_patch(&painter, &spots, &members, p.violet_light.gamma_multiply(AREA_HOVER));
        }
    }
    if let Some(other) = other {
        area_tip(ctx, p, other);
    }
    if st.handle_drag.is_none()
        && st.handle_entry.is_none()
        && ctx.input(|i| i.pointer.primary_clicked())
        && let Some(at) = ctx.input(|i| i.pointer.interact_pos())
        && !widgets::pointer_over_ui(ctx, at)
        && !on_dot(at)
    {
        // a click off the dots: the area under it (this one, another), or
        // off the avatar, back to all of them
        st.handles_area = area_under(at);
    }
    dots.retain(|d| area_of_dot(d) == st.handles_area);
    let mut tip: Option<(egui::Pos2, Vec<i32>)> = None;
    // the handle held, or the one hovered last frame, and which of its dots
    let focus = st
        .handle_drag
        .as_ref()
        .map(|(d, _)| (d.grip.handle, d.grip.mirrored))
        .or(st.handle_hover);
    st.handle_hover = None;
    if let Some(focus) = focus {
        paint_tracks(ctx, p, world, st, scene, &frame, &dots, focus);
    }
    let size = dot_size(scene);
    // the handle held or typed into: the others step back meanwhile
    let busy = st
        .handle_drag
        .as_ref()
        .map(|(d, _)| d.grip.handle)
        .or(st.handle_entry.map(|e| e.handle));
    for dot in &dots {
        let dragged = st.handle_drag.as_ref().is_some_and(|(d, _)| d.grip.same_dot(&dot.grip));
        // its twin lights with it: one slider moves both
        let lit = dragged || focus.is_some_and(|(handle, _)| handle == dot.grip.handle);
        let behind = busy.is_some_and(|handle| handle != dot.grip.handle);
        // a resize works radially from any side: never faded
        let faded = dot.visible < 0.3 && !dragged && !dot.grip.resize;
        let response = egui::Area::new(egui::Id::unique(("shape-handle", dot.grip.handle, dot.grip.mirrored)))
            .fixed_pos(dot.at - egui::Vec2::splat(size * 0.5))
            .order(egui::Order::Background)
            .movable(false)
            .show(ctx, |ui| {
                if behind {
                    ui.multiply_opacity(BEHIND_OPACITY);
                } else if faded {
                    ui.multiply_opacity(0.35);
                }
                let (rect, response) = ui.allocate_exact_size(egui::Vec2::splat(size), Sense::click_and_drag());
                paint_handle(ui.painter(), p, rect.center(), size, dot.arrow, dot.grip.resize, lit);
                response
            })
            .inner;
        if response.contains_pointer() || response.dragged() {
            owned = true;
            st.handle_hover = Some((dot.grip.handle, dot.grip.mirrored));
            tip = Some((dot.at, dot.grip.params.clone()));
        }
        // a double-click opens the number window: the sliders' own value
        // fields, to type an exact value
        if response.double_clicked() {
            st.handle_entry = Some(crate::shape_editor::HandleEntry {
                handle: dot.grip.handle,
                mirrored: dot.grip.mirrored,
                at: dot.at,
                focused: false,
            });
        }
        if response.drag_started()
            && let Some(pointer) = response.interact_pointer_pos()
            && let Some(draft) = &mut st.draft
            && let Some(pose) = Pose::from_joints(&lib.rig, frame.model, &frame.joints)
        {
            let body = Body::new(&lib, world.server_visual_params(), pose.clone());
            let origin = ctx.input(|i| i.pointer.press_origin()).unwrap_or(pointer);
            if let Some(drag) = Drag::start(
                &body,
                &draft.edited.params,
                dot.grip.clone(),
                dot.drawn,
                dot.drawn - scene.eye,
                |p| screen(scene.proj, p),
                glam::Vec2::new(origin.x, origin.y),
            ) {
                draft.history.begin(&draft.edited.params);
                st.handle_drag = Some((drag, pose));
            }
        }
        // what the drag did, in the log (a dot reported going the wrong way)
        if response.drag_stopped()
            && let (Some((drag, _)), Some(draft)) = (&st.handle_drag, &st.draft)
            && drag.grip.same_dot(&dot.grip)
        {
            let pointer = ctx.input(|i| i.pointer.latest_pos()).unwrap_or(dot.at);
            log::info!(
                "{}",
                drag.describe(
                    &draft.edited.params,
                    glam::Vec2::new(pointer.x, pointer.y),
                    glam::Vec2::new(dot.at.x, dot.at.y)
                )
            );
        }
        if dragged
            && response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            let mut changed = false;
            if let (Some((drag, pose)), Some(draft)) = (&st.handle_drag, &mut st.draft) {
                let body = Body::new(&lib, world.server_visual_params(), pose.clone());
                changed = drag.step(
                    &body,
                    &mut draft.edited.params,
                    range,
                    |p| screen(scene.proj, p),
                    glam::Vec2::new(pointer.x, pointer.y),
                );
            }
            st.editing = true;
            if changed {
                st.changed(world);
            }
        }
    }
    if st.handle_drag.is_some() && st.demo_drag_from.is_none() && !ctx.input(|i| i.pointer.primary_down()) {
        end_drag(st);
    }
    owned |= number_window(ctx, p, world, st, &dots);
    // the number window says it all for its handle
    let tip = tip.filter(|_| st.handle_entry.is_none());
    if let Some((at, params)) = tip {
        let draft = st.draft.as_ref();
        egui::Tooltip::always_open(
            ctx.clone(),
            egui::LayerId::background(),
            // one per handle: a tooltip keeps the size it first had
            egui::Id::unique(("shape-handle-tip", params.clone())),
            egui::PopupAnchor::Position(at + egui::vec2(16.0, 14.0)),
        )
        .show(|ui| {
            for param in params.iter().filter_map(|id| catalog::PARAMS.iter().find(|p| p.id == *id)) {
                let value = draft.map_or(param.default, |d| d.value(param));
                let percent = (value - param.min) / (param.max - param.min) * 100.0;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(param.labels().0).strong().color(p.ink));
                    ui.label(RichText::new(aurora_i18n::number(f64::from(percent), 1)).color(p.muted));
                });
            }
        });
    }
    st.handles_pointer = owned;
    owned
}

/// The number window of a double-clicked handle, beside its dot: for each
/// of its sliders, the shape editor's own value field (0 to 100, the same
/// steps and number format), its keyboard taken at once so that a value can
/// be typed straight away. Escape or a press elsewhere closes it; what is
/// typed is one undo step, as a slider's. Whether the pointer is over it.
fn number_window(ctx: &egui::Context, p: &Palette, world: &mut World, st: &mut State, dots: &[Dot]) -> bool {
    let Some(mut entry) = st.handle_entry else {
        return false;
    };
    // it follows its dot while the camera moves
    let Some(dot) = dots
        .iter()
        .find(|d| d.grip.handle == entry.handle && d.grip.mirrored == entry.mirrored)
        .or_else(|| dots.iter().find(|d| d.grip.handle == entry.handle))
    else {
        st.handle_entry = None;
        return false;
    };
    entry.at = dot.at;
    let params: Vec<&catalog::Param> = dot
        .grip
        .params
        .iter()
        .filter_map(|id| catalog::PARAMS.iter().find(|p| p.id == *id))
        .collect();
    let Some(draft) = &mut st.draft else {
        st.handle_entry = None;
        return false;
    };
    let before = draft.edited.params.clone();
    let mut changed = false;
    let mut editing = false;
    let area = egui::Area::new(egui::Id::unique("shape-handle-number"))
        .fixed_pos(entry.at + egui::vec2(18.0, -14.0))
        .order(egui::Order::Foreground)
        .movable(false)
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::same(8), |ui| {
                for (i, param) in params.iter().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(param.labels().0).strong().color(p.ink));
                        let old = draft.value(param);
                        let mut percent = (old - param.min) / (param.max - param.min) * 100.0;
                        let field = ui.add_sized(
                            Vec2::new(62.0, 22.0),
                            egui::DragValue::new(&mut percent)
                                .range(0.0..=100.0)
                                .speed(0.1)
                                .custom_formatter(|value, _| aurora_i18n::number(value, 1))
                                .custom_parser(aurora_i18n::parse_number),
                        );
                        if i == 0 && !entry.focused {
                            field.request_focus();
                            entry.focused = true;
                        }
                        if field.changed() {
                            draft.set(param, param.min + (param.max - param.min) * percent / 100.0);
                            changed = true;
                        }
                        editing |= field.has_focus() || field.dragged();
                    });
                }
            });
        });
    if changed {
        draft.history.begin(&before);
    }
    let (escape, pressed_elsewhere) = ctx.input(|i| {
        (
            i.key_pressed(egui::Key::Escape),
            i.pointer.primary_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|at| !area.response.rect.contains(at) && at.distance(entry.at) > 12.0),
        )
    });
    st.handle_entry = (!escape && !pressed_elsewhere).then_some(entry);
    st.editing |= editing;
    if changed {
        st.changed(world);
    }
    area.response.contains_pointer()
}

/// The tracks of the focused handle (both its dots, one slider moving
/// both), under the dots: each slider's path from its minimum to its
/// maximum, a tick at its default, and, on the dot under the pointer, the
/// slider's own end labels (the shape editor's comparison captions).
#[allow(clippy::too_many_arguments)]
fn paint_tracks(
    ctx: &egui::Context,
    p: &Palette,
    world: &World,
    st: &State,
    scene: &Scene,
    frame: &crate::ik_edit::Frame,
    dots: &[Dot],
    (handle, mirrored): (usize, bool),
) {
    let Some(draft) = &st.draft else {
        return;
    };
    let lib = world.avatar_lib.clone();
    // a drag keeps the pose it was pressed in; a hover reads the drawn one
    let pose = match &st.handle_drag {
        Some((_, pose)) => Some(pose.clone()),
        None => Pose::from_joints(&lib.rig, frame.model, &frame.joints),
    };
    let Some(pose) = pose else {
        return;
    };
    let body = Body::new(&lib, world.server_visual_params(), pose);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("shape-handle-tracks")));
    let line = Stroke::new(2.0, p.violet_light.gamma_multiply(0.85));
    for dot in dots.iter().filter(|d| d.grip.handle == handle) {
        let Some(model) = body.point(&draft.edited.params, dot.grip.anchor) else {
            continue;
        };
        // the drawn point carries what the model leaves out (see `Drag`)
        let offset = dot.drawn - model;
        let project = |at: Vec3| scene.proj.project(at + offset).map(|(pos, _)| pos);
        for param in dot.grip.params.iter().filter_map(|id| catalog::PARAMS.iter().find(|p| p.id == *id)) {
            // (a bone has no size of its own to outline: its resize scrubs)
            if dot.grip.resize && !handles::is_volume(&lib.rig, dot.grip.anchor.joint) {
                continue;
            }
            if dot.grip.resize {
                paint_outlines(
                    &painter,
                    ctx,
                    p,
                    &body,
                    draft,
                    dot,
                    param,
                    &project,
                    dot.grip.mirrored == mirrored,
                    scene.eye,
                );
                continue;
            }
            let points: Vec<egui::Pos2> = handles::track(
                &body,
                &draft.edited.params,
                dot.grip.anchor,
                param.id,
                (param.min, param.max),
                TRACK_SAMPLES,
            )
            .into_iter()
            .filter_map(project)
            .collect();
            if points.len() < 2 {
                continue;
            }
            painter.add(egui::Shape::line(points.clone(), line));
            let mut at_default = draft.edited.params.clone();
            at_default.insert(param.id, param.default);
            if let Some(tick) = body.point(&at_default, dot.grip.anchor).and_then(project) {
                painter.circle_stroke(tick, 3.5, Stroke::new(1.5, p.muted));
            }
            if dot.grip.mirrored == mirrored {
                let (_, less, more) = param.labels();
                let (first, last) = (points[0], points[points.len() - 1]);
                let outward = |end: egui::Pos2, before: egui::Pos2| (end - before).normalized();
                paint_end_label(&painter, ctx, p, first, outward(first, points[1]), less);
                paint_end_label(&painter, ctx, p, last, outward(last, points[points.len() - 2]), more);
            }
        }
    }
}

/// A resize handle's gauge: the outline of its part at the slider's
/// smallest and largest values (the collision volume a fitted mesh
/// follows), and, on the dot under the pointer, the slider's end captions
/// on the side of the dot.
#[allow(clippy::too_many_arguments)]
fn paint_outlines(
    painter: &egui::Painter,
    ctx: &egui::Context,
    p: &Palette,
    body: &Body,
    draft: &crate::shape_editor::Draft,
    dot: &Dot,
    param: &catalog::Param,
    project: &dyn Fn(Vec3) -> Option<egui::Pos2>,
    labels: bool,
    eye: Vec3,
) {
    let view = (dot.drawn - eye).normalize_or_zero();
    let centre = body
        .point(&draft.edited.params, handles::Anchor::on(dot.grip.anchor.joint, Vec3::ZERO))
        .and_then(project);
    let way = centre.map_or(egui::Vec2::X, |c| (dot.at - c).normalized());
    let (_, less, more) = param.labels();
    // the smaller caption on the far side, so that close outlines never
    // stack the two
    for (value, stroke, text, way) in [
        (param.min, Stroke::new(1.5, p.muted), less, -way),
        (param.max, Stroke::new(2.0, p.violet_light.gamma_multiply(0.85)), more, way),
    ] {
        let mut values = draft.edited.params.clone();
        values.insert(param.id, value);
        let Some(volume) = body.joint(&values, dot.grip.anchor.joint) else {
            continue;
        };
        let ring: Vec<egui::Pos2> = handles::outline(volume, view, 24).into_iter().filter_map(project).collect();
        if ring.len() < 3 {
            continue;
        }
        painter.add(egui::Shape::closed_line(ring.clone(), stroke));
        if labels && let Some(c) = centre {
            // the ring's point on the dot's side, the caption beyond it
            let edge = ring
                .iter()
                .copied()
                .max_by(|a, b| (*a - c).dot(way).total_cmp(&(*b - c).dot(way)))
                .unwrap_or(c);
            paint_end_label(painter, ctx, p, edge, way, text);
        }
    }
}

/// The drag is over: one undo step. Avatar Studio calls it too when the
/// overlay stops being drawn mid-drag, so that no gesture stays open.
pub(crate) fn end_drag(st: &mut State) {
    if st.handle_drag.take().is_some()
        && let Some(draft) = &mut st.draft
    {
        draft.history.settle(&draft.edited.params);
    }
    st.handles_pointer = false;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An area's glows that touch make one patch (the row of an eye), apart
    /// ones another (the other ear), and two areas never share one.
    #[test]
    fn touching_glows_of_one_area_make_one_patch() {
        let spot = |x: f32, area: usize| (egui::pos2(x, 100.0), 10.0, area);
        let spots = [spot(0.0, 2), spot(12.0, 2), spot(20.0, 2), spot(200.0, 2), spot(14.0, 4)];
        let mut found: Vec<(usize, Vec<usize>)> = patches(&spots, 0.8);
        for (_, members) in &mut found {
            members.sort_unstable();
        }
        found.sort();
        assert_eq!(found, vec![(2, vec![0, 1, 2]), (2, vec![3]), (4, vec![4])]);
    }

    #[test]
    fn the_area_under_the_pointer_is_the_nearest_glow_reaching_it() {
        let spots = [(egui::pos2(0.0, 0.0), 10.0, 2), (egui::pos2(15.0, 0.0), 10.0, 4)];
        assert_eq!(area_at(egui::pos2(6.0, 0.0), &spots, 1.0), Some(2));
        assert_eq!(area_at(egui::pos2(9.0, 0.0), &spots, 1.0), Some(4));
        assert_eq!(area_at(egui::pos2(0.0, 30.0), &spots, 1.0), None);
    }
}
