//! The outfit's auto portrait and its square photo mode on the app's side
//! (Fider #447): the rules are `snapshot::portrait`, the window and the
//! photo mode's helpers `ui::appearance::portrait`.
//!
//! A session saves the camera and the photo's isolation, then frames our
//! avatar from its drawn joints (`portrait::frame`), poses it on this
//! screen only (a poser layer, `ik_edit::poser::standing_pose`), isolates
//! it over the studio backdrop or hides the other avatars, and lights it
//! with the studio light (`photo_light`). Nothing of it is sent: others see
//! us as usual. The window's preview is a small photo drawn a few times a
//! second while it is open, nothing while it is closed; the photo itself is
//! shown for confirmation, then becomes the outfit's free thumbnail through
//! the gallery snapshot's steps (`inventory_thumbnail`), and when chosen
//! and its price confirmed, the outfit's paid HD picture too (`outfit_hd`).
//! Closing restores the camera, the isolation and the light.
//!
//! An outfit that is not worn is never put on behind the user's back: the
//! window offers to wear it, then to put the previous outfit back.

use super::App;
use super::snapshot::Purpose;
use crate::snapshot::isolate::{Isolation, Mode};
use crate::snapshot::portrait::{self, Backdrop, Lighting, PortraitSettings};
use crate::ui::appearance::portrait as window;
use aurora_render::Renderer;
use glam::{Quat, Vec3};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Where the photo is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// The auto portrait's window.
    Studio,
    /// The photo mode's viewfinder with a square frame.
    Viewfinder,
}

/// The window's preview is drawn again this often while the camera moves,
/// and this often once it holds still (the avatar's idle motion, an outfit
/// arriving). Each preview costs the render thread a photo (about 10 ms
/// in the demo on the release build), hence the slower pace at rest.
const PREVIEW_MOVING: Duration = Duration::from_millis(250);
const PREVIEW_STILL: Duration = Duration::from_millis(1000);
/// How long the camera follows the joints after a change (the pose
/// settling in), then it holds still.
const REFRAME: Duration = Duration::from_millis(1200);
/// The studio light: the sun behind the camera, this far to its side and
/// this high, over the midday sky (`portrait::STUDIO_TIME`).
const KEY_SIDE: f32 = 30.0;
const KEY_HEIGHT: f32 = 40.0;

/// The photo awaiting "Use".
pub(super) struct Shot {
    size: (u32, u32),
    pixels: Vec<u8>,
    texture: egui::TextureHandle,
}

pub(super) struct Session {
    outfit: Uuid,
    kind: Kind,
    settings: PortraitSettings,
    /// What the session changes, as it was.
    camera: crate::camera::Camera,
    yaw: f32,
    pitch: f32,
    isolation: Isolation,
    /// The photo mode was open before (the square mode closes it after
    /// only when it opened it).
    was_open: bool,
    /// The settings the camera was last framed for; frames left to follow.
    framed: Option<PortraitSettings>,
    reframe: Option<Instant>,
    /// The studio light's sun is chosen at the next framing.
    sun_due: bool,
    /// The studio light's sun, chosen when framing.
    sun: Option<Quat>,
    /// The local pose, for the arms' angle it was made with.
    pose: Option<(f32, Arc<crate::ik_edit::poser::SubjectPose>)>,
    preview_at: Option<Instant>,
    /// The camera (position, target) the last preview was drawn from.
    preview_view: Option<(Vec3, Vec3)>,
    preview: Option<egui::TextureHandle>,
    shot_wanted: bool,
    shooting: bool,
    shot: Option<Shot>,
    /// Use waits for the HD picture's price to be confirmed.
    hd_confirm: Option<i32>,
    /// Worn from here: the outfit worn before (None: none was known).
    previous: Option<Option<Uuid>>,
    /// The photo part is over (camera, isolation and light restored): the
    /// window asks whether to put the previous outfit back.
    over: bool,
    saved: bool,
    /// The photo mode's helpers.
    vf_pose: bool,
    vf_studio: bool,
    /// The window is open (its X closes the session).
    open: bool,
}

