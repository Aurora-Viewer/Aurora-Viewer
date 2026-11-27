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
use crate::camera::{CameraPreset, CameraShot, SavedViews, free};
use crate::keybinds::{Action, Binding, KeyBindings};
use crate::theme::Palette;
use egui::{FontId, Pos2, Rect, Sense, Vec2};

/// The Aurora window, and Firestorm's two.
pub const WINDOW: &str = "controls";
pub const CLASSIC_CAMERA: &str = "camera_controls";
pub const CLASSIC_MOVEMENT: &str = "movement_controls";
/// The Classic style's small camera window (the Legacy Viewer's
/// fs_camera_floater_small, its own window with its own place, as there).
pub const CLASSIC_CAMERA_SMALL: &str = "camera_controls_small";

const WIDTH: f32 = 300.0;
/// The body's width inside the window's margins (`widgets::Floater`).
#[cfg(test)]
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
// The Classic style's small camera window, as floater_fs_camera_small.xml
// lays it out in 165 × 77: a row of 18 px buttons on top (the four views,
// then object view, mouselook and reset view), the orbit joystick, the
// zoom's column and the pan joystick under it, 50 px tall, the roll
// buttons in the orbit joystick's bottom corners.
const SMALL_WIDTH: f32 = 165.0;
/// The window's least size, as it opens: floater_fs_camera_small.xml's
/// min_width and min_height.
const SMALL_MIN: Vec2 = Vec2::new(SMALL_WIDTH, 77.0);
/// Its body: the window less its margins and its border.
const SMALL_BODY: f32 = SMALL_WIDTH - 2.0 * SMALL_INSET;
const SMALL_PAD: f32 = 2.0;
/// Where the body starts in the window: its margin and its border.
const SMALL_INSET: f32 = SMALL_PAD + 1.0;
/// The top row's 22 px take the Legacy window's top (its buttons at 0, its
/// joysticks at 25), so 1 px above them where it has none.
const SMALL_MARGIN: egui::Margin = egui::Margin {
    left: SMALL_PAD as i8,
    right: SMALL_PAD as i8,
    top: 1,
    bottom: SMALL_PAD as i8,
};
const SMALL_BUTTON: f32 = 18.0;
const SMALL_BUTTON_GAP: f32 = 2.0;
/// The top row (the buttons and the room under them).
const SMALL_ROW: f32 = 22.0;
const SMALL_STICK: f32 = 50.0;
/// The least room around each joystick and the zoom of an enlarged small
/// window (the default one has 10.75).
const SMALL_STICK_GAP: f32 = 4.0;
const SMALL_ZOOM_W: f32 = 16.0;
const SMALL_ZOOM_BUTTON: f32 = 14.0;
/// The small zoom slider's thumb.
const SMALL_ZOOM_THUMB: Vec2 = Vec2::new(10.0, 4.0);
/// Pixels of drag on the small zoom slider for the whole zoom range.
const SMALL_ZOOM_TRAVEL: f32 = 200.0;
/// An arrow key or a wheel notch on the small zoom slider: LLF32UICtrl's
/// default increment (LLSlider::handleKeyHere, handleScrollWheel).
const SMALL_ZOOM_STEP: f32 = 0.1;
const SMALL_ROLL: f32 = 12.0;
/// Room between two items of a row of buttons.
const ROW_GAP: f32 = 4.0;
const BUTTON_H: f32 = 26.0;
/// Width of `widgets::icon_button` (the delete button of a saved position,
/// the classic window's buttons after its preset list).
const ICON_BUTTON_W: f32 = 20.0;
/// Room between the body's rows.
const BODY_GAP: f32 = 8.0;
/// The camera position window's button, in both styles.
const POSITION_ICON: &str = "adjustments-horizontal-filled";
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

/// The compact choice of each tab (#3904): `Settings::compact_controls`
/// and `Settings::compact_movement_controls`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Compact {
    pub camera: bool,
    pub movement: bool,
}

impl Compact {
    fn of(self, tab: Tab) -> bool {
        match tab {
            Tab::Camera => self.camera,
            Tab::Movement => self.movement,
        }
    }

    fn set(&mut self, tab: Tab, on: bool) {
        match tab {
            Tab::Camera => self.camera = on,
            Tab::Movement => self.movement = on,
        }
    }
}

#[derive(Default)]
pub struct ControlsUi {
    pub tab: Tab,
    /// The flags of the last frame, to see which one was just raised.
    was: (bool, bool),
    /// The tab shown on the last frame, and whether in the free camera
    /// (`None`: closed); a tab turned to shows in its saved form.
    shown: Option<(Tab, bool)>,
    /// The saved compact choices of the last frame, to apply a change made
    /// in Preferences while the window is open.
    compact_seen: Option<Compact>,
    naming: ShotNaming,
}

/// The name being typed for a new camera position (the Legacy Viewer's
/// LLFloaterSaveCameraPreset, inline).
#[derive(Default)]
pub(crate) struct ShotNaming {
    pub(crate) open: bool,
    /// Give the field the keyboard once, on its next frame. A request on
    /// every frame interrupts the input method each time (egui's
    /// `Memory::request_focus`, which egui-winit turns into
    /// `set_ime_allowed` off then on): a Japanese name could not be
    /// composed and the IME stalled the viewer (#3889).
    focus: bool,
    pub(crate) name: String,
}

impl ShotNaming {
    pub(crate) fn start(&mut self) {
        self.open = true;
        self.focus = true;
    }
}

/// What the window shows of the world.
pub struct View<'a> {
    /// LLAgentCamera::getCameraZoomFraction (1 all the way in).
    pub zoom: f32,
    /// Camera roll in radians.
    pub roll: f32,
    pub preset: CameraPreset,
    pub shots: &'a [CameraShot],
    /// The views the user saved over their templates.
    pub views: &'a SavedViews,
    pub flying: bool,
    pub always_run: bool,
    pub seated: bool,
    /// What is greyed, as LLFloaterMove::sUpdateMovementStatus and
    /// LLPanelStandStopFlying::onStandButtonClick (indra/newview/
    /// llmoveview.cpp) decide it: the Run mode under RLVa's @alwaysrun, the
    /// Fly mode where no takeoff is allowed (LLAgent::canFly; never while
    /// flying, so that it still lands), Stand up under @unsit.
    pub run_locked: bool,
    pub fly_locked: bool,
    pub stand_locked: bool,
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
    RollBy(f32),
    LevelRoll,
    Mouselook,
    ObjectView,
    Walk,
    Run,
    Fly,
    StandUp,
    UseShot(usize),
    SaveShot(String),
    DeleteShot(usize),
    /// The camera where it is becomes this view (a view's right click).
    SaveView(CameraPreset),
    /// The view back to its template.
    RestoreView(CameraPreset),
    /// "Position...": the camera position window (`ui::camera_position`).
    Position,
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

/// The Legacy Viewer's camera panel uses icon-only preset cells
/// (skins/default/xui/en/floater_camera.xml). Aurora also names them:
/// measure those labels instead of assuming the default font still fits.
fn controls_width(ctx: &egui::Context, view: &View, classic: bool) -> f32 {
    let text = |label: &str, font: FontId| ctx.fonts_mut(|f| f.layout_no_wrap(label.to_owned(), font, egui::Color32::PLACEHOLDER).size().x);
    let regular = |label: &str, size| text(label, FontId::proportional(size));
    let button_height = (ctx.fonts_mut(|f| f.row_height(&FontId::proportional(12.5))) + 8.0).max(BUTTON_H);
    let shot = [
        aurora_i18n::tr!("controls-shots-none"),
        aurora_i18n::tr!("controls-shots"),
        aurora_i18n::tr!("controls-use-preset"),
    ]
    .into_iter()
    .map(|label| regular(label, 12.5))
    .fold(0.0_f32, f32::max)
        + 46.0
        // the Position and save squares
        + 2.0 * (button_height + ROW_GAP);
    let tabs = if classic {
        0.0
    } else {
        2.0 * tab_min_width(ctx, view.free.is_some()) + ROW_GAP
    };
    (if classic { 250.0_f32 } else { WIDTH }).max(tabs + 24.0).max(shot + 24.0)
}

fn tab_min_width(ctx: &egui::Context, free: bool) -> f32 {
    let camera = if free {
        aurora_i18n::tr!("controls-free-title")
    } else {
        aurora_i18n::tr!("controls-tab-camera")
    };
    let font = FontId::new(12.5, crate::ui::fonts::semibold(ctx));
    [camera, aurora_i18n::tr!("controls-tab-movement")]
        .into_iter()
        .map(|label| {
            ctx.fonts_mut(|f| {
                f.layout_no_wrap(label.to_owned(), font.clone(), egui::Color32::PLACEHOLDER)
                    .size()
                    .x
            }) + 44.0
        })
        .fold(0.0_f32, f32::max)
}

/// A button's height at the interface's text size (also the camera
/// position window's).
pub(crate) fn control_height(ui: &egui::Ui) -> f32 {
    (ui.painter()
        .layout_no_wrap("Ag".into(), widgets::control_font(ui, 12.5), egui::Color32::PLACEHOLDER)
        .size()
        .y
        + 8.0)
        .max(BUTTON_H)
}

