//! The 3D tile of Inspect and Area Search (`crate::object_stage`): the objects
//! in a photo studio, turned and moved by dragging or with the buttons. What
//! the pointer is on is outlined and named. It never moves by itself.

use crate::object_stage::{Absent, Frame, Stage, StageModel, Stance, Standing, mixed};
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_render::preview::{Highlight, Pose, Studio, View};
use egui::{Color32, PointerButton, Rect, RichText, Sense, Vec2};
use glam::Mat4;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Height of the bar of controls under the view.
const BAR: f32 = 34.0;
const BUTTON: f32 = 26.0;
/// Smallest tile with its controls.
const WITH_BAR: f32 = 150.0;
/// Seconds between two looks at what the pointer is over.
const POINTED: f64 = 0.05;
/// The camera as it starts.
const CAMERA: Camera = Camera {
    yaw: 0.45,
    pitch: 0.10,
    zoom: 1.0,
    pan: [0.0, 0.0],
};
/// How far the objects can be moved, in pictures (zoomed in as far as it goes).
const FAR: f32 = 12.0;
const NEAREST: f32 = 12.0;
const FURTHEST: f32 = 0.4;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Camera {
    yaw: f32,
    pitch: f32,
    zoom: f32,
    /// In widths and heights of the picture.
    pan: [f32; 2],
}

/// What the view shows of the objects.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Showing {
    /// The object alone in the view.
    pub alone: Option<Uuid>,
    /// The prim outlined in the world.
    pub outlined: Option<Uuid>,
    /// The object the pointer is on elsewhere in the window.
    pub pointed: Option<Uuid>,
}

/// What was clicked in the view, or is pointed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub root: Uuid,
    pub prim: Uuid,
}

/// The view of one window.
pub struct StageUi {
    pub stage: Stage,
    /// How what is worn stands, when there is a choice.
    pub stance: Stance,
    /// On a light ground.
    pub light: bool,
    /// What is shown is worn: there is a choice.
    pub worn: bool,
    /// The avatar's joints, while it is shown as it stands in the world.
    live: Option<Vec<Mat4>>,
    /// The bones drawn last, and what they frame.
    bones: Arc<[Mat4]>,
    steady: Option<Frame>,
    /// How far the item alone has come to the middle (0 to 1).
    near: f32,
    camera: Camera,
    /// The object alone, kept while the view goes back to them all.
    alone: Option<Uuid>,
    /// The last one shown alone: another starts in the middle.
    framed: Option<Uuid>,
    /// What the pose shown was made for.
    posed: Option<(u64, Standing, Frame, Arc<[Mat4]>, Pose)>,
    classes: Option<(u64, Classes, Arc<[u8]>)>,
    over: Option<Hit>,
    /// What is outlined as pointed at, kept while it fades out.
    fading: Option<Pointed>,
    looked: f64,
}

/// What is pointed at: an object, or a prim of the one alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pointed {
    Object(Uuid),
    Prim(Uuid),
}

/// What the outlines depend on.
type Classes = (Option<Pointed>, Option<Uuid>);

impl Default for StageUi {
    fn default() -> Self {
        StageUi {
            stage: Stage::default(),
            stance: Stance::default(),
            light: false,
            worn: false,
            live: None,
            bones: Arc::from([]),
            steady: None,
            near: 0.0,
            camera: CAMERA,
            alone: None,
            framed: None,
            posed: None,
            classes: None,
            over: None,
            fading: None,
            looked: 0.0,
        }
    }
}

impl StageUi {
    /// Other objects: the view starts over.
    pub fn restart(&mut self) {
        self.camera = CAMERA;
        self.alone = None;
        self.framed = None;
        self.over = None;
        self.fading = None;
        self.posed = None;
        self.classes = None;
        self.live = None;
        self.steady = None;
    }

    /// The avatar's joints are wanted: it shows as it stands in the world, all
    /// of it.
    pub fn wants_joints(&self) -> bool {
        self.worn && self.stance == Stance::Live && (self.framed.is_none() || self.near < 1.0)
    }

    /// The avatar's joints this frame; None: it has no pose.
    pub fn set_joints(&mut self, joints: Option<Vec<Mat4>>) {
        self.live = joints;
    }

    /// What the pointer is on in the view.
    pub fn over(&self) -> Option<Hit> {
        self.over
    }

