//! AURORA_DEMO_STREAM=<n>[,<wave>]: streaming hitch test. As when arriving
//! in a busy region or turning the camera toward a full shop, `n` textured
//! objects keep arriving in waves (`wave` every 250 ms; 100 by default,
//! ~400 a second as measured on Agni; `wave` = `n` sends everything at
//! once, as a teleport arrival), each with its own texture and its own
//! shape. Each texture is
//! decoded on the background pool at a quarter of its size right after
//! its object arrives, then at full size 1.5 s later (a discard upgrade),
//! and goes through the real path from there: finished job, upload queue,
//! texture pages. The sizes are those of a grid region (mostly 512 and
//! 1024, a few small ones, one not a power of two), so the uploads weigh
//! what they weigh on the grid. The log tells when everything is loaded
//! (`demo stream: fully loaded`), to check that smoothing the uploads does
//! not slow loading down.
//!
//! With `,leave` after the counts, the region is then left as by a
//! teleport: a second after the loading, every object is removed at once
//! and its texture, now holding downloaded data of the size of its J2C
//! file, is no longer used; two seconds later they are all due for
//! eviction (`demo stream: textures evicted`).

use super::{HANDLE, height, prim, shape, te};
use crate::scene::jobs::{AlphaKind, Jobs};
use crate::scene::textures::{TexSource, TextureStreamer, texture_result};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use aurora_render::{Renderer, StagingPool};
use glam::{Quat, Vec3};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Default objects per wave, and time between waves.
const WAVE: u32 = 100;
const WAVE_EVERY: Duration = Duration::from_millis(250);
/// First wave once the loading fade is over: not before this frame, nor
/// before this long after the scenario was first driven (at several
/// hundred frames a second, frame 300 comes while the loading screen still
/// covers the view).
const FIRST_FRAME: u64 = 300;
const FIRST_AFTER: Duration = Duration::from_secs(4);
/// Full resolution this long after the quarter-size level.
const UPGRADE_AFTER: Duration = Duration::from_millis(1500);
/// Local ids of the objects (apart from the other demo objects).
const FIRST_LOCAL_ID: u32 = 80_000;

/// Level-0 sizes, cycled: what a busy grid region holds.
const SIZES: [(u32, u32); 8] = [
    (1024, 1024),
    (512, 512),
    (512, 512),
    (256, 256),
    (1024, 512),
    (512, 256),
    (128, 128),
    (384, 384),
];

/// The region is left this long after it is loaded (`,leave`), and its
/// textures are evicted this long after that.
const LEAVE_AFTER: Duration = Duration::from_secs(1);
const EVICT_AFTER: Duration = Duration::from_secs(2);

/// Object count, objects per wave and whether the region is left at the
/// end, from the variable's value (2000 objects for a count that is not a
/// number, 100 per wave by default).
fn parse(value: &str) -> (u32, u32, bool) {
    let leave = value.split(',').any(|p| p.trim() == "leave");
    let mut parts = value
        .split(',')
        .filter(|p| p.trim() != "leave")
        .map(|p| p.trim().parse::<u32>().ok());
    let n = parts.next().flatten().unwrap_or(2000).clamp(1, 20_000);
    let wave = parts.next().flatten().unwrap_or(WAVE).clamp(1, n);
    (n, wave, leave)
}

/// Bytes of downloaded data held by texture `i` once loaded: about what
/// its J2C file weighs on the grid (2 bits a pixel).
fn data_len(i: u32) -> usize {
    let (w, h) = SIZES[i as usize % SIZES.len()];
    (w * h / 4) as usize
}

pub fn texture(i: u32) -> Uuid {
    Uuid::from_u128(0xDE57_4EA0_0000_0000_0000_0000_0000_0000 | i as u128)
}

