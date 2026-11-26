//! Where the listener thread stages the images Blender sends, and the images
//! it staged for the last snapshot. Both belong to one GPU device: they are
//! given up when it is lost (`Live::forget_gpu`, app/gpu_recovery.rs), and the
//! thread stages in the new device's pool from its next snapshot on
//! (`Live::gpu_reset`).
use super::protocol::{CachedImage, Prepared, PreparedImage};
use aurora_render::{StagedTexture, StagingPool};
use std::{collections::HashMap, sync::Arc};
use uuid::Uuid;

#[derive(Default)]
pub(crate) struct Staging {
    /// None while the device is lost (`begin` then gives nothing), and in the
    /// tests, which have no GPU: the images are left unstaged there.
    pool: Option<Arc<StagingPool>>,
    /// The device is gone and the new one not made yet.
    lost: bool,
    /// Counts the devices: a snapshot begun for one is not taken for another.
    device: u64,
    /// The images of the connection's last snapshot, not decoded nor staged
    /// again when the next one has them too.
    images: HashMap<Uuid, CachedImage>,
}

/// What a snapshot is prepared with, while the device may be replaced.
pub(crate) struct Turn {
    device: u64,
    pub pool: Option<Arc<StagingPool>>,
    pub images: HashMap<Uuid, CachedImage>,
}

impl Staging {
    pub fn new(pool: Option<Arc<StagingPool>>) -> Self {
        Self {
            pool,
            ..Default::default()
        }
    }

    /// None while there is no device to stage for.
    pub fn begin(&self) -> Option<Turn> {
        (!self.lost).then(|| Turn {
            device: self.device,
            pool: self.pool.clone(),
            images: self.images.clone(),
        })
    }

    /// Keeps the images of a snapshot prepared with `turn`; false when the
    /// device it was staged for was lost meanwhile.
    pub fn finish(&mut self, turn: &Turn, snapshot: &Prepared) -> bool {
        if self.lost || turn.device != self.device {
            return false;
        }
        self.images = snapshot.images.iter().map(|image| (image.id, CachedImage::of(image))).collect();
        true
    }

    /// The connection ended: nothing of its snapshots is kept.
    pub fn clear_images(&mut self) {
        self.images.clear();
    }

    pub fn forget_gpu(&mut self) {
        self.pool = None;
        self.lost = true;
        self.device += 1;
        self.images.clear();
    }

    pub fn gpu_reset(&mut self, pool: Option<Arc<StagingPool>>) {
        self.pool = pool;
        self.lost = false;
        self.device += 1;
        self.images.clear();
    }
}

/// Images staged again from their pixels on a job, for one pool
/// (`Live::restage_missing`).
pub(crate) struct Restaging {
    pool: std::sync::Weak<StagingPool>,
    rx: std::sync::Mutex<std::sync::mpsc::Receiver<HashMap<Uuid, Arc<StagedTexture>>>>,
}

impl Restaging {
    pub fn start(mut images: Vec<PreparedImage>, pool: Arc<StagingPool>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let weak = Arc::downgrade(&pool);
        rayon::spawn(move || {
            let mut done = HashMap::new();
            restage(&mut images, Some(&pool), &mut done);
            let _ = tx.send(done);
        });
        Self {
            pool: weak,
            rx: std::sync::Mutex::new(rx),
        }
    }

    /// The images staged, once the job is done; none for another pool than
    /// `pool` (the device was replaced meanwhile) or a job that failed.
    pub fn done(&self, pool: &Arc<StagingPool>) -> Option<HashMap<Uuid, Arc<StagedTexture>>> {
        use std::sync::mpsc::TryRecvError;
        let received = self.rx.lock().unwrap_or_else(|e| e.into_inner()).try_recv();
        match received {
            Ok(done) => Some(if std::ptr::eq(self.pool.as_ptr(), Arc::as_ptr(pool)) {
                done
            } else {
                HashMap::new()
            }),
            Err(TryRecvError::Disconnected) => Some(HashMap::new()),
            Err(TryRecvError::Empty) => None,
        }
    }
}

