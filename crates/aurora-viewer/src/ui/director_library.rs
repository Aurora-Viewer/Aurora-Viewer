//! The Director's library (`crate::director_library`): the saved tracks
//! (load, rename, delete, import, export), the montage that plays several
//! one after the other, and the help shown in Recording while there is no
//! ffmpeg. Aurora's own: the Legacy Viewer has no camera path tool.
//!
//! Each part is a section of its own, called with one line by
//! `ui::director::window`:
//! - `library_section` (Library): Import / Export first, the name to save
//!   the track under, then one row per saved track (its first view, its
//!   name, its length, and Load, Rename, Delete on the row);
//! - `montage_section` (Library): the shots in order, moved up and down,
//!   their total length, "Play montage" and "Film montage"; saving it under
//!   a name, the saved montages and their export fold under "More options";
//! - `ffmpeg_section` (Output): in place of the recording's controls while
//!   no ffmpeg is found: what it is, why, three steps, and the buttons;
//! - `library_prompts`: the questions (a delete, the pack's download),
//!   each asked in a `widgets::Modal` that Escape cancels.

use super::director::{Action, Tracks};
use super::director_ruler;
use super::widgets::{self, ActionKind};
use crate::camera::track;
use crate::director_ffmpeg::{LookAgain, PackState};
use crate::director_library::{self as library, ShotInfo};
use crate::director_thumbs::PoseKey;
use crate::theme::Palette;
use egui::{Id, Pos2, Rect, RichText, Sense, Vec2};
use std::collections::HashMap;

const BUTTON: Vec2 = Vec2::new(0.0, 26.0);
/// A saved track's first view, on its row.
const ROW_THUMB: Vec2 = Vec2::new(32.0, 18.0);

/// What the library asks of the app (files, threads, the montage).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LibAction {
    /// Choose a track's or a montage's file to import.
    Import,
    /// Export the track being edited, under the name typed.
    ExportTrack,
    Rename {
        from: String,
        to: String,
    },
    /// Delete a saved track (asked first).
    Delete(String),
    /// Save the montage under the name typed.
    SaveMontage,
    LoadMontage(String),
    /// Delete a saved montage (asked first): its tracks stay.
    DeleteMontage(String),
    /// Export the montage with the tracks it plays.
    ExportMontage,
    PlayMontage,
    FilmMontage,
    /// The ffmpeg pack: the question, then the download.
    AskPack,
    DeclinePack,
    DownloadPack,
}

/// What is to be deleted, while the question is asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Deleting {
    Track(String),
    Montage(String),
}

/// The library's state, kept by the app between frames.
#[derive(Debug, Default)]
pub struct LibraryUi {
    /// What was asked this frame.
    pub out: Vec<LibAction>,
    /// Each saved track's keyframes and length, once its file was read.
    pub infos: HashMap<String, ShotInfo>,
    /// Each saved track's first view (for its thumbnail).
    pub firsts: HashMap<String, PoseKey>,
    /// The names of the montages saved for this account.
    pub montages: Vec<String>,
    /// The montage being put together: its shots' track names, in order.
    pub shots: Vec<String>,
    pub montage_name: String,
    /// The track chosen to add as a shot.
    pub adding: Option<String>,
    /// The track being renamed, and the name typed.
    pub renaming: Option<(String, String)>,
    /// The rename's field takes the keyboard (the frame it opens).
    pub rename_focus: bool,
    pub deleting: Option<Deleting>,
    /// What the last import, export, rename or delete did, or why it failed.
    pub note: Option<Result<String, String>>,
    /// A file dialog or a file thread of the library is at work.
    pub busy: bool,
    /// The montage plays or is filmed.
    pub montage_on: bool,
    pub look: LookAgain,
    pub pack: PackState,
}

impl LibraryUi {
    /// Seconds the montage lasts: the shots that play, end to end.
    pub fn total(&self) -> f32 {
        library::total_seconds(self.shots.iter().map(|name| self.infos.get(name).copied()))
    }

