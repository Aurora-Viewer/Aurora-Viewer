//! Image fitting follows LLPreviewTexture::{populateRatioList,adjustAspectRatio}
//! (indra/newview/llpreviewtexture.cpp). Native pixel display is an extra option.
use super::*;
use std::sync::Arc;

#[derive(Default, Debug, PartialEq)]
pub(super) struct Metadata {
    pub uploader: Option<Uuid>,
    pub date: Option<String>,
}
struct Decoded {
    image: Option<(egui::TextureHandle, Arc<aurora_render::preview::Image>, bool)>,
    metadata: Metadata,
}

fn decode_source(ctx: &egui::Context, asset: Uuid, mut source: crate::scene::textures::PreviewSource) -> Decoded {
    if source.bytes.is_empty()
        && let Ok(bytes) = std::fs::read(&source.jpeg)
    {
        source.bytes = Arc::new(bytes);
    }
    let metadata = metadata(&source.bytes);
    // Missing dimensions in a cache-only scene entry do not mean its pixels
    // are full resolution. Only procedural images can arrive without a level.
    let complete = source.decoded.is_none_or(|level| level == 0);
    let image = source.inspector_image(asset).map(|image| {
        let [w, h] = image.size;
        let texture = ctx.load_texture(
            format!("resource-full-{asset}"),
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &image.rgba),
            egui::TextureOptions::LINEAR,
        );
        (texture, image, complete)
    });
    Decoded { metadata, image }
}

/// LLViewerFetchedTexture::createTexture parses `a=UUID&z=YYYYMMDDHHMMSS`
/// in the JPEG2000 COM marker. Stop at the first tile, never scan image data.
fn metadata(bytes: &[u8]) -> Metadata {
    let mut result = Metadata::default();
    let Some(start) = bytes.windows(2).position(|b| b == [0xff, 0x4f]) else {
        return result;
    };
    let mut pos = start + 2;
    while let Some(header) = bytes.get(pos..pos + 4) {
        if header[0] != 0xff || header[1] == 0x90 {
            break;
        }
        let length = u16::from_be_bytes([header[2], header[3]]) as usize;
        if length < 2 {
            break;
        }
        let Some(segment) = bytes.get(pos + 4..pos + 2 + length) else {
            break;
        };
        if header[1] == 0x64
            && let Some(comment) = segment.get(2..).and_then(|s| std::str::from_utf8(s).ok())
        {
            for field in comment.split('&') {
                if let Some((key, value)) = field.split_once('=') {
                    match key {
                        "a" => result.uploader = Uuid::parse_str(value).ok().filter(|id| !id.is_nil()),
                        "z" if value.len() == 14 && value.bytes().all(|c| c.is_ascii_digit()) => {
                            result.date = Some(format!(
                                "{}-{}-{} {}:{}:{} UTC",
                                &value[..4],
                                &value[4..6],
                                &value[6..8],
                                &value[8..10],
                                &value[10..12],
                                &value[12..]
                            ));
                        }
                        _ => {}
                    }
                }
            }
        }
        pos += 2 + length;
    }
    result
}

