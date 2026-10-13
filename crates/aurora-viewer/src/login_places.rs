//! Per-account last visited scene, captured without the interface.
//! Resize/PNG work runs off the frame thread; complete files publish atomically.
use crate::login_accounts::{Account, LastPlace};
use egui::{ColorImage, Context, TextureHandle};
use std::collections::HashMap;
use std::sync::Arc;

struct Ready {
    place: LastPlace,
    account: Option<(String, String)>,
    image: ColorImage,
}

#[derive(Default)]
pub struct Cache {
    textures: HashMap<uuid::Uuid, TextureHandle>,
    pending: Vec<crossbeam_channel::Receiver<Ready>>,
}

fn path(account: &Account, image: uuid::Uuid) -> Option<std::path::PathBuf> {
    Some(crate::settings::account_dir(&account.agent?).join(format!("login-place-{image}.png")))
}

fn thumbnail(w: u32, h: u32, pixels: &[u8]) -> Option<image::RgbaImage> {
    if w == 0 || h == 0 || pixels.len() != (w as usize).checked_mul(h as usize)?.checked_mul(4)? {
        return None;
    }
    let view = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(w, h, pixels)?;
    let scale = (512.0 / w as f32).min(288.0 / h as f32).min(1.0);
    Some(image::imageops::resize(
        &view,
        (w as f32 * scale).round().max(1.0) as u32,
        (h as f32 * scale).round().max(1.0) as u32,
        image::imageops::FilterType::Triangle,
    ))
}

impl Cache {
    pub fn get(&self, place: &LastPlace) -> Option<&TextureHandle> {
        self.textures.get(&place.image)
    }

    pub fn load(&mut self, accounts: &[Account]) {
        let accounts: Vec<_> = accounts
            .iter()
            .filter(|account| {
                account
                    .last_place
                    .as_ref()
                    .is_some_and(|place| !self.textures.contains_key(&place.image))
            })
            .cloned()
            .collect();
        if accounts.is_empty() {
            return;
        }
        let (tx, rx) = crossbeam_channel::unbounded();
        self.pending.push(rx);
        rayon::spawn(move || {
            for account in accounts {
                let Some(place) = account.last_place else {
                    continue;
                };
                let Some(agent) = account.agent else {
                    continue;
                };
                let Ok(bytes) = std::fs::read(crate::settings::account_dir(&agent).join(format!("login-place-{}.png", place.image))) else {
                    continue;
                };
                if bytes.len() > 1024 * 1024 {
                    continue;
                }
                let Ok(mut reader) = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format() else {
                    continue;
                };
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(512);
                limits.max_image_height = Some(512);
                limits.max_alloc = Some(2 * 1024 * 1024);
                reader.limits(limits);
                let Ok(image) = reader.decode() else {
                    continue;
                };
                let rgba = image.to_rgba8();
                let image = ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
                let _ = tx.send(Ready {
                    place,
                    account: None,
                    image,
                });
            }
        });
    }

    pub fn capture(&mut self, account: Account, region: String, w: u32, h: u32, pixels: Arc<Vec<u8>>, wait: bool) {
        let place = LastPlace {
            region,
            image: uuid::Uuid::new_v4(),
        };
        let Some(target) = path(&account, place.image) else {
            return;
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.pending.push(rx);
        let job = move || {
            let Some(small) = thumbnail(w, h, &pixels) else {
                return;
            };
            let image = ColorImage::from_rgba_unmultiplied([small.width() as usize, small.height() as usize], small.as_raw());
            let mut bytes = std::io::Cursor::new(Vec::new());
            if image::DynamicImage::ImageRgba8(small)
                .write_to(&mut bytes, image::ImageFormat::Png)
                .is_err()
            {
                return;
            }
            if let Some(parent) = target.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if crate::atomic_file::write(&target, &bytes.into_inner()).is_err() {
                return;
            }
            if !crate::login_accounts::remember_place(&account, place.clone()) {
                let _ = std::fs::remove_file(target);
                return;
            }
            if let Some(old) = &account.last_place
                && let Some(old_path) = path(&account, old.image)
            {
                let _ = std::fs::remove_file(old_path);
            }
            let _ = tx.send(Ready {
                account: Some(account.key()),
                place,
                image,
            });
        };
        if wait {
            job();
        } else {
            rayon::spawn(job);
        }
    }

    pub fn poll(&mut self, ctx: &Context, accounts: &mut [Account]) {
        self.pending.retain(|rx| {
            loop {
                match rx.try_recv() {
                    Ok(ready) => {
                        self.textures.insert(
                            ready.place.image,
                            ctx.load_texture(
                                format!("login-place-{}", ready.place.image),
                                ready.image,
                                egui::TextureOptions::LINEAR,
                            ),
                        );
                        if let Some(key) = ready.account
                            && let Some(account) = accounts.iter_mut().find(|account| account.key() == key)
                        {
                            account.last_place = Some(ready.place);
                        }
                        ctx.request_repaint();
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => return true,
                    Err(crossbeam_channel::TryRecvError::Disconnected) => return false,
                }
            }
        });
        self.textures.retain(|image, _| {
            accounts
                .iter()
                .any(|account| account.last_place.as_ref().is_some_and(|place| place.image == *image))
        });
    }

    /// Synthetic landscapes, without reading a saved scene or connecting a grid.
    pub fn preview(&mut self, ctx: &Context, p: &crate::theme::Palette, accounts: &mut [Account]) {
        for (index, account) in accounts.iter_mut().enumerate() {
            let place = LastPlace {
                region: format!("Example landscape {}", index + 1),
                image: uuid::Uuid::from_u128(index as u128 + 1),
            };
            let mut image = ColorImage::filled([256, 128], p.navy);
            for y in 0..128 {
                for x in 0..256 {
                    let ridge = 60.0 + (x as f32 / 28.0 + index as f32).sin() * 14.0;
                    image[(x, y)] = if (y as f32) < ridge {
                        p.indigo.lerp_to_gamma(p.violet_pale, 1.0 - y as f32 / 128.0)
                    } else {
                        p.navy.lerp_to_gamma(p.teal, (y as f32 - ridge) / 180.0)
                    };
                }
            }
            self.textures.insert(
                place.image,
                ctx.load_texture(format!("synthetic-place-{index}"), image, egui::TextureOptions::LINEAR),
            );
            account.last_place = Some(place);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scene_thumbnail_bounds_and_invalid_pixels() {
        let pixels = vec![120; 1024 * 768 * 4];
        let image = thumbnail(1024, 768, &pixels).expect("synthetic scene");
        assert_eq!(image.dimensions(), (384, 288));
        assert!(thumbnail(0, 768, &pixels).is_none());
        assert!(thumbnail(1024, 768, &pixels[..16]).is_none());
    }
}
