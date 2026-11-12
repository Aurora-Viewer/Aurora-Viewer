//! World map tiles from the grid's map server (LLWorldMipmap):
//! `<map server>map-<level>-<x>-<y>-objects.jpg`, 256 px JPEGs where level L
//! covers 2^(L-1) regions per side, its corner aligned on that many regions.
//! Shared by the mini-map (level 1 under each region, like
//! LLSurface::getSTexture) and the world map. Kept on disk for a day (the
//! servers redraw them about daily) and as egui textures while in use.
//!
//! Ported from Firestorm's llworldmipmap.cpp (originally LGPL 2.1, Linden Research, Inc.).

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// LLWorldMipmap::MAP_LEVELS.
pub const MAP_LEVELS: u32 = 8;
/// Simultaneous downloads.
const MAX_IN_FLIGHT: usize = 6;
/// Tiles unused this long are dropped (textures freed) once over the budget.
const UNUSED_DROP: Duration = Duration::from_secs(30);
const MAX_TILES: usize = 320;
/// A missing tile (404: no region there) is asked again after this long.
const MISSING_RETRY: Duration = Duration::from_secs(600);
const DISK_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

/// `scaleToLevel`: mip level for a zoom in pixels per region.
pub fn scale_to_level(scale: f32) -> u32 {
    if scale <= f32::MIN_POSITIVE {
        return MAP_LEVELS;
    }
    ((256.0 / scale).log2() + 1.0).floor().clamp(1.0, MAP_LEVELS as f32) as u32
}

/// `globalToMipmap`: tile corner (region units) holding a grid position.
pub fn tile_origin(gx: u32, gy: u32, level: u32) -> (u32, u32) {
    let n = 1u32 << (level - 1);
    (gx - gx % n, gy - gy % n)
}

enum Tile {
    Queued,
    Loading,
    Ready(egui::TextureHandle, Arc<egui::ColorImage>),
    Missing(Instant),
}

struct Picture {
    tex: egui::TextureHandle,
    sources: Vec<Arc<egui::ColorImage>>,
}

struct Entry {
    tile: Tile,
    used: Instant,
}

type Key = (u32, u32, u32);

pub struct MapTiles {
    base_url: String,
    rt: Option<tokio::runtime::Handle>,
    http: Option<reqwest::Client>,
    tiles: HashMap<Key, Entry>,
    /// One stitched picture per region origin and cell count.
    pictures: HashMap<(u32, u32, u32, u32), Picture>,
    queue: VecDeque<Key>,
    in_flight: usize,
    tx: crossbeam_channel::Sender<(Key, Option<egui::ColorImage>)>,
    rx: crossbeam_channel::Receiver<(Key, Option<egui::ColorImage>)>,
    disk: PathBuf,
}

impl MapTiles {
    pub fn new() -> MapTiles {
        let (tx, rx) = crossbeam_channel::unbounded();
        MapTiles {
            base_url: String::new(),
            rt: None,
            http: None,
            tiles: HashMap::new(),
            pictures: HashMap::new(),
            queue: VecDeque::new(),
            in_flight: 0,
            tx,
            rx,
            disk: crate::settings::cache_dir().join("maptiles"),
        }
    }

    /// Map server of the grid (login "map-server-url", else MapServerURL).
    /// Empty: no tiles (offline demo).
    pub fn configure(&mut self, base_url: &str, rt: &tokio::runtime::Handle, http: reqwest::Client) {
        let mut url = base_url.trim().to_owned();
        if !url.is_empty() && !url.ends_with('/') {
            url.push('/');
        }
        if url == self.base_url && self.rt.is_some() {
            return;
        }
        if url != self.base_url {
            self.tiles.clear();
            self.pictures.clear();
            self.queue.clear();
            self.base_url = url;
        }
        self.rt = Some(rt.clone());
        self.http = Some(http);
    }

    pub fn enabled(&self) -> bool {
        !self.base_url.is_empty() && self.rt.is_some()
    }

