//! Avatars still loading, after LLVOAvatar::updateIsFullyLoaded /
//! getRezzedStatus / processFullyLoadedChange of the Second Life / Firestorm
//! viewer (originally LGPL 2.1, Copyright (C) 2001-2024 Linden Research, Inc. and the
//! Firestorm project): an avatar shows once its bakes are downloaded and its
//! worn objects have arrived and been built, a little after it settles (3 to
//! 15 s the first time, 1 s later on); until then it is a particle cloud
//! (Firestorm cloud.xml) with a progress bar under its name.
//!
//! What cannot come is not waited for, as in Firestorm: an invisible bake is
//! a defined bake with nothing to download (LLVOAvatar::isTextureDefined,
//! getBakedTextureImage), and a mesh the asset server does not have is not
//! expected (check_object_for_mesh_loading: isMeshAssetUnavaliable). Waiting
//! for them kept the avatar a cloud for the whole 60 s limit.

use super::{GeomKey, Scene, avatar};
use crate::world::World;
use std::collections::HashMap;
use std::time::Instant;
use uuid::Uuid;

/// processFullyLoadedChange: LOADED_DELAY, FIRST_APPEARANCE_CLOUD_MIN_DELAY.
const LOADED_DELAY: f32 = 1.0;
const FIRST_MIN_DELAY: f32 = 3.0;
/// MAX_TEXTURE_WAIT_TIME_SEC / MAX_ATTACHMENT_WAIT_TIME_SEC: shown anyway.
const MAX_WAIT: f32 = 60.0;
/// The worn objects count changed less than this long ago: more may come
/// (LL compares with the simulator's attachment list).
const ATTACHMENTS_SETTLE: f32 = 2.0;
/// Still a cloud after this long: what is missing goes to the log, once.
const REPORT_AFTER: f32 = 20.0;

/// What a bake slot asks of an avatar before it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BakeNeed {
    /// No appearance yet for a required slot (LLVOAvatar::getHasMissingParts).
    Undefined,
    /// Nothing to download.
    Nothing,
    /// A baked texture to download.
    Texture(Uuid),
}

/// LLVOAvatar::isTextureDefined: a bake slot is undefined while it holds no
/// texture or the default avatar texture, and only the head, upper and lower
/// bakes are required (getHasMissingParts). IMG_INVISIBLE is a defined bake,
/// an alpha layer hiding the whole part, that is never downloaded
/// (getBakedTextureImage).
fn bake_need(tex: Option<Uuid>, main: bool) -> BakeNeed {
    match tex {
        Some(t) if t == avatar::IMG_INVISIBLE => BakeNeed::Nothing,
        Some(t) if !t.is_nil() && t != avatar::IMG_DEFAULT_AVATAR => BakeNeed::Texture(t),
        _ if main => BakeNeed::Undefined,
        _ => BakeNeed::Nothing,
    }
}

/// Where the geometry of a worn prim stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GeomWait {
    Built,
    Pending,
    /// It cannot come: not expected (the avatar shows without it).
    GivenUp,
}

/// `pending`: asked for and not bound yet. As check_object_for_mesh_loading
/// stops expecting a mesh marked unavailable, it is given up when its asset
/// cannot be had (`unavailable`: missing or invalid mesh asset, sculpt map
/// that cannot be downloaded) or could not be built from it (`build_failed`).
fn geom_wait(pending: bool, unavailable: bool, build_failed: bool) -> GeomWait {
    if !pending {
        GeomWait::Built
    } else if unavailable || build_failed {
        GeomWait::GivenUp
    } else {
        GeomWait::Pending
    }
}

/// A part an avatar still waits for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Missing {
    /// A required bake slot is undefined.
    Appearance,
    /// A bake (appearance-service name) being downloaded.
    Bake(&'static str, Uuid),
    /// Geometry of a worn prim (object index).
    Geometry(usize),
}

