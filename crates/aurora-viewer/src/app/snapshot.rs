//! Photos in the app (LLFloaterSnapshot, LLSnapshotLivePreview,
//! LLViewerWindow::rawSnapshot / saveImageLocal, indra/newview, originally
//! LGPL 2.1): a photo is a job of one or more frames drawn offscreen at its
//! size (`Renderer::snapshot_request`), stitched when it is drawn in tiles,
//! then a preview, a held freeze frame, or a file written on a worker.

use super::App;
use crate::snapshot::{self, Tile, gallery, save};
use crate::ui::snapshot::Action;
use crate::ui_sound::UiSound;
use aurora_render::{Renderer, SnapshotSpec};
use glam::Mat4;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Long side of the preview image.
const PREVIEW_SIDE: u32 = 720;
/// The preview follows the camera once it has been still this long.
const SETTLE: Duration = Duration::from_millis(250);
/// Bytes of video memory a pixel of a photo takes while it is drawn: its
/// render targets and readback. Measured on 2026-10-14 (demo, MSAA 4x):
/// +289 MB for 3840 x 2160, +1182 MB for 7680 x 4320, about 36 bytes; with
/// room for MSAA 8x. The photo is tiled when the free memory cannot hold it.
const TARGET_BYTES_PER_PIXEL: u64 = 48;
/// Pixels drawn in one pass when the driver does not report its memory.
const DEFAULT_MAX_PIXELS: u64 = 34_000_000;

#[derive(Debug, Clone, PartialEq)]
enum Purpose {
    Preview,
    /// To a file; Some: the one chosen in "Save as".
    Save(Option<PathBuf>),
    /// Freeze frame: kept, shown, saved by "Take the photo".
    Hold,
}

struct Job {
    purpose: Purpose,
    size: (u32, u32),
    tiles: Vec<Tile>,
    next: usize,
    /// The current tile's frame is asked.
    asked: bool,
    pixels: Vec<u8>,
    interface: bool,
    huds: bool,
    started: Instant,
}

/// A held photo (freeze frame) and what shows it.
struct Held {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
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
    preview_wanted: bool,
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
    /// The window was closed when the photo was asked (Ctrl+`).
    quiet: bool,
    demo: Option<DemoPhoto>,
}

struct DemoPhoto {
    frame: u64,
    /// None: only the window.
    save: Option<((u32, u32), snapshot::FileFormat, Option<PathBuf>)>,
    full_frame: bool,
    aspect: Option<snapshot::Aspect>,
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
            quiet: false,
            demo: None,
        }
    }
}

