//! Photos in the app (LLFloaterSnapshot, LLSnapshotLivePreview,
//! LLViewerWindow::rawSnapshot / saveImageLocal, indra/newview, originally
//! LGPL 2.1): a photo is a job of one or more frames drawn offscreen at its
//! size (`Renderer::snapshot_request`), stitched when it is drawn in tiles,
//! then a preview, a held freeze frame, a file written on a worker, or a
//! texture encoded on a worker and uploaded to the Photo Album
//! (LLPanelSnapshotInventory::onSend, LLSnapshotLivePreview::saveTexture),
//! or a PNG posted to the profile feed (LLPanelSnapshotProfile::onSend,
//! LLWebProfile::uploadImage).

use super::App;
use crate::snapshot::looks::{self, Placement, Resolved};
use crate::snapshot::luts::{Imported, LutLibrary};
use crate::snapshot::{self, TilePass, gallery, inventory, profile, save};
use crate::ui::snapshot::{Action, Destination, EmailView, InventoryView, ProfileView};
use crate::ui_sound::UiSound;
use aurora_net::NetCommand;
use aurora_net::inventory::upload::{self as upload, Kind, Prices, Receipt};
use aurora_render::{Renderer, SnapshotSpec};
use glam::Mat4;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The avatar raising its camera (ANIM_AGENT_SNAPSHOT, llanimationstates.cpp).
const ANIM_AGENT_SNAPSHOT: Uuid = Uuid::from_u128(0xeae8905b_271a_99e2_4c0e_31106afd100c);
/// Long side of the preview image.
const PREVIEW_SIDE: u32 = 720;
/// The preview follows the camera once it has been still this long.
const SETTLE: Duration = Duration::from_millis(250);
/// How long a part of a photo waits for a render thread still busy with
/// the frame before (a pipeline made for the photo quality, a slow part)
/// before the photo is given up: far past any frame, short of forever.
const PHOTO_WAIT: Duration = Duration::from_secs(30);
/// Bytes of video memory a pixel of a photo takes while it is drawn (its
/// render targets and readback), besides those per MSAA sample. Measured on
/// 2026-10-14 (demo, full-resolution reflections): about 54 bytes with MSAA
/// 4x, 150 with MSAA 8x; with some room. The photo is tiled when the free
/// memory cannot hold it.
const TARGET_BYTES_PER_PIXEL: u64 = 16;
const TARGET_BYTES_PER_SAMPLE: u64 = snapshot::quality::BYTES_PER_SAMPLE;
/// Bytes a photo may take when the driver does not report its memory.
const DEFAULT_TARGET_BYTES: u64 = 1_600_000_000;

#[derive(Debug, Clone, PartialEq)]
enum Purpose {
    Preview,
    /// To a file; Some: the one chosen in "Save as".
    Save(Option<PathBuf>),
    /// Freeze frame: kept, shown, saved by "Take the photo".
    Hold,
    /// To the inventory, at the confirmed price.
    Inventory(i32),
    /// To the profile feed.
    Profile,
    /// By e-mail.
    Postcard,
}

/// A photo on its way to the inventory or the profile feed.
enum Upload {
    /// Scaled and encoded on a worker.
    Encoding {
        rx: crossbeam_channel::Receiver<Result<Vec<u8>, String>>,
        cost: i32,
    },
    /// Sent (`NetCommand::UploadAsset`), its answer awaited.
    Sent(Uuid),
    /// Encoded, then posted to the web profile on the network's runtime.
    Profile {
        rx: crossbeam_channel::Receiver<ProfileStep>,
        /// The PNG's size, once encoded.
        bytes: Option<usize>,
    },
    /// Encoded, then sent through SendPostcard on the network's runtime.
    Postcard {
        rx: crossbeam_channel::Receiver<PostcardStep>,
        encoded: bool,
    },
}

/// How a postcard goes.
enum PostcardStep {
    Encoded,
    Done(Result<(), aurora_net::postcard::Error>),
}

/// How a profile post goes.
enum ProfileStep {
    Encoded(usize),
    Done(Result<(), profile::Error>),
}

struct Job {
    purpose: Purpose,
    size: (u32, u32),
    tiles: Vec<TilePass>,
    next: usize,
    /// The current tile's frame is asked.
    asked: bool,
    /// Frames the current tile waited for the render thread (said once),
    /// since when.
    waited: u32,
    waiting: Option<Instant>,
    pixels: Vec<u8>,
    interface: bool,
    huds: bool,
    started: Instant,
    /// Drawn at this many times the photo's size, reduced once complete
    /// (the photo quality's supersampling).
    supersample: u32,
    /// Cut out by the isolation (`snapshot::isolate`): its alpha is kept.
    cutout: bool,
    /// The looks it is drawn with, chosen when it started, their strength,
    /// and the focus distance of their depth operations.
    looks: Vec<(Resolved, u8)>,
    focus: Option<f32>,
    /// The photo's size, before its supersampling or the preview's
    /// reduction: the look's pixels.
    photo: (u32, u32),
    /// Drawn in several parts: the instant of the first, which the others
    /// show (`snapshot::Hold`).
    hold: Option<snapshot::Hold>,
}

/// A held photo (freeze frame) and what shows it.
struct Held {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    cutout: bool,
    texture: Option<egui::TextureHandle>,
}

/// The photo frame drawn this frame: how the app lays it out.
pub(super) struct PhotoFrame {
    /// Width over height of the whole photo.
    pub aspect: f32,
    /// Off-centre projection of the tile drawn (identity in one pass).
    pub tile: Mat4,
    /// The photo's height: the levels of detail are chosen for it.
    pub height: u32,
    /// How many times taller it is than the viewfinder's view of it
    /// (`snapshot::viewfinder_scale`).
    pub scale: f32,
    /// Its look, placed for the part drawn.
    pub look: Option<aurora_render::looks::PhotoLook>,
}

type Thumb = (PathBuf, Option<(u32, u32, Vec<u8>)>);

pub(super) struct Snapshots {
    pub ui: crate::ui::snapshot::SnapshotUi,
    job: Option<Job>,
    /// The photo frame of the frame being drawn (its interface hidden).
    pub capturing: Option<SnapshotSpec>,
    preview: Option<egui::TextureHandle>,
    /// What the preview was drawn for (photo size, interface, HUDs).
    preview_for: Option<((u32, u32), bool, bool)>,
    pub(super) preview_wanted: bool,
    camera: Option<[f32; 7]>,
    camera_moved: Option<Instant>,
    saving: Option<save::Saving>,
    message: Option<(String, bool)>,
    last_saved: Option<PathBuf>,
    gallery: Vec<(PathBuf, Option<egui::TextureHandle>)>,
    /// The folder the gallery was read from.
    gallery_folder: Option<PathBuf>,
    thumbs: (crossbeam_channel::Sender<Thumb>, crossbeam_channel::Receiver<Thumb>),
    held: Option<Held>,
    held_view: Option<crossbeam_channel::Receiver<(u32, u32, Vec<u8>)>>,
    dialog: Option<crossbeam_channel::Receiver<(bool, Option<PathBuf>)>>,
    /// The user's 3D tables for the photo's look, opened the first time
    /// they are needed (`luts`).
    luts: Option<LutLibrary>,
    /// A table being imported (its file chosen, read and copied on a
    /// worker; None: the dialog was closed).
    lut_import: Option<crossbeam_channel::Receiver<Option<Imported>>>,
    /// The focus distance of the depth of field last frame, for the depth
    /// looks (None: off, the middle of the photo then).
    pub look_focus: Option<f32>,
    /// The window was closed when the photo was asked (Ctrl+`).
    quiet: bool,
    /// The price to confirm before the upload.
    confirm: Option<i32>,
    /// The price the balance cannot pay.
    short: Option<i32>,
    upload: Option<Upload>,
    /// The message tells of this item, saved to the inventory.
    message_item: Option<Uuid>,
    /// The message tells of a post to the profile feed ("View").
    message_profile: bool,
    /// The photo quality waits since then for the textures in view.
    detail_wait: Option<Instant>,
    /// The wait is over: the next "Take" goes ahead.
    detail_ready: bool,
    /// The photo quality's step for the video memory while it is on
    /// (`snapshot::quality::plan`).
    fit: Option<QualityFit>,
    /// The photo quality was just lowered: the viewfinder says so once.
    fit_note: bool,
    /// The world frozen by the photo mode (`SnapshotUi::freeze_world`):
    /// the instant and the shaders' clock it shows.
    world_freeze: Option<snapshot::Hold>,
    /// The free camera flies, started by the window.
    free_flying: bool,
    /// The Controls window was opened by the free camera: it closes with it.
    free_opened_controls: bool,
    /// The photo quality raised or restored the settings: applied like a
    /// change made in Preferences (renderer, draw distance sent to the
    /// region…).
    pub(super) settings_changed: bool,
    /// The web profile's session cookie, for the OpenID cookie it was
    /// asked with (getOpenIDCookieCoro gives it once per login).
    web_auth: std::sync::Arc<parking_lot::Mutex<Option<(String, String)>>>,
    /// The demo's web profile.
    demo_profile: Option<profile::demo::Server>,
    demo: Option<DemoPhoto>,
    /// The viewfinder's state for the session (`ui::viewfinder`).
    pub(super) vf: crate::ui::viewfinder::Viewfinder,
    /// The focus beep, the self-timer's beep and a lens ring's click, made
    /// on first use.
    pub(super) beep: Option<aurora_audio::SoundClip>,
    pub(super) tick: Option<aurora_audio::SoundClip>,
    pub(super) click: Option<aurora_audio::SoundClip>,
    /// The self-timer's last beep: (its slot, its beat).
    pub(super) vf_tick_slot: Option<(i64, u64)>,
    /// The camera pad rises or lowers the free camera (−1 to 1).
    pub(super) vf_free_up: f32,
    /// The field of view before the viewfinder opened: its focal length
    /// is the photo's, the view gets this back when it closes.
    pub(super) fov_before: Option<f32>,
    /// The last photo's card in the viewfinder: what it says, what it
    /// offers, its thumbnail (made on a worker for the inventory and the
    /// profile feed).
    review: Option<(String, crate::ui::viewfinder::Review)>,
    review_tex: Option<egui::TextureHandle>,
    review_rx: Option<crossbeam_channel::Receiver<(u32, u32, Vec<u8>)>>,
    /// Choosing for the isolation (`app::photo_isolate`): the points asked
    /// of the renderer, this frame's point (window pixels), and the avatar
    /// or linkset under the pointer.
    pub(super) isolate_picks: Vec<(u64, super::photo_isolate::Pick)>,
    pub(super) isolate_point: Option<((f32, f32), super::photo_isolate::Pick)>,
    pub(super) isolate_hover: Option<Uuid>,
}

/// The photo quality's step for the video memory, and when it was read and
/// changed (`App::photo_quality_fit`).
struct QualityFit {
    step: usize,
    checked: Instant,
    changed: Instant,
}

/// The video memory read again this often while the photo quality is on…
const FIT_EVERY: Duration = Duration::from_secs(1);
/// …but not this soon after a step, while the driver's figures follow
/// the render targets just made or freed…
const FIT_SETTLE: Duration = Duration::from_secs(2);
/// …and a step up only this long after the last change.
const FIT_RAISE_AFTER: Duration = Duration::from_secs(5);

/// AURORA_DEMO_SNAPSHOT=quality-tight: the driver's budget as the photo
/// quality reads it, `room` MB above what the viewer held when it came on,
/// `later` MB from AURORA_DEMO_SNAPSHOT_FRAME (the budget shrinking or
/// growing while the window stays open).
struct DemoTight {
    room: u64,
    later: Option<u64>,
    base: Option<u64>,
}

struct DemoPhoto {
    frame: u64,
    /// None: only the window.
    save: Option<((u32, u32), snapshot::FileFormat, Option<PathBuf>)>,
    full_frame: bool,
    aspect: Option<snapshot::Aspect>,
    inventory: Option<DemoInventory>,
    /// AURORA_DEMO_SNAPSHOT=profile | profile-fail: posted to the demo's
    /// web profile (which refuses it with -fail).
    profile: bool,
    /// AURORA_DEMO_SNAPSHOT=postcard | postcard-fail: sent by e-mail, the
    /// demo answering itself (too big with -fail).
    postcard: bool,
    /// AURORA_DEMO_SNAPSHOT=dof | full-dof: the window's depth of field on,
    /// f/1.4, focused by hand on the own avatar (3.3 m).
    dof: bool,
    /// AURORA_DEMO_SNAPSHOT=prefs-dof (or the viewfinder's prefsdof): the
    /// Preferences' depth of field on; prefs-dof leaves the window's at
    /// All sharp.
    prefs_dof: bool,
    /// AURORA_DEMO_SNAPSHOT=quality: the window with its photo quality on.
    quality: bool,
    /// AURORA_DEMO_SNAPSHOT=quality-tight[:<MB>[:<MB later>]]: the same
    /// on a card short of memory.
    tight: Option<DemoTight>,
    /// The photo mode's look for the run (classic, panel, full…,
    /// viewfinder…), the setting itself left alone.
    style: Option<crate::settings::PhotoWindowStyle>,
    /// AURORA_DEMO_SNAPSHOT=viewfinder[-essential|-hidden][-menu|-help|
    /// -drawer][-mf|-afc][-point][-frozen][-focus|-shoot|-group]: the
    /// viewfinder, its display, a panel open, its focus mode, and the own
    /// avatar focused (then the photo taken, or the others in view added)
    /// at AURORA_DEMO_SNAPSHOT_FRAME.
    viewfinder: Option<DemoViewfinder>,
    /// AURORA_DEMO_SNAPSHOT=light: sunset, a brighter exposure, the clouds
    /// still and the worn lights off.
    light: bool,
    /// AURORA_DEMO_SNAPSHOT=windlight[-keep]: the library's "Coastal
    /// Sunset" sky and "Murky Water" with the sun turned low in the east,
    /// the Light section in view; `-keep` keeps that light (frame 900) and
    /// closes the window (960).
    windlight: Option<bool>,
}

