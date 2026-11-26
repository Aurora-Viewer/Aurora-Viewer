//! Where the listener thread stages the images Blender sends, and the images
//! it staged for the last snapshot. Both belong to one GPU device: they are
//! given up when it is lost (`Live::forget_gpu`, app/gpu_recovery.rs), and the
//! thread stages in the new device's pool from its next snapshot on
//! (`Live::gpu_reset`).
use super::protocol::{Prepared, PreparedImage};
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
    /// The images of the connection's last snapshot, not staged again when
    /// the next one has them too.
    images: HashMap<Uuid, PreparedImage>,
}

/// What a snapshot is prepared with, while the device may be replaced.
pub(crate) struct Turn {
    device: u64,
    pub pool: Option<Arc<StagingPool>>,
    pub images: HashMap<Uuid, PreparedImage>,
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
        self.images = snapshot.images.iter().map(|image| (image.id, image.clone())).collect();
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
                "Blender live: an image could not be staged again after the GPU recovery; the preview waits for Blender's next snapshot"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Prepared {
        use base64::Engine;
        let mut value = super::super::tests::triangle();
        let rgba = base64::engine::general_purpose::STANDARD.encode([255u8; 4]);
        value["images"] = serde_json::json!([{"width": 1, "height": 1, "rgba": rgba}]);
        let bytes = serde_json::to_vec(&value).expect("JSON");
        super::super::protocol::prepare(&bytes, &["mPelvis".to_owned()].into()).expect("snapshot")
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
