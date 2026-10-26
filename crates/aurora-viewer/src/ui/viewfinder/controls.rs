//! The viewfinder's on-screen controls, for the people who would rather
//! not learn the shortcuts (which stay): the lens ring (the value chosen in
//! the bottom bar as a ruler dragged under a fixed index), the AF mode and
//! area as choices, the camera pad (turn, slide, a spring-loaded zoom
//! rocker, the tilt) and the self-timer's countdown. The bottom bar's
//! values turn like the ring (`turn_dial`).

use super::{Dial, Frame, GAP, Out, Viewfinder, bold, icon, regular, shadow_text};
use crate::dof::PhotoFocus;
use crate::snapshot::viewfinder::{self as vf, AfArea, OnScreen};
use crate::snapshot::{Settings, focal_of_fov, fov_of_focal};
use crate::theme::Palette;
use crate::ui::snapshot::SnapshotUi;
use crate::ui::widgets;
use egui::{Align2, Color32, Id, Order, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};

/// The ruler of the lens ring.
const RULER: Vec2 = Vec2::new(600.0, 46.0);
/// A stick of the pad, and how far its puck goes.
const STICK: f32 = 96.0;
const STICK_TRAVEL: f32 = 28.0;
/// The focal lengths − and + go to.
const FOCAL_STOPS: [f32; 6] = [10.0, 24.0, 35.0, 50.0, 85.0, 135.0];

/// What the camera pad drives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CamMode {
    /// Around the avatar or the point aimed at.
    Orbit,
    /// The free camera.
    Free,
    /// From the avatar's eyes (mouselook).
    First,
}

/// The pad's sticks and rocker this frame, each from −1 to 1 (right and
/// down positive), back to 0 when let go.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Pad {
    pub orbit: Vec2,
    pub pan: Vec2,
    /// The zoom rocker: up (negative) zooms in.
    pub rocker: f32,
}

/// The ruler of a value: where it is (`u`), points per unit, its range,
/// its marks (position, label, major).
struct Ring {
    u: f32,
    k: f32,
    min: f32,
    max: f32,
    marks: Vec<(f32, String, bool)>,
    title: &'static str,
    keys: &'static str,
}

/// A mark's label: one decimal under 10 when it has one.
fn mark_label(v: f32) -> String {
    let d = if v < 10.0 && v.fract() > 0.01 { 1 } else { 0 };
    aurora_i18n::number(f64::from(v), d)
}

