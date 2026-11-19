//! Worn objects drawn from a cache while the region sends them. Aurora's
//! own: the Legacy Viewer draws an avatar's attachments only once the
//! region has sent them. At login our outfit took 15 to 74 s to be whole
//! (the region puts back about half of it by itself, `outfit.rs` asks for
//! the rest once it is quiet), and the objects another avatar wears came
//! 0.5 to 4 s after it (grid logs of 2026-11-19).
//!
//! What an avatar wears is kept ([`capture`]) in a sealed store file of its
//! own ([`file`]), written in the background once what it wears is whole
//! and changed. When the avatar is seen again, the objects it is known to
//! wear now are drawn from that file as copies, until the region's own
//! come and are drawn:
//! - ours: the objects the Current Outfit folder links to, by inventory
//!   item;
//! - another avatar's: the objects its appearance lists
//!   (LLVOAvatar::mSimAttachments), by object id. An object's id changes
//!   each time it is attached, so only an avatar still logged in since it
//!   was kept is matched: a copy is never of something it no longer wears.
//!
//! A copy is a client-only object (`objects::PREVIEW_REGION`, like the
//! mirror and Blender Live's previews: never clicked, picked, selected or
//! listed) under the real avatar, without the AttachItemID name-value, so
//! that the outfit restore, RLVa and the attachment counts never take it
//! for a worn object. It goes when the region's object is drawn (5 s at
//! most after it came), when the avatar no longer wears it, when the
//! avatar leaves or changes region, or when the time it was waited for is
//! over. Nothing about a copy is sent to the region.

pub mod capture;
pub mod file;

use super::World;
use super::objects::{ObjKey, PREVIEW_REGION};
use file::{WornObject, WornSet};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

/// The copies' local ids: below the mirror's (`mirror::FIRST_LOCAL`), far
/// above Blender Live's.
const FIRST_LOCAL: u32 = 0xFFE0_0000;
const LAST_LOCAL: u32 = 0xFFEF_FFFF;
/// How long after an avatar is first seen copies of what it wears may
/// still be drawn: ours waits for the outfit restore (74 s on a busy
/// region), another's objects come in seconds.
const WINDOW_OWN: Duration = Duration::from_secs(120);
const WINDOW: Duration = Duration::from_secs(60);
/// A copy whose object came stays until that object is drawn, at most this.
const HANDOVER: Duration = Duration::from_secs(5);
/// How often what the avatars wear is looked at, and how many are written
/// at most each time.
const CAPTURE_EVERY: Duration = Duration::from_secs(15);
const CAPTURES_PER_PASS: usize = 8;
/// A file older than this is not used (what was worn a month ago).
const MAX_AGE: Duration = Duration::from_secs(30 * 24 * 3600);

enum Job {
    Read(Uuid, PathBuf),
    Write(PathBuf, Vec<u8>),
}

/// The store files, read and written by a thread of their own.
struct Io {
    dir: PathBuf,
    jobs: Sender<Job>,
    // (behind a lock: the world is shared with the frame's worker threads)
    read: std::sync::Mutex<Receiver<(Uuid, Option<WornSet>)>>,
}

impl Io {
    fn start(dir: PathBuf) -> Option<Io> {
        let (jobs, todo) = std::sync::mpsc::channel::<Job>();
        let (done, read) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("aurora-worn-cache".into()).spawn(move || {
            for job in todo {
                match job {
                    Job::Read(avatar, path) => {
                        let set = crate::cache::read_touch(&path).ok().and_then(|d| file::decode(&d));
                        if done.send((avatar, set)).is_err() {
                            return;
                        }
                    }
                    Job::Write(path, bytes) => {
                        if let Some(parent) = path.parent()
                            && let Err(e) = std::fs::create_dir_all(parent)
                        {
                            log::warn!("worn objects cache: cannot create its folder: {e}");
                            continue;
                        }
                        if let Err(e) = crate::cache::write(&path, &bytes) {
                            log::warn!("worn objects cache: cannot write a file: {e}");
                        }
                    }
                }
            }
        });
        match spawned {
            Ok(_) => Some(Io {
                dir,
                jobs,
                read: std::sync::Mutex::new(read),
            }),
            Err(e) => {
                log::warn!("worn objects cache: cannot start its thread: {e}");
                None
            }
        }
    }

