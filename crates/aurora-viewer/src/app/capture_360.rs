//! The 360° snapshot on the app's side. Port of LLFloater360Capture
//! (indra/newview/llfloater360capture.cpp, originally LGPL 2.1): while the
//! window is open the simulator is asked to send everything around (the
//! InterestList capability, mode "360"); a capture freezes the world, hides
//! every avatar when asked, and draws six square 90° faces from the camera's
//! position through the photo frames (`Renderer::snapshot_request`), one
//! per frame; the renderer then projects them to the panorama on the GPU
//! (`aurora_render::equirect`), shown flat in the window and saved as a
//! JPEG with its XMP block (`snapshot::capture_360`). The Legacy Viewer
//! shows the panorama in a draggable three.js view: Aurora shows the
//! equirectangular image itself.

use super::App;
use crate::camera::FixedView;
use crate::snapshot::capture_360::{self as rules, Meta, Quality};
use crate::ui::capture_360::{Action, Capture360Ui, View};
use aurora_net::{InterestListMode, NetCommand};
use aurora_render::{CAPTURE_CAMERAS, EquirectJob, Renderer, SnapshotSpec};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Frames a face may wait for its answer before the capture is given up
/// (a GPU that never finishes: the photo's own limit is 30 s).
const FACE_WAIT: u32 = 1800;

/// The frame `AURORA_DEMO_SNAPSHOT_360` opens the window.
const DEMO_OPEN_FRAME: u64 = 600;

/// A capture in progress.
enum Stage {
    /// The faces drawn so far, by cube layer; `next` is the capture camera
    /// to draw, `asked` whether its frame is out, `waited` how many frames
    /// it has been.
    Faces {
        faces: [Vec<u8>; 6],
        next: usize,
        asked: bool,
        waited: u32,
        started: Instant,
    },
    /// The faces handed to the renderer, the panorama awaited.
    Projecting { started: Instant, faces_ms: u128 },
}

/// What a capture was started with: the settings and the camera of that
/// instant, so that the window's controls and the camera's motion during
/// the capture change nothing of it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Plan {
    quality: Quality,
    face_size: u32,
    hide_avatars: bool,
    /// Degrees, for the XMP block.
    heading: u32,
    /// The eye of all six faces.
    eye: glam::Vec3,
}

impl Plan {
    fn new(quality: Quality, hide_avatars: bool, forward: glam::Vec3, eye: glam::Vec3) -> Self {
        Plan {
            quality,
            face_size: quality.face_size(),
            hide_avatars,
            heading: rules::heading_degrees(forward),
            eye,
        }
    }
}

/// Renderer replies owed to captures given up: the renderer answers a
/// request once, whoever still waits for it, and the answer of a capture
/// that is gone must not reach the next capture or an ordinary photo.
#[derive(Debug, Default, PartialEq)]
struct Stale(u32);

impl Stale {
    /// A request is given up; `in_flight` when the render thread has it.
    fn abandon(&mut self, in_flight: bool) {
        if in_flight {
            self.0 += 1;
        }
    }

    /// A projection is given up: the render thread owes an answer only when
    /// the answer has not arrived and the request was no longer waiting to be
    /// taken (`withdrawn`).
    fn abandon_projecting(&mut self, arrived: bool, withdrawn: bool) {
        self.abandon(!arrived && !withdrawn);
    }

    /// An answer arrived: true when it belongs to a capture given up (and
    /// is to be thrown away).
    fn swallow(&mut self, arrived: bool) -> bool {
        if arrived && self.0 > 0 {
            self.0 -= 1;
            return true;
        }
        false
    }

    fn pending(&self) -> bool {
        self.0 > 0
    }
}

/// The last panorama made.
pub(super) struct Panorama {
    width: u32,
    height: u32,
    /// Shared with the worker that makes the preview and with the saves.
    rgba: Arc<Vec<u8>>,
    /// What the XMP block says of it.
    heading: u32,
    quality: Quality,
    face_size: u32,
    /// Local date and time of the capture.
    taken: (u16, u16, u16, u16, u16, u16),
    region_name: String,
    region_url: String,
}