    /// Draw the view over `rect`, its controls when there is room. `top`: the
    /// room the caller writes in over the view; `name`: what an object or a
    /// prim is called. Returns the click.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        rect: Rect,
        top: f32,
        showing: Showing,
        spoken: &str,
        name: &dyn Fn(Hit) -> Option<String>,
    ) -> Option<Hit> {
        let model = match self.stage.model() {
            Ok(model) => model.clone(),
            Err(absent) => {
                if rect.height() >= WITH_BAR {
                    notice(ui, p, rect, absent);
                }
                return None;
            }
        };
        let controls = rect.height() >= WITH_BAR;
        // the studio fills the tile; what it shows stands between the title
        // and the bar
        let view_rect = rect.shrink(1.0);
        let free = if controls {
            Rect::from_min_max(
                egui::pos2(view_rect.left(), view_rect.top() + top),
                egui::pos2(view_rect.right(), view_rect.bottom() - BAR - 8.0),
            )
        } else {
            view_rect
        };
        let id = ui.scope_id().with("object_stage");
        let resp = ui.interact(view_rect, id, Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, spoken));
        let middle = (free.center() - view_rect.center()) / view_rect.size().max(Vec2::splat(1.0));
        self.steer(ui, &resp, view_rect, middle);
        // one alone comes to the middle, and goes back the same way
        if showing.alone.is_some() {
            self.alone = showing.alone;
        }
        if self.framed != showing.alone {
            self.framed = showing.alone;
            self.camera.pan = CAMERA.pan;
            self.camera.zoom = CAMERA.zoom;
        }
        let quick = ui.style().animation_time > 0.0;
        let time = if quick { 0.22 } else { 0.0 };
        let near = ui.ctx().animate_bool_with_time(id.with("alone"), showing.alone.is_some(), time);
        self.near = near;
        let group = self.alone.and_then(|root| model.groups.iter().position(|g| g.root == root));
        let standing = Standing {
            focus: group.filter(|_| near > 0.0).map(|g| (g, near)),
        };
        let frame = self.stand(ui, id, &model, standing, near);
        let view = self.view(p, &frame, view_rect, free, middle);
        self.look(ui, &resp, &model, &frame, &view, view_rect, standing);
        let pose = self.pose(&model, standing, frame);
        let highlight = self.highlight(ui, p, id, &model, showing);
        aurora_render::preview::paint_stage(ui, id, model.model.clone(), view, view_rect, Some(pose), highlight);
        let hit = resp.clicked().then(|| resp.interact_pointer_pos()).flatten().and_then(|at| {
            let (from, dir) = view.ray(&model.model, view_rect, at);
            let unit = model.pick(standing, &frame, &self.bones, from, dir)?;
            Some(Hit {
                root: model.groups[unit.group].root,
                prim: unit.id,
            })
        });
        if let Some(over) = self.over.filter(|_| controls)
            && let Some(at) = resp.hover_pos()
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            if let Some(text) = name(over) {
                tag(ui, p, view_rect, at, text.trim());
            }
        }
        if controls {
            let bar = Rect::from_min_max(
                egui::pos2(rect.left() + 8.0, rect.bottom() - BAR - 8.0),
                egui::pos2(rect.right() - 8.0, rect.bottom() - 8.0),
            );
            self.bar(ui, p, bar);
        }
        hit
    }

    /// Drag to turn, right or middle drag (or Shift) to move, wheel to come
    /// closer; the arrows do the same with the keyboard.
    fn steer(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: Rect, middle: Vec2) {
        let camera = &mut self.camera;
        let shift = ui.input(|i| i.modifiers.shift);
        let d = resp.drag_delta();
        let moving = resp.dragged_by(PointerButton::Secondary) || resp.dragged_by(PointerButton::Middle) || (shift && resp.dragged());
        if moving {
            camera.pan[0] += d.x / rect.width();
            camera.pan[1] += d.y / rect.height();
            ui.ctx().set_cursor_icon(egui::CursorIcon::Move);
        } else if resp.dragged_by(PointerButton::Primary) {
            camera.yaw -= d.x * 0.01;
            camera.pitch += d.y * 0.01;
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
        if resp.hovered() {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if wheel != 0.0 {
                ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
                // what is under the pointer stays under it
                let at = resp.hover_pos().unwrap_or(rect.center());
                camera.zoom_at((wheel * 0.002).exp(), (at - rect.center()) / rect.size(), middle);
            }
        }
        if resp.has_focus() {
            ui.memory_mut(|m| {
                m.set_focus_lock_filter(
                    resp.id,
                    egui::EventFilter {
                        horizontal_arrows: true,
                        vertical_arrows: true,
                        ..Default::default()
                    },
                );
            });
            let pressed = |key| ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, key) || i.consume_key(egui::Modifiers::NONE, key));
            let [left, right, up, down] = [
                egui::Key::ArrowLeft,
                egui::Key::ArrowRight,
                egui::Key::ArrowUp,
                egui::Key::ArrowDown,
            ]
            .map(|key| f32::from(u8::from(pressed(key))));
            if shift {
                camera.pan[0] += (right - left) * 0.06;
                camera.pan[1] += (down - up) * 0.06;
            } else {
                camera.yaw += (left - right) * 0.2;
                camera.pitch += (up - down) * 0.15;
            }
        }
        camera.pitch = camera.pitch.clamp(-1.2, 1.4);
        camera.pan = camera.pan.map(|v| v.clamp(-FAR, FAR));
    }

    /// The camera over `rect`, its objects in the middle of `free`.
    fn view(&self, p: &Palette, frame: &Frame, rect: Rect, free: Rect, middle: Vec2) -> View {
        let height = rect.height().max(1.0);
        // all of it in sight, whichever side is the shorter
        let room = (height / free.height().max(1.0)).max(height / free.width().max(1.0));
        View {
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            distance: 2.7 * room / self.camera.zoom,
            pan: [self.camera.pan[0] + middle.x, self.camera.pan[1] + middle.y],
            background: shown(self.ground(p)),
            studio: Some(Studio {
                floor: Some(frame.floor()),
            }),
            ..Default::default()
        }
    }

    /// What the pointer is over, looked at some times a second.
    #[allow(clippy::too_many_arguments)]
    fn look(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        model: &StageModel,
        frame: &Frame,
        view: &View,
        rect: Rect,
        standing: Standing,
    ) {
        let now = ui.input(|i| i.time);
        match resp.hover_pos().filter(|_| !resp.dragged()) {
            Some(at) if now - self.looked > POINTED => {
                self.looked = now;
                let (from, dir) = view.ray(&model.model, rect, at);
                self.over = model.pick(standing, frame, &self.bones, from, dir).map(|unit| Hit {
                    root: model.groups[unit.group].root,
                    prim: unit.id,
                });
            }
            Some(_) => {}
            None => self.over = None,
        }
    }

    /// The bones of this frame, and what they frame. As the avatar stands in
    /// the world while all of it is shown; still while one item is alone.
    fn stand(&mut self, ui: &egui::Ui, id: egui::Id, model: &StageModel, standing: Standing, near: f32) -> Frame {
        let still = model.still(self.stance == Stance::Relaxed);
        let quick = ui.style().animation_time > 0.0;
        let live = self.worn && self.stance == Stance::Live && self.live.is_some();
        let time = if quick { 0.25 } else { 0.0 };
        let mix = ui.ctx().animate_bool_with_time(id.with("live"), live, time) * (1.0 - near);
        let bones = match &self.live {
            Some(joints) if mix > 0.0 => mixed(still, &model.bones(joints), mix),
            _ => still.clone(),
        };
        if *self.bones != *bones {
            self.bones = bones;
        }
        let Some(joints) = self.live.as_ref().filter(|_| mix > 0.0) else {
            self.steady = None;
            return model.frame(standing.focus, still);
        };
        let asked = model.live_frame(joints);
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        // (a body that leaves its place is followed without a jolt)
        let steady = match self.steady {
            Some(steady) => steady.mix(asked, 1.0 - (-dt * 2.5).exp()),
            None => asked,
        };
        self.steady = Some(steady);
        ui.ctx().request_repaint();
        // the still frame, or the item's, as far as it comes to it
        match standing.focus {
            Some((group, near)) => steady.mix(model.frame(Some((group, 1.0)), still), near),
            None => model.frame(None, still).mix(steady, mix),
        }
    }

    fn pose(&mut self, model: &Arc<StageModel>, standing: Standing, frame: Frame) -> Pose {
        let at = self.stage.serial();
        if let Some((m, s, f, b, pose)) = &self.posed
            && (*m, *s, *f) == (at, standing, frame)
            && Arc::ptr_eq(b, &self.bones)
        {
            return pose.clone();
        }
        let pose = model.pose(standing, &frame, &self.bones);
        self.posed = Some((at, standing, frame, self.bones.clone(), pose.clone()));
        pose
    }

    /// The ground, and what is written over it.
    pub fn ground(&self, p: &Palette) -> Color32 {
        if self.light { light_ground(p) } else { ground_color(p) }
    }

    /// Text that reads over the ground.
    pub fn ink(&self, p: &Palette) -> Color32 {
        if self.light { dark(p) } else { p.ink }
    }

    /// Quieter text.
    pub fn muted(&self, p: &Palette) -> Color32 {
        if self.light { dark(p).gamma_multiply(0.78) } else { p.muted }
    }

    /// The face of a button over a light ground.
    pub fn face(&self, p: &Palette) -> Option<Color32> {
        self.light.then(|| p.bar.to_opaque())
    }

    /// What the pointer is on is outlined in light, fading in and out; the
    /// prim outlined in the world takes the accent.
    fn highlight(&mut self, ui: &egui::Ui, p: &Palette, id: egui::Id, model: &StageModel, showing: Showing) -> Option<Highlight> {
        let pointed = match (showing.alone, self.over) {
            (Some(_), Some(over)) => Some(Pointed::Prim(over.prim)),
            (Some(_), None) => None,
            (None, over) => showing.pointed.or(over.map(|o| o.root)).map(Pointed::Object),
        };
        // an object of one prim is all there is to see
        let single = showing.alone.is_some_and(|root| model.parts_of(&root) < 2);
        let pointed = pointed.filter(|_| !single);
        if pointed.is_some() {
            self.fading = pointed;
        }
        let time = if ui.style().animation_time > 0.0 { 0.14 } else { 0.0 };
        let strength = ui.ctx().animate_bool_with_time(id.with("pointed"), pointed.is_some(), time);
        if strength == 0.0 {
            self.fading = None;
        }
        // an object selected whole has its root prim outlined in the world:
        // only a prim picked among others is worth the accent here
        let selected = showing
            .outlined
            .filter(|prim| showing.alone.is_some_and(|root| root != *prim) && !single);
        if self.fading.is_none() && selected.is_none() {
            return None;
        }
        let key: Classes = (self.fading, selected);
        let at = self.stage.serial();
        let classes = match &self.classes {
            Some((m, k, classes)) if (*m, *k) == (at, key) => classes.clone(),
            _ => {
                let classes = model.classes(|unit| {
                    let root = model.groups[unit.group].root;
                    if selected == Some(unit.id) {
                        2
                    } else {
                        u8::from(match self.fading {
                            Some(Pointed::Object(id)) => root == id,
                            Some(Pointed::Prim(id)) => unit.id == id,
                            None => false,
                        })
                    }
                });
                self.classes = Some((at, key, classes.clone()));
                classes
            }
        };
        // (in steps: the picture is drawn again for each)
        let strength = (strength * 12.0).round() / 12.0;
        let color = |c: Color32, a: f32| {
            let [r, g, b, _] = c.to_normalized_gamma_f32();
            [r, g, b, a]
        };
        Some(Highlight {
            classes,
            colors: [
                color(self.ink(p), strength * 0.8),
                color(if self.light { p.violet } else { p.violet_light }, 0.9),
            ],
        })
    }

    /// The controls: turn, look from higher or lower, nearer or further, and
    /// back to the start; what dragging does, written at their right.
    fn bar(&mut self, ui: &mut egui::Ui, p: &Palette, rect: Rect) {
        let radius = ui.visuals().window_corner_radius;
        let face = if self.light {
            p.bar.to_opaque()
        } else {
            p.bar.gamma_multiply(0.78)
        };
        ui.painter().rect_filled(rect, radius, face);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("object_stage_bar")
                .max_rect(rect.shrink2(Vec2::new(4.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = 2.0;
        let camera = &mut self.camera;
        if tool(ui, p, "rotate", aurora_i18n::tr!("stage-turn-left")) {
            camera.yaw += 0.5;
        }
        if tool(ui, p, "rotate-clockwise", aurora_i18n::tr!("stage-turn-right")) {
            camera.yaw -= 0.5;
        }
        if tool(ui, p, "arrow-up", aurora_i18n::tr!("stage-higher")) {
            camera.pitch = (camera.pitch + 0.25).min(1.4);
        }
        if tool(ui, p, "arrow-down", aurora_i18n::tr!("stage-lower")) {
            camera.pitch = (camera.pitch - 0.25).max(-1.2);
        }
        if tool(ui, p, "plus", aurora_i18n::tr!("stage-nearer")) {
            camera.zoom_at(1.25, Vec2::ZERO, Vec2::ZERO);
        }
        if tool(ui, p, "minus", aurora_i18n::tr!("stage-further")) {
            camera.zoom_at(0.8, Vec2::ZERO, Vec2::ZERO);
        }
        if *camera != CAMERA && tool(ui, p, "restore", aurora_i18n::tr!("stage-reset")) {
            *camera = CAMERA;
        }
        let tip = if self.light {
            aurora_i18n::tr!("stage-dark")
        } else {
            aurora_i18n::tr!("stage-light")
        };
        if tool_lit(ui, p, "bulb-filled", tip, self.light) {
            self.light = !self.light;
        }
        if self.worn {
            self.stances(ui, p, rect);
            return;
        }
        let hint = aurora_i18n::tr!("stage-hint");
        let room = ui.available_width() - 8.0;
        let font = egui::FontId::proportional(11.0);
        let width = ui.painter().layout_no_wrap(hint.to_owned(), font.clone(), p.muted).size().x;
        if width < room {
            let at = egui::pos2(rect.right() - 8.0 - width, rect.center().y);
            widgets::paint_elided(ui, hint, at, width + 1.0, font, p.muted);
        }
    }

    /// How what is worn stands: a switch of three little figures at the right
    /// of the bar, the chosen one named.
    fn stances(&mut self, ui: &mut egui::Ui, p: &Palette, rect: Rect) {
        const ICON: f32 = 30.0;
        const HEIGHT: f32 = 26.0;
        const INSET: f32 = 2.0;
        let font = egui::FontId::new(12.0, super::fonts::semibold(ui.ctx()));
        let names = Stance::ALL.map(stance_name);
        let wide = |name: &str| ui.painter().layout_no_wrap(name.to_owned(), font.clone(), p.ink).size().x + 8.0;
        let longest = names.iter().map(|n| wide(n)).fold(0.0, f32::max);
        let room = ui.available_width() - 4.0;
        let small = ICON * 3.0 + 2.0 * INSET;
        // the name where there is room for it, the figures alone otherwise
        let named = if small + longest <= room { longest } else { 0.0 };
        let track = Rect::from_min_size(
            egui::pos2(rect.right() - 4.0 - small - named, rect.center().y - HEIGHT / 2.0 - INSET),
            Vec2::new(small + named, HEIGHT + 2.0 * INSET),
        );
        let radius = ui.visuals().widgets.inactive.corner_radius;
        let edge = egui::Stroke::new(1.0, p.muted.gamma_multiply(0.55));
        ui.painter()
            .rect(track, radius, p.field.to_opaque(), edge, egui::StrokeKind::Inside);
        let id = ui.scope_id().with("object_stage_stance");
        let at = Stance::ALL.iter().position(|s| *s == self.stance).unwrap_or(0);
        let time = if ui.style().animation_time > 0.0 { 0.16 } else { 0.0 };
        // how open each segment is, and where the lit face stands
        let open: [f32; 3] = std::array::from_fn(|i| ui.ctx().animate_bool_with_time(id.with(("open", i)), i == at, time));
        let total: f32 = open.iter().sum::<f32>().max(0.001);
        let mut x = track.left() + INSET;
        let segments: [Rect; 3] = std::array::from_fn(|i| {
            let width = ICON + named * open[i] / total;
            let r = Rect::from_min_size(egui::pos2(x, track.top() + INSET), Vec2::new(width, HEIGHT));
            x += width;
            r
        });
        let left = ui
            .ctx()
            .animate_value_with_time(id.with("left"), segments[at].left() - track.left(), time);
        let right = ui
            .ctx()
            .animate_value_with_time(id.with("right"), segments[at].right() - track.left(), time);
        let lit = Rect::from_x_y_ranges(track.left() + left..=track.left() + right, segments[at].y_range());
        ui.painter().rect_filled(lit, radius, p.violet);
        for (i, stance) in Stance::ALL.into_iter().enumerate() {
            let r = segments[i];
            let resp = ui.interact(r, id.with(i), Sense::click());
            let chosen = i == at;
            if !chosen && resp.hovered() {
                ui.painter().rect_filled(r, radius, p.raised);
            }
            if i > 0 && i != at && i != at + 1 {
                let y = r.center().y;
                ui.painter()
                    .vline(r.left(), y - 6.0..=y + 6.0, egui::Stroke::new(1.0, p.muted.gamma_multiply(0.35)));
            }
            let ink = match (chosen, resp.hovered()) {
                (true, _) => p.on_accent,
                (false, true) => p.ink,
                (false, false) => p.muted,
            };
            // the figure of AO moves while it is the one chosen
            let moving = chosen && stance == Stance::Live && time > 0.0 && self.live.is_some();
            let sway = if moving { (ui.input(|s| s.time) * 2.6).sin() as f32 } else { 0.0 };
            let center = egui::pos2(r.left() + ICON / 2.0, r.center().y);
            figure(ui, center, stance, sway, ink);
            if chosen && named > 0.0 && open[i] > 0.6 {
                // in the middle of the room the longest name takes
                let text = (wide(names[i]) - 8.0).min(named);
                let left = r.left() + ICON - 2.0 + (named - 8.0 - text).max(0.0) / 2.0;
                let width = (r.right() - left).max(1.0);
                let ink = ink.gamma_multiply((open[i] - 0.6) / 0.4);
                widgets::paint_elided(ui, names[i], egui::pos2(left, r.center().y), width, font.clone(), ink);
            }
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, chosen, names[i]));
            if resp.on_hover_text(stance_tip(stance)).clicked() {
                self.stance = stance;
            }
        }
    }
}

