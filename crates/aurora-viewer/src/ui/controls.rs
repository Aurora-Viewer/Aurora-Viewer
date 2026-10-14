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
/// The body's width inside the window's margins (`widgets::Floater`).
const INNER: f32 = WIDTH - 24.0;
/// The minimized window's body: the joysticks and the zoom.
const COMPACT_WIDTH: f32 = 2.0 * STICK + ZOOM_W + 3.0 * 12.0;
const STICK: f32 = 88.0;
const KNOB: f32 = 30.0;
/// A joystick pulled less than this (of its reach) does nothing
/// (lljoystickbutton.cpp's "slop" around the press).
const DEAD_ZONE: f32 = 0.25;
const PAD_BUTTON: f32 = 40.0;
/// The joysticks' names under them.
const CAPTION_H: f32 = 16.0;
/// The zoom's column between the joysticks: its + and − and the slider.
const ZOOM_W: f32 = 24.0;
/// The zoom's + and −.
const ZOOM_BUTTON: f32 = 18.0;
/// Room between two items of a row of buttons.
const ROW_GAP: f32 = 4.0;
const BUTTON_H: f32 = 26.0;

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
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, camera: &mut bool, movement: &mut bool, classic: bool, view: &View) -> Output {
        let mut out = Output::default();
        if classic {
            if *camera {
                let size = Vec2::new(250.0, 210.0);
                Floater::new(
                    CLASSIC_CAMERA,
                    aurora_i18n::tr!("controls-camera-title"),
                    Pos2::new(60.0, 160.0),
                    size,
                )
                .auto_height()
                .show(ctx, p, camera, |ui| classic_camera(ui, p, view, &mut out));
            }
            if *movement {
                let size = Vec2::new(230.0, 140.0);
                Floater::new(
                    CLASSIC_MOVEMENT,
                    aurora_i18n::tr!("controls-tab-movement"),
                    Pos2::new(60.0, 400.0),
                    size,
                )
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
            .compact_when_minimized(COMPACT_WIDTH + 20.0)
            .show(ctx, p, &mut open, |ui| {
                // minimized: the joysticks and the zoom only (the mockup's
                // compact window)
                if widgets::floater_minimized(ui.ctx(), WINDOW) {
                    sticks(ui, p, view, &mut out, COMPACT_WIDTH);
                } else {
                    self.body(ui, p, view, &mut out);
                }
            });
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
        ui.set_width(INNER);
        ui.spacing_mut().item_spacing.y = 8.0;
        // the two tabs in pills, each half of the width
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (tab, icon, label) in [
                (Tab::Camera, "video-filled", aurora_i18n::tr!("controls-tab-camera")),
                (Tab::Movement, "walk", aurora_i18n::tr!("controls-tab-movement")),
            ] {
                if widgets::pills::pill_tab_wide(ui, p, icon, label, self.tab == tab, INNER / 2.0).clicked() {
                    self.tab = tab;
                }
            }
        });
        match self.tab {
            Tab::Camera => self.camera_tab(ui, p, view, out),
            Tab::Movement => movement_tab(ui, p, view, out),
        }
    }

    fn camera_tab(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
        sticks(ui, p, view, out, INNER);
        // the four views of Preferences › Camera, in one click
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = ROW_GAP;
            let w = (INNER - 3.0 * ROW_GAP) / 4.0;
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
        // two equal buttons and the reset; the saved positions below on the
        // same columns
        let grid = ButtonGrid::new(INNER);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = ROW_GAP;
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
                Vec2::new(grid.button, BUTTON_H),
            )
            .on_hover_text(aurora_i18n::tr!("controls-object-view-tip"));
            if object.clicked() {
                out.actions.push(ControlAction::ObjectView);
            }
            let mouselook = widgets::action_button_keyed(
                ui,
                p,
                "eye-filled",
                aurora_i18n::tr!("common-mouselook"),
                widgets::ActionKind::Flat,
                Vec2::new(grid.button, BUTTON_H),
                true,
                pad_binding(view.keys, Action::Mouselook).as_ref(),
            );
            if mouselook.clicked() {
                out.actions.push(ControlAction::Mouselook);
            }
            if square_button(ui, p, "restore", aurora_i18n::tr!("controls-reset-camera")) {
                out.actions.push(ControlAction::ResetCamera);
            }
        });
        self.shots(ui, p, view, out, &grid);
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
                (vec![alt(false, false)], aurora_i18n::tr!("controls-drag-zoom")),
                (vec![alt(true, false)], aurora_i18n::tr!("controls-drag-orbit")),
                (vec![alt(true, true)], aurora_i18n::tr!("controls-drag-pan")),
                (
                    pad_binding(view.keys, Action::ResetCamera).into_iter().collect(),
                    aurora_i18n::tr!("keybinds-return-to-avatar"),
                ),
            ],
        );
    }

    /// The saved camera positions (Firestorm's camera presets): a field as
    /// wide as the two buttons above, the save button under the reset.
    fn shots(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output, grid: &ButtonGrid) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = ROW_GAP;
            if self.naming {
                let edit = ui.add_sized(
                    Vec2::new(grid.field, BUTTON_H),
                    egui::TextEdit::singleline(&mut self.shot_name).hint_text(aurora_i18n::tr!("controls-shot-name")),
                );
                edit.request_focus();
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let name = self.shot_name.trim().to_owned();
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    self.naming = false;
                }
                if (square_button(ui, p, "check", aurora_i18n::tr!("controls-shot-save")) || enter) && !name.is_empty() {
                    out.actions.push(ControlAction::SaveShot(name));
                    self.naming = false;
                    self.shot_name.clear();
                }
                return;
            }
            let text = if view.shots.is_empty() {
                aurora_i18n::tr!("controls-shots-none")
            } else {
                aurora_i18n::tr!("controls-shots")
            };
            let field = widgets::action_button(
                ui,
                p,
                "bookmark-filled",
                text,
                widgets::ActionKind::Flat,
                Vec2::new(grid.field, BUTTON_H),
            );
            egui::Popup::menu(&field).show(|ui| {
                ui.set_min_width(grid.field);
                for (i, shot) in view.shots.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if super::menu::item(ui, p, "bookmark-filled", &shot.name) {
                            out.actions.push(ControlAction::UseShot(i));
                        }
                        if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("controls-shot-delete"), true) {
                            out.actions.push(ControlAction::DeleteShot(i));
                        }
                    });
                }
            });
            if square_button(ui, p, "device-floppy-filled", aurora_i18n::tr!("controls-shot-save-tip")) {
                self.naming = true;
            }
        });
    }
}