/// `AURORA_DEMO_SNAPSHOT_360=<folder>[:preview|medium|high|maximum][:hide]`.
struct Demo {
    folder: PathBuf,
    quality: Option<Quality>,
    hide: bool,
    opened: bool,
    saved: bool,
}

#[derive(Default)]
pub(super) struct Capture360 {
    pub ui: Capture360Ui,
    stage: Option<Stage>,
    /// The camera looks along a face this frame; its own view comes back
    /// next frame.
    face_view: bool,
    /// The face drawn this frame (its side), for the photo frame.
    face_frame: Option<u32>,
    /// What the capture in progress (or the last) was started with.
    plan: Option<Plan>,
    /// Panorama answers still owed to captures given up.
    stale_equirect: Stale,
    /// The preview of the last panorama, reduced on a worker.
    preview_work: Option<crossbeam_channel::Receiver<(u32, u32, Vec<u8>)>>,
    image: Option<Panorama>,
    preview: Option<egui::TextureHandle>,
    saving: Option<crossbeam_channel::Receiver<Result<PathBuf, String>>>,
    dialog: Option<crossbeam_channel::Receiver<Option<PathBuf>>>,
    /// The window's status line, and whether it is an error.
    message: Option<(String, bool)>,
    /// The window was open last frame.
    was_open: bool,
    /// A capture asked (the window's opening, its button).
    wanted: bool,
    demo: Option<Demo>,
    demo_read: bool,
}

impl Capture360 {
    /// A capture is drawing its faces or waiting for its panorama: the
    /// world holds still meanwhile (LLFloater360Capture::freezeWorld).
    pub fn capturing(&self) -> bool {
        self.stage.is_some()
    }

    /// The face frame of this frame, as the photo frame describes one.
    pub fn photo_frame(&self, window: (u32, u32), render_scale_percent: u32) -> Option<super::snapshot::PhotoFrame> {
        let size = self.face_frame?;
        Some(super::snapshot::PhotoFrame {
            aspect: 1.0,
            tile: glam::Mat4::IDENTITY,
            height: size,
            scale: crate::snapshot::viewfinder_scale((size, size), window, render_scale_percent),
            look: None,
            context: aurora_render::PhotoContext::None,
            cube_face: true,
        })
    }
}

impl App {
    /// Every frame, before the photo frame is chosen: the camera of the
    /// last face put back, the window's opening and closing told to the
    /// simulator, the faces and the panorama taken in, the next face set up.
    pub(super) fn capture_360_frame(&mut self, renderer: &mut Renderer) {
        self.capture_360_demo();
        self.capture_360.face_frame = None;
        if std::mem::take(&mut self.capture_360.face_view) {
            self.camera.release_fixed_view();
            renderer.cut_motion_history();
        }
        let open = self.panels.capture_360 && self.in_world();
        if open != self.capture_360.was_open {
            self.capture_360.was_open = open;
            // LLFloater360Capture's constructor and destructor: everything
            // around while the window is open, the camera's view again after
            let mode = if open {
                InterestListMode::Capture360
            } else {
                InterestListMode::Default
            };
            log::info!("360 capture: interest list mode {} asked of the regions", mode.as_str());
            self.send(NetCommand::SetInterestListMode(mode));
            if open {
                // onOpen: a first capture at the chosen quality
                self.capture_360.wanted = true;
            } else {
                self.capture_360_abandon(renderer, "the window closed");
            }
        }
        self.capture_360_take_answers(renderer);
        // (a photo's preview or part still on its way would be taken for a
        // face, and a panorama owed to a capture given up for this one's)
        if self.capture_360.wanted
            && self.capture_360.stage.is_none()
            && open
            && !self.photo_work_pending(renderer)
            && !self.capture_360.stale_equirect.pending()
        {
            self.capture_360.wanted = false;
            self.capture_360_start();
        }
        self.capture_360_set_up_face(renderer);
        self.capture_360_poll_workers();
    }

