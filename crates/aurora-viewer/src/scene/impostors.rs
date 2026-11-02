//! Avatar impostors, after LLVOAvatar::updateImpostor / needsImpostorUpdate
//! of the Second Life / Firestorm viewer (originally LGPL 2.1, Copyright (C) 2001-2024
//! Linden Research, Inc. and the Firestorm project): avatars beyond the
//! nearest RenderAvatarMaxNonImpostors are drawn as a card showing a picture
//! of them, taken from the camera and refreshed now and then (more often when
//! near, at once when the view turns or the avatar moves).

use aurora_render::{
    DrawCmd, IMPOSTOR_CAPTURES, IMPOSTOR_FULLBRIGHT, IMPOSTOR_MAX_TILES, IMPOSTOR_PENDING, IMPOSTOR_TILE, IMPOSTOR_TILES, ImpostorCapture,
    ImpostorSprite,
};
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::time::Instant;

/// Half size of the framed square around an avatar (m): body, hair, worn
/// objects.
const HALF: f32 = 1.4;
/// View turned by more than this since the picture: take a new one.
const MAX_ANGLE_COS: f32 = 0.9976; // 4°
/// The avatar moved by more than this: take a new one.
const MAX_MOVE: f32 = 0.3;
/// An impostor is posed again this long before its picture is due (s), so
/// that the frame that takes it has a fresh pose.
const POSE_LEAD: f32 = 0.05;

/// How impostors are posed. The Legacy Viewer animates an impostor only on
/// the frames that may refresh its picture (LLVOAvatar::computeNeedsUpdate,
/// every 32 to 64 frames, `updateMotions(HIDDEN_UPDATE)` in between); Aurora
/// posed every avatar in range at every frame, impostors included (about
/// half the skeletons posed at KB), and the anchors of all they wear with
/// them. Now an impostor keeps its last pose until its picture is nearly
/// due ([`Impostors::pose_can_wait`]); its animations still start and stop
/// on time. AURORA_IMPOSTOR_POSES=every poses them at every frame as
/// before; `alt` alternates the two ways by the second (one second in three
/// the old way).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PoseMode {
    Picture,
    Every,
    Alternate,
}

pub fn pose_mode() -> PoseMode {
    static MODE: std::sync::OnceLock<PoseMode> = std::sync::OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("AURORA_IMPOSTOR_POSES").as_deref() {
        Ok("every") => PoseMode::Every,
        Ok("alt") => PoseMode::Alternate,
        _ => PoseMode::Picture,
    })
}

static EVERY_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether impostors are posed at every frame in this second (the old way).
pub fn pose_every_frame() -> bool {
    match pose_mode() {
        PoseMode::Every => true,
        PoseMode::Picture => false,
        PoseMode::Alternate => EVERY_NOW.load(std::sync::atomic::Ordering::Relaxed),
    }
}

/// `AURORA_IMPOSTOR_POSES=alt`: the way of second `n` (true: the old way),
/// the same seconds as `AURORA_MATERIAL_REBUILD=alt`. None without `alt`.
pub fn alternate_poses(n: u64) -> Option<bool> {
    (pose_mode() == PoseMode::Alternate).then(|| {
        let old = n.is_multiple_of(3);
        EVERY_NOW.store(old, std::sync::atomic::Ordering::Relaxed);
        old
    })
}

/// An avatar to draw as an impostor.
#[derive(Debug, Clone, Copy)]
pub struct Far {
    /// Object index.
    pub idx: usize,
    pub center: Vec3,
    pub distance: f32,
    pub in_view: bool,
    /// Over the complexity limit: its picture is its fullbright silhouette.
    pub silhouette: bool,
}

/// Pixels an object of radius `radius` (m) covers in the picture of its
/// avatar: its textures are of no use finer than that.
pub fn picture_pixels(radius: f32) -> f32 {
    radius / HALF * IMPOSTOR_TILE as f32
}

