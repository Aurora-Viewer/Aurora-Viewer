//! The photo viewfinder in the app (`ui::viewfinder`, the logic in
//! `snapshot::viewfinder`): the depth under the AF point asked of the
//! renderer and the hunt it starts, the avatar's face the focus takes, the
//! beep, the two-stage shutter, the keys and the wheel while it shows, and
//! the frame it draws over the world.

use super::App;
use crate::dof::{self, PhotoFocus};
use crate::keybinds::{Action as Bind, Input, Mods};
use crate::settings::PhotoWindowStyle;
use crate::snapshot::viewfinder::{self as vf, AfArea, Event, Phase, Subject};
use crate::ui::snapshot::Action;
use crate::ui::viewfinder::{CamMode, Dial, Frame, Keys, Out, Preset};
use aurora_render::Renderer;
use egui::{Pos2, Rect, Vec2};
use std::time::Instant;
use winit::keyboard::KeyCode;

impl App {
    /// The photo mode shows the viewfinder (Preferences › Aurora › Photo
    /// mode › Viewfinder).
    pub(super) fn viewfinder_on(&self) -> bool {
        self.panels.snapshot && self.photo_style() == PhotoWindowStyle::Aurora
    }

    /// The HUDs show (and take clicks): Preferences' switch and, in
    /// immersive mode, its button (#1190); a HUD locked by RLVa keeps them
    /// shown; in the viewfinder the photo's own, so that the screen shows
    /// what the photo will hold.
    pub(super) fn huds_shown(&self) -> bool {
        let wanted = self.settings.show_huds && (!self.interface.immersive || self.settings.immersive_huds);
        (wanted || self.world.rlva_has_locked_hud()) && (!self.viewfinder_on() || self.settings.snapshot.huds)
    }

    fn vf_now(&self) -> f64 {
        self.egui_ctx.input(|i| i.time)
    }

    /// The AF point in window pixels: the crosshair in mouselook, else the
    /// point chosen (the screen's centre at first).
    fn vf_point_px(&self, renderer: &Renderer) -> (f32, f32) {
        let (w, h) = renderer.size();
        let at = match self.snapshots.vf.point {
            Some(p) if !self.camera.mouselook() => p,
            _ => Pos2::new(0.5, 0.5),
        };
        (at.x * w as f32, at.y * h as f32)
    }

