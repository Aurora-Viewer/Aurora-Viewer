//! What the cursor is over, without searching the scene every frame.
//!
//! The hover cursor (touch, sit, buy…) needs the object under the pointer
//! in every frame (LLToolPie::handleHover asks the pick of the frame). The
//! search itself (`Scene::interaction_point` then
//! `Scene::interaction_at_ray`, picking.rs) walked every object three
//! times: ~0.5 ms a frame in a region of 16,000 objects, which is why the
//! frame rate rose while Alt, Ctrl or Shift (no hover cursor) was held.
//!
//! Two things make it cost nothing when nothing changes, and little when
//! something does:
//!
//! - [`PickIndex`]: the bounding sphere of every object in one compact
//!   array, kept by the scene sync. One pass over it gives the few objects
//!   the search can meet: those near the cursor ray, or around the depth
//!   point (read back from the GPU a frame or two later, so it lags a
//!   moving cursor). The exact tests (triangles, click actions,
//!   transparency) then run on those only, in the same order, so the
//!   answer is the one the full search gives.
//! - [`HoverCache`]: the answer is kept while the ray, the depth under the
//!   cursor (anything drawn there that moves changes it) and the objects
//!   around them stay the same. The index reports every object added,
//!   removed, moved or changed there. What it cannot see (a slow drift,
//!   an animated attachment, the agent sitting down, the root of a linkset
//!   changing its flags) is caught by searching again at least every
//!   [`REFRESH`].
//!
//! Among avatars that was not enough: what they wear moves at each frame,
//! so the search runs at each frame, and it went through every rigged mesh
//! of the region each time (they are tested whatever the ray), three
//! times: 0.6 ms a search with 10,000 of them. Hence:
//!
//! - the classes ([`class`]): what the search needs to know of an object
//!   tested whatever the ray to leave it out of the steps it cannot count
//!   in, worked out from the object once and again when it changes;
//! - the triangle hierarchies of the large faces (pick_accel.rs), for the
//!   objects the ray does meet.
//!
//! Clicks never read the cache: they search their geometry again, with
//! their own depth read (`app/click_pick.rs`).
//!
//! `AURORA_HOVER_CHECK=1` compares every answer with the full search and
//! logs the differences (`hover check`), with a count of searches and
//! reused answers every ten seconds; it also compares the index and the
//! classes with the scene. `AURORA_HOVER_LEGACY=1` searches as before the
//! classes and the hierarchies, `ab` both ways in turn. With
//! `AURORA_PROFILE`, a `hover summary` line a second tells what the
//! searches did and why they were needed.

use super::ObjGpu;
use glam::Vec3;
use std::time::{Duration, Instant};

/// A kept answer is searched again after this long whatever happened.
pub const REFRESH: Duration = Duration::from_millis(100);

/// A move or a resize smaller than this (m) between two syncs does not
/// disturb a kept answer by itself: the idle sway of an avatar moves what
/// it wears by less at each frame. Anything drawn shows in the depth under
/// the cursor anyway, and the rest at the next refresh.
const STILL: f32 = 0.002;

/// What the index knows of an object: its bounding sphere and how the
/// search treats it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickEntry {
    center: Vec3,
    /// Negative: nothing to pick (no geometry, or a HUD attachment).
    radius: f32,
    /// Tested whatever the ray: avatars (their own box test), rigged meshes
    /// (the exact tests do not trust their bounds), objects without bounds.
    always: bool,
    /// An avatar, or worn by one: what the class of the object reads of
    /// the scene's side to tell whether the search for the object at a
    /// point leaves it out (`class::OFF_POINT`).
    aside: bool,
}

/// The ray and the depth point of a search.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Probe {
    ray: (Vec3, Vec3),
    depth: Option<Vec3>,
}

impl PickEntry {
    pub const NEVER: PickEntry = PickEntry {
        center: Vec3::ZERO,
        radius: -1.0,
        always: false,
        aside: false,
    };

    /// The entry of an object as its last sync left it.
    pub fn of(g: &ObjGpu) -> PickEntry {
        // Fully transparent texture entries have CPU geometry but no draw
        // records. Selection still needs their bounds (pick_transparent).
        if (g.faces.is_empty() && g.geom.is_none()) || g.hud {
            PickEntry::NEVER
        } else if g.is_avatar {
            // (the extent the scene gives a wearer's rigged meshes)
            PickEntry {
                center: g.center,
                radius: 2.5,
                always: true,
                aside: true,
            }
        } else {
            PickEntry {
                center: g.center,
                radius: g.radius.max(0.0),
                always: g.rigged || g.radius <= 0.0,
                aside: g.owner_avatar.is_some(),
            }
        }
    }

    fn pickable(&self) -> bool {
        self.radius >= 0.0
    }

    /// What a change of this entry counts as in the profile.
    fn kind(&self) -> Disturbed {
        if self.always { Disturbed::Always } else { Disturbed::Bounded }
    }

    /// Whether the sphere is where the exact tests of picking.rs look for
    /// `probe`. Wider than each of them: `ray_may_hit` allows 5 % + 5 cm
    /// around the ray, `interaction_at_ray` the radius + 6 cm around the
    /// point it is given, which is the depth point or a point of the ray.
    fn near(&self, probe: &Probe) -> bool {
        let to = self.center - probe.ray.0;
        let along = to.dot(probe.ray.1);
        let r = self.radius * 1.05 + 0.15;
        (along >= -r && to.length_squared() - along * along <= r * r)
            || probe
                .depth
                .is_some_and(|p| self.center.distance_squared(p) <= (self.radius + 0.16).powi(2))
    }

    /// The search for `probe` must test this object.
    fn candidate(&self, probe: &Probe) -> bool {
        self.pickable() && (self.always || self.near(probe))
    }

    /// A change of this object can change the answer for `probe`.
    fn concerns(&self, probe: &Probe) -> bool {
        // (without bounds: anywhere)
        self.pickable() && (self.near(probe) || (self.always && self.radius == 0.0))
    }

    /// Too small a change to disturb a kept answer by itself.
    fn about(&self, o: &PickEntry) -> bool {
        self.pickable()
            && o.pickable()
            && self.always == o.always
            && self.aside == o.aside
            && self.center.distance_squared(o.center) <= STILL * STILL
            && (self.radius - o.radius).abs() <= STILL
    }
}

/// What disturbed the watched search first (profile).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Disturbed {
    /// An object with bounds moved, came or left near the ray.
    Bounded,
    /// The same for an object tested whatever the ray (avatar, rigged mesh).
    Always,
    /// An object there changed without moving (update, faces, alpha class).
    Changed,
    /// Everything went away.
    Reset,
}

/// What the hover search knows of an object tested whatever the ray
/// ([`PickEntry::always`]), so that it leaves out those that cannot change
/// its answer: in a crowd they are thousands, the rigged meshes the
/// avatars wear, and going through them all for each search was its cost.
///
/// Such an object counts in three places only: set to IGNORE, its
/// triangles can stop the ray before the depth point; with a blended face,
/// it can be an actionable surface absent from the GPU depth; and near the
/// point found, it can be the object there, unless it is an avatar or a
/// mesh worn by one, which that step leaves out. (And everywhere once an
/// ignored object is in the way: that search takes them all, as before.)
/// A dressed avatar is a few hundred objects with none of the three: the
/// search does not look at them, and their moves do not make it search
/// again.
///
/// A class is worked out from the object as it is (`Scene::hover_class`)
/// at the first search after the object appeared or changed: the index
/// forgets it on every `changed` and on a change of kind, and the search
/// forgets those of the objects updated and of the textures reclassified
/// since the last sync.
mod class {
    /// The class is known (else the object is a candidate everywhere).
    pub const KNOWN: u8 = 1;
    /// Click action IGNORE.
    pub const IGNORE: u8 = 2;
    /// A blended face (and not an avatar).
    pub const SEE_THROUGH: u8 = 4;
    /// Unknown and already in the list of classes to work out.
    pub const QUEUED: u8 = 8;
    /// Left out by the search for the object at a point: an avatar, or a
    /// mesh worn by one.
    pub const OFF_POINT: u8 = 16;
}