#[derive(Default)]
pub(crate) struct ImageView {
    pub display: Display,
    pub image: Option<egui::TextureHandle>,
    pub dimensions: Option<(u32, u32)>,
    pub export: Option<bool>,
    pub export_path: Option<std::path::PathBuf>,
    pub(super) metadata: Metadata,
    pub message: String,
    pixels: Option<Arc<aurora_render::preview::Image>>,
    revision: Option<crate::scene::textures::PreviewRevision>,
    pending: Option<crossbeam_channel::Receiver<Decoded>>,
    export_reply: Option<crossbeam_channel::Receiver<String>>,
}
impl ImageView {
    pub(super) fn batch_export(path: std::path::PathBuf, png: bool) -> Self {
        Self {
            export_path: Some(path),
            export: Some(png),
            ..Default::default()
        }
    }
    pub(super) fn export_finished(&self) -> bool {
        self.export.is_none() && self.export_reply.is_none() && !self.message.is_empty()
    }
    pub fn prepare(&mut self, ctx: &egui::Context, asset: Uuid, scene: &mut Scene) {
        if let Some(rx) = &self.pending
            && let Ok(decoded) = rx.try_recv()
        {
            self.pending = None;
            self.metadata = decoded.metadata;
            if let Some((texture, image, complete)) = decoded.image {
                let [w, h] = image.size;
                if complete {
                    self.dimensions = Some((w, h));
                    self.pixels = Some(image);
                }
                if complete || self.pixels.is_none() {
                    self.image = Some(texture);
                }
            }
        }
        let source = scene.textures.preview_source(&asset);
        let revision = source.revision();
        if self.pending.is_none() && self.revision != Some(revision) {
            self.revision = Some(revision);
            let (tx, rx) = crossbeam_channel::bounded(1);
            self.pending = Some(rx);
            let wake = ctx.clone();
            scene.jobs.execute(move || {
                let _ = tx.send(decode_source(&wake, asset, source));
                wake.request_repaint();
            });
        }
        if let Some(rx) = &self.export_reply
            && let Ok(message) = rx.try_recv()
        {
            self.message = message;
            self.export_reply = None;
        }
        if self.export_reply.is_none()
            && let Some(png) = self.export
            && let Some(pixels) = self.pixels.clone()
        {
            self.export = None;
            let (tx, rx) = crossbeam_channel::bounded(1);
            self.export_reply = Some(rx);
            let wake = ctx.clone();
            let batch_path = self.export_path.take();
            std::thread::spawn(move || {
                let (extension, format) = if png {
                    ("png", image::ImageFormat::Png)
                } else {
                    ("tga", image::ImageFormat::Tga)
                };
                let message = if let Some(path) = batch_path {
                    let result = write_batch(&path, &pixels, format);
                    match result {
                        Ok(()) => aurora_i18n::tr!("resource-preview-exported").to_owned(),
                        Err(reason) => aurora_i18n::tr!("resource-preview-export-error", reason = reason.to_string()),
                    }
                } else if let Some(path) = rfd::FileDialog::new()
                    .set_title(aurora_i18n::tr!("common-save-as"))
                    .set_file_name(format!("{asset}.{extension}"))
                    .add_filter(extension, &[extension])
                    .save_file()
                {
                    match image::save_buffer_with_format(
                        path,
                        &pixels.rgba,
                        pixels.size[0],
                        pixels.size[1],
                        image::ColorType::Rgba8,
                        format,
                    ) {
                        Ok(()) => aurora_i18n::tr!("resource-preview-exported").to_owned(),
                        Err(reason) => aurora_i18n::tr!("resource-preview-export-error", reason = reason.to_string()),
                    }
                } else {
                    String::new()
                };
                let _ = tx.send(message);
                wake.request_repaint();
            });
        }
    }
}

fn write_batch(path: &std::path::Path, pixels: &aurora_render::preview::Image, format: image::ImageFormat) -> image::ImageResult<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(image::ImageError::IoError)?;
    let mut writer = std::io::BufWriter::new(file);
    image::write_buffer_with_format(
        &mut writer,
        &pixels.rgba,
        pixels.size[0],
        pixels.size[1],
        image::ColorType::Rgba8,
        format,
    )?;
    std::io::Write::flush(&mut writer).map_err(image::ImageError::IoError)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Display {
    pub actual_size: bool,
    pub keep_ratio: bool,
    /// Zero means the original image ratio; the rest are the Legacy presets.
    pub ratio: usize,
    pub custom_ratio: [f32; 2],
}
impl Default for Display {
    fn default() -> Self {
        Self {
            actual_size: false,
            keep_ratio: true,
            ratio: 0,
            custom_ratio: [1.0, 1.0],
        }
    }
}
const RATIOS: [(&str, f32); 8] = [
    ("", 0.0),
    ("1:1", 1.0),
    ("4:3", 4.0 / 3.0),
    ("10:7", 10.0 / 7.0),
    ("3:2", 1.5),
    ("16:10", 1.6),
    ("16:9", 16.0 / 9.0),
    ("2:1", 2.0),
];