impl App {
    /// The outfit's auto portrait (`Kind::Studio`) or its square photo mode.
    pub(super) fn open_outfit_portrait(&mut self, outfit: Uuid, kind: Kind) {
        if self
            .outfit_portrait
            .as_ref()
            .is_some_and(|s| s.outfit == outfit && s.kind == kind && !s.over)
        {
            return;
        }
        if self.outfit_portrait.is_some() {
            self.close_outfit_portrait();
        }
        let was_open = self.panels.snapshot;
        // the light and isolation the photo mode had are its own: a
        // portrait starts from the user's view
        if kind == Kind::Studio && was_open {
            self.panels.snapshot = false;
        }
        let mut settings = PortraitSettings::opening(&self.settings.outfit_portrait);
        if kind == Kind::Viewfinder {
            // (the square mode starts on the whole body, straight in front)
            settings.framing = portrait::Framing::Body;
            settings.angle = portrait::Angle::Front;
        }
        log::info!("outfit portrait: opened for {outfit} ({kind:?})");
        self.outfit_portrait = Some(Session {
            outfit,
            kind,
            settings,
            camera: self.camera.clone(),
            yaw: self.world.agent.yaw,
            pitch: self.world.agent.pitch,
            isolation: self.world.photo_isolation.clone(),
            was_open: was_open && kind == Kind::Viewfinder,
            framed: None,
            reframe: (kind == Kind::Viewfinder).then(|| Instant::now() + REFRAME),
            sun_due: true,
            sun: None,
            pose: None,
            preview_at: None,
            preview_view: None,
            preview: None,
            shot_wanted: false,
            shooting: false,
            shot: None,
            hd_confirm: None,
            previous: None,
            over: false,
            saved: false,
            vf_pose: false,
            vf_studio: false,
            open: true,
        });
        if kind == Kind::Viewfinder {
            self.panels.snapshot = true;
        } else {
            crate::ui::widgets::open_floater(&self.egui_ctx, window::WINDOW, &mut true);
        }
    }

    /// The camera, the isolation, the light and the pose back as they were;
    /// the window stays for the previous outfit's question when the outfit
    /// was worn from it.
    fn end_outfit_portrait_photo(&mut self) {
        let Some(s) = &mut self.outfit_portrait else {
            return;
        };
        if s.over {
            return;
        }
        s.over = true;
        s.pose = None;
        s.preview = None;
        s.shot = None;
        s.hd_confirm = None;
        s.shot_wanted = false;
        self.camera = s.camera.clone();
        self.world.agent.yaw = s.yaw;
        self.world.agent.pitch = s.pitch;
        // (bumped past what it is now: the scene flags every object again)
        let version = self.world.photo_isolation.version;
        self.world.photo_isolation = s.isolation.clone();
        self.world.photo_isolation.version = version.wrapping_add(1);
        if s.kind == Kind::Viewfinder && !s.was_open {
            self.panels.snapshot = false;
        }
        // the choices kept for the next portrait
        if self.settings.outfit_portrait != s.settings {
            self.settings.outfit_portrait = s.settings;
            self.settings.save();
        }
        let ask = s.previous.is_some();
        log::info!("outfit portrait: camera, pose and render restored");
        if !ask {
            self.outfit_portrait = None;
        }
    }

    pub(super) fn close_outfit_portrait(&mut self) {
        self.end_outfit_portrait_photo();
        if self.outfit_portrait.take().is_some() {
            log::info!("outfit portrait: closed");
        }
    }

    /// The auto portrait isolates the photo with the photo mode closed.
    pub(super) fn outfit_portrait_studio(&self) -> bool {
        self.outfit_portrait.as_ref().is_some_and(|s| s.kind == Kind::Studio && !s.over)
    }

    /// The photo mode is the outfit's square.
    pub(super) fn outfit_photo_mode(&self) -> bool {
        self.panels.snapshot && self.outfit_portrait.as_ref().is_some_and(|s| s.kind == Kind::Viewfinder && !s.over)
    }

    /// The studio light over the user's while the auto portrait is open.
    pub(super) fn outfit_portrait_light(&self) -> Option<crate::world::eep_photo::PhotoChoice> {
        let s = self.outfit_portrait.as_ref()?;
        (s.kind == Kind::Studio && !s.over && s.settings.lighting == Lighting::Studio).then(|| crate::world::eep_photo::PhotoChoice {
            time: Some(portrait::STUDIO_TIME),
            sun: s.sun,
            ..Default::default()
        })
    }

    /// The photo mode's shutter: the square photo, shown for confirmation.
    pub(super) fn outfit_portrait_shoot(&mut self) {
        if let Some(s) = &mut self.outfit_portrait
            && !s.shooting
        {
            s.shot_wanted = true;
        }
    }