/// Classes worked out per search at most: the others stay candidates
/// everywhere until their turn (a region arriving at once).
const CLASSES_PER_SEARCH: usize = 512;

/// Known classes verified again per search, in turn: whatever change the
/// index was not told of is repaired within seconds instead of never.
const CLASSES_SWEPT_PER_SEARCH: usize = 16;

/// The candidates of a hover search, in index order: for each of its
/// steps, the objects near the ray or the depth point, and those tested
/// whatever the ray whose class does not rule them out.
#[derive(Default)]
pub struct Candidates {
    /// For `Scene::ignored_before`.
    ignore: Vec<usize>,
    /// For `Scene::transparent_action_point`.
    see_through: Vec<usize>,
    /// For `Scene::interaction_at_ray_among`.
    point: Vec<usize>,
    /// For `Scene::point_behind_ignored`: filled when it is needed.
    all: Vec<usize>,
}

/// Compact copy of the objects' bounds (indexed like the object slab) and
/// the search whose surroundings are watched for changes.
#[derive(Default)]
pub struct PickIndex {
    entries: Vec<PickEntry>,
    /// [`class`] bits of the entries (same length).
    classes: Vec<u8>,
    /// Entries whose class is to work out (each has `class::QUEUED`).
    unresolved: Vec<u32>,
    /// Next entry whose known class is verified again.
    sweep: usize,
    /// Ray and depth point of the answer kept by the cache.
    watch: Option<Probe>,
    /// The watched search used the classes and met no ignored object: the
    /// objects its classes left out everywhere can move without changing
    /// its answer.
    watch_classes: bool,
    /// Something changed around them since.
    disturbed: bool,
    /// What it was (the first change since the watch began).
    disturbed_by: Option<Disturbed>,
    /// Moves near the watched search that did not disturb it, of objects
    /// left out everywhere (profile).
    left_out_moves: u32,
}

impl PickIndex {
    /// A separate query for inventory placement; do not disturb hover's watch.
    pub fn ray_candidates(&self, ray: (Vec3, Vec3), out: &mut Vec<usize>) {
        self.candidates(&Probe { ray, depth: None }, out);
    }

    /// The objects whose bounds reach the box `min`..`max`, those without
    /// bounds included (`foot_support`).
    pub fn box_candidates(&self, min: Vec3, max: Vec3, out: &mut Vec<usize>) {
        out.clear();
        out.extend(
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.pickable() && (e.always || e.center.clamp(min, max).distance_squared(e.center) <= e.radius * e.radius))
                .map(|(i, _)| i),
        );
    }
    /// The objects whose bounds come within `pad` of one of `segments`
    /// (at most 64), each with a bit per segment it is near (sound
    /// occlusion). Never avatars, what they wear, rigged meshes nor objects
    /// without bounds: none of them blocks a sound.
    pub fn segment_candidates(&self, segments: &[(Vec3, Vec3)], pad: f32, out: &mut Vec<(usize, u64)>) {
        out.clear();
        let Some(&(first, _)) = segments.first() else {
            return;
        };
        let segments = &segments[..segments.len().min(64)];
        let (lo, hi) = segments
            .iter()
            .fold((first, first), |(lo, hi), (a, b)| (lo.min(*a).min(*b), hi.max(*a).max(*b)));
        for (i, e) in self.entries.iter().enumerate() {
            if !e.pickable() || e.always || e.aside {
                continue;
            }
            let r = e.radius * 1.05 + pad;
            if e.center.cmplt(lo - Vec3::splat(r)).any() || e.center.cmpgt(hi + Vec3::splat(r)).any() {
                continue;
            }
            let mut bits = 0u64;
            for (k, (a, b)) in segments.iter().enumerate() {
                let ab = *b - *a;
                let t = ((e.center - *a).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
                if (*a + ab * t).distance_squared(e.center) <= r * r {
                    bits |= 1 << k;
                }
            }
            if bits != 0 {
                out.push((i, bits));
            }
        }
    }

    fn note(&mut self, e: PickEntry, by: Disturbed) {
        if !self.disturbed && self.watch.is_some_and(|probe| e.concerns(&probe)) {
            self.disturbed = true;
            self.disturbed_by = Some(by);
        }
    }

    /// Whether the watched search left the object out of every step, by
    /// its class: wherever it goes, the answer stays the same.
    fn left_out(&self, idx: usize, e: &PickEntry) -> bool {
        let c = self.classes.get(idx).copied().unwrap_or(0);
        self.watch_classes
            && e.always
            && e.radius > 0.0
            && c & (class::KNOWN | class::IGNORE | class::SEE_THROUGH | class::OFF_POINT) == class::KNOWN | class::OFF_POINT
    }

    /// The object's bounds after a sync. A change around the watched
    /// search (where the object was, or where it is now) disturbs it.
    pub fn set(&mut self, idx: usize, e: PickEntry) {
        if self.entries.len() <= idx {
            if !e.pickable() {
                return;
            }
            self.entries.resize(idx + 1, PickEntry::NEVER);
            self.classes.resize(idx + 1, 0);
        }
        let old = std::mem::replace(&mut self.entries[idx], e);
        // (the class holds while only the bounds change)
        if old.always != e.always || old.pickable() != e.pickable() || old.aside != e.aside {
            self.forget_class(idx);
        }
        if old != e && !old.about(&e) {
            if self.left_out(idx, &old) && self.left_out(idx, &e) {
                if !self.disturbed && self.watch.is_some_and(|probe| old.concerns(&probe) || e.concerns(&probe)) {
                    self.left_out_moves += 1;
                }
                return;
            }
            self.note(old, old.kind());
            self.note(e, e.kind());
        }
    }

    /// The object changed in a way its bounds do not tell: updated by the
    /// simulator (click action, flags), faces rebuilt, alpha class.
    pub fn changed(&mut self, idx: usize) {
        if let Some(&e) = self.entries.get(idx) {
            self.forget_class(idx);
            self.note(e, Disturbed::Changed);
        }
    }

    /// The class of the object is to work out again before it is used.
    fn forget_class(&mut self, idx: usize) {
        let (Some(e), Some(c)) = (self.entries.get(idx), self.classes.get_mut(idx)) else {
            return;
        };
        if !(e.always && e.pickable()) {
            // (never read; a queued entry is dropped when its turn comes)
            *c &= class::QUEUED;
        } else if *c & class::QUEUED == 0 {
            *c = class::QUEUED;
            self.unresolved.push(idx as u32);
        } else {
            *c = class::QUEUED;
        }
    }

    /// The next entry whose class is to work out, if any.
    fn next_unresolved(&mut self) -> Option<usize> {
        loop {
            let idx = self.unresolved.pop()? as usize;
            let (Some(e), Some(c)) = (self.entries.get(idx), self.classes.get_mut(idx)) else {
                continue;
            };
            *c &= !class::QUEUED;
            if e.always && e.pickable() && *c & class::KNOWN == 0 {
                return Some(idx);
            }
        }
    }

    /// The next entry whose known class is verified again, if the entry
    /// at the sweep position has one.
    fn next_swept(&mut self) -> Option<(usize, u8)> {
        if self.entries.is_empty() {
            return None;
        }
        let idx = self.sweep % self.entries.len();
        self.sweep = idx + 1;
        let (e, c) = (self.entries[idx], self.classes[idx]);
        (e.always && e.pickable() && c & class::KNOWN != 0).then_some((idx, c))
    }

    fn set_class(&mut self, idx: usize, c: u8) {
        if let Some(slot) = self.classes.get_mut(idx) {
            *slot = c | (*slot & class::QUEUED);
        }
    }

    /// The object is gone.
    pub fn remove(&mut self, idx: usize) {
        self.set(idx, PickEntry::NEVER);
    }

    /// Everything is gone (teleport, logout).
    pub fn reset(&mut self) {
        self.entries.clear();
        self.classes.clear();
        self.unresolved.clear();
        if !self.disturbed {
            self.disturbed_by = Some(Disturbed::Reset);
        }
        self.disturbed = true;
    }

    /// The objects each step of a hover search for `probe` must test, in
    /// index order. An object with bounds: near the ray or the depth point,
    /// for every step. An object tested whatever the ray: for the steps its
    /// class leaves it in, wherever it is for the first two, near the ray
    /// or the depth point for the third (`interaction_at_ray` leaves out by
    /// itself an object whose sphere is away from the point); for all three
    /// without `classes`, as before the classes, while its class is not
    /// known, or if it has no bounds.
    fn hover_candidates(&self, probe: &Probe, classes: bool, out: &mut Candidates) {
        out.ignore.clear();
        out.see_through.clear();
        out.point.clear();
        for (i, (e, &c)) in self.entries.iter().zip(&self.classes).enumerate() {
            if !e.pickable() {
                continue;
            }
            if e.always && classes && c & class::KNOWN != 0 && e.radius > 0.0 {
                if c & class::IGNORE != 0 {
                    out.ignore.push(i);
                }
                if c & class::SEE_THROUGH != 0 {
                    out.see_through.push(i);
                }
                if c & class::OFF_POINT == 0 && e.near(probe) {
                    out.point.push(i);
                }
            } else if e.always || e.near(probe) {
                out.ignore.push(i);
                out.see_through.push(i);
                out.point.push(i);
            }
        }
    }

    /// The objects a search for `probe` must test, in index order, into `out`.
    fn candidates(&self, probe: &Probe, out: &mut Vec<usize>) {
        out.clear();
        out.extend(self.entries.iter().enumerate().filter(|(_, e)| e.candidate(probe)).map(|(i, _)| i));
    }

    /// Watch the surroundings of `probe` from now on.
    fn watch(&mut self, probe: Probe, classes: bool) {
        self.watch = Some(probe);
        self.watch_classes = classes;
        self.disturbed = false;
        self.disturbed_by = None;
    }
}

