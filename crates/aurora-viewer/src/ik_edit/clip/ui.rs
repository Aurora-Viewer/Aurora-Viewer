//! Compact creation pages: timeline, keyed bones, and native playback metadata.
use super::*;

impl Editor {
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
        });
        if self.clip.active {
            return;
        }
        if self.clip.document.is_some() {
            ui.horizontal_wrapped(|ui| {
                ui.label(aurora_i18n::tr!("anim-edit-target-frame"));
                let mut at = self.clip.frame;
                if ui.add(egui::DragValue::new(&mut at).range(0..=LAST)).changed() {
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
        if let Some(document) = &mut self.clip.document
            && (frame < document.start || frame > document.end)
        {
            let doc = Arc::make_mut(document);
            doc.range(doc.start.min(frame), doc.end.max(frame));
            self.clip.changed();
        }
        self.clip.seek(frame);
        self.goals.clear();
        self.authoring = authoring::State::default();
        self.disabled.clear();
        self.control = Control::Bone;
        self.drag = None;
        self.gizmo.cancel();
        self.refresh = true;
    }
    pub(in crate::ik_edit) fn clip_ui(&mut self, ui: &mut egui::Ui, palette: &Palette, world: &World, frame: &Frame) {
        let Some(document) = self.clip.document.as_ref() else { return };
        let (start, end, mut name, keys) = (document.start, document.end, document.name.clone(), document.frames(self.selected));
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
                self.goals.clear();
                self.authoring = authoring::State::default();
                self.clip.modified.clear();
                self.clip.translations.clear();
                self.clip.clear_after = None;
                self.clip.playing = !self.clip.playing;
                if self.clip.playing && self.clip.frame >= end {
                    self.clip.frame = start;
                    self.clip.cursor = f64::from(start);
                }
                self.refresh = true;
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
            seek |= ui.add(egui::DragValue::new(&mut value).range(0..=LAST).speed(1)).changed();
            ui.label(format!("{:.3} s", value as f32 / FPS));
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
            if let Some(document) = self.clip.document.as_mut() {
                Arc::make_mut(document).remove(bone, self.clip.frame);
            }
            self.clip.changed();
            self.goals.clear();
            self.authoring = authoring::State::default();
        }
        ui.label(aurora_i18n::tr!("anim-edit-key-list"));
        ui.horizontal_wrapped(|ui| {
            let visible: Vec<_> = keys.into_iter().filter(|k| *k >= start && *k <= end).collect();
            let pivot = visible.partition_point(|k| *k < self.clip.frame);
            let first = pivot.saturating_sub(12);
            for key in visible.into_iter().skip(first).take(24) {
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
        for (index, track) in document.tracks.iter().enumerate() {
            ui.push_id(("native-track", track.bone), |ui| {
                ui.horizontal(|ui| {
                    let mut enabled = track.enabled;
                    let mut priority = track.motion.priority;
                    let mut changed = widgets::checkbox(ui, palette, &mut enabled, "").changed();
                    if ui
                        .add(egui::Button::selectable(self.selected == Some(track.bone), &track.motion.joint_name).truncate())
                        .clicked()
                    {
                        selected = Some(track.bone);
                    }
                    changed |= ui.add(egui::DragValue::new(&mut priority).range(-1..=6).speed(1)).changed();
                    if changed {
                        changes.push((index, enabled, priority));
                    }
                })
            });
        }
        if !changes.is_empty()
            && let Some(document) = self.clip.document.as_mut()
        {
            let document = Arc::make_mut(document);
            for (index, enabled, priority) in changes {
                document.tracks[index].enabled = enabled;
                document.tracks[index].motion.priority = priority;
            }
            self.clip.changed();
        }
        if let Some(bone) = selected {
            self.select(bone, &world.avatar_lib.rig, frame);
        }
    }
    fn clip_settings_ui(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let Some(document) = &self.clip.document else { return };
        let mut meta = document.meta.clone();
        let mut start = document.start;
        let mut end = document.end;
        let mut changed = false;
        egui::Grid::new("native-animation-range").num_columns(2).show(ui, |ui| {
            ui.label(aurora_i18n::tr!("anim-edit-start"));
            changed |= ui.add(egui::DragValue::new(&mut start).range(0..=LAST - 1)).changed();
            ui.end_row();
            ui.label(aurora_i18n::tr!("anim-edit-end"));
            changed |= ui.add(egui::DragValue::new(&mut end).range(start + 1..=LAST)).changed();
            ui.end_row();
            ui.label(aurora_i18n::tr!("import-priority"));
            changed |= ui.add(egui::DragValue::new(&mut meta.base_priority).range(0..=6)).changed();
            ui.end_row();
        });
        ui.label(aurora_i18n::tr!(
            "anim-edit-duration",
            seconds = format!("{:.3}", end.saturating_sub(start) as f32 / FPS)
        ));
        changed |= widgets::switch_labeled(ui, palette, &mut meta.looping, aurora_i18n::tr!("import-loop")).changed();
        let duration = end.saturating_sub(start) as f32 / FPS;
        egui::Grid::new("native-animation-timing").num_columns(2).show(ui, |ui| {
            for (key, value) in [
                ("import-loop-in", &mut meta.loop_in),
                ("import-loop-out", &mut meta.loop_out),
                ("import-ease-in", &mut meta.ease_in),
                ("import-ease-out", &mut meta.ease_out),
            ] {
                ui.label(aurora_i18n::text(key));
                changed |= ui
                    .add(
                        egui::DragValue::new(value)
                            .range(0.0..=duration)
                            .speed(1.0 / f64::from(FPS))
                            .suffix(" s"),
                    )
                    .changed();
                ui.end_row();
            }
        });
        ui.label(aurora_i18n::tr!("import-animation-hands"));
        let hands = crate::imports::animation::hands();
        egui::ComboBox::from_id_salt("native-hand-pose")
            .selected_text(hands[(meta.hand_pose as usize).min(13)])
            .show_ui(ui, |ui| {
                for (index, label) in hands.iter().enumerate() {
                    changed |= ui.selectable_value(&mut meta.hand_pose, index as u32, *label).changed();
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
                    changed |= ui.selectable_value(&mut meta.emote_name, name.to_owned(), label).changed();
                }
            });
        ui.label(aurora_i18n::tr!("anim-edit-range-hint"));
        if changed && let Some(document) = self.clip.document.as_mut() {
            let document = Arc::make_mut(document);
            document.meta = meta;
            document.range(start, end);
            self.clip.frame = self.clip.frame.clamp(document.start, document.end);
            self.clip.cursor = f64::from(self.clip.frame);
            self.clip.playing = false;
            self.clip.changed();
            self.goals.clear();
            self.authoring = authoring::State::default();
            self.refresh = true;
        }
    }
    pub(in crate::ik_edit) fn clip_export_ui(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        self.clip_capture_ui(ui);
        let enabled = !self.clip.busy
            && !self.clip.waiting_source()
            && self.clip.document.as_ref().is_some_and(|d| {
                d.tracks
                    .iter()
                    .any(|t| t.enabled && (!t.motion.rot_keys.is_empty() || !t.motion.pos_keys.is_empty()))
            });
        ui.horizontal(|ui| {
            if ui
                .add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("anim-edit-save")))
                .clicked()
            {
                self.clip.export = Some(files::Action::Save);
                self.clip.busy = true;
                self.clip.error = None;
            }
            if ui
                .add_enabled(enabled, egui::Button::new(aurora_i18n::tr!("anim-edit-import")))
                .on_hover_text(aurora_i18n::tr!("anim-edit-import-hint"))
                .clicked()
            {
                self.clip.export = Some(files::Action::Import);
                self.clip.busy = true;
                self.clip.error = None;
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