impl Viewfinder {
    /// The on-screen controls show (the preference, and in Hover the
    /// pointer near the bottom of the screen or the pad).
    pub(super) fn controls_shown(&mut self, ctx: &egui::Context, s: &Settings, screen: Rect, now: f64) -> bool {
        match s.viewfinder.controls {
            OnScreen::Always => true,
            OnScreen::Hidden => false,
            OnScreen::Hover => {
                let near = ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| {
                    p.y > screen.bottom() - 260.0 || (s.viewfinder.pad && p.x < screen.left() + 360.0 && p.y > screen.bottom() - 360.0)
                });
                if near || self.ring_drag.is_some() || self.pad != Pad::default() {
                    self.hover_seen = now;
                }
                // a moment after the pointer leaves
                now - self.hover_seen < 1.2
            }
        }
    }

    fn ring(&self, dial: Dial, s: &Settings, panel: &SnapshotUi, fov: f32, f: &Frame) -> Ring {
        let ln = f32::ln;
        match dial {
            Dial::Aperture => {
                let n = s.dof.f_number;
                // past f/22, one more stop: all sharp (the depth of field off)
                let all_sharp = vf::STOPS.len() as f32;
                let u = if s.dof.enabled {
                    vf::STOPS
                        .iter()
                        .enumerate()
                        .min_by(|a, b| (a.1 - n).abs().total_cmp(&(b.1 - n).abs()))
                        .map_or(0, |(i, _)| i) as f32
                } else {
                    all_sharp
                };
                let majors = [1.4, 2.0, 2.8, 4.0, 5.6, 8.0, 11.0, 16.0, 22.0];
                let mut marks: Vec<(f32, String, bool)> = vf::STOPS
                    .iter()
                    .enumerate()
                    .map(|(i, &v)| (i as f32, mark_label(v), majors.contains(&v)))
                    .collect();
                marks.push((all_sharp, aurora_i18n::tr!("photo-vf-ring-all-sharp").to_owned(), true));
                Ring {
                    u,
                    k: 46.0,
                    min: 0.0,
                    max: all_sharp,
                    marks,
                    title: aurora_i18n::tr!("photo-vf-ring-aperture"),
                    keys: aurora_i18n::tr!("photo-vf-ring-aperture-keys"),
                }
            }
            Dial::Exposure => {
                let ev = vf::ev_of(panel.light.exposure.unwrap_or(f.base_exposure), f.base_exposure);
                Ring {
                    u: ev * 3.0,
                    k: 30.0,
                    min: -6.0,
                    max: 6.0,
                    marks: (-6..=6)
                        .map(|t: i32| {
                            let label = if t % 3 == 0 {
                                match t.signum() {
                                    1 => format!("+{}", t / 3),
                                    -1 => format!("−{}", -t / 3),
                                    _ => "0".to_owned(),
                                }
                            } else {
                                String::new()
                            };
                            (t as f32, label, t % 3 == 0)
                        })
                        .collect(),
                    title: aurora_i18n::tr!("photo-vf-ring-exposure"),
                    keys: aurora_i18n::tr!("photo-vf-ring-exposure-keys"),
                }
            }
            Dial::Distance => {
                let d = self.af.distance().clamp(vf::MIN_FOCUS, 150.0);
                let majors = [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 150.0];
                Ring {
                    u: ln(d),
                    k: 120.0,
                    min: ln(vf::MIN_FOCUS),
                    max: ln(150.0),
                    marks: [0.3, 0.5, 0.7, 1.0, 1.5, 2.0, 3.0, 5.0, 7.0, 10.0, 15.0, 20.0, 30.0, 50.0, 150.0]
                        .into_iter()
                        .map(|v: f32| (ln(v), if v >= 150.0 { "∞".to_owned() } else { mark_label(v) }, majors.contains(&v)))
                        .collect(),
                    title: aurora_i18n::tr!("photo-vf-ring-focus"),
                    keys: aurora_i18n::tr!("photo-vf-ring-focus-keys"),
                }
            }
            Dial::Focal => {
                let mm = focal_of_fov(fov).clamp(10.0, 135.0);
                let labelled = [10.0, 14.0, 18.0, 24.0, 35.0, 50.0, 70.0, 85.0, 135.0];
                Ring {
                    u: ln(mm),
                    k: 231.0,
                    min: ln(10.0),
                    max: ln(135.0),
                    marks: [
                        10.0, 12.0, 14.0, 16.0, 18.0, 21.0, 24.0, 28.0, 35.0, 40.0, 50.0, 60.0, 70.0, 85.0, 100.0, 120.0, 135.0,
                    ]
                    .into_iter()
                    .map(|v: f32| {
                        let label = if labelled.contains(&v) { format!("{v:.0}") } else { String::new() };
                        (ln(v), label, FOCAL_STOPS[1..].contains(&v))
                    })
                    .collect(),
                    title: aurora_i18n::tr!("photo-vf-ring-focal"),
                    keys: aurora_i18n::tr!("photo-vf-ring-focal-keys"),
                }
            }
        }
    }

    /// The value at `u` on the ring of `dial`.
    #[allow(clippy::too_many_arguments)]
    fn ring_set(&mut self, dial: Dial, u: f32, s: &mut Settings, panel: &mut SnapshotUi, fov: &mut f32, f: &Frame) {
        match dial {
            Dial::Focal => {
                let mm = u.exp().round().clamp(10.0, 135.0);
                *fov = fov_of_focal(mm);
            }
            Dial::Aperture => {
                let i = u.round() as usize;
                if i >= vf::STOPS.len() {
                    // the last stop: all sharp
                    if s.dof.enabled {
                        s.dof.enabled = false;
                        self.say(aurora_i18n::tr!("photo-vf-dof-off"), false, f.now);
                    }
                } else {
                    s.dof.f_number = vf::STOPS[i];
                    // an aperture means a depth of field
                    if !s.dof.enabled {
                        s.dof.enabled = true;
                        self.say(aurora_i18n::tr!("photo-vf-dof-on"), false, f.now);
                    }
                }
            }
            Dial::Exposure => {
                let ev = u.round() / 3.0;
                panel.light.exposure = (ev.abs() > 0.01).then(|| vf::exposure_at(ev, f.base_exposure));
            }
            Dial::Distance => {
                if s.dof.focus != PhotoFocus::Manual {
                    // direct manual focus, from where the lens is
                    s.dof.focus = PhotoFocus::Manual;
                    self.say(aurora_i18n::tr!("photo-vf-direct-manual"), false, f.now);
                }
                let d = u.exp();
                s.dof.distance = if d >= vf::INFINITY { 200.0 } else { d.max(vf::MIN_FOCUS) };
            }
        }
    }

    /// − or + on the ring: the next stop (the presets for the focal length).
    #[allow(clippy::too_many_arguments)]
    fn ring_step(&mut self, dial: Dial, dir: i32, s: &mut Settings, panel: &mut SnapshotUi, fov: &mut f32, f: &Frame, out: &mut Vec<Out>) {
        let r = self.ring(dial, s, panel, *fov, f);
        if dial == Dial::Focal {
            let mm = focal_of_fov(*fov).round();
            let next = if dir > 0 {
                FOCAL_STOPS.iter().copied().find(|&v| v > mm + 0.5)
            } else {
                FOCAL_STOPS.iter().rev().copied().find(|&v| v < mm - 0.5)
            };
            if let Some(v) = next {
                *fov = fov_of_focal(v);
                out.push(Out::Detent);
            }
            return;
        }
        let step = if dial == Dial::Distance { 0.16 } else { 1.0 };
        let u = (r.u + dir as f32 * step).clamp(r.min, r.max);
        if (u - r.u).abs() > 1e-4 {
            self.ring_set(dial, u, s, panel, fov, f);
            out.push(Out::Detent);
        }
    }

    /// A value turned by the pointer over `resp`, the lens ring or a value
    /// of the bottom bar, pressed and dragged at once (`sign` −1 for the
    /// ruler, which moves under its index; 1 for the bar's value, right for
    /// more), or the wheel over it a stop a notch; a click at each mark
    /// crossed. The ring keeps its value: only a click on the bar's value
    /// gives it another (`dial_input`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn turn_dial(
        &mut self,
        ui: &egui::Ui,
        resp: &egui::Response,
        dial: Dial,
        sign: f32,
        s: &mut Settings,
        panel: &mut SnapshotUi,
        fov: &mut f32,
        f: &Frame,
        out: &mut Vec<Out>,
    ) {
        let r = self.ring(dial, s, panel, *fov, f);
        let x = resp.interact_pointer_pos().map(|p| p.x);
        if resp.drag_started()
            && let Some(x) = x
        {
            self.ring_drag = Some((r.u, x));
            self.ring_mark = None;
        }
        if resp.dragged()
            && let (Some((u0, x0)), Some(x)) = (self.ring_drag, x)
        {
            let u = (u0 + sign * (x - x0) / r.k).clamp(r.min, r.max);
            self.ring_set(dial, u, s, panel, fov, f);
            let mark = r.marks.iter().filter(|m| m.0 <= u + 1e-4).count() as i32;
            if self.ring_mark.is_some_and(|m| m != mark) {
                out.push(Out::Detent);
            }
            self.ring_mark = Some(mark);
        } else if resp.hovered() {
            let dy = wheel_notches(ui);
            if dy != 0.0 {
                self.ring_step(dial, if dy > 0.0 { 1 } else { -1 }, s, panel, fov, f, out);
            }
        }
        if resp.drag_stopped() {
            self.ring_drag = None;
        }
        if resp.clicked() || resp.drag_stopped() {
            resp.surrender_focus();
        }
    }

    /// The lens ring above the bottom bar.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ring_deck(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        screen: Rect,
        s: &mut Settings,
        panel: &mut SnapshotUi,
        fov: &mut f32,
        f: &Frame,
        out: &mut Vec<Out>,
    ) {
        let dial = self.dial;
        egui::Area::new(Id::unique("vf_ring"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(Pos2::new(screen.center().x, screen.bottom() - 70.0))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing = Vec2::new(10.0, 4.0);
                let r = self.ring(dial, s, panel, *fov, f);
                // the title and its keys
                let (head, _) = ui.allocate_exact_size(Vec2::new(RULER.x + 2.0 * 42.0, 14.0), Sense::hover());
                shadow_text(
                    ui.painter(),
                    p,
                    Pos2::new(head.left() + 42.0, head.center().y),
                    Align2::LEFT_CENTER,
                    r.title,
                    bold(ctx, 11.0),
                    p.ink,
                );
                shadow_text(
                    ui.painter(),
                    p,
                    Pos2::new(head.right() - 42.0, head.center().y),
                    Align2::RIGHT_CENTER,
                    r.keys,
                    regular(11.0),
                    p.muted,
                );
                ui.horizontal(|ui| {
                    let minus = aurora_i18n::tr!("photo-vf-ring-less");
                    if step_button(ui, p, "minus", minus) {
                        self.ring_step(dial, -1, s, panel, fov, f, out);
                    }
                    // the ruler moves under the index: dragged right, the
                    // value comes down
                    let (rect, resp) = ui.allocate_exact_size(RULER, Sense::click_and_drag());
                    self.turn_dial(ui, &resp, dial, -1.0, s, panel, fov, f, out);
                    let _ = resp.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
                    // the ruler drawn from the value after this frame's change
                    let r = self.ring(dial, s, panel, *fov, f);
                    paint_ruler(ui.painter(), ctx, p, rect, &r, dial, s, f, self.af.distance());
                    let plus = aurora_i18n::tr!("photo-vf-ring-more");
                    if step_button(ui, p, "plus", plus) {
                        self.ring_step(dial, 1, s, panel, fov, f, out);
                    }
                });
            });
    }

    /// The AF mode and area as choices, at the start of the bottom bar.
    pub(super) fn af_choices(&mut self, ctx: &egui::Context, p: &Palette, screen: Rect, s: &Settings, at: Pos2, out: &mut Vec<Out>) {
        egui::Area::new(Id::unique("vf_af_choices"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::LEFT_CENTER)
            .fixed_pos(at)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let modes = [PhotoFocus::Click, PhotoFocus::Pointer, PhotoFocus::Manual];
                    let items: Vec<(&str, bool, &str)> = modes
                        .iter()
                        .map(|m| {
                            let tip = match m {
                                PhotoFocus::Click => aurora_i18n::tr!("photo-vf-af-s-tip"),
                                PhotoFocus::Pointer => aurora_i18n::tr!("photo-vf-af-c-tip"),
                                PhotoFocus::Manual => aurora_i18n::tr!("photo-vf-mf-tip"),
                            };
                            (super::af_label(*m), s.dof.focus == *m, tip)
                        })
                        .collect();
                    if let Some(i) = segmented(ui, p, &items, None) {
                        out.push(Out::SetAf(modes[i]));
                    }
                    if s.dof.focus != PhotoFocus::Manual {
                        let areas = [AfArea::Avatar, AfArea::Point];
                        let items = [
                            (
                                aurora_i18n::tr!("photo-vf-area-avatar"),
                                s.viewfinder.area == AfArea::Avatar,
                                aurora_i18n::tr!("photo-vf-area-avatar-tip"),
                            ),
                            (
                                aurora_i18n::tr!("photo-vf-area-point"),
                                s.viewfinder.area == AfArea::Point,
                                aurora_i18n::tr!("photo-vf-area-point-tip"),
                            ),
                        ];
                        if let Some(i) = segmented(ui, p, &items, None) {
                            out.push(Out::SetArea(areas[i]));
                        }
                    }
                });
            });
        let _ = screen;
    }

    /// The camera pad, bottom left: what drives the camera, two sticks, the
    /// zoom rocker and the tilt.
    pub(super) fn camera_pad(&mut self, ctx: &egui::Context, p: &Palette, screen: Rect, f: &Frame, out: &mut Vec<Out>) {
        let mode = if f.first_person {
            CamMode::First
        } else if f.free_camera {
            CamMode::Free
        } else {
            CamMode::Orbit
        };
        let (look, slide, zoom) = match mode {
            CamMode::Orbit => (
                aurora_i18n::tr!("photo-vf-pad-turn"),
                aurora_i18n::tr!("photo-vf-pad-slide"),
                aurora_i18n::tr!("photo-vf-pad-zoom"),
            ),
            CamMode::Free => (
                aurora_i18n::tr!("photo-vf-pad-look"),
                aurora_i18n::tr!("photo-vf-pad-glide"),
                aurora_i18n::tr!("photo-vf-pad-forward"),
            ),
            CamMode::First => (
                aurora_i18n::tr!("photo-vf-pad-look"),
                aurora_i18n::tr!("photo-vf-pad-slide"),
                aurora_i18n::tr!("photo-vf-pad-zoom"),
            ),
        };
        let mut pad = Pad::default();
        egui::Area::new(Id::unique("vf_pad"))
            .order(Order::Foreground)
            .fade_in(false)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(Pos2::new(screen.left() + GAP, screen.bottom() - 84.0))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::same(12), |ui| {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(14.0, 10.0);
                        ui.vertical(|ui| {
                            ui.set_width(STICK * 2.0 + 14.0);
                            let modes = cam_modes(f.first_person_allowed, mode);
                            let items: Vec<(&str, bool, &str)> = modes
                                .iter()
                                .map(|m| match m {
                                    CamMode::Orbit => (
                                        aurora_i18n::tr!("photo-vf-cam-orbit"),
                                        mode == CamMode::Orbit,
                                        aurora_i18n::tr!("photo-vf-cam-orbit-tip"),
                                    ),
                                    CamMode::Free => (
                                        aurora_i18n::tr!("photo-vf-cam-free"),
                                        mode == CamMode::Free,
                                        aurora_i18n::tr!("photo-vf-cam-free-tip"),
                                    ),
                                    CamMode::First => (
                                        aurora_i18n::tr!("photo-vf-cam-first"),
                                        mode == CamMode::First,
                                        aurora_i18n::tr!("photo-vf-cam-first-tip"),
                                    ),
                                })
                                .collect();
                            if let Some(i) = segmented(ui, p, &items, Some(STICK * 2.0 + 14.0)) {
                                out.push(Out::Camera(modes[i]));
                            }
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 14.0;
                                ui.allocate_ui_with_layout(
                                    Vec2::new(STICK, STICK + 24.0),
                                    egui::Layout::top_down(egui::Align::Center),
                                    |ui| {
                                        ui.set_width(STICK);
                                        let tip = if mode == CamMode::Orbit {
                                            aurora_i18n::tr!("photo-vf-pad-turn-tip")
                                        } else {
                                            aurora_i18n::tr!("photo-vf-pad-look-tip")
                                        };
                                        pad.orbit = stick(ui, p, "vf_stick_orbit", "rotate-360", true, true, tip);
                                        label(ui, p, look);
                                    },
                                );
                                ui.allocate_ui_with_layout(
                                    Vec2::new(STICK, STICK + 24.0),
                                    egui::Layout::top_down(egui::Align::Center),
                                    |ui| {
                                        ui.set_width(STICK);
                                        let enabled = mode != CamMode::First;
                                        let tip = if mode == CamMode::Free {
                                            aurora_i18n::tr!("photo-vf-pad-glide-tip")
                                        } else {
                                            aurora_i18n::tr!("photo-vf-pad-slide-tip")
                                        };
                                        pad.pan = stick(ui, p, "vf_stick_pan", "arrows-move", false, enabled, tip);
                                        if !enabled {
                                            pad.pan = Vec2::ZERO;
                                        }
                                        label(ui, p, slide);
                                    },
                                );
                            });
                            // the tilt
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 8.0;
                                label(ui, p, aurora_i18n::tr!("photo-vf-pad-tilt"));
                                let value_w = 34.0;
                                let w = ui.available_width() - value_w - 8.0;
                                if let Some(roll) = tilt(ui, p, w, f.roll) {
                                    out.push(Out::Roll(roll));
                                }
                                let text = roll_text(f.roll);
                                let color = if f.roll == 0.0 { p.success } else { p.ink };
                                let (r, _) = ui.allocate_exact_size(Vec2::new(value_w, 18.0), Sense::hover());
                                ui.painter()
                                    .text(r.right_center(), Align2::RIGHT_CENTER, text, regular(11.5), color);
                            })
                            .response
                            .on_hover_text(aurora_i18n::tr!("photo-vf-pad-tilt-tip"));
                        });
                        // the zoom rocker, as tall as the rest
                        ui.allocate_ui_with_layout(Vec2::new(32.0, 200.0), egui::Layout::top_down(egui::Align::Center), |ui| {
                            ui.set_width(32.0);
                            ui.spacing_mut().item_spacing.y = 4.0;
                            let (in_tip, out_tip) = if mode == CamMode::Free {
                                (
                                    aurora_i18n::tr!("photo-vf-pad-forward-tip"),
                                    aurora_i18n::tr!("photo-vf-pad-back-tip"),
                                )
                            } else {
                                (aurora_i18n::tr!("photo-vf-pad-in-tip"), aurora_i18n::tr!("photo-vf-pad-out-tip"))
                            };
                            let up = hold_button(ui, p, "plus", in_tip);
                            let mut rocker = rocker(ui, p, aurora_i18n::tr!("photo-vf-pad-zoom-tip"));
                            let down = hold_button(ui, p, "minus", out_tip);
                            if up {
                                rocker = -0.6;
                            } else if down {
                                rocker = 0.6;
                            }
                            pad.rocker = rocker;
                            label(ui, p, zoom);
                        });
                    });
                });
            });
        if pad != self.pad {
            ctx.request_repaint();
        }
        self.pad = pad;
    }

    /// The self-timer's countdown, at the top.
    pub(super) fn countdown(&self, ctx: &egui::Context, p: &Palette, screen: Rect, now: f64) {
        let Some(end) = self.countdown else {
            return;
        };
        let left = ((end - now).max(0.0)).ceil() as i32;
        egui::Area::new(Id::unique("vf_countdown"))
            .order(Order::Foreground)
            .fade_in(false)
            .interactable(false)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(Pos2::new(screen.center().x, screen.top() + 96.0))
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::symmetric(18, 10), |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 12.0;
                        let (r, _) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::hover());
                        icon(ui.painter(), "clock-filled", r, p.amber);
                        ui.label(egui::RichText::new(left.to_string()).font(bold(ctx, 28.0)).color(p.amber));
                        ui.label(
                            egui::RichText::new(aurora_i18n::tr!("photo-vf-countdown", seconds = left))
                                .size(12.5)
                                .color(p.muted),
                        );
                    });
                });
            });
        ctx.request_repaint();
    }
}

