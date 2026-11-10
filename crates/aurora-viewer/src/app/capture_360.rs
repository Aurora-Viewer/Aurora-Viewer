//! The 360° snapshot on the app's side: a format of the photo window
//! (`ui/snapshot.rs`), taken by its shutter. Port of LLFloater360Capture
//! (indra/newview/llfloater360capture.cpp, originally LGPL 2.1): while the
//! photo window is open in 360° the simulator is asked to send everything
//! around (the InterestList capability, mode "360"); a capture freezes the
//! world, hides every avatar when asked, and draws six square 90° faces from
//! the camera's position through the photo frames, one per frame. Each face
//! is copied on the GPU into a cube (`Renderer::snapshot_request` with
//! `SnapshotSpec::cube`): no face is read back, the main thread waits for none
//! of them, and the renderer projects the cube to the panorama
//! (`aurora_render::equirect`) and reads only that back, asynchronously.
//!
//! The panorama goes to a JPEG with its XMP block (`snapshot::capture_360`)
//! or, for the inventory, rolled so that the camera's heading is its middle
//! column and uploaded as a 2048 x 1024 texture named for the marker
//! (`capture_360::item_name`) to the Photo Album. It is shown in the photo
//! window by the panorama viewer (`aurora_render::panorama`), which the Legacy
//! Viewer's three.js page also is; the Legacy Viewer has no inventory upload
//! and no viewer in the inventory.
//!
//! Differences from the Legacy Viewer, deliberate: the capture is taken by the
//! shutter, not when the window opens; the disk panorama is four faces wide
//! (512 to 8192 pixels, `Quality::output`) instead of always 4096; the viewer
//! also zooms; the colour looks of the photo window apply to the faces.

use super::App;
use super::snapshot::PhotoFrame;
use crate::camera::FixedView;
use crate::snapshot::capture_360::{self as rules, Meta, Quality};
use aurora_net::{InterestListMode, NetCommand};
use aurora_render::preview::Image;
use aurora_render::{CAPTURE_CAMERAS, EquirectJob, Renderer, SnapshotSpec};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Frames a face may wait for the render thread before the capture is given
/// up (a GPU that never finishes: the photo's own limit is 30 s).
const FACE_WAIT: u32 = 1800;

/// The window's aspect the textures' interest is computed for while the photo
/// window is open in 360°: wide enough for every direction to count as in
/// view (`CullView::with_texture_camera` caps the half field at a half turn).
pub(super) const OMNI_TEXTURE_ASPECT: f32 = 1000.0;

/// The frame `AURORA_DEMO_SNAPSHOT_360` opens the photo window.
const DEMO_OPEN_FRAME: u64 = 600;
/// The frame it presses the shutter (the window has been drawn by then).
const DEMO_SHUTTER_FRAME: u64 = 630;

/// Where a capture goes: a JPEG (the folder's, or the file chosen) or the
/// inventory (at this price, already confirmed).
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Shutter {
    Disk(Option<PathBuf>),
    Inventory(i32),
}

/// A capture in progress.
enum Stage {
    /// `next` is the capture camera to draw, `asked` whether its frame was
    /// asked and not yet sent, `waited` how many frames it has been.
    Faces {
        next: usize,
        asked: bool,
        waited: u32,
        started: Instant,
    },
    /// The six faces sent; the cube projected and read back by the renderer.
    Projecting { started: Instant, faces_ms: u128 },
}

/// What a capture was started with: the settings and the camera of that
/// instant, so that the window's controls and the camera's motion during the
/// capture change nothing of it.
#[derive(Debug, Clone, PartialEq)]
struct Plan {
    quality: Quality,
    face_size: u32,
    /// The panorama's size.
    output: (u32, u32),
    hide_avatars: bool,
    /// Degrees, for the XMP block and the roll.
    heading: u32,
    /// The eye of all six faces.
    eye: glam::Vec3,
    dest: Shutter,
}

impl Plan {
    fn new(quality: Quality, hide_avatars: bool, forward: glam::Vec3, eye: glam::Vec3, max_texture: u32, dest: Shutter) -> Self {
        // the inventory's panorama is the texture the grid takes, whatever
        // the quality chosen for the disk
        let face_size = match dest {
            Shutter::Inventory(_) => rules::INVENTORY_FACE,
            Shutter::Disk(_) => quality.face_size(),
        }
        .min(max_texture);
        Plan {
            quality,
            face_size,
            output: rules::output_size(face_size, max_texture),
            hide_avatars,
            heading: rules::heading_degrees(forward),
            eye,
            dest,
        }
    }
}

/// What the renderer's answer means to the capture in hand.
#[derive(Debug, PartialEq)]
enum Judged {
    /// An answer of a capture given up (or none in progress): thrown away.
    Ignore,
    /// The panorama could not be made.
    Failed,
    Ready(u32, u32, Vec<u8>),
}