pub(super) fn image_size(display: Display, source: Vec2, available: Vec2, pixels_per_point: f32) -> Vec2 {
    if display.actual_size {
        return source / pixels_per_point.max(0.1);
    }
    if !display.keep_ratio {
        return available;
    }
    let ratio = if display.ratio == RATIOS.len() {
        let [x, y] = display.custom_ratio;
        if x.is_finite() && y.is_finite() && x > 0.0 && y > 0.0 {
            (x / y).clamp(0.0005, 2000.0)
        } else {
            1.0
        }
    } else {
        RATIOS
            .get(display.ratio)
            .filter(|(_, r)| *r > 0.0)
            .map_or(source.x / source.y.max(1.0), |(_, r)| *r)
    };
    let width = available.x.min(available.y * ratio);
    Vec2::new(width, width / ratio)
}

/// The display choices over the picture: fit or actual size, refresh, copy
/// the UUID, save as, the ratio and the dimensions.
pub(super) fn toolbar(
    ui: &mut egui::Ui,
    p: &Palette,
    item: &aurora_net::inventory::InvItem,
    dimensions: Option<(u32, u32)>,
    display: &mut Display,
    actions: &mut Vec<Action>,
) {
    ui.horizontal_wrapped(|ui| {
        display_mode(ui, p, display);
        if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("resource-preview-refresh"), true) {
            actions.push(Action::RefreshTexture(item.asset_id));
        }
        if widgets::icon_button(ui, p, "copy-filled", aurora_i18n::tr!("resource-preview-copy-uuid"), true) {
            ui.ctx().copy_text(item.asset_id.to_string());
        }
        let full = rules::COPY | rules::MODIFY | rules::TRANSFER;
        ui.add_enabled_ui(item.owner_mask & full == full, |ui| {
            egui::ComboBox::from_id_salt("texture_export")
                .selected_text(aurora_i18n::tr!("common-save-as"))
                .show_ui(ui, |ui| {
                    for (label, png) in [("PNG", true), ("TGA", false)] {
                        if ui.button(label).clicked() {
                            actions.push(Action::ExportTexture { item: item.id, png });
                            ui.close();
                        }
                    }
                });
        });
    });
    ratio_row(ui, p, dimensions, display);
}

/// Shared by inventory textures and profile photos; one image pixel per
/// physical screen pixel in native mode, including on scaled displays.
pub(crate) fn display_mode(ui: &mut egui::Ui, p: &Palette, display: &mut Display) {
    let mut mode = usize::from(display.actual_size);
    widgets::segmented(
        ui,
        p,
        &mut mode,
        &[
            (aurora_i18n::tr!("resource-preview-fit"), true),
            (aurora_i18n::tr!("resource-preview-actual"), true),
        ],
    );
    display.actual_size = mode == 1;
}

/// The ratio choices and the picture's dimensions.
fn ratio_row(ui: &mut egui::Ui, p: &Palette, dimensions: Option<(u32, u32)>, display: &mut Display) {
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(!display.actual_size, |ui| {
            widgets::switch_labeled(ui, p, &mut display.keep_ratio, aurora_i18n::tr!("resource-preview-keep-ratio"));
            ui.add_enabled_ui(display.keep_ratio, |ui| {
                let original = aurora_i18n::tr!("resource-preview-original-ratio");
                let current = if display.ratio == RATIOS.len() {
                    aurora_i18n::tr!("resource-preview-custom-ratio")
                } else {
                    RATIOS
                        .get(display.ratio)
                        .map_or(original, |(label, _)| if label.is_empty() { original } else { label })
                };
                egui::ComboBox::from_id_salt("texture_ratio")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for (index, (label, _)) in RATIOS.iter().enumerate() {
                            ui.selectable_value(&mut display.ratio, index, if index == 0 { original } else { label });
                        }
                        ui.selectable_value(&mut display.ratio, RATIOS.len(), aurora_i18n::tr!("resource-preview-custom-ratio"));
                    });
                if display.ratio == RATIOS.len() {
                    ui.add(egui::DragValue::new(&mut display.custom_ratio[0]).range(0.001..=2000.0).speed(0.1));
                    ui.label(":");
                    ui.add(egui::DragValue::new(&mut display.custom_ratio[1]).range(0.001..=2000.0).speed(0.1));
                }
            });
        });
        ui.add(
            egui::Label::new(
                RichText::new(dimensions.map_or_else(
                    || aurora_i18n::tr!("resource-preview-dimensions-loading").to_owned(),
                    |(width, height)| {
                        aurora_i18n::tr!(
                            "resource-preview-dimensions",
                            width = width.to_string(),
                            height = height.to_string()
                        )
                    },
                ))
                .small()
                .color(p.muted),
            )
            .wrap_mode(egui::TextWrapMode::Extend),
        );
    });
}