#[derive(Debug, Clone, Copy)]
struct State {
    first_seen: Instant,
    /// Last time it was still loading.
    loading_at: Instant,
    /// Shown fully once (later loads use the short delay).
    shown_once: bool,
    attachments: usize,
    attachments_changed: Instant,
    /// "Still loading" already logged.
    reported: bool,
    /// Progress shown so far (it never goes back when more parts arrive).
    shown: f32,
}

/// One frame of an avatar's loading state.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tick {
    /// Seconds since the avatar was first seen.
    waited: f32,
    /// Its worn objects count changed a moment ago.
    settling: bool,
    /// Something is still expected: the delay before showing restarts.
    still: bool,
    /// Shown fully.
    loaded: bool,
    /// Shown fully for the first time, this frame...
    first_show: bool,
    /// ...by the time limit, with parts still expected.
    timed_out: bool,
    /// Progress of the delay before showing (0..1).
    delay: f32,
}

impl State {
    fn new(now: Instant, attachments: usize) -> State {
        State {
            first_seen: now,
            loading_at: now,
            shown_once: false,
            attachments,
            attachments_changed: now,
            reported: false,
            shown: 0.0,
        }
    }

    /// processFullyLoadedChange: once nothing is missing (`complete`) and
    /// the worn objects have stopped arriving, 3 s the first time (1 s later
    /// on); LL's 3..15 s delay (x1.25 for impostors) only applies while
    /// still loading, when the timer is reset anyway. After MAX_WAIT the
    /// avatar shows whatever is missing.
    fn tick(&mut self, now: Instant, attachments: usize, complete: bool) -> Tick {
        if self.attachments != attachments {
            self.attachments = attachments;
            self.attachments_changed = now;
        }
        let settling = now.duration_since(self.attachments_changed).as_secs_f32() < ATTACHMENTS_SETTLE;
        let waited = now.duration_since(self.first_seen).as_secs_f32();
        let expecting = !complete || settling;
        let still = expecting && waited < MAX_WAIT;
        if still {
            self.loading_at = now;
        }
        let delay = if self.shown_once { LOADED_DELAY } else { FIRST_MIN_DELAY };
        let since = now.duration_since(self.loading_at).as_secs_f32();
        let loaded = !still && (since > delay || waited > MAX_WAIT);
        let first_show = loaded && !self.shown_once;
        if loaded {
            self.shown_once = true;
        }
        Tick {
            waited,
            settling,
            still,
            loaded,
            first_show,
            timed_out: first_show && expecting,
            delay: (since / delay).min(1.0),
        }
    }
}

#[derive(Default)]
pub struct Loading {
    states: HashMap<Uuid, State>,
    /// Avatars not fully loaded yet, with their progress (0..1).
    pub progress: HashMap<Uuid, f32>,
    /// Their object indices.
    pub indices: std::collections::HashSet<usize>,
}