impl App {
    /// Before a frame is laid out: the photo frame it carries, if any.
    pub(super) fn photo_frame(&mut self, renderer: &mut Renderer) -> Option<PhotoFrame> {
        let s = &mut self.snapshots;
        s.capturing = None;
        let job = s.job.as_mut()?;
        if job.asked {
            // the frame asked never came back (the render thread is gone)
            log::warn!("photo: the frame of part {} was not drawn", job.next + 1);
            s.job = None;
            s.message = Some((aurora_i18n::tr!("photo-draw-failed").to_owned(), true));
            return None;
        }
        let tile = job.tiles[job.next];
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
        Some(PhotoFrame {
            aspect: job.size.0 as f32 / job.size.1.max(1) as f32,
            tile: if single {
                Mat4::IDENTITY
            } else {
                snapshot::tile_matrix(job.size, tile)
            },
            height: job.size.1,
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
        let tile = job.tiles[job.next];
        let Some((w, h, pixels)) = answer.filter(|(w, h, _)| (*w, *h) == (tile.width, tile.height)) else {
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
            snapshot::stitch(
                &mut job.pixels,
                job.size.0,
                Tile {
                    width: w,
                    height: h,
                    ..tile
                },
                &pixels,
            );
        }
        job.next += 1;
        if job.next < job.tiles.len() {
            return;
        }
        let Some(job) = self.snapshots.job.take() else {
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
                    texture: None,
                });
                self.photo_shutter();
            }
            Purpose::Save(path) => self.photo_save(job.size, job.pixels, path),
        }
    }

    /// Start a photo of the settings' size.
    fn photo_job(&mut self, purpose: Purpose, renderer: &Renderer) {
        let window = renderer.size();
        let s = &self.settings.snapshot;
        let size = if purpose == Purpose::Preview {
            let (w, h) = s.size(window);
            let scale = (PREVIEW_SIDE as f32 / w.max(h) as f32).min(1.0);
            (
                ((w as f32 * scale).round() as u32).max(16),
                ((h as f32 * scale).round() as u32).max(16),
            )
        } else {
            s.size(window)
        };
        let max_pixels = renderer.video_memory.read(&renderer.device).map_or(DEFAULT_MAX_PIXELS, |m| {
            // half of what is free, besides what the window's frame holds
            m.budget.saturating_sub(m.usage) / 2 / TARGET_BYTES_PER_PIXEL
        });
        let tiles = snapshot::plan(size, self.photo_max_side(renderer), max_pixels);
        let interface = s.interface && s.interface_possible(window);
        self.snapshots.job = Some(Job {
            purpose,
            size,
            tiles,
            next: 0,
            asked: false,
            pixels: Vec::new(),
            interface,
            huds: s.huds,
            started: Instant::now(),
        });
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

    fn photo_shutter(&mut self) {
        if self.settings.snapshot.sound {
            self.play_ui_sound(UiSound::Snapshot);
        }
    }

    fn photo_save(&mut self, size: (u32, u32), rgba: Vec<u8>, chosen: Option<PathBuf>) {
        let s = &self.settings.snapshot;
        let (format, path) = match chosen {
            Some(path) => (snapshot::FileFormat::of_path(&path).unwrap_or(s.format), path),
            None => {
                let stamp = s.timestamp_names.then(snapshot::local_stamp);
                (s.format, snapshot::free_path(&s.folder(), &s.base_name, stamp.as_deref(), s.format))
            }
        };
        self.snapshots.saving = Some(save::Saving::start(save::Job {
            width: size.0,
            height: size.1,
            rgba,
            format,
            jpeg_quality: s.jpeg_quality,
            path,
        }));
        self.photo_shutter();
    }

    /// "Take the photo": the held one, or a new one.
    fn photo_take(&mut self, renderer: &Renderer, chosen: Option<PathBuf>) {
        if self.snapshots.saving.is_some() || self.snapshots.job.as_ref().is_some_and(|j| j.purpose != Purpose::Preview) {
            return;
        }
        if let Some(held) = &self.snapshots.held {
            let (size, rgba) = ((held.width, held.height), held.rgba.clone());
            self.photo_save(size, rgba, chosen);
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
                    s.gallery.insert(0, (saved.path.clone(), Some(tex)));
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
                    s.message = Some((aurora_i18n::tr!("photo-failed", error = e.as_str()).to_owned(), true));
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
        let s = &mut self.snapshots;
        // the preview: asked, its photo changed, or the camera settled
        if !self.panels.snapshot || s.held.is_some() {
            return;
        }
        let window = renderer.size();
        let key = (
            self.settings.snapshot.size(window),
            self.settings.snapshot.interface,
            self.settings.snapshot.huds,
        );
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
                self.photo_take(renderer, None);
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
        }
    }

    /// The photo window, the full-frame view or the held photo, in the
    /// interface of the world.
    pub(super) fn photo_ui(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, renderer: &Renderer) {
        // a photo with the interface does not show the photo window
        if self.snapshots.capturing.is_some_and(|c| c.interface) {
            return;
        }
        let window = renderer.size();
        let photo = self.settings.snapshot.size(window);
        let mut actions = Vec::new();
        if self.snapshots.ui.full_frame && self.panels.snapshot {
            actions = self.snapshots.ui.full_frame(
                ctx,
                p,
                &mut self.settings.snapshot,
                &mut self.settings.camera.fov,
                crate::camera::DEFAULT_FOV,
                photo,
            );
        } else if self.panels.snapshot {
            let mut state = std::mem::take(&mut self.snapshots.ui);
            let max_pixels = renderer.video_memory.read(&renderer.device).map_or(DEFAULT_MAX_PIXELS, |m| {
                m.budget.saturating_sub(m.usage) / 2 / TARGET_BYTES_PER_PIXEL
            });
            let tiles = snapshot::plan(photo, self.photo_max_side(renderer), max_pixels).len();
            let s = &self.snapshots;
            let busy = if let Some(saving) = &s.saving {
                Some((0.5 + 0.5 * saving.fraction(), aurora_i18n::tr!("photo-saving").to_owned()))
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
            };
            let mut open = self.panels.snapshot;
            actions = state.show(
                ctx,
                p,
                &mut open,
                &mut self.settings.snapshot,
                &mut self.settings.camera.fov,
                crate::camera::DEFAULT_FOV,
                &view,
            );
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

    /// The full-frame view hides the rest of the interface.
    pub(super) fn photo_full_frame(&self) -> bool {
        self.panels.snapshot && self.snapshots.ui.full_frame
    }

    /// Escape leaves the full-frame view instead of resetting the camera.
    pub(super) fn photo_escape(&mut self) -> bool {
        if self.photo_full_frame() {
            self.snapshots.ui.full_frame = false;
            return true;
        }
        false
    }

    /// AURORA_DEMO_SNAPSHOT=open | full[-<W>x<H> proportions, e.g. full-4x5] |
    /// save:<W>x<H>[:png|jpg|bmp][:<folder>]
    /// (AURORA_DEMO_SNAPSHOT_FRAME, 700 by default): the window at frame
    /// 600, the photo saved at that frame.
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
            let save = (kind == "save").then(|| {
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
            self.snapshots.demo = Some(DemoPhoto {
                frame,
                save,
                full_frame: kind.starts_with("full"),
                aspect,
            });
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
            self.snapshots.quiet = false;
            self.photo_take(renderer, None);
        }
    }
}
