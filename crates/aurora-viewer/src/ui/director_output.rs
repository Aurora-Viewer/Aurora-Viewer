//! The Director window's Output section: the film, live or rendered. One
//! section, two modes: "Live" records the view in real time (the track's
//! playback, or the live view until pressed again; `recorder.rs`), "Render"
//! films the track offline, frame after frame, at a size of its own
//! (`director_render.rs`). Both share the format, the rate and the quality;
//! the main action and its key choices show, the rest folds under "More
//! options". A section of the Output tab, called from `ui::director::window`.

use super::director::{Action, Live, Record, clock, film_format, row};
use super::widgets::{self, ActionKind};
use crate::director_render::Resolution;
use crate::recorder;
use crate::theme::Palette;
use egui::{Color32, RichText, Vec2};

/// What the window shows of a render.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RenderState {
    /// The render under way: frames written, of how many, seconds left.
    pub progress: Option<(u64, u64, Option<f32>)>,
    /// The size its frames have (or would have now).
    pub size: (u32, u32),
    /// The track can be rendered: two keyframes, the camera free of RLVa.
    pub can_render: bool,
    /// The encoder of the chosen format's sound; `None`: the ffmpeg found
    /// has none, the film is silent.
    pub sound: Option<&'static str>,
}

const BUTTON: Vec2 = Vec2::new(0.0, 26.0);

fn note(ui: &mut egui::Ui, text: &str, color: Color32) {
    ui.label(RichText::new(text).size(12.0).color(color));
}

fn resolution_text(resolution: Resolution) -> String {
    match resolution.lines() {
        Some(lines) => format!("{lines}p"),
        None => aurora_i18n::tr!("director-render-window").to_owned(),
    }
}

/// The section: the ffmpeg (or how to get one), Live or Render, and the
/// last film.
pub fn output_section(
    ui: &mut egui::Ui,
    p: &Palette,
    record: Record<'_>,
    live: &Live,
    library: &mut super::director_library::LibraryUi,
    actions: &mut Vec<Action>,
) {
    let Record {
        options,
        changed,
        ffmpeg,
        formats,
        recording,
        finishing,
        last,
        render,
    } = record;
    let before = options.clone();
    let mut mode = usize::from(options.render);
    let busy = recording.is_some() || render.progress.is_some();
    let modes = [
        (aurora_i18n::tr!("director-output-live"), !busy || !options.render),
        (aurora_i18n::tr!("director-output-render"), !busy || options.render),
    ];
    let pick = ui.horizontal(|ui| widgets::segmented(ui, p, &mut mode, &modes)).inner;
    if pick {
        options.render = mode == 1;
    }
    let png = options.render && options.png;
    if !png && matches!(ffmpeg, Some(None)) {
        // no ffmpeg: its help (`director_library`); a live film needs it, so
        // the help takes the controls' place, while a render can still
        // choose PNG images below
        super::director_library::ffmpeg_section(ui, p, library, actions);
        if !options.render {
            *changed |= *options != before;
            return;
        }
    } else if !png {
        ffmpeg_status(ui, p, ffmpeg, actions);
    }
    // the format: a film's, or (rendered) PNG images
    row(ui, p, aurora_i18n::tr!("director-record-format"), |ui| {
        let shown = if png {
            aurora_i18n::tr!("director-render-png")
        } else {
            film_format(options.format)
        };
        egui::ComboBox::from_id_salt("director_output_format")
            .selected_text(shown)
            .show_ui(ui, |ui| {
                for (format, available) in recorder::Format::ALL.into_iter().zip(formats) {
                    // (a format the ffmpeg found cannot write is shown, greyed)
                    ui.add_enabled_ui(available || ffmpeg.is_none(), |ui| {
                        if ui.selectable_label(!png && options.format == format, film_format(format)).clicked() {
                            options.format = format;
                            options.png = false;
                        }
                    });
                }
                if options.render && ui.selectable_label(png, aurora_i18n::tr!("director-render-png")).clicked() {
                    options.png = true;
                }
            })
            .response
            .on_hover_text(aurora_i18n::tr!("director-output-format-tip"));
    });
    if options.render {
        row(ui, p, aurora_i18n::tr!("director-render-size"), |ui| {
            egui::ComboBox::from_id_salt("director_render_size")
                .selected_text(resolution_text(options.resolution))
                .show_ui(ui, |ui| {
                    for resolution in Resolution::ALL {
                        ui.selectable_value(&mut options.resolution, resolution, resolution_text(resolution));
                    }
                })
                .response
                .on_hover_text(aurora_i18n::tr!("director-render-size-tip"));
            let (w, h) = render.size;
            ui.label(RichText::new(format!("{w} × {h}")).size(12.0).color(p.muted));
        });
    } else {
        row(ui, p, aurora_i18n::tr!("director-sound"), |ui| {
            let can = render.sound.is_some() || ffmpeg.is_none();
            ui.add_enabled_ui(can, |ui| {
                widgets::switch(ui, p, &mut options.sound)
                    .on_hover_text(aurora_i18n::tr!("director-sound-tip"))
                    .on_disabled_hover_text(aurora_i18n::tr!("director-sound-no-encoder"));
            });
        });
    }
    widgets::collapsing(
        ui,
        p,
        "director_output_more",
        aurora_i18n::tr!("director-output-more"),
        false,
        |ui| {
            more_options(ui, p, options, png);
        },
    );
    *changed |= *options != before;
    if options.render {
        render_part(ui, p, options, &render, finishing, recording.is_some(), live, actions);
    } else {
        live_part(ui, p, options, &render, recording, finishing, live, actions);
    }
    last_film(ui, p, recording.is_some() || render.progress.is_some() || finishing, last, actions);
}

