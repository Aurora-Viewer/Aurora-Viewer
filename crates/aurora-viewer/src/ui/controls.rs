//! The Controls window: Firestorm's camera controls (LLFloaterCamera,
//! floater_camera.xml: the orbit and pan joysticks of lljoystickbutton.cpp,
//! the zoom slider and its buttons, the camera views, object view,
//! mouselook, the saved camera positions of LLFloaterSaveCameraPreset) and
//! movement controls (LLFloaterMove, floater_moveview.xml: turn, walk, slide,
//! fly up and down, walk / run / fly, stop flying, stand up) in one window
//! with two tabs, or, with Preferences › Aurora, Firestorm's two windows.
//! The buttons and the joysticks are held keys: the app moves the avatar and
//! the camera with them as with the keyboard (`App::update_move_input`).

use super::widgets::{self, Floater, keycap};
use crate::camera::{CameraPreset, CameraShot};
use crate::keybinds::{Action, Binding, KeyBindings};
use crate::theme::Palette;
use egui::{Pos2, Rect, Sense, Vec2};

/// The Aurora window, and Firestorm's two.
pub const WINDOW: &str = "controls";
pub const CLASSIC_CAMERA: &str = "camera_controls";
pub const CLASSIC_MOVEMENT: &str = "movement_controls";

const WIDTH: f32 = 300.0;
const STICK: f32 = 88.0;
const KNOB: f32 = 30.0;
/// A joystick pulled less than this (of its reach) does nothing
/// (lljoystickbutton.cpp's "slop" around the press).
const DEAD_ZONE: f32 = 0.25;
const PAD_BUTTON: f32 = 40.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Camera,
    Movement,
}

#[derive(Default)]
pub struct ControlsUi {
    pub tab: Tab,
    /// The flags of the last frame, to see which one was just raised.
    was: (bool, bool),
    /// The name being typed for a new camera position.
    shot_name: String,
    naming: bool,
}

/// What the window shows of the world.
pub struct View<'a> {
    /// LLAgentCamera::getCameraZoomFraction (1 all the way in).
    pub zoom: f32,
    pub preset: CameraPreset,
    pub shots: &'a [CameraShot],
    pub flying: bool,
    pub always_run: bool,
    pub seated: bool,
    pub mouselook: bool,
    /// "Object view": a click in the world focuses the camera (Alt+click).
    pub object_view: bool,
    pub keys: &'a KeyBindings,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlAction {
    Preset(CameraPreset),
    /// The orbit joystick's centre (LLJoystickCameraRotate::resetJoystickCamera).
    ResetOrbit,
    /// Every camera setting back, like Preferences › Camera.
    ResetCamera,
    Zoom(f32),
    Mouselook,
    ObjectView,
    Walk,
    Run,
    Fly,
    StandUp,
    UseShot(usize),
    SaveShot(String),
    DeleteShot(usize),
}

#[derive(Default)]
pub struct Output {
    /// Actions held by the buttons and the joysticks this frame.
    pub held: Vec<Action>,
    pub actions: Vec<ControlAction>,
}