    /// Where our avatar's joints are drawn.
    fn portrait_joints(&self) -> Option<portrait::Joints> {
        let now = Instant::now();
        let id = self.world.agent_id;
        let at = |name: &str| self.scene.joint_world(&self.world, &id, name, now);
        let facing = self
            .world
            .objects
            .index_of_uuid(&id)
            .and_then(|index| crate::scene::Scene::object_transform(&self.world, index, now, 0))
            .map(|(_, rotation, _)| rotation * Vec3::X)
            .filter(|at| at.truncate().length_squared() > 1e-6)
            .map_or(self.world.agent.yaw, |at| at.y.atan2(at.x));
        Some(portrait::Joints {
            skull: at("mSkull")?,
            head: at("mHead")?,
            neck: at("mNeck")?,
            chest: at("mChest")?,
            pelvis: at("mPelvis")?,
            feet: portrait::FEET.iter().filter_map(|n| at(n)).collect(),
            arms: portrait::ARMS.iter().filter_map(|n| at(n)).collect(),
            facing,
        })
    }

    /// Every frame, before the camera is read: the isolation, the framing,
    /// the previews and the photo.
    pub(super) fn outfit_portrait_frame(&mut self, renderer: &Renderer) {
        let Some(s) = &self.outfit_portrait else {
            return;
        };
        if s.over {
            return;
        }
        // the photo mode closed by its own ways (Esc, its X)
        if s.kind == Kind::Viewfinder && !self.panels.snapshot {
            self.close_outfit_portrait();
            return;
        }
        // the auto portrait leaves the photo mode to itself
        if s.kind == Kind::Studio && self.panels.snapshot {
            self.close_outfit_portrait();
            return;
        }
        if s.kind == Kind::Studio {
            let (mode, transparent) = match s.settings.backdrop {
                Backdrop::Scene if s.settings.hide_others => (Mode::OthersHidden, false),
                Backdrop::Scene => (Mode::Off, false),
                _ => (Mode::Own, true),
            };
            let iso = &mut self.world.photo_isolation;
            iso.set_picking(false);
            iso.set_mode(mode);
            iso.set_transparent(transparent);
        }
        let fov = self.camera.fov_y;
        let joints = self.portrait_joints();
        let Some(s) = &mut self.outfit_portrait else {
            return;
        };
        // a change of the framing's choices: the camera follows again
        let framing = (s.settings.framing, s.settings.angle, s.settings.pose);
        if s.kind == Kind::Studio && s.framed.as_ref().map(|f| (f.framing, f.angle, f.pose)) != Some(framing) {
            s.reframe = Some(Instant::now() + REFRAME);
            s.sun_due = true;
        }
        if s.framed.as_ref() != Some(&s.settings) {
            s.framed = Some(s.settings);
            s.preview_at = None;
        }
        if s.reframe.is_some_and(|until| Instant::now() < until)
            && s.shot.is_none()
            && !s.shooting
            && let Some(j) = &joints
        {
            let (eye, target) = portrait::frame(j, s.settings.framing, s.settings.angle, fov);
            if std::mem::take(&mut s.sun_due) {
                // the key light behind the camera, to its side and high
                let toward = (eye - target).truncate();
                let yaw = toward.y.atan2(toward.x) + KEY_SIDE.to_radians();
                let yaw = yaw.to_degrees().round().to_radians();
                let dir = Vec3::new(yaw.cos(), yaw.sin(), KEY_HEIGHT.to_radians().tan()).normalize();
                s.sun = Some(Quat::from_rotation_arc(Vec3::X, dir));
                log::info!(
                    "outfit portrait: framed {:?} {:?} at {:.2} m",
                    s.settings.framing,
                    s.settings.angle,
                    (eye - target).length()
                );
            }
            self.camera.set_view(eye, target, &mut self.world.agent, &self.settings.camera);
        }
        let Some(s) = &mut self.outfit_portrait else {
            return;
        };
        if self.snapshots.drawing() || self.inventory_images.photo.is_some() {
            return;
        }
        if s.shot_wanted {
            s.shot_wanted = false;
            s.shooting = true;
            log::info!("outfit portrait: photo");
            self.photo_job(Purpose::Portrait { preview: false }, renderer);
            return;
        }
        let view = (self.camera.position, self.camera.target);
        let moving = s.reframe.is_some_and(|until| Instant::now() < until)
            || s.preview_view
                .is_none_or(|(at, to)| at.distance(view.0) > 1e-3 || to.distance(view.1) > 1e-3);
        let every = if moving { PREVIEW_MOVING } else { PREVIEW_STILL };
        let due = s.preview_at.is_none_or(|t| t.elapsed() >= every);
        if s.kind == Kind::Studio && s.shot.is_none() && !s.shooting && due {
            s.preview_at = Some(Instant::now());
            s.preview_view = Some(view);
            self.photo_job(Purpose::Portrait { preview: true }, renderer);
        }
    }