/// Mip chain of texture `i` at a discard level, down to 1×1: an 8 × 8
/// checker of a color proper to the texture and its inverse, drawn at each
/// level (the same picture at every level and discard, so a level shown in
/// the wrong place, or a missing one, is visible).
pub fn texture_mips(i: u32, discard: u32) -> Vec<(u32, u32, Vec<u8>)> {
    let (w, h) = SIZES[i as usize % SIZES.len()];
    let base = [(i * 73 % 256) as u8, (i * 151 % 256) as u8, (i * 29 % 256) as u8];
    let inverse = base.map(|v| 255 - v);
    let mut out = Vec::new();
    let mut level = discard;
    loop {
        let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
        let mut px = Vec::with_capacity((lw * lh * 4) as usize);
        for y in 0..lh {
            let row = y * 8 / lh;
            for x in 0..lw {
                let c = if (x * 8 / lw + row) % 2 == 0 { base } else { inverse };
                px.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        out.push((lw, lh, px));
        if lw == 1 && lh == 1 {
            break;
        }
        level += 1;
    }
    out
}

/// Objects `first..first + count` of `n`: a square field east of the plaza,
/// facing the start position. Shapes cycle through cylinders, spheres and
/// tori with a hollow proper to each object, so each one has its own
/// geometry to build and upload, as the varied prims of a shop.
fn wave_events(n: u32, first: u32, count: u32) -> Vec<NetEvent> {
    let side = (n as f32).sqrt().ceil().max(1.0) as u32;
    let objects = (first..first + count)
        .map(|i| {
            let (x, y) = (
                146.0 + (i / side) as f32 * 0.8,
                126.0 + ((i % side) as f32 - side as f32 * 0.5) * 0.8,
            );
            let hollow = (i * 37 % 450) as u16 * 100;
            let volume = match i % 3 {
                0 => shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, hollow, 0),
                1 => shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, hollow, 0),
                _ => shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE, 50, hollow, 0),
            };
            let mut t = (*te([1.0; 4], 0, false, 0.0)).clone();
            for f in &mut t.faces {
                f.texture = texture(i);
            }
            prim(
                FIRST_LOCAL_ID + i,
                Vec3::new(x, y, height(x, y) + 0.25),
                Quat::IDENTITY,
                Vec3::splat(0.5),
                volume,
                Arc::new(t),
                ExtraParams::default(),
                "",
            )
        })
        .collect();
    vec![NetEvent::ObjectUpdates { handle: HANDLE, objects }]
}

/// Decodes of textures at a discard level, on the background pool: they
/// come back as finished jobs, staged like JPEG 2000 decodes.
fn spawn_decodes(jobs: &Jobs, pool: Option<Arc<StagingPool>>, textures: Vec<u32>, discard: u8) {
    jobs.spawn_many(textures, move |i| {
        texture_result(
            texture(i),
            discard,
            texture_mips(i, discard as u32),
            (AlphaKind::Opaque, false),
            None,
            pool.as_deref(),
        )
    });
}

/// The scenario, driven once a frame.
pub struct StreamDemo {
    n: u32,
    wave: u32,
    next: u32,
    /// First call of `tick`.
    armed: Option<Instant>,
    started: Option<Instant>,
    last_wave: Option<Instant>,
    upgrades: VecDeque<(Instant, u32)>,
    loaded_logged: bool,
    /// `,leave`: the region is left once loaded.
    leave: bool,
    loaded_at: Option<Instant>,
    /// Textures given their downloaded data so far.
    with_data: u32,
    left_at: Option<Instant>,
    evicted_logged: bool,
}

impl StreamDemo {
    pub fn from_env() -> Option<Self> {
        let (n, wave, leave) = parse(&std::env::var("AURORA_DEMO_STREAM").ok()?);
        Some(StreamDemo {
            n,
            wave,
            next: 0,
            armed: None,
            started: None,
            last_wave: None,
            upgrades: VecDeque::new(),
            loaded_logged: false,
            leave,
            loaded_at: None,
            with_data: 0,
            left_at: None,
            evicted_logged: false,
        })
    }

    /// `,leave`: the objects removed and their textures released a second
    /// after the loading, then the wait for their eviction.
    fn leave_region(&mut self, frame: u64, world: &mut crate::world::World, textures: &mut TextureStreamer) {
        let now = Instant::now();
        let Some(loaded_at) = self.loaded_at else {
            return;
        };
        let Some(left_at) = self.left_at else {
            // what a downloaded texture keeps in memory, a few per frame
            // (so that the scenario's own work does not show as a hitch)
            for i in self.with_data..(self.with_data + 64).min(self.n) {
                textures.demo_attach_data(&texture(i), vec![0x5A; data_len(i)]);
            }
            self.with_data = (self.with_data + 64).min(self.n);
            if self.with_data < self.n || now.duration_since(loaded_at) < LEAVE_AFTER {
                return;
            }
            self.left_at = Some(now);
            textures.evict_after = EVICT_AFTER;
            for i in 0..self.n {
                textures.release(&texture(i));
            }
            world.apply(NetEvent::ObjectsKilled {
                handle: HANDLE,
                local_ids: (0..self.n).map(|i| FIRST_LOCAL_ID + i).collect(),
            });
            log::info!("demo stream: region left ({} objects removed, their textures unused)", self.n);
            return;
        };
        if !self.evicted_logged && frame.is_multiple_of(10) && !(0..self.n).any(|i| textures.is_wanted(&texture(i))) {
            self.evicted_logged = true;
            log::info!(
                "demo stream: textures evicted {:.2} s after the region was left ({} textures)",
                now.duration_since(left_at).as_secs_f32(),
                self.n
            );
        }
    }

