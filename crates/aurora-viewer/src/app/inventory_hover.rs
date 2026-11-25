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
    image: Option<egui::TextureHandle>,
}

#[derive(Default)]
pub(super) struct Images {
    pub ready: HashMap<Uuid, egui::TextureHandle>,
    entries: HashMap<Uuid, Entry>,
    grid: Grid,
    streaming: HashSet<Uuid>,
}

struct Plan {
    active: HashSet<Uuid>,
    #[cfg(test)]
    removed: Vec<Uuid>,
}

impl Images {
    fn remove(&mut self, id: Uuid) {
        self.entries.remove(&id);
        self.publish(id);
    }

    fn publish(&mut self, id: Uuid) {
        let hover = self.entries.get(&id).and_then(|entry| entry.image.as_ref());
        let tile = self.grid.tiles.get(&id).and_then(|tile| tile.image.as_ref());
        let image = hover.into_iter().chain(tile).max_by_key(|image| image.byte_size());
        if let Some(image) = image {
            self.ready.insert(id, image.clone());
        } else {
            self.ready.remove(&id);
        }
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
                    image: None,
                },
            );
        }
        Plan {
            active,
            #[cfg(test)]
            removed,
        }
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
        if let Some(texture) = entry.image.as_mut() {
            texture.set(image, egui::TextureOptions::LINEAR);
        } else {
            entry.image = Some(ctx.load_texture(format!("inventory-hover-{id}"), image, egui::TextureOptions::LINEAR));
        }
        entry.width = width;
        self.publish(id);
        true
    }
}

// At 77 × 66 points per small tile, a 4K pane can exceed 1,400 visible
// entries. The byte limit also covers multiple large-icon windows.
const GRID_LIMIT: usize = 2048;
const GRID_BYTES: usize = 48 * 1024 * 1024;
const GRID_SIDE: u32 = 128;
const GRID_STREAMS: usize = 8;
const GRID_ATTEMPT: Duration = Duration::from_secs(5);
const GRID_BACKOFF: Duration = Duration::from_secs(20);

struct Tile {
    side: u32,
    have: (u32, u32),
    image: Option<egui::TextureHandle>,
    done: bool,
    started: Option<Instant>,
    retry: Option<Instant>,
}

#[derive(Default)]
struct Grid {
    order: Vec<Uuid>,
    tiles: HashMap<Uuid, Tile>,
    previous: Vec<(Uuid, u32)>,
}

impl Grid {
    // Only visible grid images own handles. Keep hover's 32-entry LRU
    // independent; returning to a folder reuses the streamer's disk cache.
    fn visible(&mut self, requests: &mut Vec<(Uuid, u32)>) -> Vec<Uuid> {
        if &self.previous == requests {
            requests.clear();
            return Vec::new();
        }
        self.previous.clone_from(requests);
        self.order.clear();
        let mut sizes: HashMap<Uuid, u32> = HashMap::new();
        let mut bytes = 0usize;
        for (id, side) in requests.drain(..) {
            if id.is_nil() || (!sizes.contains_key(&id) && sizes.len() == GRID_LIMIT) {
                continue;
            }
            let size = side.clamp(1, GRID_SIDE);
            let slot = sizes.entry(id).or_insert_with(|| {
                self.order.push(id);
                0
            });
            if *slot < size {
                bytes += ((size * size - *slot * *slot) * 4) as usize;
                *slot = size;
            }
        }
        // For an exceptionally large set of visible tiles, lower their
        // resolution together so even the last one can keep its image.
        let scale = ((GRID_BYTES as f64 / bytes.max(1) as f64).sqrt()).min(1.0);
        let mut changed = Vec::new();
        self.tiles.retain(|id, _| {
            if sizes.contains_key(id) {
                true
            } else {
                changed.push(*id);
                false
            }
        });
        for (&id, &size) in &sizes {
            let side = ((size as f64 * scale) as u32).max(1);
            let tile = self.tiles.entry(id).or_insert_with(|| Tile {
                side,
                have: (0, 0),
                image: None,
                done: false,
                started: None,
                retry: None,
            });
            if tile.side != side {
                // Release the previous size now, including on a move from
                // a large tile to a smaller one; never transiently retain a
                // grid full of over-budget handles.
                tile.image = None;
                tile.have = (0, 0);
                tile.done = false;
                tile.started = None;
                tile.retry = None;
                tile.side = side;
                changed.push(id);
            }
        }
        changed
    }

