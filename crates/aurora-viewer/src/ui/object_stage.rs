//! The 3D tile of Inspect and Area Search (`crate::object_stage`): the objects
//! on a quiet ground, turned by dragging or with its buttons, pulled apart
//! with its slider. It never moves by itself.

use crate::object_stage::{Absent, Stage, StageModel, build_standing};
use crate::theme::Palette;
use aurora_render::preview::{Pose, Tones, View};
use egui::{Color32, Rect, RichText, Sense, Vec2};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Height of the bar of controls under the view.
const BAR: f32 = 34.0;
const BUTTON: f32 = 26.0;
/// Smallest tile with its controls.
const WITH_BAR: f32 = 150.0;
/// Seconds between two looks at what the pointer is over.
const POINTED: f64 = 0.06;

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

/// What was clicked in the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub root: Uuid,
    pub prim: Uuid,
}

/// The view of one window.
pub struct StageUi {
    pub stage: Stage,
    yaw: f32,
    pitch: f32,
    zoom: f32,
    apart: f32,
    /// The object alone, kept while the view goes back to them all.
    alone: Option<Uuid>,
    /// What the pose and the tones shown were made for.
    posed: Option<(u64, build_standing::Key, Pose)>,
    toned: Option<(u64, Showing, Option<Uuid>, Tones)>,
    over: Option<Uuid>,
    looked: f64,
}

impl Default for StageUi {
    fn default() -> Self {
        StageUi {
            stage: Stage::default(),
            yaw: 0.45,
            pitch: 0.12,
            zoom: 1.0,
            apart: 0.0,
            alone: None,
            posed: None,
            toned: None,
            over: None,
            looked: 0.0,
        }
    }
}

impl StageUi {
    /// Other objects: the view starts over, its camera as it was left.
    pub fn restart(&mut self) {
        self.apart = 0.0;
        self.alone = None;
        self.over = None;
        self.posed = None;
        self.toned = None;
    }

    /// How far the objects are pulled apart (0 to 1).
    pub fn pull_apart(&mut self, apart: f32) {
        self.apart = apart;
    }