impl ControlsUi {
    /// The Aurora window (`camera` / `movement`: the toolbar's View and
    /// Movement, the World menu's entries; either opens it on its tab), or
    /// Firestorm's two windows when `classic`.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        camera: &mut bool,
        movement: &mut bool,
        classic: bool,
        view: &View,
    ) -> Output {
        let mut out = Output::default();
        if classic {
            if *camera {
                let size = Vec2::new(250.0, 210.0);
                Floater::new(CLASSIC_CAMERA, aurora_i18n::tr!("controls-camera-title"), Pos2::new(60.0, 160.0), size)
                    .auto_height()
                    .show(ctx, p, camera, |ui| classic_camera(ui, p, view, &mut out));
            }
            if *movement {
                let size = Vec2::new(230.0, 140.0);
                Floater::new(CLASSIC_MOVEMENT, aurora_i18n::tr!("controls-tab-movement"), Pos2::new(60.0, 400.0), size)
                    .auto_height()
                    .show(ctx, p, movement, |ui| classic_movement(ui, p, view, &mut out));
            }
            self.was = (*camera, *movement);
            return out;
        }
        sync_tab(self, camera, movement);
        let mut open = *camera || *movement;
        if open {
            let screen = ctx.content_rect();
            Floater::new(
                WINDOW,
                aurora_i18n::tr!("controls-title"),
                Pos2::new(screen.left() + 24.0, screen.bottom() - 460.0),
                Vec2::new(WIDTH, 420.0),
            )
            .fixed()
            .auto_height()
            .show(ctx, p, &mut open, |ui| self.body(ui, p, view, &mut out));
        }
        if open {
            *camera = self.tab == Tab::Camera;
            *movement = self.tab == Tab::Movement;
        } else {
            *camera = false;
            *movement = false;
        }
        self.was = (*camera, *movement);
        out
    }

    fn body(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
        ui.set_width(WIDTH - 24.0);
        ui.spacing_mut().item_spacing.y = 8.0;
        let mut tab = self.tab as usize;
        let labels = [
            (aurora_i18n::tr!("controls-tab-camera"), true),
            (aurora_i18n::tr!("controls-tab-movement"), true),
        ];
        if widgets::tabs(ui, p, &mut tab, &labels) {
            self.tab = if tab == 0 { Tab::Camera } else { Tab::Movement };
        }
        match self.tab {
            Tab::Camera => self.camera_tab(ui, p, view, out),
            Tab::Movement => movement_tab(ui, p, view, out),
        }
    }

    fn camera_tab(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
        sticks(ui, p, view, out);
        // the four views of Preferences › Camera, in one click
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let w = (ui.available_width() - 12.0) / 4.0;
            for (preset, icon) in [
                (CameraPreset::Rear, "user"),
                (CameraPreset::Front, "user-circle"),
                (CameraPreset::Side, "user-square"),
                (CameraPreset::Shoulder, "user-scan"),
            ] {
                let chip = view_chip(ui, p, icon, short_view(preset), view.preset == preset, w);
                if chip.on_hover_text(preset.label()).clicked() {
                    out.actions.push(ControlAction::Preset(preset));
                }
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // two equal buttons and the reset at the end of the row
            let w = ((ui.available_width() - 28.0) / 2.0).floor();
            let object = widgets::action_button(
                ui,
                p,
                "box",
                aurora_i18n::tr!("controls-object-view"),
                if view.object_view {
                    widgets::ActionKind::Primary
                } else {
                    widgets::ActionKind::Flat
                },
                Vec2::new(w, 26.0),
            )
            .on_hover_text(aurora_i18n::tr!("controls-object-view-tip"));
            if object.clicked() {
                out.actions.push(ControlAction::ObjectView);
            }
            let mouselook = widgets::action_button(
                ui,
                p,
                "eye-filled",
                aurora_i18n::tr!("common-mouselook"),
                widgets::ActionKind::Flat,
                Vec2::new(w, 26.0),
            );
            if mouselook.on_hover_text(key_tip(view.keys, Action::Mouselook)).clicked() {
                out.actions.push(ControlAction::Mouselook);
            }
            if widgets::icon_button(ui, p, "restore", aurora_i18n::tr!("controls-reset-camera"), true) {
                out.actions.push(ControlAction::ResetCamera);
            }
        });
        self.shots(ui, p, view, out);
        // the alt-camera (LLToolCamera): Alt, Ctrl+Alt, Ctrl+Alt+Shift with a drag
        let alt = |ctrl: bool, shift: bool| Binding {
            input: crate::keybinds::Input::key(winit::keyboard::KeyCode::AltLeft),
            ctrl,
            shift,
            alt: false,
        };
        caps_rows(
            ui,
            p,
            &[
                (Some(alt(false, false)), aurora_i18n::tr!("controls-drag-zoom")),
                (Some(alt(true, false)), aurora_i18n::tr!("controls-drag-orbit")),
                (Some(alt(true, true)), aurora_i18n::tr!("controls-drag-pan")),
                (pad_binding(view.keys, Action::ResetCamera), aurora_i18n::tr!("keybinds-return-to-avatar")),
            ],
        );
    }

    /// The saved camera positions (Firestorm's camera presets).
    fn shots(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if self.naming {
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut self.shot_name)
                        .hint_text(aurora_i18n::tr!("controls-shot-name"))
                        .desired_width(ui.available_width() - 52.0),
                );
                edit.request_focus();
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let name = self.shot_name.trim().to_owned();
                if (widgets::icon_button(ui, p, "check", aurora_i18n::tr!("controls-shot-save"), !name.is_empty()) || enter)
                    && !name.is_empty()
                {
                    out.actions.push(ControlAction::SaveShot(name));
                    self.naming = false;
                    self.shot_name.clear();
                }
                if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), true) {
                    self.naming = false;
                }
                return;
            }
            let text = if view.shots.is_empty() {
                aurora_i18n::tr!("controls-shots-none").to_owned()
            } else {
                aurora_i18n::tr!("controls-shots").to_owned()
            };
            egui::ComboBox::from_id_salt("camera_shots")
                .width(ui.available_width() - 52.0)
                .selected_text(text)
                .show_ui(ui, |ui| {
                    for (i, shot) in view.shots.iter().enumerate() {
                        ui.horizontal(|ui| {
                            if ui.selectable_label(false, &shot.name).clicked() {
                                out.actions.push(ControlAction::UseShot(i));
                            }
                            if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("controls-shot-delete"), true) {
                                out.actions.push(ControlAction::DeleteShot(i));
                            }
                        });
                    }
                });
            if widgets::icon_button(ui, p, "device-floppy-filled", aurora_i18n::tr!("controls-shot-save-tip"), true) {
                self.naming = true;
            }
        });
    }
}

