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
//! Clicks never read the cache: they search again with a blocking pick.
//!
//! `AURORA_HOVER_CHECK=1` compares every answer with the full search and
//! logs the differences (`hover check`), with a count of searches and
//! reused answers every ten seconds.

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
    /// Negative: nothing to pick (no face, or a HUD attachment).
    radius: f32,
    /// Tested whatever the ray: avatars (their own box test), rigged meshes
    /// (the exact tests do not trust their bounds), objects without bounds.
    always: bool,
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
    };

    /// The entry of an object as its last sync left it.
    pub fn of(g: &ObjGpu) -> PickEntry {
        if g.faces.is_empty() || g.hud {
            PickEntry::NEVER
        } else if g.is_avatar {
            // (the extent the scene gives a wearer's rigged meshes)
            PickEntry {
                center: g.center,
                radius: 2.5,
                always: true,
            }
        } else {
            PickEntry {
                center: g.center,
                radius: g.radius.max(0.0),
                always: g.rigged || g.radius <= 0.0,
            }
        }
    }

    fn pickable(&self) -> bool {
        self.radius >= 0.0
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
            && self.center.distance_squared(o.center) <= STILL * STILL
            && (self.radius - o.radius).abs() <= STILL
    }
}

/// Compact copy of the objects' bounds (indexed like the object slab) and
/// the search whose surroundings are watched for changes.
#[derive(Default)]
pub struct PickIndex {
    entries: Vec<PickEntry>,
    /// Ray and depth point of the answer kept by the cache.
    watch: Option<Probe>,
    /// Something changed around them since.
    disturbed: bool,
}

impl PickIndex {
    fn note(&mut self, e: PickEntry) {
        if !self.disturbed && self.watch.is_some_and(|probe| e.concerns(&probe)) {
            self.disturbed = true;
        }
    }

    /// The object's bounds after a sync. A change around the watched
    /// search (where the object was, or where it is now) disturbs it.
    pub fn set(&mut self, idx: usize, e: PickEntry) {
        if self.entries.len() <= idx {
            if !e.pickable() {
                return;
            }
            self.entries.resize(idx + 1, PickEntry::NEVER);
        }
        let old = std::mem::replace(&mut self.entries[idx], e);
        if old != e && !old.about(&e) {
            self.note(old);
            self.note(e);
        }
    }

    /// The object changed in a way its bounds do not tell: updated by the
    /// simulator (click action, flags), faces rebuilt, alpha class.
    pub fn changed(&mut self, idx: usize) {
        if let Some(&e) = self.entries.get(idx) {
            self.note(e);
        }
    }

    /// The object is gone.
    pub fn remove(&mut self, idx: usize) {
        self.set(idx, PickEntry::NEVER);
    }

    /// Everything is gone (teleport, logout).
    pub fn reset(&mut self) {
        self.entries.clear();
        self.disturbed = true;
    }

    /// The objects a search for `probe` must test, in index order, into `out`.
    fn candidates(&self, probe: &Probe, out: &mut Vec<usize>) {
        out.clear();
        out.extend(self.entries.iter().enumerate().filter(|(_, e)| e.candidate(probe)).map(|(i, _)| i));
    }

    /// Watch the surroundings of `probe` from now on.
    fn watch(&mut self, probe: Probe) {
        self.watch = Some(probe);
        self.disturbed = false;
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
        let (a, b) = (&self.probe, &o.probe);
        let depth = match (a.depth, b.depth) {
            (None, None) => true,
            (Some(p), Some(q)) => p.distance(q) <= 0.002 + 1e-4 * p.distance(a.ray.0),
            _ => false,
        };
        depth && self.far == o.far && a.ray.0.distance(b.ray.0) <= 5e-4 && a.ray.1.distance(b.ray.1) <= 2e-5
    }
}

/// The object under the cursor, kept between frames.
#[derive(Default)]
pub struct HoverCache {
    kept: Option<(Asked, Option<usize>, Instant)>,
    /// Candidate list, reused.
    candidates: Vec<usize>,
    /// Searches done and answers reused.
    pub searches: u64,
    pub reused: u64,
    /// AURORA_HOVER_CHECK: answers that differed from the full search (a
    /// new search / a kept answer), and when the counts were last logged.
    wrong: u64,
    stale: u64,
    logged: Option<Instant>,
}

impl HoverCache {
    /// The kept answer when it still holds for this question at `now`.
    fn get(&self, index: &PickIndex, asked: &Asked, now: Instant) -> Option<Option<usize>> {
        let (kept, answer, at) = self.kept.as_ref()?;
        (!index.disturbed && kept.same(asked) && now.saturating_duration_since(*at) < REFRESH).then_some(*answer)
    }
}

/// AURORA_HOVER_CHECK=1: every answer is compared with the full search.
fn check_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("AURORA_HOVER_CHECK").is_some())
}

