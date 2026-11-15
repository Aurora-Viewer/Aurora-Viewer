//! The Director's interface (`crate::director`): its window, the timeline
//! strip above the bottom bar, the track drawn in the world and what shows
//! while it plays. Aurora's own: the Legacy Viewer has no camera path tool.
//!
//! - The window: the live camera (view angle, roll, free camera), the
//!   keyframe selected, the path's options, the playback's, and the
//!   environment block of Quick preferences (`EnvironmentUi::selector_body`).
//! - The strip: back to the start, play, loop, "Keyframe", then the ruler
//!   with the keyframes (click: select; drag: move in time; double click:
//!   the camera goes there) and the playhead (held: the view follows it).
//! - In the world: the path, a dot each second (close dots: a slow camera),
//!   a small camera at each keyframe showing where it looks and how wide,
//!   and the Bézier handles of the selected one.
//! - Playing: nothing but the countdown of the start delay.

use super::environment::{EnvAction, EnvironmentUi};
use super::hud::Projector;
use super::viewfinder::{Isolate, IsolateRow};
use super::widgets::{self, ActionKind, Floater};
use crate::camera::track::{self, Curve, Keyframe, Pose};
use crate::director::{self, Director, Format, Joint, Mode, Target};
use crate::recorder;
use crate::snapshot::isolate::{Isolation, Mode as Shown};
use crate::theme::Palette;
use crate::world::eep_env::LocalView;
use egui::{Align2, Color32, FontId, Id, Pos2, Rect, RichText, Sense, Stroke, Vec2};
use glam::Vec3;
use std::ops::RangeInclusive;

/// The window's id.
pub const WINDOW: &str = "director";

const WIDTH: f32 = 340.0;
const LABEL_W: f32 = 112.0;
const SLIDER_W: f32 = 120.0;
/// What the window keeps clear of the room the bars leave: its own title
/// and margins, the timeline strip under it, and a little air.
const WINDOW_MARGIN: f32 = 230.0;
/// The body is never shorter than this, however small the viewer's window.
const WINDOW_MIN_BODY: f32 = 220.0;
/// The view angle's slider, degrees (within `camera::FOV_RANGE`).
const LENS_DEGREES: RangeInclusive<f32> = 10.0..=170.0;
const ROLL_DEGREES: RangeInclusive<f32> = -180.0..=180.0;

/// Space between the strip and the bottom bar, and the window's sides.
const STRIP_GAP: f32 = 8.0;
const STRIP_MAX_W: f32 = 1100.0;
const STRIP_BUTTON: Vec2 = Vec2::new(30.0, 30.0);
const RULER_H: f32 = 40.0;
const LANE_H: f32 = 16.0;
const RULER_PAD: f32 = 12.0;
const KEY_HALF: f32 = 6.0;
const CLOCK_W: f32 = 112.0;
/// The ruler shows this many seconds at least.
const MIN_SPAN: f32 = 12.0;
/// Two labelled ticks of the ruler are this far apart at least.
const TICK_ROOM: f32 = 52.0;

/// The speed a span can be given, metres a second.
const SPAN_SPEED: RangeInclusive<f32> = 0.01..=500.0;
/// A span of the ruler writes its speed when it is this wide at least.
const SPEED_ROOM: f32 = 64.0;

/// Straight pieces a span of the path is drawn with.
const PATH_STEPS: usize = 24;
/// A keyframe's camera: half the height of its frame, metres, and how far
/// the frame may stand from the camera (a long lens draws a long camera).
const CAMERA_HALF: f32 = 0.22;
const CAMERA_DEPTH: RangeInclusive<f32> = 0.2..=1.4;
/// The side of a keyframe's click target in the world.
const MARKER: f32 = 24.0;

/// What the Director asks of the app: what needs the camera.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    /// A keyframe of the current view.
    AddKey,
    /// The current view into this keyframe.
    Update(usize),
    /// The camera to this keyframe.
    GoTo(usize),
    Play,
    /// The view angle, radians.
    Lens(f32),
    /// The view's roll, radians.
    Roll(f32),
    Free(bool),
    /// The free camera's speed, metres a second.
    FreeSpeed(f32),
    /// Lock the view on what the camera is focused on (Alt + click): the
    /// avatar it belongs to, or the object.
    LockFocus,
    /// The track as it was before its last change, and that change again.
    Undo,
    Redo,
    /// Start a film, or end the one being recorded.
    Record,
    /// Choose the ffmpeg to record with, in a file dialog.
    ChooseFfmpeg,
    /// Look for ffmpeg again.
    FindFfmpeg,
    /// Open ffmpeg's download page.
    FfmpegSite,
    /// Show the last film in its folder.
    RevealFilm,
    /// Save the track under the name typed, or load the saved one at this
    /// place of the list.
    SaveTrack,
    LoadTrack(usize),
}

/// The saved tracks as the window shows them.
pub struct Tracks<'a> {
    /// The name being typed.
    pub name: &'a mut String,
    /// The names of the tracks saved for this account.
    pub saved: &'a [String],
    /// What the last save or load did, or why it failed.
    pub note: Option<Result<&'a str, &'a str>>,
}

/// The live camera and what restricts it, as the app gives them.
#[derive(Debug, Clone, Copy)]
pub struct Live {
    /// The view angle (the setting), radians.
    pub fov: f32,
    pub roll: f32,
    pub free: bool,
    pub free_speed: f32,
    /// RLVa holds the view angle.
    pub lens_locked: bool,
    /// RLVa restricts the camera: no playback, no scrub, no lock.
    pub restricted: bool,
    /// The camera is focused on an avatar or an object: the lock can take it.
    pub focus: bool,
    /// Where the lock's target is now (smoothed); `None` when it is not in
    /// sight, or nothing needs it.
    pub target: Option<Vec3>,
    /// The track has a change to undo, an undone one to redo.
    pub can_undo: bool,
    pub can_redo: bool,
    /// An ffmpeg is there to record with.
    pub can_record: bool,
    /// A film is being recorded.
    pub recording: bool,
}

/// The recording as the window shows it (`recorder.rs`).
pub struct Record<'a> {
    pub options: &'a mut recorder::Options,
    /// The options changed: to be saved.
    pub changed: &'a mut bool,
    /// The ffmpeg found (its file), `Some(None)` when there is none; `None`
    /// while it is looked for.
    pub ffmpeg: Option<Option<&'a std::path::Path>>,
    /// The formats that ffmpeg writes, in the order of `recorder::Format::ALL`.
    pub formats: [bool; 5],
    /// Seconds of film so far, while one is recorded.
    pub recording: Option<f64>,
    /// The encoder is finishing the last film's file.
    pub finishing: bool,
    /// The last film: its file, or why it could not be written.
    pub last: Option<Result<&'a std::path::Path, &'a str>>,
}

/// The photo's isolation as the window shows it (`snapshot::isolate`): its
/// state, the names of what is chosen, and what the section asks.
pub struct IsolateView<'a> {
    pub isolation: &'a Isolation,
    pub rows: &'a [IsolateRow],
    pub out: &'a mut Vec<Isolate>,
}

/// The environment block's needs (those of Quick preferences).
pub struct Env<'a> {
    pub ui: &'a mut EnvironmentUi,
    pub view: LocalView,
    pub loading: bool,
    pub time_of_day: u8,
    pub lighting: &'a mut bool,
    pub out: &'a mut Vec<EnvAction>,
}

fn row<R>(ui: &mut egui::Ui, p: &Palette, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(Vec2::new(LABEL_W, 20.0), Sense::hover());
        ui.painter()
            .text(rect.left_center(), Align2::LEFT_CENTER, label, FontId::proportional(12.5), p.ink);
        add(ui)
    })
    .inner
}

/// An angle in radians as the sliders show it, in whole degrees: under
/// half a degree it is 0, never the "-0°" a rounding leaves on a level view.
fn shown_degrees(radians: f32) -> f32 {
    let degrees = radians.to_degrees();
    if degrees.abs() < 0.5 { 0.0 } else { degrees }
}

/// A slider of an angle kept in radians and shown in degrees: the new angle
/// when it moved.
fn degrees(ui: &mut egui::Ui, value: f32, range: RangeInclusive<f32>, fill: bool) -> Option<f32> {
    let mut shown = shown_degrees(value).clamp(*range.start(), *range.end());
    ui.spacing_mut().slider_width = SLIDER_W;
    let slider = egui::Slider::new(&mut shown, range)
        .custom_parser(aurora_i18n::parse_number)
        .fixed_decimals(0)
        .suffix("°")
        .trailing_fill(fill);
    ui.add(slider).changed().then(|| shown.to_radians())
}

/// "m:ss", with the tenths when `tenths`.
fn clock(seconds: f32, tenths: bool) -> String {
    let total = (seconds.max(0.0) * 10.0).round() as u32;
    let (minutes, rest, tenth) = (total / 600, (total / 10) % 60, total % 10);
    if tenths {
        aurora_i18n::number_text(&format!("{minutes}:{rest:02}.{tenth}"))
    } else {
        format!("{minutes}:{rest:02}")
    }
}