/// What AURORA_DEMO_SNAPSHOT=viewfinder… shows.
struct DemoViewfinder {
    display: crate::snapshot::viewfinder::Display,
    menu: bool,
    help: bool,
    drawer: bool,
    light: bool,
    focus_mode: Option<crate::dof::PhotoFocus>,
    point: bool,
    focus: bool,
    shoot: bool,
    /// After the own avatar's focus, the other avatars in view added to
    /// it (Shift + click: the group focus).
    group: bool,
    /// The aperture ring on All sharp (the window's depth of field off).
    sharp: bool,
    /// The camera pad, the self-timer (2 s), AE-L, the controls on hover
    /// or hidden.
    pad: bool,
    timer: bool,
    ael: bool,
    /// The world frozen (avatars, animations, objects, particles…).
    frozen: bool,
    controls: crate::snapshot::viewfinder::OnScreen,
    /// The camera tilted clockwise (degrees; 0: straight): 20° with
    /// `tilt`, a quarter turn with `tilt90` (the level turns the other
    /// way, the AF frame keeps the head's size).
    tilt: f32,
    /// The poser opened over the viewfinder (the dock's Poser, #1389).
    poser: bool,
}

/// AURORA_DEMO_SNAPSHOT=inventory…: the photo sent to the inventory.
struct DemoInventory {
    size: inventory::TextureSize,
    /// The price is confirmed 60 frames after it is asked.
    confirm: bool,
    /// A balance below the price.
    poor: bool,
}

impl Default for Snapshots {
    fn default() -> Self {
        Snapshots {
            ui: Default::default(),
            job: None,
            capturing: None,
            preview: None,
            preview_for: None,
            preview_wanted: false,
            camera: None,
            camera_moved: None,
            saving: None,
            message: None,
            last_saved: None,
            gallery: Vec::new(),
            gallery_folder: None,
            thumbs: crossbeam_channel::unbounded(),
            held: None,
            held_view: None,
            dialog: None,
            luts: None,
            lut_import: None,
            look_focus: None,
            quiet: false,
            confirm: None,
            short: None,
            upload: None,
            message_item: None,
            message_profile: false,
            detail_wait: None,
            detail_ready: false,
            fit: None,
            fit_note: false,
            world_freeze: None,
            free_flying: false,
            free_opened_controls: false,
            settings_changed: false,
            web_auth: Default::default(),
            demo_profile: None,
            demo: None,
            vf: Default::default(),
            beep: None,
            tick: None,
            click: None,
            vf_tick_slot: None,
            vf_free_up: 0.0,
            fov_before: None,
            review: None,
            review_tex: None,
            review_rx: None,
            isolate_picks: Vec::new(),
            isolate_point: None,
            isolate_hover: None,
        }
    }
}

impl App {
    /// At the start of a frame: the instant the world shows while a photo
    /// is drawn in several parts, the first part's from its frame to the
    /// last part's (`snapshot::Hold`); None otherwise.
    /// The world frozen by the photo mode shows its instant throughout,
    /// the photo's parts included.
    pub(super) fn photo_hold(&mut self, now: Instant) -> Option<snapshot::Hold> {
        let clock = self.start.elapsed().as_secs_f32();
        let frozen = self.photo_freeze_world(now, clock);
        let Some(job) = self.snapshots.job.as_mut() else {
            return frozen;
        };
        let now = frozen.unwrap_or(snapshot::Hold {
            at: now,
            clock,
            later: false,
        });
        snapshot::Hold::for_part(job.tiles.len(), job.next, &mut job.hold, now).or(frozen)
    }

    /// The world frozen for the photo (issue #1154), from the switch to the
    /// photo mode's end: what moves holds still (avatars, animations,
    /// objects, particles, flexible prims, animated textures, water,
    /// clouds), the camera flies on. LLFloater360Capture::freezeWorld
    /// (indra/newview/llfloater360capture.cpp, originally LGPL 2.1) pauses
    /// every character, sets FreezeTime and stops the particles and the
    /// clouds the same way.
    fn photo_freeze_world(&mut self, now: Instant, clock: f32) -> Option<snapshot::Hold> {
        if !self.panels.snapshot || !self.in_world() {
            self.snapshots.ui.freeze_world = false;
        }
        let wanted = self.snapshots.ui.freeze_world;
        if wanted != self.snapshots.world_freeze.is_some() {
            self.snapshots.world_freeze = wanted.then_some(snapshot::Hold {
                at: now,
                clock,
                later: false,
            });
            self.world.freeze_motion(wanted.then_some(now));
            // every avatar and animated object holds its pose
            // (LLCharacter::requestPause)
            self.scene.pause_all_skeletons = wanted;
        }
        self.snapshots.world_freeze
    }

    /// Whether the photo mode holds the world still.
    pub(super) fn photo_world_frozen(&self) -> bool {
        self.snapshots.world_freeze.is_some()
    }

    /// Before a frame is laid out: the photo frame it carries, if any.
    pub(super) fn photo_frame(&mut self, renderer: &mut Renderer) -> Option<PhotoFrame> {
        let s = &mut self.snapshots;
        s.capturing = None;
        let job = s.job.as_mut()?;
        if job.asked {
            if renderer.failed() || renderer.device_lost() {
                // the frame asked never came back (the render thread is gone)
                log::warn!("photo: the frame of part {} was not drawn", job.next + 1);
                s.job = None;
                s.message = Some((aurora_i18n::tr!("photo-draw-failed").to_owned(), true));
                return None;
            }
            // the GPU still had the frame before (the photo quality just
            // switched on, a part that took long): no frame was built and the
            // part is still asked, so this frame carries it again
            let since = *job.waiting.get_or_insert_with(Instant::now);
            if since.elapsed() > PHOTO_WAIT {
                // a GPU that never finishes, though not lost: the photo
                // is given up, not drawn later for nobody
                log::warn!(
                    "photo: part {} waited {:?} for the render thread: given up",
                    job.next + 1,
                    PHOTO_WAIT
                );
                renderer.drop_snapshot();
                s.job = None;
                s.message = Some((aurora_i18n::tr!("photo-draw-failed").to_owned(), true));
                return None;
            }
            if job.waited == 0 {
                log::info!("photo: part {} waits for the render thread", job.next + 1);
            }
            job.waited += 1;
        }
        let tile = job.tiles[job.next].draw;
        let single = job.tiles.len() == 1;
        let spec = SnapshotSpec {
            width: tile.width,
            height: tile.height,
            // tiles are laid out for the whole photo: no interface or HUD,
            // which the window places on its own screen
            interface: job.interface && single,
            huds: job.huds && single,
        };
        renderer.snapshot_request = Some(spec);
        job.asked = true;
        s.capturing = Some(spec);
        // the look in the photo's own pixels, from the corner of the part
        // drawn; the tiles share the histogram of the live view's last frame
        let place = Placement {
            origin: [-(tile.x as f32), -(tile.y as f32)],
            photo: job.photo,
            scale: job.size.1 as f32 / job.photo.1.max(1) as f32,
        };
        let look = looks::frame_look(&job.looks, place, None, !single, job.focus);
        Some(PhotoFrame {
            aspect: job.size.0 as f32 / job.size.1.max(1) as f32,
            tile: if single {
                Mat4::IDENTITY
            } else {
                snapshot::tile_matrix(job.size, tile)
            },
            height: job.size.1,
            scale: snapshot::viewfinder_scale(job.size, renderer.size(), self.settings.render_scale_percent),
            look,
        })
    }

    /// After a frame was drawn: the part of the photo it carried.
    pub(super) fn photo_drawn(&mut self, renderer: &mut Renderer) {
        let Some(answer) = renderer.snapshot.take() else {
            return;
        };
        let Some(job) = self.snapshots.job.as_mut() else {
            return;
        };
        job.asked = false;
        if job.waited > 0 {
            log::info!("photo: part {} drawn after {} frame(s) of waiting", job.next + 1, job.waited);
            job.waited = 0;
        }
        job.waiting = None;
        let pass = job.tiles[job.next];
        let Some((_, _, pixels)) = answer.filter(|(w, h, _)| (*w, *h) == (pass.draw.width, pass.draw.height)) else {
            log::warn!("photo: part {} of {} not drawn", job.next + 1, job.tiles.len());
            self.snapshots.job = None;
            self.snapshots.message = Some((aurora_i18n::tr!("photo-draw-failed").to_owned(), true));
            return;
        };
        if job.tiles.len() == 1 {
            job.pixels = pixels;
        } else {
            if job.pixels.is_empty() {
                job.pixels = vec![0; job.size.0 as usize * job.size.1 as usize * 4];
            }
            snapshot::stitch(&mut job.pixels, job.size.0, pass, &pixels);
        }
        job.next += 1;
        if job.next < job.tiles.len() {
            return;
        }
        let Some(mut job) = self.snapshots.job.take() else {
            return;
        };
        let render_ms = job.started.elapsed().as_millis();
        if job.purpose != Purpose::Preview {
            log::info!(
                "photo: {} x {} drawn in {} part(s), {render_ms} ms",
                job.size.0,
                job.size.1,
                job.tiles.len()
            );
        }
        if job.supersample > 1 {
            // back to the photo's size: each pixel the mean of the ones drawn
            let t0 = Instant::now();
            let (w, h) = (job.size.0 / job.supersample, job.size.1 / job.supersample);
            let (_, _, px) = save::downscale(job.size.0, job.size.1, &job.pixels, w.max(h));
            job.size = (w, h);
            job.pixels = px;
            log::info!("photo: reduced to {w} x {h} in {} ms", t0.elapsed().as_millis());
        }
        match job.purpose {
            Purpose::Preview => {
                let image = egui::ColorImage::from_rgba_unmultiplied([job.size.0 as usize, job.size.1 as usize], &job.pixels);
                self.snapshots.preview = Some(self.egui_ctx.load_texture("photo_preview", image, egui::TextureOptions::LINEAR));
            }
            Purpose::Hold => {
                let (tx, rx) = crossbeam_channel::bounded(1);
                let (w, h) = job.size;
                let rgba = job.pixels.clone();
                // shown at the screen's size, reduced on a worker
                let side = renderer.size().0.max(renderer.size().1);
                std::thread::spawn(move || {
                    let _ = tx.send(save::downscale(w, h, &rgba, side));
                });
                self.snapshots.held_view = Some(rx);
                self.snapshots.held = Some(Held {
                    width: w,
                    height: h,
                    rgba: job.pixels,
                    cutout: job.cutout,
                    texture: None,
                });
                self.photo_shutter();
            }
            Purpose::Save(path) => self.photo_save(job.size, job.pixels, path, job.cutout),
            Purpose::Inventory(cost) => self.photo_encode(job.size, job.pixels, cost),
            Purpose::Profile => self.photo_post(job.size, job.pixels),
            Purpose::Postcard => self.photo_postcard(job.size, job.pixels),
        }
    }

    /// The size of the photo for the chosen destination: the disk's
    /// settings, or the inventory's size before it becomes a texture.
    pub(super) fn photo_size(&self, window: (u32, u32)) -> (u32, u32) {
        let s = &self.settings.snapshot;
        match self.snapshots.ui.destination {
            Destination::Disk => s.size(window),
            Destination::Inventory => inventory::capture_size(s.aspect, s.inventory_size, window),
            Destination::Profile => profile::capture_size(s.aspect, s.profile_size, window),
            Destination::Email => profile::capture_size(s.aspect, s.postcard_size, window),
        }
    }

    /// The web profile a post goes to: the grid's with the login's OpenID
    /// cookie, or the demo's server.
    fn photo_profile_web(&mut self) -> Option<profile::Web> {
        let user_agent = crate::media::user_agent();
        if self.demo {
            if self.snapshots.demo_profile.is_none() {
                let fail = std::env::var("AURORA_DEMO_SNAPSHOT").is_ok_and(|m| m == "profile-fail");
                match profile::demo::start(fail) {
                    Ok(server) => self.snapshots.demo_profile = Some(server),
                    Err(e) => log::warn!("photo: demo web profile not started: {e}"),
                }
            }
            let server = self.snapshots.demo_profile.as_ref()?;
            return Some(profile::Web {
                profile: server.url.clone(),
                openid: profile::demo::OPENID.into(),
                user_agent,
            });
        }
        let login = self.world.login.as_ref()?;
        let base = profile::grid_web_profile(&self.settings.grid, &login.raw["web_profile_url"].to_string_value())?;
        Some(profile::Web {
            profile: profile::profile_url(&base)?,
            openid: self.media.openid.raw()?,
            user_agent,
        })
    }

    /// What the window shows of a photo for the profile feed.
    fn photo_profile_view(&mut self, window: (u32, u32)) -> ProfileView {
        ProfileView {
            connected: self.demo || self.world.login.is_some(),
            ready: self.demo || (self.world.login.is_some() && self.media.openid.raw().is_some()),
            size: self.photo_size(window),
        }
    }