impl ControlsUi {
    /// The Aurora window (`camera` / `movement`: the toolbar's View and
    /// Movement, the World menu's entries; either opens it on its tab), or
    /// the Legacy Viewer's two windows when `classic`. `compact` is the saved
    /// compact choice of each tab (`Compact`): the title bar's compact /
    /// full button (or a double-click on the title bar, as on every window)
    /// sets the shown tab's, and a tab shows that way.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        camera: &mut bool,
        movement: &mut bool,
        compact: &mut Compact,
        classic: bool,
        view: &View,
    ) -> Output {
        let mut out = Output::default();
        // the free camera keeps Aurora's window (Firestorm has none)
        if classic && view.free.is_none() {
            if *camera && compact.camera {
                // the Legacy Viewer's small camera window (#677 on the
                // feedback board): no title bar, moved by its background,
                // closed by its toolbar button or menu entry (Ctrl+W
                // passes over it, as `without_close` says). It is enlarged
                // by its edges from its default size, as the Legacy
                // Viewer's (can_resize, 165×77 at least; #4840)
                let size = Vec2::new(SMALL_WIDTH, SMALL_ROW + SMALL_STICK);
                let mut small = Floater::new(
                    CLASSIC_CAMERA_SMALL,
                    aurora_i18n::tr!("controls-camera-title"),
                    Pos2::new(60.0, 160.0),
                    size,
                )
                .without_close()
                .margin(SMALL_MARGIN)
                .steady_size();
                // the outline's least size, its margins and border included:
                // the body's size let it go 5 px shorter than it opens
                small.min_size = SMALL_MIN;
                small.show(ctx, p, camera, |ui| small_camera(ui, p, view, &mut out));
            } else if *camera {
                let size = Vec2::new(controls_width(ctx, view, true), 210.0);
                Floater::new(
                    CLASSIC_CAMERA,
                    aurora_i18n::tr!("controls-camera-title"),
                    Pos2::new(60.0, 160.0),
                    size,
                )
                .auto_height()
                .show(ctx, p, camera, |ui| classic_camera(ui, p, view, &mut self.naming, &mut out));
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
            self.shown = None;
            self.compact_seen = Some(*compact);
            return out;
        }
        sync_tab(self, camera, movement);
        if view.free.is_some() {
            self.tab = Tab::Camera;
        }
        let mut open = *camera || *movement;
        if open {
            // #3904: compact is a form each tab keeps, chosen by the title
            // button or in Preferences, not a minimize forgotten on closing:
            // a tab shows in its saved form whenever it is turned to (the
            // toolbar, a menu, its tab, an opening) or its choice changes in
            // Preferences. A difference from the Legacy Viewer, whose small
            // camera window is a separate floater behind a setting
            // (FSUseSmallCameraFloater, floater_fs_camera_small.xml): here it
            // stays the one window, and its title button switches it.
            // The free camera's compact window is its own.
            let free = view.free.is_some();
            let saved = compact.of(self.tab);
            let entered = self.shown != Some((self.tab, free));
            let switched = self.compact_seen.is_some_and(|seen| seen.of(self.tab) != saved);
            // a "Reset window positions and sizes" made while it was closed
            // expands it at this opening, after the saved form is applied:
            // applied again at the next frame
            let reset_on_opening = entered && crate::ui::layout::reset_pending(ctx, egui::Id::unique(WINDOW));
            if (entered || switched) && !free {
                // the Movement button's expand (#1815) yields to a compact
                // movement saved
                let minimized = widgets::floater_minimized(ctx, WINDOW);
                if saved {
                    widgets::cancel_expand(ctx, WINDOW);
                    if !minimized {
                        widgets::minimize_floater(ctx, WINDOW);
                    }
                } else if minimized {
                    // an expand asked for, so not the title button
                    widgets::expand_floater(ctx, WINDOW);
                }
            }
            let shown = self.tab;
            let screen = ctx.content_rect();
            let minimized = widgets::floater_minimized(ctx, WINDOW);
            // an expand asked for elsewhere (the Movement button, the photo
            // window's Controls, a window reset) is not the title button
            let asked = widgets::floater_expand_pending(ctx, WINDOW);
            // minimized, the free camera names the window
            let title = match view.free {
                Some(_) if minimized => aurora_i18n::tr!("controls-free-title"),
                _ => aurora_i18n::tr!("controls-title"),
            };
            Floater::new(
                WINDOW,
                title,
                Pos2::new(screen.left() + 24.0, screen.bottom() - 460.0),
                Vec2::new(controls_width(ctx, view, false), 420.0),
            )
            .fixed()
            .auto_height()
            .compact_when_minimized(COMPACT_WIDTH + 20.0)
            .show(ctx, p, &mut open, |ui| {
                // minimized: the camera's joysticks, zoom and roll (the
                // mockup's compact window), the movement pad, or the free
                // camera's joysticks, speed and way back
                if widgets::floater_minimized(ui.ctx(), WINDOW) {
                    match (view.free, self.tab) {
                        (Some(free), _) => free_compact(ui, p, view, free, &mut out),
                        // the Movement tab compact: the Legacy Viewer's
                        // movement window, the pad and walk / run / fly
                        (None, Tab::Movement) => classic_movement(ui, p, view, &mut out),
                        (None, Tab::Camera) => {
                            sticks(ui, p, view, &mut out, COMPACT_WIDTH);
                            roll_controls(ui, p, view, &mut out);
                        }
                    }
                } else {
                    self.body(ui, p, view, &mut out);
                }
            });
            // the title button switched it (or a double-click on the title
            // bar, which every window takes as its button): the one change
            // kept as the choice (a close expands it too; an expand asked for elsewhere
            // and the free camera's own compact window are not choices of
            // the camera's form)
            let now = widgets::floater_minimized(ctx, WINDOW);
            let by_title = open && now != minimized && !asked;
            if let Some(chosen) = compact_choice(by_title, now, free) {
                compact.set(shown, chosen);
            }
            // a tab just clicked shows in its form from the next frame; closed
            // here (its cross, a close asked for), it opens again as entered
            self.shown = (open && !reset_on_opening).then_some((shown, free));
        } else {
            self.shown = None;
        }
        if open {
            *camera = self.tab == Tab::Camera;
            *movement = self.tab == Tab::Movement;
        } else {
            *camera = false;
            *movement = false;
        }
        self.was = (*camera, *movement);
        self.compact_seen = Some(*compact);
        out
    }

    fn body(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
        ui.spacing_mut().item_spacing.y = BODY_GAP;
        let width = ui.available_width();
        let tab_min = tab_min_width(ui.ctx(), view.free.is_some());
        let tab_width = if width >= 2.0 * tab_min + ROW_GAP {
            (width - ROW_GAP) / 2.0
        } else {
            width
        };
        // Two equal tabs, stacked if the screen cannot fit both labels;
        // the free camera takes the first and greys the other.
        let free = view.free.is_some();
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(ROW_GAP);
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
                if widgets::pill_tab_wide(ui, p, icon, label, self.tab == tab, tab_width, enabled, why).clicked() {
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
        let width = ui.available_width();
        sticks(ui, p, view, out, width);
        roll_controls(ui, p, view, out);
        // the four views of Preferences › Camera, in one click
        let presets = [
            (CameraPreset::Rear, "user"),
            (CameraPreset::Front, "user-circle"),
            (CameraPreset::Side, "user-square"),
            (CameraPreset::Shoulder, "user-scan"),
        ];
        let widest = presets
            .iter()
            .map(|(preset, _)| {
                ui.painter()
                    .layout_no_wrap(short_view(*preset).to_owned(), FontId::proportional(11.0), p.ink)
                    .size()
                    .x
                    + 16.0
            })
            .fold(0.0_f32, f32::max);
        let columns = ((width + ROW_GAP) / (widest + ROW_GAP)).floor().clamp(1.0, 4.0) as usize;
        let chip_width = (width - ROW_GAP * (columns - 1) as f32) / columns as f32;
        for row in presets.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = ROW_GAP;
                for &(preset, icon) in row {
                    let chip = view_chip(ui, p, icon, short_view(preset), view.preset == preset, chip_width);
                    view_menu(&chip, p, view, preset, out);
                    if chip.on_hover_text(view_tip(preset)).clicked() {
                        out.actions.push(ControlAction::Preset(preset));
                    }
                }
            });
        }
        // Equal action buttons wrap before their text would collide. The
        // saved positions below still fill the row up to its square button.
        let height = control_height(ui);
        let grid = ButtonGrid::with_height(width, height);
        let key = pad_binding(view.keys, Action::Mouselook);
        let button_min = widgets::action_button_keyed_content(ui, "box", aurora_i18n::tr!("controls-object-view"), None).max(
            widgets::action_button_keyed_content(ui, "eye-filled", aurora_i18n::tr!("common-mouselook"), key.as_ref()),
        ) + 12.0;
        let button_width = grid.button.max(button_min).min(width);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = Vec2::splat(ROW_GAP);
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
                Vec2::new(button_width, height),
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
                Vec2::new(button_width, height),
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

    /// The saved camera positions (Firestorm's camera presets): a field,
    /// the Position button, and the save button under the reset.
    fn shots(&mut self, ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output, grid: &ButtonGrid) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = ROW_GAP;
            if self.naming.open {
                if let Some(name) = name_row(ui, p, &mut self.naming, grid.field) {
                    out.actions.push(ControlAction::SaveShot(name));
                }
                return;
            }
            let text = if view.shots.is_empty() {
                aurora_i18n::tr!("controls-shots-none")
            } else {
                aurora_i18n::tr!("controls-shots")
            };
            let height = control_height(ui);
            let field_width = grid.field - height - ROW_GAP;
            let field = widgets::action_button(
                ui,
                p,
                "bookmark-filled",
                text,
                widgets::ActionKind::Flat,
                Vec2::new(field_width, height),
            );
            egui::Popup::menu(&field).show(|ui| {
                ui.set_min_width(field_width);
                for (i, shot) in view.shots.iter().enumerate() {
                    ui.horizontal(|ui| {
                        // the row takes the menu's width less the delete
                        // button: a full-width row pushed the button out and
                        // the menu widened by that much every frame (#718)
                        let room = ui.available_width() - ICON_BUTTON_W - ui.spacing().item_spacing.x;
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
            // camera_position_btn of floater_camera.xml ("Position...")
            if square_button(ui, p, POSITION_ICON, aurora_i18n::tr!("controls-position-tip")) {
                out.actions.push(ControlAction::Position);
            }
            if square_button(ui, p, "device-floppy-filled", aurora_i18n::tr!("controls-shot-save-tip")) {
                self.naming.start();
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
    #[cfg(test)]
    pub fn new(width: f32) -> ButtonGrid {
        Self::with_height(width, BUTTON_H)
    }

    fn with_height(width: f32, height: f32) -> ButtonGrid {
        let button = (width - height - 2.0 * ROW_GAP) / 2.0;
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
        SticksRow::sized(row, STICK, ZOOM_W)
    }

    /// Joysticks `stick` wide and a zoom `zoom_w` wide (the Classic
    /// style's small camera window), spread as `new` spreads them.
    pub fn sized(row: Rect, stick: f32, zoom_w: f32) -> SticksRow {
        let gap = (row.width() - 2.0 * stick - zoom_w) / 4.0;
        let y = row.center().y;
        let at = |left: f32, w: f32| Rect::from_center_size(egui::pos2(left + w / 2.0, y), Vec2::new(w, stick));
        let orbit = at(row.left() + gap, stick);
        let zoom = at(orbit.right() + gap, zoom_w);
        let pan = at(zoom.right() + gap, stick);
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

/// The compact choice the title button just made for the tab shown
/// (`by_title`: the window went `now` compact or full by its title
/// button), kept outside the free camera, whose compact window is its own
/// (#3904).
fn compact_choice(by_title: bool, now: bool, free: bool) -> Option<bool> {
    (by_title && !free).then_some(now)
}

/// A flag just raised opens the window on its tab; otherwise it stays.
/// The camera (`movement` false) or movement controls asked for by a
/// toolbar button or a shortcut (`toggle`, LLFloaterReg::toggleInstance) or
/// a menu item that only opens (`toggle` false, LLFloaterReg::showInstance:
/// the camera presets' Camera controls). Compact, the window shows
/// the tab it is on (#3904): the button of that tab closes it with
/// `toggle`, as it closes the full window; the other tab's button turns
/// to it (#1815), in that tab's saved form (`ControlsUi::show`). `classic`: Legacy's two windows, each its own,
/// the camera's the small one when `small_camera` (Compact camera controls,
/// as the Legacy Viewer's `FSUseSmallCameraFloater`).
pub fn request(
    ctx: &egui::Context,
    camera: &mut bool,
    movement: &mut bool,
    classic: bool,
    small_camera: bool,
    want_movement: bool,
    toggle: bool,
) {
    if classic {
        // the camera's window of the form shown: the full one may be left
        // minimized, which the small one never is
        let (id, open) = if want_movement {
            (CLASSIC_MOVEMENT, movement)
        } else if small_camera {
            (CLASSIC_CAMERA_SMALL, camera)
        } else {
            (CLASSIC_CAMERA, camera)
        };
        if toggle {
            widgets::toggle_floater(ctx, id, open);
        } else {
            widgets::open_floater(ctx, id, open);
        }
        return;
    }
    let other = if want_movement { *camera } else { *movement };
    let open = if want_movement { movement } else { camera };
    let compact = widgets::floater_minimized(ctx, WINDOW);
    if *open {
        if compact && toggle {
            // as its cross does: it opens again as the saved choice says,
            // not as it was left
            widgets::close_floater(ctx, WINDOW);
        } else if compact {
            // shown: to the front, compact still
            ctx.move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(WINDOW)));
        } else if toggle {
            *open = false;
        }
    } else if other {
        // (`ControlsUi::show` turns to the tab just asked for, in its form)
        *open = true;
        if compact && want_movement {
            widgets::expand_floater(ctx, WINDOW);
        }
    } else {
        widgets::open_floater(ctx, WINDOW, open);
    }
}

fn sync_tab(ui: &mut ControlsUi, camera: &mut bool, movement: &mut bool) {
    if *movement && !ui.was.1 {
        ui.tab = Tab::Movement;
    } else if *camera && !ui.was.0 {
        ui.tab = Tab::Camera;
    }
}

/// LLPanelCameraZoom::getOrbitRate / onRollLeftHeldDown / onRollRightHeldDown
/// (indra/newview/llfloatercamera.cpp): a gentle nudge ramps over 250 ms.
/// LLAgentCamera::updateCamera uses 45 degrees per second at full speed.
fn roll_controls(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
    ui.add_enabled_ui(!view.mouselook, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = ROW_GAP;
            let height = control_height(ui);
            let readout_width = ui.available_width() - 2.0 * (height + ROW_GAP);
            for (icon, label, direction) in [
                ("rotate-2", aurora_i18n::tr!("controls-roll-left"), -1.0),
                ("rotate-clockwise-2", aurora_i18n::tr!("controls-roll-right"), 1.0),
            ] {
                if direction > 0.0 {
                    let degrees = (view.roll.to_degrees() + 180.0).rem_euclid(360.0) - 180.0;
                    let angle = format!("{}°", aurora_i18n::number(f64::from(degrees), 1));
                    let text = aurora_i18n::tr!("controls-roll-angle", angle = angle.as_str());
                    if widgets::action_button(
                        ui,
                        p,
                        "tilt-shift-filled",
                        &text,
                        widgets::ActionKind::Flat,
                        Vec2::new(readout_width, height),
                    )
                    .on_hover_text(aurora_i18n::tr!("controls-roll-level"))
                    .clicked()
                    {
                        out.actions.push(ControlAction::LevelRoll);
                    }
                }
                let response = widgets::action_button(ui, p, icon, "", widgets::ActionKind::Flat, Vec2::splat(height))
                    // A sustained press belongs to this control, including pointer
                    // jitter: a click-only button lets the floater acquire its drag.
                    .interact(Sense::click_and_drag())
                    .on_hover_text(label);
                response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
                roll_held(ui, &response, direction, out);
            }
        });
    });
}

/// A roll button held: the camera rolls `direction` (-1 left, 1 right),
/// slowly at first, at full speed after 250 ms (`roll_controls`).
fn roll_held(ui: &egui::Ui, response: &egui::Response, direction: f32, out: &mut Output) {
    let id = response.id.with("roll-start");
    let now = ui.input(|i| i.time);
    if ui.is_enabled() && response.is_pointer_button_down_on() {
        let start = ui.ctx().data_mut(|d| *d.get_temp_mut_or_insert_with(id, || now));
        let rate = 0.05 + ((now - start) as f32 / 0.25).clamp(0.0, 1.0) * 0.95;
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        out.actions
            .push(ControlAction::RollBy(direction * rate * 45.0_f32.to_radians() * dt));
        ui.ctx().request_repaint();
    } else {
        ui.ctx().data_mut(|d| d.remove::<f64>(id));
    }
}

/// The orbit and pan joysticks, with zoom between them.
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
    let captions = stick_captions(
        ui,
        p,
        &at,
        row.bottom(),
        [aurora_i18n::tr!("controls-orbit"), aurora_i18n::tr!("controls-pan")],
    );
    ui.allocate_rect(Rect::from_min_size(row.min, Vec2::new(width, STICK + captions)), Sense::hover());
}

/// Captions stay in their own joystick's column and reserve their actual
/// height, including wrapped lines with large fonts or long translations.
fn stick_captions(ui: &egui::Ui, p: &Palette, at: &SticksRow, top: f32, names: [&str; 2]) -> f32 {
    let mut height = CAPTION_H;
    // Each caption also owns the gaps on both sides of its joystick:
    // these columns end at the zoom rail and at the row's outer edges.
    let gap = (at.zoom.left() - at.orbit.right()).max(0.0);
    for (stick, name) in [at.orbit, at.pan].into_iter().zip(names) {
        let galley = ui
            .painter()
            .layout(name.to_owned(), FontId::proportional(11.0), p.muted, stick.width() + 2.0 * gap);
        height = height.max(galley.size().y + ROW_GAP);
        let pos = egui::pos2(stick.center().x - galley.size().x / 2.0, top + ROW_GAP / 2.0);
        ui.painter().galley(pos, galley, p.muted);
    }
    height
}

/// The free camera's tab (the mockup's third state): the banner and its way
/// back, the move and look joysticks with up / down between them, the
/// speed, the collisions and the keys.
fn free_tab(ui: &mut egui::Ui, p: &Palette, view: &View, free: FreeView, out: &mut Output) {
    banner(ui, p, view, out);
    ui.spacing_mut().item_spacing.y = FREE_GAP;
    free_sticks(ui, p, view, out, ui.available_width(), true);
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
    let captions = stick_captions(ui, p, &at, row.bottom(), [walk, aurora_i18n::tr!("controls-free-look")]);
    ui.allocate_rect(Rect::from_min_size(block.min, Vec2::new(width, STICK + captions)), Sense::hover());
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
        let w = (ui.available_width() - ROW_GAP) / 2.0;
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
            view.seated && !view.stand_locked,
        );
        if stand.clicked() && view.seated && !view.stand_locked {
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
    let drag_id = ui.scope_id().with("movement-pad-press");
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
                if pad_button(ui, p, icon, &tip, binding.as_ref(), drag_id) {
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
            let enabled = !mode_locked(view, &action);
            if mode_button(ui, p, icon, label, n == mode, enabled, w).clicked() && n != mode && enabled {
                mode = n;
                out.actions.push(action);
            }
        }
    });
}