/// The window: the camera, the lock, the selected keyframe, the path, the
/// playback, the picture and the environment, each a section that folds
/// (the first four open at first), in a body that scrolls when the room
/// the bars leave is too short for them all.
#[allow(clippy::too_many_arguments)]
pub fn window(
    ctx: &egui::Context,
    p: &Palette,
    open: &mut bool,
    d: &mut Director,
    live: &Live,
    isolate: IsolateView<'_>,
    record: Record<'_>,
    tracks: Tracks<'_>,
    env: Env<'_>,
    actions: &mut Vec<Action>,
) {
    if !*open {
        return;
    }
    let screen = ctx.content_rect();
    let pos = egui::pos2(screen.right() - WIDTH - 24.0, screen.top() + 90.0);
    let room = (super::layout::work_area(ctx).height() - WINDOW_MARGIN).max(WINDOW_MIN_BODY);
    Floater::new(WINDOW, aurora_i18n::tr!("director-title"), pos, Vec2::new(WIDTH, 560.0))
        .auto_height()
        .show(ctx, p, open, |ui| {
            egui::ScrollArea::vertical()
                .max_height(room)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
                    widgets::collapsing(ui, p, "director_camera", aurora_i18n::tr!("director-camera"), true, |ui| {
                        camera_section(ui, p, live, actions);
                    });
                    widgets::collapsing(ui, p, "director_lock", aurora_i18n::tr!("director-lock"), true, |ui| {
                        lock_section(ui, p, d, live, actions);
                    });
                    let keyframe = match d.selected.filter(|index| *index < d.track.len()) {
                        Some(index) => aurora_i18n::tr!("director-keyframe", number = index + 1),
                        None => aurora_i18n::tr!("director-keyframe-title").to_owned(),
                    };
                    widgets::collapsing(ui, p, "director_keyframe", keyframe, true, |ui| {
                        keyframe_section(ui, p, d, actions);
                    });
                    widgets::collapsing(ui, p, "director_path", aurora_i18n::tr!("director-path"), true, |ui| {
                        path_section(ui, p, d);
                    });
                    widgets::collapsing(ui, p, "director_playback", aurora_i18n::tr!("director-playback"), false, |ui| {
                        playback_section(ui, p, d);
                    });
                    widgets::collapsing(ui, p, "director_picture", aurora_i18n::tr!("director-picture"), false, |ui| {
                        picture_section(ui, p, d, live.target.is_some());
                    });
                    widgets::collapsing(ui, p, "director_record", aurora_i18n::tr!("director-record"), true, |ui| {
                        record_section(ui, p, record, live, actions);
                    });
                    widgets::collapsing(ui, p, "director_tracks", aurora_i18n::tr!("director-tracks"), false, |ui| {
                        tracks_section(ui, p, tracks, d.track.is_empty(), actions);
                    });
                    widgets::collapsing(ui, p, "director_isolate", aurora_i18n::tr!("photo-isolate"), false, |ui| {
                        isolate_section(ui, p, isolate.isolation, isolate.rows, isolate.out, &mut d.key_color);
                    });
                    let Env {
                        ui: lists,
                        view,
                        loading,
                        time_of_day,
                        lighting,
                        out,
                    } = env;
                    widgets::collapsing(
                        ui,
                        p,
                        "director_environment",
                        aurora_i18n::tr!("director-environment"),
                        false,
                        |ui| {
                            lists.selector_body(ui, p, view, loading, Some(time_of_day), LABEL_W, lighting, None, out);
                        },
                    );
                });
        });
}

fn camera_section(ui: &mut egui::Ui, p: &Palette, live: &Live, actions: &mut Vec<Action>) {
    row(ui, p, aurora_i18n::tr!("director-lens"), |ui| {
        ui.add_enabled_ui(!live.lens_locked, |ui| {
            if let Some(fov) = degrees(ui, live.fov, LENS_DEGREES, true) {
                actions.push(Action::Lens(fov));
            }
        });
    });
    row(ui, p, aurora_i18n::tr!("director-roll"), |ui| {
        if let Some(roll) = degrees(ui, live.roll, ROLL_DEGREES, false) {
            actions.push(Action::Roll(roll));
        }
        if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("director-roll-reset"), live.roll != 0.0) {
            actions.push(Action::Roll(0.0));
        }
    });
    let mut free = live.free;
    if widgets::switch_labeled(ui, p, &mut free, aurora_i18n::tr!("director-free-camera"))
        .on_hover_text(aurora_i18n::tr!("director-free-camera-tip"))
        .changed()
    {
        actions.push(Action::Free(free));
    }
    if live.free {
        row(ui, p, aurora_i18n::tr!("director-free-speed"), |ui| {
            let range = crate::camera::free::SPEED_RANGE;
            let mut speed = live.free_speed.clamp(*range.start(), *range.end());
            ui.spacing_mut().slider_width = SLIDER_W;
            let slider = egui::Slider::new(&mut speed, range)
                .custom_parser(aurora_i18n::parse_number)
                .fixed_decimals(1)
                .suffix(" m/s")
                .logarithmic(true);
            if ui.add(slider).changed() {
                actions.push(Action::FreeSpeed(speed));
            }
        });
    }
}

fn joint_label(joint: Joint) -> &'static str {
    match joint {
        Joint::Head => aurora_i18n::tr!("director-joint-head"),
        Joint::Neck => aurora_i18n::tr!("director-joint-neck"),
        Joint::Chest => aurora_i18n::tr!("director-joint-chest"),
        Joint::Pelvis => aurora_i18n::tr!("director-joint-pelvis"),
        Joint::LeftHand => aurora_i18n::tr!("director-joint-left-hand"),
        Joint::RightHand => aurora_i18n::tr!("director-joint-right-hand"),
    }
}

/// A format as the list writes it: its ratio with the language's decimal
/// separator, or "Window".
fn format_label(format: Format) -> String {
    match format.ratio() {
        Some(ratio) => aurora_i18n::number_text(ratio),
        None => aurora_i18n::tr!("director-format-window").to_owned(),
    }
}

/// The lock of the view on a target: live on the user's own camera, with
/// no keyframe needed, and kept by the keyframes taken meanwhile.
fn lock_section(ui: &mut egui::Ui, p: &Palette, d: &mut Director, live: &Live, actions: &mut Vec<Action>) {
    ui.add_enabled_ui(!live.restricted, |ui| {
        widgets::switch_labeled(ui, p, &mut d.lock.on, aurora_i18n::tr!("director-lock-on"))
            .on_hover_text(aurora_i18n::tr!("director-lock-on-tip"));
    });
    row(ui, p, aurora_i18n::tr!("director-lock-target"), |ui| {
        let name = match d.lock.target {
            Target::Own => aurora_i18n::tr!("director-lock-own"),
            Target::Avatar(_) => aurora_i18n::tr!("director-lock-avatar"),
            Target::Object(_) => aurora_i18n::tr!("director-lock-object"),
        };
        ui.label(RichText::new(name).size(12.5).color(p.ink));
    });
    ui.horizontal(|ui| {
        let size = Vec2::new(0.0, 26.0);
        if widgets::action_button(
            ui,
            p,
            "user-filled",
            aurora_i18n::tr!("director-lock-pick-own"),
            ActionKind::Flat,
            size,
        )
        .clicked()
        {
            d.lock.target = Target::Own;
        }
        if widgets::action_button_if(
            ui,
            p,
            "eye-filled",
            aurora_i18n::tr!("director-lock-pick-focus"),
            ActionKind::Flat,
            size,
            live.focus,
        )
        .on_hover_text(aurora_i18n::tr!("director-lock-pick-focus-tip"))
        .on_disabled_hover_text(aurora_i18n::tr!("director-lock-pick-focus-tip"))
        .clicked()
        {
            actions.push(Action::LockFocus);
        }
    });
    if !matches!(d.lock.target, Target::Object(_)) {
        row(ui, p, aurora_i18n::tr!("director-lock-joint"), |ui| {
            egui::ComboBox::from_id_salt("director_joint")
                .selected_text(joint_label(d.lock.joint))
                .show_ui(ui, |ui| {
                    for joint in Joint::ALL {
                        ui.selectable_value(&mut d.lock.joint, joint, joint_label(joint));
                    }
                });
        });
    }
    row(ui, p, aurora_i18n::tr!("director-lock-smoothing"), |ui| {
        ui.spacing_mut().slider_width = SLIDER_W;
        let slider = egui::Slider::new(&mut d.lock.smoothing, director::SMOOTHING_RANGE)
            .custom_parser(aurora_i18n::parse_number)
            .fixed_decimals(2)
            .suffix(" s")
            .trailing_fill(true);
        ui.add(slider).on_hover_text(aurora_i18n::tr!("director-lock-smoothing-tip"));
    });
    // a lock that is asked for and finds nothing says so: the view is free
    let wanted = d.lock.on || d.track.keys().iter().any(|key| key.locked);
    if live.restricted {
        ui.label(RichText::new(aurora_i18n::tr!("director-restricted")).size(12.0).color(p.warn));
    } else if wanted && live.target.is_none() {
        ui.label(RichText::new(aurora_i18n::tr!("director-lock-lost")).size(12.0).color(p.warn));
    }
}

/// The picture's proportions, the guides drawn over it and the lens's
/// depth of field (`locked`: the view has a target in sight, it focuses
/// on it).
fn picture_section(ui: &mut egui::Ui, p: &Palette, d: &mut Director, locked: bool) {
    row(ui, p, aurora_i18n::tr!("director-format"), |ui| {
        egui::ComboBox::from_id_salt("director_format")
            .selected_text(format_label(d.format))
            .show_ui(ui, |ui| {
                for format in Format::ALL {
                    ui.selectable_value(&mut d.format, format, format_label(format));
                }
            })
            .response
            .on_hover_text(aurora_i18n::tr!("director-format-tip"));
    });
    widgets::checkbox(ui, p, &mut d.thirds, aurora_i18n::tr!("director-thirds"));
    // the Director's own lens: its blur, and where it is sharp
    let mut on = d.dof.is_some();
    if widgets::checkbox(ui, p, &mut on, aurora_i18n::tr!("director-dof"))
        .on_hover_text(aurora_i18n::tr!("director-dof-tip"))
        .changed()
    {
        d.dof = on.then_some(director::DEFAULT_APERTURE);
    }
    if let Some(aperture) = &mut d.dof {
        row(ui, p, aurora_i18n::tr!("director-dof-aperture"), |ui| {
            ui.spacing_mut().slider_width = SLIDER_W;
            ui.add(
                egui::Slider::new(aperture, director::APERTURE_RANGE)
                    .custom_parser(aurora_i18n::parse_number)
                    .fixed_decimals(1)
                    .prefix("f/")
                    .logarithmic(true),
            );
        });
        if locked {
            ui.label(RichText::new(aurora_i18n::tr!("director-dof-on-target")).size(12.0).color(p.muted));
        } else {
            row(ui, p, aurora_i18n::tr!("director-dof-focus"), |ui| {
                ui.spacing_mut().slider_width = SLIDER_W;
                ui.add(
                    egui::Slider::new(&mut d.focus, director::FOCUS_RANGE)
                        .custom_parser(aurora_i18n::parse_number)
                        .fixed_decimals(1)
                        .suffix(" m")
                        .logarithmic(true),
                );
            });
        }
    }
}