#[derive(Debug, Clone, Copy)]
struct State {
    tile: u32,
    /// Picture taken: when, from which direction, of which center.
    taken: Option<(Instant, Vec3, Vec3)>,
    /// Card axes of the picture (half extents).
    right: Vec3,
    up: Vec3,
    /// The picture shows the silhouette (`Far::silhouette` when taken).
    silhouette: bool,
    seen: Instant,
}

#[derive(Default)]
pub struct Impostors {
    states: HashMap<usize, State>,
    free: Vec<u32>,
    capacity: u32,
}

/// This frame's impostor work: avatars to picture (slot in the captures)
/// and the cards to draw.
pub struct Plan {
    /// Avatar object index -> capture slot.
    pub capture: HashMap<usize, usize>,
    pub captures: Vec<ImpostorCapture>,
    pub sprites: Vec<ImpostorSprite>,
}

/// Picture period: near avatars more often (LL refreshes impostors on a
/// period that grows with distance).
fn period(distance: f32) -> f32 {
    (0.15 + distance / 60.0).min(1.0)
}

impl Impostors {
    /// Whether impostor `a` (as the last frame's lists saw it) can keep the
    /// pose it was given at `posed` in this frame: posed less than its
    /// picture period ago, and, in view, its picture still good (taken less
    /// than a period ago, from about the same direction, at the same place,
    /// as what it is now). Otherwise this frame may take its picture.
    pub fn pose_can_wait(&self, a: &Far, eye: Vec3, posed: Instant, now: Instant) -> bool {
        let wait = period(a.distance) - POSE_LEAD;
        if now.duration_since(posed).as_secs_f32() >= wait {
            return false;
        }
        if !a.in_view {
            return true;
        }
        let Some(s) = self.states.get(&a.idx) else {
            return false;
        };
        let Some((at, d, c)) = s.taken else {
            return false;
        };
        let dir = (a.center - eye).normalize_or(Vec3::X);
        s.silhouette == a.silhouette
            && dir.dot(d) >= MAX_ANGLE_COS
            && a.center.distance(c) <= MAX_MOVE
            && now.duration_since(at).as_secs_f32() < wait
    }

