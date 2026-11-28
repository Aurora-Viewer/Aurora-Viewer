//! Camera presets in the app (`camera::presets`, issue #1164): what a preset
//! takes when it is saved from the viewfinder or the Controls window, and
//! what applying it changes. The position follows the Legacy Viewer's
//! camera presets on the avatar (LLFloaterSaveCameraPreset::onBtnSave,
//! LLPresetsManager::loadPreset, indra/newview, originally LGPL 2.1), the
//! `pstcampos` command in the world (setCameraPosAndFocusGlobal) and the
//! context menu's Zoom on an object (TrackFocusObject); the lens, looks and
//! light are the photo mode's and the environment window's own.

use super::App;
use crate::camera::presets::{self, Anchor, AnchorKind, Include, Lens, Light};
use crate::camera::{CameraPreset, CameraShot, FreeCamera};
use crate::world::notifications::{Data, Kind};
use std::time::Instant;
use uuid::Uuid;

impl App {
    pub(super) fn refresh_camera_shots(&mut self) {
        self.camera_shots.clear();
        self.camera_shots
            .extend(self.settings.camera_shots.iter().filter(|s| s.quick_slot.is_none()).cloned());
        self.camera_shots.extend(self.camera_slots.slots.iter().cloned());
    }

    fn select_camera_account(&mut self, account: Option<Uuid>) {
        let path = account
            .map(|id| crate::settings::account_dir(&id).join(crate::camera::account_slots::FILE))
            .unwrap_or_default();
        self.camera_slots.select(account, path);
        self.refresh_camera_shots();
        self.controls_ui.reset_quick_names();
    }

    pub(super) fn sync_camera_slots(&mut self) {
        let demo_mode = self.demo.then(|| std::env::var("AURORA_DEMO_CAMERA_ACCOUNTS").ok()).flatten();
        let demo_mode = demo_mode.as_deref().filter(|m| matches!(*m, "switch" | "aperture-import"));
        let account = self.world.login.as_ref().map(|l| {
            if demo_mode.is_some() {
                Uuid::from_u128(if demo_mode == Some("switch") && (700..800).contains(&self.frame_count) {
                    103
                } else {
                    102
                })
            } else {
                l.agent_id
            }
        });
        if account != self.camera_slots.account {
            self.slot_import.reset();
            // Every offline demo owns its slots, including the quick-view layout captures.
            if let Some(id) = account.filter(|_| self.demo) {
                let path = match self.camera_slots.demo_path(id) {
                    Ok(path) => path,
                    Err(error) => {
                        log::warn!("cannot create camera demo directory: {error}");
                        return;
                    }
                };
                self.camera_slots.select(account, path);
                self.refresh_camera_shots();
                self.controls_ui.reset_quick_names();
            } else {
                self.select_camera_account(account);
            }
        }
        if self.camera_slots.poll() {
            self.refresh_camera_shots();
            self.controls_ui.reset_quick_names();
        }
        self.poll_legacy_slot_import();
        if let Some(mode) = demo_mode {
            self.camera_accounts_demo(mode);
        }
    }

    fn camera_accounts_demo(&mut self, mode: &str) {
        use crate::ui::controls::ControlAction;
        if self.frame_count >= 600 {
            self.panels.camera_controls = true;
            self.panels.chat = false;
            self.panels.people = false;
        }
        if mode == "switch" {
            match self.frame_count {
                660 | 760 => {
                    let first = self.frame_count == 660;
                    self.settings.camera.fov = if first { 0.7 } else { 0.9 };
                    self.camera.roll = if first { 0.3 } else { 0.6 };
                    self.control_action(ControlAction::QuickSave(0, if first { "Account A" } else { "Account B" }.into()));
                }
                840 => self.control_action(ControlAction::QuickLoad(0)),
                _ => {}
            }
        }
        if mode == "aperture-import" {
            match self.frame_count {
                660 => self.demo_legacy_slot_preview(),
                780 => self.confirm_legacy_slot_import(),
                _ => {}
            }
        }
        if matches!(self.frame_count, 690 | 790 | 870) {
            log::info!(
                "demo camera accounts: frame {}, slots {:?}, lens {}, roll {}",
                self.frame_count,
                self.camera_slots.slots.iter().map(|s| &s.name).collect::<Vec<_>>(),
                self.settings.camera.fov,
                self.camera.roll
            );
        }
    }