/// What a kept answer was computed from.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Asked {
    probe: Probe,
    far: f32,
}

impl Asked {
    /// The same question, up to what cannot change the answer: the eye
    /// within half a millimeter and the direction within 2 mm at 100 m (a
    /// smoothed camera settles asymptotically), the depth point within
    /// 2 mm plus a ten-thousandth of its distance (depth precision).
    fn same(&self, o: &Asked) -> bool {
        self.same_ray(o) && self.same_depth(o)
    }

    fn same_ray(&self, o: &Asked) -> bool {
        let (a, b) = (&self.probe, &o.probe);
        self.far == o.far && a.ray.0.distance(b.ray.0) <= 5e-4 && a.ray.1.distance(b.ray.1) <= 2e-5
    }

    fn same_depth(&self, o: &Asked) -> bool {
        let (a, b) = (&self.probe, &o.probe);
        match (a.depth, b.depth) {
            (None, None) => true,
            (Some(p), Some(q)) => p.distance(q) <= 0.002 + 1e-4 * p.distance(a.ray.0),
            _ => false,
        }
    }
}

/// What the hover search did over a second (AURORA_PROFILE: one
/// `hover summary` line a second while the cursor is over the world).
struct Stats {
    since: Option<Instant>,
    /// The searches of this second use the classes and the triangle
    /// hierarchies (not those of AURORA_HOVER_LEGACY).
    classes: bool,
    /// Frames that asked, and searches among them.
    asked: u32,
    searches: u32,
    /// Why the kept answer did not hold; a search can count in several.
    no_answer: u32,
    ray: u32,
    depth: u32,
    refresh: u32,
    index: u32,
    /// What disturbed the index first.
    by: [u32; 4],
    /// Searches that met an ignored object (all the candidates again).
    ignored: u32,
    /// Candidates of the searches, by step ([`Candidates`]).
    candidates: [u64; 3],
    /// Classes worked out.
    classes_set: u32,
    /// Time of the searches (ms): candidates from the index, point under
    /// the cursor, object at that point; and the longest search.
    index_ms: f32,
    point_ms: f32,
    object_ms: f32,
    longest_ms: f32,
    /// Exact tests of the searches ([`ExactTests`]).
    tests: [u64; 4],
    /// HUD picks done before asking for the object of the world, and
    /// their time (ms).
    hud_picks: u32,
    hud_ms: f32,
}

impl Stats {
    fn new(classes: bool) -> Stats {
        Stats {
            since: None,
            classes,
            asked: 0,
            searches: 0,
            no_answer: 0,
            ray: 0,
            depth: 0,
            refresh: 0,
            index: 0,
            by: [0; 4],
            ignored: 0,
            candidates: [0; 3],
            classes_set: 0,
            index_ms: 0.0,
            point_ms: 0.0,
            object_ms: 0.0,
            longest_ms: 0.0,
            tests: [0; 4],
            hud_picks: 0,
            hud_ms: 0.0,
        }
    }
}

/// AURORA_HOVER_LEGACY: `1` searches as before the classes and the triangle
/// hierarchies (every object tested whatever the ray is a candidate of every
/// step, every triangle of an object is tested, and the HUD pick looks at
/// every object of the scene), `ab` both ways in turn, a second each, to
/// compare.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ClassMode {
    On,
    Off,
    Alternate,
}

fn class_mode() -> ClassMode {
    static MODE: std::sync::OnceLock<ClassMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_HOVER_LEGACY").as_deref() {
        Ok("ab") => ClassMode::Alternate,
        Ok(v) if !v.is_empty() && v != "0" => ClassMode::Off,
        _ => ClassMode::On,
    })
}

impl Default for Stats {
    fn default() -> Self {
        Stats::new(class_mode() != ClassMode::Off)
    }
}

/// Exact tests done by the searches of picking.rs (profile): objects whose
/// triangles were asked for, those whose box the ray met, triangles tested
/// one by one, faces answered by their hierarchy instead.
#[derive(Default)]
pub struct ExactTests {
    pub objects: std::sync::atomic::AtomicU32,
    pub boxes: std::sync::atomic::AtomicU32,
    pub triangles: std::sync::atomic::AtomicU64,
    pub hierarchies: std::sync::atomic::AtomicU32,
}

impl ExactTests {
    fn take(&self) -> [u64; 4] {
        use std::sync::atomic::Ordering::Relaxed;
        [
            self.objects.swap(0, Relaxed) as u64,
            self.boxes.swap(0, Relaxed) as u64,
            self.triangles.swap(0, Relaxed),
            self.hierarchies.swap(0, Relaxed) as u64,
        ]
    }
}

/// The object under the cursor, kept between frames.
type HoverHit = (Option<Vec3>, Option<usize>);

#[derive(Default)]
pub struct HoverCache {
    kept: Option<(Asked, HoverHit, Instant)>,
    /// Candidate lists, reused.
    candidates: Candidates,
    /// Searches done and answers reused.
    pub searches: u64,
    pub reused: u64,
    /// Known classes the sweep found out of date.
    repaired: u64,
    /// AURORA_HOVER_CHECK: answers that differed from the full search (a
    /// new search / a kept answer), index entries and classes that differed
    /// from their object, and when the counts were last logged.
    wrong: u64,
    stale: u64,
    entries_wrong: u64,
    classes_wrong: u64,
    logged: Option<Instant>,
    stats: Stats,
}

impl HoverCache {
    pub(super) fn invalidate(&mut self) {
        self.kept = None;
    }

    /// AURORA_HOVER_LEGACY, this second: what the cursor is over is
    /// searched as before (no class, no triangle hierarchy, every object
    /// of the scene looked at for the HUD pick).
    pub(super) fn legacy(&self) -> bool {
        !self.stats.classes
    }

    /// The kept answer when it still holds for this question at `now`.
    fn get(&self, index: &PickIndex, asked: &Asked, now: Instant) -> Option<HoverHit> {
        let (kept, answer, at) = self.kept.as_ref()?;
        (!index.disturbed && kept.same(asked) && now.saturating_duration_since(*at) < REFRESH).then_some(*answer)
    }

