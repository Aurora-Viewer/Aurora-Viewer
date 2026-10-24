//! Native Aurora implementation of Main.dc.html and Settings.dc.html.
use super::*;
use controls::Operation;
use document::LoadMode;

impl Editor {
    pub(in crate::ik_edit) fn poser_ui(&mut self, ctx: &egui::Context, p: &Palette, world: &World, frame: &Frame) {
        if !ctx.input(|i| i.pointer.primary_down() || i.pointer.primary_released()) {
            self.poser.drag_checkpoint = false;
        }
        let rig = &world.avatar_lib.rig;
        let (mut library, mut settings, mut undo, mut redo) = (false, false, false, false);
        let mut open = self.open;
        let mut floater = Floater::new(
            "poser",
            aurora_i18n::tr!("poser-title"),
            egui::pos2(650.0, 70.0),
            egui::vec2(if self.poser.library_open { 1030.0 } else { 800.0 }, 736.0),
        );
        floater.min_size = egui::vec2(if self.poser.library_open { 950.0 } else { 720.0 }, 480.0);
        floater.steady_size = true;
        floater.actions = vec![
            ("settings-filled", aurora_i18n::text("poser-settings"), &mut settings),
            ("arrow-forward-up", aurora_i18n::text("poser-redo"), &mut redo),
            ("arrow-back-up", aurora_i18n::text("poser-undo"), &mut undo),
            ("layout-sidebar-left-expand", aurora_i18n::text("poser-library"), &mut library),
        ];
        floater.show(ctx, p, &mut open, |ui| {
            ui.horizontal_wrapped(|ui| {
                self.subject_picker(ui, p, world);
                let count = self
                    .poser
                    .sessions
                    .get(&self.poser.subject)
                    .map_or(0, |s| (0..rig.len()).filter(|b| s.edited(*b)).count());
                let enabled = self.poser.sessions.get(&self.poser.subject).is_some_and(|s| s.enabled);
                ui.colored_label(
                    if enabled { p.teal } else { p.muted },
                    if enabled {
                        aurora_i18n::tr!("poser-active-status", count = count)
                    } else {
                        aurora_i18n::tr!("poser-stopped-status", count = count)
                    },
                );
                if ui.button(aurora_i18n::tr!("poser-t-pose")).clicked() {
                    self.pose_operation(rig, frame, Operation::TPose);
                }
                if let Some(session) = self.poser.sessions.get_mut(&self.poser.subject)
                    && widgets::action_button(
                        ui,
                        p,
                        if session.enabled {
                            "player-stop-filled"
                        } else {
                            "player-play-filled"
                        },
                        aurora_i18n::text(if session.enabled { "poser-stop" } else { "poser-start" }),
                        if session.enabled {
                            widgets::ActionKind::Flat
                        } else {
                            widgets::ActionKind::Primary
                        },
                        egui::vec2(0.0, 28.0),
                    )
                    .clicked()
                {
                    session.enabled = !session.enabled;
                }
                if ui.button(aurora_i18n::tr!("poser-other-avatars")).clicked() {
                    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("poser-people-open"), true));
                }
            });
            ui.colored_label(p.muted, aurora_i18n::tr!("poser-photo-notice"));
            let (mut body, footer) = widgets::above_footer(ui);
            self.pose_body_ui(&mut body, p, world, frame);
            footer.show(ui, |ui| self.pose_footer_ui(ui, p, rig, frame));
        });
        if library {
            self.poser.library_open = !self.poser.library_open;
            widgets::size_floater(
                ctx,
                "poser",
                egui::vec2(if self.poser.library_open { 1030.0 } else { 800.0 }, 736.0),
            );
        }
        if settings {
            self.poser.settings_open = true;
        }
        if let Some(session) = self.poser.sessions.get_mut(&self.poser.subject)
            && (undo || redo)
        {
            session.history(redo);
            self.refresh = true;
        }
        if self.poser.settings_open {
            self.pose_settings_ui(ctx, p, rig, frame);
        }
        if let Some((document, pose, bvh, unlock)) = self.poser.storage.overwrite.clone() {
            widgets::Modal::new(egui::Id::unique("poser-overwrite-confirm")).show(ctx, |ui| {
                ui.heading(aurora_i18n::tr!("poser-overwrite"));
                ui.label(&document.name);
                ui.horizontal(|ui| {
                    if ui.button(aurora_i18n::tr!("common-cancel")).clicked() {
                        self.poser.storage.overwrite = None;
                    }
                    if widgets::action_button(
                        ui,
                        p,
                        "device-floppy-filled",
                        aurora_i18n::text("poser-save-library"),
                        widgets::ActionKind::Primary,
                        egui::vec2(0.0, 28.0),
                    )
                    .clicked()
                    {
                        self.poser.storage.overwrite = None;
                        self.poser.storage.job = Some(storage::Job::Save(document, pose, bvh, unlock, false));
                    }
                });
            });
        }
        if !open {
            self.close_poser();
        }
    }
    fn pose_body_ui(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, frame: &Frame) {
        let mut rect = ui.available_rect_before_wrap();
        let gap = ui.spacing().item_spacing.x;
        // Horizontal rows offer their default row height to children. Use the
        // measured body rectangles so all three panes receive the full height.
        let pane = |ui: &mut egui::Ui, id: &str, rect: egui::Rect| {
            let mut pane = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(id)
                    .max_rect(rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            pane.set_clip_rect(rect.intersect(ui.clip_rect()));
            pane
        };
        if self.poser.library_open {
            let library = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + 220.0, rect.bottom()));
            self.pose_library_ui(&mut pane(ui, "poser-library-pane", library), p, world);
            rect.min.x = library.right() + gap;
        }
        let mut content = pane(ui, "poser-content-pane", rect);
        let previous_group = self.poser.group;
        widgets::tabs(
            &mut content,
            p,
            &mut self.poser.group,
            &[
                (aurora_i18n::tr!("poser-body"), true),
                (aurora_i18n::tr!("poser-face"), true),
                (aurora_i18n::tr!("poser-hands"), true),
                (aurora_i18n::tr!("poser-misc"), true),
                (aurora_i18n::tr!("poser-physics"), true),
            ],
        );
        if previous_group != self.poser.group {
            self.select_pose_group(self.poser.group, &world.avatar_lib.rig, frame);
        }
        let rect = content.available_rect_before_wrap();
        let bones = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + 245.0, rect.bottom()));
        let controls = egui::Rect::from_min_max(egui::pos2(bones.right() + gap, rect.top()), rect.max);
        let rig = &world.avatar_lib.rig;
        egui::ScrollArea::vertical()
            .id_salt("poser-joints")
            .max_height(rect.height())
            .auto_shrink([false, false])
            .show(&mut pane(&mut content, "poser-bones-pane", bones), |ui| {
                self.bone_list(ui, p, rig, frame)
            });
        egui::ScrollArea::vertical()
            .id_salt("poser-joint-controls")
            .max_height(rect.height())
            .auto_shrink([false, false])
            .show(&mut pane(&mut content, "poser-controls-pane", controls), |ui| {
                self.joint_controls(ui, p, rig, frame)
            });
    }
    fn pose_footer_ui(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.poser.name)
                    .char_limit(100)
                    .desired_width(170.0)
                    .hint_text(aurora_i18n::tr!("poser-name")),
            );
            if widgets::action_button_if(
                ui,
                p,
                "device-floppy-filled",
                aurora_i18n::text("poser-save-library"),
                widgets::ActionKind::Primary,
                egui::vec2(0.0, 28.0),
                !self.poser.storage.busy && !self.poser.name.trim().is_empty(),
            )
            .clicked()
                && let Some(session) = self.poser.sessions.get(&self.poser.subject)
                && let Some(snapshot) = session.pose.snapshot(rig, frame.model)
            {
                let mut capture = session.clone();
                capture.pose = Arc::new(snapshot);
                let document = document::Document::capture(rig, &capture, self.poser.name.clone(), self.poser.priority);
                self.poser.storage.job = Some(storage::Job::Save(
                    document,
                    capture.pose,
                    self.poser.also_bvh,
                    self.poser.unlock_pelvis,
                    self.poser.confirm_overwrite,
                ));
            }
            ui.menu_button(aurora_i18n::tr!("poser-load-menu"), |ui| {
                for (mode, key) in [
                    (LoadMode::All, "poser-load-all"),
                    (LoadMode::Rotations, "poser-load-rotations"),
                    (LoadMode::Selective, "poser-load-selective"),
                    (LoadMode::SelectiveRotations, "poser-load-selective-rotations"),
                ] {
                    if ui.button(aurora_i18n::text(key)).clicked() {
                        let path = self
                            .poser
                            .storage
                            .selected
                            .and_then(|i| self.poser.storage.entries.get(i))
                            .map(|e| e.path.clone());
                        self.poser.storage.job = Some(storage::Job::Load(path, mode, self.poser.subject));
                        ui.close();
                    }
                }
            });
            widgets::checkbox(ui, p, &mut self.poser.also_bvh, aurora_i18n::tr!("poser-also-bvh"));
            if widgets::action_button_if(
                ui,
                p,
                "file-upload-filled",
                aurora_i18n::text("poser-upload"),
                widgets::ActionKind::Flat,
                egui::vec2(0.0, 28.0),
                !self.poser.storage.busy,
            )
            .clicked()
                && let Some(session) = self.poser.sessions.get(&self.poser.subject)
                && let Some(snapshot) = session.pose.snapshot(rig, frame.model)
            {
                let translations = (0..rig.len())
                    .filter(|b| snapshot.positions[*b].distance_squared(session.base.positions[*b]) > 1e-10)
                    .collect();
                self.poser.storage.job = Some(storage::Job::Import(
                    Arc::new(snapshot),
                    translations,
                    self.poser.name.clone(),
                    self.poser.priority,
                ));
            }
        });
        if self.poser.storage.busy {
            ui.label(aurora_i18n::tr!("anim-edit-building"));
        }
        if let Some(message) = &self.poser.storage.message {
            ui.label(message);
        }
    }
    fn pose_library_ui(&mut self, ui: &mut egui::Ui, _p: &Palette, world: &World) {
        ui.strong(aurora_i18n::tr!("poser-library"));
        if ui.button(aurora_i18n::tr!("poser-library-refresh")).clicked() {
            self.poser.storage.ready = false;
        }
        ui.add(
            egui::TextEdit::singleline(&mut self.poser.storage.filter)
                .hint_text(aurora_i18n::tr!("poser-library-search"))
                .desired_width(200.0),
        );
        let filter = self.poser.storage.filter.to_lowercase();
        let height = (ui.available_height() - 70.0).max(30.0);
        egui::ScrollArea::vertical()
            .id_salt("poser-library")
            .max_height(height)
            .show(ui, |ui| {
                for (i, entry) in self.poser.storage.entries.iter().enumerate() {
                    if !entry.name.to_lowercase().contains(&filter) {
                        continue;
                    }
                    if ui.selectable_label(self.poser.storage.selected == Some(i), &entry.name).clicked() {
                        self.poser.storage.selected = Some(i);
                    }
                    ui.small(aurora_i18n::tr!("poser-library-meta", date = entry.date_label()));
                }
            });
        if ui.button(aurora_i18n::tr!("poser-load-all")).clicked()
            && let Some(entry) = self.poser.storage.selected.and_then(|i| self.poser.storage.entries.get(i))
        {
            self.poser.storage.job = Some(storage::Job::Load(Some(entry.path.clone()), LoadMode::All, self.poser.subject));
        }
        if ui.button(aurora_i18n::tr!("poser-open-folder")).clicked() {
            let path = crate::settings::account_dir(&world.agent_id).join("poses");
            if let Err(error) = std::fs::create_dir_all(&path).and_then(|()| crate::logging::open_directory(&path)) {
                self.poser.storage.message = Some(error.to_string());
            }
        }
    }
    fn pose_settings_ui(&mut self, ctx: &egui::Context, p: &Palette, rig: &Rig, frame: &Frame) {
        let mut open = self.poser.settings_open;
        let floater = Floater::new(
            "poser-settings",
            aurora_i18n::tr!("poser-settings"),
            egui::pos2(180.0, 110.0),
            egui::vec2(400.0, 500.0),
        );
        floater.show(ctx, p, &mut open, |ui| {
            widgets::switch_labeled(ui, p, &mut self.poser.stop_on_close, aurora_i18n::tr!("poser-stop-on-close"));
            widgets::switch_labeled(ui, p, &mut self.poser.markers, aurora_i18n::tr!("poser-markers"));
            if widgets::switch_labeled(ui, p, &mut self.poser.natural, aurora_i18n::tr!("poser-natural")).changed() {
                self.space = if self.poser.natural { Space::Local } else { Space::World };
                self.gizmo.cancel();
            }
            widgets::switch_labeled(ui, p, &mut self.poser.dof, aurora_i18n::tr!("poser-dof"));
            ui.label(aurora_i18n::tr!("poser-sensitivity"));
            ui.add(egui::Slider::new(&mut self.poser.sensitivity, 0.05..=2.0));
            widgets::switch_labeled(ui, p, &mut self.poser.also_bvh, aurora_i18n::tr!("poser-also-bvh"));
            widgets::switch_labeled(ui, p, &mut self.poser.unlock_pelvis, aurora_i18n::tr!("poser-unlock-pelvis"));
            widgets::switch_labeled(ui, p, &mut self.poser.confirm_overwrite, aurora_i18n::tr!("poser-overwrite"));
            ui.horizontal(|ui| {
                ui.label(aurora_i18n::tr!("poser-priority"));
                ui.add(egui::DragValue::new(&mut self.poser.priority).range(0..=6));
            });
            if ui.button(aurora_i18n::tr!("ik-edit-reset-all")).clicked() {
                self.pose_operation(rig, frame, Operation::ResetAll);
            }
            ui.label(aurora_i18n::tr!("poser-format-limits"));
        });
        self.poser.settings_open = open;
    }
}
