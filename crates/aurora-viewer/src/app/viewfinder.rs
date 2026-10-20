//! The photo viewfinder in the app (`ui::viewfinder`, the logic in
//! `snapshot::viewfinder`): the depth under the AF point asked of the
//! renderer and the hunt it starts, the avatar's face the focus takes, the
//! beep, the two-stage shutter, the keys and the wheel while it shows, and
//! the frame it draws over the world.

use super::App;
use crate::dof::{self, PhotoFocus};
use crate::keybinds::{Action as Bind, Input, Mods};
use crate::settings::PhotoWindowStyle;
use crate::snapshot::viewfinder::{self as vf, AfArea, Event, Phase};
use crate::ui::snapshot::Action;
use crate::ui::viewfinder::{Dial, Frame, Keys, Out};
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
        if !self.viewfinder_on() {
            let v = &mut self.snapshots.vf;
            if v.menu || v.help || v.drawer || v.pressed || v.pick.is_some() {
                v.menu = false;
                v.help = false;
                v.drawer = false;
                v.pressed = false;
                v.pending_shot = false;
                v.pick = None;
            }
            return;
        }
        let now = self.vf_now();
        if self.settings.snapshot.viewfinder.display == vf::Display::Full {
            renderer.histogram_request = Some(self.photo_exposure());
        }
        if std::mem::take(&mut self.snapshots.vf.half_press) {
            self.vf_half_press(renderer);
            if std::mem::take(&mut self.snapshots.vf.release) {
                self.vf_release(renderer);
            }
        }
        let eye = self.camera.position;
        let at = self.camera.target - self.camera.position;
        // the depth asked by a click or the shutter
        if let Some(ticket) = self.snapshots.vf.pick
            && let Some(answer) = renderer.take_pick(ticket)
        {
            self.snapshots.vf.pick = None;
            let mut target = answer.map_or(vf::FAR, |p| dof::distance_along(eye, at, p));
            self.snapshots.vf.subject = None;
            if let Some(hit) = answer
                && self.settings.snapshot.viewfinder.area == AfArea::Avatar
                && let Some((id, eyes)) = self.vf_avatar_at(hit)
            {
                self.snapshots.vf.subject = Some(id);
                target = dof::distance_along(eye, at, eyes);
            }
            log::info!("viewfinder: focus asked at {target:.2} m");
            self.snapshots.vf.af.hunt(target, now);
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
    /// its head), and the point between its eyes.
    fn vf_avatar_at(&self, hit: glam::Vec3) -> Option<(uuid::Uuid, glam::Vec3)> {
        let now = Instant::now();
        self.world
            .objects
            .avatars()
            .filter_map(|(_, o)| {
                let head = self.scene.head_world(&self.world, &o.full_id, now)?;
                let across = (hit.truncate() - head.truncate()).length();
                let inside = across < 0.6 && hit.z > head.z - 1.9 && hit.z < head.z + 0.35;
                inside.then(|| {
                    let eyes = self.scene.eyes_world(&self.world, &o.full_id, now).unwrap_or(head);
                    (across, o.full_id, eyes)
                })
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, id, eyes)| (id, eyes))
    }

    /// The two beeps of a locked focus (`snapshot::viewfinder::beep_wav`),
    /// on the interface channel.
    fn vf_beep(&mut self) {
        if !self.settings.audio.ui_sounds {
            return;
        }
        let Some(engine) = self.audio_engine.as_ref() else {
            return;
        };
        if self.snapshots.beep.is_none() {
            match aurora_audio::SoundClip::decode(&vf::beep_wav()) {
                Ok(clip) => self.snapshots.beep = Some(clip),
                Err(e) => {
                    log::warn!("viewfinder: beep not made: {e}");
                    return;
                }
            }
        }
        if let Some(clip) = &self.snapshots.beep {
            engine.play_clip(aurora_audio::Channel::Ui, clip, 0.6);
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
        self.snapshots.vf.hinted = true;
    }

    /// The photo, with the shutter's blackout.
    fn vf_shoot(&mut self, renderer: &Renderer) {
        let now = self.vf_now();
        let v = &mut self.snapshots.vf;
        v.flash_at = Some(now);
        v.hinted = true;
        log::info!("viewfinder: shutter");
        self.photo_action(Action::Take, renderer);
    }

    /// The shutter pressed half way: the focus (AF-S, unless already
    /// locked).
    fn vf_half_press(&mut self, renderer: &mut Renderer) {
        let v = &self.snapshots.vf;
        if v.menu || v.help || v.pressed {
            return;
        }
        self.snapshots.vf.pressed = true;
        if self.settings.snapshot.dof.focus == PhotoFocus::Click && self.snapshots.vf.af.phase != Phase::Lock {
            self.vf_focus(renderer);
        }
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
    /// the AF point moves there and focuses; a menu open closes instead.
    /// True when taken.
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
        let Some(mut gfx) = self.gfx.take() else {
            return true;
        };
        let (w, h) = gfx.renderer.size();
        self.snapshots.vf.point = Some(Pos2::new(self.cursor_pos.0 / w.max(1) as f32, self.cursor_pos.1 / h.max(1) as f32));
        self.snapshots.vf.subject = None;
        self.snapshots.vf.hinted = true;
        self.vf_focus(&mut gfx.renderer);
        self.gfx = Some(gfx);
        true
    }

    /// The wheel while the viewfinder shows: the underlined value, the
    /// aperture with Shift, the exposure with Alt, the distance with Ctrl
    /// (which turns to manual focus). True when taken.
    pub(super) fn viewfinder_wheel(&mut self, steps: f32) -> bool {
        if !self.viewfinder_on() || steps == 0.0 {
            return false;
        }
        let v = &self.snapshots.vf;
        if v.menu || v.help {
            return true;
        }
        let dir = if steps > 0.0 { 1 } else { -1 };
        let dial = if self.shift {
            Dial::Aperture
        } else if self.alt {
            Dial::Exposure
        } else if self.ctrl {
            Dial::Distance
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
            Dial::Aperture => s.dof.f_number = vf::step_aperture(s.dof.f_number, dir),
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
        let s = &mut self.settings.snapshot;
        s.dof.focus = match s.dof.focus {
            PhotoFocus::Click => PhotoFocus::Pointer,
            PhotoFocus::Pointer => PhotoFocus::Manual,
            PhotoFocus::Manual => PhotoFocus::Click,
        };
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
        v.subject = None;
        self.settings.save();
    }

    fn vf_toggle_area(&mut self) {
        let a = &mut self.settings.snapshot.viewfinder.area;
        *a = match a {
            AfArea::Avatar => AfArea::Point,
            AfArea::Point => AfArea::Avatar,
        };
        self.snapshots.vf.subject = None;
        self.settings.save();
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
            _ if free => letter.chars().next().filter(|c| matches!(c, 'q' | 'i' | 'n' | 'v' | 'g' | 'h')),
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
            let (c, _) = projector.project(head + glam::Vec3::Z * 0.09)?;
            let (top, _) = projector.project(head + glam::Vec3::Z * 0.25)?;
            let (bottom, _) = projector.project(head - glam::Vec3::Z * 0.06)?;
            let h = (bottom.y - top.y).abs().clamp(18.0, 400.0);
            Some(Rect::from_center_size(
                Pos2::new(c.x, (top.y + bottom.y) * 0.5),
                Vec2::new(h * 0.82, h),
            ))
        };
        let area_avatar = self.settings.snapshot.viewfinder.area == AfArea::Avatar;
        let mut face = None;
        if area_avatar && self.settings.snapshot.dof.focus != PhotoFocus::Manual {
            if let Some(id) = self.snapshots.vf.subject {
                match face_of(self, &id) {
                    Some(r) => {
                        point = r.center();
                        face = Some((r, true));
                    }
                    None => self.snapshots.vf.subject = None,
                }
            }
            if face.is_none() {
                // the face nearest the AF point, detected
                face = self
                    .world
                    .objects
                    .avatars()
                    .filter_map(|(_, o)| face_of(self, &o.full_id))
                    .map(|r| (r.center().distance(point), r))
                    .filter(|(d, _)| *d < 120.0)
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map(|(_, r)| (r, false));
            }
        }
        // the zone kept sharp, as the renderer blurs it
        let s = &self.settings;
        let mm = crate::snapshot::focal_of_fov(s.camera.fov);
        let lens = s.snapshot.dof.applied(&s.dof).0;
        let k = dof::coc_scale(
            dof::focal_length_at(&lens, s.camera.fov),
            lens.f_number,
            self.snapshots.vf.af.distance(),
            s.camera.fov,
            window.1,
        );
        let sharp = vf::sharp_range(k, self.snapshots.vf.af.distance());
        let ev = vf::ev_of(self.photo_exposure(), s.exposure);
        let dir = (self.camera.target - self.camera.position).normalize_or_zero();
        let pitch = dir.z.clamp(-1.0, 1.0).asin().to_degrees();
        let review = self.photo_review();
        let (af_key, menu_key, first_key) = self.vf_keys();
        let frame = Frame {
            now,
            photo,
            fps: self.perf.fps,
            focal_mm: mm,
            ev,
            sharp,
            point,
            face,
            pitch,
            histogram: renderer.histogram.as_deref(),
            review: review.as_ref().map(|(t, l, k)| (t, l.clone(), k.clone())),
            first_person: self.camera.mouselook(),
            free_camera: self.camera.free().is_some(),
            keys: Keys {
                menu: &menu_key,
                af_mode: &af_key,
                first_person: first_key.as_deref(),
            },
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
                Out::CycleAf => self.vf_cycle_af(),
                Out::ToggleArea => self.vf_toggle_area(),
                Out::FirstPerson(on) => self.vf_first_person(on),
                Out::Exit => self.panels.snapshot = false,
            }
        }
    }
}
