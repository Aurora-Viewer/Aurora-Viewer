//! The interface's images of a whole session: profile pictures, insignias,
//! thumbnails. Each is an egui texture (and its CPU copy, egui_shadow.rs),
//! made once from a texture of the streamer and asked for by the windows
//! that show it. The copies nothing has used for a while go, oldest first,
//! once they hold more than `MAX_BYTES`: the window that shows one again
//! asks for it, as for the first time. The Legacy Viewer frees them sooner:
//! 30 s after the last widget that holds one lets it go, however few they
//! are (`lazy_flush_timeout`, LLViewerTextureList::updateImageDecodePriority,
//! indra/newview/llviewertexturelist.cpp).
//!
//! Used means asked for or drawn: many windows only ask for what they lack,
//! and only the shapes of the frame tell what those still show. Images that
//! cannot be read again (imported files, the demo's own) are never noted
//! here, and stay.

use egui::{Shape, TextureId, epaint::ClippedShape};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// About 400 pictures of 256 px: a long list scrolled back, a panel closed
/// and opened again show theirs at once.
const MAX_BYTES: u64 = 96 << 20;
/// Under memory pressure (texture_budget.rs).
const PRESSED_BYTES: u64 = 24 << 20;
/// Nothing younger goes: the time to come back to a window.
const IDLE: Duration = Duration::from_secs(30);
/// Frames between two looks at what is drawn (cheap, but nothing needs it
/// more often: `IDLE` is much longer).
const LOOK_EVERY: u64 = 16;

struct Held {
    used: Instant,
    /// A picture and an image of the same asset are two textures.
    textures: Vec<(TextureId, u64)>,
}

#[derive(Default)]
pub(super) struct UiImageCache {
    held: HashMap<Uuid, Held>,
    owners: HashMap<TextureId, Uuid>,
    bytes: u64,
}

impl UiImageCache {
    /// `texture` now shows `image`, `width` × `height` (a finer copy of the
    /// same texture replaces the size noted).
    pub fn note(&mut self, image: Uuid, texture: TextureId, width: u32, height: u32, now: Instant) {
        let bytes = u64::from(width) * u64::from(height) * 4;
        let held = self.held.entry(image).or_insert_with(|| Held {
            used: now,
            textures: Vec::new(),
        });
        held.used = now;
        match held.textures.iter_mut().find(|t| t.0 == texture) {
            Some(t) => {
                self.bytes = self.bytes - t.1 + bytes;
                t.1 = bytes;
            }
            None => {
                held.textures.push((texture, bytes));
                self.bytes += bytes;
                self.owners.insert(texture, image);
            }
        }
    }

    /// `texture` was dropped to be read again: its replacement is noted
    /// anew.
    pub fn forget(&mut self, image: &Uuid, texture: TextureId) {
        let Some(held) = self.held.get_mut(image) else {
            return;
        };
        if let Some(at) = held.textures.iter().position(|t| t.0 == texture) {
            self.bytes -= held.textures.swap_remove(at).1;
            self.owners.remove(&texture);
        }
        if held.textures.is_empty() {
            self.held.remove(image);
        }
    }

    /// A window asked for `image`.
    pub fn used(&mut self, image: &Uuid, now: Instant) {
        if let Some(held) = self.held.get_mut(image) {
            held.used = now;
        }
    }

    /// A frame's shapes, every `LOOK_EVERY` frames: what they draw is used.
    pub fn look(&mut self, frame: u64, shapes: &[ClippedShape], now: Instant) -> bool {
        if !frame.is_multiple_of(LOOK_EVERY) {
            return false;
        }
        if !self.held.is_empty() {
            for clipped in shapes {
                self.drawn(&clipped.shape, now);
            }
        }
        true
    }

    fn drawn(&mut self, shape: &Shape, now: Instant) {
        if let Shape::Vec(shapes) = shape {
            for shape in shapes {
                self.drawn(shape, now);
            }
        } else if let Some(image) = self.owners.get(&shape.texture_id()) {
            let image = *image;
            self.used(&image, now);
        }
    }

