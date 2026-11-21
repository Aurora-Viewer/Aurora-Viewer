//! Small interface images for assets already resident on the GPU. The normal
//! upload path supplies new decodes; these bounded workers cover cache hits
//! without forcing a world texture upload or reading pixels back from the GPU.

use super::{
    jobs::Jobs,
    textures::{PreviewRevision, PreviewSource},
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};
use uuid::Uuid;

pub(super) type Pixels = (u32, u32, Vec<u8>);
type Request = (PreviewRevision, u32);
type Reply = (Uuid, Request, Option<Pixels>);

pub(super) struct Images {
    tx: crossbeam_channel::Sender<Reply>,
    rx: crossbeam_channel::Receiver<Reply>,
    pending: HashMap<Uuid, Request>,
    attempted: HashMap<Uuid, (Request, Instant)>,
}

impl Default for Images {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::bounded(8);
        Self {
            tx,
            rx,
            pending: Default::default(),
            attempted: Default::default(),
        }
    }
}

impl Images {
    pub fn request(&mut self, id: Uuid, source: PreviewSource, side: u32, jobs: &Jobs) {
        let revision = (if side > 256 { source.revision() } else { source.image_revision() }, side);
        let now = Instant::now();
        if self.pending.len() >= 8
            || self.pending.contains_key(&id)
            || self
                .attempted
                .get(&id)
                .is_some_and(|(old, when)| *old == revision && now.duration_since(*when) < Duration::from_secs(1))
        {
            return;
        }
        // Failures are retried with backoff: a progressive cache file can
        // arrive without changing the streamer's in-memory revision.
        if self.attempted.len() >= 256 {
            self.attempted
                .retain(|_, (_, when)| now.duration_since(*when) < Duration::from_secs(1));
        }
        self.pending.insert(id, revision);
        self.attempted.insert(id, (revision, now));
        let tx = self.tx.clone();
        jobs.execute(move || {
            let pixels = source.image_at_size(id, side).and_then(|image| {
                let rgba = image::RgbaImage::from_raw(image.size[0], image.size[1], image.rgba.to_vec())?;
                let image = if rgba.width().max(rgba.height()) > side {
                    image::DynamicImage::ImageRgba8(rgba).thumbnail(side, side).into_rgba8()
                } else {
                    rgba
                };
                Some((image.width(), image.height(), image.into_raw()))
            });
            let _ = tx.try_send((id, revision, pixels));
        });
    }

    pub fn poll(&mut self, ready: &mut HashMap<Uuid, Pixels>, wanted: &HashSet<Uuid>) {
        while let Ok((id, revision, pixels)) = self.rx.try_recv() {
            if self.pending.remove(&id) == Some(revision)
                && wanted.contains(&id)
                && let Some(pixels) = pixels
            {
                // Prefer the normal upload path's potentially newer image.
                ready.entry(id).or_insert(pixels);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn inventory_hover_cache_late_replies_cannot_resurrect_released_images_or_grow_the_worker_queue() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (jobs, _results) = Jobs::new();
        let bytes = aurora_assets::j2k::encode_texture_j2k(64, 64, 3, &vec![128; 64 * 64 * 3], true).unwrap();
        let source = || PreviewSource {
            local: None,
            bytes: Arc::new(bytes.clone()),
            jpeg: std::env::temp_dir().join("unused-hover-cache.j2c"),
            blocks: std::env::temp_dir().join("unused-hover-cache.bc7"),
            decoded: Some(0),
            size: Some((64, 64)),
            procedural: false,
        };
        let mut images = Images::default();
        let mut ready = HashMap::new();
        for n in 1..=1000 {
            images.request(Uuid::from_u128(n), source(), 256, &jobs);
            assert!(images.pending.len() <= 8);
        }
        assert_eq!(images.pending.len(), 8);
        images.poll(&mut ready, &HashSet::new());
        assert!(
            ready.is_empty() && images.pending.is_empty(),
            "released image results are discarded"
        );
        images.request(Uuid::from_u128(1001), source(), 256, &jobs);
        images.poll(&mut ready, &HashSet::from([Uuid::from_u128(1001)]));
        assert_eq!(ready.len(), 1, "new active image can load after obsolete jobs drain");
    }

    #[test]
    fn a_cached_outfit_photo_reloads_in_hd_after_its_thumbnail_and_after_eviction() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (jobs, _results) = Jobs::new();
        let id = Uuid::new_v4();
        let dir = std::env::temp_dir().join(format!("aurora-outfit-hd-{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        let jpeg = dir.join("portrait.j2c");
        let bytes = aurora_assets::j2k::encode_texture_j2k(1024, 512, 3, &vec![128; 1024 * 512 * 3], true).unwrap();
        crate::cache::write(&jpeg, &bytes).unwrap();
        let source = || PreviewSource {
            local: None,
            bytes: Arc::default(),
            jpeg: jpeg.clone(),
            blocks: dir.join("portrait.bc7"),
            decoded: Some(0),
            size: Some((1024, 512)),
            procedural: false,
        };
        let wanted = HashSet::from([id]);
        let mut jobs_ui = Images::default();
        let mut ready = HashMap::new();
        for (side, width, height) in [(256, 256, 128), (1024, 1024, 512)] {
            jobs_ui.request(id, source(), side, &jobs);
            jobs_ui.poll(&mut ready, &wanted);
            let pixels = ready.remove(&id).expect("disk image at requested resolution");
            assert_eq!((pixels.0, pixels.1), (width, height));
        }
        let mut reopened = Images::default();
        reopened.request(id, source(), 1024, &jobs);
        reopened.poll(&mut ready, &wanted);
        assert_eq!(ready[&id].0, 1024);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn resident_and_cache_only_default_textures_get_ui_pixels_without_an_upload() {
        let _inline = super::super::jobs::execute_inline_on_this_thread();
        let (jobs, _results) = Jobs::new();
        let cache = std::env::temp_dir().join(format!("aurora-ui-textures-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&cache).unwrap();
        let jpeg = cache.join("default.j2c");
        let bytes = aurora_assets::j2k::encode_texture_j2k(64, 64, 3, &vec![128; 64 * 64 * 3], true).unwrap();
        std::fs::write(&jpeg, &bytes).unwrap();
        for resident in [false, true] {
            let source = PreviewSource {
                local: None,
                bytes: if resident { Arc::new(bytes.clone()) } else { Arc::default() },
                jpeg: jpeg.clone(),
                blocks: cache.join("missing.bc7"),
                decoded: resident.then_some(0),
                size: resident.then_some((64, 64)),
                procedural: false,
            };
            let mut images = Images::default();
            let mut ready = HashMap::new();
            images.request(aurora_prim::te::DEFAULT_TEXTURE, source, 256, &jobs);
            images.poll(&mut ready, &HashSet::from([aurora_prim::te::DEFAULT_TEXTURE]));
            let (width, height, rgba) = ready.get(&aurora_prim::te::DEFAULT_TEXTURE).unwrap();
            assert_eq!((*width, *height, rgba.len()), (64, 64, 64 * 64 * 4));
            assert!(
                rgba.as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| pixel[3] == 255 && pixel[0].abs_diff(128) <= 1)
            );
        }
        std::fs::remove_file(jpeg).unwrap();
        std::fs::remove_dir(cache).unwrap();
    }
}
