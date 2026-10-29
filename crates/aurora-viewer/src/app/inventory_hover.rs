//! Bounded inventory hover images, separate from persistent profile/editor
//! images. Dropping a cached egui handle frees it through the normal frame
//! handover; streamer interests last only while the visible preview loads.

use super::*;
use std::collections::HashMap;
use uuid::Uuid;

const LIMIT: usize = 32;
const MAX_SIDE: u32 = 256;
const IDLE: Duration = Duration::from_secs(30);

struct Entry {
    touched: Instant,
    width: u32,
    complete: bool,
    streaming: bool,
}

#[derive(Default)]
pub(super) struct Images {
    pub ready: HashMap<Uuid, egui::TextureHandle>,
    entries: HashMap<Uuid, Entry>,
}

struct Plan {
    active: HashSet<Uuid>,
    removed: Vec<Uuid>,
}

impl Images {
    fn remove(&mut self, id: Uuid) {
        self.entries.remove(&id);
        self.ready.remove(&id);
    }

    fn touch(&mut self, wanted: HashSet<Uuid>, now: Instant) -> Plan {
        // There is normally one pointer tooltip. Keep the hard bound even
        // for an oversized batch or several independent inventory windows.
        let active: HashSet<_> = wanted.into_iter().filter(|id| !id.is_nil()).take(LIMIT).collect();
        let mut removed: Vec<_> = self
            .entries
            .iter()
            .filter_map(|(id, e)| (!active.contains(id) && now.saturating_duration_since(e.touched) >= IDLE).then_some(*id))
            .collect();
        for id in &removed {
            self.remove(*id);
        }
        for id in &active {
            if let Some(entry) = self.entries.get_mut(id) {
                entry.touched = now;
                continue;
            }
            if self.entries.len() >= LIMIT {
                let oldest = self
                    .entries
                    .iter()
                    .filter(|(id, _)| !active.contains(*id))
                    .min_by_key(|(_, e)| e.touched)
                    .map(|(id, _)| *id);
                let Some(oldest) = oldest else { continue };
                self.remove(oldest);
                removed.push(oldest);
            }
            self.entries.insert(
                *id,
                Entry {
                    touched: now,
                    width: 0,
                    complete: false,
                    streaming: false,
                },
            );
        }
        Plan { active, removed }
    }

    fn store(&mut self, ctx: &egui::Context, id: Uuid, width: u32, height: u32, rgba: &[u8]) -> bool {
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE || rgba.len() != (width * height * 4) as usize {
            return false;
        }
        let Some(entry) = self.entries.get_mut(&id) else { return false };
        if width <= entry.width {
            return false;
        }
        let image = egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], rgba);
        if let Some(texture) = self.ready.get_mut(&id) {
            texture.set(image, egui::TextureOptions::LINEAR);
        } else {
            self.ready.insert(
                id,
                ctx.load_texture(format!("inventory-hover-{id}"), image, egui::TextureOptions::LINEAR),
            );
        }
        entry.width = width;
        true
    }
}