/// Whether a movement mode's button is greyed (`View::run_locked`,
/// `View::fly_locked`); Walk never is.
fn mode_locked(view: &View, action: &ControlAction) -> bool {
    match action {
        ControlAction::Run => view.run_locked,
        ControlAction::Fly => view.fly_locked && !view.flying,
        _ => false,
    }
}

/// Firestorm's camera window: views on top, the joysticks and the zoom,
/// then its presets (floater_camera.xml).
fn classic_camera(ui: &mut egui::Ui, p: &Palette, view: &View, naming: &mut ShotNaming, out: &mut Output) {
    ui.spacing_mut().item_spacing.y = 6.0;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(3.0, ROW_GAP);
        for preset in [CameraPreset::Front, CameraPreset::Side, CameraPreset::Rear, CameraPreset::Shoulder] {
            let button = widgets::flat_button(ui, p, short_view(preset));
            view_menu(&button, p, view, preset, out);
            if button.on_hover_text(view_tip(preset)).clicked() {
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
    roll_controls(ui, p, view, out);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if naming.open {
            let width = ui.available_width() - control_height(ui) - ROW_GAP;
            if let Some(name) = name_row(ui, p, naming, width) {
                out.actions.push(ControlAction::SaveShot(name));
            }
            return;
        }
        egui::ComboBox::from_id_salt("camera_shots_classic")
            // room for the Position, save and reset buttons after it
            .width(ui.available_width() - 76.0 - ICON_BUTTON_W - 4.0)
            .selected_text(aurora_i18n::tr!("controls-use-preset"))
            .show_ui(ui, |ui| {
                for (i, shot) in view.shots.iter().enumerate() {
                    if ui.selectable_label(false, &shot.name).clicked() {
                        out.actions.push(ControlAction::UseShot(i));
                    }
                }
            });
        // camera_position_btn of floater_camera.xml ("Position...")
        if widgets::icon_button(ui, p, POSITION_ICON, aurora_i18n::tr!("controls-position-tip"), true) {
            out.actions.push(ControlAction::Position);
        }
        // save_preset_btn of floater_camera.xml ("Save as Preset")
        if widgets::icon_button(ui, p, "device-floppy-filled", aurora_i18n::tr!("controls-shot-save-tip"), true) {
            naming.start();
        }
        if widgets::icon_button(ui, p, "restore", aurora_i18n::tr!("controls-reset-camera"), true) {
            out.actions.push(ControlAction::ResetCamera);
        }
    });
}

/// The Legacy Viewer's small camera window (floater_fs_camera_small.xml),
/// laid out to its pixels: the four views, object view, mouselook and
/// reset view on top, then the orbit joystick with the roll buttons in its
/// bottom corners, the zoom's column and the pan joystick.
fn small_camera(ui: &mut egui::Ui, p: &Palette, view: &View, out: &mut Output) {
    // the whole body of the window, enlarged or not (#4840)
    let room = ui.available_size().max(Vec2::new(SMALL_BODY, SMALL_ROW + SMALL_STICK));
    let (body, _) = ui.allocate_exact_size(room, Sense::hover());
    let at = SmallCamera::sized(body.min, body.size());
    let presets = [
        (CameraPreset::Front, "user-circle"),
        (CameraPreset::Side, "user-square"),
        (CameraPreset::Rear, "user"),
        (CameraPreset::Shoulder, "user-scan"),
    ];
    for (rect, (preset, icon)) in at.views.into_iter().zip(presets) {
        let button = small_button(ui, p, rect, icon, &view_tip(preset), view.preset == preset);
        view_menu(&button, p, view, preset, out);
        if button.clicked() {
            out.actions.push(ControlAction::Preset(preset));
        }
    }
    let [object, mouselook, reset] = at.tools;
    if small_button(ui, p, object, "box", aurora_i18n::tr!("controls-object-view-tip"), view.object_view).clicked() {
        out.actions.push(ControlAction::ObjectView);
    }
    let look = match pad_binding(view.keys, Action::Mouselook) {
        Some(b) => format!("{} ({})", aurora_i18n::tr!("common-mouselook"), b.label()),
        None => aurora_i18n::tr!("common-mouselook").to_owned(),
    };
    if small_button(ui, p, mouselook, "eye-filled", &look, view.mouselook).clicked() {
        out.actions.push(ControlAction::Mouselook);
    }
    if small_button(ui, p, reset, "refresh", aurora_i18n::tr!("controls-reset-view"), false).clicked() {
        out.actions.push(ControlAction::ResetOrbit);
    }
    let orbit = joystick(ui, p, at.sticks.orbit, "rotate-360", aurora_i18n::tr!("controls-orbit-tip"));
    if orbit.center {
        out.actions.push(ControlAction::ResetOrbit);
    }
    out.held.extend(orbit_actions(orbit.dir));
    let pan = joystick(ui, p, at.sticks.pan, "arrows-move", aurora_i18n::tr!("controls-pan-tip"));
    out.held.extend(pan_actions(pan.dir));
    let z = at.sticks.zoom;
    let plus = Rect::from_min_size(
        z.min + Vec2::new((z.width() - SMALL_ZOOM_BUTTON) / 2.0, 0.0),
        Vec2::splat(SMALL_ZOOM_BUTTON),
    );
    let minus = Rect::from_min_size(
        egui::pos2(plus.left(), z.bottom() - SMALL_ZOOM_BUTTON),
        Vec2::splat(SMALL_ZOOM_BUTTON),
    );
    if hold_icon(ui, p, plus, "zoom-in-filled", &key_tip(view.keys, Action::CamZoomIn), false) {
        out.held.push(Action::CamZoomIn);
    }
    if hold_icon(ui, p, minus, "zoom-out-filled", &key_tip(view.keys, Action::CamZoomOut), false) {
        out.held.push(Action::CamZoomOut);
    }
    let rail = Rect::from_min_max(egui::pos2(z.left(), plus.bottom() + 1.0), egui::pos2(z.right(), minus.top() - 1.0));
    if let Some(zoom) = small_zoom(ui, p, rail, view.zoom) {
        out.actions.push(ControlAction::Zoom(zoom));
    }
    // after the orbit joystick, so that a press in a corner rolls; greyed
    // in mouselook, as the full window's, still taking the press there
    // rather than letting it orbit
    for (rect, icon, label, direction) in [
        (at.roll_left, "rotate-2", aurora_i18n::tr!("controls-roll-left"), -1.0),
        (at.roll_right, "rotate-clockwise-2", aurora_i18n::tr!("controls-roll-right"), 1.0),
    ] {
        let response = small_hold(ui, p, rect, icon, label, !view.mouselook);
        if !view.mouselook {
            roll_held(ui, &response, direction, out);
        }
    }
}

