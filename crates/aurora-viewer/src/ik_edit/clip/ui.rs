//! Compact creation pages: timeline, keyed bones, and native playback metadata.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TimelineShortcut {
    PlayPause,
    Rewind,
    Undo,
    Redo,
    CopyPose,
    PastePose,
    DeleteKey,
    SaveProject,
}

pub(super) fn timeline_key(key: &winit::keyboard::Key) -> Option<egui::Key> {
    match key {
        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Space) => Some(egui::Key::Space),
        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Home) => Some(egui::Key::Home),
        winit::keyboard::Key::Named(winit::keyboard::NamedKey::Delete) => Some(egui::Key::Delete),
        winit::keyboard::Key::Character(character) => match character.to_lowercase().as_str() {
            "z" => Some(egui::Key::Z),
            "y" => Some(egui::Key::Y),
            "c" => Some(egui::Key::C),
            "v" => Some(egui::Key::V),
            "s" => Some(egui::Key::S),
            _ => None,
        },
        _ => None,
    }
}

fn timeline_action_row(
    ui: &mut egui::Ui,
    width: f32,
    left: impl FnOnce(&mut egui::Ui),
    center: impl FnOnce(&mut egui::Ui),
    right: impl FnOnce(&mut egui::Ui),
) {
    if width >= 640.0 {
        ui.allocate_ui_with_layout(egui::vec2(width, 0.0), egui::Layout::left_to_right(egui::Align::Min), |row| {
            row.columns(3, |columns| {
                columns[0].with_layout(egui::Layout::left_to_right(egui::Align::Center).with_main_justify(false), left);
                columns[1].with_layout(egui::Layout::left_to_right(egui::Align::Center).with_main_justify(false), center);
                columns[2].with_layout(egui::Layout::left_to_right(egui::Align::Center).with_main_justify(false), right);
            });
        });
    } else {
        ui.horizontal(|row| left(row));
        ui.horizontal(|row| center(row));
        ui.horizontal(|row| right(row));
    }
}

pub(super) fn timeline_default_position(content: egui::Rect, size: egui::Vec2) -> egui::Pos2 {
    egui::pos2(
        (content.right() - size.x - 16.0).max(content.left()),
        (content.bottom() - size.y - 16.0).max(content.top()),
    )
}

pub(super) fn timeline_default_size(content: egui::Rect) -> egui::Vec2 {
    let width = (content.width() - 700.0).clamp(700.0, 960.0).min(content.width());
    egui::vec2(width, content.height().min(440.0))
}

fn auto_key_step_control(ui: &mut egui::Ui, enabled: bool, seconds: &mut f32, max_width: f32) -> bool {
    ui.label(aurora_i18n::tr!("anim-edit-auto-key-step"));
    let width = ui.available_width().min(max_width).max(40.0);
    ui.add_enabled_ui(enabled, |ui| {
        ui.add_sized(
            egui::vec2(width, ui.spacing().interact_size.y),
            egui::Slider::new(seconds, 0.1..=5.0)
                .step_by(0.1)
                .fixed_decimals(1)
                .suffix(aurora_i18n::tr!("anim-edit-seconds-short")),
        )
    })
    .inner
    .on_hover_text(aurora_i18n::tr!("anim-edit-auto-key-step-hint"))
    .changed()
}

pub(super) fn timeline_shortcut(key: egui::Key, modifiers: egui::Modifiers) -> Option<TimelineShortcut> {
    let command = modifiers.ctrl || modifiers.command;
    if key == egui::Key::Space && modifiers.matches_exact(egui::Modifiers::NONE) {
        return Some(TimelineShortcut::PlayPause);
    }
    if key == egui::Key::Delete && modifiers.matches_exact(egui::Modifiers::NONE) {
        return Some(TimelineShortcut::DeleteKey);
    }
    if key == egui::Key::Home && modifiers.matches_exact(egui::Modifiers::NONE) {
        return Some(TimelineShortcut::Rewind);
    }
    if command && !modifiers.alt {
        return match (key, modifiers.shift) {
            (egui::Key::Z, false) => Some(TimelineShortcut::Undo),
            (egui::Key::Z, true) | (egui::Key::Y, false) => Some(TimelineShortcut::Redo),
            (egui::Key::C, false) => Some(TimelineShortcut::CopyPose),
            (egui::Key::V, false) => Some(TimelineShortcut::PastePose),
            (egui::Key::S, false) => Some(TimelineShortcut::SaveProject),
            _ => None,
        };
    }
    None
}

fn checkpoint_on_edit(response: &egui::Response) -> bool {
    response.drag_started() || (response.changed() && !response.dragged())
}

pub(super) fn timeline_view(start: u32, end: u32, zoom: f32, requested_start: u32) -> (u32, u32) {
    let total = end.saturating_sub(start).max(1);
    let visible = ((total as f32 / zoom.clamp(1.0, 64.0)).round() as u32).clamp(1, total);
    let view_start = requested_start.clamp(start, end.saturating_sub(visible));
    (view_start, view_start + visible)
}

pub(super) fn timeline_start_around(start: u32, end: u32, zoom: f32, anchor: u32) -> u32 {
    let (_, view_end) = timeline_view(start, end, zoom, start);
    let visible = view_end - start;
    timeline_view(start, end, zoom, anchor.saturating_sub(visible / 2)).0
}

pub(super) fn reveal_timeline_frame(start: u32, end: u32, zoom: f32, requested_start: u32, frame: u32) -> u32 {
    let (left, right) = timeline_view(start, end, zoom, requested_start);
    if (left..=right).contains(&frame) {
        left
    } else {
        timeline_start_around(start, end, zoom, frame.clamp(start, end))
    }
}

pub(super) fn follow_timeline_playhead(start: u32, end: u32, zoom: f32, requested_start: u32, frame: u32, playing: bool) -> u32 {
    let (left, right) = timeline_view(start, end, zoom, requested_start);
    if !playing || (frame >= left && frame <= right) {
        return left;
    }
    let visible = right - left;
    timeline_view(start, end, zoom, frame.saturating_sub(visible * 9 / 10)).0
}

impl Editor {
    pub(crate) fn map_timeline_key(key: &winit::keyboard::Key) -> Option<egui::Key> {
        timeline_key(key)
    }

