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

/// A tile's decoded pixels, kept for the region pictures. `stamp` tells
/// one load from the next, so a picture knows when to compose again.
#[derive(Clone)]
struct Pixels {
    image: Arc<egui::ColorImage>,
    stamp: u64,
}

enum Tile {
    Queued,
    Loading,
    /// The pixels are kept only for a tile a region picture asked for
    /// (`Entry::keep`): the world map's tiles need the texture alone.
    Ready(egui::TextureHandle, Option<Pixels>),
    /// Drawn before without its pixels, which a region picture now wants:
    /// the texture stays on the minimap and the world map while they are
    /// read again (disk cache first). `true` once the read started.
    Refetch(egui::TextureHandle, bool),
    Missing(Instant),
}

/// One region's stitched picture. It is composed on the rayon pool and
/// picked up by `update`; the texture shown meanwhile is the last one.
struct Picture {
    tex: Option<egui::TextureHandle>,
    /// The tiles' stamps `tex` was composed from.
    built: Vec<u64>,
    composing: bool,
    used: Instant,
}

/// Region origin and cell count of a picture.
type PictureKey = (u32, u32, u32, u32);
type Composed = (PictureKey, Vec<u64>, Option<egui::ColorImage>);

struct Entry {
    tile: Tile,
    used: Instant,
    /// A region picture stitches this tile: keep its pixels.
    keep: bool,
}

type Key = (u32, u32, u32);

pub struct MapTiles {
    base_url: String,
    rt: Option<tokio::runtime::Handle>,
    http: Option<reqwest::Client>,
    tiles: HashMap<Key, Entry>,
    /// One stitched picture per region origin and cell count.
    pictures: HashMap<PictureKey, Picture>,
    /// The next `Pixels::stamp`.
    stamp: u64,
    queue: VecDeque<Key>,
    in_flight: usize,
    tx: crossbeam_channel::Sender<(Key, Option<egui::ColorImage>)>,
    rx: crossbeam_channel::Receiver<(Key, Option<egui::ColorImage>)>,
    composed_tx: crossbeam_channel::Sender<Composed>,
    composed_rx: crossbeam_channel::Receiver<Composed>,
    disk: PathBuf,
}