    /// After the scene's poser layers: our avatar's local pose.
    pub(super) fn outfit_portrait_pose(&mut self) {
        let Some(s) = &mut self.outfit_portrait else {
            return;
        };
        let lowered = match s.kind {
            _ if s.over => None,
            Kind::Studio => portrait::arms_lowered(s.settings.pose),
            Kind::Viewfinder => s.vf_pose.then(|| portrait::arms_lowered(portrait::Pose::A)).flatten(),
        };
        let id = self.world.agent_id;
        // the poser's own pose of our avatar is "the current pose"
        let Some(lowered) = lowered.filter(|_| !self.scene.poser.contains_key(&id)) else {
            s.pose = None;
            return;
        };
        if s.pose.as_ref().is_none_or(|(a, _)| *a != lowered) {
            s.pose = crate::ik_edit::poser::standing_pose(&self.world, id, lowered).map(|p| (lowered, p));
            if s.pose.is_some() {
                log::info!("outfit portrait: local pose, arms lowered {:.0}°", lowered.to_degrees());
            }
        }
        if let Some((_, pose)) = &s.pose {
            self.scene.poser.insert(id, pose.clone());
        }
    }

    /// A portrait photo drawn: the preview, or the photo to confirm.
    pub(super) fn outfit_portrait_drawn(&mut self, preview: bool, size: (u32, u32), mut pixels: Vec<u8>, cutout: bool) {
        let Some(s) = &mut self.outfit_portrait else {
            return;
        };
        if cutout {
            // the cut-out avatar over the backdrop chosen (grey when the
            // photo mode's isolation cut it out with the scene chosen)
            let rgb = portrait::backdrop_rgb(s.settings.backdrop)
                .or(portrait::backdrop_rgb(Backdrop::Grey))
                .unwrap_or([0, 0, 0]);
            portrait::flatten(&mut pixels, rgb);
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([size.0 as usize, size.1 as usize], &pixels);
        if preview {
            if s.over || s.shot.is_some() {
                return;
            }
            match &mut s.preview {
                Some(t) => t.set(image, egui::TextureOptions::LINEAR),
                None => {
                    s.preview = Some(
                        self.egui_ctx
                            .load_texture("outfit_portrait_preview", image, egui::TextureOptions::LINEAR),
                    );
                }
            }
            return;
        }
        s.shooting = false;
        if s.over {
            return;
        }
        log::info!("outfit portrait: {} x {} photo taken, awaiting confirmation", size.0, size.1);
        let texture = self
            .egui_ctx
            .load_texture("outfit_portrait_shot", image, egui::TextureOptions::LINEAR);
        s.shot = Some(Shot { size, pixels, texture });
    }

    /// The photo becomes the outfit's picture; with the HD picture chosen,
    /// its price is confirmed first (`confirmed`: the price agreed to).
    fn outfit_portrait_use(&mut self, confirmed: Option<i32>) {
        let Some(s) = &self.outfit_portrait else {
            return;
        };
        if s.shot.is_none() {
            return;
        }
        let side = s.settings.hd_side.side();
        let hd = match self.outfit_hd_offer(side) {
            portrait::HdOffer::Ready(cost) if s.settings.hd && !self.outfit_hd_busy() => Some(cost),
            _ => None,
        };
        let Some(s) = &mut self.outfit_portrait else {
            return;
        };
        // a price not yet agreed to (or changed since): asked first
        if let Some(cost) = hd.filter(|cost| *cost > 0 && confirmed != Some(*cost)) {
            s.hd_confirm = Some(cost);
            return;
        }
        s.hd_confirm = None;
        let Some(shot) = s.shot.take() else {
            return;
        };
        let outfit = s.outfit;
        let hd_pixels = hd.map(|_| shot.pixels.clone());
        if !self.outfit_photo_pixels(outfit, shot.size, shot.pixels) {
            return;
        }
        if let (Some(cost), Some(pixels)) = (hd, hd_pixels) {
            self.outfit_hd_start(outfit, shot.size, pixels, side, cost);
        }
        log::info!("outfit portrait: photo used for {outfit}");
        if let Some(s) = &mut self.outfit_portrait {
            s.saved = true;
        }
        self.end_outfit_portrait_photo();
    }

    fn outfit_portrait_view(&self) -> Option<window::View<'_>> {
        let s = self.outfit_portrait.as_ref()?;
        let inv = &self.world.inventory;
        let name = inv.folders.get(&s.outfit).map_or("", |f| f.info.name.as_str());
        let base = crate::world::appearance::base(inv);
        let image = crate::world::appearance::outfit_image(inv, s.outfit);
        let hd = window::HdView {
            offer: self.outfit_hd_offer(s.settings.hd_side.side()),
            busy: self.outfit_hd_busy(),
            confirm: s.hd_confirm,
            has_one: !crate::world::appearance::hd_pictures(inv, s.outfit).is_empty(),
        };
        Some(window::View {
            name,
            preview: s.preview.as_ref(),
            shot: s.shot.as_ref().map(|shot| &shot.texture),
            current: self.ui_images.get(&image).filter(|_| !image.is_nil()),
            worn: base == Some(s.outfit),
            previous: s
                .previous
                .map(|p| p.and_then(|id| inv.folders.get(&id)).map(|f| f.info.name.as_str())),
            dirty: crate::world::appearance::dirty(inv),
            shooting: s.shooting || s.shot_wanted,
            over: s.over,
            saved: s.saved,
            pose: s.vf_pose,
            studio: s.vf_studio,
            hd,
        })
    }