    /// "Post" (LLPanelSnapshotProfile::onSend): the held photo or a new one.
    fn photo_to_profile(&mut self, renderer: &Renderer) {
        if self.photo_busy() {
            return;
        }
        if let Some(held) = &self.snapshots.held {
            let (size, rgba) = ((held.width, held.height), held.rgba.clone());
            self.photo_post(size, rgba);
            return;
        }
        self.photo_job(Purpose::Profile, renderer);
    }

    /// Encode the PNG and post it, off the main thread: the session cookie
    /// of the web profile first when it is not known for this login.
    fn photo_post(&mut self, size: (u32, u32), rgba: Vec<u8>) {
        let Some(web) = self.photo_profile_web() else {
            self.snapshots.message = Some((aurora_i18n::tr!("photo-profile-not-ready").to_owned(), true));
            return;
        };
        self.photo_review_thumb(size, &rgba);
        let caption: String = self.snapshots.ui.caption.chars().take(profile::MAX_CAPTION).collect();
        let add_location = self.settings.snapshot.profile_location;
        let auth = self.snapshots.web_auth.clone();
        let (tx, rx) = crossbeam_channel::unbounded();
        self.net.runtime().spawn(async move {
            let encoded = tokio::task::spawn_blocking(move || profile::encode_png(size.0, size.1, &rgba)).await;
            let png = match encoded {
                Ok(Ok(png)) => png,
                Ok(Err(e)) => {
                    log::warn!("photo: PNG not encoded: {e}");
                    let _ = tx.send(ProfileStep::Done(Err(profile::Error::ConfigData)));
                    return;
                }
                Err(_) => return,
            };
            let _ = tx.send(ProfileStep::Encoded(png.len()));
            let known = auth
                .lock()
                .as_ref()
                .filter(|(openid, _)| *openid == web.openid)
                .map(|(_, a)| a.clone());
            let cookie = match known {
                Some(cookie) => cookie,
                None => match profile::auth_cookie(&web).await {
                    Ok(cookie) => {
                        *auth.lock() = Some((web.openid.clone(), cookie.clone()));
                        cookie
                    }
                    Err(e) => {
                        let _ = tx.send(ProfileStep::Done(Err(e)));
                        return;
                    }
                },
            };
            let result = profile::post(&web, &cookie, png, &caption, add_location).await;
            let _ = tx.send(ProfileStep::Done(result));
        });
        self.snapshots.upload = Some(Upload::Profile { rx, bytes: None });
        self.photo_shutter();
    }

    /// What the window shows of a postcard: the region takes them
    /// (SendPostcard), or the demo.
    fn photo_email_view(&self, window: (u32, u32)) -> EmailView {
        EmailView {
            available: self.demo || self.world.main_has_cap("SendPostcard"),
            size: self.photo_size(window),
            sender: self.photo_postcard_sender(),
        }
    }

    /// "Send" (LLPanelSnapshotPostcard::onSend): the recipients checked
    /// first (PromptRecipientEmail), then the held photo or a new one.
    fn photo_to_postcard(&mut self, renderer: &Renderer) {
        if self.photo_busy() {
            return;
        }
        if !aurora_net::postcard::valid_recipients(&self.snapshots.ui.postcard.to) {
            let text = aurora_i18n::tr!("photo-email-bad-recipient");
            self.snapshots.message = Some((text.to_owned(), true));
            self.photo_vf_error(text);
            return;
        }
        if let Some(held) = &self.snapshots.held {
            let (size, rgba) = ((held.width, held.height), held.rgba.clone());
            self.photo_postcard(size, rgba);
            return;
        }
        self.photo_job(Purpose::Postcard, renderer);
    }

    /// Our full name, the sender's while "From" is empty
    /// (LLAgentUI::buildFullname).
    fn photo_postcard_sender(&self) -> String {
        self.world
            .legacy_name(&self.world.agent_id)
            .unwrap_or_else(|| self.world.own_name())
    }

    /// The postcard's fields, with the defaults of an empty name, subject
    /// or message (LLPanelSnapshotPostcard::onOpen, missingSubjMsgAlertCallback).
    fn photo_postcard_fields(&self) -> aurora_net::postcard::Postcard {
        let form = &self.snapshots.ui.postcard;
        let or = |text: &str, default: &str| {
            if text.trim().is_empty() {
                default.to_owned()
            } else {
                text.to_owned()
            }
        };
        let own = self.photo_postcard_sender();
        // the sender's address, off Second Life only (OpenSim compatibility)
        let from = (self.settings.grid == crate::settings::GridChoice::Custom)
            .then(|| self.world.user_info.info.as_ref().map(|i| i.email.clone()))
            .flatten()
            .filter(|e| !e.is_empty());
        aurora_net::postcard::Postcard {
            to: form.to.trim().to_owned(),
            name: or(&form.name, &own),
            subject: or(&form.subject, aurora_i18n::tr!("photo-email-default-subject")),
            message: or(&form.message, aurora_i18n::tr!("photo-email-default-message")),
            pos_global: self.world.agent_global().map_or([0.0; 3], |g| g.to_array()),
            from,
        }
    }

    /// Encode the JPEG and send the postcard, off the main thread; the
    /// demo has no simulator and answers itself (AURORA_DEMO_SNAPSHOT=
    /// postcard-fail: refused).
    fn photo_postcard(&mut self, size: (u32, u32), rgba: Vec<u8>) {
        let url = self.world.main().and_then(|r| r.caps.get("SendPostcard").cloned());
        if url.is_none() && !self.demo {
            self.snapshots.message = Some((aurora_i18n::tr!("photo-email-unavailable").to_owned(), true));
            return;
        }
        self.photo_review_thumb(size, &rgba);
        let postcard = self.photo_postcard_fields();
        let quality = self.settings.snapshot.jpeg_quality;
        let demo_fail = self.demo && std::env::var("AURORA_DEMO_SNAPSHOT").is_ok_and(|m| m == "postcard-fail");
        let http = self.net.caps_http();
        let (tx, rx) = crossbeam_channel::unbounded();
        self.net.runtime().spawn(async move {
            let encoded = tokio::task::spawn_blocking(move || save::encode_jpeg(size.0, size.1, &rgba, quality)).await;
            let jpeg = match encoded {
                Ok(Ok(jpeg)) => jpeg,
                Ok(Err(e)) => {
                    log::warn!("photo: JPEG not encoded: {e}");
                    let _ = tx.send(PostcardStep::Done(Err(aurora_net::postcard::Error::Failed(e))));
                    return;
                }
                Err(_) => return,
            };
            let _ = tx.send(PostcardStep::Encoded);
            log::info!("photo: postcard of {} bytes", jpeg.len());
            let result = match url {
                Some(url) => aurora_net::postcard::send(&http, &url, &postcard, jpeg).await,
                None if demo_fail => Err(aurora_net::postcard::Error::TooBig),
                None => Ok(()),
            };
            let _ = tx.send(PostcardStep::Done(result));
        });
        self.snapshots.upload = Some(Upload::Postcard { rx, encoded: false });
        self.photo_shutter();
    }

    /// The postcard's progress and outcome (LLPostCard::reportPostResult).
    fn photo_postcard_poll(&mut self) {
        let Some(Upload::Postcard { rx, encoded }) = &mut self.snapshots.upload else {
            return;
        };
        let mut done = None;
        while let Ok(step) = rx.try_recv() {
            match step {
                PostcardStep::Encoded => *encoded = true,
                PostcardStep::Done(result) => done = Some(result),
            }
        }
        let Some(result) = done else {
            return;
        };
        self.snapshots.upload = None;
        let s = &mut self.snapshots;
        s.message_item = None;
        s.message_profile = false;
        match result {
            Ok(()) => {
                log::info!("photo: postcard sent");
                s.message = Some((aurora_i18n::tr!("photo-email-sent").to_owned(), false));
                self.photo_review_show(
                    aurora_i18n::tr!("photo-destination-email").to_owned(),
                    crate::ui::viewfinder::Review::Email,
                );
            }
            Err(e) => {
                log::warn!("photo: postcard not sent: {e}");
                // CannotUploadSnapshotEmailTooBig
                let text = match e {
                    aurora_net::postcard::Error::TooBig => aurora_i18n::tr!("photo-email-too-big"),
                    aurora_net::postcard::Error::Failed(_) => aurora_i18n::tr!("photo-email-failed"),
                };
                s.message = Some((text.to_owned(), true));
                self.photo_vf_error(text);
            }
        }
    }

    /// Start a photo of the destination's size.
    fn photo_job(&mut self, purpose: Purpose, renderer: &Renderer) {
        let window = renderer.size();
        let full = self.photo_size(window);
        let s = &self.settings.snapshot;
        let size = if purpose == Purpose::Preview {
            let (w, h) = full;
            let scale = (PREVIEW_SIDE as f32 / w.max(h) as f32).min(1.0);
            (
                ((w as f32 * scale).round() as u32).max(16),
                ((h as f32 * scale).round() as u32).max(16),
            )
        } else {
            full
        };
        // the photo quality draws a photo (not its preview) at twice its size
        let supersample = if purpose != Purpose::Preview && self.settings.photo_held.is_some() {
            snapshot::quality::supersample(size)
        } else {
            1
        };
        let size = (size.0 * supersample, size.1 * supersample);
        // the choosing in the scene stops: its outlines are not in the
        // photo, which shows what is chosen (the scene follows before the
        // photo's frame is drawn)
        if purpose != Purpose::Preview {
            self.world.photo_isolation.set_picking(false);
        }
        let tiles = self.photo_tiles(renderer, size);
        let interface = s.interface && snapshot::same_proportions(full, window);
        self.snapshots.job = Some(Job {
            purpose,
            size,
            tiles,
            next: 0,
            asked: false,
            waited: 0,
            waiting: None,
            pixels: Vec::new(),
            interface,
            huds: s.huds,
            started: Instant::now(),
            supersample,
            cutout: self.world.photo_isolation.empty_background(),
            looks: Vec::new(),
            focus: self.snapshots.look_focus,
            photo: full,
            hold: None,
        });
        let stack = self.settings.snapshot.looks.clone();
        let settings = self.settings.snapshot.look_settings.clone();
        let resolved = looks::resolve_stack(&stack, &settings, self.photo_luts());
        if let Some(job) = &mut self.snapshots.job {
            job.looks = resolved;
        }
    }

    /// The library of the user's 3D tables.
    pub(super) fn photo_luts(&mut self) -> &mut LutLibrary {
        self.snapshots
            .luts
            .get_or_insert_with(|| LutLibrary::new(crate::settings::config_dir().join("photo-luts")))
    }

