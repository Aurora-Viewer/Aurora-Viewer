//! Day cycle editor: Firestorm's LLFloaterEditExtDayCycle
//! (newview/llfloatereditextdaycycle.cpp, floater_edit_ext_day_cycle.xml),
//! in its inventory, parcel and region contexts. Top to bottom like the
//! XML: the name and Import; the track buttons (Water, Ground Level, Sky 2
//! to 4, named by their altitude for a parcel or a region, greyed without
//! the extended environment); the timeline with the time cursor
//! (WLTimeSlider) above the key frames (WLDayCycleFrames) and the 0..100 %
//! labels; Clone / Load / Clear Track, the play buttons, Add / Load / Delete
//! the frame; the selected frame's sky tabs or water panel (the panels of
//! the sky and water editors); Save (or "Apply To Parcel / Region" in those
//! contexts) with its menu and Cancel. Originally LGPL 2.1.

use super::env_editor::{EditorAccess, EnvEditorUi};
use super::widgets::{self, ActionKind, Floater};
use crate::theme::Palette;
use crate::world::World;
use crate::world::eep::{DayCycle, SkyFrame, WaterFrame};
use crate::world::env_day_edit::{self as day, DayDoc, TRACK_GROUND, TRACK_WATER, TRACKS, TrackError};
use crate::world::env_edit::{self, Rights, SettingsDoc};
use crate::world::env_select::{SettingsCatalog, SettingsKind};
use crate::world::inventory::Inventory;
use egui::{RichText, Sense, Vec2};
use std::collections::{HashMap, HashSet};
use std::time::Instant;
use uuid::Uuid;

pub const WINDOW: &str = "env_day_editor";

#[derive(Debug, Clone, PartialEq)]
pub enum DayAction {
    /// The day or the time cursor changed: show the day there.
    Changed,
    /// "Load [frame]": a sky or water item for the time cursor.
    LoadFrame(Uuid),
    /// "Load Track From": a day cycle item whose track is copied.
    LoadTrack(Uuid),
    /// "Import": a Windlight day file.
    Import,
    Save,
    /// "Save As" chosen: the name is asked once the day is checked.
    SaveAsAsk,
    SaveAs(String),
    ApplyLocal,
    /// "Apply To Parcel" / "Apply To Region" (inventory context).
    ApplyParcel,
    ApplyRegion,
    /// The parcel and region contexts' "Apply To …" (doApplyCommit).
    Commit,
    Closed,
}

/// LLFloaterEditExtDayCycle's edit context.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DayContext {
    /// A day cycle item, or a new day.
    #[default]
    Inventory,
    /// About Land's "Custom": the parcel's day, by its local id.
    Parcel(i32),
    /// The region's day.
    Region,
}