/// The small window's zoom slider: a thin track and a small thumb in the
/// 16 px column. Unlike the Legacy Viewer's slider, which puts the zoom
/// where the pointer is, a drag moves it by the drag over
/// `SMALL_ZOOM_TRAVEL`: on a rail of some 20 px the pointer threw the camera
/// across its whole range. As the Legacy Viewer's, an arrow key (once the
/// slider has focus, by Tab or a press) or a wheel notch over it moves the
/// zoom by `SMALL_ZOOM_STEP`, up and right zooming in. Gives the new zoom
/// fraction (1 zoomed in, at the top).
fn small_zoom(ui: &mut egui::Ui, p: &Palette, rail: Rect, zoom: f32) -> Option<f32> {
    let label = aurora_i18n::tr!("common-zoom");
    let id = ui.scope_id().with("small_zoom");
    let resp = ui.interact(rail, id, Sense::click_and_drag());
    if resp.drag_started() || resp.clicked() {
        resp.request_focus();
    }
    let mut steps = 0.0;
    if resp.has_focus() {
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            );
        });
        steps += ui.input(|i| {
            let up = i.num_presses(egui::Key::ArrowUp) + i.num_presses(egui::Key::ArrowRight);
            let down = i.num_presses(egui::Key::ArrowDown) + i.num_presses(egui::Key::ArrowLeft);
            up as f32 - down as f32
        });
    }
    steps += ui.input(|i| {
        use egui::accesskit::Action;
        i.num_accesskit_action_requests(id, Action::Increment) as f32 - i.num_accesskit_action_requests(id, Action::Decrement) as f32
    });
    steps += widgets::spin_wheel(ui, rail, id, true, true).notches;
    let resp = resp.on_hover_text(label);
    let travel = rail.height() - SMALL_ZOOM_THUMB.y;
    let y = rail.bottom() - SMALL_ZOOM_THUMB.y / 2.0 - zoom.clamp(0.0, 1.0) * travel;
    let painter = ui.painter();
    painter.line_segment(
        [
            egui::pos2(rail.center().x, rail.top() + 1.0),
            egui::pos2(rail.center().x, rail.bottom() - 1.0),
        ],
        egui::Stroke::new(2.0, p.raised),
    );
    let thumb = Rect::from_center_size(egui::pos2(rail.center().x, y), SMALL_ZOOM_THUMB);
    let ink = if resp.dragged() || resp.hovered() || resp.has_focus() {
        p.violet
    } else {
        p.ink
    };
    painter.rect_filled(thumb, widgets::inner_radius(ui.visuals()), ink);
    resp.widget_info(|| egui::WidgetInfo::slider(true, f64::from(zoom), label));
    let dy = resp.drag_delta().y;
    let dragged = resp.dragged() && dy != 0.0;
    let drag = if dragged { dy / SMALL_ZOOM_TRAVEL } else { 0.0 };
    (dragged || steps != 0.0).then(|| (zoom - drag + steps * SMALL_ZOOM_STEP).clamp(0.0, 1.0))
}

/// Where the small camera window's parts sit in its body, from the body's
/// top left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SmallCamera {
    /// Front, side, rear and over the shoulder.
    pub views: [Rect; 4],
    /// Object view, mouselook and reset view, right after the views.
    pub tools: [Rect; 3],
    pub sticks: SticksRow,
    pub roll_left: Rect,
    pub roll_right: Rect,
}

impl SmallCamera {
    /// At the window's default size (`SMALL_BODY` × `SMALL_ROW` +
    /// `SMALL_STICK`).
    #[cfg(test)]
    pub fn new(min: Pos2) -> SmallCamera {
        SmallCamera::sized(min, Vec2::new(SMALL_BODY, SMALL_ROW + SMALL_STICK))
    }

    /// In a body of `size`, the window enlarged (#4840): the joysticks and
    /// the zoom grow into the room under the buttons as
    /// floater_fs_camera_small.xml's layout panels do (follows="all"),
    /// still round, `SMALL_STICK` at the least; the buttons stay on top,
    /// their row kept centred over them as the window widens (the Legacy
    /// Viewer leaves it at the left edge, away from the joysticks: the
    /// human's check of 2026-11-25).
    pub fn sized(min: Pos2, size: Vec2) -> SmallCamera {
        let size = size.max(Vec2::new(SMALL_BODY, SMALL_ROW + SMALL_STICK));
        let shift = ((size.x - SMALL_BODY) / 2.0).floor();
        let button = |left: f32| Rect::from_min_size(egui::pos2(left + shift, min.y), Vec2::splat(SMALL_BUTTON));
        let step = SMALL_BUTTON + SMALL_BUTTON_GAP;
        // floater_fs_camera_small.xml, from the window's left edge: the
        // views at 5 (buttons_view, 76 wide), the tools 4 px after them, at
        // 85 to 143; the body starts SMALL_INSET in
        let views = std::array::from_fn(|i| button(min.x + 5.0 - SMALL_INSET + i as f32 * step));
        let tools = std::array::from_fn(|i| button(min.x + 85.0 - SMALL_INSET + i as f32 * step));
        let row = Rect::from_min_size(egui::pos2(min.x, min.y + SMALL_ROW), Vec2::new(size.x, size.y - SMALL_ROW));
        // as tall as the row, and both as wide as the zoom and the four
        // gaps leave them
        let stick = row
            .height()
            .min((row.width() - SMALL_ZOOM_W - 4.0 * SMALL_STICK_GAP) / 2.0)
            .max(SMALL_STICK);
        let sticks = SticksRow::sized(row, stick, SMALL_ZOOM_W);
        let o = sticks.orbit;
        let corner = |x: f32| Rect::from_min_size(egui::pos2(x, o.bottom() - SMALL_ROLL), Vec2::splat(SMALL_ROLL));
        SmallCamera {
            views,
            tools,
            sticks,
            roll_left: corner(o.left()),
            roll_right: corner(o.right() - SMALL_ROLL),
        }
    }
}

/// One of the small camera window's 18 px buttons: its icon on the field,
/// on the accent while `on` (the view shown, object view, mouselook), named
/// `tip` for its tooltip and screen readers. Placed at `rect`, unlike
/// `widgets::icon_toggle_sized`, and its response kept for the views'
/// right click.
fn small_button(ui: &mut egui::Ui, p: &Palette, rect: Rect, icon: &str, tip: &str, on: bool) -> egui::Response {
    let resp = ui.interact(rect, ui.scope_id().with(("small", icon)), Sense::click());
    let fill = if on {
        p.violet
    } else if resp.hovered() {
        p.raised
    } else {
        p.field
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    paint_icon(ui, icon, rect.center(), 14.0, if on { p.on_accent } else { p.ink });
    let resp = resp.on_hover_text(tip);
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, tip));
    resp
}

/// A roll button in a corner of the small window's orbit joystick: held,
/// it rolls (`roll_held`); greyed when not `enabled`.
fn small_hold(ui: &mut egui::Ui, p: &Palette, rect: Rect, icon: &str, label: &str, enabled: bool) -> egui::Response {
    // a sustained press belongs to this control, as `roll_controls` says
    let resp = ui.interact(rect, ui.scope_id().with(("small", icon)), Sense::click_and_drag());
    let held = enabled && resp.is_pointer_button_down_on();
    let fill = if held {
        p.violet
    } else if resp.hovered() && enabled {
        p.raised.gamma_multiply(1.3)
    } else {
        p.raised
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    let ink = match (held, enabled) {
        (true, _) => p.on_accent,
        (false, true) => p.ink,
        (false, false) => p.muted_dim,
    };
    paint_icon(ui, icon, rect.center(), 10.0, ink);
    let resp = resp.on_hover_text(label);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    resp
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
            if mode_locked(view, &action) {
                let _ = widgets::icon_button(ui, p, icon, tip, false);
            } else if widgets::icon_toggle(ui, p, icon, tip, on) {
                out.actions.push(action);
            }
        }
        if view.seated
            && ui
                .add_enabled_ui(!view.stand_locked, |ui| {
                    widgets::flat_button(ui, p, aurora_i18n::tr!("common-stand-up")).clicked()
                })
                .inner
        {
            out.actions.push(ControlAction::StandUp);
        }
    });
}

/// The name of a new camera preset being typed and its save button (the
/// Legacy Viewer's LLFloaterSaveCameraPreset, inline): Enter or the button
/// saves, Escape gives up. A view's name ("Front View") replaces that view
/// (`CameraPreset::from_preset_name`, in the app). The field takes the
/// keyboard once as it opens, like the dialog's line editor; a click
/// elsewhere then keeps it.
///
/// Gives the name typed when it is saved (also the camera position
/// window's).
pub(crate) fn name_row(ui: &mut egui::Ui, p: &Palette, naming: &mut ShotNaming, width: f32) -> Option<String> {
    let edit = ui.add_sized(
        Vec2::new(width, control_height(ui)),
        egui::TextEdit::singleline(&mut naming.name).hint_text(aurora_i18n::tr!("controls-shot-name")),
    );
    if std::mem::take(&mut naming.focus) {
        edit.request_focus();
    }
    let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let typed = naming.name.trim().to_owned();
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        naming.open = false;
    }
    if (square_button(ui, p, "check", aurora_i18n::tr!("controls-shot-save")) || enter) && !typed.is_empty() {
        naming.open = false;
        naming.name.clear();
        return Some(typed);
    }
    if enter {
        // an empty name saves nothing and the editor keeps the keyboard
        // (LLFloaterSaveCameraPreset::onBtnSave returns on an empty name)
        naming.focus = true;
    }
    None
}

/// A view's right click, in both styles: the camera where it is becomes
/// the view, or the view goes back to its template (the Legacy Viewer's
/// "Save as preset" over "Front View", and the reset button of its Camera
/// Presets window, LLFloaterCameraPresets).
fn view_menu(resp: &egui::Response, p: &Palette, view: &View, preset: CameraPreset, out: &mut Output) {
    super::menu::context_menu(resp, p, |ui| {
        let save = aurora_i18n::tr!("controls-view-save", view = preset.label());
        if super::menu::item(ui, p, "device-floppy-filled", &save) {
            out.actions.push(ControlAction::SaveView(preset));
        }
        let restore = aurora_i18n::tr!("controls-view-restore", view = preset.label());
        if super::menu::item_if(ui, p, "restore", &restore, view.views.get(preset).is_some()) {
            out.actions.push(ControlAction::RestoreView(preset));
        }
    });
}

fn view_tip(preset: CameraPreset) -> String {
    aurora_i18n::tr!("controls-view-tip", view = preset.label())
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
/// without pulling resets. As wide as `rect` (`STICK` in the Controls
/// window, `SMALL_STICK` in the Classic style's small camera window), its
/// knob, arrows and icon in proportion.
fn joystick(ui: &mut egui::Ui, p: &Palette, rect: Rect, icon: &str, tip: &str) -> Stick {
    let resp = ui.interact(rect, ui.scope_id().with(icon), Sense::click_and_drag());
    let resp = resp.on_hover_text(tip);
    let center = rect.center();
    let size = rect.width();
    let scale = size / STICK;
    let knob_size = KNOB * scale;
    let reach = (size - knob_size) / 2.0;
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
    if resp.clicked()
        && resp
            .interact_pointer_pos()
            .is_some_and(|pos| pos.distance(center) < knob_size / 2.0)
    {
        out.center = true;
    }
    let painter = ui.painter();
    painter.circle_filled(center, size / 2.0, p.field);
    painter.circle_stroke(center, size / 2.0 - 0.5, egui::Stroke::new(1.0, p.raised));
    // the four directions
    for (dx, dy) in [(0.0, -1.0), (0.0, 1.0), (-1.0, 0.0), (1.0, 0.0)] {
        let d = Vec2::new(dx, dy);
        let tip = center + d * (size / 2.0 - 6.0 * scale);
        let base = center + d * (size / 2.0 - 11.0 * scale);
        let side = Vec2::new(-d.y, d.x) * 4.0 * scale;
        painter.add(egui::Shape::convex_polygon(
            vec![tip, base + side, base - side],
            p.muted_dim,
            egui::Stroke::NONE,
        ));
    }
    let held = resp.is_pointer_button_down_on();
    painter.circle_filled(knob, knob_size / 2.0, if held { p.violet } else { p.raised });
    if let Some(t) = super::icons::global(icon) {
        let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
        let tint = if held { p.on_accent } else { p.violet_light };
        painter.image(t.id(), Rect::from_center_size(knob, Vec2::splat(16.0 * scale)), uv, tint);
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
    // 15 px, smaller on the small camera window's 14 px buttons
    paint_icon(ui, icon, rect.center(), (rect.height() - 3.0).min(15.0), ink);
    held
}

/// A square icon button as tall as the row's buttons (the reset, the save).
fn square_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str) -> bool {
    widgets::action_button(ui, p, icon, "", widgets::ActionKind::Flat, Vec2::splat(control_height(ui)))
        .on_hover_text(tip)
        .clicked()
}

/// One of the movement buttons: its icon, its key in the corner, lit while
/// held.
fn pad_button(ui: &mut egui::Ui, p: &Palette, icon: &str, tip: &str, binding: Option<&Binding>, drag_id: egui::Id) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(PAD_BUTTON), Sense::click_and_drag());
    // Continue only a primary press observed on this pad in consecutive frames.
    // A hidden pad may miss a release; a fresh press must start on a visible button.
    let frame = ui.ctx().cumulative_frame_nr_for(egui::ViewportId::ROOT);
    let (down, pressed) = ui.input(|i| (i.pointer.primary_down(), i.pointer.primary_pressed()));
    let pad_pressed = down
        && ui.data_mut(|d| {
            let last = d.get_temp::<u64>(drag_id);
            let continuing = last == Some(frame) || (!pressed && last.is_some_and(|last| last + 1 == frame));
            let active = continuing || (pressed && resp.is_pointer_button_down_on() && resp.contains_pointer());
            if active && last != Some(frame) {
                d.insert_temp(drag_id, frame);
            }
            active
        });
    let held = pad_pressed && resp.contains_pointer();
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