    /// Profile: why `get` gave nothing.
    fn count_why(&mut self, index: &PickIndex, asked: &Asked, now: Instant) {
        let s = &mut self.stats;
        let Some((kept, _, at)) = self.kept.as_ref() else {
            s.no_answer += 1;
            return;
        };
        s.ray += !kept.same_ray(asked) as u32;
        s.depth += !kept.same_depth(asked) as u32;
        s.refresh += (now.saturating_duration_since(*at) >= REFRESH) as u32;
        if index.disturbed {
            s.index += 1;
            s.by[match index.disturbed_by {
                Some(Disturbed::Bounded) | None => 0,
                Some(Disturbed::Always) => 1,
                Some(Disturbed::Changed) => 2,
                Some(Disturbed::Reset) => 3,
            }] += 1;
        }
    }
}

/// AURORA_HOVER_CHECK=1: every answer is compared with the full search.
fn check_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AURORA_HOVER_CHECK").is_some())
}

/// AURORA_PROFILE: the `hover summary` line.
fn profile_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AURORA_PROFILE").is_some())
}

impl super::Scene {
    /// The HUD object the hover cursor is over, including invisible Mask/Blend
    /// buttons, which comes before any object of the world.
    pub fn hover_hud(&mut self, world: &crate::world::World, cursor: (f32, f32)) -> Option<usize> {
        if !profile_enabled() {
            return self.hud_pick(world, cursor, false).map(|(idx, _, _)| idx);
        }
        let started = Instant::now();
        let picked = self.hud_pick(world, cursor, false).map(|(idx, _, _)| idx);
        let now = Instant::now();
        self.roll_hover_stats(now, true);
        self.hover.stats.hud_picks += 1;
        self.hover.stats.hud_ms += (now - started).as_secs_f32() * 1000.0;
        picked
    }

    /// The object the hover cursor is over, including a non-actionable
    /// Blend surface in front of the displayed depth. Searched again
    /// only when the question or the scene around it changed, or every
    /// [`REFRESH`]. Not for clicks.
    #[cfg(test)]
    pub fn hover_object(&mut self, world: &crate::world::World, ray: Option<(Vec3, Vec3)>, depth: Option<Vec3>, far: f32) -> Option<usize> {
        self.hover_pick(world, ray, depth, far).1
    }

    /// The cached surface point as well, to bound a focused media's hover
    /// by the same foreground object as the world cursor.
    pub fn hover_pick(&mut self, world: &crate::world::World, ray: Option<(Vec3, Vec3)>, depth: Option<Vec3>, far: f32) -> HoverHit {
        let Some(ray) = ray else { return (depth, None) };
        let probe = Probe { ray, depth };
        let asked = Asked { probe, far };
        let now = Instant::now();
        let full = |scene: &Self| scene.full_hover_pick(world, ray, depth, far).1;
        let profile = profile_enabled();
        if profile || class_mode() == ClassMode::Alternate {
            self.roll_hover_stats(now, profile);
            self.hover.stats.asked += 1;
        }
        if let Some(answer) = self.hover.get(&self.pick, &asked, now) {
            self.hover.reused += 1;
            if check_enabled() {
                let expected = full(self);
                if expected != answer.1 {
                    // allowed for up to REFRESH: what the index cannot see
                    self.hover.stale += 1;
                    let age = self
                        .hover
                        .kept
                        .as_ref()
                        .map_or(0.0, |k| now.duration_since(k.2).as_secs_f32() * 1000.0);
                    log::info!("hover check: kept answer {answer:?}, a new search gives {expected:?} ({age:.0} ms old)");
                }
                self.log_hover_check(now);
            }
            return answer;
        }
        if profile {
            self.hover.count_why(&self.pick, &asked, now);
            self.exact_tests.take();
        }
        let classes = self.hover.stats.classes;
        if classes {
            self.work_out_hover_classes(world);
        }
        let mut candidates = std::mem::take(&mut self.hover.candidates);
        self.pick.hover_candidates(&probe, classes, &mut candidates);
        let t_index = profile.then(Instant::now);
        // (the steps of `interaction_point`, each over its own candidates)
        let ignored = self.ignored_before(world, ray, depth, far, now, Some(&candidates.ignore));
        let point = if ignored {
            self.pick.candidates(&probe, &mut candidates.all);
            self.point_behind_ignored(world, ray, far, now, Some(&candidates.all))
        } else {
            self.transparent_action_point(world, ray, depth, false, far, Some(&candidates.see_through))
        };
        let blend = self.blend_hit(world, ray, point, far, Some(&candidates.see_through));
        let point = blend.map(|(t, _)| ray.0 + ray.1 * t).or(point);
        let t_point = profile.then(Instant::now);
        let object = blend
            .map(|(_, idx)| idx)
            .or_else(|| point.and_then(|point| self.interaction_at_ray_among(world, point, ray, false, Some(&candidates.point))));
        let answer = (point, object);
        if let (Some(t_index), Some(t_point)) = (t_index, t_point) {
            let done = Instant::now();
            let ms = |a: Instant, b: Instant| (b - a).as_secs_f32() * 1000.0;
            let tests = self.exact_tests.take();
            let s = &mut self.hover.stats;
            s.searches += 1;
            s.ignored += ignored as u32;
            s.candidates[0] += candidates.ignore.len() as u64;
            s.candidates[1] += candidates.see_through.len() as u64;
            s.candidates[2] += candidates.point.len() as u64;
            s.index_ms += ms(now, t_index);
            s.point_ms += ms(t_index, t_point);
            s.object_ms += ms(t_point, done);
            s.longest_ms = s.longest_ms.max(ms(now, done));
            for (sum, n) in s.tests.iter_mut().zip(tests) {
                *sum += n;
            }
        }
        if check_enabled() {
            let expected = full(self);
            if expected != object {
                self.hover.wrong += 1;
                log::warn!(
                    "hover check: the search among {} / {} / {} candidates gives {answer:?}, the full search {expected:?}",
                    candidates.ignore.len(),
                    candidates.see_through.len(),
                    candidates.point.len()
                );
            }
            self.check_hover_index(world, classes);
            self.log_hover_check(now);
        }
        self.hover.candidates = candidates;
        self.hover.searches += 1;
        self.hover.kept = Some((asked, answer, now));
        self.pick.watch(probe, classes && !ignored);
        answer
    }

    /// Reference search for AURORA_HOVER_CHECK, without the spatial index,
    /// classes or triangle hierarchies.
    fn full_hover_pick(&self, world: &crate::world::World, ray: (Vec3, Vec3), depth: Option<Vec3>, far: f32) -> HoverHit {
        let point = self.interaction_point(world, Some(ray), depth, false, far);
        if let Some((t, idx)) = self.blend_hit(world, ray, point, far, None) {
            return (Some(ray.0 + ray.1 * t), Some(idx));
        }
        (point, point.and_then(|point| self.interaction_at_ray(world, point, ray, false)))
    }

    /// The class of an object for the hover search ([`class`]), from what
    /// the steps of the search read: `ignored_before` looks at the objects
    /// set to IGNORE only, `transparent_action_point` at those with a
    /// blended face that are not avatars, `interaction_at_ray` at neither
    /// the avatars nor the meshes they wear.
    fn hover_class(&self, world: &crate::world::World, idx: usize) -> u8 {
        let mut c = class::KNOWN;
        // (no object: every step leaves the slot out)
        let (Some(o), Some(g)) = (world.objects.get(idx), self.gpu.get(idx)) else {
            return c;
        };
        if o.extra.reflection_probe.is_some() {
            return c | class::OFF_POINT;
        }
        if o.click_action == crate::interaction::code::IGNORE {
            c |= class::IGNORE;
        }
        if !g.is_avatar && (self.see_through_face(g, false) || self.has_blend_face(world, idx)) {
            c |= class::SEE_THROUGH;
        }
        // (what `interaction_at_ray` leaves out whatever the point)
        if g.is_avatar || (g.owner_avatar.is_some() && o.volume.is_mesh()) {
            c |= class::OFF_POINT;
        }
        c
    }

