//! The Controls window: Firestorm's camera controls (LLFloaterCamera,
//! floater_camera.xml: the orbit and pan joysticks of lljoystickbutton.cpp,
//! the zoom slider and its buttons, the camera views, object view,
//! mouselook, the saved camera positions of LLFloaterSaveCameraPreset) and
//! movement controls (LLFloaterMove, floater_moveview.xml: turn, walk, slide,
//! fly up and down, walk / run / fly, stop flying, stand up) in one window
//! with two tabs, or, with Preferences › Aurora, Firestorm's two windows.
//! The buttons and the joysticks are held keys: the app moves the avatar and
//! the camera with them as with the keyboard (`App::update_move_input`).
//! While the photo window's free camera flies, the camera tab drives it
//! (Aurora's own: Firestorm has no keyboard free camera, `camera/free.rs`).

use super::widgets::{self, Floater, keycap};
use crate::camera::{CameraPreset, CameraShot, free};
use crate::keybinds::{Action, Binding, KeyBindings};
use crate::theme::Palette;
use egui::{FontId, Pos2, Rect, Sense, Vec2};

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
/// Width of `widgets::icon_button`, the delete button of a saved position.
const SHOT_DELETE_W: f32 = 20.0;
/// Room between the body's rows.
const BODY_GAP: f32 = 8.0;
/// A key list's row (`caps_rows`).
const KEY_ROW_H: f32 = 16.0;

// The free camera's tab, from the mockup's third state (maquette-camera,
// variant A): its body's rows are FREE_GAP apart (`.body` gap); the banner
// (`.free`) has its padding, its icon, then its text and its button
// BANNER_GAP apart, and the accent bar on its left; the column between the
// joysticks holds the up button, the up key, the down key and the down
// button ROW_GAP apart (`.zoomcol` gap) with KEYS_SPACER between the keys;
// a field row's label, slider and value are FIELD_GAP apart (`.row`); a key
// list's keys and what they do are KEYS_LABEL_GAP apart (`.kl`).
const FREE_GAP: f32 = 10.0;
const BANNER_PAD_X: f32 = 10.0;
const BANNER_PAD_Y: f32 = 8.0;
const BANNER_GAP: f32 = 8.0;
const BANNER_ICON: f32 = 18.0;
const BANNER_BAR: f32 = 3.0;
const KEYS_SPACER: f32 = 8.0;
const FIELD_GAP: f32 = 6.0;
/// "Back"'s padding on each side: in the banner (`.free .btn`) and
/// minimized (`.btn`).
const BANNER_BUTTON_PAD: f32 = 8.0;
const BUTTON_PAD: f32 = 10.0;
/// The speed's value box at least (`.val` min-width), its text's padding;
/// the row is as tall as the box.
const VALUE_MIN_W: f32 = 42.0;
const VALUE_PAD: Vec2 = Vec2::new(6.0, 3.0);
const KEYS_LABEL_GAP: f32 = 8.0;

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
    /// The photo window's free camera, while it flies.
    pub free: Option<FreeView>,
}

/// The free camera's settings in the window (the photo window's, never
/// saved).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreeView {
    /// Metres a second.
    pub speed: f32,
    pub no_collisions: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ControlAction {
    /// The free camera's speed, metres a second.
    FreeSpeed(f32),
    FreeCollisions(bool),
    /// "Back": the camera returns to the avatar.
    FreeExit,
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
    /// The free camera's joysticks: move (x right, y forward) and look
    /// (x right, y up), each from -1 to 1.
    pub free_move: Vec2,
    pub free_look: Vec2,
}