/// The saved tracks: a name to save this one under, and those saved
/// before, each loaded by its button (the track being edited is replaced:
/// Ctrl+Z brings it back).
fn tracks_section(ui: &mut egui::Ui, p: &Palette, tracks: Tracks<'_>, empty: bool, actions: &mut Vec<Action>) {
    let Tracks { name, saved, note } = tracks;
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(name)
                .hint_text(aurora_i18n::tr!("director-track-name"))
                .desired_width(170.0),
        );
        let valid = track::file_stem(name).is_some() && !empty;
        if widgets::action_button_if(
            ui,
            p,
            "device-floppy-filled",
            aurora_i18n::tr!("director-track-save"),
            ActionKind::Flat,
            Vec2::new(0.0, 26.0),
            valid,
        )
        .clicked()
        {
            actions.push(Action::SaveTrack);
        }
    });
    if saved.is_empty() {
        ui.label(RichText::new(aurora_i18n::tr!("director-track-none")).size(12.0).color(p.muted));
    }
    for (index, saved_name) in saved.iter().enumerate() {
        ui.horizontal(|ui| {
            if widgets::icon_button(ui, p, "folder-open-filled", aurora_i18n::tr!("director-track-load"), true) {
                actions.push(Action::LoadTrack(index));
            }
            ui.add(egui::Label::new(RichText::new(saved_name).size(12.5).color(p.ink)).truncate());
        });
    }
    match note {
        Some(Ok(text)) => {
            ui.label(RichText::new(text).size(12.0).color(p.success));
        }
        Some(Err(text)) => {
            ui.label(RichText::new(text).size(12.0).color(p.warn));
        }
        None => {}
    }
}

fn film_format(format: recorder::Format) -> &'static str {
    match format {
        recorder::Format::Mp4 => aurora_i18n::tr!("director-record-format-mp4"),
        recorder::Format::ProRes => aurora_i18n::tr!("director-record-format-prores"),
        recorder::Format::ProResAlpha => aurora_i18n::tr!("director-record-format-prores-alpha"),
        recorder::Format::Webm => aurora_i18n::tr!("director-record-format-webm"),
        recorder::Format::Lossless => aurora_i18n::tr!("director-record-format-lossless"),
    }
}

/// The recording: the ffmpeg it goes through (or how to get one), the
/// film's format, rate and quality, the button, and the last film.
fn record_section(ui: &mut egui::Ui, p: &Palette, record: Record<'_>, live: &Live, actions: &mut Vec<Action>) {
    let Record {
        options,
        changed,
        ffmpeg,
        formats,
        recording,
        finishing,
        last,
    } = record;
    let size = Vec2::new(0.0, 26.0);
    let note = |ui: &mut egui::Ui, text: &str, color: Color32| {
        ui.label(RichText::new(text).size(12.0).color(color));
    };
    match ffmpeg {
        None => note(ui, aurora_i18n::tr!("director-record-looking"), p.muted),
        Some(None) => {
            note(ui, aurora_i18n::tr!("director-record-missing"), p.warn);
            note(ui, aurora_i18n::tr!("director-record-missing-hint"), p.muted);
            ui.horizontal_wrapped(|ui| {
                for (icon, label, action) in [
                    ("external-link", aurora_i18n::tr!("director-record-site"), Action::FfmpegSite),
                    (
                        "folder-open-filled",
                        aurora_i18n::tr!("director-record-choose"),
                        Action::ChooseFfmpeg,
                    ),
                    ("refresh", aurora_i18n::tr!("director-record-again"), Action::FindFfmpeg),
                ] {
                    if widgets::action_button(ui, p, icon, label, ActionKind::Flat, size).clicked() {
                        actions.push(action);
                    }
                }
            });
        }
        Some(Some(path)) => {
            ui.horizontal(|ui| {
                let name = path.file_name().unwrap_or(path.as_os_str()).to_string_lossy();
                ui.label(
                    RichText::new(aurora_i18n::tr!("director-record-found", name = name.as_ref()))
                        .size(12.0)
                        .color(p.muted),
                )
                .on_hover_text(path.to_string_lossy());
                if widgets::icon_button(ui, p, "folder-open-filled", aurora_i18n::tr!("director-record-choose"), true) {
                    actions.push(Action::ChooseFfmpeg);
                }
            });
        }
    }
    let before = options.clone();
    row(ui, p, aurora_i18n::tr!("director-record-format"), |ui| {
        egui::ComboBox::from_id_salt("director_record_format")
            .selected_text(film_format(options.format))
            .show_ui(ui, |ui| {
                for (format, available) in recorder::Format::ALL.into_iter().zip(formats) {
                    // (a format the ffmpeg found cannot write is shown, greyed)
                    ui.add_enabled_ui(available || ffmpeg.is_none(), |ui| {
                        ui.selectable_value(&mut options.format, format, film_format(format));
                    });
                }
            });
    });
    row(ui, p, aurora_i18n::tr!("director-record-fps"), |ui| {
        options.fps = options.rate();
        egui::ComboBox::from_id_salt("director_record_fps")
            .selected_text(options.fps.to_string())
            .show_ui(ui, |ui| {
                for rate in recorder::RATES {
                    ui.selectable_value(&mut options.fps, rate, rate.to_string());
                }
            });
    });
    row(ui, p, aurora_i18n::tr!("director-record-quality"), |ui| {
        let levels = [recorder::Quality::Light, recorder::Quality::Good, recorder::Quality::Best];
        let mut level = levels.iter().position(|quality| *quality == options.quality).unwrap_or(1);
        let names = [
            (aurora_i18n::tr!("director-record-quality-light"), true),
            (aurora_i18n::tr!("director-record-quality-good"), true),
            (aurora_i18n::tr!("director-record-quality-best"), true),
        ];
        if widgets::segmented(ui, p, &mut level, &names) {
            options.quality = levels[level];
        }
    });
    *changed |= *options != before;
    if options.format.alpha() {
        note(ui, aurora_i18n::tr!("director-record-alpha-tip"), p.muted);
    }
    note(ui, aurora_i18n::tr!("director-record-no-sound"), p.muted);
    let (icon, label, kind) = if recording.is_some() {
        ("player-stop-filled", aurora_i18n::tr!("director-record-stop"), ActionKind::Danger)
    } else {
        ("player-record-filled", aurora_i18n::tr!("director-record-start"), ActionKind::Flat)
    };
    let tip = aurora_i18n::tr!("director-record-tip");
    if widgets::action_button_if(ui, p, icon, label, kind, size, live.can_record && !finishing)
        .on_hover_text(tip)
        .on_disabled_hover_text(tip)
        .clicked()
    {
        actions.push(Action::Record);
    }
    if let Some(seconds) = recording {
        let text = aurora_i18n::tr!("director-record-recording", time = clock(seconds as f32, true));
        note(ui, &text, p.danger);
    } else if finishing {
        note(ui, aurora_i18n::tr!("director-record-finishing"), p.muted);
    } else if let Some(film) = last {
        match film {
            Ok(file) => {
                let name = file.file_name().unwrap_or(file.as_os_str()).to_string_lossy();
                let text = aurora_i18n::tr!("director-record-saved", name = name.as_ref());
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.success)).truncate())
                        .on_hover_text(file.to_string_lossy());
                });
                if widgets::action_button(
                    ui,
                    p,
                    "folder-open-filled",
                    aurora_i18n::tr!("director-record-reveal"),
                    ActionKind::Flat,
                    size,
                )
                .clicked()
                {
                    actions.push(Action::RevealFilm);
                }
            }
            Err(why) => {
                let text = aurora_i18n::tr!("director-record-failed", why = why);
                note(ui, &text, p.warn);
            }
        }
    }
}

/// The photo mode's isolation, for the film: everything, our avatar alone,
/// or the avatars and objects chosen in the scene, on an empty background or
/// not. The same state, rule and choices as the photo's (`snapshot::isolate`,
/// `ui::viewfinder::isolate`): what is chosen here is chosen there.
fn isolate_section(
    ui: &mut egui::Ui,
    p: &Palette,
    iso: &Isolation,
    rows: &[IsolateRow],
    out: &mut Vec<Isolate>,
    key: &mut Option<[u8; 3]>,
) {
    let modes = [Shown::Off, Shown::Own, Shown::Chosen];
    let mut mode = modes.iter().position(|shown| *shown == iso.mode).unwrap_or(0);
    let options = [
        (aurora_i18n::tr!("photo-isolate-off"), true),
        (aurora_i18n::tr!("photo-isolate-own"), true),
        (aurora_i18n::tr!("photo-isolate-chosen"), true),
    ];
    if widgets::segmented(ui, p, &mut mode, &options) {
        out.push(Isolate::Mode(modes[mode]));
    }
    if iso.mode == Shown::Chosen {
        if rows.is_empty() {
            ui.label(RichText::new(aurora_i18n::tr!("photo-isolate-none")).size(12.0).color(p.muted));
        }
        for row in rows {
            ui.horizontal(|ui| {
                if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("photo-isolate-remove"), true) {
                    out.push(Isolate::Remove(row.id));
                }
                ui.add(egui::Label::new(RichText::new(&row.name).size(12.5).color(p.ink)).truncate());
            });
        }
        let (glyph, label, kind) = if iso.picking {
            ("check", aurora_i18n::tr!("photo-isolate-pick-done"), ActionKind::Primary)
        } else {
            ("pointer-filled", aurora_i18n::tr!("photo-isolate-pick"), ActionKind::Flat)
        };
        if widgets::action_button(ui, p, glyph, label, kind, Vec2::new(0.0, 26.0)).clicked() {
            out.push(Isolate::Picking(!iso.picking));
        }
        if iso.picking {
            ui.label(RichText::new(aurora_i18n::tr!("photo-isolate-pick-hint")).size(12.0).color(p.muted));
        }
    }
    if iso.mode != Shown::Off {
        let mut empty = iso.transparent;
        if widgets::checkbox(ui, p, &mut empty, aurora_i18n::tr!("director-isolate-empty"))
            .on_hover_text(aurora_i18n::tr!("director-isolate-empty-tip"))
            .changed()
        {
            out.push(Isolate::Transparent(empty));
        }
        // a flat color in place of the empty background: a green screen
        ui.add_enabled_ui(iso.transparent, |ui| {
            ui.horizontal(|ui| {
                let mut on = key.is_some();
                if widgets::checkbox(ui, p, &mut on, aurora_i18n::tr!("director-key-color"))
                    .on_hover_text(aurora_i18n::tr!("director-key-color-tip"))
                    .changed()
                {
                    *key = on.then_some(director::KEY_GREEN);
                }
                if let Some([r, g, b]) = *key {
                    let mut color = [r, g, b, 255];
                    if widgets::color_button(ui, p, "director_key_color", &mut color, false).changed() {
                        *key = Some([color[0], color[1], color[2]]);
                    }
                }
            });
        });
    }
}