    /// The auto portrait's window, and after either way of taking the
    /// photo, the previous outfit's question.
    pub(super) fn outfit_portrait_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let Some(s) = &self.outfit_portrait else {
            return;
        };
        if s.kind == Kind::Viewfinder && !s.over {
            return;
        }
        let mut settings = s.settings;
        let mut open = s.open;
        let actions = match self.outfit_portrait_view() {
            Some(view) => window::window(ctx, p, &mut settings, &view, &mut open),
            None => return,
        };
        if let Some(s) = &mut self.outfit_portrait {
            s.settings = settings;
        }
        if !open {
            self.outfit_portrait_action(window::Action::Cancel);
        }
        for action in actions {
            self.outfit_portrait_action(action);
        }
    }

    /// The square photo mode's banner and helpers, over the viewfinder.
    pub(super) fn outfit_photo_mode_ui(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        if !self.outfit_photo_mode() {
            return;
        }
        let Some(mut settings) = self.outfit_portrait.as_ref().map(|s| s.settings) else {
            return;
        };
        let actions = match self.outfit_portrait_view() {
            Some(view) => window::photo_mode(ctx, p, &mut settings, &view),
            None => return,
        };
        if let Some(s) = &mut self.outfit_portrait {
            s.settings = settings;
        }
        for action in actions {
            self.outfit_portrait_action(action);
        }
    }

    fn outfit_portrait_action(&mut self, action: window::Action) {
        let Some(s) = &mut self.outfit_portrait else {
            return;
        };
        let outfit = s.outfit;
        match action {
            window::Action::Shoot => self.outfit_portrait_shoot(),
            window::Action::Use => self.outfit_portrait_use(None),
            window::Action::ConfirmHd => {
                let cost = s.hd_confirm;
                self.outfit_portrait_use(cost);
            }
            window::Action::CancelHd => s.hd_confirm = None,
            window::Action::Retake => {
                s.hd_confirm = None;
                s.shot = None;
                s.preview_at = None;
            }
            window::Action::Cancel => {
                if s.over {
                    self.close_outfit_portrait();
                } else {
                    self.end_outfit_portrait_photo();
                }
            }
            window::Action::Wear => {
                if s.previous.is_none() {
                    s.previous = Some(crate::world::appearance::base(&self.world.inventory));
                }
                s.reframe = Some(Instant::now() + REFRAME);
                log::info!("outfit portrait: wearing {outfit} for the photo");
                self.apply_appearance_action(crate::world::appearance::Action::Wear(outfit, false));
            }
            window::Action::PutBack => {
                if let Some(Some(previous)) = s.previous {
                    log::info!("outfit portrait: the previous outfit {previous} put back");
                    self.apply_appearance_action(crate::world::appearance::Action::Wear(previous, false));
                }
                self.close_outfit_portrait();
            }
            window::Action::Keep => self.close_outfit_portrait(),
            window::Action::PhotoMode => {
                // the same outfit in the square photo mode, the choices kept
                let settings = s.settings;
                let previous = s.previous;
                self.end_outfit_portrait_photo();
                self.outfit_portrait = None;
                self.settings.outfit_portrait = settings;
                self.open_outfit_portrait(outfit, Kind::Viewfinder);
                if let Some(s) = &mut self.outfit_portrait {
                    s.previous = previous;
                }
            }
            window::Action::Reframe => {
                s.reframe = Some(Instant::now() + REFRAME);
                s.settings.framing = portrait::Framing::Body;
                s.settings.angle = portrait::Angle::Front;
            }
            window::Action::Pose => s.vf_pose = !s.vf_pose,
            window::Action::Studio => {
                s.vf_studio = !s.vf_studio;
                let iso = &mut self.world.photo_isolation;
                if s.vf_studio {
                    iso.set_mode(Mode::Own);
                    iso.set_transparent(true);
                } else {
                    iso.set_mode(s.isolation.mode);
                    iso.set_transparent(s.isolation.transparent);
                }
            }
        }
    }
}

