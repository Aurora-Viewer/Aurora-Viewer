//! AURORA_DEMO_SLIDESHOW=1[,<seconds>][,net=<ms>][,mbps=<m>][,side=<px>][,count=<n>][,crowd=<n>][,preload]:
//! a scripted slideshow without a grid. A panel stands 4 m ahead of the camera,
//! and every `seconds` (3 by default) its texture entry changes to the next
//! of `count` pictures (6), as a script calling llSetTexture makes the
//! simulator send a full update of the prim. The pictures are served by the
//! local asset server (demo_asset_server.rs) after `net` milliseconds (150),
//! at `side` pixels (1024), so that they follow the whole download path:
//! byte ranges, a first level then the finer ones. With `preload`, the
//! panel's back face wears the next picture, as most slideshow scripts do.
//! `crowd` boxes beyond it, each with its own picture, compete for the
//! downloads as the textures of a busy region do, on a link of `mbps`
//! megabits a second if given.
//! The log tells, at each change, whether the picture that leaves was shown
//! and, once a second, a picture still not shown after two seconds.

use super::{HANDLE, height, prim, shape, te};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec3};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};
use uuid::Uuid;

const LOCAL_ID: u32 = 81_400;
/// The panel arrives once the loading screen is gone.
const ARRIVE_FRAME: u64 = 300;
/// A picture not shown after this long is logged, once a second.
const LATE: Duration = Duration::from_secs(2);
/// The panel's face turned away from the start view (+X of a box).
const BACK_FACE: usize = 2;
/// Index of the first picture of the boxes around (`crowd=`), each its
/// own asset, served as one of the show's files.
const CROWD_FIRST: u32 = 1000;

/// The picture `i` of the show (apart from the demo's other fixed ids).
pub fn picture(i: u32) -> Uuid {
    Uuid::from_u128(0xde7e_5115_0000_4000_8000_0000_0000_0000 | i as u128)
}

fn index_of(id: &Uuid) -> Option<u32> {
    let v = id.as_u128();
    (v & !0xFFFF == 0xde7e_5115_0000_4000_8000_0000_0000_0000).then_some((v & 0xFFFF) as u32)
}

struct Options {
    interval: Duration,
    delay: Duration,
    side: u32,
    count: u32,
    preload: bool,
    /// Boxes around, each with its own picture to download, as the
    /// textures of a busy region the show's compete with.
    crowd: u32,
    /// The link's throughput, in megabits a second (None: no limit).
    mbps: Option<f64>,
}

impl Options {
    fn parse(value: &str) -> Options {
        let mut o = Options {
            interval: Duration::from_secs(3),
            delay: Duration::from_millis(150),
            side: 1024,
            count: 6,
            preload: false,
            crowd: 0,
            mbps: None,
        };
        for part in value.split(',').map(str::trim) {
            if let Some(ms) = part.strip_prefix("net=").and_then(|v| v.parse::<u64>().ok()) {
                o.delay = Duration::from_millis(ms);
            } else if let Some(px) = part.strip_prefix("side=").and_then(|v| v.parse::<u32>().ok()) {
                o.side = px.clamp(64, 2048).next_power_of_two();
            } else if let Some(n) = part.strip_prefix("count=").and_then(|v| v.parse::<u32>().ok()) {
                o.count = n.clamp(2, 32);
            } else if let Some(n) = part.strip_prefix("crowd=").and_then(|v| v.parse::<u32>().ok()) {
                o.crowd = n.min(2000);
            } else if let Some(m) = part.strip_prefix("mbps=").and_then(|v| v.parse::<f64>().ok()) {
                o.mbps = Some(m).filter(|m| *m > 0.0);
            } else if part == "preload" {
                o.preload = true;
            } else if let Ok(s) = part.parse::<f32>()
                && part != "1"
                && s > 0.0
            {
                o.interval = Duration::from_secs_f32(s);
            }
        }
        o
    }
}

pub struct SlideshowDemo {
    options: Options,
    /// The encoded pictures, from their thread.
    ready: Option<Receiver<Option<String>>>,
    /// The local asset server's url, once the pictures are encoded.
    url: Option<String>,
    shown: Option<(u32, Instant)>,
    late_logged: Option<Instant>,
    changes: u32,
    /// Where the panel stands (centre) and the camera's horizontal
    /// direction when it came.
    place: Option<(Vec3, Vec3)>,
}