/// Choosing in the scene for the isolation: over the world, above the
/// timeline strip and under the windows, the pointer names what is under it
/// (`hover`: its name, and whether it is chosen) and a click adds it or
/// removes it, as in the photo mode's viewfinder.
pub fn isolate_picking(ctx: &egui::Context, p: &Palette, iso: &Isolation, hover: Option<&(String, bool)>, out: &mut Vec<Isolate>) {
    if !iso.picking {
        return;
    }
    let room = super::layout::work_area(ctx);
    // (the strip as it was last drawn: it takes its own clicks)
    let bottom = ctx
        .memory(|m| m.area_rect(Id::unique("director_timeline")))
        .map_or(room.bottom(), |strip| strip.top().min(room.bottom()));
    let size = Vec2::new(room.width(), (bottom - room.top()).max(0.0));
    egui::Area::new(Id::unique("director_isolate_pick"))
        .order(egui::Order::Background)
        .fixed_pos(room.min)
        .constrain(false)
        .show(ctx, |ui| {
            let (rect, response) = ui.allocate_exact_size(size, Sense::click());
            let Some(at) = response.hover_pos() else {
                return;
            };
            ctx.set_cursor_icon(egui::CursorIcon::Crosshair);
            out.push(Isolate::Hover(at));
            if response.clicked() && rect.contains(at) {
                out.push(Isolate::Click(at));
            }
            if let Some((name, chosen)) = hover {
                let text = if *chosen {
                    aurora_i18n::tr!("photo-isolate-hover-remove", name = name.as_str())
                } else {
                    aurora_i18n::tr!("photo-isolate-hover-add", name = name.as_str())
                };
                let (place, font) = (at + egui::vec2(16.0, 14.0), FontId::proportional(13.0));
                ui.painter()
                    .text(place + egui::vec2(1.0, 1.0), Align2::LEFT_TOP, &text, font.clone(), p.bar);
                ui.painter()
                    .text(place, Align2::LEFT_TOP, &text, font, if *chosen { p.danger } else { p.ink });
            }
        });
}

fn keyframe_section(ui: &mut egui::Ui, p: &Palette, d: &mut Director, actions: &mut Vec<Action>) {
    let Some((index, key)) = d.selected.and_then(|i| d.track.keys().get(i).map(|key| (i, *key))) else {
        ui.label(RichText::new(aurora_i18n::tr!("director-keyframe-none")).size(12.0).color(p.muted));
        return;
    };
    row(ui, p, aurora_i18n::tr!("director-key-time"), |ui| {
        // the first keyframe is where the track starts: always 0
        let mut time = key.time;
        let field = egui::DragValue::new(&mut time)
            .speed(0.05)
            .range(0.0..=track::MAX_TIME)
            .fixed_decimals(1)
            .custom_parser(aurora_i18n::parse_number)
            .suffix(" s");
        if ui
            .add_enabled(index > 0, field)
            .on_hover_text(aurora_i18n::tr!("director-key-time-tip"))
            .changed()
        {
            d.time = d.track.set_time(index, time);
        }
    });
    if index > 0 {
        span_rows(ui, p, d, index);
    }
    row(ui, p, aurora_i18n::tr!("director-lens"), |ui| {
        if let Some(fov) = degrees(ui, key.pose.fov, LENS_DEGREES, true) {
            d.track.set_pose(index, Pose { fov, ..key.pose });
        }
    });
    row(ui, p, aurora_i18n::tr!("director-roll"), |ui| {
        let (yaw, pitch, roll) = key.pose.angles();
        if let Some(roll) = degrees(ui, roll, ROLL_DEGREES, false) {
            d.track
                .set_pose(index, Pose::from_angles(key.pose.position, yaw, pitch, roll, key.pose.fov));
        }
    });
    let mut rest = key.rest;
    if widgets::checkbox(ui, p, &mut rest, aurora_i18n::tr!("director-key-rest"))
        .on_hover_text(aurora_i18n::tr!("director-key-rest-tip"))
        .changed()
    {
        d.track.set_rest(index, rest);
    }
    let mut locked = key.locked;
    if widgets::checkbox(ui, p, &mut locked, aurora_i18n::tr!("director-key-locked"))
        .on_hover_text(aurora_i18n::tr!("director-key-locked-tip"))
        .changed()
    {
        d.track.set_locked(index, locked);
    }
    ui.horizontal(|ui| {
        let size = Vec2::new(0.0, 26.0);
        if widgets::action_button(
            ui,
            p,
            "current-location-filled",
            aurora_i18n::tr!("director-go-to"),
            ActionKind::Flat,
            size,
        )
        .on_hover_text(aurora_i18n::tr!("director-go-to-tip"))
        .clicked()
        {
            actions.push(Action::GoTo(index));
        }
        if widgets::action_button(ui, p, "refresh", aurora_i18n::tr!("director-update"), ActionKind::Flat, size)
            .on_hover_text(aurora_i18n::tr!("director-update-tip"))
            .clicked()
        {
            actions.push(Action::Update(index));
        }
        if widgets::action_button(
            ui,
            p,
            "trash-filled",
            aurora_i18n::tr!("director-delete"),
            ActionKind::FlatDanger,
            size,
        )
        .clicked()
        {
            d.remove(index);
        }
    });
}

/// The way to keyframe `index` from the one before: the time it takes and
/// the camera's mean speed there. Changing either moves the keyframe and
/// all those after it, so the other spans keep their own pace.
fn span_rows(ui: &mut egui::Ui, p: &Palette, d: &mut Director, index: usize) {
    let mut changed = false;
    row(ui, p, aurora_i18n::tr!("director-span-time"), |ui| {
        let mut seconds = d.track.span_time(index).unwrap_or(track::MIN_GAP);
        let field = egui::DragValue::new(&mut seconds)
            .speed(0.05)
            .range(track::MIN_GAP..=track::MAX_TIME)
            .fixed_decimals(1)
            .custom_parser(aurora_i18n::parse_number)
            .suffix(" s");
        if ui.add(field).on_hover_text(aurora_i18n::tr!("director-span-time-tip")).changed() {
            d.track.set_span(index, seconds);
            changed = true;
        }
    });
    row(ui, p, aurora_i18n::tr!("director-span-speed"), |ui| {
        // a camera that only turns or zooms there has no speed to set
        let moves = d.track.span_length(index - 1) >= track::MIN_TRAVEL;
        let mut speed = d.track.span_speed(index).unwrap_or(0.0);
        let field = egui::DragValue::new(&mut speed)
            .speed(0.02)
            .range(SPAN_SPEED)
            .fixed_decimals(2)
            .custom_parser(aurora_i18n::parse_number)
            .suffix(" m/s");
        if ui
            .add_enabled(moves, field)
            .on_hover_text(aurora_i18n::tr!("director-span-speed-tip"))
            .on_disabled_hover_text(aurora_i18n::tr!("director-span-speed-still"))
            .changed()
        {
            d.track.set_span_speed(index, speed);
            changed = true;
        }
    });
    if changed && let Some(key) = d.track.keys().get(index) {
        // the playhead stays on the keyframe being edited
        d.time = key.time;
    }
}

fn path_section(ui: &mut egui::Ui, p: &Palette, d: &mut Director) {
    row(ui, p, aurora_i18n::tr!("director-duration"), |ui| {
        let mut seconds = d.track.duration();
        let field = egui::DragValue::new(&mut seconds)
            .speed(0.1)
            .range(0.0..=track::MAX_TIME)
            .fixed_decimals(1)
            .custom_parser(aurora_i18n::parse_number)
            .suffix(" s");
        if ui
            .add_enabled(d.can_play(), field)
            .on_hover_text(aurora_i18n::tr!("director-duration-tip"))
            .changed()
        {
            // the playhead keeps its place in the track
            let share = if d.track.duration() > 0.0 {
                d.time / d.track.duration()
            } else {
                0.0
            };
            d.track.set_duration(seconds);
            d.time = share * d.track.duration();
        }
    });
    ui.horizontal(|ui| {
        let mut curve = usize::from(d.track.curve == Curve::Linear);
        let options = [
            (aurora_i18n::tr!("director-curve-smooth"), true),
            (aurora_i18n::tr!("director-curve-linear"), true),
        ];
        if widgets::segmented(ui, p, &mut curve, &options) {
            d.track.curve = if curve == 1 { Curve::Linear } else { Curve::Smooth };
        }
        if widgets::action_button_if(
            ui,
            p,
            "gauge-filled",
            aurora_i18n::tr!("director-even-speed"),
            ActionKind::Flat,
            Vec2::new(0.0, 26.0),
            d.track.len() >= 3,
        )
        .on_hover_text(aurora_i18n::tr!("director-even-speed-tip"))
        .on_disabled_hover_text(aurora_i18n::tr!("director-even-speed-tip"))
        .clicked()
        {
            d.track.even_speed();
            // the playhead stays on the keyframe being edited
            if let Some(key) = d.selected.and_then(|i| d.track.keys().get(i)) {
                d.time = key.time;
            }
        }
    });
    widgets::checkbox(ui, p, &mut d.track.ease_ends, aurora_i18n::tr!("director-ease"))
        .on_hover_text(aurora_i18n::tr!("director-ease-tip"));
    widgets::checkbox(ui, p, &mut d.show_path, aurora_i18n::tr!("director-show-path"));
    let summary = aurora_i18n::tr!(
        "director-summary",
        count = d.track.len(),
        seconds = aurora_i18n::number(f64::from(d.track.duration()), 1),
        meters = aurora_i18n::number(f64::from(d.track.length()), 1)
    );
    ui.label(RichText::new(summary).size(12.0).color(p.muted));
}