/// A little person standing as `stance`, in a square of sixteen points.
fn figure(ui: &egui::Ui, center: egui::Pos2, stance: Stance, sway: f32, ink: Color32) {
    let at = |x: f32, y: f32| center + Vec2::new(x, y);
    let stroke = egui::Stroke::new(1.7, ink);
    let painter = ui.painter();
    let limb = |points: &[egui::Pos2]| {
        for pair in points.windows(2) {
            painter.line_segment([pair[0], pair[1]], stroke);
        }
        for p in points {
            painter.circle_filled(*p, stroke.width / 2.0, ink);
        }
    };
    let lean = if stance == Stance::Live { sway * 0.7 } else { 0.0 };
    painter.circle_filled(at(lean, -6.2), 2.2, ink);
    let (neck, hip) = (at(lean * 0.6, -3.0), at(0.0, 1.8));
    limb(&[neck, hip]);
    match stance {
        Stance::Wide => {
            limb(&[at(-6.6, -2.4), at(6.6, -2.4)]);
            limb(&[at(-2.2, 7.6), hip, at(2.2, 7.6)]);
        }
        Stance::Relaxed => {
            limb(&[at(-4.6, 3.0), neck, at(4.6, 3.0)]);
            limb(&[at(-2.6, 7.6), hip, at(2.6, 7.6)]);
        }
        Stance::Live => {
            // one arm up, a step
            limb(&[neck, at(3.6, -1.4 - sway * 0.6), at(5.6, -5.0 - sway)]);
            limb(&[neck, at(-3.4, -0.2 + sway * 0.4), at(-4.6, 3.0 + sway * 0.6)]);
            limb(&[hip, at(2.6, 4.2), at(1.6, 7.6)]);
            limb(&[hip, at(-1.8, 4.8), at(-4.0, 7.2)]);
        }
    }
}