/// Settings loaded into the day that cannot be transferred or copied,
/// waiting for SettingsMakeNoTrans (onAssetLoadedForInsertion).
#[derive(Debug, Clone)]
pub enum NoTransLoad {
    /// A sky or water for the frame at the time cursor.
    Frame(SettingsDoc),
    /// A day whose track is to be chosen ("Load Track From").
    Track(DayDoc),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Confirm {
    Close,
    Import,
}

/// A key frame dragged on the timeline: (index, copy with Shift, position).
#[derive(Debug, Clone, Copy)]
struct Drag {
    index: usize,
    copy: bool,
    pos: f32,
}

#[derive(Default)]
pub struct DayEditorUi {
    pub open: bool,
    pub doc: Option<DayDoc>,
    pub item: Option<Uuid>,
    pub rights: Rights,
    pub dirty: bool,
    pub loading: Option<Uuid>,
    pub saving: Option<Uuid>,
    pub message: Option<(String, bool)>,
    /// LLSettingsDay::TRACK_*: 0 water, 1 ground, 2..=4 skies.
    pub track: usize,
    /// The time cursor (0..1).
    pub pos: f32,
    /// The last sky track chosen (the water keeps the sky shown).
    sky_track: usize,
    /// The selected key frame of the track (selectFrame).
    pub selected: Option<usize>,
    playing: Option<(Instant, f32)>,
    drag: Option<Drag>,
    /// The panels editing the selected frame, and which frame they hold.
    frame: EnvEditorUi,
    frame_key: Option<(usize, usize)>,
    /// The day as rendered, rebuilt when the document changes.
    rendered: Option<DayCycle>,
    /// A day loaded by "Load Track From", waiting for its track to be chosen.
    pub track_source: Option<DayDoc>,
    /// Day length (s) for the time labels (parcel and region contexts).
    pub day_length: Option<f64>,
    /// Where the day comes from and goes back to.
    pub context: DayContext,
    /// The day as opened in a parcel or region context: committed only when
    /// changed (onEditCommitted compares the hashes).
    pub committed: Option<aurora_llsd::Llsd>,
    /// The parcel's or region's day length and offset (s, -1 unknown).
    pub timing: (i32, i32),
    /// The parcel's day as About Land last showed it (None: it had none),
    /// to follow its changes (updateEditFloater).
    pub followed: Option<std::sync::Arc<aurora_llsd::Llsd>>,
    /// The parcel can no longer take the edited day: the commit is off
    /// (mCommitSignal disconnected), the changes kept.
    pub commit_lost: bool,
    /// Loaded settings waiting for "make the day no-transfer?".
    pub no_trans_ask: Option<NoTransLoad>,
    /// Its "Do not show again" box, and the request to stop asking (the
    /// app clears the setting).
    no_trans_dont_ask: bool,
    pub stop_asking_no_trans: bool,
    /// The sun and moon beacons (kept by the app).
    pub beacons: [bool; 2],
    /// The region's sky altitudes for the track names (parcel and region
    /// contexts with the extended environment).
    pub altitudes: Option<[f32; 4]>,
    /// The region has the extended environment: skies 2 to 4 can be edited.
    pub extended: bool,
    /// Waiting for Firestorm's default day (setEditDefaultDayCycle).
    pub wait_default: bool,
    confirm: Option<Confirm>,
    save_as: Option<String>,
    name: String,
    pub wanted_images: HashSet<Uuid>,
    /// Demo: an item to open once its folder has loaded.
    pub wait_item: Option<Uuid>,
}

/// An altitude as onOpen writes it into "Sky [ALT]" (a float streamed
/// with "m").
fn altitude_text(alt: f32) -> String {
    if alt.fract() == 0.0 {
        format!("{alt:.0}m")
    } else {
        format!("{alt}m")
    }
}

/// The track buttons' labels: skies 2 to 4 by their altitude when known.
pub fn track_label(t: usize, altitudes: Option<&[f32; 4]>) -> String {
    match (t, altitudes) {
        (TRACK_WATER, _) => aurora_i18n::tr!("common-water").to_owned(),
        (TRACK_GROUND, _) => aurora_i18n::tr!("ui-day-editor-ground").to_owned(),
        (n, Some(a)) => aurora_i18n::tr!("ui-day-editor-sky-n", n = altitude_text(a[n - 1])),
        (n, None) => aurora_i18n::tr!("ui-day-editor-sky-n", n = format!("{n}")),
    }
}

impl DayEditorUi {
    pub fn start(&mut self, mut doc: DayDoc, item: Option<Uuid>, rights: Rights) {
        doc.ensure_defaults();
        self.open = true;
        self.name = doc.name().to_owned();
        self.doc = Some(doc);
        self.item = item;
        self.rights = rights;
        self.dirty = false;
        self.loading = None;
        self.track = TRACK_GROUND;
        self.sky_track = TRACK_GROUND;
        self.pos = 0.0;
        self.playing = None;
        self.drag = None;
        self.frame_key = None;
        self.rendered = None;
        self.track_source = None;
        self.confirm = None;
        self.save_as = None;
        self.wait_default = false;
        self.select_at_cursor();
    }

    pub fn start_loading(&mut self, item: Uuid) {
        self.open = true;
        self.loading = Some(item);
        self.message = None;
    }

    /// Open on a day that is not an item (a parcel's or region's day, the
    /// default day), in `context`; `timing` is the environment's day length
    /// and offset (s, -1 when unknown) the commit sends back, the length
    /// also naming the times of the timeline (KEY_DAY_LENGTH, four hours
    /// when unknown).
    pub fn start_in(&mut self, doc: DayDoc, context: DayContext, timing: (i32, i32)) {
        let rights = Rights::from_flags(doc.flags());
        self.start(doc, None, rights);
        self.context = context;
        self.timing = timing;
        let (day_length, committed) = match context {
            DayContext::Inventory => (None, None),
            _ => (
                Some(if timing.0 > 0 { timing.0 as f64 } else { 4.0 * 3600.0 }),
                self.doc.as_ref().map(DayDoc::to_llsd),
            ),
        };
        self.day_length = day_length;
        self.committed = committed;
    }

    /// "Save As" confirmed by the app: the name prompt.
    pub fn ask_save_as(&mut self) {
        self.save_as = Some(self.name.clone());
    }

    /// Import over the current day (loadSettingFromFile: no item, the
    /// ground track selected, unsaved).
    pub fn imported(&mut self, doc: DayDoc) {
        // the context, day length and committed day stay
        let rights = Rights::from_flags(doc.flags());
        self.start(doc, None, rights);
        self.dirty = true;
    }

    /// The label of the commit action in a parcel or region context.
    fn commit_label(&self) -> Option<&'static str> {
        match self.context {
            DayContext::Inventory => None,
            DayContext::Parcel(_) => Some(aurora_i18n::tr!("ui-env-editor-apply-parcel")),
            DayContext::Region => Some(aurora_i18n::tr!("ui-env-editor-apply-region")),
        }
    }

    pub fn close(&mut self) {
        *self = DayEditorUi::default();
    }

    /// The day as rendered (cached).
    pub fn rendered(&mut self) -> Option<&DayCycle> {
        if self.rendered.is_none() {
            self.rendered = self.doc.as_ref().map(DayDoc::day);
        }
        self.rendered.as_ref()
    }

    /// What the world shows: the day at the time cursor.
    pub fn shown(&mut self) -> Option<(SkyFrame, WaterFrame)> {
        let (pos, sky_track) = (self.pos, self.sky_track);
        self.rendered()?;
        let (doc, day) = (self.doc.as_ref()?, self.rendered.as_ref()?);
        Some(doc.frames_at(day, pos, sky_track))
    }

