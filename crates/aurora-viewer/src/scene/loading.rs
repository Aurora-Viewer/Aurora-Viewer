//! Avatars still loading, after LLVOAvatar::updateIsFullyLoaded /
//! getRezzedStatus / processFullyLoadedChange of the Second Life / Firestorm
//! viewer (originally LGPL 2.1, Copyright (C) 2001-2024 Linden Research, Inc. and the
//! Firestorm project): an avatar shows once its bakes are downloaded and its
//! worn objects have arrived and been built, a little after it settles (1 s;
//! the first time, until the textures of what it wears show, 3 s at most;
//! `State::tick`); until then it is a particle cloud
//! (Firestorm cloud.xml) with a progress bar under its name.
//!
//! The worn objects to wait for are those the simulator lists with the
//! appearance (mSimAttachments): another avatar shows once as many have
//! come, within the 60 s limit. Ours is not compared with that list
//! (`!isSelf()`), and neither is an avatar the simulator lists nothing for.
//!
//! What cannot come is not waited for, as in Firestorm: an invisible bake is
//! a defined bake with nothing to download (LLVOAvatar::isTextureDefined,
//! getBakedTextureImage), and a mesh the asset server does not have is not
//! expected (check_object_for_mesh_loading: isMeshAssetUnavaliable). Waiting
//! for them kept the avatar a cloud for the whole 60 s limit.
//!
//! Our own avatar is remembered while no region has it: Firestorm never
//! destroys gAgentAvatarp (LLViewerObjectList::killObject moves it to the
//! agent's region), so arriving from a teleport is not a first appearance
//! and it shows at once. The world keeps it and what it wears across the
//! region change (`ObjectStore::carry`); this covers what is left: an
//! avatar removed all the same, then sent again.

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
/// Still a cloud after this long: what is missing goes to the log, once.
const REPORT_AFTER: f32 = 20.0;
/// Weights of the progress bar under a loading avatar's name
/// (`Scene::avatar_load_progress`): a bake, the geometry of a worn prim
/// (or a worn object the simulator lists that has not come), a texture on
/// what it wears.
const BAR_BAKE: f32 = 3.0;
const BAR_GEOMETRY: f32 = 2.0;
const BAR_TEXTURE: f32 = 1.0;
/// Share of the bar the parts fill; the delay before showing fills the rest.
const BAR_PARTS: f32 = 0.9;
/// Size on screen the textures a loading avatar waits for are asked for at
/// least (its body is not drawn, so nothing else notes them).
const HURRY_PX: f32 = 512.0;

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
    /// Worn objects the simulator lists (mSimAttachments)...
    announced: usize,
    /// ...and as many came once: not compared again
    /// (mLastCloudAttachmentCount reaching the list size).
    all_came: bool,
    /// "Still loading" already logged.
    reported: bool,
    /// Progress shown so far (it never goes back when more parts arrive).
    shown: f32,
    /// When each part was last complete (the `shown after` log line).
    timeline: Timeline,
}

/// When each kind of part of a loading avatar became complete, in seconds
/// since it was first seen, for the `shown after` log line: which part
/// held it back. A part that goes missing again (a worn object arriving
/// later, a new appearance) is cleared and marked again when it completes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Timeline {
    /// Required bakes defined: the appearance has come.
    appearance: Option<f32>,
    /// Bakes downloaded.
    bakes: Option<f32>,
    /// As many worn objects as the simulator lists have come.
    worn: Option<f32>,
    /// Geometry of what it wears built.
    geometry: Option<f32>,
    /// Textures on what it wears shown (not waited for): (shown, total) at
    /// the last frame before it showed.
    textures: Option<f32>,
    textures_count: (usize, usize),
    /// Nothing missing any more: the delay before showing starts.
    ready: Option<f32>,
}

impl Timeline {
    fn mark(at: &mut Option<f32>, done: bool, t: f32) {
        if !done {
            *at = None;
        } else if at.is_none() {
            *at = Some(t);
        }
    }

    fn describe(&self) -> String {
        let at = |t: Option<f32>| t.map_or_else(|| "not yet".to_owned(), |t| format!("{t:.1} s"));
        let (tex, tex_total) = self.textures_count;
        format!(
            "appearance {}, bakes {}, worn objects {}, geometry {}, ready {}, worn textures {} ({tex} of {tex_total})",
            at(self.appearance),
            at(self.bakes),
            at(self.worn),
            at(self.geometry),
            at(self.ready),
            at(self.textures),
        )
    }
}