fn stance_name(stance: Stance) -> &'static str {
    match stance {
        Stance::Live => aurora_i18n::tr!("stage-stance-live"),
        Stance::Relaxed => aurora_i18n::tr!("stage-stance-relaxed"),
        Stance::Wide => aurora_i18n::tr!("stage-stance-wide"),
    }
}

impl Camera {
    /// Closer or further by `ratio`, what is at `at` staying there (from the
    /// picture's middle, in widths and heights); `middle` is where the objects
    /// stand.
    fn zoom_at(&mut self, ratio: f32, at: Vec2, middle: Vec2) {
        let zoom = (self.zoom * ratio).clamp(FURTHEST, NEAREST);
        let ratio = zoom / self.zoom;
        self.zoom = zoom;
        for (pan, (at, middle)) in self.pan.iter_mut().zip([(at.x, middle.x), (at.y, middle.y)]) {
            let from = at - middle;
            *pan = from - (from - *pan) * ratio;
        }
    }
}

fn stance_tip(stance: Stance) -> &'static str {
    match stance {
        Stance::Live => aurora_i18n::tr!("stage-stance-live-tip"),
        Stance::Relaxed => aurora_i18n::tr!("stage-stance-relaxed-tip"),
        Stance::Wide => aurora_i18n::tr!("stage-stance-wide-tip"),
    }
}