    /// The editor follows a save (onInventoryCreated / Updated).
    pub fn saved(&mut self, item: Uuid, name: &str, rights: Rights) {
        if let Some(doc) = &mut self.doc {
            doc.set_name(name);
        }
        self.name = name.to_owned();
        self.item = Some(item);
        self.rights = rights;
        self.dirty = false;
        self.message = Some((aurora_i18n::tr!("ui-env-editor-saved").into(), false));
    }

    fn changed(&mut self, out: &mut Vec<DayAction>) {
        self.dirty = true;
        self.rendered = None;
        self.message = None;
        if !out.contains(&DayAction::Changed) {
            out.push(DayAction::Changed);
        }
    }

    /// selectFrame(pos, DEFAULT_FRAME_SLOP_FACTOR): the frame at the cursor.
    fn select_at_cursor(&mut self) {
        self.selected = self.doc.as_ref().and_then(|d| d.near(self.track, self.pos, day::FRAME_SLOP));
        if let (Some(i), Some(d)) = (self.selected, &self.doc) {
            self.pos = d.tracks[self.track][i].0;
        }
    }

    pub fn select_track(&mut self, t: usize) {
        self.track = t.min(TRACKS - 1);
        if self.track != TRACK_WATER {
            self.sky_track = self.track;
        }
        self.select_at_cursor();
    }

    pub fn set_pos(&mut self, pos: f32) {
        self.pos = day::snap(pos);
        self.select_at_cursor();
    }

    /// A sky or water loaded for "Load [frame]".
    pub fn put_loaded_frame(&mut self, frame: &SettingsDoc, out: &mut Vec<DayAction>) {
        let (track, pos) = (self.track, self.pos);
        let Some(doc) = &mut self.doc else { return };
        if doc.put_frame(track, pos, frame).is_none() {
            self.message = Some((aurora_i18n::tr!("ui-day-editor-frame-full").into(), true));
            return;
        }
        self.frame_key = None;
        self.select_at_cursor();
        self.changed(out);
    }

    /// Demo and tests: the cursor on a frame, edited by `edit`.
    pub fn edit_selected_sky(&mut self, edit: impl FnOnce(&mut SkyFrame)) -> bool {
        let (Some(i), Some(doc)) = (self.selected, self.doc.as_mut()) else {
            return false;
        };
        let Some(mut f) = doc.frame_doc(self.track, i) else { return false };
        let before = f.sky();
        let mut after = before;
        edit(&mut after);
        if !f.edit_sky(&before, &after) {
            return false;
        }
        doc.set_frame(self.track, i, &f);
        self.frame_key = None;
        self.dirty = true;
        self.rendered = None;
        true
    }

    pub fn add_frame_at_cursor(&mut self) -> bool {
        let (track, pos, sky) = (self.track, self.pos, self.sky_track);
        let Some(doc) = &mut self.doc else { return false };
        let added = doc.add_frame(track, pos, sky).is_some();
        if added {
            self.dirty = true;
            self.rendered = None;
            self.select_at_cursor();
        }
        added
    }

    /// SettingsMakeNoTrans answered "Yes" (or not asked): the day becomes
    /// no-transfer and takes the loaded settings.
    pub fn accept_no_trans(&mut self, load: NoTransLoad, out: &mut Vec<DayAction>) {
        self.rights.transfer = false;
        if let Some(doc) = &mut self.doc {
            doc.set_flags(doc.flags() | env_edit::FLAG_NOTRANS);
        }
        self.dirty = true;
        match load {
            NoTransLoad::Frame(frame) => self.put_loaded_frame(&frame, out),
            NoTransLoad::Track(day) => self.track_source = Some(day),
        }
    }

    /// "Clone Track From": track `from` of this day into `to`.
    pub fn copy_track(&mut self, from: usize, to: usize) -> bool {
        let Some(doc) = &mut self.doc else { return false };
        let src = doc.clone();
        if doc.clone_track(&src, from, to).is_err() {
            return false;
        }
        self.frame_key = None;
        self.dirty = true;
        self.rendered = None;
        self.select_at_cursor();
        true
    }