/// The camera pad's choices: the first person view only when Preferences
/// allow it (Camera › Mouselook › first person view, EnableMouselook), or
/// while it is on.
pub fn cam_modes(first_person_allowed: bool, now: CamMode) -> Vec<CamMode> {
    let mut modes = vec![CamMode::Orbit, CamMode::Free];
    if first_person_allowed || now == CamMode::First {
        modes.push(CamMode::First);
    }
    modes
}

/// The wheel's notches this frame (each notch, not egui's smoothed scroll).
fn wheel_notches(ui: &egui::Ui) -> f32 {
    ui.input(|i| {
        i.raw
            .events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { delta, .. } => Some(delta.y),
                _ => None,
            })
            .sum()
    })
}

/// The tilt as written: "+6°", "−2,5°", "0°".
pub fn roll_text(roll: f32) -> String {
    let v = aurora_i18n::number(f64::from(roll.abs()), if roll.fract().abs() > 0.01 { 1 } else { 0 });
    match roll.partial_cmp(&0.0) {
        Some(std::cmp::Ordering::Greater) => format!("+{v}°"),
        Some(std::cmp::Ordering::Less) => format!("−{v}°"),
        _ => "0°".to_owned(),
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_ruler(
    painter: &egui::Painter,
    ctx: &egui::Context,
    p: &Palette,
    rect: Rect,
    r: &Ring,
    dial: Dial,
    s: &Settings,
    f: &Frame,
    distance: f32,
) {
    let radius = ctx.global_style().visuals.window_corner_radius;
    painter.rect_filled(rect, radius, p.navy.gamma_multiply(0.35));
    let line = rect.top() + 15.0;
    let cx = rect.center().x;
    // the ends fade, like a ring turning out of sight
    let fade = |x: f32| ((rect.width() * 0.5 - (x - cx).abs()) / 70.0).clamp(0.0, 1.0);
    painter.line_segment(
        [Pos2::new(rect.left() + 8.0, line), Pos2::new(rect.right() - 8.0, line)],
        Stroke::new(1.0, p.ink.gamma_multiply(0.45)),
    );
    // the zone kept sharp, on the focus ring
    if dial == Dial::Distance && s.dof.enabled {
        let at = |d: f32| cx + (d.clamp(vf::MIN_FOCUS, 150.0).ln() - r.u) * r.k;
        let (near, far) = f.sharp;
        let (a, b) = (at(near).max(rect.left()), at(far).min(rect.right()));
        if b > a {
            painter.rect_filled(
                Rect::from_min_max(Pos2::new(a, line - 3.0), Pos2::new(b.max(a + 3.0), line + 4.0)),
                widgets::inner_radius(&ctx.global_style().visuals),
                p.success.gamma_multiply(0.55),
            );
        }
        let _ = distance;
    }
    for (u, label, major) in &r.marks {
        let x = cx + (u - r.u) * r.k;
        let a = fade(x);
        if a <= 0.0 {
            continue;
        }
        let h = if *major {
            14.0
        } else if label.is_empty() {
            6.0
        } else {
            10.0
        };
        let color = if *major { p.ink } else { p.ink.gamma_multiply(0.6) };
        painter.line_segment(
            [Pos2::new(x, rect.top() + 8.0), Pos2::new(x, rect.top() + 8.0 + h)],
            Stroke::new(if *major { 1.5 } else { 1.0 }, color.gamma_multiply(a)),
        );
        if !label.is_empty() {
            let font = if *major { bold(ctx, 10.5) } else { regular(10.5) };
            let c = if *major { p.ink } else { p.muted };
            painter.text(
                Pos2::new(x, rect.top() + 25.0),
                Align2::CENTER_TOP,
                label,
                font,
                c.gamma_multiply(a),
            );
        }
    }
    // the fixed index
    painter.add(egui::Shape::convex_polygon(
        vec![
            Pos2::new(cx - 6.0, rect.top() + 1.0),
            Pos2::new(cx + 6.0, rect.top() + 1.0),
            Pos2::new(cx, rect.top() + 9.0),
        ],
        p.teal,
        Stroke::NONE,
    ));
    painter.line_segment(
        [Pos2::new(cx, rect.top() + 8.0), Pos2::new(cx, rect.bottom() - 6.0)],
        Stroke::new(2.0, p.teal),
    );
}

/// Choices side by side; the index clicked. `width`: share it equally.
pub(super) fn segmented(ui: &mut egui::Ui, p: &Palette, items: &[(&str, bool, &str)], width: Option<f32>) -> Option<usize> {
    let font = bold(ui.ctx(), 12.0);
    let pad = 2.0;
    let widths: Vec<f32> = items
        .iter()
        .map(|(t, _, _)| ui.painter().layout_no_wrap((*t).to_owned(), font.clone(), p.ink).size().x + 18.0)
        .collect();
    let total = width.unwrap_or(widths.iter().sum::<f32>() + 2.0 * (items.len() - 1) as f32 + 2.0 * pad);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, 26.0), Sense::hover());
    let radius = widgets::inner_radius(ui.visuals());
    ui.painter().rect_filled(rect, radius, p.navy.gamma_multiply(0.55));
    ui.painter()
        .rect_stroke(rect, radius, Stroke::new(1.0, p.ink.gamma_multiply(0.14)), StrokeKind::Inside);
    let inner = rect.shrink(pad);
    let even = (inner.width() - 2.0 * (items.len() - 1) as f32) / items.len() as f32;
    let mut x = inner.left();
    let mut clicked = None;
    for (i, (text, on, tip)) in items.iter().enumerate() {
        let w = if width.is_some() { even } else { widths[i] };
        let r = Rect::from_min_size(Pos2::new(x, inner.top()), Vec2::new(w, inner.height()));
        let resp = ui.interact(r, ui.scope_id().with(("vf_seg", i, *text)), Sense::click());
        if *on {
            ui.painter().rect_filled(r, radius, p.ink.gamma_multiply(0.16));
        } else if resp.hovered() {
            ui.painter().rect_filled(r, radius, p.ink.gamma_multiply(0.07));
        }
        let color = if *on || resp.hovered() { p.ink } else { p.muted };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *text, font.clone(), color);
        if resp.clicked() {
            resp.surrender_focus();
            clicked = Some(i);
        }
        let _ = resp.on_hover_text(*tip);
        x += w + 2.0;
    }
    clicked
}