    pub(super) fn camera_slots_dialog(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        use crate::camera::account_slots::State;
        use crate::ui::widgets;
        let state = self.camera_slots.state;
        if !self.camera_slots.prompt_open || (state != State::Failed && !self.camera_slots.write_failed) {
            return;
        }
        let mut retry = false;
        let modal = widgets::Modal::new(egui::Id::unique("camera_account_error")).show(ctx, |ui| {
            ui.set_width(420.0);
            ui.label(egui::RichText::new(aurora_i18n::tr!("camera-account-title")).strong().color(p.ink));
            ui.label(if self.camera_slots.write_failed {
                aurora_i18n::tr!("camera-account-save-error")
            } else {
                aurora_i18n::tr!("camera-account-load-error")
            });
            retry = widgets::flat_button(ui, p, aurora_i18n::tr!("camera-account-retry")).clicked();
            if widgets::flat_button(ui, p, aurora_i18n::tr!("common-close")).clicked() {
                self.camera_slots.prompt_open = false;
            }
        });
        if modal.should_close() {
            self.camera_slots.prompt_open = false;
        }
        if retry {
            if self.camera_slots.write_failed {
                self.camera_slots.save();
            } else {
                self.camera_slots.reload();
            }
        }
    }

    /// Eight world-anchored views, using the same capture, persistence and
    /// restore path as named presets (Aperture's LLAgentCamera::storeCameraPosition).
    pub(super) fn save_quick_camera(&mut self, slot: u8, name: String) -> Result<(), String> {
        if self.camera_slots.state != crate::camera::account_slots::State::Ready {
            return Err(aurora_i18n::tr!("camera-account-not-ready").to_owned());
        }
        if slot >= 8 || self.world.rlva.camera_preset_locked(Uuid::nil()) {
            return Err(aurora_i18n::tr!("camera-preset-locked").to_owned());
        }
        let name = name.trim();
        let name = if name.is_empty() {
            aurora_i18n::tr!("camera-quick-name", slot = i64::from(slot) + 1)
        } else {
            name.chars().take(64).collect()
        };
        let include = Include {
            anchor: Some(AnchorKind::World),
            lens: false,
            looks: false,
            light: false,
        };
        let mut shot = self.capture_camera_preset(name.clone(), include, None)?;
        shot.quick_slot = Some(slot);
        shot.roll = Some(self.camera.roll);
        // An extension of storeCameraPosition: keep LLZoomer's lens too,
        // without opting into the photo preset's DOF and exposure bundle.
        shot.quick_fov = Some(self.camera.flycam.as_ref().map_or(self.settings.camera.fov, |flight| flight.fov));
        presets::store(&mut self.camera_slots.slots, shot);
        self.camera_slots.save();
        self.refresh_camera_shots();
        self.camera_preset_message(aurora_i18n::tr!("camera-preset-saved", name = name.as_str()));
        Ok(())
    }

