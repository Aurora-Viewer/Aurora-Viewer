//! AURORA_DEMO_ARRIVAL=<folder>[,net=<ms>][,n=<objects>][,turn=<s>]: an
//! arrival in a busy region, to measure what the loading shows first.
//!
//! `n` objects (1,200 by default) arrive all around our avatar, the nearest
//! first and about 600 a second as a simulator sends them, from 3 to 120 m
//! away and larger with the distance; one in three is a mesh, each has its
//! own texture. Sixteen avatars stand nearby, each playing two animations.
//! Every asset is downloaded from the local asset server
//! (demo_asset_server.rs, on this machine only), which answers each request
//! after `net` milliseconds (200 by default): the textures (`.j2c`), meshes
//! (`.mesh`, static ones) and animations (`.anim`) are the files of
//! `<folder>`'s `tex`, `mesh` and `anim` subfolders, under ids proper to the
//! run, so no disk cache holds them. `turn` seconds after the first objects
//! (3 by default) the avatar turns round: what was behind is now in front.
//!
//! The log tells when half, nine tenths and all of what is in front of the
//! camera after the turn showed (its geometry built and its texture with its
//! first pixels), weighted by size on screen (`demo arrival: front`),
//! the same for everything around (`demo arrival: all`), when the
//! animations arrived (`demo arrival: animations`), then what the server
//! sent (`demo arrival: done`).

use super::{HANDLE, height, prim, shape, te};
use crate::scene::Scene;
use crate::world::World;
use aurora_net::NetEvent;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use glam::{Quat, Vec2, Vec3};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Objects sent every `WAVE_EVERY`: about what a simulator sends on an
/// arrival when it is asked for 4,000 kb/s (network audit, item 11).
const WAVE: usize = 150;
const WAVE_EVERY: Duration = Duration::from_millis(250);
/// The arrival starts once the loading fade is over.
const FIRST_FRAME: u64 = 300;
const FIRST_AFTER: Duration = Duration::from_secs(4);
const AVATARS: u32 = 16;
const ANIMS_EACH: u32 = 2;
/// Distinct meshes, cycled over the mesh objects.
const MESHES: usize = 400;
/// Local ids of the objects and of the avatars.
const FIRST_LOCAL_ID: u32 = 120_000;
const FIRST_AVATAR_ID: u32 = 125_000;
/// Half the angle of what counts as in front after the turn.
const FRONT_COS: f32 = 0.707;
/// Shares the log tells, in tenths.
const STEPS: [u32; 3] = [5, 9, 10];

const IDS: u128 = 0xDEA2_21A0_0000_0000_0000_0000_0000_0000;
const KIND_TEXTURE: u32 = 1;
const KIND_MESH: u32 = 2;
const KIND_ANIM: u32 = 3;
const KIND_AVATAR: u32 = 4;

/// Part of the ids proper to this run: never in a disk cache.
static RUN: AtomicU32 = AtomicU32::new(0);

fn id(kind: u32, i: u32) -> Uuid {
    Uuid::from_u128(IDS | (RUN.load(Ordering::Relaxed) as u128) << 32 | (kind as u128) << 24 | (i & 0xFF_FFFF) as u128)
}

/// Kind and number of one of the run's ids.
fn parse_id(id: &Uuid) -> Option<(u32, u32)> {
    let n = id.as_u128();
    let run = IDS | (RUN.load(Ordering::Relaxed) as u128) << 32;
    (n & !(u32::MAX as u128) == run).then_some(((n >> 24) as u32 & 0xFF, n as u32 & 0xFF_FFFF))
}

/// The files the server hands out.
struct Files {
    textures: Vec<PathBuf>,
    meshes: Vec<PathBuf>,
    anims: Vec<PathBuf>,
}

/// Files of `dir` with extension `ext` and a size within `bytes`, in name
/// order, at most `most` of them that `keep` accepts.
fn list_files(dir: &Path, ext: &str, bytes: std::ops::Range<u64>, most: usize, keep: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut all: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| log::warn!("demo arrival: {}: {e}", dir.display()))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == ext))
        .collect();
    all.sort();
    all.into_iter()
        .filter(|p| std::fs::metadata(p).is_ok_and(|m| bytes.contains(&m.len())))
        .filter(|p| keep(p))
        .take(most)
        .collect()
}

/// A mesh drawn as it is, without a skeleton.
fn static_mesh(path: &Path) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|d| aurora_assets::parse_mesh_header(&d).ok())
        .is_some_and(|h| h.skin.is_none() && h.lods.iter().any(Option::is_some))
}

