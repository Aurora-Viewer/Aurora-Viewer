//! AURORA_DEMO_REZ_EFFECT=1|loop|styles: the rez effect (scene/rez.rs) in front of the
//! camera (with AURORA_DEMO_POS=129,134, looking east). Frame-numbered
//! steps, for captures at AURORA_FPS_LIMIT=60:
//!
//! - when the loading screen lifts: our avatar, the line from its feet;
//! - frame 300: four prims arrive whose textures take 2.5 s to come, as on
//!   a slow link: each face is the grid until its texture comes, then
//!   fades into it;
//! - frame 840: someone else rezzes a four-prim table whose textures take
//!   a second: the grid, then one line for the linkset;
//! - frame 960: we rez a cube (FLAGS_CREATE_SELECTED, as from the build
//!   tools), ready at once: a quick line;
//! - frame 1020: we copy it (Shift-drag in the build tools): a quick line
//!   on the copy;
//! - frame 1080: an avatar comes into view: the grid while it loads (with
//!   AURORA_DEMO_CLOUD=1, instead of the cloud), 3 s later the line;
//! - frame 1380: someone rezzes thirty boxes at once: `MAX_SWEEPS` lines,
//!   the others just show;
//! - frame 1560: someone rezzes a small building (six prims, 3.6 m high)
//!   whose textures take 1.5 s: the grid, then a slower line;
//! - frame 1640: the avatar puts on a hat: it rezzes on its head;
//! - frame 1700: the table is deleted: its derez;
//! - frame 1760: the avatar takes the hat off: its derez;
//! - frame 1820: the avatar changes outfit (three items at once and new
//!   bakes, all coming 50 frames later): it stays as it is (its old bakes
//!   while the new ones download, never the grid), then one line;
//! - frame 1960: the avatar leaves: its derez;
//! - frame 2060: what the effect counted, in the log (`demo rez:`).
//!
//! `loop`: at frame 2160 everything the scenario put there goes at once (a
//! burst: no derez) and it starts again from frame 300, for watching it in
//! one window. With `AURORA_DEMO_TOD=4`, the same at midnight.
//!
//! `cold`: a cold arrival, again and again: from frame 300 and every
//! 10 s, 400 objects of all sizes arrive in front of the camera, near and
//! far (up to 60 m), in four waves; their textures come from 0.1 s to 4 s
//! later, one in eight at once (from the caches). With the effect on no
//! face is ever grey: each is the grid, then fades into its texture when
//! it comes, as soon as without the effect; the biggest on the screen get
//! a glint (the line hiding nothing) as soon as they are built. A wall
//! stands in the field's left half: nothing behind it gets a glint;
//! with Grey (Legacy) they are grey until their textures come, as before.
//! Each round's objects go at once when the next one starts (a burst: no
//! derez).
//!
//! `styles`: the three looks side by side, each with a look of its own
//! (`Rez::styles`) whatever the setting: a cube and, behind it, an avatar
//! wearing two items, under a label, in three columns. From frame 300 and
//! every 9 s, in step: they arrive (the grid traces in and waits while
//! their textures take 3 s, as the avatars' loading does), the line
//! reveals them, they stay, they leave (the derez) and stay away 2 s.
//!
//! The textures are the checkers of AURORA_DEMO_STREAM under ids of their
//! own (new ones at each round), decoded on the background pool when the
//! step says.

use super::{DEMO_NOVA, HANDLE, floor_at, prim, shape};
use crate::scene::Scene;
use crate::scene::jobs::{AlphaKind, Jobs};
use crate::scene::textures::{TexSource, TextureStreamer, texture_result};
use aurora_net::NetEvent;
use aurora_net::objects::ObjectUpdate;
use aurora_prim::ExtraParams;
use aurora_prim::params::*;
use aurora_render::Renderer;
use glam::{Quat, Vec3};
use std::sync::Arc;
use uuid::Uuid;