    /// The preset `name` as things are: the parts `include` asks for, the
    /// camera held by `object` for an object anchor. Err: the message that
    /// says why it cannot be saved.
    pub(super) fn capture_camera_preset(&self, name: String, include: Include, object: Option<Uuid>) -> Result<CameraShot, String> {
        if !include.any() {
            return Err(aurora_i18n::tr!("camera-preset-nothing").to_owned());
        }
        let (camera, focus) = self.camera.current_offsets(&self.world.agent);
        let mut shot = CameraShot::at_avatar(name, camera.to_array(), focus.to_array());
        let (eye, at) = (self.camera.position, self.camera.target);
        shot.anchor = match include.anchor {
            None => None,
            Some(AnchorKind::Avatar) => Some(Anchor::Avatar),
            Some(AnchorKind::World) => {
                let origin = self
                    .world
                    .main_origin()
                    .ok_or_else(|| aurora_i18n::tr!("camera-preset-no-region").to_owned())?;
                Some(Anchor::World {
                    camera: presets::to_global(origin, eye),
                    focus: presets::to_global(origin, at),
                })
            }
            Some(AnchorKind::Object) => {
                let id = object.ok_or_else(|| aurora_i18n::tr!("camera-preset-choose-object").to_owned())?;
                let g = crate::camera::focus_object_geom(&self.world, &id, Instant::now())
                    .ok_or_else(|| aurora_i18n::tr!("camera-preset-object-gone").to_owned())?;
                Some(Anchor::Object {
                    id,
                    name: self.photo_isolate_name(&id).0,
                    camera: presets::into_frame(g.position, g.rotation, eye),
                    focus: presets::into_frame(g.position, g.rotation, at),
                })
            }
        };
        if include.lens {
            shot.lens = Some(Lens {
                fov: self.settings.camera.fov,
                dof: self.settings.snapshot.dof.clone(),
                exposure: self.snapshots.ui.light.exposure,
                tilt: self.camera.roll.to_degrees(),
            });
        }
        if include.looks {
            shot.looks = Some(presets::looks_of(
                &self.settings.snapshot.looks,
                &self.settings.snapshot.look_settings,
            ));
        }
        if include.light {
            // the light shown: the photo's, as "Keep this light" would keep it
            let (environment, time_of_day) = self
                .world
                .eep
                .saved_photo()
                .unwrap_or_else(|| (self.world.eep.saved_local(), self.panels.time_of_day));
            shot.light = Some(Light {
                environment: environment.map(|d| aurora_llsd::to_notation(&d)),
                time_of_day,
            });
        }
        Ok(shot)
    }

    /// Save a preset (a known name is replaced). Err: why it was not.
    pub(super) fn save_camera_preset(&mut self, name: String, include: Include, object: Option<Uuid>) -> Result<(), String> {
        if self.world.rlva.camera_preset_locked(Uuid::nil()) {
            return Err(aurora_i18n::tr!("camera-preset-locked").to_owned());
        }
        let shot = self.capture_camera_preset(name, include, object)?;
        log::info!(
            "camera preset saved: {} (anchor {:?}, lens {}, looks {}, light {})",
            shot.name,
            shot.anchor.as_ref().map(Anchor::kind),
            shot.lens.is_some(),
            shot.looks.is_some(),
            shot.light.is_some()
        );
        // on the avatar, the position becomes the view (onBtnSave)
        if shot.anchor == Some(Anchor::Avatar) {
            self.settings.camera.apply_shot(&shot);
        }
        presets::store(&mut self.settings.camera_shots, shot);
        self.refresh_camera_shots();
        self.settings.save();
        Ok(())
    }

    /// Rear, Front, Side or Shoulder view, as the user saved it or as its
    /// template (LLFloaterCamera::switchToPreset): the Controls window, the
    /// system island's menu.
    pub(super) fn switch_camera_view(&mut self, view: CameraPreset) {
        self.world.rlva.user_chose_camera_view();
        self.settings.camera.apply_view(view, &self.settings.camera_views);
        self.camera.reset_to_defaults(&mut self.world.agent, &self.settings.camera);
        self.settings.save();
    }

    /// The camera where it is becomes `view`, kept for the next sessions
    /// (LLFloaterSaveCameraPreset::onBtnSave over "Front View.xml", issue
    /// #2755): the position and the focus point in the avatar's frame, as a
    /// position saved by name.
    pub(super) fn save_camera_view(&mut self, view: CameraPreset) {
        if self.world.rlva.camera_preset_locked(Uuid::nil()) {
            self.camera_preset_message(aurora_i18n::tr!("camera-preset-locked").to_owned());
            return;
        }
        let (camera, focus) = self.camera.current_offsets(&self.world.agent);
        let saved = self
            .settings
            .camera
            .save_view(view, camera.to_array(), focus.to_array(), &mut self.settings.camera_views);
        if !saved {
            self.camera_preset_message(aurora_i18n::tr!("controls-view-not-saved", view = view.label()));
            return;
        }
        log::info!("camera view saved: {view:?} at {camera:.2?}, focus {focus:.2?}");
        self.camera.reset_to_defaults(&mut self.world.agent, &self.settings.camera);
        self.settings.save();
    }