/// Stages `images` in `pool`, from the pixels kept with them. `done` holds
/// those already staged, which several snapshots can share.
pub(super) fn restage(images: &mut [PreparedImage], pool: Option<&StagingPool>, done: &mut HashMap<Uuid, Arc<StagedTexture>>) {
    use rayon::prelude::*;
    let Some(pool) = pool else {
        return;
    };
    let levels: Vec<_> = images
        .par_iter()
        .map(|image| {
            let pixels = &image.pixels;
            (!done.contains_key(&image.id)).then(|| aurora_render::build_mips(pixels.width, pixels.height, pixels.data.clone(), 12))
        })
        .collect();
    for (image, levels) in images.iter_mut().zip(levels) {
        image.staged = done.get(&image.id).cloned().or_else(|| {
            let levels = levels?;
            let refs: Vec<_> = levels.iter().map(|(w, h, bytes)| (*w, *h, bytes.as_slice())).collect();
            let staged = Arc::new(StagedTexture::new(pool, &refs)?);
            done.insert(image.id, staged.clone());
            Some(staged)
        });
        if image.staged.is_none() {
            log::warn!(
                "Blender live: an image could not be staged again from its pixels; the preview waits for it or for Blender's next snapshot"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(images: usize) -> Vec<u8> {
        use base64::Engine;
        let mut value = super::super::tests::triangle();
        let images: Vec<_> = (0..images)
            .map(|i| {
                let rgba = base64::engine::general_purpose::STANDARD.encode([255u8, i as u8, 0, 255]);
                serde_json::json!({"width": 1, "height": 1, "rgba": rgba})
            })
            .collect();
        value["images"] = serde_json::json!(images);
        serde_json::to_vec(&value).expect("JSON")
    }

    fn snapshot() -> Prepared {
        super::super::protocol::prepare(&bytes(1), &["mPelvis".to_owned()].into()).expect("snapshot")
    }

    /// The listener's cache gives the next snapshot the pixels it decoded,
    /// and holds no staged copy: one kept alive there pinned a staging chunk
    /// for the whole connection, and held every upload back.
    #[test]
    fn blender_live_cache_keeps_pixels_not_the_staged_copy() {
        let supported = ["mPelvis".to_owned()].into();
        let first = snapshot();
        let mut staging = Staging::default();
        let turn = staging.begin().expect("a device");
        assert!(staging.finish(&turn, &first));
        let turn = staging.begin().expect("a device");
        assert!(
            turn.images[&first.images[0].id].reuse().staged.is_none(),
            "the cache holds no staged copy"
        );
        let second = super::super::protocol::prepare_staged(&bytes(1), &supported, None, &turn.images).expect("next snapshot");
        assert!(Arc::ptr_eq(&second.images[0].pixels, &first.images[0].pixels), "not decoded again");
        assert!(second.images[0].staged.is_none(), "uploaded already: not staged again");
    }

    /// The safety net stages again only the images neither on the GPU nor
    /// staged; those on the GPU give their staged copies up.
    #[test]
    fn blender_live_restages_only_images_neither_on_the_gpu_nor_staged() {
        let snapshot = super::super::protocol::prepare(&bytes(2), &["mPelvis".to_owned()].into()).expect("snapshot");
        let (shown, missing) = (snapshot.images[0].id, snapshot.images[1].id);
        let mut live = super::super::Live::default();
        live.receive(snapshot);
        live.release_staged(|id| *id == shown);
        let unstaged = live.unstaged(|id| *id == shown);
        assert_eq!(unstaged.iter().map(|image| image.id).collect::<Vec<_>>(), [missing]);
        assert!(live.unstaged(|_| true).is_empty(), "all on the GPU: nothing to stage");
    }

    #[test]
    fn blender_live_staging_belongs_to_one_gpu_device() {
        let snapshot = snapshot();
        let mut staging = Staging::default();
        let first = staging.begin().expect("a device");
        assert!(staging.finish(&first, &snapshot));
        assert_eq!(staging.begin().expect("a device").images.len(), 1, "the next snapshot reuses them");

        staging.forget_gpu();
        assert!(staging.begin().is_none(), "no device to stage for");
        assert!(staging.images.is_empty(), "nothing keeps the lost device");
        assert!(!staging.finish(&first, &snapshot));

        staging.gpu_reset(None);
        assert!(!staging.finish(&first, &snapshot), "staged for the lost device");
        assert!(staging.images.is_empty());
        let second = staging.begin().expect("the new device");
        assert!(second.images.is_empty(), "the old device's images are not reused");
        assert!(staging.finish(&second, &snapshot));
    }
}