    /// `avatars`: the avatars to draw as impostors.
    pub fn plan(&mut self, avatars: &[Far], eye: Vec3, now: Instant) -> Plan {
        if self.capacity == 0 {
            self.capacity = IMPOSTOR_TILES;
            self.free = (0..IMPOSTOR_TILES).rev().collect();
        }
        // LLVOAvatar::updateImpostors (indra/newview/llvoavatar.cpp) only
        // pictures visible avatars. Cached offscreen pictures are expendable.
        let visible: HashSet<usize> = avatars.iter().filter(|a| a.in_view).map(|a| a.idx).collect();
        // Forget absent or offscreen avatars after five seconds. A visible
        // picture survives long frames and is never evicted for a refresh.
        let free = &mut self.free;
        self.states.retain(|idx, s| {
            let keep = visible.contains(idx) || now.duration_since(s.seen).as_secs_f32() < 5.0;
            if !keep {
                free.push(s.tile);
            }
            keep
        });
        let mut wanted: Vec<(f32, usize)> = Vec::new();
        let mut in_view: Vec<&Far> = avatars.iter().filter(|a| a.in_view).collect();
        in_view.sort_by(|a, b| a.distance.total_cmp(&b.distance).then(a.idx.cmp(&b.idx)));
        for a in &in_view {
            let Far { idx, center, distance, .. } = **a;
            if !self.states.contains_key(&idx) {
                if self.free.is_empty() {
                    let victim = self
                        .states
                        .iter()
                        .filter(|(i, _)| !visible.contains(i))
                        .min_by_key(|(i, s)| (s.seen, **i))
                        .map(|(&i, _)| i);
                    if let Some(victim) = victim {
                        if let Some(s) = self.states.remove(&victim) {
                            self.free.push(s.tile);
                        }
                    } else if self.capacity < IMPOSTOR_MAX_TILES {
                        let next = (self.capacity * 2).min(IMPOSTOR_MAX_TILES);
                        self.free.extend((self.capacity..next).rev());
                        self.capacity = next;
                    }
                }
                let Some(tile) = self.free.pop() else {
                    continue; // Memory budget reached: a silhouette below, never full geometry.
                };
                self.states.insert(
                    idx,
                    State {
                        tile,
                        taken: None,
                        right: Vec3::X,
                        up: Vec3::Z,
                        silhouette: a.silhouette,
                        seen: now,
                    },
                );
            }
            let Some(s) = self.states.get_mut(&idx) else {
                continue;
            };
            s.seen = now;
            let dir = (center - eye).normalize_or(Vec3::X);
            let stale = match s.taken {
                None => f32::INFINITY,
                // it went over the complexity limit, or back under it: its
                // picture shows what it no longer is
                Some(_) if s.silhouette != a.silhouette => f32::INFINITY,
                Some((at, d, c)) => {
                    let age = now.duration_since(at).as_secs_f32();
                    if dir.dot(d) < MAX_ANGLE_COS || center.distance(c) > MAX_MOVE {
                        age + 10.0
                    } else if age > period(distance) {
                        age
                    } else {
                        continue;
                    }
                }
            };
            wanted.push((stale, idx));
        }
        // the stalest first
        wanted.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut plan = Plan {
            capture: HashMap::new(),
            captures: Vec::new(),
            sprites: Vec::new(),
        };
        for (_, idx) in wanted.into_iter().take(IMPOSTOR_CAPTURES) {
            let Some(a) = avatars.iter().find(|a| a.idx == idx) else {
                continue;
            };
            let Some(s) = self.states.get_mut(&idx) else {
                continue;
            };
            let center = a.center;
            let dir = (center - eye).normalize_or(Vec3::X);
            let world_up = if dir.z.abs() > 0.95 { Vec3::Y } else { Vec3::Z };
            let right = dir.cross(world_up).normalize_or(Vec3::X);
            let up = right.cross(dir);
            let eye_c = center - dir * (HALF + 1.0);
            s.taken = Some((now, dir, center));
            s.right = right * HALF;
            s.up = up * HALF;
            s.silhouette = a.silhouette;
            plan.capture.insert(idx, plan.captures.len());
            plan.captures.push(ImpostorCapture {
                tile: s.tile,
                eye: eye_c,
                view: glam::camera::rh::view::look_at_mat4(eye_c, center, up),
                half: HALF,
                depth: 2.0 * HALF + 2.0,
                opaque: Vec::new(),
                blend: Vec::new(),
            });
        }
        // A card for every visible impostor, even before its first capture
        // or beyond the memory budget. Existing pictures survive refreshes.
        for a in in_view {
            let s = self.states.get(&a.idx).filter(|s| s.taken.is_some());
            let dir = (a.center - eye).normalize_or(Vec3::X);
            let world_up = if dir.z.abs() > 0.95 { Vec3::Y } else { Vec3::Z };
            let right = dir.cross(world_up).normalize_or(Vec3::X);
            let up = right.cross(dir);
            plan.sprites.push(ImpostorSprite {
                center: a.center.to_array(),
                tile: s.map_or(IMPOSTOR_PENDING | IMPOSTOR_FULLBRIGHT, |s| {
                    s.tile | if s.silhouette { IMPOSTOR_FULLBRIGHT } else { 0 }
                }),
                right: s.map_or(right * HALF, |s| s.right).to_array(),
                up: s.map_or(up * HALF, |s| s.up).to_array(),
            });
        }
        plan
    }
}