/// The last panorama, as the photo window shows it.
pub(super) struct Shown {
    pub image: Arc<Image>,
    /// Radians: where the viewer opens (the camera's heading on disk, the
    /// middle column for the inventory's rolled picture).
    pub yaw: f32,
    /// Which capture it is, to start the viewer afresh for a new one.
    pub serial: u64,
}

/// `AURORA_DEMO_SNAPSHOT_360=<folder>[:preview|medium|high|maximum][:hide][:inventory][:panel|:classic]`.
struct Demo {
    folder: PathBuf,
    quality: Option<Quality>,
    hide: bool,
    inventory: bool,
    /// The photo window's style for the demo (the viewfinder otherwise).
    style: Option<crate::settings::PhotoWindowStyle>,
    opened: bool,
    pressed: bool,
}

type Saved = Result<PathBuf, String>;

#[derive(Default)]
pub(super) struct Capture360 {
    stage: Option<Stage>,
    /// The camera looks along a face this frame; its own view comes back
    /// next frame.
    face_view: bool,
    /// The face drawn this frame (its side), for the photo frame.
    face_frame: Option<u32>,
    /// What the capture in progress (or the last) was started with.
    plan: Option<Plan>,
    /// The colour looks of the photo window, applied to each face.
    look: Option<aurora_render::looks::PhotoLook>,
    /// The number of the capture in hand, carried by its panorama job and
    /// its answer: an answer of another number (a capture given up, whenever
    /// it comes, or never) is thrown away, and nothing is ever owed.
    epoch: u64,
    /// A shutter pressed, waiting for the world to be ready.
    wanted: Option<Shutter>,
    /// The photo window was open in 360° last frame.
    interest: bool,
    /// The picture shown in the photo window, being made on a worker.
    display_work: Option<crossbeam_channel::Receiver<Arc<Image>>>,
    display_yaw: f32,
    shown: Option<Shown>,
    serial: u64,
    saving: Option<crossbeam_channel::Receiver<Saved>>,
    dialog: Option<crossbeam_channel::Receiver<Option<PathBuf>>>,
    demo: Option<Demo>,
    demo_read: bool,
    demo_inventory_asked: bool,
}

impl Capture360 {
    /// A capture is drawing its faces or waiting for its panorama: the
    /// world holds still meanwhile (LLFloater360Capture::freezeWorld).
    pub fn capturing(&self) -> bool {
        self.stage.is_some()
    }

    /// Work that keeps the photo window's shutter busy.
    pub fn busy(&self) -> bool {
        self.stage.is_some() || self.saving.is_some() || self.dialog.is_some()
    }

    /// A face is being drawn this frame (no depth of field: it would differ
    /// from face to face).
    pub fn drawing_face(&self) -> bool {
        self.face_frame.is_some()
    }

    /// The panorama shown is let go (the photo window closed): its pixels
    /// here and its copy on the GPU, which the renderer drops with the last
    /// reference.
    pub fn release_shown(&mut self) {
        self.shown = None;
        self.display_work = None;
    }

    /// The renderer's answer for the capture in hand. The projection stage
    /// ends here, answered or failed, so that giving the capture up after a
    /// failed panorama owes nothing.
    fn judge(&mut self, answer: aurora_render::EquirectAnswer) -> Judged {
        let (id, pixels) = answer;
        if id != self.epoch || !matches!(self.stage, Some(Stage::Projecting { .. })) {
            return Judged::Ignore;
        }
        self.stage = None;
        match pixels {
            Some((w, h, rgba)) => Judged::Ready(w, h, rgba),
            None => Judged::Failed,
        }
    }

    /// The progress line of the photo window.
    pub fn busy_line(&self) -> Option<(f32, String)> {
        match &self.stage {
            Some(Stage::Faces { next, .. }) => Some((
                0.1 + 0.4 * *next as f32 / 6.0,
                aurora_i18n::tr!("capture-360-capturing", face = *next + 1).to_owned(),
            )),
            Some(Stage::Projecting { .. }) => Some((0.6, aurora_i18n::tr!("capture-360-projecting").to_owned())),
            None if self.saving.is_some() => Some((0.85, aurora_i18n::tr!("capture-360-saving").to_owned())),
            None => None,
        }
    }

    /// The panorama the photo window shows, if there is one.
    pub fn shown(&self) -> Option<&Shown> {
        self.shown.as_ref()
    }

