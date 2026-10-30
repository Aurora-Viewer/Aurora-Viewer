//! Bounded thumbnail jobs. File dialogs, decoding, resizing and JPEG2000
//! encoding run off the event thread; AIS is applied only after confirmation.

use super::*;
use crate::ui::inventory::thumbnail::FileChoices;
use crate::ui::inventory::{
    AnimationStats, InventoryUi,
    thumbnail::{Input as ImageInput, ThumbnailUi},
};
use crate::world::inventory::actions as rules;
use std::collections::{HashMap, VecDeque};
use uuid::Uuid;

enum Prepared {
    Photo(Vec<u8>),
    Choices(FileChoices),
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
impl ImageJobs {
    pub(super) fn contains_request(&self, request: Uuid) -> bool {
        self.requests.contains_key(&request)
    }
}
fn panel(st: &mut InventoryUi, request: Uuid) -> Option<&mut ThumbnailUi> {
    if st.thumbnail.pending == Some(request) {
        return Some(&mut st.thumbnail);
    }
    st.windows.iter_mut().find_map(|w| panel(&mut w.state, request))
}
impl App {
    pub(super) fn demo_inventory_file_choices(&mut self, landscape: bool) {
        let Some(item) = self.inventory_ui.thumbnail.item else { return };
        let request = Uuid::new_v4();
        self.inventory_ui.thumbnail.pending = Some(request);
        self.inventory_images.requests.insert(request, item);
        let tx = self.inventory_images.tx.clone();
        std::thread::spawn(move || {
            let (width, height) = if landscape { (640, 320) } else { (320, 640) };
            let source = image::RgbaImage::from_fn(width, height, |x, y| {
                let u = x as f32 / width as f32;
                let v = y as f32 / height as f32;
                let color = if !(0.04..0.96).contains(&u) || !(0.02..0.98).contains(&v) {
                    [238, 173, 92, 255]
                } else if (u - 0.72).powi(2) + (v - 0.22).powi(2) * 4.0 < 0.016 {
                    [255, 220, 151, 255]
                } else if v > 0.82 {
                    if (0.1..0.9).contains(&u) && ((v > 0.86 && v < 0.88) || (v > 0.92 && v < 0.94)) {
                        [196, 237, 223, 255]
                    } else {
                        [24, 74, 83, 255]
                    }
                } else if v > 0.68 - 0.5 * (1.0 - (u - 0.35).abs() * 2.0).max(0.0) {
                    [55, 139, 143, 255]
                } else {
                    [35, 48 + (v * 60.0) as u8, 88 + (v * 50.0) as u8, 255]
                };
                image::Rgba(color)
            });
            let result = file_choices(&source).map(|pixels| Some(Prepared::Choices(pixels)));
            let _ = tx.send(Reply { request, result });
        });
    }