fn playback_section(ui: &mut egui::Ui, p: &Palette, d: &mut Director) {
    row(ui, p, aurora_i18n::tr!("director-start-delay"), |ui| {
        ui.spacing_mut().slider_width = SLIDER_W;
        let slider = egui::Slider::new(&mut d.start_delay, director::START_DELAY_RANGE)
            .custom_parser(aurora_i18n::parse_number)
            .fixed_decimals(1)
            .suffix(" s")
            .trailing_fill(true);
        ui.add(slider).on_hover_text(aurora_i18n::tr!("director-start-delay-tip"));
    });
    row(ui, p, aurora_i18n::tr!("director-shake"), |ui| {
        ui.spacing_mut().slider_width = SLIDER_W;
        let mut percent = (d.shake * 100.0).round();
        let slider = egui::Slider::new(&mut percent, 0.0..=100.0)
            .custom_parser(aurora_i18n::parse_number)
            .fixed_decimals(0)
            .suffix(" %")
            .trailing_fill(true);
        if ui.add(slider).on_hover_text(aurora_i18n::tr!("director-shake-tip")).changed() {
            d.shake = percent / 100.0;
        }
    });
    widgets::checkbox(ui, p, &mut d.hide_huds, aurora_i18n::tr!("director-hide-huds"));
    widgets::checkbox(ui, p, &mut d.hide_pointer, aurora_i18n::tr!("director-hide-pointer"));
    widgets::checkbox(ui, p, &mut d.looping, aurora_i18n::tr!("director-loop"));
    if widgets::action_button_if(
        ui,
        p,
        "trash-filled",
        aurora_i18n::tr!("director-clear"),
        ActionKind::FlatDanger,
        Vec2::new(0.0, 26.0),
        !d.track.is_empty(),
    )
    .clicked()
    {
        d.clear();
    }
}

/// The seconds between two labelled ticks, at `scale` points a second.
fn tick_step(scale: f32) -> f32 {
    [0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 60.0, 120.0, 300.0, 600.0]
        .into_iter()
        .find(|step| step * scale >= TICK_ROOM)
        .unwrap_or(1200.0)
}

/// Seconds the ruler shows for a track of `duration`: room after the end to
/// put the playhead where the next keyframe goes.
fn ruler_span(duration: f32) -> f32 {
    (duration + 2.0 * track::DEFAULT_GAP).max(MIN_SPAN)
}

fn paint_key(painter: &egui::Painter, center: Pos2, half: f32, color: Color32) {
    painter.add(egui::Shape::convex_polygon(
        vec![
            center - egui::vec2(0.0, half),
            center + egui::vec2(half, 0.0),
            center + egui::vec2(0.0, half),
            center - egui::vec2(half, 0.0),
        ],
        color,
        Stroke::NONE,
    ));
}

/// The strip above the bottom bar (`bottom_top` is the bar's top). Returns
/// its own top: what else sits above the bar goes above it.
pub fn timeline(ctx: &egui::Context, p: &Palette, d: &mut Director, bottom_top: f32, live: &Live, actions: &mut Vec<Action>) -> f32 {
    let room = super::layout::work_area(ctx);
    let width = (room.width() - 2.0 * STRIP_GAP).clamp(320.0, STRIP_MAX_W);
    let area = egui::Area::new(Id::unique("director_timeline"))
        .order(egui::Order::Background)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(egui::pos2(room.center().x.round(), (bottom_top - STRIP_GAP).round()))
        .constrain(false)
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::symmetric(10, 8), |ui| {
                ui.set_width(width - 20.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    strip_buttons(ui, p, d, live, actions);
                    let ruler_w = (ui.available_width() - CLOCK_W - 6.0).max(80.0);
                    ruler(ui, p, d, ruler_w, live.restricted, actions);
                    let (rect, _) = ui.allocate_exact_size(Vec2::new(CLOCK_W, RULER_H), Sense::hover());
                    let text = format!("{} / {}", clock(d.time, true), clock(d.track.duration(), true));
                    ui.painter()
                        .text(rect.right_center(), Align2::RIGHT_CENTER, text, FontId::monospace(12.0), p.ink);
                });
            });
        });
    area.response.rect.top()
}

fn strip_buttons(ui: &mut egui::Ui, p: &Palette, d: &mut Director, live: &Live, actions: &mut Vec<Action>) {
    if widgets::action_button(ui, p, "player-skip-back-filled", "", ActionKind::Flat, STRIP_BUTTON)
        .on_hover_text(aurora_i18n::tr!("director-rewind"))
        .clicked()
    {
        d.stop();
        d.time = 0.0;
    }
    let tip = if live.restricted {
        aurora_i18n::tr!("director-restricted")
    } else if d.can_play() {
        aurora_i18n::tr!("director-play")
    } else {
        aurora_i18n::tr!("director-play-needs-keys")
    };
    if widgets::action_button_if(
        ui,
        p,
        "player-play-filled",
        "",
        ActionKind::Flat,
        STRIP_BUTTON,
        d.can_play() && !live.restricted,
    )
    .on_hover_text(tip)
    .on_disabled_hover_text(tip)
    .clicked()
    {
        actions.push(Action::Play);
    }
    let (icon, kind) = if live.recording {
        ("player-stop-filled", ActionKind::Danger)
    } else {
        ("player-record-filled", ActionKind::Flat)
    };
    let tip = if !live.can_record {
        aurora_i18n::tr!("director-record-unavailable")
    } else if live.recording {
        aurora_i18n::tr!("director-record-stop")
    } else {
        aurora_i18n::tr!("director-record-tip")
    };
    if widgets::action_button_if(ui, p, icon, "", kind, STRIP_BUTTON, live.can_record)
        .on_hover_text(tip)
        .on_disabled_hover_text(tip)
        .clicked()
    {
        actions.push(Action::Record);
    }
    let kind = if d.looping { ActionKind::Primary } else { ActionKind::Flat };
    if widgets::action_button(ui, p, "repeat", "", kind, STRIP_BUTTON)
        .on_hover_text(aurora_i18n::tr!("director-loop"))
        .clicked()
    {
        d.looping = !d.looping;
    }
    let tip = if d.track.is_full() {
        aurora_i18n::tr!("director-track-full")
    } else {
        aurora_i18n::tr!("director-add-keyframe-tip")
    };
    if widgets::action_button_if(
        ui,
        p,
        "plus",
        aurora_i18n::tr!("director-add-keyframe"),
        ActionKind::Primary,
        Vec2::new(0.0, STRIP_BUTTON.y),
        !d.track.is_full(),
    )
    .on_hover_text(tip)
    .on_disabled_hover_text(tip)
    .clicked()
    {
        actions.push(Action::AddKey);
    }
    let tip = aurora_i18n::tr!("director-delete-tip");
    if widgets::action_button_if(
        ui,
        p,
        "trash-filled",
        "",
        ActionKind::FlatDanger,
        STRIP_BUTTON,
        d.selected.is_some_and(|index| index < d.track.len()),
    )
    .on_hover_text(tip)
    .on_disabled_hover_text(tip)
    .clicked()
        && let Some(index) = d.selected
    {
        d.remove(index);
    }
    for (icon, tip, enabled, action) in [
        ("arrow-back-up", aurora_i18n::tr!("director-undo"), live.can_undo, Action::Undo),
        ("arrow-forward-up", aurora_i18n::tr!("director-redo"), live.can_redo, Action::Redo),
    ] {
        if widgets::action_button_if(ui, p, icon, "", ActionKind::Flat, STRIP_BUTTON, enabled)
            .on_hover_text(tip)
            .on_disabled_hover_text(tip)
            .clicked()
        {
            actions.push(action);
        }
    }
}

/// Whether the Director has the keyboard (Delete, Ctrl+Z, Ctrl+Y): since the
/// last press of a mouse button, which gives it when it lands on the
/// timeline strip, on the window or on a keyframe's camera in the world,
/// and takes it away anywhere else. `had`: what it was before this frame.
pub fn keys(ctx: &egui::Context, had: bool, keyframes: usize) -> bool {
    let Some(at) = ctx.input(|i| i.pointer.press_origin().filter(|_| i.pointer.any_pressed())) else {
        return had;
    };
    let inside = |id: Id| ctx.memory(|m| m.area_rect(id)).is_some_and(|rect| rect.contains(at));
    inside(Id::unique("director_timeline"))
        || inside(Id::unique(WINDOW))
        || (0..keyframes).any(|index| inside(Id::unique(("director_world_key", index))))
}