    pub fn tick(
        &mut self,
        frame: u64,
        world: &mut crate::world::World,
        textures: &mut TextureStreamer,
        renderer: &mut Renderer,
        jobs: &Jobs,
        geom_pending: usize,
    ) {
        let armed = *self.armed.get_or_insert_with(Instant::now);
        if frame < FIRST_FRAME || armed.elapsed() < FIRST_AFTER {
            return;
        }
        if self.loaded_logged {
            if self.leave {
                self.leave_region(frame, world, textures);
            }
            return;
        }
        let now = Instant::now();
        let started = *self.started.get_or_insert(now);
        if self.next < self.n && self.last_wave.is_none_or(|t| now.duration_since(t) >= WAVE_EVERY) {
            self.last_wave = Some(now);
            let count = self.wave.min(self.n - self.next);
            // the textures are referenced before their objects are synced
            // (never evicted: the scenario holds them)
            for i in self.next..self.next + count {
                let _ = textures.acquire(renderer, texture(i), TexSource::Asset);
                self.upgrades.push_back((now + UPGRADE_AFTER, i));
            }
            spawn_decodes(jobs, textures.staging.clone(), (self.next..self.next + count).collect(), 2);
            for ev in wave_events(self.n, self.next, count) {
                world.apply(ev);
            }
            self.next += count;
            if self.next == self.n {
                log::info!(
                    "demo stream: all {} objects sent in {:.1} s",
                    self.n,
                    now.duration_since(started).as_secs_f32()
                );
            }
        }
        let mut due = Vec::new();
        while self.upgrades.front().is_some_and(|(at, _)| *at <= now) {
            due.extend(self.upgrades.pop_front().map(|(_, i)| i));
        }
        spawn_decodes(jobs, textures.staging.clone(), due, 0);
        if self.next == self.n && self.upgrades.is_empty() && frame.is_multiple_of(10) {
            let full = (0..self.n).filter(|&i| textures.decoded_level(&texture(i)) == Some(0)).count();
            if full == self.n as usize && geom_pending == 0 {
                self.loaded_logged = true;
                self.loaded_at = Some(now);
                log::info!(
                    "demo stream: fully loaded in {:.2} s ({} textures at full resolution, geometry built; uploads {} staged / {} direct)",
                    now.duration_since(started).as_secs_f32(),
                    self.n,
                    textures.stats.uploads_staged,
                    textures.stats.uploads_direct,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mips_go_down_to_one_pixel_at_every_discard() {
        for i in 0..SIZES.len() as u32 {
            let (w, h) = SIZES[i as usize];
            for discard in [0, 2] {
                let mips = texture_mips(i, discard);
                assert_eq!((mips[0].0, mips[0].1), ((w >> discard).max(1), (h >> discard).max(1)));
                assert_eq!(mips.last().map(|m| (m.0, m.1)), Some((1, 1)));
                for (lw, lh, px) in &mips {
                    assert_eq!(px.len(), (lw * lh * 4) as usize);
                }
            }
        }
    }

    #[test]
    fn variable_gives_count_and_wave() {
        assert_eq!(parse("3000"), (3000, 100, false));
        assert_eq!(parse("1"), (1, 1, false));
        assert_eq!(parse("oui"), (2000, 100, false));
        assert_eq!(parse("500, 500"), (500, 500, false));
        // a wave larger than the count is the whole count
        assert_eq!(parse("50,9999"), (50, 50, false));
        // the region left at the end, wherever the word is
        assert_eq!(parse("3000,leave"), (3000, 100, true));
        assert_eq!(parse("3000, 3000, leave"), (3000, 3000, true));
        assert_eq!(parse("leave"), (2000, 100, true));
    }

    #[test]
    fn downloaded_data_weighs_like_a_j2c_file() {
        assert_eq!(data_len(0), 1024 * 1024 / 4);
        assert_eq!(data_len(6), 128 * 128 / 4);
    }

    #[test]
    fn waves_give_each_object_its_texture() {
        let events = wave_events(250, 100, 50);
        let [NetEvent::ObjectUpdates { objects, .. }] = events.as_slice() else {
            panic!("one update");
        };
        assert_eq!(objects.len(), 50);
        assert_eq!(objects[0].local_id, FIRST_LOCAL_ID + 100);
        assert!(
            objects[0]
                .texture_entry
                .as_ref()
                .is_some_and(|t| t.faces.iter().all(|f| f.texture == texture(100)))
        );
    }
}