    /// `view` back to its template (LLPresetsManager::resetCameraPreset):
    /// shown again at once when it is the view in use.
    pub(super) fn restore_camera_view(&mut self, view: CameraPreset) {
        if !self.settings.camera_views.restore(view) {
            return;
        }
        log::info!("camera view restored: {view:?}");
        if self.settings.camera.preset == view {
            self.switch_camera_view(view);
        } else {
            self.settings.save();
        }
    }

    pub(super) fn delete_camera_preset(&mut self, i: usize) {
        if let Some(gone) = self.camera_shots.get(i).cloned() {
            if let Some(slot) = gone.quick_slot {
                self.camera_slots.slots.retain(|s| s.quick_slot != Some(slot));
                self.camera_slots.save();
                self.controls_ui.reset_quick_names();
            } else {
                self.settings.camera_shots.retain(|s| s.quick_slot.is_some() || s.name != gone.name);
                self.settings.save();
            }
            self.refresh_camera_shots();
            log::info!("camera preset deleted: {}", gone.name);
        }
    }

    /// Apply preset `i`: each part it keeps. Err: a part that could not be
    /// applied (the rest is), said to the user.
    pub(super) fn apply_camera_preset(&mut self, i: usize) -> Result<(), String> {
        if self.world.rlva.camera_preset_locked(Uuid::nil()) {
            return Err(aurora_i18n::tr!("camera-preset-locked").to_owned());
        }
        let Some(shot) = self.camera_shots.get(i).cloned() else {
            return Ok(());
        };
        log::info!("camera preset applied: {}", shot.name);
        self.world.rlva.user_chose_camera_view();
        // the lens first: the anchor's view is framed with its field of view
        if let Some(lens) = &shot.lens {
            self.world.rlva.user_chose_fov();
            self.settings.camera.fov = lens.fov;
            self.settings.snapshot.dof = lens.dof.clone();
            // the photo's own, while the photo mode is open
            self.snapshots.ui.light.exposure = lens.exposure;
        }
        if let Some(looks) = &shot.looks {
            self.settings.snapshot.looks = looks.layers.clone();
            for (id, values) in &looks.settings {
                self.settings.snapshot.look_settings.insert(id.clone(), values.clone());
            }
            self.snapshots.ui.looks.selected = 0;
        }
        if let Some(light) = &shot.light {
            self.apply_preset_light(light);
        }
        let placed = match &shot.anchor {
            Some(anchor) => self.apply_preset_anchor(&shot, anchor),
            None => Ok(()),
        };
        // Position changes can reset roll. Restore it only after placement;
        // a photo lens preset below retains upstream's explicit tilt behavior.
        if let Some(roll) = presets::position_roll(&shot, placed.is_ok()) {
            self.camera.roll = roll;
        }
        if placed.is_ok()
            && shot.quick_slot.is_some()
            && shot.anchor.is_some()
            && let Some(fov) = shot.quick_fov
        {
            // Use the same window and RLVa bounds as Ctrl-0/Ctrl-8.
            let aspect = self.view_aspect();
            self.settings.camera.fov = crate::camera::zoom_lens(fov, Some(1.0), aspect, self.world.rlva.camera_fov_limits(aspect));
        }
        // the tilt after the anchor: placing the camera on the avatar levels it
        if let Some(lens) = &shot.lens {
            let max = crate::snapshot::viewfinder::ROLL_MAX;
            self.camera.roll = lens.tilt.clamp(-max, max).to_radians();
        }
        self.snapshots.preview_wanted = true;
        self.settings.save();
        placed
    }