    pub(super) fn inventory_image_action(&mut self, request: Uuid, item: Uuid, input: ImageInput) {
        if self.inventory_images.requests.len() >= 4
            || self.inventory_ui.pending.is_some()
            || !rules::mutable(&self.world.inventory, item, &self.settings.inventory.protected)
        {
            self.finish_inventory_image(
                request,
                Some(aurora_i18n::tr!("app-inventory-thumbnail-an-edit-is-already-in-progress-or-the-item").into()),
            );
            return;
        }
        self.inventory_images.requests.insert(request, item);
        let tx = self.inventory_images.tx.clone();
        match input {
            ImageInput::File => {
                std::thread::spawn(move || {
                    let result = rfd::FileDialog::new()
                        .set_title(aurora_i18n::tr!("app-inventory-thumbnail-choose-inventory-image"))
                        .add_filter(
                            aurora_i18n::tr!("app-inventory-thumbnail-images"),
                            &["png", "jpg", "jpeg", "bmp", "tga", "j2c", "j2k", "jp2"],
                        )
                        .pick_file()
                        .map(|path| file_thumbnails(&path).map(Prepared::Choices))
                        .transpose();
                    let _ = tx.send(Reply { request, result });
                });
            }
            ImageInput::Capture => {
                if self.inventory_images.capture.is_some() || self.inventory_images.capturing.is_some() {
                    self.finish_inventory_image(
                        request,
                        Some(aurora_i18n::tr!("app-inventory-thumbnail-a-snapshot-is-already-in-progress").into()),
                    );
                } else {
                    self.inventory_images.capture = Some(request);
                }
            }
            ImageInput::Pixels(pixels) => {
                std::thread::spawn(move || {
                    let result = aurora_assets::j2k::encode_thumbnail_j2k(256, 256, 4, &pixels)
                        .map(|data| Some(Prepared::Upload(data)))
                        .map_err(|_| aurora_i18n::tr!("app-inventory-thumbnail-could-not-encode-the-picture").into());
                    let _ = tx.send(Reply { request, result });
                });
            }
            ImageInput::Crop { image, origin } => {
                std::thread::spawn(move || {
                    let result = crop_at(&image, origin)
                        .and_then(|pixels| {
                            aurora_assets::j2k::encode_thumbnail_j2k(256, 256, 4, pixels.as_raw())
                                .map_err(|_| aurora_i18n::tr!("app-inventory-thumbnail-could-not-encode-the-picture").to_owned())
                        })
                        .map(|data| Some(Prepared::Upload(data)));
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
                self.finish_inventory_image(
                    request,
                    Some(aurora_i18n::tr!("app-inventory-thumbnail-this-texture-must-allow-copying-and-transfer").into()),
                );
                return;
            }
            it.asset_id
        } else {
            asset
        };
        let matching: Vec<_> = inv.items.values().filter(|it| it.asset_type == 0 && it.asset_id == asset).collect();
        if asset.is_nil() || (!matching.is_empty() && !matching.iter().any(|it| crate::ui::inventory::thumbnail::usable(inv, it))) {
            self.finish_inventory_image(
                request,
                Some(aurora_i18n::tr!("app-inventory-thumbnail-this-texture-must-allow-copying-and-transfer").into()),
            );
            return;
        }
        if self.demo {
            let pixels = vec![128; 256 * 256 * 4];
            match aurora_assets::j2k::encode_thumbnail_j2k(256, 256, 4, &pixels) {
                Ok(data) => self.inventory_image_source(request, asset, resize, Ok(data)),
                Err(_) => self.finish_inventory_image(
                    request,
                    Some(aurora_i18n::tr!("app-inventory-thumbnail-demo-image-unavailable").into()),
                ),
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
                .ok_or_else(|| aurora_i18n::tr!("app-inventory-thumbnail-invalid-capture").to_owned())
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
    pub(super) fn finish_inventory_image_upload(&mut self, request: Uuid, error: Option<String>) {
        if let Some(st) = panel(&mut self.inventory_ui, request) {
            st.saved = error.is_none();
        }
        self.inventory_ui
            .finish_thumbnail_upload(request, error.as_deref(), std::time::Instant::now());
        self.finish_inventory_image(request, error);
    }
    pub(super) fn poll_inventory_images(&mut self) {
        while let Some(request) = self.inventory_images.paste.pop_front() {
            let text = self.gfx.as_mut().and_then(|g| g.egui_state.clipboard_text()).unwrap_or_default();
            match Uuid::parse_str(text.trim()) {
                Ok(asset) => self.fetch_inventory_image(request, asset, false),
                Err(_) => self.finish_inventory_image(
                    request,
                    Some(aurora_i18n::tr!("app-inventory-thumbnail-the-clipboard-must-contain-an-image-uuid").into()),
                ),
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
                Ok(Some(Prepared::Choices(pixels))) => {
                    if let Some(st) = panel(&mut self.inventory_ui, reply.request) {
                        st.set_file_choices(&self.egui_ctx, pixels);
                    }
                    self.finish_inventory_image(reply.request, None);
                }
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
                        self.finish_inventory_image(
                            reply.request,
                            Some(aurora_i18n::tr!("app-inventory-thumbnail-this-item-can-no-longer-be-modified").into()),
                        );
                        continue;
                    }
                    let Some(parent) = rules::parent(&self.world.inventory, item).filter(|id| !id.is_nil()) else {
                        self.finish_inventory_image(
                            reply.request,
                            Some(aurora_i18n::tr!("app-inventory-thumbnail-parent-folder-not-found").into()),
                        );
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
                                    self.inventory_ui.begin_thumbnail_upload(reply.request);
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
                            self.inventory_ui.begin_thumbnail_upload(reply.request);
                            self.inventory_ui.pending_refresh = vec![parent];
                            self.send(NetCommand::UploadInventoryThumbnail {
                                request: reply.request,
                                item,
                                folder: self.world.inventory.folders.contains_key(&item),
                                parent,
                                data,
                            });
                        }
                        Prepared::Photo(_) | Prepared::Choices(_) => unreachable!(),
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
        return Err(aurora_i18n::tr!("app-inventory-thumbnail-the-image-is-empty").into());
    }
    let crop = image::imageops::crop_imm(image, (image.width() - edge) / 2, (image.height() - edge) / 2, edge, edge).to_image();
    Ok(image::imageops::resize(&crop, 256, 256, image::imageops::FilterType::Lanczos3))
}
fn rgba(data: &[u8]) -> Result<image::RgbaImage, String> {
    let info = aurora_assets::j2k_info(data)
        .ok_or_else(|| aurora_i18n::tr!("app-inventory-thumbnail-the-jpeg2000-image-is-invalid").to_owned())?;
    if info.width > 4096 || info.height > 4096 {
        return Err(aurora_i18n::tr!("app-inventory-thumbnail-the-image-exceeds-4096-pixels").into());
    }
    let decoded = aurora_assets::decode_j2k(data, 0)
        .map_err(|_| aurora_i18n::tr!("app-inventory-thumbnail-could-not-decode-the-jpeg2000-image").to_owned())?;
    let mut pixels = Vec::with_capacity(decoded.width as usize * decoded.height as usize * 4);
    for pixel in decoded.data.chunks_exact(decoded.components as usize) {
        let color = match pixel {
            [v] => [*v, *v, *v, 255],
            [v, a] => [*v, *v, *v, *a],
            [r, g, b] => [*r, *g, *b, 255],
            [r, g, b, a] => [*r, *g, *b, *a],
            _ => return Err(aurora_i18n::tr!("app-inventory-thumbnail-invalid-image-format").into()),
        };
        pixels.extend_from_slice(&color);
    }
    image::RgbaImage::from_raw(decoded.width, decoded.height, pixels)
        .ok_or_else(|| aurora_i18n::tr!("app-inventory-thumbnail-invalid-image-pixels").into())
}
fn texture_thumbnail(asset: Uuid, resize: bool, bytes: &[u8]) -> Result<Prepared, String> {
    let info = aurora_assets::j2k_info(bytes)
        .ok_or_else(|| aurora_i18n::tr!("app-inventory-thumbnail-the-selected-image-is-invalid").to_owned())?;
    if info.width != info.height || info.width < 64 {
        return Err(aurora_i18n::tr!("app-inventory-thumbnail-the-image-must-be-square-and-at-least-64").into());
    }
    if info.width <= 256 {
        rgba(bytes)?;
        return Ok(Prepared::Existing(asset));
    }
    if !resize {
        return Err(aurora_i18n::tr!("app-inventory-thumbnail-the-clipboard-image-must-be-between-64-and-256").into());
    }
    let image = square(&rgba(bytes)?)?;
    aurora_assets::j2k::encode_thumbnail_j2k(256, 256, 4, image.as_raw())
        .map(Prepared::Upload)
        .map_err(|_| aurora_i18n::tr!("app-inventory-thumbnail-could-not-encode-the-image").into())
}
fn file_thumbnails(path: &std::path::Path) -> Result<FileChoices, String> {
    if std::fs::metadata(path)
        .map_err(|_| aurora_i18n::tr!("app-inventory-thumbnail-cannot-access-file").to_owned())?
        .len()
        > 32 * 1024 * 1024
    {
        return Err(aurora_i18n::tr!("app-inventory-thumbnail-the-file-exceeds-32-mb").into());
    }
    let bytes = std::fs::read(path).map_err(|_| aurora_i18n::tr!("app-inventory-thumbnail-could-not-read-the-file").to_owned())?;
    let image = if matches!(
        path.extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("j2c" | "j2k" | "jp2")
    ) {
        rgba(&bytes)?
    } else {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|_| aurora_i18n::tr!("app-inventory-thumbnail-invalid-image-format").to_owned())?;
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(4096);
        limits.max_image_height = Some(4096);
        limits.max_alloc = Some(64 * 1024 * 1024);
        reader.limits(limits);
        reader
            .decode()
            .map_err(|_| aurora_i18n::tr!("app-inventory-thumbnail-invalid-or-oversized-image").to_owned())?
            .into_rgba8()
    };
    file_choices(&image)
}

fn file_choices(image: &image::RgbaImage) -> Result<FileChoices, String> {
    if image.width() == 0 || image.height() == 0 {
        return Err(aurora_i18n::tr!("app-inventory-thumbnail-the-image-is-empty").into());
    }
    // LLFloaterSimpleSnapshot::uploadThumbnail(file) requests force_square in
    // LLViewerTextureList::convertToUploadFile (indra/newview/llviewertexturelist.cpp).
    // Preserve the entire source; proportional fit and crop are optional alternatives.
    let stretched = image::imageops::resize(image, 256, 256, image::imageops::FilterType::Lanczos3);
    let edge = image.width().max(image.height());
    let width = (u64::from(image.width()) * 256 / u64::from(edge)).max(1) as u32;
    let height = (u64::from(image.height()) * 256 / u64::from(edge)).max(1) as u32;
    let fitted = image::imageops::resize(image, width, height, image::imageops::FilterType::Lanczos3);
    let mut padded = image::RgbaImage::new(256, 256);
    image::imageops::replace(&mut padded, &fitted, i64::from((256 - width) / 2), i64::from((256 - height) / 2));
    // A draggable crop keeps the entire source, with its short edge at most 256 px.
    // This retains every output pixel's detail while bounding the canvas to 4 MiB,
    // including 1 x 4096 inputs; never upscale a long strip just for its preview.
    let short = image.width().min(image.height());
    let crop = if short > 256 {
        image::imageops::resize(
            image,
            (u64::from(image.width()) * 256 / u64::from(short)) as u32,
            (u64::from(image.height()) * 256 / u64::from(short)) as u32,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        image.clone()
    };
    Ok(FileChoices {
        pixels: [stretched.into_raw(), padded.into_raw()],
        crop,
    })
}

fn crop_at(image: &image::RgbaImage, [x, y]: [u32; 2]) -> Result<image::RgbaImage, String> {
    let edge = image.width().min(image.height());
    if edge == 0 || x > image.width() - edge || y > image.height() - edge {
        return Err(aurora_i18n::tr!("app-inventory-thumbnail-invalid-image-pixels").into());
    }
    let pixels = image::imageops::crop_imm(image, x, y, edge, edge).to_image();
    Ok(image::imageops::resize(&pixels, 256, 256, image::imageops::FilterType::Lanczos3))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn uploaded_pngs_become_valid_square_jpeg2000_with_alpha() {
        let path = std::env::temp_dir().join(format!("aurora-inventory-thumbnail-{}.png", Uuid::new_v4()));
        let source = image::RgbaImage::from_pixel(96, 80, image::Rgba([21, 42, 63, 170]));
        source.save(&path).expect("generated PNG");
        let result = file_thumbnails(&path);
        let _ = std::fs::remove_file(&path);
        let choices = result.expect("thumbnail choices");
        let edge = choices.crop.width().min(choices.crop.height());
        let cropped = crop_at(
            &choices.crop,
            [(choices.crop.width() - edge) / 2, (choices.crop.height() - edge) / 2],
        )
        .expect("center crop");
        for pixels in choices.pixels.into_iter().chain([cropped.into_raw()]) {
            let data = aurora_assets::j2k::encode_thumbnail_j2k(256, 256, 4, &pixels).expect("encode chosen thumbnail");
            let image = rgba(&data).expect("JPEG2000");
            assert_eq!(image.dimensions(), (256, 256));
            assert_eq!(image.get_pixel(128, 128), &image::Rgba([21, 42, 63, 170]));
            assert_eq!(image.as_raw(), &pixels, "the uploaded choice preserves its preview and alpha");
        }
    }
    #[test]
    fn rectangular_png_and_jpeg2000_files_produce_the_same_thumbnail_choices() {
        let source = image::RgbaImage::from_fn(128, 512, |x, y| image::Rgba([x as u8, (y / 2) as u8, 63, 170]));
        let expected = file_choices(&source).expect("source choices");
        for extension in ["png", "j2c"] {
            let path = std::env::temp_dir().join(format!("aurora-thumbnail-{}.{}", Uuid::new_v4(), extension));
            if extension == "png" {
                source.save(&path).expect("generated PNG");
            } else {
                let data = aurora_assets::j2k::encode_texture_j2k(128, 512, 4, source.as_raw(), true).expect("generated JPEG2000");
                std::fs::write(&path, data).expect("JPEG2000 file");
            }
            let actual = file_thumbnails(&path);
            let _ = std::fs::remove_file(&path);
            assert_eq!(actual.expect("file choices"), expected, "{extension}");
        }
    }

    #[test]
    fn local_thumbnail_choices_preserve_all_edges_or_crop_only_when_requested() {
        let red = image::Rgba([230, 0, 0, 170]);
        let blue = image::Rgba([0, 0, 230, 170]);
        let green = image::Rgba([0, 230, 0, 170]);
        let yellow = image::Rgba([230, 230, 0, 170]);
        for (width, height) in [(512, 1024), (1024, 512), (256, 256), (32, 64), (64, 32)] {
            let source = image::RgbaImage::from_fn(width, height, |x, y| {
                if y < height / 8 {
                    red
                } else if y >= height * 7 / 8 {
                    blue
                } else if x < width / 8 {
                    green
                } else if x >= width * 7 / 8 {
                    yellow
                } else {
                    image::Rgba([21, 42, 63, 170])
                }
            });
            let choices = file_choices(&source).expect("choices");
            let edge = choices.crop.width().min(choices.crop.height());
            let cropped = crop_at(
                &choices.crop,
                [(choices.crop.width() - edge) / 2, (choices.crop.height() - edge) / 2],
            )
            .expect("center crop");
            let [stretched, fitted] = choices
                .pixels
                .map(|pixels| image::RgbaImage::from_raw(256, 256, pixels).expect("256 px RGBA"));
            assert_eq!(stretched.get_pixel(128, 8), &red);
            assert_eq!(stretched.get_pixel(128, 248), &blue);
            assert_eq!(stretched.get_pixel(8, 128), &green);
            assert_eq!(stretched.get_pixel(248, 128), &yellow);
            if width < height {
                assert_eq!(fitted.get_pixel(8, 128).0[3], 0);
                assert_eq!(fitted.get_pixel(128, 8), &red);
                assert_eq!(fitted.get_pixel(128, 248), &blue);
                assert_ne!(cropped.get_pixel(128, 8), &red);
                assert_ne!(cropped.get_pixel(128, 248), &blue);
            } else if width > height {
                assert_eq!(fitted.get_pixel(128, 8).0[3], 0);
                assert_eq!(fitted.get_pixel(8, 128), &green);
                assert_eq!(fitted.get_pixel(248, 128), &yellow);
                assert_ne!(cropped.get_pixel(8, 128), &green);
                assert_ne!(cropped.get_pixel(248, 128), &yellow);
            } else {
                assert_eq!(stretched, fitted);
                assert_eq!(stretched, cropped);
            }
        }
        assert!(file_choices(&image::RgbaImage::new(0, 1)).is_err());
        assert!(file_choices(&image::RgbaImage::new(1, 0)).is_err());
        for (width, height) in [(1, 4096), (4096, 1)] {
            let choices = file_choices(&image::RgbaImage::from_pixel(width, height, red)).expect("thin source");
            assert_eq!(choices.crop.dimensions(), (width, height));
        }
    }

    #[test]
    fn dragged_crop_uploads_the_selected_source_region_with_alpha() {
        for (width, height) in [(256, 640), (640, 256)] {
            let source = image::RgbaImage::from_fn(width, height, |x, y| image::Rgba([(x / 3) as u8, (y / 3) as u8, 77, 170]));
            let choices = file_choices(&source).expect("choices");
            let edge = choices.crop.width().min(choices.crop.height());
            let limit = [width - edge, height - edge];
            for origin in [[0, 0], [limit[0] / 2, limit[1] / 2], limit] {
                let cropped = crop_at(&choices.crop, origin).expect("valid position");
                assert_eq!(cropped.dimensions(), (256, 256));
                assert_eq!(cropped.get_pixel(128, 128), source.get_pixel(origin[0] + 128, origin[1] + 128));
                let data = aurora_assets::j2k::encode_thumbnail_j2k(256, 256, 4, cropped.as_raw()).expect("encoded crop");
                assert_eq!(rgba(&data).expect("decoded crop"), cropped);
            }
            assert!(crop_at(&choices.crop, [limit[0] + 1, 0]).is_err());
            assert!(crop_at(&choices.crop, [0, limit[1] + 1]).is_err());
        }
        assert!(crop_at(&image::RgbaImage::new(0, 1), [0, 0]).is_err());
        let small = image::RgbaImage::from_pixel(32, 80, image::Rgba([21, 42, 63, 170]));
        assert_eq!(
            crop_at(&small, [0, 48]).expect("small source").get_pixel(128, 128),
            &image::Rgba([21, 42, 63, 170])
        );
    }

    #[test]
    fn panning_source_keeps_full_image_content_in_a_bounded_canvas() {
        for (width, height) in [(1024, 2048), (4096, 256), (256, 4096), (1, 4096), (4096, 1)] {
            let source = image::RgbaImage::from_pixel(width, height, image::Rgba([21, 42, 63, 170]));
            let choices = file_choices(&source).expect("choices");
            assert!(choices.crop.width().min(choices.crop.height()) <= 256);
            assert!(choices.crop.as_raw().len() <= 4 * 1024 * 1024);
            assert_eq!(choices.crop.get_pixel(0, 0), source.get_pixel(0, 0));
            assert_eq!(
                choices.crop.get_pixel(choices.crop.width() - 1, choices.crop.height() - 1),
                source.get_pixel(width - 1, height - 1)
            );
        }
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
