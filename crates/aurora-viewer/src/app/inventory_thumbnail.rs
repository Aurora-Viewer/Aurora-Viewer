//! Bounded thumbnail jobs. File dialogs, decoding, resizing and JPEG2000
//! encoding run off the event thread; AIS is applied only after confirmation.

use super::*;
use crate::ui::inventory::{
    AnimationStats, InventoryUi,
    thumbnail::{Input as ImageInput, ThumbnailUi},
};
use crate::world::inventory::actions as rules;
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

enum Prepared {
    Photo(Vec<u8>),
    Upload(Vec<u8>),
    Existing(Uuid),
}
struct Reply {
    request: Uuid,
    result: Result<Option<Prepared>, String>,
}
pub(super) struct ImageJobs {
    tx: crossbeam_channel::Sender<Reply>,
    rx: crossbeam_channel::Receiver<Reply>,
    requests: HashMap<Uuid, Uuid>,
    paste: VecDeque<Uuid>,
    pub capture: Option<Uuid>,
    pub capturing: Option<Uuid>,
    pub auto_capture_pending: bool,
}
impl Default for ImageJobs {
    fn default() -> Self {
        let (tx, rx) = crossbeam_channel::bounded(4);
        Self {
            tx,
            rx,
            requests: HashMap::new(),
            paste: VecDeque::new(),
            capture: None,
            capturing: None,
            auto_capture_pending: false,
        }
    }
}
fn panel(st: &mut InventoryUi, request: Uuid) -> Option<&mut ThumbnailUi> {
    if st.thumbnail.pending == Some(request) {
        return Some(&mut st.thumbnail);
    }
    st.windows.iter_mut().find_map(|w| panel(&mut w.state, request))
}
impl App {
    pub(super) fn inventory_image_action(&mut self, request: Uuid, item: Uuid, input: ImageInput) {
        if self.inventory_images.requests.len() >= 4
            || self.inventory_ui.pending.is_some()
            || !rules::mutable(&self.world.inventory, item, &self.settings.inventory.protected)
        {
            self.finish_inventory_image(request, Some("Une modification est déjà en cours ou l’élément est protégé.".into()));
            return;
        }
        self.inventory_images.requests.insert(request, item);
        let tx = self.inventory_images.tx.clone();
        match input {
            ImageInput::File => {
                std::thread::spawn(move || {
                    let result = rfd::FileDialog::new()
                        .set_title("Choisir une image d’inventaire")
                        .add_filter("Images", &["png", "jpg", "jpeg", "bmp", "tga", "j2c", "j2k", "jp2"])
                        .pick_file()
                        .map(|path| file_thumbnail(&path).map(Prepared::Upload))
                        .transpose();
                    let _ = tx.send(Reply { request, result });
                });
            }
            ImageInput::Capture => {
                if self.inventory_images.capture.is_some() || self.inventory_images.capturing.is_some() {
                    self.finish_inventory_image(request, Some("Une photo est déjà en cours.".into()));
                } else {
                    self.inventory_images.capture = Some(request);
                }
            }
            ImageInput::Photo(pixels) => {
                std::thread::spawn(move || {
                    let result = aurora_assets::j2k::encode_thumbnail_j2k(256, 256, 4, &pixels)
                        .map(|data| Some(Prepared::Upload(data)))
                        .map_err(|_| "La photo n’a pas pu être encodée.".into());
                    let _ = tx.send(Reply { request, result });
                });
            }
            ImageInput::Texture { asset, resize } => self.fetch_inventory_image(request, asset, resize),
            ImageInput::Paste => self.inventory_images.paste.push_back(request),
        }
    }
    fn fetch_inventory_image(&mut self, request: Uuid, asset: Uuid, resize: bool) {
        let inv = &self.world.inventory;
        let asset = if let Some(it) = inv.items.get(&asset) {
            if !crate::ui::inventory::thumbnail::usable(inv, it) {
                self.finish_inventory_image(request, Some("Cette texture doit autoriser la copie et le transfert.".into()));
                return;
            }
            it.asset_id
        } else {
            asset
        };
        let matching: Vec<_> = inv.items.values().filter(|it| it.asset_type == 0 && it.asset_id == asset).collect();
        if asset.is_nil() || (!matching.is_empty() && !matching.iter().any(|it| crate::ui::inventory::thumbnail::usable(inv, it))) {
            self.finish_inventory_image(request, Some("Cette texture doit autoriser la copie et le transfert.".into()));
            return;
        }
        if self.demo {
            let pixels = vec![128; 256 * 256 * 4];
            match aurora_assets::j2k::encode_thumbnail_j2k(256, 256, 4, &pixels) {
                Ok(data) => self.inventory_image_source(request, asset, resize, Ok(data)),
                Err(_) => self.finish_inventory_image(request, Some("Image de démonstration indisponible.".into())),
            }
        } else {
            self.send(NetCommand::FetchInventoryThumbnail { request, asset, resize });
        }
    }
    pub(super) fn inventory_image_source(&mut self, request: Uuid, asset: Uuid, resize: bool, result: Result<Vec<u8>, String>) {
        if !self.inventory_images.requests.contains_key(&request) {
            return;
        }
        let tx = self.inventory_images.tx.clone();
        std::thread::spawn(move || {
            let result = result.and_then(|data| texture_thumbnail(asset, resize, &data)).map(Some);
            let _ = tx.send(Reply { request, result });
        });
    }
    pub(super) fn prepare_inventory_photo(&mut self, request: Uuid, w: u32, h: u32, pixels: Vec<u8>) {
        let tx = self.inventory_images.tx.clone();
        std::thread::spawn(move || {
            let result = image::RgbaImage::from_raw(w, h, pixels)
                .ok_or_else(|| "La capture est invalide.".to_owned())
                .and_then(|image| square(&image))
                .map(|image| Some(Prepared::Photo(image.into_raw())));
            let _ = tx.send(Reply { request, result });
        });
    }
    pub(super) fn finish_inventory_image(&mut self, request: Uuid, error: Option<String>) {
        self.inventory_images.requests.remove(&request);
        if let Some(st) = panel(&mut self.inventory_ui, request) {
            st.pending = None;
            st.error = error.unwrap_or_default();
        }
    }
    pub(super) fn poll_inventory_images(&mut self) {
        while let Some(request) = self.inventory_images.paste.pop_front() {
            let text = self.gfx.as_mut().and_then(|g| g.egui_state.clipboard_text()).unwrap_or_default();
            match Uuid::parse_str(text.trim()) {
                Ok(asset) => self.fetch_inventory_image(request, asset, false),
                Err(_) => self.finish_inventory_image(request, Some("Le presse-papiers doit contenir l’UUID d’une image.".into())),
            }
        }
        while let Ok(reply) = self.inventory_images.rx.try_recv() {
            let Some(item) = self.inventory_images.requests.get(&reply.request).copied() else {
                continue;
            };
            if panel(&mut self.inventory_ui, reply.request).is_none_or(|st| st.item != Some(item)) {
                self.finish_inventory_image(reply.request, None);
                continue;
            }
            match reply.result {
                Err(error) => self.finish_inventory_image(reply.request, Some(error)),
                Ok(None) => self.finish_inventory_image(reply.request, None),
                Ok(Some(Prepared::Photo(pixels))) => {
                    let texture = self.egui_ctx.load_texture(
                        format!("inventory_snapshot_{}", reply.request),
                        egui::ColorImage::from_rgba_unmultiplied([256, 256], &pixels),
                        egui::TextureOptions::LINEAR,
                    );
                    if let Some(st) = panel(&mut self.inventory_ui, reply.request) {
                        st.photo = Some(texture);
                        st.pixels = Some(Arc::new(pixels));
                    }
                    self.finish_inventory_image(reply.request, None);
                }
                Ok(Some(prepared)) => {
                    if self.inventory_ui.pending.is_some() {
                        // Keep a prepared image until the current AIS operation ends.
                        let _ = self.inventory_images.tx.try_send(Reply {
                            request: reply.request,
                            result: Ok(Some(prepared)),
                        });
                        break;
                    }
                    if !rules::mutable(&self.world.inventory, item, &self.settings.inventory.protected) {
                        self.finish_inventory_image(reply.request, Some("Cet élément n’est plus modifiable.".into()));
                        continue;
                    }
                    let Some(parent) = rules::parent(&self.world.inventory, item).filter(|id| !id.is_nil()) else {
                        self.finish_inventory_image(reply.request, Some("Dossier parent introuvable.".into()));
                        continue;
                    };
                    match prepared {
                        Prepared::Existing(asset) => {
                            match rules::patch(
                                &self.world.inventory,
                                item,
                                aurora_net::inventory::operations::metadata_thumbnail(asset),
                            ) {
                                Ok(change) => {
                                    self.inventory_ui.pending = Some(reply.request);
                                    self.inventory_ui.pending_refresh = change.refresh.clone();
                                    self.send(NetCommand::EditInventory {
                                        request: reply.request,
                                        change,
                                    });
                                }
                                Err(error) => self.finish_inventory_image(reply.request, Some(error)),
                            }
                        }
                        Prepared::Upload(data) => {
                            self.inventory_ui.pending = Some(reply.request);
                            self.inventory_ui.pending_refresh = vec![parent];
                            self.send(NetCommand::UploadInventoryThumbnail {
                                request: reply.request,
                                item,
                                folder: self.world.inventory.folders.contains_key(&item),
                                parent,
                                data,
                            });
                        }
                        Prepared::Photo(_) => unreachable!(),
                    }
                }
            }
        }
    }
    pub(super) fn poll_inventory_animation_stats(&mut self) {
        fn poll(st: &mut InventoryUi, inv: &crate::world::inventory::Inventory, anims: &mut crate::scene::anim::AnimStreamer) {
            if let Some(it) = st.preview.and_then(|id| inv.items.get(&id)).filter(|it| it.asset_type == 20)
                && let Some(bound) = anims.get(&it.asset_id)
            {
                let a = &bound.anim;
                st.animation_stats = Some((
                    it.id,
                    AnimationStats {
                        priority: a.base_priority,
                        duration: a.duration,
                        looping: a.looping,
                        entry: a.ease_in,
                        exit: a.ease_out,
                        joints: a.joints.len(),
                    },
                ));
            }
            for w in &mut st.windows {
                poll(&mut w.state, inv, anims);
            }
        }
        poll(&mut self.inventory_ui, &self.world.inventory, &mut self.scene.anims);
    }
}