impl SlideshowDemo {
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("AURORA_DEMO_SLIDESHOW").ok()?;
        Some(SlideshowDemo {
            options: Options::parse(&value),
            ready: None,
            url: None,
            shown: None,
            late_logged: None,
            changes: 0,
            place: None,
        })
    }

    /// Once a frame.
    /// Once a frame, with the camera's position and direction.
    pub fn tick(
        &mut self,
        frame: u64,
        world: &mut crate::world::World,
        textures: &mut crate::scene::textures::TextureStreamer,
        eye: Vec3,
        forward: Vec3,
    ) {
        if self.ready.is_none() && self.url.is_none() {
            // (the files a previous run left in the disk cache: each
            // picture is downloaded, as one met for the first time)
            for i in 0..self.options.count {
                textures.refresh(&picture(i));
            }
            for k in 0..self.options.crowd {
                textures.refresh(&picture(CROWD_FIRST + k));
            }
            self.ready = Some(self.encode());
            log::info!(
                "demo slideshow: {} pictures of {} px, one every {:.1} s, served after {} ms{}{}, {} boxes around",
                self.options.count,
                self.options.side,
                self.options.interval.as_secs_f32(),
                self.options.delay.as_millis(),
                self.options.mbps.map(|m| format!(" at {m} Mbit/s")).unwrap_or_default(),
                if self.options.preload {
                    ", the next one on the back face"
                } else {
                    ""
                },
                self.options.crowd,
            );
        }
        if let Some(rx) = &self.ready {
            match rx.try_recv() {
                Ok(url) => {
                    self.ready = None;
                    match url {
                        Some(url) => {
                            let mut caps = world.main().map(|r| (*r.caps).clone()).unwrap_or_default();
                            caps.insert("ViewerAsset".to_owned(), url.clone());
                            world.apply(NetEvent::Capabilities {
                                handle: HANDLE,
                                caps: Arc::new(caps),
                            });
                            self.url = Some(url);
                        }
                        None => self.url = Some(String::new()),
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.ready = None;
                    self.url = Some(String::new());
                }
            }
        }
        if frame < ARRIVE_FRAME || self.url.as_deref().is_none_or(str::is_empty) {
            return;
        }
        let now = Instant::now();
        let next = match self.shown {
            None => Some(0),
            Some((i, since)) if now.duration_since(since) >= self.options.interval => {
                let id = picture(i);
                log::info!(
                    "demo slideshow: picture {i} leaves after {:.1} s, {}: {}",
                    now.duration_since(since).as_secs_f32(),
                    if textures.is_loaded(&id) { "shown" } else { "NEVER SHOWN" },
                    textures.describe(&id)
                );
                Some((i + 1) % self.options.count)
            }
            Some((i, since)) => {
                let id = picture(i);
                if now.duration_since(since) >= LATE
                    && !textures.is_loaded(&id)
                    && self.late_logged.is_none_or(|t| now.duration_since(t) >= Duration::from_secs(1))
                {
                    self.late_logged = Some(now);
                    log::warn!(
                        "demo slideshow: picture {i} still not shown after {:.1} s: {}",
                        now.duration_since(since).as_secs_f32(),
                        textures.describe(&id)
                    );
                }
                None
            }
        };
        if let Some(i) = next {
            self.shown = Some((i, now));
            self.late_logged = None;
            self.changes += 1;
            let preload = self.options.preload.then(|| (i + 1) % self.options.count);
            let ahead = Vec3::new(forward.x, forward.y, 0.0).try_normalize().unwrap_or(Vec3::X);
            let (centre, ahead) = *self.place.get_or_insert((eye + ahead * 4.0, ahead));
            world.apply(panel(centre, ahead, i, preload));
            if self.changes == 1 && self.options.crowd > 0 {
                world.apply(crowd(centre, ahead, self.options.crowd));
            }
            log::info!("demo slideshow: change {} shows picture {i}", self.changes);
        }
    }

    /// The pictures, encoded off the frame, then the local asset server
    /// that holds them; None when that failed.
    fn encode(&self) -> Receiver<Option<String>> {
        let (tx, rx) = std::sync::mpsc::channel();
        let (count, side, delay, mbps) = (self.options.count, self.options.side, self.options.delay, self.options.mbps);
        let spawned = std::thread::Builder::new().name("demo-slideshow".into()).spawn(move || {
            let dir = std::env::temp_dir().join("aurora-demo-slideshow");
            let _ = std::fs::create_dir_all(&dir);
            let mut files: Vec<PathBuf> = Vec::new();
            for i in 0..count {
                let rgba = picture_pixels(i, side);
                let Ok(bytes) = aurora_assets::j2k::encode_texture_j2k(side, side, 4, &rgba, false) else {
                    log::warn!("demo slideshow: picture {i} could not be encoded");
                    let _ = tx.send(None);
                    return;
                };
                let path = dir.join(format!("picture-{side}-{i}.j2c"));
                if let Err(e) = std::fs::write(&path, &bytes) {
                    log::warn!("demo slideshow: picture {i} not written ({e})");
                    let _ = tx.send(None);
                    return;
                }
                files.push(path);
            }
            let files = Arc::new(files);
            let resolve: super::asset_server::Resolve = Arc::new(move |kind, id| {
                let i = index_of(id).filter(|_| kind == "texture")?;
                files.get(i as usize % files.len().max(1)).cloned()
            });
            let url = super::asset_server::start_assets(resolve, super::asset_server::Link::new(delay, mbps))
                .map_err(|e| log::warn!("demo slideshow: no local asset server ({e})"))
                .ok();
            let _ = tx.send(url);
        });
        if let Err(e) = spawned {
            log::warn!("demo slideshow: pictures not encoded ({e})");
        }
        rx
    }
}