impl ControlsUi {
    /// The Aurora window (`camera` / `movement`: the toolbar's View and
    /// Movement, the World menu's entries; either opens it on its tab), or
    /// Firestorm's two windows when `classic`.
    pub fn show(&mut self, ctx: &egui::Context, p: &Palette, camera: &mut bool, movement: &mut bool, classic: bool, view: &View) -> Output {
        let mut out = Output::default();
        // the free camera keeps Aurora's window (Firestorm has none)
        if classic && view.free.is_none() {
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
        if view.free.is_some() {
            self.tab = Tab::Camera;
        }
        let mut open = *camera || *movement;
        if open {
            let screen = ctx.content_rect();
            let minimized = widgets::floater_minimized(ctx, WINDOW);
            // minimized, the free camera names the window
            let title = match view.free {
                Some(_) if minimized => aurora_i18n::tr!("controls-free-title"),
                _ => aurora_i18n::tr!("controls-title"),
            };
            Floater::new(
                WINDOW,
                title,
                Pos2::new(screen.left() + 24.0, screen.bottom() - 460.0),
                Vec2::new(WIDTH, 420.0),
            )
            .fixed()
            .auto_height()
            .compact_when_minimized(COMPACT_WIDTH + 20.0)
            .show(ctx, p, &mut open, |ui| {
                // minimized: the joysticks and the zoom only (the mockup's
                // compact window), or the free camera's joysticks, speed
                // and way back
                if widgets::floater_minimized(ui.ctx(), WINDOW) {
                    match view.free {
                        Some(free) => free_compact(ui, p, view, free, &mut out),
                        None => sticks(ui, p, view, &mut out, COMPACT_WIDTH),
                    }
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
        ui.spacing_mut().item_spacing.y = BODY_GAP;
        // the two tabs in pills, each half of the width; the free camera
        // takes the first and greys the other
        let free = view.free.is_some();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            for (tab, icon, label) in [
                (Tab::Camera, "video-filled", aurora_i18n::tr!("controls-tab-camera")),
                (Tab::Movement, "walk", aurora_i18n::tr!("controls-tab-movement")),
            ] {
                let label = if free && tab == Tab::Camera {
                    aurora_i18n::tr!("controls-free-title")
                } else {
                    label
                };
                let enabled = !free || tab == Tab::Camera;
                let why = aurora_i18n::tr!("controls-free-movement-off");
                if widgets::pill_tab_wide(ui, p, icon, label, self.tab == tab, INNER / 2.0, enabled, why).clicked() {
                    self.tab = tab;
                }
            }
        });
        match (self.tab, view.free) {
            (Tab::Camera, Some(free)) => free_tab(ui, p, view, free, out),
            (Tab::Camera, None) => self.camera_tab(ui, p, view, out),
            (Tab::Movement, _) => movement_tab(ui, p, view, out),
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
        // no list of the Alt-camera's keys below: they move the camera
        // without this window (#1035), and floater_camera.xml has none
        self.shots(ui, p, view, out, &grid);
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
                        // the row takes the menu's width less the delete
                        // button: a full-width row pushed the button out and
                        // the menu widened by that much every frame (#718)
                        let room = ui.available_width() - SHOT_DELETE_W - ui.spacing().item_spacing.x;
                        let used = ui.scope(|ui| {
                            ui.set_max_width(room);
                            // the icon tells what holds the preset's camera
                            super::menu::item(ui, p, super::viewfinder::anchor_icon(shot.anchor.as_ref()), &shot.name)
                        });
                        if used.inner {
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

/// The free camera's banner, from its top left corner and width: the icon
/// and the text on the left, the way back on the right, all centred on the
/// banner's height, which the taller of them sets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Banner {
    pub rect: Rect,
    pub icon: Rect,
    /// Top left corner of the text.
    pub text: Pos2,
    pub button: Rect,
}

impl Banner {
    /// What the text may take beside the icon and a button `button_w` wide.
    pub fn text_width(width: f32, button_w: f32) -> f32 {
        width - 2.0 * BANNER_PAD_X - BANNER_ICON - 2.0 * BANNER_GAP - button_w
    }

    pub fn new(min: Pos2, width: f32, button_w: f32, text_h: f32) -> Banner {
        let inner = text_h.max(BANNER_ICON).max(BUTTON_H);
        let rect = Rect::from_min_size(min, Vec2::new(width, inner + 2.0 * BANNER_PAD_Y));
        let y = rect.center().y;
        let icon = Rect::from_min_size(
            egui::pos2(rect.left() + BANNER_PAD_X, y - BANNER_ICON / 2.0),
            Vec2::splat(BANNER_ICON),
        );
        let button = Rect::from_min_size(
            egui::pos2(rect.right() - BANNER_PAD_X - button_w, y - BUTTON_H / 2.0),
            Vec2::new(button_w, BUTTON_H),
        );
        Banner {
            rect,
            icon,
            text: egui::pos2(icon.right() + BANNER_GAP, y - text_h / 2.0),
            button,
        }
    }
}

/// The free camera's column between the joysticks, centred on `center`:
/// up, its key, its spacer, the down key, down; minimized, the two buttons
/// only.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FreeColumn {
    pub up: Rect,
    pub down: Rect,
    /// Centres of the up and down keys.
    pub keys: Option<(Pos2, Pos2)>,
}

impl FreeColumn {
    /// `key`: the keys' height (`keycap::HEIGHT`), None without them.
    pub fn new(center: Pos2, key: Option<f32>) -> FreeColumn {
        let height = match key {
            Some(key) => 2.0 * BUTTON_H + 2.0 * key + KEYS_SPACER + 4.0 * ROW_GAP,
            None => 2.0 * BUTTON_H + ROW_GAP,
        };
        let top = center.y - height / 2.0;
        let up = Rect::from_min_size(egui::pos2(center.x - BUTTON_H / 2.0, top), Vec2::splat(BUTTON_H));
        let down = Rect::from_min_size(egui::pos2(up.left(), top + height - BUTTON_H), Vec2::splat(BUTTON_H));
        let keys = key.map(|key| {
            let up_key = egui::pos2(center.x, up.bottom() + ROW_GAP + key / 2.0);
            let down_key = egui::pos2(center.x, down.top() - ROW_GAP - key / 2.0);
            (up_key, down_key)
        });
        FreeColumn { up, down, keys }
    }
}

/// A field row of the free camera's tab: its name on the left, the value
/// box against the right, the slider between them, FIELD_GAP apart, all as
/// tall as the row.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FieldRow {
    pub label: Rect,
    pub slider: Rect,
    pub value: Rect,
}

impl FieldRow {
    pub fn new(row: Rect, label_w: f32, value_w: f32) -> FieldRow {
        let label = Rect::from_min_size(row.min, Vec2::new(label_w, row.height()));
        let value = Rect::from_min_size(egui::pos2(row.right() - value_w, row.top()), Vec2::new(value_w, row.height()));
        let slider = Rect::from_min_max(
            egui::pos2(label.right() + FIELD_GAP, row.top()),
            egui::pos2(value.left() - FIELD_GAP, row.bottom()),
        );
        FieldRow { label, slider, value }
    }
}

/// The free camera's speed as the window writes it: "2 m/s", "0.5 m/s";
/// whole metres from 10 m/s.
pub fn speed_text(speed: f32) -> String {
    let n = if speed >= 10.0 || (speed - speed.round()).abs() < 0.05 {
        aurora_i18n::number(f64::from(speed.round()), 0)
    } else {
        aurora_i18n::number(f64::from(speed), 1)
    };
    aurora_i18n::tr!("controls-free-speed-value", speed = n.as_str())
}

/// A speed typed in its box, with or without its unit.
fn parse_speed(text: &str) -> Option<f64> {
    aurora_i18n::parse_number(
        text.trim()
            .trim_end_matches(|c: char| c.is_alphabetic() || c == '/' || c.is_whitespace()),
    )
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
    if hold_icon(ui, p, plus, "zoom-in-filled", &key_tip(view.keys, Action::CamZoomIn), false) {
        out.held.push(Action::CamZoomIn);
    }
    if hold_icon(ui, p, minus, "zoom-out-filled", &key_tip(view.keys, Action::CamZoomOut), false) {
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

/// The free camera's tab (the mockup's third state): the banner and its way
/// back, the move and look joysticks with up / down between them, the
/// speed, the collisions and the keys.
fn free_tab(ui: &mut egui::Ui, p: &Palette, view: &View, free: FreeView, out: &mut Output) {
    banner(ui, p, view, out);
    ui.spacing_mut().item_spacing.y = FREE_GAP;
    free_sticks(ui, p, view, out, INNER, true);
    speed_row(ui, p, free, out);
    collisions_row(ui, p, free, out);
    // the keys, as the keyboard drives the free camera
    let key = |a: Action| pad_binding(view.keys, a).map(KeyPart::Key);
    let plain = |code| {
        KeyPart::Key(Binding {
            input: crate::keybinds::Input::key(code),
            ctrl: false,
            shift: false,
            alt: false,
        })
    };
    ui.spacing_mut().item_spacing.y = ROW_GAP;
    key_rows(
        ui,
        p,
        &[
            (
                [Action::Forward, Action::Left, Action::Back, Action::Right]
                    .into_iter()
                    .filter_map(key)
                    .collect(),
                aurora_i18n::tr!("controls-free-keys-move"),
            ),
            (
                [Action::Up, Action::Down].into_iter().filter_map(key).collect(),
                aurora_i18n::tr!("controls-up-down"),
            ),
            (
                vec![KeyPart::Text(aurora_i18n::tr!("controls-free-keys-mouse"))],
                aurora_i18n::tr!("controls-free-keys-look"),
            ),
            (
                vec![
                    plain(winit::keyboard::KeyCode::ShiftLeft),
                    KeyPart::Text("/"),
                    plain(winit::keyboard::KeyCode::ControlLeft),
                ],
                aurora_i18n::tr!("controls-free-keys-speed"),
            ),
        ],
        KEYS_LABEL_GAP,
    );
}

/// The banner on top of the free camera's tab: what flies, and the way back
/// with its key.
fn banner(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
    let key = pad_binding(view.keys, Action::ResetCamera);
    let back = aurora_i18n::tr!("controls-free-back");
    let button_w = widgets::action_button_keyed_content(ui, "user-filled", back, key.as_ref()) + 2.0 * BANNER_BUTTON_PAD;
    let width = ui.available_width();
    let wrap = Banner::text_width(width, button_w);
    let title = ui.painter().layout(
        aurora_i18n::tr!("controls-free-title").to_owned(),
        FontId::new(12.5, crate::ui::fonts::semibold(ui.ctx())),
        p.ink,
        wrap,
    );
    let sub = ui.painter().layout(
        aurora_i18n::tr!("controls-free-stays").to_owned(),
        FontId::proportional(11.0),
        p.muted,
        wrap,
    );
    let at = Banner::new(ui.cursor().min, width, button_w, title.size().y + sub.size().y);
    let (rect, _) = ui.allocate_exact_size(at.rect.size(), Sense::hover());
    let radius = ui.visuals().window_corner_radius;
    ui.painter().rect_filled(rect, radius, p.violet.gamma_multiply(0.16));
    // the accent bar along the left edge, inside the banner's corners
    let bar = Rect::from_min_size(rect.min, Vec2::new(BANNER_BAR, rect.height()));
    let left = egui::CornerRadius { ne: 0, se: 0, ..radius };
    ui.painter().rect_filled(bar, left, p.violet);
    paint_icon(ui, "video-filled", at.icon.center(), BANNER_ICON, p.violet_light);
    let title_h = title.size().y;
    ui.painter().galley(at.text, title, p.ink);
    ui.painter().galley(at.text + Vec2::new(0.0, title_h), sub, p.muted);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(at.button));
    if back_button(&mut child, p, key.as_ref(), at.button.width()) {
        out.actions.push(ControlAction::FreeExit);
    }
}

/// "Back" and its key (the reset camera's, Escape by default).
fn back_button(ui: &mut egui::Ui, p: &Palette, key: Option<&Binding>, width: f32) -> bool {
    widgets::action_button_keyed(
        ui,
        p,
        "user-filled",
        aurora_i18n::tr!("controls-free-back"),
        widgets::ActionKind::Primary,
        Vec2::new(width, BUTTON_H),
        true,
        key,
    )
    .on_hover_text(aurora_i18n::tr!("controls-free-back-tip"))
    .clicked()
}

/// The move and look joysticks on the camera tab's line, up and down
/// between them (with their keys unless minimized).
fn free_sticks(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output, width: f32, full: bool) {
    let row = Rect::from_min_size(ui.cursor().min, Vec2::new(width, STICK));
    let at = SticksRow::new(row);
    out.free_move = past_dead_zone(joystick(ui, p, at.orbit, "arrows-move", aurora_i18n::tr!("controls-free-move-tip")).dir);
    out.free_look = past_dead_zone(joystick(ui, p, at.pan, "eye-filled", aurora_i18n::tr!("controls-free-look-tip")).dir);
    // the column on the middle of the joysticks with their names
    let block = Rect::from_min_size(row.min, Vec2::new(width, STICK + CAPTION_H));
    let column = FreeColumn::new(egui::pos2(at.zoom.center().x, block.center().y), full.then_some(keycap::HEIGHT));
    for (rect, icon, action, tip) in [
        (column.up, "chevrons-up", Action::Up, aurora_i18n::tr!("controls-free-up")),
        (column.down, "chevrons-down", Action::Down, aurora_i18n::tr!("controls-free-down")),
    ] {
        let tip = match pad_binding(view.keys, action) {
            Some(b) => format!("{tip} ({})", b.label()),
            None => tip.to_owned(),
        };
        if hold_icon(ui, p, rect, icon, &tip, true) {
            out.held.push(action);
        }
    }
    if let Some((up, down)) = column.keys {
        for (center, action) in [(up, Action::Up), (down, Action::Down)] {
            if let Some(b) = pad_binding(view.keys, action) {
                let caps = keycap::caps(ui, &b, p.panel);
                let w = keycap::row_width(&caps);
                keycap::paint_row(ui, p, &caps, egui::pos2(center.x - w / 2.0, center.y), p.panel);
            }
        }
    }
    let walk = if full {
        aurora_i18n::tr!("controls-free-move")
    } else {
        aurora_i18n::tr!("controls-free-move-short")
    };
    for (stick, name) in [(at.orbit, walk), (at.pan, aurora_i18n::tr!("controls-free-look"))] {
        ui.painter().text(
            egui::pos2(stick.center().x, row.bottom() + CAPTION_H / 2.0),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(11.0),
            p.muted,
        );
    }
    ui.allocate_rect(block, Sense::hover());
}

/// The free camera's speed: its name, a slider and the value typed or shown.
fn speed_row(ui: &mut egui::Ui, p: &Palette, free: FreeView, out: &mut Output) {
    let label = field_label(ui, p, aurora_i18n::tr!("controls-free-speed"));
    // as wide as the widest value, so the slider does not move with it
    let font = ui.style().drag_value_text_style.resolve(ui.style());
    let widest = [9.5, free::SPEED_RANGE.end().to_owned()]
        .map(|v| ui.painter().layout_no_wrap(speed_text(v), font.clone(), p.ink).size())
        .into_iter()
        .fold(Vec2::ZERO, Vec2::max);
    let value = Vec2::new((widest.x + 2.0 * VALUE_PAD.x).max(VALUE_MIN_W), widest.y + 2.0 * VALUE_PAD.y);
    let value_w = value.x;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), value.y), Sense::hover());
    let at = FieldRow::new(rect, label.size().x, value_w);
    let label_at = crate::ui::widgets::row_text_origin(ui.painter(), &label, at.label.left(), at.label.center().y);
    ui.painter().galley(label_at, label, p.ink);
    let mut speed = free.speed;
    // the slider and the box exactly in their places, as tall as the row
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect));
    child.spacing_mut().slider_width = at.slider.width();
    child.spacing_mut().interact_size = value;
    child.spacing_mut().button_padding = VALUE_PAD;
    let slider = child.put(
        at.slider,
        egui::Slider::new(&mut speed, free::SPEED_RANGE).logarithmic(true).show_value(false),
    );
    let value = child.put(
        at.value,
        egui::DragValue::new(&mut speed)
            .range(free::SPEED_RANGE)
            .speed(0.05)
            .custom_formatter(|v, _| speed_text(v as f32))
            .custom_parser(parse_speed),
    );
    if slider.changed() || value.changed() {
        out.actions.push(ControlAction::FreeSpeed(speed));
    }
}