/// One frame of an avatar's loading state.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Tick {
    /// Seconds since the avatar was first seen.
    waited: f32,
    /// Worn objects the simulator lists have not come yet.
    waiting: bool,
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
    fn new(now: Instant) -> State {
        State {
            first_seen: now,
            loading_at: now,
            shown_once: false,
            announced: 0,
            all_came: false,
            reported: false,
            shown: 0.0,
            timeline: Timeline::default(),
        }
    }

    /// updateIsFullyLoaded, the worn objects against the simulator's list
    /// (`announced`, 0 for our avatar: not compared), until the avatar has
    /// shown once: fewer than listed are waited for. LL gives up 60 s after
    /// the last one came; here MAX_WAIT ends the wait, sooner.
    fn waiting_for_worn(&mut self, worn: usize, announced: usize) -> bool {
        // processAvatarAppearance: a list of another size starts again
        if self.announced != announced {
            self.announced = announced;
            self.all_came = false;
        }
        if self.shown_once || self.all_came || announced == 0 {
            return false;
        }
        self.all_came = worn >= announced;
        !self.all_came
    }

    /// processFullyLoadedChange: once nothing is missing (`complete`) and
    /// the worn objects the simulator lists have come, 1 s to settle (late
    /// prims of a linkset, the first LODs); LL's 3..15 s delay (x1.25 for
    /// impostors) only applies while still loading, when the timer is
    /// reset anyway. After MAX_WAIT the avatar shows whatever is missing.
    ///
    /// The first time, the Legacy Viewer waits 3 s (FIRST_APPEARANCE_CLOUD_
    /// MIN_DELAY) for the textures of what it wears, which it does not wait
    /// for otherwise. Here the avatar shows as soon as they all show
    /// something (`textures`: shown or unavailable), 3 s at most: never
    /// later than the Legacy Viewer, sooner when they come from the cache,
    /// and in cold-cache arrivals at KB on 2026-10-27, almost every avatar
    /// showed after its 3 s with some of them still missing.
    fn tick(&mut self, now: Instant, worn: usize, announced: usize, complete: bool, textures: bool) -> Tick {
        let waiting = self.waiting_for_worn(worn, announced);
        let waited = now.duration_since(self.first_seen).as_secs_f32();
        let expecting = !complete || waiting;
        let still = expecting && waited < MAX_WAIT;
        if still {
            self.loading_at = now;
        }
        let delay = if self.shown_once { LOADED_DELAY } else { FIRST_MIN_DELAY };
        let since = now.duration_since(self.loading_at).as_secs_f32();
        let settled = since > delay || (!self.shown_once && textures && since > LOADED_DELAY);
        let loaded = !still && (settled || waited > MAX_WAIT);
        let first_show = loaded && !self.shown_once;
        if loaded {
            self.shown_once = true;
        }
        Tick {
            waited,
            waiting,
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
    /// (empty when complete) and, when asked for, the textures to hurry
    /// (those of its worn objects: every face is looked up, so only for an
    /// avatar not shown yet).
    fn avatar_load_parts(&self, world: &World, idx: usize, mut hurry: Option<&mut Vec<Uuid>>) -> (u32, u32, Vec<Missing>) {
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
                        if let Some(h) = hurry.as_deref_mut() {
                            h.push(t);
                        }
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
            let prims = std::iter::once(att).chain(world.objects.children_of(&root.key).iter().copied());
            for p in prims {
                let Some(g) = self.gpu.get(p) else {
                    continue;
                };
                total += 1;
                // geometry asked for and not there yet; why is only looked
                // up for those (this runs for every worn prim, every frame)
                let pending = g.wanted_geom.is_some_and(|k| Some(k) != g.geom) || g.sculpt_wait.is_some();
                let wait = if pending {
                    let unavailable = match g.wanted_geom {
                        Some(GeomKey::Mesh { id, .. }) => self.meshes.failed(&id),
                        Some(GeomKey::Sculpt { .. }) => g.sculpt_wait.is_some_and(|t| self.textures.is_unavailable(&t)),
                        _ => false,
                    };
                    let build_failed = g.wanted_geom.is_some_and(|k| self.geom_failed(&k));
                    geom_wait(pending, unavailable, build_failed)
                } else {
                    GeomWait::Built
                };
                if wait == GeomWait::Pending {
                    missing.push(Missing::Geometry(p));
                } else {
                    done += 1;
                }
                // their textures first (the avatar is not drawn meanwhile)
                if let Some(h) = hurry.as_deref_mut() {
                    for f in &g.faces {
                        if !f.tex_id.is_nil() && !self.textures.is_loaded(&f.tex_id) {
                            h.push(f.tex_id);
                        }
                    }
                }
            }
        }
        (done, total.max(1), missing)
    }

    /// What the progress bar under a loading avatar's name shows (0..1),
    /// weighted (`BAR_*`): its bakes downloaded, the geometry of what it
    /// wears built, the textures on what it wears (not waited for, as in
    /// the Legacy Viewer, but they are part of what shows) and the worn
    /// objects the simulator lists (`announced`) that have not come yet.
    /// Every part counts, so the bar moves as they come instead of
    /// waiting at its last step for a part it did not count. Also the
    /// textures on what it wears: (shown or unavailable, total).
    fn avatar_load_progress(&self, world: &World, idx: usize, announced: usize) -> (f32, usize, usize) {
        let Some(av) = world.objects.get(idx) else {
            return (0.0, 0, 0);
        };
        let (mut done, mut total) = (0.0f32, 0.0f32);
        for (te, name, _) in avatar::BAKES {
            let main = matches!(name, "head" | "upper" | "lower");
            match bake_need(world.bake_texture(idx, te), main) {
                BakeNeed::Texture(t) if self.textures.is_wanted(&t) => {
                    total += BAR_BAKE;
                    if self.textures.is_loaded(&t) {
                        done += BAR_BAKE;
                    }
                }
                BakeNeed::Undefined => total += BAR_BAKE,
                _ => {}
            }
        }
        let mut textures = std::collections::HashSet::new();
        let mut worn = 0;
        for &att in world.objects.children_of(&av.key) {
            let Some(root) = world.objects.get(att) else {
                continue;
            };
            if (31..=38).contains(&root.attachment_point()) {
                continue; // HUD
            }
            worn += 1;
            let prims = std::iter::once(att).chain(world.objects.children_of(&root.key).iter().copied());
            for p in prims {
                let Some(g) = self.gpu.get(p) else {
                    continue;
                };
                total += BAR_GEOMETRY;
                let pending = !g.geometry_built();
                let given_up = g
                    .wanted_geom
                    .is_some_and(|k| self.geom_failed(&k) || matches!(k, GeomKey::Mesh { id, .. } if self.meshes.failed(&id)));
                if !pending || given_up {
                    done += BAR_GEOMETRY;
                }
                textures.extend(g.faces.iter().map(|f| f.tex_id).filter(|t| !t.is_nil()));
            }
        }
        let mut shown_textures = 0;
        for t in &textures {
            total += BAR_TEXTURE;
            if self.textures.is_settled(t) {
                done += BAR_TEXTURE;
                shown_textures += 1;
            }
        }
        total += BAR_GEOMETRY * announced.saturating_sub(worn) as f32;
        let progress = if total <= 0.0 { 1.0 } else { done / total };
        (progress, shown_textures, textures.len())
    }

    /// Whether nothing of an avatar is missing (its bakes downloaded, what
    /// it wears built): the rez effect's line over our own avatar waits
    /// for it after an appearance change.
    pub(super) fn avatar_complete(&self, world: &World, idx: usize) -> bool {
        self.avatar_load_parts(world, idx, None).2.is_empty()
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
            if !world.parcel_object_visible(idx) {
                continue;
            }
            // silhouettes are not clouded (LL: no cloud for too complex avatars)
            if self.too_complex.contains(&idx) {
                seen.push(o.full_id);
                continue;
            }
            let id = o.full_id;
            seen.push(id);
            let s = loading.states.entry(id).or_insert_with(|| State::new(now));
            // shown once, an avatar is never a cloud again: nothing below
            // would be used, and what it wears was looked up here every
            // frame (a club crowd wears hundreds of prims each)
            if s.shown_once {
                continue;
            }
            let own = id == world.agent_id;
            let worn = world.objects.children_of(&o.key).len();
            // (ours is not compared with the simulator's list: `!isSelf()`)
            let announced = if own { 0 } else { world.announced_attachments(&id) };
            // its missing bakes rank with the meshes it wears (`wait_px`),
            // the textures on what it wears just behind them: the geometry
            // is what it waits for, the textures only shorten the delay
            let mut wanted = Vec::new();
            let (done, total, missing) = self.avatar_load_parts(world, idx, Some(&mut wanted));
            let px = self.wait_px(idx);
            hurry.extend(wanted.into_iter().map(|t| {
                let bake = missing.iter().any(|m| matches!(m, Missing::Bake(_, b) if *b == t));
                (t, if bake { px } else { (px * 0.5).max(HURRY_PX) })
            }));
            // what has come of its parts, weighted, and of the textures on
            // what it wears (the first time, it shows once they are there)
            let (parts, tex, tex_total) = self.avatar_load_progress(world, idx, announced);
            let t = s.tick(now, worn, announced, missing.is_empty(), tex == tex_total);
            let appearance = !missing.contains(&Missing::Appearance);
            let tl = &mut s.timeline;
            Timeline::mark(&mut tl.appearance, appearance, t.waited);
            let bakes = appearance && !missing.iter().any(|m| matches!(m, Missing::Bake(..)));
            Timeline::mark(&mut tl.bakes, bakes, t.waited);
            Timeline::mark(&mut tl.worn, !t.waiting, t.waited);
            let geometry = !missing.iter().any(|m| matches!(m, Missing::Geometry(_)));
            Timeline::mark(&mut tl.geometry, geometry, t.waited);
            Timeline::mark(&mut tl.ready, !t.still, t.waited);
            // (for the log: ours has no list to compare with)
            let listed = || if own { String::new() } else { format!(" of {announced} listed") };
            let own = if own { " (own)" } else { "" };
            // diagnostic: still a cloud after a while
            if !t.loaded && t.waited > REPORT_AFTER && !s.reported {
                s.reported = true;
                log::info!(
                    "avatar {id}{own} still loading after {:.0} s: {done}/{total} parts, worn objects {worn}{} (waited for: {}), missing [{}]",
                    t.waited,
                    listed(),
                    t.waiting,
                    self.describe_missing(world, &missing)
                );
            }
            if t.timed_out {
                // shown by the time limit: what it was waiting for is the
                // cause to look at
                log::warn!(
                    "avatar {id}{own} shown by the {MAX_WAIT:.0} s limit after {:.1} s: {done}/{total} parts, worn objects {worn}{} (waited for: {}), still missing [{}]; {}",
                    t.waited,
                    listed(),
                    t.waiting,
                    self.describe_missing(world, &missing),
                    s.timeline.describe()
                );
            } else if t.first_show {
                // (the timeline tells which part held it back)
                log::info!(
                    "avatar {id}{own} shown after {:.1} s, worn objects {worn}{}; {}",
                    t.waited,
                    listed(),
                    s.timeline.describe()
                );
            }
            let tl = &mut s.timeline;
            tl.textures_count = (tex, tex_total);
            Timeline::mark(&mut tl.textures, tex == tex_total, t.waited);
            if !t.loaded && !s.shown_once {
                // the bar: its parts, then the delay before showing (it
                // fills as the avatar is about to show)
                let delay = if t.still { 0.0 } else { t.delay };
                let p = parts * BAR_PARTS + (1.0 - BAR_PARTS) * delay;
                s.shown = s.shown.max(p.min(0.99));
                loading.progress.insert(id, s.shown);
                loading.indices.insert(idx);
            }
        }
        // (ours stays between the region left by a teleport and the next
        // one: forgotten with its object, it was a cloud again for 6 to 8 s
        // at every arrival)
        loading.states.retain(|id, _| seen.contains(id) || *id == world.agent_id);
        self.loading = loading;
        for (t, px) in hurry {
            self.textures.note_usage(&t, px);
        }
    }

    /// Size on screen the textures a loading avatar waits for are asked
    /// for at (its missing bakes, the textures on what it wears): the rank
    /// of the meshes it wears (`fetch_priority::mesh` of the avatar's 2.2 m
    /// radius, `sync_plan::mesh_rank_radius`), as the Legacy Viewer sizes a
    /// bake by the avatar's whole pixel area (LLVOAvatar::
    /// addBakedTextureStats, BOOST_AVATAR_BAKED). A body is not drawn while
    /// it loads, so nothing else notes them: ranked at `HURRY_PX`, the bakes
    /// came last, after every worn mesh and most of the scenery (about 30 s
    /// into a cold-cache arrival at KB, the last part of half the avatars).
    fn wait_px(&self, idx: usize) -> f32 {
        let (Some(view), Some(g)) = (self.last_cull, self.gpu.get(idx)) else {
            return HURRY_PX;
        };
        let (centre, _) = g.sphere();
        (2.0 * view.pixel_size(centre, super::sync_plan::AVATAR_LOD_RADIUS)).max(HURRY_PX)
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
    fn timeline_keeps_the_last_completion() {
        let mut tl = Timeline::default();
        Timeline::mark(&mut tl.geometry, false, 0.5);
        assert_eq!(tl.geometry, None);
        Timeline::mark(&mut tl.geometry, true, 1.0);
        Timeline::mark(&mut tl.geometry, true, 1.5);
        assert_eq!(tl.geometry, Some(1.0));
        // a worn object arriving later: missing again, then complete later
        Timeline::mark(&mut tl.geometry, false, 2.0);
        Timeline::mark(&mut tl.geometry, true, 2.5);
        assert_eq!(tl.geometry, Some(2.5));
        tl.textures_count = (3, 4);
        let line = tl.describe();
        assert!(line.contains("geometry 2.5 s"), "{line}");
        assert!(line.contains("bakes not yet"), "{line}");
        assert!(line.contains("worn textures not yet (3 of 4)"), "{line}");
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
    fn shows_three_seconds_after_everything_is_there() {
        let t0 = Instant::now();
        let mut s = State::new(t0);
        // complete from the start, no worn object listed: 3 s
        let t = s.tick(t0, 4, 0, true, false);
        assert!(!t.still && !t.waiting && !t.loaded);
        let t = s.tick(t0 + secs(1.5), 4, 0, true, false);
        assert!(!t.loaded && t.delay > 0.0 && t.delay < 1.0);
        assert!(!s.tick(t0 + secs(2.9), 4, 0, true, false).loaded);
        let t = s.tick(t0 + secs(3.1), 4, 0, true, false);
        assert!(t.loaded && t.first_show && !t.timed_out);
        assert_eq!(t.delay, 1.0);
        // later frames: shown, no longer "first"
        let t = s.tick(t0 + secs(3.2), 4, 0, true, false);
        assert!(t.loaded && !t.first_show);
    }

    #[test]
    fn shows_after_one_second_once_its_worn_textures_are_there() {
        let t0 = Instant::now();
        let mut s = State::new(t0);
        assert!(!s.tick(t0, 4, 0, true, true).loaded);
        // textures there: 1 s to settle, not 3
        assert!(!s.tick(t0 + secs(0.9), 4, 0, true, true).loaded);
        let t = s.tick(t0 + secs(1.1), 4, 0, true, true);
        assert!(t.loaded && t.first_show);
        // a texture still missing: 3 s at most, as before
        let mut s = State::new(t0);
        assert!(!s.tick(t0 + secs(1.1), 4, 0, true, false).loaded);
        assert!(!s.tick(t0 + secs(2.9), 4, 0, true, false).loaded);
        assert!(s.tick(t0 + secs(3.1), 4, 0, true, false).loaded);
        // nothing expected yet: textures change nothing
        let mut s = State::new(t0);
        assert!(!s.tick(t0 + secs(2.0), 4, 0, false, true).loaded);
    }

    #[test]
    fn a_missing_part_restarts_the_delay() {
        let t0 = Instant::now();
        let mut s = State::new(t0);
        assert!(s.tick(t0 + secs(3.0), 0, 0, false, false).still);
        // complete at 3 s: not shown before 6 s
        assert!(!s.tick(t0 + secs(5.9), 0, 0, true, false).loaded);
        // a mesh changes LOD at 5.95 s: the 3 s start again
        assert!(s.tick(t0 + secs(5.95), 0, 0, false, false).still);
        assert!(!s.tick(t0 + secs(8.9), 0, 0, true, false).loaded);
        assert!(s.tick(t0 + secs(9.0), 0, 0, true, false).loaded);
    }

    #[test]
    fn worn_objects_the_simulator_lists_are_waited_for() {
        let t0 = Instant::now();
        let mut s = State::new(t0);
        // 3 listed, 1 here: the others are expected, however long ago the
        // last one came (the blind 2 s of before showed half-dressed avatars)
        let t = s.tick(t0, 1, 3, true, false);
        assert!(t.still && t.waiting);
        assert!(s.tick(t0 + secs(10.0), 1, 3, true, false).waiting);
        assert!(s.tick(t0 + secs(10.5), 2, 3, true, false).waiting);
        // the last one comes: nothing more to wait for, 3 s
        let t = s.tick(t0 + secs(12.0), 3, 3, true, false);
        assert!(!t.still && !t.waiting && !t.loaded);
        // (the delay runs from the last frame it was still loading: 10.5 s)
        assert!(!s.tick(t0 + secs(13.4), 3, 3, true, false).loaded);
        let t = s.tick(t0 + secs(13.6), 3, 3, true, false);
        assert!(t.loaded && t.first_show && !t.timed_out);
    }

    #[test]
    fn listed_worn_objects_that_never_come_end_with_the_time_limit() {
        let t0 = Instant::now();
        let mut s = State::new(t0);
        assert!(s.tick(t0, 0, 2, true, false).waiting);
        assert!(s.tick(t0 + secs(30.0), 1, 2, true, false).still);
        let t = s.tick(t0 + secs(59.9), 1, 2, true, false);
        assert!(t.still && t.waiting && !t.loaded);
        let t = s.tick(t0 + secs(60.1), 1, 2, true, false);
        assert!(t.loaded && t.first_show && t.timed_out);
    }

    #[test]
    fn worn_objects_that_all_came_once_are_not_counted_again() {
        let t0 = Instant::now();
        let mut s = State::new(t0);
        assert!(!s.tick(t0, 3, 3, true, false).waiting);
        // one is taken off before the avatar shows
        let t = s.tick(t0 + secs(1.0), 2, 3, true, false);
        assert!(!t.waiting && !t.still);
    }

    #[test]
    fn another_list_starts_the_count_again() {
        let t0 = Instant::now();
        let mut s = State::new(t0);
        assert!(!s.tick(t0, 2, 2, true, false).waiting);
        // a new appearance lists one more
        let t = s.tick(t0 + secs(1.0), 2, 3, true, false);
        assert!(t.waiting && t.still);
        assert!(!s.tick(t0 + secs(2.0), 3, 3, true, false).waiting);
    }

    #[test]
    fn own_avatar_and_unlisted_ones_do_not_wait_for_worn_objects() {
        let t0 = Instant::now();
        // (ours is given no list: `!isSelf()`)
        let mut s = State::new(t0);
        for (i, worn) in [0, 3, 7, 12].into_iter().enumerate() {
            let t = s.tick(t0 + secs(i as f32 * 0.5), worn, 0, true, false);
            assert!(!t.waiting && !t.still);
        }
        // 3 s after it was complete, whatever came since
        assert!(s.tick(t0 + secs(3.1), 12, 0, true, false).loaded);
    }

    #[test]
    fn worn_objects_coming_after_an_avatar_showed_do_not_hide_it() {
        let t0 = Instant::now();
        let mut s = State::new(t0);
        assert!(s.tick(t0 + secs(3.1), 2, 2, true, false).loaded);
        // a new outfit: listed and not here yet
        let t = s.tick(t0 + secs(10.0), 2, 5, true, false);
        assert!(t.loaded && !t.waiting && !t.first_show);
    }

    #[test]
    fn time_limit_shows_an_incomplete_avatar_and_says_so() {
        let t0 = Instant::now();
        let mut s = State::new(t0);
        let t = s.tick(t0 + secs(59.9), 0, 0, false, false);
        assert!(t.still && !t.loaded && !t.timed_out);
        let t = s.tick(t0 + secs(60.1), 0, 0, false, false);
        assert!(!t.still && t.loaded && t.first_show && t.timed_out);
        // reported once
        let t = s.tick(t0 + secs(60.2), 0, 0, false, false);
        assert!(t.loaded && !t.first_show && !t.timed_out);
    }

    #[test]
    fn once_shown_an_avatar_uses_the_short_delay() {
        let t0 = Instant::now();
        let mut s = State::new(t0);
        assert!(s.tick(t0 + secs(3.1), 0, 0, true, false).loaded);
        // a new bake to download at 10 s: loading again, 1 s after it is there
        let t = s.tick(t0 + secs(10.0), 0, 0, false, false);
        assert!(t.still && !t.loaded && !t.first_show);
        assert!(!s.tick(t0 + secs(10.9), 0, 0, true, false).loaded);
        let t = s.tick(t0 + secs(11.1), 0, 0, true, false);
        assert!(t.loaded && !t.first_show && !t.timed_out);
    }

    /// The demo region with our avatar dressed by AURORA_DEMO_CLOUD=self.
    fn cloud_self() -> (World, Scene, usize, usize) {
        let lib = Arc::new(AvatarLibrary::load());
        let mut world = World::new(lib.clone());
        world.agent_id = crate::demo::DEMO_AGENT;
        for ev in crate::demo::events()
            .into_iter()
            .chain(crate::demo::cloud_self_events(&lib.shape_params))
        {
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
        let (done, total, missing) = scene.avatar_load_parts(&world, own, Some(&mut hurry));
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
            visual_params: world.avatar_lib.shape_params.values_with(&[]),
            cof_version: 1,
            appearance_version: 1,
            hover_height: 0.0,
            attachments: 0,
        }));
        let (_, _, missing) = scene.avatar_load_parts(&world, loup, Some(&mut hurry));
        assert_eq!(missing, [Missing::Appearance; 3]);
    }

    #[test]
    fn own_avatar_is_not_a_cloud_again_after_a_teleport() {
        let (mut world, mut scene, _, _) = cloud_self();
        let own = world.agent_id;
        let t0 = Instant::now();
        scene.update_loading(&world, t0);
        assert!(scene.loading_avatars().contains_key(&own), "first appearance: a cloud");
        scene.update_loading(&world, t0 + secs(5.1));
        assert!(scene.loading_avatars().is_empty(), "{:?}", scene.loading_avatars());
        // the region left goes with its objects, ours included
        let handle = world.main_region.expect("demo region");
        world.apply(aurora_net::NetEvent::RegionRemoved { handle });
        assert!(world.objects.index_of_uuid(&own).is_none());
        scene.update_loading(&world, t0 + secs(20.0));
        // the destination sends everyone as new objects
        for ev in crate::demo::events()
            .into_iter()
            .chain(crate::demo::cloud_self_events(&world.avatar_lib.shape_params))
        {
            world.apply(ev);
        }
        scene.gpu.resize_with(world.objects.slots.len(), ObjGpu::default);
        scene.update_loading(&world, t0 + secs(23.0));
        let clouds = scene.loading_avatars();
        assert!(!clouds.contains_key(&own), "shown as it arrives");
        // the others are first appearances there, as in Firestorm
        assert!(clouds.contains_key(&crate::demo::DEMO_LOUP));
        // logged out: nothing is remembered for the next session
        world.reset();
        scene.update_loading(&world, t0 + secs(30.0));
        assert!(scene.loading.states.is_empty());
    }

    #[test]
    fn a_shown_avatar_is_not_a_cloud_again_when_a_part_goes_missing() {
        let (world, mut scene, own, _) = cloud_self();
        let id = world.agent_id;
        let t0 = Instant::now();
        scene.update_loading(&world, t0);
        scene.update_loading(&world, t0 + secs(5.1));
        assert!(!scene.loading_avatars().contains_key(&id));
        // what it wears changes LOD: its geometry is pending again
        let key = GeomKey::Mesh {
            id: crate::demo::CLOUD_MESH,
            lod: 3,
        };
        scene.gpu[worn(&world, own, 1)].wanted_geom = Some(key);
        scene.geom_insert(key, GeomState::Pending);
        assert!(!scene.avatar_complete(&world, own));
        scene.update_loading(&world, t0 + secs(6.0));
        assert!(!scene.loading_avatars().contains_key(&id));
        assert!(!scene.loading.indices.contains(&own));
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
        let (_, _, missing) = scene.avatar_load_parts(&world, own, Some(&mut hurry));
        assert_eq!(missing, [Missing::Geometry(mesh)]);
        assert!(scene.describe_missing(&world, &missing).contains("mesh"));
        // the asset server does not have it (404): not expected any more
        scene.meshes.insert_demo_unavailable(crate::demo::CLOUD_MESH);
        let (done, total, missing) = scene.avatar_load_parts(&world, own, Some(&mut hurry));
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
        let (done, total, missing) = scene.avatar_load_parts(&world, own, Some(&mut hurry));
        assert!(missing.is_empty(), "{missing:?}");
        assert_eq!(done, total);
    }
}
