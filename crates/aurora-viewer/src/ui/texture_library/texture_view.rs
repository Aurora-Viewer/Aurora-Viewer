//! Image fitting follows LLPreviewTexture::{populateRatioList,adjustAspectRatio}
//! (indra/newview/llpreviewtexture.cpp). Native pixel display is an extra option.
use super::*;
use std::sync::Arc;

#[derive(Default, Debug, PartialEq)]
pub(crate) struct Metadata {
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
/// The object exporter reads the uploader too (DAESaver::updateTextureInfo).
pub(crate) fn metadata(bytes: &[u8]) -> Metadata {
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
    /// The latest decoded level, complete or not: what the panorama viewer
    /// looks into (the export keeps using `pixels`, complete only).
    pub(super) shown: Option<Arc<aurora_render::preview::Image>>,
    revision: Option<crate::scene::textures::PreviewRevision>,
    pending: Option<crossbeam_channel::Receiver<Decoded>>,
    export_reply: Option<crossbeam_channel::Receiver<String>>,
    /// Copy UUID allowed for (asset, inventory generation, god): the rule
    /// walks the whole inventory, so not on every frame (as the picker and
    /// the material editor keep it).
    uuid_shown: Option<(uuid::Uuid, u64, bool, bool)>,
}
impl ImageView {
    /// Whether "Copy UUID" is offered for `asset`
    /// (LLPreviewTexture::postBuild: full permissions; any god sees it),
    /// computed again only when the asset, the inventory or the god state
    /// changes.
    pub(super) fn copy_uuid_allowed(&mut self, inventory: &crate::world::inventory::Inventory, asset: uuid::Uuid, god: bool) -> bool {
        match self.uuid_shown {
            Some((a, generation, g, shown)) if a == asset && generation == inventory.generation && g == god => shown,
            _ => {
                let shown = rules::asset_uuid_shown_to(inventory, asset, god);
                self.uuid_shown = Some((asset, inventory.generation, god, shown));
                shown
            }
        }
    }

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
                let first = self.pixels.is_none();
                if complete || first {
                    self.shown = Some(image.clone());
                }
                if complete {
                    self.dimensions = Some((w, h));
                    self.pixels = Some(image);
                }
                if complete || first {
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

/// What the toolbar lets an item's holder do with its texture: copy the asset
/// UUID and save the image, both for full permissions only
/// (LLPreviewTexture::postBuild shows "Copy UUID" for them, and Save As needs
/// the same, LLPreviewTexture::canSaveAs / get_can_copy_texture, indra/newview/
/// llpreviewtexture.cpp, lltexturectrl.cpp). The panorama view of a texture
/// reads the same gates: it opens a window on the image, never a way out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Gates {
    pub copy_uuid: bool,
    pub save_as: bool,
}

/// The gates of an item whose holder has the permission bits `held`.
/// `copy_uuid_shown` is the shared asset-id rule (`rules::asset_uuid_shown_to`,
/// through `ImageView::copy_uuid_allowed`: full permissions, or a god);
/// Save As needs full permissions.
pub(super) fn gates(held: u32, copy_uuid_shown: bool) -> Gates {
    Gates {
        copy_uuid: copy_uuid_shown,
        save_as: held & rules::FULL == rules::FULL,
    }
}

/// The display choices over the picture: fit or actual size, refresh, copy
/// the UUID, save as, the ratio and the dimensions.
pub(super) fn toolbar(
    ui: &mut egui::Ui,
    p: &Palette,
    item: &aurora_net::inventory::InvItem,
    inventory: &crate::world::inventory::Inventory,
    copy_uuid: bool,
    dimensions: Option<(u32, u32)>,
    display: &mut Display,
    // the 360° switch of a texture that is 2:1, and whether it is on
    mut panorama: Option<&mut bool>,
    actions: &mut Vec<Action>,
) {
    ui.horizontal_wrapped(|ui| {
        display_mode(ui, p, display);
        if widgets::icon_button(ui, p, "refresh", aurora_i18n::tr!("resource-preview-refresh"), true) {
            actions.push(Action::RefreshTexture(item.asset_id));
        }
        // LLPreviewTexture::postBuild shows "Copy UUID" for full permissions
        // only, or to a god (`ImageView::copy_uuid_allowed`); Save As needs
        // full permissions (both through `gates`)
        let gates = gates(rules::held(inventory, item), copy_uuid);
        if widgets::icon_button(
            ui,
            p,
            "copy-filled",
            aurora_i18n::tr!("resource-preview-copy-uuid"),
            gates.copy_uuid,
        ) {
            ui.ctx().copy_text(item.asset_id.to_string());
        }
        ui.add_enabled_ui(gates.save_as, |ui| {
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
        if let Some(on) = panorama.as_deref_mut() {
            widgets::switch_labeled(ui, p, on, aurora_i18n::tr!("resource-preview-360"));
        }
    });
    if panorama.is_some_and(|on| *on) {
        // (a panorama has no ratio to keep)
        ui.horizontal_wrapped(|ui| dimensions_label(ui, p, dimensions));
    } else {
        ratio_row(ui, p, dimensions, display);
    }
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

/// The picture's dimensions, small and muted.
fn dimensions_label(ui: &mut egui::Ui, p: &Palette, dimensions: Option<(u32, u32)>) {
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
        dimensions_label(ui, p, dimensions);
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

/// The 360° view of a texture in all the room `ui` has: the panorama
/// viewer (drag, wheel, keys, enlarge), not the flat picture.
pub(super) fn panorama_picture(
    ui: &mut egui::Ui,
    p: &Palette,
    id: egui::Id,
    view: &mut crate::ui::panorama::PanoView,
    image: &Arc<aurora_render::preview::Image>,
) {
    let available = Vec2::new(ui.available_width().max(24.0), ui.available_height().max(24.0));
    view.show(ui, p, id, image, available);
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

    /// Copy UUID is decided once per asset, inventory generation and god
    /// state, not by walking the inventory on every frame.
    #[test]
    fn copy_uuid_is_computed_again_only_when_its_inputs_change() {
        use aurora_net::inventory::InvItem;
        let asset = uuid::Uuid::from_u128(77);
        let mut inventory = crate::world::inventory::Inventory::default();
        let mut view = ImageView::default();
        assert!(view.copy_uuid_allowed(&inventory, asset, true), "a god sees it");
        assert!(!view.copy_uuid_allowed(&inventory, asset, false), "not held");
        // a full-permission copy arrives, but the generation did not move: kept
        let item = InvItem {
            id: uuid::Uuid::from_u128(78),
            asset_id: asset,
            owner: inventory.owner,
            owner_mask: rules::FULL,
            ..Default::default()
        };
        inventory.items.insert(item.id, item);
        assert!(!view.copy_uuid_allowed(&inventory, asset, false), "cached for this generation");
        inventory.generation += 1;
        assert!(view.copy_uuid_allowed(&inventory, asset, false), "read again after a change");
    }

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
                local: None,
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

    /// What a toolbar of `held` permissions lets be done, found by clicking:
    /// (Copy UUID copies the asset id, Save As opens its list), in the flat
    /// view or with the 360° switch on.
    fn toolbar_gates(held: u32, panorama: bool) -> (bool, bool) {
        use crate::ui::widgets::elide_tests::Frames;
        let p = crate::theme::Theme::default().palette();
        let mut inventory = crate::world::inventory::Inventory::default();
        let item = aurora_net::inventory::InvItem {
            id: Uuid::from_u128(4),
            asset_id: Uuid::from_u128(5),
            owner: inventory.owner,
            owner_mask: held,
            ..Default::default()
        };
        inventory.items.insert(item.id, item.clone());
        let copy_uuid = rules::asset_uuid_shown_to(&inventory, item.asset_id, false);
        let draw = |frames: &mut Frames, events: Vec<egui::Event>| {
            let mut display = Display::default();
            let mut on = panorama;
            frames.run_with_output(events, |ui| {
                let mut actions = Vec::new();
                toolbar(
                    ui,
                    &p,
                    &item,
                    &inventory,
                    copy_uuid,
                    Some((2048, 1024)),
                    &mut display,
                    panorama.then_some(&mut on),
                    &mut actions,
                );
            })
        };
        let click = |x: f32| {
            let mut frames = Frames::new();
            let pos = egui::pos2(x, 14.0);
            let button = |pressed| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            };
            draw(&mut frames, Vec::new());
            draw(&mut frames, vec![egui::Event::PointerMoved(pos)]);
            draw(&mut frames, vec![button(true)]);
            let (_, released) = draw(&mut frames, vec![button(false)]);
            let copied = released
                .commands
                .iter()
                .any(|c| matches!(c, egui::OutputCommand::CopyText(text) if *text == item.asset_id.to_string()));
            // (the list a click on Save As opens, a frame later)
            let (texts, _) = draw(&mut frames, Vec::new());
            let listed = texts.iter().any(|t| t.source == "PNG");
            (copied, listed)
        };
        let (mut copy, mut save) = (false, false);
        for x in (4..420).step_by(5) {
            let (copied, listed) = click(x as f32);
            copy |= copied;
            save |= listed;
        }
        (copy, save)
    }

    /// Copy UUID and Save As of the texture preview follow `gates` for the
    /// item of the owner, a group-owned one, someone else's, a Library item
    /// and each missing right, in the flat view and in the 360° view alike:
    /// the panorama is a window on the image, never a way out.
    #[test]
    fn the_toolbar_follows_the_permission_gates_in_the_flat_and_the_360_view() {
        use crate::world::inventory::actions::{COPY, FULL, MODIFY, TRANSFER};
        let cases = [
            ("the owner's, full", 0x7fff_ffffu32, true),
            ("group-owned or someone else's", 0, false),
            ("a Library item, copy only", COPY, false),
            ("a Library item, everyone full", FULL, true),
            ("no copy", MODIFY | TRANSFER, false),
            ("no transfer", COPY | MODIFY, false),
            ("no modify", COPY | TRANSFER, false),
            ("full and more", FULL | 0x1_0000, true),
        ];
        for (name, held, allowed) in cases {
            for panorama in [false, true] {
                assert_eq!(toolbar_gates(held, panorama), (allowed, allowed), "{name}, 360°: {panorama}");
            }
        }
    }

    /// The layout of the texture preview with its 360° switch on (the
    /// toolbar, the viewer in the room above the footer) keeps its window's
    /// size at every scale.
    #[test]
    fn the_360_texture_preview_layout_keeps_its_window_still() {
        let image = Arc::new(aurora_render::preview::Image {
            size: [2, 1],
            rgba: Arc::from([0u8; 8]),
        });
        let item = aurora_net::inventory::InvItem {
            asset_id: Uuid::from_u128(6),
            ..Default::default()
        };
        crate::ui::widgets::steady_tests::assert_window_steady(
            "texture-360-test",
            &[Vec2::new(460.0, 520.0), Vec2::new(300.0, 300.0)],
            "OK",
            || (Display::default(), crate::ui::panorama::PanoView::new(0.0)),
            |(display, view), ctx, p| {
                let mut open = true;
                Floater::new("texture-360-test", "360", egui::pos2(40.0, 40.0), Vec2::new(460.0, 520.0)).show(ctx, p, &mut open, |ui| {
                    let mut on = true;
                    toolbar(
                        ui,
                        p,
                        &item,
                        &crate::world::inventory::Inventory::default(),
                        true,
                        Some((2048, 1024)),
                        display,
                        Some(&mut on),
                        &mut Vec::new(),
                    );
                    let (mut room, foot) = crate::ui::widgets::above_footer(ui);
                    panorama_picture(&mut room, p, egui::Id::unique("texture-360-test-view"), view, &image);
                    foot.show(ui, |ui| {
                        let _ = ui.button("OK");
                    });
                });
            },
        );
    }
}