/// The ruler: the seconds, the keyframes on their lane and the playhead.
fn ruler(ui: &mut egui::Ui, p: &Palette, d: &mut Director, width: f32, restricted: bool, actions: &mut Vec<Action>) {
    let (rect, background) = ui.allocate_exact_size(Vec2::new(width, RULER_H), Sense::click_and_drag());
    let lane = Rect::from_min_max(egui::pos2(rect.left(), rect.bottom() - LANE_H), rect.max);
    let end = d.track.duration();
    // while a keyframe is dragged the ruler keeps the seconds it showed
    // when the drag began: a track that grows under the pointer would move
    // the very marks the pointer aims at
    let held_span = ui.scope_id().with("director_ruler_span");
    let span = ui.data(|data| data.get_temp::<f32>(held_span)).unwrap_or_else(|| ruler_span(end));
    let mut dragging = false;
    let left = rect.left() + RULER_PAD;
    // points a second
    let scale = (rect.width() - 2.0 * RULER_PAD).max(1.0) / span;
    let x_of = |time: f32| left + time.min(span) * scale;
    let time_at = |x: f32| ((x - left) / scale).clamp(0.0, span);
    let painter = ui.painter_at(rect);
    painter.rect_filled(lane, ui.visuals().widgets.inactive.corner_radius, p.field);
    if end > 0.0 {
        // what plays: from the first keyframe to the last
        let played = Rect::from_min_max(egui::pos2(x_of(0.0), lane.top()), egui::pos2(x_of(end), lane.bottom()));
        painter.rect_filled(played, 0.0, p.violet.gamma_multiply(0.22));
    }
    let step = tick_step(scale);
    for n in 0..=(span / step).floor() as usize {
        let time = n as f32 * step;
        let x = x_of(time);
        painter.line_segment(
            [egui::pos2(x, lane.top() - 4.0), egui::pos2(x, lane.top())],
            Stroke::new(1.0, p.muted_dim),
        );
        painter.text(
            egui::pos2(x, lane.top() - 6.0),
            Align2::CENTER_BOTTOM,
            clock(time, step < 1.0),
            FontId::proportional(10.5),
            p.muted,
        );
    }
    // the playhead held anywhere on the ruler: the view follows the track
    // (the keyframes, laid over it below, take their own clicks)
    match background.interact_pointer_pos() {
        Some(pointer) if background.is_pointer_button_down_on() && !restricted => d.scrub(time_at(pointer.x)),
        Some(pointer) if background.is_pointer_button_down_on() => d.time = time_at(pointer.x),
        _ => d.end_scrub(),
    }
    let keys: Vec<Keyframe> = d.track.keys().to_vec();
    // the camera's mean speed on each span wide enough to write it
    for index in 1..keys.len() {
        let (from, to) = (x_of(keys[index - 1].time), x_of(keys[index].time));
        let (length, seconds) = (d.track.span_length(index - 1), keys[index].time - keys[index - 1].time);
        if to - from >= SPEED_ROOM && length >= track::MIN_TRAVEL && seconds > 0.0 {
            painter.text(
                egui::pos2((from + to) / 2.0, lane.center().y),
                Align2::CENTER_CENTER,
                format!("{} m/s", aurora_i18n::number(f64::from(length / seconds), 1)),
                FontId::proportional(10.0),
                p.muted,
            );
        }
    }
    let ripple = ui.input(|i| i.modifiers.shift);
    for (index, key) in keys.iter().enumerate() {
        let center = egui::pos2(x_of(key.time), lane.center().y);
        let hit = Rect::from_center_size(center, Vec2::new(KEY_HALF * 2.0 + 4.0, LANE_H + 6.0));
        let response = ui.interact(hit, ui.scope_id().with(("director_key", index)), Sense::click_and_drag());
        if response.double_clicked() {
            actions.push(Action::GoTo(index));
        } else if response.clicked() || response.drag_started() {
            d.select(index);
        }
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
        {
            dragging = true;
            d.selected = Some(index);
            let time = time_at(pointer.x);
            if ripple && index > 0 {
                // Shift: the keyframes after it keep their distance to it
                d.track.set_span(index, time - keys[index - 1].time);
            } else {
                d.track.set_time(index, time);
            }
            if let Some(moved) = d.track.keys().get(index) {
                d.time = moved.time;
            }
        }
        let selected = d.selected == Some(index);
        let color = if selected {
            p.violet_light
        } else if response.hovered() {
            p.on_accent
        } else {
            p.ink
        };
        paint_key(&painter, center, if selected { KEY_HALF + 1.5 } else { KEY_HALF }, color);
        if !dragging {
            response.on_hover_text(format!(
                "{}\n{}",
                aurora_i18n::tr!("director-keyframe-at", number = index + 1, time = clock(key.time, true)),
                aurora_i18n::tr!("director-keyframe-hint")
            ));
        }
    }
    ui.data_mut(|data| {
        if dragging {
            data.insert_temp(held_span, span);
        } else {
            data.remove::<f32>(held_span);
        }
    });
    let x = x_of(d.time);
    painter.line_segment(
        [egui::pos2(x, rect.top() + 2.0), egui::pos2(x, rect.bottom())],
        Stroke::new(2.0, p.teal),
    );
    painter.add(egui::Shape::convex_polygon(
        vec![
            egui::pos2(x - 5.0, rect.top()),
            egui::pos2(x + 5.0, rect.top()),
            egui::pos2(x, rect.top() + 7.0),
        ],
        p.teal,
        Stroke::NONE,
    ));
}

/// A world segment over everything.
fn line(painter: &egui::Painter, proj: &Projector, a: Vec3, b: Vec3, stroke: Stroke) {
    if let Some(segment) = super::minimap::beacon_segment(proj.view_proj * a.extend(1.0), proj.view_proj * b.extend(1.0), proj) {
        painter.line_segment(segment, stroke);
    }
}

/// A world segment that the scene in front of it hides.
fn depth_line(ctx: &egui::Context, painter: &egui::Painter, proj: &Projector, a: Vec3, b: Vec3, stroke: Stroke) {
    if let Some(segment) = super::minimap::beacon_segment(proj.view_proj * a.extend(1.0), proj.view_proj * b.extend(1.0), proj) {
        aurora_render::world_line_at(ctx, a, b);
        painter.line_segment(segment, stroke);
    }
}

/// The edges of a keyframe's camera: from the eye to the four corners of
/// its frame, the frame, and a peak over it on the side that is up. The
/// frame has the proportions of the view (`aspect`) and stands the further
/// from the eye the longer the lens is.
fn camera_edges(pose: &Pose, aspect: f32) -> Vec<(Vec3, Vec3)> {
    let half_tan = (pose.fov * 0.5).tan().max(1e-3);
    let depth = (CAMERA_HALF / half_tan).clamp(*CAMERA_DEPTH.start(), *CAMERA_DEPTH.end());
    let up = pose.up() * (depth * half_tan).min(CAMERA_HALF * 2.0);
    let left = pose.left() * (up.length() * aspect.clamp(0.5, 2.5));
    let eye = pose.position;
    let center = eye + pose.forward() * depth;
    let corners = [center + left + up, center - left + up, center - left - up, center + left - up];
    let mut edges = Vec::with_capacity(10);
    for (i, corner) in corners.iter().enumerate() {
        edges.push((eye, *corner));
        edges.push((*corner, corners[(i + 1) % 4]));
    }
    let peak = center + up * 1.4;
    edges.push((corners[0], peak));
    edges.push((corners[1], peak));
    edges
}

/// The picture of the chosen format in `screen`: centred, as large as fits.
fn picture_rect(format: Format, screen: Rect) -> Rect {
    let (width, height) = format.fit(screen.width(), screen.height());
    Rect::from_center_size(screen.center(), Vec2::new(width, height))
}

/// What `screen` shows outside `picture`: the two bars above and below it,
/// or on its sides.
fn bars(screen: Rect, picture: Rect) -> [Rect; 2] {
    if picture.width() < screen.width() - 0.5 {
        [
            Rect::from_min_max(screen.min, egui::pos2(picture.left(), screen.bottom())),
            Rect::from_min_max(egui::pos2(picture.right(), screen.top()), screen.max),
        ]
    } else {
        [
            Rect::from_min_max(screen.min, egui::pos2(screen.right(), picture.top())),
            Rect::from_min_max(egui::pos2(screen.left(), picture.bottom()), screen.max),
        ]
    }
}

/// The guides over the world while the interface shows: the frame of the
/// chosen format (what lies outside it dimmed), the rule of thirds, and a
/// ring on the lock's target (`target`, when one is in sight).
pub fn guides(ctx: &egui::Context, p: &Palette, proj: &Projector, d: &Director, target: Option<Vec3>) {
    let painter = super::hud::world_painter(ctx);
    aurora_render::world_label_at(ctx, None);
    let screen = ctx.content_rect();
    let picture = picture_rect(d.format, screen);
    if d.format != Format::Window {
        for bar in bars(screen, picture) {
            painter.rect_filled(bar, 0.0, p.bar.gamma_multiply(0.55));
        }
        painter.rect_stroke(picture, 0.0, Stroke::new(1.0, p.ink.gamma_multiply(0.5)), egui::StrokeKind::Inside);
    }
    if d.thirds {
        let stroke = Stroke::new(1.0, p.ink.gamma_multiply(0.35));
        for third in [1.0 / 3.0, 2.0 / 3.0] {
            let x = picture.left() + picture.width() * third;
            let y = picture.top() + picture.height() * third;
            painter.line_segment([egui::pos2(x, picture.top()), egui::pos2(x, picture.bottom())], stroke);
            painter.line_segment([egui::pos2(picture.left(), y), egui::pos2(picture.right(), y)], stroke);
        }
    }
    if d.mode == Mode::Editing
        && let Some((at, _)) = target.and_then(|point| proj.project(point))
    {
        painter.circle_stroke(at, 9.0, Stroke::new(3.0, p.bar.gamma_multiply(0.6)));
        painter.circle_stroke(at, 9.0, Stroke::new(1.5, p.amber));
        painter.circle_filled(at, 2.0, p.amber);
    }
}