    /// LLFloater360Capture::capture360Images: the world frozen, the
    /// avatars hidden when asked, the heading recorded, six faces to draw.
    fn capture_360_start(&mut self) {
        let settings = self.settings.snapshot.capture_360;
        let plan = Plan::new(settings.quality, settings.hide_avatars, self.camera.forward(), self.camera.position);
        let face_size = plan.face_size;
        self.world.photo_isolation.set_hide_avatars(settings.hide_avatars);
        self.capture_360.plan = Some(plan);
        self.capture_360.message = None;
        self.capture_360.stage = Some(Stage::Faces {
            faces: Default::default(),
            next: 0,
            asked: false,
            waited: 0,
            started: Instant::now(),
        });
        log::info!(
            "360 capture: heading {}°, faces of {face_size} px, avatars {}",
            plan.heading,
            if settings.hide_avatars { "hidden" } else { "drawn" }
        );
    }

    /// The face to draw this frame: the camera swapped for the capture
    /// camera (put back next frame), the square photo frame asked. A face
    /// whose answer has not come yet is asked again, as a photo's part is.
    fn capture_360_set_up_face(&mut self, renderer: &mut Renderer) {
        let (next, asked, waited) = match &self.capture_360.stage {
            Some(Stage::Faces { next, asked, waited, .. }) => (*next, *asked, *waited),
            _ => return,
        };
        let Some(plan) = self.capture_360.plan else {
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
        });
        self.capture_360.face_frame = Some(size);
    }

    /// After a frame was drawn, before the photo takes its answer: the face
    /// asked, if this frame carried it.
    pub(super) fn capture_360_drawn(&mut self, renderer: &mut Renderer) {
        let Some(Stage::Faces { asked, .. }) = &self.capture_360.stage else {
            return;
        };
        if !*asked || renderer.snapshot.is_none() {
            return;
        }
        let Some(answer) = renderer.snapshot.take() else {
            return;
        };
        let Some(size) = self.capture_360.plan.map(|p| p.face_size) else {
            return;
        };
        let Some((_, _, pixels)) = answer.filter(|(w, h, _)| (*w, *h) == (size, size)) else {
            self.capture_360_fail(renderer, "a face was not drawn");
            return;
        };
        if renderer.device_lost() {
            self.capture_360_fail(renderer, "the GPU was lost while a face was drawn");
            return;
        }
        let Some(Stage::Faces {
            faces,
            next,
            asked,
            waited,
            started,
        }) = &mut self.capture_360.stage
        else {
            return;
        };
        let layer = CAPTURE_CAMERAS[*next].layer;
        // the demo keeps each face as a PNG beside the panorama, as the
        // Legacy Viewer's 360CaptureDebugSaveImage does
        if let Some(folder) = self.capture_360.demo.as_ref().map(|d| d.folder.clone()) {
            let path = folder.join(format!("face_{layer}.png"));
            let _ = std::fs::create_dir_all(&folder);
            if let Err(e) = image::save_buffer(&path, &pixels, size, size, image::ExtendedColorType::Rgba8) {
                log::warn!("360 capture: face {layer} not kept: {e}");
            }
        }
        faces[layer] = pixels;
        *next += 1;
        *asked = false;
        *waited = 0;
        if *next < CAPTURE_CAMERAS.len() {
            return;
        }
        let faces_ms = started.elapsed().as_millis();
        log::info!("360 capture: 6 faces of {size} px drawn in {faces_ms} ms");
        let faces = std::mem::take(faces);
        // the faces drawn: the world moves again, the avatars come back
        self.world.photo_isolation.set_hide_avatars(false);
        renderer.equirect_request = Some(Box::new(EquirectJob {
            face_size: size,
            faces,
            width: rules::OUTPUT_WIDTH,
            height: rules::OUTPUT_WIDTH / 2,
        }));
        self.capture_360.stage = Some(Stage::Projecting {
            started: Instant::now(),
            faces_ms,
        });
    }

    /// The panorama the renderer made, if it came back.
    fn capture_360_take_answers(&mut self, renderer: &mut Renderer) {
        // the answer of a capture given up goes nowhere
        if self.capture_360.stale_equirect.pending() {
            let arrived = renderer.equirect.is_some();
            if self.capture_360.stale_equirect.swallow(arrived) {
                renderer.equirect = None;
            }
            if renderer.failed() || renderer.device_lost() {
                self.capture_360.stale_equirect = Stale::default();
            }
            return;
        }
        let (started, faces_ms) = match &self.capture_360.stage {
            Some(Stage::Projecting { started, faces_ms }) => (*started, *faces_ms),
            _ => return,
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
        let Some((width, height, rgba)) = answer else {
            self.capture_360_fail(renderer, "the panorama could not be made");
            return;
        };
        log::info!(
            "360 capture: {width} x {height} panorama in {} ms ({faces_ms} ms of faces)",
            started.elapsed().as_millis()
        );
        self.capture_360.stage = None;
        let Some(plan) = self.capture_360.plan else {
            return;
        };
        let region_name = self.world.region_name();
        let region_url = if region_name.is_empty() {
            "https://secondlife.com".to_owned()
        } else {
            crate::slurl::make(&region_name, self.world.agent.position)
        };
        // the preview at half the panorama's width: the window never shows
        // more, and the texture stays modest; reduced on a worker, the
        // frame does not wait for 8 megapixels
        let rgba = Arc::new(rgba);
        let (tx, rx) = crossbeam_channel::bounded(1);
        let source = rgba.clone();
        let spawned = std::thread::Builder::new().name("aurora-360-preview".into()).spawn(move || {
            let _ = tx.send(crate::snapshot::save::downscale(width, height, &source, width / 2));
        });
        match spawned {
            Ok(_) => self.capture_360.preview_work = Some(rx),
            Err(e) => log::warn!("360 capture: cannot start the preview thread: {e}"),
        }
        self.capture_360.image = Some(Panorama {
            width,
            height,
            rgba,
            heading: plan.heading,
            quality: plan.quality,
            face_size: plan.face_size,
            taken: crate::snapshot::local_time(),
            region_name,
            region_url,
        });
        self.capture_360.message = None;
        if self.capture_360.demo.as_ref().is_some_and(|d| !d.saved) {
            let folder = self.capture_360.demo.as_ref().map(|d| d.folder.clone()).unwrap_or_default();
            if let Some(d) = self.capture_360.demo.as_mut() {
                d.saved = true;
            }
            let name = self.capture_360_proposed_name();
            self.capture_360_save(folder.join(name));
        }
    }

    /// The capture given up: what it changed put back.
    fn capture_360_abandon(&mut self, renderer: &mut Renderer, why: &str) {
        match self.capture_360.stage.take() {
            Some(Stage::Faces { asked, .. }) => {
                log::info!("360 capture: given up, {why}");
                if asked {
                    // the face asked is drawn for nobody: its answer is
                    // swallowed, and one that already came is thrown away
                    renderer.drop_snapshot();
                    renderer.snapshot = None;
                }
            }
            Some(Stage::Projecting { .. }) => {
                log::info!("360 capture: given up, {why}");
                // not yet taken by the render thread: simply withdrawn
                // (an answer already there is thrown away, and owes nothing)
                let arrived = renderer.equirect.take().is_some();
                let withdrawn = renderer.equirect_request.take().is_some();
                self.capture_360.stale_equirect.abandon_projecting(arrived, withdrawn);
            }
            None => {}
        }
        self.camera.release_fixed_view();
        self.capture_360.face_view = false;
        self.capture_360.wanted = false;
        self.world.photo_isolation.set_hide_avatars(false);
    }

    fn capture_360_fail(&mut self, renderer: &mut Renderer, why: &str) {
        log::warn!("360 capture: {why}");
        self.capture_360_abandon(renderer, why);
        self.capture_360.message = Some((aurora_i18n::tr!("capture-360-failed").to_owned(), true));
    }

    /// LLFloater360Capture::generate_proposed_filename for the last panorama.
    fn capture_360_proposed_name(&self) -> String {
        let Some(image) = &self.capture_360.image else {
            return String::new();
        };
        rules::proposed_file_name(
            &image.region_name,
            image.width,
            image.height,
            image.face_size,
            &rules::compact_stamp(image.taken),
        )
    }

    /// The last panorama encoded and written at `path` on a worker.
    fn capture_360_save(&mut self, path: PathBuf) {
        let Some(image) = &self.capture_360.image else {
            return;
        };
        let software = format!("Aurora Viewer {}", aurora_version::version());
        let meta = Meta {
            software,
            width: image.width,
            height: image.height,
            heading: image.heading,
            source_size: image.quality.face_size(),
            scaled_source_size: image.face_size,
            date: rules::ctime_stamp(image.taken),
            region_name: image.region_name.clone(),
            region_url: image.region_url.clone(),
        };
        let (width, height, rgba) = (image.width, image.height, image.rgba.clone());
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
        if let Err(e) = spawned {
            log::warn!("360 capture: cannot start the save thread: {e}");
            return;
        }
        self.capture_360.saving = Some(rx);
        self.capture_360.message = Some((aurora_i18n::tr!("capture-360-saving").to_owned(), false));
    }

    /// The save dialog's choice and the save's end.
    fn capture_360_poll_workers(&mut self) {
        if let Some(rx) = &self.capture_360.preview_work
            && let Ok((pw, ph, preview)) = rx.try_recv()
        {
            self.capture_360.preview_work = None;
            let image = egui::ColorImage::from_rgba_unmultiplied([pw as usize, ph as usize], &preview);
            self.capture_360.preview = Some(
                self.egui_ctx
                    .load_texture("capture_360_preview", image, egui::TextureOptions::LINEAR),
            );
        }
        if let Some(rx) = &self.capture_360.dialog
            && let Ok(choice) = rx.try_recv()
        {
            self.capture_360.dialog = None;
            if let Some(path) = choice {
                self.capture_360_save(path);
            }
        }
        if let Some(rx) = &self.capture_360.saving
            && let Ok(result) = rx.try_recv()
        {
            self.capture_360.saving = None;
            self.capture_360.message = Some(match result {
                Ok(path) => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    (aurora_i18n::tr!("capture-360-saved", name = name.as_str()).to_owned(), false)
                }
                Err(e) => {
                    log::warn!("360 capture: not saved: {e}");
                    (aurora_i18n::tr!("capture-360-save-failed", error = e.as_str()).to_owned(), true)
                }
            });
        }
    }

    /// The window, while open.
    pub(super) fn capture_360_window(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let mut open = self.panels.capture_360;
        if !open {
            return;
        }
        let c = &mut self.capture_360;
        let progress = match &c.stage {
            Some(Stage::Faces { next, .. }) => Some(aurora_i18n::tr!("capture-360-capturing", face = *next + 1).to_owned()),
            Some(Stage::Projecting { .. }) => Some(aurora_i18n::tr!("capture-360-projecting").to_owned()),
            None => None,
        };
        let view = View {
            busy: c.stage.is_some() || c.saving.is_some() || c.dialog.is_some(),
            progress: progress.as_deref(),
            preview: c.preview.as_ref(),
            can_save: c.image.is_some() && c.stage.is_none() && c.saving.is_none() && c.dialog.is_none(),
            message: c.message.as_ref().map(|(m, err)| (m.as_str(), *err)),
            quality_shown: c.image.as_ref().map(|i| i.quality),
        };
        let actions = c.ui.show(ctx, p, &view, &mut self.settings.snapshot.capture_360, &mut open);
        self.panels.capture_360 = open;
        for action in actions {
            match action {
                Action::Capture => self.capture_360.wanted = true,
                Action::Changed => self.settings.save(),
                Action::SaveAs => self.capture_360_ask_path(),
            }
        }
    }

    /// LLFloater360Capture::onSaveLocalBtn: the save dialog with the
    /// proposed name, in the photos' folder.
    fn capture_360_ask_path(&mut self) {
        if self.capture_360.image.is_none() || self.capture_360.dialog.is_some() {
            return;
        }
        let name = self.capture_360_proposed_name();
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

    /// `AURORA_DEMO_SNAPSHOT_360=<folder>[:preview|medium|high|maximum]
    /// [:hide]`: the window opened at frame 600 (its capture follows), the
    /// panorama written to the folder under its proposed name.
    fn capture_360_demo(&mut self) {
        if !self.demo {
            return;
        }
        if !self.capture_360.demo_read {
            self.capture_360.demo_read = true;
            if let Ok(value) = std::env::var("AURORA_DEMO_SNAPSHOT_360") {
                self.capture_360.demo = parse_demo(&value);
            }
        }
        let due = self.capture_360.demo.as_ref().is_some_and(|d| !d.opened) && self.frame_count >= DEMO_OPEN_FRAME && self.in_world();
        if !due {
            return;
        }
        let Some(demo) = self.capture_360.demo.as_mut() else {
            return;
        };
        demo.opened = true;
        let (quality, hide) = (demo.quality, demo.hide);
        let s = &mut self.settings.snapshot.capture_360;
        if let Some(q) = quality {
            s.quality = q;
        }
        s.hide_avatars = hide;
        log::info!(
            "demo: 360 capture at frame {}, quality {:?}, avatars {}",
            self.frame_count,
            s.quality,
            if s.hide_avatars { "hidden" } else { "drawn" }
        );
        self.panels.capture_360 = true;
    }
}