/// A thin square panel at `centre`, its face turned to the camera (which
/// looks along `ahead`), showing picture `shown`, and `preload` on its
/// back face.
fn panel(centre: Vec3, ahead: Vec3, shown: u32, preload: Option<u32>) -> NetEvent {
    let mut t = (*te([1.0; 4], 0, true, 0.0)).clone();
    for (fi, f) in t.faces.iter_mut().enumerate() {
        f.texture = match preload {
            Some(p) if fi == BACK_FACE => picture(p),
            _ => picture(shown),
        };
    }
    let object = prim(
        LOCAL_ID,
        centre,
        facing(ahead),
        Vec3::new(0.05, 1.6, 1.6),
        shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
        Arc::new(t),
        ExtraParams::default(),
        "",
    );
    NetEvent::ObjectUpdates {
        handle: HANDLE,
        objects: vec![object],
    }
}

/// `n` boxes in rows beyond the panel, each wearing its own picture.
fn crowd(centre: Vec3, ahead: Vec3, n: u32) -> NetEvent {
    let side = Vec3::new(-ahead.y, ahead.x, 0.0);
    let objects = (0..n)
        .map(|k| {
            let (col, row) = ((k % 25) as f32, (k / 25) as f32);
            let at = centre + ahead * (4.0 + row * 0.8) + side * ((col - 12.0) * 0.8);
            let mut t = (*te([1.0; 4], 0, true, 0.0)).clone();
            for f in &mut t.faces {
                f.texture = picture(CROWD_FIRST + k);
            }
            prim(
                LOCAL_ID + 1 + k,
                Vec3::new(at.x, at.y, height(at.x, at.y).max(at.z - 2.5) + 0.5 + (k % 3) as f32 * 0.7),
                facing(ahead),
                Vec3::splat(0.6),
                shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
                Arc::new(t),
                ExtraParams::default(),
                "",
            )
        })
        .collect();
    NetEvent::ObjectUpdates { handle: HANDLE, objects }
}

/// The rotation that turns a prim's +X along `ahead` (horizontal).
fn facing(ahead: Vec3) -> Quat {
    Quat::from_rotation_z(ahead.y.atan2(ahead.x))
}

/// Picture `i`: its own hue, `i + 1` white bars and a frame, so that each
/// one is told apart on a screenshot.
fn picture_pixels(i: u32, side: u32) -> Vec<u8> {
    const HUES: [[u8; 3]; 8] = [
        [139, 92, 246],
        [94, 234, 212],
        [244, 114, 182],
        [250, 204, 21],
        [96, 165, 250],
        [74, 222, 128],
        [251, 146, 60],
        [248, 113, 113],
    ];
    let base = HUES[i as usize % HUES.len()];
    let bars = i + 1;
    let cell = side / (2 * bars + 1);
    let border = side / 32;
    let mut px = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        for x in 0..side {
            let frame = x < border || y < border || x >= side - border || y >= side - border;
            let col = x / cell.max(1);
            let bar = col % 2 == 1 && col < 2 * bars && y > side / 4 && y < side * 3 / 4;
            let c = if frame || bar { [255, 255, 255] } else { base };
            // (grain, as in a photo: the file is the size of a real
            // picture's, downloaded in several ranges)
            let mut h = (x as u64) << 32 | y as u64 | (i as u64) << 56;
            h = h.wrapping_mul(0x9E37_79B9_7F4A_7C15);
            h ^= h >> 29;
            let grain = (h % 81) as i32 - 40;
            let g = |v: u8| (v as i32 + grain).clamp(0, 255) as u8;
            px.extend_from_slice(&[g(c[0]), g(c[1]), g(c[2]), 255]);
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_read_the_interval_the_link_and_the_preload() {
        let o = Options::parse("1");
        assert_eq!(
            (o.interval, o.delay, o.side, o.count, o.preload),
            (Duration::from_secs(3), Duration::from_millis(150), 1024, 6, false)
        );
        let o = Options::parse("1,2.5,net=400,side=500,count=3,preload,crowd=200,mbps=20");
        assert_eq!(o.interval, Duration::from_secs_f32(2.5));
        assert_eq!((o.delay, o.side, o.count, o.preload), (Duration::from_millis(400), 512, 3, true));
        assert_eq!((o.crowd, o.mbps), (200, Some(20.0)));
    }

    #[test]
    fn the_server_finds_each_picture_by_its_id() {
        assert_eq!(index_of(&picture(0)), Some(0));
        assert_eq!(index_of(&picture(5)), Some(5));
        assert_eq!(index_of(&Uuid::from_u128(5)), None);
    }
}
