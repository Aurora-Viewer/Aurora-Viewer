//! The animation editor's panel: the name and drafts, one row of pages (Bone,
//! Tracks, Clip, Display) and the key/export footer. Playback, scrubbing and
//! key navigation live in the timeline window, never twice.
use super::*;

pub(super) fn checkpoint_on_edit(response: &egui::Response) -> bool {
    response.drag_started() || (response.changed() && !response.dragged())
}

fn interpolation_label(mode: document::Interpolation) -> &'static str {
    match mode {
        document::Interpolation::Smooth => aurora_i18n::tr!("anim-edit-interpolation-smooth"),
        document::Interpolation::Linear => aurora_i18n::tr!("anim-edit-interpolation-linear"),
        document::Interpolation::Stepped => aurora_i18n::tr!("anim-edit-interpolation-stepped"),
        document::Interpolation::Curve => aurora_i18n::tr!("anim-edit-interpolation-curve"),
    }
}

/// The interpolation choice of one track: whether it changed, and whether
/// that change starts an undo step.
fn interpolation_combo(ui: &mut egui::Ui, id_salt: impl egui::AsIdSalt, interpolation: &mut document::Interpolation) -> (bool, bool) {
    let mut changed = false;
    let mut checkpoint = false;
    egui::ComboBox::from_id_salt(id_salt)
        .selected_text(interpolation_label(*interpolation))
        .show_ui(ui, |ui| {
            for mode in [
                document::Interpolation::Smooth,
                document::Interpolation::Linear,
                document::Interpolation::Stepped,
                document::Interpolation::Curve,
            ] {
                let response = ui.selectable_value(interpolation, mode, interpolation_label(mode));
                changed |= response.changed();
                checkpoint |= checkpoint_on_edit(&response);
            }
        });
    (changed, checkpoint)
}