/// The track in the world, while it is edited: the path, a dot each second,
/// a camera at each keyframe (click: select; double click: the camera goes
/// there), the handles of the selected one and where the playhead is. A
/// locked keyframe's camera points at the lock's `target`, as it will when
/// the track plays.
pub fn world(
    ctx: &egui::Context,
    p: &Palette,
    proj: &Projector,
    d: &mut Director,
    aspect: f32,
    target: Option<Vec3>,
    actions: &mut Vec<Action>,
) {
    if !d.show_path || d.track.is_empty() || d.mode != Mode::Editing {
        return;
    }
    let painter = super::hud::world_painter(ctx);
    aurora_render::world_label_at(ctx, None);
    // faint through everything, bright where nothing hides it
    let path = d.track.path(PATH_STEPS);
    for pair in path.windows(2) {
        line(&painter, proj, pair[0], pair[1], Stroke::new(2.0, p.teal.gamma_multiply(0.25)));
    }
    for pair in path.windows(2) {
        depth_line(ctx, &painter, proj, pair[0], pair[1], Stroke::new(2.0, p.teal));
    }
    let end = d.track.duration();
    for second in 1..end.ceil() as usize {
        if let Some(pose) = d.track.sample(second as f32)
            && let Some((at, _)) = proj.project(pose.position)
        {
            aurora_render::world_label_at(ctx, Some(pose.position));
            painter.circle_filled(at, 3.0, p.teal);
        }
    }
    aurora_render::world_label_at(ctx, None);
    let keys: Vec<Keyframe> = d.track.keys().to_vec();
    if let Some(index) = d.selected.filter(|i| *i < keys.len()) {
        // the Bézier handles the neighbours give this keyframe
        let at = keys[index].pose.position;
        let before = index.checked_sub(1).and_then(|i| d.track.handles(i)).map(|(_, handle)| handle);
        let after = d.track.handles(index).map(|(handle, _)| handle);
        for handle in [before, after].into_iter().flatten() {
            line(&painter, proj, at, handle, Stroke::new(1.0, p.violet_light));
            if let Some((point, _)) = proj.project(handle) {
                painter.circle_filled(point, 3.0, p.violet_light);
            }
        }
    }
    for (index, key) in keys.iter().enumerate() {
        let selected = d.selected == Some(index);
        let color = if selected { p.violet_light } else { p.ink };
        let stroke = Stroke::new(if selected { 2.0 } else { 1.5 }, color);
        let shown = match target {
            Some(point) if key.locked => key.pose.aimed(point, 1.0),
            _ => key.pose,
        };
        for (a, b) in camera_edges(&shown, aspect) {
            line(&painter, proj, a, b, Stroke::new(stroke.width, color.gamma_multiply(0.3)));
            depth_line(ctx, &painter, proj, a, b, stroke);
        }
        aurora_render::world_label_at(ctx, None);
        let Some((at, _)) = proj.project(key.pose.position) else {
            continue;
        };
        painter.circle_filled(at, if selected { 5.0 } else { 4.0 }, color);
        let label = (index + 1).to_string();
        let font = FontId::proportional(12.0);
        let above = at - egui::vec2(0.0, 9.0);
        painter.text(above + egui::vec2(1.0, 1.0), Align2::CENTER_BOTTOM, &label, font.clone(), p.bar);
        painter.text(above, Align2::CENTER_BOTTOM, &label, font, color);
        let response = egui::Area::new(Id::unique(("director_world_key", index)))
            .order(egui::Order::Background)
            .pivot(Align2::CENTER_CENTER)
            .fixed_pos(at)
            .constrain(false)
            .show(ctx, |ui| ui.allocate_response(Vec2::splat(MARKER), Sense::click()))
            .inner;
        if response.double_clicked() {
            actions.push(Action::GoTo(index));
        } else if response.clicked() {
            d.select(index);
        }
        response.on_hover_text(aurora_i18n::tr!(
            "director-keyframe-at",
            number = index + 1,
            time = clock(key.time, true)
        ));
    }
    // where the playhead is on the path
    if d.time <= end
        && end > 0.0
        && let Some(pose) = d.track.sample(d.time)
        && let Some((at, _)) = proj.project(pose.position)
    {
        painter.circle_filled(at, 5.0, p.bar);
        painter.circle_filled(at, 3.5, p.teal);
    }
}