impl Scene {
    /// What is loaded of an avatar: (done, total), what is still missing
    /// (empty when complete) and the textures to hurry.
    fn avatar_load_parts(&self, world: &World, idx: usize, hurry: &mut Vec<Uuid>) -> (u32, u32, Vec<Missing>) {
        let Some(av) = world.objects.get(idx) else {
            return (0, 1, vec![Missing::Appearance]);
        };
        let (mut done, mut total, mut missing) = (0u32, 0u32, Vec::new());
        // bakes: defined (getHasMissingParts) and downloaded; only those
        // something shows count (bakes on mesh avatars define unused ones)
        for (te, name, _) in avatar::BAKES {
            let main = matches!(name, "head" | "upper" | "lower");
            match bake_need(world.bake_texture(idx, te), main) {
                BakeNeed::Texture(t) => {
                    if !self.textures.is_wanted(&t) {
                        continue;
                    }
                    total += 1;
                    if self.textures.is_loaded(&t) {
                        done += 1;
                    } else {
                        hurry.push(t);
                        missing.push(Missing::Bake(name, t));
                    }
                }
                BakeNeed::Undefined => {
                    // no appearance yet
                    total += 1;
                    missing.push(Missing::Appearance);
                }
                BakeNeed::Nothing => {}
            }
        }
        // worn objects: geometry built (meshes downloaded)
        for &att in world.objects.children_of(&av.key) {
            let Some(root) = world.objects.get(att) else {
                continue;
            };
            if (31..=38).contains(&root.attachment_point()) {
                continue; // HUD
            }
            let mut prims = vec![att];
            prims.extend_from_slice(world.objects.children_of(&root.key));
            for p in prims {
                let Some(g) = self.gpu.get(p) else {
                    continue;
                };
                total += 1;
                // geometry asked for and not there yet
                let pending = g.wanted_geom.is_some_and(|k| Some(k) != g.geom) || g.sculpt_wait.is_some();
                let unavailable = match g.wanted_geom {
                    Some(GeomKey::Mesh { id, .. }) => self.meshes.failed(&id),
                    Some(GeomKey::Sculpt { .. }) => g.sculpt_wait.is_some_and(|t| self.textures.is_unavailable(&t)),
                    _ => false,
                };
                let build_failed = g.wanted_geom.is_some_and(|k| self.geom_failed(&k));
                if geom_wait(pending, unavailable, build_failed) == GeomWait::Pending {
                    missing.push(Missing::Geometry(p));
                } else {
                    done += 1;
                }
                // their textures first (the avatar is not drawn meanwhile)
                for f in &g.faces {
                    if !f.tex_id.is_nil() && !self.textures.is_loaded(&f.tex_id) {
                        hurry.push(f.tex_id);
                    }
                }
            }
        }
        (done, total.max(1), missing)
    }

    /// Diagnostic: the first missing parts with the state of what they wait
    /// for (asset ids and fetch states, never a URL).
    fn describe_missing(&self, world: &World, missing: &[Missing]) -> String {
        const SHOWN: usize = 8;
        let mut parts: Vec<String> = missing
            .iter()
            .take(SHOWN)
            .map(|m| match *m {
                Missing::Appearance => "appearance (bake undefined)".to_owned(),
                Missing::Bake(name, id) => format!("bake {name} {id} ({})", self.textures.describe(&id)),
                Missing::Geometry(p) => {
                    let local = world.objects.get(p).map(|o| o.key.local_id).unwrap_or(0);
                    let g = self.gpu.get(p);
                    match (g.and_then(|g| g.wanted_geom), g.and_then(|g| g.sculpt_wait)) {
                        (Some(GeomKey::Mesh { id, lod }), _) => {
                            format!("geometry of {local}: mesh {id} LOD {lod} ({})", self.meshes.describe(&id))
                        }
                        (Some(GeomKey::Sculpt { .. }), Some(t)) => {
                            format!("geometry of {local}: sculpt map {t} ({})", self.textures.describe(&t))
                        }
                        _ => format!("geometry of {local}: being built"),
                    }
                }
            })
            .collect();
        if missing.len() > SHOWN {
            parts.push(format!("and {} more", missing.len() - SHOWN));
        }
        parts.join("; ")
    }