/// A flag just raised opens the window on its tab; otherwise it stays.
fn sync_tab(ui: &mut ControlsUi, camera: &mut bool, movement: &mut bool) {
    if *movement && !ui.was.1 {
        ui.tab = Tab::Movement;
    } else if *camera && !ui.was.0 {
        ui.tab = Tab::Camera;
    }
}

/// The two joysticks and the zoom between them (floater_camera.xml).
fn sticks(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
    ui.horizontal(|ui| {
        let side = (ui.available_width() - 2.0 * STICK - 40.0) / 2.0;
        ui.add_space(side.max(0.0));
        let orbit = joystick(ui, p, "rotate-360", aurora_i18n::tr!("controls-orbit-tip"));
        if orbit.center {
            out.actions.push(ControlAction::ResetOrbit);
        }
        out.held.extend(orbit_actions(orbit.dir));
        ui.vertical(|ui| {
            ui.set_width(40.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.vertical_centered(|ui| {
                if hold_icon(ui, p, "zoom-in-filled", &key_tip(view.keys, Action::CamZoomIn), Vec2::splat(22.0)) {
                    out.held.push(Action::CamZoomIn);
                }
                let mut zoom = view.zoom;
                ui.spacing_mut().slider_width = STICK - 40.0;
                let slider = ui.add(egui::Slider::new(&mut zoom, 0.0..=1.0).vertical().show_value(false));
                if slider.changed() {
                    out.actions.push(ControlAction::Zoom(zoom));
                }
                if hold_icon(ui, p, "zoom-out-filled", &key_tip(view.keys, Action::CamZoomOut), Vec2::splat(22.0)) {
                    out.held.push(Action::CamZoomOut);
                }
            });
        });
        let pan = joystick(ui, p, "arrows-move", aurora_i18n::tr!("controls-pan-tip"));
        out.held.extend(pan_actions(pan.dir));
    });
}

fn movement_tab(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
    pad(ui, p, view, out);
    modes(ui, p, view, out);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let fly = if view.flying {
            aurora_i18n::tr!("ui-bars-stop-flying")
        } else {
            aurora_i18n::tr!("common-fly")
        };
        let icon = if view.flying { "plane-departure-filled" } else { "plane-tilt-filled" };
        if widgets::action_button(ui, p, icon, fly, widgets::ActionKind::Flat, Vec2::new(0.0, 26.0))
            .on_hover_text(key_tip(view.keys, Action::Fly))
            .clicked()
        {
            out.actions.push(ControlAction::Fly);
        }
        if view.seated
            && widgets::action_button(
                ui,
                p,
                "man-filled",
                aurora_i18n::tr!("common-stand-up"),
                widgets::ActionKind::Flat,
                Vec2::new(0.0, 26.0),
            )
            .clicked()
        {
            out.actions.push(ControlAction::StandUp);
        }
    });
    keys_list(
        ui,
        p,
        view.keys,
        &[
            (Action::AlwaysRun, aurora_i18n::tr!("keybinds-always-run")),
            (Action::Fly, aurora_i18n::tr!("extra-fly-land")),
        ],
    );
}