fn square(image: &image::RgbaImage) -> Result<image::RgbaImage, String> {
    let edge = image.width().min(image.height());
    if edge == 0 {
        return Err("L’image est vide.".into());
    }
    let crop = image::imageops::crop_imm(image, (image.width() - edge) / 2, (image.height() - edge) / 2, edge, edge).to_image();
    Ok(image::imageops::resize(&crop, 256, 256, image::imageops::FilterType::Lanczos3))
}
fn rgba(data: &[u8]) -> Result<image::RgbaImage, String> {
    let info = aurora_assets::j2k_info(data).ok_or_else(|| "L’image JPEG2000 est invalide.".to_owned())?;
    if info.width > 4096 || info.height > 4096 {
        return Err("L’image dépasse 4096 pixels.".into());
    }
    let decoded = aurora_assets::decode_j2k(data, 0).map_err(|_| "L’image JPEG2000 n’a pas pu être décodée.".to_owned())?;
    let mut pixels = Vec::with_capacity(decoded.width as usize * decoded.height as usize * 4);
    for pixel in decoded.data.chunks_exact(decoded.components as usize) {
        let color = match pixel {
            [v] => [*v, *v, *v, 255],
            [v, a] => [*v, *v, *v, *a],
            [r, g, b] => [*r, *g, *b, 255],
            [r, g, b, a] => [*r, *g, *b, *a],
            _ => return Err("Format d’image invalide.".into()),
        };
        pixels.extend_from_slice(&color);
    }
    image::RgbaImage::from_raw(decoded.width, decoded.height, pixels).ok_or_else(|| "Pixels d’image invalides.".into())
}
fn texture_thumbnail(asset: Uuid, resize: bool, bytes: &[u8]) -> Result<Prepared, String> {
    let info = aurora_assets::j2k_info(bytes).ok_or_else(|| "L’image sélectionnée est invalide.".to_owned())?;
    if info.width != info.height || info.width < 64 {
        return Err("L’image doit être carrée et mesurer au moins 64 pixels.".into());
    }
    if info.width <= 256 {
        rgba(bytes)?;
        return Ok(Prepared::Existing(asset));
    }
    if !resize {
        return Err("L’image du presse-papiers doit mesurer entre 64 et 256 pixels.".into());
    }
    let image = square(&rgba(bytes)?)?;
    aurora_assets::j2k::encode_thumbnail_j2k(256, 256, 4, image.as_raw())
        .map(Prepared::Upload)
        .map_err(|_| "L’image n’a pas pu être encodée.".into())
}
fn file_thumbnail(path: &std::path::Path) -> Result<Vec<u8>, String> {
    if std::fs::metadata(path).map_err(|_| "Fichier inaccessible.".to_owned())?.len() > 32 * 1024 * 1024 {
        return Err("Le fichier dépasse 32 Mo.".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "Le fichier n’a pas pu être lu.".to_owned())?;
    let image = if matches!(
        path.extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("j2c" | "j2k" | "jp2")
    ) {
        rgba(&bytes)?
    } else {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| "Format d’image invalide.".to_owned())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        reader
            .decode()
            .map_err(|_| "Image invalide ou trop volumineuse.".to_owned())?
            .into_rgba8()
    };
    let image = square(&image)?;
    aurora_assets::j2k::encode_thumbnail_j2k(256, 256, 4, image.as_raw()).map_err(|_| "L’image n’a pas pu être encodée.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uploaded_pngs_become_valid_square_jpeg2000_with_alpha() {
        let path = std::env::temp_dir().join(format!("aurora-inventory-thumbnail-{}.png", Uuid::new_v4()));
        let source = image::RgbaImage::from_pixel(96, 80, image::Rgba([21, 42, 63, 170]));
        source.save(&path).expect("generated PNG");
        let result = file_thumbnail(&path);
        let _ = std::fs::remove_file(&path);
        let image = rgba(&result.expect("thumbnail")).expect("JPEG2000");
        assert_eq!(image.dimensions(), (256, 256));
        assert_eq!(image.get_pixel(128, 128), &image::Rgba([21, 42, 63, 170]));
    }
    #[test]
    fn pasted_images_are_validated_and_photos_are_cropped_to_square() {
        let id = Uuid::from_u128(1);
        let pixels = vec![32; 64 * 64 * 4];
        let data = aurora_assets::j2k::encode_thumbnail_j2k(64, 64, 4, &pixels).expect("encode");
        assert!(matches!(texture_thumbnail(id,false,&data), Ok(Prepared::Existing(actual)) if actual == id));
        assert!(texture_thumbnail(id, false, b"invalid").is_err());
        let image = image::RgbaImage::from_fn(640, 480, |x, _| {
            if (80..560).contains(&x) {
                image::Rgba([10, 20, 30, 128])
            } else {
                image::Rgba([255, 0, 0, 255])
            }
        });
        let photo = square(&image).expect("square");
        assert_eq!(photo.dimensions(), (256, 256));
        assert_eq!(photo.get_pixel(0, 0), &image::Rgba([10, 20, 30, 128]));
    }
}