impl App {
    pub(super) fn update_inventory_hover_images(&mut self, renderer: &mut Renderer) {
        let wanted = std::mem::take(&mut self.inventory_ui.wanted_hover_images);
        let cache = &mut self.inventory_hover_images;
        let plan = cache.touch(wanted, Instant::now());
        let textures = &mut self.scene.textures;
        for id in plan.removed {
            textures.release_transient_ui_image(&id);
        }
        // A tooltip left while loading must not keep fetching/refining in
        // the background. A partial cached picture can resume on next hover.
        for (id, entry) in &mut cache.entries {
            if entry.streaming && !plan.active.contains(id) {
                textures.release_transient_ui_image(id);
                entry.streaming = false;
            }
        }
        for id in plan.active {
            if cache.entries.get(&id).is_none_or(|entry| entry.complete) {
                continue;
            }
            textures.want_transient_ui_image(renderer, id, &self.scene.jobs);
            if let Some(entry) = cache.entries.get_mut(&id) {
                entry.streaming = true;
            }
            if let Some((width, height, rgba)) = textures.take_ui_image(&id) {
                cache.store(&self.egui_ctx, id, width, height, &rgba);
            }
            let Some(entry) = cache.entries.get_mut(&id) else { continue };
            if entry.width > 0 && textures.ui_image_width(&id).is_none_or(|target| entry.width >= target) {
                entry.complete = true;
                entry.streaming = false;
                textures.release_transient_ui_image(&id);
            } else {
                textures.refine_ui_image(id, entry.width, &self.scene.jobs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn inventory_hover_cache_a_thousand_images_remain_bounded_and_free_old_handles() {
        let ctx = egui::Context::default();
        let mut cache = Images::default();
        let start = Instant::now();
        let rgba = vec![127; (MAX_SIDE * MAX_SIDE * 4) as usize];
        let mut all_handles = HashMap::new();
        for number in 1..=1000 {
            let image = id(number);
            let now = start + Duration::from_millis(number as u64 * 800);
            ctx.begin_pass(egui::RawInput::default());
            let plan = cache.touch(HashSet::from([image]), now);
            for evicted in plan.removed {
                if let Some(handle) = all_handles.get(&evicted) {
                    assert!(ctx.tex_manager().read().meta(*handle).is_none(), "evicted egui allocation released");
                }
            }
            assert!(cache.store(&ctx, image, MAX_SIDE, MAX_SIDE, &rgba));
            all_handles.insert(image, cache.ready[&image].id());
            assert!(cache.entries.len() <= LIMIT && cache.ready.len() <= LIMIT);
            assert!(cache.ready.values().map(egui::TextureHandle::byte_size).sum::<usize>() <= LIMIT * 256 * 256 * 4);
            // Headless: no renderer applies the texture updates.
            let mut output = ctx.end_pass();
            output.textures_delta.clear();
        }
        let last_handle = cache.ready[&id(1000)].id();
        let expired = cache.touch(HashSet::new(), start + Duration::from_secs(831));
        assert_eq!(expired.removed.len(), LIMIT);
        assert!(cache.entries.is_empty() && cache.ready.is_empty());
        assert!(ctx.tex_manager().read().meta(last_handle).is_none());
        ctx.begin_pass(egui::RawInput::default());
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
    }

    #[test]
    fn inventory_hover_cache_keeps_recent_hits_and_active_previews_and_rejects_late_images() {
        let ctx = egui::Context::default();
        let mut cache = Images::default();
        let now = Instant::now();
        for n in 1..=LIMIT {
            cache.touch(HashSet::from([id(n as u128)]), now + Duration::from_millis(n as u64));
        }
        cache.touch(HashSet::from([id(1)]), now + Duration::from_millis(50));
        let plan = cache.touch(HashSet::from([id(33)]), now + Duration::from_millis(51));
        assert_eq!(plan.removed, [id(2)], "a cache hit refreshes its recency");
        assert!(
            !cache.store(&ctx, id(2), 1, 1, &[0; 4]),
            "late reply cannot recreate an evicted image"
        );
        assert!(
            !cache.store(&ctx, id(1), 4096, 4096, &[]),
            "unexpected large images cannot exceed the pixel budget"
        );
        cache.touch(HashSet::from([id(1)]), now + IDLE + Duration::from_secs(1));
        assert_eq!(cache.entries.len(), 1, "active hover survives idle expiry");
        let before = cache.entries.len();
        cache.touch(HashSet::from([Uuid::nil()]), now + IDLE + Duration::from_secs(2));
        assert_eq!(cache.entries.len(), before, "nil UUID never takes a cache slot");
        cache.touch((100..1100).map(id).collect(), now + IDLE + Duration::from_secs(3));
        assert_eq!(cache.entries.len(), LIMIT, "oversized request batches are bounded too");
    }
}