/// The columns of the camera tab's button rows, from the body's width:
/// two equal buttons and a square one at the end; the saved positions'
/// field spans the two buttons, its button sits under the square one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonGrid {
    pub button: f32,
    pub field: f32,
    /// Left edge of the square button, from the row's left.
    pub square_left: f32,
}

impl ButtonGrid {
    pub fn new(width: f32) -> ButtonGrid {
        let button = (width - BUTTON_H - 2.0 * ROW_GAP) / 2.0;
        ButtonGrid {
            button,
            field: 2.0 * button + ROW_GAP,
            square_left: 2.0 * (button + ROW_GAP),
        }
    }
}

/// Where the joysticks and the zoom sit in their row, from its width: the
/// three spread evenly (the same room at both ends and between them), on
/// one horizontal line.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SticksRow {
    pub orbit: Rect,
    pub zoom: Rect,
    pub pan: Rect,
}

impl SticksRow {
    pub fn new(row: Rect) -> SticksRow {
        let gap = (row.width() - 2.0 * STICK - ZOOM_W) / 4.0;
        let y = row.center().y;
        let at = |left: f32, w: f32| Rect::from_center_size(egui::pos2(left + w / 2.0, y), Vec2::new(w, STICK));
        let orbit = at(row.left() + gap, STICK);
        let zoom = at(orbit.right() + gap, ZOOM_W);
        let pan = at(zoom.right() + gap, STICK);
        SticksRow { orbit, zoom, pan }
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

/// The two joysticks and the zoom between them (floater_camera.xml), on
/// one line across `width`.
fn sticks(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output, width: f32) {
    // (drawn in place, then the row is taken: the slider's own ui must not
    // move the parent's cursor)
    let row = Rect::from_min_size(ui.cursor().min, Vec2::new(width, STICK));
    let at = SticksRow::new(row);
    let orbit = joystick(ui, p, at.orbit, "rotate-360", aurora_i18n::tr!("controls-orbit-tip"));
    if orbit.center {
        out.actions.push(ControlAction::ResetOrbit);
    }
    out.held.extend(orbit_actions(orbit.dir));
    let pan = joystick(ui, p, at.pan, "arrows-move", aurora_i18n::tr!("controls-pan-tip"));
    out.held.extend(pan_actions(pan.dir));
    // the zoom: + at the top, the slider, − at the bottom, centred on the line
    let z = at.zoom;
    let plus = Rect::from_center_size(egui::pos2(z.center().x, z.top() + ZOOM_BUTTON / 2.0), Vec2::splat(ZOOM_BUTTON));
    let minus = Rect::from_center_size(egui::pos2(z.center().x, z.bottom() - ZOOM_BUTTON / 2.0), Vec2::splat(ZOOM_BUTTON));
    if hold_icon(ui, p, plus, "zoom-in-filled", &key_tip(view.keys, Action::CamZoomIn)) {
        out.held.push(Action::CamZoomIn);
    }
    if hold_icon(ui, p, minus, "zoom-out-filled", &key_tip(view.keys, Action::CamZoomOut)) {
        out.held.push(Action::CamZoomOut);
    }
    let rail = Rect::from_min_max(egui::pos2(z.left(), plus.bottom() + 2.0), egui::pos2(z.right(), minus.top() - 2.0));
    let mut zoom = view.zoom;
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(rail)
            .layout(egui::Layout::top_down(egui::Align::Center)),
    );
    child.spacing_mut().slider_width = rail.height();
    let slider = child.add(egui::Slider::new(&mut zoom, 0.0..=1.0).vertical().show_value(false));
    if slider.changed() {
        out.actions.push(ControlAction::Zoom(zoom));
    }
    // their names under the joysticks
    for (stick, name) in [
        (at.orbit, aurora_i18n::tr!("controls-orbit")),
        (at.pan, aurora_i18n::tr!("controls-pan")),
    ] {
        ui.painter().text(
            egui::pos2(stick.center().x, row.bottom() + CAPTION_H / 2.0),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(11.0),
            p.muted,
        );
    }
    ui.allocate_rect(Rect::from_min_size(row.min, Vec2::new(width, STICK + CAPTION_H)), Sense::hover());
}

fn movement_tab(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
    pad(ui, p, view, out);
    modes(ui, p, view, out);
    // Stop flying and Stand up stay in place, greyed when they do not apply
    // (panel_stand_stop_flying shows them only then), so the window keeps
    // its layout
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = ROW_GAP;
        let w = (INNER - ROW_GAP) / 2.0;
        let stop = widgets::action_button_keyed(
            ui,
            p,
            "plane-departure-filled",
            aurora_i18n::tr!("ui-bars-stop-flying"),
            widgets::ActionKind::Flat,
            Vec2::new(w, BUTTON_H),
            view.flying,
            pad_binding(view.keys, Action::Fly).as_ref(),
        );
        if stop.clicked() && view.flying {
            out.actions.push(ControlAction::Fly);
        }
        let stand = widgets::action_button_if(
            ui,
            p,
            "man-filled",
            aurora_i18n::tr!("common-stand-up"),
            widgets::ActionKind::Flat,
            Vec2::new(w, BUTTON_H),
            view.seated,
        );
        if stand.clicked() && view.seated {
            out.actions.push(ControlAction::StandUp);
        }
    });
    let first = |a: Action| view.keys.get(a)[0].clone();
    caps_rows(
        ui,
        p,
        &[
            (
                pad_binding(view.keys, Action::AlwaysRun).into_iter().collect(),
                aurora_i18n::tr!("keybinds-always-run"),
            ),
            (
                first(Action::Up).into_iter().chain(first(Action::Down)).collect(),
                aurora_i18n::tr!("controls-up-down"),
            ),
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
            if widgets::flat_button(ui, p, short_view(preset))
                .on_hover_text(preset.label())
                .clicked()
            {
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
    sticks(ui, p, view, out, ui.available_width());
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
            (
                "walk",
                aurora_i18n::tr!("controls-walk"),
                !view.flying && !view.always_run,
                ControlAction::Walk,
            ),
            (
                "run",
                aurora_i18n::tr!("controls-run"),
                !view.flying && view.always_run,
                ControlAction::Run,
            ),
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
fn joystick(ui: &mut egui::Ui, p: &Palette, rect: Rect, icon: &str, tip: &str) -> Stick {
    let resp = ui.interact(rect, ui.id().with(icon), Sense::click_and_drag());
    let resp = resp.on_hover_text(tip);
    let center = rect.center();
    let reach = (STICK - KNOB) / 2.0;
    let mut out = Stick::default();
    let mut knob = center;
    if resp.is_pointer_button_down_on()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        let pulled = pos - center;
        let offset = if pulled.length() > reach {
            pulled.normalized() * reach
        } else {
            pulled
        };
        knob = center + offset;
        out.dir = Vec2::new(offset.x, -offset.y) / reach;
    }
    if resp.clicked() && resp.interact_pointer_pos().is_some_and(|pos| pos.distance(center) < KNOB / 2.0) {
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

/// An icon button that counts while held (the zoom's + and −), at `rect`.
fn hold_icon(ui: &mut egui::Ui, p: &Palette, rect: Rect, icon: &str, tip: &str) -> bool {
    let resp = ui.interact(rect, ui.id().with(icon), Sense::click_and_drag());
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

/// A square icon button as tall as the row's buttons (the reset, the save).
fn square_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str) -> bool {
    widgets::action_button(ui, p, icon, "", widgets::ActionKind::Flat, Vec2::splat(BUTTON_H))
        .on_hover_text(tip)
        .clicked()
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
    paint_icon(
        ui,
        icon,
        rect.center() - Vec2::new(0.0, 3.0),
        18.0,
        if held { p.on_accent } else { p.ink },
    );
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
        ui.painter()
            .image(t.id(), Rect::from_center_size(center, Vec2::splat(size)), uv, tint);
    }
}

/// A movement mode: its icon and its name, the accent's face when chosen.
fn mode_button(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, on: bool, width: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, BUTTON_H + 2.0), Sense::click());
    let fill = match (on, resp.hovered()) {
        (true, _) => p.violet,
        (false, true) => p.raised.gamma_multiply(1.3),
        (false, false) => p.raised,
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    let color = if on { p.on_accent } else { p.muted };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), egui::FontId::proportional(12.0), color);
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

/// Keycaps and what they do, one row each (the mockup's list); a row
/// without a key is left out.
fn caps_rows(ui: &mut egui::Ui, p: &Palette, rows: &[(Vec<Binding>, &str)]) {
    let caps: Vec<_> = rows
        .iter()
        .map(|(bs, _)| (!bs.is_empty()).then(|| bs.iter().flat_map(|b| keycap::caps(ui, b, p.panel)).collect::<Vec<_>>()))
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

    /// The joysticks and the zoom on one line, the same room around each,
    /// the whole centred; the saved positions on the buttons' columns.
    #[test]
    fn the_camera_tab_is_laid_out_on_its_grid() {
        let close = |a: f32, b: f32| (a - b).abs() <= 1.0;
        for width in [INNER, COMPACT_WIDTH, 320.0] {
            let row = Rect::from_min_size(egui::pos2(10.0, 50.0), Vec2::new(width, STICK));
            let s = SticksRow::new(row);
            let y = row.center().y;
            assert!(close(s.orbit.center().y, y) && close(s.zoom.center().y, y) && close(s.pan.center().y, y));
            let left = s.orbit.left() - row.left();
            let gap1 = s.zoom.left() - s.orbit.right();
            let gap2 = s.pan.left() - s.zoom.right();
            let right = row.right() - s.pan.right();
            assert!(close(gap1, gap2), "{gap1} {gap2}");
            assert!(close(left, right), "centred: {left} {right}");
        }
        let g = ButtonGrid::new(INNER);
        // the two buttons, the gaps and the square fill the row
        assert!(close(2.0 * g.button + 2.0 * ROW_GAP + BUTTON_H, INNER));
        // the field ends where the second button ends, the save button is
        // in the square's column
        assert!(close(g.field, 2.0 * g.button + ROW_GAP));
        assert!(close(g.field + ROW_GAP, g.square_left));
        assert!(close(g.square_left + BUTTON_H, INNER));
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