/// The ffmpeg found, or how to get one.
fn ffmpeg_status(ui: &mut egui::Ui, p: &Palette, ffmpeg: Option<Option<&std::path::Path>>, actions: &mut Vec<Action>) {
    match ffmpeg {
        None => note(ui, aurora_i18n::tr!("director-record-looking"), p.muted),
        // no ffmpeg: `director_library::ffmpeg_section` shows its help
        Some(None) => {}
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
}

/// The choices that fold: the rate and the quality, and the sound's
/// channels (live) or why a render has none.
fn more_options(ui: &mut egui::Ui, p: &Palette, options: &mut recorder::Options, png: bool) {
    row(ui, p, aurora_i18n::tr!("director-record-fps"), |ui| {
        options.fps = options.rate();
        egui::ComboBox::from_id_salt("director_record_fps")
            .selected_text(options.fps.to_string())
            .show_ui(ui, |ui| {
                for rate in recorder::RATES {
                    ui.selectable_value(&mut options.fps, rate, rate.to_string());
                }
            })
            .response
            .on_hover_text(aurora_i18n::tr!("director-output-fps-tip"));
    });
    row(ui, p, aurora_i18n::tr!("director-record-quality"), |ui| {
        let levels = [recorder::Quality::Light, recorder::Quality::Good, recorder::Quality::Best];
        let mut level = levels.iter().position(|quality| *quality == options.quality).unwrap_or(1);
        let names = [
            (aurora_i18n::tr!("director-record-quality-light"), true),
            (aurora_i18n::tr!("director-record-quality-good"), true),
            (aurora_i18n::tr!("director-record-quality-best"), true),
        ];
        // (PNG images are lossless: no quality to choose)
        let picked = ui.add_enabled_ui(!png, |ui| widgets::segmented(ui, p, &mut level, &names));
        picked
            .response
            .on_hover_text(aurora_i18n::tr!("director-output-quality-tip"))
            .on_disabled_hover_text(aurora_i18n::tr!("director-render-png-quality"));
        if picked.inner {
            options.quality = levels[level];
        }
    });
    if options.render {
        let mut none = false;
        ui.add_enabled_ui(false, |ui| {
            widgets::checkbox(ui, p, &mut none, aurora_i18n::tr!("director-sound"))
                .on_disabled_hover_text(aurora_i18n::tr!("director-render-no-sound"));
        });
    } else {
        ui.add_enabled_ui(options.sound, |ui| {
            widgets::checkbox(ui, p, &mut options.ui_sounds, aurora_i18n::tr!("director-sound-ui"))
                .on_hover_text(aurora_i18n::tr!("director-sound-ui-tip"))
                .on_disabled_hover_text(aurora_i18n::tr!("director-sound-off"));
            widgets::checkbox(ui, p, &mut options.voice, aurora_i18n::tr!("director-sound-voice"))
                .on_hover_text(aurora_i18n::tr!("director-sound-voice-tip"))
                .on_disabled_hover_text(aurora_i18n::tr!("director-sound-off"));
        });
    }
    if options.format.alpha() && !png {
        note(ui, aurora_i18n::tr!("director-record-alpha-tip"), p.muted);
    }
}

/// Live: Record (or Stop), and the time filmed.
#[allow(clippy::too_many_arguments)]
fn live_part(
    ui: &mut egui::Ui,
    p: &Palette,
    options: &recorder::Options,
    render: &RenderState,
    recording: Option<f64>,
    finishing: bool,
    live: &Live,
    actions: &mut Vec<Action>,
) {
    if !options.sound || render.sound.is_none() {
        note(ui, aurora_i18n::tr!("director-record-no-sound"), p.muted);
    } else if options.voice {
        note(ui, aurora_i18n::tr!("director-sound-voice-on"), p.warn);
    }
    let (icon, label, kind) = if recording.is_some() {
        ("player-stop-filled", aurora_i18n::tr!("director-record-stop"), ActionKind::Danger)
    } else {
        (
            "player-record-filled",
            aurora_i18n::tr!("director-record-start"),
            ActionKind::Primary,
        )
    };
    let why = if finishing {
        aurora_i18n::tr!("director-record-finishing")
    } else {
        aurora_i18n::tr!("director-record-unavailable")
    };
    if widgets::action_button_if(ui, p, icon, label, kind, BUTTON, live.can_record && !finishing)
        .on_hover_text(aurora_i18n::tr!("director-record-tip"))
        .on_disabled_hover_text(why)
        .clicked()
    {
        actions.push(Action::Record);
    }
    if let Some(seconds) = recording {
        let text = aurora_i18n::tr!("director-record-recording", time = clock(seconds as f32, true));
        note(ui, &text, p.danger);
    }
}

/// Render: the button, then the progress and Cancel while it renders.
#[allow(clippy::too_many_arguments)]
fn render_part(
    ui: &mut egui::Ui,
    p: &Palette,
    options: &recorder::Options,
    render: &RenderState,
    finishing: bool,
    recording: bool,
    live: &Live,
    actions: &mut Vec<Action>,
) {
    let Some((done, count, left)) = render.progress else {
        let ready = render.can_render && (options.png || live.can_record) && !finishing && !recording;
        let why = if !render.can_render {
            aurora_i18n::tr!("director-render-needs-track")
        } else if finishing || recording {
            aurora_i18n::tr!("director-record-finishing")
        } else {
            aurora_i18n::tr!("director-record-unavailable")
        };
        if widgets::action_button_if(
            ui,
            p,
            "movie",
            aurora_i18n::tr!("director-render-start"),
            ActionKind::Primary,
            BUTTON,
            ready,
        )
        .on_hover_text(aurora_i18n::tr!("director-render-tip"))
        .on_disabled_hover_text(why)
        .clicked()
        {
            actions.push(Action::Render);
        }
        return;
    };
    let fraction = if count == 0 { 0.0 } else { done as f32 / count as f32 };
    let text = match left {
        Some(seconds) => aurora_i18n::tr!(
            "director-render-progress-left",
            done = done,
            count = count,
            left = clock(seconds, false)
        ),
        None => aurora_i18n::tr!("director-render-progress", done = done, count = count),
    };
    let width = ui.available_width().min(300.0);
    widgets::progress(ui, p, "director_render_progress", fraction, width, &text, None);
    if widgets::action_button(
        ui,
        p,
        "player-stop-filled",
        aurora_i18n::tr!("director-render-cancel"),
        ActionKind::Danger,
        BUTTON,
    )
    .on_hover_text(aurora_i18n::tr!("director-render-cancel-tip"))
    .clicked()
    {
        actions.push(Action::CancelRender);
    }
}

/// The last film (or images): where it is, or why it failed.
fn last_film(ui: &mut egui::Ui, p: &Palette, busy: bool, last: Option<Result<&std::path::Path, &str>>, actions: &mut Vec<Action>) {
    if busy {
        return;
    }
    match last {
        Some(Ok(file)) => {
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
                BUTTON,
            )
            .clicked()
            {
                actions.push(Action::RevealFilm);
            }
        }
        Some(Err(why)) => {
            let text = aurora_i18n::tr!("director-record-failed", why = why);
            note(ui, &text, p.warn);
        }
        None => {}
    }
}