    /// Before a search: the classes forgotten since the last one are
    /// worked out again, with those of the objects the sync has not heard
    /// of yet (updated since, or whose texture changed its alpha class).
    fn work_out_hover_classes(&mut self, world: &crate::world::World) {
        for &idx in world.objects.touched() {
            self.pick.forget_class(idx);
        }
        for &slot in self.textures.pending_alpha_changes() {
            for &(_, idx) in self.slot_users.range((slot, 0)..=(slot, u32::MAX)) {
                self.pick.forget_class(idx as usize);
            }
        }
        for _ in 0..CLASSES_PER_SEARCH {
            let Some(idx) = self.pick.next_unresolved() else {
                break;
            };
            let c = self.hover_class(world, idx);
            self.pick.set_class(idx, c);
            self.hover.stats.classes_set += 1;
        }
        for _ in 0..CLASSES_SWEPT_PER_SEARCH {
            if let Some((idx, known)) = self.pick.next_swept() {
                let c = self.hover_class(world, idx);
                if c != known {
                    self.pick.set_class(idx, c);
                    self.hover.repaired += 1;
                    log::debug!("hover: class of object {idx} was {known:#b}, is {c:#b}");
                }
            }
        }
    }

    /// AURORA_HOVER_CHECK: the index against the scene. Every entry is
    /// what `PickEntry::of` gives for its object now, and (`with_classes`)
    /// every known class what `hover_class` gives now.
    fn check_hover_index(&mut self, world: &crate::world::World, with_classes: bool) {
        let (mut entries, mut classes) = (0u32, 0u32);
        for idx in 0..self.pick.entries.len().max(self.gpu.len()) {
            let e = self.pick.entries.get(idx).copied().unwrap_or(PickEntry::NEVER);
            let expected = self.gpu.get(idx).map_or(PickEntry::NEVER, PickEntry::of);
            if e != expected {
                entries += 1;
                if entries <= 3 {
                    log::warn!("hover check: entry of object {idx} is {e:?}, its object gives {expected:?}");
                }
            }
            let known = self.pick.classes.get(idx).copied().unwrap_or(0);
            if with_classes && e.always && e.pickable() && known & class::KNOWN != 0 && known != self.hover_class(world, idx) {
                classes += 1;
                if classes <= 3 {
                    log::warn!(
                        "hover check: class of object {idx} is {known:#b}, its object gives {:#b}",
                        self.hover_class(world, idx)
                    );
                }
            }
        }
        self.hover.entries_wrong += entries as u64;
        self.hover.classes_wrong += classes as u64;
    }

    /// Once a second: with AURORA_PROFILE, the `hover summary` line (what
    /// the searches of the second did and why they were needed; per
    /// search: candidates of each step, times in ms, exact tests); with
    /// AURORA_HOVER_LEGACY=ab, the next second searches the other way.
    fn roll_hover_stats(&mut self, now: Instant, log: bool) {
        let since = *self.hover.stats.since.get_or_insert(now);
        if now.duration_since(since) < Duration::from_secs(1) {
            return;
        }
        let next = match class_mode() {
            ClassMode::Alternate => !self.hover.stats.classes,
            mode => mode == ClassMode::On,
        };
        let s = std::mem::replace(&mut self.hover.stats, Stats::new(next));
        self.hover.stats.since = Some(now);
        if !log || (s.asked == 0 && s.hud_picks == 0) {
            return;
        }
        let n = s.searches.max(1) as f32;
        log::info!(
            "hover summary: search={} hud_ms_per_frame={:.4} asked={} searches={} ms_per_frame={:.4} longest_ms={:.3} | \
             why: none={} ray={} depth={} index={} refresh={} | index: bounded={} always={} changed={} reset={} | \
             per search: ignore={:.0} see_through={:.0} point={:.0} index_ms={:.4} point_ms={:.4} object_ms={:.4} \
             tested={:.1} boxes={:.1} triangles={:.0} hierarchies={:.1} | ignored={} classes_set={} repaired={} \
             left_out_moves={}",
            if s.classes { "new" } else { "old" },
            s.hud_ms / s.hud_picks.max(1) as f32,
            s.asked,
            s.searches,
            (s.index_ms + s.point_ms + s.object_ms) / s.asked.max(1) as f32,
            s.longest_ms,
            s.no_answer,
            s.ray,
            s.depth,
            s.index,
            s.refresh,
            s.by[0],
            s.by[1],
            s.by[2],
            s.by[3],
            s.candidates[0] as f32 / n,
            s.candidates[1] as f32 / n,
            s.candidates[2] as f32 / n,
            s.index_ms / n,
            s.point_ms / n,
            s.object_ms / n,
            s.tests[0] as f32 / n,
            s.tests[1] as f32 / n,
            s.tests[2] as f32 / n,
            s.tests[3] as f32 / n,
            s.ignored,
            s.classes_set,
            self.hover.repaired,
            std::mem::take(&mut self.pick.left_out_moves),
        );
    }