    /// Refresh which avatars are still loading (every frame).
    pub fn update_loading(&mut self, world: &World, now: Instant) {
        let mut loading = std::mem::take(&mut self.loading);
        loading.progress.clear();
        loading.indices.clear();
        let mut seen = Vec::new();
        let mut hurry = Vec::new();
        for (idx, o) in world.objects.avatars() {
            // silhouettes are not clouded (LL: no cloud for too complex avatars)
            if self.too_complex.contains(&idx) {
                seen.push(o.full_id);
                continue;
            }
            let id = o.full_id;
            seen.push(id);
            let attachments = world.objects.children_of(&o.key).len();
            let s = loading.states.entry(id).or_insert_with(|| State::new(now, attachments));
            let mut wanted = Vec::new();
            let (done, total, missing) = self.avatar_load_parts(world, idx, &mut wanted);
            let shown_before = s.shown_once;
            let t = s.tick(now, attachments, missing.is_empty());
            let own = if id == world.agent_id { " (own)" } else { "" };
            if !shown_before {
                hurry.extend(wanted);
                // diagnostic: still a cloud after a while
                if !t.loaded && t.waited > REPORT_AFTER && !s.reported {
                    s.reported = true;
                    log::info!(
                        "avatar {id}{own} still loading after {:.0} s: {done}/{total} parts, attachments settling {}, missing [{}]",
                        t.waited,
                        t.settling,
                        self.describe_missing(world, &missing)
                    );
                }
            }
            if t.timed_out {
                // shown by the time limit: what it was waiting for is the
                // cause to look at
                log::warn!(
                    "avatar {id}{own} shown by the {MAX_WAIT:.0} s limit after {:.1} s: {done}/{total} parts, attachments settling {}, still missing [{}]",
                    t.waited,
                    t.settling,
                    self.describe_missing(world, &missing)
                );
            } else if t.first_show {
                log::info!("avatar {id}{own} shown after {:.1} s", t.waited);
            }
            if !t.loaded && !s.shown_once {
                // parts ready over parts known; the last bit is the settling
                // delay (it fills as the avatar is about to show)
                let parts = done as f32 / total as f32;
                let p = if t.still { parts * 0.9 } else { 0.9 + 0.1 * t.delay };
                s.shown = s.shown.max(p.min(0.99));
                loading.progress.insert(id, s.shown);
                loading.indices.insert(idx);
            }
        }
        loading.states.retain(|id, _| seen.contains(id));
        self.loading = loading;
        for t in hurry {
            self.textures.note_usage(&t, 512.0);
        }
    }

    /// Avatars to show as clouds (not fully loaded yet).
    pub fn loading_avatars(&self) -> &HashMap<Uuid, f32> {
        &self.loading.progress
    }
}

#[cfg(test)]
mod tests {
    use super::super::{GeomState, ObjGpu};
    use super::*;
    use crate::scene::avatar::AvatarLibrary;
    use std::sync::Arc;
    use std::time::Duration;

    fn secs(s: f32) -> Duration {
        Duration::from_secs_f32(s)
    }

    #[test]
    fn invisible_bake_is_a_defined_bake() {
        let tex = Uuid::from_u128(7);
        // isTextureDefined: only "no texture" and the default avatar texture
        // leave a required slot undefined
        assert_eq!(bake_need(None, true), BakeNeed::Undefined);
        assert_eq!(bake_need(Some(Uuid::nil()), true), BakeNeed::Undefined);
        assert_eq!(bake_need(Some(avatar::IMG_DEFAULT_AVATAR), true), BakeNeed::Undefined);
        // a full alpha layer: defined, and nothing to download
        assert_eq!(bake_need(Some(avatar::IMG_INVISIBLE), true), BakeNeed::Nothing);
        assert_eq!(bake_need(Some(avatar::IMG_INVISIBLE), false), BakeNeed::Nothing);
        assert_eq!(bake_need(Some(tex), true), BakeNeed::Texture(tex));
        assert_eq!(bake_need(Some(tex), false), BakeNeed::Texture(tex));
        // the other slots (eyes, hair, skirt, universal) are optional
        assert_eq!(bake_need(None, false), BakeNeed::Nothing);
        assert_eq!(bake_need(Some(avatar::IMG_DEFAULT_AVATAR), false), BakeNeed::Nothing);
    }

    #[test]
    fn geometry_that_cannot_come_is_given_up() {
        assert_eq!(geom_wait(false, false, false), GeomWait::Built);
        assert_eq!(geom_wait(true, false, false), GeomWait::Pending);
        assert_eq!(geom_wait(true, true, false), GeomWait::GivenUp);
        assert_eq!(geom_wait(true, false, true), GeomWait::GivenUp);
        // built with another LOD while the wanted one failed: nothing to wait for
        assert_eq!(geom_wait(false, true, true), GeomWait::Built);
    }