    fn path(&self, avatar: &Uuid) -> PathBuf {
        crate::cache::path(&self.dir, "objects", avatar, "worn")
    }
}

/// A copy drawn for an avatar.
#[derive(Debug, Clone)]
struct Copy {
    /// The avatar it hangs from, as it was then: a copy goes when its
    /// avatar changes key (another region).
    avatar: ObjKey,
    root: ObjKey,
    /// What it stands for: the root's id when kept, its inventory item.
    root_id: Uuid,
    item_id: Uuid,
    /// When the region's object came.
    real_since: Option<Instant>,
}

#[derive(Default)]
pub struct WornCache {
    enabled: bool,
    io: Option<Io>,
    /// The files read: none when there was none (or it was too old).
    sets: HashMap<Uuid, Option<Arc<WornSet>>>,
    first_seen: HashMap<Uuid, Instant>,
    copies: HashMap<Uuid, Vec<Copy>>,
    /// What was last written for each avatar ([`capture::signature`]).
    saved: HashMap<Uuid, u64>,
    next_local: u32,
    last_capture: Option<Instant>,
    /// Counts for the log line at the end of a login's restore.
    drawn: usize,
    handed_over: usize,
}

impl WornCache {
    /// Use the store at `dir` (the asset cache folder), or none.
    pub fn set_store(&mut self, dir: Option<PathBuf>) {
        self.io = dir.and_then(Io::start);
    }

    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// Copies drawn now, for all avatars.
    pub fn copy_count(&self) -> usize {
        self.copies.values().map(Vec::len).sum()
    }

    /// Give a set read elsewhere (the demo, the tests).
    pub fn insert_set(&mut self, avatar: Uuid, set: WornSet) {
        self.sets.insert(avatar, Some(Arc::new(set)));
    }

    fn alloc_local(&mut self, taken: impl Fn(u32) -> bool) -> Option<u32> {
        let span = LAST_LOCAL - FIRST_LOCAL + 1;
        for _ in 0..span {
            if !(FIRST_LOCAL..=LAST_LOCAL).contains(&self.next_local) {
                self.next_local = FIRST_LOCAL;
            }
            let id = self.next_local;
            self.next_local += 1;
            if !taken(id) {
                return Some(id);
            }
        }
        None
    }
}

/// A prim of a copy drawn from the cache (other client previews are not).
pub fn is_copy(key: ObjKey) -> bool {
    key.region == PREVIEW_REGION && (FIRST_LOCAL..=LAST_LOCAL).contains(&key.local_id)
}

/// The id of the copy of the object `id`: fixed, like the mirror's.
pub fn copy_id(id: Uuid) -> Uuid {
    Uuid::from_u128(id.as_u128() ^ 0x776f_726e_0000_0000_0000_0000_0000_0001)
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Whether a set written at `saved_at` is still used at `now` (Unix seconds).
fn fresh(saved_at: u64, now: u64) -> bool {
    now.saturating_sub(saved_at) <= MAX_AGE.as_secs()
}

/// Which of the kept objects are wanted now: for us, those whose item the
/// outfit links to; for another avatar, those whose id its appearance lists.
/// `present` tells whether the region's object of a kept one is there.
pub fn wanted<'a>(
    set: &'a WornSet,
    own: bool,
    outfit: &HashSet<Uuid>,
    listed: &HashSet<Uuid>,
    present: impl Fn(&WornObject) -> bool,
) -> Vec<&'a WornObject> {
    set.objects
        .iter()
        .filter(|o| {
            if own {
                !o.item_id.is_nil() && outfit.contains(&o.item_id)
            } else {
                listed.contains(&o.root_id)
            }
        })
        .filter(|o| !present(o))
        .collect()
}