    /// The look of the live view (`snapshot::looks`) while the photo
    /// window is open: the filter pointed at in a list, or the one chosen,
    /// over the photo's frame of the `window` (pixels; `ui::snapshot::fit`),
    /// with the viewfinder's before / after line.
    pub(super) fn live_look(&mut self, window: (u32, u32), focus: Option<f32>) -> Option<aurora_render::looks::PhotoLook> {
        if !self.panels.snapshot {
            return None;
        }
        let ui = &self.snapshots.ui.looks;
        let stack = looks::tried(
            &self.settings.snapshot.looks,
            ui.hover.as_ref().map(|h| h.as_deref()),
            ui.mode,
            ui.selected,
        );
        let settings = self.settings.snapshot.look_settings.clone();
        let layers = looks::resolve_stack(&stack, &settings, self.photo_luts());
        let photo = self.photo_size(window);
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(window.0 as f32, window.1 as f32));
        let frame = crate::ui::snapshot::fit(screen, photo.0 as f32 / photo.1.max(1) as f32);
        let place = Placement {
            origin: [frame.left(), frame.top()],
            photo,
            scale: frame.height() / photo.1.max(1) as f32,
        };
        let compare = self.snapshots.ui.looks.compare.filter(|_| self.viewfinder_on());
        looks::frame_look(&layers, place, compare, false, focus)
    }

    /// How a photo of `size` is drawn (`snapshot::plan`), with the guard
    /// band its effects need: the bloom and the depth of field (at the blur
    /// this photo gets, its `max_coc` grows with its height) read the image
    /// farther than the screen-space passes, and the contact shadows as far
    /// as their rays reach at this photo's scale.
    fn photo_tiles(&self, renderer: &Renderer, size: (u32, u32)) -> Vec<snapshot::TilePass> {
        let bloom = if self.settings.bloom { self.settings.bloom_strength } else { 0.0 };
        let dof = self
            .photo_dof_max_coc(size.1, renderer.size().1)
            .map_or(0, aurora_render::dof_tile_reach);
        let scale = snapshot::viewfinder_scale(size, renderer.size(), self.settings.render_scale_percent);
        let contact = aurora_render::contact_tile_reach(self.settings.contact_shadows, scale);
        let guard = snapshot::TILE_GUARD
            .max(aurora_render::bloom_tile_reach(bloom))
            .max(dof)
            .max(contact);
        snapshot::plan(size, self.photo_max_side(renderer), self.photo_max_pixels(renderer), guard)
    }

    /// The largest side drawn in one pass: the card's, or in demo
    /// AURORA_DEMO_SNAPSHOT_TILE (to see the tiles and their seams).
    fn photo_max_side(&self, renderer: &Renderer) -> u32 {
        let forced = self
            .demo
            .then(|| std::env::var("AURORA_DEMO_SNAPSHOT_TILE").ok()?.parse::<u32>().ok())
            .flatten();
        forced.unwrap_or_else(|| renderer.max_texture_dimension())
    }

    /// MSAA samples the frames are drawn with (the renderer takes the most
    /// the GPU supports up to the setting's).
    fn msaa_samples(&self, renderer: &Renderer) -> u32 {
        match self.settings.render_settings().aa {
            aurora_render::AntiAliasing::Msaa(n) => renderer.msaa_supported().iter().copied().filter(|&m| m <= n).max().unwrap_or(1),
            _ => 1,
        }
    }

    /// The most pixels drawn in one pass (`snapshot::pass_pixels`): half of
    /// the free video memory, besides what the window's frame holds, and
    /// never fewer than the window's.
    fn photo_max_pixels(&self, renderer: &Renderer) -> u64 {
        let per_pixel = TARGET_BYTES_PER_PIXEL + TARGET_BYTES_PER_SAMPLE * u64::from(self.msaa_samples(renderer));
        let free = renderer
            .video_memory
            .read(&renderer.device)
            .map_or(DEFAULT_TARGET_BYTES, |m| m.budget.saturating_sub(m.usage) / 2);
        snapshot::pass_pixels(free, per_pixel, renderer.size())
    }

    /// LLViewerWindow::playSnapshotAnimAndSound: the snapshot animation and
    /// the shutter (UISndSnapshot) as a SoundTrigger, both seen and heard by
    /// everyone around; "Quiet snapshots" (PlayModeUISndSnapshot, the
    /// setting off) skips the two.
    fn photo_shutter(&mut self) {
        if !self.settings.snapshot.sound {
            return;
        }
        self.own_animation(ANIM_AGENT_SNAPSHOT, true);
        self.send_sound_trigger(self.settings.audio.ui.uuid(UiSound::Snapshot));
    }

    fn photo_save(&mut self, size: (u32, u32), rgba: Vec<u8>, chosen: Option<PathBuf>, cutout: bool) {
        let s = &self.settings.snapshot;
        // a cut-out photo is a PNG whatever the format chosen: the only
        // one of them with an alpha
        let wanted = if cutout { snapshot::FileFormat::Png } else { s.format };
        let (format, path) = match chosen {
            Some(path) if cutout => (wanted, path.with_extension(wanted.extension())),
            Some(path) => (snapshot::FileFormat::of_path(&path).unwrap_or(wanted), path),
            None => {
                let stamp = s.timestamp_names.then(snapshot::local_stamp);
                (wanted, snapshot::free_path(&s.folder(), &s.base_name, stamp.as_deref(), wanted))
            }
        };
        self.snapshots.message_item = None;
        self.snapshots.message_profile = false;
        self.snapshots.saving = Some(save::Saving::start(save::Job {
            width: size.0,
            height: size.1,
            rgba,
            format,
            jpeg_quality: s.jpeg_quality,
            path,
            alpha: cutout,
        }));
        self.photo_shutter();
    }

    /// A photo is being taken, saved or uploaded.
    fn photo_busy(&self) -> bool {
        let s = &self.snapshots;
        s.saving.is_some() || s.upload.is_some() || s.detail_wait.is_some() || s.job.as_ref().is_some_and(|j| j.purpose != Purpose::Preview)
    }

    /// Upload prices: the account's benefits (texture_upload_cost, by area
    /// for the large ones), the demo's, or the economy's single price.
    fn photo_prices(&self) -> Prices {
        let prices = if self.demo {
            Prices {
                texture: Some(10),
                large_texture: Some(40),
                sound: Some(10),
                animation: Some(10),
            }
        } else {
            self.world.login.as_ref().map(|l| Prices::from_login(&l.raw)).unwrap_or_default()
        };
        prices.with_fallback(self.world.upload_cost)
    }

    /// What the window shows of a photo for the inventory.
    fn photo_inventory_view(&self, window: (u32, u32)) -> InventoryView {
        let texture = inventory::texture_size(self.photo_size(window));
        InventoryView {
            texture,
            cost: self.photo_prices().cost(Kind::Snapshot, texture),
            connected: self.demo || self.world.login.is_some(),
        }
    }

    /// "Save to inventory" (LLPanelSnapshotInventory::onSend): the price
    /// confirmed first, refused when the balance cannot pay it
    /// (can_afford_transaction, ErrorPhotoCannotAfford).
    fn photo_to_inventory(&mut self, renderer: &Renderer) {
        if self.photo_busy() {
            return;
        }
        let view = self.photo_inventory_view(renderer.size());
        let Some(cost) = view.cost.filter(|_| view.connected) else {
            return;
        };
        if self.world.balance.is_some_and(|b| b < cost) {
            self.snapshots.short = Some(cost);
        } else if cost > 0 {
            self.snapshots.confirm = Some(cost);
        } else {
            self.photo_inventory_start(cost, renderer);
        }
    }

    /// The price confirmed: the held photo, or a new one.
    fn photo_inventory_start(&mut self, cost: i32, renderer: &Renderer) {
        if self.photo_busy() {
            return;
        }
        if let Some(held) = &self.snapshots.held {
            let (size, rgba) = ((held.width, held.height), held.rgba.clone());
            self.photo_encode(size, rgba, cost);
            return;
        }
        self.photo_job(Purpose::Inventory(cost), renderer);
    }

    /// Scale and encode the texture on a worker; the shutter sounds as the
    /// upload starts, like saveTexture's playSnapshotAnimAndSound.
    fn photo_encode(&mut self, size: (u32, u32), rgba: Vec<u8>, cost: i32) {
        self.photo_review_thumb(size, &rgba);
        let texture = inventory::texture_size(size);
        let (tx, rx) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let t0 = Instant::now();
            let encoded = inventory::encode(size.0, size.1, &rgba, texture);
            if let Ok(data) = &encoded {
                log::info!(
                    "photo: texture {} x {} encoded in {} ms ({} KB)",
                    texture.0,
                    texture.1,
                    t0.elapsed().as_millis(),
                    data.len() >> 10
                );
            }
            let _ = tx.send(encoded);
        });
        self.snapshots.upload = Some(Upload::Encoding { rx, cost });
        self.photo_shutter();
    }

    /// The encoded texture: its upload to the Photo Album, named after the
    /// place (LLSnapshotLivePreview::saveTexture).
    fn photo_upload(&mut self, data: Vec<u8>, cost: i32) {
        let w = &self.world;
        let maturity = w
            .main()
            .and_then(|r| r.info.as_ref())
            .map(|i| crate::ui::bars::maturity_name(i.sim_access))
            .unwrap_or("");
        let place = inventory::location(&w.parcel_name, &w.region_name(), w.agent.position, maturity);
        // LLAgentUI::buildFullname: the legacy name
        let who = w
            .login
            .as_ref()
            .map(|l| format!("{} {}", l.first_name, l.last_name).trim().to_owned())
            .unwrap_or_else(|| w.own_name());
        let folder = crate::world::inventory::actions::system(&w.inventory, inventory::PHOTO_ALBUM).unwrap_or(w.inventory.root);
        let uploads = self.settings.imports.permissions();
        let request = Uuid::new_v4();
        let upload = upload::Request {
            kind: Kind::Snapshot,
            folder,
            name: inventory::item_name(&place),
            description: inventory::item_description(&who, &place),
            // PERM_ALL for the next owner; the group and everyone masks of
            // the uploads (LLFloaterPerms::getGroupPerms("Uploads"))
            permissions: upload::Permissions {
                next_owner: 0x7fff_ffff,
                ..uploads
            },
            expected_cost: cost,
            data,
        };
        log::info!("photo: upload of {} bytes to the Photo Album for L$ {cost}", upload.data.len());
        self.snapshots.upload = Some(Upload::Sent(request));
        self.send(NetCommand::UploadAsset { request, upload });
    }

    /// The upload this request belongs to is a photo's.
    pub(super) fn photo_owns_upload(&self, request: Uuid) -> bool {
        matches!(self.snapshots.upload, Some(Upload::Sent(r)) if r == request)
    }

    /// The server's answer to a photo's upload.
    pub(super) fn photo_uploaded(&mut self, result: Result<Receipt, upload::Error>) {
        self.snapshots.upload = None;
        match result {
            Ok(receipt) => {
                let item = receipt.item;
                log::info!("photo: saved to the inventory for L$ {}", receipt.cost);
                let id = item.id;
                self.world.inventory.add_items(vec![item]);
                // the inventory shows the new photo (upload_done_callback,
                // openInventoryPanelAndSetSelection)
                self.panels.inventory = true;
                self.inventory_ui.show_original(&self.world.inventory, id);
                self.snapshots.message = Some((aurora_i18n::tr!("photo-uploaded").to_owned(), false));
                self.snapshots.message_item = Some(id);
                self.snapshots.message_profile = false;
                self.photo_review_show(
                    aurora_i18n::tr!("photo-destination-inventory").to_owned(),
                    crate::ui::viewfinder::Review::Inventory,
                );
            }
            Err(e) => {
                log::warn!("photo: upload failed: {e}");
                let text = match e {
                    upload::Error::PriceChanged(Some(cost)) => aurora_i18n::tr!("photo-upload-price-changed", cost = cost).to_owned(),
                    upload::Error::Unknown | upload::Error::Uncertain(_) | upload::Error::Verification => {
                        aurora_i18n::tr!("photo-upload-unknown").to_owned()
                    }
                    upload::Error::Rejected | upload::Error::Refused(_) => aurora_i18n::tr!("photo-upload-refused").to_owned(),
                    other => aurora_i18n::tr!("photo-upload-failed", error = other.to_string()).to_owned(),
                };
                self.photo_vf_error(&text);
                self.snapshots.message = Some((text, true));
                self.snapshots.message_item = None;
            }
        }
    }

    /// The profile post's progress and outcome (reportImageUploadStatus).
    fn photo_profile_poll(&mut self) {
        let Some(Upload::Profile { rx, bytes }) = &mut self.snapshots.upload else {
            return;
        };
        let mut done = None;
        while let Ok(step) = rx.try_recv() {
            match step {
                ProfileStep::Encoded(n) => *bytes = Some(n),
                ProfileStep::Done(result) => done = Some(result),
            }
        }
        let Some(result) = done else {
            return;
        };
        let too_big = bytes.is_some_and(|n| n > profile::MAX_WEB_DATASIZE);
        self.snapshots.upload = None;
        let s = &mut self.snapshots;
        s.message_item = None;
        match result {
            Ok(()) => {
                log::info!("photo: posted to the profile feed");
                s.message = Some((aurora_i18n::tr!("photo-profile-posted").to_owned(), false));
                s.message_profile = true;
                self.photo_review_show(
                    aurora_i18n::tr!("photo-destination-profile").to_owned(),
                    crate::ui::viewfinder::Review::Profile,
                );
            }
            Err(e) => {
                log::warn!("photo: not posted to the profile feed: {e}");
                // CannotUploadSnapshotWebTooBig when the PNG is over the limit
                let text = if too_big {
                    aurora_i18n::tr!("photo-profile-too-big")
                } else {
                    aurora_i18n::tr!("photo-profile-failed")
                };
                s.message = Some((text.to_owned(), true));
                s.message_profile = false;
                self.photo_vf_error(text);
            }
        }
    }

    /// "Take the photo" to disk: the held one, or a new one.
    fn photo_take(&mut self, renderer: &Renderer, chosen: Option<PathBuf>) {
        if self.photo_busy() {
            return;
        }
        if let Some(held) = &self.snapshots.held {
            let (size, rgba, cutout) = ((held.width, held.height), held.rgba.clone(), held.cutout);
            self.photo_save(size, rgba, chosen, cutout);
            return;
        }
        self.photo_job(Purpose::Save(chosen), renderer);
    }

    /// "Take a photo to disk" (Firestorm Ctrl+`): the settings of the photo
    /// window, without it.
    pub(super) fn photo_to_disk(&mut self) {
        let Some(gfx) = self.gfx.take() else {
            return;
        };
        self.snapshots.quiet = !self.panels.snapshot;
        self.photo_take(&gfx.renderer, None);
        self.gfx = Some(gfx);
    }

    /// Every frame: the saves, thumbnails, dialogs and preview to follow.
    pub(super) fn photo_update(&mut self, renderer: &Renderer) {
        self.photo_demo(renderer);
        self.photo_luts_update();
        self.photo_quality(renderer);
        self.photo_light();
        self.photo_free_camera();
        if let Some(t) = self.snapshots.detail_wait {
            let pending = self.scene.textures.pending_in_view();
            if pending == 0 || t.elapsed() >= snapshot::quality::DETAIL_WAIT || self.settings.photo_held.is_none() {
                log::info!(
                    "photo quality: details loaded in {} ms ({pending} textures left)",
                    t.elapsed().as_millis()
                );
                self.snapshots.detail_wait = None;
                self.snapshots.detail_ready = true;
                self.photo_action(Action::Take, renderer);
                self.snapshots.detail_ready = false;
            }
        }
        // the texture encoded on its worker: its upload
        if let Some(Upload::Encoding { rx, cost }) = &self.snapshots.upload
            && let Ok(encoded) = rx.try_recv()
        {
            let cost = *cost;
            match encoded {
                Ok(data) => self.photo_upload(data, cost),
                Err(e) => {
                    log::warn!("photo: texture not encoded: {e}");
                    self.snapshots.upload = None;
                    self.snapshots.message = Some((aurora_i18n::tr!("photo-upload-failed", error = e.as_str()).to_owned(), true));
                }
            }
        }
        self.photo_profile_poll();
        self.photo_postcard_poll();
        let viewfinder = self.viewfinder_on();
        let s = &mut self.snapshots;
        // the save on its worker
        if let Some(r) = s.saving.as_ref().and_then(|saving| saving.poll()) {
            s.saving = None;
            match r {
                Ok(saved) => {
                    let name = saved.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    log::info!(
                        "photo saved: {} ({} KB, written in {} ms)",
                        saved.path.display(),
                        saved.bytes >> 10,
                        saved.millis
                    );
                    s.message = Some((aurora_i18n::tr!("photo-saved", name = name.as_str()).to_owned(), false));
                    let (w, h, px) = saved.thumb;
                    let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
                    let tex = self.egui_ctx.load_texture("photo_thumb", image, egui::TextureOptions::LINEAR);
                    s.gallery.retain(|(p, _)| *p != saved.path);
                    s.gallery.insert(0, (saved.path.clone(), Some(tex.clone())));
                    s.review_tex = Some(tex);
                    s.review = Some((
                        format!("{} · {name}", aurora_i18n::tr!("photo-destination-disk")),
                        crate::ui::viewfinder::Review::Disk(saved.path.clone()),
                    ));
                    if viewfinder {
                        s.vf.review_until = self.egui_ctx.input(|i| i.time) + 4.8;
                    }
                    s.gallery.truncate(gallery::SHOWN);
                    if std::mem::take(&mut s.quiet) {
                        self.world.notifications.push(
                            crate::world::notifications::Kind::Info,
                            aurora_i18n::tr!("photo-saved-title"),
                            saved.path.display().to_string(),
                            crate::world::notifications::Data::None,
                        );
                    }
                    s.last_saved = Some(saved.path);
                }
                Err(e) => {
                    log::warn!("photo: not saved: {e}");
                    let text = aurora_i18n::tr!("photo-failed", error = e.as_str()).to_owned();
                    let now = self.egui_ctx.input(|i| i.time);
                    s.vf.say(text.clone(), true, now);
                    s.message = Some((text, true));
                }
            }
        }
        // the gallery, read when the window shows it and its folder changed
        let folder = self.settings.snapshot.folder();
        if self.panels.snapshot && s.gallery_folder.as_ref() != Some(&folder) {
            s.gallery_folder = Some(folder.clone());
            let files = gallery::recent(&folder, gallery::SHOWN);
            s.gallery = files.iter().map(|p| (p.clone(), None)).collect();
            let tx = s.thumbs.0.clone();
            std::thread::spawn(move || {
                for path in files {
                    let thumb = gallery::load_thumbnail(&path);
                    if tx.send((path, thumb)).is_err() {
                        break;
                    }
                }
            });
        }
        while let Ok((path, thumb)) = s.thumbs.1.try_recv() {
            if let (Some(slot), Some((w, h, px))) = (s.gallery.iter_mut().find(|(p, _)| *p == path), thumb) {
                let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
                slot.1 = Some(self.egui_ctx.load_texture("photo_thumb", image, egui::TextureOptions::LINEAR));
            }
        }
        // the last photo's thumbnail, made on a worker
        if let Some(rx) = &s.review_rx
            && let Ok((w, h, px)) = rx.try_recv()
        {
            s.review_rx = None;
            let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
            s.review_tex = Some(self.egui_ctx.load_texture("photo_review", image, egui::TextureOptions::LINEAR));
        }
        // the held photo, reduced for the screen
        if let Some(rx) = &s.held_view
            && let Ok((w, h, px)) = rx.try_recv()
        {
            s.held_view = None;
            if let Some(held) = &mut s.held {
                let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
                held.texture = Some(self.egui_ctx.load_texture("photo_held", image, egui::TextureOptions::LINEAR));
            }
        }
        // a file dialog's answer: (Save as, the path)
        if let Some(rx) = &s.dialog
            && let Ok((save_as, path)) = rx.try_recv()
        {
            s.dialog = None;
            if let Some(path) = path {
                if save_as {
                    if let Some(folder) = path.parent() {
                        self.settings.snapshot.folder = Some(folder.to_path_buf());
                    }
                    let path = if snapshot::FileFormat::of_path(&path).is_some() {
                        path
                    } else {
                        path.with_extension(self.settings.snapshot.format.extension())
                    };
                    self.settings.save();
                    self.photo_take(renderer, Some(path));
                } else {
                    self.settings.snapshot.folder = Some(path);
                    self.settings.save();
                }
            }
        }
        let in_world = self.in_world();
        let viewfinder = self.viewfinder_on();
        let s = &mut self.snapshots;
        // the preview: asked, its photo changed, or the camera settled (the
        // viewfinder needs none: it is one)
        if !self.panels.snapshot || s.held.is_some() || viewfinder {
            return;
        }
        let window = renderer.size();
        let key = (
            self.photo_size(window),
            self.settings.snapshot.interface,
            self.settings.snapshot.huds,
        );
        let s = &mut self.snapshots;
        if s.preview_for != Some(key) {
            s.preview_wanted = true;
        }
        let cam = &self.camera;
        let now = [
            cam.position.x,
            cam.position.y,
            cam.position.z,
            cam.target.x,
            cam.target.y,
            cam.target.z,
            cam.fov_y,
        ];
        if s.camera != Some(now) {
            s.camera = Some(now);
            s.camera_moved = Some(Instant::now());
        } else if self.settings.snapshot.auto_refresh && s.camera_moved.is_some_and(|t| t.elapsed() >= SETTLE) {
            s.camera_moved = None;
            s.preview_wanted = true;
        }
        if s.preview_wanted && s.job.is_none() && in_world {
            s.preview_wanted = false;
            s.preview_for = Some(key);
            self.photo_job(Purpose::Preview, renderer);
        }
    }

    /// What the photo window asked.
    pub(super) fn photo_action(&mut self, action: Action, renderer: &Renderer) {
        match action {
            Action::Refresh => self.snapshots.preview_wanted = true,
            Action::Take => {
                self.snapshots.quiet = false;
                // the photo quality waits for the textures in view to load
                // at full resolution (photo_update takes it then, or after
                // quality::DETAIL_WAIT)
                if self.settings.photo_held.is_some()
                    && !std::mem::take(&mut self.snapshots.detail_ready)
                    && self.scene.textures.pending_in_view() > 0
                {
                    if self.snapshots.detail_wait.is_none() && !self.photo_busy() {
                        log::info!("photo quality: waiting for {} textures", self.scene.textures.pending_in_view());
                        self.snapshots.detail_wait = Some(Instant::now());
                    }
                    return;
                }
                match self.snapshots.ui.destination {
                    Destination::Disk => self.photo_take(renderer, None),
                    Destination::Inventory => self.photo_to_inventory(renderer),
                    Destination::Profile => self.photo_to_profile(renderer),
                    Destination::Email => self.photo_to_postcard(renderer),
                }
            }
            Action::Confirm(yes) => {
                if let Some(cost) = self.snapshots.confirm.take()
                    && yes
                {
                    self.photo_inventory_start(cost, renderer);
                }
            }
            Action::CloseAfford => self.snapshots.short = None,
            Action::PersonalLighting => self.panels.personal_lighting = true,
            Action::KeepLight => {
                if self.world.eep.keep_photo() {
                    self.snapshots.ui.light.env = Default::default();
                }
            }
            Action::Controls => {
                if !self.panels.camera_controls && !self.panels.movement_controls {
                    self.panels.camera_controls = true;
                }
                crate::ui::widgets::expand_floater(&self.egui_ctx, crate::ui::controls::WINDOW);
                // opened by hand: it stays when the flight ends
                self.snapshots.free_opened_controls = false;
            }
            Action::ShowItem => {
                if let Some(id) = self.snapshots.message_item {
                    self.panels.inventory = true;
                    self.inventory_ui.show_original(&self.world.inventory, id);
                }
            }
            // our profile, whose Feed tab shows the post
            Action::ShowProfile => {
                let me = self.world.agent_id;
                self.profile_ui.open(&mut self.world, me);
            }
            Action::SaveAs | Action::ChooseFolder => {
                if self.snapshots.dialog.is_some() {
                    return;
                }
                let save_as = action == Action::SaveAs;
                let s = &self.settings.snapshot;
                let folder = s.folder();
                let _ = std::fs::create_dir_all(&folder);
                let stamp = s.timestamp_names.then(snapshot::local_stamp);
                let name = snapshot::free_path(&folder, &s.base_name, stamp.as_deref(), s.format);
                let name = name.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let dialog = rfd::FileDialog::new().set_directory(&folder);
                    let path = if save_as {
                        dialog
                            .set_title(aurora_i18n::tr!("photo-save-dialog"))
                            .set_file_name(name)
                            .add_filter(aurora_i18n::tr!("photo-images"), &["png", "jpg", "jpeg", "bmp"])
                            .save_file()
                    } else {
                        dialog.set_title(aurora_i18n::tr!("photo-folder-dialog")).pick_folder()
                    };
                    let _ = tx.send((save_as, path));
                });
                self.snapshots.dialog = Some(rx);
            }
            Action::OpenFolder => {
                gallery::reveal(&self.settings.snapshot.folder(), self.snapshots.last_saved.as_deref());
            }
            Action::OpenPhoto(path) => gallery::open(&path),
            Action::Freeze(on) => {
                self.snapshots.held = None;
                self.snapshots.held_view = None;
                if on {
                    self.photo_job(Purpose::Hold, renderer);
                } else {
                    self.snapshots.preview_wanted = true;
                }
            }
            Action::Changed => {
                self.snapshots.preview_wanted = true;
                self.settings.save();
            }
            Action::Saved => self.settings.save(),
            Action::ImportLut => {
                if self.snapshots.lut_import.is_some() {
                    return;
                }
                let dir = self.photo_luts().dir().to_path_buf();
                let (tx, rx) = crossbeam_channel::bounded(1);
                std::thread::spawn(move || {
                    let picked = rfd::FileDialog::new()
                        .set_title(aurora_i18n::tr!("photo-lut-dialog"))
                        .add_filter(aurora_i18n::tr!("photo-lut-files"), &["cube", "CUBE"])
                        .pick_file();
                    let _ = tx.send(picked.map(|path| LutLibrary::import(&dir, &path)));
                });
                self.snapshots.lut_import = Some(rx);
            }
            Action::RemoveLut(name) => {
                if let Err(e) = self.photo_luts().remove(&name) {
                    log::warn!("photo LUT {name} not removed: {e}");
                }
                let id = format!("{}{name}", looks::LUT);
                self.settings.snapshot.looks.retain(|l| l.id != id);
                self.settings.save();
            }
            Action::OpenLutFolder => {
                let dir = self.photo_luts().dir().to_path_buf();
                let _ = std::fs::create_dir_all(&dir);
                gallery::reveal(&dir, None);
            }
        }
    }

    /// The tables' list for the interface, and an import that came back:
    /// the table chosen and said.
    fn photo_luts_update(&mut self) {
        if !self.panels.snapshot {
            return;
        }
        self.photo_luts().poll();
        for failure in self.photo_luts().take_failures() {
            let text = aurora_i18n::tr!("photo-lut-failed", error = failure);
            self.snapshots.message = Some((text.clone(), true));
            self.photo_vf_error(&text);
        }
        let names = self.photo_luts().names().to_vec();
        if self.snapshots.ui.lut_names != names {
            self.snapshots.ui.lut_names = names;
        }
        let Some(rx) = &self.snapshots.lut_import else { return };
        let Ok(result) = rx.try_recv() else { return };
        self.snapshots.lut_import = None;
        match result {
            None => {}
            Some(Ok((name, lut))) => {
                let text = aurora_i18n::tr!("photo-lut-imported", name = looks::lut_display(&name));
                self.photo_luts().imported(name.clone(), lut);
                let id = format!("{}{name}", looks::LUT);
                self.snapshots.ui.looks.pick(&mut self.settings.snapshot, &id);
                self.settings.save();
                self.snapshots.message = Some((text.clone(), false));
                let now = self.egui_ctx.input(|i| i.time);
                self.snapshots.vf.say(text, false, now);
            }
            Some(Err(e)) => {
                log::warn!("photo LUT not imported: {e}");
                let text = aurora_i18n::tr!("photo-lut-failed", error = e);
                self.snapshots.message = Some((text.clone(), true));
                self.photo_vf_error(&text);
            }
        }
    }

    /// AURORA_DEMO_PHOTO_LOOK=<look>[+<look>…][:<percent>][:compare][:panel]
    /// [:tab=legacy|aurora|luts][:search=<text>][:set=<n>,<v>,…], with
    /// AURORA_DEMO_SNAPSHOT: the looks stacked in that order (`Sepia`,
    /// `aurora:vhs`, `lut:<file>`, or `lut:demo` for a warm table written
    /// into the library), the last one chosen, at that strength (100 by
    /// default), the before / after line in the middle, the viewfinder's
    /// filter panel open on the chosen look's tab or the one given, a
    /// search typed, the chosen Aurora look's settings set (the photo
    /// window's settings scrolled to the filters). Without it, the
    /// scenarios start with no look.
    fn photo_look_demo(&mut self) {
        let value = std::env::var("AURORA_DEMO_PHOTO_LOOK").unwrap_or_default();
        let mut parts: Vec<&str> = value.split('+').collect();
        let last = parts.pop().unwrap_or_default();
        let mut tokens = last.split(':');
        // `aurora:` and `lut:` ids hold a colon
        let id_of = |tokens: &mut std::str::Split<'_, char>| {
            let mut id = tokens.next().unwrap_or_default().to_owned();
            if matches!(id.as_str(), "aurora" | "lut")
                && let Some(rest) = tokens.next()
            {
                id = format!("{id}:{rest}");
            }
            id
        };
        let mut ids: Vec<String> = parts.iter().map(|part| id_of(&mut part.split(':'))).collect();
        ids.push(id_of(&mut tokens));
        for id in &mut ids {
            if id == "lut:demo" {
                *id = format!("{}{}", looks::LUT, self.photo_demo_lut());
            }
        }
        let s = &mut self.settings.snapshot;
        s.looks = ids
            .iter()
            .filter(|id| looks::name(id).is_some())
            .map(|id| looks::Layer::new(id.clone()))
            .collect();
        let ui = &mut self.snapshots.ui.looks;
        ui.selected = s.looks.len().saturating_sub(1);
        ui.compare = None;
        ui.mode = looks::Mode::Replace;
        if let Some(layer) = s.looks.last() {
            ui.tab = looks::tab_of(&layer.id);
        }
        for part in tokens {
            match part.split_once('=') {
                Some(("tab", tab)) => {
                    ui.tab = match tab {
                        "aurora" => looks::Tab::Aurora,
                        "luts" => looks::Tab::Luts,
                        _ => looks::Tab::Legacy,
                    };
                }
                Some(("search", text)) => ui.query = text.to_owned(),
                Some(("set", values)) => {
                    if let Some(layer) = s.looks.last() {
                        let values: Vec<f32> = values.split(',').filter_map(|v| v.parse().ok()).collect();
                        s.look_settings.insert(layer.id.clone(), values);
                    }
                }
                _ => match part {
                    "compare" => ui.compare = Some(0.5),
                    "panel" => {
                        self.snapshots.vf.looks = true;
                        self.snapshots.vf.hinted = true;
                        ui.reveal = true;
                    }
                    strength => {
                        if let (Ok(v), Some(layer)) = (strength.parse::<u8>(), s.looks.last_mut()) {
                            layer.strength = v.min(100);
                        }
                    }
                },
            }
        }
        for layer in &s.looks {
            log::info!("photo look demo: {} at {} %", layer.id, layer.strength);
        }
    }

    /// The demo's table: warm lights and teal shadows, 17 points a side,
    /// written into the library once.
    fn photo_demo_lut(&mut self) -> String {
        const NAME: &str = "Demo warm.cube";
        let path = self.photo_luts().dir().join(NAME);
        if !path.exists() {
            let n = 17;
            let mut text = format!(
                "TITLE \"Demo warm\"
LUT_3D_SIZE {n}
"
            );
            for b in 0..n {
                for g in 0..n {
                    for r in 0..n {
                        let c = [r, g, b].map(|v| v as f32 / (n - 1) as f32);
                        let l = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
                        let tint = [0.08 * l - 0.04 * (1.0 - l), 0.02 * l, -0.06 * l + 0.05 * (1.0 - l)];
                        let o = [0, 1, 2].map(|k| (c[k] + tint[k]).clamp(0.0, 1.0));
                        text.push_str(&format!(
                            "{:.5} {:.5} {:.5}
",
                            o[0], o[1], o[2]
                        ));
                    }
                }
            }
            let _ = std::fs::create_dir_all(self.photo_luts().dir());
            if let Err(e) = std::fs::write(&path, text) {
                log::warn!("photo look demo: {NAME} not written: {e}");
            }
            self.photo_luts().refresh();
        }
        NAME.to_owned()
    }

    /// What the photo is doing, for a progress line: details loading, the
    /// drawing, the save, the upload.
    fn photo_busy_line(&self) -> Option<(f32, String)> {
        let s = &self.snapshots;
        if let Some(t) = s.detail_wait {
            let wait = snapshot::quality::DETAIL_WAIT.as_secs_f32();
            Some((
                (t.elapsed().as_secs_f32() / wait).min(1.0) * 0.5,
                aurora_i18n::tr!("photo-quality-loading", count = self.scene.textures.pending_in_view()).to_owned(),
            ))
        } else if let Some(saving) = &s.saving {
            Some((0.5 + 0.5 * saving.fraction(), aurora_i18n::tr!("photo-saving").to_owned()))
        } else if let Some(upload) = &s.upload {
            Some(match upload {
                Upload::Encoding { .. } => (0.6, aurora_i18n::tr!("photo-encoding").to_owned()),
                Upload::Sent(_) => (0.85, aurora_i18n::tr!("photo-uploading").to_owned()),
                Upload::Profile { bytes: None, .. } => (0.6, aurora_i18n::tr!("photo-profile-encoding").to_owned()),
                Upload::Profile { bytes: Some(_), .. } => (0.85, aurora_i18n::tr!("photo-profile-posting").to_owned()),
                Upload::Postcard { encoded: false, .. } => (0.6, aurora_i18n::tr!("photo-profile-encoding").to_owned()),
                Upload::Postcard { encoded: true, .. } => (0.85, aurora_i18n::tr!("photo-email-sending").to_owned()),
            })
        } else {
            s.job.as_ref().filter(|j| j.purpose != Purpose::Preview).map(|j| {
                let f = 0.5 * j.next as f32 / j.tiles.len().max(1) as f32;
                let what = if j.tiles.len() > 1 {
                    aurora_i18n::tr!("photo-rendering-tiles", tile = j.next + 1, count = j.tiles.len()).to_owned()
                } else {
                    aurora_i18n::tr!("photo-rendering").to_owned()
                };
                (f, what)
            })
        }
    }

    /// The photo mode's look: Preferences', or the demo's for the run
    /// (AURORA_DEMO_SNAPSHOT=classic the Legacy floater, viewfinder… the
    /// viewfinder, any other scenario the panel).
    pub(super) fn photo_style(&self) -> crate::settings::PhotoWindowStyle {
        self.snapshots
            .demo
            .as_ref()
            .and_then(|d| d.style)
            .unwrap_or(self.settings.photo_window)
    }

    /// The demo's viewfinder steps due this frame: aim at the own avatar's
    /// face and half-press, then release.
    pub(super) fn photo_demo_viewfinder(&self) -> (bool, bool) {
        let Some(d) = self.snapshots.demo.as_ref() else {
            return (false, false);
        };
        let Some(v) = d.viewfinder.as_ref() else {
            return (false, false);
        };
        (
            (v.focus || v.shoot || v.group) && self.frame_count == d.frame,
            v.shoot && self.frame_count == d.frame + 2,
        )
    }

    /// The demo adds the other avatars in view to the focus this frame
    /// (`group`), once the own avatar's has locked.
    pub(super) fn photo_demo_group(&self) -> bool {
        self.snapshots
            .demo
            .as_ref()
            .is_some_and(|d| d.viewfinder.as_ref().is_some_and(|v| v.group) && self.frame_count == d.frame + 40)
    }

    /// The last photo's card of the viewfinder: its thumbnail, what it
    /// says, what it offers.
    pub(super) fn photo_review(&self) -> Option<(egui::TextureHandle, String, crate::ui::viewfinder::Review)> {
        let s = &self.snapshots;
        let (label, kind) = s.review.clone()?;
        Some((s.review_tex.clone()?, label, kind))
    }

    /// The card shows a photo sent to the inventory or the profile feed.
    fn photo_review_show(&mut self, label: String, kind: crate::ui::viewfinder::Review) {
        self.snapshots.review = Some((label, kind));
        if self.viewfinder_on() {
            self.snapshots.vf.review_until = self.egui_ctx.input(|i| i.time) + 4.8;
        }
    }

    /// The card's thumbnail of a photo that is not saved to disk, reduced
    /// on a worker.
    fn photo_review_thumb(&mut self, size: (u32, u32), rgba: &[u8]) {
        if !self.viewfinder_on() {
            return;
        }
        let (tx, rx) = crossbeam_channel::bounded(1);
        let rgba = rgba.to_vec();
        std::thread::spawn(move || {
            let _ = tx.send(save::downscale(size.0, size.1, &rgba, 300));
        });
        self.snapshots.review_tex = None;
        self.snapshots.review_rx = Some(rx);
    }

    /// An upload's error, on the viewfinder's line too.
    fn photo_vf_error(&mut self, text: &str) {
        let now = self.egui_ctx.input(|i| i.time);
        self.snapshots.vf.say(text.to_owned(), true, now);
    }

    /// The photo window, the viewfinder, the full-frame view or the held
    /// photo, in the interface of the world.
    pub(super) fn photo_ui(
        &mut self,
        ctx: &egui::Context,
        p: &crate::theme::Palette,
        projector: &crate::ui::hud::Projector,
        renderer: &Renderer,
    ) {
        // a photo with the interface does not show the photo window
        if self.snapshots.capturing.is_some_and(|c| c.interface) {
            return;
        }
        let window = renderer.size();
        let photo = self.photo_size(window);
        let quality_reduced = self.settings.photo_held.is_some() && self.snapshots.fit.as_ref().is_some_and(|f| f.step > 0);
        // (the panel shows it in its own line, the viewfinder once at the top)
        let quality_note = std::mem::take(&mut self.snapshots.fit_note) && quality_reduced;
        let mut actions = self.snapshots.ui.dialogs(ctx, p, self.snapshots.confirm, self.snapshots.short);
        if self.viewfinder_on() {
            // the held photo under the overlay
            if let Some(t) = self.snapshots.held.as_ref().and_then(|h| h.texture.as_ref()) {
                crate::ui::snapshot::frozen(ctx, p, t);
            }
            let inventory = (self.snapshots.ui.destination == Destination::Inventory).then(|| self.photo_inventory_view(window));
            let profile = (self.snapshots.ui.destination == Destination::Profile).then(|| self.photo_profile_view(window));
            let email = (self.snapshots.ui.destination == Destination::Email).then(|| self.photo_email_view(window));
            let tiles = self.photo_tiles(renderer, photo).len();
            let busy = self.photo_busy_line();
            if let Some((_, what)) = &busy {
                let now = ctx.input(|i| i.time);
                self.snapshots.vf.message = Some((what.clone(), false, now + 0.3));
            } else if quality_note {
                // the photo quality lowered for the video memory: said once
                let now = ctx.input(|i| i.time);
                let line = aurora_i18n::tr!("photo-quality-reduced").to_owned();
                self.snapshots.vf.message = Some((line, false, now + 4.0));
            }
            let message = self.snapshots.message.clone();
            let last_saved = self.snapshots.last_saved.clone();
            let gallery = self.snapshots.gallery.clone();
            // (lent to the drawer's Light section while the viewfinder draws)
            self.photo_env_lists(ctx);
            let env_ui = std::mem::take(&mut self.env_ui);
            let view = crate::ui::snapshot::View {
                window,
                photo,
                tiles,
                preview: None,
                busy,
                message: message.as_ref(),
                last_saved: last_saved.as_deref(),
                gallery: &gallery,
                folder: self.settings.snapshot.folder(),
                inventory,
                item_saved: self.snapshots.message_item.is_some(),
                profile,
                profile_posted: self.snapshots.message_profile,
                email,
                exposure: self.settings.exposure,
                hdr: self.settings.hdr_emissive,
                env: env_view(&env_ui, &self.world.eep),
                back_key: self.settings.keybinds.get(crate::keybinds::Action::ResetCamera)[0].clone(),
                quality_reduced,
            };
            self.viewfinder_ui(ctx, p, projector, renderer, &view);
            drop(view);
            self.env_ui = env_ui;
        } else if self.snapshots.ui.full_frame && self.panels.snapshot {
            actions.extend(self.snapshots.ui.full_frame(
                ctx,
                p,
                &mut self.settings.snapshot,
                &mut self.settings.camera.fov,
                crate::camera::DEFAULT_FOV,
                photo,
            ));
        } else if self.panels.snapshot {
            let inventory = (self.snapshots.ui.destination == Destination::Inventory).then(|| self.photo_inventory_view(window));
            let profile = (self.snapshots.ui.destination == Destination::Profile).then(|| self.photo_profile_view(window));
            let email = (self.snapshots.ui.destination == Destination::Email).then(|| self.photo_email_view(window));
            let mut state = std::mem::take(&mut self.snapshots.ui);
            self.photo_env_lists(ctx);
            let tiles = self.photo_tiles(renderer, photo).len();
            let s = &self.snapshots;
            let busy = if let Some(t) = s.detail_wait {
                let wait = snapshot::quality::DETAIL_WAIT.as_secs_f32();
                Some((
                    (t.elapsed().as_secs_f32() / wait).min(1.0) * 0.5,
                    aurora_i18n::tr!("photo-quality-loading", count = self.scene.textures.pending_in_view()).to_owned(),
                ))
            } else if let Some(saving) = &s.saving {
                Some((0.5 + 0.5 * saving.fraction(), aurora_i18n::tr!("photo-saving").to_owned()))
            } else if let Some(upload) = &s.upload {
                Some(match upload {
                    Upload::Encoding { .. } => (0.6, aurora_i18n::tr!("photo-encoding").to_owned()),
                    Upload::Sent(_) => (0.85, aurora_i18n::tr!("photo-uploading").to_owned()),
                    Upload::Profile { bytes: None, .. } => (0.6, aurora_i18n::tr!("photo-profile-encoding").to_owned()),
                    Upload::Profile { bytes: Some(_), .. } => (0.85, aurora_i18n::tr!("photo-profile-posting").to_owned()),
                    Upload::Postcard { encoded: false, .. } => (0.6, aurora_i18n::tr!("photo-profile-encoding").to_owned()),
                    Upload::Postcard { encoded: true, .. } => (0.85, aurora_i18n::tr!("photo-email-sending").to_owned()),
                })
            } else {
                s.job.as_ref().filter(|j| j.purpose != Purpose::Preview).map(|j| {
                    let f = 0.5 * j.next as f32 / j.tiles.len().max(1) as f32;
                    let what = if j.tiles.len() > 1 {
                        aurora_i18n::tr!("photo-rendering-tiles", tile = j.next + 1, count = j.tiles.len()).to_owned()
                    } else {
                        aurora_i18n::tr!("photo-rendering").to_owned()
                    };
                    (f, what)
                })
            };
            let gallery = s.gallery.clone();
            let view = crate::ui::snapshot::View {
                window,
                photo,
                tiles,
                preview: s.held.as_ref().and_then(|h| h.texture.as_ref()).or(s.preview.as_ref()),
                busy,
                message: s.message.as_ref(),
                last_saved: s.last_saved.as_deref(),
                gallery: &gallery,
                folder: self.settings.snapshot.folder(),
                inventory,
                item_saved: s.message_item.is_some(),
                profile,
                profile_posted: s.message_profile,
                email,
                exposure: self.settings.exposure,
                hdr: self.settings.hdr_emissive,
                env: env_view(&self.env_ui, &self.world.eep),
                back_key: self.settings.keybinds.get(crate::keybinds::Action::ResetCamera)[0].clone(),
                quality_reduced,
            };
            let mut open = self.panels.snapshot;
            let classic = self.photo_style() == crate::settings::PhotoWindowStyle::Legacy;
            actions.extend(state.show(
                ctx,
                p,
                &mut open,
                &mut self.settings.snapshot,
                &mut self.settings.camera.fov,
                crate::camera::DEFAULT_FOV,
                &view,
                classic,
            ));
            self.snapshots.ui = state;
            self.panels.snapshot = open;
        }
        for a in actions {
            self.photo_action(a, renderer);
        }
        if !self.panels.snapshot {
            self.snapshots.ui.full_frame = false;
            if self.snapshots.held.take().is_some() {
                self.snapshots.ui.freeze = false;
            }
        }
    }

    /// The held photo over the world (drawn first, under the windows).
    pub(super) fn photo_held(&self, ctx: &egui::Context, p: &crate::theme::Palette) -> bool {
        match self.snapshots.held.as_ref().and_then(|h| h.texture.as_ref()) {
            Some(t) if self.panels.snapshot => {
                crate::ui::snapshot::frozen(ctx, p, t);
                true
            }
            _ => false,
        }
    }

    /// The full-frame view and the viewfinder hide the rest of the
    /// interface.
    pub(super) fn photo_full_frame(&self) -> bool {
        self.panels.snapshot && (self.snapshots.ui.full_frame || self.viewfinder_on())
    }

    /// Escape leaves the full-frame view, then brings the free camera back,
    /// instead of resetting the camera. In the viewfinder: a menu open
    /// closes, the free camera comes back, the first person view ends, then
    /// the photo mode closes.
    pub(super) fn photo_escape(&mut self) -> bool {
        if self.viewfinder_on() {
            let v = &mut self.snapshots.vf;
            if v.countdown.is_some() {
                v.countdown = None;
                let now = self.egui_ctx.input(|i| i.time);
                self.snapshots.vf.say(aurora_i18n::tr!("photo-vf-timer-cancelled"), false, now);
            } else if self.world.photo_isolation.picking {
                // the choosing in the scene stops, its panel left open
                self.world.photo_isolation.set_picking(false);
            } else if v.preset_form.picking {
                v.preset_form.picking = false;
            } else if v.menu || v.help || v.drawer || v.isolate || v.presets || v.light {
                v.menu = false;
                v.help = false;
                v.drawer = false;
                v.isolate = false;
                v.presets = false;
                v.light = false;
            } else if self.camera.free().is_some() {
                self.snapshots.ui.free.on = false;
                self.photo_free_camera();
            } else if self.camera.mouselook() {
                self.toggle_mouselook();
            } else {
                self.panels.snapshot = false;
            }
            return true;
        }
        if self.photo_full_frame() {
            self.snapshots.ui.full_frame = false;
            return true;
        }
        if self.camera.free().is_some() {
            self.snapshots.ui.free.on = false;
            self.photo_free_camera();
            return true;
        }
        false
    }

    /// The free camera follows the window's switch while the window is
    /// open: it leaves the avatar where it is and the Controls window opens
    /// to drive it; the switch off, the window closed or Escape bring it
    /// back. Mouselook and a teleport end it too (`Camera::free`), which
    /// turns the switch off.
    pub(super) fn photo_free_camera(&mut self) {
        let flying = self.camera.free().is_some();
        if self.snapshots.free_flying && !flying {
            log::info!("free camera: ended by the camera (mouselook or teleport)");
            self.photo_free_finish();
            return;
        }
        if !self.panels.snapshot {
            // the next opening of the window starts on the avatar
            self.snapshots.ui.free.on = false;
        }
        let want = self.panels.snapshot && self.snapshots.ui.free.on && self.in_world();
        if want && !flying {
            let mouselook = self.camera.mouselook();
            self.camera.start_free(&mut self.world.agent, &self.settings.camera);
            if mouselook {
                self.set_mouselook_grab(false);
            }
            // the Controls window drives it, on its camera tab
            self.snapshots.free_opened_controls = !self.panels.camera_controls && !self.panels.movement_controls;
            self.panels.camera_controls = true;
            self.panels.movement_controls = false;
            self.snapshots.free_flying = true;
            log::info!("free camera: on at {:.1?}", self.camera.position);
        } else if !want && flying {
            self.camera.end_free(&self.settings.camera);
            log::info!("free camera: back to the avatar");
            self.photo_free_finish();
        }
    }

    /// After a flight: the switch off for the next one, the Controls window
    /// closed again if the flight opened it.
    fn photo_free_finish(&mut self) {
        self.snapshots.free_flying = false;
        self.snapshots.ui.free.on = false;
        if std::mem::take(&mut self.snapshots.free_opened_controls) {
            self.panels.camera_controls = false;
            self.panels.movement_controls = false;
        }
    }

    /// The free camera this frame: the movement keys, the Controls window's
    /// buttons (`i`, held like the keys) and its joysticks move it, Shift
    /// faster, Ctrl slower; with collisions it stops short of the objects
    /// and the ground and slides along them.
    pub(super) fn photo_free_move(&mut self, i: &crate::agent::MoveInput, dt: f32) {
        use crate::camera::free::{self, FreeMove};
        let Some(mut cam) = self.camera.free().copied() else {
            return;
        };
        let (stick, look) = self.controls_free;
        let axis = |pos: bool, neg: bool| f32::from(u8::from(pos)) - f32::from(u8::from(neg));
        let m = FreeMove {
            forward: axis(i.forward, i.back) + stick.y,
            // the turn keys slide: the free camera turns with the mouse
            left: axis(i.turn_left || i.strafe_left, i.turn_right || i.strafe_right) - stick.x,
            up: axis(i.up, i.down) + self.snapshots.vf_free_up,
            fast: self.shift,
            slow: self.ctrl,
        };
        cam.turn(-look.x * free::LOOK_RATE * dt, look.y * free::LOOK_RATE * dt);
        let flight = self.snapshots.ui.free;
        if !m.is_still() {
            let delta = cam.step(m, flight.speed, dt);
            cam.position += if flight.no_collisions {
                delta
            } else {
                self.photo_free_collide(cam.position, delta)
            };
        }
        if let Some(f) = self.camera.free_mut() {
            *f = cam;
        }
    }

    /// `delta` from `from`, stopped short of the objects (their real
    /// triangles, the ones a click finds, phantom ones included) and of the
    /// ground.
    fn photo_free_collide(&self, from: glam::Vec3, delta: glam::Vec3) -> glam::Vec3 {
        use crate::camera::free::{CLEARANCE, collide};
        let (world, scene) = (&self.world, &self.scene);
        let moved = collide(from, delta, |at, dir, length| {
            let (hit, _) = scene.selection_pick(world, Some((at, dir)), None, true, false, length);
            let mut room = hit.map_or(length, |p| (p - at).dot(dir).clamp(0.0, length));
            if dir.z < 0.0
                && let Some(land) = world.ground_height(at)
            {
                room = room.min(((at.z - land) / -dir.z).max(0.0));
            }
            room
        });
        // a slope under a level move
        let mut to = from + moved;
        if let Some(land) = world.ground_height(to) {
            to.z = to.z.max(land + CLEARANCE);
        }
        to - from
    }

    /// The photo's quick light (`snapshot::light`) while the window is open;
    /// the user's own as soon as it closes.
    fn photo_light(&mut self) {
        if !self.panels.snapshot {
            self.snapshots.ui.light = snapshot::light::PhotoLight::default();
        }
        let light = &mut self.snapshots.ui.light;
        // a change made elsewhere, a camera preset or "Keep this light"
        // took over part of the photo's environment
        if self.world.eep.take_photo_changed() && self.panels.snapshot {
            light.env = self.world.eep.photo_choice();
        }
        self.world.eep.set_photo(light.env);
        self.scene.attached_lights = light.attached_lights;
    }

    /// The lists of the Light section load like the environment window's.
    fn photo_env_lists(&mut self, ctx: &egui::Context) {
        if !self.env_scan {
            self.env_scan = true;
            self.env_ui.scan_left = usize::MAX;
        }
        self.env_ui.refresh(ctx, &self.world.inventory);
    }

    /// The exposure the frame is drawn with: the photo's, else Preferences'.
    pub(super) fn photo_exposure(&self) -> f32 {
        self.snapshots.ui.light.exposure.unwrap_or(self.settings.exposure)
    }

    /// The photo's light holds the clouds still.
    pub(super) fn photo_clouds_paused(&self) -> bool {
        self.snapshots.ui.light.clouds_paused
    }

    /// "Photo quality": raised while the window is open and the switch on,
    /// the user's values back as soon as either is off (they are also what
    /// a save in the meantime writes, `Settings::to_saved`).
    fn photo_quality(&mut self, renderer: &Renderer) {
        let want = self.panels.snapshot && self.settings.snapshot.quality;
        match (&self.settings.photo_held, want) {
            (None, true) => {
                let mut held = snapshot::quality::boost(&mut self.settings);
                // within what the video memory leaves, from the start: the
                // whole boost is never drawn first on a card short of it
                let plan = self.photo_quality_plan(renderer, held.user(), None, false);
                if plan.reduced() {
                    held.retarget(&mut self.settings, plan.values);
                }
                let samples = self.msaa_samples(renderer);
                log::info!(
                    "photo quality: on (draw distance {} m, shadows {}, LOD {}, MSAA {samples}x{})",
                    self.settings.draw_distance,
                    self.settings.shadow_quality,
                    self.settings.lod_factor,
                    if plan.reduced() {
                        ", lowered for the video memory"
                    } else if samples < 8 {
                        ", the most this GPU supports"
                    } else {
                        ""
                    }
                );
                if plan.reduced() {
                    self.photo_quality_log(renderer, &plan);
                }
                let now = Instant::now();
                self.snapshots.fit = Some(QualityFit {
                    step: plan.step,
                    checked: now,
                    changed: now,
                });
                self.snapshots.fit_note = plan.reduced();
                self.settings.photo_held = Some(held);
                self.snapshots.settings_changed = true;
            }
            (Some(_), false) => {
                if let Some(mut held) = self.settings.photo_held.take() {
                    held.sync(&self.settings);
                    held.restore(&mut self.settings);
                }
                log::info!("photo quality: off, the user's settings back");
                self.snapshots.fit = None;
                self.snapshots.settings_changed = true;
            }
            (Some(_), true) => {
                // what the user changes meanwhile (Preferences) is theirs
                let mut held = self.settings.photo_held.take();
                if let Some(h) = held.as_mut() {
                    h.sync(&self.settings);
                    self.photo_quality_fit(renderer, h);
                }
                self.settings.photo_held = held;
            }
            (None, false) => {}
        }
        // the textures in view at full resolution only with the whole boost:
        // lowered, the video memory has no room for them either
        self.scene.textures.full_resolution = self.settings.photo_held.is_some() && self.snapshots.fit.as_ref().is_none_or(|f| f.step == 0);
    }

    /// The photo quality follows the video memory while it is on (the
    /// budget the driver grants can shrink while the window stays open):
    /// a step down as soon as the boost no longer fits, a step up once
    /// there is room to spare (`snapshot::quality::choose`). Never while a
    /// photo is drawn: its passes keep the targets they started with.
    fn photo_quality_fit(&mut self, renderer: &Renderer, held: &mut snapshot::quality::Held) {
        let now = Instant::now();
        let Some(fit) = &self.snapshots.fit else {
            return;
        };
        if self.snapshots.job.is_some() || now.duration_since(fit.checked) < FIT_EVERY || now.duration_since(fit.changed) < FIT_SETTLE {
            return;
        }
        let (current, may_raise) = (fit.step, now.duration_since(fit.changed) >= FIT_RAISE_AFTER);
        let plan = self.photo_quality_plan(renderer, held.user(), Some(current), may_raise);
        let Some(fit) = &mut self.snapshots.fit else {
            return;
        };
        fit.checked = now;
        if plan.step == current {
            return;
        }
        fit.step = plan.step;
        fit.changed = now;
        held.retarget(&mut self.settings, plan.values);
        self.snapshots.settings_changed = true;
        self.snapshots.fit_note |= plan.step > current;
        log::info!(
            "photo quality: {} (draw distance {} m, shadows {}, MSAA {}x)",
            if plan.step > current { "lowered" } else { "raised" },
            self.settings.draw_distance,
            self.settings.shadow_quality,
            self.msaa_samples(renderer),
        );
        self.photo_quality_log(renderer, &plan);
    }

    /// The step of the photo quality for the video memory of the moment.
    fn photo_quality_plan(
        &mut self,
        renderer: &Renderer,
        user: crate::graphics_presets::Values,
        current: Option<usize>,
        may_raise: bool,
    ) -> snapshot::quality::Plan {
        let (w, h) = renderer.size();
        let screen = snapshot::quality::Screen {
            pixels: u64::from(w) * u64::from(h),
            max_samples: renderer.msaa_supported().iter().copied().filter(|&n| n <= 8).max().unwrap_or(1),
        };
        let memory = self.photo_memory(renderer);
        snapshot::quality::plan(memory, user, screen, current, may_raise)
    }

    /// The video memory as the photo quality reads it: the driver's
    /// figures (what it counts or the live resources, the larger) and the
    /// textures' pressure; in demo with `quality-tight`, a budget of its own.
    fn photo_memory(&mut self, renderer: &Renderer) -> Option<snapshot::quality::Memory> {
        let video = renderer.video_memory.read(&renderer.device)?;
        let mut memory = snapshot::quality::Memory {
            budget: video.budget,
            usage: video.usage.max(video.allocated),
            pressure: self.scene.textures.stats.pressure,
        };
        let frame = self.frame_count;
        if let Some(demo) = self.snapshots.demo.as_mut()
            && let Some(tight) = demo.tight.as_mut()
        {
            let base = *tight.base.get_or_insert(memory.usage);
            let room = if frame >= demo.frame {
                tight.later.unwrap_or(tight.room)
            } else {
                tight.room
            };
            memory.budget = base + (room << 20);
        }
        Some(memory)
    }

    /// The step chosen, with the figures it was chosen on.
    fn photo_quality_log(&mut self, renderer: &Renderer, plan: &snapshot::quality::Plan) {
        let Some(m) = self.photo_memory(renderer) else {
            return;
        };
        let live = renderer.video_memory.read(&renderer.device).map_or(0, |v| v.allocated);
        log::info!(
            "photo quality: step {} of {} for the video memory, about {} MB beyond the user's settings ({} MB taken, {} MB of resources, of a budget of {} MB, {} MB kept free, texture pressure {})",
            plan.step,
            plan.steps - 1,
            plan.extra >> 20,
            m.usage >> 20,
            live >> 20,
            m.budget >> 20,
            snapshot::quality::margin(m) >> 20,
            m.pressure,
        );
    }

    /// AURORA_DEMO_SNAPSHOT=open | full[-<W>x<H> proportions, e.g. full-4x5] |
    /// save:<W>x<H>[:png|jpg|bmp][:<folder>] | inventory… | profile[-fail] |
    /// dof | full-dof | prefs-dof | classic | viewfinder[-<words>]
    /// (AURORA_DEMO_SNAPSHOT_FRAME, 700 by default): the window at frame
    /// 600, the photo saved, sent or posted at that frame.
    fn photo_demo(&mut self, renderer: &Renderer) {
        if !self.demo {
            return;
        }
        if self.snapshots.demo.is_none() {
            let Ok(mode) = std::env::var("AURORA_DEMO_SNAPSHOT") else {
                return;
            };
            let frame = std::env::var("AURORA_DEMO_SNAPSHOT_FRAME")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(700);
            let mut parts = mode.split(':');
            let kind = parts.next().unwrap_or("open");
            // quality-tight[:<MB>[:<MB later>]], 900 MB by default
            let tight = (kind == "quality-tight").then(|| {
                let mut mb = mode.split(':').skip(1).map(|v| v.trim().parse::<u64>().ok());
                DemoTight {
                    room: mb.next().flatten().unwrap_or(900),
                    later: mb.next().flatten(),
                    base: None,
                }
            });
            // save-quality: the same with the photo quality on
            let save = kind.starts_with("save").then(|| {
                let size = parts
                    .next()
                    .and_then(|s| s.split_once('x'))
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .unwrap_or((3840, 2160));
                let format = match parts.next() {
                    Some("jpg" | "jpeg") => snapshot::FileFormat::Jpeg,
                    Some("bmp") => snapshot::FileFormat::Bmp,
                    _ => snapshot::FileFormat::Png,
                };
                // a folder may hold a drive letter: the rest of the value
                let rest: Vec<&str> = parts.collect();
                let folder = (!rest.is_empty()).then(|| PathBuf::from(rest.join(":")));
                (size, format, folder)
            });
            // full-4x5: the full-frame view with those proportions
            let aspect = kind.strip_prefix("full-").and_then(|a| {
                snapshot::Aspect::ALL
                    .into_iter()
                    .find(|x| x.short().is_some_and(|s| s.replace(':', "x") == a))
            });
            // inventory[:<side>|window], inventory-ask (the price left on
            // screen), inventory-poor (a balance below it)
            let inventory = kind.starts_with("inventory").then(|| {
                let side = std::env::var("AURORA_DEMO_SNAPSHOT").ok().and_then(|m| {
                    let v = m.split(':').nth(1)?.to_owned();
                    inventory::TextureSize::ALL
                        .into_iter()
                        .find(|t| t.side().map_or(v == "window", |s| s.to_string() == v))
                });
                DemoInventory {
                    size: side.unwrap_or(inventory::TextureSize::S512),
                    confirm: kind == "inventory",
                    poor: kind == "inventory-poor",
                }
            });
            let words: Vec<&str> = kind.split('-').collect();
            let viewfinder = (words[0] == "viewfinder").then(|| {
                use crate::snapshot::viewfinder::Display;
                let has = |w: &str| words.contains(&w);
                DemoViewfinder {
                    display: if has("essential") {
                        Display::Essential
                    } else if has("hidden") {
                        Display::Hidden
                    } else {
                        Display::Full
                    },
                    menu: has("menu"),
                    help: has("help"),
                    drawer: has("drawer"),
                    light: has("light"),
                    focus_mode: if has("mf") {
                        Some(crate::dof::PhotoFocus::Manual)
                    } else if has("afc") {
                        Some(crate::dof::PhotoFocus::Pointer)
                    } else {
                        None
                    },
                    point: has("point"),
                    focus: has("focus"),
                    shoot: has("shoot"),
                    group: has("group"),
                    sharp: has("sharp"),
                    pad: has("pad"),
                    timer: has("timer"),
                    ael: has("ael"),
                    frozen: has("frozen"),
                    controls: if has("hover") {
                        crate::snapshot::viewfinder::OnScreen::Hover
                    } else if has("nocontrols") {
                        crate::snapshot::viewfinder::OnScreen::Hidden
                    } else {
                        crate::snapshot::viewfinder::OnScreen::Always
                    },
                    tilt: if has("tilt90") {
                        90.0
                    } else if has("tilt") {
                        20.0
                    } else {
                        0.0
                    },
                    poser: has("poser"),
                }
            });
            let style = if kind == "classic" {
                Some(crate::settings::PhotoWindowStyle::Legacy)
            } else if viewfinder.is_some() {
                Some(crate::settings::PhotoWindowStyle::Aurora)
            } else {
                // the panel's scenarios keep the panel (open, full…, save…,
                // inventory…, profile…, dof, quality, light, panel)
                Some(crate::settings::PhotoWindowStyle::Panel)
            };
            self.snapshots.demo = Some(DemoPhoto {
                frame,
                save,
                full_frame: kind.starts_with("full"),
                aspect,
                inventory,
                profile: kind.starts_with("profile"),
                postcard: kind.starts_with("postcard"),
                dof: kind.ends_with("dof") && kind != "prefs-dof",
                prefs_dof: kind == "prefs-dof" || words.contains(&"prefsdof"),
                quality: kind.ends_with("quality") || tight.is_some(),
                tight,
                light: kind == "light",
                windlight: kind.starts_with("windlight").then(|| kind == "windlight-keep"),
                style,
                viewfinder,
            });
        }
        if self.frame_count == 600 && self.snapshots.demo.is_some() {
            self.photo_look_demo();
        }
        let Some(demo) = self.snapshots.demo.as_ref() else {
            return;
        };
        if self.frame_count == 600 {
            self.panels.snapshot = true;
            self.panels.chat = false;
            self.panels.people = false;
            self.panels.perf = false;
            self.snapshots.ui.full_frame = demo.full_frame;
            if let Some(a) = demo.aspect {
                self.settings.snapshot.aspect = a;
            }
            if let Some(inv) = &demo.inventory {
                self.snapshots.ui.destination = Destination::Inventory;
                self.settings.snapshot.inventory_size = inv.size;
                if inv.poor {
                    self.world.balance = Some(5);
                }
            }
            if demo.quality {
                self.settings.snapshot.quality = true;
            }
            if demo.light {
                let mut light = snapshot::light::PhotoLight {
                    exposure: Some(1.6),
                    clouds_paused: true,
                    attached_lights: false,
                    ..Default::default()
                };
                light.env.set_time(Some(3));
                self.snapshots.ui.light = light;
            }
            if demo.windlight.is_some() {
                let (sky, water) = crate::demo::env::photo_picks();
                let env = &mut self.snapshots.ui.light.env;
                env.pick(crate::world::env_select::SettingsKind::Sky, Some(sky));
                env.pick(crate::world::env_select::SettingsKind::Water, Some(water));
                env.sun = Some(crate::ui::environment::rotation_from(80.0, 12.0));
            }
            if demo.dof {
                self.settings.snapshot.dof = crate::dof::PhotoDof {
                    enabled: true,
                    f_number: 1.4,
                    focus: crate::dof::PhotoFocus::Manual,
                    distance: 3.3,
                };
            }
            if demo.prefs_dof {
                // a strong lens (its defaults, f/9 at 50 mm, hardly blur
                // the viewfinder's wide view), the window's own at All
                // sharp (the viewfinder's words below decide for it): the
                // view must stay sharp
                self.settings.dof.enabled = true;
                self.settings.dof.f_number = 1.0;
                self.settings.dof.focal_length = 100.0;
                self.settings.snapshot.dof.enabled = false;
            }
            if demo.profile {
                self.snapshots.ui.destination = Destination::Profile;
                self.snapshots.ui.caption = aurora_i18n::tr!("demo-photo-caption").to_owned();
            }
            if demo.postcard {
                self.snapshots.ui.destination = Destination::Email;
                self.snapshots.ui.postcard.to = "friend@example.com".into();
                self.snapshots.ui.postcard.message = aurora_i18n::tr!("demo-photo-caption").to_owned();
            }
            if let Some(v) = &demo.viewfinder {
                let s = &mut self.settings.snapshot;
                s.viewfinder.display = v.display;
                s.dof.enabled = !v.sharp;
                s.dof.f_number = 1.8;
                s.dof.focus = v.focus_mode.unwrap_or(crate::dof::PhotoFocus::Click);
                if v.focus_mode == Some(crate::dof::PhotoFocus::Manual) {
                    s.dof.distance = 3.3;
                }
                if v.point {
                    s.viewfinder.area = crate::snapshot::viewfinder::AfArea::Point;
                }
                s.viewfinder.pad = v.pad;
                s.viewfinder.timer = if v.timer { 2 } else { 0 };
                s.viewfinder.controls = v.controls;
                if v.tilt != 0.0 {
                    self.camera.roll = v.tilt.to_radians();
                }
                self.snapshots.ui.freeze_world = v.frozen;
                let vf = &mut self.snapshots.vf;
                vf.ae_lock = v.ael;
                vf.menu = v.menu;
                vf.help = v.help;
                vf.drawer = v.drawer;
                vf.light = v.light;
                vf.hinted = v.menu || v.help || v.drawer || vf.looks || v.light;
                if v.poser && self.ik_editor.toggle_poser() {
                    crate::ui::widgets::open_floater(&self.egui_ctx, "poser", &mut self.ik_editor.open);
                }
            }
        }
        // (once the window has its layout)
        if demo.windlight.is_some() && self.frame_count == 660 {
            self.snapshots.ui.reveal_light = true;
        }
        if demo.windlight == Some(true) {
            if self.frame_count == 900 {
                log::info!("demo photo: the photo's light kept");
                self.photo_action(Action::KeepLight, renderer);
            }
            if self.frame_count == 960 {
                self.panels.snapshot = false;
            }
            return;
        }
        if demo.profile {
            if self.frame_count == demo.frame {
                self.snapshots.quiet = false;
                self.photo_to_profile(renderer);
            }
            return;
        }
        if demo.postcard {
            if self.frame_count == demo.frame {
                self.snapshots.quiet = false;
                self.photo_to_postcard(renderer);
            }
            return;
        }
        if let Some(inv) = &demo.inventory {
            let (frame, confirm) = (demo.frame, inv.confirm);
            if self.frame_count == frame {
                self.snapshots.quiet = false;
                self.photo_to_inventory(renderer);
            }
            if confirm && self.frame_count == frame + 60 {
                self.photo_action(Action::Confirm(true), renderer);
            }
            return;
        }
        if self.frame_count == demo.frame
            && let Some((size, format, folder)) = demo.save.clone()
        {
            let s = &mut self.settings.snapshot;
            s.resolution = snapshot::Resolution::Custom;
            s.keep_ratio = false;
            s.custom = [size.0, size.1];
            s.format = format;
            if let Some(folder) = folder {
                s.folder = Some(folder);
            }
            self.snapshots.ui.destination = Destination::Disk;
            self.photo_action(Action::Take, renderer);
        }
    }
}

/// The Light section's view of the environment beneath the photo's.
fn env_view<'a>(
    env_ui: &'a crate::ui::environment::EnvironmentUi,
    eep: &crate::world::eep_env::EnvSelector,
) -> crate::ui::snapshot::EnvView<'a> {
    let sky = eep.shown_sky().unwrap_or_default();
    crate::ui::snapshot::EnvView {
        catalog: env_ui.catalog(),
        own: eep.local_view(),
        sun: crate::ui::environment::azimuth_elevation(sky.sun_rotation),
        locked: eep.rlva_locked,
        loading: eep.photo_loading(),
    }
}