fn label(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(egui::RichText::new(text).size(11.5).color(p.ink));
}

/// A 32-point button of the ring, in the middle of a cell as tall as the
/// ruler (laid first in the row, egui would leave it at the row's top);
/// true when clicked.
fn step_button(ui: &mut egui::Ui, p: &Palette, name: &str, tip: &str) -> bool {
    let (cell, _) = ui.allocate_exact_size(Vec2::new(32.0, RULER.y), Sense::hover());
    let rect = Rect::from_center_size(cell.center(), Vec2::splat(32.0));
    let resp = ui.interact(rect, ui.scope_id().with(("vf_step", name)), Sense::click());
    paint_step(ui, p, rect, name, resp.hovered());
    if resp.clicked() {
        resp.surrender_focus();
    }
    resp.on_hover_text(tip).clicked()
}

/// A 32-point button of the rocker, true while it is held.
fn hold_button(ui: &mut egui::Ui, p: &Palette, name: &str, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(32.0), Sense::click_and_drag());
    let held = resp.is_pointer_button_down_on();
    paint_step(ui, p, rect, name, resp.hovered() || held);
    let _ = resp.on_hover_text(tip);
    held
}

fn paint_step(ui: &egui::Ui, p: &Palette, rect: Rect, name: &str, hot: bool) {
    let radius = widgets::inner_radius(ui.visuals());
    let fill = if hot {
        p.panel.gamma_multiply(0.9)
    } else {
        p.navy.gamma_multiply(0.55)
    };
    ui.painter().rect_filled(rect, radius, fill);
    ui.painter()
        .rect_stroke(rect, radius, Stroke::new(1.0, p.ink.gamma_multiply(0.16)), StrokeKind::Inside);
    icon(ui.painter(), name, Rect::from_center_size(rect.center(), Vec2::splat(16.0)), p.ink);
}