    /// The light of a preset, as a choice of the environment window: the
    /// local environment it kept (or the shared one), then its time of day.
    fn apply_preset_light(&mut self, light: &Light) {
        let saved = light
            .environment
            .as_ref()
            .and_then(|e| aurora_llsd::from_notation(e.as_bytes()).ok())
            .filter(|d| d.is_map());
        if !self.world.eep.rlva_locked {
            self.world.eep.drop_photo();
        }
        match saved {
            Some(data) => self.world.eep.apply_saved(&data),
            None => self.world.eep.clear_local(),
        }
        self.world.eep.set_time_of_day(light.time_of_day);
        self.panels.time_of_day = light.time_of_day;
        self.settings.time_of_day = light.time_of_day;
    }

    fn apply_preset_anchor(&mut self, shot: &CameraShot, anchor: &Anchor) -> Result<(), String> {
        match anchor {
            Anchor::Avatar => {
                self.end_preset_flycam();
                self.end_preset_free_camera();
                // LLFloaterCamera::switchToPreset with the preset's offsets
                self.settings.camera.apply_shot(shot);
                self.camera.reset_to_defaults(&mut self.world.agent, &self.settings.camera);
            }
            Anchor::World { camera, focus } => {
                let origin = self
                    .world
                    .main_origin()
                    .ok_or_else(|| aurora_i18n::tr!("camera-preset-no-region").to_owned())?;
                let (eye, at) = (presets::from_global(origin, *camera), presets::from_global(origin, *focus));
                let away = eye.distance(self.world.agent.position);
                let reach = self.settings.draw_distance.max(64.0);
                if away > reach {
                    return Err(aurora_i18n::tr!(
                        "camera-preset-too-far",
                        meters = away.round() as i64,
                        name = shot.name.as_str()
                    ));
                }
                self.end_preset_flycam();
                // loadCameraPosition resolves the optional focus object at
                // load time. The saved global pose remains the fallback when
                // that object is no longer in the scene.
                let object = shot
                    .quick_focus_object
                    .and_then(|id| crate::camera::focus_object_geom(&self.world, &id, Instant::now()).map(|geom| (id, geom)));
                // the free camera flies on from there
                match self.camera.free_mut() {
                    Some(free) => *free = FreeCamera::from_view(eye, at),
                    None if object.is_some() => self
                        .camera
                        .set_script_focus(eye, at, object, &mut self.world.agent, &self.settings.camera),
                    None => self.camera.set_view(eye, at, &mut self.world.agent, &self.settings.camera),
                }
            }
            Anchor::Object { id, name, camera, focus } => {
                let g = crate::camera::focus_object_geom(&self.world, id, Instant::now())
                    .ok_or_else(|| aurora_i18n::tr!("camera-preset-object-away", name = name.as_str()))?;
                self.end_preset_flycam();
                self.end_preset_free_camera();
                let eye = presets::out_of_frame(g.position, g.rotation, *camera);
                let at = presets::out_of_frame(g.position, g.rotation, *focus);
                self.camera
                    .set_script_focus(eye, at, Some((*id, g)), &mut self.world.agent, &self.settings.camera);
            }
        }
        Ok(())
    }

    /// Validate the destination first, then stop hardware flycam before it
    /// can overwrite the restored position and roll on the next frame.
    fn end_preset_flycam(&mut self) {
        if self.joystick.active || self.camera.flycam.is_some() {
            if self.joystick.active {
                self.toggle_joystick_flycam();
            }
            self.camera.clear_flycam();
        }
    }

    /// A preset that holds the camera on the avatar or an object ends the
    /// photo mode's free camera first.
    fn end_preset_free_camera(&mut self) {
        if self.snapshots.ui.free.on {
            self.snapshots.ui.free.on = false;
            self.photo_free_camera();
        }
    }

    /// Say why a preset was not saved or applied: on the viewfinder while it
    /// shows, else as a notice.
    pub(super) fn camera_preset_message(&mut self, text: String) {
        if self.viewfinder_on() {
            let now = self.egui_ctx.input(|i| i.time);
            self.snapshots.vf.say(text, true, now);
        } else {
            self.world
                .notifications
                .push(Kind::Info, aurora_i18n::tr!("camera-presets"), text, Data::None);
        }
    }
}

