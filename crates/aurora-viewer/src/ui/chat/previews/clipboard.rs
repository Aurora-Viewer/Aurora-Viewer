//! A copied picture is also a file on Windows, where Explorer cannot paste
//! a bitmap. Encode/publish the PNG and offer both formats on one worker.

use egui::{ColorImage, Context, Id};
use std::sync::Arc;

fn pending_id() -> Id {
    Id::unique("chat_image_clipboard_request")
}

#[derive(Clone)]
enum State {
    Working,
    Copied(std::time::Instant),
    Failed,
}

/// Kept across discarded UI passes; delivery happens after the final pass.
pub(super) fn request(ctx: &Context, image: &Arc<ColorImage>) {
    ctx.data_mut(|d| {
        d.insert_temp(pending_id(), image.clone());
        d.insert_temp(pending_id().with("state"), (Arc::as_ptr(image) as usize, State::Working));
    });
    ctx.request_repaint();
}

pub(super) fn take(ctx: &Context) -> Option<Arc<ColorImage>> {
    ctx.data_mut(|d| d.remove_temp(pending_id()))
}

fn finish(ctx: &Context, image: &Arc<ColorImage>, result: Result<(), String>) {
    let state = match result {
        Ok(()) => State::Copied(std::time::Instant::now()),
        Err(error) => {
            log::warn!("Could not copy a chat image: {error}");
            State::Failed
        }
    };
    ctx.data_mut(|d| {
        let key = pending_id().with("state");
        if d.get_temp::<(usize, State)>(key)
            .is_some_and(|(id, _)| id == Arc::as_ptr(image) as usize)
        {
            d.insert_temp(key, (Arc::as_ptr(image) as usize, state));
        }
    });
    ctx.request_repaint();
    ctx.request_repaint_after(std::time::Duration::from_secs(2));
}

pub(super) fn tip(ctx: &Context, image: &Arc<ColorImage>) -> &'static str {
    let state = ctx.data(|d| d.get_temp::<(usize, State)>(pending_id().with("state")));
    match state.filter(|(id, _)| *id == Arc::as_ptr(image) as usize).map(|(_, state)| state) {
        Some(State::Working) => aurora_i18n::tr!("ui-chat-preview-copy-working"),
        Some(State::Copied(at)) if at.elapsed() < std::time::Duration::from_secs(2) => aurora_i18n::tr!("ui-chat-preview-copied"),
        Some(State::Failed) => aurora_i18n::tr!("ui-chat-preview-copy-failed"),
        Some(State::Copied(_)) | None => aurora_i18n::tr!("ui-chat-preview-copy"),
    }
}

pub(super) fn update(ctx: &Context, output: &mut egui::PlatformOutput) {
    let Some(image) = take(ctx) else { return };
    #[cfg(windows)]
    {
        let _ = output;
        native::send(ctx, image);
    }
    #[cfg(not(windows))]
    {
        output.commands.push(egui::OutputCommand::CopyImage((*image).clone()));
        finish(ctx, &image, Ok(()));
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::hash::{Hash, Hasher};
    use std::path::{Path, PathBuf};
    use std::sync::{OnceLock, mpsc};

    pub(super) fn send(ctx: &Context, image: Arc<ColorImage>) {
        type Request = (Context, Arc<ColorImage>);
        static WORKER: OnceLock<Option<mpsc::Sender<Request>>> = OnceLock::new();
        let worker = WORKER.get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Request>();
            std::thread::Builder::new()
                .name("image-clipboard".into())
                .spawn(move || {
                    while let Ok((ctx, image)) = rx.recv() {
                        let result = copy(&crate::settings::cache_dir().join("clipboard-images"), &image);
                        finish(&ctx, &image, result);
                    }
                })
                .ok()
                .map(|_| tx)
        });
        if worker.as_ref().is_none_or(|tx| tx.send((ctx.clone(), image.clone())).is_err()) {
            finish(ctx, &image, Err("clipboard worker unavailable".into()));
        }
    }

    /// Content-addressed files survive closing the picture/viewer so a later
    /// Explorer paste still has its source. Re-copying reuses the same file;
    /// the viewer's normal cache clearing owns their lifetime.
    fn prepare(root: &Path, image: &ColorImage) -> Result<(PathBuf, Vec<u8>), String> {
        let rgba: Vec<u8> = image.pixels.iter().flat_map(|pixel| pixel.to_srgba_unmultiplied()).collect();
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        image.size.hash(&mut hash);
        rgba.hash(&mut hash);
        let path = root.join(format!("Aurora-image-{:016x}.png", hash.finish()));
        std::fs::create_dir_all(root).map_err(|error| error.to_string())?;
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, image.width() as u32, image.height() as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        {
            let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
            writer.write_image_data(&rgba).map_err(|error| error.to_string())?;
        }
        crate::cache::write(&path, &bytes).map_err(|error| error.to_string())?;
        Ok((path, rgba))
    }

    fn copy(root: &Path, image: &ColorImage) -> Result<(), String> {
        let (path, rgba) = prepare(root, image)?;
        let mut board = arboard::Clipboard::new().map_err(|error| error.to_string())?;
        board
            .set_image(arboard::ImageData {
                width: image.width(),
                height: image.height(),
                bytes: std::borrow::Cow::Borrowed(&rgba),
            })
            .map_err(|error| error.to_string())?;
        // On Windows arboard's file_list appends CF_HDROP without emptying
        // the PNG/DIB offer installed by set_image (platform/windows.rs).
        board.set().file_list(&[path]).map_err(|error| error.to_string())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn explorer_source_is_a_complete_png_with_original_alpha_and_a_stable_name() {
            let root = std::env::temp_dir().join(format!("aurora-clipboard-test-{}", uuid::Uuid::new_v4()));
            let image = ColorImage::new(
                [2, 1],
                vec![
                    egui::Color32::from_rgb(25, 80, 210),
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 128),
                ],
            );
            let (path, rgba) = prepare(&root, &image).unwrap();
            let decoded = image::open(&path).unwrap().to_rgba8();
            assert_eq!(decoded.dimensions(), (2, 1));
            assert_eq!(decoded.as_raw(), &rgba);
            assert_eq!(decoded.get_pixel(1, 0).0[3], 128);
            assert_eq!(prepare(&root, &image).unwrap().0, path);
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
            std::fs::remove_file(path).unwrap();
            std::fs::remove_dir(root).unwrap();
        }
    }
}