    fn schedule(&mut self, now: Instant) -> Vec<Uuid> {
        let mut active = Vec::with_capacity(GRID_STREAMS);
        for &id in &self.order {
            let tile = self.tiles.get_mut(&id).unwrap();
            if tile.done {
                continue;
            }
            if let Some(start) = tile.started {
                if now.saturating_duration_since(start) < GRID_ATTEMPT {
                    active.push(id);
                } else {
                    tile.started = None;
                    tile.retry = Some(now + GRID_BACKOFF);
                }
            }
        }
        // Untried tiles precede all retries regardless of paint order.
        // A stationary pane therefore makes progress past failed assets.
        for retrying in [false, true] {
            for &id in &self.order {
                if active.len() == GRID_STREAMS {
                    return active;
                }
                let tile = self.tiles.get_mut(&id).unwrap();
                if tile.done || tile.started.is_some() || tile.retry.is_some() != retrying || tile.retry.is_some_and(|at| at > now) {
                    continue;
                }
                tile.started = Some(now);
                active.push(id);
            }
        }
        active
    }

    fn store(&mut self, ctx: &egui::Context, id: Uuid, w: u32, h: u32, rgba: &[u8], finest: Option<(u32, u32)>) -> bool {
        let Some(tile) = self.tiles.get_mut(&id) else { return false };
        if w == 0 || h == 0 || w > MAX_SIDE || h > MAX_SIDE || rgba.len() != (w * h * 4) as usize || (w <= tile.have.0 && h <= tile.have.1)
        {
            return false;
        }
        let longest = w.max(h);
        let side = tile.side.min(longest);
        let nw = (w * side / longest).max(1);
        let nh = (h * side / longest).max(1);
        let image = if (nw, nh) == (w, h) {
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], rgba)
        } else {
            let source = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(w, h, rgba).unwrap();
            let scaled = image::imageops::resize(&source, nw, nh, image::imageops::FilterType::Triangle);
            egui::ColorImage::from_rgba_unmultiplied([nw as usize, nh as usize], scaled.as_raw())
        };
        if let Some(t) = tile.image.as_mut() {
            t.set(image, egui::TextureOptions::LINEAR);
        } else {
            tile.image = Some(ctx.load_texture(format!("inventory-tile-{id}"), image, egui::TextureOptions::LINEAR));
        }
        tile.have = (w, h);
        tile.done = longest >= tile.side || finest.is_none_or(|(fw, fh)| w >= fw && h >= fh);
        if tile.done {
            tile.started = None;
        }
        true
    }
}