    /// The montage has a shot that plays.
    pub fn playable(&self) -> bool {
        self.shots
            .iter()
            .any(|name| self.infos.get(name).and_then(|info| info.plays()).is_some())
    }
}

fn note(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    ui.label(RichText::new(text).size(12.0).color(color));
}

fn seconds(value: f32) -> String {
    aurora_i18n::number_text(&format!("{value:.1}"))
}

/// The saved tracks (Library): Import and Export first, the name to save
/// the track being edited under, then each saved track on its row.
pub fn library_section(ui: &mut egui::Ui, p: &Palette, tracks: &mut Tracks<'_>, empty: bool, actions: &mut Vec<Action>) {
    let Tracks {
        name,
        saved,
        note: filed,
        library,
    } = tracks;
    let (name, saved, filed): (&mut String, &[String], _) = (name, saved, *filed);
    let idle = !library.busy && !library.montage_on;
    ui.horizontal_wrapped(|ui| {
        let tip = aurora_i18n::tr!("director-library-import-tip");
        if widgets::action_button_if(
            ui,
            p,
            "file-import",
            aurora_i18n::tr!("director-library-import"),
            ActionKind::Flat,
            BUTTON,
            idle,
        )
        .on_hover_text(tip)
        .on_disabled_hover_text(aurora_i18n::tr!("director-library-busy"))
        .clicked()
        {
            library.out.push(LibAction::Import);
        }
        let can_export = idle && !empty && track::file_stem(name).is_some();
        let why = if empty {
            aurora_i18n::tr!("director-library-export-empty")
        } else if track::file_stem(name).is_none() {
            aurora_i18n::tr!("director-track-bad-name")
        } else {
            aurora_i18n::tr!("director-library-busy")
        };
        if widgets::action_button_if(
            ui,
            p,
            "file-export",
            aurora_i18n::tr!("director-library-export"),
            ActionKind::Flat,
            BUTTON,
            can_export,
        )
        .on_hover_text(aurora_i18n::tr!("director-library-export-tip"))
        .on_disabled_hover_text(why)
        .clicked()
        {
            library.out.push(LibAction::ExportTrack);
        }
    });
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(name)
                .hint_text(aurora_i18n::tr!("director-track-name"))
                .desired_width(170.0),
        )
        .on_hover_text(aurora_i18n::tr!("director-track-name"));
        let valid = track::file_stem(name).is_some() && !empty;
        let why = if empty {
            aurora_i18n::tr!("director-library-export-empty")
        } else {
            aurora_i18n::tr!("director-track-bad-name")
        };
        if widgets::action_button_if(
            ui,
            p,
            "device-floppy-filled",
            aurora_i18n::tr!("director-track-save"),
            ActionKind::Flat,
            BUTTON,
            valid,
        )
        .on_hover_text(aurora_i18n::tr!("director-library-save-tip"))
        .on_disabled_hover_text(why)
        .clicked()
        {
            actions.push(Action::SaveTrack);
        }
    });
    if saved.is_empty() {
        note(ui, aurora_i18n::tr!("director-track-none"), p.muted);
    }
    let pictures = director_ruler::published(ui.ctx());
    for (index, saved_name) in saved.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            row_thumbnail(ui, p, library.firsts.get(saved_name).and_then(|key| pictures.by_view.get(key)));
            if let Some((from, typed)) = library.renaming.as_mut().filter(|(from, _)| from == saved_name) {
                let edit = ui.add(egui::TextEdit::singleline(typed).desired_width(130.0));
                if std::mem::take(&mut library.rename_focus) {
                    edit.request_focus();
                }
                let valid = track::file_stem(typed).is_some();
                let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                let confirm = widgets::icon_button(ui, p, "check", aurora_i18n::tr!("director-library-rename-confirm"), valid && idle);
                let cancel = widgets::icon_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), true);
                if (confirm || (enter && valid && idle)) && typed.trim() != from.as_str() {
                    library.out.push(LibAction::Rename {
                        from: from.clone(),
                        to: typed.trim().to_owned(),
                    });
                    library.renaming = None;
                } else if confirm || enter || cancel || escape {
                    library.renaming = None;
                }
                return;
            }
            let shown = match library.infos.get(saved_name) {
                Some(ShotInfo::Ready { seconds: length, .. }) => {
                    format!(
                        "{saved_name}  ·  {}",
                        aurora_i18n::tr!("director-library-length", time = seconds(*length))
                    )
                }
                _ => saved_name.clone(),
            };
            // the buttons keep their place at the row's end
            let buttons = 3.0 * 20.0 + 2.0 * 4.0;
            let room = (ui.available_width() - buttons - 6.0).max(40.0);
            ui.add_sized(
                Vec2::new(room, 20.0),
                egui::Label::new(RichText::new(shown).size(12.5).color(p.ink)).truncate(),
            );
            if widgets::icon_button(
                ui,
                p,
                "folder-open-filled",
                aurora_i18n::tr!("director-track-load"),
                !library.montage_on,
            ) {
                actions.push(Action::LoadTrack(index));
            }
            if widgets::icon_button(ui, p, "pencil-filled", aurora_i18n::tr!("director-library-rename-tip"), idle) {
                library.renaming = Some((saved_name.clone(), saved_name.clone()));
                library.rename_focus = true;
            }
            if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("director-library-delete-tip"), idle) {
                library.deleting = Some(Deleting::Track(saved_name.clone()));
            }
        });
    }
    for shown in [filed.map(|n| n.map(str::to_owned).map_err(str::to_owned)), library.note.clone()]
        .into_iter()
        .flatten()
    {
        match shown {
            Ok(text) => note(ui, &text, p.success),
            Err(text) => note(ui, &text, p.warn),
        }
    }
}