const LOADING: u64 = 300;
/// The loading prims' textures come this many frames later.
const LOADING_TEXTURES: u64 = 150;
const TABLE: u64 = 840;
const TABLE_TEXTURES: u64 = 60;
const OWN: u64 = 960;
const COPY: u64 = 1020;
const AVATAR: u64 = 1080;
const STORM: u64 = 1380;
const BUILDING: u64 = 1560;
const BUILDING_TEXTURES: u64 = 90;
const HAT_ON: u64 = 1640;
const TABLE_GONE: u64 = 1700;
const HAT_OFF: u64 = 1760;
const OUTFIT: u64 = 1820;
const OUTFIT_TEXTURES: u64 = 50;
const AVATAR_GONE: u64 = 1960;
const REPORT: u64 = 2060;
/// `loop`: the round ends here and the next starts at `LOADING`.
const ROUND_END: u64 = 2160;
const STORM_COUNT: u32 = 30;

/// Local ids of a round's objects, its avatar's, and the room each round
/// takes.
const FIRST_LOCAL_ID: u32 = 140_000;
const AVATAR_LOCAL_ID: u32 = 139_000;
const ROUND_IDS: u32 = 200;
/// Texture ids of the scenario (apart from AURORA_DEMO_STREAM's).
const TEXTURE_IDS: u128 = 0xDE57_4E2E_0000_0000_0000_0000_0000_0000;

/// Attachment point in an object's `state` (`Object::attachment_point`).
fn attach_state(point: u8) -> u8 {
    point.rotate_left(4)
}

/// `styles`: the columns' looks (`AURORA_REZ_STYLE`'s numbers).
const STYLES: [u32; 3] = [1, 2, 3];
/// `styles`: one round's length, when its textures come and when it leaves.
const STYLES_ROUND: u64 = 540;
const STYLES_TEXTURES: u64 = 180;
const STYLES_LEAVE: u64 = 420;
const STYLES_FIRST_ID: u32 = 145_000;

/// `cold`: objects of a round, its length, its waves and when their
/// textures come.
const COLD_OBJECTS: u32 = 400;
const COLD_ROUND: u64 = 600;
const COLD_WAVE: u32 = 100;
const COLD_WAVE_EVERY: u64 = 8;
const COLD_FIRST_ID: u32 = 150_000;
/// `cold`: the wall hiding part of the field.
const COLD_WALL_ID: u32 = 149_900;
/// The avatar's bakes before and after its outfit change (texture numbers).
const BAKES_BEFORE: u32 = 95;
const BAKES_AFTER: u32 = 96;

/// `cold`: frames after its object's arrival that texture `i` comes: one in
/// eight at once (from the caches), the others within 4 s.
fn cold_delay(i: u32) -> u64 {
    if i.is_multiple_of(8) { 2 } else { 6 + (i as u64 * 53) % 234 }
}

pub struct RezDemo {
    /// `styles`: the three looks side by side, again and again.
    styles: bool,
    /// `cold`: a cold arrival, again and again.
    cold: bool,
    /// `cold`: textures to decode, at their frame.
    due: Vec<(u64, u32)>,
    looping: bool,
    /// Round of the scenario (`loop`), for fresh ids.
    round: u32,
}

impl RezDemo {
    pub fn from_env() -> Option<Self> {
        let value = std::env::var("AURORA_DEMO_REZ_EFFECT").ok()?;
        log::info!("demo rez: scenario armed");
        Some(RezDemo {
            styles: value.trim() == "styles",
            cold: value.trim() == "cold",
            due: Vec::new(),
            looping: value.trim() == "loop",
            round: 0,
        })
    }

    fn id(&self, i: u32) -> u32 {
        FIRST_LOCAL_ID + self.round * ROUND_IDS + i
    }

    fn texture(&self, i: u32) -> Uuid {
        Uuid::from_u128(TEXTURE_IDS | (self.round * ROUND_IDS + i) as u128)
    }