impl App {
    pub(super) fn update_inventory_hover_images(&mut self, renderer: &mut Renderer) {
        // Demand was collected in an earlier UI frame. A restriction may have arrived
        // before the streamer runs; discard that frame's requests and release interests.
        if self.world.inventory.can_preview(0) && !self.world.rlva.has("viewtexture") {
            self.inventory_ui
                .explorer_materials
                .update(self.world.inventory.root, &mut self.scene.materials);
        } else {
            self.inventory_ui.explorer_materials = Default::default();
            self.inventory_ui.wanted_explorer_images.clear();
            self.inventory_ui.wanted_hover_images.clear();
        }
        let cache = &mut self.inventory_hover_images;
        let now = Instant::now();
        let hover = cache.touch(std::mem::take(&mut self.inventory_ui.wanted_hover_images), now).active;
        let changed = cache.grid.visible(&mut self.inventory_ui.wanted_explorer_images);
        for id in changed {
            cache.publish(id);
        }
        // A hover and a grid tile can refer to the same asset. They share a
        // single transient interest and consume a decoded pixel buffer once.
        let mut wanted: Vec<_> = hover.iter().copied().filter(|id| !cache.entries[id].complete).collect();
        let grid = cache.grid.schedule(now);
        wanted.extend(grid.iter().copied().filter(|id| !hover.contains(id) || cache.entries[id].complete));
        let active: HashSet<_> = wanted.iter().copied().collect();
        let textures = &mut self.scene.textures;
        for id in cache.streaming.difference(&active) {
            textures.release_transient_ui_image(id);
        }
        cache.streaming = active;
        for id in wanted {
            textures.want_transient_ui_image(renderer, id, &self.scene.jobs);
            let finest = textures.ui_image_size(&id);
            if let Some((w, h, rgba)) = textures.take_ui_image(&id) {
                if hover.contains(&id) {
                    cache.store(&self.egui_ctx, id, w, h, &rgba);
                }
                if cache.grid.store(&self.egui_ctx, id, w, h, &rgba, finest) {
                    cache.publish(id);
                }
            }
            let mut have = cache.grid.tiles.get(&id).map_or((0, 0), |tile| tile.have);
            if hover.contains(&id) {
                let entry = cache.entries.get_mut(&id).unwrap();
                if entry.width > 0 && finest.is_none_or(|(w, _)| entry.width >= w) {
                    entry.complete = true;
                }
                if !entry.complete {
                    have = (entry.width, entry.width);
                }
            }
            if hover.contains(&id) && !cache.entries[&id].complete || cache.grid.tiles.get(&id).is_some_and(|tile| !tile.done) {
                textures.refine_ui_image_size(id, have, &self.scene.jobs);
            } else {
                textures.release_transient_ui_image(&id);
                cache.streaming.remove(&id);
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

    #[test]
    fn stationary_explorer_tiles_past_32_load_despite_eight_stalled_assets() {
        let ctx = egui::Context::default();
        let mut cache = Images::default();
        let start = Instant::now();
        let requests: Vec<_> = (1..=120).map(|n| (id(n), 24)).collect();
        cache.grid.visible(&mut requests.clone());
        let stalled: Vec<_> = (1..=GRID_STREAMS as u128).map(id).collect();
        assert_eq!(cache.grid.schedule(start), stalled);
        assert_eq!(cache.grid.schedule(start + Duration::from_secs(1)), stalled);
        let now = start + GRID_ATTEMPT;
        for _ in 0..20 {
            // A stationary pane reports the same UUID/size sequence on every frame.
            assert!(cache.grid.visible(&mut requests.clone()).is_empty());
            for asset in cache.grid.schedule(now) {
                assert!(!stalled.contains(&asset), "fresh tiles precede timed-out retries");
                assert!(cache.grid.store(&ctx, asset, 16, 16, &[255; 16 * 16 * 4], Some((16, 16))));
                cache.publish(asset);
            }
        }
        assert_eq!(cache.ready.len(), requests.len() - stalled.len());
        assert!(cache.ready.contains_key(&id(33)) && cache.ready.contains_key(&id(120)));
        assert!(cache.grid.schedule(now).is_empty());
        assert_eq!(cache.grid.schedule(now + GRID_BACKOFF), stalled);
        for asset in &stalled {
            assert!(cache.grid.store(&ctx, *asset, 48, 48, &[255; 48 * 48 * 4], Some((48, 48))));
            cache.publish(*asset);
            assert_eq!(cache.ready[asset].size(), [24, 24]);
        }
        assert_eq!(cache.ready.len(), 120);
    }

    #[test]
    fn explorer_deduplicates_panes_orders_work_and_limits_total_pixels() {
        let mut grid = Grid::default();
        let mut visible = vec![(id(9), 24), (Uuid::nil(), 120), (id(3), 40), (id(9), 120), (id(4), 200)];
        grid.visible(&mut visible);
        assert_eq!(grid.order, [id(9), id(3), id(4)]);
        assert_eq!(grid.tiles[&id(9)].side, 120);
        assert_eq!(grid.tiles[&id(4)].side, GRID_SIDE);
        assert_eq!(grid.schedule(Instant::now()), [id(9), id(3), id(4)]);
        grid.visible(&mut (100..100 + GRID_LIMIT as u128 + 500).map(|n| (id(n), 120)).collect());
        assert_eq!(grid.tiles.len(), GRID_LIMIT);
        let bytes: usize = grid.tiles.values().map(|t| (t.side * t.side * 4) as usize).sum();
        assert!(bytes <= GRID_BYTES);
    }

    #[test]
    fn shared_thumbnail_is_released_only_when_its_last_ui_owner_leaves() {
        let ctx = egui::Context::default();
        let mut cache = Images::default();
        let now = Instant::now();
        let asset = id(41);
        let mut requests = vec![(asset, 24)];
        cache.grid.visible(&mut requests);
        cache.grid.store(&ctx, asset, 64, 32, &[127; 64 * 32 * 4], Some((64, 32)));
        cache.publish(asset);
        assert_eq!(cache.ready[&asset].size(), [24, 12]);
        cache.touch(HashSet::from([asset]), now);
        cache.store(&ctx, asset, 4, 2, &[127; 4 * 2 * 4]);
        assert_eq!(
            cache.ready[&asset].size(),
            [24, 12],
            "a partial tooltip cannot blur the existing tile"
        );
        cache.store(&ctx, asset, 64, 32, &[127; 64 * 32 * 4]);
        assert_eq!(cache.ready[&asset].size(), [64, 32]);
        cache.touch(HashSet::new(), now + IDLE);
        assert_eq!(
            cache.ready[&asset].size(),
            [24, 12],
            "the same grid tile remains visible after hover expiration"
        );
        for id in cache.grid.visible(&mut vec![]) {
            cache.publish(id);
        }
        assert!(!cache.ready.contains_key(&asset));
    }
}