    /// Tile texture at a level and tile corner (region units), queued for
    /// download when `load` and unknown (LLWorldMipmap::getObjectsTile).
    /// `Err(true)` = settled without an image (missing), `Err(false)` = pending.
    pub fn get(&mut self, level: u32, x: u32, y: u32, load: bool) -> Result<egui::TextureHandle, bool> {
        self.touch(level, x, y, load).map(|(tex, _)| tex)
    }

    /// Level-1 tiles of one region, north up, as a single texture.
    /// `LLViewerRegion::getWorldMapTiles` (llviewerregion.cpp) asks for
    /// `map-1-(gridX+x)-(gridY+y)` on every 256 m cell, and `LLNetMap` draws
    /// each cell in place with the higher grid Y further north
    /// (llnetmap.cpp). A 256 m region is that one tile. The Legacy Viewer
    /// squares the cell count; both extents are kept here so a region that
    /// is wider than it is deep is not cropped, and the pin uses each side.
    pub fn region_picture(
        &mut self,
        ctx: &egui::Context,
        gx: u32,
        gy: u32,
        cells_x: u32,
        cells_y: u32,
    ) -> Result<egui::TextureHandle, bool> {
        let cells_x = cells_x.max(1);
        let cells_y = cells_y.max(1);
        if cells_x == 1 && cells_y == 1 {
            return self.get(1, gx, gy, true);
        }
        let mut images = Vec::new();
        let mut pending = false;
        for dy in 0..cells_y {
            for dx in 0..cells_x {
                match self.touch(1, gx + dx, gy + dy, true) {
                    Ok((_, image)) => images.push(((dx, dy), image)),
                    Err(false) => pending = true,
                    Err(true) => {}
                }
            }
        }
        if pending {
            return Err(false);
        }
        if images.is_empty() {
            return Err(true);
        }
        let key = (gx, gy, cells_x, cells_y);
        if let Some(cached) = self.pictures.get(&key)
            && cached.sources.len() == images.len()
            && cached
                .sources
                .iter()
                .zip(&images)
                .all(|(kept, (_, image))| Arc::ptr_eq(kept, image))
        {
            return Ok(cached.tex.clone());
        }
        let refs: Vec<_> = images.iter().map(|(at, image)| (*at, image.as_ref())).collect();
        let Some(composed) = compose_region(cells_x, cells_y, &refs) else {
            return Err(true);
        };
        let tex = ctx.load_texture(
            format!("map-region-{gx}-{gy}-{cells_x}x{cells_y}"),
            composed,
            egui::TextureOptions::LINEAR,
        );
        let sources = images.into_iter().map(|(_, image)| image).collect();
        self.pictures.insert(key, Picture { tex: tex.clone(), sources });
        Ok(tex)
    }

    fn touch(&mut self, level: u32, x: u32, y: u32, load: bool) -> Result<(egui::TextureHandle, Arc<egui::ColorImage>), bool> {
        let now = Instant::now();
        let key = (level, x, y);
        if let Some(e) = self.tiles.get_mut(&key) {
            e.used = now;
            return match &e.tile {
                Tile::Ready(tex, image) => Ok((tex.clone(), Arc::clone(image))),
                Tile::Missing(at) if load && now.duration_since(*at) > MISSING_RETRY => {
                    e.tile = Tile::Queued;
                    self.queue.push_back(key);
                    Err(false)
                }
                Tile::Missing(_) => Err(true),
                _ => Err(false),
            };
        }
        if !load || !self.enabled() {
            return Err(!self.enabled());
        }
        self.tiles.insert(
            key,
            Entry {
                tile: Tile::Queued,
                used: now,
            },
        );
        self.queue.push_back(key);
        Err(false)
    }