/// The ratio choices and the picture in all the room left in `ui`; the
/// picture's rect once drawn.
pub(crate) fn show_image(
    ui: &mut egui::Ui,
    p: &Palette,
    image: Option<&egui::TextureHandle>,
    dimensions: Option<(u32, u32)>,
    display: &mut Display,
    drag_item: Option<Uuid>,
) -> Option<egui::Rect> {
    ratio_row(ui, p, dimensions, display);
    picture(ui, p, image, dimensions, display, drag_item)
}

/// The picture, in all the room `ui` has; its rect once drawn. `drag_item`
/// is the inventory item its drag carries.
pub(super) fn picture(
    ui: &mut egui::Ui,
    p: &Palette,
    image: Option<&egui::TextureHandle>,
    dimensions: Option<(u32, u32)>,
    display: &Display,
    drag_item: Option<Uuid>,
) -> Option<egui::Rect> {
    let available = Vec2::new(ui.available_width(), ui.available_height().max(24.0));
    let mut image_rect = None;
    ui.allocate_ui_with_layout(available, egui::Layout::top_down(egui::Align::Center), |ui| {
        ui.set_min_size(available);
        if let Some(image) = image {
            let source = dimensions.map_or(image.size_vec2(), |(w, h)| Vec2::new(w as f32, h as f32));
            let size = image_size(*display, source, available, ui.ctx().pixels_per_point());
            let mut draw = |ui: &mut egui::Ui| {
                let (canvas, _) = ui.allocate_exact_size(if display.actual_size { size } else { available }, egui::Sense::hover());
                let rect = egui::Rect::from_center_size(canvas.center(), size);
                checkerboard(ui, p, rect);
                egui::Image::new(image)
                    .fit_to_exact_size(size)
                    .maintain_aspect_ratio(false)
                    .paint_at(ui, rect);
                image_rect = Some(rect);
                if let Some(item) = drag_item {
                    let response = ui.interact(rect, ui.scope_id().with("image_drag"), egui::Sense::click_and_drag());
                    response.dnd_set_drag_payload(InvDrag(item));
                    ui.ctx()
                        .data_mut(|data| data.insert_temp(egui::Id::unique(("resource_texture_preview", item)), response.rect));
                    response.on_hover_text(aurora_i18n::tr!("texture-library-drag-hint"));
                }
            };
            if display.actual_size {
                egui::ScrollArea::both()
                    .id_salt("texture_canvas")
                    .auto_shrink([false; 2])
                    .max_width(available.x)
                    .max_height(available.y)
                    .show(ui, draw);
            } else {
                draw(ui);
            }
        } else {
            ui.centered_and_justified(|ui| ui.label(aurora_i18n::tr!("ui-inventory-dialogs-loading-image")));
        }
    });
    image_rect
}