    #[test]
    fn shows_three_seconds_after_everything_settled() {
        let t0 = Instant::now();
        let mut s = State::new(t0, 4);
        // complete from the start: the worn objects settle (2 s), then 3 s
        let t = s.tick(t0 + secs(1.9), 4, true);
        assert!(t.still && t.settling && !t.loaded);
        let t = s.tick(t0 + secs(2.5), 4, true);
        assert!(!t.still && !t.settling && !t.loaded);
        assert!(t.delay > 0.0 && t.delay < 1.0);
        // (the delay runs from the last frame it was still loading: 1.9 s)
        assert!(!s.tick(t0 + secs(4.8), 4, true).loaded);
        let t = s.tick(t0 + secs(5.0), 4, true);
        assert!(t.loaded && t.first_show && !t.timed_out);
        assert_eq!(t.delay, 1.0);
        // later frames: shown, no longer "first"
        let t = s.tick(t0 + secs(5.1), 4, true);
        assert!(t.loaded && !t.first_show);
    }

    #[test]
    fn a_missing_part_restarts_the_delay() {
        let t0 = Instant::now();
        let mut s = State::new(t0, 0);
        assert!(s.tick(t0 + secs(3.0), 0, false).still);
        // complete at 3 s: not shown before 6 s
        assert!(!s.tick(t0 + secs(5.9), 0, true).loaded);
        // a mesh changes LOD at 5.95 s: the 3 s start again
        assert!(s.tick(t0 + secs(5.95), 0, false).still);
        assert!(!s.tick(t0 + secs(8.9), 0, true).loaded);
        assert!(s.tick(t0 + secs(9.0), 0, true).loaded);
    }

    #[test]
    fn new_attachments_restart_the_wait() {
        let t0 = Instant::now();
        let mut s = State::new(t0, 1);
        assert!(!s.tick(t0 + secs(2.5), 1, true).still);
        // another worn object arrives: 2 s of settling again
        let t = s.tick(t0 + secs(3.0), 2, true);
        assert!(t.still && t.settling);
        assert!(s.tick(t0 + secs(4.9), 2, true).still);
        assert!(!s.tick(t0 + secs(5.1), 2, true).still);
        assert!(!s.tick(t0 + secs(7.8), 2, true).loaded);
        assert!(s.tick(t0 + secs(8.0), 2, true).loaded);
    }

    #[test]
    fn time_limit_shows_an_incomplete_avatar_and_says_so() {
        let t0 = Instant::now();
        let mut s = State::new(t0, 0);
        let t = s.tick(t0 + secs(59.9), 0, false);
        assert!(t.still && !t.loaded && !t.timed_out);
        let t = s.tick(t0 + secs(60.1), 0, false);
        assert!(!t.still && t.loaded && t.first_show && t.timed_out);
        // reported once
        let t = s.tick(t0 + secs(60.2), 0, false);
        assert!(t.loaded && !t.first_show && !t.timed_out);
    }

    #[test]
    fn once_shown_an_avatar_uses_the_short_delay() {
        let t0 = Instant::now();
        let mut s = State::new(t0, 0);
        assert!(s.tick(t0 + secs(5.1), 0, true).loaded);
        // a new bake to download at 10 s: loading again, 1 s after it is there
        let t = s.tick(t0 + secs(10.0), 0, false);
        assert!(t.still && !t.loaded && !t.first_show);
        assert!(!s.tick(t0 + secs(10.9), 0, true).loaded);
        let t = s.tick(t0 + secs(11.1), 0, true);
        assert!(t.loaded && !t.first_show && !t.timed_out);
    }