impl super::Scene {
    /// The object the hover cursor is over: what
    /// `interaction_point(…, false, far)` then `interaction_at_ray(…, false)`
    /// give for the cursor `ray` and the depth pick `depth`, searched again
    /// only when the question or the scene around it changed, or every
    /// [`REFRESH`]. Not for clicks.
    pub fn hover_object(&mut self, world: &crate::world::World, ray: Option<(Vec3, Vec3)>, depth: Option<Vec3>, far: f32) -> Option<usize> {
        let ray = ray?;
        let probe = Probe { ray, depth };
        let asked = Asked { probe, far };
        let now = Instant::now();
        let full = |scene: &Self| {
            scene
                .interaction_point(world, Some(ray), depth, false, far)
                .and_then(|point| scene.interaction_at_ray(world, point, ray, false))
        };
        if let Some(answer) = self.hover.get(&self.pick, &asked, now) {
            self.hover.reused += 1;
            if check_enabled() {
                let expected = full(self);
                if expected != answer {
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
        let mut candidates = std::mem::take(&mut self.hover.candidates);
        self.pick.candidates(&probe, &mut candidates);
        let answer = self
            .interaction_point_among(world, Some(ray), depth, false, far, Some(&candidates))
            .and_then(|point| self.interaction_at_ray_among(world, point, ray, false, Some(&candidates)));
        if check_enabled() {
            let expected = full(self);
            if expected != answer {
                self.hover.wrong += 1;
                log::warn!(
                    "hover check: the search among {} candidates gives {answer:?}, the full search {expected:?}",
                    candidates.len()
                );
            }
            self.log_hover_check(now);
        }
        self.hover.candidates = candidates;
        self.hover.searches += 1;
        self.hover.kept = Some((asked, answer, now));
        self.pick.watch(probe);
        answer
    }

    /// AURORA_HOVER_CHECK: the counts, every ten seconds.
    fn log_hover_check(&mut self, now: Instant) {
        let h = &mut self.hover;
        let last = *h.logged.get_or_insert(now);
        if now.duration_since(last) >= Duration::from_secs(10) {
            h.logged = Some(now);
            log::info!(
                "hover check: {} searches, {} answers reused, {} wrong, {} kept answers out of date",
                h.searches,
                h.reused,
                h.wrong,
                h.stale
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAY: (Vec3, Vec3) = (Vec3::ZERO, Vec3::X);
    const PROBE: Probe = Probe { ray: RAY, depth: None };

    fn sphere(x: f32, y: f32, r: f32) -> PickEntry {
        PickEntry {
            center: Vec3::new(x, y, 0.0),
            radius: r,
            always: false,
        }
    }

    fn always(x: f32, y: f32) -> PickEntry {
        PickEntry {
            center: Vec3::new(x, y, 0.0),
            radius: 2.5,
            always: true,
        }
    }

    fn face() -> crate::scene::FaceDraw {
        crate::scene::FaceDraw {
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
            aux_repeats: [1.0; 3],
            te_alpha: 1.0,
            pbr_alpha: None,
            legacy_alpha: None,
            two_sided: false,
            repeats: 1.0,
            glow: false,
            pass: crate::scene::Pass::Opaque,
            glow_pool: false,
        }
    }

    #[test]
    fn entries_follow_the_object_state() {
        let mut g = ObjGpu::default();
        // no face yet
        assert_eq!(PickEntry::of(&g), PickEntry::NEVER);
        g.faces.push(face());
        g.center = Vec3::new(1.0, 2.0, 3.0);
        g.radius = 0.5;
        let e = PickEntry::of(&g);
        assert_eq!((e.center, e.radius, e.always), (g.center, 0.5, false));
        g.rigged = true;
        assert!(PickEntry::of(&g).always);
        g.rigged = false;
        g.radius = 0.0;
        assert!(PickEntry::of(&g).always && PickEntry::of(&g).pickable());
        g.radius = 0.5;
        g.is_avatar = true;
        let e = PickEntry::of(&g);
        assert!(e.always && e.radius == 2.5);
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
        index.watch(PROBE);
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
        index.watch(PROBE);
        index.set(1, sphere(5.1, 0.0, 0.5));
        assert!(index.disturbed);
        index.watch(PROBE);
        index.set(40, sphere(20.0, 0.0, 1.0));
        assert!(index.disturbed);
        index.watch(PROBE);
        index.set(40, sphere(20.0, 30.0, 1.0));
        assert!(index.disturbed);
        // arriving and leaving
        index.watch(PROBE);
        index.set(50, sphere(3.0, 0.1, 0.2));
        assert!(index.disturbed);
        index.watch(PROBE);
        index.remove(50);
        assert!(index.disturbed);
        // an avatar walking through the ray
        index.watch(PROBE);
        index.set(3, always(30.0, 1.0));
        assert!(index.disturbed);
        // around the depth point of the watched search, off its ray
        index.watch(Probe {
            ray: RAY,
            depth: Some(Vec3::new(6.0, 9.0, 0.0)),
        });
        index.set(2, sphere(6.0, 9.5, 0.5));
        assert!(index.disturbed);
        // an object without bounds: anywhere
        index.watch(PROBE);
        index.set(
            60,
            PickEntry {
                center: Vec3::new(0.0, 500.0, 0.0),
                radius: 0.0,
                always: true,
            },
        );
        assert!(index.disturbed);
        index.watch(PROBE);
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
        cache.kept = Some((asked, Some(1), t));
        index.watch(probe);
        let ms = Duration::from_millis;
        assert_eq!(cache.get(&index, &asked, t + ms(5)), Some(Some(1)));
        // camera settling: a hair's width
        let settling = with(
            (Vec3::splat(1e-4), (Vec3::X + Vec3::Y * 1e-5).normalize()),
            Some(Vec3::new(4.5005, 0.0, 0.0)),
        );
        assert_eq!(cache.get(&index, &settling, t + ms(5)), Some(Some(1)));
        // searched again after 100 ms whatever happens
        assert_eq!(cache.get(&index, &asked, t + ms(99)), Some(Some(1)));
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
        cache.kept = Some((asked, None, t));
        index.watch(probe);
        assert_eq!(cache.get(&index, &asked, t + ms(5)), Some(None));
    }
}