/// The darker and the lighter of the theme's text and ground: a light ground
/// and its text, in any theme.
fn dark(p: &Palette) -> Color32 {
    let (a, b) = (p.ink.to_opaque(), p.bar.to_opaque());
    if a.intensity() < b.intensity() { a } else { b }
}

fn light_ground(p: &Palette) -> Color32 {
    let (a, b) = (p.ink.to_opaque(), p.bar.to_opaque());
    let light = if a.intensity() < b.intensity() { b } else { a };
    // (the studio's glow takes its middle to white)
    light.lerp_to_gamma(dark(p), 0.1)
}

/// The name of what the pointer is on, beside it.
fn tag(ui: &egui::Ui, p: &Palette, rect: Rect, at: egui::Pos2, text: &str) {
    if text.is_empty() {
        return;
    }
    let font = egui::FontId::new(12.0, super::fonts::semibold(ui.ctx()));
    let wide = ui.painter().layout_no_wrap(text.to_owned(), font.clone(), p.ink).size();
    let size = Vec2::new(wide.x.min(rect.width() * 0.7) + 16.0, wide.y + 9.0);
    let min = egui::pos2(
        (at.x + 14.0).min(rect.right() - size.x - 4.0).max(rect.left() + 4.0),
        (at.y + 16.0).min(rect.bottom() - size.y - 4.0),
    );
    let r = Rect::from_min_size(min, size);
    let radius = ui.visuals().widgets.inactive.corner_radius;
    let stroke = egui::Stroke::new(1.0, p.muted.gamma_multiply(0.5));
    ui.painter().rect(r, radius, p.panel.to_opaque(), stroke, egui::StrokeKind::Inside);
    widgets::paint_elided(ui, text, r.left_center() + Vec2::new(8.0, 0.0), size.x - 14.0, font, p.ink);
}