/// While the track plays: the bars of the chosen format, and the countdown
/// of the start delay with the key that stops it. Once the camera leaves,
/// nothing else.
pub fn playing(ctx: &egui::Context, p: &Palette, d: &Director, recording: Option<f64>) {
    let screen = ctx.content_rect();
    if let Some(seconds) = recording {
        // (on the screen only: the film is the view without the interface)
        egui::Area::new(Id::unique("director_recording"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen.min + egui::vec2(16.0, 16.0))
            .constrain(false)
            .interactable(false)
            .show(ctx, |ui| {
                widgets::overlay(ui, p, egui::Margin::symmetric(10, 6), |ui| {
                    let text = aurora_i18n::tr!("director-record-recording", time = clock(seconds as f32, true));
                    ui.label(RichText::new(text).size(12.0).strong().color(p.danger));
                });
            });
    }
    if d.format != Format::Window {
        // under the countdown, which is in the foreground
        let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Middle, Id::unique("director_bars")));
        for bar in bars(screen, picture_rect(d.format, screen)) {
            // theme-guard: content — the bars are part of the picture being filmed, black in every skin
            painter.rect_filled(bar, 0.0, Color32::BLACK);
        }
    }
    let Some(left) = d.countdown() else {
        return;
    };
    egui::Area::new(Id::unique("director_countdown"))
        .order(egui::Order::Foreground)
        .pivot(Align2::CENTER_BOTTOM)
        .fixed_pos(egui::pos2(screen.center().x.round(), screen.bottom() - 48.0))
        .constrain(false)
        .interactable(false)
        .show(ctx, |ui| {
            widgets::overlay(ui, p, egui::Margin::symmetric(14, 8), |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 12.0;
                    let seconds = left.ceil().max(1.0) as usize;
                    ui.label(
                        RichText::new(aurora_i18n::tr!("director-starts-in", seconds = seconds))
                            .size(13.0)
                            .strong()
                            .color(p.ink),
                    );
                    ui.label(RichText::new(aurora_i18n::tr!("director-stop-hint")).size(12.0).color(p.muted));
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Vec2 = Vec2::new(1600.0, 900.0);
    const BOTTOM_TOP: f32 = 840.0;

    fn live() -> Live {
        Live {
            fov: 1.0,
            roll: 0.0,
            free: false,
            free_speed: 2.0,
            lens_locked: false,
            restricted: false,
            focus: false,
            target: None,
            can_undo: false,
            can_redo: false,
            can_record: false,
            recording: false,
        }
    }

    fn three() -> Director {
        let mut d = Director::default();
        for x in [0.0, 6.0, 12.0] {
            d.add_key(Pose::from_view(Vec3::new(x, 0.0, 22.0), Vec3::Y, Vec3::Z, 1.0));
        }
        d
    }

    /// Frames of the interface with a pointer: `draw` each frame, the
    /// pointer moved, pressed or released before it.
    struct Bench {
        ctx: egui::Context,
        p: Palette,
        time: f64,
    }

    impl Bench {
        fn new() -> Bench {
            let ctx = egui::Context::default();
            let theme = crate::theme::Theme::default();
            theme.apply(&ctx, 1.0);
            Bench {
                ctx,
                p: theme.palette(),
                time: 0.0,
            }
        }

        fn frame(&mut self, events: Vec<egui::Event>, mut draw: impl FnMut(&egui::Context, &Palette)) {
            self.time += 1.0 / 60.0;
            let p = self.p;
            let mut output = self.ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |ui| draw(ui.ctx(), &p),
            );
            output.textures_delta.clear();
        }

        fn button(at: Pos2, pressed: bool) -> egui::Event {
            egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            }
        }
    }

    #[test]
    fn the_strip_sits_above_the_bar_and_asks_for_a_keyframe() {
        let mut bench = Bench::new();
        let mut d = Director::default();
        let mut actions = Vec::new();
        let mut top = BOTTOM_TOP;
        for _ in 0..4 {
            bench.frame(vec![], |ctx, p| top = timeline(ctx, p, &mut d, BOTTOM_TOP, &live(), &mut actions));
        }
        let strip = bench
            .ctx
            .memory(|m| m.area_rect(Id::unique("director_timeline")))
            .expect("the strip");
        assert!(strip.bottom() <= BOTTOM_TOP && (strip.top() - top).abs() <= 1.0);
        assert!((strip.center().x - SCREEN.x / 2.0).abs() <= 1.0, "centred");
        assert!(actions.is_empty());
        // the fifth button, after back to the start, play, record and loop
        let keyframe = egui::pos2(strip.left() + 10.0 + 4.0 * (STRIP_BUTTON.x + 6.0) + 20.0, strip.center().y);
        for events in [
            vec![egui::Event::PointerMoved(keyframe)],
            vec![Bench::button(keyframe, true)],
            vec![Bench::button(keyframe, false)],
        ] {
            bench.frame(events, |ctx, p| {
                timeline(ctx, p, &mut d, BOTTOM_TOP, &live(), &mut actions);
            });
        }
        assert_eq!(actions, [Action::AddKey]);
    }

    #[test]
    fn holding_the_ruler_shows_the_track_and_letting_go_gives_the_view_back() {
        let mut bench = Bench::new();
        let mut d = three();
        d.selected = None;
        let mut actions = Vec::new();
        for _ in 0..4 {
            bench.frame(vec![], |ctx, p| {
                timeline(ctx, p, &mut d, BOTTOM_TOP, &live(), &mut actions);
            });
        }
        let strip = bench
            .ctx
            .memory(|m| m.area_rect(Id::unique("director_timeline")))
            .expect("the strip");
        // on the ruler's lane, past the last keyframe
        let at = egui::pos2(strip.left() + strip.width() * 0.8, strip.bottom() - 8.0 - LANE_H / 2.0);
        bench.frame(vec![egui::Event::PointerMoved(at)], |ctx, p| {
            timeline(ctx, p, &mut d, BOTTOM_TOP, &live(), &mut actions);
        });
        bench.frame(vec![Bench::button(at, true)], |ctx, p| {
            timeline(ctx, p, &mut d, BOTTOM_TOP, &live(), &mut actions);
        });
        bench.frame(vec![], |ctx, p| {
            timeline(ctx, p, &mut d, BOTTOM_TOP, &live(), &mut actions);
        });
        assert_eq!(d.mode, Mode::Scrubbing);
        assert!(d.time > d.track.duration(), "the playhead went where the pointer is: {}", d.time);
        assert!(d.view(None).is_some());
        bench.frame(vec![Bench::button(at, false)], |ctx, p| {
            timeline(ctx, p, &mut d, BOTTOM_TOP, &live(), &mut actions);
        });
        bench.frame(vec![], |ctx, p| {
            timeline(ctx, p, &mut d, BOTTOM_TOP, &live(), &mut actions);
        });
        assert_eq!(d.mode, Mode::Editing);
        assert!(actions.is_empty());
        // with the camera restricted the playhead moves but the view stays
        let restricted = Live {
            restricted: true,
            ..live()
        };
        for events in [vec![Bench::button(at, true)], vec![]] {
            bench.frame(events, |ctx, p| {
                timeline(ctx, p, &mut d, BOTTOM_TOP, &restricted, &mut actions);
            });
        }
        assert_eq!(d.mode, Mode::Editing);
    }

    #[test]
    fn a_click_on_a_camera_in_the_world_selects_its_keyframe() {
        let mut bench = Bench::new();
        let mut d = three();
        d.selected = None;
        // a view from the side, the whole track in front of it
        let eye = Vec3::new(6.0, -20.0, 24.0);
        let view = glam::camera::rh::view::look_to_mat4(eye, Vec3::new(0.0, 1.0, -0.1).normalize(), Vec3::Z);
        let proj = glam::camera::rh::proj::directx::perspective_infinite_reverse(1.0, SCREEN.x / SCREEN.y, 0.1);
        let projector = Projector {
            view_proj: proj * view,
            width: SCREEN.x,
            height: SCREEN.y,
            ppp: 1.0,
        };
        let mut actions = Vec::new();
        for _ in 0..3 {
            bench.frame(vec![], |ctx, p| world(ctx, p, &projector, &mut d, 16.0 / 9.0, None, &mut actions));
        }
        let (at, _) = projector.project(d.track.keys()[1].pose.position).expect("in view");
        for events in [
            vec![egui::Event::PointerMoved(at)],
            vec![Bench::button(at, true)],
            vec![Bench::button(at, false)],
        ] {
            bench.frame(events, |ctx, p| world(ctx, p, &projector, &mut d, 16.0 / 9.0, None, &mut actions));
        }
        assert_eq!(d.selected, Some(1));
        assert_eq!(d.time, 3.0, "the playhead follows the selection");
        assert!(actions.is_empty());
        // nothing is drawn, nor clickable, while the track has the view
        d.selected = None;
        d.scrub(1.0);
        for events in [vec![Bench::button(at, true)], vec![Bench::button(at, false)]] {
            bench.frame(events, |ctx, p| world(ctx, p, &projector, &mut d, 16.0 / 9.0, None, &mut actions));
        }
        assert_eq!(d.selected, None);
    }

    #[test]
    fn a_click_on_the_strip_gives_the_director_the_keyboard() {
        let mut bench = Bench::new();
        let mut d = three();
        let mut actions = Vec::new();
        let mut has = false;
        let mut frame = |bench: &mut Bench, events: Vec<egui::Event>, has: &mut bool| {
            bench.frame(events, |ctx, p| {
                timeline(ctx, p, &mut d, BOTTOM_TOP, &live(), &mut actions);
                *has = keys(ctx, *has, 3);
            });
        };
        for _ in 0..4 {
            frame(&mut bench, vec![], &mut has);
        }
        assert!(!has, "not before a click");
        let strip = bench
            .ctx
            .memory(|m| m.area_rect(Id::unique("director_timeline")))
            .expect("the strip");
        // on the clock, at the strip's right end: nothing to press there
        let on = egui::pos2(strip.right() - 40.0, strip.center().y);
        frame(&mut bench, vec![egui::Event::PointerMoved(on)], &mut has);
        frame(&mut bench, vec![Bench::button(on, true)], &mut has);
        assert!(has, "a press on the strip gives it");
        frame(&mut bench, vec![Bench::button(on, false)], &mut has);
        frame(&mut bench, vec![], &mut has);
        assert!(has, "and it keeps it until another press");
        // a press anywhere else (the world, another window) takes it away
        let away = egui::pos2(120.0, 120.0);
        frame(&mut bench, vec![egui::Event::PointerMoved(away)], &mut has);
        frame(&mut bench, vec![Bench::button(away, true)], &mut has);
        assert!(!has);
    }

    #[test]
    fn choosing_in_the_scene_reports_the_pointer_and_the_click() {
        let mut bench = Bench::new();
        let mut iso = Isolation::default();
        iso.set_open(true);
        iso.set_mode(Shown::Chosen);
        let mut out = Vec::new();
        let at = egui::pos2(500.0, 400.0);
        // not choosing: nothing over the world
        bench.frame(vec![egui::Event::PointerMoved(at)], |ctx, p| {
            isolate_picking(ctx, p, &iso, None, &mut out)
        });
        bench.frame(vec![], |ctx, p| isolate_picking(ctx, p, &iso, None, &mut out));
        assert!(out.is_empty());
        iso.set_picking(true);
        let hover = ("Someone".to_owned(), false);
        for events in [vec![], vec![], vec![Bench::button(at, true)], vec![Bench::button(at, false)]] {
            bench.frame(events, |ctx, p| isolate_picking(ctx, p, &iso, Some(&hover), &mut out));
        }
        assert!(out.contains(&Isolate::Hover(at)), "the pointer asks what is under it");
        assert_eq!(out.iter().filter(|o| **o == Isolate::Click(at)).count(), 1, "one click, one choice");
    }

    #[test]
    fn the_bars_cover_what_the_format_leaves_out() {
        let screen = Rect::from_min_size(Pos2::ZERO, SCREEN);
        // scope in a 16:9 window: above and below
        let picture = picture_rect(Format::Scope, screen);
        assert_eq!(picture.center(), screen.center());
        assert!((picture.width() - 1600.0).abs() < 0.01 && (picture.height() - 1600.0 / 2.39).abs() < 0.01);
        let [top, bottom] = bars(screen, picture);
        assert_eq!((top.min, top.right()), (screen.min, screen.right()));
        assert!((top.bottom() - picture.top()).abs() < 0.01 && (bottom.top() - picture.bottom()).abs() < 0.01);
        assert_eq!(bottom.max, screen.max);
        // vertical: on the sides
        let picture = picture_rect(Format::Vertical, screen);
        let [left, right] = bars(screen, picture);
        assert!((left.right() - picture.left()).abs() < 0.01 && (right.left() - picture.right()).abs() < 0.01);
        assert_eq!((left.min, right.max), (screen.min, screen.max));
        assert!((left.width() - (1600.0 - 900.0 * 9.0 / 16.0) / 2.0).abs() < 0.01);
        // the window's own: nothing left out
        assert_eq!(picture_rect(Format::Window, screen), screen);
    }

    #[test]
    fn the_guides_and_the_playback_draw_without_a_track() {
        let mut bench = Bench::new();
        let d = Director {
            format: Format::Scope,
            thirds: true,
            ..Default::default()
        };
        let projector = Projector {
            view_proj: glam::Mat4::IDENTITY,
            width: SCREEN.x,
            height: SCREEN.y,
            ppp: 1.0,
        };
        bench.frame(vec![], |ctx, p| guides(ctx, p, &projector, &d, Some(Vec3::new(0.2, 0.1, 0.5))));
        let mut playing = d.clone();
        for x in [0.0, 5.0] {
            playing.add_key(Pose::from_view(Vec3::X * x, Vec3::Y, Vec3::Z, 1.0));
        }
        assert!(playing.play());
        bench.frame(vec![], |ctx, p| super::playing(ctx, p, &playing, Some(3.2)));
        assert!(bench.ctx.memory(|m| m.area_rect(Id::unique("director_recording"))).is_some());
        assert!(
            bench.ctx.memory(|m| m.area_rect(Id::unique("director_countdown"))).is_some(),
            "the start delay counts down"
        );
    }

    #[test]
    fn a_level_view_shows_zero_degrees() {
        assert_eq!(shown_degrees(-1e-7).to_bits(), 0.0f32.to_bits(), "no -0°");
        assert_eq!(shown_degrees(0.004), 0.0);
        assert!((shown_degrees(-0.5) + 28.65).abs() < 0.01);
        assert!((shown_degrees(1.0) - 57.3).abs() < 0.01);
    }

    #[test]
    fn the_clock_counts_minutes_seconds_and_tenths() {
        assert_eq!(clock(0.0, false), "0:00");
        assert_eq!(clock(75.0, false), "1:15");
        assert_eq!(clock(-3.0, false), "0:00");
        // (the decimal separator is the language's)
        let tenths = clock(4.26, true);
        assert!(tenths.starts_with("0:04") && tenths.ends_with('3'), "{tenths}");
    }

    #[test]
    fn the_ruler_keeps_its_labels_apart() {
        // 12 s across 600 points: a label each 2 s
        assert_eq!(tick_step(50.0), 2.0);
        assert_eq!(tick_step(200.0), 0.5);
        assert_eq!(tick_step(1.0), 60.0);
        assert_eq!(tick_step(0.001), 1200.0);
        // room after the last keyframe for the next one
        assert_eq!(ruler_span(0.0), MIN_SPAN);
        assert_eq!(ruler_span(30.0), 30.0 + 2.0 * track::DEFAULT_GAP);
    }

    #[test]
    fn a_long_lens_draws_a_long_camera() {
        let wide = Pose::from_view(Vec3::ZERO, Vec3::X, Vec3::Z, 1.6);
        let long = Pose::from_view(Vec3::ZERO, Vec3::X, Vec3::Z, 0.3);
        let reach = |pose: &Pose| camera_edges(pose, 16.0 / 9.0).iter().map(|(_, b)| b.x).fold(0.0, f32::max);
        assert!(reach(&long) > reach(&wide) * 2.0);
        // every edge stays near the keyframe
        for (a, b) in camera_edges(&long, 16.0 / 9.0) {
            assert!(a.length() < 2.0 && b.length() < 2.0);
        }
        // the frame faces where the camera looks, its peak on the upper side
        let edges = camera_edges(&wide, 1.0);
        assert_eq!(edges.len(), 10);
        assert!(edges[9].1.z > 0.0 && edges[9].1.x > 0.0);
    }
}