/// A saved track's first view on its row, or the placeholder.
fn row_thumbnail(ui: &mut egui::Ui, p: &Palette, picture: Option<&(egui::TextureId, Vec2)>) {
    let (rect, response) = ui.allocate_exact_size(ROW_THUMB, Sense::hover());
    let painter = ui.painter_at(rect);
    let radius = ui.visuals().widgets.inactive.corner_radius;
    match picture {
        Some((texture, _)) => {
            painter.image(
                *texture,
                rect,
                Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                // theme-guard: content — a picture of the view, in its own colours
                egui::Color32::WHITE,
            );
        }
        None => {
            painter.rect_filled(rect, radius, p.field);
            if let Some(icon) = super::icons::global("photo-filled") {
                painter.image(
                    icon.id(),
                    Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    p.muted_dim,
                );
            }
        }
    }
    response.on_hover_text(aurora_i18n::tr!("director-library-thumb-tip"));
}

/// The montage (Library): the shots in order, their total length, and its
/// two actions; its name, the saved montages and the export under "More
/// options".
pub fn montage_section(ui: &mut egui::Ui, p: &Palette, saved: &[String], library: &mut LibraryUi, can_record: bool, restricted: bool) {
    let idle = !library.busy && !library.montage_on;
    if library.shots.is_empty() {
        note(ui, aurora_i18n::tr!("director-montage-empty"), p.muted);
    }
    let mut moved = None;
    let count = library.shots.len();
    for index in 0..count {
        let name = library.shots[index].clone();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            // a shot that does not play is said so, and skipped
            let (text, plays) = match library.infos.get(&name) {
                Some(info @ ShotInfo::Ready { seconds: length, .. }) if info.plays().is_some() => (
                    format!(
                        "{}. {name}  ·  {}",
                        index + 1,
                        aurora_i18n::tr!("director-library-length", time = seconds(*length))
                    ),
                    true,
                ),
                Some(ShotInfo::Ready { .. }) => (
                    format!("{}. {name}  ·  {}", index + 1, aurora_i18n::tr!("director-montage-shot-short")),
                    false,
                ),
                Some(ShotInfo::Unusable) => (
                    format!("{}. {name}  ·  {}", index + 1, aurora_i18n::tr!("director-montage-shot-unusable")),
                    false,
                ),
                None => (
                    format!("{}. {name}  ·  {}", index + 1, aurora_i18n::tr!("director-montage-shot-missing")),
                    false,
                ),
            };
            let color = if plays { p.ink } else { p.warn };
            let buttons = 3.0 * 20.0 + 2.0 * 4.0;
            let room = (ui.available_width() - buttons - 6.0).max(40.0);
            let label = ui.add_sized(
                Vec2::new(room, 20.0),
                egui::Label::new(RichText::new(text).size(12.5).color(color)).truncate(),
            );
            if !plays {
                label.on_hover_text(aurora_i18n::tr!("director-montage-skipped-tip"));
            }
            if widgets::icon_button(
                ui,
                p,
                "caret-up-filled",
                aurora_i18n::tr!("director-montage-up-tip"),
                idle && index > 0,
            ) {
                moved = Some((index, index - 1));
            }
            if widgets::icon_button(
                ui,
                p,
                "caret-down-filled",
                aurora_i18n::tr!("director-montage-down-tip"),
                idle && index + 1 < count,
            ) {
                moved = Some((index, index + 1));
            }
            if widgets::icon_button(ui, p, "x", aurora_i18n::tr!("director-montage-remove-tip"), idle) {
                moved = Some((index, usize::MAX));
            }
        });
    }
    match moved {
        Some((index, usize::MAX)) => {
            library.shots.remove(index);
        }
        Some((from, to)) => library.shots.swap(from, to),
        None => {}
    }
    ui.horizontal(|ui| {
        if library.adding.as_ref().is_none_or(|name| !saved.contains(name)) {
            library.adding = saved.first().cloned();
        }
        let shown = library
            .adding
            .clone()
            .unwrap_or_else(|| aurora_i18n::tr!("director-track-none").to_owned());
        ui.add_enabled_ui(idle && !saved.is_empty(), |ui| {
            egui::ComboBox::from_id_salt("director_montage_add")
                .width(170.0)
                .selected_text(shown)
                .show_ui(ui, |ui| {
                    for name in saved {
                        ui.selectable_value(&mut library.adding, Some(name.clone()), name.as_str());
                    }
                })
                .response
                .on_hover_text(aurora_i18n::tr!("director-montage-add-tip"))
                .on_disabled_hover_text(aurora_i18n::tr!("director-montage-add-none"));
        });
        let can_add = idle && library.adding.is_some() && library.shots.len() < library::MAX_SHOTS;
        if widgets::action_button_if(
            ui,
            p,
            "plus",
            aurora_i18n::tr!("director-montage-add"),
            ActionKind::Flat,
            BUTTON,
            can_add,
        )
        .on_hover_text(aurora_i18n::tr!("director-montage-add-tip"))
        .on_disabled_hover_text(aurora_i18n::tr!("director-montage-add-none"))
        .clicked()
            && let Some(name) = library.adding.clone()
        {
            library.shots.push(name);
        }
    });
    let playing = library
        .shots
        .iter()
        .filter(|name| library.infos.get(*name).and_then(|i| i.plays()).is_some())
        .count();
    note(
        ui,
        &aurora_i18n::tr!("director-montage-total", time = seconds(library.total()), shots = playing),
        p.muted,
    );
    let playable = library.playable();
    let why = if restricted {
        aurora_i18n::tr!("director-montage-restricted")
    } else if !playable {
        aurora_i18n::tr!("director-montage-nothing")
    } else {
        aurora_i18n::tr!("director-library-busy")
    };
    ui.horizontal_wrapped(|ui| {
        let can_play = idle && playable && !restricted;
        if widgets::action_button_if(
            ui,
            p,
            "player-play-filled",
            aurora_i18n::tr!("director-montage-play"),
            ActionKind::Primary,
            BUTTON,
            can_play,
        )
        .on_hover_text(aurora_i18n::tr!("director-montage-play-tip"))
        .on_disabled_hover_text(why)
        .clicked()
        {
            library.out.push(LibAction::PlayMontage);
        }
        let film_why = if can_play && !can_record {
            aurora_i18n::tr!("director-record-unavailable")
        } else {
            why
        };
        if widgets::action_button_if(
            ui,
            p,
            "player-record-filled",
            aurora_i18n::tr!("director-montage-film"),
            ActionKind::Flat,
            BUTTON,
            can_play && can_record,
        )
        .on_hover_text(aurora_i18n::tr!("director-montage-film-tip"))
        .on_disabled_hover_text(film_why)
        .clicked()
        {
            library.out.push(LibAction::FilmMontage);
        }
    });
    widgets::collapsing(
        ui,
        p,
        "director_montage_more",
        aurora_i18n::tr!("director-more-options"),
        false,
        |ui| {
            montage_more(ui, p, library, idle);
        },
    );
}