    /// Draw the view over `rect`, its controls when there is room. `top`: what
    /// the caller writes over the top of the view. Returns the click.
    pub fn show(&mut self, ui: &mut egui::Ui, p: &Palette, rect: Rect, top: f32, showing: Showing, spoken: &str) -> Option<Hit> {
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
        let ground = rect.shrink(1.0);
        let view_rect = if controls {
            Rect::from_min_max(
                egui::pos2(ground.left(), ground.top() + top),
                egui::pos2(ground.right(), ground.bottom() - BAR - 8.0),
            )
        } else {
            ground
        };
        let id = ui.scope_id().with("object_stage");
        let resp = ui.interact(view_rect, id, Sense::click_and_drag());
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, spoken));
        if resp.dragged() {
            let d = resp.drag_delta();
            self.yaw -= d.x * 0.01;
            self.pitch = (self.pitch + d.y * 0.01).clamp(-1.2, 1.4);
        }
        if resp.hovered() {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if wheel != 0.0 {
                ui.input_mut(|i| i.smooth_scroll_delta = Vec2::ZERO);
                self.zoom = (self.zoom * (wheel * 0.002).exp()).clamp(0.4, 6.0);
            }
        }
        if resp.double_clicked() {
            (self.yaw, self.pitch, self.zoom) = (0.45, 0.12, 1.0);
        }
        // one alone comes to the middle, and goes back the same way
        if showing.alone.is_some() {
            self.alone = showing.alone;
        }
        let quick = ui.style().animation_time > 0.0;
        let time = if quick { 0.22 } else { 0.0 };
        let near = ui.ctx().animate_bool_with_time(id.with("alone"), showing.alone.is_some(), time);
        let apart = ui.ctx().animate_value_with_time(id.with("apart"), self.apart, time * 0.6);
        let group = self.alone.and_then(|root| model.groups.iter().position(|g| g.root == root));
        let standing = crate::object_stage::Standing {
            apart,
            focus: group.filter(|_| near > 0.0).map(|g| (g, near)),
        };
        let view = View {
            yaw: self.yaw,
            pitch: self.pitch,
            distance: 2.75 / self.zoom,
            background: shown(ground_color(p)),
            ambient: [0.42; 3],
            ..Default::default()
        };
        self.pointed(ui, &resp, &model, &view, view_rect, standing);
        let pose = self.pose(&model, standing);
        let tones = self.tones(p, &model, showing);
        aurora_render::preview::paint_stage(ui, id, model.model.clone(), view, view_rect, Some(pose), Some(tones));
        let hit = resp.clicked().then(|| resp.interact_pointer_pos()).flatten().and_then(|at| {
            let (from, dir) = view.ray(&model.model, view_rect, at);
            let unit = model.pick(standing, from, dir)?;
            Some(Hit {
                root: model.groups[unit.group].root,
                prim: unit.id,
            })
        });
        if controls {
            let bar = Rect::from_min_max(
                egui::pos2(rect.left() + 8.0, rect.bottom() - BAR - 8.0),
                egui::pos2(rect.right() - 8.0, rect.bottom() - 8.0),
            );
            let parts = match showing.alone {
                Some(root) => model.parts_of(&root),
                None => model.shown_groups(),
            };
            self.bar(ui, p, bar, parts > 1);
        }
        hit
    }

    /// What the pointer is over, looked at a few times a second.
    fn pointed(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        model: &StageModel,
        view: &View,
        rect: Rect,
        standing: crate::object_stage::Standing,
    ) {
        let now = ui.input(|i| i.time);
        match resp.hover_pos().filter(|_| !resp.dragged()) {
            Some(at) if now - self.looked > POINTED => {
                self.looked = now;
                let (from, dir) = view.ray(&model.model, rect, at);
                self.over = model.pick(standing, from, dir).map(|unit| model.groups[unit.group].root);
            }
            Some(_) => {}
            None => self.over = None,
        }
    }

    fn pose(&mut self, model: &Arc<StageModel>, standing: crate::object_stage::Standing) -> Pose {
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

    /// The prim outlined in the world takes the accent; pointing at an object
    /// dims the others.
    fn tones(&mut self, p: &Palette, model: &Arc<StageModel>, showing: Showing) -> Tones {
        let at = self.stage.serial();
        let pointed = showing.pointed.or(self.over).filter(|_| showing.alone.is_none());
        if let Some((m, s, o, tones)) = &self.toned
            && (*m, *s, *o) == (at, showing, pointed)
        {
            return tones.clone();
        }
        let mix = |c: Color32, amount: f32| {
            let [r, g, b, _] = c.to_normalized_gamma_f32();
            [r, g, b, amount]
        };
        let (accent, ground) = (mix(p.violet, 0.6), mix(ground_color(p), 0.62));
        let tones = model.tones(|unit| {
            let root = model.groups[unit.group].root;
            // an object selected whole has its root prim outlined: only a
            // prim picked among others is worth the accent here
            if showing.outlined == Some(unit.id) && showing.alone == Some(root) && unit.id != root {
                accent
            } else if pointed.is_some_and(|r| r != root) {
                ground
            } else {
                [0.0; 4]
            }
        });
        self.toned = Some((at, showing, pointed, tones.clone()));
        tones
    }

    /// The controls: turn, look from higher or lower, nearer or further, pull
    /// apart. Every movement of the mouse has its button.
    fn bar(&mut self, ui: &mut egui::Ui, p: &Palette, rect: Rect, can_part: bool) {
        let radius = ui.visuals().window_corner_radius;
        ui.painter().rect_filled(rect, radius, p.bar.gamma_multiply(0.92));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt("object_stage_bar")
                .max_rect(rect.shrink2(Vec2::new(4.0, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let ui = &mut child;
        ui.spacing_mut().item_spacing.x = 2.0;
        if tool(ui, p, "rotate", aurora_i18n::tr!("stage-turn-left")) {
            self.yaw += 0.5;
        }
        if tool(ui, p, "rotate-clockwise", aurora_i18n::tr!("stage-turn-right")) {
            self.yaw -= 0.5;
        }
        if tool(ui, p, "arrow-up", aurora_i18n::tr!("stage-higher")) {
            self.pitch = (self.pitch + 0.25).min(1.4);
        }
        if tool(ui, p, "arrow-down", aurora_i18n::tr!("stage-lower")) {
            self.pitch = (self.pitch - 0.25).max(-1.2);
        }
        if tool(ui, p, "plus", aurora_i18n::tr!("stage-nearer")) {
            self.zoom = (self.zoom * 1.25).min(6.0);
        }
        if tool(ui, p, "minus", aurora_i18n::tr!("stage-further")) {
            self.zoom = (self.zoom / 1.25).max(0.4);
        }
        ui.add_space(6.0);
        if can_part && ui.available_width() > 90.0 {
            let label = aurora_i18n::tr!("stage-apart");
            ui.label(RichText::new(label).size(12.0).color(p.ink));
            ui.add_space(4.0);
            ui.spacing_mut().slider_width = (ui.available_width() - 10.0).max(40.0);
            let mut percent = (self.apart * 100.0).round();
            let slider = egui::Slider::new(&mut percent, 0.0..=100.0).show_value(false);
            if ui.add(slider).on_hover_text(aurora_i18n::tr!("stage-apart-tip")).changed() {
                self.apart = percent / 100.0;
            }
        } else if !can_part {
            self.apart = 0.0;
        }
    }
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