/// A stick: dragged, where its puck is (−1 to 1 each way); let go, it
/// springs back to the middle.
fn stick(ui: &mut egui::Ui, p: &Palette, id: &str, puck_icon: &str, arrows: bool, enabled: bool, tip: &str) -> Vec2 {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(STICK), if enabled { Sense::drag() } else { Sense::hover() });
    let c = rect.center();
    let mut v = Vec2::ZERO;
    if enabled
        && resp.dragged()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let d = pos - c;
        v = if d.length() > STICK_TRAVEL {
            d / d.length()
        } else {
            d / STICK_TRAVEL
        };
    }
    let shown = Vec2::new(
        ui.ctx()
            .animate_value_with_time(Id::unique(id).with("x"), v.x, if resp.dragged() { 0.0 } else { 0.18 }),
        ui.ctx()
            .animate_value_with_time(Id::unique(id).with("y"), v.y, if resp.dragged() { 0.0 } else { 0.18 }),
    );
    let alpha = if enabled { 1.0 } else { 0.35 };
    let painter = ui.painter();
    painter.circle_filled(c, STICK / 2.0, p.navy.gamma_multiply(0.5 * alpha));
    painter.circle_stroke(c, STICK / 2.0 - 0.5, Stroke::new(1.0, p.ink.gamma_multiply(0.2 * alpha)));
    let mark = p.ink.gamma_multiply(0.5 * alpha);
    if arrows {
        for (dx, dy) in [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
            let tip = c + Vec2::new(dx, dy) * (STICK / 2.0 - 6.0);
            let side = Vec2::new(dy, dx) * 4.0;
            let base = tip - Vec2::new(dx, dy) * 5.0;
            painter.add(egui::Shape::convex_polygon(vec![tip, base + side, base - side], mark, Stroke::NONE));
        }
    } else {
        let s = Stroke::new(1.0, p.ink.gamma_multiply(0.3 * alpha));
        painter.line_segment([c - Vec2::new(0.0, STICK / 2.0 - 14.0), c + Vec2::new(0.0, STICK / 2.0 - 14.0)], s);
        painter.line_segment([c - Vec2::new(STICK / 2.0 - 14.0, 0.0), c + Vec2::new(STICK / 2.0 - 14.0, 0.0)], s);
    }
    let puck = c + shown * STICK_TRAVEL;
    painter.circle_filled(puck + Vec2::new(0.0, 2.0), 17.0, p.navy.gamma_multiply(0.4 * alpha));
    painter.circle_filled(puck, 17.0, p.ink.gamma_multiply(alpha));
    icon(painter, puck_icon, Rect::from_center_size(puck, Vec2::splat(18.0)), p.navy);
    if shown != v {
        ui.ctx().request_repaint();
    }
    let cursor = if resp.dragged() {
        egui::CursorIcon::Grabbing
    } else {
        egui::CursorIcon::Grab
    };
    let _ = resp.on_hover_cursor(cursor).on_hover_text(tip);
    v
}