impl Editor {
    /// The draft selector. Choosing a draft swaps it with the current one and
    /// starts it at its first frame.
    fn drafts_ui(&mut self, ui: &mut egui::Ui, selected_text: String, width: Option<f32>) {
        let mut choice = None;
        let mut combo = egui::ComboBox::from_id_salt("animation-drafts").selected_text(selected_text);
        if let Some(width) = width {
            combo = combo.width(width);
        }
        combo
            .show_ui(ui, |ui| {
                for (index, draft) in self.clip.history.iter().enumerate() {
                    if ui.selectable_label(false, format!("{} · {}", index + 1, draft.name)).clicked() {
                        choice = Some(index);
                    }
                }
            })
            .response
            .on_hover_text(aurora_i18n::tr!("anim-edit-drafts-hint"));
        let Some(index) = choice else { return };
        if let Some(current) = self.clip.document.as_mut() {
            std::mem::swap(current, &mut self.clip.history[index]);
        } else {
            self.clip.document = Some(self.clip.history.remove(index));
        }
        self.clip.clear_undo_history();
        self.clip.auto_key_retime_from = None;
        let at = self.clip.document.as_ref().map_or(0, |d| d.start);
        if self.clip.active {
            // the staged pose edits belong to the draft that was left
            self.clip_seek(at);
        } else {
            self.clip.seek(at);
        }
        self.clip.changed();
    }
    pub(in crate::ik_edit) fn clip_entry_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        if !self.clip.history.is_empty() {
            let current = self.clip.document.as_ref().map_or_else(String::new, |d| d.name.clone());
            self.drafts_ui(ui, current, None);
        }
        ui.horizontal_wrapped(|ui| {
            // This page belongs to IK Edit: an editor entered from here is
            // not Avatar Studio's, whatever opened the previous one, and
            // gets its own timeline, Timeline button and Return.
            if self.clip.document.is_some() && ui.button(aurora_i18n::tr!("anim-edit-resume")).clicked() {
                self.clip.from_avatar_studio = false;
                self.enter_clip(world, frame);
            }
            if ui
                .add_enabled(
                    self.clip.history.len() < MAX_DRAFTS,
                    egui::Button::new(aurora_i18n::tr!("anim-edit-create")),
                )
                .on_hover_text(aurora_i18n::tr!("anim-edit-drafts-hint"))
                .clicked()
            {
                self.clip.from_avatar_studio = false;
                self.new_clip(world, frame);
            }
            // Opening is refused with the drafts full, like Create: the
            // current document would have nowhere to go.
            let can_open = self.clip.file_action_allowed(files::Action::Load);
            if ui
                .add_enabled(can_open, egui::Button::new(aurora_i18n::tr!("anim-edit-open-file")))
                .on_disabled_hover_text(aurora_i18n::tr!("anim-edit-drafts-hint"))
                .clicked()
            {
                self.queue_studio_file_action(files::Action::Load);
            }
            if ui
                .add_enabled(can_open, egui::Button::new(aurora_i18n::tr!("anim-edit-open-project")))
                .on_disabled_hover_text(aurora_i18n::tr!("anim-edit-drafts-hint"))
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
                    self.clip.from_avatar_studio = false;
                    self.capture_ik_clip(world, frame);
                }
            });
        }
        if let Some(error) = &self.clip.error {
            ui.colored_label(palette.danger, error);
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
            self.clip.timeline_start = timeline::reveal_timeline_frame(
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
    /// The animation editor's own window, narrower than the IK contacts one,
    /// and its timeline when Avatar Studio does not draw it already.
    pub(in crate::ik_edit) fn clip_window_ui(&mut self, ctx: &egui::Context, palette: &Palette, world: &World, frame: &Frame) {
        let mut open = self.open;
        let mut floater = Floater::new(
            WINDOW,
            aurora_i18n::tr!("anim-edit-title"),
            egui::pos2(24.0, 85.0),
            egui::vec2(400.0, 640.0),
        );
        floater.min_size.x = 360.0;
        floater.show(ctx, palette, &mut open, |ui| self.clip_panel_ui(ui, palette, world, frame));
        self.open = open;
        if !self.clip.from_avatar_studio {
            self.studio_timeline_overlay(ctx, palette, world);
        }
    }
    fn clip_panel_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        if self.clip_header_ui(ui, palette) {
            return;
        }
        let mut tab = ClipTab::ALL.iter().position(|tab| *tab == self.clip.tab).unwrap_or(0);
        widgets::tabs(
            ui,
            palette,
            &mut tab,
            &[
                (aurora_i18n::tr!("anim-panel-tab-bone"), true),
                (aurora_i18n::tr!("anim-panel-tab-tracks"), true),
                (aurora_i18n::tr!("anim-panel-tab-clip"), true),
                (aurora_i18n::tr!("ik-edit-tab-visible"), true),
            ],
        );
        self.clip.tab = ClipTab::ALL[tab.min(ClipTab::ALL.len() - 1)];
        let footer = if self.clip.footer_height > 0.0 {
            self.clip.footer_height + ui.spacing().item_spacing.y
        } else {
            150.0
        };
        egui::ScrollArea::vertical()
            .id_salt(("animation-editor-page", tab))
            .auto_shrink([false, true])
            .max_height((ui.available_height() - footer).max(60.0))
            .show(ui, |ui| {
                if self.clip.waiting_source() {
                    ui.label(aurora_i18n::tr!("anim-edit-building"));
                    return;
                }
                match self.clip.tab {
                    ClipTab::Bone => self.clip_bone_ui(ui, palette, world, frame),
                    ClipTab::Tracks => self.clip_tracks_ui(ui, palette, world, frame),
                    ClipTab::Clip => self.clip_settings_ui(ui, palette),
                    ClipTab::Display => self.visibility_ui(ui, palette, world, frame),
                }
            });
        // Measure once per frame so wrapped buttons and status lines reserve
        // their real height without replaying actions.
        let footer = ui.scope(|ui| self.clip_export_ui(ui, palette));
        if (self.clip.footer_height - footer.response.rect.height()).abs() > 0.5 {
            self.clip.footer_height = footer.response.rect.height();
            ui.ctx().request_repaint();
        }
    }
    /// The name with the drafts beside it, then the way back to IK contacts
    /// (not from Avatar Studio, where it means nothing) and the timeline when
    /// its window was closed. True once the editor was left.
    fn clip_header_ui(&mut self, ui: &mut egui::Ui, palette: &Palette) -> bool {
        let Some(mut name) = self.clip.document.as_ref().map(|document| document.name.clone()) else {
            return false;
        };
        let drafts = !self.clip.history.is_empty();
        let drafts_width = 120.0;
        ui.horizontal(|ui| {
            let reserved = if drafts {
                drafts_width + ui.spacing().item_spacing.x + ui.spacing().icon_width
            } else {
                0.0
            };
            let edit = egui::TextEdit::singleline(&mut name)
                .char_limit(100)
                .hint_text(aurora_i18n::tr!("anim-edit-name"))
                .desired_width((ui.available_width() - reserved).max(80.0));
            if ui.add(edit).changed()
                && let Some(document) = self.clip.document.as_mut()
            {
                Arc::make_mut(document).name = name;
            }
            if drafts {
                self.drafts_ui(ui, aurora_i18n::tr!("anim-panel-drafts").to_owned(), Some(drafts_width));
            }
        });
        let back = !self.clip.from_avatar_studio;
        let timeline = !self.clip.timeline_open;
        let mut left = false;
        if back || timeline {
            ui.horizontal(|ui| {
                if back && ui.button(aurora_i18n::tr!("anim-edit-return")).clicked() {
                    self.leave_clip();
                    left = true;
                }
                if timeline
                    && widgets::action_button(
                        ui,
                        palette,
                        "timeline",
                        aurora_i18n::tr!("anim-edit-timeline"),
                        widgets::ActionKind::Flat,
                        egui::vec2(0.0, ui.spacing().interact_size.y),
                    )
                    .on_hover_text(aurora_i18n::tr!("avatar-studio-animation-timeline-show"))
                    .clicked()
                {
                    self.clip.timeline_open = true;
                }
            });
        }
        left
    }
    /// The selected bone's track: its interpolation, priority and, for a
    /// custom curve, the curve editor. Changing them makes the track part of
    /// the export, as on the Tracks page.
    pub(in crate::ik_edit) fn clip_bone_track_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, bone: usize) {
        let Some(document) = &self.clip.document else { return };
        let Some((index, track)) = document.tracks.iter().enumerate().find(|(_, track)| track.bone == bone) else {
            ui.colored_label(palette.muted, aurora_i18n::tr!("anim-panel-no-track"));
            return;
        };
        let (old_priority, old_interpolation, authored) = (track.motion.priority, track.interpolation, track.authored);
        let mut priority = old_priority;
        let mut interpolation = old_interpolation;
        let curve = track.curve;
        let mut changed = false;
        let mut checkpoint = false;
        egui::Grid::new(("animation-bone-track", bone)).num_columns(2).show(ui, |ui| {
            ui.label(aurora_i18n::tr!("anim-panel-interpolation"));
            let (edited, undo) = interpolation_combo(ui, ("animation-bone-interpolation", bone), &mut interpolation);
            changed |= edited;
            checkpoint |= undo;
            ui.end_row();
            ui.label(aurora_i18n::tr!("import-priority"));
            let response = ui
                .add(egui::DragValue::new(&mut priority).range(-1..=6).speed(1))
                .on_hover_text(aurora_i18n::tr!("anim-edit-priority-hint"));
            changed |= response.changed();
            checkpoint |= checkpoint_on_edit(&response);
            ui.end_row();
        });
        let mut curve_change = None;
        if interpolation == document::Interpolation::Curve {
            curve_change = Self::curve_editor_ui(ui, palette, curve);
        }
        if !changed && curve_change.is_none() {
            return;
        }
        if checkpoint || curve_change.is_some_and(|(_, checkpoint)| checkpoint) {
            self.clip.checkpoint();
        }
        if let Some(document) = self.clip.document.as_mut()
            && let Some(track) = Arc::make_mut(document).tracks.get_mut(index)
        {
            track.motion.priority = priority;
            track.interpolation = interpolation;
            track.authored = authored || priority != old_priority || interpolation != old_interpolation;
            if let Some((curve, _)) = curve_change {
                track.curve = curve;
                track.authored = true;
            }
            self.clip.changed();
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
                    let (edited, undo) = interpolation_combo(ui, ("native-track-interpolation", track.bone), &mut interpolation);
                    changed |= edited;
                    checkpoint |= undo;
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
                .add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("anim-edit-export-bvh")))
                .on_hover_text(aurora_i18n::tr!("anim-edit-export-bvh-hint"))
                .clicked()
            {
                let last_key_only = self.clip.timeline_looping && self.clip.timeline_loop_last_key;
                self.queue_studio_file_action(files::Action::ExportBvh { last_key_only });
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
            ui.add(egui::Label::new(egui::RichText::new(error).color(palette.danger)).truncate())
                .on_hover_text(error);
        }
    }
}