    /// Receive downloads, start queued ones, drop unused tiles. Once a frame.
    pub fn update(&mut self, ctx: &egui::Context) {
        while let Ok((key, img)) = self.rx.try_recv() {
            self.in_flight = self.in_flight.saturating_sub(1);
            let Some(e) = self.tiles.get_mut(&key) else {
                continue;
            };
            e.tile = match img {
                Some(ci) => {
                    let image = Arc::new(ci);
                    let tex = ctx.load_texture(
                        format!("map-{}-{}-{}", key.0, key.1, key.2),
                        (*image).clone(),
                        egui::TextureOptions::LINEAR,
                    );
                    Tile::Ready(tex, image)
                }
                None => Tile::Missing(Instant::now()),
            };
        }
        let now = Instant::now();
        // newest requests first: what is on screen now matters most
        while self.in_flight < MAX_IN_FLIGHT {
            let Some(key) = self.queue.pop_back() else {
                break;
            };
            let Some(e) = self.tiles.get_mut(&key) else {
                continue;
            };
            if !matches!(e.tile, Tile::Queued) {
                continue;
            }
            if now.duration_since(e.used) > Duration::from_secs(2) {
                // scrolled away before its turn
                self.tiles.remove(&key);
                continue;
            }
            e.tile = Tile::Loading;
            self.start(key);
        }
        if self.tiles.len() > MAX_TILES {
            self.tiles
                .retain(|_, e| matches!(e.tile, Tile::Loading) || now.duration_since(e.used) < UNUSED_DROP);
        }
    }

    fn start(&mut self, key: Key) {
        let (Some(rt), Some(http)) = (self.rt.clone(), self.http.clone()) else {
            return;
        };
        self.in_flight += 1;
        let name = format!("map-{}-{}-{}-objects.jpg", key.0, key.1, key.2);
        let url = format!("{}{}", self.base_url, name);
        let path = self.disk.join(format!("{:08x}", fxhash(&self.base_url))).join(&name);
        let tx = self.tx.clone();
        rt.spawn(async move {
            let fresh = std::fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| SystemTime::now().duration_since(t).ok())
                .is_some_and(|age| age < DISK_MAX_AGE);
            let mut bytes = if fresh { std::fs::read(&path).ok() } else { None };
            if bytes.is_none()
                && let Ok(r) = http.get(&url).timeout(Duration::from_secs(20)).send().await
                && r.status().is_success()
                && let Ok(b) = r.bytes().await
            {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&path, &b);
                bytes = Some(b.to_vec());
            }
            let img = bytes.and_then(|b| {
                let img = image::load_from_memory_with_format(&b, image::ImageFormat::Jpeg).ok()?.to_rgba8();
                let (w, h) = img.dimensions();
                Some(egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw()))
            });
            let _ = tx.send((key, img));
        });
    }
}

fn fxhash(s: &str) -> u32 {
    s.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ b as u32).wrapping_mul(0x0100_0193))
}

/// How many 256 m map cells a region covers. Smaller than one cell still
/// has one tile (`LLViewerRegion::getWorldMapTiles`).
pub fn region_cells(size_x: u32, size_y: u32) -> (u32, u32) {
    ((size_x / 256).max(1), (size_y / 256).max(1))
}

/// Arrival point on the region picture, 0..1 from the south-west corner.
/// Each side uses its own length.
pub fn region_pin(x: f32, y: f32, size_x: f32, size_y: f32) -> (f32, f32) {
    (x / size_x.max(1.0), y / size_y.max(1.0))
}

/// North-up composite. `tiles` are `(dx, dy)` with `dy` 0 at the south edge.
/// Image row 0 is the north row, so `(gx, gy + cells_y - 1)` is the top-left
/// cell, as `LLNetMap` draws the higher grid Y further north.
pub fn compose_region(cells_x: u32, cells_y: u32, tiles: &[((u32, u32), &egui::ColorImage)]) -> Option<egui::ColorImage> {
    if cells_x == 0 || cells_y == 0 || tiles.is_empty() {
        return None;
    }
    let cell_w = tiles.iter().map(|(_, image)| image.size[0]).max()?;
    let cell_h = tiles.iter().map(|(_, image)| image.size[1]).max()?;
    if cell_w == 0 || cell_h == 0 {
        return None;
    }
    let width = cell_w * cells_x as usize;
    let height = cell_h * cells_y as usize;
    // theme-guard: content — a missing map cell, painted into the picture
    let gap = egui::Color32::from_rgb(32, 48, 64);
    let mut pixels = vec![gap; width * height];
    for ((dx, dy), image) in tiles {
        if *dx >= cells_x || *dy >= cells_y {
            continue;
        }
        let row0 = (cells_y as usize - 1 - *dy as usize) * cell_h;
        let col0 = *dx as usize * cell_w;
        let copy_w = image.size[0].min(cell_w);
        let copy_h = image.size[1].min(cell_h);
        for y in 0..copy_h {
            for x in 0..copy_w {
                pixels[(row0 + y) * width + col0 + x] = image.pixels[y * image.size[0] + x];
            }
        }
    }
    Some(egui::ColorImage::new([width, height], pixels))
}