    /// Each frame, before it is laid out: the depth that came back under
    /// the AF point, the hunt, the lens in AF-C and MF, the histogram.
    pub(super) fn viewfinder_frame(&mut self, renderer: &mut Renderer) {
        self.camera_presets_demo(renderer.size());
        // the focal length is the photo's: the view's own comes back when
        // the viewfinder closes
        // the depth of field on, once, for the profiles that saved it off
        // when that was the default; the user's choice after that stays
        if self.viewfinder_on() && !self.settings.snapshot.viewfinder.dof_defaulted {
            self.settings.snapshot.viewfinder.dof_defaulted = true;
            self.settings.snapshot.dof.enabled = true;
            self.settings.save();
            log::info!("viewfinder: depth of field on (first opening)");
        }
        match (self.viewfinder_on(), self.snapshots.fov_before) {
            (true, None) => self.snapshots.fov_before = Some(self.settings.camera.fov),
            (false, Some(fov)) => {
                self.snapshots.fov_before = None;
                if (self.settings.camera.fov - fov).abs() > 1e-6 {
                    self.settings.camera.fov = fov;
                    self.settings.save();
                    log::info!("viewfinder: field of view back to {:.1}°", fov.to_degrees());
                }
            }
            _ => {}
        }
        if !self.viewfinder_on() {
            self.camera.roll = 0.0;
            let v = &mut self.snapshots.vf;
            v.ae_lock = false;
            v.countdown = None;
            v.pad = Default::default();
            if v.menu || v.help || v.drawer || v.presets || v.pressed || v.pick.is_some() {
                v.menu = false;
                v.help = false;
                v.drawer = false;
                v.presets = false;
                v.preset_form.picking = false;
                v.pressed = false;
                v.pending_shot = false;
                v.pick = None;
            }
            return;
        }
        let now = self.vf_now();
        if let Some(end) = self.snapshots.vf.countdown {
            let left = end - now;
            if left <= 0.0 {
                self.snapshots.vf.countdown = None;
                self.vf_fire(renderer);
            } else {
                // a beep a second, two a second for the last two
                let step = if left <= 2.0 { 0.5 } else { 1.0 };
                let slot = (left / step).ceil() as i64;
                if self.snapshots.vf_tick_slot != Some((slot, step.to_bits())) {
                    self.snapshots.vf_tick_slot = Some((slot, step.to_bits()));
                    if self.settings.snapshot.viewfinder.beep {
                        self.vf_play(VfSound::Tick);
                    }
                }
            }
        }
        if self.settings.snapshot.viewfinder.display == vf::Display::Full {
            renderer.histogram_request = Some(self.photo_exposure());
        }
        // the shutter pressed, then released (a quick click: both in one
        // frame, the release after the press)
        if std::mem::take(&mut self.snapshots.vf.half_press) {
            self.vf_half_press(renderer);
        }
        if std::mem::take(&mut self.snapshots.vf.release) {
            self.vf_release(renderer);
        }
        let eye = self.camera.position;
        let at = self.camera.target - self.camera.position;
        // the depth asked by a click or the shutter
        if let Some(ticket) = self.snapshots.vf.pick
            && let Some(answer) = renderer.take_pick(ticket)
        {
            self.snapshots.vf.pick = None;
            let avatar = answer
                .filter(|_| self.settings.snapshot.viewfinder.area == AfArea::Avatar)
                .and_then(|hit| self.vf_avatar_at(hit));
            if std::mem::take(&mut self.snapshots.vf.adding) {
                // an avatar's face on the Avatar area, else the point itself
                let subject = match (avatar, answer) {
                    (Some((id, _)), _) => Some(Subject::Avatar(id)),
                    (None, Some(hit)) => Some(Subject::Point(hit)),
                    (None, None) => None,
                };
                self.vf_group_toggle(subject, eye, at, now);
            } else {
                let mut target = answer.map_or(vf::FAR, |p| dof::distance_along(eye, at, p));
                self.snapshots.vf.subjects.clear();
                self.snapshots.vf.last_point = answer;
                if let Some((id, head)) = avatar {
                    self.snapshots.vf.subjects.push(Subject::Avatar(id));
                    self.snapshots.vf.last_point = None;
                    target = dof::distance_along(eye, at, head);
                }
                if std::mem::take(&mut self.snapshots.vf.recheck) && self.snapshots.vf.af.holds(target) {
                    self.vf_lock_kept(renderer);
                } else {
                    log::info!("viewfinder: focus asked at {target:.2} m");
                    self.snapshots.vf.af.hunt(target, now);
                }
            }
        }
        if self.photo_demo_group() {
            self.vf_demo_group(eye, at, now);
        }
        // locked on avatars: the lens stays on their heads as they (or the
        // camera) move, like a camera's subject tracking; a group's zone
        // follows its nearest and farthest subjects
        self.snapshots.vf.zone = None;
        if self.settings.snapshot.dof.focus == PhotoFocus::Click && !self.snapshots.vf.subjects.is_empty() {
            let distances = self.vf_subject_distances(eye, at);
            let zone = vf::group_zone(&distances);
            self.snapshots.vf.zone = zone;
            if let Some(target) = zone.map(|z| z.middle).or(distances.first().copied()) {
                self.snapshots.vf.af.follow(target, now);
            }
        }
        match self.settings.snapshot.dof.focus {
            PhotoFocus::Manual => {
                let d = self.settings.snapshot.dof.distance;
                self.snapshots.vf.af.manual(d);
            }
            PhotoFocus::Pointer => {
                let over_ui = self.egui_ctx.is_pointer_over_egui();
                let (x, y) = if self.camera.mouselook() {
                    let (w, h) = renderer.size();
                    (w as f32 * 0.5, h as f32 * 0.5)
                } else {
                    self.cursor_pos
                };
                if !over_ui && let Some(p) = renderer.hover_pick(x, y) {
                    let (w, h) = renderer.size();
                    if !self.camera.mouselook() {
                        self.snapshots.vf.point = Some(Pos2::new(x / w.max(1) as f32, y / h.max(1) as f32));
                    }
                    self.snapshots.vf.af.track(dof::distance_along(eye, at, p), now);
                }
            }
            PhotoFocus::Click => {}
        }
        match self.snapshots.vf.af.update(now) {
            Some(Event::Locked) => {
                log::info!("viewfinder: locked at {:.2} m", self.snapshots.vf.af.distance());
                if self.settings.snapshot.viewfinder.beep {
                    self.vf_beep();
                }
                if std::mem::take(&mut self.snapshots.vf.pending_shot) {
                    self.vf_shoot(renderer);
                }
            }
            Some(Event::Failed) => {
                log::info!("viewfinder: focus failed (closer than {} m)", vf::MIN_FOCUS);
                let lost = std::mem::take(&mut self.snapshots.vf.pending_shot);
                let text = if lost {
                    aurora_i18n::tr!("photo-vf-no-photo")
                } else {
                    aurora_i18n::tr!("photo-vf-too-close")
                };
                self.snapshots.vf.say(text, true, now);
            }
            None => {}
        }
    }

