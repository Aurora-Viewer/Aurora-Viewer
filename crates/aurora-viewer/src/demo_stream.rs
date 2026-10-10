//! AURORA_DEMO_STREAM=<n>: streaming hitch test. As when arriving in a busy
//! region or turning the camera toward a full shop, `n` textured objects
//! keep arriving in waves (100 every 250 ms, ~400 a second as measured on
//! Agni), each with its own texture and its own shape. Each texture is
//! decoded on the background pool at a quarter of its size right after
//! its object arrives, then at full size 1.5 s later (a discard upgrade),
//! and goes through the real path from there: finished job, upload queue,
//! texture pages. The sizes are those of a grid region (mostly 512 and
//! 1024, a few small ones, one not a power of two), so the uploads weigh
//! what they weigh on the grid. The log tells when everything is loaded
//! (`demo stream: fully loaded`), to check that smoothing the uploads does
//! not slow loading down.

use super::{HANDLE, height, prim, shape, te};
use crate::scene::jobs::{AlphaKind, JobResult, Jobs};
use crate::scene::textures::{TexSource, TextureStreamer};
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use aurora_render::Renderer;
use glam::{Quat, Vec3};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Objects per wave and time between waves.
const WAVE: u32 = 100;
const WAVE_EVERY: Duration = Duration::from_millis(250);
/// First wave once the loading fade is over.
const FIRST_FRAME: u64 = 300;
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

/// Object count from the variable (2000 for a value that is not a number).
pub fn count() -> Option<u32> {
    let v = std::env::var("AURORA_DEMO_STREAM").ok()?;
    Some(v.trim().parse().unwrap_or(2000).clamp(1, 20_000))
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

/// Decode of texture `i` at a discard level, on the background pool: it
/// comes back as a finished job, like a JPEG 2000 decode.
fn spawn_decode(jobs: &Jobs, i: u32, discard: u8) {
    jobs.spawn(move || JobResult::Texture {
        id: texture(i),
        discard,
        mips: texture_mips(i, discard as u32),
        alpha: AlphaKind::Opaque,
        alpha_channel: false,
        sculpt: None,
    });
}

/// The scenario, driven once a frame.
pub struct StreamDemo {
    n: u32,
    next: u32,
    started: Option<Instant>,
    last_wave: Option<Instant>,
    upgrades: VecDeque<(Instant, u32)>,
    loaded_logged: bool,
}

impl StreamDemo {
    pub fn from_env() -> Option<Self> {
        Some(StreamDemo {
            n: count()?,
            next: 0,
            started: None,
            last_wave: None,
            upgrades: VecDeque::new(),
            loaded_logged: false,
        })
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
        if frame < FIRST_FRAME || self.loaded_logged {
            return;
        }
        let now = Instant::now();
        let started = *self.started.get_or_insert(now);
        if self.next < self.n && self.last_wave.is_none_or(|t| now.duration_since(t) >= WAVE_EVERY) {
            self.last_wave = Some(now);
            let count = WAVE.min(self.n - self.next);
            // the textures are referenced before their objects are synced
            // (never evicted: the scenario holds them)
            for i in self.next..self.next + count {
                let _ = textures.acquire(renderer, texture(i), TexSource::Asset);
                spawn_decode(jobs, i, 2);
                self.upgrades.push_back((now + UPGRADE_AFTER, i));
            }
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
        while self.upgrades.front().is_some_and(|(due, _)| *due <= now) {
            if let Some((_, i)) = self.upgrades.pop_front() {
                spawn_decode(jobs, i, 0);
            }
        }
        if self.next == self.n && self.upgrades.is_empty() && frame.is_multiple_of(10) {
            let full = (0..self.n).filter(|&i| textures.decoded_level(&texture(i)) == Some(0)).count();
            if full == self.n as usize && geom_pending == 0 {
                self.loaded_logged = true;
                log::info!(
                    "demo stream: fully loaded in {:.2} s ({} textures at full resolution, geometry built)",
                    now.duration_since(started).as_secs_f32(),
                    self.n
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
