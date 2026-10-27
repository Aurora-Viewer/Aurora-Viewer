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
use crate::camera::{CameraShot, FreeCamera};
use crate::world::notifications::{Data, Kind};
use std::time::Instant;
use uuid::Uuid;

impl App {
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
            });
        }
        if include.looks {
            shot.looks = Some(presets::looks_of(
                &self.settings.snapshot.looks,
                &self.settings.snapshot.look_settings,
            ));
        }
        if include.light {
            shot.light = Some(Light {
                environment: self.world.eep.saved_local().map(|d| aurora_llsd::to_notation(&d)),
                time_of_day: self.panels.time_of_day,
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
        self.settings.save();
        Ok(())
    }

    pub(super) fn delete_camera_preset(&mut self, i: usize) {
        if i < self.settings.camera_shots.len() {
            let gone = self.settings.camera_shots.remove(i);
            log::info!("camera preset deleted: {}", gone.name);
            self.settings.save();
        }
    }

    /// Apply preset `i`: each part it keeps. Err: a part that could not be
    /// applied (the rest is), said to the user.
    pub(super) fn apply_camera_preset(&mut self, i: usize) -> Result<(), String> {
        if self.world.rlva.camera_preset_locked(Uuid::nil()) {
            return Err(aurora_i18n::tr!("camera-preset-locked").to_owned());
        }
        let Some(shot) = self.settings.camera_shots.get(i).cloned() else {
            return Ok(());
        };
        log::info!("camera preset applied: {}", shot.name);
        // the lens first: the anchor's view is framed with its field of view
        if let Some(lens) = &shot.lens {
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
                // the free camera flies on from there
                match self.camera.free_mut() {
                    Some(free) => *free = FreeCamera::from_view(eye, at),
                    None => self.camera.set_view(eye, at, &mut self.world.agent, &self.settings.camera),
                }
            }
            Anchor::Object { id, name, camera, focus } => {
                let g = crate::camera::focus_object_geom(&self.world, id, Instant::now())
                    .ok_or_else(|| aurora_i18n::tr!("camera-preset-object-away", name = name.as_str()))?;
                self.end_preset_free_camera();
                let eye = presets::out_of_frame(g.position, g.rotation, *camera);
                let at = presets::out_of_frame(g.position, g.rotation, *focus);
                self.camera
                    .set_script_focus(eye, at, Some((*id, g)), &mut self.world.agent, &self.settings.camera);
            }
        }
        Ok(())
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
                if let Some(i) = self.settings.camera_shots.iter().position(|s| s.name == name)
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