/// `AURORA_DEMO_APPEARANCE=portrait|portrait-not-worn|portrait-shot|photo-mode`,
/// and the HD picture's `portrait-hd|portrait-hd-poor|photo-mode-hd`.
fn demo_mode() -> Option<&'static str> {
    static MODE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    MODE.get_or_init(|| {
        std::env::var("AURORA_DEMO_APPEARANCE")
            .ok()
            .filter(|m| m.starts_with("portrait") || m.starts_with("photo-mode"))
    })
    .as_deref()
}

impl App {
    /// The demo's steps (`AURORA_DEMO_APPEARANCE`): over the showcase, at
    /// frame 600 the auto portrait of the outfit worn (`portrait`), of one
    /// not worn (`portrait-not-worn`), the square photo mode
    /// (`photo-mode`); `portrait-shot` takes the photo at frame 900. The HD
    /// picture's: chosen at 1024 with a fictional L$ 500 balance
    /// (`portrait-hd`, `photo-mode-hd`), or a L$ 5 one that cannot pay the
    /// demo's L$ 10 (`portrait-hd-poor`); the demo's upload takes the price
    /// off the balance.
    pub(super) fn outfit_portrait_demo(&mut self) {
        if !self.demo {
            return;
        }
        let Some(mode) = demo_mode() else {
            return;
        };
        let worn = crate::world::appearance::base(&self.world.inventory).unwrap_or(Uuid::from_u128(702));
        match self.frame_count {
            600 => {
                let (outfit, kind) = match mode {
                    "portrait-not-worn" => (
                        [703, 704, 702]
                            .map(Uuid::from_u128)
                            .into_iter()
                            .find(|id| *id != worn)
                            .unwrap_or(worn),
                        Kind::Studio,
                    ),
                    "photo-mode" | "photo-mode-hd" => (worn, Kind::Viewfinder),
                    _ => (worn, Kind::Studio),
                };
                log::info!("demo: outfit portrait ({mode})");
                self.open_outfit_portrait(outfit, kind);
                // `portrait-bust`, `portrait-face`, `portrait-profile`:
                // that framing or angle
                if let Some(s) = &mut self.outfit_portrait {
                    match mode {
                        "portrait-bust" => s.settings.framing = portrait::Framing::Bust,
                        "portrait-face" => s.settings.framing = portrait::Framing::Face,
                        "portrait-profile" => s.settings.angle = portrait::Angle::Profile,
                        _ => {}
                    }
                    if mode.ends_with("-hd") || mode == "portrait-hd-poor" {
                        s.settings.hd = true;
                        s.settings.hd_side = portrait::HdSide::Side1024;
                    }
                }
                match mode {
                    "portrait-hd" | "photo-mode-hd" => self.world.balance = Some(500),
                    "portrait-hd-poor" => self.world.balance = Some(5),
                    _ => {}
                }
            }
            900 if mode == "portrait-shot" => self.outfit_portrait_shoot(),
            _ => {}
        }
    }
}