    /// The avatar whose body holds `hit` (a column from its feet to above
    /// its head), and the point of its head the lens takes (`vf_head`).
    fn vf_avatar_at(&self, hit: glam::Vec3) -> Option<(uuid::Uuid, glam::Vec3)> {
        let now = Instant::now();
        self.world
            .objects
            .avatars()
            .filter_map(|(_, o)| {
                let head = self.scene.head_world(&self.world, &o.full_id, now)?;
                let across = (hit.truncate() - head.truncate()).length();
                let inside = across < 0.6 && hit.z > head.z - 1.9 && hit.z < head.z + 0.35;
                inside.then_some((across, o.full_id))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .and_then(|(_, id)| Some((id, self.vf_head(&id, now)?)))
    }

    /// Where the lens takes a subject: an avatar's face (`vf_head`; None
    /// once it is gone), or the point.
    fn vf_subject_at(&self, s: &Subject, now: Instant) -> Option<glam::Vec3> {
        match s {
            Subject::Avatar(id) => self.vf_head(id, now),
            Subject::Point(p) => Some(*p),
        }
    }

    /// The subjects' distances along the view axis (`eye`, `at`); the
    /// avatars gone (left the region, no head) leave the group.
    fn vf_subject_distances(&mut self, eye: glam::Vec3, at: glam::Vec3) -> Vec<f32> {
        let now = Instant::now();
        let mut subjects = std::mem::take(&mut self.snapshots.vf.subjects);
        let mut distances = Vec::with_capacity(subjects.len());
        subjects.retain(|s| match self.vf_subject_at(s, now) {
            Some(h) => {
                distances.push(dof::distance_along(eye, at, h));
                true
            }
            None => false,
        });
        self.snapshots.vf.subjects = subjects;
        distances
    }

    /// Shift + click answered: the avatar or the point under it joins the
    /// group focus, or leaves it if it was in, and the lens hunts the
    /// group's middle (the beep once it is sharp). A group started after a
    /// click alone on a point keeps that point. The sky: the group stays
    /// as it is.
    fn vf_group_toggle(&mut self, subject: Option<Subject>, eye: glam::Vec3, at: glam::Vec3, now: f64) {
        let Some(subject) = subject else {
            self.snapshots.vf.say(aurora_i18n::tr!("photo-vf-group-sky"), true, now);
            return;
        };
        let v = &mut self.snapshots.vf;
        if v.subjects.is_empty()
            && let Some(first) = v.last_point.take()
        {
            v.subjects.push(Subject::Point(first));
        }
        let added = vf::toggle_subject(&mut v.subjects, subject, eye);
        let name = match subject {
            Subject::Avatar(id) => self.vf_name(&id),
            Subject::Point(_) => aurora_i18n::tr!("photo-vf-group-point").to_owned(),
        };
        let distances = self.vf_subject_distances(eye, at);
        let count = distances.len();
        if added && count >= 2 {
            let text = aurora_i18n::tr!("photo-vf-group-added", name = name.as_str(), count = count);
            self.snapshots.vf.say(text, false, now);
        } else if !added {
            let text = aurora_i18n::tr!("photo-vf-group-removed", name = name.as_str());
            self.snapshots.vf.say(text, false, now);
        }
        let Some(target) = vf::group_zone(&distances).map(|z| z.middle).or(distances.first().copied()) else {
            // the last one out: the lens stays where it is
            self.snapshots.vf.af.idle();
            return;
        };
        log::info!("viewfinder: group focus, {count} subject(s), lens at {target:.2} m");
        self.snapshots.vf.af.hunt(target, now);
    }

    /// The demo's group (AURORA_DEMO_SNAPSHOT=viewfinder-group): the other
    /// avatars in view within 25 m added one by one, as Shift + clicks on
    /// them would.
    fn vf_demo_group(&mut self, eye: glam::Vec3, at: glam::Vec3, now: f64) {
        let wall = Instant::now();
        let half_fov = self.settings.camera.fov * 0.5;
        let dir = at.normalize_or_zero();
        let ids: Vec<uuid::Uuid> = self
            .world
            .objects
            .avatars()
            .filter(|(_, o)| o.full_id != self.world.agent_id)
            .filter_map(|(_, o)| Some((o.full_id, self.vf_head(&o.full_id, wall)?)))
            .filter(|(_, h)| {
                let to = *h - eye;
                to.length() < 25.0 && dir.dot(to.normalize_or_zero()) > (half_fov * 0.8).cos()
            })
            .map(|(id, _)| id)
            .collect();
        log::info!("viewfinder demo: {} avatar(s) added to the focus", ids.len());
        for id in ids {
            self.vf_group_toggle(Some(Subject::Avatar(id)), eye, at, now);
        }
    }

    /// What the lens takes on an avatar's head, whichever way its face is
    /// turned: the side of the head nearest the camera (the face, or the
    /// back of the head), not the mHead bone, which sits at the base of the
    /// skull a hand behind the face: at f/1.8 close up, the face would blur.
    fn vf_head(&self, id: &uuid::Uuid, now: Instant) -> Option<glam::Vec3> {
        // the head's middle, about where the face box is centred
        let centre = self.scene.head_world(&self.world, id, now)? + glam::Vec3::Z * 0.09;
        let toward = (self.camera.position - centre).normalize_or_zero();
        Some(centre + toward * HEAD_RADIUS)
    }

    /// The name the AF frame shows over an avatar's head: its display name
    /// as the name preferences make it, else the name of its object.
    fn vf_name(&self, id: &uuid::Uuid) -> String {
        let name = self.world.social.display_name_of(id);
        if name == "…" {
            self.world.avatar_name(id).unwrap_or(name)
        } else {
            name
        }
    }

    /// The two beeps of a locked focus (`snapshot::viewfinder::beep_wav`),
    /// on the interface channel.
    fn vf_beep(&mut self) {
        self.vf_play(VfSound::Beep);
    }

    /// One of the viewfinder's sounds, made on first use.
    fn vf_play(&mut self, sound: VfSound) {
        if !self.settings.audio.ui_sounds {
            return;
        }
        let Some(engine) = self.audio_engine.as_ref() else {
            return;
        };
        let (slot, make, gain): (&mut Option<aurora_audio::SoundClip>, fn() -> Vec<u8>, f32) = match sound {
            VfSound::Beep => (&mut self.snapshots.beep, vf::beep_wav, 0.6),
            VfSound::Tick => (&mut self.snapshots.tick, vf::tick_wav, 0.45),
            VfSound::Click => (&mut self.snapshots.click, vf::click_wav, 0.5),
        };
        if slot.is_none() {
            match aurora_audio::SoundClip::decode(&make()) {
                Ok(clip) => *slot = Some(clip),
                Err(e) => {
                    log::warn!("viewfinder: sound not made: {e}");
                    return;
                }
            }
        }
        if let Some(clip) = slot.as_ref() {
            engine.play_clip(aurora_audio::Channel::Ui, clip, gain);
        }
    }

    /// Focus where the AF point is (a click moved it, or the shutter is
    /// half-pressed): the depth under it, then the hunt.
    fn vf_focus(&mut self, renderer: &mut Renderer) {
        if self.settings.snapshot.dof.focus != PhotoFocus::Click {
            return;
        }
        let (x, y) = self.vf_point_px(renderer);
        if let Some(old) = self.snapshots.vf.pick.take() {
            renderer.forget_pick(old);
        }
        self.snapshots.vf.pick = Some(renderer.request_pick(x, y));
        self.snapshots.vf.recheck = false;
        self.snapshots.vf.adding = false;
        self.snapshots.vf.hinted = true;
    }

    /// A new press found the lens still on its subject: no hunt (it would
    /// blur the frame for nothing), the beep again, and the photo if it
    /// was waiting for the focus.
    fn vf_lock_kept(&mut self, renderer: &Renderer) {
        let now = self.vf_now();
        log::info!("viewfinder: lock kept at {:.2} m", self.snapshots.vf.af.distance());
        self.snapshots.vf.af.confirm(now);
        if self.settings.snapshot.viewfinder.beep {
            self.vf_beep();
        }
        if std::mem::take(&mut self.snapshots.vf.pending_shot) {
            self.vf_shoot(renderer);
        }
    }

    /// The photo: at once, or when the self-timer has counted down.
    fn vf_shoot(&mut self, renderer: &Renderer) {
        let secs = self.settings.snapshot.viewfinder.timer;
        if secs == 0 {
            self.vf_fire(renderer);
            return;
        }
        if self.snapshots.vf.countdown.is_none() {
            let now = self.vf_now();
            log::info!("viewfinder: self-timer, {secs} s");
            self.snapshots.vf.countdown = Some(now + f64::from(secs));
            self.snapshots.vf.hinted = true;
            self.snapshots.vf_tick_slot = None;
        }
    }

    /// The photo, with the shutter's blackout.
    fn vf_fire(&mut self, renderer: &Renderer) {
        let now = self.vf_now();
        let v = &mut self.snapshots.vf;
        v.flash_at = Some(now);
        v.hinted = true;
        log::info!("viewfinder: shutter");
        self.photo_action(Action::Take, renderer);
    }

    /// The shutter pressed half way: in AF-S the lens focuses, and the beep
    /// says when it is sharp. Already locked, the lock holds while its
    /// subject is in the frame: the avatar followed, else the same depth
    /// under the AF point; something else there, it focuses again.
    fn vf_half_press(&mut self, renderer: &mut Renderer) {
        let v = &self.snapshots.vf;
        if v.countdown.is_some() {
            self.snapshots.vf.countdown = None;
            let now = self.vf_now();
            self.snapshots.vf.say(aurora_i18n::tr!("photo-vf-timer-cancelled"), false, now);
            return;
        }
        if v.menu || v.help || v.pressed {
            return;
        }
        self.snapshots.vf.pressed = true;
        if self.settings.snapshot.dof.focus != PhotoFocus::Click {
            return;
        }
        let v = &self.snapshots.vf;
        let locked = v.af.phase == Phase::Lock;
        // the avatars followed are dropped as they leave the frame
        // (`viewfinder_ui`)
        if locked && !v.subjects.is_empty() {
            self.vf_lock_kept(renderer);
            return;
        }
        self.vf_focus(renderer);
        self.snapshots.vf.recheck = locked;
    }

    /// The shutter released: the photo, at once or once the lens has
    /// locked (focus priority); none after a failed focus.
    fn vf_release(&mut self, renderer: &Renderer) {
        if !std::mem::take(&mut self.snapshots.vf.pressed) {
            return;
        }
        let v = &mut self.snapshots.vf;
        let hunting = v.af.phase == Phase::Hunt || v.pick.is_some();
        if self.settings.snapshot.dof.focus == PhotoFocus::Click && hunting {
            v.pending_shot = true;
        } else if v.af.phase == Phase::Fail {
            let now = self.vf_now();
            self.snapshots.vf.say(aurora_i18n::tr!("photo-vf-no-photo"), true, now);
        } else {
            self.vf_shoot(renderer);
        }
    }

    /// A press of the left button in the world while the viewfinder shows:
    /// the AF point moves there and focuses; a menu open closes instead;
    /// a HUD or a poser handle under the pointer keeps it. With Shift in
    /// AF-S, the avatar (Avatar area) or the point there joins the group
    /// focus, or leaves it. True when taken.
    pub(super) fn viewfinder_click(&mut self) -> bool {
        // Alt + click (focus the camera), Ctrl + Alt (orbit), Ctrl + Alt +
        // Shift (pan) and the Controls window's object view stay the
        // camera's: the photographer frames with them
        if !self.viewfinder_on() || self.camera.mouselook() || self.alt || self.object_view {
            return false;
        }
        let v = &mut self.snapshots.vf;
        if v.menu || v.help {
            v.menu = false;
            v.help = false;
            return true;
        }
        // A poser handle or a shown HUD under the pointer keeps the press
        // (#1389): the Legacy Viewer's snapshot floater takes no scene
        // clicks, so a HUD's poses and the poser stay usable while framing.
        // The world's press path then decides between the two, as outside
        // the viewfinder.
        if (self.ik_editor.open && self.ik_editor.poser_owns_pointer())
            || (self.huds_shown() && self.scene.hud_pick(&self.world, self.cursor_pos, false).is_some())
        {
            return false;
        }
        let Some(mut gfx) = self.gfx.take() else {
            return true;
        };
        let add = self.shift && self.settings.snapshot.dof.focus == PhotoFocus::Click;
        let (w, h) = gfx.renderer.size();
        self.snapshots.vf.point = Some(Pos2::new(self.cursor_pos.0 / w.max(1) as f32, self.cursor_pos.1 / h.max(1) as f32));
        if !add {
            self.snapshots.vf.subjects.clear();
        }
        self.snapshots.vf.hinted = true;
        self.vf_focus(&mut gfx.renderer);
        self.snapshots.vf.adding = add;
        self.gfx = Some(gfx);
        true
    }

    /// The wheel while the viewfinder shows: alone it zooms the camera as
    /// outside the viewfinder (Ctrl or Shift alone keep their camera
    /// heights); Ctrl + Shift tilts the camera a degree a notch; with Alt it
    /// drives the underlined value of the bottom bar, with Alt + Shift the
    /// aperture, with Alt + Ctrl the exposure. True when taken.
    pub(super) fn viewfinder_wheel(&mut self, steps: f32) -> bool {
        let tilt = self.ctrl && self.shift && !self.alt;
        if !self.viewfinder_on() || steps == 0.0 || !(self.alt || tilt) {
            return false;
        }
        let v = &self.snapshots.vf;
        if v.menu || v.help {
            return true;
        }
        let dir = if steps > 0.0 { 1 } else { -1 };
        if tilt {
            let roll = vf::step_roll(self.camera.roll.to_degrees(), dir);
            self.camera.roll = roll.to_radians();
            return true;
        }
        let dial = if self.shift {
            Dial::Aperture
        } else if self.ctrl {
            Dial::Exposure
        } else {
            v.dial
        };
        self.vf_adjust(dial, dir);
        true
    }

    fn vf_adjust(&mut self, dial: Dial, dir: i32) {
        let now = self.vf_now();
        let s = &mut self.settings.snapshot;
        match dial {
            Dial::Focal => {
                let mm = crate::snapshot::focal_of_fov(self.settings.camera.fov);
                let next = vf::step_focal(mm, dir, crate::ui::snapshot::FOCAL_RANGE);
                self.settings.camera.fov = crate::snapshot::fov_of_focal(next);
            }
            // past f/22, all sharp (the photo's depth of field off), and back
            Dial::Aperture => {
                if !s.dof.enabled {
                    if dir < 0 {
                        s.dof.enabled = true;
                        s.dof.f_number = vf::STOPS[vf::STOPS.len() - 1];
                    }
                } else if dir > 0 && s.dof.f_number >= vf::STOPS[vf::STOPS.len() - 1] {
                    s.dof.enabled = false;
                } else {
                    s.dof.f_number = vf::step_aperture(s.dof.f_number, dir);
                }
            }
            Dial::Exposure => {
                let base = self.settings.exposure;
                let ev = vf::ev_of(self.snapshots.ui.light.exposure.unwrap_or(base), base) + dir as f32 / 3.0;
                let e = vf::exposure_at(ev, base);
                self.snapshots.ui.light.exposure = if (e - base).abs() < 1e-4 { None } else { Some(e) };
            }
            Dial::Distance => {
                // direct manual focus: from where the lens is
                if s.dof.focus != PhotoFocus::Manual {
                    s.dof.distance = self.snapshots.vf.af.distance().min(200.0);
                    s.dof.focus = PhotoFocus::Manual;
                    self.snapshots.vf.dial = Dial::Distance;
                    self.snapshots.vf.say(aurora_i18n::tr!("photo-vf-direct-manual"), false, now);
                }
                let d = vf::step_distance(s.dof.distance, dir);
                s.dof.distance = d.min(200.0);
            }
        }
        self.settings.save();
    }

    /// AF-S, AF-C, MF in turn.
    fn vf_cycle_af(&mut self) {
        let next = match self.settings.snapshot.dof.focus {
            PhotoFocus::Click => PhotoFocus::Pointer,
            PhotoFocus::Pointer => PhotoFocus::Manual,
            PhotoFocus::Manual => PhotoFocus::Click,
        };
        self.vf_set_af(next);
    }

    fn vf_set_af(&mut self, focus: PhotoFocus) {
        let s = &mut self.settings.snapshot;
        if s.dof.focus == focus {
            return;
        }
        s.dof.focus = focus;
        let v = &mut self.snapshots.vf;
        match s.dof.focus {
            PhotoFocus::Manual => {
                s.dof.distance = v.af.distance().clamp(vf::MIN_FOCUS, 200.0);
                v.dial = Dial::Distance;
            }
            _ => {
                if v.dial == Dial::Distance {
                    v.dial = Dial::Focal;
                }
                v.af.idle();
            }
        }
        v.subjects.clear();
        v.last_point = None;
        self.settings.save();
    }

    fn vf_toggle_area(&mut self) {
        let next = match self.settings.snapshot.viewfinder.area {
            AfArea::Avatar => AfArea::Point,
            AfArea::Point => AfArea::Avatar,
        };
        self.vf_set_area(next);
    }

    fn vf_set_area(&mut self, area: AfArea) {
        self.settings.snapshot.viewfinder.area = area;
        self.snapshots.vf.subjects.clear();
        self.snapshots.vf.last_point = None;
        self.settings.save();
    }

    /// What the camera pad drives: the viewer's camera, the free camera,
    /// or the avatar's eyes.
    fn vf_camera(&mut self, mode: CamMode) {
        match mode {
            CamMode::Orbit => {
                if self.camera.free().is_some() {
                    self.snapshots.ui.free.on = false;
                    self.photo_free_camera();
                }
                if self.camera.mouselook() {
                    self.toggle_mouselook();
                }
            }
            CamMode::Free => {
                if self.camera.mouselook() {
                    self.toggle_mouselook();
                }
                self.snapshots.ui.free.on = true;
                self.photo_free_camera();
            }
            CamMode::First => self.vf_first_person(true),
        }
    }

    /// The camera pad this frame (`dt` seconds): the free camera's flight
    /// and look, the eyes' look and focal length in the first person
    /// view; around the avatar, the rates of the camera keys it holds
    /// (orbit right, left, over, under, zoom in, out, pan left, right, up,
    /// down, like `CameraKeys`).
    pub(super) fn viewfinder_pad(&mut self, dt: f32) -> Option<[f32; 10]> {
        if !self.viewfinder_on() {
            return None;
        }
        let pad = self.snapshots.vf.pad;
        if self.camera.free().is_some() {
            // Regarder, Glisser (and rise), the rocker forward and back
            self.controls_free = (egui::Vec2::new(pad.pan.x, -pad.rocker), egui::Vec2::new(pad.orbit.x, -pad.orbit.y));
            self.snapshots.vf_free_up = -pad.pan.y;
            return None;
        }
        self.snapshots.vf_free_up = 0.0;
        if self.camera.mouselook() {
            // look around (about 90° a second at full tilt), the rocker on
            // the focal length
            let a = &mut self.world.agent;
            a.yaw -= pad.orbit.x * 1.6 * dt;
            a.pitch = (a.pitch - pad.orbit.y * 1.2 * dt).clamp(-1.5, 1.5);
            if pad.rocker != 0.0 {
                let mm = crate::snapshot::focal_of_fov(self.settings.camera.fov);
                let range = crate::ui::snapshot::FOCAL_RANGE;
                let next = (mm * (-pad.rocker * 0.9 * dt).exp()).clamp(*range.start(), *range.end());
                self.settings.camera.fov = crate::snapshot::fov_of_focal(next);
            }
            return None;
        }
        // pushed right, the camera goes round to the left, as the Controls
        // window's orbit stick (LLJoystickCameraRotate::onHeldDown)
        let pos = |v: f32| v.max(0.0);
        Some([
            pos(-pad.orbit.x) * PAD_TURN,
            pos(pad.orbit.x) * PAD_TURN,
            pos(-pad.orbit.y) * PAD_TURN,
            pos(pad.orbit.y) * PAD_TURN,
            pos(-pad.rocker),
            pos(pad.rocker),
            pos(-pad.pan.x) * PAD_PAN,
            pos(pad.pan.x) * PAD_PAN,
            pos(-pad.pan.y) * PAD_PAN,
            pos(pad.pan.y) * PAD_PAN,
        ])
    }

    /// The first person view (mouselook) on or off.
    fn vf_first_person(&mut self, on: bool) {
        if on != self.camera.mouselook() {
            if on && self.camera.free().is_some() {
                self.snapshots.ui.free.on = false;
                self.photo_free_camera();
            }
            self.toggle_mouselook();
        }
    }

    /// A key while the viewfinder shows; true when it is the viewfinder's.
    /// Enter is the shutter (held: the focus); Tab (and Q when no shortcut
    /// of the viewer has it) the quick menu, I the display, N the AF mode,
    /// V the AF area, G the guide, H or F1 the help, 1 to 5 the focal
    /// lengths. The letters only when no shortcut of the viewer uses the
    /// key: the movement keys stay the camera's and the avatar's.
    pub(super) fn viewfinder_key(&mut self, code: KeyCode, logical: &winit::keyboard::Key, pressed: bool, repeat: bool) -> bool {
        if !self.viewfinder_on() || self.ctrl || self.alt {
            return false;
        }
        if matches!(code, KeyCode::Enter | KeyCode::NumpadEnter) {
            if pressed && !repeat {
                if let Some(mut gfx) = self.gfx.take() {
                    self.vf_half_press(&mut gfx.renderer);
                    self.gfx = Some(gfx);
                }
            } else if !pressed && let Some(gfx) = self.gfx.take() {
                self.vf_release(&gfx.renderer);
                self.gfx = Some(gfx);
            }
            return true;
        }
        let free = !self.vf_key_bound(code);
        let letter = match logical {
            winit::keyboard::Key::Character(c) => c.to_lowercase(),
            _ => String::new(),
        };
        let which = match code {
            KeyCode::Tab => Some('q'),
            KeyCode::F1 => Some('h'),
            KeyCode::Digit1 | KeyCode::Numpad1 => Some('1'),
            KeyCode::Digit2 | KeyCode::Numpad2 => Some('2'),
            KeyCode::Digit3 | KeyCode::Numpad3 => Some('3'),
            KeyCode::Digit4 | KeyCode::Numpad4 => Some('4'),
            KeyCode::Digit5 | KeyCode::Numpad5 => Some('5'),
            _ if free => letter
                .chars()
                .next()
                .filter(|c| matches!(c, 'q' | 'i' | 'n' | 'v' | 'g' | 'h' | 'l')),
            _ => None,
        };
        let Some(which) = which else {
            return false;
        };
        if !pressed || repeat {
            return true;
        }
        let v = &mut self.snapshots.vf;
        let s = &mut self.settings.snapshot;
        match which {
            'q' => {
                v.menu = !v.menu;
                v.help = false;
            }
            'h' => {
                v.help = !v.help;
                v.menu = false;
            }
            'i' => {
                s.viewfinder.display = s.viewfinder.display.next();
                self.settings.save();
            }
            'g' => {
                s.guide = crate::ui::viewfinder::next_guide(s.guide);
                self.settings.save();
            }
            'n' => self.vf_cycle_af(),
            'v' => self.vf_toggle_area(),
            'l' => {
                v.ae_lock = !v.ae_lock;
                let text = if v.ae_lock {
                    aurora_i18n::tr!("photo-vf-ael-on")
                } else {
                    aurora_i18n::tr!("photo-vf-ael-off")
                };
                let now = self.vf_now();
                self.snapshots.vf.say(text, false, now);
            }
            d => {
                let i = d.to_digit(10).unwrap_or(3) as usize;
                let mm = vf::FOCAL_PRESETS[i.clamp(1, 5) - 1];
                self.settings.camera.fov = crate::snapshot::fov_of_focal(mm);
                self.settings.save();
            }
        }
        true
    }

    /// A shortcut of the viewer uses this key alone.
    fn vf_key_bound(&self, code: KeyCode) -> bool {
        let input = Input::key(code);
        let kb = &self.settings.keybinds;
        kb.triggered(&input, Mods::default()).is_some() || !kb.held_actions_of(&input).is_empty()
    }

    /// The keys the help names: the AF mode's, the menu's (Q too when no
    /// shortcut of the viewer has it) and the first person's.
    fn vf_keys(&self) -> (String, String, Option<String>) {
        let kb = &self.settings.keybinds;
        let q_taken = [
            Bind::Forward,
            Bind::Back,
            Bind::Left,
            Bind::Right,
            Bind::StrafeLeft,
            Bind::StrafeRight,
            Bind::Up,
            Bind::Down,
            Bind::Fly,
            Bind::Mouselook,
        ]
        .iter()
        .flat_map(|a| kb.get(*a))
        .flatten()
        .any(|b| !b.ctrl && !b.alt && !b.shift && b.label() == "Q");
        let menu = if q_taken { "Tab".to_owned() } else { "Q / Tab".to_owned() };
        let first = kb.get(Bind::Mouselook)[0].as_ref().map(|b| b.label());
        ("N".to_owned(), menu, first)
    }

    /// The viewfinder over the world (the interface hidden): the overlay,
    /// the AF point on the face it holds, and what it asks.
    pub(super) fn viewfinder_ui(
        &mut self,
        ctx: &egui::Context,
        p: &crate::theme::Palette,
        projector: &crate::ui::hud::Projector,
        renderer: &Renderer,
        view: &crate::ui::snapshot::View,
    ) {
        let now = self.vf_now();
        let screen = ctx.content_rect();
        let window = renderer.size();
        let photo = self.photo_size(window);
        // the AF point: the subject's face, else the point chosen
        let mut point = match self.snapshots.vf.point {
            Some(f) if !self.camera.mouselook() => Pos2::new(screen.left() + f.x * screen.width(), screen.top() + f.y * screen.height()),
            _ => screen.center(),
        };
        let wall = Instant::now();
        let face_of = |app: &App, id: &uuid::Uuid| -> Option<Rect> {
            let head = app.scene.head_world(&app.world, id, wall)?;
            // mHead sits at the base of the skull: the face is above it
            let (top, _) = projector.project(head + glam::Vec3::Z * 0.25)?;
            let (bottom, _) = projector.project(head - glam::Vec3::Z * 0.06)?;
            // the head's height along its own axis on screen, which the
            // camera's tilt turns: the box holds the face turned with it
            let axis = top - bottom;
            let h = axis.length().clamp(18.0, 400.0);
            let up = if axis.length() > 0.5 {
                axis / axis.length()
            } else {
                Vec2::new(0.0, -1.0)
            };
            let (w, h) = (h * 0.82, h);
            Some(Rect::from_center_size(
                top + (bottom - top) * 0.5,
                Vec2::new(w * up.y.abs() + h * up.x.abs(), w * up.x.abs() + h * up.y.abs()),
            ))
        };
        let area_avatar = self.settings.snapshot.viewfinder.area == AfArea::Avatar;
        let mut face = None;
        let mut group = Vec::new();
        let mut points = Vec::new();
        let mut held = false;
        if self.settings.snapshot.dof.focus != PhotoFocus::Manual {
            // the subjects out of the frame are dropped; the first one left
            // holds the AF point, the others their own boxes and brackets
            let mut subjects = std::mem::take(&mut self.snapshots.vf.subjects);
            subjects.retain(|s| {
                let first = !held;
                let kept = match s {
                    Subject::Avatar(id) => face_of(self, id).map(|r| {
                        if first {
                            point = r.center();
                            face = Some((r, true, self.vf_name(id)));
                        } else {
                            group.push((r, self.vf_name(id)));
                        }
                    }),
                    Subject::Point(at) => projector.project(*at).map(|(at, _)| {
                        if first {
                            point = at;
                        } else {
                            points.push(at);
                        }
                    }),
                };
                held |= kept.is_some();
                kept.is_some()
            });
            self.snapshots.vf.subjects = subjects;
        }
        if area_avatar && self.settings.snapshot.dof.focus != PhotoFocus::Manual && !held {
            // the face nearest the AF point, detected
            face = self
                .world
                .objects
                .avatars()
                // (not those the photo's isolation leaves out)
                .filter(|(idx, _)| crate::snapshot::isolate::object_visible(&self.world, *idx))
                .filter_map(|(_, o)| Some((face_of(self, &o.full_id)?, o.full_id)))
                .map(|(r, id)| (r.center().distance(point), r, id))
                .filter(|(d, _, _)| *d < 120.0)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, r, id)| (r, false, self.vf_name(&id)));
        }
        // the zone kept sharp, as the renderer blurs it
        let s = &self.settings;
        let mm = crate::snapshot::focal_of_fov(s.camera.fov);
        let lens = s.snapshot.dof.applied(&s.dof).0;
        // a group's zone from its nearest subject to its farthest
        let lens_at = self.snapshots.vf.af.distance();
        let zone = self.snapshots.vf.zone.filter(|_| held).map(|z| z.at(lens_at));
        let (near, far) = zone.unwrap_or((lens_at, lens_at));
        let k = dof::coc_scale(
            dof::focal_length_at(&lens, s.camera.fov),
            lens.f_number,
            near,
            s.camera.fov,
            window.1,
        );
        let sharp = vf::sharp_zone(k, near, far);
        let ev = vf::ev_of(self.photo_exposure(), s.exposure);
        let dir = (self.camera.target - self.camera.position).normalize_or_zero();
        let pitch = dir.z.clamp(-1.0, 1.0).asin().to_degrees();
        let review = self.photo_review();
        let (af_key, menu_key, first_key) = self.vf_keys();
        let (isolate_rows, isolate_hover) = self.photo_isolate_rows();
        // an object anchor holds the object the camera is on, until one is
        // chosen in the scene
        let form = &mut self.snapshots.vf.preset_form;
        if self.snapshots.vf.presets && form.object.is_none() && !form.picking {
            form.object = self.camera.focus_object_id();
        }
        let preset_object = self.snapshots.vf.preset_form.object.map(|id| self.photo_isolate_name(&id).0);
        let first_allowed = self.settings.camera.enable_mouselook;
        let frame = Frame {
            now,
            photo,
            window,
            fps: self.perf.fps,
            focal_mm: mm,
            ev,
            sharp,
            point,
            face,
            group,
            points,
            zone,
            pitch,
            histogram: renderer.histogram.as_deref(),
            review: review.as_ref().map(|(t, l, k)| (t, l.clone(), k.clone())),
            first_person: self.camera.mouselook(),
            first_person_allowed: first_allowed,
            base_exposure: self.settings.exposure,
            roll: self.camera.roll.to_degrees(),
            free_camera: self.camera.free().is_some(),
            keys: Keys {
                menu: &menu_key,
                af_mode: &af_key,
                first_person: first_key.as_deref().filter(|_| first_allowed),
            },
            isolation: &self.world.photo_isolation,
            isolate_rows: &isolate_rows,
            isolate_hover,
            presets: &self.settings.camera_shots,
            preset_object,
            posing: self.ik_editor.open && self.ik_editor.posing(),
        };
        // the demo aims at the own avatar's face, then shoots
        let (aim, shoot) = self.photo_demo_viewfinder();
        if aim && let Some(r) = face_of(self, &self.world.agent_id) {
            let c = r.center();
            self.snapshots.vf.point = Some(Pos2::new(
                (c.x - screen.left()) / screen.width(),
                (c.y - screen.top()) / screen.height(),
            ));
            self.snapshots.vf.half_press = true;
            log::info!("viewfinder demo: half-press on the own avatar's face");
        }
        if shoot {
            self.snapshots.vf.release = true;
            log::info!("viewfinder demo: release");
        }
        let mut v = std::mem::take(&mut self.snapshots.vf);
        let mut panel = std::mem::take(&mut self.snapshots.ui);
        let outs = v.show(
            ctx,
            p,
            &mut self.settings.snapshot,
            &mut panel,
            &mut self.settings.camera.fov,
            crate::camera::DEFAULT_FOV,
            &frame,
            view,
        );
        drop(frame);
        self.snapshots.vf = v;
        self.snapshots.ui = panel;
        for o in outs {
            match o {
                Out::Panel(a) => self.photo_action(a, renderer),
                // (the frame holds the renderer: the next one asks the depth)
                Out::HalfPress => self.snapshots.vf.half_press = true,
                Out::Release => {
                    if self.snapshots.vf.half_press {
                        self.snapshots.vf.release = true;
                    } else {
                        self.vf_release(renderer);
                    }
                }
                Out::Cancel => self.snapshots.vf.pressed = false,
                Out::FirstPerson(on) => self.vf_first_person(on),
                Out::SetAf(focus) => self.vf_set_af(focus),
                Out::SetArea(area) => self.vf_set_area(area),
                Out::Camera(mode) => self.vf_camera(mode),
                Out::Roll(deg) => self.camera.roll = deg.clamp(-vf::ROLL_MAX, vf::ROLL_MAX).to_radians(),
                Out::Detent => {
                    if self.settings.snapshot.viewfinder.beep {
                        self.vf_play(VfSound::Click);
                    }
                }
                Out::Isolate(o) => self.photo_isolate_out(o, ctx.pixels_per_point()),
                Out::Preset(o) => self.vf_preset(o, ctx.pixels_per_point()),
                Out::Poser => {
                    if self.ik_editor.toggle_poser() {
                        crate::ui::widgets::open_floater(ctx, "poser", &mut self.ik_editor.open);
                    }
                }
                Out::Exit => self.panels.snapshot = false,
            }
        }
    }
}

