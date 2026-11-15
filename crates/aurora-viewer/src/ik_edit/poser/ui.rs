//! The poser window, as redesigned with the human on 2026-10-24: the subject
//! strip, the tabs, the joints column (or the pose library in its place),
//! the selected joint (with the settings laid over it) and the file footer.
use super::*;
use crate::keybinds::EditHistory;
use crate::ui::menu;
use document::LoadMode;

const DEFAULT_SIZE: egui::Vec2 = egui::vec2(880.0, 760.0);
const MIN_SIZE: egui::Vec2 = egui::vec2(720.0, 480.0);
/// The joints column, and the settings panel over the joint.
const COLUMN: f32 = 270.0;
const SETTINGS_W: f32 = 330.0;

/// A child laid out in `rect` exactly, cut at its edges: horizontal rows
/// would offer their own height to the panes.
fn pane(ui: &mut egui::Ui, id: &str, rect: egui::Rect) -> egui::Ui {
    let mut pane = ui.new_child(
        egui::UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    pane.set_clip_rect(rect.intersect(ui.clip_rect()));
    pane
}

impl Editor {
    pub(in crate::ik_edit) fn poser_ui(&mut self, ctx: &egui::Context, p: &Palette, world: &World, frame: &Frame) {
        if !ctx.input(|i| i.pointer.primary_down() || i.pointer.primary_released()) {
            self.poser.drag_checkpoint = false;
        }
        let rig = &world.avatar_lib.rig;
        self.mirrors.initialize(rig);
        self.follow_selected_side(rig);
        let (mut library, mut settings, mut undo, mut redo, mut switch) = (false, false, false, false, false);
        let mut markers = false;
        self.poser.edited_total = self
            .poser
            .sessions
            .values()
            .map(|s| (0..rig.len()).filter(|b| s.edited(*b)).count())
            .sum();
        let mut open = self.open;
        if self.compact_mode() {
            // undo and redo sit in the window's last row, with the reset;
            // the gear opens the few options the window does not show
            let actions = vec![
                ("arrows-maximize", aurora_i18n::text("poser-full-mode"), &mut switch),
                ("settings-filled", aurora_i18n::text("poser-settings"), &mut settings),
            ];
            self.compact_window(ctx, p, world, frame, &mut open, actions);
        } else {
            // The settings' "Show joint markers and gizmos" switch, one click
            // away while posing, as the Legacy Viewer's toggleVisualManipulators
            // button (FSFloaterPoser::onToggleVisualManipulators,
            // indra/newview/fsfloaterposer.cpp); the eye says what is on screen.
            // The compact window has its own markers button in its last row.
            let (markers_icon, markers_tip) = if self.poser.markers {
                ("eye-filled", aurora_i18n::text("poser-markers-hide"))
            } else {
                ("eye-off", aurora_i18n::text("poser-markers"))
            };
            let actions = vec![
                ("arrows-minimize", aurora_i18n::text("poser-compact-mode"), &mut switch),
                ("settings-filled", aurora_i18n::text("poser-settings"), &mut settings),
                (markers_icon, markers_tip, &mut markers),
                ("arrow-forward-up", aurora_i18n::text("poser-redo"), &mut redo),
                ("arrow-back-up", aurora_i18n::text("poser-undo"), &mut undo),
                ("book-filled", aurora_i18n::text("poser-library"), &mut library),
            ];
            self.full_window(ctx, p, world, frame, &mut open, actions);
        }
        if switch {
            self.switch_window(ctx);
        }
        if library {
            self.poser.library_open = !self.poser.library_open;
        }
        if settings {
            if self.compact_mode() {
                self.poser.options_open = !self.poser.options_open;
            } else {
                self.poser.settings_open = !self.poser.settings_open;
            }
        }
        // after the gear's click, so that the click that closes it does not
        // open it again
        if self.compact_mode() && open {
            self.compact_options(ctx, p);
        } else {
            self.poser.options_open = false;
        }
        if markers {
            self.poser.markers = !self.poser.markers;
        }
        if let Some(history) = self.history_key(ctx) {
            undo |= history == EditHistory::Undo;
            redo |= history == EditHistory::Redo;
        }
        if let Some(session) = self.poser.sessions.get_mut(&self.poser.subject)
            && (undo || redo)
        {
            session.history(redo);
            self.refresh = true;
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
                        self.poser.storage.job = Some(storage::Job::Save(storage::Save {
                            document,
                            pose,
                            bvh,
                            unlock,
                            confirm: false,
                            pick: false,
                        }));
                    }
                });
            });
        }
        if !open {
            self.ask_close();
        }
        if self.poser.close_asked {
            self.close_confirm_ui(ctx, p);
        }
    }
    /// The full window: the subject strip, the tabs, the joints column (or
    /// the library), the joint and the file footer.
    fn full_window(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        world: &World,
        frame: &Frame,
        open: &mut bool,
        actions: Vec<(&str, &str, &mut bool)>,
    ) {
        let rig = &world.avatar_lib.rig;
        let mut floater = Floater::new(FULL_WINDOW, aurora_i18n::tr!("poser-title"), egui::pos2(650.0, 70.0), DEFAULT_SIZE);
        floater.min_size = MIN_SIZE;
        floater.steady_size = true;
        if self.over_viewfinder {
            floater.order = egui::Order::Foreground;
        }
        floater.actions = actions;
        floater.show(ctx, p, open, |ui| {
            self.subject_strip(ui, p, world);
            ui.add_space(2.0);
            let previous_group = self.poser.group;
            widgets::tabs(
                ui,
                p,
                &mut self.poser.group,
                &[
                    (aurora_i18n::text("poser-body"), true),
                    (aurora_i18n::text("poser-face"), true),
                    (aurora_i18n::text("poser-hands"), true),
                    (aurora_i18n::text("poser-misc"), true),
                    (aurora_i18n::text("poser-physics"), true),
                ],
            );
            if previous_group != self.poser.group {
                self.poser.library_open = false;
                self.select_pose_group(self.poser.group, rig, frame);
            }
            ui.add_space(4.0);
            let (mut body, footer) = widgets::above_footer(ui);
            self.pose_body_ui(&mut body, p, world, frame);
            footer.show(ui, |ui| self.pose_footer_ui(ui, p, rig, frame));
        });
    }
    /// The title bar's mode button: the other window opens where this one
    /// was, its top right corner kept, at the size it had last.
    fn switch_window(&mut self, ctx: &egui::Context) {
        let from = window_id(self.compact_mode());
        let compact = !self.compact_mode();
        self.poser.window_override = None;
        self.poser.compact = compact;
        let to = window_id(compact);
        let size = ctx
            .data_mut(|d| d.get_persisted::<egui::Vec2>(egui::Id::unique(to).with("expanded_size")))
            .unwrap_or(if compact { compact::COMPACT_SIZE } else { DEFAULT_SIZE });
        if let Some(rect) = ctx.memory(|m| m.area_rect(egui::Id::unique(from))) {
            let pos = egui::pos2((rect.right() - size.x).max(0.0), rect.top());
            widgets::place_floater(ctx, to, pos, size);
        }
    }
    /// Ctrl+Z / Ctrl+Y (or Ctrl+Shift+Z) as the title's arrows, the key
    /// taken from the build tools' Undo: none while a field keeps its own
    /// history, a menu or a dialog is open, or the build tools' window is in
    /// front of the poser's. The Legacy Viewer's poser has the buttons only
    /// (FSFloaterPoser undo_change / button_redo_change).
    fn history_key(&self, ctx: &egui::Context) -> Option<EditHistory> {
        if self.poser.close_asked
            || ctx.egui_wants_keyboard_input()
            || egui::Popup::is_any_open(ctx)
            || ctx.memory(|m| m.top_modal_layer()).is_some()
            || widgets::floater_above(ctx, crate::build::ui::INSPECTOR_ID, window_id(self.compact_mode()))
        {
            return None;
        }
        let (key, modifiers, history) = ctx.input(|i| {
            i.events.iter().find_map(|e| match *e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers: m,
                    ..
                } => {
                    let mods = crate::keybinds::Mods {
                        ctrl: m.command || m.ctrl,
                        shift: m.shift,
                        alt: m.alt,
                    };
                    EditHistory::from_key(key, mods).map(|h| (key, m, h))
                }
                _ => None,
            })
        })?;
        ctx.input_mut(|i| i.consume_key(modifiers, key));
        Some(history)
    }
    /// Closing stops the pose (`stop_on_close`) and loses its edited joints:
    /// Minimize keeps them in the window's bar, Close anyway goes on, Escape
    /// or a click beside the dialog leaves the window as it was.
    fn close_confirm_ui(&mut self, ctx: &egui::Context, p: &Palette) {
        let (mut minimize, mut close) = (false, false);
        let response = widgets::Modal::new(egui::Id::unique("poser-close-confirm")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(egui::RichText::new(aurora_i18n::tr!("poser-close-title")).size(17.0).color(p.ink));
            ui.add_space(8.0);
            ui.label(aurora_i18n::tr!("poser-close-confirm", count = self.poser.edited_total));
            ui.add_space(6.0);
            widgets::checkbox(ui, p, &mut self.poser.close_dont_ask, aurora_i18n::tr!("poser-close-dont-ask"));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let size = egui::vec2(0.0, 26.0);
                minimize = widgets::action_button(
                    ui,
                    p,
                    "minus",
                    aurora_i18n::text("poser-close-minimize"),
                    widgets::ActionKind::Primary,
                    size,
                )
                .clicked();
                close = widgets::action_button(
                    ui,
                    p,
                    "x",
                    aurora_i18n::text("poser-close-anyway"),
                    widgets::ActionKind::Danger,
                    size,
                )
                .clicked();
            });
        });
        if minimize || close {
            if self.poser.close_dont_ask {
                self.poser.confirm_close = false;
            }
            self.poser.close_dont_ask = false;
        }
        if minimize {
            self.poser.close_asked = false;
            widgets::minimize_floater(ctx, window_id(self.compact_mode()));
        } else if close {
            self.close_poser();
        } else if response.should_close() {
            self.poser.close_asked = false;
            self.poser.close_dont_ask = false;
        }
    }
    /// The subject, how many joints it has edited, the local-only notice and
    /// Start / Stop.
    fn subject_strip(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        let rig = &world.avatar_lib.rig;
        let count = self
            .poser
            .sessions
            .get(&self.poser.subject)
            .map_or(0, |s| (0..rig.len()).filter(|b| s.edited(*b)).count());
        let enabled = self.poser.sessions.get(&self.poser.subject).is_some_and(|s| s.enabled);
        ui.horizontal(|ui| {
            ui.set_min_height(42.0);
            ui.spacing_mut().item_spacing.x = 10.0;
            self.subject_button(ui, p, world);
            let status = if enabled {
                aurora_i18n::tr!("poser-active-status", count = count)
            } else {
                aurora_i18n::tr!("poser-stopped-status", count = count)
            };
            let font = egui::FontId::proportional(12.0);
            let galley = ui.painter().layout_no_wrap(status.to_string(), font, p.ink);
            let (pill, _) = ui.allocate_exact_size(egui::vec2(galley.size().x + 30.0, 24.0), Sense::hover());
            ui.painter().rect_filled(pill, ui.visuals().window_corner_radius, p.bar);
            ui.painter().circle_filled(
                egui::pos2(pill.left() + 12.0, pill.center().y),
                4.0,
                if enabled { p.teal } else { p.muted_dim },
            );
            let text = widgets::row_text_origin(ui.painter(), &galley, pill.left() + 22.0, pill.center().y);
            ui.painter().galley(text, galley, p.ink);
            let notice = aurora_i18n::text("poser-visible-you");
            let galley = ui
                .painter()
                .layout_no_wrap(notice.to_owned(), egui::FontId::proportional(12.0), p.muted);
            let (rect, response) = ui.allocate_exact_size(egui::vec2(galley.size().x + 20.0, 24.0), Sense::hover());
            if let Some(eye) = crate::ui::icons::global("eye-filled") {
                let at = egui::Rect::from_center_size(egui::pos2(rect.left() + 7.0, rect.center().y), egui::Vec2::splat(14.0));
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter().image(eye.id(), at, uv, p.muted);
            }
            let text = widgets::row_text_origin(ui.painter(), &galley, rect.left() + 20.0, rect.center().y);
            ui.painter().galley(text, galley, p.muted);
            response.on_hover_text(aurora_i18n::text("poser-photo-notice"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
            });
        });
    }
    /// The joints column (or the library) and the joint (with the settings
    /// over it).
    fn pose_body_ui(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World, frame: &Frame) {
        let rig = &world.avatar_lib.rig;
        let rect = ui.available_rect_before_wrap();
        let gap = 14.0;
        let column = egui::Rect::from_min_max(rect.min, egui::pos2(rect.left() + COLUMN, rect.bottom()));
        let joint = egui::Rect::from_min_max(egui::pos2(column.right() + gap, rect.top()), rect.max);
        ui.painter()
            .rect_filled(column, ui.visuals().window_corner_radius, p.field.gamma_multiply(0.55));
        let mut left = pane(ui, "poser-column-pane", column.shrink(8.0));
        if self.poser.library_open {
            self.pose_library_ui(&mut left, p, world);
        } else {
            self.side_selector(&mut left, p, rig, frame);
            left.add_space(2.0);
            crate::ui::widgets::search_text_edit(&mut left, p, &mut self.poser.filter, |edit| {
                edit.hint_text(aurora_i18n::tr!("poser-search")).desired_width(f32::INFINITY)
            });
            left.add_space(2.0);
            let height = left.available_height();
            egui::ScrollArea::vertical()
                .id_salt("poser-joints")
                .max_height(height)
                .auto_shrink([false, false])
                .show(&mut left, |ui| {
                    if self.poser.group == 2 {
                        self.hands_panel(ui, p, rig, frame);
                    } else {
                        self.bone_list(ui, p, rig, frame);
                    }
                });
        }
        let mut right = pane(ui, "poser-joint-pane", joint);
        egui::ScrollArea::vertical()
            .id_salt("poser-joint-controls")
            .max_height(joint.height())
            .auto_shrink([false, false])
            .show(&mut right, |ui| self.joint_controls(ui, p, rig, frame));
        if self.poser.settings_open {
            let panel = egui::Rect::from_min_max(egui::pos2((joint.right() - SETTINGS_W).max(joint.left()), joint.top()), joint.max);
            // the joint's controls under the panel do not take its clicks
            ui.interact(panel, ui.scope_id().with("poser-settings-cover"), Sense::click_and_drag());
            let mut over = pane(ui, "poser-settings-pane", panel);
            widgets::overlay(&mut over, p, egui::Margin::same(14), |ui| {
                ui.set_min_size(panel.size() - egui::vec2(28.0, 28.0));
                self.pose_settings_ui(ui, p);
            });
        }
    }
    /// Name, Save (its menu holds the export options), Load (its modes) and
    /// the upload.
    fn pose_footer_ui(&mut self, ui: &mut egui::Ui, p: &Palette, rig: &Rig, frame: &Frame) {
        ui.horizontal(|ui| {
            widgets::restricted_text_field(ui, true, &mut self.poser.name, widgets::TextPolicy::PrintableNoPipe, |b, _| {
                egui::TextEdit::singleline(b)
                    .char_limit(100)
                    .desired_width(220.0)
                    .hint_text(aurora_i18n::tr!("poser-name"))
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                if widgets::action_button_if(
                    ui,
                    p,
                    "device-floppy-filled",
                    aurora_i18n::text("poser-save-library"),
                    widgets::ActionKind::Primary,
                    egui::vec2(0.0, 28.0),
                    !self.poser.storage.busy && !self.poser.name.trim().is_empty(),
                )
                .on_hover_text(aurora_i18n::text("poser-export-hint"))
                .clicked()
                {
                    self.save_pose(rig, frame, false);
                }
                let options = widgets::action_button(ui, p, "chevron-down", "", widgets::ActionKind::Primary, egui::vec2(26.0, 28.0))
                    .on_hover_text(aurora_i18n::text("poser-save-options"));
                menu::dropdown(&options, p, egui::PopupCloseBehavior::CloseOnClickOutside, |ui| {
                    let can_save = !self.poser.storage.busy && !self.poser.name.trim().is_empty();
                    if menu::item_if(ui, p, "device-floppy-filled", aurora_i18n::text("poser-save-as"), can_save) {
                        self.save_pose(rig, frame, true);
                    }
                    menu::separator(ui, p);
                    menu::toggle(
                        ui,
                        p,
                        "file-upload-filled",
                        aurora_i18n::text("poser-also-bvh"),
                        &mut self.poser.also_bvh,
                    );
                    menu::toggle(
                        ui,
                        p,
                        "accessible-filled",
                        aurora_i18n::text("poser-unlock-pelvis"),
                        &mut self.poser.unlock_pelvis,
                    );
                    menu::separator(ui, p);
                    ui.horizontal(|ui| {
                        ui.add_space(8.0);
                        ui.label(aurora_i18n::tr!("poser-priority"));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.add_space(8.0);
                            ui.add(egui::DragValue::new(&mut self.poser.priority).range(0..=6));
                        });
                    });
                });
            });
            let response = ui.add_enabled(!self.poser.storage.busy, |ui: &mut egui::Ui| {
                let open = false;
                widgets::action_menu_button(ui, p, "folder-open-filled", aurora_i18n::text("poser-load-menu"), open, 28.0)
            });
            menu::dropdown(&response, p, egui::PopupCloseBehavior::CloseOnClick, |ui| {
                if menu::item(ui, p, "book-filled", aurora_i18n::text("poser-library")) {
                    self.poser.library_open = true;
                    self.poser.storage.ready = false;
                }
                menu::separator(ui, p);
                for (mode, key) in [
                    (LoadMode::All, "poser-load-all"),
                    (LoadMode::Rotations, "poser-load-rotations"),
                    (LoadMode::Selective, "poser-load-selective"),
                    (LoadMode::SelectiveRotations, "poser-load-selective-rotations"),
                ] {
                    if menu::item(ui, p, "folder-open-filled", aurora_i18n::text(key)) {
                        let path = self.selected_library_path();
                        self.poser.storage.job = Some(storage::Job::Load(path, mode, self.poser.subject));
                    }
                }
                menu::separator(ui, p);
                if menu::item(ui, p, "folder-filled", aurora_i18n::text("poser-open-file")) {
                    self.poser.storage.job = Some(storage::Job::Load(None, LoadMode::All, self.poser.subject));
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::action_button_if(
                    ui,
                    p,
                    "file-upload-filled",
                    aurora_i18n::text("poser-upload"),
                    widgets::ActionKind::Flat,
                    egui::vec2(0.0, 28.0),
                    !self.poser.storage.busy,
                )
                .on_hover_text(aurora_i18n::text("poser-export-hint"))
                .clicked()
                {
                    self.import_pose(rig, frame);
                }
            });
        });
        if self.poser.storage.busy {
            ui.label(aurora_i18n::tr!("anim-edit-building"));
        }
        if let Some(message) = &self.poser.storage.message {
            ui.label(message);
        }
    }
    fn selected_library_path(&self) -> Option<std::path::PathBuf> {
        if !self.poser.library_open {
            return None;
        }
        self.poser
            .storage
            .selected
            .and_then(|i| self.poser.storage.entries.get(i))
            .map(|e| e.path.clone())
    }
    /// `pick`: Save as…, the file is chosen in the system dialog.
    pub(super) fn save_pose(&mut self, rig: &Rig, frame: &Frame, pick: bool) {
        let Some(session) = self.poser.sessions.get(&self.poser.subject) else {
            return;
        };
        let Some(snapshot) = session.pose.snapshot(rig, frame.model) else {
            self.poser.storage.message = Some(aurora_i18n::text("anim-edit-invalid").to_owned());
            return;
        };
        let mut capture = session.clone();
        capture.pose = Arc::new(snapshot);
        let document = document::Document::capture(rig, &capture, self.poser.name.clone(), self.poser.priority);
        let Some(authored) = session.authored(rig, frame.model) else {
            self.poser.storage.message = Some(aurora_i18n::text("anim-edit-invalid").to_owned());
            return;
        };
        self.poser.storage.job = Some(storage::Job::Save(storage::Save {
            document,
            pose: Arc::new(authored),
            bvh: self.poser.also_bvh,
            unlock: self.poser.unlock_pelvis,
            confirm: self.poser.confirm_overwrite,
            pick,
        }));
    }
    pub(super) fn import_pose(&mut self, rig: &Rig, frame: &Frame) {
        let Some(session) = self.poser.sessions.get(&self.poser.subject) else {
            return;
        };
        let Some(authored) = session.authored(rig, frame.model) else {
            self.poser.storage.message = Some(aurora_i18n::text("anim-edit-invalid").to_owned());
            return;
        };
        let translations = (0..rig.len())
            .filter(|b| authored.positions[*b].distance_squared(rig.local_pos[*b]) > 1e-10)
            .collect();
        self.poser.storage.job = Some(storage::Job::Import(
            Arc::new(authored),
            translations,
            self.poser.name.clone(),
            self.poser.priority,
        ));
    }
    /// The library in the joints column's place: the window keeps its size.
    fn pose_library_ui(&mut self, ui: &mut egui::Ui, p: &Palette, world: &World) {
        ui.horizontal(|ui| {
            if widgets::icon_button(ui, p, "chevron-left", aurora_i18n::text("poser-library-back"), true) {
                self.poser.library_open = false;
            }
            ui.strong(aurora_i18n::tr!("poser-library"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, p, "folder-open-filled", aurora_i18n::text("poser-open-folder"), true) {
                    let path = crate::settings::account_dir(&world.agent_id).join("poses");
                    if let Err(error) = std::fs::create_dir_all(&path).and_then(|()| crate::logging::open_directory(&path)) {
                        self.poser.storage.message = Some(error.to_string());
                    }
                }
                if widgets::icon_button(ui, p, "refresh", aurora_i18n::text("poser-library-refresh"), true) {
                    self.poser.storage.ready = false;
                }
            });
        });
        crate::ui::widgets::search_text_edit(ui, p, &mut self.poser.storage.filter, |edit| {
            edit.hint_text(aurora_i18n::tr!("poser-library-search"))
                .desired_width(f32::INFINITY)
        });
        let filter = self.poser.storage.filter.clone();
        let scroll_to = std::mem::take(&mut self.poser.storage.scroll_to_selected) || widgets::search_cleared(ui);
        let height = (ui.available_height() - 40.0).max(30.0);
        let mut load = None;
        egui::ScrollArea::vertical()
            .id_salt("poser-library")
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                for (i, entry) in self.poser.storage.entries.iter().enumerate() {
                    if !storage::filter_matches(&entry.name, &filter) {
                        continue;
                    }
                    let selected = self.poser.storage.selected == Some(i);
                    let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::click());
                    let face = if selected || response.hovered() { p.raised } else { p.bar };
                    ui.painter().rect_filled(rect, ui.visuals().widgets.inactive.corner_radius, face);
                    widgets::paint_elided(
                        ui,
                        &entry.name,
                        egui::pos2(rect.left() + 10.0, rect.top() + 13.0),
                        rect.width() - 20.0,
                        egui::FontId::proportional(13.0),
                        p.ink,
                    );
                    let meta = aurora_i18n::tr!("poser-library-meta", date = entry.date_label()).to_string();
                    widgets::paint_elided(
                        ui,
                        &meta,
                        egui::pos2(rect.left() + 10.0, rect.bottom() - 11.0),
                        rect.width() - 20.0,
                        egui::FontId::proportional(11.0),
                        p.muted,
                    );
                    if selected && scroll_to {
                        response.scroll_to_me_animation(Some(egui::Align::Center), egui::style::ScrollAnimation::none());
                    }
                    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &entry.name));
                    if response.clicked() {
                        self.poser.storage.selected = Some(i);
                    }
                    if response.double_clicked() {
                        load = Some(entry.path.clone());
                    }
                }
            });
        if let Some(path) = load {
            self.poser.storage.job = Some(storage::Job::Load(Some(path), LoadMode::All, self.poser.subject));
        }
        ui.label(
            egui::RichText::new(aurora_i18n::text("poser-library-hint"))
                .size(11.5)
                .color(p.muted),
        );
    }
    /// The settings, grouped, over the joint panel (the gear of the title).
    fn pose_settings_ui(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.horizontal(|ui| {
            ui.strong(aurora_i18n::tr!("poser-settings"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::icon_button(ui, p, "x", aurora_i18n::text("common-close"), true) {
                    self.poser.settings_open = false;
                }
            });
        });
        bones::caption(ui, p, aurora_i18n::text("poser-settings-window"));
        widgets::switch_labeled(ui, p, &mut self.poser.stop_on_close, aurora_i18n::tr!("poser-stop-on-close"));
        // only a close that stops the pose loses it
        ui.add_enabled_ui(self.poser.stop_on_close, |ui| {
            widgets::switch_labeled(ui, p, &mut self.poser.confirm_close, aurora_i18n::tr!("poser-confirm-close"));
        });
        widgets::switch_labeled(ui, p, &mut self.poser.dof, aurora_i18n::tr!("poser-dof"));
        ui.add_space(6.0);
        bones::caption(ui, p, aurora_i18n::text("poser-settings-handling"));
        widgets::switch_labeled(ui, p, &mut self.poser.markers, aurora_i18n::tr!("poser-markers"));
        ui.add_enabled_ui(self.poser.markers, |ui| {
            widgets::switch_labeled(ui, p, &mut self.poser.markers_hover, aurora_i18n::tr!("poser-markers-hover"));
            widgets::switch_labeled(ui, p, &mut self.poser.part_hints, aurora_i18n::tr!("poser-part-hints"));
            widgets::switch_labeled(ui, p, &mut self.poser.avatar_hints, aurora_i18n::tr!("poser-avatar-hints"));
        });
        widgets::switch_labeled(ui, p, &mut self.poser.gizmo, aurora_i18n::tr!("poser-show-gizmo"));
        if widgets::switch_labeled(ui, p, &mut self.poser.natural, aurora_i18n::tr!("poser-natural")).changed() {
            self.space = if self.poser.natural { Space::Local } else { Space::World };
            self.gizmo.cancel();
        }
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("poser-sensitivity"));
            if ui
                .add(egui::Slider::new(&mut self.poser.sensitivity, 0.05..=2.0))
                .on_hover_text(aurora_i18n::tr!("poser-sensitivity-hint"))
                .changed()
            {
                self.set_poser_sensitivity(self.poser.sensitivity);
            }
        });
        ui.add_space(6.0);
        bones::caption(ui, p, aurora_i18n::text("poser-settings-files"));
        widgets::switch_labeled(ui, p, &mut self.poser.confirm_overwrite, aurora_i18n::tr!("poser-overwrite"));
        widgets::switch_labeled(ui, p, &mut self.poser.also_bvh, aurora_i18n::tr!("poser-also-bvh"));
        widgets::switch_labeled(ui, p, &mut self.poser.unlock_pelvis, aurora_i18n::tr!("poser-unlock-pelvis"));
        ui.horizontal(|ui| {
            ui.label(aurora_i18n::tr!("poser-priority"));
            ui.add(egui::DragValue::new(&mut self.poser.priority).range(0..=6));
        });
        ui.add_space(8.0);
        ui.label(
            egui::RichText::new(aurora_i18n::text("poser-format-limits"))
                .size(11.5)
                .color(p.muted),
        );
    }
}
