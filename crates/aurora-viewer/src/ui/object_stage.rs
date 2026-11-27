//! The 3D tile of Inspect and Area Search (`crate::object_stage`): the objects
//! in a photo studio, turned and moved by dragging or with the buttons. What
//! the pointer is on is outlined and named. It never moves by itself.

use crate::object_stage::{Absent, Frame, Stage, StageModel, Standing, Steady, mixed};
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
    /// On a light ground.
    pub light: bool,
    /// What is shown is worn: it stands as its wearer does.
    pub worn: bool,
    /// One item is inspected: it does not move.
    pub single: bool,
    /// What the pointer is on is outlined, named and clicked (Inspect, Area
    /// Search); off, the view only turns (the AO window's preview).
    pub picking: bool,
    /// The bar of buttons on a face of its own; off, on the view's ground
    /// (the AO window's preview, whose caption lies on a fade there).
    pub bar_face: bool,
    /// The animation plays; None: it was not chosen (it does, unless motion is
    /// reduced).
    playing: Option<bool>,
    /// The joints of the wearer and of their animesh, as last seen and as
    /// shown.
    live: Option<Vec<Mat4>>,
    joints: Option<Vec<Mat4>>,
    /// How fast the shown ones follow (0 paused, 1 playing).
    speed: f32,
    /// The animesh the joints shown are for.
    own: Vec<Uuid>,
    /// When the animation was last paused or played.
    toggled: Option<f64>,
    /// The bones drawn last, and what they frame.
    bones: Arc<[Mat4]>,
    steady: Option<Steady>,
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
            light: false,
            worn: false,
            single: false,
            picking: true,
            bar_face: true,
            playing: None,
            live: None,
            joints: None,
            speed: 0.0,
            own: Vec::new(),
            toggled: None,
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
        self.joints = None;
        self.steady = None;
        self.playing = None;
        self.toggled = None;
    }

    /// Hold the animation.
    pub fn pause(&mut self) {
        self.playing = Some(false);
    }

    /// What is worn moves in the view.
    pub fn moves(&self) -> bool {
        self.worn && !self.single && self.speed > 0.0
    }

    /// The wearer's joints are wanted this frame: while they move in the view,
    /// and once for what does not move.
    pub fn wants_joints(&self) -> bool {
        self.worn && (self.live.is_none() || (!self.single && (self.speed > 0.0 || self.framed.is_none())))
    }

    /// The wearer's joints and their animesh's (`shown_joints`); None: not
    /// asked for, or they have no pose.
    pub fn set_joints(&mut self, joints: Option<Vec<Mat4>>) {
        if joints.is_some() {
            self.live = joints;
        }
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
        let frame = self.stand(ui, id, &model, standing);
        let view = self.view(p, &frame, view_rect, free, middle);
        if self.picking {
            self.look(ui, &resp, &model, &frame, &view, view_rect, standing);
        } else {
            self.over = None;
        }
        let pose = self.pose(&model, standing, frame);
        let highlight = if self.picking {
            self.highlight(ui, p, id, &model, showing)
        } else {
            None
        };
        aurora_render::preview::paint_stage(ui, id, model.model.clone(), view, view_rect, Some(pose), highlight);
        let clicked = resp.clicked() && self.picking;
        let hit = clicked.then(|| resp.interact_pointer_pos()).flatten().and_then(|at| {
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
        if controls && self.animated() {
            self.badge(ui, p, free);
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

    /// The bones of this frame, and what they frame. What is worn stands as its
    /// wearer does, easing to a stop when paused or when an item is alone, and
    /// off again.
    fn stand(&mut self, ui: &egui::Ui, id: egui::Id, model: &StageModel, standing: Standing) -> Frame {
        let quick = ui.style().animation_time > 0.0;
        let playing = self.plays(quick) && self.framed.is_none() && !self.single;
        let dt = ui.input(|i| i.stable_dt).clamp(0.001, 0.1);
        let time = match (quick, playing) {
            (false, _) => 0.0,
            (true, true) => 0.5,
            (true, false) => 0.32,
        };
        self.speed = ui.ctx().animate_bool_with_time(id.with("speed"), playing, time);
        // other animesh: theirs are taken again
        if !model.own().map(|(root, _)| root).eq(&self.own) {
            self.own = model.own().map(|(root, _)| *root).collect();
            self.live = None;
            if let Some(joints) = &mut self.joints {
                joints.truncate(model.wearer_joints());
            }
        }
        if let Some(live) = &self.live {
            let shown = match &self.joints {
                Some(joints) if self.speed < 1.0 => {
                    // the same at any frame rate
                    let follow = 1.0 - (1.0 - self.speed * self.speed).powf(dt * 60.0);
                    if follow > 0.0 {
                        mixed(joints, live, follow)
                    } else {
                        joints.clone()
                    }
                }
                _ => live.clone(),
            };
            self.joints = Some(shown);
        }
        let bones = match &self.joints {
            Some(joints) if self.worn => model.bones(joints),
            _ => model.still().clone(),
        };
        if *self.bones != *bones {
            self.bones = bones;
        }
        if self.speed > 0.0 {
            ui.ctx().request_repaint();
        }
        let anchored = match &self.joints {
            Some(joints) if self.worn && !self.single => {
                let (bounds, body) = (model.bounds(&self.bones), model.body(joints));
                let steady = match &mut self.steady {
                    Some(steady) if !steady.outgrown(&bounds) || self.framed.is_some() => {
                        steady.follow(&bounds, body, 1.0 - (-dt * 0.9).exp());
                        *steady
                    }
                    steady => *steady.insert(Steady::of(&bounds, body)),
                };
                steady.frame
            }
            _ => model.frame(None, &self.bones),
        };
        match standing.focus {
            Some((group, near)) => anchored.mix(model.frame(Some((group, 1.0)), &self.bones), near),
            None => anchored,
        }
    }

    /// The animation plays, as far as the user says.
    fn plays(&self, quick: bool) -> bool {
        self.playing.unwrap_or(quick)
    }

    fn toggle(&mut self, ui: &egui::Ui) {
        let quick = ui.style().animation_time > 0.0;
        self.playing = Some(!self.plays(quick));
        self.toggled = Some(ui.input(|i| i.time));
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
        if self.bar_face {
            let radius = ui.visuals().window_corner_radius;
            let face = if self.light {
                p.bar.to_opaque()
            } else {
                p.bar.gamma_multiply(0.78)
            };
            ui.painter().rect_filled(rect, radius, face);
        }
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
        if self.animated() {
            self.player(ui, p, rect);
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

    /// There is an animation to pause.
    fn animated(&self) -> bool {
        self.worn && !self.single && self.live.is_some()
    }

    /// Play / pause, at the right of the bar: the button, lit from within while
    /// the animation plays, and what it does now.
    fn player(&mut self, ui: &mut egui::Ui, p: &Palette, rect: Rect) {
        const GAP: f32 = 9.0;
        let font = egui::FontId::new(12.0, super::fonts::semibold(ui.ctx()));
        let (on, off) = (aurora_i18n::tr!("stage-playing"), aurora_i18n::tr!("stage-paused"));
        let wide = |text: &str| ui.painter().layout_no_wrap(text.to_owned(), font.clone(), p.ink).size().x;
        let word = wide(on).max(wide(off)).ceil();
        // as far from the bar's end as the first button is from its start
        let inset = (rect.height() - BUTTON) / 2.0;
        let face = Rect::from_center_size(
            egui::pos2(rect.right() - inset - BUTTON / 2.0, rect.center().y),
            Vec2::splat(BUTTON),
        );
        let word = if BUTTON + GAP + word + 4.0 <= ui.available_width() {
            word
        } else {
            0.0
        };
        let left = if word > 0.0 { face.left() - GAP - word - 4.0 } else { face.left() };
        let all = face.with_min_x(left);
        let quick = ui.style().animation_time > 0.0;
        let id = ui.scope_id().with("object_stage_player");
        let resp = ui.interact(all, id, Sense::click());
        if resp.clicked() {
            self.toggle(ui);
        }
        let playing = self.plays(quick);
        let since = self
            .toggled
            .map(|at| (ui.input(|i| i.time) - at) as f32)
            .filter(|t| quick && *t < 0.9);
        if since.is_some() {
            ui.ctx().request_repaint();
        }
        let radius = ui.visuals().widgets.inactive.corner_radius;
        let time = |t: f32| if quick { t } else { 0.0 };
        let hover = ui.ctx().animate_bool_with_time(id.with("hover"), resp.hovered(), time(0.12));
        let down = ui
            .ctx()
            .animate_bool_with_time(id.with("down"), resp.is_pointer_button_down_on(), time(0.07));
        let lit = ui.ctx().animate_bool_with_time(id.with("lit"), playing, time(0.28));
        // pressed in, then back past its size and to rest
        let spring = since.map_or(0.0, |t| (-8.0 * t).exp() * (20.0 * t).sin() * 0.12);
        let scale = 1.0 - 0.10 * down + spring;
        let body = Rect::from_center_size(face.center(), face.size() * scale);
        // its light: a slow breath while it plays, a flare as it is pressed,
        // nothing when paused
        let breath = if quick && playing && self.speed > 0.0 {
            0.5 + 0.5 * (ui.input(|i| i.time) * 1.6).sin() as f32
        } else {
            0.5
        };
        let flare = since.map_or(0.0, |t| (-5.0 * t).exp());
        let glow = lit * (0.16 + 0.10 * breath + 0.10 * hover) + 0.35 * flare;
        if glow > 0.01 {
            let light = egui::Shadow {
                offset: [0, 0],
                blur: 12,
                spread: 1,
                color: p.violet_light.gamma_multiply(glow.min(1.0)),
            };
            ui.painter().add(light.as_shape(body, radius));
        }
        let rest = p.raised.to_opaque().lerp_to_gamma(p.ink, 0.06 * hover);
        let playing_face = p.violet.lerp_to_gamma(p.violet_light, 0.06 * breath + 0.16 * hover);
        let fill = rest
            .lerp_to_gamma(playing_face, lit)
            .lerp_to_gamma(p.violet_light, 0.35 * flare * lit);
        ui.painter().rect_filled(body, radius, fill);
        let ink = p.ink.lerp_to_gamma(p.on_accent, lit);
        play_pause(ui, body.center(), 10.0 * scale, lit, ink);
        if word > 0.0 {
            let right = face.left() - GAP;
            let painter = ui.painter().with_clip_rect(all.intersect(ui.clip_rect()));
            // one word rises as the other leaves
            for (text, shown, from) in [(on, lit, 7.0), (off, 1.0 - lit, -7.0)] {
                if shown > 0.02 {
                    let ink = p.muted.lerp_to_gamma(p.ink, hover).gamma_multiply(shown);
                    let galley = painter.layout_no_wrap(text.to_owned(), font.clone(), ink);
                    let at = egui::pos2(right - galley.size().x, rect.center().y + (1.0 - shown) * from);
                    widgets::paint_ink_left(&painter, galley, at, ink);
                }
            }
        }
        let tip = if playing {
            aurora_i18n::tr!("stage-pause-tip")
        } else {
            aurora_i18n::tr!("stage-play-tip")
        };
        resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, playing, tip));
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        resp.on_hover_text(tip);
    }

    /// The badge that shows over the view as the animation is paused or played.
    fn badge(&self, ui: &egui::Ui, p: &Palette, rect: Rect) {
        let quick = ui.style().animation_time > 0.0;
        let Some(t) = self
            .toggled
            .map(|at| (ui.input(|i| i.time) - at) as f32)
            .filter(|t| quick && *t < 0.55)
        else {
            return;
        };
        ui.ctx().request_repaint();
        let gone = t / 0.55;
        let rise = 1.0 - (1.0 - (gone * 2.2).min(1.0)).powi(3);
        let fade = if gone < 0.25 { gone / 0.25 } else { 1.0 - (gone - 0.25) / 0.75 };
        let side = 52.0 * (0.78 + 0.26 * rise);
        let r = Rect::from_center_size(rect.center(), Vec2::splat(side));
        let radius = ui.visuals().window_corner_radius;
        ui.painter().rect_filled(r, radius, p.panel.to_opaque().gamma_multiply(0.72 * fade));
        // what it does now: the sign of what was asked
        let playing = if self.plays(quick) { 0.0 } else { 1.0 };
        play_pause(ui, r.center(), side * 0.42, playing, p.ink.gamma_multiply(fade));
    }
}

/// The sign of the button: `t` 1 the two bars of pause, 0 the triangle of
/// play, one becoming the other in between.
fn play_pause(ui: &egui::Ui, center: egui::Pos2, size: f32, t: f32, ink: Color32) {
    if t <= 0.0 {
        let points = [(-0.38, -0.5), (0.5, 0.0), (-0.38, 0.5)].map(|(x, y)| center + Vec2::new(x, y) * size);
        ui.painter()
            .add(egui::Shape::convex_polygon(points.to_vec(), ink, egui::Stroke::NONE));
        return;
    }
    // each half of the triangle becomes a bar
    let play = [
        [(-0.38, -0.5), (0.06, -0.25), (0.06, 0.25), (-0.38, 0.5)],
        [(0.06, -0.25), (0.5, 0.0), (0.5, 0.0), (0.06, 0.25)],
    ];
    let pause = [
        [(-0.42, -0.48), (-0.12, -0.48), (-0.12, 0.48), (-0.42, 0.48)],
        [(0.12, -0.48), (0.42, -0.48), (0.42, 0.48), (0.12, 0.48)],
    ];
    // eased at both ends
    let t = t * t * (3.0 - 2.0 * t);
    for (a, b) in play.into_iter().zip(pause) {
        let points: Vec<egui::Pos2> = a
            .into_iter()
            .zip(b)
            .map(|((ax, ay), (bx, by))| center + Vec2::new(ax + (bx - ax) * t, ay + (by - ay) * t) * size)
            .collect();
        ui.painter().add(egui::Shape::convex_polygon(points, ink, egui::Stroke::NONE));
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