impl World {
    /// Read, draw, hand over and keep what the avatars wear (every frame).
    pub fn update_worn_cache(&mut self, now: Instant) {
        if !self.worn_cache.enabled {
            if self.worn_cache.copy_count() > 0 {
                let all: Vec<Copy> = self.worn_cache.copies.drain().flat_map(|(_, v)| v).collect();
                for c in all {
                    self.remove_worn_copy(c.root);
                }
            }
            return;
        }
        self.read_worn_sets(now);
        self.draw_worn_copies(now);
        self.retire_worn_copies(now);
        self.capture_worn_sets(now);
    }

    /// The avatars in the world a copy can hang from (not the viewer's own
    /// previews, the mirror among them).
    fn worn_cache_avatars(&self) -> Vec<(usize, Uuid, ObjKey)> {
        self.objects
            .avatars()
            .filter(|(_, o)| !o.is_client_preview())
            .map(|(i, o)| (i, o.full_id, o.key))
            .collect()
    }

    fn read_worn_sets(&mut self, now: Instant) {
        let cache = &mut self.worn_cache;
        if let Some(io) = &cache.io
            && let Ok(read) = io.read.lock()
        {
            let today = unix_now();
            while let Ok((avatar, set)) = read.try_recv() {
                let set = set.filter(|s| fresh(s.saved_at, today)).map(Arc::new);
                cache.sets.insert(avatar, set);
            }
        }
        let avatars = self.worn_cache_avatars();
        let present: HashSet<Uuid> = avatars.iter().map(|a| a.1).collect();
        let cache = &mut self.worn_cache;
        // an avatar gone is seen for the first time again when it comes back
        cache.first_seen.retain(|id, _| present.contains(id));
        for (_, id, _) in avatars {
            cache.first_seen.entry(id).or_insert(now);
            if !cache.sets.contains_key(&id)
                && let Some(io) = &cache.io
            {
                let path = io.path(&id);
                if io.jobs.send(Job::Read(id, path)).is_ok() {
                    // (none until it is read)
                    cache.sets.insert(id, None);
                }
            }
        }
    }

    /// The objects our Current Outfit folder links to, while it is known.
    fn worn_cache_outfit(&self) -> Option<HashSet<Uuid>> {
        super::outfit::current_outfit_objects(&self.inventory).map(|v| v.into_iter().collect())
    }

    fn draw_worn_copies(&mut self, now: Instant) {
        let outfit = self.worn_cache_outfit().unwrap_or_default();
        let worn_items = self.outfit_attachment_items();
        for (_, id, key) in self.worn_cache_avatars() {
            let own = id == self.agent_id;
            let Some(Some(set)) = self.worn_cache.sets.get(&id).cloned() else {
                continue;
            };
            let window = if own { WINDOW_OWN } else { WINDOW };
            if self
                .worn_cache
                .first_seen
                .get(&id)
                .is_none_or(|t| now.saturating_duration_since(*t) > window)
            {
                continue;
            }
            let listed: HashSet<Uuid> = if own {
                HashSet::new()
            } else {
                self.listed_attachments(&id).iter().map(|(object, _)| *object).collect()
            };
            let shown: HashSet<Uuid> = self
                .worn_cache
                .copies
                .get(&id)
                .map(|v| v.iter().map(|c| c.root_id).collect())
                .unwrap_or_default();
            let objects = &self.objects;
            let todo: Vec<WornObject> = wanted(&set, own, &outfit, &listed, |o| {
                shown.contains(&o.root_id)
                    || if own {
                        worn_items.contains(&o.item_id)
                    } else {
                        objects.index_of_uuid(&o.root_id).is_some()
                    }
            })
            .into_iter()
            .cloned()
            .collect();
            for object in todo {
                if let Some(root) = self.insert_worn_copy(key, &object) {
                    self.worn_cache.drawn += 1;
                    self.worn_cache.copies.entry(id).or_default().push(Copy {
                        avatar: key,
                        root,
                        root_id: object.root_id,
                        item_id: object.item_id,
                        real_since: None,
                    });
                }
            }
        }
    }