impl App {
    /// What the camera presets panel asks (`ppp`: pixels per point).
    fn vf_preset(&mut self, o: Preset, ppp: f32) {
        use super::photo_isolate::Pick;
        let now = self.vf_now();
        match o {
            Preset::Apply(i) => {
                let name = self.settings.camera_shots.get(i).map(|s| s.name.clone()).unwrap_or_default();
                match self.apply_camera_preset(i) {
                    Ok(()) => self
                        .snapshots
                        .vf
                        .say(aurora_i18n::tr!("camera-preset-applied", name = name.as_str()), false, now),
                    Err(text) => self.snapshots.vf.say(text, true, now),
                }
            }
            Preset::Delete(i) => self.delete_camera_preset(i),
            Preset::Save { name, include } => {
                let object = self.snapshots.vf.preset_form.object;
                match self.save_camera_preset(name.clone(), include, object) {
                    Ok(()) => self
                        .snapshots
                        .vf
                        .say(aurora_i18n::tr!("camera-preset-saved", name = name.as_str()), false, now),
                    Err(text) => self.snapshots.vf.say(text, true, now),
                }
            }
            Preset::Click(at) => self.snapshots.isolate_point = Some(((at.x * ppp, at.y * ppp), Pick::Anchor)),
            Preset::Hover(at) => {
                if self.snapshots.isolate_point.is_none() {
                    self.snapshots.isolate_point = Some(((at.x * ppp, at.y * ppp), Pick::Hover));
                }
            }
        }
    }
}

/// The pad's Turn and Slide sticks at full tilt, as a share of the orbit
/// and pan keys' rates: framing wants smaller moves than the keys give.
const PAD_TURN: f32 = 0.6;
const PAD_PAN: f32 = 0.4;

/// About half a head's depth (m): from its middle to the face.
const HEAD_RADIUS: f32 = 0.09;

/// The viewfinder's sounds (`snapshot::viewfinder`).
#[derive(Debug, Clone, Copy)]
enum VfSound {
    /// The focus locked: two beeps.
    Beep,
    /// The self-timer: one beep.
    Tick,
    /// A lens ring's detent.
    Click,
}