    fn avatar_id(&self) -> u32 {
        AVATAR_LOCAL_ID + self.round
    }

    /// A prim of the scenario with texture `tex` on every face.
    fn textured(&self, i: u32, pos: Vec3, rot: Quat, scale: Vec3, volume: aurora_prim::VolumeParams, tex: u32) -> ObjectUpdate {
        let mut t = (*super::te([1.0; 4], 0, false, 0.0)).clone();
        for f in &mut t.faces {
            f.texture = self.texture(tex);
        }
        let mut o = prim(self.id(i), pos, rot, scale, volume, Arc::new(t), ExtraParams::default(), "");
        o.update_flags = 0;
        o
    }

    /// Texture `i` referenced now (before its object syncs) and, unless
    /// `later`, decoded at once.
    fn texture_now(&self, textures: &mut TextureStreamer, renderer: &mut Renderer, jobs: &Jobs, i: u32, later: bool) {
        let _ = textures.acquire(renderer, self.texture(i), TexSource::Asset);
        if !later {
            self.decode(textures, jobs, vec![i]);
        }
    }

    fn decode(&self, textures: &TextureStreamer, jobs: &Jobs, list: Vec<u32>) {
        let pool = textures.staging.clone();
        let ids: Vec<(Uuid, u32)> = list.into_iter().map(|i| (self.texture(i), i)).collect();
        jobs.spawn_many(ids, move |(id, i)| {
            texture_result(
                id,
                0,
                super::stream::texture_mips(i * 7 + 3, 0),
                (AlphaKind::Opaque, false),
                None,
                pool.as_deref(),
            )
        });
    }

    /// A worn prim of the avatar at attachment `point`.
    fn worn(&self, i: u32, point: u8, at: Vec3, scale: Vec3, volume: aurora_prim::VolumeParams, tex: u32) -> ObjectUpdate {
        let mut o = self.textured(i, at, Quat::IDENTITY, scale, volume, tex);
        o.parent_id = self.avatar_id();
        o.state = attach_state(point);
        o
    }

    /// The avatar's bakes (head, upper and lower body) on texture `tex`, as
    /// the appearance service sends them.
    fn bakes(&self, shapes: &crate::scene::shape::ShapeParams, avatar: uuid::Uuid, tex: u32) -> NetEvent {
        let mut t = (*super::te([1.0; 4], 0, false, 0.0)).clone();
        for (te, _, _) in crate::scene::avatar::BAKES.iter().take(3) {
            t.faces[*te].texture = self.texture(tex);
        }
        NetEvent::Appearance(aurora_net::AvatarAppearance {
            avatar_id: avatar,
            texture_entry: Some(Arc::new(t)),
            visual_params: shapes.values_with(&[]),
            cof_version: 1,
            appearance_version: 1,
            hover_height: 0.0,
            attachments: 0,
        })
    }

    fn avatar_uuid(&self) -> Uuid {
        Uuid::from_u128(0xA0E0_4E2E_0000_0000_0000_0000_0000_0001 + self.round as u128)
    }

    /// `styles`: column `k`'s cube, the avatar behind it as the camera
    /// sees it and the label above (region coordinates; the camera stands
    /// near `EYE`, behind our avatar at 129, 134, looking east). Left to
    /// right on the screen, between our avatar and the left edge.
    fn styles_places(k: usize) -> (Vec3, Vec3, Vec3) {
        const EYE: Vec3 = Vec3::new(126.3, 132.7, 0.0);
        let (x, y) = (132.0, 141.7 - 1.9 * k as f32);
        let cube = Vec3::new(x, y, floor_at(x, y) + 0.5);
        let a = Vec3::new(x, y, 0.0) + (Vec3::new(x, y, 0.0) - EYE).normalize() * 3.6;
        let avatar = Vec3::new(a.x, a.y, floor_at(a.x, a.y) + 0.95);
        (cube, avatar, cube + Vec3::Z * 0.95)
    }

