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
    Ready(egui::TextureHandle),
    Missing(Instant),
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
        let now = Instant::now();
        let key = (level, x, y);
        if let Some(e) = self.tiles.get_mut(&key) {
            e.used = now;
            return match &e.tile {
                Tile::Ready(t) => Ok(t.clone()),
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
                Some(ci) => Tile::Ready(ctx.load_texture(format!("map-{}-{}-{}", key.0, key.1, key.2), ci, egui::TextureOptions::LINEAR)),
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

    #[test]
    fn origins() {
        assert_eq!(tile_origin(1000, 1000, 1), (1000, 1000));
        assert_eq!(tile_origin(1001, 1003, 2), (1000, 1002));
        assert_eq!(tile_origin(1001, 1003, 3), (1000, 1000));
        assert_eq!(tile_origin(1015, 1009, 4), (1008, 1008));
    }
}