impl MapTiles {
    pub fn new() -> MapTiles {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (composed_tx, composed_rx) = crossbeam_channel::unbounded();
        MapTiles {
            base_url: String::new(),
            rt: None,
            http: None,
            tiles: HashMap::new(),
            pictures: HashMap::new(),
            stamp: 0,
            queue: VecDeque::new(),
            in_flight: 0,
            tx,
            rx,
            composed_tx,
            composed_rx,
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
        self.touch(level, x, y, load, false).map(|(tex, _)| tex)
    }

    /// Level-1 tiles of one region, north up, as a single texture.
    /// `LLViewerRegion::getWorldMapTiles` (llviewerregion.cpp) asks for
    /// `map-1-(gridX+x)-(gridY+y)` on every 256 m cell, and `LLNetMap` draws
    /// each cell in place with the higher grid Y further north
    /// (llnetmap.cpp). A 256 m region is that one tile. The Legacy Viewer
    /// squares the cell count; both extents are kept here so a region that
    /// is wider than it is deep is not cropped, and the pin uses each side.
    pub fn region_picture(&mut self, gx: u32, gy: u32, cells_x: u32, cells_y: u32) -> Result<egui::TextureHandle, bool> {
        let cells_x = cells_x.max(1);
        let cells_y = cells_y.max(1);
        if cells_x == 1 && cells_y == 1 {
            return self.get(1, gx, gy, true);
        }
        let mut images = Vec::new();
        let mut pending = false;
        for dy in 0..cells_y {
            for dx in 0..cells_x {
                let key = (1, gx + dx, gy + dy);
                match self.touch(key.0, key.1, key.2, true, true) {
                    Ok((_, Some(pixels))) => images.push(((dx, dy), pixels)),
                    // shown on the world map before, without its pixels:
                    // read again and kept this time, its texture still drawn
                    Ok((_, None)) => {
                        if let Some(e) = self.tiles.get_mut(&key)
                            && let Tile::Ready(tex, None) = &e.tile
                        {
                            e.tile = Tile::Refetch(tex.clone(), false);
                            self.queue.push_back(key);
                        }
                        pending = true;
                    }
                    Err(false) => pending = true,
                    Err(true) => {}
                }
            }
        }
        let key = (gx, gy, cells_x, cells_y);
        if !pending && images.is_empty() {
            return Err(true);
        }
        let picture = self.pictures.entry(key).or_insert_with(|| Picture {
            tex: None,
            built: Vec::new(),
            composing: false,
            used: Instant::now(),
        });
        picture.used = Instant::now();
        let stamps: Vec<u64> = images.iter().map(|(_, pixels)| pixels.stamp).collect();
        if !pending && !picture.composing && picture.built != stamps {
            // up to 1024 x 1024 samples: off the frame, like the tiles' decoding
            picture.composing = true;
            let tx = self.composed_tx.clone();
            rayon::spawn(move || {
                let refs: Vec<_> = images.iter().map(|(at, pixels)| (*at, pixels.image.as_ref())).collect();
                let _ = tx.send((key, stamps, compose_region(cells_x, cells_y, &refs)));
            });
        }
        match &picture.tex {
            Some(tex) => Ok(tex.clone()),
            // composed without a picture (empty cells): settled
            None if !pending && !picture.composing && !picture.built.is_empty() => Err(true),
            None => Err(false),
        }
    }

    fn touch(&mut self, level: u32, x: u32, y: u32, load: bool, keep: bool) -> Result<(egui::TextureHandle, Option<Pixels>), bool> {
        let now = Instant::now();
        let key = (level, x, y);
        if let Some(e) = self.tiles.get_mut(&key) {
            e.used = now;
            e.keep |= keep;
            return match &e.tile {
                Tile::Ready(tex, pixels) => Ok((tex.clone(), pixels.clone())),
                Tile::Refetch(tex, _) => Ok((tex.clone(), None)),
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
                keep,
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
                    // one buffer for the texture upload and the kept pixels
                    let image = Arc::new(ci);
                    let pixels = e.keep.then(|| {
                        self.stamp += 1;
                        Pixels {
                            image: image.clone(),
                            stamp: self.stamp,
                        }
                    });
                    let tex = ctx.load_texture(format!("map-{}-{}-{}", key.0, key.1, key.2), image, egui::TextureOptions::LINEAR);
                    Tile::Ready(tex, pixels)
                }
                None => Tile::Missing(Instant::now()),
            };
        }
        while let Ok((key, stamps, image)) = self.composed_rx.try_recv() {
            // dropped meanwhile (unused, or another map server)
            let Some(picture) = self.pictures.get_mut(&key) else {
                continue;
            };
            picture.composing = false;
            picture.built = stamps;
            if let Some(image) = image {
                let (gx, gy, cells_x, cells_y) = key;
                picture.tex = Some(ctx.load_texture(
                    format!("map-region-{gx}-{gy}-{cells_x}x{cells_y}"),
                    image,
                    egui::TextureOptions::LINEAR,
                ));
            }
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
            if !matches!(e.tile, Tile::Queued | Tile::Refetch(_, false)) {
                continue;
            }
            let stale = now.duration_since(e.used) > Duration::from_secs(2);
            e.tile = match std::mem::replace(&mut e.tile, Tile::Loading) {
                // scrolled away before its turn: the texture alone stays
                Tile::Refetch(tex, _) if stale => Tile::Ready(tex, None),
                Tile::Refetch(tex, _) => Tile::Refetch(tex, true),
                _ if stale => {
                    self.tiles.remove(&key);
                    continue;
                }
                _ => Tile::Loading,
            };
            if !stale {
                self.start(key);
            }
        }
        if self.tiles.len() > MAX_TILES {
            self.tiles
                .retain(|_, e| matches!(e.tile, Tile::Loading | Tile::Refetch(_, true)) || now.duration_since(e.used) < UNUSED_DROP);
        }
        // a picture holds a texture up to MAX_PICTURE square: none is kept
        // once no window draws it
        self.pictures.retain(|_, picture| now.duration_since(picture.used) < UNUSED_DROP);
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

/// Longest side of a stitched region picture, in pixels.
const MAX_PICTURE: usize = 1024;

/// The cell's pixel at `(x, y)` of an `out_w` x `out_h` cell: a copy at
/// full size, else the average of its source footprint's four corners.
fn sample(image: &egui::ColorImage, x: usize, y: usize, out_w: usize, out_h: usize) -> egui::Color32 {
    let [src_w, src_h] = image.size;
    if out_w == src_w && out_h == src_h {
        return image.pixels[y * src_w + x];
    }
    let (x0, x1) = (
        x * src_w / out_w,
        ((x + 1) * src_w / out_w).saturating_sub(1).max(x * src_w / out_w),
    );
    let (y0, y1) = (
        y * src_h / out_h,
        ((y + 1) * src_h / out_h).saturating_sub(1).max(y * src_h / out_h),
    );
    let corners = [(x0, y0), (x1, y0), (x0, y1), (x1, y1)].map(|(sx, sy)| image.pixels[sy.min(src_h - 1) * src_w + sx.min(src_w - 1)]);
    let avg = |c: fn(&egui::Color32) -> u8| (corners.iter().map(|p| u32::from(c(p))).sum::<u32>() / 4) as u8;
    egui::Color32::from_rgba_premultiplied(avg(|p| p.r()), avg(|p| p.g()), avg(|p| p.b()), avg(|p| p.a()))
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
    // The windows show the picture a few hundred pixels wide: a large
    // region's cells are shrunk so the longest side stays within
    // MAX_PICTURE (a 4096 m region would otherwise be 4096 x 4096).
    let cells = cells_x.max(cells_y) as usize;
    let scale = (MAX_PICTURE / cells).clamp(1, cell_w.max(cell_h));
    let (out_w, out_h) = if cell_w.max(cell_h) * cells > MAX_PICTURE {
        (
            (cell_w * scale / cell_w.max(cell_h)).max(1),
            (cell_h * scale / cell_w.max(cell_h)).max(1),
        )
    } else {
        (cell_w, cell_h)
    };
    let width = out_w * cells_x as usize;
    let height = out_h * cells_y as usize;
    // theme-guard: content — a missing map cell, painted into the picture
    let gap = egui::Color32::from_rgb(32, 48, 64);
    let mut pixels = vec![gap; width * height];
    for ((dx, dy), image) in tiles {
        if *dx >= cells_x || *dy >= cells_y {
            continue;
        }
        let row0 = (cells_y as usize - 1 - *dy as usize) * out_h;
        let col0 = *dx as usize * out_w;
        let [src_w, src_h] = image.size;
        if src_w == 0 || src_h == 0 {
            continue;
        }
        for y in 0..out_h {
            for x in 0..out_w {
                pixels[(row0 + y) * width + col0 + x] = sample(image, x, y, out_w, out_h);
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
        let _ = tiles.region_picture(1000, 2000, 2, 2);
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
        let _ = tiles.region_picture(1000, 2000, 1, 1);
        assert_eq!(queued(&tiles), vec![(1, 1000, 2000)]);
    }

    /// A 4096 m region (16 x 16 cells of 256 px) is not stitched at
    /// 4096 x 4096: the longest side stays at MAX_PICTURE.
    #[test]
    fn a_large_region_picture_is_capped() {
        let cell = egui::ColorImage::filled([256, 256], egui::Color32::from_rgb(40, 80, 120));
        let cells: Vec<((u32, u32), &egui::ColorImage)> =
            (0..16).flat_map(|y| (0..16).map(move |x| (x, y))).map(|at| (at, &cell)).collect();
        let image = compose_region(16, 16, &cells).expect("picture");
        assert_eq!(image.size, [MAX_PICTURE, MAX_PICTURE]);
        assert_eq!(image.pixels[0], cell.pixels[0], "a uniform cell keeps its colour when shrunk");
        // a wide region keeps its shape: 16 x 4 cells
        let wide: Vec<_> = (0..4).flat_map(|y| (0..16).map(move |x| (x, y))).map(|at| (at, &cell)).collect();
        assert_eq!(compose_region(16, 4, &wide).expect("picture").size, [MAX_PICTURE, MAX_PICTURE / 4]);
        // a 512 m region stays at full size
        let small: Vec<_> = (0..2).flat_map(|y| (0..2).map(move |x| (x, y))).map(|at| (at, &cell)).collect();
        assert_eq!(compose_region(2, 2, &small).expect("picture").size, [512, 512]);
    }

    /// The world map's tiles keep the texture only; a tile a region picture
    /// asks for keeps its pixels.
    #[test]
    fn only_region_picture_tiles_keep_their_pixels() {
        let mut tiles = MapTiles::new();
        let rt = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        tiles.configure("http://map.example/", rt.handle(), reqwest::Client::new());
        let _ = tiles.get(1, 10, 10, true);
        let _ = tiles.region_picture(20, 20, 2, 1);
        assert!(!tiles.tiles[&(1, 10, 10)].keep);
        assert!(tiles.tiles[&(1, 20, 20)].keep && tiles.tiles[&(1, 21, 20)].keep);
    }

    fn configured() -> (MapTiles, tokio::runtime::Runtime) {
        let mut tiles = MapTiles::new();
        let rt = tokio::runtime::Builder::new_current_thread().build().expect("runtime");
        tiles.configure("http://map.example/", rt.handle(), reqwest::Client::new());
        (tiles, rt)
    }

    /// A downloaded tile, with its pixels when a region picture keeps them.
    fn ready(ctx: &egui::Context, tiles: &mut MapTiles, key: Key, keep: bool) -> egui::TextureHandle {
        let image = Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::from_rgb(key.1 as u8, 0, 0)));
        let tex = ctx.load_texture(format!("test-{key:?}"), image.clone(), egui::TextureOptions::LINEAR);
        tiles.stamp += 1;
        let pixels = keep.then_some(Pixels { image, stamp: tiles.stamp });
        tiles.tiles.insert(
            key,
            Entry {
                tile: Tile::Ready(tex.clone(), pixels),
                used: Instant::now(),
                keep,
            },
        );
        tex
    }

    /// Frames until the rayon pool's composite is picked up.
    fn composed(ctx: &egui::Context, tiles: &mut MapTiles, gx: u32, gy: u32, cells_x: u32, cells_y: u32) -> egui::TextureHandle {
        let start = Instant::now();
        loop {
            tiles.update(ctx);
            if let Ok(tex) = tiles.region_picture(gx, gy, cells_x, cells_y) {
                return tex;
            }
            assert!(start.elapsed() < Duration::from_secs(10), "the picture is never composed");
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// The picture is composed off the frame, kept while drawn, and its
    /// texture dropped once no window has drawn it for UNUSED_DROP.
    #[test]
    fn a_region_picture_is_composed_off_the_frame_and_dropped_once_unused() {
        let ctx = egui::Context::default();
        let (mut tiles, _rt) = configured();
        let _ = ready(&ctx, &mut tiles, (1, 20, 20), true);
        let _ = ready(&ctx, &mut tiles, (1, 21, 20), true);
        assert_eq!(tiles.region_picture(20, 20, 2, 1).map(|t| t.id()), Err(false), "composing");
        let key = (20, 20, 2, 1);
        assert!(tiles.pictures[&key].composing);
        let tex = composed(&ctx, &mut tiles, 20, 20, 2, 1);
        assert_eq!(tex.size(), [2, 1]);
        let again = tiles.region_picture(20, 20, 2, 1).expect("cached");
        assert_eq!(again.id(), tex.id());
        assert!(!tiles.pictures[&key].composing, "the same tiles are not composed again");
        let long_ago = Instant::now().checked_sub(UNUSED_DROP * 2).expect("uptime");
        tiles.pictures.get_mut(&key).expect("picture").used = long_ago;
        tiles.update(&ctx);
        assert!(tiles.pictures.is_empty(), "an unused picture is dropped");
    }

    /// A world map tile without its pixels keeps drawing its texture while
    /// they are read again, and a picture already made stays shown while a
    /// cell reloads.
    #[test]
    fn a_tile_read_again_for_its_pixels_keeps_its_texture() {
        let ctx = egui::Context::default();
        let (mut tiles, _rt) = configured();
        let shown = ready(&ctx, &mut tiles, (1, 20, 20), false);
        let _ = ready(&ctx, &mut tiles, (1, 21, 20), true);
        assert_eq!(tiles.region_picture(20, 20, 2, 1).map(|t| t.id()), Err(false));
        assert!(matches!(tiles.tiles[&(1, 20, 20)].tile, Tile::Refetch(_, false)));
        assert!(tiles.queue.contains(&(1, 20, 20)));
        assert_eq!(tiles.get(1, 20, 20, true).map(|t| t.id()), Ok(shown.id()), "still drawn");
        tiles.update(&ctx);
        assert!(matches!(tiles.tiles[&(1, 20, 20)].tile, Tile::Refetch(_, true)), "read started");
        assert_eq!(tiles.get(1, 20, 20, true).map(|t| t.id()), Ok(shown.id()));
        let pixels = egui::ColorImage::filled([1, 1], egui::Color32::from_rgb(20, 0, 0));
        tiles.tx.send(((1, 20, 20), Some(pixels))).expect("channel");
        let picture = composed(&ctx, &mut tiles, 20, 20, 2, 1);
        assert!(matches!(&tiles.tiles[&(1, 20, 20)].tile, Tile::Ready(_, Some(_))));
        // a cell reloading: the picture of before stays
        let tex = ready(&ctx, &mut tiles, (1, 21, 20), true);
        tiles.tiles.get_mut(&(1, 21, 20)).expect("tile").tile = Tile::Refetch(tex, true);
        assert_eq!(tiles.region_picture(20, 20, 2, 1).map(|t| t.id()), Ok(picture.id()));
    }

    #[test]
    fn origins() {
        assert_eq!(tile_origin(1000, 1000, 1), (1000, 1000));
        assert_eq!(tile_origin(1001, 1003, 2), (1000, 1002));
        assert_eq!(tile_origin(1001, 1003, 3), (1000, 1000));
        assert_eq!(tile_origin(1015, 1009, 4), (1008, 1008));
    }
}