    /// The images to drop to fit (under memory pressure, the lower limit),
    /// oldest first: forgotten here.
    pub fn evict(&mut self, pressed: bool, now: Instant) -> Vec<Uuid> {
        let limit = if pressed { PRESSED_BYTES } else { MAX_BYTES };
        if self.bytes <= limit {
            return Vec::new();
        }
        let mut idle: Vec<(Instant, Uuid)> = self
            .held
            .iter()
            .filter(|(_, held)| now.duration_since(held.used) >= IDLE)
            .map(|(id, held)| (held.used, *id))
            .collect();
        idle.sort_unstable();
        let mut gone = Vec::new();
        for (_, id) in idle {
            if self.bytes <= limit {
                break;
            }
            if let Some(held) = self.held.remove(&id) {
                for (texture, bytes) in held.textures {
                    self.bytes -= bytes;
                    self.owners.remove(&texture);
                }
                gone.push(id);
            }
        }
        gone
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIDE: u32 = 256;
    const PICTURE: u64 = (SIDE * SIDE * 4) as u64;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n + 1)
    }

    fn filled(n: u64, at: Instant) -> UiImageCache {
        let mut cache = UiImageCache::default();
        for k in 0..n {
            cache.note(id(k.into()), TextureId::User(k), SIDE, SIDE, at);
        }
        cache
    }

    #[test]
    fn the_copies_stop_at_a_ceiling_oldest_first() {
        let start = Instant::now();
        let fit = MAX_BYTES / PICTURE;
        let mut cache = filled(fit, start);
        let later = start + IDLE;
        assert!(cache.evict(false, later).is_empty(), "exactly at the limit nothing goes");
        cache.note(id(9_000), TextureId::User(9_000), SIDE, SIDE, later);
        cache.note(id(9_001), TextureId::User(9_001), SIDE, SIDE, later);
        cache.used(&id(0), later);
        let gone = cache.evict(false, later);
        assert_eq!(gone.len(), 2);
        assert!(!gone.contains(&id(0)), "used since: another one goes");
        assert!(!gone.contains(&id(9_000)) && !gone.contains(&id(9_001)), "too young");
        assert!(cache.bytes <= MAX_BYTES);
        assert_eq!(cache.owners.len() as u64, fit);
    }

    #[test]
    fn what_was_used_lately_stays_over_the_limit_until_it_is_idle() {
        let start = Instant::now();
        let mut cache = filled(MAX_BYTES / PICTURE + 500, start);
        assert!(cache.evict(false, start + IDLE / 2).is_empty());
        assert!(cache.bytes > MAX_BYTES);
        assert_eq!(cache.evict(false, start + IDLE).len(), 500);
    }

    #[test]
    fn memory_pressure_lowers_the_ceiling() {
        let start = Instant::now();
        let mut cache = filled(MAX_BYTES / PICTURE, start);
        let gone = cache.evict(true, start + IDLE);
        assert_eq!(gone.len() as u64, (MAX_BYTES - PRESSED_BYTES) / PICTURE);
        assert!(cache.bytes <= PRESSED_BYTES);
    }

    #[test]
    fn a_picture_noted_again_counts_once_and_two_textures_of_an_asset_go_together() {
        let now = Instant::now();
        let mut cache = UiImageCache::default();
        cache.note(id(1), TextureId::User(1), 64, 64, now);
        cache.note(id(1), TextureId::User(1), SIDE, SIDE, now);
        assert_eq!(cache.bytes, PICTURE, "a finer copy replaces the pixels in place");
        cache.note(id(1), TextureId::User(2), SIDE, SIDE, now);
        assert_eq!(cache.bytes, 2 * PICTURE);
        cache.bytes = MAX_BYTES + 1;
        assert_eq!(cache.evict(false, now + IDLE), vec![id(1)]);
        assert!(cache.owners.is_empty());
    }

    #[test]
    fn a_refreshed_texture_leaves_nothing_of_its_old_copy() {
        let now = Instant::now();
        let mut cache = UiImageCache::default();
        cache.note(id(1), TextureId::User(1), SIDE, SIDE, now);
        cache.note(id(1), TextureId::User(2), SIDE, SIDE, now);
        cache.forget(&id(1), TextureId::User(1));
        cache.note(id(1), TextureId::User(3), SIDE, SIDE, now);
        assert_eq!(cache.bytes, 2 * PICTURE, "the picture of the same asset stays");
        cache.forget(&id(1), TextureId::User(2));
        cache.forget(&id(1), TextureId::User(3));
        assert!(cache.held.is_empty() && cache.owners.is_empty());
        assert_eq!(cache.bytes, 0);
    }

    #[test]
    fn a_picture_drawn_is_used_however_it_is_painted() {
        let ctx = egui::Context::default();
        let image = |name: &str| ctx.load_texture(name, egui::ColorImage::filled([4, 4], egui::Color32::RED), Default::default());
        let (by_image, by_painter, by_mesh, nested, hidden) = (image("a"), image("b"), image("c"), image("d"), image("e"));
        let start = Instant::now();
        let mut cache = UiImageCache::default();
        for (k, t) in [&by_image, &by_painter, &by_mesh, &nested, &hidden].into_iter().enumerate() {
            cache.note(id(k as u128), t.id(), 4, 4, start);
        }
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.image(egui::load::SizedTexture::new(by_image.id(), [16.0, 16.0]));
            let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(16.0, 16.0));
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(by_painter.id(), rect, uv, egui::Color32::WHITE);
            let mut mesh = egui::Mesh::with_texture(by_mesh.id());
            mesh.add_rect_with_uv(rect, uv, egui::Color32::WHITE);
            ui.painter().add(mesh.clone());
            mesh.texture_id = nested.id();
            ui.painter().add(Shape::Vec(vec![Shape::mesh(mesh)]));
        });
        let later = start + IDLE;
        assert!(!cache.look(1, &output.shapes, later), "only every few frames");
        assert!(cache.look(LOOK_EVERY, &output.shapes, later));
        for k in 0..4 {
            assert_eq!(cache.held[&id(k)].used, later, "image {k}");
        }
        assert_eq!(cache.held[&id(4)].used, start, "not drawn");
    }
}