fn checkerboard(ui: &egui::Ui, p: &Palette, image: egui::Rect) {
    let visible = image.intersect(ui.clip_rect());
    let cell = 12.0;
    let start_x = ((visible.left() - image.left()) / cell).floor() as i32;
    let start_y = ((visible.top() - image.top()) / cell).floor() as i32;
    for y in start_y..=((visible.bottom() - image.top()) / cell).ceil() as i32 {
        for x in start_x..=((visible.right() - image.left()) / cell).ceil() as i32 {
            let rect =
                egui::Rect::from_min_size(image.min + Vec2::new(x as f32 * cell, y as f32 * cell), Vec2::splat(cell)).intersect(visible);
            let color = if (x + y) % 2 == 0 { p.field } else { p.raised };
            // The checkerboard reveals the source image's transparency.
            ui.painter().rect_filled(rect, 0.0, color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_export_preserves_dimensions_and_never_overwrites_existing_files() {
        let pixels = aurora_render::preview::Image {
            size: [19, 7],
            rgba: (0..19 * 7).flat_map(|index| [index as u8, 103, 219, (index * 7) as u8]).collect(),
        };
        for (extension, format) in [("png", image::ImageFormat::Png), ("tga", image::ImageFormat::Tga)] {
            let path = std::env::temp_dir().join(format!("aurora-texture-export-{}.{extension}", Uuid::new_v4()));
            write_batch(&path, &pixels, format).expect("new export");
            let decoded = image::open(&path).expect("exported texture").into_rgba8();
            assert_eq!(decoded.dimensions(), (19, 7));
            assert_eq!(decoded.into_raw(), pixels.rgba.as_ref());
            assert!(write_batch(&path, &pixels, format).is_err());
            std::fs::remove_file(path).expect("temporary export cleanup");
        }
    }
    #[test]
    fn cache_only_coarse_pixels_are_previewed_but_never_accepted_for_full_resolution_export() {
        let pixels = aurora_assets::DecodedImage {
            width: 1024,
            height: 512,
            components: 3,
            data: [48, 96, 144].repeat(1024 * 512),
            discard: 0,
        };
        let bytes = Arc::new(aurora_assets::import_texture::encode(&pixels, Default::default()).expect("encode fixture"));
        let ctx = egui::Context::default();
        for level in [2, 0] {
            let source = crate::scene::textures::PreviewSource {
                bytes: bytes.clone(),
                jpeg: Default::default(),
                blocks: Default::default(),
                decoded: Some(level),
                size: None, // Cache-only entries can lack the original J2K dimensions.
                procedural: false,
            };
            let (_, image, complete) = decode_source(&ctx, Uuid::from_u128(799), source).image.expect("decoded pixels");
            assert_eq!(image.size, [1024 >> level, 512 >> level]);
            assert_eq!(complete, level == 0, "a coarse preview must not unlock PNG/TGA export");
        }
    }
    #[test]
    fn image_fit_preserves_stretches_and_displays_native_pixels_at_any_dpi() {
        let source = Vec2::new(1024.0, 512.0);
        assert_eq!(
            image_size(Display::default(), source, Vec2::new(900.0, 600.0), 1.0),
            Vec2::new(900.0, 450.0)
        );
        assert_eq!(
            image_size(
                Display {
                    keep_ratio: false,
                    ..Default::default()
                },
                source,
                Vec2::new(900.0, 600.0),
                1.0
            ),
            Vec2::new(900.0, 600.0)
        );
        assert_eq!(
            image_size(
                Display {
                    actual_size: true,
                    ..Default::default()
                },
                source,
                Vec2::new(300.0, 200.0),
                2.0
            ),
            Vec2::new(512.0, 256.0)
        );
        assert_eq!(
            image_size(
                Display {
                    ratio: 1,
                    ..Default::default()
                },
                source,
                Vec2::new(900.0, 600.0),
                1.0
            ),
            Vec2::splat(600.0)
        );
        assert_eq!(
            image_size(
                Display {
                    ratio: 8,
                    custom_ratio: [5.0, 4.0],
                    ..Default::default()
                },
                source,
                Vec2::new(900.0, 600.0),
                1.0
            ),
            Vec2::new(750.0, 600.0)
        );
        assert_eq!(serde_json::from_str::<Display>("{}").expect("old settings"), Display::default());
    }
    #[test]
    fn uploader_metadata_handles_absent_malformed_and_truncated_jpeg2000_segments() {
        let id = Uuid::from_u128(33);
        let comment = format!("a={id}&z=20261020140005&ignored=value");
        let mut bytes = vec![0xff, 0x4f, 0xff, 0x64];
        bytes.extend_from_slice(&((comment.len() + 4) as u16).to_be_bytes());
        bytes.extend_from_slice(&[0, 1]);
        bytes.extend_from_slice(comment.as_bytes());
        bytes.extend_from_slice(&[0xff, 0x90]);
        assert_eq!(
            metadata(&bytes),
            Metadata {
                uploader: Some(id),
                date: Some("2026-10-20 14:00:05 UTC".into())
            }
        );
        for end in 0..bytes.len() - 2 {
            assert_eq!(metadata(&bytes[..end]), Metadata::default());
        }
        assert_eq!(metadata(&[0xff, 0x4f, 0xff, 0x64, 0, 1]), Metadata::default());
    }
}