/// Draws of a pictured avatar, by pass.
pub fn add_draw(capture: &mut ImpostorCapture, cmd: DrawCmd, blend: bool) {
    if blend {
        capture.blend.push(cmd);
    } else {
        capture.opaque.push(cmd);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn far(idx: usize, silhouette: bool) -> Far {
        Far {
            idx,
            center: Vec3::new(20.0, 0.0, 1.0),
            distance: 20.0,
            in_view: true,
            silhouette,
        }
    }

    #[test]
    fn an_impostor_keeps_its_pose_until_its_picture_is_nearly_due() {
        let mut imp = Impostors::default();
        let t0 = Instant::now();
        let a = far(1, false);
        let eye = Vec3::new(0.0, -30.0, 1.0);
        // never pictured: posed
        assert!(!imp.pose_can_wait(&a, eye, t0, t0));
        let plan = imp.plan(&[a], eye, t0);
        assert!(plan.capture.contains_key(&1));
        let p = period(a.distance);
        let soon = t0 + std::time::Duration::from_secs_f32(p * 0.5);
        assert!(imp.pose_can_wait(&a, eye, t0, soon));
        // its picture is nearly due, or its pose too old
        let late = t0 + std::time::Duration::from_secs_f32(p - POSE_LEAD * 0.5);
        assert!(!imp.pose_can_wait(&a, eye, t0, late));
        // the view turned: the plan takes it at once
        let turned = Vec3::new(10.0, -30.0, 1.0);
        assert!(!imp.pose_can_wait(&a, turned, t0, soon));
        // it moved
        let moved = Far {
            center: a.center + Vec3::X,
            ..a
        };
        assert!(!imp.pose_can_wait(&moved, eye, t0, soon));
        // out of view: only its pose's age counts
        let hidden = Far { in_view: false, ..moved };
        assert!(imp.pose_can_wait(&hidden, turned, t0, soon));
        assert!(!imp.pose_can_wait(&hidden, turned, t0, late));
    }

    #[test]
    fn a_picture_is_taken_again_when_the_avatar_becomes_a_silhouette() {
        let mut imp = Impostors::default();
        let t0 = Instant::now();
        let plan = imp.plan(&[far(7, false)], Vec3::ZERO, t0);
        assert_eq!(plan.capture.get(&7), Some(&0));
        assert_eq!(plan.sprites[0].tile & IMPOSTOR_FULLBRIGHT, 0);
        // a moment later, well within its period: the picture is kept
        let t1 = t0 + Duration::from_millis(50);
        assert!(imp.plan(&[far(7, false)], Vec3::ZERO, t1).captures.is_empty());
        // over the complexity limit: pictured at once, and its card is
        // marked fullbright with the picture that shows the silhouette
        let plan = imp.plan(&[far(7, true)], Vec3::ZERO, t1);
        assert_eq!(plan.captures.len(), 1);
        assert_eq!(plan.sprites[0].tile & IMPOSTOR_FULLBRIGHT, IMPOSTOR_FULLBRIGHT);
        assert_eq!(plan.sprites[0].tile & !IMPOSTOR_FULLBRIGHT, plan.captures[0].tile);
        // and back under the limit
        let plan = imp.plan(&[far(7, false)], Vec3::ZERO, t1);
        assert_eq!(plan.captures.len(), 1);
        assert_eq!(plan.sprites[0].tile & IMPOSTOR_FULLBRIGHT, 0);
    }

    #[test]
    fn a_card_keeps_the_flag_of_its_picture_until_the_next_one() {
        let mut imp = Impostors::default();
        let t0 = Instant::now();
        // more avatars turn into silhouettes than pictures are taken in a frame
        let n = IMPOSTOR_CAPTURES + 1;
        let before: Vec<Far> = (0..n).map(|i| far(i, false)).collect();
        let mut t = t0;
        while imp
            .plan(&before, Vec3::ZERO, t)
            .sprites
            .iter()
            .filter(|s| s.tile & IMPOSTOR_PENDING == 0)
            .count()
            < n
        {
            t += Duration::from_millis(1);
        }
        let after: Vec<Far> = (0..n).map(|i| far(i, true)).collect();
        let plan = imp.plan(&after, Vec3::ZERO, t);
        assert_eq!(plan.captures.len(), IMPOSTOR_CAPTURES);
        let marked = plan.sprites.iter().filter(|s| s.tile & IMPOSTOR_FULLBRIGHT != 0).count();
        assert_eq!(marked, IMPOSTOR_CAPTURES, "the avatar not pictured yet still shows its old picture");
    }

    #[test]
    fn offscreen_avatars_do_not_reserve_picture_slots() {
        let mut imp = Impostors::default();
        let mut crowd: Vec<_> = (0..200)
            .map(|i| Far {
                in_view: false,
                ..far(i, false)
            })
            .collect();
        crowd.push(far(200, false));
        let plan = imp.plan(&crowd, Vec3::ZERO, Instant::now());
        assert_eq!(plan.capture.len(), 1);
        assert!(plan.capture.contains_key(&200));
        assert_eq!(plan.sprites.len(), 1);
        assert_eq!(imp.states.len(), 1);
        assert_eq!(imp.capacity, IMPOSTOR_TILES);
    }

    #[test]
    fn a_camera_turn_reclaims_offscreen_slots_before_growing() {
        let mut imp = Impostors::default();
        let t = Instant::now();
        let mut crowd: Vec<_> = (0..IMPOSTOR_TILES as usize).map(|i| far(i, false)).collect();
        imp.plan(&crowd, Vec3::ZERO, t);
        for a in &mut crowd {
            a.in_view = false;
        }
        crowd.push(far(1000, false));
        let plan = imp.plan(&crowd, Vec3::ZERO, t + Duration::from_millis(1));
        assert!(plan.capture.contains_key(&1000));
        assert_eq!(plan.sprites.len(), 1);
        assert_eq!(plan.sprites[0].tile & IMPOSTOR_PENDING, 0);
        assert_eq!(imp.capacity, IMPOSTOR_TILES);
    }

    #[test]
    fn more_than_one_page_of_visible_impostors_all_receive_distinct_pictures() {
        let mut imp = Impostors::default();
        let crowd: Vec<_> = (0..96).map(|i| far(i, false)).collect();
        let mut t = Instant::now();
        for _ in 0..48 {
            let plan = imp.plan(&crowd, Vec3::ZERO, t);
            assert_eq!(plan.sprites.len(), crowd.len(), "waiting avatars still have a card");
            assert!(plan.captures.len() <= IMPOSTOR_CAPTURES);
            t += Duration::from_millis(1);
        }
        let plan = imp.plan(&crowd, Vec3::ZERO, t);
        assert!(plan.sprites.iter().all(|s| s.tile & IMPOSTOR_PENDING == 0));
        let tiles: HashSet<_> = plan.sprites.iter().map(|s| s.tile).collect();
        assert_eq!(tiles.len(), crowd.len());
        assert_eq!(imp.capacity, 128);
        // A changed view refreshes only two pictures, keeping all the others.
        let turned = imp.plan(&crowd, Vec3::new(0.0, 30.0, 0.0), t);
        assert_eq!(turned.captures.len(), IMPOSTOR_CAPTURES);
        assert!(turned.sprites.iter().all(|s| s.tile & IMPOSTOR_PENDING == 0));
    }

    #[test]
    fn excess_visible_avatars_keep_a_card_without_exceeding_the_memory_budget() {
        let mut imp = Impostors::default();
        let crowd: Vec<_> = (0..IMPOSTOR_MAX_TILES as usize + 10).map(|i| far(i, false)).collect();
        let plan = imp.plan(&crowd, Vec3::ZERO, Instant::now());
        assert_eq!(imp.capacity, IMPOSTOR_MAX_TILES);
        assert_eq!(imp.states.len(), IMPOSTOR_MAX_TILES as usize);
        assert_eq!(plan.sprites.len(), crowd.len());
        assert_eq!(
            plan.sprites.iter().filter(|s| s.tile & IMPOSTOR_PENDING == 0).count(),
            IMPOSTOR_CAPTURES
        );
        assert!(plan.captures.iter().all(|c| c.tile < IMPOSTOR_MAX_TILES));
    }

    #[test]
    fn textures_are_wanted_as_fine_as_the_picture_shows_them() {
        // the framed square is a tile wide
        assert_eq!(picture_pixels(HALF), IMPOSTOR_TILE as f32);
        assert_eq!(picture_pixels(HALF / 4.0), IMPOSTOR_TILE as f32 / 4.0);
    }
}