/// Firestorm's eight buttons: turn, forward, turn, up / slide, back,
/// slide, down; each held moves like its key, whose cap it shows.
fn pad(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
    let rows = [
        [
            ("rotate-2", Action::Left, "controls-turn-left"),
            ("arrow-up", Action::Forward, "controls-forward"),
            ("rotate-clockwise-2", Action::Right, "controls-turn-right"),
            ("chevrons-up", Action::Up, "controls-up"),
        ],
        [
            ("arrow-left", Action::StrafeLeft, "controls-slide-left"),
            ("arrow-down", Action::Back, "controls-back"),
            ("arrow-right", Action::StrafeRight, "controls-slide-right"),
            ("chevrons-down", Action::Down, "controls-down"),
        ],
    ];
    let width = 4.0 * PAD_BUTTON + 3.0 * 4.0 + 12.0;
    for row in rows {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
            for (n, (icon, action, tip)) in row.into_iter().enumerate() {
                if n == 3 {
                    ui.add_space(12.0);
                }
                let binding = pad_binding(view.keys, action);
                let tip = match &binding {
                    Some(b) => format!("{} ({})", tr_key(tip), b.label()),
                    None => tr_key(tip).to_owned(),
                };
                if pad_button(ui, p, icon, &tip, binding.as_ref()) {
                    out.held.push(action);
                }
            }
        });
    }
}

/// Walk, run, fly (floater_moveview.xml's mode buttons).
fn modes(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
    let mut mode = if view.flying {
        2
    } else if view.always_run {
        1
    } else {
        0
    };
    let options = [
        ("walk", aurora_i18n::tr!("controls-walk"), ControlAction::Walk),
        ("run", aurora_i18n::tr!("controls-run"), ControlAction::Run),
        ("plane-tilt-filled", aurora_i18n::tr!("common-fly"), ControlAction::Fly),
    ];
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let w = (ui.available_width() - 8.0) / 3.0;
        for (n, (icon, label, action)) in options.into_iter().enumerate() {
            if mode_button(ui, p, icon, label, n == mode, w).clicked() && n != mode {
                mode = n;
                out.actions.push(action);
            }
        }
    });
}

/// Firestorm's camera window: views on top, the joysticks and the zoom,
/// then its presets (floater_camera.xml).
fn classic_camera(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for preset in [CameraPreset::Front, CameraPreset::Side, CameraPreset::Rear, CameraPreset::Shoulder] {
            if widgets::flat_button(ui, p, short_view(preset)).on_hover_text(preset.label()).clicked() {
                out.actions.push(ControlAction::Preset(preset));
            }
        }
        if widgets::icon_toggle(ui, p, "box", aurora_i18n::tr!("controls-object-view"), view.object_view) {
            out.actions.push(ControlAction::ObjectView);
        }
        if widgets::icon_toggle(ui, p, "eye-filled", aurora_i18n::tr!("common-mouselook"), view.mouselook) {
            out.actions.push(ControlAction::Mouselook);
        }
        if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("controls-reset-view"), true) {
            out.actions.push(ControlAction::ResetOrbit);
        }
    });
    sticks(ui, p, view, out);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        egui::ComboBox::from_id_salt("camera_shots_classic")
            .width(ui.available_width() - 52.0)
            .selected_text(aurora_i18n::tr!("controls-use-preset"))
            .show_ui(ui, |ui| {
                for (i, shot) in view.shots.iter().enumerate() {
                    if ui.selectable_label(false, &shot.name).clicked() {
                        out.actions.push(ControlAction::UseShot(i));
                    }
                }
            });
        if widgets::icon_button(ui, p, "restore", aurora_i18n::tr!("controls-reset-camera"), true) {
            out.actions.push(ControlAction::ResetCamera);
        }
    });
}