/// The zoom rocker: pushed up or down (−1 to 1), back to the middle when
/// let go.
fn rocker(ui: &mut egui::Ui, p: &Palette, tip: &str) -> f32 {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(32.0, 96.0), Sense::drag());
    let mut v = 0.0;
    if resp.dragged()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        v = ((pos.y - rect.center().y) / (rect.height() / 2.0)).clamp(-1.0, 1.0);
    }
    let shown = ui
        .ctx()
        .animate_value_with_time(Id::unique("vf_rocker"), v, if resp.dragged() { 0.0 } else { 0.18 });
    let painter = ui.painter();
    let c = rect.center();
    let track = Rect::from_center_size(c, Vec2::new(4.0, rect.height()));
    painter.rect_filled(track, widgets::TRACK_RADIUS, p.ink.gamma_multiply(0.2));
    painter.line_segment(
        [c - Vec2::new(6.0, 0.0), c + Vec2::new(6.0, 0.0)],
        Stroke::new(1.0, p.ink.gamma_multiply(0.45)),
    );
    let y = c.y + shown * rect.height() / 2.0;
    if shown.abs() > 0.01 {
        let fill = Rect::from_min_max(Pos2::new(c.x - 2.0, y.min(c.y)), Pos2::new(c.x + 2.0, y.max(c.y)));
        painter.rect_filled(fill, widgets::TRACK_RADIUS, p.teal);
    }
    let thumb = Rect::from_center_size(Pos2::new(c.x, y), Vec2::new(22.0, 14.0));
    painter.rect_filled(thumb, widgets::inner_radius(ui.visuals()), p.ink);
    if shown != v {
        ui.ctx().request_repaint();
    }
    let _ = resp.on_hover_cursor(egui::CursorIcon::ResizeVertical).on_hover_text(tip);
    v
}