impl App {
    /// AURORA_DEMO_CAMERA_PRESETS=<steps> with the viewfinder open
    /// (AURORA_DEMO_SNAPSHOT=viewfinder), steps joined by `-`: at frame 600
    /// the presets panel opens; `save` saves "Close up" (on the avatar, the
    /// position only), "Plaza" (the place, the lens and the light) and
    /// "Bench" (the nearest object) at 610; `apply` turns the camera away at
    /// 620 and applies "Plaza" at 660 (the view comes back); `object`
    /// applies "Bench" at 660 instead; `pick` chooses the object in the
    /// scene, the pointer at the middle of the window, 70 % down, from 630
    /// and a click there at 640. The log gives the camera where "Plaza" was
    /// saved and at 700, once applied.
    pub(super) fn camera_presets_demo(&mut self, size: (u32, u32)) {
        if !self.demo || !(600..=700).contains(&self.frame_count) {
            return;
        }
        let Ok(steps) = std::env::var("AURORA_DEMO_CAMERA_PRESETS") else {
            return;
        };
        let steps: Vec<&str> = steps.split('-').collect();
        let at = (size.0 as f32 / 2.0, size.1 as f32 * 0.7);
        match self.frame_count {
            600 => self.snapshots.vf.presets = true,
            610 if steps.contains(&"save") => {
                let only_position = Include {
                    anchor: Some(AnchorKind::Avatar),
                    lens: false,
                    looks: false,
                    light: false,
                };
                let place = Include {
                    anchor: Some(AnchorKind::World),
                    looks: false,
                    ..Include::default()
                };
                let nearest = self.demo_nearest_object();
                let object = Include {
                    anchor: Some(AnchorKind::Object),
                    ..Include::default()
                };
                log::info!("camera presets demo: camera at {:.2?} when saved", self.camera.position);
                for (name, include, object) in [
                    ("Close up", only_position, None),
                    ("Plaza", place, None),
                    ("Bench", object, nearest),
                ] {
                    if let Err(e) = self.save_camera_preset(name.to_owned(), include, object) {
                        log::warn!("camera presets demo: {name} not saved: {e}");
                    }
                }
            }
            620 if steps.contains(&"apply") => {
                self.camera.demo_orbit(1.4, 0.35, Some(9.0), &self.settings.camera);
                log::info!("camera presets demo: camera turned away");
            }
            630 if steps.contains(&"pick") => {
                let form = &mut self.snapshots.vf.preset_form;
                form.include.anchor = Some(AnchorKind::Object);
                form.object = None;
                form.picking = true;
            }
            631..640 if steps.contains(&"pick") => {
                self.snapshots.isolate_point = Some((at, super::photo_isolate::Pick::Hover));
            }
            640 if steps.contains(&"pick") => {
                self.snapshots.isolate_point = Some((at, super::photo_isolate::Pick::Anchor));
            }
            660 => {
                let name = if steps.contains(&"object") {
                    "Bench"
                } else if steps.contains(&"apply") {
                    "Plaza"
                } else {
                    return;
                };
                if let Some(i) = self.camera_shots.iter().position(|s| s.name == name)
                    && let Err(e) = self.apply_camera_preset(i)
                {
                    log::warn!("camera presets demo: {name} not applied: {e}");
                }
            }
            700 => log::info!("camera presets demo: camera at {:.2?}", self.camera.position),
            _ => {}
        }
    }

    /// The demo's object nearest the avatar (a linkset root, not an avatar).
    fn demo_nearest_object(&self) -> Option<Uuid> {
        let now = Instant::now();
        let me = self.world.agent.position;
        self.world
            .objects
            .iter()
            .filter(|(_, o)| o.parent_id == 0 && !o.is_avatar())
            .filter_map(|(_, o)| {
                let g = crate::camera::focus_object_geom(&self.world, &o.full_id, now)?;
                Some((g.position.distance(me), o.full_id))
            })
            .filter(|(d, _)| *d < 30.0)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id)| id)
    }
}