/// The scenario's words: the folder first (it may hold a drive letter),
/// then a quality level and `hide`, in any order.
fn parse_demo(value: &str) -> Option<Demo> {
    let mut parts: Vec<&str> = value.split(':').collect();
    let mut quality = None;
    let mut hide = false;
    while parts.len() > 1 {
        let word = parts[parts.len() - 1].trim();
        if word == "hide" {
            hide = true;
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
        opened: false,
        saved: false,
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
        assert!(d.hide);
        let d = parse_demo("out").expect("a folder");
        assert_eq!(d.quality, None);
        assert!(!d.hide);
        assert!(parse_demo("").is_none());
    }

    /// The plan keeps the quality, the eye and the heading of the start.
    #[test]
    fn the_plan_is_the_start_of_the_capture() {
        let eye = glam::Vec3::new(10.0, 20.0, 30.0);
        let plan = Plan::new(Quality::High, true, glam::Vec3::Y, eye);
        assert_eq!(plan.quality, Quality::High);
        assert_eq!(plan.face_size, Quality::High.face_size());
        assert_eq!(plan.eye, eye);
        assert!(plan.hide_avatars);
        let other = Plan::new(Quality::Preview, false, glam::Vec3::Y, eye);
        assert_ne!(plan.face_size, other.face_size);
    }

    /// The answers owed to captures given up are thrown away, in order,
    /// and only they.
    #[test]
    fn a_given_up_answer_never_reaches_the_next_capture() {
        let mut stale = Stale::default();
        assert!(!stale.swallow(true), "nothing owed: the answer is the capture's");
        stale.abandon(false);
        assert!(!stale.pending(), "a request the render thread never took is owed nothing");
        stale.abandon(true);
        assert!(stale.pending());
        assert!(!stale.swallow(false), "no answer yet");
        assert!(stale.swallow(true), "the first answer is the given up one's");
        assert!(!stale.pending());
        assert!(!stale.swallow(true), "the next is the new capture's");
    }

    /// Given up while projecting: an answer that already arrived is thrown
    /// away and owes nothing, or the next capture would swallow its own.
    #[test]
    fn abandoning_a_projection_owes_nothing_for_an_answer_that_arrived() {
        let mut stale = Stale::default();
        stale.abandon_projecting(true, false);
        assert!(!stale.pending(), "the answer is here: nothing is owed");
        stale.abandon_projecting(false, true);
        assert!(!stale.pending(), "the request was withdrawn before the render thread took it");
        stale.abandon_projecting(false, false);
        assert!(stale.pending(), "the render thread has the request: its answer is owed");
        assert!(!Stale::default().swallow(true), "the next capture keeps its own answer");
    }
}