#[cfg(test)]
mod tests {
    use super::*;

    // tests/llworldmipmap_test.cpp
    #[test]
    fn levels() {
        assert_eq!(scale_to_level(0.0), 8);
        assert_eq!(scale_to_level(1.0), 8);
        assert_eq!(scale_to_level(10.0), 5);
        assert_eq!(scale_to_level(64.0), 3);
        assert_eq!(scale_to_level(65.0), 2);
        assert_eq!(scale_to_level(128.0), 2);
        assert_eq!(scale_to_level(129.0), 1);
        assert_eq!(scale_to_level(256.0), 1);
        assert_eq!(scale_to_level(1000.0), 1);
    }

    fn queued(tiles: &MapTiles) -> Vec<Key> {
        let mut keys: Vec<_> = tiles.tiles.keys().copied().collect();
        keys.sort();
        keys
    }

    #[test]
    fn a_var_region_queues_every_level_one_tile() {
        let mut tiles = MapTiles::new();
        let rt = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        tiles.configure("http://map.example/", rt.handle(), reqwest::Client::new());
        let _ = tiles.region_picture(&egui::Context::default(), 1000, 2000, 2, 2);
        assert_eq!(
            queued(&tiles),
            vec![(1, 1000, 2000), (1, 1000, 2001), (1, 1001, 2000), (1, 1001, 2001)]
        );
    }

    #[test]
    fn a_var_region_picture_puts_the_north_tile_at_the_top() {
        let sw = egui::ColorImage::filled([1, 1], egui::Color32::from_rgb(10, 0, 0));
        let se = egui::ColorImage::filled([1, 1], egui::Color32::from_rgb(0, 10, 0));
        let nw = egui::ColorImage::filled([1, 1], egui::Color32::from_rgb(0, 0, 10));
        let ne = egui::ColorImage::filled([1, 1], egui::Color32::from_rgb(10, 10, 10));
        let image = compose_region(2, 2, &[((0, 0), &sw), ((1, 0), &se), ((0, 1), &nw), ((1, 1), &ne)]).expect("picture");
        assert_eq!(image.size, [2, 2]);
        assert_eq!(image.pixels[0], nw.pixels[0], "(gx, gy+1) is the top-left");
        assert_eq!(image.pixels[1], ne.pixels[0]);
        assert_eq!(image.pixels[2], sw.pixels[0]);
        assert_eq!(image.pixels[3], se.pixels[0]);
    }

    #[test]
    fn a_region_pin_uses_each_extent() {
        let (x, y) = region_pin(400.0, 100.0, 512.0, 512.0);
        assert!((x - 0.78).abs() < 0.01, "{x}");
        assert!((y - 0.20).abs() < 0.01, "{y}");
        let (x, y) = region_pin(400.0, 100.0, 512.0, 256.0);
        assert!((x - 400.0 / 512.0).abs() < 1e-4);
        assert!((y - 100.0 / 256.0).abs() < 1e-4);
    }

    #[test]
    fn a_standard_region_still_asks_for_one_tile() {
        let mut tiles = MapTiles::new();
        let rt = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        tiles.configure("http://map.example/", rt.handle(), reqwest::Client::new());
        let _ = tiles.region_picture(&egui::Context::default(), 1000, 2000, 1, 1);
        assert_eq!(queued(&tiles), vec![(1, 1000, 2000)]);
    }

    #[test]
    fn origins() {
        assert_eq!(tile_origin(1000, 1000, 1), (1000, 1000));
        assert_eq!(tile_origin(1001, 1003, 2), (1000, 1002));
        assert_eq!(tile_origin(1001, 1003, 3), (1000, 1000));
        assert_eq!(tile_origin(1015, 1009, 4), (1008, 1008));
    }
}