    /// AURORA_HOVER_CHECK: the counts, every ten seconds.
    fn log_hover_check(&mut self, now: Instant) {
        let h = &mut self.hover;
        let last = *h.logged.get_or_insert(now);
        if now.duration_since(last) >= Duration::from_secs(10) {
            h.logged = Some(now);
            log::info!(
                "hover check: {} searches, {} answers reused, {} wrong, {} kept answers out of date, \
                 {} index entries and {} classes unlike their object, {} classes repaired",
                h.searches,
                h.reused,
                h.wrong,
                h.stale,
                h.entries_wrong,
                h.classes_wrong,
                h.repaired
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAY: (Vec3, Vec3) = (Vec3::ZERO, Vec3::X);
    const PROBE: Probe = Probe { ray: RAY, depth: None };

    #[test]
    fn inventory_query_reuses_candidate_capacity_without_changing_hover_watch() {
        let mut index = PickIndex::default();
        index.set(0, sphere(5.0, 0.0, 1.0));
        index.set(1, sphere(5.0, 8.0, 1.0));
        index.watch(PROBE, false);
        let mut out = Vec::with_capacity(8);
        let capacity = out.capacity();
        index.ray_candidates(RAY, &mut out);
        assert_eq!(out, [0]);
        assert_eq!(out.capacity(), capacity);
        assert_eq!(index.watch, Some(PROBE));
        assert!(!index.disturbed);
    }

    fn sphere(x: f32, y: f32, r: f32) -> PickEntry {
        PickEntry {
            center: Vec3::new(x, y, 0.0),
            radius: r,
            always: false,
            aside: false,
        }
    }

    fn always(x: f32, y: f32) -> PickEntry {
        PickEntry {
            center: Vec3::new(x, y, 0.0),
            radius: 2.5,
            always: true,
            aside: true,
        }
    }

    fn face() -> crate::scene::FaceDraw {
        crate::scene::FaceDraw {
            te_index: 0,
            record: 0,
            cmd: aurora_render::DrawCmd {
                index_count: 6,
                first_index: 0,
                base_vertex: 0,
                record: 0,
                bounds: [0.0; 4],
            },
            base_slot: 0,
            tex_id: uuid::Uuid::nil(),
            aux_tex: [uuid::Uuid::nil(); 3],
            te_alpha: 1.0,
            pbr_alpha: None,
            legacy_alpha: None,
            two_sided: false,
            repeats: 1.0,
            glow: false,
            pass: crate::scene::Pass::Opaque,
            glow_pool: false,
            rez_glow: false,
            system: None,
            first_person_hidden: false,
        }
    }

    #[test]
    fn entries_follow_the_object_state() {
        let mut g = ObjGpu::default();
        // no face yet
        assert_eq!(PickEntry::of(&g), PickEntry::NEVER);
        g.geom = Some(super::super::GeomKey::Prim { hash: 0, lod: 0 });
        assert!(PickEntry::of(&g).pickable()); // invisible faces still have geometry
        g.geom = None;
        g.faces.push(face());
        g.center = Vec3::new(1.0, 2.0, 3.0);
        g.radius = 0.5;
        let e = PickEntry::of(&g);
        assert_eq!((e.center, e.radius, e.always, e.aside), (g.center, 0.5, false, false));
        g.owner_avatar = Some(3);
        assert!(PickEntry::of(&g).aside);
        g.rigged = true;
        assert!(PickEntry::of(&g).always);
        g.rigged = false;
        g.radius = 0.0;
        assert!(PickEntry::of(&g).always && PickEntry::of(&g).pickable());
        g.radius = 0.5;
        g.is_avatar = true;
        let e = PickEntry::of(&g);
        assert!(e.always && e.aside && e.radius == 2.5);
        g.hud = true;
        assert!(!PickEntry::of(&g).pickable());
    }

    #[test]
    fn candidates_are_a_superset_of_the_exact_tests() {
        // every object `ray_may_hit` or the point test of
        // `interaction_at_ray` accepts must be a candidate
        let mut state = 99u64;
        let mut rnd = |m: f32| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 40) as f32 / (1u64 << 24) as f32) * m
        };
        for _ in 0..20_000 {
            let center = Vec3::new(rnd(40.0) - 5.0, rnd(6.0) - 3.0, rnd(6.0) - 3.0);
            let radius = 0.01 + rnd(2.0);
            let g = ObjGpu {
                center,
                radius,
                faces: vec![face()],
                ..Default::default()
            };
            let e = PickEntry::of(&g);
            if crate::scene::picking::ray_may_hit(&g, RAY) {
                assert!(e.candidate(&PROBE), "{center} {radius}");
            }
            // a point of the ray
            let on_ray = Vec3::X * rnd(40.0);
            if center.distance_squared(on_ray) <= (radius + 0.06).powi(2) {
                assert!(e.candidate(&PROBE), "{center} {radius} {on_ray}");
            }
            // the depth point of an earlier cursor position, off the ray
            let depth = Vec3::new(rnd(40.0), rnd(10.0) - 5.0, rnd(10.0) - 5.0);
            if center.distance_squared(depth) <= (radius + 0.06).powi(2) {
                let probe = Probe {
                    ray: RAY,
                    depth: Some(depth),
                };
                assert!(e.candidate(&probe), "{center} {radius} {depth}");
            }
        }
        assert!(!sphere(10.0, 3.0, 1.0).candidate(&PROBE));
        assert!(!sphere(-5.0, 0.0, 1.0).candidate(&PROBE));
        // far from the ray, but around the depth point
        let probe = Probe {
            ray: RAY,
            depth: Some(Vec3::new(10.0, 3.5, 0.0)),
        };
        assert!(sphere(10.0, 3.0, 1.0).candidate(&probe));
        assert!(always(0.0, 90.0).candidate(&PROBE) && !PickEntry::NEVER.candidate(&PROBE));
    }

    #[test]
    fn candidates_come_in_index_order() {
        let mut index = PickIndex::default();
        index.set(7, sphere(5.0, 0.2, 0.5));
        index.set(2, always(0.0, 50.0));
        index.set(4, sphere(5.0, 9.0, 0.5));
        index.set(9, sphere(30.0, 0.0, 0.1));
        index.set(20, PickEntry::NEVER);
        let mut out = vec![99];
        index.candidates(&PROBE, &mut out);
        assert_eq!(out, [2, 7, 9]);
        // a slot never set, or removed, is not a candidate
        index.remove(7);
        index.remove(1000);
        index.candidates(&PROBE, &mut out);
        assert_eq!(out, [2, 9]);
    }

    #[test]
    fn only_changes_around_the_watched_search_disturb_it() {
        let mut index = PickIndex::default();
        index.set(1, sphere(5.0, 0.0, 0.5)); // on the ray
        index.set(2, sphere(5.0, 8.0, 0.5)); // away from it
        index.set(3, always(60.0, 40.0)); // an avatar far from the ray
        // nothing is watched yet
        assert!(!index.disturbed);
        index.watch(PROBE, false);
        // the same bounds again, changes far from the ray: undisturbed
        index.set(1, sphere(5.0, 0.0, 0.5));
        index.set(2, sphere(6.0, 9.0, 0.5));
        index.changed(2);
        index.set(40, sphere(0.0, 50.0, 1.0));
        index.remove(2);
        index.changed(77);
        index.changed(3);
        index.set(3, always(61.0, 40.0));
        assert!(!index.disturbed);
        // the idle sway of something on the ray: under 2 mm a sync
        index.set(1, sphere(5.0, 0.001, 0.5));
        index.set(1, sphere(5.0, 0.002, 0.501));
        assert!(!index.disturbed);
        // an object on the ray changed without moving
        index.changed(1);
        assert!(index.disturbed);
        // moving along the ray, onto it, and away from it
        index.watch(PROBE, false);
        index.set(1, sphere(5.1, 0.0, 0.5));
        assert!(index.disturbed);
        index.watch(PROBE, false);
        index.set(40, sphere(20.0, 0.0, 1.0));
        assert!(index.disturbed);
        index.watch(PROBE, false);
        index.set(40, sphere(20.0, 30.0, 1.0));
        assert!(index.disturbed);
        // arriving and leaving
        index.watch(PROBE, false);
        index.set(50, sphere(3.0, 0.1, 0.2));
        assert!(index.disturbed);
        index.watch(PROBE, false);
        index.remove(50);
        assert!(index.disturbed);
        // an avatar walking through the ray
        index.watch(PROBE, false);
        index.set(3, always(30.0, 1.0));
        assert!(index.disturbed);
        // around the depth point of the watched search, off its ray
        index.watch(
            Probe {
                ray: RAY,
                depth: Some(Vec3::new(6.0, 9.0, 0.0)),
            },
            false,
        );
        index.set(2, sphere(6.0, 9.5, 0.5));
        assert!(index.disturbed);
        // an object without bounds: anywhere
        index.watch(PROBE, false);
        index.set(
            60,
            PickEntry {
                center: Vec3::new(0.0, 500.0, 0.0),
                radius: 0.0,
                always: true,
                aside: false,
            },
        );
        assert!(index.disturbed);
        index.watch(PROBE, true);
        index.reset();
        assert!(index.disturbed);
    }

    #[test]
    fn answer_is_kept_until_something_relevant_changes() {
        let mut index = PickIndex::default();
        index.set(1, sphere(5.0, 0.0, 0.5));
        let probe = Probe {
            ray: RAY,
            depth: Some(Vec3::new(4.5, 0.0, 0.0)),
        };
        let asked = Asked { probe, far: 96.0 };
        let with = |ray: (Vec3, Vec3), depth: Option<Vec3>| Asked {
            probe: Probe { ray, depth },
            far: 96.0,
        };
        let t = Instant::now();
        let mut cache = HoverCache::default();
        assert_eq!(cache.get(&index, &asked, t), None);
        let hit = (probe.depth, Some(1));
        cache.kept = Some((asked, hit, t));
        index.watch(probe, false);
        let ms = Duration::from_millis;
        assert_eq!(cache.get(&index, &asked, t + ms(5)), Some(hit));
        // camera settling: a hair's width
        let settling = with(
            (Vec3::splat(1e-4), (Vec3::X + Vec3::Y * 1e-5).normalize()),
            Some(Vec3::new(4.5005, 0.0, 0.0)),
        );
        assert_eq!(cache.get(&index, &settling, t + ms(5)), Some(hit));
        // searched again after 100 ms whatever happens
        assert_eq!(cache.get(&index, &asked, t + ms(99)), Some(hit));
        assert_eq!(cache.get(&index, &asked, t + REFRESH), None);
        // the cursor or the camera moved
        let moved = with((Vec3::ZERO, (Vec3::X + Vec3::Y * 1e-3).normalize()), probe.depth);
        assert_eq!(cache.get(&index, &moved, t + ms(5)), None);
        let stepped = with((Vec3::Z * 0.01, Vec3::X), probe.depth);
        assert_eq!(cache.get(&index, &stepped, t + ms(5)), None);
        // something else is drawn under the cursor (or nothing any more)
        let nearer = with(RAY, Some(Vec3::new(4.2, 0.0, 0.0)));
        assert_eq!(cache.get(&index, &nearer, t + ms(5)), None);
        assert_eq!(cache.get(&index, &with(RAY, None), t + ms(5)), None);
        assert_eq!(cache.get(&index, &Asked { far: 128.0, ..asked }, t + ms(5)), None);
        // an object along the ray changed
        index.changed(1);
        assert_eq!(cache.get(&index, &asked, t + ms(5)), None);
        // a kept "nothing" is an answer too
        cache.kept = Some((asked, (None, None), t));
        index.watch(probe, false);
        assert_eq!(cache.get(&index, &asked, t + ms(5)), Some((None, None)));
    }

