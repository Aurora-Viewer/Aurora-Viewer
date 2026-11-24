//! The 3D tile of Inspect and Area Search (`crate::object_stage`): the objects
//! in a photo studio, turned and moved by dragging or with the buttons. What
//! the pointer is on is outlined and named. It never moves by itself.

use crate::object_stage::{Absent, Stage, StageModel, Standing, build_standing};
use crate::theme::Palette;
use crate::ui::widgets;
use aurora_render::preview::{Highlight, Pose, Studio, View};
use egui::{Color32, PointerButton, Rect, RichText, Sense, Vec2};
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
/// How far the objects can be moved, in pictures.
const FAR: f32 = 2.0;

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
    camera: Camera,
    /// The object alone, kept while the view goes back to them all.
    alone: Option<Uuid>,
    /// The last one shown alone: another starts in the middle.
    framed: Option<Uuid>,
    /// What the pose shown was made for.
    posed: Option<(u64, build_standing::Key, Pose)>,
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
        self.steer(ui, &resp, view_rect);
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
        let group = self.alone.and_then(|root| model.groups.iter().position(|g| g.root == root));
        let standing = Standing {
            focus: group.filter(|_| near > 0.0).map(|g| (g, near)),
        };
        let view = self.view(p, &model, standing, view_rect, free);
        self.look(ui, &resp, &model, &view, view_rect, standing);
        let pose = self.pose(&model, standing);
        let highlight = self.highlight(ui, p, id, &model, showing);
        aurora_render::preview::paint_stage(ui, id, model.model.clone(), view, view_rect, Some(pose), highlight);
        let hit = resp.clicked().then(|| resp.interact_pointer_pos()).flatten().and_then(|at| {
            let (from, dir) = view.ray(&model.model, view_rect, at);
            let unit = model.pick(standing, from, dir)?;
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
    fn steer(&mut self, ui: &egui::Ui, resp: &egui::Response, rect: Rect) {
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
                camera.zoom *= (wheel * 0.002).exp();
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
        camera.zoom = camera.zoom.clamp(0.4, 12.0);
        camera.pan = camera.pan.map(|v| v.clamp(-FAR, FAR));
    }

    /// The camera over `rect`, its objects in the middle of `free`.
    fn view(&self, p: &Palette, model: &StageModel, standing: Standing, rect: Rect, free: Rect) -> View {
        let height = rect.height().max(1.0);
        // all of it in sight, whichever side is the shorter
        let room = (height / free.height().max(1.0)).max(height / free.width().max(1.0));
        let middle = (free.center() - rect.center()) / rect.size().max(Vec2::splat(1.0));
        View {
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            distance: 2.7 * room / self.camera.zoom,
            pan: [self.camera.pan[0] + middle.x, self.camera.pan[1] + middle.y],
            background: shown(ground_color(p)),
            studio: Some(Studio {
                floor: Some(model.floor(standing)),
            }),
            ..Default::default()
        }
    }

    /// What the pointer is over, looked at some times a second.
    fn look(&mut self, ui: &egui::Ui, resp: &egui::Response, model: &StageModel, view: &View, rect: Rect, standing: Standing) {
        let now = ui.input(|i| i.time);
        match resp.hover_pos().filter(|_| !resp.dragged()) {
            Some(at) if now - self.looked > POINTED => {
                self.looked = now;
                let (from, dir) = view.ray(&model.model, rect, at);
                self.over = model.pick(standing, from, dir).map(|unit| Hit {
                    root: model.groups[unit.group].root,
                    prim: unit.id,
                });
            }
            Some(_) => {}
            None => self.over = None,
        }
    }

    fn pose(&mut self, model: &Arc<StageModel>, standing: Standing) -> Pose {
        let (at, key) = (self.stage.serial(), build_standing::key(standing));
        if let Some((m, k, pose)) = &self.posed
            && (*m, *k) == (at, key)
        {
            return pose.clone();
        }
        let pose = model.pose(standing);
        self.posed = Some((at, key, pose.clone()));
        pose
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
            colors: [color(p.ink, strength), color(p.violet_light, 1.0)],
        })
    }

    /// The controls: turn, look from higher or lower, nearer or further, and
    /// back to the start; what dragging does, written at their right.
    fn bar(&mut self, ui: &mut egui::Ui, p: &Palette, rect: Rect) {
        let radius = ui.visuals().window_corner_radius;
        ui.painter().rect_filled(rect, radius, p.bar.gamma_multiply(0.78));
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
            camera.zoom = (camera.zoom * 1.25).min(12.0);
        }
        if tool(ui, p, "minus", aurora_i18n::tr!("stage-further")) {
            camera.zoom = (camera.zoom / 1.25).max(0.4);
        }
        if *camera != CAMERA && tool(ui, p, "restore", aurora_i18n::tr!("stage-reset")) {
            *camera = CAMERA;
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
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, p.raised);
    }
    let ink = if resp.hovered() { p.ink } else { p.muted };
    super::sheet::icon_at(ui, icon, rect.center(), 16.0, ink);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
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