    /// The face frame of this frame, as the photo frame describes one.
    pub fn photo_frame(&self, window: (u32, u32), render_scale_percent: u32) -> Option<PhotoFrame> {
        let size = self.face_frame?;
        Some(PhotoFrame {
            aspect: 1.0,
            tile: glam::Mat4::IDENTITY,
            height: size,
            scale: crate::snapshot::viewfinder_scale((size, size), window, render_scale_percent),
            look: self.look.clone(),
            context: aurora_render::PhotoContext::None,
            cube_face: true,
        })
    }
}

impl App {
    /// Every frame, before the photo frame is chosen: the camera of the
    /// last face put back, the window's opening and closing told to the
    /// simulator, the panorama taken in, the next face set up.
    pub(super) fn capture_360_frame(&mut self, renderer: &mut Renderer) {
        self.capture_360_demo(renderer);
        self.panels.snapshot_pano = self.snapshots.ui.pano;
        self.capture_360.face_frame = None;
        if std::mem::take(&mut self.capture_360.face_view) {
            self.camera.release_fixed_view();
            renderer.cut_motion_history();
        }
        let open = self.pano_open() && self.in_world();
        if open != self.capture_360.interest {
            self.capture_360.interest = open;
            // LLFloater360Capture's constructor and destructor: everything
            // around while the 360° window is open, the camera's view after
            let mode = if open {
                InterestListMode::Capture360
            } else {
                InterestListMode::Default
            };
            log::info!("360 capture: interest list mode {} asked of the regions", mode.as_str());
            self.send(NetCommand::SetInterestListMode(mode));
        }
        if !open && (self.capture_360.stage.is_some() || self.capture_360.wanted.is_some()) {
            self.capture_360_abandon(renderer, "the 360° format was left");
        }
        self.capture_360_take_answers(renderer);
        // (a photo's preview or part still on its way would be taken for a
        // face, and a panorama owed to a capture given up for this one's)
        if self.capture_360.wanted.is_some()
            && self.capture_360.stage.is_none()
            && open
            && !self.photo_work_pending(renderer)
            && let Some(dest) = self.capture_360.wanted.take()
        {
            self.capture_360_start(renderer, dest);
        }
        self.capture_360_set_up_face(renderer);
        self.capture_360_poll_workers();
    }

    /// The photo window is open in its 360° format.
    pub(super) fn pano_open(&self) -> bool {
        self.panels.snapshot && self.snapshots.ui.pano
    }

    /// The shutter in 360°: a capture to `dest` as soon as the world is ready.
    pub(super) fn capture_360_shutter(&mut self, dest: Shutter) {
        if self.capture_360.wanted.is_some() || self.photo_busy() {
            return;
        }
        self.snapshots.message = None;
        self.snapshots.message_item = None;
        self.snapshots.message_profile = false;
        self.capture_360.wanted = Some(dest);
    }

    /// LLFloater360Capture::capture360Images: the world frozen, the
    /// avatars hidden when asked, the heading recorded, six faces to draw.
    fn capture_360_start(&mut self, renderer: &Renderer, dest: Shutter) {
        let settings = self.settings.snapshot.capture_360;
        let plan = Plan::new(
            settings.quality,
            settings.hide_avatars,
            self.camera.forward(),
            self.camera.position,
            renderer.max_texture_dimension(),
            dest,
        );
        self.world.photo_isolation.set_hide_avatars(settings.hide_avatars);
        self.capture_360.epoch += 1;
        self.capture_360.look = self.capture_360_look(plan.face_size);
        self.capture_360.plan = Some(plan.clone());
        self.capture_360.stage = Some(Stage::Faces {
            next: 0,
            asked: false,
            waited: 0,
            started: Instant::now(),
        });
        if plan.output.0 != plan.face_size * 4 {
            log::info!(
                "360 capture: the card's textures end at {}: a panorama of {} x {} instead of {}",
                renderer.max_texture_dimension(),
                plan.output.0,
                plan.output.1,
                plan.face_size * 4
            );
        }
        log::info!(
            "360 capture: heading {}°, faces of {} px, panorama {} x {}, avatars {}, {}",
            plan.heading,
            plan.face_size,
            plan.output.0,
            plan.output.1,
            if plan.hide_avatars { "hidden" } else { "drawn" },
            match plan.dest {
                Shutter::Disk(_) => "to disk",
                Shutter::Inventory(_) => "to the inventory",
            }
        );
    }

    /// The photo window's looks that a face can take alone (the colour
    /// ones: a vignette, a blur or a grain would differ from face to face),
    /// placed over a face.
    fn capture_360_look(&mut self, face: u32) -> Option<aurora_render::looks::PhotoLook> {
        use crate::snapshot::looks;
        let stack = self.settings.snapshot.looks.clone();
        let settings = self.settings.snapshot.look_settings.clone();
        let mut layers = looks::resolve_stack(&stack, &settings, self.photo_luts());
        layers.retain(|(look, _)| look.ops.iter().all(aurora_render::looks::LookOp::is_pointwise));
        looks::frame_look(
            &layers,
            looks::Placement {
                origin: [0.0, 0.0],
                photo: (face, face),
                scale: 1.0,
            },
            None,
            false,
            None,
        )
    }