    fn lists(index: &PickIndex, classes: bool) -> (Vec<usize>, Vec<usize>, Vec<usize>) {
        let mut out = Candidates::default();
        index.hover_candidates(&PROBE, classes, &mut out);
        (out.ignore, out.see_through, out.point)
    }

    #[test]
    fn without_classes_every_step_gets_the_candidates_of_before() {
        let mut state = 5u64;
        let mut rnd = |m: f32| {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((state >> 40) as f32 / (1u64 << 24) as f32) * m
        };
        let mut index = PickIndex::default();
        for i in 0..2000 {
            let e = match (rnd(10.0)) as u32 {
                0 => PickEntry::NEVER,
                1..=3 => PickEntry {
                    center: Vec3::new(rnd(40.0), rnd(40.0) - 20.0, 0.0),
                    radius: if rnd(1.0) < 0.1 { 0.0 } else { 2.0 },
                    always: true,
                    aside: false,
                },
                _ => sphere(rnd(40.0), rnd(8.0) - 4.0, 0.1 + rnd(1.0)),
            };
            index.set(i, e);
        }
        let mut all = Vec::new();
        index.candidates(&PROBE, &mut all);
        assert!(all.len() > 300, "{}", all.len());
        // classes unknown, or not used: as before
        for classes in [false, true] {
            let (ignore, see_through, point) = lists(&index, classes);
            assert_eq!(ignore, all);
            assert_eq!(see_through, all);
            assert_eq!(point, all);
        }
        // every class known and empty: the objects with bounds near the
        // ray and those without bounds for the first two steps; for the
        // third, also the others near the ray
        while let Some(idx) = index.next_unresolved() {
            index.set_class(idx, class::KNOWN);
        }
        let kept = |with_bounds: bool| -> Vec<usize> {
            all.iter()
                .copied()
                .filter(|&i| {
                    let e = &index.entries[i];
                    e.radius == 0.0 || (e.near(&PROBE) && (with_bounds || !e.always))
                })
                .collect()
        };
        let (plain, near) = (kept(false), kept(true));
        assert!(
            plain.len() + 20 < near.len() && near.len() + 100 < all.len(),
            "{} {} {}",
            plain.len(),
            near.len(),
            all.len()
        );
        assert_eq!(lists(&index, true), (plain.clone(), plain, near));
        assert_eq!(lists(&index, false).0, all);
    }

    #[test]
    fn a_class_puts_an_object_away_from_the_ray_in_its_steps_only() {
        let mut index = PickIndex::default();
        index.set(1, always(10.0, 40.0)); // away from the ray
        index.set(2, always(10.0, 41.0));
        index.set(3, always(10.0, 42.0));
        index.set(4, always(10.0, 1.0)); // near it
        index.set(5, sphere(10.0, 40.0, 1.0)); // with bounds, away
        let mut unknown = Vec::new();
        while let Some(idx) = index.next_unresolved() {
            unknown.push(idx);
        }
        assert_eq!(unknown, [4, 3, 2, 1]);
        for (idx, c) in [(1, 0), (2, class::IGNORE), (3, class::SEE_THROUGH), (4, 0)] {
            index.set_class(idx, class::KNOWN | c);
        }
        assert_eq!(lists(&index, true), (vec![2], vec![3], vec![4]));
        // near the ray and left out by the search for the object at a point
        index.set_class(4, class::KNOWN | class::OFF_POINT);
        assert_eq!(lists(&index, true), (vec![2], vec![3], vec![]));
        // both, and back to unknown after a change
        index.set_class(2, class::KNOWN | class::IGNORE | class::SEE_THROUGH);
        assert_eq!(lists(&index, true), (vec![2], vec![2, 3], vec![]));
        index.changed(3);
        index.changed(4);
        assert_eq!(lists(&index, true), (vec![2, 3, 4], vec![2, 3, 4], vec![3, 4]));
        // with bounds: near the ray only, whatever is known of it
        index.set(5, sphere(10.0, 0.5, 1.0));
        assert_eq!(lists(&index, true), (vec![2, 3, 4, 5], vec![2, 3, 4, 5], vec![3, 4, 5]));
    }

    #[test]
    fn classes_are_forgotten_when_the_object_may_have_changed() {
        let mut index = PickIndex::default();
        index.set(1, always(10.0, 40.0));
        index.set(2, sphere(10.0, 40.0, 1.0));
        // only the object tested whatever the ray has a class to work out
        assert_eq!(index.next_unresolved(), Some(1));
        assert_eq!(index.next_unresolved(), None);
        index.set_class(1, class::KNOWN | class::IGNORE);
        // moving keeps it
        index.set(1, always(11.0, 40.0));
        assert_eq!(index.classes[1], class::KNOWN | class::IGNORE);
        assert_eq!(index.next_unresolved(), None);
        // a change forgets it, and queues the object once
        index.changed(1);
        index.changed(1);
        index.changed(2);
        assert_eq!(index.unresolved, [1]);
        assert_eq!(index.classes[1] & class::KNOWN, 0);
        assert_eq!(index.next_unresolved(), Some(1));
        index.set_class(1, class::KNOWN);
        // another kind of object in the slot
        index.set(1, sphere(11.0, 40.0, 1.0));
        assert_eq!(index.classes[1] & class::KNOWN, 0);
        index.set(1, always(11.0, 40.0));
        assert_eq!(index.next_unresolved(), Some(1));
        index.set_class(1, class::KNOWN);
        // gone: nothing to work out when its turn comes
        index.changed(1);
        index.remove(1);
        assert_eq!(index.next_unresolved(), None);
        // the sweep goes round the entries and gives the known classes
        index.set(1, always(11.0, 40.0));
        assert_eq!(index.next_unresolved(), Some(1));
        index.set_class(1, class::KNOWN | class::SEE_THROUGH);
        let swept: Vec<_> = (0..6).filter_map(|_| index.next_swept()).collect();
        assert_eq!(swept, [(1, class::KNOWN | class::SEE_THROUGH); 2]);
        index.reset();
        assert!(index.classes.is_empty() && index.next_swept().is_none());
    }

    #[test]
    fn an_object_left_out_everywhere_moves_without_disturbing_the_search() {
        let mut index = PickIndex::default();
        index.set(1, always(10.0, 1.0)); // near the ray
        index.set(2, always(12.0, 1.0));
        while let Some(idx) = index.next_unresolved() {
            index.set_class(idx, class::KNOWN | class::OFF_POINT);
        }
        // the watched search used the classes: an avatar or a worn mesh
        // can cross the ray
        index.watch(PROBE, true);
        index.set(1, always(10.0, 0.5));
        index.set(1, always(10.0, -0.5));
        assert!(!index.disturbed);
        assert_eq!(index.left_out_moves, 2);
        // not once its class is in a step of the search, or unknown
        index.set_class(2, class::KNOWN | class::OFF_POINT | class::SEE_THROUGH);
        index.set(2, always(12.0, 0.5));
        assert!(index.disturbed);
        index.watch(PROBE, true);
        index.changed(1);
        assert!(index.disturbed);
        index.watch(PROBE, true);
        index.set(1, always(10.0, 0.2));
        assert!(index.disturbed);
        // nor when the search met an ignored object (it took them all) or
        // did not use the classes
        assert_eq!(index.next_unresolved(), Some(1));
        index.set_class(1, class::KNOWN | class::OFF_POINT);
        index.watch(PROBE, false);
        index.set(1, always(10.0, 0.8));
        assert!(index.disturbed);
        // nor an object the search for the object at a point looks at
        index.set_class(1, class::KNOWN);
        index.watch(PROBE, true);
        index.set(1, always(10.0, 0.1));
        assert!(index.disturbed);
    }