/// "More options" of the montage: its name and Save, the saved montages,
/// its export, and Clear.
fn montage_more(ui: &mut egui::Ui, p: &Palette, library: &mut LibraryUi, idle: bool) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut library.montage_name)
                .hint_text(aurora_i18n::tr!("director-montage-name"))
                .desired_width(170.0),
        )
        .on_hover_text(aurora_i18n::tr!("director-montage-name"));
        let named = track::file_stem(&library.montage_name).is_some();
        let can = idle && named && !library.shots.is_empty();
        let why = if library.shots.is_empty() {
            aurora_i18n::tr!("director-montage-empty")
        } else if !named {
            aurora_i18n::tr!("director-montage-bad-name")
        } else {
            aurora_i18n::tr!("director-library-busy")
        };
        if widgets::action_button_if(
            ui,
            p,
            "device-floppy-filled",
            aurora_i18n::tr!("director-track-save"),
            ActionKind::Flat,
            BUTTON,
            can,
        )
        .on_hover_text(aurora_i18n::tr!("director-montage-save-tip"))
        .on_disabled_hover_text(why)
        .clicked()
        {
            library.out.push(LibAction::SaveMontage);
        }
    });
    if library.montages.is_empty() {
        note(ui, aurora_i18n::tr!("director-montage-none"), p.muted);
    }
    for name in library.montages.clone() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let buttons = 2.0 * 20.0 + 4.0;
            let room = (ui.available_width() - buttons - 6.0).max(40.0);
            ui.add_sized(
                Vec2::new(room, 20.0),
                egui::Label::new(RichText::new(&name).size(12.5).color(p.ink)).truncate(),
            );
            if widgets::icon_button(ui, p, "folder-open-filled", aurora_i18n::tr!("director-montage-load-tip"), idle) {
                library.out.push(LibAction::LoadMontage(name.clone()));
            }
            if widgets::icon_button(ui, p, "trash-filled", aurora_i18n::tr!("director-montage-delete-tip"), idle) {
                library.deleting = Some(Deleting::Montage(name.clone()));
            }
        });
    }
    ui.horizontal_wrapped(|ui| {
        let named = track::file_stem(&library.montage_name).is_some();
        let can = idle && named && !library.shots.is_empty();
        let why = if library.shots.is_empty() {
            aurora_i18n::tr!("director-montage-empty")
        } else if !named {
            aurora_i18n::tr!("director-montage-bad-name")
        } else {
            aurora_i18n::tr!("director-library-busy")
        };
        if widgets::action_button_if(
            ui,
            p,
            "file-export",
            aurora_i18n::tr!("director-montage-export"),
            ActionKind::Flat,
            BUTTON,
            can,
        )
        .on_hover_text(aurora_i18n::tr!("director-montage-export-tip"))
        .on_disabled_hover_text(why)
        .clicked()
        {
            library.out.push(LibAction::ExportMontage);
        }
        if widgets::action_button_if(
            ui,
            p,
            "x",
            aurora_i18n::tr!("director-montage-clear"),
            ActionKind::Flat,
            BUTTON,
            idle && !library.shots.is_empty(),
        )
        .on_hover_text(aurora_i18n::tr!("director-montage-clear-tip"))
        .on_disabled_hover_text(aurora_i18n::tr!("director-montage-empty"))
        .clicked()
        {
            library.shots.clear();
        }
    });
}