    /// Draw a copy of `object` on the avatar `avatar`; its root's key.
    fn insert_worn_copy(&mut self, avatar: ObjKey, object: &WornObject) -> Option<ObjKey> {
        // (never a HUD, even from a file of another version: its prims
        // would be picked and selected on the screen)
        if capture::is_hud_point(object.point) {
            return None;
        }
        let mut locals: HashMap<u32, u32> = HashMap::new();
        let mut root = None;
        for (i, prim) in object.prims.iter().enumerate() {
            let Some(mut u) = aurora_net::objects::parse_compressed(&prim.data, prim.update_flags) else {
                if i == 0 {
                    return None;
                }
                continue;
            };
            let parent = if i == 0 {
                avatar
            } else {
                // (a child whose parent was not drawn is left out)
                let Some(&parent) = locals.get(&u.parent_id) else { continue };
                ObjKey {
                    region: PREVIEW_REGION,
                    local_id: parent,
                }
            };
            let objects = &self.objects;
            let local = self.worn_cache.alloc_local(|id| {
                objects
                    .index_of(&ObjKey {
                        region: PREVIEW_REGION,
                        local_id: id,
                    })
                    .is_some()
            })?;
            locals.insert(u.local_id, local);
            u.local_id = local;
            u.parent_id = parent.local_id;
            u.full_id = copy_id(u.full_id);
            // (what capture leaves out, again: a file of another version)
            u.update_flags = 0;
            u.click_action = 0;
            u.name_values.clear();
            u.text.clear();
            u.media_url.clear();
            u.sound = Default::default();
            u.particles = aurora_net::objects::ParticleUpdate::Clear;
            self.objects.upsert_preview(Some(parent), u);
            let key = ObjKey {
                region: PREVIEW_REGION,
                local_id: local,
            };
            let sides = capture::read_overrides(&prim.overrides);
            if !sides.is_empty() {
                self.gltf_overrides.insert(key, sides.into());
            }
            if i == 0 {
                root = Some(key);
            }
        }
        root
    }

    /// Take a copy away, with its prims.
    fn remove_worn_copy(&mut self, root: ObjKey) {
        let Some(idx) = self.objects.index_of(&root) else {
            return;
        };
        let subtree = self.objects.subtree(idx);
        let keys: Vec<ObjKey> = subtree.iter().filter_map(|&i| self.objects.get(i).map(|o| o.key)).collect();
        self.objects.killed.push(subtree);
        for key in keys {
            self.objects.remove(&key);
            self.gltf_overrides.remove(&key);
        }
    }

    /// Whether the region's object `root` is drawn (each of its prims has
    /// its geometry), so that its copy can go without a gap.
    fn worn_object_drawn(&self, root: usize) -> bool {
        self.objects
            .subtree(root)
            .iter()
            .filter_map(|&i| self.objects.get(i))
            .all(|o| o.render.geom.is_some() && o.render.pending_geom.is_none())
    }