    /// Route editor accelerators before the viewer's global keybindings see
    /// them. This keeps timeline commands such as Home and Ctrl+S from also
    /// changing flight or resynchronizing avatar animations.
    pub(crate) fn handle_timeline_shortcut(&mut self, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        if !self.clip.active
            || !self.open
            || self.clip.waiting_source()
            || ctx.egui_wants_keyboard_input()
            || egui::Popup::is_any_open(ctx)
            || ctx.memory(|memory| memory.top_modal_layer().is_some() || memory.focused().is_some())
        {
            return false;
        }
        let Some(shortcut) = timeline_shortcut(key, modifiers) else {
            return false;
        };
        match shortcut {
            TimelineShortcut::PlayPause => self.studio_toggle_clip_playback(),
            TimelineShortcut::Rewind => {
                if let Some(start) = self.clip.document.as_ref().map(|document| document.start) {
                    self.clip_seek(start);
                }
            }
            TimelineShortcut::Undo if !self.clip.undo.is_empty() => self.studio_clip_undo(),
            TimelineShortcut::Redo if !self.clip.redo.is_empty() => self.studio_clip_redo(),
            TimelineShortcut::CopyPose => self.studio_copy_clip_keyframe(),
            TimelineShortcut::PastePose if !self.clip.clipboard.is_empty() => self.studio_paste_clip_keyframe(),
            TimelineShortcut::DeleteKey if self.selected.is_some_and(|bone| self.clip.has_key(Some(bone), self.clip.frame)) => {
                self.studio_delete_selected_clip_bone_keyframe();
            }
            TimelineShortcut::SaveProject if !self.clip.busy => self.queue_studio_file_action(files::Action::SaveProject),
            _ => {}
        }
        true
    }