    fn tick_styles(&mut self, frame: u64, world: &mut crate::world::World, scene: &mut Scene, renderer: &mut Renderer) {
        if frame < LOADING {
            return;
        }
        let t = (frame - LOADING) % STYLES_ROUND;
        let round = ((frame - LOADING) / STYLES_ROUND) as u32;
        let id = |i: u32| STYLES_FIRST_ID + i;
        let avatar_uuid = |k: usize| Uuid::from_u128(0xA0E0_4E2E_0000_0000_0000_0000_0001_0000 + k as u128);
        // (fresh textures each round: they load again, as on arriving)
        let tex = |k: u32| round * 16 + k;
        let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
        let cylinder = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0);
        let mut objects = Vec::new();
        if frame == LOADING {
            // the labels, which stay: hover text over an invisible prim
            for k in 0..STYLES.len() {
                let (_, _, label) = Self::styles_places(k);
                let mut o = prim(
                    id(100 + k as u32),
                    label,
                    Quat::IDENTITY,
                    Vec3::splat(0.05),
                    boxp,
                    super::te([1.0, 1.0, 1.0, 0.0], 0, false, 0.0),
                    ExtraParams::default(),
                    match k {
                        0 => aurora_i18n::tr!("demo-rez-style-grid"),
                        1 => aurora_i18n::tr!("demo-rez-style-scan"),
                        _ => aurora_i18n::tr!("demo-rez-style-wire"),
                    },
                );
                o.update_flags = 0;
                objects.push(o);
            }
            log::info!("demo rez: the three looks side by side, a round every {} frames", STYLES_ROUND);
        }
        match t {
            0 => {
                for (k, style) in STYLES.iter().enumerate() {
                    let k32 = k as u32;
                    let (cube_at, avatar_at, _) = Self::styles_places(k);
                    for i in 0..3 {
                        self.texture_now(&mut scene.textures, renderer, &scene.jobs, tex(k32 * 3 + i), true);
                    }
                    let mut cube = self.textured(0, cube_at, Quat::from_rotation_z(0.5), Vec3::splat(1.0), boxp, tex(k32 * 3));
                    cube.local_id = id(k32 * 10);
                    cube.full_id = Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | cube.local_id as u128);
                    cube.update_flags = crate::build::FLAGS_CREATE_SELECTED;
                    scene.rez.styles.insert(cube.full_id, *style);
                    objects.push(cube);
                    let mut a = prim(
                        id(k32 * 10 + 1),
                        avatar_at,
                        Quat::from_rotation_z(std::f32::consts::PI + 0.45),
                        Vec3::new(0.45, 0.6, 1.9),
                        boxp,
                        super::te([0.2, 0.2, 0.25, 1.0], 0, false, 0.0),
                        ExtraParams::default(),
                        "",
                    );
                    a.pcode = LL_PCODE_LEGACY_AVATAR;
                    a.full_id = avatar_uuid(k);
                    a.name_values = format!("FirstName STRING RW SV Style\nLastName STRING RW SV {style}");
                    scene.rez.styles.insert(a.full_id, *style);
                    objects.push(a);
                    // what it wears: a hat and a box on its chest
                    for (j, (point, at, size, volume)) in [
                        (2u8, Vec3::new(0.0, 0.0, 0.14), Vec3::new(0.32, 0.32, 0.18), cylinder),
                        (1, Vec3::new(0.12, 0.0, 0.0), Vec3::new(0.12, 0.42, 0.5), boxp),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let mut w = self.textured(0, at, Quat::IDENTITY, size, volume, tex(k32 * 3 + 1 + j as u32));
                        w.local_id = id(k32 * 10 + 2 + j as u32);
                        w.full_id = Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | w.local_id as u128);
                        w.parent_id = id(k32 * 10 + 1);
                        w.state = attach_state(point);
                        objects.push(w);
                    }
                }
            }
            STYLES_TEXTURES => {
                self.decode(&scene.textures, &scene.jobs, (0..9).map(tex).collect());
            }
            STYLES_LEAVE => {
                let local_ids = (0..STYLES.len() as u32).flat_map(|k| [id(k * 10), id(k * 10 + 1)]).collect();
                world.apply(NetEvent::ObjectsKilled { handle: HANDLE, local_ids });
            }
            _ => {}
        }
        if !objects.is_empty() {
            world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
        }
    }

    /// `cold`: a round of objects arriving with slow textures.
    fn tick_cold(&mut self, frame: u64, world: &mut crate::world::World, scene: &mut Scene, renderer: &mut Renderer) {
        if frame < LOADING {
            return;
        }
        let t = (frame - LOADING) % COLD_ROUND;
        let round = ((frame - LOADING) / COLD_ROUND) as u32;
        let id = |r: u32, i: u32| COLD_FIRST_ID + (r % 2) * COLD_OBJECTS + i;
        // (fresh textures each round, `texture` spacing rounds by 2,000
        // ids: they load again)
        self.round = round * 10;
        if t == 0 && round > 0 {
            let local_ids = (0..COLD_OBJECTS).map(|i| id(round - 1, i)).collect();
            world.apply(NetEvent::ObjectsKilled { handle: HANDLE, local_ids });
        }
        if frame == LOADING {
            // a wall across the field's left half (north of the camera's
            // axis), 16 m ahead: what stands behind it is never seen
            let (x, y) = (145.0, 146.0);
            let mut wall = prim(
                COLD_WALL_ID,
                Vec3::new(x, y, floor_at(x, y) + 4.0),
                Quat::IDENTITY,
                Vec3::new(0.4, 24.0, 8.0),
                shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
                super::te([0.55, 0.55, 0.6, 1.0], 0, false, 0.0),
                ExtraParams::default(),
                "",
            );
            wall.update_flags = 0;
            world.apply(NetEvent::ObjectUpdates {
                handle: HANDLE,
                objects: vec![wall],
            });
        }
        if t.is_multiple_of(COLD_WAVE_EVERY) && t / COLD_WAVE_EVERY < (COLD_OBJECTS / COLD_WAVE) as u64 {
            let first = (t / COLD_WAVE_EVERY) as u32 * COLD_WAVE;
            let shapes = [
                shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0),
                shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0),
                shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0),
            ];
            let mut objects = Vec::new();
            for i in first..first + COLD_WAVE {
                // a field ahead of the camera, 4 to 60 m away, of all sizes
                let (row, col) = (i / 20, i % 20);
                let (x, y) = (133.0 + row as f32 * 2.9, 112.0 + col as f32 * 2.4 + (row % 2) as f32 * 1.2);
                let size = 0.5 + ((i * 37) % 10) as f32 * 0.28;
                let tex = 200 + i;
                let _ = scene.textures.acquire(renderer, self.texture(tex), TexSource::Asset);
                self.due.push((frame + cold_delay(i), tex));
                let mut o = self.textured(
                    0,
                    Vec3::new(x, y, floor_at(x, y) + size * 0.5),
                    Quat::from_rotation_z(i as f32),
                    Vec3::splat(size),
                    shapes[i as usize % 3],
                    tex,
                );
                o.local_id = id(round, i);
                o.full_id = Uuid::from_u128(0xA0E0_0000_0000_0000_0000_0000_0000_0000 | o.local_id as u128);
                objects.push(o);
            }
            world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
            if first == 0 {
                log::info!("demo rez: cold arrival, round {}", round + 1);
            }
        }
        let now: Vec<u32> = self.due.iter().filter(|(at, _)| *at <= frame).map(|&(_, t)| t).collect();
        if !now.is_empty() {
            self.due.retain(|(at, _)| *at > frame);
            self.decode(&scene.textures, &scene.jobs, now);
        }
        if t == COLD_ROUND - 1 {
            let s = scene.rez.stats;
            log::info!(
                "demo rez: round {} done: {} faces out of the grid, {} grids, {} lines, {} glints, {} fades ({} with every line busy), {} behind a wall, {} followed now",
                round + 1,
                s.face_fades,
                s.holograms,
                s.sweeps,
                s.glints,
                s.fades,
                s.no_line,
                s.hidden,
                s.units
            );
        }
    }

    pub fn tick(&mut self, frame: u64, world: &mut crate::world::World, scene: &mut Scene, renderer: &mut Renderer) {
        if self.cold {
            self.tick_cold(frame, world, scene, renderer);
            return;
        }
        if self.styles {
            self.tick_styles(frame, world, scene, renderer);
            return;
        }
        // (`loop`: frames past the first round count from its start again)
        let step = if self.looping && frame >= ROUND_END {
            LOADING + (frame - ROUND_END) % (ROUND_END - LOADING)
        } else {
            frame
        };
        if self.looping && frame >= ROUND_END && step == LOADING {
            // everything at once, a burst: no derez
            let mut local_ids: Vec<u32> = (0..ROUND_IDS).map(|i| self.id(i)).collect();
            local_ids.push(self.avatar_id());
            world.apply(NetEvent::ObjectsKilled { handle: HANDLE, local_ids });
            self.round += 1;
            log::info!("demo rez: round {}", self.round + 1);
        }
        let boxp = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_SQUARE, 100, 0, 0);
        let cylinder = shape(LL_PCODE_PATH_LINE, LL_PCODE_PROFILE_CIRCLE, 100, 0, 0);
        let sphere = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE_HALF, 100, 0, 0);
        let torus = shape(LL_PCODE_PATH_CIRCLE, LL_PCODE_PROFILE_CIRCLE, 50, 0, 0);
        let jobs = &scene.jobs;
        let mut objects = Vec::new();
        let mut killed = Vec::new();
        match step {
            LOADING => {
                for (k, volume) in [boxp, cylinder, sphere, torus].into_iter().enumerate() {
                    let (x, y) = (139.0, 136.0 + k as f32 * 2.2);
                    let size = Vec3::new(1.2, 1.2, 1.6);
                    self.texture_now(&mut scene.textures, renderer, jobs, k as u32, true);
                    objects.push(self.textured(
                        k as u32,
                        Vec3::new(x, y, floor_at(x, y) + size.z * 0.5),
                        Quat::from_rotation_z(0.4 * k as f32),
                        size,
                        volume,
                        k as u32,
                    ));
                }
                log::info!("demo rez: four prims arrive, their textures {LOADING_TEXTURES} frames later");
            }
            f if f == LOADING + LOADING_TEXTURES => {
                self.decode(&scene.textures, jobs, (0..4).collect());
                log::info!("demo rez: the four prims' textures come");
            }
            TABLE => {
                // a table: its top the root, three legs linked to it
                let (x, y) = (134.0, 137.5);
                let z = floor_at(x, y) + 1.0;
                self.texture_now(&mut scene.textures, renderer, jobs, 10, true);
                let mut top = self.textured(10, Vec3::new(x, y, z), Quat::IDENTITY, Vec3::new(1.8, 1.0, 0.08), boxp, 10);
                top.owner_id = DEMO_NOVA;
                objects.push(top);
                for (k, (dx, dy)) in [(-0.8, -0.4), (0.8, -0.4), (0.0, 0.4)].into_iter().enumerate() {
                    let mut leg = self.textured(
                        11 + k as u32,
                        Vec3::new(dx, dy, -0.5),
                        Quat::IDENTITY,
                        Vec3::new(0.08, 0.08, 1.0),
                        cylinder,
                        10,
                    );
                    leg.parent_id = self.id(10);
                    objects.push(leg);
                }
                log::info!("demo rez: someone rezzes a table (4 prims), its texture {TABLE_TEXTURES} frames later");
            }
            f if f == TABLE + TABLE_TEXTURES => self.decode(&scene.textures, jobs, vec![10]),
            OWN | COPY => {
                let (x, y) = if step == OWN { (134.5, 140.5) } else { (134.5, 142.0) };
                self.texture_now(&mut scene.textures, renderer, jobs, 20, false);
                let i = if step == OWN { 20 } else { 21 };
                let mut cube = self.textured(
                    i,
                    Vec3::new(x, y, floor_at(x, y) + 0.6),
                    Quat::from_rotation_z(0.6),
                    Vec3::splat(1.2),
                    boxp,
                    20,
                );
                cube.owner_id = world.agent_id;
                cube.update_flags = crate::build::FLAGS_CREATE_SELECTED
                    | crate::ui::context::flags::OBJECT_YOU_OWNER
                    | crate::ui::context::flags::OBJECT_MODIFY;
                objects.push(cube);
                log::info!(
                    "demo rez: {}",
                    if step == OWN { "we rez a cube" } else { "we copy it (Shift-drag)" }
                );
            }
            AVATAR => {
                let (x, y) = (136.0, 136.0);
                let mut o = prim(
                    self.avatar_id(),
                    Vec3::new(x, y, floor_at(x, y) + 0.95),
                    Quat::from_rotation_z(std::f32::consts::PI),
                    Vec3::new(0.45, 0.6, 1.9),
                    boxp,
                    super::te([0.2, 0.2, 0.25, 1.0], 0, false, 0.0),
                    ExtraParams::default(),
                    "",
                );
                o.pcode = LL_PCODE_LEGACY_AVATAR;
                o.full_id = self.avatar_uuid();
                o.name_values = "FirstName STRING RW SV Rez\nLastName STRING RW SV Visitor".into();
                objects.push(o);
                self.texture_now(&mut scene.textures, renderer, jobs, BAKES_BEFORE, false);
                let bakes = self.bakes(&world.avatar_lib.shape_params, self.avatar_uuid(), BAKES_BEFORE);
                world.apply(bakes);
                log::info!("demo rez: an avatar comes into view");
            }
            STORM => {
                for k in 0..STORM_COUNT {
                    let (x, y) = (142.0 + (k / 6) as f32 * 1.1, 135.0 + (k % 6) as f32 * 1.6);
                    self.texture_now(&mut scene.textures, renderer, jobs, 30 + k, false);
                    let mut o = self.textured(
                        30 + k,
                        Vec3::new(x, y, floor_at(x, y) + 0.35),
                        Quat::from_rotation_z(k as f32),
                        Vec3::splat(0.7),
                        boxp,
                        30 + k,
                    );
                    o.owner_id = DEMO_NOVA;
                    objects.push(o);
                }
                log::info!("demo rez: someone rezzes {STORM_COUNT} boxes at once");
            }
            BUILDING => {
                // a floor (the root), four walls and a roof
                let (x, y) = (147.0, 141.0);
                let z = floor_at(x, y) + 0.1;
                self.texture_now(&mut scene.textures, renderer, jobs, 70, true);
                self.texture_now(&mut scene.textures, renderer, jobs, 71, true);
                let mut root = self.textured(70, Vec3::new(x, y, z), Quat::IDENTITY, Vec3::new(5.0, 4.0, 0.2), boxp, 70);
                root.owner_id = DEMO_NOVA;
                objects.push(root);
                let parts = [
                    (Vec3::new(0.0, -1.95, 1.7), Vec3::new(5.0, 0.1, 3.2), 71),
                    (Vec3::new(0.0, 1.95, 1.7), Vec3::new(5.0, 0.1, 3.2), 71),
                    (Vec3::new(-2.45, 0.0, 1.7), Vec3::new(0.1, 4.0, 3.2), 71),
                    (Vec3::new(2.45, 0.0, 1.7), Vec3::new(0.1, 4.0, 1.2), 71),
                    (Vec3::new(0.0, 0.0, 3.4), Vec3::new(5.4, 4.4, 0.2), 70),
                ];
                for (k, (at, size, tex)) in parts.into_iter().enumerate() {
                    let mut o = self.textured(72 + k as u32, at, Quat::IDENTITY, size, boxp, tex);
                    o.parent_id = self.id(70);
                    objects.push(o);
                }
                log::info!("demo rez: someone rezzes a building (6 prims), its textures {BUILDING_TEXTURES} frames later");
            }
            f if f == BUILDING + BUILDING_TEXTURES => self.decode(&scene.textures, jobs, vec![70, 71]),
            HAT_ON => {
                self.texture_now(&mut scene.textures, renderer, jobs, 90, false);
                objects.push(self.worn(90, 2, Vec3::new(0.0, 0.0, 0.14), Vec3::new(0.32, 0.32, 0.18), cylinder, 90));
                log::info!("demo rez: the avatar puts on a hat");
            }
            TABLE_GONE => {
                killed.push(self.id(10));
                log::info!("demo rez: the table is deleted");
            }
            HAT_OFF => {
                killed.push(self.id(90));
                log::info!("demo rez: the avatar takes the hat off");
            }
            OUTFIT => {
                for (k, (point, at, size, volume)) in [
                    (1u8, Vec3::new(0.12, 0.0, 0.0), Vec3::new(0.12, 0.42, 0.5), boxp),
                    (3, Vec3::new(0.0, 0.0, 0.06), Vec3::splat(0.22), sphere),
                    (4, Vec3::new(0.0, 0.0, 0.06), Vec3::splat(0.22), sphere),
                ]
                .into_iter()
                .enumerate()
                {
                    self.texture_now(&mut scene.textures, renderer, jobs, 91 + k as u32, true);
                    objects.push(self.worn(91 + k as u32, point, at, size, volume, 91 + k as u32));
                }
                self.texture_now(&mut scene.textures, renderer, jobs, BAKES_AFTER, true);
                let bakes = self.bakes(&world.avatar_lib.shape_params, self.avatar_uuid(), BAKES_AFTER);
                world.apply(bakes);
                log::info!("demo rez: the avatar changes outfit (3 items and its bakes), their textures {OUTFIT_TEXTURES} frames later");
            }
            f if f == OUTFIT + OUTFIT_TEXTURES => self.decode(&scene.textures, jobs, vec![91, 92, 93, BAKES_AFTER]),
            AVATAR_GONE => {
                killed.push(self.avatar_id());
                log::info!("demo rez: the avatar leaves");
            }
            REPORT => {
                let s = scene.rez.stats;
                log::info!(
                    "demo rez: {} grids, {} lines, {} glints, {} rezzed, {} derezzed, {} worn, {} outfits, {} shown without a line (all busy), {} behind a wall, {} running",
                    s.holograms,
                    s.sweeps,
                    s.glints,
                    s.rezzed,
                    s.derezzed,
                    s.worn,
                    s.outfits,
                    s.no_line,
                    s.hidden,
                    scene.rez.sweeping()
                );
            }
            _ => {}
        }
        if !objects.is_empty() {
            world.apply(NetEvent::ObjectUpdates { handle: HANDLE, objects });
        }
        if !killed.is_empty() {
            world.apply(NetEvent::ObjectsKilled {
                handle: HANDLE,
                local_ids: killed,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_points_land_where_objects_read_them() {
        for point in [1u8, 2, 3, 4, 31, 38] {
            let s = attach_state(point);
            assert_eq!(((s & 0xF0) >> 4) | ((s & 0x0F) << 4), point);
        }
    }
}