/// Firestorm's movement window: the 4 × 2 buttons, then walk / run / fly.
fn classic_movement(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
    ui.spacing_mut().item_spacing.y = 6.0;
    pad(ui, p, view, out);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (icon, tip, on, action) in [
            ("walk", aurora_i18n::tr!("controls-walk"), !view.flying && !view.always_run, ControlAction::Walk),
            ("run", aurora_i18n::tr!("controls-run"), !view.flying && view.always_run, ControlAction::Run),
            ("plane-tilt-filled", aurora_i18n::tr!("common-fly"), view.flying, ControlAction::Fly),
        ] {
            if widgets::icon_toggle(ui, p, icon, tip, on) {
                out.actions.push(action);
            }
        }
        if view.seated && widgets::flat_button(ui, p, aurora_i18n::tr!("common-stand-up")).clicked() {
            out.actions.push(ControlAction::StandUp);
        }
    });
}

fn short_view(preset: CameraPreset) -> &'static str {
    match preset {
        CameraPreset::Front => aurora_i18n::tr!("controls-view-front"),
        CameraPreset::Side => aurora_i18n::tr!("controls-view-side"),
        CameraPreset::Shoulder => aurora_i18n::tr!("controls-view-shoulder"),
        _ => aurora_i18n::tr!("controls-view-rear"),
    }
}

fn tr_key(key: &str) -> &'static str {
    match key {
        "controls-turn-left" => aurora_i18n::tr!("controls-turn-left"),
        "controls-forward" => aurora_i18n::tr!("controls-forward"),
        "controls-turn-right" => aurora_i18n::tr!("controls-turn-right"),
        "controls-up" => aurora_i18n::tr!("controls-up"),
        "controls-slide-left" => aurora_i18n::tr!("controls-slide-left"),
        "controls-back" => aurora_i18n::tr!("controls-back"),
        "controls-slide-right" => aurora_i18n::tr!("controls-slide-right"),
        _ => aurora_i18n::tr!("controls-down"),
    }
}

/// The joystick's state this frame: where it is pulled (x right, y up, each
/// -1 to 1), and a click on its centre.
#[derive(Debug, Default, Clone, Copy)]
struct Stick {
    dir: Vec2,
    center: bool,
}

/// lljoystickbutton.cpp's joysticks: pressed and pulled, the knob follows
/// the pointer and the camera moves while it is held; a click on the centre
/// without pulling resets.
fn joystick(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str) -> Stick {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(STICK), Sense::click_and_drag());
    let resp = resp.on_hover_text(tip);
    let center = rect.center();
    let reach = (STICK - KNOB) / 2.0;
    let mut out = Stick::default();
    let mut knob = center;
    if resp.is_pointer_button_down_on()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let pulled = pos - center;
        let offset = if pulled.length() > reach { pulled.normalized() * reach } else { pulled };
        knob = center + offset;
        out.dir = Vec2::new(offset.x, -offset.y) / reach;
    }
    if resp.clicked()
        && resp
            .interact_pointer_pos()
            .is_some_and(|pos| pos.distance(center) < KNOB / 2.0)
    {
        out.center = true;
    }
    let painter = ui.painter();
    painter.circle_filled(center, STICK / 2.0, p.field);
    painter.circle_stroke(center, STICK / 2.0 - 0.5, egui::Stroke::new(1.0, p.raised));
    // the four directions
    for (dx, dy) in [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
        let d = Vec2::new(dx, dy);
        let tip = center + d * (STICK / 2.0 - 6.0);
        let base = center + d * (STICK / 2.0 - 11.0);
        let side = Vec2::new(-d.y, d.x) * 4.0;
        painter.add(egui::Shape::convex_polygon(
            vec![tip, base + side, base - side],
            p.muted_dim,
            egui::Stroke::NONE,
        ));
    }
    let held = resp.is_pointer_button_down_on();
    painter.circle_filled(knob, KNOB / 2.0, if held { p.violet } else { p.raised });
    if let Some(t) = super::icons::global(icon) {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let tint = if held { p.on_accent } else { p.violet_light };
        painter.image(t.id(), Rect::from_center_size(knob, Vec2::splat(16.0)), uv, tint);
    }
    out
}

/// LLJoystickCameraRotate::onHeldDown: right orbits left, up orbits up.
pub fn orbit_actions(dir: Vec2) -> Vec<Action> {
    let mut v = Vec::new();
    if dir.x > DEAD_ZONE {
        v.push(Action::CamOrbitCcw);
    } else if dir.x < -DEAD_ZONE {
        v.push(Action::CamOrbitCw);
    }
    if dir.y > DEAD_ZONE {
        v.push(Action::CamOrbitOver);
    } else if dir.y < -DEAD_ZONE {
        v.push(Action::CamOrbitUnder);
    }
    v
}