/// A movement mode: its icon and its name, the accent's face when chosen,
/// dimmed and deaf to clicks when not `enabled`.
fn mode_button(ui: &mut egui::Ui, p: &Palette, icon: &str, label: &str, on: bool, enabled: bool, width: f32) -> egui::Response {
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, BUTTON_H + 2.0), sense);
    let fill = match (on, resp.hovered() && enabled) {
        (true, _) => p.violet,
        (false, true) => p.raised.gamma_multiply(1.3),
        (false, false) => p.raised,
    };
    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, fill);
    let color = match (on, enabled) {
        (true, _) => p.on_accent,
        (false, true) => p.muted,
        (false, false) => p.muted_dim.gamma_multiply(0.7),
    };
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
    let ink = if on { p.ink } else { p.muted };
    let galley = ui
        .painter()
        .layout(label.to_owned(), FontId::proportional(11.0), ink, (width - 8.0).max(1.0));
    let height = (17.0 + ROW_GAP + galley.size().y + 8.0).max(42.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
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
    paint_icon(ui, icon, egui::pos2(rect.center().x, rect.top() + 4.0 + 8.5), 17.0, p.ink);
    let pos = egui::pos2(rect.center().x - galley.size().x / 2.0, rect.bottom() - 4.0 - galley.size().y);
    ui.painter().galley(pos, galley, ink);
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

    /// Runs the camera preset's name field for one frame with these events.
    fn name_frame(
        ctx: &egui::Context,
        p: &Palette,
        naming: &mut ShotNaming,
        events: Vec<egui::Event>,
    ) -> (Output, Option<egui::output::IMEOutput>) {
        let mut out = Output::default();
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
                events,
                ..Default::default()
            },
            |ui| {
                ui.horizontal(|ui| {
                    if let Some(name) = name_row(ui, p, naming, 200.0) {
                        out.actions.push(ControlAction::SaveShot(name));
                    }
                });
            },
        );
        // no renderer here: the font atlas would trip TexturesDelta's drop check
        output.textures_delta.clear();
        (out, output.platform_output.ime)
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn a_camera_preset_name_composed_with_an_ime_is_saved() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let mut naming = ShotNaming::default();
        naming.start();
        name_frame(&ctx, &p, &mut naming, vec![]);
        assert!(ctx.memory(|m| m.focused().is_some()), "the field takes the keyboard as it opens");
        // #3889: a focus request on every frame interrupted the composition
        for _ in 0..5 {
            let (_, ime) = name_frame(&ctx, &p, &mut naming, vec![]);
            let ime = ime.expect("the focused field drives the IME");
            assert!(!ime.should_interrupt_composition, "an open field must leave the IME composing");
        }
        let preedit = egui::ImeEvent::Preedit {
            text: "かめら".to_owned(),
            active_range_chars: Some(0..3),
        };
        name_frame(&ctx, &p, &mut naming, vec![egui::Event::Ime(preedit)]);
        let (_, ime) = name_frame(
            &ctx,
            &p,
            &mut naming,
            vec![egui::Event::Ime(egui::ImeEvent::Commit("カメラ".to_owned()))],
        );
        assert!(!ime.is_some_and(|i| i.should_interrupt_composition));
        assert_eq!(naming.name, "カメラ");
        let (out, _) = name_frame(&ctx, &p, &mut naming, vec![key(egui::Key::Enter)]);
        assert!(matches!(out.actions.as_slice(), [ControlAction::SaveShot(name)] if name == "カメラ"));
        assert!(!naming.open && naming.name.is_empty());
    }

    #[test]
    fn the_camera_preset_name_field_focuses_once() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let mut naming = ShotNaming::default();
        naming.start();
        name_frame(&ctx, &p, &mut naming, vec![]);
        // Enter on an empty name saves nothing and keeps the keyboard
        let (out, _) = name_frame(&ctx, &p, &mut naming, vec![key(egui::Key::Enter)]);
        assert!(out.actions.is_empty());
        name_frame(&ctx, &p, &mut naming, vec![]);
        assert!(naming.open && ctx.memory(|m| m.focused().is_some()));
        // the keyboard given elsewhere is not taken back
        ctx.memory_mut(|m| m.stop_text_input());
        for _ in 0..3 {
            name_frame(&ctx, &p, &mut naming, vec![]);
        }
        assert!(naming.open && ctx.memory(|m| m.focused().is_none()));
    }

    #[test]
    fn a_long_roll_press_never_drags_or_snaps_its_floater() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let keys = KeyBindings::default();
        let view = View {
            zoom: 0.5,
            roll: 0.0,
            preset: CameraPreset::Rear,
            shots: &[],
            views: &SavedViews::default(),
            flying: false,
            always_run: false,
            seated: false,
            run_locked: false,
            fly_locked: false,
            stand_locked: false,
            mouselook: false,
            object_view: false,
            keys: &keys,
            free: None,
        };
        let window = egui::Id::unique("roll_hold_fixture");
        let mut time = 0.0;
        let mut frame = |events| {
            time += 1.0 / 60.0;
            let mut button = Rect::NOTHING;
            let mut out = Output::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    super::super::layout::frame(ui.ctx(), &p, true);
                    Floater::new("roll_hold_fixture", "Camera", Pos2::new(100.0, 100.0), Vec2::new(WIDTH, 160.0))
                        .fixed()
                        .auto_height()
                        .show(ui.ctx(), &p, &mut true, |ui| {
                            button = Rect::from_min_size(ui.cursor().min, Vec2::splat(BUTTON_H));
                            roll_controls(ui, &p, &view, &mut out);
                        });
                },
            );
            // no renderer here: the font atlas would trip TexturesDelta's drop check
            output.textures_delta.clear();
            (ctx.memory(|m| m.area_rect(window).expect("the controls floater")), button, out)
        };
        for _ in 0..8 {
            frame(vec![]);
        }
        let (before, button, _) = frame(vec![]);
        for start in [
            button.center(),
            egui::pos2(before.right() - 12.0 - BUTTON_H / 2.0, button.center().y),
        ] {
            frame(vec![egui::Event::PointerMoved(start)]);
            frame(vec![egui::Event::PointerButton {
                pos: start,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }]);
            let end = start + Vec2::new(5.0, 3.0);
            for n in 0..180 {
                let (rect, _, out) = frame(vec![egui::Event::PointerMoved(if n % 2 == 0 { start } else { end })]);
                assert_eq!(rect, before, "holding roll must not move the controls");
                assert!(!widgets::window_dragging(&ctx), "the roll button must own a sustained press");
                assert!(
                    matches!(out.actions.as_slice(), [ControlAction::RollBy(_)]),
                    "roll must continue throughout the hold"
                );
            }
            frame(vec![egui::Event::PointerButton {
                pos: end,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }]);
            assert!(frame(vec![]).2.actions.is_empty());
        }
    }

    #[test]
    fn held_movement_pad_switches_to_the_button_under_the_pointer() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let mut time = 0.0;
        let mut draw = |events, visible: bool, cover: Option<Rect>| {
            time += 1.0 / 60.0;
            let mut rects = [Rect::NOTHING; 2];
            let mut held = [false; 2];
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    if !visible {
                        return;
                    }
                    let drag_id = ui.scope_id().with("movement-pad-test");
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            rects[0] = Rect::from_min_size(ui.cursor().min, Vec2::splat(PAD_BUTTON));
                            held[0] = pad_button(ui, &p, "arrow-up", "", None, drag_id);
                        });
                        ui.horizontal(|ui| {
                            rects[1] = Rect::from_min_size(ui.cursor().min, Vec2::splat(PAD_BUTTON));
                            held[1] = pad_button(ui, &p, "rotate-2", "", None, drag_id);
                        });
                    });
                    if let Some(rect) = cover {
                        egui::Area::new(egui::Id::unique("cover-pad-button"))
                            .order(egui::Order::Foreground)
                            .fixed_pos(rect.min)
                            .show(ui.ctx(), |ui| ui.allocate_exact_size(rect.size(), Sense::click()));
                    }
                },
            );
            output.textures_delta.clear();
            (rects, held)
        };
        let mut frame = |events| draw(events, true, None);
        let (rects, _) = frame(vec![]);
        let start = rects[0].center();
        let end = rects[1].center();
        let moved = |pos| vec![egui::Event::PointerMoved(pos)];
        let primary = |pos, pressed| {
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }]
        };
        frame(moved(start));
        frame(primary(start, true));
        let (_, held) = frame(moved(end));
        assert_eq!(held, [false, true], "dragging between pad buttons switches the held direction");
        let (_, held) = frame(moved(start));
        assert_eq!(held, [true, false], "dragging back switches to the original direction");
        frame(primary(start, false));
        let (_, held) = frame(vec![]);
        assert_eq!(held, [false, false], "releasing the mouse stops the movement buttons");

        let outside = Pos2::new(350.0, 180.0);
        frame(moved(outside));
        frame(primary(outside, true));
        let (_, held) = frame(moved(end));
        assert_eq!(held, [false, false], "a drag that starts outside the pad does not activate it");
        frame(primary(end, false));

        let cover = Some(rects[1]);
        draw(moved(start), true, cover);
        draw(vec![], true, cover);
        assert_eq!(draw(primary(start, true), true, cover).1, [true, false]);
        assert_eq!(draw(moved(end), true, cover).1, [false, false], "a window covers the second button");

        draw(vec![], false, None);
        draw(primary(end, false), false, None);
        draw(moved(outside), false, None);
        draw(primary(outside, true), false, None);
        draw(vec![], true, None);
        assert_eq!(
            draw(moved(end), true, None).1,
            [false, false],
            "a new press started while the pad was closed"
        );
    }

    #[test]
    fn roll_buttons_ramp_stop_on_release_and_level_without_resetting_the_view() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let keys = KeyBindings::default();
        let mut time = 0.0;
        let mut frame = |events, mouselook| {
            time += 0.05;
            let view = View {
                zoom: 0.5,
                roll: 0.3,
                preset: CameraPreset::Rear,
                shots: &[],
                views: &SavedViews::default(),
                flying: false,
                always_run: false,
                seated: false,
                run_locked: false,
                fly_locked: false,
                stand_locked: false,
                mouselook,
                object_view: false,
                keys: &keys,
                free: None,
            };
            let mut out = Output::default();
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(INNER);
                    roll_controls(ui, &p, &view, &mut out);
                },
            );
            output.textures_delta.clear();
            out.actions
        };
        frame(vec![], false);
        let event = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        for (x, sign) in [(15.0, -1.0), (260.0, 1.0)] {
            let pos = egui::pos2(x, 15.0);
            let first = frame(event(pos, true), false);
            assert!(matches!(first.as_slice(), [ControlAction::RollBy(v)] if v * sign > 0.0));
            let mut last = first;
            for _ in 0..6 {
                last = frame(vec![], false);
            }
            assert!(matches!(last.as_slice(), [ControlAction::RollBy(v)] if v * sign > 0.01));
            assert!(frame(event(pos, false), false).is_empty());
            assert!(frame(vec![], false).is_empty());
        }
        let level = egui::pos2(140.0, 15.0);
        frame(event(level, true), false);
        assert_eq!(frame(event(level, false), false), vec![ControlAction::LevelRoll]);
        assert!(frame(event(egui::pos2(15.0, 15.0), true), true).is_empty());
    }

    /// #1815: Move on a minimized window opened its camera joysticks.
    #[test]
    fn a_request_expands_a_minimized_window_on_the_tab_asked_for() {
        let ctx = egui::Context::default();
        let (mut camera, mut movement) = (true, false);
        widgets::minimize_floater(&ctx, WINDOW);
        assert!(widgets::floater_minimized(&ctx, WINDOW));
        request(&ctx, &mut camera, &mut movement, false, false, true, true);
        assert!(movement, "the movement tab asked for");
        let expand = ctx.data(|d| d.get_temp::<bool>(egui::Id::unique(WINDOW).with(widgets::EXPAND)));
        assert_eq!(expand, Some(true), "the window expands");
        // shown on that tab, the button closes it; again, it opens
        let ctx = egui::Context::default();
        let (mut camera, mut movement) = (false, true);
        request(&ctx, &mut camera, &mut movement, false, false, true, true);
        assert!(!movement && !camera);
        request(&ctx, &mut camera, &mut movement, false, false, true, true);
        assert!(movement && !camera);
        // a menu item only opens
        request(&ctx, &mut camera, &mut movement, false, false, true, false);
        assert!(movement);
        // Legacy's two windows: each toggles on its own
        let (mut camera, mut movement) = (true, false);
        request(&ctx, &mut camera, &mut movement, true, false, true, true);
        assert!(camera && movement);
        request(&ctx, &mut camera, &mut movement, true, false, false, true);
        assert!(!camera && movement);
    }

    /// #3904: the compact window is the camera shown, so its button
    /// closes it; a request that only opens (`toggle` false) leaves it
    /// compact.
    #[test]
    fn the_camera_button_closes_the_compact_window() {
        let ctx = egui::Context::default();
        let (mut camera, mut movement) = (true, false);
        widgets::minimize_floater(&ctx, WINDOW);
        request(&ctx, &mut camera, &mut movement, false, false, false, true);
        let expand = ctx.data(|d| d.get_temp::<bool>(egui::Id::unique(WINDOW).with(widgets::EXPAND)));
        assert_eq!(expand, None, "not expanded");
        let close = ctx.data(|d| d.get_temp::<u64>(egui::Id::unique(WINDOW).with("close")));
        assert!(close.is_some(), "closed as its cross does");
        let ctx = egui::Context::default();
        let (mut camera, mut movement) = (true, false);
        widgets::minimize_floater(&ctx, WINDOW);
        request(&ctx, &mut camera, &mut movement, false, false, false, false);
        let expand = ctx.data(|d| d.get_temp::<bool>(egui::Id::unique(WINDOW).with(widgets::EXPAND)));
        assert!(camera && !movement && expand.is_none() && widgets::floater_minimized(&ctx, WINDOW));
    }

    /// #3904: the compact form is kept across closing and reopening, and
    /// follows the saved choice when it changes (set here by hand, as the
    /// title button or Preferences would); the Movement button and a window
    /// reset do not change it.
    #[test]
    fn the_compact_window_opens_again_compact() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let keys = KeyBindings::default();
        let views = SavedViews::default();
        let view = View {
            zoom: 0.5,
            roll: 0.0,
            preset: CameraPreset::Rear,
            shots: &[],
            views: &views,
            flying: false,
            always_run: false,
            seated: false,
            run_locked: false,
            fly_locked: false,
            stand_locked: false,
            mouselook: false,
            object_view: false,
            keys: &keys,
            free: None,
        };
        let mut controls = ControlsUi::default();
        let (mut camera, mut movement) = (false, false);
        let mut compact = Compact {
            camera: true,
            movement: false,
        };
        let mut time = 0.0;
        let mut run = |controls: &mut ControlsUi, camera: &mut bool, movement: &mut bool, compact: &mut Compact| {
            for _ in 0..3 {
                time += 0.1;
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 1000.0))),
                        time: Some(time),
                        ..Default::default()
                    },
                    |ui| {
                        controls.show(ui.ctx(), &p, camera, movement, compact, false, &view);
                    },
                );
                output.textures_delta.clear();
            }
        };
        let minimized = || widgets::floater_minimized(&ctx, WINDOW);
        // the toolbar's camera: it opens compact
        request(&ctx, &mut camera, &mut movement, false, false, false, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(camera && minimized(), "opens compact");
        // the button again closes it, a third press opens it compact
        request(&ctx, &mut camera, &mut movement, false, false, false, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(!camera && !movement, "closed");
        request(&ctx, &mut camera, &mut movement, false, false, false, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(camera && minimized() && compact.camera, "compact again");
        // Movement: the full window on its tab, the choice kept
        request(&ctx, &mut camera, &mut movement, false, false, true, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(movement && !camera && !minimized() && compact.camera);
        // back to the camera from the toolbar: compact, as saved
        request(&ctx, &mut camera, &mut movement, false, false, false, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(camera && minimized());
        // the photo window's Controls asks for it full: the choice kept
        widgets::expand_floater(&ctx, WINDOW);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(camera && !minimized() && compact.camera, "an expand asked for is no choice");
        // closed and opened again: compact
        request(&ctx, &mut camera, &mut movement, false, false, false, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        request(&ctx, &mut camera, &mut movement, false, false, false, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(camera && minimized());
        // "Reset window positions and sizes" expands it: the choice kept
        crate::ui::layout::reset_windows(&ctx);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(camera && !minimized() && compact.camera, "a reset is no choice");
        // the choice made full, then closed and opened again: full
        compact.camera = false;
        request(&ctx, &mut camera, &mut movement, false, false, false, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        request(&ctx, &mut camera, &mut movement, false, false, false, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(camera && !minimized());
        // the Preferences switch while the window is open: at once, both ways
        compact.camera = true;
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(camera && minimized() && compact.camera, "switched on in Preferences");
        // Movement, then back by the Camera tab: the camera compact again
        request(&ctx, &mut camera, &mut movement, false, false, true, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(movement && !minimized(), "Movement full, as saved");
        controls.tab = Tab::Camera;
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(camera && minimized(), "the Camera tab: compact, as saved");
        compact.camera = false;
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(camera && !minimized() && !compact.camera, "switched off in Preferences");
        // the Movement tab's own choice: it opens compact, the camera full
        compact.movement = true;
        request(&ctx, &mut camera, &mut movement, false, false, true, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(movement && minimized(), "Movement compact, as saved");
        request(&ctx, &mut camera, &mut movement, false, false, false, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(camera && !minimized(), "the camera full, as saved");
        request(&ctx, &mut camera, &mut movement, false, false, true, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(movement && minimized(), "Movement compact again");
        // its button closes the compact movement, as the full one
        request(&ctx, &mut camera, &mut movement, false, false, true, true);
        run(&mut controls, &mut camera, &mut movement, &mut compact);
        assert!(!camera && !movement, "closed from Movement");
    }

    /// The Controls window driven frame by frame, as `App::controls_window`
    /// does, with real clicks on its title bar.
    struct Harness {
        ctx: egui::Context,
        p: Palette,
        keys: KeyBindings,
        views: SavedViews,
        controls: ControlsUi,
        camera: bool,
        movement: bool,
        compact: Compact,
        free: Option<FreeView>,
        /// The Legacy Viewer's windows (Preferences › Aurora).
        classic: bool,
        time: f64,
    }

    impl Harness {
        fn new(compact: Compact) -> Self {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            Self {
                ctx,
                p: theme.palette(),
                keys: KeyBindings::default(),
                views: SavedViews::default(),
                controls: ControlsUi::default(),
                camera: false,
                movement: false,
                compact,
                free: None,
                classic: false,
                time: 0.0,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>) {
            self.time += 0.1;
            let view = View {
                zoom: 0.5,
                roll: 0.0,
                preset: CameraPreset::Rear,
                shots: &[],
                views: &self.views,
                flying: false,
                always_run: false,
                seated: false,
                run_locked: false,
                fly_locked: false,
                stand_locked: false,
                mouselook: false,
                object_view: false,
                keys: &self.keys,
                free: self.free,
            };
            let (controls, camera, movement, compact, p, classic) = (
                &mut self.controls,
                &mut self.camera,
                &mut self.movement,
                &mut self.compact,
                &self.p,
                self.classic,
            );
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 1000.0))),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    // as App does at the start of the interface's frame
                    crate::ui::close_key::begin(ui.ctx());
                    controls.show(ui.ctx(), p, camera, movement, compact, classic, &view);
                },
            );
            output.textures_delta.clear();
        }

        /// The toolbar's camera (or Move) button, then one frame only.
        fn press(&mut self, want_movement: bool) {
            let classic = self.classic && self.free.is_none();
            request(
                &self.ctx,
                &mut self.camera,
                &mut self.movement,
                classic,
                self.compact.camera,
                want_movement,
                true,
            );
            self.frame(Vec::new());
        }

        fn settle(&mut self) {
            for _ in 0..3 {
                self.frame(Vec::new());
            }
        }

        /// A click on the title bar's compact / full button.
        fn click_title_button(&mut self) {
            let rect = self
                .ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::unique(("floater_minimize", WINDOW))))
                .expect("the title button drawn");
            let at = rect.center();
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
            self.settle();
        }

        /// A double-click on the title bar, left of its buttons (the reset
        /// button is the leftmost).
        fn double_click_title(&mut self) {
            let rect = self
                .ctx
                .data(|d| d.get_temp::<Rect>(egui::Id::unique(("floater_reset", WINDOW))))
                .expect("the title button drawn");
            let at = rect.center() - Vec2::new(rect.width() * 2.0, 0.0);
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::default(),
            };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![button(true), button(false)]);
            self.frame(vec![button(true), button(false)]);
            self.settle();
        }

        fn minimized(&self) -> bool {
            widgets::floater_minimized(&self.ctx, WINDOW)
        }
    }

    /// #3904: the title button's compact / full, or a double-click on the
    /// title bar, is the shown tab's saved choice; in the free camera it is
    /// the free camera's own window.
    #[test]
    fn the_title_button_sets_the_compact_choice() {
        let mut h = Harness::new(Compact::default());
        h.press(false);
        h.settle();
        assert!(h.camera && !h.minimized());
        h.click_title_button();
        assert!(
            h.minimized() && h.compact.camera && !h.compact.movement,
            "compact, saved for the camera"
        );
        h.click_title_button();
        assert!(!h.minimized() && !h.compact.camera, "full, saved");
        // the Movement tab keeps its own choice
        h.press(true);
        h.settle();
        assert!(h.movement && !h.minimized());
        h.click_title_button();
        assert!(
            h.minimized() && h.compact.movement && !h.compact.camera,
            "compact, saved for Movement"
        );
        // a double-click on the title bar is the same choice
        h.double_click_title();
        assert!(!h.minimized() && !h.compact.movement, "full by a double-click, saved");
        // the free camera's compact window is no choice of the camera's
        let mut h = Harness::new(Compact::default());
        h.free = Some(FreeView {
            speed: 2.0,
            no_collisions: false,
        });
        h.press(false);
        h.settle();
        h.click_title_button();
        assert!(h.minimized() && h.compact == Compact::default(), "the free camera's own");
        // the pure rule underneath
        assert_eq!(compact_choice(true, true, false), Some(true));
        assert_eq!(compact_choice(true, false, false), Some(false));
        assert_eq!(compact_choice(true, true, true), None);
        assert_eq!(compact_choice(false, false, false), None);
    }

    /// #3904: "Reset window positions and sizes" made while the window is
    /// closed, or a close and an opening one frame apart, still open it in
    /// its saved compact form.
    #[test]
    fn a_reset_while_closed_keeps_the_compact_form() {
        let mut h = Harness::new(Compact {
            camera: true,
            movement: false,
        });
        h.press(false);
        h.settle();
        assert!(h.camera && h.minimized(), "opens compact");
        h.press(false);
        h.settle();
        assert!(!h.camera, "closed");
        crate::ui::layout::reset_windows(&h.ctx);
        h.press(false);
        h.settle();
        assert!(h.camera && h.minimized() && h.compact.camera, "compact after the reset");
        // closed by its button, opened again at the very next frame
        h.press(false);
        assert!(!h.camera, "closed at once");
        h.press(false);
        h.settle();
        assert!(h.camera && h.minimized(), "compact, the frame after a close");
    }

    /// The window `id`'s rect on the last frame, if it was shown.
    fn shown_rect(ctx: &egui::Context, id: &str) -> Option<Rect> {
        ctx.memory(|m| m.area_rect(egui::Id::unique(id))).filter(|_| {
            ctx.memory(|m| {
                m.areas()
                    .visible_last_frame(&egui::LayerId::new(egui::Order::Middle, egui::Id::unique(id)))
            })
        })
    }

    /// Feedback board #677: the Classic style's compact camera is the
    /// Legacy Viewer's small window, 165 × 77, without a title bar or a
    /// close cross; full, it is the Classic camera window as before.
    #[test]
    fn the_classic_compact_camera_is_the_legacy_small_window() {
        let mut h = Harness::new(Compact {
            camera: true,
            movement: true,
        });
        h.classic = true;
        h.press(false);
        h.settle();
        assert!(h.camera, "opened");
        let small = shown_rect(&h.ctx, CLASSIC_CAMERA_SMALL).expect("the small window shown");
        assert!(
            (small.width() - 165.0).abs() < 0.5 && (small.height() - 77.0).abs() < 0.5,
            "the Legacy Viewer's 165 × 77, got {:?}",
            small.size()
        );
        assert!(shown_rect(&h.ctx, CLASSIC_CAMERA).is_none(), "not the full window");
        let cross = egui::Id::unique(CLASSIC_CAMERA_SMALL).with("headless_close");
        assert!(h.ctx.memory(|m| m.area_rect(cross)).is_none(), "no close cross");
        // the Classic movement window is small already: unchanged
        h.press(true);
        h.settle();
        assert!(shown_rect(&h.ctx, CLASSIC_MOVEMENT).is_some(), "the movement window");
        // Preferences switches compact camera off: the full window, as before
        h.compact.camera = false;
        h.settle();
        assert!(shown_rect(&h.ctx, CLASSIC_CAMERA).is_some(), "the full window");
        assert!(shown_rect(&h.ctx, CLASSIC_CAMERA_SMALL).is_none(), "the small one gone");
        // the same saved choice is the Aurora window's compact camera
        let mut h = Harness::new(Compact {
            camera: true,
            movement: false,
        });
        h.classic = true;
        h.press(false);
        h.settle();
        h.classic = false;
        h.settle();
        assert!(h.camera && h.minimized(), "compact in the Aurora style too");
    }

    /// The toolbar button closes the small window even when the full one
    /// was left minimized (its persisted state is the full window's).
    #[test]
    fn the_toolbar_closes_the_small_window_whatever_the_full_one_was_left() {
        let ctx = egui::Context::default();
        widgets::minimize_floater(&ctx, CLASSIC_CAMERA);
        let (mut camera, mut movement) = (true, false);
        request(&ctx, &mut camera, &mut movement, true, true, false, true);
        assert!(!camera, "closed at the first press");
        request(&ctx, &mut camera, &mut movement, true, true, false, true);
        assert!(camera, "open again");
    }

    /// Ctrl+W passes over the small window, which has no close cross, and
    /// closes the window behind it; Ctrl+Shift+W closes it with the others
    /// (LLFloaterView::getFrontmostClosableFloater skips a floater that is
    /// not closeable, closeAllChildren does not).
    #[test]
    fn ctrl_w_passes_over_the_small_window_and_close_all_takes_it() {
        let mut h = Harness::new(Compact {
            camera: true,
            movement: true,
        });
        h.classic = true;
        h.press(false);
        h.press(true);
        h.settle();
        h.ctx
            .move_to_top(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(CLASSIC_CAMERA_SMALL)));
        h.settle();
        assert!(h.camera && h.movement, "both open");
        let w = |modifiers, pressed| egui::Event::Key {
            key: egui::Key::W,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        h.frame(vec![w(egui::Modifiers::COMMAND, true)]);
        h.frame(vec![w(egui::Modifiers::COMMAND, false)]);
        assert!(h.camera, "the small window stays, on top");
        assert!(!h.movement, "the window behind it closed");
        h.frame(vec![w(egui::Modifiers::COMMAND, true)]);
        h.frame(vec![w(egui::Modifiers::COMMAND, false)]);
        assert!(h.camera, "alone, it still stays");
        h.frame(vec![w(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, true)]);
        h.frame(Vec::new());
        assert!(!h.camera, "Close All closes it");
    }

    /// #4840: the small window is enlarged by its edges, and never made
    /// smaller than it opens (165 × 77, the Legacy Viewer's min_width and
    /// min_height): the human's check of 2026-11-25 took it 5 px shorter.
    #[test]
    fn the_small_camera_window_is_resized_down_to_its_opening_size_only() {
        let mut h = Harness::new(Compact {
            camera: true,
            movement: false,
        });
        h.classic = true;
        h.press(false);
        h.settle();
        let opened = shown_rect(&h.ctx, CLASSIC_CAMERA_SMALL).expect("the small window shown");
        // drag the edge at `from` by `by`, a few pixels a frame
        let drag = |h: &mut Harness, from: Pos2, by: Vec2| {
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            h.frame(vec![egui::Event::PointerMoved(from)]);
            h.frame(vec![button(from, true)]);
            for step in 1..=10 {
                h.frame(vec![egui::Event::PointerMoved(from + by * step as f32 / 10.0)]);
            }
            h.frame(vec![button(from + by, false)]);
            h.settle();
        };
        drag(&mut h, Pos2::new(opened.center().x, opened.bottom() - 1.0), Vec2::new(0.0, -40.0));
        drag(&mut h, Pos2::new(opened.right() - 1.0, opened.center().y), Vec2::new(-60.0, 0.0));
        let squeezed = shown_rect(&h.ctx, CLASSIC_CAMERA_SMALL).expect("still shown");
        assert!(
            squeezed.width() >= 165.0 - 0.5 && squeezed.height() >= 77.0 - 0.5,
            "no smaller than 165 × 77, got {:?}",
            squeezed.size()
        );
        drag(
            &mut h,
            Pos2::new(squeezed.center().x, squeezed.bottom() - 1.0),
            Vec2::new(0.0, 80.0),
        );
        drag(
            &mut h,
            Pos2::new(squeezed.right() - 1.0, squeezed.center().y),
            Vec2::new(120.0, 0.0),
        );
        let enlarged = shown_rect(&h.ctx, CLASSIC_CAMERA_SMALL).expect("still shown");
        assert!(
            enlarged.width() > squeezed.width() + 100.0 && enlarged.height() > squeezed.height() + 60.0,
            "enlarged by its edges: {:?} from {:?}",
            enlarged.size(),
            squeezed.size()
        );
    }

    /// The small window's parts, as floater_fs_camera_small.xml places them:
    /// all in its body, none over another, the roll buttons in the orbit
    /// joystick's bottom corners.
    #[test]
    fn the_small_camera_window_is_laid_out_like_the_legacy_one() {
        let at = SmallCamera::new(Pos2::ZERO);
        let body = Rect::from_min_size(Pos2::ZERO, Vec2::new(SMALL_BODY, SMALL_ROW + SMALL_STICK));
        let buttons: Vec<Rect> = at.views.iter().chain(at.tools.iter()).copied().collect();
        for (i, a) in buttons.iter().enumerate() {
            assert!(body.contains_rect(*a), "button {i} in the body");
            assert_eq!(a.size(), Vec2::splat(18.0), "18 px buttons");
            for b in &buttons[i + 1..] {
                assert!(!a.intersects(*b), "{a:?} over {b:?}");
            }
        }
        let s = at.sticks;
        for r in [s.orbit, s.zoom, s.pan] {
            assert!(body.contains_rect(r), "{r:?} in the body");
            assert!(r.top() >= buttons[0].bottom(), "under the buttons");
        }
        assert!(s.orbit.right() < s.zoom.left() && s.zoom.right() < s.pan.left());
        assert_eq!((s.orbit.size(), s.zoom.width()), (Vec2::splat(50.0), 16.0));
        for roll in [at.roll_left, at.roll_right] {
            assert!(s.orbit.contains_rect(roll), "in the orbit joystick's square");
            assert_eq!(roll.bottom(), s.orbit.bottom());
        }
        assert_eq!((at.roll_left.left(), at.roll_right.right()), (s.orbit.left(), s.orbit.right()));
        // from the window's left edge: the views at 5 to 83, the tools at 85 to 143
        let x = |r: Rect| (r.left() + SMALL_INSET, r.right() + SMALL_INSET);
        assert_eq!((x(at.views[0]).0, x(at.views[3]).1), (5.0, 83.0));
        assert_eq!((x(at.tools[0]).0, x(at.tools[2]).1), (85.0, 143.0));
    }

    /// #4840: the small window enlarged, its joysticks grow into the room,
    /// still round and apart, the buttons keep their place; squeezed below
    /// its default size, it keeps the default layout.
    #[test]
    fn the_small_camera_window_grows_like_the_legacy_one() {
        let default = SmallCamera::new(Pos2::ZERO);
        for size in [Vec2::new(400.0, 160.0), Vec2::new(200.0, 300.0), Vec2::new(600.0, 90.0)] {
            let at = SmallCamera::sized(Pos2::ZERO, size);
            let body = Rect::from_min_size(Pos2::ZERO, size);
            // the buttons stay on top, their row as far from the body's
            // middle as at the default size: centred over the joysticks
            let shift = ((size.x - SMALL_BODY) / 2.0).floor();
            for (a, d) in at
                .views
                .iter()
                .chain(at.tools.iter())
                .zip(default.views.iter().chain(default.tools.iter()))
            {
                assert_eq!(
                    *a,
                    d.translate(Vec2::new(shift, 0.0)),
                    "the buttons move with the joysticks at {size:?}"
                );
            }
            let s = at.sticks;
            for r in [s.orbit, s.zoom, s.pan] {
                assert!(body.contains_rect(r), "{r:?} in the body {size:?}");
                assert!(r.top() >= at.views[0].bottom(), "under the buttons");
            }
            assert!(
                s.orbit.right() < s.zoom.left() && s.zoom.right() < s.pan.left(),
                "apart at {size:?}"
            );
            assert_eq!(s.orbit.width(), s.orbit.height(), "round");
            assert_eq!(s.orbit.size(), s.pan.size());
            assert!(s.orbit.width() > default.sticks.orbit.width(), "larger at {size:?}");
            assert_eq!(s.zoom.height(), s.orbit.height(), "the zoom as tall");
            assert!(s.orbit.contains_rect(at.roll_left) && s.orbit.contains_rect(at.roll_right));
        }
        assert_eq!(SmallCamera::sized(Pos2::ZERO, Vec2::new(100.0, 40.0)).sticks, default.sticks);
    }

    /// The small window's controls, a frame with `events` at `mouselook`.
    fn small_frame(
        ctx: &egui::Context,
        p: &Palette,
        keys: &KeyBindings,
        time: f64,
        events: Vec<egui::Event>,
        mouselook: bool,
    ) -> (Pos2, Output) {
        let views = SavedViews::default();
        let view = View {
            zoom: 0.5,
            roll: 0.0,
            preset: CameraPreset::Rear,
            shots: &[],
            views: &views,
            flying: false,
            always_run: false,
            seated: false,
            run_locked: false,
            fly_locked: false,
            stand_locked: false,
            mouselook,
            object_view: false,
            keys,
            free: None,
        };
        let mut out = Output::default();
        let mut origin = Pos2::ZERO;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                origin = ui.cursor().min;
                // the body of the window at its default size, as the
                // floater gives it: the body takes the room it has (#4840)
                let body = Rect::from_min_size(origin, Vec2::new(SMALL_BODY, SMALL_ROW + SMALL_STICK));
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(body));
                small_camera(&mut child, p, &view, &mut out);
            },
        );
        output.textures_delta.clear();
        (origin, out)
    }

    /// A press in a corner of the orbit joystick rolls and does not orbit;
    /// in mouselook it does nothing, as the full window's roll buttons; the
    /// views and the tools do what the full window's do.
    #[test]
    fn the_small_window_rolls_in_its_corners_and_takes_the_views() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let keys = KeyBindings::default();
        let (origin, _) = small_frame(&ctx, &p, &keys, 0.0, vec![], false);
        let at = SmallCamera::new(origin);
        let press = |pos, pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ]
        };
        let mut time = 0.0;
        for (corner, sign) in [(at.roll_right, 1.0), (at.roll_left, -1.0)] {
            for mouselook in [false, true] {
                // the pointer comes over the button, then presses it
                time += 0.05;
                small_frame(&ctx, &p, &keys, time, vec![egui::Event::PointerMoved(corner.center())], mouselook);
                time += 0.05;
                let (_, out) = small_frame(&ctx, &p, &keys, time, press(corner.center(), true), mouselook);
                if mouselook {
                    assert!(out.actions.is_empty(), "no roll in mouselook");
                } else {
                    assert!(
                        matches!(out.actions.as_slice(), [ControlAction::RollBy(v)] if v * sign > 0.0),
                        "{:?}",
                        out.actions
                    );
                }
                time += 0.05;
                let (_, held) = small_frame(&ctx, &p, &keys, time, vec![], mouselook);
                assert!(held.held.is_empty(), "the orbit joystick not pulled: {:?}", held.held);
                time += 0.05;
                small_frame(&ctx, &p, &keys, time, press(corner.center(), false), mouselook);
            }
        }
        let click = |time: &mut f64, rect: Rect| {
            *time += 0.05;
            small_frame(&ctx, &p, &keys, *time, press(rect.center(), true), false);
            *time += 0.05;
            small_frame(&ctx, &p, &keys, *time, press(rect.center(), false), false).1.actions
        };
        assert_eq!(click(&mut time, at.views[0]), vec![ControlAction::Preset(CameraPreset::Front)]);
        assert_eq!(click(&mut time, at.views[3]), vec![ControlAction::Preset(CameraPreset::Shoulder)]);
        assert_eq!(click(&mut time, at.tools[0]), vec![ControlAction::ObjectView]);
        assert_eq!(click(&mut time, at.tools[1]), vec![ControlAction::Mouselook]);
        assert_eq!(click(&mut time, at.tools[2]), vec![ControlAction::ResetOrbit]);
    }

    #[test]
    fn the_small_zoom_slider_follows_the_drag_slowly() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let keys = KeyBindings::default();
        let (origin, _) = small_frame(&ctx, &p, &keys, 0.0, vec![], false);
        let rail = SmallCamera::new(origin).sticks.zoom.center();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut time = 0.05;
        small_frame(
            &ctx,
            &p,
            &keys,
            time,
            vec![egui::Event::PointerMoved(rail), button(rail, true)],
            false,
        );
        // up the whole rail and past it, 4 px a frame: the zoom (0.5 in
        // `small_frame`) moves by the drag, never to the pointer
        let mut zooms = vec![];
        for step in 1..=10 {
            time += 0.05;
            let at = rail - Vec2::new(0.0, 4.0 * step as f32);
            let (_, out) = small_frame(&ctx, &p, &keys, time, vec![egui::Event::PointerMoved(at)], false);
            zooms.extend(out.actions.iter().filter_map(|a| match a {
                ControlAction::Zoom(z) => Some(*z),
                _ => None,
            }));
        }
        assert!(!zooms.is_empty(), "the drag zooms");
        for z in zooms {
            assert!(z > 0.5 && z <= 0.5 + 8.0 / SMALL_ZOOM_TRAVEL, "in, by the frame's drag: {z}");
        }
    }

    #[test]
    fn the_small_zoom_slider_takes_the_arrows_and_the_wheel() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 1.0);
        let p = theme.palette();
        let keys = KeyBindings::default();
        let (origin, _) = small_frame(&ctx, &p, &keys, 0.0, vec![], false);
        let rail = SmallCamera::new(origin).sticks.zoom.center();
        let away = rail + Vec2::new(120.0, 0.0);
        let zooms = |out: Output| -> Vec<f32> {
            out.actions
                .iter()
                .filter_map(|a| match a {
                    ControlAction::Zoom(z) => Some(*z),
                    _ => None,
                })
                .collect()
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let wheel = |y| egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, y),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: egui::Modifiers::NONE,
        };
        let mut time = 0.0;
        let mut frame = |events| {
            time += 0.05;
            small_frame(&ctx, &p, &keys, time, events, false).1
        };
        // not focused yet: an arrow is not the slider's
        assert!(zooms(frame(vec![key(egui::Key::ArrowUp)])).is_empty());
        // a press on it gives it the keys (the zoom is 0.5 in `small_frame`)
        frame(vec![egui::Event::PointerMoved(rail), button(rail, true)]);
        frame(vec![button(rail, false)]);
        frame(vec![egui::Event::PointerMoved(away)]);
        assert_eq!(zooms(frame(vec![key(egui::Key::ArrowUp)])), vec![0.5 + SMALL_ZOOM_STEP]);
        assert_eq!(zooms(frame(vec![key(egui::Key::ArrowLeft)])), vec![0.5 - SMALL_ZOOM_STEP]);
        // the wheel, over the slider only: rolled up, it zooms in
        assert!(zooms(frame(vec![wheel(1.0)])).is_empty());
        frame(vec![egui::Event::PointerMoved(rail)]);
        assert_eq!(zooms(frame(vec![wheel(1.0)])), vec![0.5 + SMALL_ZOOM_STEP]);
        assert_eq!(zooms(frame(vec![wheel(-2.0)])), vec![0.5 - 2.0 * SMALL_ZOOM_STEP]);
    }

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
    fn camera_labels_fit_at_normal_and_double_text_size() {
        for scale in [1.0, 2.0] {
            for classic in [false, true] {
                let ctx = egui::Context::default();
                let theme = crate::theme::Theme::default();
                theme.apply(&ctx, scale);
                let p = theme.palette();
                let keys = KeyBindings::default();
                let views = SavedViews::default();
                let view = View {
                    zoom: 0.5,
                    roll: 0.0,
                    preset: CameraPreset::Rear,
                    shots: &[],
                    views: &views,
                    flying: false,
                    always_run: false,
                    seated: false,
                    run_locked: false,
                    fly_locked: false,
                    stand_locked: false,
                    mouselook: false,
                    object_view: false,
                    keys: &keys,
                    free: None,
                };
                let mut controls = ControlsUi::default();
                let (mut camera, mut movement) = (true, false);
                let mut output = None;
                for frame in 0..4 {
                    let mut frame_output = ctx.run_ui(
                        egui::RawInput {
                            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 1000.0))),
                            time: Some(f64::from(frame) * 0.1),
                            ..Default::default()
                        },
                        |ui| {
                            controls.show(ui.ctx(), &p, &mut camera, &mut movement, &mut Compact::default(), classic, &view);
                        },
                    );
                    frame_output.textures_delta.clear();
                    output = Some(frame_output);
                }
                let mut labels = vec![
                    aurora_i18n::tr!("controls-orbit"),
                    aurora_i18n::tr!("controls-pan"),
                    short_view(CameraPreset::Rear),
                    short_view(CameraPreset::Front),
                    short_view(CameraPreset::Side),
                    short_view(CameraPreset::Shoulder),
                ];
                if !classic {
                    labels.extend([
                        aurora_i18n::tr!("controls-tab-camera"),
                        aurora_i18n::tr!("controls-tab-movement"),
                        aurora_i18n::tr!("controls-object-view"),
                        aurora_i18n::tr!("common-mouselook"),
                        aurora_i18n::tr!("controls-shots-none"),
                    ]);
                }
                let output = output.unwrap();
                let bounds: Vec<_> = labels
                    .iter()
                    .map(|label| {
                        output
                            .shapes
                            .iter()
                            .find_map(|shape| match &shape.shape {
                                egui::Shape::Text(text) if text.galley.text() == *label => {
                                    Some((*label, Rect::from_min_size(text.pos, text.galley.size())))
                                }
                                _ => None,
                            })
                            .unwrap_or_else(|| panic!("missing {label}, scale={scale}, classic={classic}"))
                    })
                    .collect();
                for (index, (label, rect)) in bounds.iter().enumerate() {
                    assert!(
                        rect.left() >= 0.0 && rect.right() <= 900.0,
                        "{label}: {rect:?}, scale={scale}, classic={classic}"
                    );
                    for (other, other_rect) in &bounds[index + 1..] {
                        assert!(
                            !rect.shrink(0.5).intersects(other_rect.shrink(0.5)),
                            "{label} {rect:?} overlaps {other} {other_rect:?}, scale={scale}, classic={classic}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn french_stick_captions_use_their_whole_columns_at_double_text_size() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 2.0);
        let p = theme.palette();
        let row = Rect::from_min_size(egui::pos2(10.0, 10.0), Vec2::new(414.0, STICK));
        let at = SticksRow::new(row);
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            stick_captions(ui, &p, &at, row.bottom(), ["Orbite", "Panoramique"]);
        });
        output.textures_delta.clear();
        for (name, left, right) in [
            ("Orbite", row.left(), at.zoom.left()),
            ("Panoramique", at.zoom.right(), row.right()),
        ] {
            let text = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == name => Some(text),
                    _ => None,
                })
                .expect("the joystick caption");
            assert_eq!(text.galley.rows.len(), 1, "{name} should fit on one line");
            let bounds = Rect::from_min_size(text.pos, text.galley.size());
            assert!(
                bounds.left() >= left && bounds.right() <= right,
                "{name}: {bounds:?} leaves its column {left}..{right}"
            );
        }
    }

    #[test]
    fn control_tab_face_contains_double_size_text() {
        let ctx = egui::Context::default();
        let theme = crate::theme::Theme::default();
        theme.apply(&ctx, 2.0);
        let p = theme.palette();
        let label = aurora_i18n::tr!("controls-tab-movement");
        let mut rect = Rect::NOTHING;
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            rect = widgets::pill_tab_wide(ui, &p, "walk", label, true, tab_min_width(ui.ctx(), false), true, "").rect;
        });
        output.textures_delta.clear();
        let text = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => Some(Rect::from_min_size(text.pos, text.galley.size())),
                _ => None,
            })
            .expect("the tab label");
        assert!(rect.contains_rect(text), "{rect:?} must contain {text:?}");
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
        // (in the viewer's language: "0.5 m/s", "0,5 m/s" or "0.5ｍ／ｓ")
        let written = |speed: &str| aurora_i18n::tr!("controls-free-speed-value", speed = speed);
        assert_eq!(speed_text(2.0), written("2"));
        assert!(speed_text(0.5).starts_with('0') && speed_text(0.5).contains('5'));
        assert_eq!(speed_text(12.4), written("12"), "whole metres from 10");
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
                roll: 0.0,
                preset: CameraPreset::Rear,
                shots: &shots,
                views: &SavedViews::default(),
                flying: false,
                always_run: false,
                seated: false,
                run_locked: false,
                fly_locked: false,
                stand_locked: false,
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

    #[test]
    fn the_run_and_fly_modes_are_greyed_when_refused_and_landing_never_is() {
        let keys = KeyBindings::default();
        let views = SavedViews::default();
        let view = |flying: bool, run_locked: bool, fly_locked: bool| View {
            zoom: 0.5,
            roll: 0.0,
            preset: CameraPreset::Rear,
            shots: &[],
            views: &views,
            flying,
            always_run: false,
            seated: false,
            run_locked,
            fly_locked,
            stand_locked: false,
            mouselook: false,
            object_view: false,
            keys: &keys,
            free: None,
        };
        let free = view(false, false, false);
        for action in [ControlAction::Walk, ControlAction::Run, ControlAction::Fly] {
            assert!(!mode_locked(&free, &action), "{action:?}");
        }
        // @alwaysrun: Run alone
        let no_run = view(false, true, false);
        assert!(mode_locked(&no_run, &ControlAction::Run));
        assert!(!mode_locked(&no_run, &ControlAction::Walk) && !mode_locked(&no_run, &ControlAction::Fly));
        // no takeoff: Fly alone, and only on the ground
        let no_fly = view(false, false, true);
        assert!(mode_locked(&no_fly, &ControlAction::Fly));
        assert!(!mode_locked(&no_fly, &ControlAction::Walk) && !mode_locked(&no_fly, &ControlAction::Run));
        assert!(
            !mode_locked(&view(true, false, true), &ControlAction::Fly),
            "the Fly mode still lands"
        );
    }
}