/// A field row's name, laid out to be drawn at its place.
fn field_label(ui: &egui::Ui, p: &Palette, text: &str) -> std::sync::Arc<egui::Galley> {
    ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(12.5), p.ink)
}

/// "No collisions": its name on the left, its switch against the right.
fn collisions_row(ui: &mut egui::Ui, p: &Palette, free: FreeView, out: &mut Output) {
    let label = field_label(ui, p, aurora_i18n::tr!("controls-free-no-collisions"));
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), widgets::SWITCH_SIZE.y), Sense::hover());
    let at = crate::ui::widgets::row_text_origin(ui.painter(), &label, rect.left(), rect.center().y);
    ui.painter().galley(at, label, p.ink);
    let switch = Rect::from_min_size(egui::pos2(rect.right() - widgets::SWITCH_SIZE.x, rect.top()), widgets::SWITCH_SIZE);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(switch));
    let mut on = free.no_collisions;
    let sw = widgets::switch(&mut child, p, &mut on);
    if sw.on_hover_text(aurora_i18n::tr!("controls-free-no-collisions-tip")).changed() {
        out.actions.push(ControlAction::FreeCollisions(on));
    }
}

/// Minimized during the free camera (the mockup's compact state): the two
/// joysticks, up and down, the speed and the way back.
fn free_compact(ui: &mut egui::Ui, p: &Palette, view: &View, free: FreeView, out: &mut Output) {
    ui.set_width(COMPACT_WIDTH);
    ui.spacing_mut().item_spacing.y = BODY_GAP;
    free_sticks(ui, p, view, out, COMPACT_WIDTH, false);
    let key = pad_binding(view.keys, Action::ResetCamera);
    let back = aurora_i18n::tr!("controls-free-back");
    let button_w = widgets::action_button_keyed_content(ui, "user-filled", back, key.as_ref()) + 2.0 * BUTTON_PAD;
    let (row, _) = ui.allocate_exact_size(Vec2::new(COMPACT_WIDTH, BUTTON_H), Sense::hover());
    ui.painter().text(
        egui::pos2(row.left(), row.center().y),
        egui::Align2::LEFT_CENTER,
        speed_text(free.speed),
        egui::FontId::proportional(11.0),
        p.muted,
    );
    let button = Rect::from_min_size(egui::pos2(row.right() - button_w, row.top()), Vec2::new(button_w, BUTTON_H));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(button));
    if back_button(&mut child, p, key.as_ref(), button_w) {
        out.actions.push(ControlAction::FreeExit);
    }
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
    let resp = ui.interact(rect, ui.scope_id().with(icon), Sense::click_and_drag());
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