/// Recording without ffmpeg (Output): in place of its controls, what
/// ffmpeg is, why it is needed, and three steps to get it, with the
/// project's download when one is published, "Choose ffmpeg…" and "Look
/// again" (which looks without a restart).
pub fn ffmpeg_section(ui: &mut egui::Ui, p: &Palette, library: &mut LibraryUi, actions: &mut Vec<Action>) {
    ui.label(
        RichText::new(aurora_i18n::tr!("director-ffmpeg-title"))
            .size(14.0)
            .strong()
            .color(p.ink),
    );
    note(ui, aurora_i18n::tr!("director-record-missing"), p.muted);
    let install = if cfg!(windows) {
        aurora_i18n::tr!("director-ffmpeg-step-install-windows")
    } else if cfg!(target_os = "macos") {
        aurora_i18n::tr!("director-ffmpeg-step-install-macos")
    } else {
        aurora_i18n::tr!("director-ffmpeg-step-install-linux")
    };
    let offered = matches!(library.pack, PackState::Offered { .. } | PackState::Failed);
    let first = if offered {
        aurora_i18n::tr!("director-ffmpeg-step-pack")
    } else {
        aurora_i18n::tr!("director-ffmpeg-step-download")
    };
    for (number, step) in [first, install, aurora_i18n::tr!("director-ffmpeg-step-look")]
        .into_iter()
        .enumerate()
    {
        note(ui, &format!("{}. {step}", number + 1), p.ink);
    }
    ui.horizontal_wrapped(|ui| {
        if offered
            && widgets::action_button(
                ui,
                p,
                "download-filled",
                aurora_i18n::tr!("director-ffmpeg-pack"),
                ActionKind::Primary,
                BUTTON,
            )
            .on_hover_text(aurora_i18n::tr!("director-ffmpeg-pack-tip"))
            .clicked()
        {
            library.out.push(LibAction::AskPack);
        }
        let site_kind = if offered || matches!(library.pack, PackState::Downloading { .. }) {
            ActionKind::Flat
        } else {
            ActionKind::Primary
        };
        if widgets::action_button(ui, p, "external-link", aurora_i18n::tr!("director-record-site"), site_kind, BUTTON)
            .on_hover_text(aurora_i18n::tr!("director-ffmpeg-site-tip"))
            .clicked()
        {
            actions.push(Action::FfmpegSite);
        }
        if widgets::action_button(
            ui,
            p,
            "folder-open-filled",
            aurora_i18n::tr!("director-record-choose"),
            ActionKind::Flat,
            BUTTON,
        )
        .on_hover_text(aurora_i18n::tr!("director-ffmpeg-choose-tip"))
        .clicked()
        {
            actions.push(Action::ChooseFfmpeg);
        }
        if widgets::action_button(
            ui,
            p,
            "refresh",
            aurora_i18n::tr!("director-record-again"),
            ActionKind::Flat,
            BUTTON,
        )
        .on_hover_text(aurora_i18n::tr!("director-ffmpeg-again-tip"))
        .clicked()
        {
            actions.push(Action::FindFfmpeg);
        }
    });
    match library.pack {
        PackState::Downloading {
            received,
            total,
            preparing,
        } => {
            let fraction = if total > 0 { received as f32 / total as f32 } else { 0.0 };
            let text = if preparing {
                aurora_i18n::tr!("director-ffmpeg-pack-preparing").to_owned()
            } else {
                aurora_i18n::tr!(
                    "director-ffmpeg-pack-progress",
                    done = megabytes(received),
                    total = megabytes(total)
                )
            };
            let width = ui.available_width().min(280.0);
            widgets::progress(ui, p, "director_ffmpeg_pack", fraction, width, &text, None);
        }
        PackState::Failed => note(ui, aurora_i18n::tr!("director-ffmpeg-pack-failed"), p.warn),
        _ => {}
    }
    if let Some(times) = library.look.in_vain() {
        note(ui, &aurora_i18n::tr!("director-ffmpeg-still-missing", count = times), p.warn);
    }
}