    /// The face to draw this frame: the camera swapped for the capture
    /// camera (put back next frame), the square photo frame asked. A face
    /// the render thread did not take is asked again, as a photo's part is.
    fn capture_360_set_up_face(&mut self, renderer: &mut Renderer) {
        let (next, asked, waited) = match &self.capture_360.stage {
            Some(Stage::Faces { next, asked, waited, .. }) => (*next, *asked, *waited),
            _ => return,
        };
        let Some(plan) = self.capture_360.plan.clone() else {
            return;
        };
        if asked {
            if renderer.failed() || renderer.device_lost() {
                self.capture_360_fail(renderer, "the frame of a face was not drawn");
                return;
            }
            if waited >= FACE_WAIT {
                self.capture_360_fail(renderer, "a face waited too long for the render thread");
                return;
            }
        } else if next == 0 && !self.photo_world_frozen() {
            // the freeze takes hold at the start of the next frame: the
            // first face waits for it, so that the six faces share one
            // instant (the clock, the animations, the avatars hidden)
            return;
        }
        if let Some(Stage::Faces { asked, waited, .. }) = &mut self.capture_360.stage {
            if *asked {
                *waited += 1;
            }
            *asked = true;
        }
        let face = CAPTURE_CAMERAS[next];
        let size = plan.face_size;
        // the capture camera: the eye where the capture started, the view
        // along the face, 90° high and wide
        // (LLViewerCamera::setView(F_PI_BY_TWO), setAspect(1))
        self.camera.fixed_view = Some(FixedView {
            eye: plan.eye,
            forward: face.forward,
            up: face.up,
            fov_y: std::f32::consts::FRAC_PI_2,
        });
        self.capture_360.face_view = true;
        renderer.cut_motion_history();
        renderer.snapshot_request = Some(SnapshotSpec {
            width: size,
            height: size,
            interface: false,
            huds: false,
            photo: (size, size),
            cube: Some(next as u8),
        });
        self.capture_360.face_frame = Some(size);
    }

    /// After a frame was drawn: the face asked, if this frame carried it
    /// (the renderer took the request), and the projection asked when it was
    /// the last. Nothing comes back for a face: it stays on the GPU.
    pub(super) fn capture_360_drawn(&mut self, renderer: &mut Renderer) {
        let Some(Stage::Faces { asked, .. }) = &self.capture_360.stage else {
            return;
        };
        if !*asked || renderer.snapshot_request.is_some() {
            return;
        }
        let Some(plan) = self.capture_360.plan.clone() else {
            return;
        };
        let Some(Stage::Faces {
            next,
            asked,
            waited,
            started,
            ..
        }) = &mut self.capture_360.stage
        else {
            return;
        };
        *next += 1;
        *asked = false;
        *waited = 0;
        if *next < CAPTURE_CAMERAS.len() {
            return;
        }
        let faces_ms = started.elapsed().as_millis();
        log::info!("360 capture: 6 faces of {} px sent in {faces_ms} ms", plan.face_size);
        // the faces are in the frames sent: the avatars come back (the world
        // stays frozen until the panorama is read back)
        self.world.photo_isolation.set_hide_avatars(false);
        renderer.equirect_request = Some(Box::new(EquirectJob {
            id: self.capture_360.epoch,
            width: plan.output.0,
            height: plan.output.1,
        }));
        self.capture_360.stage = Some(Stage::Projecting {
            started: Instant::now(),
            faces_ms,
        });
    }

    /// The panorama the renderer made, if it came back.
    fn capture_360_take_answers(&mut self, renderer: &mut Renderer) {
        let (started, faces_ms) = match &self.capture_360.stage {
            Some(Stage::Projecting { started, faces_ms }) => (*started, *faces_ms),
            _ => {
                // no capture awaits a panorama: one that comes is a capture
                // given up's, and goes nowhere
                renderer.equirect = None;
                return;
            }
        };
        if renderer.failed() || renderer.device_lost() {
            self.capture_360_fail(renderer, "the panorama was not made");
            return;
        }
        let Some(answer) = renderer.equirect.take() else {
            if started.elapsed().as_secs() > 60 {
                self.capture_360_fail(renderer, "the panorama waited too long");
            }
            return;
        };
        let (width, height, rgba) = match self.capture_360.judge(answer) {
            Judged::Ignore => return,
            Judged::Failed => {
                self.capture_360_fail(renderer, "the panorama could not be made");
                return;
            }
            Judged::Ready(width, height, rgba) => (width, height, rgba),
        };
        log::info!(
            "360 capture: {width} x {height} panorama read back {} ms after the last face ({faces_ms} ms of faces)",
            started.elapsed().as_millis()
        );
        self.capture_360.look = None;
        self.capture_360_arrived(width, height, rgba);
    }