    /// The demo region with our avatar dressed by AURORA_DEMO_CLOUD=self.
    fn cloud_self() -> (World, Scene, usize, usize) {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        world.agent_id = crate::demo::DEMO_AGENT;
        for ev in crate::demo::events().into_iter().chain(crate::demo::cloud_self_events()) {
            world.apply(ev);
        }
        let mut scene = Scene::new(std::path::PathBuf::new(), lib);
        scene.gpu.resize_with(world.objects.slots.len(), ObjGpu::default);
        let own = world.objects.index_of_uuid(&world.agent_id).expect("demo agent");
        let loup = world.objects.index_of_uuid(&crate::demo::DEMO_LOUP).expect("other demo avatar");
        (world, scene, own, loup)
    }

    /// Index of the prim our avatar wears on the given attachment point.
    fn worn(world: &World, own: usize, point: u8) -> usize {
        let key = world.objects.get(own).expect("avatar").key;
        *world
            .objects
            .children_of(&key)
            .iter()
            .find(|i| world.objects.get(**i).is_some_and(|o| o.attachment_point() == point))
            .expect("worn object")
    }

    #[test]
    fn avatar_with_an_invisible_head_bake_has_an_appearance() {
        let (world, scene, own, loup) = cloud_self();
        assert_eq!(world.bake_texture(own, 8), Some(avatar::IMG_INVISIBLE));
        let mut hurry = Vec::new();
        let (done, total, missing) = scene.avatar_load_parts(&world, own, &mut hurry);
        assert!(missing.is_empty(), "nothing to wait for: {missing:?}");
        assert_eq!(done, total);
        // an avatar the simulator has no bakes for yet (default avatar
        // texture everywhere) still waits for its appearance
        let mut unbaked = aurora_prim::te::TextureEntry::default();
        for f in &mut unbaked.faces {
            f.texture = avatar::IMG_DEFAULT_AVATAR;
        }
        let mut world = world;
        world.apply(aurora_net::NetEvent::Appearance(aurora_net::AvatarAppearance {
            avatar_id: crate::demo::DEMO_LOUP,
            texture_entry: Some(Arc::new(unbaked)),
            visual_params: Vec::new(),
            cof_version: 1,
            hover_height: 0.0,
        }));
        let (_, _, missing) = scene.avatar_load_parts(&world, loup, &mut hurry);
        assert_eq!(missing, [Missing::Appearance; 3]);
    }

    #[test]
    fn unavailable_worn_mesh_is_not_waited_for() {
        let (world, mut scene, own, _) = cloud_self();
        let mesh = worn(&world, own, 1);
        let key = GeomKey::Mesh {
            id: crate::demo::CLOUD_MESH,
            lod: 3,
        };
        scene.gpu[mesh].wanted_geom = Some(key);
        scene.geom_insert(key, GeomState::Pending);
        let mut hurry = Vec::new();
        // being downloaded: waited for
        let (_, _, missing) = scene.avatar_load_parts(&world, own, &mut hurry);
        assert_eq!(missing, [Missing::Geometry(mesh)]);
        assert!(scene.describe_missing(&world, &missing).contains("mesh"));
        // the asset server does not have it (404): not expected any more
        scene.meshes.insert_demo_unavailable(crate::demo::CLOUD_MESH);
        let (done, total, missing) = scene.avatar_load_parts(&world, own, &mut hurry);
        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(done, total);
    }

    #[test]
    fn worn_mesh_that_cannot_be_built_is_not_waited_for() {
        let (world, mut scene, own, _) = cloud_self();
        let mesh = worn(&world, own, 1);
        // the asset arrived, this LOD could not be decoded
        let key = GeomKey::Mesh {
            id: Uuid::from_u128(0x77),
            lod: 2,
        };
        scene.gpu[mesh].wanted_geom = Some(key);
        scene.geom_insert(key, GeomState::Failed);
        let mut hurry = Vec::new();
        let (done, total, missing) = scene.avatar_load_parts(&world, own, &mut hurry);
        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(done, total);
    }
}