/// A free camera's joystick: nothing in the dead zone, then from 0 to 1 at
/// its edge in the direction pulled (the flycam's dead zone, moveFlycam).
pub fn past_dead_zone(dir: Vec2) -> Vec2 {
    let pulled = dir.length();
    if pulled <= DEAD_ZONE {
        return Vec2::ZERO;
    }
    dir / pulled * ((pulled.min(1.0) - DEAD_ZONE) / (1.0 - DEAD_ZONE))
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

/// An icon button that counts while held, at `rect`: bare (the zoom's + and
/// −) or on a button's face (the free camera's up and down).
fn hold_icon(ui: &mut egui::Ui, p: &Palette, rect: Rect, icon: &str, tip: &str, face: bool) -> bool {
    let resp = ui.interact(rect, ui.scope_id().with(icon), Sense::click_and_drag());
    let held = resp.is_pointer_button_down_on();
    let resp = resp.on_hover_text(tip);
    let fill = match (held, resp.hovered(), face) {
        (true, _, _) => Some(p.violet),
        (false, true, true) => Some(p.raised.gamma_multiply(1.3)),
        (false, true, false) | (false, false, true) => Some(p.raised),
        (false, false, false) => None,
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    }
    let ink = match (held, face) {
        (true, _) => p.on_accent,
        (false, true) => p.ink,
        (false, false) => p.muted,
    };
    paint_icon(ui, icon, rect.center(), 15.0, ink);
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
    let at = crate::ui::widgets::row_text_origin(ui.painter(), &galley, left + 21.0, rect.center().y);
    ui.painter().galley(at, galley, color);
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
    let rows: Vec<(Vec<KeyPart>, &str)> = rows
        .iter()
        .map(|(bs, label)| (bs.iter().cloned().map(KeyPart::Key).collect(), *label))
        .collect();
    key_rows(ui, p, &rows, 10.0);
}

/// A piece of a key list's row: a key's caps, or words (a mouse gesture,
/// the slash between two keys).
enum KeyPart<'a> {
    Key(Binding),
    Text(&'a str),
}

/// What a `KeyPart` draws.
enum Piece {
    Caps(Vec<keycap::Keycap>),
    Text(std::sync::Arc<egui::Galley>),
}

impl Piece {
    fn width(&self) -> f32 {
        match self {
            Piece::Caps(caps) => keycap::row_width(caps),
            Piece::Text(galley) => galley.size().x,
        }
    }
}

/// `caps_rows` with words among the keys; what they do starts `gap` after
/// the widest keys.
fn key_rows(ui: &mut egui::Ui, p: &Palette, rows: &[(Vec<KeyPart>, &str)], gap: f32) {
    let font = egui::FontId::proportional(11.0);
    let pieces: Vec<Option<Vec<Piece>>> = rows
        .iter()
        .map(|(parts, _)| {
            (!parts.is_empty()).then(|| {
                parts
                    .iter()
                    .map(|part| match part {
                        KeyPart::Key(b) => Piece::Caps(keycap::caps(ui, b, p.panel)),
                        KeyPart::Text(t) => Piece::Text(ui.painter().layout_no_wrap((*t).to_owned(), font.clone(), p.muted)),
                    })
                    .collect()
            })
        })
        .collect();
    let width = |row: &[Piece]| row.iter().map(Piece::width).sum::<f32>() + keycap::GAP * row.len().saturating_sub(1) as f32;
    let column = pieces.iter().flatten().map(|r| width(r)).fold(0.0f32, f32::max);
    for ((_, label), row) in rows.iter().zip(pieces) {
        let Some(row) = row else {
            continue;
        };
        let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), KEY_ROW_H), Sense::hover());
        let y = rect.center().y;
        let mut x = rect.left();
        for piece in row {
            let w = piece.width();
            match piece {
                Piece::Caps(caps) => keycap::paint_row(ui, p, &caps, egui::pos2(x, y), p.panel),
                Piece::Text(galley) => {
                    let h = galley.size().y;
                    ui.painter().galley(egui::pos2(x, y - h / 2.0), galley, p.muted);
                }
            }
            x += w + keycap::GAP;
        }
        ui.painter().text(
            egui::pos2(rect.left() + column + gap, y),
            egui::Align2::LEFT_CENTER,
            *label,
            font.clone(),
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

    /// The free camera's tab against the mockup (maquette-camera, variant
    /// A, third state), measured in the browser: given the mockup's own
    /// sizes (its width, its button and text, its 18-pixel keys), the
    /// layout puts everything where the mockup does, to the pixel.
    #[test]
    fn the_free_camera_tab_is_laid_out_like_the_mockup() {
        let close = |a: f32, b: f32| (a - b).abs() <= 1.0;
        // the banner, 274 wide, its button 125.22 wide, its two lines of
        // text 44.84 high (positions from the banner's top left corner)
        assert!(close(Banner::text_width(274.0, 125.22), 94.78));
        let b = Banner::new(Pos2::ZERO, 274.0, 125.22, 44.84);
        assert!(close(b.rect.height(), 60.84), "{:?}", b.rect);
        assert!(close(b.icon.left(), 10.0) && close(b.icon.top(), 21.42), "{:?}", b.icon);
        assert!(close(b.text.x, 36.0) && close(b.text.y, 8.0), "{:?}", b.text);
        // (the mockup's button is 28 high, ours BUTTON_H: the same centre)
        assert!(
            close(b.button.left(), 138.78) && close(b.button.center().y, 16.42 + 14.0),
            "{:?}",
            b.button
        );
        assert!(close(b.button.right(), 264.0));
        // the column between the joysticks: from the top of its 112 pixels
        let c = FreeColumn::new(egui::pos2(13.0, 56.0), Some(18.0));
        assert!(close(c.up.top(), 0.0) && close(c.up.width(), 26.0) && close(c.up.left(), 0.0));
        let (up, down) = c.keys.expect("keys");
        assert!(close(up.y, 30.0 + 9.0) && close(down.y, 64.0 + 9.0), "{up:?} {down:?}");
        assert!(close(c.down.top(), 86.0) && close(c.down.bottom(), 112.0));
        // with our keys it spans the joysticks and their names exactly
        let block = Rect::from_min_size(Pos2::ZERO, Vec2::new(INNER, STICK + CAPTION_H));
        let ours = FreeColumn::new(block.center(), Some(keycap::HEIGHT));
        assert!(close(ours.up.top(), block.top()) && close(ours.down.bottom(), block.bottom()));
        // minimized, the two buttons on the middle of the same block
        let two = FreeColumn::new(block.center(), None);
        assert!(two.keys.is_none());
        assert!(close((two.up.top() + two.down.bottom()) / 2.0, block.center().y));
        assert!(close(two.down.top() - two.up.bottom(), ROW_GAP));
        // the speed row: "Vitesse" 43.39 wide, its value box 44.19
        let row = Rect::from_min_size(Pos2::ZERO, Vec2::new(274.0, 21.0));
        let f = FieldRow::new(row, 43.39, 44.19);
        assert!(close(f.slider.left(), 49.39) && close(f.slider.width(), 174.42), "{:?}", f.slider);
        assert!(close(f.value.left(), 229.81) && close(f.value.right(), 274.0));
    }

    #[test]
    fn the_free_camera_joysticks_start_past_their_dead_zone() {
        assert_eq!(past_dead_zone(Vec2::new(0.2, 0.0)), Vec2::ZERO);
        let half = past_dead_zone(Vec2::new(0.0, (1.0 + DEAD_ZONE) / 2.0));
        assert!((half.y - 0.5).abs() < 1e-5 && half.x == 0.0);
        let full = past_dead_zone(Vec2::new(-1.0, 0.0));
        assert!((full.x + 1.0).abs() < 1e-5);
    }

    #[test]
    fn the_speed_reads_and_types_in_metres_a_second() {
        // (in the viewer's language: "0.5" or "0,5")
        assert!(speed_text(2.0).starts_with("2 "), "{}", speed_text(2.0));
        assert!(speed_text(0.5).starts_with('0') && speed_text(0.5).contains('5'));
        assert!(speed_text(12.4).starts_with("12 "), "whole metres from 10");
        assert!(speed_text(2.0).ends_with("m/s"));
        assert_eq!(parse_speed("3 m/s"), Some(3.0));
        assert_eq!(parse_speed(" 12"), Some(12.0));
        assert_eq!(parse_speed("vite"), None);
    }

    /// The saved positions' menu keeps its width once open, a long name
    /// included: its rows used to push their delete button out of the menu,
    /// which widened by that much every frame, off the screen (#718).
    #[test]
    fn the_saved_positions_menu_keeps_its_width() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let keys = KeyBindings::default();
        let shot = |name: &str| CameraShot::at_avatar(name.to_owned(), [-3.0, 0.0, 1.0], [0.0, 0.0, 0.5]);
        // one clock for both menus: egui's time never goes back
        let mut time = 0.0;
        for shots in [
            vec![shot("Close up"), shot("Wide")],
            vec![shot("Close up"), shot("A rather long name for a saved camera position")],
        ] {
            let mut controls = ControlsUi::default();
            let view = View {
                zoom: 0.5,
                preset: CameraPreset::Rear,
                shots: &shots,
                flying: false,
                always_run: false,
                seated: false,
                mouselook: false,
                object_view: false,
                keys: &keys,
                free: None,
            };
            let mut frame = |events| {
                time += 0.1;
                let mut out = Output::default();
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                        time: Some(time),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        ui.allocate_ui(Vec2::new(INNER, 200.0), |ui| {
                            controls.shots(ui, &p, &view, &mut out, &ButtonGrid::new(INNER));
                        });
                    },
                );
                output.textures_delta.clear();
                output
            };
            let output = frame(Vec::new());
            let field = output
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Text(t) if t.galley.text() == aurora_i18n::tr!("controls-shots") => Some(t.pos + t.galley.size() * 0.5),
                    _ => None,
                })
                .expect("the saved positions' field");
            for pressed in [true, false] {
                frame(vec![
                    egui::Event::PointerMoved(field),
                    egui::Event::PointerButton {
                        pos: field,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]);
            }
            let menu = |ctx: &egui::Context| {
                ctx.memory(|m| {
                    let layer = m.areas().top_layer_id(egui::Order::Foreground).expect("the menu is open");
                    m.area_rect(layer.id).expect("the menu's rect")
                })
            };
            for _ in 0..4 {
                frame(Vec::new());
            }
            let settled = menu(&ctx);
            for _ in 0..20 {
                frame(Vec::new());
            }
            let later = menu(&ctx);
            assert_eq!(settled.width(), later.width(), "{settled:?} then {later:?}");
            assert!(later.right() <= 800.0, "{later:?}");
        }
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
