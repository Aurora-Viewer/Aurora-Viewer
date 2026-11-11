//! The Avatar Studio animation timeline window: transport, keys and bone tracks.
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
}