/// One button of the bar.
fn tool(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str) -> bool {
    tool_lit(ui, p, icon, tip, false)
}

fn tool_lit(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, on: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    let ink = match (on, resp.hovered()) {
        (true, _) => p.warn,
        (false, true) => p.ink,
        (false, false) => p.muted,
    };
    super::sheet::icon_at(ui, icon, rect.center(), 16.0, ink);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, tip));
    resp.on_hover_text(tip).clicked()
}

/// Why there is nothing to see.
fn notice(ui: &mut egui::Ui, p: &Palette, rect: Rect, absent: Absent) {
    let text = match absent {
        Absent::Loading => {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
            aurora_i18n::tr!("stage-loading")
        }
        Absent::Nothing => aurora_i18n::tr!("stage-nothing"),
        Absent::Refused => aurora_i18n::tr!("stage-refused"),
    };
    let inner = Rect::from_center_size(rect.center(), Vec2::new(rect.width() - 40.0, 40.0));
    ui.put(inner, egui::Label::new(RichText::new(text).size(12.5).color(p.muted)).wrap());
}

/// The ground of the view, opaque: the previews are drawn on it.
pub fn ground_color(p: &Palette) -> Color32 {
    p.bar.to_opaque()
}

/// The view's target is not sRGB: its ground is given as it shows.
fn shown(c: Color32) -> [f32; 4] {
    c.to_normalized_gamma_f32()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the pointer is on stays under it, in and out, off the middle too.
    #[test]
    fn zoom_keeps_what_is_under_the_pointer() {
        let middle = Vec2::new(0.0, 0.05);
        let at = Vec2::new(0.3, -0.2);
        let mut camera = Camera {
            pan: [0.1, 0.02],
            ..CAMERA
        };
        // a point of the model, as far from its centre as the zoom is strong
        let place = |c: &Camera, p: Vec2| Vec2::new(c.pan[0], c.pan[1]) + middle + p * c.zoom;
        let point = (at - middle - Vec2::new(camera.pan[0], camera.pan[1])) / camera.zoom;
        for ratio in [1.3, 2.0, 0.4, 100.0, 0.001] {
            camera.zoom_at(ratio, at, middle);
            assert!((place(&camera, point) - at).length() < 1e-3, "{ratio}: {camera:?}");
            assert!((FURTHEST..=NEAREST).contains(&camera.zoom));
        }
    }
}