fn megabytes(bytes: u64) -> String {
    aurora_i18n::number_text(&format!("{:.1}", bytes as f64 / (1024.0 * 1024.0)))
}

/// The library's questions, while one is asked: "Delete the track?" (or
/// the montage), and the pack's download with its size. Cancel, Escape or
/// a click beside the question leaves everything as it is.
pub fn library_prompts(ctx: &egui::Context, p: &Palette, library: &mut LibraryUi) {
    if let Some(deleting) = library.deleting.clone() {
        let (title, text, name) = match &deleting {
            Deleting::Track(name) => (
                aurora_i18n::tr!("director-library-delete-confirm"),
                aurora_i18n::tr!("director-library-delete-text", name = name.as_str()),
                name,
            ),
            Deleting::Montage(name) => (
                aurora_i18n::tr!("director-montage-delete-confirm"),
                aurora_i18n::tr!("director-montage-delete-text", name = name.as_str()),
                name,
            ),
        };
        let mut answer = None;
        let modal = widgets::Modal::new(Id::unique("director_library_delete")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(title).size(17.0).color(p.ink));
            ui.add_space(8.0);
            ui.label(text);
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if widgets::action_button(
                    ui,
                    p,
                    "trash-filled",
                    aurora_i18n::tr!("director-library-delete"),
                    ActionKind::Danger,
                    BUTTON,
                )
                .clicked()
                {
                    answer = Some(true);
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, BUTTON).clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        if let Some(delete) = answer {
            library.deleting = None;
            if delete {
                library.out.push(match deleting {
                    Deleting::Track(_) => LibAction::Delete(name.clone()),
                    Deleting::Montage(_) => LibAction::DeleteMontage(name.clone()),
                });
            }
        }
    }
    if let PackState::Asking { size } = library.pack {
        let mut answer = None;
        let modal = widgets::Modal::new(Id::unique("director_ffmpeg_pack")).show(ctx, |ui| {
            ui.set_width(400.0);
            ui.label(
                RichText::new(aurora_i18n::tr!("director-ffmpeg-pack-confirm"))
                    .size(17.0)
                    .color(p.ink),
            );
            ui.add_space(8.0);
            ui.label(aurora_i18n::tr!("director-ffmpeg-pack-confirm-text", size = megabytes(size)));
            ui.add_space(10.0);
            ui.horizontal_wrapped(|ui| {
                if widgets::action_button(
                    ui,
                    p,
                    "download-filled",
                    aurora_i18n::tr!("director-ffmpeg-pack-download"),
                    ActionKind::Primary,
                    BUTTON,
                )
                .clicked()
                {
                    answer = Some(true);
                }
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, BUTTON).clicked() {
                    answer = Some(false);
                }
            });
        });
        if modal.should_close() && answer.is_none() {
            answer = Some(false);
        }
        match answer {
            Some(true) => library.out.push(LibAction::DownloadPack),
            Some(false) => library.out.push(LibAction::DeclinePack),
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_total_counts_the_shots_that_play() {
        let mut library = LibraryUi::default();
        library.infos.insert("A".into(), ShotInfo::Ready { keys: 3, seconds: 6.0 });
        library.infos.insert("One".into(), ShotInfo::Ready { keys: 1, seconds: 0.0 });
        library.infos.insert("Bad".into(), ShotInfo::Unusable);
        library.shots = vec!["A".into(), "One".into(), "Gone".into(), "Bad".into(), "A".into()];
        assert_eq!(library.total(), 12.0);
        assert!(library.playable());
        library.shots = vec!["One".into(), "Gone".into()];
        assert!(!library.playable());
        assert_eq!(library.total(), 0.0);
    }
}