/// The tilt, ±`ROLL_MAX` (finer near the middle, detents at 0 and a
/// quarter turn; a double click puts it back, the wheel over it a degree a
/// notch); the new value when it changed.
fn tilt(ui: &mut egui::Ui, p: &Palette, width: f32, roll: f32) -> Option<f32> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width.max(60.0), 24.0), Sense::click_and_drag());
    let mut new = None;
    if resp.double_clicked() {
        new = Some(0.0);
    } else if resp.dragged()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let r = vf::roll_of_slider((pos.x - rect.left()) / rect.width() * 2.0 - 1.0);
        if (r - roll).abs() > 1e-3 {
            new = Some(r);
        }
    } else if resp.hovered() {
        let dy = wheel_notches(ui);
        if dy != 0.0 {
            new = Some(vf::step_roll(roll, if dy > 0.0 { 1 } else { -1 }));
        }
    }
    let painter = ui.painter();
    let c = rect.center();
    painter.rect_filled(
        Rect::from_center_size(c, Vec2::new(rect.width(), 4.0)),
        widgets::TRACK_RADIUS,
        p.ink.gamma_multiply(0.2),
    );
    let half = rect.width() / 2.0;
    painter.line_segment(
        [c - Vec2::new(0.0, 8.0), c + Vec2::new(0.0, 8.0)],
        Stroke::new(1.0, p.ink.gamma_multiply(0.5)),
    );
    // the quarter turns
    for side in [-1.0, 1.0] {
        let x = c.x + vf::slider_of_roll(90.0 * side) * half;
        painter.line_segment(
            [Pos2::new(x, c.y - 5.0), Pos2::new(x, c.y + 5.0)],
            Stroke::new(1.0, p.ink.gamma_multiply(0.35)),
        );
    }
    let shown = new.unwrap_or(roll);
    let x = c.x + vf::slider_of_roll(shown) * half;
    if shown != 0.0 {
        let fill = Rect::from_min_max(Pos2::new(x.min(c.x), c.y - 2.0), Pos2::new(x.max(c.x), c.y + 2.0));
        painter.rect_filled(fill, widgets::TRACK_RADIUS, p.teal);
    }
    painter.rect_filled(
        Rect::from_center_size(Pos2::new(x, c.y), Vec2::new(14.0, 18.0)),
        widgets::inner_radius(ui.visuals()),
        p.ink,
    );
    if resp.drag_stopped() || resp.double_clicked() {
        resp.surrender_focus();
    }
    let _ = resp.on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    new
}

/// The colour of the level and of the tilt's value: green when straight.
pub fn level_color(p: &Palette, roll: f32, pitch: f32) -> Color32 {
    if roll == 0.0 && pitch.abs() < 0.5 { p.success } else { p.ink }
}
