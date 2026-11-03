//! The photo reel of the session (`snapshot::reel`, `ui::photo_reel`):
//! every photo taken joins it, whatever its destination; its thumbnail and
//! the reel's own copy of its pixels are made on a worker. The photo being
//! sent is followed from its start to its outcome: one at a time, like the
//! photo window's (`photo_busy`).

use super::App;
use crate::snapshot::reel::{self, Place, Reel};
use crate::snapshot::save;
use std::collections::HashMap;
use std::path::PathBuf;

/// A worker's result.
enum Done {
    Thumb(u64, (u32, u32, Vec<u8>)),
    Copied(u64, Result<PathBuf, String>),
}

pub(super) struct ReelState {
    pub reel: Reel,
    pub thumbs: HashMap<u64, egui::TextureHandle>,
    /// The quick view's pictures, at the screen's size.
    pub full: HashMap<u64, egui::TextureHandle>,
    /// The session's copies, opened with the first one.
    folder: Option<reel::Folder>,
    tx: crossbeam_channel::Sender<Done>,
    rx: crossbeam_channel::Receiver<Done>,
    /// The photo a destination is sending.
    sending: Option<u64>,
    /// The held photo is this one of the reel, to be sent again.
    pub held: Option<u64>,
    /// The photo the last photo's card shows.
    pub card: Option<u64>,
}

impl Default for ReelState {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        ReelState {
            reel: Reel::default(),
            thumbs: HashMap::new(),
            full: HashMap::new(),
            folder: None,
            tx,
            rx,
            sending: None,
            held: None,
            card: None,
        }
    }
}

impl ReelState {
    /// Forget photos (pushed out, deleted, the session over): their
    /// pictures, and the reel's copies on a worker.
    fn forget(&mut self, photos: Vec<reel::Photo>) {
        let mut copies = Vec::new();
        for photo in photos {
            self.thumbs.remove(&photo.id);
            self.full.remove(&photo.id);
            if self.card == Some(photo.id) {
                self.card = None;
            }
            if self.held == Some(photo.id) {
                self.held = None;
            }
            if let Some(file) = photo.file.filter(|_| photo.own_copy) {
                copies.push(file);
            }
        }
        delete_files(copies);
    }
}

/// Delete the reel's copies, off the frame.
fn delete_files(files: Vec<PathBuf>) {
    if files.is_empty() {
        return;
    }
    std::thread::spawn(move || {
        for file in files {
            if let Err(e) = std::fs::remove_file(&file)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                log::warn!("photo reel: {} not deleted: {e}", file.display());
            }
        }
    });
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl ReelState {
    /// A destination starts with a photo: the held reel photo sent again,
    /// or a new one joining the reel. `rgba` is given when the photo is not
    /// written to disk: its thumbnail and the reel's copy are made from it.
    pub(super) fn start(&mut self, size: (u32, u32), rgba: Option<&[u8]>, cutout: bool) {
        let r = self;
        if let Some(id) = r.held.filter(|id| r.reel.get(*id).is_some()) {
            r.sending = Some(id);
            return;
        }
        let (id, dropped) = r.reel.add(unix_now(), size, cutout, None, rgba.is_some(), Vec::new());
        r.forget(dropped);
        r.sending = Some(id);
        let Some(rgba) = rgba else {
            return;
        };
        if r.folder.is_none() {
            match reel::Folder::open(&crate::settings::cache_dir().join("photo-reel")) {
                Ok(folder) => r.folder = Some(folder),
                Err(e) => log::warn!("photo reel: no folder for the copies: {e}"),
            }
        }
        let copy = r.folder.as_ref().map(|f| f.path(id));
        let rgba = rgba.to_vec();
        let tx = r.tx.clone();
        let spawned = std::thread::Builder::new().name("aurora-photo-reel".into()).spawn(move || {
            let _ = tx.send(Done::Thumb(id, save::thumbnail(size.0, size.1, &rgba)));
            if let Some(path) = copy {
                let t0 = std::time::Instant::now();
                let result = reel::write_copy(&path, size, &rgba).map(|()| path);
                if result.is_ok() {
                    log::info!(
                        "photo reel: copy of {} x {} written in {} ms",
                        size.0,
                        size.1,
                        t0.elapsed().as_millis()
                    );
                }
                let _ = tx.send(Done::Copied(id, result));
            }
        });
        if let Err(e) = spawned {
            log::warn!("photo reel: cannot start its worker: {e}");
        }
    }

    /// The photo being sent arrived (`place`), or failed (None). A photo
    /// never written anywhere leaves the reel (a save to disk that failed:
    /// nothing to send again); one with its own copy stays, to be sent
    /// again. `thumb`: the save's thumbnail.
    pub(super) fn done(&mut self, place: Option<Place>, thumb: Option<egui::TextureHandle>) {
        let r = self;
        let Some(id) = r.sending.take() else {
            return;
        };
        if let Some(tex) = thumb {
            r.thumbs.entry(id).or_insert(tex);
        }
        match place {
            Some(place) => {
                if let Some(freed) = r.reel.sent(id, place) {
                    delete_files(vec![freed]);
                }
                r.card = Some(id);
            }
            None => {
                if r.reel.get(id).is_some_and(|p| !p.own_copy && p.places.is_empty())
                    && let Some(photo) = r.reel.remove(id)
                {
                    r.forget(vec![photo]);
                }
            }
        }
    }
}

impl App {
    /// Every frame: the workers' thumbnails and copies.
    pub(super) fn photo_reel_update(&mut self) {
        let r = &mut self.snapshots.reel;
        while let Ok(done) = r.rx.try_recv() {
            match done {
                Done::Thumb(id, (w, h, px)) => {
                    if r.reel.get(id).is_some() {
                        let image = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &px);
                        let tex = self
                            .egui_ctx
                            .load_texture(format!("photo_reel_{id}"), image, egui::TextureOptions::LINEAR);
                        r.thumbs.insert(id, tex);
                    }
                }
                Done::Copied(id, Ok(path)) => {
                    if r.reel.get(id).is_some_and(|p| p.own_copy) {
                        r.reel.copied(id, path);
                    } else {
                        // gone meanwhile, or saved to disk since
                        delete_files(vec![path]);
                    }
                }
                Done::Copied(id, Err(e)) => log::warn!("photo reel: copy of photo {id} not written: {e}"),
            }
        }
    }

    /// The last photo's card: its thumbnail, from the reel.
    pub(super) fn photo_reel_card(&self) -> Option<egui::TextureHandle> {
        let r = &self.snapshots.reel;
        r.thumbs.get(&r.card?).cloned()
    }

    /// The session is over: the reel empties, its folder goes.
    pub(super) fn photo_reel_end(&mut self) {
        let r = &mut self.snapshots.reel;
        let photos = r.reel.clear();
        r.forget(photos);
        r.sending = None;
        // (dropped: its folder is deleted)
        r.folder = None;
    }
}