    pub fn start_play(&mut self) {
        self.playing = Some((Instant::now(), self.pos));
        self.drag = None;
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        inv: &Inventory,
        images: &HashMap<Uuid, egui::TextureHandle>,
        access: EditorAccess,
    ) -> Vec<DayAction> {
        let mut out = Vec::new();
        if !self.open {
            return out;
        }
        // Play: a whole day in DAY_CYCLE_PLAY_TIME_SECONDS (onIdlePlay)
        if let Some((start, from)) = self.playing {
            self.pos = (from + start.elapsed().as_secs_f32() / day::PLAY_SECONDS).fract();
            self.selected = None;
            if !out.contains(&DayAction::Changed) {
                out.push(DayAction::Changed);
            }
            ctx.request_repaint();
        }
        let screen = ctx.content_rect();
        let size = Vec2::new(640.0, 760.0);
        let mut open = true;
        let mut floater = Floater::new(WINDOW, aurora_i18n::tr!("ui-day-editor-title"), screen.center() - size * 0.5, size);
        floater.min_size = Vec2::new(600.0, 600.0);
        floater.help(aurora_i18n::tr!("ui-day-editor-help")).show(ctx, p, &mut open, |ui| {
            ui.spacing_mut().item_spacing = Vec2::new(6.0, 5.0);
            if self.doc.is_none() || self.loading.is_some() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| match &self.message {
                    Some((text, true)) => ui.label(RichText::new(text).color(p.danger)),
                    _ => ui.label(RichText::new(aurora_i18n::tr!("ui-env-editor-loading")).color(p.muted)),
                });
                return;
            }
            self.name_row(ui, p, &mut out);
            ui.add_space(4.0);
            self.tracks_row(ui, p, &mut out);
            self.timeline(ui, p, &mut out);
            self.track_row(ui, p, inv, &mut out);
            self.frame_row(ui, p, inv, &mut out);
            widgets::divider(ui, p);
            // the panels in exactly the room the footer leaves (a guessed
            // footer grew the window at every frame with a larger text)
            let (mut panels, foot) = widgets::above_footer(ui);
            let body_h = panels.available_height();
            self.frame_panels(&mut panels, p, images, body_h, &mut out);
            foot.show(ui, |ui| {
                widgets::divider(ui, p);
                self.footer(ui, p, access, &mut out);
            });
        });
        if !open {
            if self.dirty {
                self.confirm = Some(Confirm::Close);
            } else {
                self.close();
                out.push(DayAction::Closed);
            }
        }
        self.dialogs(ctx, p, &mut out);
        out
    }

    fn name_row(&mut self, ui: &mut egui::Ui, p: &Palette, out: &mut Vec<DayAction>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("ui-day-editor-name")).size(12.0).color(p.ink));
            // refresh: renaming needs modify rights and an item
            let enabled = self.rights.modify && self.rights.save && self.item.is_some();
            let r = ui.add_enabled(
                enabled,
                egui::TextEdit::singleline(&mut self.name).desired_width(320.0).char_limit(63),
            );
            if r.changed()
                && let Some(doc) = &mut self.doc
            {
                // onNameKeystroke
                doc.set_name(&self.name);
                self.dirty = true;
            }
            // btn_import (mImportButton->setEnabled(mCanMod))
            if widgets::action_button_if(
                ui,
                p,
                "folder-open-filled",
                aurora_i18n::tr!("ui-env-editor-import"),
                ActionKind::Flat,
                Vec2::new(0.0, 24.0),
                self.rights.modify,
            )
            .on_hover_text(aurora_i18n::tr!("ui-day-editor-import-tip"))
            .clicked()
            {
                if self.dirty {
                    self.confirm = Some(Confirm::Import);
                } else {
                    out.push(DayAction::Import);
                }
            }
        });
    }

    fn tracks_row(&mut self, ui: &mut egui::Ui, p: &Palette, out: &mut Vec<DayAction>) {
        // the track buttons, Water first like timeline_layers; skies 2 to 4
        // need the extended environment (onOpen)
        let labels: Vec<String> = (0..TRACKS).map(|t| track_label(t, self.altitudes.as_ref())).collect();
        let tabs: Vec<(&str, bool)> = labels
            .iter()
            .enumerate()
            .map(|(t, l)| (l.as_str(), self.playing.is_none() && (t <= TRACK_GROUND || self.extended)))
            .collect();
        if !self.extended && self.track > TRACK_GROUND {
            self.select_track(TRACK_GROUND);
            out.push(DayAction::Changed);
        }
        let mut t = self.track;
        if widgets::tabs(ui, p, &mut t, &tabs) {
            self.select_track(t);
            out.push(DayAction::Changed);
        }
    }

    /// WLTimeSlider over WLDayCycleFrames, with the p0..p4 labels.
    fn timeline(&mut self, ui: &mut egui::Ui, p: &Palette, out: &mut Vec<DayAction>) {
        let width = ui.available_width();
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 58.0), Sense::click_and_drag());
        let resp = resp.on_hover_text(aurora_i18n::tr!("ui-day-editor-timeline-tip"));
        let pad = 10.0;
        let x0 = rect.left() + pad;
        let span = rect.width() - 2.0 * pad;
        let x_of = |pos: f32| x0 + pos * span;
        let pos_of = |x: f32| ((x - x0) / span).clamp(0.0, 1.0);
        let radius = ui.visuals().widgets.inactive.corner_radius;
        let painter = ui.painter_at(rect);
        let lane = egui::Rect::from_min_max(egui::pos2(x0, rect.top() + 18.0), egui::pos2(x0 + span, rect.top() + 38.0));
        painter.rect_filled(lane, radius, p.field);
        // grid at the quarters
        for i in 0..=4 {
            let x = x_of(i as f32 / 4.0);
            painter.line_segment([egui::pos2(x, lane.top()), egui::pos2(x, lane.bottom())], (1.0, p.raised));
        }
        let can_edit = self.rights.modify && self.playing.is_none();
        let frames: Vec<f32> = self
            .doc
            .as_ref()
            .map(|d| d.tracks[self.track].iter().map(|k| k.0).collect())
            .unwrap_or_default();
        let hit = |x: f32| frames.iter().position(|f| (x_of(*f) - x).abs() <= 7.0);
        // interactions: press on a key frame selects it (and starts a drag),
        // elsewhere moves the time cursor; double click adds a frame
        if resp.drag_started() || resp.clicked() {
            self.playing = None;
            if let Some(ptr) = resp.interact_pointer_pos() {
                match hit(ptr.x).filter(|_| ptr.y >= lane.top() - 4.0) {
                    Some(i) => {
                        self.selected = Some(i);
                        self.pos = frames[i];
                        if resp.drag_started() && can_edit {
                            let copy = ui.input(|i| i.modifiers.shift);
                            self.drag = Some(Drag {
                                index: i,
                                copy,
                                pos: frames[i],
                            });
                        }
                    }
                    None => self.set_pos(pos_of(ptr.x)),
                }
                out.push(DayAction::Changed);
            }
        }
        if resp.dragged()
            && let Some(ptr) = resp.interact_pointer_pos()
        {
            match &mut self.drag {
                Some(d) => d.pos = day::snap(pos_of(ptr.x)),
                None => {
                    self.set_pos(pos_of(ptr.x));
                    out.push(DayAction::Changed);
                }
            }
        }
        if resp.drag_stopped()
            && let Some(d) = self.drag.take()
            && let Some(doc) = &mut self.doc
        {
            let result = if d.copy {
                doc.copy_frame(self.track, d.index, d.pos)
            } else {
                doc.move_frame(self.track, d.index, d.pos)
            };
            if let Some(i) = result {
                self.selected = Some(i);
                self.pos = doc.tracks[self.track][i].0;
                self.frame_key = None;
                self.changed(out);
            }
        }
        if resp.double_clicked() && can_edit && self.add_frame_at_cursor() {
            self.frame_key = None;
            self.changed(out);
        }
        // key frames
        for (i, f) in frames.iter().enumerate() {
            let shown = match self.drag {
                Some(d) if d.index == i && !d.copy => d.pos,
                _ => *f,
            };
            let c = egui::pos2(x_of(shown), lane.center().y);
            let r = egui::Rect::from_center_size(c, Vec2::new(10.0, 16.0));
            let fill = if self.selected == Some(i) {
                p.violet
            } else {
                p.ink.gamma_multiply(0.75)
            };
            painter.rect_filled(r, radius, fill);
        }
        if let Some(d) = self.drag.filter(|d| d.copy) {
            let c = egui::pos2(x_of(d.pos), lane.center().y);
            painter.rect_stroke(
                egui::Rect::from_center_size(c, Vec2::new(10.0, 16.0)),
                radius,
                (1.5, p.violet_light),
                egui::StrokeKind::Inside,
            );
        }
        // the time cursor: a triangle above the lane and its line
        let cx = x_of(self.pos);
        painter.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(cx - 6.0, rect.top() + 6.0),
                egui::pos2(cx + 6.0, rect.top() + 6.0),
                egui::pos2(cx, rect.top() + 15.0),
            ],
            p.amber,
            egui::Stroke::NONE,
        ));
        painter.line_segment([egui::pos2(cx, rect.top() + 15.0), egui::pos2(cx, lane.bottom())], (1.5, p.amber));
        // p0..p4
        for i in 0..=4 {
            let at = i as f32 / 4.0;
            let align = match i {
                0 => egui::Align2::LEFT_TOP,
                4 => egui::Align2::RIGHT_TOP,
                _ => egui::Align2::CENTER_TOP,
            };
            painter.text(
                egui::pos2(x_of(at), lane.bottom() + 4.0),
                align,
                day::time_label(at, self.day_length),
                egui::FontId::proportional(11.0),
                p.muted,
            );
        }
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(day::time_label(self.pos, self.day_length))
                    .size(12.0)
                    .strong()
                    .color(p.ink),
            );
        });
    }

    fn track_row(&mut self, ui: &mut egui::Ui, p: &Palette, inv: &Inventory, out: &mut Vec<DayAction>) {
        let size = Vec2::new(0.0, 24.0);
        let can = self.rights.modify && self.playing.is_none();
        let (can_clone, can_clear) = self
            .doc
            .as_ref()
            .map_or((false, false), |d| (d.can_clone(self.track), d.can_clear(self.track)));
        ui.horizontal(|ui| {
            let clone = widgets::action_button_if(
                ui,
                p,
                "copy-filled",
                aurora_i18n::tr!("ui-day-editor-clone-track"),
                ActionKind::Flat,
                size,
                can && can_clone,
            );
            super::menu::dropdown(&clone, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                // LLFloaterTrackPicker: the other sky tracks with frames
                for t in TRACK_GROUND..TRACKS {
                    let filled = self.doc.as_ref().is_some_and(|d| !d.tracks[t].is_empty());
                    if t != self.track
                        && super::menu::item_if(ui, p, "sun-filled", &track_label(t, self.altitudes.as_ref()), filled)
                        && self.copy_track(t, self.track)
                    {
                        self.changed(out);
                    }
                }
            });
            let load = widgets::action_button_if(
                ui,
                p,
                "briefcase-filled",
                aurora_i18n::tr!("ui-day-editor-load-track"),
                ActionKind::Flat,
                size,
                can,
            );
            super::menu::dropdown(&load, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                let order = super::inventory::view::picker_order(ui.ctx(), inv);
                let items = SettingsCatalog::items_of(inv, SettingsKind::Day, |id| order.rank(id));
                if items.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-day-editor-no-days")).size(12.0).color(p.muted));
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (id, name) in items {
                        if super::menu::item(ui, p, "sunset-2-filled", &name) {
                            out.push(DayAction::LoadTrack(id));
                        }
                    }
                });
            });
            if widgets::action_button_if(
                ui,
                p,
                "trash-filled",
                aurora_i18n::tr!("ui-day-editor-clear-track"),
                ActionKind::Flat,
                size,
                can && can_clear,
            )
            .clicked()
                && let Some(doc) = &mut self.doc
            {
                doc.clear_track(self.track);
                self.frame_key = None;
                self.select_at_cursor();
                self.changed(out);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // skip forward, play / pause, skip back (track_progress_lp)
                let step = |s: &mut Self, forward: bool, out: &mut Vec<DayAction>| {
                    s.playing = None;
                    if let Some(to) = s.doc.as_ref().and_then(|d| d.step(s.track, s.pos, forward)) {
                        s.pos = to;
                        s.select_at_cursor();
                        out.push(DayAction::Changed);
                    }
                };
                if widgets::icon_button(ui, p, "caret-right-filled", aurora_i18n::tr!("ui-day-editor-forward"), true) {
                    step(self, true, out);
                }
                if self.playing.is_some() {
                    if widgets::icon_button(ui, p, "player-pause-filled", aurora_i18n::tr!("ui-day-editor-pause"), true) {
                        self.playing = None;
                        self.select_at_cursor();
                    }
                } else if widgets::icon_button(ui, p, "player-play-filled", aurora_i18n::tr!("ui-day-editor-play"), true) {
                    self.start_play();
                }
                if widgets::icon_button(ui, p, "caret-left-filled", aurora_i18n::tr!("ui-day-editor-back"), true) {
                    step(self, false, out);
                }
            });
        });
    }

    fn frame_row(&mut self, ui: &mut egui::Ui, p: &Palette, inv: &Inventory, out: &mut Vec<DayAction>) {
        let size = Vec2::new(0.0, 24.0);
        let can = self.rights.modify && self.playing.is_none();
        let water = self.track == TRACK_WATER;
        let (add, load, delete) = if water {
            (
                aurora_i18n::tr!("ui-day-editor-add-water"),
                aurora_i18n::tr!("ui-day-editor-load-water"),
                aurora_i18n::tr!("ui-day-editor-delete-water"),
            )
        } else {
            (
                aurora_i18n::tr!("ui-day-editor-add-sky"),
                aurora_i18n::tr!("ui-day-editor-load-sky"),
                aurora_i18n::tr!("ui-day-editor-delete-sky"),
            )
        };
        let can_add = self.selected.is_none() && self.doc.as_ref().is_some_and(|d| d.can_add(self.track, self.pos));
        let can_remove = self.selected.is_some() && self.doc.as_ref().is_some_and(|d| d.can_remove(self.track));
        ui.horizontal(|ui| {
            if widgets::action_button_if(ui, p, "circle-plus-filled", add, ActionKind::Flat, size, can && can_add).clicked()
                && self.add_frame_at_cursor()
            {
                self.frame_key = None;
                self.changed(out);
            }
            let l = widgets::action_button_if(ui, p, "briefcase-filled", load, ActionKind::Flat, size, can);
            super::menu::dropdown(&l, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                let kind = if water { SettingsKind::Water } else { SettingsKind::Sky };
                let order = super::inventory::view::picker_order(ui.ctx(), inv);
                let items = SettingsCatalog::items_of(inv, kind, |id| order.rank(id));
                if items.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("ui-env-editor-no-items")).size(12.0).color(p.muted));
                }
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    for (id, name) in items {
                        let icon = if water { "droplet-filled" } else { "sun-filled" };
                        if super::menu::item(ui, p, icon, &name) {
                            out.push(DayAction::LoadFrame(id));
                        }
                    }
                });
            });
            if widgets::action_button_if(ui, p, "circle-x-filled", delete, ActionKind::Flat, size, can && can_remove).clicked()
                && let (Some(i), Some(doc)) = (self.selected, &mut self.doc)
                && doc.remove_frame(self.track, i)
            {
                self.frame_key = None;
                self.select_at_cursor();
                self.changed(out);
            }
        });
    }

    /// frame_edit_controls: the selected frame in the sky tabs or the water
    /// panel, else "Select a key frame…" (icn_lock_edit).
    fn frame_panels(
        &mut self,
        ui: &mut egui::Ui,
        p: &Palette,
        images: &HashMap<Uuid, egui::TextureHandle>,
        height: f32,
        out: &mut Vec<DayAction>,
    ) {
        let key = self.selected.filter(|_| self.playing.is_none()).map(|i| (self.track, i));
        let Some((track, index)) = key else {
            self.frame_key = None;
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(aurora_i18n::tr!("ui-day-editor-select-frame")).color(p.muted));
            });
            ui.add_space((height - 40.0).max(0.0));
            return;
        };
        if self.frame_key != Some((track, index)) {
            let Some(doc) = self.doc.as_ref().and_then(|d| d.frame_doc(track, index)) else {
                return;
            };
            let tab = self.frame.tab_index();
            self.frame.start(doc, None, self.rights);
            self.frame.set_tab(tab);
            self.frame_key = Some((track, index));
        }
        self.frame.beacons = self.beacons;
        let changed = self.frame.panels(ui, p, images, height);
        self.beacons = self.frame.beacons;
        if changed {
            if let (Some(f), Some(doc)) = (self.frame.doc.as_ref(), self.doc.as_mut()) {
                doc.set_frame(track, index, f);
            }
            self.changed(out);
        }
        self.wanted_images.extend(self.frame.wanted_images.drain());
    }

    /// The texture picker of the frame panels.
    pub fn show_picker(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &mut World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) -> bool {
        let Some((track, index)) = self.frame_key else {
            return false;
        };
        if !self.frame.show_picker(ctx, p, world, images) {
            return false;
        }
        if let (Some(f), Some(doc)) = (self.frame.doc.as_ref(), self.doc.as_mut()) {
            doc.set_frame(track, index, f);
        }
        self.dirty = true;
        self.rendered = None;
        true
    }

    fn footer(&mut self, ui: &mut egui::Ui, p: &Palette, access: EditorAccess, out: &mut Vec<DayAction>) {
        let can_inventory = access.inventory;
        let commit = self.commit_label();
        ui.horizontal(|ui| {
            if let Some((text, error)) = &self.message {
                ui.label(RichText::new(text).size(11.5).color(if *error { p.danger } else { p.muted }));
            } else if self.saving.is_some() {
                ui.label(RichText::new(aurora_i18n::tr!("ui-env-editor-saving")).size(11.5).color(p.muted));
            } else if !can_inventory {
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-env-editor-no-inventory"))
                        .size(11.5)
                        .color(p.muted),
                );
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let size = Vec2::new(0.0, 26.0);
                if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                    if self.dirty {
                        self.confirm = Some(Confirm::Close);
                    } else {
                        self.close();
                        out.push(DayAction::Closed);
                        return;
                    }
                }
                let idle = self.saving.is_none();
                let save = idle && can_inventory && self.rights.modify && self.rights.save && self.item.is_some();
                let save_as = idle && can_inventory && self.rights.copy && self.rights.save;
                let more = widgets::action_button(ui, p, "caret-down-filled", "", ActionKind::Flat, Vec2::new(26.0, 26.0))
                    .on_hover_text(aurora_i18n::tr!("ui-env-editor-more"));
                // refresh: the commit entry in the parcel and region contexts,
                // Apply To Parcel / Region in the inventory one; Save and
                // Save As only where the region keeps settings
                let can_commit = !self.commit_lost;
                let commit_icon = if self.context == DayContext::Region {
                    "world-filled"
                } else {
                    "map-pin-filled"
                };
                super::menu::dropdown(&more, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                    if let Some(label) = commit
                        && super::menu::item_if(ui, p, commit_icon, label, can_commit)
                    {
                        out.push(DayAction::Commit);
                    }
                    if can_inventory {
                        if super::menu::item_if(ui, p, "device-floppy-filled", aurora_i18n::tr!("common-save"), save) {
                            out.push(DayAction::Save);
                        }
                        if super::menu::item_if(ui, p, "copy-filled", aurora_i18n::tr!("common-save-as"), save_as) {
                            out.push(DayAction::SaveAsAsk);
                        }
                    }
                    super::menu::separator(ui, p);
                    if super::menu::item(ui, p, "user-filled", aurora_i18n::tr!("common-apply-to-myself-only")) {
                        out.push(DayAction::ApplyLocal);
                    }
                    if commit.is_none() {
                        if super::menu::item_if(
                            ui,
                            p,
                            "map-pin-filled",
                            aurora_i18n::tr!("ui-env-editor-apply-parcel"),
                            access.parcel,
                        ) {
                            out.push(DayAction::ApplyParcel);
                        }
                        if super::menu::item_if(ui, p, "world-filled", aurora_i18n::tr!("ui-env-editor-apply-region"), access.region) {
                            out.push(DayAction::ApplyRegion);
                        }
                    }
                });
                // the flyout's selected item: Save, or the commit
                let (icon, label, enabled, action) = match commit {
                    Some(label) => (commit_icon, label, idle && can_commit, DayAction::Commit),
                    None => ("device-floppy-filled", aurora_i18n::tr!("common-save"), save, DayAction::Save),
                };
                if widgets::action_button_if(ui, p, icon, label, ActionKind::Primary, size, enabled).clicked() {
                    out.push(action);
                }
            });
        });
    }

    fn dialogs(&mut self, ctx: &egui::Context, p: &Palette, out: &mut Vec<DayAction>) {
        if self.confirm.is_some() {
            let name = self.name.clone();
            let mut answer = None;
            let modal = widgets::Modal::new(egui::Id::unique("env_day_editor_confirm")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-env-editor-confirm-loss", name = name))
                        .size(13.0)
                        .color(p.ink),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let size = Vec2::new(0.0, 26.0);
                    if widgets::action_button(ui, p, "check", aurora_i18n::tr!("common-yes"), ActionKind::Danger, size).clicked() {
                        answer = Some(true);
                    }
                    if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-no"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if modal.should_close() && answer.is_none() {
                answer = Some(false);
            }
            match (answer, self.confirm) {
                (Some(true), Some(Confirm::Import)) => {
                    self.confirm = None;
                    out.push(DayAction::Import);
                }
                (Some(true), _) => {
                    self.close();
                    out.push(DayAction::Closed);
                }
                (Some(false), _) => self.confirm = None,
                (None, _) => {}
            }
        }
        if let Some(mut name) = self.save_as.clone() {
            let mut close = false;
            let modal = widgets::Modal::new(egui::Id::unique("env_day_editor_save_as")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(RichText::new(aurora_i18n::tr!("common-save-as")).size(17.0).strong().color(p.ink));
                ui.add_space(6.0);
                ui.label(aurora_i18n::tr!("ui-env-editor-save-as-prompt"));
                ui.add_space(8.0);
                ui.add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY).char_limit(63));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let size = Vec2::new(0.0, 26.0);
                    let enter = widgets::enter_pressed(ui);
                    if widgets::action_button(ui, p, "check", aurora_i18n::tr!("common-ok"), ActionKind::Primary, size).clicked() || enter {
                        out.push(DayAction::SaveAs(env_edit::correct_item_name(&name)));
                        close = true;
                    }
                    if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size).clicked() {
                        close = true;
                    }
                });
            });
            self.save_as = if close || modal.should_close() { None } else { Some(name) };
        }
        // SettingsMakeNoTrans: the loaded settings make the day no-transfer
        if self.no_trans_ask.is_some() {
            let mut answer = None;
            let modal = widgets::Modal::new(egui::Id::unique("env_day_editor_no_trans")).show(ctx, |ui| {
                ui.set_width(400.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(aurora_i18n::tr!("ui-day-editor-make-no-trans"))
                            .size(13.0)
                            .color(p.ink),
                    )
                    .wrap(),
                );
                ui.add_space(8.0);
                // okcancelignore: the box saved once "Yes" is chosen
                widgets::checkbox(ui, p, &mut self.no_trans_dont_ask, aurora_i18n::tr!("ui-context-do-not-show-again"));
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let size = Vec2::new(0.0, 26.0);
                    if widgets::action_button(ui, p, "check", aurora_i18n::tr!("common-yes"), ActionKind::Primary, size).clicked() {
                        answer = Some(true);
                    }
                    if widgets::action_button(ui, p, "x", aurora_i18n::tr!("common-no"), ActionKind::Flat, size).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if modal.should_close() && answer.is_none() {
                answer = Some(false);
            }
            if let Some(yes) = answer
                && let Some(load) = self.no_trans_ask.take()
            {
                let dont_ask = std::mem::take(&mut self.no_trans_dont_ask);
                if yes {
                    self.stop_asking_no_trans |= dont_ask;
                    self.accept_no_trans(load, out);
                }
            }
        }
        // LLFloaterTrackPicker after "Load Track From": which track to copy
        if let Some(src) = self.track_source.clone() {
            let mut done = false;
            let modal = widgets::Modal::new(egui::Id::unique("env_day_editor_track_source")).show(ctx, |ui| {
                ui.set_width(340.0);
                ui.label(
                    RichText::new(aurora_i18n::tr!("ui-day-editor-choose-track"))
                        .size(15.0)
                        .strong()
                        .color(p.ink),
                );
                ui.add_space(8.0);
                for t in 0..TRACKS {
                    // the water track loads into the water only
                    let fits = (t == TRACK_WATER) == (self.track == TRACK_WATER);
                    let enabled = fits && !src.tracks[t].is_empty();
                    if widgets::action_button_if(
                        ui,
                        p,
                        "sun-filled",
                        &track_label(t, self.altitudes.as_ref()),
                        ActionKind::Flat,
                        Vec2::new(0.0, 24.0),
                        enabled,
                    )
                    .clicked()
                    {
                        let to = self.track;
                        if let Some(doc) = &mut self.doc {
                            match doc.clone_track(&src, t, to) {
                                Ok(()) => {
                                    self.frame_key = None;
                                    self.select_at_cursor();
                                    self.changed(out);
                                }
                                Err(TrackError::Mismatch) => {
                                    self.message = Some((aurora_i18n::tr!("ui-day-editor-track-mismatch").into(), true));
                                }
                                Err(TrackError::Empty) => {
                                    self.message = Some((aurora_i18n::tr!("ui-day-editor-track-empty").into(), true));
                                }
                            }
                        }
                        done = true;
                    }
                }
                ui.add_space(6.0);
                if widgets::action_button(
                    ui,
                    p,
                    "x",
                    aurora_i18n::tr!("common-cancel"),
                    ActionKind::Flat,
                    Vec2::new(0.0, 26.0),
                )
                .clicked()
                {
                    done = true;
                }
            });
            if done || modal.should_close() {
                self.track_source = None;
            }
        }
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    /// The day editor keeps its size.
    #[test]
    fn the_window_keeps_its_size() {
        let inv = Inventory::default();
        crate::ui::widgets::steady_tests::assert_window_steady(
            WINDOW,
            &[Vec2::new(640.0, 760.0), Vec2::new(600.0, 600.0)],
            aurora_i18n::tr!("common-cancel"),
            || {
                let mut ui = DayEditorUi::default();
                let doc = DayDoc::from_llsd(aurora_llsd::llsd_map! {"type" => "daycycle"}).expect("day");
                ui.start(doc, None, Rights::default());
                ui
            },
            |ui, ctx, p| {
                ui.show(ctx, p, &inv, &HashMap::new(), EditorAccess::default());
            },
        );
    }
}