/// LLJoystickCameraTrack::onHeldDown.
pub fn pan_actions(dir: Vec2) -> Vec<Action> {
    let mut v = Vec::new();
    if dir.x > DEAD_ZONE {
        v.push(Action::CamPanRight);
    } else if dir.x < -DEAD_ZONE {
        v.push(Action::CamPanLeft);
    }
    if dir.y > DEAD_ZONE {
        v.push(Action::CamPanUp);
    } else if dir.y < -DEAD_ZONE {
        v.push(Action::CamPanDown);
    }
    v
}

/// An icon button that counts while held (the zoom's + and −).
fn hold_icon(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, size: Vec2) -> bool {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let held = resp.is_pointer_button_down_on();
    let resp = resp.on_hover_text(tip);
    let fill = if held {
        Some(p.violet)
    } else if resp.hovered() {
        Some(p.raised)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    }
    paint_icon(ui, icon, rect.center(), 15.0, if held { p.on_accent } else { p.muted });
    held
}

/// One of the movement buttons: its icon, its key in the corner, lit while
/// held.
fn pad_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, binding: Option<&Binding>) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(PAD_BUTTON), Sense::click_and_drag());
    let held = resp.is_pointer_button_down_on();
    let resp = resp.on_hover_text(tip);
    let fill = if held {
        p.violet
    } else if resp.hovered() {
        p.raised.gamma_multiply(1.3)
    } else {
        p.raised
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    paint_icon(ui, icon, rect.center() - Vec2::new(0.0, 3.0), 18.0, if held { p.on_accent } else { p.ink });
    if let Some(b) = binding {
        // (Shift as an arrow before the key: its name would not fit)
        let plain = Binding { shift: false, ..b.clone() };
        let caps = keycap::caps(ui, &plain, fill);
        let w = keycap::row_width(&caps);
        let pos = egui::pos2(rect.right() - 3.0 - w, rect.bottom() - 3.0 - keycap::HEIGHT / 2.0);
        if w < PAD_BUTTON - 6.0 {
            keycap::paint_row(ui, p, &caps, pos, fill);
            if b.shift {
                paint_icon(ui, "arrow-big-up-filled", pos - Vec2::new(7.0, 0.0), 11.0, p.violet_light);
            }
        }
    }
    held
}

fn paint_icon(ui: &egui::Ui, icon: &str, center: Pos2, size: f32, tint: egui::Color32) {
    if let Some(t) = super::icons::global(icon) {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        ui.painter().image(t.id(), Rect::from_center_size(center, Vec2::splat(size)), uv, tint);
    }
}

/// A movement mode: its icon and its name, the accent's face when chosen.
fn mode_button(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, on: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
    let fill = match (on, resp.hovered()) {
        (true, _) => p.violet,
        (false, true) => p.raised.gamma_multiply(1.3),
        (false, false) => p.raised,
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    let color = if on { p.on_accent } else { p.muted };
    let galley = ui.painter().layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.0), color);
    let total = 16.0 + 5.0 + galley.size().x;
    let left = rect.center().x - total / 2.0;
    paint_icon(ui, icon, egui::pos2(left + 8.0, rect.center().y), 15.0, color);
    ui.painter()
        .galley(egui::pos2(left + 21.0, rect.center().y - galley.size().y / 2.0), galley, color);
    resp
}

/// A view's chip, its icon over its name: lit while it is the one in use.
fn view_chip(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, on: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 42.0), Sense::click());
    let fill = match (on, resp.hovered()) {
        (true, _) => p.violet.gamma_multiply(0.35),
        (false, true) => p.raised.gamma_multiply(1.3),
        (false, false) => p.raised,
    };
    let radius = ui.visuals().widgets.inactive.corner_radius;
    ui.painter().rect_filled(rect, radius, fill);
    if on {
        ui.painter()
            .rect_stroke(rect, radius, egui::Stroke::new(1.0, p.violet_light), egui::StrokeKind::Inside);
    }
    paint_icon(ui, icon, rect.center() - Vec2::new(0.0, 8.0), 17.0, p.ink);
    ui.painter().text(
        rect.center() + Vec2::new(0.0, 10.0),
        egui::Align2::CENTER_CENTER,
        label,
        egui::FontId::proportional(11.0),
        if on { p.ink } else { p.muted },
    );
    resp
}

