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
    pub fn open_map_picker(&mut self, id: Uuid, slot: usize) {
        let Some(Source::Material { value, images }) = self.entries.iter().find(|e| e.id == id).and_then(|e| e.source.as_ref()) else {
            return;
        };
        let asset = ids(value)[slot];
        let current = asset.or_else(|| images[slot].as_ref().map(|image| image.asset.unwrap_or(image.key)));
        self.map_before = Some((asset, images[slot].clone()));
        self.map_target = Some((id, slot));
        self.map_picker
            .open_with_defaults(aurora_i18n::tr!("import-texture"), current.unwrap_or_default(), None, true);
    }

    pub fn show_map_picker(
        &mut self,
        ctx: &egui::Context,
        p: &crate::theme::Palette,
        world: &mut crate::world::World,
        images: &HashMap<Uuid, egui::TextureHandle>,
    ) {
        let chosen = self.map_picker.show_live(ctx, p, world, images);
        if let Some((id, slot)) = self.map_target {
            if self.map_picker.cancelled() {
                // An imported map may have local pixels and no inventory UUID.
                // Restore them even if the last choice had the original UUID.
                if let Some((asset, image)) = &self.map_before
                    && let Some(entry) = self.entries.iter_mut().find(|e| e.id == id)
                    && let Some(Source::Material { value, images }) = &mut entry.source
                {
                    set(value, slot, *asset);
                    images[slot] = image.clone();
                    entry.preview_generation += 1;
                    entry.model = None;
                    entry.preview_error = None;
                }
            } else if let Some(asset) = chosen {
                self.select_map(id, slot, asset);
            }
        }
        if !self.map_picker.open {
            self.map_target = None;
            self.map_before = None;
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_picker_restores_imported_pixels_and_keeps_its_target_until_confirmation() {
        let mut ui = crate::ui::texture_picker::tests::PickerUi::new();
        let mut world = crate::world::World::new(Arc::new(crate::scene::avatar::AvatarLibrary::load()));
        let pixels = Arc::new(DecodedImage {
            width: 1,
            height: 1,
            components: 3,
            data: vec![10, 20, 30],
            discard: 0,
        });
        let image = LocalImage {
            path: None,
            key: Uuid::from_u128(77),
            name: "Imported map".into(),
            pixels: pixels.clone(),
            asset: None,
            empty_alpha: false,
        };
        let mut images = Box::new(std::array::from_fn(|_| None));
        images[1] = Some(image);
        let entry = Entry::new(
            "material.gltf".into(),
            Ok(Source::Material {
                value: PbrMaterial::default(),
                images,
            }),
            Default::default(),
        );
        let id = entry.id;
        let mut imports = Imports::default();
        imports.entries.push(entry);
        imports.open_map_picker(id, 1);
        ui.settle(|ctx, p| imports.show_map_picker(ctx, p, &mut world, &HashMap::new()));
        ui.click(aurora_i18n::tr!("common-blank"), |ctx, p| {
            imports.show_map_picker(ctx, p, &mut world, &HashMap::new())
        });
        assert!(imports.map_picker.open);
        assert_eq!(imports.map_target, Some((id, 1)));
        let Some(Source::Material { value, images }) = &imports.entries[0].source else {
            panic!("material")
        };
        assert_eq!(value.normal_texture, Some(aurora_prim::te::BLANK_TEXTURE));
        assert!(images[1].is_none());
        ui.click(aurora_i18n::tr!("common-none-2"), |ctx, p| {
            imports.show_map_picker(ctx, p, &mut world, &HashMap::new())
        });
        let Some(Source::Material { value, .. }) = &imports.entries[0].source else {
            panic!("material")
        };
        assert_eq!(value.normal_texture, None);
        ui.click(aurora_i18n::tr!("common-cancel"), |ctx, p| {
            imports.show_map_picker(ctx, p, &mut world, &HashMap::new())
        });
        let Some(Source::Material { value, images }) = &imports.entries[0].source else {
            panic!("material")
        };
        assert_eq!(value.normal_texture, None);
        assert!(Arc::ptr_eq(&images[1].as_ref().unwrap().pixels, &pixels));
        assert_eq!(imports.map_target, None);
        imports.open_map_picker(id, 1);
        ui.settle(|ctx, p| imports.show_map_picker(ctx, p, &mut world, &HashMap::new()));
        ui.click(aurora_i18n::tr!("common-none-2"), |ctx, p| {
            imports.show_map_picker(ctx, p, &mut world, &HashMap::new())
        });
        let Some(Source::Material { images, .. }) = &imports.entries[0].source else {
            panic!("material")
        };
        assert!(
            images[1].is_none(),
            "None clears a local map whose inventory UUID was already absent"
        );
        let generation = imports.entries[0].preview_generation;
        ui.click(aurora_i18n::tr!("common-ok"), |ctx, p| {
            imports.show_map_picker(ctx, p, &mut world, &HashMap::new())
        });
        assert_eq!(
            imports.entries[0].preview_generation, generation,
            "OK does not rebuild the preview again"
        );
        assert_eq!(imports.map_target, None);
    }
}