    /// The panorama in hand: shown in the photo window, and sent where the
    /// shutter said.
    fn capture_360_arrived(&mut self, width: u32, height: u32, rgba: Vec<u8>) {
        let Some(plan) = self.capture_360.plan.clone() else {
            return;
        };
        match plan.dest.clone() {
            Shutter::Disk(chosen) => {
                let region_name = self.world.region_name();
                let region_url = if region_name.is_empty() {
                    "https://secondlife.com".to_owned()
                } else {
                    crate::slurl::make(&region_name, self.world.agent.position)
                };
                let taken = crate::snapshot::local_time();
                let path = chosen.unwrap_or_else(|| {
                    let name = rules::proposed_file_name(&region_name, width, height, plan.face_size, &rules::compact_stamp(taken));
                    unique(self.settings.snapshot.folder().join(name))
                });
                let meta = Meta {
                    software: format!("Aurora Viewer {}", aurora_version::version()),
                    width,
                    height,
                    heading: plan.heading,
                    source_size: plan.quality.face_size(),
                    scaled_source_size: plan.face_size,
                    date: rules::ctime_stamp(taken),
                    region_name,
                    region_url,
                };
                let rgba = Arc::new(rgba);
                self.capture_360_show(width, height, rgba.clone(), (plan.heading as f32).to_radians(), plan.quality);
                self.capture_360_save(path, width, height, rgba, meta);
            }
            Shutter::Inventory(cost) => {
                // the XMP block with the heading does not survive the
                // texture upload: the picture is rolled instead
                let rolled = Arc::new(rules::roll_to_heading(&rgba, width, height, plan.heading));
                drop(rgba);
                self.capture_360_show(width, height, rolled.clone(), 0.0, plan.quality);
                self.photo_encode_panorama((width, height), rolled.to_vec(), cost);
                if let Some(folder) = self.capture_360.demo.as_ref().filter(|d| d.inventory).map(|d| d.folder.clone()) {
                    // (the demo has no grid to look at the item in: the
                    // picture that was encoded, beside the log)
                    let (w, h) = (width, height);
                    std::thread::spawn(move || {
                        let _ = std::fs::create_dir_all(&folder);
                        match crate::snapshot::save::encode_jpeg(w, h, &rolled, 90) {
                            Ok(bytes) => {
                                let path = folder.join("inventory_360.jpg");
                                if let Err(e) = std::fs::write(&path, bytes) {
                                    log::warn!("360 capture: demo picture not kept: {e}");
                                } else {
                                    log::info!("360 capture: demo inventory picture {}", path.display());
                                }
                            }
                            Err(e) => log::warn!("360 capture: demo picture not encoded: {e}"),
                        }
                    });
                }
            }
        }
    }

    /// The picture for the photo window, made on a worker (the panorama is
    /// reduced to what the viewer needs: 4096 wide, 8192 at Maximum).
    fn capture_360_show(&mut self, width: u32, height: u32, rgba: Arc<Vec<u8>>, yaw: f32, quality: Quality) {
        let cap = if quality == Quality::Maximum { 8192 } else { 4096 };
        let (tx, rx) = crossbeam_channel::bounded(1);
        let spawned = std::thread::Builder::new().name("aurora-360-shown".into()).spawn(move || {
            // (at full size, one copy into the shared image, not two)
            let image = if width.max(height) <= cap {
                Image {
                    size: [width, height],
                    rgba: Arc::from(rgba.as_slice()),
                }
            } else {
                let (w, h, px) = crate::snapshot::save::downscale(width, height, &rgba, cap);
                Image {
                    size: [w, h],
                    rgba: Arc::from(px),
                }
            };
            let _ = tx.send(Arc::new(image));
        });
        match spawned {
            Ok(_) => {
                self.capture_360.display_work = Some(rx);
                self.capture_360.display_yaw = yaw;
            }
            Err(e) => log::warn!("360 capture: cannot start the display thread: {e}"),
        }
    }

    /// The capture given up: what it changed put back.
    fn capture_360_abandon(&mut self, renderer: &mut Renderer, why: &str) {
        match self.capture_360.stage.take() {
            Some(Stage::Faces { asked, .. }) => {
                log::info!("360 capture: given up, {why}");
                if asked {
                    // the face asked is drawn for nobody: its request is
                    // withdrawn (one already sent is drawn into a cube that
                    // the next capture starts afresh)
                    renderer.drop_snapshot();
                }
            }
            Some(Stage::Projecting { .. }) => {
                log::info!("360 capture: given up, {why}");
                // the request not yet taken by the render thread is withdrawn,
                // an answer already there thrown away; one still to come
                // carries this capture's number, which no capture has again
                renderer.equirect_request = None;
                renderer.equirect = None;
            }
            None => {}
        }
        self.camera.release_fixed_view();
        self.capture_360.face_view = false;
        self.capture_360.wanted = None;
        self.capture_360.look = None;
        self.world.photo_isolation.set_hide_avatars(false);
    }