/// The binding a movement button shows: the letter of the user's layout
/// first (Z on AZERTY), else the other key. The slides have none by
/// default: Shift with the turn's letter, as the keyboard does.
fn pad_binding(keys: &KeyBindings, action: Action) -> Option<Binding> {
    let pick = |a: Action| {
        let [first, second] = keys.get(a);
        let letter = |b: &Option<Binding>| {
            b.as_ref()
                .is_some_and(|b| matches!(&b.input, crate::keybinds::Input::Key(k) if k.starts_with("Key")))
        };
        if letter(&second) {
            second
        } else if letter(&first) {
            first
        } else {
            first.or(second)
        }
    };
    match (pick(action), action) {
        (Some(b), _) => Some(b),
        (None, Action::StrafeLeft) => pick(Action::Left).map(|b| Binding { shift: true, ..b }),
        (None, Action::StrafeRight) => pick(Action::Right).map(|b| Binding { shift: true, ..b }),
        _ => None,
    }
}

/// "Fly (F)": a hover text with the user's key.
fn key_tip(keys: &KeyBindings, action: Action) -> String {
    match keys.get(action).into_iter().flatten().next() {
        Some(b) => b.label(),
        None => String::new(),
    }
}

/// A few actions with their keycaps, under the controls.
fn keys_list(ui: &mut egui::Ui, p: &Palette, keys: &KeyBindings, rows: &[(Action, &str)]) {
    let rows: Vec<(Option<Binding>, &str)> = rows.iter().map(|(a, l)| (pad_binding(keys, *a), *l)).collect();
    caps_rows(ui, p, &rows);
}

/// Keycaps and what they do, one row each (the mockup's list).
fn caps_rows(ui: &mut egui::Ui, p: &Palette, rows: &[(Option<Binding>, &str)]) {
    let caps: Vec<_> = rows
        .iter()
        .map(|(b, _)| b.as_ref().map(|b| keycap::caps(ui, b, p.panel)))
        .collect();
    let column = caps.iter().flatten().map(|c| keycap::row_width(c)).fold(0.0f32, f32::max);
    for ((_, label), caps) in rows.iter().zip(caps) {
        let Some(caps) = caps else {
            continue;
        };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::hover());
        keycap::paint_row(ui, p, &caps, egui::pos2(rect.left(), rect.center().y), p.panel);
        ui.painter().text(
            egui::pos2(rect.left() + column + 10.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            *label,
            egui::FontId::proportional(11.0),
            p.muted,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_joysticks_hold_firestorms_keys() {
        assert_eq!(orbit_actions(Vec2::new(0.8, 0.0)), vec![Action::CamOrbitCcw]);
        assert_eq!(orbit_actions(Vec2::new(-0.5, 0.6)), vec![Action::CamOrbitCw, Action::CamOrbitOver]);
        assert!(orbit_actions(Vec2::new(0.1, -0.2)).is_empty());
        assert_eq!(pan_actions(Vec2::new(0.0, -1.0)), vec![Action::CamPanDown]);
        assert_eq!(pan_actions(Vec2::new(1.0, 0.0)), vec![Action::CamPanRight]);
    }

    #[test]
    fn a_raised_flag_opens_the_window_on_its_tab() {
        let mut ui = ControlsUi::default();
        let (mut camera, mut movement) = (false, true);
        sync_tab(&mut ui, &mut camera, &mut movement);
        assert_eq!(ui.tab, Tab::Movement);
        ui.was = (false, true);
        // the toolbar's View while it is open on Movement
        camera = true;
        sync_tab(&mut ui, &mut camera, &mut movement);
        assert_eq!(ui.tab, Tab::Camera);
    }

    #[test]
    fn the_buttons_show_the_letter_of_the_layout() {
        let keys = KeyBindings::default();
        let forward = pad_binding(&keys, Action::Forward).expect("forward is bound");
        assert!(matches!(&forward.input, crate::keybinds::Input::Key(k) if k.starts_with("Key")));
        // the slides borrow the turn's key with Shift
        let slide = pad_binding(&keys, Action::StrafeLeft).expect("a slide key");
        assert!(slide.shift);
    }
}