    fn retire_worn_copies(&mut self, now: Instant) {
        if self.worn_cache.copies.is_empty() {
            return;
        }
        let outfit = self.worn_cache_outfit();
        let worn_items: HashMap<Uuid, usize> = self
            .objects
            .index_of_uuid(&self.agent_id)
            .and_then(|i| self.objects.get(i))
            .map(|me| {
                self.objects
                    .children_of(&me.key)
                    .iter()
                    .filter_map(|&i| self.objects.get(i).and_then(|o| o.attachment_item_id()).map(|item| (item, i)))
                    .collect()
            })
            .unwrap_or_default();
        let mut gone = Vec::new();
        let mut handed = 0;
        let mut copies = std::mem::take(&mut self.worn_cache.copies);
        for (avatar, list) in &mut copies {
            let own = *avatar == self.agent_id;
            let current = self.objects.index_of_uuid(avatar).and_then(|i| self.objects.get(i)).map(|o| o.key);
            let window = if own { WINDOW_OWN } else { WINDOW };
            let late = self
                .worn_cache
                .first_seen
                .get(avatar)
                .is_none_or(|t| now.saturating_duration_since(*t) > window);
            let listed: Option<HashSet<Uuid>> = (!own).then(|| self.listed_attachments(avatar).iter().map(|(o, _)| *o).collect());
            list.retain_mut(|c| {
                let real = if own {
                    worn_items.get(&c.item_id).copied()
                } else {
                    self.objects.index_of_uuid(&c.root_id)
                };
                if let Some(real) = real {
                    let since = *c.real_since.get_or_insert(now);
                    if self.worn_object_drawn(real) || now.saturating_duration_since(since) >= HANDOVER {
                        handed += 1;
                        gone.push(c.root);
                        return false;
                    }
                    return true;
                }
                let still_worn = if own {
                    outfit.as_ref().is_none_or(|o| o.contains(&c.item_id))
                } else {
                    listed.as_ref().is_some_and(|l| l.contains(&c.root_id))
                };
                let keep = current == Some(c.avatar) && self.objects.index_of(&c.root).is_some() && still_worn && !late;
                if !keep {
                    gone.push(c.root);
                }
                keep
            });
        }
        copies.retain(|_, list| !list.is_empty());
        self.worn_cache.copies = copies;
        self.worn_cache.handed_over += handed;
        for root in gone {
            self.remove_worn_copy(root);
        }
        if self.worn_cache.copies.is_empty() && self.worn_cache.drawn > 0 {
            log::info!(
                "worn objects cache: {} copies drawn, {} handed over to the region's objects",
                self.worn_cache.drawn,
                self.worn_cache.handed_over
            );
            self.worn_cache.drawn = 0;
            self.worn_cache.handed_over = 0;
        }
    }

    /// Keep what the avatars wear, once it is whole and changed.
    fn capture_worn_sets(&mut self, now: Instant) {
        if self.worn_cache.io.is_none() {
            return;
        }
        if self
            .worn_cache
            .last_capture
            .is_some_and(|t| now.saturating_duration_since(t) < CAPTURE_EVERY)
        {
            return;
        }
        self.worn_cache.last_capture = Some(now);
        let today = unix_now();
        let mut written = 0;
        for (idx, id, _) in self.worn_cache_avatars() {
            if written >= CAPTURES_PER_PASS {
                break;
            }
            if self.worn_cache.copies.contains_key(&id) || !self.worn_set_whole(idx, id) {
                continue;
            }
            let signature = capture::signature(self, idx);
            if self.worn_cache.saved.get(&id) == Some(&signature) {
                continue;
            }
            let set = capture::capture(self, idx, today);
            let bytes = file::encode(&set);
            let Some(io) = &self.worn_cache.io else { return };
            if io.jobs.send(Job::Write(io.path(&id), bytes)).is_ok() {
                self.worn_cache.saved.insert(id, signature);
                // what a return to this avatar later in the session draws
                self.worn_cache.sets.insert(id, Some(Arc::new(set)));
                written += 1;
            }
        }
    }

    /// Whether what the avatar wears is all there: ours once the outfit
    /// restore is over, another's once every object its appearance lists
    /// has come.
    fn worn_set_whole(&self, idx: usize, id: Uuid) -> bool {
        if id == self.agent_id {
            return self.outfit.is_done();
        }
        if !self.listed_worn.contains_key(&id) {
            return false;
        }
        let roots: HashSet<Uuid> = capture::worn_roots(self, idx)
            .into_iter()
            .filter_map(|i| self.objects.get(i).map(|o| o.full_id))
            .collect();
        self.listed_attachments(&id)
            .iter()
            .all(|(object, point)| capture::is_hud_point(*point) || roots.contains(object))
    }
}

#[cfg(test)]
mod tests;