struct Placed {
    pos: Vec3,
    size: f32,
    texture: Uuid,
    mesh: Option<Uuid>,
    /// Its object's id (`prim`'s).
    full_id: Uuid,
}

/// Bytes and requests the server answered, per kind.
#[derive(Default)]
struct Served {
    requests: [AtomicU32; 3],
    bytes: [AtomicU64; 3],
}

struct Run {
    objects: Vec<Placed>,
    order: Vec<usize>,
    anims: Vec<Uuid>,
    sent: usize,
    started: Instant,
    last_wave: Option<Instant>,
    start: Vec3,
    front: Vec2,
    turned: bool,
    told: [usize; 3],
    done: bool,
    served: Arc<Served>,
}

pub struct ArrivalDemo {
    delay: Duration,
    n: u32,
    turn: Duration,
    files: Arc<OnceLock<Files>>,
    armed: Option<Instant>,
    run: Option<Run>,
}

fn option<T: std::str::FromStr>(value: &str, name: &str) -> Option<T> {
    value
        .split(',')
        .find_map(|p| p.trim().strip_prefix(name)?.strip_prefix('=')?.trim().parse().ok())
}

impl ArrivalDemo {
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("AURORA_DEMO_ARRIVAL").ok()?;
        let folder = PathBuf::from(value.split(',').next()?.trim());
        let n = option::<u32>(&value, "n").unwrap_or(1200).clamp(10, 20_000);
        let nanos = std::time::UNIX_EPOCH.elapsed().map_or(1, |t| t.subsec_nanos());
        RUN.store(nanos.max(1), Ordering::Relaxed);
        // (the files are looked over off the main thread, during the login fade)
        let files = Arc::new(OnceLock::new());
        let ready = files.clone();
        let objects = n as usize;
        let spawned = std::thread::Builder::new().name("demo-arrival-files".into()).spawn(move || {
            let textures = list_files(&folder.join("tex"), "j2c", 4096..8 << 20, objects, |_| true);
            let meshes = list_files(&folder.join("mesh"), "mesh", 4096..512 << 10, MESHES, static_mesh);
            let anims = list_files(
                &folder.join("anim"),
                "anim",
                512..256 << 10,
                (AVATARS * ANIMS_EACH) as usize,
                |_| true,
            );
            log::info!(
                "demo arrival: {} textures, {} static meshes, {} animations to serve",
                textures.len(),
                meshes.len(),
                anims.len()
            );
            let _ = ready.set(Files { textures, meshes, anims });
        });
        if let Err(e) = spawned {
            log::warn!("demo arrival: files not looked over ({e})");
            return None;
        }
        Some(ArrivalDemo {
            delay: Duration::from_millis(option(&value, "net").unwrap_or(200)),
            n,
            turn: Duration::from_secs_f32(option::<f32>(&value, "turn").unwrap_or(3.0).max(0.0)),
            files,
            armed: None,
            run: None,
        })
    }

    /// Start the server, place the objects and the avatars.
    fn begin(&mut self, world: &mut World) -> Option<Run> {
        let files = self.files.get()?;
        if files.textures.is_empty() || files.meshes.is_empty() || files.anims.is_empty() {
            log::warn!("demo arrival: textures, meshes or animations missing from the folder");
            return None;
        }
        let served = Arc::new(Served::default());
        let resolve: super::asset_server::Resolve = {
            let files = self.files.clone();
            let served = served.clone();
            Arc::new(move |kind, id| {
                let files = files.get()?;
                let (k, i) = parse_id(id)?;
                let (list, slot) = match (kind, k) {
                    ("texture", KIND_TEXTURE) => (&files.textures, 0),
                    ("mesh", KIND_MESH) => (&files.meshes, 1),
                    ("animatn", KIND_ANIM) => (&files.anims, 2),
                    _ => return None,
                };
                let path = list.get(i as usize % list.len())?.clone();
                served.requests[slot].fetch_add(1, Ordering::Relaxed);
                // (a texture is asked for by ranges: its bytes are in `texture load:`)
                if slot > 0 {
                    served.bytes[slot].fetch_add(std::fs::metadata(&path).map_or(0, |m| m.len()), Ordering::Relaxed);
                }
                Some(path)
            })
        };
        let url = match super::asset_server::start_assets(resolve, self.delay) {
            Ok(url) => url,
            Err(e) => {
                log::warn!("demo arrival: no local asset server ({e})");
                return None;
            }
        };
        let mut caps = world.main().map(|r| (*r.caps).clone()).unwrap_or_default();
        caps.insert("ViewerAsset".to_owned(), url);
        world.apply(NetEvent::Capabilities {
            handle: HANDLE,
            caps: Arc::new(caps),
        });
        let start = world.agent.position;
        let yaw = world.agent.yaw + std::f32::consts::PI;
        let n = self.n as usize;
        let objects: Vec<Placed> = (0..n)
            .map(|i| {
                // a disc around the avatar, denser near it (golden angle)
                let u = (i as f32 + 0.5) / n as f32;
                let d = 3.0 + 117.0 * u.powf(1.6);
                let a = i as f32 * 2.399_963;
                let size = 0.6 + d * 0.05 + (i % 4) as f32 * 0.4;
                let (x, y) = ((start.x + a.cos() * d).clamp(2.0, 254.0), (start.y + a.sin() * d).clamp(2.0, 254.0));
                Placed {
                    pos: Vec3::new(x, y, height(x, y) + size * 0.5),
                    size,
                    texture: id(KIND_TEXTURE, i as u32),
                    full_id: Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | (FIRST_LOCAL_ID + i as u32) as u128),
                    mesh: (i % 3 == 0).then(|| id(KIND_MESH, (i / 3) as u32)),
                }
            })
            .collect();
        // the nearest first, as a simulator's interest list sends them
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|&a, &b| {
            let d = |i: usize| objects[i].pos.truncate().distance(start.truncate());
            d(a).total_cmp(&d(b))
        });
        // the avatars and their animations, with the first objects
        let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
        let mut avatars = Vec::new();
        let mut anims = Vec::new();
        let mut played = Vec::new();
        for k in 0..AVATARS {
            let a = k as f32 * std::f32::consts::TAU / AVATARS as f32;
            let d = 5.0 + (k % 4) as f32 * 5.0;
            let (x, y) = (start.x + a.cos() * d, start.y + a.sin() * d);
            let mut o = prim(
                FIRST_AVATAR_ID + k,
                Vec3::new(x, y, height(x, y) + 0.85),
                Quat::from_rotation_z(a + std::f32::consts::PI),
                Vec3::new(0.45, 0.6, 1.9),
                boxp,
                te([0.2, 0.2, 0.25, 1.0], 0, false, 0.0),
                ExtraParams::default(),
                "",
            );
            o.pcode = LL_PCODE_LEGACY_AVATAR;
            o.full_id = id(KIND_AVATAR, k);
            o.name_values = format!("FirstName STRING RW SV Arrival\nLastName STRING RW SV {k}");
            avatars.push(o);
            let list: Vec<(Uuid, i32)> = (0..ANIMS_EACH).map(|j| (id(KIND_ANIM, k * ANIMS_EACH + j), j as i32 + 1)).collect();
            anims.extend(list.iter().map(|(a, _)| *a));
            played.push(NetEvent::AvatarAnimations {
                sources: Vec::new(),
                avatar: id(KIND_AVATAR, k),
                anims: list,
            });
        }
        world.apply(NetEvent::ObjectUpdates {
            handle: HANDLE,
            objects: avatars,
        });
        for ev in played {
            world.apply(ev);
        }
        let meshes = objects.iter().filter(|o| o.mesh.is_some()).count();
        log::info!(
            "demo arrival: {} objects ({} meshes), {} animations on {} avatars, {} ms a request, turn after {:.1} s",
            n,
            meshes,
            anims.len(),
            AVATARS,
            self.delay.as_millis(),
            self.turn.as_secs_f32()
        );
        Some(Run {
            objects,
            order,
            anims,
            sent: 0,
            started: Instant::now(),
            last_wave: None,
            start,
            front: Vec2::new(yaw.cos(), yaw.sin()),
            turned: false,
            told: [0; 3],
            done: false,
            served,
        })
    }

    pub fn tick(&mut self, frame: u64, world: &mut World, scene: &Scene) {
        let armed = *self.armed.get_or_insert_with(Instant::now);
        if frame < FIRST_FRAME || armed.elapsed() < FIRST_AFTER {
            return;
        }
        if self.run.is_none() {
            if self.files.get().is_none() {
                return;
            }
            self.run = self.begin(world);
            if self.run.is_none() {
                // (said in the log; not tried again)
                self.armed = Some(Instant::now() + Duration::from_secs(1 << 20));
                return;
            }
        }
        let turn = self.turn;
        let Some(run) = self.run.as_mut() else {
            return;
        };
        if run.done {
            return;
        }
        let now = Instant::now();
        let elapsed = now.duration_since(run.started).as_secs_f32();
        if run.sent < run.order.len() && run.last_wave.is_none_or(|t| now.duration_since(t) >= WAVE_EVERY) {
            run.last_wave = Some(now);
            let end = (run.sent + WAVE).min(run.order.len());
            let objects = run.order[run.sent..end]
                .iter()
                .map(|&i| {
                    let p = &run.objects[i];
                    let mut t = (*te([1.0; 4], 0, false, 0.0)).clone();
                    for f in &mut t.faces {
                        f.texture = p.texture;
                    }
                    let (volume, extra) = match p.mesh {
                        Some(mesh) => {
                            let sculpt = SculptParams {
                                texture: mesh,
                                sculpt_type: LL_SCULPT_TYPE_MESH,
                            };
                            (
                                aurora_prim::VolumeParams {
                                    sculpt: Some(sculpt),
                                    ..Default::default()
                                },
                                ExtraParams {
                                    sculpt: Some(sculpt),
                                    ..Default::default()
                                },
                            )
                        }
                        None => (
                            shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
                            ExtraParams::default(),
                        ),
                    };
                    prim(
                        FIRST_LOCAL_ID + i as u32,
                        p.pos,
                        Quat::IDENTITY,
                        Vec3::splat(p.size),
                        volume,
                        Arc::new(t),
                        extra,
                        "",
                    )
                })
                .collect();
            world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
            run.sent = end;
        }
        if !run.turned && elapsed >= turn.as_secs_f32() {
            run.turned = true;
            world.agent.yaw += std::f32::consts::PI;
            log::info!("demo arrival: turned round after {elapsed:.2} s");
        }
        if !frame.is_multiple_of(10) {
            return;
        }
        // shares shown of what is in front, of everything, of the animations
        let (mut front, mut front_all, mut all, mut all_all) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for p in &run.objects[..] {
            let to = p.pos.truncate() - run.start.truncate();
            let d = to.length().max(1.0);
            let w = p.size / d;
            // (its texture shows its first pixels, and its geometry is built)
            let built = world
                .objects
                .index_of_uuid(&p.full_id)
                .and_then(|i| scene.gpu.get(i))
                .is_some_and(|g| g.geom.is_some());
            let shown = built && scene.textures.decoded_level(&p.texture).is_some();
            let ahead = to.normalize_or_zero().dot(run.front) > FRONT_COS;
            all_all += w;
            all += if shown { w } else { 0.0 };
            if ahead {
                front_all += w;
                front += if shown { w } else { 0.0 };
            }
        }
        let anims = run.anims.iter().filter(|a| scene.anims.asset_settled(a)).count() as f32;
        let shares = [
            ("front", front / front_all.max(1e-6)),
            ("all", all / all_all.max(1e-6)),
            ("animations", anims / run.anims.len().max(1) as f32),
        ];
        for (k, (name, share)) in shares.iter().enumerate() {
            while run.told[k] < STEPS.len() && *share * 10.0 >= STEPS[run.told[k]] as f32 - 1e-4 {
                log::info!("demo arrival: {name} {}0% shown after {elapsed:.2} s", STEPS[run.told[k]]);
                run.told[k] += 1;
            }
        }
        if run.told.iter().all(|t| *t == STEPS.len()) && run.sent == run.order.len() {
            run.done = true;
            let s = &run.served;
            let mb = |slot: usize| s.bytes[slot].load(Ordering::Relaxed) as f32 / 1e6;
            log::info!(
                "demo arrival: done after {elapsed:.2} s; served {} texture requests, {} mesh requests ({:.1} MB), {} animation requests ({:.2} MB)",
                s.requests[0].load(Ordering::Relaxed),
                s.requests[1].load(Ordering::Relaxed),
                mb(1),
                s.requests[2].load(Ordering::Relaxed),
                mb(2),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_tell_their_kind_and_number() {
        RUN.store(77, Ordering::Relaxed);
        assert_eq!(parse_id(&id(KIND_MESH, 41)), Some((KIND_MESH, 41)));
        assert_eq!(parse_id(&id(KIND_TEXTURE, 0xFF_FFFF)), Some((KIND_TEXTURE, 0xFF_FFFF)));
        assert_eq!(parse_id(&Uuid::from_u128(5)), None);
    }

    #[test]
    fn options_are_read() {
        let v = "C:/cache,net=150,n=600,turn=2.5";
        assert_eq!(option::<u32>(v, "net"), Some(150));
        assert_eq!(option::<u32>(v, "n"), Some(600));
        assert_eq!(option::<f32>(v, "turn"), Some(2.5));
        assert_eq!(option::<u32>(v, "wave"), None);
    }
}