    pub(in crate::ik_edit) fn studio_timeline_overlay(&mut self, ctx: &egui::Context, palette: &Palette, world: &World) {
        if !self.clip.active || !self.open || self.clip.waiting_source() || !self.clip.timeline_open {
            return;
        }
        let Some(document) = self.clip.document.as_ref() else { return };
        let start = document.start;
        let end = document.end;
        let fps = document.fps;
        let mut timeline_start = self.clip.timeline_start;
        let mut timeline_zoom = self.clip.timeline_zoom;
        timeline_start = follow_timeline_playhead(start, end, timeline_zoom, timeline_start, self.clip.frame, self.clip.playing);
        let (mut view_start, mut view_end) = timeline_view(start, end, timeline_zoom, timeline_start);
        self.clip.timeline_start = view_start;
        let tracks = self.clip.timeline_tracks().unwrap_or_default();
        let frame_snapshot = Frame::new(world);
        let selected_bone = self.selected;
        let frame_has_selected_key = selected_bone.is_some_and(|bone| self.clip.has_key(Some(bone), self.clip.frame));
        let frame_has_any_key = self.clip.has_key(None, self.clip.frame);
        let content = ctx.content_rect();
        let size = timeline_default_size(content);
        let pos = timeline_default_position(content, size);
        let mut open = self.clip.timeline_open;
        let mut floater = widgets::Floater::new(
            "avatar-studio-animation-timeline-window-v3",
            aurora_i18n::tr!("avatar-studio-animation-timeline"),
            pos,
            size,
        );
        floater.min_size = egui::vec2(content.width().min(700.0), content.height().min(320.0));
        floater.order = egui::Order::Foreground;
        floater.steady_size = true;
        floater.show(ctx, palette, &mut open, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("avatar-studio-animation-timeline-body")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let controls_width = ui.available_width();
                    timeline_action_row(
                        ui,
                        controls_width,
                        |_ui| {},
                        |_ui| {},
                        |ui| {
                            let loop_kind = if self.clip.timeline_looping {
                                widgets::ActionKind::Primary
                            } else {
                                widgets::ActionKind::Flat
                            };
                            if widgets::action_button(
                                ui,
                                palette,
                                "repeat",
                                aurora_i18n::tr!("anim-edit-timeline-loop"),
                                loop_kind,
                                egui::vec2(0.0, 24.0),
                            )
                            .on_hover_text(aurora_i18n::tr!("anim-edit-timeline-loop-hint"))
                            .clicked()
                            {
                                self.clip.timeline_looping = !self.clip.timeline_looping;
                            }
                            if self.clip.timeline_looping {
                                egui::ComboBox::from_id_salt("animation-timeline-loop-range")
                                    .selected_text(if self.clip.timeline_loop_last_key {
                                        aurora_i18n::tr!("anim-edit-loop-last-key")
                                    } else {
                                        aurora_i18n::tr!("anim-edit-loop-full-range")
                                    })
                                    .width(128.0)
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.clip.timeline_loop_last_key,
                                            false,
                                            aurora_i18n::tr!("anim-edit-loop-full-range"),
                                        );
                                        ui.selectable_value(
                                            &mut self.clip.timeline_loop_last_key,
                                            true,
                                            aurora_i18n::tr!("anim-edit-loop-last-key"),
                                        );
                                    });
                            }
                            if ui
                                .button(if self.clip.playing {
                                    aurora_i18n::tr!("ik-graph-pause")
                                } else {
                                    aurora_i18n::tr!("ik-graph-play")
                                })
                                .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-play"))
                                .clicked()
                            {
                                self.studio_toggle_clip_playback();
                            }
                        },
                    );
                    let mut pan_back = false;
                    let mut pan_forward = false;
                    let mut zoom_out = false;
                    let mut zoom_in = false;
                    let mut fit_timeline = false;
                    let range_start = view_start;
                    let range_end = view_end;
                    let range_zoom = timeline_zoom;
                    timeline_action_row(
                        ui,
                        controls_width,
                        |ui| {
                            pan_back =
                                widgets::action_button(ui, palette, "chevron-left", "", widgets::ActionKind::Flat, egui::vec2(24.0, 24.0))
                                    .on_hover_text(aurora_i18n::tr!("anim-edit-timeline-pan-back"))
                                    .clicked();
                            pan_forward =
                                widgets::action_button(ui, palette, "chevron-right", "", widgets::ActionKind::Flat, egui::vec2(24.0, 24.0))
                                    .on_hover_text(aurora_i18n::tr!("anim-edit-timeline-pan-forward"))
                                    .clicked();
                            zoom_out = widgets::action_button(
                                ui,
                                palette,
                                "zoom-out-filled",
                                "",
                                widgets::ActionKind::Flat,
                                egui::vec2(24.0, 24.0),
                            )
                            .on_hover_text(aurora_i18n::tr!("anim-edit-timeline-zoom-out"))
                            .clicked();
                            zoom_in = widgets::action_button(
                                ui,
                                palette,
                                "zoom-in-filled",
                                "",
                                widgets::ActionKind::Flat,
                                egui::vec2(24.0, 24.0),
                            )
                            .on_hover_text(aurora_i18n::tr!("anim-edit-timeline-zoom-in"))
                            .clicked();
                        },
                        |ui| {
                            ui.label(format!("{}\u{2013}{} \u{00b7} {:.0}\u{00d7}", range_start, range_end, range_zoom));
                        },
                        |ui| {
                            fit_timeline = ui
                                .button(aurora_i18n::tr!("anim-edit-timeline-fit"))
                                .on_hover_text(aurora_i18n::tr!("anim-edit-timeline-fit-hint"))
                                .clicked()
                        },
                    );
                    if pan_back {
                        let visible = view_end - view_start;
                        timeline_start = timeline_start.saturating_sub((visible * 3 / 4).max(1));
                    }
                    if pan_forward {
                        let visible = view_end - view_start;
                        timeline_start = timeline_start.saturating_add((visible * 3 / 4).max(1));
                    }
                    if zoom_out {
                        timeline_zoom = (timeline_zoom / 2.0).max(1.0);
                        timeline_start = timeline_start_around(start, end, timeline_zoom, self.clip.frame);
                    }
                    if zoom_in {
                        timeline_zoom = (timeline_zoom * 2.0).min(64.0);
                        timeline_start = timeline_start_around(start, end, timeline_zoom, self.clip.frame);
                    }
                    if fit_timeline {
                        timeline_zoom = 1.0;
                        timeline_start = start;
                    }
                    (view_start, view_end) = timeline_view(start, end, timeline_zoom, timeline_start);
                    self.clip.timeline_zoom = timeline_zoom;
                    self.clip.timeline_start = view_start;
                    let mut rewind = false;
                    let mut undo = false;
                    let mut redo = false;
                    timeline_action_row(
                        ui,
                        controls_width,
                        |ui| {
                            rewind = ui
                                .button(aurora_i18n::tr!("ik-graph-rewind"))
                                .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-rewind"))
                                .clicked();
                        },
                        |ui| {
                            ui.label(aurora_i18n::tr!(
                                "anim-edit-time-readout",
                                frame = self.clip.frame,
                                end = end,
                                seconds = format!("{:.3}", self.clip.frame.saturating_sub(start) as f32 / fps as f32),
                                fps = fps
                            ));
                        },
                        |ui| {
                            ui.horizontal(|ui| {
                                undo = ui
                                    .add_enabled(!self.clip.undo.is_empty(), egui::Button::new(aurora_i18n::tr!("poser-undo")))
                                    .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-undo"))
                                    .clicked();
                                redo = ui
                                    .add_enabled(!self.clip.redo.is_empty(), egui::Button::new(aurora_i18n::tr!("poser-redo")))
                                    .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-redo"))
                                    .clicked();
                            });
                        },
                    );
                    if rewind {
                        self.clip_seek(start);
                    }
                    if undo {
                        self.studio_clip_undo();
                    }
                    if redo {
                        self.studio_clip_redo();
                    }
                    let mut capture_bone = None;
                    let mut copy_keyframe = false;
                    let mut paste_keyframe = false;
                    timeline_action_row(
                        ui,
                        controls_width,
                        |ui| {
                            if let Some(bone) = self.selected {
                                if ui.button(aurora_i18n::tr!("anim-edit-key-bone")).clicked() {
                                    capture_bone = Some(bone);
                                }
                            } else {
                                ui.add_enabled(false, egui::Button::new(aurora_i18n::tr!("anim-edit-key-bone")));
                            }
                        },
                        |ui| {
                            copy_keyframe = ui
                                .button(aurora_i18n::tr!("anim-edit-copy-keyframe"))
                                .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-copy"))
                                .clicked();
                        },
                        |ui| {
                            paste_keyframe = ui
                                .add_enabled(
                                    !self.clip.clipboard.is_empty(),
                                    egui::Button::new(aurora_i18n::tr!("anim-edit-paste-keyframe")),
                                )
                                .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-paste"))
                                .clicked();
                        },
                    );
                    if let Some(bone) = capture_bone {
                        self.clip.capture = Some(Capture::Bone(bone));
                    }
                    if copy_keyframe {
                        self.studio_copy_clip_keyframe();
                    }
                    if paste_keyframe {
                        self.studio_paste_clip_keyframe();
                    }
                    let mut delete_bone_key = false;
                    let mut delete_frame_keys = false;
                    timeline_action_row(
                        ui,
                        controls_width,
                        |ui| {
                            delete_bone_key = ui
                                .add_enabled(
                                    frame_has_selected_key,
                                    egui::Button::new(aurora_i18n::tr!("anim-edit-delete-bone-key")),
                                )
                                .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-delete"))
                                .clicked();
                        },
                        |_ui| {},
                        |ui| {
                            delete_frame_keys = ui
                                .add_enabled(frame_has_any_key, egui::Button::new(aurora_i18n::tr!("anim-edit-delete-keyframe")))
                                .clicked();
                        },
                    );
                    if delete_bone_key {
                        self.studio_delete_selected_clip_bone_keyframe();
                    }
                    if delete_frame_keys {
                        self.studio_delete_clip_keyframe();
                    }
                    let mut auto_keyframe = self.clip.document.as_ref().is_some_and(|document| document.auto_keyframe);
                    let mut auto_key_advance_seconds = self
                        .clip
                        .document
                        .as_ref()
                        .map_or(0.5, |document| document.auto_key_advance_seconds);
                    let (auto_key_changed, advance_changed) = if controls_width < 520.0 {
                        ui.vertical(|ui| {
                            let auto_key_changed =
                                widgets::switch_labeled(ui, palette, &mut auto_keyframe, aurora_i18n::tr!("anim-edit-auto-keyframe"))
                                    .on_hover_text(aurora_i18n::tr!("anim-edit-auto-keyframe-hint"))
                                    .changed();
                            let advance_changed = ui
                                .horizontal(|ui| {
                                    auto_key_step_control(
                                        ui,
                                        auto_keyframe,
                                        &mut auto_key_advance_seconds,
                                        (controls_width - 48.0).max(80.0),
                                    )
                                })
                                .inner;
                            (auto_key_changed, advance_changed)
                        })
                        .inner
                    } else {
                        ui.horizontal(|ui| {
                            let auto_key_changed =
                                widgets::switch_labeled(ui, palette, &mut auto_keyframe, aurora_i18n::tr!("anim-edit-auto-keyframe"))
                                    .on_hover_text(aurora_i18n::tr!("anim-edit-auto-keyframe-hint"))
                                    .changed();
                            let advance_changed = auto_key_step_control(ui, auto_keyframe, &mut auto_key_advance_seconds, 210.0);
                            (auto_key_changed, advance_changed)
                        })
                        .inner
                    };
                    if auto_key_changed || advance_changed {
                        self.clip.checkpoint();
                        if advance_changed
                            && let Some(document) = self.clip.document.as_ref()
                            && document.tracks.iter().any(|track| {
                                track
                                    .motion
                                    .rot_keys
                                    .iter()
                                    .any(|key| key.time > document.start as f32 / document.fps as f32)
                                    || track
                                        .motion
                                        .pos_keys
                                        .iter()
                                        .any(|key| key.time > document.start as f32 / document.fps as f32)
                            })
                        {
                            self.clip.auto_key_retime_from.get_or_insert(document.auto_key_advance_seconds);
                        }
                        if let Some(document) = self.clip.document.as_mut() {
                            let document = Arc::make_mut(document);
                            document.auto_keyframe = auto_keyframe;
                            document.auto_key_advance_seconds = auto_key_advance_seconds.clamp(0.1, 5.0);
                            self.clip.changed();
                        }
                    }
                    if let Some(old_step) = self.clip.auto_key_retime_from {
                        let new_step = self
                            .clip
                            .document
                            .as_ref()
                            .map_or(old_step, |document| document.auto_key_advance_seconds);
                        if ui
                            .button(aurora_i18n::tr!("anim-edit-auto-key-retime"))
                            .on_hover_text(aurora_i18n::tr!("anim-edit-auto-key-retime-hint"))
                            .clicked()
                        {
                            self.clip.checkpoint();
                            if let Some(document) = self.clip.document.as_mut() {
                                if Arc::make_mut(document).retime_keys_to_step(old_step, new_step) {
                                    self.clip.auto_key_retime_from = None;
                                    self.clip.error = None;
                                    self.clip.changed();
                                } else {
                                    self.clip.error = Some(aurora_i18n::text("anim-edit-auto-key-retime-limit").to_owned());
                                }
                            }
                        }
                    }
                    if let Some(error) = &self.clip.error {
                        ui.colored_label(palette.muted, error);
                    }
                    let mut load_file = false;
                    let mut load_project = false;
                    let mut save_project = false;
                    timeline_action_row(
                        ui,
                        controls_width,
                        |ui| {
                            if ui
                                .add_enabled(!self.clip.busy, egui::Button::new(aurora_i18n::tr!("anim-edit-open-file")))
                                .clicked()
                            {
                                load_file = true;
                            }
                        },
                        |ui| {
                            if ui
                                .add_enabled(!self.clip.busy, egui::Button::new(aurora_i18n::tr!("anim-edit-open-project")))
                                .clicked()
                            {
                                load_project = true;
                            }
                        },
                        |ui| {
                            if ui
                                .add_enabled(
                                    !self.clip.busy && self.clip.document.is_some(),
                                    egui::Button::new(aurora_i18n::tr!("anim-edit-save-project")),
                                )
                                .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-save"))
                                .clicked()
                            {
                                save_project = true;
                            }
                        },
                    );
                    if load_file {
                        self.queue_studio_file_action(files::Action::Load);
                    } else if load_project {
                        self.queue_studio_file_action(files::Action::LoadProject);
                    } else if save_project {
                        self.queue_studio_file_action(files::Action::SaveProject);
                    }
                    let mut frame = self.clip.frame.clamp(start, end);
                    if ui
                        .add_sized(
                            egui::vec2(ui.available_width(), ui.spacing().interact_size.y),
                            egui::Slider::new(&mut frame, start..=end).show_value(false),
                        )
                        .changed()
                    {
                        self.clip_seek(frame);
                    }
                    for track in tracks
                        .iter()
                        .filter(|track| track.authored || track.keys.len() > 1 || selected_bone == Some(track.bone))
                    {
                        let (bone, name, enabled, keys) = (track.bone, &track.name, track.enabled, &track.keys);
                        ui.horizontal(|ui| {
                            let mut enabled = enabled;
                            if widgets::checkbox(ui, palette, &mut enabled, "").changed() {
                                self.clip.checkpoint();
                                if let Some(document) = self.clip.document.as_mut()
                                    && let Some(track) = Arc::make_mut(document).tracks.iter_mut().find(|track| track.bone == bone)
                                {
                                    track.enabled = enabled;
                                    track.authored |= enabled;
                                }
                                self.clip.changed();
                            }
                            if ui
                                .add(egui::Button::selectable(self.selected == Some(bone), name).truncate())
                                .clicked()
                                && let Some(frame) = frame_snapshot.as_ref()
                            {
                                self.select(bone, &world.avatar_lib.rig, frame);
                            }
                            let width = (ui.available_width() - 4.0).max(120.0);
                            let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 18.0), egui::Sense::click_and_drag());
                            let x_at = |key: u32| {
                                rect.left() + rect.width() * (key.saturating_sub(view_start) as f32 / (view_end - view_start).max(1) as f32)
                            };
                            let hovered_key = response.hover_pos().and_then(|pointer| {
                                keys.iter()
                                    .copied()
                                    .filter(|key| *key >= view_start && *key <= view_end)
                                    .min_by(|a, b| (x_at(*a) - pointer.x).abs().total_cmp(&(x_at(*b) - pointer.x).abs()))
                                    .filter(|key| (x_at(*key) - pointer.x).abs() <= 8.0)
                            });
                            if let Some(key) = hovered_key {
                                response
                                    .clone()
                                    .on_hover_cursor(if response.dragged() {
                                        egui::CursorIcon::Grabbing
                                    } else {
                                        egui::CursorIcon::Grab
                                    })
                                    .on_hover_text(aurora_i18n::tr!("anim-edit-keyframe-at", frame = key));
                            } else if response.dragged() && self.clip.drag_key.is_some_and(|(dragged_bone, _, _)| dragged_bone == bone) {
                                response.clone().on_hover_cursor(egui::CursorIcon::Grabbing);
                            }
                            ui.painter()
                                .line_segment([rect.left_center(), rect.right_center()], egui::Stroke::new(1.0, palette.muted_dim));
                            if (view_start..=view_end).contains(&self.clip.frame) {
                                let x = x_at(self.clip.frame);
                                ui.painter().line_segment(
                                    [egui::pos2(x, rect.top()), egui::pos2(x, rect.bottom())],
                                    egui::Stroke::new(1.0, palette.teal),
                                );
                            }
                            let moving_key = self.clip.drag_key.filter(|(dragged_bone, _, _)| *dragged_bone == bone);
                            for key in keys.iter().copied().filter(|key| *key >= view_start && *key <= view_end) {
                                if moving_key.is_some_and(|(_, origin, _)| origin == key) {
                                    continue;
                                }
                                let x = x_at(key);
                                ui.painter().line_segment(
                                    [egui::pos2(x, rect.top() + 2.0), egui::pos2(x, rect.bottom() - 2.0)],
                                    egui::Stroke::new(
                                        if Some(key) == hovered_key { 3.0 } else { 2.0 },
                                        if Some(key) == hovered_key || key == self.clip.frame {
                                            palette.violet_light
                                        } else {
                                            palette.violet
                                        },
                                    ),
                                );
                                if Some(key) == hovered_key {
                                    ui.painter()
                                        .circle_filled(egui::pos2(x, rect.center().y), 3.5, palette.violet_light);
                                }
                            }
                            if let Some((_, _, moving_to)) = moving_key {
                                let x = x_at(moving_to);
                                ui.painter().line_segment(
                                    [egui::pos2(x, rect.top() + 2.0), egui::pos2(x, rect.bottom() - 2.0)],
                                    egui::Stroke::new(2.0, palette.teal),
                                );
                            }
                            if response.clicked()
                                && let Some(pointer) = response.interact_pointer_pos()
                            {
                                let at = view_start
                                    + (((pointer.x - rect.left()) / rect.width().max(1.0)) * (view_end - view_start) as f32).round() as u32;
                                self.clip_seek(at.clamp(view_start, view_end));
                            }
                            if response.drag_started()
                                && let Some(pointer) = response.interact_pointer_pos()
                            {
                                let key = keys
                                    .iter()
                                    .copied()
                                    .filter(|key| *key >= view_start && *key <= view_end)
                                    .min_by(|a, b| (x_at(*a) - pointer.x).abs().total_cmp(&(x_at(*b) - pointer.x).abs()))
                                    .filter(|key| (x_at(*key) - pointer.x).abs() <= 8.0);
                                self.clip.drag_key = key.map(|key| (bone, key, key));
                                if let Some(key) = key {
                                    self.clip_seek(key);
                                }
                            }
                            if response.dragged()
                                && let Some(pointer) = response.interact_pointer_pos()
                            {
                                let to = view_start
                                    + (((pointer.x - rect.left()) / rect.width().max(1.0)) * (view_end - view_start) as f32).round() as u32;
                                let to = to.clamp(view_start, view_end);
                                match self.clip.drag_key {
                                    Some((dragged_bone, origin, current)) if dragged_bone == bone && current != to => {
                                        self.clip.drag_key = Some((bone, origin, to));
                                        self.clip_seek(to);
                                    }
                                    None => self.clip_seek(to),
                                    _ => {}
                                }
                            }
                            if response.drag_stopped()
                                && let (Some((dragged_bone, from, _)), Some(pointer)) =
                                    (self.clip.drag_key.take(), response.interact_pointer_pos())
                            {
                                let to = view_start
                                    + (((pointer.x - rect.left()) / rect.width().max(1.0)) * (view_end - view_start) as f32).round() as u32;
                                let to = to.clamp(view_start, view_end);
                                if dragged_bone == bone && from != to {
                                    self.clip.checkpoint();
                                    if let Some(document) = self.clip.document.as_mut() {
                                        Arc::make_mut(document).move_track_keyframe(bone, from, to);
                                        self.clip.changed();
                                    }
                                    self.clip_seek(to);
                                }
                            }
                        });
                    }
                });
        });
        self.clip.timeline_open = open;
    }

    pub(in crate::ik_edit) fn clip_entry_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        if !self.clip.history.is_empty() {
            let current = self.clip.document.as_ref().map_or_else(String::new, |d| d.name.clone());
            let mut choice = None;
            egui::ComboBox::from_id_salt("animation-drafts")
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for (index, draft) in self.clip.history.iter().enumerate() {
                        if ui.selectable_label(false, format!("{} · {}", index + 1, draft.name)).clicked() {
                            choice = Some(index);
                        }
                    }
                });
            if let Some(index) = choice {
                if let Some(current) = self.clip.document.as_mut() {
                    std::mem::swap(current, &mut self.clip.history[index]);
                } else {
                    self.clip.document = Some(self.clip.history.remove(index));
                }
                self.clip.clear_undo_history();
                self.clip.auto_key_retime_from = None;
                let at = self.clip.document.as_ref().map_or(0, |d| d.start);
                self.clip.seek(at);
                self.clip.changed();
            }
        }
        ui.horizontal_wrapped(|ui| {
            if self.clip.document.is_some() && ui.button(aurora_i18n::tr!("anim-edit-resume")).clicked() {
                self.enter_clip(world, frame);
            }
            if ui
                .add_enabled(
                    self.clip.history.len() < 15,
                    egui::Button::new(aurora_i18n::tr!("anim-edit-create")),
                )
                .on_hover_text(aurora_i18n::tr!("anim-edit-drafts-hint"))
                .clicked()
            {
                self.new_clip(world, frame);
            }
            if ui
                .add_enabled(!self.clip.busy, egui::Button::new(aurora_i18n::tr!("anim-edit-open-file")))
                .clicked()
            {
                self.queue_studio_file_action(files::Action::Load);
            }
            if ui
                .add_enabled(!self.clip.busy, egui::Button::new(aurora_i18n::tr!("anim-edit-open-project")))
                .clicked()
            {
                self.queue_studio_file_action(files::Action::LoadProject);
            }
            if ui
                .add_enabled(
                    !self.clip.busy && self.clip.document.is_some(),
                    egui::Button::new(aurora_i18n::tr!("anim-edit-save-project")),
                )
                .on_hover_text(aurora_i18n::tr!("anim-edit-shortcut-save"))
                .clicked()
            {
                self.queue_studio_file_action(files::Action::SaveProject);
            }
        });
        if self.clip.active {
            return;
        }
        if self.clip.document.is_some() {
            ui.horizontal_wrapped(|ui| {
                ui.label(aurora_i18n::tr!("anim-edit-target-frame"));
                let mut at = self.clip.frame;
                let max_frame = self.clip.document.as_ref().map_or(MAX_FRAME, |document| document.max_frame());
                if ui.add(egui::DragValue::new(&mut at).range(0..=max_frame)).changed() {
                    if let Some(document) = self.clip.document.as_mut()
                        && (at < document.start || at > document.end)
                    {
                        let d = Arc::make_mut(document);
                        d.range(d.start.min(at), d.end.max(at));
                        self.clip.changed();
                    }
                    self.clip.seek(at);
                }
                if ui.button(aurora_i18n::tr!("anim-edit-capture-ik")).clicked() {
                    self.capture_ik_clip(world, frame);
                }
            });
        }
        if let Some(error) = &self.clip.error {
            ui.colored_label(palette.muted, error);
        }
    }
    pub(super) fn clip_seek(&mut self, frame: u32) {
        let mut frame = frame.min(MAX_FRAME);
        if let Some(document) = &mut self.clip.document {
            frame = frame.min(document.max_frame());
            if frame < document.start || frame > document.end {
                let doc = Arc::make_mut(document);
                doc.range(doc.start.min(frame), doc.end.max(frame));
                self.clip.changed();
            }
        }
        self.clip.seek(frame);
        if let Some(document) = self.clip.document.as_ref() {
            self.clip.timeline_start = reveal_timeline_frame(
                document.start,
                document.end,
                self.clip.timeline_zoom,
                self.clip.timeline_start,
                frame,
            );
        }
        self.goals.clear();
        self.authoring = authoring::State::default();
        self.disabled.clear();
        self.control = Control::Bone;
        self.drag = None;
        self.gizmo.cancel();
        self.refresh = true;
    }
    pub(in crate::ik_edit) fn clip_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        let Some((start, end, mut name)) = self
            .clip
            .document
            .as_ref()
            .map(|document| (document.start, document.end, document.name.clone()))
        else {
            return;
        };
        let tracks = self.clip.timeline_tracks().unwrap_or_default();
        let keys = self
            .selected
            .and_then(|bone| tracks.iter().find(|track| track.bone == bone))
            .map_or(&[][..], |track| track.keys.as_slice());
        if ui
            .add(
                egui::TextEdit::singleline(&mut name)
                    .char_limit(100)
                    .hint_text(aurora_i18n::tr!("anim-edit-name")),
            )
            .changed()
            && let Some(d) = self.clip.document.as_mut()
        {
            Arc::make_mut(d).name = name;
        }
        let mut tab = self.clip.tab as usize;
        widgets::tabs(
            ui,
            palette,
            &mut tab,
            &[
                (aurora_i18n::tr!("anim-edit-timeline"), true),
                (aurora_i18n::tr!("anim-edit-tracks"), true),
                (aurora_i18n::tr!("anim-edit-settings"), true),
            ],
        );
        self.clip.tab = tab as u8;
        if self.clip.tab == 1 {
            self.clip_tracks_ui(ui, palette, world, frame);
            return;
        }
        if self.clip.tab == 2 {
            self.clip_settings_ui(ui, palette);
            return;
        }
        ui.label(aurora_i18n::tr!("anim-edit-workflow"));
        ui.horizontal(|ui| {
            if ui
                .button(if self.clip.playing {
                    aurora_i18n::tr!("ik-graph-pause")
                } else {
                    aurora_i18n::tr!("ik-graph-play")
                })
                .clicked()
            {
                self.studio_toggle_clip_playback();
            }
            if ui.button(aurora_i18n::tr!("ik-graph-rewind")).clicked() {
                self.clip_seek(start);
            }
        });
        let mut value = self.clip.frame;
        ui.label(aurora_i18n::tr!("anim-edit-frame"));
        let mut seek = false;
        ui.horizontal(|ui| {
            seek |= ui.add(egui::Slider::new(&mut value, start..=end).show_value(false)).changed();
            seek |= ui.add(egui::DragValue::new(&mut value).range(start..=end).speed(1)).changed();
            let fps = self.clip.document.as_ref().map_or(FPS as u32, |document| document.fps);
            ui.label(aurora_i18n::tr!(
                "anim-edit-seconds",
                seconds = format!("{:.3}", value.saturating_sub(start) as f32 / fps as f32)
            ));
        });
        if seek {
            self.clip_seek(value);
        }
        ui.horizontal(|ui| {
            if ui.button(aurora_i18n::tr!("anim-edit-prev-key")).clicked()
                && let Some(frame) = keys.iter().copied().rev().find(|f| *f < self.clip.frame && *f >= start)
            {
                self.clip_seek(frame);
            }
            if ui.button(aurora_i18n::tr!("anim-edit-next-key")).clicked()
                && let Some(frame) = keys.iter().copied().find(|f| *f > self.clip.frame && *f <= end)
            {
                self.clip_seek(frame);
            }
        });
        if let Some(bone) = self.selected {
            ui.strong(&world.avatar_lib.rig.canonical_names[bone]);
        }
        if let Some(bone) = self.selected
            && ui
                .add_enabled(
                    keys.contains(&self.clip.frame),
                    egui::Button::new(aurora_i18n::tr!("anim-edit-remove-key")),
                )
                .clicked()
        {
            self.clip.checkpoint();
            if let Some(document) = self.clip.document.as_mut() {
                Arc::make_mut(document).remove(bone, self.clip.frame);
            }
            self.clip.changed();
            self.goals.clear();
            self.authoring = authoring::State::default();
        }
        ui.label(aurora_i18n::tr!("anim-edit-key-list"));
        ui.horizontal_wrapped(|ui| {
            let visible_start = keys.partition_point(|key| *key < start);
            let visible_end = keys.partition_point(|key| *key <= end);
            let visible = &keys[visible_start..visible_end];
            let pivot = visible.partition_point(|k| *k < self.clip.frame);
            let first = pivot.saturating_sub(12);
            for &key in visible.iter().skip(first).take(24) {
                if ui.selectable_label(self.clip.frame == key, key.to_string()).clicked() {
                    self.clip_seek(key);
                }
            }
        });
        if self.clip.building {
            ui.colored_label(palette.muted, aurora_i18n::tr!("anim-edit-building"));
        }
    }
    pub(super) fn clip_capture_ui(&mut self, ui: &mut egui::Ui) {
        ui.add_enabled_ui(!self.clip.waiting_source(), |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add_enabled(self.selected.is_some(), egui::Button::new(aurora_i18n::tr!("anim-edit-key-bone")))
                    .clicked()
                    && let Some(bone) = self.selected
                {
                    self.clip.capture = Some(Capture::Bone(bone));
                }
                if ui
                    .add_enabled(
                        !self.clip.modified.is_empty(),
                        egui::Button::new(aurora_i18n::tr!("anim-edit-key-changes")),
                    )
                    .clicked()
                {
                    self.clip.capture = Some(Capture::Modified);
                }
                if ui.button(aurora_i18n::tr!("anim-edit-key-pose")).clicked() {
                    self.clip.capture = Some(Capture::Pose);
                }
            });
        });
    }
    fn clip_tracks_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        let Some(document) = &self.clip.document else { return };
        ui.label(aurora_i18n::tr!("anim-edit-priority-hint"));
        let mut changes = Vec::new();
        let mut selected = None;
        for (index, track) in document
            .tracks
            .iter()
            .enumerate()
            .filter(|(_, track)| track.authored || !track.enabled || Some(track.bone) == self.selected)
        {
            ui.push_id(("native-track", track.bone), |ui| {
                ui.horizontal(|ui| {
                    let mut enabled = track.enabled;
                    let mut priority = track.motion.priority;
                    let mut interpolation = track.interpolation;
                    let curve = track.curve;
                    let enabled_response = widgets::checkbox(ui, palette, &mut enabled, "");
                    let mut changed = enabled_response.changed();
                    let mut checkpoint = checkpoint_on_edit(&enabled_response);
                    if ui
                        .add(egui::Button::selectable(self.selected == Some(track.bone), &track.motion.joint_name).truncate())
                        .clicked()
                    {
                        selected = Some(track.bone);
                    }
                    let priority_response = ui.add(egui::DragValue::new(&mut priority).range(-1..=6).speed(1));
                    changed |= priority_response.changed();
                    checkpoint |= checkpoint_on_edit(&priority_response);
                    egui::ComboBox::from_id_salt(("native-track-interpolation", track.bone))
                        .selected_text(match interpolation {
                            document::Interpolation::Smooth => aurora_i18n::tr!("anim-edit-interpolation-smooth"),
                            document::Interpolation::Linear => aurora_i18n::tr!("anim-edit-interpolation-linear"),
                            document::Interpolation::Stepped => aurora_i18n::tr!("anim-edit-interpolation-stepped"),
                            document::Interpolation::Curve => aurora_i18n::tr!("anim-edit-interpolation-curve"),
                        })
                        .show_ui(ui, |ui| {
                            let response = ui.selectable_value(
                                &mut interpolation,
                                document::Interpolation::Smooth,
                                aurora_i18n::tr!("anim-edit-interpolation-smooth"),
                            );
                            changed |= response.changed();
                            checkpoint |= checkpoint_on_edit(&response);
                            let response = ui.selectable_value(
                                &mut interpolation,
                                document::Interpolation::Linear,
                                aurora_i18n::tr!("anim-edit-interpolation-linear"),
                            );
                            changed |= response.changed();
                            checkpoint |= checkpoint_on_edit(&response);
                            let response = ui.selectable_value(
                                &mut interpolation,
                                document::Interpolation::Stepped,
                                aurora_i18n::tr!("anim-edit-interpolation-stepped"),
                            );
                            changed |= response.changed();
                            checkpoint |= checkpoint_on_edit(&response);
                            let response = ui.selectable_value(
                                &mut interpolation,
                                document::Interpolation::Curve,
                                aurora_i18n::tr!("anim-edit-interpolation-curve"),
                            );
                            changed |= response.changed();
                            checkpoint |= checkpoint_on_edit(&response);
                        });
                    if changed {
                        let authored = track.authored
                            || (enabled && changed)
                            || priority != track.motion.priority
                            || interpolation != track.interpolation;
                        changes.push((index, enabled, authored, priority, interpolation, curve, checkpoint));
                    }
                })
            });
        }
        let mut curve_changes = Vec::new();
        if let Some((index, track)) = document
            .tracks
            .iter()
            .enumerate()
            .find(|(_, track)| Some(track.bone) == self.selected && track.interpolation == document::Interpolation::Curve)
        {
            ui.add_space(6.0);
            ui.label(aurora_i18n::tr!("anim-edit-curve-editor", bone = track.motion.joint_name.as_str()));
            if let Some((curve, checkpoint)) = Self::curve_editor_ui(ui, palette, track.curve) {
                curve_changes.push((index, curve, checkpoint));
            }
        }
        if !changes.is_empty() {
            if changes.iter().any(|(_, _, _, _, _, _, checkpoint)| *checkpoint) {
                self.clip.checkpoint();
            }
            if let Some(document) = self.clip.document.as_mut() {
                let document = Arc::make_mut(document);
                for (index, enabled, authored, priority, interpolation, curve, _) in changes {
                    document.tracks[index].enabled = enabled;
                    document.tracks[index].authored = authored;
                    document.tracks[index].motion.priority = priority;
                    document.tracks[index].interpolation = interpolation;
                    document.tracks[index].curve = curve;
                }
                self.clip.changed();
            }
        }
        if !curve_changes.is_empty() {
            if curve_changes.iter().any(|(_, _, checkpoint)| *checkpoint) {
                self.clip.checkpoint();
            }
            if let Some(document) = self.clip.document.as_mut() {
                let document = Arc::make_mut(document);
                for (index, curve, _) in curve_changes {
                    document.tracks[index].curve = curve;
                    document.tracks[index].authored = true;
                }
                self.clip.changed();
            }
        }
        if let Some(bone) = selected {
            self.select(bone, &world.avatar_lib.rig, frame);
        }
    }

    fn curve_editor_ui(ui: &mut egui::Ui, palette: &Palette, mut curve: document::Curve) -> Option<(document::Curve, bool)> {
        let before = curve;
        let mut checkpoint = false;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(190.0, 112.0), egui::Sense::hover());
        let plot = rect.shrink(10.0);
        let point = |time: f32, value: f32| egui::pos2(plot.left() + time * plot.width(), plot.bottom() - value * plot.height());
        let p0 = point(0.0, 0.0);
        let p3 = point(1.0, 1.0);
        let mut control_points = [point(1.0 / 3.0, curve.start), point(2.0 / 3.0, curve.end)];
        let painter = ui.painter();
        painter.rect_stroke(
            plot,
            ui.visuals().widgets.noninteractive.corner_radius,
            egui::Stroke::new(1.0, palette.muted_dim),
            egui::StrokeKind::Inside,
        );
        painter.line_segment(
            [egui::pos2(plot.left(), plot.center().y), egui::pos2(plot.right(), plot.center().y)],
            egui::Stroke::new(1.0, palette.muted_dim),
        );
        painter.line_segment(
            [egui::pos2(plot.center().x, plot.top()), egui::pos2(plot.center().x, plot.bottom())],
            egui::Stroke::new(1.0, palette.muted_dim),
        );
        for (index, control_point) in control_points.iter().enumerate() {
            let response = ui.interact(
                egui::Rect::from_center_size(*control_point, egui::Vec2::splat(14.0)),
                ui.scope_id().with(("animation-curve-control", index)),
                egui::Sense::drag(),
            );
            checkpoint |= response.drag_started();
            if response.dragged()
                && let Some(pointer) = response.interact_pointer_pos()
            {
                let value = ((plot.bottom() - pointer.y) / plot.height()).clamp(0.0, 1.0);
                if index == 0 {
                    curve.start = value.min(curve.end);
                } else {
                    curve.end = value.max(curve.start);
                }
            }
        }
        curve = curve.normalized();
        control_points = [point(1.0 / 3.0, curve.start), point(2.0 / 3.0, curve.end)];
        let mut samples = Vec::with_capacity(33);
        for step in 0..=32 {
            let t = step as f32 / 32.0;
            let inverse = 1.0 - t;
            let value = 3.0 * inverse * inverse * t * curve.start + 3.0 * inverse * t * t * curve.end + t * t * t;
            samples.push(point(t, value));
        }
        let painter = ui.painter();
        painter.line_segment([p0, control_points[0]], egui::Stroke::new(1.0, palette.teal));
        painter.line_segment([p3, control_points[1]], egui::Stroke::new(1.0, palette.teal));
        painter.add(egui::Shape::line(samples, egui::Stroke::new(2.0, palette.violet_light)));
        for control in control_points {
            painter.circle_filled(control, 4.0, palette.teal);
        }
        let mut start_response = None;
        let mut end_response = None;
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("anim-edit-curve-start"));
            start_response = Some(ui.add(egui::DragValue::new(&mut curve.start).range(0.0..=curve.end).speed(0.01)));
            ui.label(aurora_i18n::tr!("anim-edit-curve-end"));
            end_response = Some(ui.add(egui::DragValue::new(&mut curve.end).range(curve.start..=1.0).speed(0.01)));
        });
        for response in [start_response, end_response].into_iter().flatten() {
            checkpoint |= checkpoint_on_edit(&response);
        }
        (curve != before).then_some((curve, checkpoint))
    }
    fn clip_settings_ui(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let Some(document) = &self.clip.document else { return };
        let mut meta = document.meta.clone();
        let mut start = document.start;
        let mut end = document.end;
        let mut fps = document.fps;
        let mut changed = false;
        let mut checkpoint_needed = false;
        let old_fps = document.fps;
        let mut fps_changed = false;
        let mut start_changed = false;
        let mut end_changed = false;
        egui::Grid::new("native-animation-range").num_columns(2).show(ui, |ui| {
            ui.label(aurora_i18n::tr!("anim-edit-fps"));
            let response = ui.add(egui::DragValue::new(&mut fps).range(1..=60));
            fps_changed = response.changed();
            checkpoint_needed |= checkpoint_on_edit(&response);
            changed |= fps_changed;
            ui.end_row();
            ui.label(aurora_i18n::tr!("anim-edit-start"));
            let max_frame = document::MAX_DURATION as u32 * fps;
            let response = ui.add(egui::DragValue::new(&mut start).range(0..=max_frame - 1));
            start_changed = response.changed();
            checkpoint_needed |= checkpoint_on_edit(&response);
            changed |= start_changed;
            ui.end_row();
            ui.label(aurora_i18n::tr!("anim-edit-end"));
            let response = ui.add(egui::DragValue::new(&mut end).range(start + 1..=max_frame));
            end_changed = response.changed();
            checkpoint_needed |= checkpoint_on_edit(&response);
            changed |= end_changed;
            ui.end_row();
            ui.label(aurora_i18n::tr!("import-priority"));
            let response = ui.add(egui::DragValue::new(&mut meta.base_priority).range(0..=6));
            changed |= response.changed();
            checkpoint_needed |= checkpoint_on_edit(&response);
            ui.end_row();
        });
        if fps_changed {
            let scale = fps as f32 / old_fps as f32;
            if !start_changed {
                start = (start as f32 * scale).round() as u32;
            }
            if !end_changed {
                end = (end as f32 * scale).round() as u32;
            }
        }
        ui.label(aurora_i18n::tr!(
            "anim-edit-duration",
            seconds = format!("{:.3}", end.saturating_sub(start) as f32 / fps as f32)
        ));
        let response = widgets::switch_labeled(ui, palette, &mut meta.looping, aurora_i18n::tr!("import-loop"));
        changed |= response.changed();
        checkpoint_needed |= checkpoint_on_edit(&response);
        let duration = end.saturating_sub(start) as f32 / fps as f32;
        egui::Grid::new("native-animation-timing").num_columns(2).show(ui, |ui| {
            for (key, value) in [
                ("import-loop-in", &mut meta.loop_in),
                ("import-loop-out", &mut meta.loop_out),
                ("import-ease-in", &mut meta.ease_in),
                ("import-ease-out", &mut meta.ease_out),
            ] {
                ui.label(aurora_i18n::text(key));
                let response = ui.add(
                    egui::DragValue::new(value)
                        .range(0.0..=duration)
                        .speed(1.0 / f64::from(fps))
                        .suffix(" s"),
                );
                changed |= response.changed();
                checkpoint_needed |= checkpoint_on_edit(&response);
                ui.end_row();
            }
        });
        ui.label(aurora_i18n::tr!("import-animation-hands"));
        let hands = crate::imports::animation::hands();
        egui::ComboBox::from_id_salt("native-hand-pose")
            .selected_text(hands[(meta.hand_pose as usize).min(13)])
            .show_ui(ui, |ui| {
                for (index, label) in hands.iter().enumerate() {
                    let response = ui.selectable_value(&mut meta.hand_pose, index as u32, *label);
                    changed |= response.changed();
                    checkpoint_needed |= checkpoint_on_edit(&response);
                }
            });
        ui.label(aurora_i18n::tr!("import-animation-expression"));
        let emotes = crate::imports::animation::emotes();
        egui::ComboBox::from_id_salt("native-emote")
            .selected_text(
                emotes
                    .iter()
                    .find(|(name, _, _)| *name == meta.emote_name)
                    .map_or(aurora_i18n::tr!("import-emote-none"), |(_, _, label)| *label),
            )
            .show_ui(ui, |ui| {
                for (name, _, label) in emotes {
                    let response = ui.selectable_value(&mut meta.emote_name, name.to_owned(), label);
                    changed |= response.changed();
                    checkpoint_needed |= checkpoint_on_edit(&response);
                }
            });
        ui.label(aurora_i18n::tr!("anim-edit-range-hint"));
        if changed {
            if checkpoint_needed {
                self.clip.checkpoint();
            }
            if let Some(document) = self.clip.document.as_mut() {
                let document = Arc::make_mut(document);
                document.meta = meta;
                if fps_changed {
                    document.set_fps(fps);
                    if !start_changed && !end_changed {
                        self.clip.frame = (self.clip.frame as f32 * fps as f32 / old_fps as f32).round() as u32;
                    }
                }
                if start_changed || end_changed {
                    document.range(start, end);
                }
                document.timing();
                self.clip.frame = self.clip.frame.clamp(document.start, document.end);
                self.clip.cursor = f64::from(self.clip.frame);
                self.clip.playing = false;
                self.clip.changed();
                self.goals.clear();
                self.authoring = authoring::State::default();
                self.refresh = true;
            }
        }
    }
    pub(in crate::ik_edit) fn clip_export_ui(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        self.clip_capture_ui(ui);
        let enabled = !self.clip.busy
            && !self.clip.waiting_source()
            && self.clip.document.as_ref().is_some_and(|document| document.has_exportable_tracks());
        ui.horizontal(|ui| {
            if ui
                .add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("anim-edit-save")))
                .clicked()
            {
                let last_key_only = self.clip.timeline_looping && self.clip.timeline_loop_last_key;
                self.queue_studio_file_action(files::Action::Save { last_key_only });
            }
            if ui
                .add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("anim-edit-upload")))
                .on_hover_text(aurora_i18n::tr!("anim-edit-upload-hint"))
                .clicked()
            {
                let last_key_only = self.clip.timeline_looping && self.clip.timeline_loop_last_key;
                self.queue_studio_file_action(files::Action::Import { last_key_only });
            }
        });
        if self.clip.busy {
            ui.label(aurora_i18n::tr!("anim-edit-preparing"));
        }
        if let Some(message) = &self.clip.message {
            ui.add(egui::Label::new(message).truncate()).on_hover_text(message);
        }
        if let Some(error) = &self.clip.error {
            ui.add(egui::Label::new(egui::RichText::new(error).color(palette.muted)).truncate())
                .on_hover_text(error);
        }
    }
}