    fn capture_360_fail(&mut self, renderer: &mut Renderer, why: &str) {
        log::warn!("360 capture: {why}");
        self.capture_360_abandon(renderer, why);
        self.snapshots.message = Some((aurora_i18n::tr!("capture-360-failed").to_owned(), true));
    }

    /// The panorama encoded and written at `path` on a worker.
    fn capture_360_save(&mut self, path: PathBuf, width: u32, height: u32, rgba: Arc<Vec<u8>>, meta: Meta) {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let spawned = std::thread::Builder::new().name("aurora-360-save".into()).spawn(move || {
            let result = rules::encode(width, height, &rgba, &meta).and_then(|bytes| {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                }
                std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
                log::info!("360 capture: saved {} ({} bytes)", path.display(), bytes.len());
                Ok(path)
            });
            let _ = tx.send(result);
        });
        match spawned {
            Ok(_) => {
                self.capture_360.saving = Some(rx);
                self.photo_shutter();
            }
            Err(e) => log::warn!("360 capture: cannot start the save thread: {e}"),
        }
    }

    /// The save dialog's choice, the picture for the window, the save's end.
    fn capture_360_poll_workers(&mut self) {
        // (a worker that ended without an answer, a panic, is a failure
        // too: nothing may stay busy for ever)
        if let Some(rx) = &self.capture_360.display_work {
            match rx.try_recv() {
                Ok(image) => {
                    self.capture_360.display_work = None;
                    self.capture_360.serial += 1;
                    self.capture_360.shown = Some(Shown {
                        image,
                        yaw: self.capture_360.display_yaw,
                        serial: self.capture_360.serial,
                    });
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    log::warn!("360 capture: the picture for the window was not made");
                    self.capture_360.display_work = None;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        let choice = self.capture_360.dialog.as_ref().map(|rx| rx.try_recv());
        if let Some(choice) = choice
            && !matches!(choice, Err(crossbeam_channel::TryRecvError::Empty))
        {
            self.capture_360.dialog = None;
            if let Ok(Some(path)) = choice {
                if let Some(folder) = path.parent() {
                    self.settings.snapshot.folder = Some(folder.to_path_buf());
                    self.settings.save();
                }
                let path = if path.extension().is_some() {
                    path
                } else {
                    path.with_extension("jpg")
                };
                self.capture_360_shutter(Shutter::Disk(Some(path)));
            }
        }
        let saved = self.capture_360.saving.as_ref().map(|rx| rx.try_recv());
        if let Some(saved) = saved
            && !matches!(saved, Err(crossbeam_channel::TryRecvError::Empty))
        {
            self.capture_360.saving = None;
            let result = saved.unwrap_or_else(|_| Err("the save ended without an answer".to_owned()));
            self.snapshots.message = Some(match result {
                Ok(path) => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    self.snapshots.last_saved = Some(path);
                    (aurora_i18n::tr!("capture-360-saved", name = name.as_str()).to_owned(), false)
                }
                Err(e) => {
                    log::warn!("360 capture: not saved: {e}");
                    (aurora_i18n::tr!("capture-360-save-failed", error = e.as_str()).to_owned(), true)
                }
            });
        }
    }

    /// "Save as…" in 360°: the save dialog with the proposed name for the
    /// quality chosen, in the photos' folder; the capture follows its answer.
    pub(super) fn capture_360_ask_path(&mut self, renderer: &Renderer) {
        if self.capture_360.busy() || self.photo_busy() {
            return;
        }
        let name = self.capture_360_disk_name(renderer);
        let folder = self.settings.snapshot.folder();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let spawned = std::thread::Builder::new().name("aurora-360-dialog".into()).spawn(move || {
            let path = rfd::FileDialog::new()
                .set_directory(&folder)
                .set_title(aurora_i18n::tr!("capture-360-save-dialog"))
                .set_file_name(name)
                .add_filter(aurora_i18n::tr!("photo-images"), &["jpg", "jpeg"])
                .save_file();
            let _ = tx.send(path);
        });
        if let Err(e) = spawned {
            log::warn!("360 capture: cannot open the save dialog: {e}");
            return;
        }
        self.capture_360.dialog = Some(rx);
    }

    /// The proposed file name of a disk panorama at the quality chosen, with
    /// the faces and the panorama the card will make of it (`Plan::new`).
    fn capture_360_disk_name(&self, renderer: &Renderer) -> String {
        let plan = Plan::new(
            self.settings.snapshot.capture_360.quality,
            false,
            glam::Vec3::Y,
            glam::Vec3::ZERO,
            renderer.max_texture_dimension(),
            Shutter::Disk(None),
        );
        rules::proposed_file_name(
            &self.world.region_name(),
            plan.output.0,
            plan.output.1,
            plan.face_size,
            &rules::compact_stamp(crate::snapshot::local_time()),
        )
    }

    /// `AURORA_DEMO_SNAPSHOT_360=<folder>[:preview|medium|high|maximum]
    /// [:hide][:inventory][:panel|:classic]`: the photo window opened in 360° at frame 600
    /// and its shutter pressed a few frames later; the panorama is written
    /// to the folder under its proposed name (with `:inventory`, the
    /// inventory's picture is kept there instead and the upload is only
    /// logged: there is no grid).
    fn capture_360_demo(&mut self, renderer: &mut Renderer) {
        if !self.demo {
            return;
        }
        if !self.capture_360.demo_read {
            self.capture_360.demo_read = true;
            if let Ok(value) = std::env::var("AURORA_DEMO_SNAPSHOT_360") {
                self.capture_360.demo = parse_demo(&value);
            }
        }
        let in_world = self.in_world();
        let Some(demo) = self.capture_360.demo.as_mut() else {
            return;
        };
        if !in_world {
            return;
        }
        if !demo.opened && self.frame_count >= DEMO_OPEN_FRAME {
            demo.opened = true;
            let (quality, hide, inventory, style) = (demo.quality, demo.hide, demo.inventory, demo.style);
            if let Some(style) = style {
                self.settings.photo_window = style;
            }
            let s = &mut self.settings.snapshot.capture_360;
            // (the demo's quality is its own: Preview unless a word says)
            s.quality = quality.unwrap_or(Quality::Preview);
            s.hide_avatars = hide;
            log::info!(
                "demo: 360 capture at frame {}, quality {:?}, avatars {}, {}",
                self.frame_count,
                s.quality,
                if s.hide_avatars { "hidden" } else { "drawn" },
                if inventory { "to the inventory" } else { "to disk" }
            );
            self.panels.snapshot = true;
            self.snapshots.ui.pano = true;
            self.snapshots.ui.destination = if inventory {
                crate::ui::snapshot::Destination::Inventory
            } else {
                crate::ui::snapshot::Destination::Disk
            };
            return;
        }
        if demo.opened && !demo.pressed && self.frame_count >= DEMO_SHUTTER_FRAME {
            demo.pressed = true;
            let (folder, inventory) = (demo.folder.clone(), demo.inventory);
            if inventory {
                self.photo_action(crate::ui::snapshot::Action::Take, renderer);
                self.capture_360.demo_inventory_asked = true;
            } else {
                let name = self.capture_360_disk_name(renderer);
                self.capture_360_shutter(Shutter::Disk(Some(folder.join(name))));
            }
            return;
        }
        // the price of the demo's upload accepted
        if self.capture_360.demo_inventory_asked && self.snapshots.confirm.is_some() {
            self.capture_360.demo_inventory_asked = false;
            self.photo_action(crate::ui::snapshot::Action::Confirm(true), renderer);
        }
    }
}

/// `path`, or the same name with `_1`, `_2`… when the file exists.
fn unique(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let extension = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    (1..1000)
        .map(|n| path.with_file_name(format!("{stem}_{n}.{extension}")))
        .find(|candidate| !candidate.exists())
        .unwrap_or(path)
}

/// The scenario's words: the folder first (it may hold a drive letter),
/// then a quality level, `hide` and `inventory`, in any order.
fn parse_demo(value: &str) -> Option<Demo> {
    let mut parts: Vec<&str> = value.split(':').collect();
    let mut quality = None;
    let mut hide = false;
    let mut inventory = false;
    let mut style = None;
    while parts.len() > 1 {
        let word = parts[parts.len() - 1].trim();
        if word == "hide" {
            hide = true;
        } else if word == "inventory" {
            inventory = true;
        } else if word == "panel" {
            style = Some(crate::settings::PhotoWindowStyle::Panel);
        } else if word == "classic" {
            style = Some(crate::settings::PhotoWindowStyle::Legacy);
        } else if let Some(q) = Quality::from_word(word) {
            quality = Some(q);
        } else {
            break;
        }
        parts.pop();
    }
    let folder = parts.join(":");
    if folder.trim().is_empty() {
        return None;
    }
    Some(Demo {
        folder: PathBuf::from(folder.trim()),
        quality,
        hide,
        inventory,
        style,
        opened: false,
        pressed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_words_keep_the_drive_letter() {
        let d = parse_demo(r"C:\work\.scratch:high:hide").expect("a folder");
        assert_eq!(d.folder, PathBuf::from(r"C:\work\.scratch"));
        assert_eq!(d.quality, Some(Quality::High));
        assert!(d.hide && !d.inventory);
        let d = parse_demo("out:inventory").expect("a folder");
        assert!(d.inventory && d.quality.is_none() && !d.hide);
        assert!(parse_demo("").is_none());
    }

    /// The plan keeps the quality, the eye and the heading of the start;
    /// the panorama follows the faces, the card's limit, and the inventory.
    #[test]
    fn the_plan_is_the_start_of_the_capture() {
        let eye = glam::Vec3::new(10.0, 20.0, 30.0);
        let plan = Plan::new(Quality::High, true, glam::Vec3::Y, eye, 16384, Shutter::Disk(None));
        assert_eq!(plan.quality, Quality::High);
        assert_eq!(plan.face_size, Quality::High.face_size());
        assert_eq!(plan.output, (4096, 2048));
        assert_eq!(plan.eye, eye);
        assert!(plan.hide_avatars);
        let other = Plan::new(Quality::Preview, false, glam::Vec3::Y, eye, 16384, Shutter::Disk(None));
        assert_ne!(plan.face_size, other.face_size);
        // the biggest on a card of 4096: the Legacy Viewer's width
        let small = Plan::new(Quality::Maximum, false, glam::Vec3::Y, eye, 4096, Shutter::Disk(None));
        assert_eq!((small.face_size, small.output), (2048, (4096, 2048)));
        // a card whose textures end at 1024: the faces too
        let tiny = Plan::new(Quality::Maximum, false, glam::Vec3::Y, eye, 1024, Shutter::Disk(None));
        assert_eq!((tiny.face_size, tiny.output), (1024, (1024, 512)));
    }

    #[test]
    fn the_inventory_panorama_is_the_texture_the_grid_takes_whatever_the_quality() {
        for quality in Quality::ALL {
            let plan = Plan::new(quality, false, glam::Vec3::Y, glam::Vec3::ZERO, 16384, Shutter::Inventory(10));
            assert_eq!(plan.face_size, 512);
            assert_eq!(plan.output, (2048, 1024));
        }
    }

    /// A capture awaiting its panorama.
    fn projecting(epoch: u64) -> Capture360 {
        Capture360 {
            epoch,
            stage: Some(Stage::Projecting {
                started: Instant::now(),
                faces_ms: 0,
            }),
            ..Default::default()
        }
    }

    /// A panorama that could not be made ends the stage with its answer:
    /// giving the capture up after it owes nothing and the photo window is
    /// not left busy for ever.
    #[test]
    fn a_failed_panorama_leaves_nothing_busy() {
        let mut c = projecting(3);
        assert!(c.busy());
        assert_eq!(c.judge((3, None)), Judged::Failed);
        assert!(!c.busy(), "the stage is over, whatever the caller does next");
        // the caller's next step, the capture given up, changes nothing
        c.stage = None;
        assert!(!c.busy());
        // and a new capture is not held by it
        let mut next = projecting(4);
        assert_eq!(next.judge((4, Some((2, 1, vec![0; 8])))), Judged::Ready(2, 1, vec![0; 8]));
        assert!(!next.busy());
    }

    /// The answer of a capture given up, early, late or never, never
    /// reaches the next capture, and none is owed.
    #[test]
    fn the_answer_of_a_capture_given_up_is_thrown_away() {
        let mut c = projecting(5);
        assert_eq!(c.judge((4, Some((1, 1, vec![1; 4])))), Judged::Ignore, "the previous capture's");
        assert!(c.busy(), "still waiting for its own");
        assert_eq!(c.judge((5, None)), Judged::Failed);
        // an answer when nothing awaits one
        assert_eq!(c.judge((5, Some((1, 1, vec![1; 4])))), Judged::Ignore);
        // a capture that never gets its answer is given up by the caller:
        // nothing remains owed for the next one
        let mut lost = projecting(6);
        lost.stage = None;
        let mut next = projecting(7);
        assert_eq!(next.judge((6, None)), Judged::Ignore);
        assert_eq!(next.judge((7, None)), Judged::Failed);
    }

    #[test]
    fn a_file_that_exists_gets_a_number() {
        let dir = std::env::temp_dir().join(format!("aurora-360-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temporary folder");
        let first = dir.join("sl360_a.jpg");
        assert_eq!(unique(first.clone()), first);
        std::fs::write(&first, b"x").expect("file");
        assert_eq!(unique(first.clone()), dir.join("sl360_a_1.jpg"));
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