    /// The scene of the picking tests, the far object (`back`) turned into
    /// a rigged mesh whose sphere is away from the ray while its triangles
    /// are in front of the near one: what a worn mesh is to the search.
    fn scene_with_a_rigged_mesh() -> (crate::world::World, crate::scene::Scene, usize, usize) {
        let (mut world, mut scene, front, back) = crate::scene::picking::tests::scene_with_two_objects();
        world.objects.get_mut(back).unwrap().position = Vec3::new(1.5, 0.0, 50.0);
        world.objects.get_mut(front).unwrap().update_flags = 1 << 7;
        for idx in [front, back] {
            let g = &mut scene.gpu[idx];
            g.center = world.objects.get(idx).unwrap().position;
            g.radius = 1.0;
        }
        scene.gpu[back].rigged = true;
        scene.gpu[back].center = Vec3::new(1.5, 40.0, 50.0);
        for idx in [front, back] {
            scene.pick.set(idx, PickEntry::of(&scene.gpu[idx]));
        }
        // (the sync took the changes above)
        world.objects.take_touched(&mut Vec::new());
        (world, scene, front, back)
    }

    #[test]
    fn the_search_with_classes_gives_the_answer_of_the_full_search() {
        let (mut world, mut scene, front, back) = scene_with_a_rigged_mesh();
        let ray = (Vec3::new(0.0, 0.0, 50.0), Vec3::X);
        let depth = Some(Vec3::new(2.5, 0.0, 50.0));
        // (another draw distance each time: never a kept answer)
        let mut far = 100.0;
        let mut both = |scene: &mut crate::scene::Scene, world: &crate::world::World| {
            far += 1.0;
            let full = scene
                .interaction_point(world, Some(ray), depth, false, far)
                .and_then(|point| scene.interaction_at_ray(world, point, ray, false));
            assert_eq!(scene.hover_object(world, Some(ray), depth, far), full);
            scene.check_hover_index(world, true);
            assert_eq!((scene.hover.entries_wrong, scene.hover.classes_wrong), (0, 0));
            full
        };
        // an opaque rigged mesh away from the ray: left out of every step
        assert_eq!(both(&mut scene, &world), Some(front));
        assert_eq!(scene.pick.classes[back], class::KNOWN);
        assert_eq!(scene.hover.candidates.see_through, [front]);
        // It gets a see-through face and a script. Its rest triangles are
        // in front, but only its GPU-posed surface may intercept a click.
        world.objects.get_mut(back).unwrap().update_flags = 1 << 7;
        scene.gpu[back].faces[0].te_alpha = 0.5;
        scene.pick.changed(back);
        world.objects.take_touched(&mut Vec::new());
        assert_eq!(both(&mut scene, &world), Some(front));
        assert_eq!(scene.pick.classes[back], class::KNOWN | class::SEE_THROUGH);
        assert_eq!(scene.hover.candidates.see_through, [front, back]);
        assert_eq!(scene.hover.candidates.point, [front]);
        // set to IGNORE since the last sync: the search hears of it from
        // the objects updated, and goes behind it
        world.objects.get_mut(back).unwrap().click_action = crate::interaction::code::IGNORE;
        assert_eq!(both(&mut scene, &world), Some(front));
        assert_eq!(scene.pick.classes[back], class::KNOWN | class::IGNORE | class::SEE_THROUGH);
        assert_eq!(scene.hover.candidates.ignore, [front, back]);
        // opaque again, by a texture that changed its alpha class
        world.objects.take_touched(&mut Vec::new());
        scene.gpu[back].faces[0].te_alpha = 1.0;
        scene.pick.changed(back);
        both(&mut scene, &world);
        assert_eq!(scene.pick.classes[back], class::KNOWN | class::IGNORE);
        // a class the index was not told of is repaired by the sweep
        scene.pick.set_class(back, class::KNOWN);
        for _ in 0..scene.pick.entries.len().div_ceil(CLASSES_SWEPT_PER_SEARCH) {
            scene.work_out_hover_classes(&world);
        }
        assert_eq!(scene.pick.classes[back], class::KNOWN | class::IGNORE);
        assert_eq!(scene.hover.repaired, 1);
    }

    #[test]
    fn a_large_face_is_searched_through_its_hierarchy_once_built() {
        use crate::scene::{GeomEntry, GeomKey, GeomState, GpuGeom, picking::PickFace};
        let (world, mut scene, front, _) = scene_with_a_rigged_mesh();
        // the near object: a mesh whose side facing the ray is a fine grid
        const N: u16 = 24;
        let mut positions = Vec::new();
        for y in 0..=N {
            for z in 0..=N {
                positions.push([-0.5, y as f32 / N as f32 - 0.5, z as f32 / N as f32 - 0.5]);
            }
        }
        let mut indices = Vec::new();
        for y in 0..N {
            for z in 0..N {
                let a = y * (N + 1) + z;
                let b = a + N + 1;
                indices.extend([a, b, a + 1, a + 1, b, b + 1]);
            }
        }
        assert!(indices.len() >= crate::scene::pick_accel::MIN_TRIANGLES * 3);
        let key = GeomKey::Mesh {
            id: uuid::Uuid::from_u128(77),
            lod: 0,
        };
        let uvs = vec![[0.0; 2]; positions.len()];
        scene.geoms.insert(
            key,
            GeomEntry {
                refs: 1,
                zero_since: 0,
                state: GeomState::Ready(std::sync::Arc::new(GpuGeom {
                    faces: vec![None],
                    min: -Vec3::splat(0.5),
                    max: Vec3::splat(0.5),
                    joint_bounds: Vec::new(),
                    pick_faces: vec![Some(PickFace::new(positions, uvs, indices))],
                })),
            },
        );
        scene.gpu[front].geom = Some(key);
        let ray = (Vec3::new(0.0, 0.13, 50.21), Vec3::X);
        let depth = Some(Vec3::new(2.5, 0.13, 50.21));
        let full = scene
            .interaction_point(&world, Some(ray), depth, false, 100.0)
            .and_then(|point| scene.interaction_at_ray(&world, point, ray, false));
        assert_eq!(full, Some(front));
        // every triangle the first time, and the hierarchy is asked for
        scene.exact_tests.take();
        assert_eq!(scene.hover_object(&world, Some(ray), depth, 100.0), full);
        let [_, _, triangles, hierarchies] = scene.exact_tests.take();
        assert_eq!((triangles, hierarchies), (2 * N as u64 * N as u64, 0));
        let geom = scene.geom_ready(&key).unwrap();
        let face = geom.pick_faces[0].as_ref().unwrap();
        let waited = Instant::now();
        while face.accel.get_or_ask(|| panic!("asked twice")).is_none() {
            assert!(waited.elapsed() < Duration::from_secs(20), "the hierarchy was not built");
            std::thread::sleep(Duration::from_millis(2));
        }
        // then the same answer without them
        assert_eq!(scene.hover_object(&world, Some(ray), depth, 101.0), full);
        let [_, _, triangles, hierarchies] = scene.exact_tests.take();
        assert_eq!((triangles, hierarchies), (0, 1));
        // beside the grid's side: nothing either way
        let beside = (Vec3::new(0.0, 0.7, 50.21), Vec3::X);
        assert_eq!(scene.hover_object(&world, Some(beside), None, 102.0), None);
    }
}
