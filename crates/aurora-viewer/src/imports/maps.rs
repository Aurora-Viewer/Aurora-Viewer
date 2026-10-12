//! Material map selection and cached inventory texture previews.

use super::*;
pub fn ids(value: &PbrMaterial) -> [Option<Uuid>; 4] {
    [
        value.base_color_texture,
        value.normal_texture,
        value.metallic_roughness_texture,
        value.emissive_texture,
    ]
}
pub fn set(value: &mut PbrMaterial, slot: usize, asset: Option<Uuid>) {
    match slot {
        0 => value.base_color_texture = asset,
        1 => value.normal_texture = asset,
        2 => value.metallic_roughness_texture = asset,
        _ => value.emissive_texture = asset,
    }
}
impl Imports {
    pub fn select_map(&mut self, id: Uuid, slot: usize, asset: Uuid) {
        if let Some(entry) = self.entries.iter_mut().find(|e| e.id == id)
            && let Some(Source::Material { value, images }) = &mut entry.source
        {
            images[slot] = None;
            set(value, slot, (!asset.is_nil()).then_some(asset));
            entry.preview_generation += 1;
            entry.model = None;
            entry.preview_error = None;
        }
    }
    pub fn cached_map(&mut self, asset: Uuid, image: LocalImage) {
        self.pending_remote.remove(&asset);
        for entry in &mut self.entries {
            if let Some(Source::Material { value, images }) = &mut entry.source {
                let mut changed = false;
                for (slot, id) in ids(value).into_iter().enumerate() {
                    if id == Some(asset) && images[slot].is_none() {
                        images[slot] = Some(image.clone());
                        changed = true;
                    }
                }
                if changed {
                    entry.preview_generation += 1;
                    entry.model = None;
                    entry.preview_error = None;
                }
            }
        }
    }
    pub fn request_remote_map(
        &mut self,
        asset: Uuid,
        pixels: Arc<crate::scene::jobs::SculptMap>,
        jobs: &crate::scene::jobs::Jobs,
        ctx: &egui::Context,
    ) {
        if !self.pending_remote.insert(asset) {
            return;
        }
        let tx = self.tx.clone();
        let wake = ctx.clone();
        jobs.execute(move || {
            let stride = pixels.width as usize * usize::from(pixels.components);
            let mut data = pixels.pixels.clone();
            for y in 0..pixels.height as usize / 2 {
                let (first, last) = data.split_at_mut((pixels.height as usize - 1 - y) * stride);
                first[y * stride..(y + 1) * stride].swap_with_slice(&mut last[..stride]);
            }
            let image = LocalImage {
                path: None,
                empty_alpha: false,
                key: asset,
                name: asset.to_string(),
                asset: Some(asset),
                pixels: Arc::new(DecodedImage {
                    width: pixels.width,
                    height: pixels.height,
                    components: pixels.components,
                    data,
                    discard: 0,
                }),
            };
            let _ = tx.send(Worker::Remote(asset, image));
            wake.request_repaint();
        });
    }
}
